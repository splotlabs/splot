// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Row-ordered frame scheduling on dedicated worker threads.
//!
//! A [`FrameWork`] is one in-flight frame. Its stages (parse, resolve,
//! reconstruct, filter) each advance one superblock row at a time, in order.
//! Workers scan a snapshot of the frames oldest first, without a shared
//! lock, and ask each frame to [`claim`](FrameWork::claim) a row whose
//! inputs are published; a claim is an atomic exchange on the stage, so two
//! scanners never run the same row. Readiness is read from published atomics
//! such as a [`WatermarkCell`](crate::WatermarkCell), so nothing registers a
//! waiter and a worker never blocks inside a row. A frame depends only on
//! older frames, and older frames are claimed first, so the oldest frame
//! always has a row that can run or is running: the pipeline cannot stall.
//!
//! A wake is sent only for a row that exists: a worker that publishes
//! progress claims the row first and queues it, and whichever worker is free
//! first runs it. A driver's push or publication only wakes one worker.

use core::num::NonZeroUsize;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering, fence};
use std::sync::{Arc, OnceLock};
use std::thread::{JoinHandle, Thread};

use parking_lot::{Condvar, Mutex};

use crate::error::ParallelError;

const WORKER_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
const IDLE: u64 = 0xffff_ffff;

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
    /// [`Stage::finish`] for it. Its worker hands out what the row released
    /// after it takes its own next row. `worker` is the index of the running
    /// worker, below [`RowScheduler::threads`], so a frame can keep state that
    /// only that worker uses.
    fn run(&self, claim: Claim, worker: usize);
    /// Whether every stage has finished, including after a failure.
    fn done(&self) -> bool;
}

impl<W: FrameWork + ?Sized> FrameWork for Arc<W> {
    fn claim(&self) -> Option<Claim> {
        (**self).claim()
    }

    fn claim_critical(&self) -> Option<Claim> {
        (**self).claim_critical()
    }

    fn run(&self, claim: Claim, worker: usize) {
        (**self).run(claim, worker);
    }

    fn done(&self) -> bool {
        (**self).done()
    }
}

/// The ordered row cursor of one stage: at most one row runs at a time.
///
/// The next row and its running bit share one atomic, so the bit is set only
/// by the claim that wins the row: a scan that sees it set knows a worker
/// runs that row and scans again after it.
#[derive(Debug)]
pub struct Stage {
    /// The next row, shifted left by one, plus [`Self::RUNNING`].
    state: AtomicUsize,
    rows: AtomicUsize,
}

impl Stage {
    const RUNNING: usize = 1;

    /// A stage of `rows` rows, none run yet.
    #[must_use]
    pub const fn new(rows: usize) -> Self {
        Self {
            state: AtomicUsize::new(0),
            rows: AtomicUsize::new(rows),
        }
    }

    /// Resets the cursor for a new frame of `rows` rows.
    pub fn reset(&mut self, rows: usize) {
        *self.state.get_mut() = 0;
        *self.rows.get_mut() = rows;
    }

    /// Sets the row count once the frame's geometry is known, before any
    /// row of this stage is claimed.
    pub fn set_rows(&self, rows: usize) {
        self.rows.store(rows, Ordering::Release);
    }

    /// Claims the next row when no row is running and `ready(row)` holds.
    pub fn claim(&self, ready: impl FnOnce(usize) -> bool) -> Option<usize> {
        let state = self.state.load(Ordering::Acquire);
        let row = state >> 1;
        if state & Self::RUNNING != 0 || row >= self.rows() || !ready(row) {
            return None;
        }
        self.state
            .compare_exchange(
                state,
                state | Self::RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| row)
    }

    /// Marks the claimed `row` as run, so the next row can be claimed.
    pub fn finish(&self, row: usize) {
        self.state
            .store(row.saturating_add(1) << 1, Ordering::Release);
    }

    /// Releases the claimed `row` unrun, so it can be claimed again once its
    /// inputs are published.
    pub fn retry(&self, row: usize) {
        self.state.store(row << 1, Ordering::Release);
    }

    /// The next row to run.
    #[must_use]
    pub fn next(&self) -> usize {
        self.state.load(Ordering::Acquire) >> 1
    }

    /// The rows this stage runs.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows.load(Ordering::Acquire)
    }

    /// Whether every row has run.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);
        state & Self::RUNNING == 0 && state >> 1 >= self.rows()
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

type FrameList<W> = Arc<Vec<W>>;

struct Frames<W> {
    live: FrameList<W>,
    /// An emptied earlier list, reused so a change does not allocate.
    spare: Option<FrameList<W>>,
}

impl<W> Default for Frames<W> {
    fn default() -> Self {
        Self {
            live: FrameList::default(),
            spare: None,
        }
    }
}

/// One worker's copy of the frame list, valid while `epoch` is current.
struct Snapshot<W> {
    epoch: u64,
    frames: Option<FrameList<W>>,
}

impl<W> Default for Snapshot<W> {
    fn default() -> Self {
        Self {
            epoch: 0,
            frames: None,
        }
    }
}

/// A worker's snapshot, and the flag a list change raises when a scan
/// held the snapshot, so the scanner checks it once the scan ends.
struct Shot<W> {
    snapshot: Mutex<Snapshot<W>>,
    stale: AtomicBool,
}

impl<W> Default for Shot<W> {
    fn default() -> Self {
        Self {
            snapshot: Mutex::default(),
            stale: AtomicBool::new(false),
        }
    }
}

type Job<W> = (W, Claim);

/// One worker's snapshot and thread.
struct Slot<W> {
    shot: Line<Shot<W>>,
    thread: OnceLock<Thread>,
}

impl<W> Default for Slot<W> {
    fn default() -> Self {
        Self {
            shot: Line(Shot::default()),
            thread: OnceLock::new(),
        }
    }
}

/// The driver's own wait, apart from the workers' idle bits.
#[derive(Default)]
struct Driver {
    parked: AtomicBool,
    lock: Mutex<()>,
    changed: Condvar,
}

/// Non-generic atomic, idle-bit, and driver-wait state shared across scheduler
/// specializations.
struct WaitState {
    epoch: Line<AtomicU64>,
    shutdown: Line<AtomicBool>,
    queued: Line<AtomicUsize>,
    /// Bit `i % 32` of word `i / 32` is set while worker `i` is idle, and
    /// bit `32 + i % 32` from when a waker clears that bit until it runs.
    idle: Box<[Line<AtomicU64>]>,
    driver: Line<Driver>,
}

impl WaitState {
    fn new(workers: usize) -> Self {
        Self {
            epoch: Line::default(),
            shutdown: Line::default(),
            queued: Line::default(),
            idle: (0..workers.div_ceil(32)).map(|_| Line::default()).collect(),
            driver: Line::default(),
        }
    }

    fn set_idle(&self, worker: usize) {
        if let Some(word) = self.idle.get(worker / 32) {
            word.0.fetch_or(1 << (worker % 32), Ordering::SeqCst);
        }
    }

    /// Clears `worker`'s idle bit and sets its waking bit; whether it was
    /// idle.
    fn take_idle(&self, worker: usize) -> bool {
        let bit = 1u64 << (worker % 32);
        self.idle.get(worker / 32).is_some_and(|word| {
            word.0
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |bits| {
                    (bits & bit != 0).then_some(bits & !bit | bit << 32)
                })
                .is_ok()
        })
    }

    /// Clears `worker`'s idle bit itself; whether a waker took it first.
    fn leave_idle(&self, worker: usize) -> bool {
        let bit = 1u64 << (worker % 32);
        self.idle
            .get(worker / 32)
            .is_some_and(|word| word.0.fetch_and(!bit, Ordering::SeqCst) & bit == 0)
    }

    /// Clears `worker`'s waking bit once it runs.
    fn awake(&self, worker: usize) {
        if let Some(word) = self.idle.get(worker / 32) {
            word.0
                .fetch_and(!(1u64 << (32 + worker % 32)), Ordering::SeqCst);
        }
    }

    fn is_idle(&self, worker: usize) -> bool {
        self.idle
            .get(worker / 32)
            .is_some_and(|word| word.0.load(Ordering::SeqCst) & (1 << (worker % 32)) != 0)
    }

    /// The first idle worker, or `None` when none is idle or one woken
    /// worker has not run yet.
    fn first_idle(&self) -> Option<usize> {
        let mut first = None;
        for (at, word) in self.idle.iter().enumerate() {
            let bits = word.0.load(Ordering::SeqCst);
            if bits & !IDLE != 0 {
                return None;
            }
            if first.is_none() && bits != 0 {
                first = Some(at * 32 + bits.trailing_zeros() as usize);
            }
        }
        first
    }

    fn wake_driver(&self) {
        let driver = &self.driver.0;
        if driver.parked.load(Ordering::SeqCst) {
            let lock = driver.lock.lock();
            driver.changed.notify_all();
            drop(lock);
        }
    }

    fn wait(&self, ready: &mut dyn FnMut() -> bool) {
        let driver = &self.driver.0;
        while !ready() {
            let mut lock = driver.lock.lock();
            driver.parked.store(true, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            if !ready() {
                driver.changed.wait(&mut lock);
            }
            driver.parked.store(false, Ordering::SeqCst);
            drop(lock);
        }
    }

    #[cfg(test)]
    fn parked_workers(&self) -> usize {
        self.idle
            .iter()
            .map(|word| (word.0.load(Ordering::SeqCst) & IDLE).count_ones() as usize)
            .sum()
    }
}

/// The scheduler's shared state.
///
/// A scan reads the scanning thread's own snapshot of the frame list, so it
/// takes no shared lock. Push and pop replace the list and raise `epoch`
/// under `frames`, then drop every snapshot of an older epoch without
/// waiting: a snapshot held by a scan is flagged stale, and the scanner
/// drops it right after the scan; an idle owner is woken to scan. A removed
/// frame is thus held by no snapshot once every scan running at the change
/// has ended, and a scan that drops the last handle on a list wakes the
/// driver.
///
/// A worker with nothing to run sets its `idle` bit, fences, looks once more,
/// and parks until a dispatcher clears the bit. A thread that publishes
/// progress or pushes a frame calls [`Self::dispatch`]: after a fence it
/// reads the idle bits, and when one is set a worker claims a row into
/// `ready` (a driver claims none), then clears one bit and unparks that
/// worker. Both sides write first, fence, then read, so either the
/// dispatcher sees the bit or the worker's last look sees the publication:
/// no wake is lost, and a wake is sent only for a row that exists. The same
/// atomic step that clears a worker's idle bit sets its waking bit, which
/// the worker clears once it runs; dispatchers skip while a bit is set, as
/// the worker looks after it clears its bit and dispatches the next. Every
/// worker takes `ready` rows before it scans, so a row a busy worker can
/// start sooner does not wait for the woken one to get a core.
struct Core<W> {
    frames: Line<Mutex<Frames<W>>>,
    slots: Box<[Slot<W>]>,
    ready: Line<Mutex<VecDeque<Job<W>>>>,
    wait: WaitState,
}

trait WakeCore: Send + Sync {
    fn dispatch(&self, from: Option<usize>);
    fn dispatch_here(&self);
    fn core_addr(&self) -> usize;
    #[cfg(test)]
    fn parked_workers(&self) -> usize;
}

impl<W: FrameWork + Clone + 'static> WakeCore for Core<W> {
    fn dispatch(&self, from: Option<usize>) {
        Core::dispatch(self, from);
    }

    fn dispatch_here(&self) {
        Core::dispatch_here(self);
    }

    fn core_addr(&self) -> usize {
        core::ptr::from_ref(self).addr()
    }

    #[cfg(test)]
    fn parked_workers(&self) -> usize {
        self.wait.parked_workers()
    }
}

type BoundWake = Option<(Arc<dyn WakeCore>, Option<usize>)>;

thread_local! {
    /// The scheduler this thread works for and its worker index there: a
    /// row worker's own, or the one its driver last pushed to or waited on.
    static BOUND: RefCell<BoundWake> = const { RefCell::new(None) };
}

/// Hands a row to an idle worker of the scheduler the calling thread works
/// for, after a publication through a primitive that holds no [`RowWake`].
pub(crate) fn wake_bound() {
    BOUND.with_borrow(|bound| {
        if let Some((core, slot)) = bound {
            core.dispatch(*slot);
        }
    });
}

#[inline(never)]
fn bind<W: FrameWork + Clone + 'static>(core: &Arc<Core<W>>, slot: Option<usize>) {
    let addr = core::ptr::from_ref(&**core).addr();
    BOUND.with_borrow_mut(|bound| {
        if !bound
            .as_ref()
            .is_some_and(|(held, at)| held.core_addr() == addr && *at == slot)
        {
            *bound = Some((Arc::clone(core) as Arc<dyn WakeCore>, slot));
        }
    });
}

/// A handle that wakes one scheduler's workers and its driver.
#[derive(Clone)]
pub struct RowWake(Option<Arc<dyn WakeCore>>);

impl core::fmt::Debug for RowWake {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("RowWake").field(&self.0.is_some()).finish()
    }
}

impl RowWake {
    /// A wake with no workers, for a decode that runs on one thread.
    #[must_use]
    pub const fn detached() -> Self {
        Self(None)
    }

    /// Hands a row to an idle worker after a frame advanced a counter of
    /// its own.
    pub fn notify(&self) {
        if let Some(core) = &self.0 {
            core.dispatch_here();
        }
    }

    #[cfg(test)]
    fn parked_workers(&self) -> usize {
        self.0.as_ref().map_or(0, |core| core.parked_workers())
    }
}

impl<W> core::fmt::Debug for Core<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Core")
    }
}

impl<W: FrameWork + Clone> Core<W> {
    fn new(workers: usize) -> Self {
        Self {
            frames: Line(Mutex::new(Frames::default())),
            slots: (0..workers).map(|_| Slot::default()).collect(),
            ready: Line(Mutex::new(VecDeque::new())),
            wait: WaitState::new(workers),
        }
    }

    /// Replaces the frame list with `edit` applied to it, unless `edit`
    /// leaves its length unchanged, and drops every stale snapshot.
    fn change(&self, edit: impl FnOnce(&mut Vec<W>)) {
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
        self.wait.epoch.0.fetch_add(1, Ordering::SeqCst);
        drop(frames);
        for (index, slot) in self.slots.iter().enumerate() {
            slot.shot.0.stale.store(true, Ordering::SeqCst);
            fence(Ordering::SeqCst);
            if let Some(mut snapshot) = slot.shot.0.snapshot.try_lock() {
                let _ = self.drop_stale(&mut snapshot);
            } else if self.wait.take_idle(index)
                && let Some(thread) = slot.thread.get()
            {
                // The holder may be another change, so an idle owner must scan to drop it.
                thread.unpark();
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
    fn drop_stale(&self, snapshot: &mut Snapshot<W>) -> bool {
        snapshot.epoch != self.wait.epoch.0.load(Ordering::SeqCst)
            && snapshot.frames.take().and_then(Arc::into_inner).is_some()
    }

    /// Claims a row from `slot`'s snapshot, critical rows of every frame
    /// first, refreshing the snapshot when the list changed.
    fn scan(&self, slot: &Slot<W>) -> Option<Job<W>> {
        let shot = &slot.shot.0;
        let mut snapshot = shot.snapshot.lock();
        let mut released = self.drop_stale(&mut snapshot);
        if snapshot.frames.is_none() {
            let frames = self.frames.0.lock();
            snapshot.frames = Some(Arc::clone(&frames.live));
            snapshot.epoch = self.wait.epoch.0.load(Ordering::Acquire);
        }
        let claimed = snapshot.frames.as_deref().and_then(|live| {
            live.iter()
                .find_map(|frame| frame.claim_critical().map(|claim| (frame, claim)))
                .or_else(|| {
                    live.iter()
                        .find_map(|frame| frame.claim().map(|claim| (frame, claim)))
                })
                .map(|(frame, claim)| (frame.clone(), claim))
        });
        drop(snapshot);
        fence(Ordering::SeqCst);
        if shot.stale.load(Ordering::SeqCst) {
            shot.stale.store(false, Ordering::SeqCst);
            released |= self.drop_stale(&mut shot.snapshot.lock());
        }
        if released {
            fence(Ordering::SeqCst);
            self.wait.wake_driver();
        }
        claimed
    }

    /// Wakes the driver, and an idle worker when there is one: worker `from`
    /// first claims a row for it into `ready`, and finds none to claim or
    /// wakes nobody; a thread that is no worker claims nothing, so the
    /// driver never claims. A dispatcher wakes one worker: the woken worker
    /// dispatches the next before it runs its row.
    fn dispatch(&self, from: Option<usize>) {
        fence(Ordering::SeqCst);
        self.wait.wake_driver();
        if self.wait.first_idle().is_none() {
            return;
        }
        if let Some(slot) = from.and_then(|from| self.slots.get(from)) {
            let Some(job) = self.scan(slot) else {
                return;
            };
            self.ready.0.lock().push_back(job);
            self.wait.queued.0.fetch_add(1, Ordering::SeqCst);
        }
        while let Some(worker) = self.wait.first_idle() {
            if self.wait.take_idle(worker) {
                if let Some(thread) = self.slots.get(worker).and_then(|slot| slot.thread.get()) {
                    thread.unpark();
                }
                return;
            }
        }
    }

    fn take_ready(&self) -> Option<Job<W>> {
        if self.wait.queued.0.load(Ordering::SeqCst) == 0 {
            return None;
        }
        let job = self.ready.0.lock().pop_front()?;
        self.wait.queued.0.fetch_sub(1, Ordering::SeqCst);
        Some(job)
    }

    /// The next row for worker `index`: a claimed row from `ready`, a
    /// scanned one, or either once a dispatcher woke it from idle; `None`
    /// once the scheduler shuts down. A worker that got a row this way
    /// dispatches once more, so it keeps its own continuation and hands out
    /// the rest.
    fn next_job(&self, index: usize, slot: &Slot<W>) -> Option<Job<W>> {
        loop {
            if let Some(job) = self.take_ready().or_else(|| self.scan(slot)) {
                self.dispatch(Some(index));
                return Some(job);
            }
            self.wait.set_idle(index);
            fence(Ordering::SeqCst);
            if let Some(job) = self.take_ready().or_else(|| self.scan(slot)) {
                if self.wait.leave_idle(index) {
                    self.wait.awake(index);
                    self.dispatch(Some(index));
                }
                return Some(job);
            }
            while self.wait.is_idle(index) {
                if self.wait.shutdown.0.load(Ordering::SeqCst) {
                    return None;
                }
                std::thread::park();
            }
            self.wait.awake(index);
            fence(Ordering::SeqCst);
        }
    }

    #[inline(never)]
    fn start_worker(&self, index: usize) -> Option<&Slot<W>> {
        let slot = self.slots.get(index)?;
        let _ = slot.thread.set(std::thread::current());
        Some(slot)
    }

    #[inline(never)]
    fn finish_job(&self, frame: W) {
        let done = frame.done();
        drop(frame);
        if done {
            self.change(|live| {
                let finished = live.iter().take_while(|frame| frame.done()).count();
                live.drain(..finished);
            });
        }
        fence(Ordering::SeqCst);
        self.wait.wake_driver();
    }

    #[inline]
    fn worker(&self, index: usize, execute: &mut impl FnMut(&W, Claim, usize)) {
        let Some(slot) = self.start_worker(index) else {
            return;
        };
        while let Some((frame, claim)) = self.next_job(index, slot) {
            execute(&frame, claim, index);
            self.finish_job(frame);
        }
    }
}

impl<W: FrameWork + Clone + 'static> Core<W> {
    /// Dispatches from the calling thread, as its worker when it is one.
    fn dispatch_here(&self) {
        let addr = core::ptr::from_ref(self).addr();
        self.dispatch(BOUND.with_borrow(|bound| {
            bound
                .as_ref()
                .filter(|(core, _)| core.core_addr() == addr)
                .and_then(|(_, slot)| *slot)
        }));
    }
}

/// Worker-thread handle passed to [`RowScheduler::new_with`] so each worker can
/// enter its own compile-time execution context before draining jobs.
pub struct WorkerLoop<W: FrameWork + Clone + 'static> {
    core: Arc<Core<W>>,
    index: usize,
}

impl<W> core::fmt::Debug for WorkerLoop<W>
where
    W: FrameWork + Clone + 'static,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WorkerLoop")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl<W: FrameWork + Clone + 'static> WorkerLoop<W> {
    /// Binds this thread to its scheduler slot and runs claimed work via `execute` until shutdown.
    #[inline]
    pub fn run(self, mut execute: impl FnMut(&W, Claim, usize)) {
        bind(&self.core, Some(self.index));
        self.core.worker(self.index, &mut execute);
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
pub struct RowScheduler<W: FrameWork + Clone + 'static> {
    core: Arc<Core<W>>,
    workers: Vec<JoinHandle<()>>,
}

impl<W: FrameWork + Clone + 'static> core::fmt::Debug for RowScheduler<W> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RowScheduler")
            .field("workers", &self.workers.len())
            .finish_non_exhaustive()
    }
}

impl<W: FrameWork + Clone + 'static> RowScheduler<W> {
    /// Starts `threads` workers named `splot-row-{i}`.
    ///
    /// # Errors
    /// Returns [`ParallelError::PoolBuild`] when a thread cannot start.
    pub fn new(threads: NonZeroUsize) -> Result<Self, ParallelError> {
        Self::new_with(threads, |worker_loop| worker_loop.run(FrameWork::run))
    }

    /// Starts `threads` workers named `splot-row-{i}` and enters `start` on each worker thread.
    ///
    /// # Errors
    /// Returns [`ParallelError::ThreadSpawn`] when a thread cannot start.
    pub fn new_with<F>(threads: NonZeroUsize, start: F) -> Result<Self, ParallelError>
    where
        F: FnOnce(WorkerLoop<W>) + Clone + Send + 'static,
    {
        let core = Arc::new(Core::new(threads.get()));
        let mut workers = Vec::with_capacity(threads.get());
        for index in 0..threads.get() {
            let core = Arc::clone(&core);
            let start = start.clone();
            workers.push(spawn_thread(&format!("splot-row-{index}"), move || {
                start(WorkerLoop { core, index });
            })?);
        }
        Ok(Self { core, workers })
    }

    /// The handle frames use to wake this scheduler.
    #[must_use]
    pub fn wake(&self) -> RowWake {
        RowWake(Some(Arc::clone(&self.core) as Arc<dyn WakeCore>))
    }

    /// Adds the newest frame. Its rows run after every older frame's ready rows.
    pub fn push(&self, frame: W) {
        bind(&self.core, None);
        self.core.change(|live| live.push(frame));
        self.core.dispatch(None);
    }

    /// Workers parked with nothing to run, for tests.
    #[cfg(test)]
    pub(crate) fn parked_workers(&self) -> usize {
        self.core.wait.parked_workers()
    }

    /// Frames pushed and not yet done, for tests.
    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.core.frames.0.lock().live.len()
    }

    /// Blocks the driver until `ready()` holds. `ready` is re-checked after
    /// every publication and every worker's drop of a frame handle.
    pub fn wait(&self, mut ready: impl FnMut() -> bool) {
        bind(&self.core, None);
        self.core.wait.wait(&mut ready);
    }

    /// The number of worker threads.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.workers.len()
    }
}

impl<W: FrameWork + Clone + 'static> Drop for RowScheduler<W> {
    fn drop(&mut self) {
        self.core.wait.shutdown.0.store(true, Ordering::SeqCst);
        for worker in &self.workers {
            worker.thread().unpark();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        self.core.change(Vec::clear);
        self.core.ready.0.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WatermarkCell;
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
        gate: Option<Arc<WatermarkCell>>,
        need: fn(usize) -> usize,
        done_rows: Arc<WatermarkCell>,
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

        fn run(&self, claim: Claim, _worker: usize) {
            self.log.lock().push((self.id, claim.row));
            if self.fail_at == Some(claim.row) {
                self.done_rows.publish(WatermarkCell::FAILED);
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
        id: usize,
        rows: usize,
        gate: Option<Arc<WatermarkCell>>,
        need: fn(usize) -> usize,
        log: &Arc<Mutex<Vec<(usize, usize)>>>,
    ) -> Arc<Chain> {
        Arc::new(Chain {
            id,
            stage: Stage::new(rows),
            gate,
            need,
            done_rows: Arc::new(WatermarkCell::new()),
            log: Arc::clone(log),
            fail_at: None,
            slow_done: false,
        })
    }

    fn wait_done<W: FrameWork + Clone + 'static>(
        scheduler: &RowScheduler<W>,
        frames: &[Arc<Chain>],
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        scheduler.wait(|| frames.iter().all(FrameWork::done) || Instant::now() > deadline);
        assert!(frames.iter().all(FrameWork::done), "frames did not finish");
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
    fn a_claim_that_loses_its_row_never_shows_it_running() {
        let stage = Stage::new(usize::MAX >> 1);
        let (window, stop, seen) = (
            AtomicUsize::new(0),
            AtomicBool::new(false),
            AtomicUsize::new(0),
        );
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !stop.load(Ordering::SeqCst) {
                    let open = window.load(Ordering::SeqCst);
                    if open != 0
                        && stage.state.load(Ordering::SeqCst) & Stage::RUNNING != 0
                        && window.load(Ordering::SeqCst) == open
                    {
                        seen.fetch_add(1, Ordering::SeqCst);
                    }
                }
            });
            let mut wrong = 0;
            for round in 1..=200_000 {
                let lost = stage.claim(|row| {
                    let won = stage.claim(|_| true) == Some(row);
                    stage.finish(row);
                    window.store(round, Ordering::SeqCst);
                    won
                });
                window.store(0, Ordering::SeqCst);
                wrong += usize::from(lost.is_some());
            }
            stop.store(true, Ordering::SeqCst);
            assert_eq!(wrong, 0, "a row was claimed twice");
        });
        assert_eq!(
            seen.load(Ordering::SeqCst),
            0,
            "a scan saw a row running that no claim held"
        );
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

        fn run(&self, _: Claim, _: usize) {}

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

        fn run(&self, _: Claim, _: usize) {}

        fn done(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_push_completes_while_every_worker_scans_without_pause() {
        let scheduler = Arc::new(RowScheduler::<Arc<dyn FrameWork>>::new(threads(8)).unwrap());
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

        fn run(&self, claim: Claim, _worker: usize) {
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
        let scheduler = RowScheduler::<Arc<dyn FrameWork>>::new(threads(2)).unwrap();
        let long = Hold::new(true, None);
        let short = Hold::new(false, Some(Arc::clone(&long)));
        scheduler.push(Arc::clone(&short) as Arc<dyn FrameWork>);
        scheduler.push(Arc::clone(&long) as Arc<dyn FrameWork>);
        let rescue = scheduler.wake();
        let _watchdog = spawn_thread("watchdog", move || {
            std::thread::sleep(Duration::from_secs(3));
            rescue.notify();
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
    fn a_missed_revocation_wakes_the_idle_owner_and_its_scan_wakes_the_driver() {
        let scheduler: RowScheduler<Arc<dyn FrameWork>> = RowScheduler {
            core: Arc::new(Core::new(1)),
            workers: Vec::new(),
        };
        let core = &scheduler.core;
        let frame: Arc<dyn FrameWork> = Arc::new(Finished);
        core.change(|live| live.push(Arc::clone(&frame)));
        let slot = &core.slots[0];
        assert!(core.scan(slot).is_none());
        core.wait.set_idle(0);
        let (owner_woken, driver_woken) = std::thread::scope(|scope| {
            let driver = scope.spawn(|| scheduler.wait(|| Arc::strong_count(&frame) == 1));
            while !core.wait.driver.0.parked.load(Ordering::SeqCst) {
                std::hint::spin_loop();
            }
            drop(core.wait.driver.0.lock.lock());
            {
                let _held_by_another_change = slot.shot.0.snapshot.lock();
                core.change(Vec::clear);
            }
            assert_eq!(
                Arc::strong_count(&frame),
                2,
                "only the owner's snapshot holds it"
            );
            let owner_woken = !core.wait.is_idle(0);
            assert!(core.scan(slot).is_none());
            let deadline = Instant::now() + Duration::from_secs(5);
            while !driver.is_finished() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            let driver_woken = driver.is_finished();
            core.wait.wake_driver();
            (owner_woken, driver_woken)
        });
        assert!(owner_woken, "the idle owner was left with a stale snapshot");
        assert!(
            driver_woken,
            "the scan dropped the last handle without waking the driver"
        );
    }

    /// Rows marked ready from outside; running an even row marks the next.
    struct Burst {
        stage: ParallelStage,
        runs: Vec<AtomicUsize>,
        wake: RowWake,
    }

    impl FrameWork for Burst {
        fn claim(&self) -> Option<Claim> {
            self.stage.claim_ready().map(|row| Claim { stage: 0, row })
        }

        fn run(&self, claim: Claim, _worker: usize) {
            if let Some(runs) = self.runs.get(claim.row) {
                runs.fetch_add(1, Ordering::SeqCst);
            }
            if claim.row.is_multiple_of(2) {
                self.stage.ready(claim.row + 1);
                self.wake.notify();
            }
            self.stage.finish(claim.row);
        }

        fn done(&self) -> bool {
            self.stage.is_finished()
        }
    }

    #[test]
    fn every_dispatched_row_runs_exactly_once_under_many_dispatchers() {
        const ROWS: usize = 4000;
        const DISPATCHERS: usize = 4;
        let scheduler = RowScheduler::new(threads(8)).unwrap();
        for _ in 0..5 {
            let frame = Arc::new(Burst {
                stage: ParallelStage::new(),
                runs: (0..ROWS).map(|_| AtomicUsize::new(0)).collect(),
                wake: scheduler.wake(),
            });
            frame.stage.set_rows(ROWS);
            scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
            let dispatchers: Vec<_> = (0..DISPATCHERS)
                .map(|first| {
                    let frame = Arc::clone(&frame);
                    spawn_thread("dispatcher", move || {
                        for row in (2 * first..ROWS).step_by(2 * DISPATCHERS) {
                            frame.stage.ready(row);
                            frame.wake.notify();
                        }
                    })
                    .unwrap()
                })
                .collect();
            for dispatcher in dispatchers {
                dispatcher.join().unwrap();
            }
            let rescue = scheduler.wake();
            let _watchdog = spawn_thread("watchdog", move || {
                std::thread::sleep(Duration::from_secs(10));
                rescue.notify();
            })
            .unwrap();
            let started = Instant::now();
            scheduler.wait(|| frame.done() || started.elapsed() > Duration::from_secs(10));
            assert!(frame.done(), "a dispatched row was dropped");
            assert!(
                frame
                    .runs
                    .iter()
                    .all(|runs| runs.load(Ordering::SeqCst) == 1),
                "a job ran twice or never"
            );
        }
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
        let gate = Arc::new(WatermarkCell::new());
        let frame = chain(0, 1, Some(Arc::clone(&gate)), |_| 1, &log);
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
                ..Arc::into_inner(chain(0, 8, None, |_| 0, &log)).unwrap()
            });
            scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
            let rescue = scheduler.wake();
            let watchdog = spawn_thread("watchdog", move || {
                std::thread::sleep(Duration::from_secs(3));
                rescue.notify();
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
        let first = chain(0, 4, None, |_| 0, &log);
        let second = chain(
            1,
            4,
            Some(Arc::clone(&first.done_rows)),
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
        let gate = Arc::new(WatermarkCell::new());
        let frame = chain(0, 1, Some(Arc::clone(&gate)), |_| 1, &log);
        scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
        std::thread::sleep(Duration::from_millis(20));
        assert!(!frame.done());
        gate.publish(1);
        wait_done(&scheduler, &[frame]);
    }

    /// A one-row frame gated on a watermark that holds no [`RowWake`].
    struct OnCell {
        cell: Arc<WatermarkCell>,
        stage: Stage,
    }

    impl FrameWork for OnCell {
        fn claim(&self) -> Option<Claim> {
            self.stage
                .claim(|_| self.cell.current() != 0)
                .map(|row| Claim { stage: 0, row })
        }

        fn run(&self, claim: Claim, _worker: usize) {
            self.stage.finish(claim.row);
        }

        fn done(&self) -> bool {
            self.stage.is_finished()
        }
    }

    /// Waits for `frame`, failing when only a watchdog's wake after three
    /// seconds would end the wait.
    fn wait_without_rescue<W: FrameWork + Clone + 'static>(
        scheduler: &RowScheduler<W>,
        frame: &OnCell,
        what: &str,
    ) {
        let rescue = scheduler.wake();
        let _watchdog = spawn_thread("watchdog", move || {
            std::thread::sleep(Duration::from_secs(3));
            rescue.notify();
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
    struct Publish(Arc<OnCell>, Arc<WatermarkCell>, Stage, RowWake);

    impl FrameWork for Publish {
        fn claim(&self) -> Option<Claim> {
            self.2.claim(|_| true).map(|row| Claim { stage: 0, row })
        }

        fn run(&self, claim: Claim, _worker: usize) {
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
        let cell = Arc::new(WatermarkCell::new());
        let waiter = Arc::new(OnCell {
            cell: Arc::clone(&cell),
            stage: Stage::new(1),
        });
        scheduler.push(Arc::clone(&waiter) as Arc<dyn FrameWork>);
        std::thread::sleep(Duration::from_millis(20));
        cell.publish(1);
        wait_without_rescue(&scheduler, &waiter, "driver");
        let gate = Arc::new(WatermarkCell::new());
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
            done_rows: Arc::new(WatermarkCell::new()),
            log: Arc::clone(&log),
            fail_at: Some(1),
            slow_done: false,
        };
        first.fail_at = Some(1);
        let first = Arc::new(first);
        let second = chain(
            1,
            3,
            Some(Arc::clone(&first.done_rows)),
            |row| row + 1,
            &log,
        );
        scheduler.push(Arc::clone(&first) as Arc<dyn FrameWork>);
        scheduler.push(Arc::clone(&second) as Arc<dyn FrameWork>);
        wait_done(&scheduler, &[first.clone(), second.clone()]);
        assert_eq!(first.done_rows.current(), WatermarkCell::FAILED);
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
        let never = Arc::new(WatermarkCell::new());
        let frame = chain(0, 2, Some(never), |_| 1, &log);
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
            let gate = frames
                .last()
                .map(|previous| Arc::clone(&previous.done_rows));
            let frame = chain(id, ROWS, gate, |row| (row + 2).min(ROWS), &log);
            scheduler.push(Arc::clone(&frame) as Arc<dyn FrameWork>);
            frames.push(frame);
        }
        wait_done(&scheduler, &frames);
        assert_eq!(log.lock().len(), FRAMES * ROWS);
        let stop = AtomicBool::new(true);
        assert!(stop.load(Ordering::Relaxed));
    }

    #[derive(Clone)]
    enum TypedWork {
        Chain(Arc<Chain>),
        Ready(Arc<Burst>),
    }

    impl FrameWork for TypedWork {
        fn claim(&self) -> Option<Claim> {
            match self {
                Self::Chain(frame) => frame.claim(),
                Self::Ready(frame) => frame.claim(),
            }
        }

        fn run(&self, claim: Claim, worker: usize) {
            match self {
                Self::Chain(frame) => frame.run(claim, worker),
                Self::Ready(frame) => frame.run(claim, worker),
            }
        }

        fn done(&self) -> bool {
            match self {
                Self::Chain(frame) => frame.done(),
                Self::Ready(frame) => frame.done(),
            }
        }
    }

    impl TypedWork {
        fn run_typed(&self, claim: Claim, worker: usize, tag: usize, visited: &AtomicUsize) {
            visited.fetch_add(tag, Ordering::Relaxed);
            self.run(claim, worker);
        }
    }

    #[test]
    fn new_with_preserves_concrete_work_and_per_worker_state_across_worker_counts() {
        for workers in [1usize, 3, 8] {
            let visited = Arc::new(AtomicUsize::new(0));
            let worker_state = Arc::clone(&visited);
            let scheduler = RowScheduler::<TypedWork>::new_with(threads(workers), move |loop_| {
                let tag = 1usize;
                loop_.run(|work, claim, worker| {
                    work.run_typed(claim, worker, tag, &worker_state);
                });
            })
            .unwrap();
            let log = Arc::new(Mutex::new(Vec::new()));
            let mut first_raw = Chain {
                id: 0,
                stage: Stage::new(6),
                gate: None,
                need: |_| 0,
                done_rows: Arc::new(WatermarkCell::new()),
                log: Arc::clone(&log),
                fail_at: Some(4),
                slow_done: false,
            };
            first_raw.fail_at = Some(4);
            let first = Arc::new(first_raw);
            let second = chain(
                1,
                6,
                Some(Arc::clone(&first.done_rows)),
                |row| row + 1,
                &log,
            );
            let burst = Arc::new(Burst {
                stage: ParallelStage::new(),
                runs: (0..10).map(|_| AtomicUsize::new(0)).collect(),
                wake: scheduler.wake(),
            });
            burst.stage.set_rows(10);
            scheduler.push(TypedWork::Chain(Arc::clone(&first)));
            scheduler.push(TypedWork::Chain(Arc::clone(&second)));
            scheduler.push(TypedWork::Ready(Arc::clone(&burst)));
            for row in (0..10).step_by(2) {
                burst.stage.ready(row);
                burst.wake.notify();
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            scheduler.wait(|| {
                (first.done() && second.done() && burst.done()) || Instant::now() > deadline
            });
            assert!(first.done() && second.done() && burst.done());
            assert_eq!(first.done_rows.current(), WatermarkCell::FAILED);
            assert_eq!(visited.load(Ordering::Relaxed), 22);
            drop(scheduler);
        }
    }

    #[test]
    fn multiple_schedulers_keep_wake_bound_isolated() {
        let sched_a = RowScheduler::<Arc<Chain>>::new(threads(3)).unwrap();
        let sched_b = RowScheduler::<Arc<Chain>>::new(threads(2)).unwrap();
        let log_a = Arc::new(Mutex::new(Vec::new()));
        let log_b = Arc::new(Mutex::new(Vec::new()));
        let gate_a = Arc::new(WatermarkCell::new());
        let gate_b = Arc::new(WatermarkCell::new());
        let frame_a = chain(0, 4, Some(Arc::clone(&gate_a)), |row| row + 1, &log_a);
        let frame_b = chain(1, 4, Some(Arc::clone(&gate_b)), |row| row + 1, &log_b);
        sched_a.push(Arc::clone(&frame_a));
        sched_b.push(Arc::clone(&frame_b));
        std::thread::sleep(Duration::from_millis(10));
        for v in 1..=4 {
            gate_a.publish(v);
            gate_b.publish(v);
            sched_a.wake().notify();
            sched_b.wake().notify();
        }
        wait_done(&sched_a, &[frame_a]);
        wait_done(&sched_b, &[frame_b]);
        assert_eq!(log_a.lock().len(), 4);
        assert_eq!(log_b.lock().len(), 4);
    }
}
