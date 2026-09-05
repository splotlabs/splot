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
//! Each frame's § 7.2 filter phase is handed to the admission scheduler and its
//! handle starts [pending](RefFrameSlot::pending); the task publishes samples through a
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
use std::sync::Arc;

use splot_parallel::{CompletionCell, Condition, TaskScope};
use splot_recon::{DecodedFrame, DecodedFrameInfo, ReconSample, SharedFrame};

use super::frame_engine::finish::{WalkStage, WalkedFrame, finish_walked_frame};
use super::frame_progress::FrameProgress;
use super::{PipelineDecodedFrame, unsupported};
use crate::error::{DecodeError, Result};
use crate::filters::wienerns_lr::FrameFilterRecords;
use crate::prediction::inter::InterDecodeScratch;
use crate::prediction::inter::reference::HeldFrameSamples;
use parking_lot::Mutex;

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
    pub(crate) fn pending_recycled(
        info: DecodedFrameInfo,
        recycled: &mut splot_recon::FramePlaneSamples<T>,
    ) -> Result<(Self, FrameSlotWriter<T>)> {
        let cell = Arc::new(CompletionCell::new());
        let progress = Arc::new(FrameProgress::recycled(info, recycled)?);
        let writer = FrameSlotWriter {
            cell: Arc::clone(&cell),
            progress: Arc::clone(&progress),
            info,
        };
        let slot = Self {
            cell,
            progress: Some(progress),
            info,
        };
        Ok((slot, writer))
    }

    #[cfg(test)]
    pub(crate) fn pending(info: DecodedFrameInfo) -> Result<(Self, FrameSlotWriter<T>)> {
        Self::pending_recycled(info, &mut splot_recon::FramePlaneSamples::default())
    }

    /// Takes the published frame when this is the last handle to both the slot
    /// and its samples, so its buffers can outlive it.
    fn into_frame(self) -> Option<DecodedFrame<T>> {
        match Arc::into_inner(self.cell)?.into_inner()? {
            SlotValue::Ready(frame) => frame.into_frame(),
            SlotValue::Failed => None,
        }
    }

    /// Whether this is the only handle to the slot, so
    /// [`Self::into_frame`] can take the published frame out of it.
    ///
    /// A frame captured as a reference by an in-flight frame is still held
    /// through its slot even once nothing shares its samples, so the driver
    /// must ask this before retiring it or the buffers are dropped instead of
    /// kept.
    pub(crate) fn is_sole_handle(&self) -> bool {
        Arc::strong_count(&self.cell) == 1
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

    /// Returns the scheduler condition for this slot settling.
    pub(crate) fn settled_condition(&self) -> Condition<'_> {
        Condition::completion(self.cell.as_ref())
    }

    /// Returns the scheduler condition for a readable luma-row prefix, or for
    /// the whole slot settling when this slot has no progressive publisher.
    pub(crate) fn row_condition(&self, rows: usize) -> Condition<'_> {
        self.progress().map_or_else(
            || self.settled_condition(),
            |progress| progress.row_condition(rows),
        )
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
        let result = match self.cell.wait_with_pool_assist() {
            SlotValue::Ready(_) => Ok(()),
            SlotValue::Failed => Err(failed_slot()),
        };
        if let Some(progress) = self.progress() {
            progress.wait_terminal();
        }
        result
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
    progress: Arc<FrameProgress<T>>,
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
        self.progress.publish_terminal(true);
    }
}

impl<T: ReconSample> Drop for FrameSlotWriter<T> {
    fn drop(&mut self) {
        let settled = self.cell.is_set();
        let _ = self.cell.set(SlotValue::Failed);
        if !settled {
            self.progress.publish_terminal(false);
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

    /// Whether this is the only handle to the slot.
    pub(crate) fn is_sole_handle(&self) -> bool {
        match self {
            Self::Eight(slot) => slot.is_sole_handle(),
            Self::Ten(slot) => slot.is_sole_handle(),
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
}

/// What a deferred filter phase reports back to the driver.
#[derive(Default)]
struct FinishOutcome {
    error: Option<DecodeError>,
    records: Option<FrameFilterRecords>,
}

struct FinishReportWriter {
    cell: Arc<CompletionCell<Mutex<FinishOutcome>>>,
    outcome: FinishOutcome,
}

impl Drop for FinishReportWriter {
    fn drop(&mut self) {
        let _ = self
            .cell
            .set(Mutex::new(core::mem::take(&mut self.outcome)));
    }
}

/// One frame whose filter phase the driver has not collected yet.
struct InflightEntry {
    frame_index: usize,
    slot: PipelineFrameSlot,
    report: Arc<CompletionCell<Mutex<FinishOutcome>>>,
}

/// The bounded set of frames whose filter phase runs on the pool.
///
/// An effective capacity of `D` overlaps `D` frames: the frame the driver walks
/// plus the `D - 1` filter phases it has not collected. The ring therefore keeps
/// up to `D - 1` entries across an admission, and reaches `D` only between a
/// frame's own push and the next admission, which harvests back down. The
/// pipeline derives `D` as the smaller of the resolved requested frame delay and
/// the pool width. Capacity one still uses the same scheduler and task graph.
///
/// Filter tasks never block on a slot, and every driver wait runs pool jobs
/// ([`CompletionCell::wait_with_pool_assist`]) instead of parking, so a
/// one-worker driver can execute the task it is waiting for.
pub(crate) struct InflightRing {
    capacity: usize,
    entries: VecDeque<InflightEntry>,
    failure: Option<(usize, DecodeError)>,
    spare_eight: Vec<splot_recon::FramePlaneSamples<u8>>,
    spare_ten: Vec<splot_recon::FramePlaneSamples<u16>>,
}

/// Routes a frame's retired sample buffers to the ring's spares of its sample
/// type.
///
/// One frame retires for every frame the ring admits, but at depth `D` up to
/// `D` of them can retire between two takes, so the spares are a stack bounded
/// by the ring's own capacity rather than a single slot: the frame taking a
/// reference slot's place decodes into the buffers a frame leaving it gave up.
pub(crate) trait SpareFramePlanes: ReconSample {
    fn spares(ring: &mut InflightRing) -> &mut Vec<splot_recon::FramePlaneSamples<Self>>;
}

impl SpareFramePlanes for u8 {
    fn spares(ring: &mut InflightRing) -> &mut Vec<splot_recon::FramePlaneSamples<Self>> {
        &mut ring.spare_eight
    }
}

impl SpareFramePlanes for u16 {
    fn spares(ring: &mut InflightRing) -> &mut Vec<splot_recon::FramePlaneSamples<Self>> {
        &mut ring.spare_ten
    }
}

/// Keeps `retired` when the ring is not already holding a full cycle of spares.
fn keep_spare<T: ReconSample>(
    spares: &mut Vec<splot_recon::FramePlaneSamples<T>>,
    capacity: usize,
    retired: splot_recon::FramePlaneSamples<T>,
) {
    if spares.len() < capacity {
        spares.push(retired);
    }
}

impl InflightRing {
    /// Builds the in-flight ring for a resolved frame-delay depth.
    pub(crate) fn new(depth: NonZeroUsize) -> Self {
        Self {
            capacity: depth.get(),
            entries: VecDeque::new(),
            failure: None,
            spare_eight: Vec::new(),
            spare_ten: Vec::new(),
        }
    }

    /// Keeps a retired frame's sample buffers for the frame that replaces it.
    ///
    /// A frame still shared by any reader keeps its own buffers: the samples
    /// are only taken when this handle is the last one holding them.
    pub(crate) fn keep_frame_planes(&mut self, slot: PipelineFrameSlot) {
        match slot {
            PipelineFrameSlot::Eight(slot) => {
                if let Some(frame) = slot.into_frame() {
                    keep_spare(
                        &mut self.spare_eight,
                        self.capacity,
                        frame.into_plane_samples(),
                    );
                }
            }
            PipelineFrameSlot::Ten(slot) => {
                if let Some(frame) = slot.into_frame() {
                    keep_spare(
                        &mut self.spare_ten,
                        self.capacity,
                        frame.into_plane_samples(),
                    );
                }
            }
        }
    }

    /// Resolved frame-admission depth used to bound pending frame slots.
    pub(crate) const fn capacity(&self) -> usize {
        self.capacity
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
    }

    fn harvest_oldest(
        &mut self,
        eight: &mut InterDecodeScratch<u8>,
        ten: &mut InterDecodeScratch<u16>,
    ) -> bool {
        let Some(entry) = self.entries.pop_front() else {
            return false;
        };
        let _ = entry.slot.wait_settled();
        let report = entry.report.wait_with_pool_assist();
        let mut outcome = report.lock();
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
/// phase is handed to the admission scheduler and the frame's handle is
/// returned pending and recorded on the in-flight ring.
///
/// # Errors
///
/// A deferred phase hands its single-use writer to the freeze, so the slot
/// settles before the frame's published row prefix closes; a freeze the phase
/// never reaches drops the writer instead, settling the slot as failed.
pub(super) fn settle_walk_stage<'job, 'scope, T: SpareFramePlanes + Send + 'static>(
    stage: WalkStage<T>,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    scope: &TaskScope<'_, 'scope>,
    scheduler: &'scope splot_parallel::AdmissionScheduler<
        'job,
        crate::pipeline::frame_pipeline::FrameTask,
    >,
    lane: &mut super::frame_pipeline::ReconAdmissionLane,
    ring: &mut InflightRing,
    frame_index: usize,
) -> Result<PipelineFrameSlot>
where
    'job: 'scope,
{
    let walked = match stage {
        WalkStage::Complete(frame) => {
            return Ok(erase(RefFrameSlot::completed(SharedFrame::new(*frame))));
        }
        WalkStage::Pending(walked) => walked,
    };
    let (slot, pending) = reserve_pending_slot(walked.info(), erase, ring, frame_index)?;
    super::frame_pipeline::schedule_finish(pending, *walked, frame_index, scope, scheduler, lane);
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
    progress: Arc<FrameProgress<T>>,
    report: FinishReportWriter,
}

/// Reserves one frame's decoded-frame handle from its known geometry and
/// records it on the in-flight ring.
///
/// # Errors
///
/// Returns the filtered-workspace allocation's own diagnostic.
pub(crate) fn reserve_pending_slot<T: SpareFramePlanes>(
    info: DecodedFrameInfo,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    ring: &mut InflightRing,
    frame_index: usize,
) -> Result<(PipelineFrameSlot, PendingFinish<T>)> {
    let mut spare = T::spares(ring).pop().unwrap_or_default();
    let (slot, writer) = RefFrameSlot::pending_recycled(info, &mut spare)?;
    let progress = Arc::clone(&writer.progress);
    let report = Arc::new(CompletionCell::new());
    ring.push(InflightEntry {
        frame_index,
        slot: erase(slot.share()),
        report: Arc::clone(&report),
    });
    Ok((
        erase(slot),
        PendingFinish {
            writer,
            progress,
            report: FinishReportWriter {
                cell: report,
                outcome: FinishOutcome::default(),
            },
        },
    ))
}

impl<T: ReconSample + Send + 'static> PendingFinish<T> {
    /// Shares the progressive output owner with scheduled filter-stripe tasks.
    pub(crate) fn progress_handle(&self) -> Arc<FrameProgress<T>> {
        Arc::clone(&self.progress)
    }

    /// Publishes a frame whose reconstruction is already terminal and has no
    /// filter records to return to the driver's scratch pool.
    pub(crate) fn complete_frame(self, frame: DecodedFrame<T>) {
        let Self {
            writer,
            progress: _,
            report,
        } = self;
        writer.complete(SharedFrame::new(frame));
        drop(report);
    }

    /// Runs the owed filter phase in the calling scheduler job.
    pub(crate) fn run_finish(
        self,
        walked: WalkedFrame<T>,
        admit: Option<&dyn splot_parallel::Admit<'_, crate::pipeline::frame_pipeline::FrameTask>>,
    ) {
        let Self {
            writer,
            progress,
            mut report,
        } = self;
        match finish_walked_frame(walked, Some(progress), admit, |frame| {
            writer.complete(frame);
        }) {
            Ok(filter_records) => {
                report.outcome.records = Some(filter_records);
            }
            Err(error) => {
                report.outcome.error = Some(error);
            }
        }
    }

    /// Freezes one scheduler-owned setup after every stripe job has settled.
    pub(crate) fn run_owned_finish(
        self,
        filter: crate::filters::wienerns_lr::recon::OwnedFilterFinish<T>,
    ) {
        let Self {
            writer,
            progress: _,
            mut report,
        } = self;
        match filter.finish(|frame| writer.complete(SharedFrame::new(frame))) {
            Ok(((), records)) => {
                report.outcome.records = Some(records);
            }
            Err(error) => {
                report.outcome.error = Some(error);
            }
        }
    }

    /// Settles the reserved slot as failed and records the reconstruction
    /// diagnostic for the in-flight ring.
    pub(crate) fn fail(mut self, error: DecodeError) {
        self.report.outcome.error = Some(error);
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
