// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Completion-backed decoded-frame handles and the frame-pipelining in-flight
//! ring.
//!
//! A [`RefFrameSlot`] names one decoded frame by an owned, shareable handle
//! instead of a borrow of the driver's frame vector. Its geometry is known as
//! soon as the frame header is parsed, while its samples are published once
//! through a [`CompletionCell`], so the handle can be stored in a reference
//! store and size-accounted independently of when the pixels land.
//!
//! When the driver runs pipelined, each frame's § 7.2 filter phase is handed to
//! a worker task through a [`FinishSpawner`] and the frame's handle starts
//! [pending](RefFrameSlot::pending); the task publishes the samples through a
//! single-use [`FrameSlotWriter`]. The driver tracks the frames whose filter
//! phase it has not collected in an [`InflightRing`] bounded by the resolved
//! frame-delay depth, and is the only thread that ever blocks on a slot: it
//! harvests the oldest in-flight entries only once admitting a new frame would
//! overlap more frames than that depth, and before reading a frame's pixels it
//! waits for that frame. A frame's own pixel references are gated inside its
//! walk instead ([`crate::prediction::inter::PixelReferenceGate`]), so the
//! walk's parse work overlaps a reference frame's filter phase. Worker tasks
//! never wait on any slot. Those driver waits run pool jobs instead of parking
//! idle, so a blocked driver still finishes frames.

use core::num::NonZeroUsize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use splot_parallel::{CompletionCell, TaskScope};
use splot_recon::{DecodedFrame, DecodedFrameInfo, ReconSample, SharedFrame};

use super::frame_engine::finish::{WalkStage, WalkedFrame, finish_walked_frame};
use super::frame_progress::FrameProgress;
use super::{PipelineDecodedFrame, unsupported};
use crate::error::{DecodeError, Result};
use crate::filters::wienerns_lr::FrameFilterRecords;
use crate::prediction::inter::InterDecodeScratch;
use crate::prediction::inter::reference::HeldFrameSamples;

/// The one-shot value a decoded-frame slot publishes.
enum SlotValue<T: ReconSample> {
    /// The filter phase published the frame's samples.
    Ready(SharedFrame<T>),
    /// The filter phase failed; its diagnostic travels on the ring entry.
    Failed,
}

impl<T: ReconSample> SlotValue<T> {
    /// Borrows the published samples, or `None` when the filter phase failed.
    const fn frame(&self) -> Option<&SharedFrame<T>> {
        match self {
            Self::Ready(frame) => Some(frame),
            Self::Failed => None,
        }
    }
}

/// An owned handle to one decoded reference frame and its known geometry.
pub(crate) struct RefFrameSlot<T: ReconSample> {
    cell: Arc<CompletionCell<SlotValue<T>>>,
    progress: Option<Arc<FrameProgress<T>>>,
    info: DecodedFrameInfo,
}

impl<T: ReconSample> RefFrameSlot<T> {
    /// Wraps an already reconstructed frame in a settled handle.
    pub(crate) fn completed(frame: SharedFrame<T>) -> Self {
        let info = frame.get().info();
        Self {
            cell: Arc::new(CompletionCell::completed(SlotValue::Ready(frame))),
            progress: None,
            info,
        }
    }

    /// Opens an unsettled handle of known geometry plus the single writer that
    /// may publish its samples.
    ///
    /// The handle carries the [`FrameProgress`] the filter phase publishes its
    /// stripes through, so a consumer can read the frame's settled row prefix
    /// while the rest is still filtering.
    ///
    /// # Errors
    ///
    /// Returns the filtered-workspace allocation's own diagnostic.
    pub(crate) fn pending(info: DecodedFrameInfo) -> Result<(Self, FrameSlotWriter<T>)> {
        let cell = Arc::new(CompletionCell::new());
        let progress = Arc::new(FrameProgress::new(info)?);
        let writer = FrameSlotWriter {
            cell: Arc::clone(&cell),
            progress: Some(Arc::clone(&progress)),
            info,
        };
        let slot = Self {
            cell,
            progress: Some(progress),
            info,
        };
        Ok((slot, writer))
    }

    /// Returns a second handle to the same completion slot without copying
    /// pixels, and without requiring the slot to be settled.
    pub(crate) fn share(&self) -> Self {
        Self {
            cell: Arc::clone(&self.cell),
            progress: self.progress.clone(),
            info: self.info,
        }
    }

    /// Borrows the row-granular publication state of a still-filtering frame.
    pub(crate) fn progress(&self) -> Option<&FrameProgress<T>> {
        self.progress.as_deref()
    }

    /// How many luma rows from the frame top this slot has published.
    ///
    /// A slot with no filter phase to watch publishes nothing row by row; its
    /// consumers gate on [`Self::is_settled`] instead.
    pub(crate) fn published_luma_rows(&self) -> usize {
        self.progress()
            .map_or(0, FrameProgress::published_luma_rows)
    }

    /// Borrows this slot's samples for one reader's block.
    ///
    /// A settled slot lends its decoded frame directly; a slot whose filter
    /// phase is still running lends the row prefix that phase has published,
    /// keeping the shared borrow alive for the handle's lifetime. The settled
    /// check runs again when the published prefix is gone, since the freeze can
    /// take the workspace between the two.
    pub(crate) fn hold_samples(&self) -> Option<HeldFrameSamples<'_, T>> {
        if let Some(frame) = self.try_frozen() {
            return Some(HeldFrameSamples::Settled(frame));
        }
        if let Some(published) = self.progress().and_then(FrameProgress::read) {
            return Some(HeldFrameSamples::Filtering(published));
        }
        self.try_frozen().map(HeldFrameSamples::Settled)
    }

    /// Borrows the decoded frame when its samples have already been published.
    pub(crate) fn try_frozen(&self) -> Option<&DecodedFrame<T>> {
        self.cell
            .get()
            .and_then(SlotValue::frame)
            .map(SharedFrame::get)
    }

    /// Whether the slot has settled, either with samples or with a failure.
    pub(crate) fn is_settled(&self) -> bool {
        self.cell.is_set()
    }

    /// Returns the decoded frame geometry, which is known before the samples.
    pub(crate) const fn info(&self) -> DecodedFrameInfo {
        self.info
    }

    /// Returns the number of live handles on the published frame storage, or
    /// one while no storage exists yet.
    pub(crate) fn handle_count(&self) -> usize {
        self.cell
            .get()
            .and_then(SlotValue::frame)
            .map_or(1, SharedFrame::handle_count)
    }

    /// Shares the published frame storage, failing closed when the samples have
    /// not landed.
    pub(crate) fn ready(&self) -> Result<SharedFrame<T>> {
        match self.cell.get() {
            Some(SlotValue::Ready(frame)) => Ok(frame.share()),
            Some(SlotValue::Failed) => Err(failed_slot()),
            None => Err(unsettled_slot()),
        }
    }

    /// Blocks the driver thread until the slot settles, then shares the
    /// published frame storage.
    pub(crate) fn wait_ready(&self) -> Result<SharedFrame<T>> {
        match self.cell.wait_with_pool_assist() {
            SlotValue::Ready(frame) => Ok(frame.share()),
            SlotValue::Failed => Err(failed_slot()),
        }
    }

    /// Blocks the driver thread until the slot settles.
    pub(crate) fn wait_settled(&self) -> Result<()> {
        match self.cell.wait_with_pool_assist() {
            SlotValue::Ready(_) => Ok(()),
            SlotValue::Failed => Err(failed_slot()),
        }
    }

    /// Retires the handle, returning the plane sample buffers to the
    /// reconstruction-plane pool when this was the last handle.
    pub(crate) fn reclaim_planes(self) {
        if let Some(SlotValue::Ready(frame)) =
            Arc::into_inner(self.cell).and_then(CompletionCell::into_inner)
        {
            frame.reclaim_planes();
        }
    }
}

/// The single publisher of one pending decoded-frame slot.
///
/// Dropping the writer settles the slot as failed, so a filter phase that
/// unwinds can never leave the driver blocked on a slot nobody will publish.
///
/// Both endings also publish a terminal row watermark, so a consumer that
/// stated a row threshold against the frame's progress is released whichever
/// way the phase ended instead of waiting for a row that will never land.
pub(crate) struct FrameSlotWriter<T: ReconSample> {
    cell: Arc<CompletionCell<SlotValue<T>>>,
    progress: Option<Arc<FrameProgress<T>>>,
    info: DecodedFrameInfo,
}

impl<T: ReconSample> FrameSlotWriter<T> {
    /// Publishes the filtered samples and wakes the driver.
    ///
    /// The geometry the slot published before the samples must be the geometry
    /// the finished frame reports, since reference-store bookkeeping and
    /// retained-byte accounting already read it.
    pub(crate) fn complete(self, frame: SharedFrame<T>) {
        debug_assert_eq!(frame.get().info(), self.info);
        let _ = self.cell.set(SlotValue::Ready(frame));
        if let Some(progress) = self.progress.as_deref() {
            progress.publish_terminal(true);
        }
    }
}

impl<T: ReconSample> Drop for FrameSlotWriter<T> {
    fn drop(&mut self) {
        let settled = self.cell.is_set();
        let _ = self.cell.set(SlotValue::Failed);
        if !settled && let Some(progress) = self.progress.as_deref() {
            progress.publish_terminal(false);
        }
    }
}

/// A decoded-frame handle in the pipeline's active sample storage bit depth.
pub(crate) enum PipelineFrameSlot {
    /// Eight-bit sample storage.
    Eight(RefFrameSlot<u8>),
    /// Ten-bit sample storage.
    Ten(RefFrameSlot<u16>),
}

impl PipelineFrameSlot {
    /// Wraps an already reconstructed frame in a settled handle.
    pub(crate) fn completed(frame: PipelineDecodedFrame) -> Self {
        match frame {
            PipelineDecodedFrame::Eight(frame) => Self::Eight(RefFrameSlot::completed(frame)),
            PipelineDecodedFrame::Ten(frame) => Self::Ten(RefFrameSlot::completed(frame)),
        }
    }

    /// Returns the decoded frame geometry, which is known before the samples.
    pub(crate) fn info(&self) -> DecodedFrameInfo {
        match self {
            Self::Eight(slot) => slot.info(),
            Self::Ten(slot) => slot.info(),
        }
    }

    /// Shares the eight-bit storage, or `None` when the slot is ten-bit.
    pub(crate) fn eight(&self) -> Option<RefFrameSlot<u8>> {
        match self {
            Self::Eight(slot) => Some(slot.share()),
            Self::Ten(_) => None,
        }
    }

    /// Shares the ten-bit storage, or `None` when the slot is eight-bit.
    pub(crate) fn ten(&self) -> Option<RefFrameSlot<u16>> {
        match self {
            Self::Ten(slot) => Some(slot.share()),
            Self::Eight(_) => None,
        }
    }

    /// Whether the slot has settled, either with samples or with a failure.
    pub(crate) fn is_settled(&self) -> bool {
        match self {
            Self::Eight(slot) => slot.is_settled(),
            Self::Ten(slot) => slot.is_settled(),
        }
    }

    /// Returns the number of live handles on the published frame storage.
    pub(crate) fn handle_count(&self) -> usize {
        match self {
            Self::Eight(slot) => slot.handle_count(),
            Self::Ten(slot) => slot.handle_count(),
        }
    }

    /// Shares the published frame storage, failing closed when the samples have
    /// not landed.
    pub(crate) fn ready(&self) -> Result<PipelineDecodedFrame> {
        Ok(match self {
            Self::Eight(slot) => PipelineDecodedFrame::Eight(slot.ready()?),
            Self::Ten(slot) => PipelineDecodedFrame::Ten(slot.ready()?),
        })
    }

    /// Blocks the driver thread until the slot settles, then shares the
    /// published frame storage.
    pub(crate) fn wait_ready(&self) -> Result<PipelineDecodedFrame> {
        Ok(match self {
            Self::Eight(slot) => PipelineDecodedFrame::Eight(slot.wait_ready()?),
            Self::Ten(slot) => PipelineDecodedFrame::Ten(slot.wait_ready()?),
        })
    }

    /// Blocks the driver thread until the slot settles.
    pub(crate) fn wait_settled(&self) -> Result<()> {
        match self {
            Self::Eight(slot) => slot.wait_settled(),
            Self::Ten(slot) => slot.wait_settled(),
        }
    }

    /// Retires the handle, returning the plane sample buffers to the
    /// reconstruction-plane pool when this was the last handle.
    pub(crate) fn reclaim_planes(self) {
        match self {
            Self::Eight(slot) => slot.reclaim_planes(),
            Self::Ten(slot) => slot.reclaim_planes(),
        }
    }
}

/// Where one frame's § 7.2 filter phase runs.
pub(crate) enum FinishSpawner<'a, 'scope> {
    /// On the driver thread, before the driver walks the next frame.
    Inline,
    /// On a worker task in the driver's scope, while the driver walks ahead.
    Deferred(&'a TaskScope<'a, 'scope>),
}

/// What a deferred filter phase reports back to the driver.
#[derive(Default)]
struct FinishOutcome {
    error: Option<DecodeError>,
    records: Option<FrameFilterRecords>,
}

/// One frame whose filter phase the driver has not collected yet.
struct InflightEntry {
    frame_index: usize,
    slot: PipelineFrameSlot,
    outcome: Arc<Mutex<FinishOutcome>>,
}

/// The bounded set of frames whose filter phase runs on the pool.
///
/// A resolved frame-delay depth of `D` overlaps `D` frames: the frame the driver
/// walks plus the `D - 1` filter phases it has not collected. The ring therefore
/// keeps up to `D - 1` entries across an admission, and reaches `D` only between
/// a frame's own push and the next admission, which harvests back down. A depth
/// of one keeps nothing in flight, which is the serial path.
///
/// [`splot_parallel::FrameDelay::resolve`] clamps `D` to the pool width, so the
/// driver plus the `D - 1` phases running beside it fit the workers. The
/// transient `D`-th entry cannot strand the pipeline either: filter tasks never
/// block on a slot, and every driver wait runs pool jobs
/// ([`CompletionCell::wait_with_pool_assist`]) instead of parking, so the driver
/// itself executes a task the pool has no free worker for.
pub(crate) struct InflightRing {
    capacity: usize,
    entries: VecDeque<InflightEntry>,
    failure: Option<(usize, DecodeError)>,
    max_in_flight: usize,
}

impl InflightRing {
    /// Builds the in-flight ring for a resolved frame-delay depth.
    pub(crate) fn new(depth: NonZeroUsize) -> Self {
        Self {
            capacity: depth.get(),
            entries: VecDeque::new(),
            failure: None,
            max_in_flight: 0,
        }
    }

    /// The high-water mark of frames whose filter phase the driver had handed
    /// out but not yet collected, which bounds how many ran at once.
    pub(crate) const fn max_in_flight(&self) -> usize {
        self.max_in_flight
    }

    /// Whether the ring still holds this frame's second slot handle, which keeps
    /// the frame's sample storage alive however few other owners remain.
    pub(crate) fn holds(&self, frame_index: usize) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.frame_index == frame_index)
    }

    /// Admits one more frame, harvesting the oldest entries until the frame
    /// about to be walked plus the uncollected filter phases fit the depth.
    pub(crate) fn reserve(
        &mut self,
        eight: &mut InterDecodeScratch<u8>,
        ten: &mut InterDecodeScratch<u16>,
    ) {
        while self.entries.len() >= self.capacity && self.harvest_oldest(eight, ten) {}
    }

    /// Collects every outstanding filter phase.
    pub(crate) fn harvest_all(
        &mut self,
        eight: &mut InterDecodeScratch<u8>,
        ten: &mut InterDecodeScratch<u16>,
    ) {
        while self.harvest_oldest(eight, ten) {}
    }

    /// Takes the lowest-indexed filter-phase failure the ring collected.
    pub(crate) fn take_failure(&mut self) -> Option<DecodeError> {
        self.failure.take().map(|(_, error)| error)
    }

    fn push(&mut self, entry: InflightEntry) {
        self.entries.push_back(entry);
        self.max_in_flight = self.max_in_flight.max(self.entries.len());
    }

    fn harvest_oldest(
        &mut self,
        eight: &mut InterDecodeScratch<u8>,
        ten: &mut InterDecodeScratch<u16>,
    ) -> bool {
        let Some(entry) = self.entries.pop_front() else {
            return false;
        };
        let started = crate::timing::start();
        let _ = entry.slot.wait_settled();
        crate::timing::report("pipeline_harvest_wait", started);
        let mut outcome = entry.outcome.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(records) = outcome.records.take() {
            match entry.slot {
                PipelineFrameSlot::Eight(_) => eight.recycle_frame_filter_records(records),
                PipelineFrameSlot::Ten(_) => ten.recycle_frame_filter_records(records),
            }
        }
        let error = outcome.error.take();
        drop(outcome);
        if let Some(error) = error {
            self.note_failure(entry.frame_index, error);
        }
        true
    }

    fn note_failure(&mut self, frame_index: usize, error: DecodeError) {
        if self
            .failure
            .as_ref()
            .is_none_or(|(seen, _)| frame_index < *seen)
        {
            self.failure = Some((frame_index, error));
        }
    }
}

/// Settles one frame's walk into a decoded-frame handle.
///
/// A frame that left the walk final settles immediately. Otherwise the filter
/// phase runs inline on the driver, or is handed to a worker task and the
/// frame's handle is returned pending and recorded on the in-flight ring.
///
/// # Errors
///
/// Returns the filter chain's own diagnostic when an inline filter phase fails.
///
/// A deferred phase hands its single-use writer to the freeze, so the slot
/// settles before the frame's published row prefix closes; a freeze the phase
/// never reaches drops the writer instead, settling the slot as failed.
pub(crate) fn settle_walk_stage<T: ReconSample + Send + 'static>(
    stage: WalkStage<T>,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    spawner: &FinishSpawner<'_, '_>,
    ring: &mut InflightRing,
    frame_index: usize,
    scratch: &mut InterDecodeScratch<T>,
) -> Result<PipelineFrameSlot> {
    let walked = match stage {
        WalkStage::Complete(frame) => {
            return Ok(erase(RefFrameSlot::completed(SharedFrame::new(*frame))));
        }
        WalkStage::Pending(walked) => walked,
    };
    let FinishSpawner::Deferred(scope) = spawner else {
        let finished = finish_walked_frame(*walked, None, core::convert::identity)?;
        scratch.recycle_frame_filter_records(finished.filter_records);
        return Ok(erase(RefFrameSlot::completed(finished.frame)));
    };
    let (slot, pending) = reserve_pending_slot(walked.info(), erase, ring, frame_index)?;
    pending.spawn_finish(*walked, scope);
    Ok(slot)
}

/// The single publisher of one reserved decoded-frame slot, plus the channel
/// its filter phase reports back through.
///
/// A frame whose reconstruction is still owed reserves its slot from its
/// header and parse products alone, so the driver's bookkeeping can record the
/// reference update before the samples exist. Dropping the reservation without
/// spawning settles the slot as failed, which is how a frame whose
/// reconstruction never ran stops the driver blocking on it.
pub(crate) struct PendingFinish<T: ReconSample> {
    writer: FrameSlotWriter<T>,
    progress: Option<Arc<FrameProgress<T>>>,
    outcome: Arc<Mutex<FinishOutcome>>,
}

/// Reserves one frame's decoded-frame handle from its known geometry and
/// records it on the in-flight ring.
///
/// # Errors
///
/// Returns the filtered-workspace allocation's own diagnostic.
pub(crate) fn reserve_pending_slot<T: ReconSample>(
    info: DecodedFrameInfo,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    ring: &mut InflightRing,
    frame_index: usize,
) -> Result<(PipelineFrameSlot, PendingFinish<T>)> {
    let (slot, writer) = RefFrameSlot::pending(info)?;
    let progress = slot.progress.clone();
    let outcome = Arc::new(Mutex::new(FinishOutcome::default()));
    ring.push(InflightEntry {
        frame_index,
        slot: erase(slot.share()),
        outcome: Arc::clone(&outcome),
    });
    Ok((
        erase(slot),
        PendingFinish {
            writer,
            progress,
            outcome,
        },
    ))
}

impl<T: ReconSample + Send + 'static> PendingFinish<T> {
    /// Hands one walked frame's § 7.2 filter phase to a worker task.
    ///
    /// The task carries the single-use writer into the freeze, so the slot
    /// settles before the frame's published row prefix closes; a freeze the
    /// phase never reaches drops the writer instead, settling the slot as
    /// failed.
    pub(crate) fn spawn_finish(self, walked: WalkedFrame<T>, scope: &TaskScope<'_, '_>) {
        let Self {
            writer,
            progress,
            outcome,
        } = self;
        scope.spawn(move |_| {
            let started = crate::timing::start();
            match finish_walked_frame(walked, progress.as_deref(), |frame| {
                writer.complete(frame);
            }) {
                Ok(finished) => {
                    outcome
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .records = Some(finished.filter_records);
                }
                Err(error) => {
                    outcome.lock().unwrap_or_else(PoisonError::into_inner).error = Some(error);
                }
            }
            crate::timing::report("finish_task", started);
        });
    }

    /// Runs one walked frame's § 7.2 filter phase on the calling thread, for a
    /// driver that has no scope to spawn into.
    ///
    /// # Errors
    ///
    /// Returns the filter chain's own diagnostic.
    pub(crate) fn finish_inline(self, walked: WalkedFrame<T>) -> Result<FrameFilterRecords> {
        let Self {
            writer,
            progress,
            outcome: _,
        } = self;
        let finished = finish_walked_frame(walked, progress.as_deref(), |frame| {
            writer.complete(frame);
        })?;
        Ok(finished.filter_records)
    }
}

fn unsettled_slot() -> DecodeError {
    unsupported(
        "decoded_frame_samples_unavailable",
        None,
        "internal invariant violation: a decoded frame handle was read before its samples landed",
    )
}

fn failed_slot() -> DecodeError {
    unsupported(
        "decoded_frame_filter_phase_failed",
        None,
        "internal invariant violation: a decoded frame handle was read after its filter phase failed",
    )
}

#[cfg(test)]
#[path = "inflight_tests.rs"]
mod tests;
