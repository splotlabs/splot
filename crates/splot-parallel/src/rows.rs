// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Row-ordered frame scheduling on dedicated worker threads.
//!
//! A [`FrameWork`] is one in-flight frame. Its stages (parse, resolve,
//! reconstruct, filter) each advance one superblock row at a time, in order.
//! Workers hold one lock, scan the frames oldest first, and ask each frame to
//! [`claim`](FrameWork::claim) a row whose inputs are published. Readiness is
//! read from [`RowProgress`] counters, so nothing registers a waiter and a
//! worker never blocks inside a row. A frame depends only on older frames, and
//! older frames are claimed first, so the oldest frame always has a row that
//! can run or is running: the pipeline cannot stall.
//!
//! A publication raises a generation counter before it signals, and a worker
//! reads that counter before it scans, so a scan that missed the publication
//! never parks on it.

use core::num::NonZeroUsize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;

use parking_lot::{Condvar, Mutex};

use crate::error::ParallelError;

const WORKER_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

/// One row of one stage, chosen by [`FrameWork::claim`] and run by
/// [`FrameWork::run`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Claim {
    /// The frame's own stage index.
    pub stage: usize,
    /// The row of that stage.
    pub row: usize,
}

/// One in-flight frame whose stages advance row by row.
pub trait FrameWork: Send + Sync {
    /// Picks a row whose inputs are published, or `None` when nothing in this
    /// frame can run now. Called under the scheduler lock, so it must only
    /// read atomics and [`Stage`] cursors.
    fn claim(&self) -> Option<Claim>;
    /// Runs the claimed row off the lock, publishes its progress, and calls
    /// [`Stage::finish`] for it.
    fn run(&self, claim: Claim);
    /// Whether every stage has finished, including after a failure.
    fn done(&self) -> bool;
}

/// A monotone row counter other frames read to decide readiness.
///
/// Publishing wakes the scheduler's workers and its waiting driver.
#[derive(Clone, Debug)]
pub struct RowProgress {
    value: Arc<AtomicUsize>,
    core: Arc<Core>,
}

impl RowProgress {
    /// The terminal value a failed producer publishes, which satisfies every
    /// threshold so dependents run and observe the failure.
    pub const FAILED: usize = usize::MAX;

    /// The highest published row count.
    #[must_use]
    pub fn current(&self) -> usize {
        self.value.load(Ordering::Acquire)
    }

    /// Raises the counter to `rows` and wakes one worker.
    pub fn publish(&self, rows: usize) {
        if self.value.fetch_max(rows, Ordering::AcqRel) < rows {
            self.core.notify(false);
        }
    }

    /// Publishes [`Self::FAILED`].
    pub fn fail(&self) {
        self.publish(Self::FAILED);
    }

    /// Whether the producer failed.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.current() == Self::FAILED
    }
}

/// The ordered row cursor of one stage: at most one row runs at a time.
#[derive(Debug)]
pub struct Stage {
    next: AtomicUsize,
    running: AtomicBool,
    rows: AtomicUsize,
}

impl Stage {
    /// A stage of `rows` rows, none run yet.
    #[must_use]
    pub const fn new(rows: usize) -> Self {
        Self {
            next: AtomicUsize::new(0),
            running: AtomicBool::new(false),
            rows: AtomicUsize::new(rows),
        }
    }

    /// Resets the cursor for a new frame of `rows` rows.
    pub fn reset(&mut self, rows: usize) {
        *self.next.get_mut() = 0;
        *self.running.get_mut() = false;
        *self.rows.get_mut() = rows;
    }

    /// Sets the row count once the frame's geometry is known, before any
    /// row of this stage is claimed.
    pub fn set_rows(&self, rows: usize) {
        self.rows.store(rows, Ordering::Release);
    }

    /// Claims the next row when no row is running and `ready(row)` holds.
    pub fn claim(&self, ready: impl FnOnce(usize) -> bool) -> Option<usize> {
        if self.running.load(Ordering::Acquire) {
            return None;
        }
        let row = self.next.load(Ordering::Acquire);
        if row >= self.rows() || !ready(row) {
            return None;
        }
        self.running.store(true, Ordering::Release);
        Some(row)
    }

    /// Marks the claimed `row` as run, so the next row can be claimed.
    pub fn finish(&self, row: usize) {
        self.next.store(row.saturating_add(1), Ordering::Release);
        self.running.store(false, Ordering::Release);
    }

    /// Releases the claimed `row` unrun, so it can be claimed again once its
    /// inputs are published.
    pub fn retry(&self, row: usize) {
        self.next.store(row, Ordering::Release);
        self.running.store(false, Ordering::Release);
    }

    /// The next row to run.
    #[must_use]
    pub fn next(&self) -> usize {
        self.next.load(Ordering::Acquire)
    }

    /// The rows this stage runs.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows.load(Ordering::Acquire)
    }

    /// Whether every row has run.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        !self.running.load(Ordering::Acquire) && self.next() >= self.rows()
    }
}

const PENDING: u8 = 0;
const READY: u8 = 1;
const RUNNING: u8 = 2;
const DONE: u8 = 3;

/// A stage whose rows are independent: any ready row runs, in any order,
/// several at a time.
#[derive(Debug, Default)]
pub struct ParallelStage {
    rows: Mutex<Vec<u8>>,
}

impl ParallelStage {
    /// A stage with no rows yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
        }
    }

    /// Sets the row count once the frame's geometry is known. Every row
    /// starts pending.
    pub fn set_rows(&self, rows: usize) {
        let mut states = self.rows.lock();
        states.clear();
        states.resize(rows, PENDING);
    }

    /// Claims the first pending row for which `ready(row)` holds.
    pub fn claim(&self, mut ready: impl FnMut(usize) -> bool) -> Option<usize> {
        let mut states = self.rows.lock();
        let row = states
            .iter()
            .enumerate()
            .position(|(row, state)| *state == PENDING && ready(row))?;
        states[row] = RUNNING;
        Some(row)
    }

    /// Marks `row` as runnable, for a producer that knows when its input lands.
    pub fn ready(&self, row: usize) {
        if let Some(state) = self.rows.lock().get_mut(row)
            && *state == PENDING
        {
            *state = READY;
        }
    }

    /// Claims the first row a producer marked [`ready`](Self::ready).
    pub fn claim_ready(&self) -> Option<usize> {
        let mut states = self.rows.lock();
        let row = states.iter().position(|state| *state == READY)?;
        states[row] = RUNNING;
        Some(row)
    }

    /// Marks the claimed `row` as run.
    pub fn finish(&self, row: usize) {
        if let Some(state) = self.rows.lock().get_mut(row) {
            *state = DONE;
        }
    }

    /// Whether `row` has run.
    #[must_use]
    pub fn is_done(&self, row: usize) -> bool {
        self.rows
            .lock()
            .get(row)
            .is_some_and(|state| *state == DONE)
    }

    /// Whether every row has run.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.rows.lock().iter().all(|state| *state == DONE)
    }
}

#[derive(Default)]
struct Frames {
    live: VecDeque<Arc<dyn FrameWork>>,
    shutdown: bool,
}

/// The scheduler's shared state and its wake-up event.
///
/// A worker reads `generation` before it scans the frames. A publisher raises
/// `generation` and signals only when a waiter is registered, so a scan that
/// missed the publication sees the raised generation and does not park. A
/// worker drops its handle on a frame before it pops the finished frame, so
/// the signal after the worker's own clone is dropped is the last change a
/// driver waiting for the frame's sole owner needs to see, whichever worker
/// popped the frame.
#[derive(Default)]
struct Core {
    frames: Mutex<Frames>,
    generation: AtomicU64,
    waiters: AtomicUsize,
    parked: AtomicUsize,
    /// Set while a single-worker signal is pending, so a burst of
    /// publications wakes one worker once; that worker clears it before it
    /// parks again.
    signaled: AtomicBool,
    park: Mutex<()>,
    changed: Condvar,
    /// The driver parks apart from the workers: a single-worker signal may
    /// wake a worker that finds nothing, so every publication wakes the
    /// driver on its own condvar.
    driver_parked: AtomicBool,
    driver_changed: Condvar,
}

/// Every live scheduler, so publications made through other primitives can
/// wake row workers while both scheduler families coexist.
static SCHEDULERS: Mutex<Vec<Weak<Core>>> = Mutex::new(Vec::new());

/// Wakes one worker of every live scheduler.
pub(crate) fn wake_all() {
    let schedulers = SCHEDULERS.lock();
    for core in schedulers.iter().filter_map(Weak::upgrade) {
        core.notify(false);
    }
}

/// A handle that makes [`RowProgress`] counters for one scheduler.
#[derive(Clone, Debug)]
pub struct RowWake(Arc<Core>);

impl RowWake {
    /// A wake with no workers, for a decode that runs on one thread.
    #[must_use]
    pub fn detached() -> Self {
        Self(Arc::new(Core::default()))
    }

    /// A row counter whose publications wake this scheduler.
    #[must_use]
    pub fn progress(&self) -> RowProgress {
        RowProgress {
            value: Arc::new(AtomicUsize::new(0)),
            core: Arc::clone(&self.0),
        }
    }

    /// Wakes one worker after a frame advanced a counter of its own.
    pub fn notify(&self) {
        self.0.notify(false);
    }
}

impl core::fmt::Debug for Core {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Core")
    }
}

impl Core {
    /// Raises the generation and signals one waiter, or all of them.
    ///
    /// A single-worker signal is skipped while one is already pending; a
    /// worker that found work signals through [`Self::pass_on`] instead, so
    /// the next idle worker still looks for more.
    fn notify(&self, all: bool) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if self.driver_parked.load(Ordering::SeqCst) {
            let park = self.park.lock();
            self.driver_changed.notify_all();
            drop(park);
        }
        if self.waiters.load(Ordering::SeqCst) == 0 {
            return;
        }
        if !all && self.signaled.swap(true, Ordering::SeqCst) {
            return;
        }
        self.signal(all);
    }

    /// Signals one idle worker after a claim, whether or not a signal is
    /// pending, so a burst of ready rows spreads across the workers.
    fn pass_on(&self) {
        self.signaled.store(true, Ordering::SeqCst);
        if self.waiters.load(Ordering::SeqCst) != 0 {
            self.signal(false);
        }
    }

    fn signal(&self, all: bool) {
        let park = self.park.lock();
        if all {
            self.changed.notify_all();
        } else {
            self.changed.notify_one();
        }
        drop(park);
    }

    /// Parks the driver until the generation moves past `observed`.
    fn wait_driver_if_unchanged(&self, observed: u64) {
        let mut park = self.park.lock();
        self.driver_parked.store(true, Ordering::SeqCst);
        if self.generation.load(Ordering::SeqCst) == observed {
            self.driver_changed.wait(&mut park);
        }
        self.driver_parked.store(false, Ordering::SeqCst);
        drop(park);
    }

    /// Parks until the generation moves past `observed`.
    fn wait_if_unchanged(&self, observed: u64) {
        let mut park = self.park.lock();
        self.waiters.fetch_add(1, Ordering::SeqCst);
        self.signaled.store(false, Ordering::SeqCst);
        if self.generation.load(Ordering::SeqCst) == observed {
            self.parked.fetch_add(1, Ordering::Relaxed);
            self.changed.wait(&mut park);
            self.parked.fetch_sub(1, Ordering::Relaxed);
        }
        self.waiters.fetch_sub(1, Ordering::SeqCst);
        drop(park);
    }

    fn worker(&self) {
        loop {
            let observed = self.generation.load(Ordering::SeqCst);
            let frames = self.frames.lock();
            let claimed = frames
                .live
                .iter()
                .find_map(|frame| frame.claim().map(|claim| (Arc::clone(frame), claim)));
            let Some((frame, claim)) = claimed else {
                let shutdown = frames.shutdown;
                drop(frames);
                if shutdown {
                    return;
                }
                self.wait_if_unchanged(observed);
                continue;
            };
            drop(frames);
            self.pass_on();
            frame.run(claim);
            let done = frame.done();
            drop(frame);
            let mut frames = self.frames.lock();
            while frames.live.front().is_some_and(|front| front.done()) {
                frames.live.pop_front();
            }
            drop(frames);
            if done {
                self.notify(true);
            }
        }
    }
}

/// Starts one named thread with the worker stack size.
///
/// # Errors
/// Returns [`ParallelError::ThreadSpawn`] when the thread cannot start.
pub fn spawn_thread(
    name: &str,
    body: impl FnOnce() + Send + 'static,
) -> Result<JoinHandle<()>, ParallelError> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(WORKER_STACK_SIZE_BYTES)
        .spawn(body)
        .map_err(|error| ParallelError::ThreadSpawn {
            message: error.to_string(),
        })
}

/// Runs `body` to completion on a thread with the worker stack size and
/// returns its result, for a decode driver whose recursion needs more stack
/// than the caller's thread has.
///
/// # Errors
/// Returns [`ParallelError::ThreadSpawn`] when the thread cannot start.
pub fn run_on_thread<R: Send>(
    name: &str,
    body: impl FnOnce() -> R + Send,
) -> Result<R, ParallelError> {
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name(name.to_owned())
            .stack_size(WORKER_STACK_SIZE_BYTES)
            .spawn_scoped(scope, body)
            .map_err(|error| ParallelError::ThreadSpawn {
                message: error.to_string(),
            })?;
        thread.join().map_err(|_| ParallelError::ThreadSpawn {
            message: "the thread panicked".to_owned(),
        })
    })
}

/// Dedicated worker threads that run [`FrameWork`] rows oldest frame first.
#[derive(Debug)]
pub struct RowScheduler {
    core: Arc<Core>,
    workers: Vec<JoinHandle<()>>,
}

impl RowScheduler {
    /// Starts `threads` workers named `splot-row-{i}`.
    ///
    /// # Errors
    /// Returns [`ParallelError::PoolBuild`] when a thread cannot start.
    pub fn new(threads: NonZeroUsize) -> Result<Self, ParallelError> {
        let core = Arc::new(Core::default());
        {
            let mut schedulers = SCHEDULERS.lock();
            schedulers.retain(|weak| weak.strong_count() != 0);
            schedulers.push(Arc::downgrade(&core));
        }
        let mut workers = Vec::with_capacity(threads.get());
        for index in 0..threads.get() {
            let core = Arc::clone(&core);
            workers.push(spawn_thread(&format!("splot-row-{index}"), move || {
                core.worker();
            })?);
        }
        Ok(Self { core, workers })
    }

    /// A row counter whose publications wake this scheduler.
    #[must_use]
    pub fn progress(&self) -> RowProgress {
        self.wake().progress()
    }

    /// The handle frames use to make counters bound to this scheduler.
    #[must_use]
    pub fn wake(&self) -> RowWake {
        RowWake(Arc::clone(&self.core))
    }

    /// Adds the newest frame. Its rows run after every older frame's ready rows.
    pub fn push(&self, frame: Arc<dyn FrameWork>) {
        let mut frames = self.core.frames.lock();
        frames.live.push_back(frame);
        drop(frames);
        self.core.notify(false);
    }

    /// Workers parked with nothing to run, for tests.
    #[cfg(test)]
    pub(crate) fn parked_workers(&self) -> usize {
        self.core.parked.load(Ordering::Relaxed)
    }

    /// Frames pushed and not yet done.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.core.frames.lock().live.len()
    }

    /// Blocks the driver until `ready()` holds. `ready` runs under the
    /// scheduler lock and is re-checked after every publication.
    pub fn wait(&self, mut ready: impl FnMut() -> bool) {
        loop {
            let observed = self.core.generation.load(Ordering::SeqCst);
            let frames = self.core.frames.lock();
            let done = ready();
            drop(frames);
            if done {
                return;
            }
            self.core.wait_driver_if_unchanged(observed);
        }
    }

    /// The number of worker threads.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.workers.len()
    }
}

impl Drop for RowScheduler {
    fn drop(&mut self) {
        self.core.frames.lock().shutdown = true;
        self.core.notify(true);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use core::num::NonZeroUsize;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    fn threads(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    /// One stage of `rows` rows whose row `r` is ready when `gate` reaches
    /// `need(r)`; running a row records it and publishes `done_rows`.
    struct Chain {
        id: usize,
        stage: Stage,
        gate: Option<RowProgress>,
        need: fn(usize) -> usize,
        done_rows: RowProgress,
        log: Arc<Mutex<Vec<(usize, usize)>>>,
        fail_at: Option<usize>,
    }

    impl FrameWork for Chain {
        fn claim(&self) -> Option<Claim> {
            self.stage
                .claim(|row| {
                    self.gate
                        .as_ref()
                        .is_none_or(|gate| gate.current() >= (self.need)(row))
                })
                .map(|row| Claim { stage: 0, row })
        }

        fn run(&self, claim: Claim) {
            self.log.lock().push((self.id, claim.row));
            if self.fail_at == Some(claim.row) {
                self.done_rows.fail();
            } else {
                self.done_rows.publish(claim.row + 1);
            }
            self.stage.finish(claim.row);
        }

        fn done(&self) -> bool {
            self.stage.is_finished()
        }
    }

    fn chain(
        scheduler: &RowScheduler,
        id: usize,
        rows: usize,
        gate: Option<RowProgress>,
        need: fn(usize) -> usize,
        log: &Arc<Mutex<Vec<(usize, usize)>>>,
    ) -> Arc<Chain> {
        Arc::new(Chain {
            id,
            stage: Stage::new(rows),
            gate,
            need,
            done_rows: scheduler.progress(),
            log: Arc::clone(log),
            fail_at: None,
        })
    }

    fn wait_done(scheduler: &RowScheduler, frames: &[Arc<Chain>]) {
        let deadline = Instant::now() + Duration::from_secs(10);
        scheduler.wait(|| frames.iter().all(|frame| frame.done()) || Instant::now() > deadline);
        assert!(
            frames.iter().all(|frame| frame.done()),
            "frames did not finish"
        );
    }

    #[test]
    fn stage_claims_one_row_at_a_time_in_order() {
        let stage = Stage::new(2);
        assert_eq!(stage.claim(|_| true), Some(0));
        assert_eq!(stage.claim(|_| true), None, "row 0 is running");
        stage.finish(0);
        assert_eq!(stage.claim(|_| false), None, "row 1 is not ready");
        assert_eq!(stage.claim(|_| true), Some(1));
        stage.finish(1);
        assert!(stage.is_finished());
        assert_eq!(stage.claim(|_| true), None);
    }

    #[test]
    fn a_retried_row_stays_claimable_and_set_rows_resizes_a_stage() {
        let stage = Stage::new(0);
        assert_eq!(stage.claim(|_| true), None, "no rows yet");
        stage.set_rows(2);
        assert_eq!(stage.claim(|_| true), Some(0));
        stage.retry(0);
        assert_eq!(stage.claim(|_| true), Some(0), "a retried row runs again");
        stage.finish(0);
        assert_eq!(stage.claim(|_| true), Some(1));
        stage.finish(1);
        assert!(stage.is_finished());
    }

    #[test]
    fn a_parallel_stage_claims_any_ready_row_once_and_finishes_when_all_ran() {
        let stage = ParallelStage::new();
        assert!(stage.is_finished(), "no rows is finished");
        stage.set_rows(3);
        assert!(!stage.is_finished());
        assert_eq!(stage.claim(|row| row != 0), Some(1));
        assert_eq!(stage.claim(|row| row != 0), Some(2), "row 1 is running");
        assert_eq!(stage.claim(|row| row != 0), None, "row 0 is not ready");
        stage.finish(2);
        stage.finish(1);
        assert_eq!(stage.claim(|_| true), Some(0));
        assert!(!stage.is_finished(), "row 0 is still running");
        stage.finish(0);
        assert!(stage.is_finished());
        assert_eq!(stage.claim(|_| true), None);
    }

    #[test]
    fn a_parallel_stage_hands_out_rows_marked_ready_without_a_predicate() {
        let stage = ParallelStage::new();
        stage.set_rows(3);
        assert_eq!(stage.claim_ready(), None);
        stage.ready(2);
        stage.ready(0);
        assert_eq!(stage.claim_ready(), Some(0));
        assert_eq!(stage.claim_ready(), Some(2));
        assert_eq!(stage.claim_ready(), None, "row 1 was never marked ready");
        stage.finish(0);
        stage.finish(2);
        stage.ready(1);
        assert_eq!(stage.claim_ready(), Some(1));
        stage.finish(1);
        assert!(stage.is_finished());
    }

    #[test]
    fn a_publication_between_scan_and_park_is_not_lost() {
        let scheduler = RowScheduler::new(threads(1)).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let gate = scheduler.progress();
        let frame = chain(&scheduler, 0, 1, Some(gate.clone()), |_| 1, &log);
        scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
        for _ in 0..200 {
            gate.publish(1);
        }
        wait_done(&scheduler, &[frame]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while scheduler.parked_workers() == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(scheduler.parked_workers(), 1, "the idle worker parks again");
    }

    #[test]
    fn dependent_frame_rows_run_after_the_rows_they_need() {
        let scheduler = RowScheduler::new(threads(2)).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let first = chain(&scheduler, 0, 4, None, |_| 0, &log);
        let second = chain(
            &scheduler,
            1,
            4,
            Some(first.done_rows.clone()),
            |row| row + 1,
            &log,
        );
        scheduler.push(Arc::clone(&first) as Arc<dyn FrameWork>);
        scheduler.push(Arc::clone(&second) as Arc<dyn FrameWork>);
        wait_done(&scheduler, &[Arc::clone(&first), Arc::clone(&second)]);
        let log = log.lock();
        for id in 0..2 {
            let rows: Vec<_> = log
                .iter()
                .filter(|(f, _)| *f == id)
                .map(|(_, r)| *r)
                .collect();
            assert_eq!(rows, vec![0, 1, 2, 3], "frame {id} rows run in order");
        }
        for row in 0..4 {
            let producer = log.iter().position(|e| *e == (0, row)).unwrap();
            let consumer = log.iter().position(|e| *e == (1, row)).unwrap();
            assert!(producer < consumer, "row {row} consumed before produced");
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while scheduler.in_flight() != 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(scheduler.in_flight(), 0, "finished frames leave the queue");
    }

    #[test]
    fn external_publication_wakes_a_parked_worker() {
        let scheduler = RowScheduler::new(threads(1)).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let gate = scheduler.progress();
        let frame = chain(&scheduler, 0, 1, Some(gate.clone()), |_| 1, &log);
        scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
        std::thread::sleep(Duration::from_millis(20));
        assert!(!frame.done());
        gate.publish(1);
        wait_done(&scheduler, &[frame]);
    }

    #[test]
    fn failure_releases_every_dependent_row() {
        let scheduler = RowScheduler::new(threads(2)).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut first = Chain {
            id: 0,
            stage: Stage::new(3),
            gate: None,
            need: |_| 0,
            done_rows: scheduler.progress(),
            log: Arc::clone(&log),
            fail_at: Some(1),
        };
        first.fail_at = Some(1);
        let first = Arc::new(first);
        let second = chain(
            &scheduler,
            1,
            3,
            Some(first.done_rows.clone()),
            |row| row + 1,
            &log,
        );
        scheduler.push(Arc::clone(&first) as Arc<dyn FrameWork>);
        scheduler.push(Arc::clone(&second) as Arc<dyn FrameWork>);
        wait_done(&scheduler, &[first.clone(), second.clone()]);
        assert_eq!(first.done_rows.current(), RowProgress::FAILED);
        assert_eq!(
            log.lock().iter().filter(|(f, _)| *f == 1).count(),
            3,
            "dependent rows ran after the failure"
        );
    }

    #[test]
    fn drop_stops_workers_that_wait_on_rows_nobody_publishes() {
        let scheduler = RowScheduler::new(threads(3)).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let never = scheduler.progress();
        let frame = chain(&scheduler, 0, 2, Some(never), |_| 1, &log);
        scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
        std::thread::sleep(Duration::from_millis(10));
        let started = Instant::now();
        drop(scheduler);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!frame.done());
    }

    #[test]
    fn wavefront_of_many_frames_completes_without_deadlock() {
        const FRAMES: usize = 12;
        const ROWS: usize = 40;
        let scheduler = RowScheduler::new(threads(4)).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut frames: Vec<Arc<Chain>> = Vec::new();
        for id in 0..FRAMES {
            let gate = frames.last().map(|previous| previous.done_rows.clone());
            let frame = chain(&scheduler, id, ROWS, gate, |row| (row + 2).min(ROWS), &log);
            scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
            frames.push(frame);
        }
        wait_done(&scheduler, &frames);
        assert_eq!(log.lock().len(), FRAMES * ROWS);
        let stop = AtomicBool::new(true);
        assert!(stop.load(Ordering::Relaxed));
    }
}
