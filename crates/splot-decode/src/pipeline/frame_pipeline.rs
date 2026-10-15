// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Admission-scheduled parse, reconstruction, and filter overlap.
//!
//! The entropy pass reads no reference samples, so the driver can submit the
//! previous frame's reconstruction and continue parsing. Every cross-frame
//! dependency is an admission condition: pool tasks never wait for motion or
//! pixel publication. Ordered continuations preserve the commit spine, while
//! the driver alone waits when later bookkeeping needs reconstruction complete.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.

use std::collections::VecDeque;
use std::sync::Arc;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_parallel::{AdmissionScheduler, CompletionCell, Condition};
use splot_recon::BitDepth;

use crate::Result;
use crate::prediction::inter;

use super::inflight::{InflightRing, PendingFinish, PipelineFrameSlot, RefFrameSlot};
use parking_lot::Mutex;

const ORDER_KEY_FRAME_STRIDE: u64 = 1 << 32;
type ReconScratchSlot = Arc<CompletionCell<Mutex<Option<ScheduledReconScratch>>>>;

pub(crate) enum ScheduledReconScratch {
    Eight(inter::InterDecodeScratch<u8>),
    Ten(inter::InterDecodeScratch<u16>),
}

/// The job shapes the pipeline schedules for frames that are not row scheduled.
pub(crate) enum FrameTask<'job> {
    FinishEight(super::inflight::ParkedFinish<u8>),
    FinishTen(super::inflight::ParkedFinish<u16>),
    TipOutputEight(ParkedTipOutput<'job, u8>),
    TipOutputTen(ParkedTipOutput<'job, u16>),
}

impl<'job> splot_parallel::Task<'job> for FrameTask<'job> {
    fn run(self, admit: &dyn splot_parallel::Admit<'job, Self>) {
        match self {
            Self::FinishEight(finish) => finish.run(admit),
            Self::FinishTen(finish) => finish.run(admit),
            Self::TipOutputEight(tip) => tip.run(),
            Self::TipOutputTen(tip) => tip.run(),
        }
    }
}

pub(crate) trait ScheduledScratchSample: splot_recon::ReconSample {
    fn tip_output_task(task: ParkedTipOutput<'_, Self>) -> FrameTask<'_>;

    fn take_scheduled_scratch(
        scratch: &mut Option<ScheduledReconScratch>,
    ) -> Option<inter::InterDecodeScratch<Self>>;

    fn wrap_scheduled_scratch(scratch: inter::InterDecodeScratch<Self>) -> ScheduledReconScratch;
}

macro_rules! impl_scheduled_scratch_sample {
    ($sample:ty, $variant:ident, $tip:ident) => {
        impl ScheduledScratchSample for $sample {
            fn tip_output_task(task: ParkedTipOutput<'_, Self>) -> FrameTask<'_> {
                FrameTask::$tip(task)
            }

            fn take_scheduled_scratch(
                scratch: &mut Option<ScheduledReconScratch>,
            ) -> Option<inter::InterDecodeScratch<Self>> {
                match scratch.take() {
                    Some(ScheduledReconScratch::$variant(scratch)) => Some(scratch),
                    other => {
                        *scratch = other;
                        None
                    }
                }
            }

            fn wrap_scheduled_scratch(
                scratch: inter::InterDecodeScratch<Self>,
            ) -> ScheduledReconScratch {
                ScheduledReconScratch::$variant(scratch)
            }
        }
    };
}

impl_scheduled_scratch_sample!(u8, Eight, TipOutputEight);
impl_scheduled_scratch_sample!(u16, Ten, TipOutputTen);

type PendingTipProducts<T> = (
    PipelineFrameSlot,
    PendingFinish<T>,
    inter::FrameDecodeGeometry,
    inter::FrameProductWriters,
    inter::MotionFieldHandle,
);

/// Reserves the pending frame and product handles published by one TIP job.
pub(super) fn reserve_tip_output<T: super::inflight::SpareFramePlanes>(
    core: &FrameHeaderCore,
    sequence: &SequenceHeader,
    bit_depth: BitDepth,
    erase: fn(RefFrameSlot<T>) -> PipelineFrameSlot,
    frames: &mut super::FrameStore,
    ring: &mut InflightRing,
    frame_index: usize,
) -> Result<PendingTipProducts<T>> {
    let geometry = inter::FrameDecodeGeometry::new(core, sequence, bit_depth, false)?;
    let motion = frames.reserve_motion(geometry.motion_layout())?;
    let (slot, finish) =
        super::inflight::reserve_pending_slot(geometry.info(), erase, ring, frames, frame_index)?;
    let products = frames.reserve_products()?;
    Ok((slot, finish, geometry, products, motion))
}

/// One TIP output frame's reconstruction inputs, parked in a reusable cell
/// until its scheduler task takes them.
pub(crate) struct TipOutputJob<'job, T: splot_recon::ReconSample> {
    pub(super) candidate: crate::bitstream::stream_plan::DecodePlannedObu,
    pub(super) core: Arc<FrameHeaderCore>,
    pub(super) sequence: Arc<SequenceHeader>,
    pub(super) options: &'job crate::DecodeOptions,
    pub(super) reference: inter::InterReferenceState<T>,
    pub(super) geometry: inter::FrameDecodeGeometry,
}

/// A reusable cell holding one TIP output job between its submission and run.
pub(crate) type TipOutputCell<'job, T> = Arc<Mutex<Option<TipOutputTask<'job, T>>>>;

pub(crate) struct TipOutputTask<'job, T: splot_recon::ReconSample> {
    job: TipOutputJob<'job, T>,
    products: inter::FrameProductWriters,
    motion: inter::MotionFieldHandle,
    finish: PendingFinish<T>,
    scratch: Option<ReconScratchSlot>,
    scratch_done: ReconScratchSlot,
}

/// The scheduler task for one parked TIP output job.
///
/// A task dropped unrun drops the parked job, which settles the frame's slot
/// as failed instead of leaving it pending.
pub(crate) struct ParkedTipOutput<'job, T: splot_recon::ReconSample>(TipOutputCell<'job, T>);

impl<T: splot_recon::ReconSample> Drop for ParkedTipOutput<'_, T> {
    fn drop(&mut self) {
        drop(self.0.lock().take());
    }
}

/// Claims a TIP output cell no task still holds, adding one when all are busy.
pub(super) fn claim_tip_output_cell<'job, T: splot_recon::ReconSample>(
    cells: &mut Vec<TipOutputCell<'job, T>>,
) -> TipOutputCell<'job, T> {
    if let Some(cell) = cells.iter_mut().find_map(|cell| {
        Arc::get_mut(cell)?;
        Some(Arc::clone(cell))
    }) {
        return cell;
    }
    let cell = Arc::new(Mutex::new(None));
    cells.push(Arc::clone(&cell));
    cell
}

impl<T: ScheduledScratchSample + Send + 'static> ParkedTipOutput<'_, T> {
    fn run(self) {
        let Some(TipOutputTask {
            job,
            mut products,
            motion,
            finish,
            scratch,
            scratch_done,
        }) = self.0.lock().take()
        else {
            return;
        };
        let mut scratch = scratch
            .as_deref()
            .and_then(CompletionCell::get)
            .and_then(|scratch| T::take_scheduled_scratch(&mut scratch.lock()))
            .unwrap_or_default();
        let progress = finish.progress_handle();
        if let Some(buffers) = progress.buffers() {
            scratch.set_decode_buffers(buffers);
        }
        let planes = progress
            .take_unfiltered_planes()
            .unwrap_or_else(|| scratch.pooled_planes());
        let TipOutputJob {
            candidate,
            core,
            sequence,
            options,
            reference,
            geometry,
        } = job;
        let offset = candidate.offset();
        let early = inter::tip_output_products(
            &core,
            &candidate,
            offset,
            &sequence,
            options,
            &reference,
            &geometry,
            &mut products,
        );
        let reconstructed = early.and_then(|(cdfs, segments)| {
            products.settle(cdfs, None, segments);
            inter::reconstruct_tip_output(
                &mut scratch,
                planes,
                &core,
                &sequence,
                &reference,
                geometry,
                offset,
            )
        });
        match reconstructed {
            Ok((frame, field)) => {
                motion.publish(field);
                finish.complete_frame(frame);
            }
            Err(error) => {
                motion.fail();
                finish.fail(error);
            }
        }
        let _ = scratch_done.set(Mutex::new(Some(T::wrap_scheduled_scratch(scratch))));
    }
}

/// Admits one reference-gated TIP output reconstruction without stopping the
/// frame driver at its pixel-reference barrier.
#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_tip_output<'job, 'scope, T>(
    job: TipOutputJob<'job, T>,
    cell: TipOutputCell<'job, T>,
    frame_index: usize,
    dependencies: &inter::TipOutputDependencies<T>,
    products: inter::FrameProductWriters,
    motion: inter::MotionFieldHandle,
    finish: PendingFinish<T>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    lane: &mut ReconAdmissionLane,
) where
    T: ScheduledScratchSample + Send + 'static,
    'job: 'scope,
{
    let (scratch, scratch_done) = match lane.reserve_recon() {
        Ok(reservation) => reservation,
        Err(error) => {
            drop(products);
            motion.fail();
            finish.fail(error);
            return;
        }
    };
    let mut conditions = dependencies
        .conditions()
        .chain(scratch.as_deref().map(Condition::completion));
    let order_key = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    *cell.lock() = Some(TipOutputTask {
        job,
        products,
        motion,
        finish,
        scratch: scratch.clone(),
        scratch_done,
    });
    scheduler.submit_iter(
        scope,
        order_key,
        &mut conditions,
        splot_parallel::Job::Inline(T::tip_output_task(ParkedTipOutput(cell))),
    );
}

type LaneReservation<T> = (Option<Arc<CompletionCell<T>>>, Arc<CompletionCell<T>>);

/// The frame-context admission bound for scheduled reconstruction.
pub(super) struct ReconAdmissionLane {
    depth: usize,
    recon: VecDeque<ReconScratchSlot>,
    filters: VecDeque<Arc<CompletionCell<()>>>,
    recon_cells: Vec<ReconScratchSlot>,
    filter_cells: Vec<Arc<CompletionCell<()>>>,
}

impl ReconAdmissionLane {
    pub(super) fn new(depth: usize) -> Self {
        let depth = depth.max(1);
        // Two identities per context (2D), in-flight job (D), and worker (W),
        // plus D queued identities and the reservation in construction.

        let cells = depth
            .saturating_mul(7)
            .saturating_add(
                splot_parallel::current_pool_width()
                    .max(1)
                    .saturating_mul(2),
            )
            .saturating_add(1);
        Self {
            depth,
            recon: VecDeque::with_capacity(depth),
            filters: VecDeque::with_capacity(depth),
            recon_cells: (0..cells)
                .map(|_| Arc::new(CompletionCell::new()))
                .collect(),
            filter_cells: (0..cells)
                .map(|_| Arc::new(CompletionCell::new()))
                .collect(),
        }
    }

    /// Whether every gate this lane would hand the next frame has settled.
    pub(super) fn is_settled(&self) -> bool {
        self.recon.iter().all(|cell| cell.is_set()) && self.filters.iter().all(|cell| cell.is_set())
    }

    fn reserve<T>(
        depth: usize,
        lane: &mut VecDeque<Arc<CompletionCell<T>>>,
        cells: &mut [Arc<CompletionCell<T>>],
    ) -> Result<LaneReservation<T>> {
        let done = cells
            .iter_mut()
            .find_map(|cell| {
                Arc::get_mut(cell)?.reset();
                Some(Arc::clone(cell))
            })
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        let gate = if lane.len() == depth {
            lane.pop_front()
        } else {
            None
        };
        lane.push_back(Arc::clone(&done));
        Ok((gate, done))
    }

    fn reserve_recon(&mut self) -> Result<(Option<ReconScratchSlot>, ReconScratchSlot)> {
        Self::reserve(self.depth, &mut self.recon, &mut self.recon_cells)
    }

    fn reserve_filter(&mut self) -> Result<LaneReservation<()>> {
        Self::reserve(self.depth, &mut self.filters, &mut self.filter_cells)
    }
}

pub(super) fn schedule_finish<'job, 'scope, T: super::inflight::SpareFramePlanes + Send>(
    finish: PendingFinish<T>,
    walked: super::frame_engine::finish::WalkedFrame<T>,
    frame_index: usize,
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    scheduler: &'scope AdmissionScheduler<'job, FrameTask<'job>>,
    lane: &mut ReconAdmissionLane,
    ring: &mut InflightRing,
) where
    'job: 'scope,
{
    let (gate, done) = match lane.reserve_filter() {
        Ok(reservation) => reservation,
        Err(error) => {
            finish.fail(error);
            return;
        }
    };
    let conditions = gate.as_deref().map(Condition::completion);
    let order_base = u64::try_from(frame_index)
        .unwrap_or(u64::MAX / ORDER_KEY_FRAME_STRIDE)
        .saturating_mul(ORDER_KEY_FRAME_STRIDE);
    scheduler.submit(
        scope,
        order_base + u64::from(u32::MAX),
        conditions.as_slice(),
        splot_parallel::Job::Inline(T::finish_task(ring.park_finish(finish, walked, done))),
    );
}

/// Shares the driver's active sequence header with the frames it defers.
///
/// A deferred frame's reconstruction outlives the driver's borrow of the
/// header, and the header only ever changes at a frame the driver flushes
/// before, so the shared copy is made once per activation.
pub(super) fn shared_sequence(
    cached: &mut Option<Arc<splot_core::headers::sequence::SequenceHeader>>,
    sequence: &splot_core::headers::sequence::SequenceHeader,
) -> Arc<splot_core::headers::sequence::SequenceHeader> {
    Arc::clone(cached.get_or_insert_with(|| Arc::new(sequence.clone())))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn recon_lane_returns_the_same_typed_context_at_bounded_depth() {
        let mut lane = ReconAdmissionLane::new(1);
        let (prior, first) = lane.reserve_recon().expect("resident cell");
        assert!(prior.is_none());
        assert!(
            first
                .set(Mutex::new(Some(ScheduledReconScratch::Ten(
                    inter::InterDecodeScratch::default(),
                ))))
                .is_ok()
        );

        let (prior, _) = lane.reserve_recon().expect("resident cell");
        assert_eq!(lane.recon.len(), 1);
        let prior = prior.expect("prior context");
        let stored = prior.get().expect("settled prior context");
        let mut stored = stored.lock();
        assert!(u8::take_scheduled_scratch(&mut stored).is_none());
        assert!(u16::take_scheduled_scratch(&mut stored).is_some());
    }

    #[test]
    fn failed_recon_context_still_settles_the_lane() {
        let mut lane = ReconAdmissionLane::new(1);
        let (_, failed) = lane.reserve_recon().expect("resident cell");
        assert!(failed.set(Mutex::new(None)).is_ok());

        let (prior, _) = lane.reserve_recon().expect("resident cell");
        assert!(
            prior
                .expect("failed prior context")
                .get()
                .expect("settled failure")
                .lock()
                .is_none()
        );
    }
    #[test]
    fn admission_cells_reuse_only_after_readers_retire() {
        let mut lane = ReconAdmissionLane::new(1);
        let addresses = lane
            .filter_cells
            .iter()
            .map(Arc::as_ptr)
            .collect::<Vec<_>>();
        let mut readers = VecDeque::new();
        for _ in 0..1200 {
            let (prior, done) = lane.reserve_filter().expect("resident cell");
            assert!(done.get().is_none());
            assert!(addresses.contains(&Arc::as_ptr(&done)));
            if let Some(prior) = prior {
                assert!(prior.get().is_some());
                readers.push_back(prior);
            }
            assert!(done.set(()).is_ok());
            while readers.len() > 2 {
                readers.pop_front();
            }
            assert!(readers.iter().all(|reader| reader.get().is_some()));
        }
        assert_eq!(
            lane.filter_cells
                .iter()
                .map(Arc::as_ptr)
                .collect::<Vec<_>>(),
            addresses
        );
        drop(readers);
        let retained = lane.filter_cells.clone();
        let previous = Arc::as_ptr(&lane.filters[0]);
        assert!(lane.reserve_filter().is_err());
        assert_eq!(Arc::as_ptr(&lane.filters[0]), previous);
        drop(retained);
        assert!(lane.reserve_filter().is_ok());
    }
}
