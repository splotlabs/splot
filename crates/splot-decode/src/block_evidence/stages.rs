// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::cell::RefCell;
use std::sync::Arc;

use splot_recon::{
    BitDepth, CurrentFrameSurface, IntraRectBlockSize, PlaneId, PlaneRect, ReconSample,
};

use super::{BlockEvidenceState, BlockEvidenceStatus};

/// Sample boundaries in the AV2 decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeStage {
    /// Prediction in destination sample units before residual addition.
    Prediction,
    /// Signed decoded residual after inverse-transform rounding and before clipped addition.
    Residual,
    /// Reconstructed samples before in-loop filters.
    Reconstruction,
    /// Samples after deblocking.
    Deblock,
    /// Samples after the constrained directional enhancement filter.
    Cdef,
    /// Samples after cross-component sample offset.
    Ccso,
    /// Samples after loop restoration and before guided detail filtering.
    Restoration,
    /// Samples after guided detail filtering.
    Gdf,
    /// Final reference samples, before any display film grain.
    Reference,
}

const STAGES: [DecodeStage; 9] = [
    DecodeStage::Prediction,
    DecodeStage::Residual,
    DecodeStage::Reconstruction,
    DecodeStage::Deblock,
    DecodeStage::Cdef,
    DecodeStage::Ccso,
    DecodeStage::Restoration,
    DecodeStage::Gdf,
    DecodeStage::Reference,
];

/// Whether the decoder executed and captured this stage.
///
/// Filter status describes the plane and the filter stripes that intersect
/// the region. An applied stripe can contain edges or blocks that skip filtering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StageEvidenceStatus {
    /// The stage branch ran and the decoder captured the selected region.
    Applied,
    /// The decoder bypassed the stage in all captured portions.
    Skipped,
    /// This decode path has no hook for the stage.
    Unsupported,
    /// The capture did not cover the region.
    Uncaptured,
}

/// Samples at one stage for a region in native plane coordinates.
#[derive(Debug)]
#[non_exhaustive]
pub struct StageEvidence {
    /// Stage that produced the samples.
    pub stage: DecodeStage,
    /// Capture status.
    pub status: StageEvidenceStatus,
    /// Region plane.
    pub plane: PlaneId,
    /// Visible plane rectangle in the same sample grid.
    pub plane_extent: PlaneRect,
    /// Captured rectangle in plane samples.
    pub region: Option<PlaneRect>,
    /// Decoder sample depth.
    pub bit_depth: BitDepth,
    /// Exact samples in raster order. Only residual samples can be negative.
    pub samples: Vec<i32>,
}

const UNCOVERED_SAMPLE: i32 = i32::MIN;

#[derive(Default)]
struct StageBuffer {
    samples: Vec<i32>,
    covered: usize,
    skipped: bool,
    applied: bool,
}

pub(super) struct StageSamples {
    pub(super) plane: PlaneId,
    pub(super) region: PlaneRect,
    extent: PlaneRect,
    buffers: [StageBuffer; STAGES.len()],
}

impl StageSamples {
    pub(super) fn new(plane: PlaneId, region: PlaneRect, extent: PlaneRect) -> Self {
        Self {
            plane,
            region,
            extent,
            buffers: Default::default(),
        }
    }

    pub(super) fn bytes(&self) -> usize {
        self.buffers
            .iter()
            .map(|b| b.samples.capacity() * size_of::<i32>())
            .sum()
    }

    /// Records the portion of a stage that intersects the selected region.
    pub(super) fn record(
        &mut self,
        stage: DecodeStage,
        source: PlaneRect,
        applied: bool,
        budget: usize,
        mut sample: impl FnMut(usize, usize) -> Option<i32>,
    ) -> Result<(), BlockEvidenceStatus> {
        let Some(overlap) = intersect(self.region, source) else {
            return Ok(());
        };
        let budget = budget.saturating_sub(self.bytes());
        let buf = &mut self.buffers[stage as usize];
        buf.applied |= applied;
        buf.skipped |= !applied;
        let count = self
            .region
            .width()
            .checked_mul(self.region.height())
            .ok_or(BlockEvidenceStatus::Truncated)?;
        if buf.samples.is_empty() {
            if count > budget / size_of::<i32>() {
                return Err(BlockEvidenceStatus::Truncated);
            }
            buf.samples
                .try_reserve_exact(count)
                .map_err(|_| BlockEvidenceStatus::AllocationFailed)?;
            buf.samples.resize(count, UNCOVERED_SAMPLE);
        }
        for y in overlap.y()..(overlap.y() + overlap.height()) {
            for x in overlap.x()..(overlap.x() + overlap.width()) {
                let dst = (y - self.region.y()) * self.region.width() + x - self.region.x();
                if let Some(value) = sample(x, y) {
                    buf.covered += usize::from(buf.samples[dst] == UNCOVERED_SAMPLE);
                    buf.samples[dst] = value;
                }
            }
        }
        Ok(())
    }

    pub(super) fn finish(
        self,
        bit_depth: BitDepth,
        walked: bool,
        interrupted: bool,
    ) -> Result<Vec<StageEvidence>, BlockEvidenceStatus> {
        let mut reports = Vec::new();
        reports
            .try_reserve_exact(STAGES.len())
            .map_err(|_| BlockEvidenceStatus::AllocationFailed)?;
        for (stage, buf) in STAGES.into_iter().zip(self.buffers) {
            let captured = !buf.samples.is_empty() && buf.covered == buf.samples.len();
            let status = if captured {
                if buf.applied {
                    StageEvidenceStatus::Applied
                } else {
                    StageEvidenceStatus::Skipped
                }
            } else if !interrupted
                && !buf.applied
                && !buf.skipped
                && (!walked || matches!(stage, DecodeStage::Prediction | DecodeStage::Residual))
            {
                StageEvidenceStatus::Unsupported
            } else {
                StageEvidenceStatus::Uncaptured
            };
            reports.push(StageEvidence {
                stage,
                status,
                plane: self.plane,
                plane_extent: self.extent,
                region: captured.then_some(self.region),
                bit_depth,
                samples: if captured { buf.samples } else { Vec::new() },
            });
        }
        Ok(reports)
    }
}

pub(super) fn intersect(a: PlaneRect, b: PlaneRect) -> Option<PlaneRect> {
    let x = a.x().max(b.x());
    let y = a.y().max(b.y());
    PlaneRect::new(
        x,
        y,
        a.x()
            .saturating_add(a.width())
            .min(b.x().saturating_add(b.width()))
            .checked_sub(x)?,
        a.y()
            .saturating_add(a.height())
            .min(b.y().saturating_add(b.height()))
            .checked_sub(y)?,
    )
    .ok()
}

pub(super) fn reference<T: ReconSample>(
    state: &super::BlockEvidenceState,
    frame: &splot_recon::DecodedFrame<T>,
) {
    let Some((plane, _)) = state.request.stage_samples else {
        return;
    };
    let Some(plane_ref) = frame.plane(plane) else {
        return;
    };
    let bounds = plane_ref.visible_rect();
    state.prepare_reference_stage(plane, bounds, frame.bit_depth());
    state.record_stage(DecodeStage::Reference, plane, bounds, true, |x, y| {
        plane_ref
            .samples()
            .get(y * plane_ref.stride_samples() + x)
            .map(|value| i32::from(value.to_u16()))
    });
}

thread_local! {
    static RECON_CAPTURE: RefCell<Option<Arc<BlockEvidenceState>>> = const { RefCell::new(None) };
}

pub(crate) struct ReconStageScope(Option<Arc<BlockEvidenceState>>);

impl ReconStageScope {
    pub(crate) fn install(capture: Arc<BlockEvidenceState>) -> Self {
        Self(RECON_CAPTURE.replace(Some(capture)))
    }
}

impl Drop for ReconStageScope {
    fn drop(&mut self) {
        RECON_CAPTURE.replace(self.0.take());
    }
}

pub(crate) fn selected_plane() -> Option<PlaneId> {
    RECON_CAPTURE.with_borrow(|state| Some(state.as_ref()?.stage_region()?.0))
}

pub(crate) fn captures_block(plane: PlaneId, rect: Option<PlaneRect>) -> bool {
    RECON_CAPTURE.with_borrow(|state| {
        state
            .as_ref()
            .and_then(|state| state.stage_region())
            .is_some_and(|(selected, region)| {
                plane == selected && rect.and_then(|rect| intersect(rect, region)).is_some()
            })
    })
}

pub(crate) fn block_rect(x: usize, y: usize, size: IntraRectBlockSize) -> Option<PlaneRect> {
    PlaneRect::new(x, y, size.width(), size.height()).ok()
}

pub(crate) fn transform_samples(
    stage: DecodeStage,
    plane: PlaneId,
    rect: Option<PlaneRect>,
    applied: bool,
    mut sample: impl FnMut(usize, usize) -> Option<i32>,
) {
    if let Some(rect) = rect
        && let Some(state) = RECON_CAPTURE.with_borrow(Clone::clone)
    {
        state.record_stage(stage, plane, rect, applied, |x, y| {
            sample(x - rect.x(), y - rect.y())
        });
    }
}

pub(crate) fn skip_inter_residual(
    block: &crate::prediction::inter::InterResidualBlock,
    (log2_width, log2_height): (u32, u32),
) {
    if let Ok(size) = IntraRectBlockSize::new(log2_width as u8, log2_height as u8) {
        skip_residual(
            block.plane,
            block_rect(usize::from(block.x), usize::from(block.y), size),
        );
    }
}

pub(crate) fn skip_residual(plane: PlaneId, rect: Option<PlaneRect>) {
    transform_samples(DecodeStage::Residual, plane, rect, false, |_, _| Some(0));
}

pub(crate) fn surface_prediction<T: ReconSample>(
    sink: &CurrentFrameSurface<'_, '_, T>,
    plane: PlaneId,
    rect: PlaneRect,
) {
    let Some(state) = RECON_CAPTURE.with_borrow(Clone::clone) else {
        return;
    };
    let Some((selected, region)) = state.stage_region() else {
        return;
    };
    if selected != plane {
        return;
    }
    let Some(area) = intersect(rect, region) else {
        return;
    };
    let Ok(mut rows) = sink.rect_rows(plane, area) else {
        return;
    };
    let mut row = rows.next();
    state.record_stage(DecodeStage::Prediction, plane, area, true, |x, _| {
        let x = x - area.x();
        let value = row?.get(x).map(|v| i32::from(v.to_u16()));
        if x + 1 == area.width() {
            row = rows.next();
        }
        value
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_requires_full_region_coverage() {
        let area = PlaneRect::new(0, 0, 2, 1).unwrap();
        let left = PlaneRect::new(0, 0, 1, 1).unwrap();
        let right = PlaneRect::new(1, 0, 1, 1).unwrap();
        for complete in [false, true] {
            let mut stages = StageSamples::new(PlaneId::Y, area, area);
            for _ in 0..2 {
                stages
                    .record(DecodeStage::Residual, left, false, 8, |_, _| Some(0))
                    .unwrap();
            }
            if complete {
                stages
                    .record(DecodeStage::Residual, right, true, 8, |_, _| Some(-8))
                    .unwrap();
            }
            let reports = stages.finish(BitDepth::Ten, true, false).unwrap();
            let residual = reports
                .iter()
                .find(|record| record.stage == DecodeStage::Residual)
                .unwrap();
            if complete {
                assert_eq!(residual.status, StageEvidenceStatus::Applied);
                assert_eq!(residual.samples, [0, -8]);
                assert_eq!(residual.region, Some(area));
            } else {
                assert_eq!(residual.status, StageEvidenceStatus::Uncaptured);
                assert!(residual.samples.is_empty());
                assert!(residual.region.is_none());
            }
        }
    }
}
