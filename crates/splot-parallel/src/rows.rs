// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Row-ordered frame scheduling on dedicated worker threads.
//!
//! A [`FrameWork`] is one in-flight frame. Its stages (parse, resolve,
//! reconstruct, filter) each advance one superblock row at a time, in order.
//! Workers scan a snapshot of the frames oldest first, without a shared
//! lock, and ask each frame to [`claim`](FrameWork::claim) a row whose
//! inputs are published; a claim is an atomic exchange on the stage, so two
//! scanners never run the same row. Readiness is read from [`RowProgress`]
//! counters, so nothing registers a waiter and a worker never blocks inside a
//! row. A frame depends only on older frames, and older frames are claimed
//! first, so the oldest frame always has a row that can run or is running:
//! the pipeline cannot stall.
//!
//! A publication raises a generation counter before it signals, and a worker
//! reads that counter before it scans, so a scan that missed the publication
//! never parks on it.

use core::num::NonZeroUsize;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering, fence};
use std::sync::{Arc, OnceLock};
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
    /// frame can run now. Several workers call it concurrently, so it must
    /// only read atomics and claim through [`Stage`] or [`ParallelStage`].
    fn claim(&self) -> Option<Claim>;
    /// Picks a row on the decode's serial critical path, which workers try in
    /// every frame before any [`FrameWork::claim`]: a frame's entropy pass
    /// gates the next frame's, so it must not queue behind older frames'
    /// parallel rows.
    fn claim_critical(&self) -> Option<Claim> {
        None
    }
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

/// The ordered row cursor of one stage: at most one row runs at a time, and
/// concurrent claimers race on the running flag.
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
        if self
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        if self.next.load(Ordering::Acquire) != row {
            self.running.store(false, Ordering::Release);
            return None;
        }
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

/// A stage whose rows are independent: any ready row runs, several at a
/// time.
///
/// The row list is published once per frame and only read after that, a
/// row changes state by an atomic exchange, and the counts of pending, ready
/// and unfinished rows let a scan of a stage with nothing to hand out return
/// early.
#[derive(Debug, Default)]
pub struct ParallelStage {
    rows: OnceLock<Vec<AtomicU8>>,
    /// The last frame's row list, kept for its allocation.
    spare: Mutex<Vec<AtomicU8>>,
    pending: AtomicUsize,
    ready: AtomicUsize,
    unfinished: AtomicUsize,
}

impl ParallelStage {
    /// A stage with no rows yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rows: OnceLock::new(),
            spare: Mutex::new(Vec::new()),
            pending: AtomicUsize::new(0),
            ready: AtomicUsize::new(0),
            unfinished: AtomicUsize::new(0),
        }
    }

    /// Sets the row count once the frame's geometry is known. Every row
    /// starts pending. Only the first call after [`Self::reset`] counts.
    pub fn set_rows(&self, rows: usize) {
        if self.rows.get().is_some() {
            return;
        }
        let mut states = core::mem::take(&mut *self.spare.lock());
        states.clear();
        states.resize_with(rows, || AtomicU8::new(PENDING));
        self.pending.store(rows, Ordering::Release);
        self.ready.store(0, Ordering::Release);
        self.unfinished.store(rows, Ordering::Release);
        let _ = self.rows.set(states);
    }

    /// Clears the rows for a new frame.
    pub fn reset(&mut self) {
        if let Some(states) = self.rows.take() {
            *self.spare.get_mut() = states;
        }
        *self.pending.get_mut() = 0;
        *self.ready.get_mut() = 0;
        *self.unfinished.get_mut() = 0;
    }

    fn states(&self) -> &[AtomicU8] {
        self.rows.get().map_or(&[], Vec::as_slice)
    }

    /// Claims the first pending row when `ready(row)` holds. Rows are
    /// admitted in order: a later row never runs ahead of an earlier pending
    /// one, so a scan evaluates one predicate per stage.
    pub fn claim(&self, ready: impl Fn(usize) -> bool) -> Option<usize> {
        if self.pending.load(Ordering::Acquire) == 0 {
            return None;
        }
        let states = self.states();
        let mut from = 0;
        loop {
            let row = from + first_in(&states[from..], PENDING)?;
            if !ready(row) {
                return None;
            }
            if exchange(&states[row], PENDING, RUNNING) {
                self.pending.fetch_sub(1, Ordering::AcqRel);
                return Some(row);
            }
            from = row + 1;
        }
    }

    /// Marks `row` as runnable, for a producer that knows when its input lands.
    pub fn ready(&self, row: usize) {
        if let Some(state) = self.states().get(row)
            && exchange(state, PENDING, READY)
        {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            self.ready.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Claims the first row a producer marked [`ready`](Self::ready).
    pub fn claim_ready(&self) -> Option<usize> {
        if self.ready.load(Ordering::Acquire) == 0 {
            return None;
        }
        let states = self.states();
        let mut from = 0;
        loop {
            let row = from + first_in(&states[from..], READY)?;
            if exchange(&states[row], READY, RUNNING) {
                self.ready.fetch_sub(1, Ordering::AcqRel);
                return Some(row);
            }
            from = row + 1;
        }
    }

    /// Marks the claimed `row` as run.
    pub fn finish(&self, row: usize) {
        if let Some(state) = self.states().get(row)
            && state.swap(DONE, Ordering::AcqRel) != DONE
        {
            self.unfinished.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// Whether `row` has run.
    #[must_use]
    pub fn is_done(&self, row: usize) -> bool {
        self.states()
            .get(row)
            .is_some_and(|state| state.load(Ordering::Acquire) == DONE)
    }

    /// Whether every row has run.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.unfinished.load(Ordering::Acquire) == 0
    }
}

fn first_in(states: &[AtomicU8], wanted: u8) -> Option<usize> {
    states
        .iter()
        .position(|state| state.load(Ordering::Acquire) == wanted)
}

fn exchange(state: &AtomicU8, from: u8, to: u8) -> bool {
    state
        .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// A value on its own 128-byte line, so a write to a neighbour does not
/// evict it from every other core's cache.
#[derive(Default)]
#[repr(align(128))]
struct Line<T>(T);

type FrameList = Arc<Vec<Arc<dyn FrameWork>>>;

#[derive(Default)]
struct Frames {
    live: FrameList,
    /// An emptied earlier list, reused so a change does not allocate.
    spare: Option<FrameList>,
}

/// One worker's copy of the frame list, valid while `epoch` is current.
#[derive(Default)]
struct Snapshot {
    epoch: u64,
    frames: Option<FrameList>,
}

/// A worker's snapshot, and the flag a list change raises when a scan
/// held the snapshot, so the scanner checks it once the scan ends.
#[derive(Default)]
struct Shot {
    snapshot: Mutex<Snapshot>,
    stale: AtomicBool,
}

/// The scheduler's shared state and its wake-up event.
///
/// A worker reads `generation` before it scans the frames. A publisher raises
/// `generation` and signals only when a waiter is registered, so a scan that
/// missed the publication sees the raised generation and does not park. Every
/// worker raises the generation after it drops its handle on a frame, so a
/// driver waiting for the frame's sole owner sees the last drop whichever
/// worker made it and whichever worker popped the finished frame.
///
/// A scan reads the worker's own snapshot of the frame list, so it takes no
/// shared lock. Push and pop replace the list and raise `epoch` under
/// `frames`, then drop every snapshot of an older epoch without waiting: a
/// snapshot held by a scan is flagged stale, and the scanner drops it right
/// after the scan. A removed frame is thus held by no snapshot once every
/// scan running at the change has ended, whatever job its worker runs next.
struct Core {
    frames: Line<Mutex<Frames>>,
    epoch: Line<AtomicU64>,
    shutdown: Line<AtomicBool>,
    snapshots: Box<[Line<Shot>]>,
    generation: Line<AtomicU64>,
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

thread_local! {
    /// The scheduler this thread works for: a row worker's own, or the one
    /// its driver last pushed to or waited on.
    static BOUND: RefCell<Option<Arc<Core>>> = const { RefCell::new(None) };
}

/// Wakes the scheduler the calling thread works for, after a publication
/// through a primitive that holds no [`RowWake`].
pub(crate) fn wake_bound() {
    BOUND.with_borrow(|core| {
        if let Some(core) = core {
            core.notify(false);
        }
    });
}

fn bind(core: &Arc<Core>) {
    BOUND.with_borrow_mut(|bound| {
        if !bound.as_ref().is_some_and(|held| Arc::ptr_eq(held, core)) {
            *bound = Some(Arc::clone(core));
        }
    });
}

/// A handle that makes [`RowProgress`] counters for one scheduler.
#[derive(Clone, Debug)]
pub struct RowWake(Arc<Core>);

impl RowWake {
    /// A wake with no workers, for a decode that runs on one thread.
    #[must_use]
    pub fn detached() -> Self {
        Self(Arc::new(Core::new(0)))
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

    #[cfg(test)]
    fn parked_workers(&self) -> usize {
        self.0.parked.load(Ordering::Relaxed)
    }
}

impl core::fmt::Debug for Core {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Core")
    }
}

impl Core {
    fn new(workers: usize) -> Self {
        Self {
            frames: Line::default(),
            epoch: Line::default(),
            shutdown: Line::default(),
            snapshots: (0..workers).map(|_| Line::default()).collect(),
            generation: Line::default(),
            waiters: AtomicUsize::new(0),
            parked: AtomicUsize::new(0),
            signaled: AtomicBool::new(false),
            park: Mutex::new(()),
            changed: Condvar::new(),
            driver_parked: AtomicBool::new(false),
            driver_changed: Condvar::new(),
        }
    }

    /// Replaces the frame list with `edit` applied to it, unless `edit`
    /// leaves its length unchanged, and drops every stale snapshot.
    fn change(&self, edit: impl FnOnce(&mut Vec<Arc<dyn FrameWork>>)) {
        let mut frames = self.frames.0.lock();
        let mut next = frames.spare.take().unwrap_or_default();
        if Arc::get_mut(&mut next).is_none() {
            next = Arc::default();
        }
        let Some(list) = Arc::get_mut(&mut next) else {
            return;
        };
        list.clear();
        list.extend(frames.live.iter().cloned());
        edit(list);
        if list.len() == frames.live.len() {
            list.clear();
            frames.spare = Some(next);
            return;
        }
        let mut old = core::mem::replace(&mut frames.live, next);
        self.epoch.0.fetch_add(1, Ordering::SeqCst);
        drop(frames);
        for slot in &self.snapshots {
            slot.0.stale.store(true, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            if let Some(mut snapshot) = slot.0.snapshot.try_lock() {
                let _ = self.drop_stale(&mut snapshot);
            }
        }
        if let Some(list) = Arc::get_mut(&mut old) {
            list.clear();
            self.frames.0.lock().spare = Some(old);
        }
    }

    /// Drops `snapshot` when the list changed since it was taken; whether
    /// that dropped the last handle on the old list, which may release a
    /// finished frame the driver waits for.
    fn drop_stale(&self, snapshot: &mut Snapshot) -> bool {
        snapshot.epoch != self.epoch.0.load(Ordering::SeqCst)
            && snapshot.frames.take().and_then(Arc::into_inner).is_some()
    }

    /// Claims a row from `slot`'s snapshot, critical rows of every frame
    /// first, refreshing the snapshot when the list changed.
    fn scan(&self, slot: &Shot) -> Option<(Arc<dyn FrameWork>, Claim)> {
        let mut snapshot = slot.snapshot.lock();
        if snapshot.frames.is_none() || snapshot.epoch != self.epoch.0.load(Ordering::Acquire) {
            let frames = self.frames.0.lock();
            snapshot.frames = Some(Arc::clone(&frames.live));
            snapshot.epoch = self.epoch.0.load(Ordering::Acquire);
        }
        let claimed = snapshot.frames.as_deref().and_then(|live| {
            live.iter()
                .find_map(|frame| frame.claim_critical().map(|claim| (frame, claim)))
                .or_else(|| {
                    live.iter()
                        .find_map(|frame| frame.claim().map(|claim| (frame, claim)))
                })
                .map(|(frame, claim)| (Arc::clone(frame), claim))
        });
        drop(snapshot);
        fence(Ordering::SeqCst);
        if slot.stale.load(Ordering::SeqCst) {
            slot.stale.store(false, Ordering::SeqCst);
            if self.drop_stale(&mut slot.snapshot.lock()) {
                self.notify(false);
            }
        }
        claimed
    }

    /// Raises the generation and signals one waiter, or all of them.
    ///
    /// A single-worker signal is skipped while one is already pending; a
    /// worker that found work signals through [`Self::pass_on`] instead, so
    /// the next idle worker still looks for more.
    fn notify(&self, all: bool) {
        self.generation.0.fetch_add(1, Ordering::SeqCst);
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

    /// Signals one more idle worker after a woken worker claims, whether or
    /// not a signal is pending, so a burst of ready rows spreads across the
    /// parked workers. A busy worker's claim does not pass on: it was about
    /// to scan anyway, and the parked worker it would wake usually finds the
    /// row already taken by the next busy worker to finish.
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
        if self.generation.0.load(Ordering::SeqCst) == observed {
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
        if self.generation.0.load(Ordering::SeqCst) == observed {
            self.parked.fetch_add(1, Ordering::Relaxed);
            self.changed.wait(&mut park);
            self.parked.fetch_sub(1, Ordering::Relaxed);
        }
        self.waiters.fetch_sub(1, Ordering::SeqCst);
        drop(park);
    }

    fn worker(&self, index: usize) {
        let Some(slot) = self.snapshots.get(index) else {
            return;
        };
        let mut woken = false;
        loop {
            let observed = self.generation.0.load(Ordering::SeqCst);
            let Some((frame, claim)) = self.scan(&slot.0) else {
                if self.shutdown.0.load(Ordering::SeqCst) {
                    return;
                }
                self.wait_if_unchanged(observed);
                woken = true;
                continue;
            };
            if woken {
                woken = false;
                self.pass_on();
            }
            frame.run(claim);
            let done = frame.done();
            drop(frame);
            if done {
                self.change(|live| {
                    let finished = live.iter().take_while(|frame| frame.done()).count();
                    live.drain(..finished);
                });
            }
            self.notify(done);
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
        let core = Arc::new(Core::new(threads.get()));
        let mut workers = Vec::with_capacity(threads.get());
        for index in 0..threads.get() {
            let core = Arc::clone(&core);
            workers.push(spawn_thread(&format!("splot-row-{index}"), move || {
                bind(&core);
                core.worker(index);
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
        bind(&self.core);
        self.core.change(|live| live.push(frame));
        self.core.notify(false);
    }

    /// Workers parked with nothing to run, for tests.
    #[cfg(test)]
    pub(crate) fn parked_workers(&self) -> usize {
        self.wake().parked_workers()
    }

    /// Frames pushed and not yet done.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.core.frames.0.lock().live.len()
    }

    /// Blocks the driver until `ready()` holds. `ready` is re-checked after
    /// every publication and every worker's drop of a frame handle.
    pub fn wait(&self, mut ready: impl FnMut() -> bool) {
        bind(&self.core);
        loop {
            let observed = self.core.generation.0.load(Ordering::SeqCst);
            if ready() {
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
        self.core.shutdown.0.store(true, Ordering::SeqCst);
        self.core.notify(true);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        self.core.change(Vec::clear);
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
        /// Holds an unfinished `done` answer, which widens the gap between a
        /// worker's `done` read and its handle drop.
        slow_done: bool,
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
            let finished = self.stage.is_finished();
            if self.slow_done && !finished {
                std::thread::sleep(Duration::from_millis(1));
            }
            finished
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
            slow_done: false,
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
    fn a_parallel_stage_admits_ready_rows_in_order_and_finishes_when_all_ran() {
        let stage = ParallelStage::new();
        assert!(stage.is_finished(), "no rows is finished");
        stage.set_rows(3);
        assert!(!stage.is_finished());
        assert_eq!(stage.claim(|row| row != 0), None, "row 0 is not ready");
        assert_eq!(stage.claim(|_| true), Some(0));
        assert_eq!(
            stage.claim(|_| true),
            Some(1),
            "row 0 running does not block row 1"
        );
        stage.finish(1);
        assert_eq!(stage.claim(|row| row != 2), None, "row 2 is not ready");
        assert_eq!(stage.claim(|_| true), Some(2));
        stage.finish(2);
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

    /// A frame whose every scan is slow and finds nothing.
    struct SlowScan;

    impl FrameWork for SlowScan {
        fn claim(&self) -> Option<Claim> {
            let until = Instant::now() + Duration::from_millis(1);
            while Instant::now() < until {
                std::hint::spin_loop();
            }
            None
        }

        fn run(&self, _: Claim) {}

        fn done(&self) -> bool {
            false
        }
    }

    /// A frame with nothing to run.
    struct Finished;

    impl FrameWork for Finished {
        fn claim(&self) -> Option<Claim> {
            None
        }

        fn run(&self, _: Claim) {}

        fn done(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_push_completes_while_every_worker_scans_without_pause() {
        let scheduler = Arc::new(RowScheduler::new(threads(8)).unwrap());
        scheduler.push(Arc::new(SlowScan));
        let stop = Arc::new(AtomicBool::new(false));
        let notifier = {
            let (wake, stop) = (scheduler.wake(), Arc::clone(&stop));
            spawn_thread("notifier", move || {
                while !stop.load(Ordering::Relaxed) {
                    wake.notify();
                }
            })
            .unwrap()
        };
        let slowest_nanos = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let pusher = {
            let (scheduler, slowest_nanos) = (Arc::clone(&scheduler), Arc::clone(&slowest_nanos));
            spawn_thread("pusher", move || {
                for _ in 0..200 {
                    let started = Instant::now();
                    scheduler.push(Arc::new(Finished));
                    let nanos = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
                    slowest_nanos.fetch_max(nanos, Ordering::Relaxed);
                }
            })
            .unwrap()
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !pusher.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let finished = pusher.is_finished();
        stop.store(true, Ordering::Relaxed);
        notifier.join().unwrap();
        assert!(finished, "the driver's pushes starved behind scans");
        pusher.join().unwrap();
        let slowest = Duration::from_nanos(slowest_nanos.load(Ordering::Relaxed));
        assert!(
            slowest < Duration::from_millis(150),
            "a push waited {slowest:?}"
        );
    }

    /// A one-row frame whose row starts after `after` started and runs while
    /// `held` is set, for at most five seconds.
    struct Hold {
        stage: Stage,
        started: AtomicBool,
        held: AtomicBool,
        after: Option<Arc<Hold>>,
    }

    impl Hold {
        fn new(held: bool, after: Option<Arc<Hold>>) -> Arc<Self> {
            Arc::new(Self {
                stage: Stage::new(1),
                started: AtomicBool::new(false),
                held: AtomicBool::new(held),
                after,
            })
        }
    }

    impl FrameWork for Hold {
        fn claim(&self) -> Option<Claim> {
            self.stage
                .claim(|_| true)
                .map(|row| Claim { stage: 0, row })
        }

        fn run(&self, claim: Claim) {
            self.started.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(5);
            while (self.held.load(Ordering::SeqCst)
                || self
                    .after
                    .as_ref()
                    .is_some_and(|after| !after.started.load(Ordering::SeqCst)))
                && Instant::now() < deadline
            {
                std::thread::yield_now();
            }
            self.stage.finish(claim.row);
        }

        fn done(&self) -> bool {
            self.stage.is_finished()
        }
    }

    #[test]
    fn no_snapshot_keeps_a_finished_frame_alive_while_workers_run_or_park() {
        let scheduler = RowScheduler::new(threads(2)).unwrap();
        let long = Hold::new(true, None);
        let short = Hold::new(false, Some(Arc::clone(&long)));
        scheduler.push(Arc::clone(&short) as Arc<dyn FrameWork>);
        scheduler.push(Arc::clone(&long) as Arc<dyn FrameWork>);
        let rescue = scheduler.progress();
        let _watchdog = spawn_thread("watchdog", move || {
            std::thread::sleep(Duration::from_secs(3));
            rescue.publish(1);
        })
        .unwrap();
        let started = Instant::now();
        scheduler
            .wait(|| Arc::strong_count(&short) == 1 || started.elapsed() > Duration::from_secs(3));
        assert_eq!(
            Arc::strong_count(&short),
            1,
            "the busy worker's snapshot kept the finished frame"
        );
        long.held.store(false, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(5);
        while (scheduler.parked_workers() < 2 || scheduler.in_flight() != 0)
            && Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert_eq!(scheduler.in_flight(), 0);
        drop(short);
        assert_eq!(
            Arc::strong_count(&long),
            1,
            "a parked worker's snapshot kept the finished frame"
        );
    }

    #[test]
    fn concurrent_claims_of_a_parallel_stage_hand_out_each_row_once() {
        const ROWS: usize = 64;
        let stage = ParallelStage::new();
        stage.set_rows(ROWS);
        for row in (0..ROWS).step_by(2) {
            stage.ready(row);
        }
        let claimed: Vec<usize> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        let mut mine = Vec::new();
                        while let Some(row) = stage.claim_ready().or_else(|| stage.claim(|_| true))
                        {
                            stage.finish(row);
                            mine.push(row);
                        }
                        mine
                    })
                })
                .collect();
            workers
                .into_iter()
                .flat_map(|w| w.join().unwrap())
                .collect()
        });
        let mut sorted = claimed;
        sorted.sort_unstable();
        assert_eq!(sorted, (0..ROWS).collect::<Vec<_>>());
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
    fn the_last_handle_drop_wakes_a_driver_waiting_for_the_sole_owner() {
        let scheduler = RowScheduler::new(threads(2)).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        for _ in 0..50 {
            let frame = Arc::new(Chain {
                slow_done: true,
                ..Arc::into_inner(chain(&scheduler, 0, 8, None, |_| 0, &log)).unwrap()
            });
            scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
            let rescue = scheduler.progress();
            let watchdog = spawn_thread("watchdog", move || {
                std::thread::sleep(Duration::from_secs(3));
                rescue.publish(1);
            })
            .unwrap();
            let started = Instant::now();
            scheduler.wait(|| Arc::strong_count(&frame) == 1);
            assert!(frame.done());
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "the driver waited for the watchdog"
            );
            drop(watchdog);
        }
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

    /// A one-row frame gated on a watermark that holds no [`RowWake`].
    struct OnCell {
        cell: Arc<crate::WatermarkCell>,
        stage: Stage,
    }

    impl FrameWork for OnCell {
        fn claim(&self) -> Option<Claim> {
            self.stage
                .claim(|_| self.cell.current() != 0)
                .map(|row| Claim { stage: 0, row })
        }

        fn run(&self, claim: Claim) {
            self.stage.finish(claim.row);
        }

        fn done(&self) -> bool {
            self.stage.is_finished()
        }
    }

    /// Waits for `frame`, failing when only a watchdog's wake after three
    /// seconds would end the wait.
    fn wait_without_rescue(scheduler: &RowScheduler, frame: &OnCell, what: &str) {
        let rescue = scheduler.progress();
        let _watchdog = spawn_thread("watchdog", move || {
            std::thread::sleep(Duration::from_secs(3));
            rescue.publish(1);
        })
        .unwrap();
        let started = Instant::now();
        scheduler.wait(|| frame.done() || started.elapsed() > Duration::from_secs(3));
        assert!(
            frame.done() && started.elapsed() < Duration::from_secs(2),
            "a {what} publication was lost"
        );
    }

    /// Publishes `gate` once the other worker parked, then holds its worker
    /// until `waiter` ran, so only the publication can wake it.
    struct Publish(Arc<OnCell>, Arc<crate::WatermarkCell>, Stage, RowWake);

    impl FrameWork for Publish {
        fn claim(&self) -> Option<Claim> {
            self.2.claim(|_| true).map(|row| Claim { stage: 0, row })
        }

        fn run(&self, claim: Claim) {
            let deadline = Instant::now() + Duration::from_secs(5);
            for _ in 0..2 {
                std::thread::sleep(Duration::from_millis(20));
                while self.3.parked_workers() == 0 && Instant::now() < deadline {
                    std::thread::yield_now();
                }
            }
            self.1.publish(1);
            while !self.0.done() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            self.2.finish(claim.row);
        }

        fn done(&self) -> bool {
            self.2.is_finished()
        }
    }

    #[test]
    fn a_cell_published_on_the_driver_or_a_worker_wakes_its_scheduler() {
        let scheduler = RowScheduler::new(threads(2)).unwrap();
        let cell = Arc::new(crate::WatermarkCell::new());
        let waiter = Arc::new(OnCell {
            cell: Arc::clone(&cell),
            stage: Stage::new(1),
        });
        scheduler.push(Arc::clone(&waiter) as Arc<dyn FrameWork>);
        std::thread::sleep(Duration::from_millis(20));
        cell.publish(1);
        wait_without_rescue(&scheduler, &waiter, "driver");
        let gate = Arc::new(crate::WatermarkCell::new());
        let waiter = Arc::new(OnCell {
            cell: Arc::clone(&gate),
            stage: Stage::new(1),
        });
        scheduler.push(Arc::clone(&waiter) as Arc<dyn FrameWork>);
        scheduler.push(Arc::new(Publish(
            Arc::clone(&waiter),
            gate,
            Stage::new(1),
            scheduler.wake(),
        )));
        wait_without_rescue(&scheduler, &waiter, "worker");
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
            slow_done: false,
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
