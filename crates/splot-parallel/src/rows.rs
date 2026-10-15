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
//! A [`RowProgress::publish`] takes the scheduler lock to signal, so a worker
//! that scanned the counters before the publication is woken after it.

use core::num::NonZeroUsize;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    rows: usize,
}

impl Stage {
    /// A stage of `rows` rows, none run yet.
    #[must_use]
    pub const fn new(rows: usize) -> Self {
        Self {
            next: AtomicUsize::new(0),
            running: AtomicBool::new(false),
            rows,
        }
    }

    /// Resets the cursor for a new frame of `rows` rows.
    pub fn reset(&mut self, rows: usize) {
        *self.next.get_mut() = 0;
        *self.running.get_mut() = false;
        self.rows = rows;
    }

    /// Claims the next row when no row is running and `ready(row)` holds.
    pub fn claim(&self, ready: impl FnOnce(usize) -> bool) -> Option<usize> {
        if self.running.load(Ordering::Acquire) {
            return None;
        }
        let row = self.next.load(Ordering::Acquire);
        if row >= self.rows || !ready(row) {
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

    /// The next row to run.
    #[must_use]
    pub fn next(&self) -> usize {
        self.next.load(Ordering::Acquire)
    }

    /// The rows this stage runs.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Whether every row has run.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        !self.running.load(Ordering::Acquire) && self.next() >= self.rows
    }
}

#[derive(Default)]
struct Frames {
    live: VecDeque<Arc<dyn FrameWork>>,
    shutdown: bool,
}

#[derive(Default)]
struct Core {
    frames: Mutex<Frames>,
    changed: Condvar,
}

impl core::fmt::Debug for Core {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Core")
    }
}

impl Core {
    /// Signals after a publication. Taking the lock orders the signal after
    /// any scan that missed the publication, so that scanner wakes.
    fn notify(&self, all: bool) {
        let frames = self.frames.lock();
        if all {
            self.changed.notify_all();
        } else {
            self.changed.notify_one();
        }
        drop(frames);
    }

    fn worker(&self) {
        let mut frames = self.frames.lock();
        loop {
            let claimed = frames
                .live
                .iter()
                .find_map(|frame| frame.claim().map(|claim| (Arc::clone(frame), claim)));
            let Some((frame, claim)) = claimed else {
                if frames.shutdown {
                    return;
                }
                self.changed.wait(&mut frames);
                continue;
            };
            self.changed.notify_one();
            drop(frames);
            frame.run(claim);
            frames = self.frames.lock();
            let mut finished = false;
            while frames.live.front().is_some_and(|front| front.done()) {
                frames.live.pop_front();
                finished = true;
            }
            if finished {
                self.changed.notify_all();
            }
        }
    }
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
        let mut workers = Vec::with_capacity(threads.get());
        for index in 0..threads.get() {
            let core = Arc::clone(&core);
            let worker = std::thread::Builder::new()
                .name(format!("splot-row-{index}"))
                .stack_size(WORKER_STACK_SIZE_BYTES)
                .spawn(move || core.worker())
                .map_err(|error| ParallelError::ThreadSpawn {
                    message: error.to_string(),
                })?;
            workers.push(worker);
        }
        Ok(Self { core, workers })
    }

    /// A row counter whose publications wake this scheduler.
    #[must_use]
    pub fn progress(&self) -> RowProgress {
        RowProgress {
            value: Arc::new(AtomicUsize::new(0)),
            core: Arc::clone(&self.core),
        }
    }

    /// Adds the newest frame. Its rows run after every older frame's ready rows.
    pub fn push(&self, frame: Arc<dyn FrameWork>) {
        let mut frames = self.core.frames.lock();
        frames.live.push_back(frame);
        self.core.changed.notify_one();
        drop(frames);
    }

    /// Frames pushed and not yet done.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.core.frames.lock().live.len()
    }

    /// Blocks the driver until `ready()` holds. `ready` runs under the
    /// scheduler lock and is re-checked after every publication.
    pub fn wait(&self, mut ready: impl FnMut() -> bool) {
        let mut frames = self.core.frames.lock();
        while !ready() {
            self.core.changed.wait(&mut frames);
        }
        drop(frames);
    }

    /// The number of worker threads.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.workers.len()
    }
}

impl Drop for RowScheduler {
    fn drop(&mut self) {
        {
            let mut frames = self.core.frames.lock();
            frames.shutdown = true;
            self.core.changed.notify_all();
        }
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
        assert_eq!(scheduler.in_flight(), 0);
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
