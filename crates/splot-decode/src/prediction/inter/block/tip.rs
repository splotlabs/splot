// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::DecodeHeaderStateError;
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_recon::{CurrentFrameWorkspace, DecodedFrame, ReconError};

use super::super::reference::{HeldFrameSamples, ReferenceSamples};

use super::super::find_mv_stack::TemporalMotionBlock;

#[doc = "AV2 § 7.13.3.1 Tip_Weighting_Factor."]
const TIP_WEIGHTING_FACTORS: [i16; 8] = [8, 12, 16, 18, 20, 4, 6, -4];
const TIP_SINGLE_WEIGHT: i16 = 16;

fn tip_prediction_controls(
    inter: Option<&splot_core::headers::frame::InterControl>,
) -> Result<(
    &splot_core::headers::frame::InterControl,
    TipFrameMode,
    i16,
    u32,
)> {
    let inter = inter.ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let mode = inter
        .tip_frame_mode
        .filter(|mode| matches!(mode, TipFrameMode::AsRef | TipFrameMode::AsOutput))
        .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
    let weight_index = usize::from(
        inter
            .tip_global_wtd_index
            .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?,
    );
    let weight = TIP_WEIGHTING_FACTORS
        .get(weight_index)
        .copied()
        .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
    let opfl_refine_type = inter
        .opfl_refine_type
        .filter(|&value| value <= 2)
        .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
    Ok((inter, mode, weight, opfl_refine_type))
}

#[derive(Debug)]
struct TipUnit {
    rect: mc::McBlockRect,
    has_chroma: bool,
    mvs: [Mv; 2],
}

#[derive(Clone, Copy)]
struct TipPrediction<'a, T: ReconSample> {
    reference0: ReferenceSamples<'a, T>,
    reference1: Option<ReferenceSamples<'a, T>>,
    interpolation_filter: ReconInterpolationFilter,
    blend: mc::CompoundBlend,
    optflow_distances: Option<[i32; 2]>,
    use_refinemv: bool,
    search_refinemv: bool,
    optflow_sad_threshold: Option<u32>,
}

impl<'a, T: ReconSample> TipPrediction<'a, T> {
    fn block_params(&self, unit: &TipUnit) -> mc::InterBlockParams<'a, T> {
        let params = if let Some(reference1) = self.reference1 {
            mc::InterBlockParams::compound_average(
                self.reference0,
                reference1,
                unit.rect,
                unit.mvs[0],
                unit.mvs[1],
                self.interpolation_filter,
                self.blend,
            )
            .with_optflow_distances(self.optflow_distances)
        } else {
            mc::InterBlockParams::single(
                self.reference0,
                unit.rect,
                unit.mvs[0],
                self.interpolation_filter,
            )
        };
        params
            .with_chroma(unit.has_chroma)
            .with_refinemv(self.use_refinemv)
            .with_refinemv_search(self.search_refinemv)
            .with_optflow_sad_threshold(self.optflow_sad_threshold)
    }
}

/// The reference pair and § 7.13.3.2 prediction settings one TIP block reads
/// through, resolved once so the borrows can be retaken per unit batch.
#[derive(Clone, Copy, Debug)]
struct TipReferencePlan {
    past: u32,
    future: Option<u32>,
    interpolation_filter: ReconInterpolationFilter,
    blend: mc::CompoundBlend,
    optflow_distances: Option<[i32; 2]>,
    use_refinemv: bool,
    search_refinemv: bool,
    optflow_sad_threshold: Option<u32>,
}

/// One TIP block's borrow of its reference pair.
struct TipHeldReferences<'a, T: ReconSample> {
    past: HeldFrameSamples<'a, T>,
    /// The future reference's borrow, absent when it names the past slot.
    future: Option<HeldFrameSamples<'a, T>>,
}

impl TipReferencePlan {
    /// Borrows the reference pair for one block's prediction units.
    fn hold<'a, T: ReconSample>(
        &self,
        reference: &'a InterReferenceState<T>,
    ) -> Result<TipHeldReferences<'a, T>> {
        let (past, future) = super::super::hold_reference_pair(reference, self.past, self.future)?;
        Ok(TipHeldReferences { past, future })
    }
}

impl<T: ReconSample> TipHeldReferences<'_, T> {
    /// Resolves the borrow into the samples the block's units read.
    fn prediction(&self, plan: &TipReferencePlan) -> Result<TipPrediction<'_, T>> {
        let compound_reference = match self.future.as_ref() {
            Some(future) => future,
            None => &self.past,
        };
        Ok(TipPrediction {
            reference0: self.past.samples()?,
            reference1: plan
                .future
                .is_some()
                .then(|| compound_reference.samples())
                .transpose()?,
            interpolation_filter: plan.interpolation_filter,
            blend: plan.blend,
            optflow_distances: plan.optflow_distances,
            use_refinemv: plan.use_refinemv,
            search_refinemv: plan.search_refinemv,
            optflow_sad_threshold: plan.optflow_sad_threshold,
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct TipReconstructScratch {
    units: Vec<TipUnit>,
    unit_mvs: Vec<[Mv; 2]>,
    /// The per-unit refine-MV candidates a batch's motion grid holds.
    ///
    /// One batch grid is live at a time, so the list travels into the grid and
    /// comes back here as each unit is published.
    grid_candidates: Vec<[Mv; 2]>,
    motion_plan: Option<TipBlockPlan>,
    /// Each unit's [`whole_sample_mvs`] from the motion half, so a fresh
    /// prediction skips the refine-MV search it already ran.
    whole_mvs: Vec<Option<[Mv; 2]>>,
    search: mc::TipSearchScratch,
}

/// The TIP output walk's reusable state: one entry per full-width row band.
#[derive(Debug, Default)]
pub(crate) struct TipOutputScratch {
    bands: Vec<parking_lot::Mutex<TipOutputBand>>,
    rects: Vec<splot_recon::PlaneRect>,
    rows: Vec<(usize, usize)>,
    motion: Option<super::temporal::MotionFieldUnits>,
}

/// The storage TIP output frames of one decode hand back when they finish,
/// shared by every TIP work so the frames in flight bound it, not the slots.
#[derive(Debug, Default)]
pub(crate) struct TipSpares {
    temporal: parking_lot::Mutex<Vec<TemporalMvContext>>,
    output: parking_lot::Mutex<Vec<TipOutputScratch>>,
}

impl TipSpares {
    pub(crate) fn take(&self) -> (TemporalMvContext, TipOutputScratch) {
        (
            self.temporal
                .lock()
                .pop()
                .unwrap_or_else(TemporalMvContext::empty),
            self.output.lock().pop().unwrap_or_default(),
        )
    }

    pub(crate) fn give(&self, temporal: TemporalMvContext, output: TipOutputScratch) {
        self.temporal.lock().push(temporal);
        self.output.lock().push(output);
    }
}

/// One TIP output band's scratch, the motion records it produced and the
/// surface it predicts into, until the band is committed.
#[derive(Debug, Default)]
struct TipOutputBand {
    scratch: TipReconstructScratch,
    records: Vec<TemporalMotionBlock>,
}

const fn tip_uses_two_references(weight: i16) -> bool {
    weight != TIP_SINGLE_WEIGHT
}

const fn tip_uses_refinemv(
    output: bool,
    enable_refinemv: bool,
    enable_tip_refinemv: bool,
    weight: i16,
) -> bool {
    !output && enable_refinemv && enable_tip_refinemv && weight == mc::CWP_EQUAL
}

#[doc = "AV2 § 7.13.3.1 optical-flow gate: equal weight for a TIP block (§ 7.10.6 output skips it) and `opfl_allowed_for_refs` (§ 5.20.7.6) always."]
const fn tip_optflow_weight_and_references_allowed(
    output: bool,
    weight: i16,
    references_allowed: bool,
) -> bool {
    (output || weight == mc::CWP_EQUAL) && references_allowed
}

#[doc = "AV2 § 5.20.7.17 equal-distance reference gate for refine-MV search."]
const fn tip_refinemv_offsets_allowed(past_offset: i32, future_offset: i32) -> bool {
    past_offset != 0 && past_offset == -future_offset
}

#[doc = "AV2 § 5.20.7.14 opposite-direction reference gate for optical flow."]
const fn tip_optflow_offsets_allowed(past_offset: i32, future_offset: i32) -> bool {
    (past_offset <= 0) != (future_offset <= 0)
}

#[doc = "AV2 § 5.20.7.14 and § 5.20.7.17 unscaled-reference gate of refine-MV search and optical flow."]
fn tip_references_unscaled(
    frame_type: Option<FrameType>,
    frame_size: Option<splot_core::headers::frame::FrameSize>,
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    ref_frame_width: &[u32],
    ref_frame_height: &[u32],
    references: [i8; 2],
) -> bool {
    if frame_type == Some(FrameType::Switch) {
        return false;
    }
    let Some(frame_size) = frame_size else {
        return false;
    };
    let no_scale = 1_u32 << 14;
    references.into_iter().all(|ref_frame| {
        usize::try_from(ref_frame)
            .ok()
            .and_then(|index| ref_frame_idx.get(index))
            .and_then(|&slot| usize::try_from(slot).ok())
            .is_some_and(|slot| {
                let scale = |reference: &[u32], current: u32| {
                    current != 0
                        && reference.get(slot).is_some_and(|&dimension| {
                            dimension
                                .checked_mul(no_scale)
                                .and_then(|scaled| scaled.checked_add(current / 2))
                                .is_some_and(|scaled| scaled / current == no_scale)
                        })
                };
                ref_order_hint
                    .get(slot)
                    .is_some_and(|&order_hint| order_hint != u32::MAX)
                    && scale(ref_frame_width, frame_size.width)
                    && scale(ref_frame_height, frame_size.height)
            })
    })
}

#[doc = "AV2 § 7.13.3.1 tipSize selection for TIP prediction."]
const fn prediction_unit_size(width: usize, height: usize, enable_tip_refinemv: bool) -> usize {
    if (!enable_tip_refinemv && width >= 16 && height >= 16) || (width >= 256 && height >= 256) {
        16
    } else {
        8
    }
}

pub(super) const fn reference_uses_16x16_units(
    n4w: usize,
    n4h: usize,
    enable_tip_refinemv: bool,
) -> bool {
    prediction_unit_size(n4w * 4, n4h * 4, enable_tip_refinemv) == 16
}

#[doc = "AV2 § 7.10.6 TIP-as-output prediction-unit size."]
const fn output_prediction_unit_size(
    enable_tip_refinemv: bool,
    interpolation_filter: ReconInterpolationFilter,
) -> usize {
    if enable_tip_refinemv
        && matches!(
            interpolation_filter,
            ReconInterpolationFilter::EightTapSharp
        )
    {
        8
    } else {
        16
    }
}

fn prediction_unit_extent(output: bool, remaining: usize, unit_size: usize) -> usize {
    if output {
        unit_size
    } else {
        remaining.min(unit_size)
    }
}

fn output_interpolation_filter(
    inter: &splot_core::headers::frame::InterControl,
) -> Result<ReconInterpolationFilter> {
    match inter.tip_interpolation_filter {
        Some(splot_core::headers::frame::InterpolationFilter::Eighttap) => {
            Ok(ReconInterpolationFilter::EightTap)
        }
        Some(splot_core::headers::frame::InterpolationFilter::EighttapSmooth) => {
            Ok(ReconInterpolationFilter::EightTapSmooth)
        }
        Some(splot_core::headers::frame::InterpolationFilter::EighttapSharp) => {
            Ok(ReconInterpolationFilter::EightTapSharp)
        }
        _ => Err(DecodeHeaderStateError::IncompleteTipOutput.into()),
    }
}

pub(super) fn prepare_motion_field(
    temporal: &mut TemporalMvContext,
    core: &FrameHeaderCore,
    sb_h4: usize,
    references: Option<super::TipReferencePair>,
) -> Result<()> {
    let projection_step = tmvp_projection_step(core);
    let tmvp_unit_size8 = tmvp_unit_size8(projection_step, sb_h4);
    let Some(inter) = core.inter.as_ref() else {
        temporal.fill_sampling_gaps(projection_step, tmvp_unit_size8);
        return Ok(());
    };
    let tip_mode = inter
        .tip_frame_mode
        .ok_or(DecodeHeaderStateError::IncompleteInterFrameTools)?;
    if tip_mode == TipFrameMode::Disabled {
        temporal.fill_sampling_gaps(projection_step, tmvp_unit_size8);
        return Ok(());
    }
    let references = references.ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
    let fill_holes = inter
        .allow_tip_hole_fill
        .ok_or(DecodeHeaderStateError::IncompleteInterFrameTools)?;
    temporal.prepare_tip(references, projection_step, tmvp_unit_size8, fill_holes)?;
    Ok(())
}

pub(super) fn tmvp_projection_step(core: &FrameHeaderCore) -> usize {
    core.inter.as_ref().map_or(1, |inter| {
        usize::from(inter.tmvp_sample_step_minus_1.unwrap_or(false)) + 1
    })
}

pub(super) fn tmvp_unit_size8(projection_step: usize, sb_h4: usize) -> usize {
    if projection_step == 1 || sb_h4 == 16 {
        8
    } else {
        16
    }
}

pub(super) fn read_reference(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tip_frame_mode: TipFrameMode,
    frontier: &DecodeBlockFrontier,
    neighbour_ctx: &BlockNeighbourContext,
    n4: (usize, usize),
) -> bool {
    let (n4w, n4h) = n4;
    if tip_frame_mode == TipFrameMode::Disabled || !tip_block_allowed(frontier, n4w, n4h) {
        return false;
    }
    let rows = &mut cdfs.block_rows_mut().tip_mode;
    read_row(symbols, row_at(rows, neighbour_ctx.tip_mode_ctx())) != 0
}

pub(super) fn tip_block_allowed(frontier: &DecodeBlockFrontier, n4w: usize, n4h: usize) -> bool {
    tip_allowed_for_block_indices(
        frontier.chroma_offset,
        frontier.is_luma_part(),
        frontier.is_chroma_part(),
        frontier.b_size.index(),
        frontier.chroma_ref_geometry().size().index(),
        n4w,
        n4h,
    )
}

pub(crate) fn tip_allowed_for_block_indices(
    chroma_offset: bool,
    is_luma_part: bool,
    is_chroma_part: bool,
    mi_size: usize,
    chroma_mi_size: usize,
    n4w: usize,
    n4h: usize,
) -> bool {
    !chroma_offset
        && !is_luma_part
        && !is_chroma_part
        && mi_size == chroma_mi_size
        && n4w >= 2
        && n4h >= 2
}

/// The frame-constant § 7.13.3.2 settings every TIP block of a frame shares.
#[derive(Clone, Copy, Debug)]
pub(super) struct TipFramePlan {
    plan: TipReferencePlan,
    output: bool,
    enable_tip_refinemv: bool,
    optflow_allowed: bool,
    frame_size: splot_recon::PlaneSize,
    current_order_hint: u32,
    ref_order_hints: [Option<u32>; 2],
}

/// The § 7.13.3.2 settings both halves of one TIP block derive from.
///
/// The motion half leaves its plan with its units in the scratch for a
/// prediction that follows at once; a later one derives its own.
#[derive(Debug)]
struct TipBlockPlan {
    plan: TipReferencePlan,
    output: bool,
    use_optflow: bool,
    unit_size: usize,
    unit_count: usize,
    block_w: usize,
    block_h: usize,
    batch_rect: mc::McBlockRect,
    batch_has_chroma: bool,
    frame_mi_rows: usize,
    frame_mi_cols: usize,
}

impl TipBlockPlan {
    const fn batched(&self) -> bool {
        self.use_optflow
    }

    const fn columns(&self) -> usize {
        self.block_w / 8
    }

    const fn unit_rect(&self, index: usize) -> mc::McBlockRect {
        let columns = self.columns();
        mc::McBlockRect::from_luma_rect(
            self.batch_rect.luma_x + index % columns * 8,
            self.batch_rect.luma_y + index / columns * 8,
            8,
            8,
        )
    }

    /// The whole batched block predicted with `mvs`, its first unit's candidates.
    const fn batch_unit(&self, mvs: [Mv; 2]) -> TipUnit {
        TipUnit {
            rect: self.batch_rect,
            has_chroma: self.batch_has_chroma,
            mvs,
        }
    }

    /// Borrows the block's reference pair.
    fn hold<'a, T: ReconSample>(
        &self,
        reference: &'a InterReferenceState<T>,
    ) -> Result<TipHeldReferences<'a, T>> {
        self.plan.hold(reference)
    }

    /// The § 7.22 temporal motion record of the luma rectangle `rect`.
    fn temporal_record(
        &self,
        frame: &TipFramePlan,
        rect: &mc::McBlockRect,
        stored_mvs: [Mv; 2],
    ) -> TemporalMotionBlock {
        TemporalMotionBlock::new(
            rect.luma_y / 4,
            rect.luma_x / 4,
            rect.luma_w.div_ceil(4),
            rect.luma_h.div_ceil(4),
            self.frame_mi_rows,
            self.frame_mi_cols,
            frame.current_order_hint,
            frame.ref_order_hints,
            stored_mvs,
            [None, None],
        )
    }
}

impl TipFramePlan {
    pub(super) fn new<T: ReconSample>(
        info: splot_recon::DecodedFrameInfo,
        temporal: &TemporalMvContext,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        reference: &InterReferenceState<T>,
    ) -> Result<Self> {
        let references = temporal
            .tip_references()
            .ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        let (past_offset, future_offset) = (references.past_offset, references.future_offset);
        let (inter, mode, weight, opfl_refine_type) = tip_prediction_controls(core.inter.as_ref())?;
        let tools = sequence.inter.as_ref();
        let implicit_mask = tools.is_some_and(|tools| tools.enable_imp_msk_bld);
        let blend = mc::CompoundBlend::average_with_implicit_mask(implicit_mask)
            .average_with_cwp_weight(weight);
        let two_references = tip_uses_two_references(weight);
        let enable_tip_refinemv = tools.is_some_and(|tools| tools.enable_tip_refinemv);
        let output = mode == TipFrameMode::AsOutput;
        let unscaled = tip_references_unscaled(
            core.frame_type,
            core.frame_size,
            ref_frame_idx,
            &reference.ref_order_hint,
            &reference.ref_frame_width,
            &reference.ref_frame_height,
            [references.past_ref, references.future_ref],
        );
        let use_refinemv = tools.is_some_and(|tools| {
            tip_uses_refinemv(
                output,
                tools.enable_refinemv,
                tools.enable_tip_refinemv,
                weight,
            )
        });
        let search_refinemv =
            use_refinemv && unscaled && tip_refinemv_offsets_allowed(past_offset, future_offset);
        let interpolation_filter = if output {
            output_interpolation_filter(inter)?
        } else {
            ReconInterpolationFilter::EightTapSharp
        };
        let optflow_allowed = opfl_refine_type != 0
            && enable_tip_refinemv
            && interpolation_filter == ReconInterpolationFilter::EightTapSharp
            && two_references
            && tip_optflow_weight_and_references_allowed(
                output,
                weight,
                unscaled && tip_optflow_offsets_allowed(past_offset, future_offset),
            );
        let past = super::super::block_reference_slot(ref_frame_idx, references.past_ref)?;
        let future = two_references
            .then(|| super::super::block_reference_slot(ref_frame_idx, references.future_ref))
            .transpose()?;
        let order_hint = |slot: u32| reference.ref_order_hint.get(slot as usize).copied();
        Ok(Self {
            plan: TipReferencePlan {
                past,
                future,
                interpolation_filter,
                blend,
                optflow_distances: Some([past_offset, future_offset]),
                use_refinemv,
                search_refinemv,
                optflow_sad_threshold: Some(if output { 15 } else { 6 }),
            },
            output,
            enable_tip_refinemv,
            optflow_allowed,
            frame_size: info.storage_luma_size(),
            current_order_hint: core
                .display_order_hint()
                .ok_or(DecodeHeaderStateError::MissingDisplayOrderHint)?,
            ref_order_hints: [order_hint(past), future.and_then(order_hint)],
        })
    }

    fn block(&self, placed: &PlacedInterBlock) -> Result<TipBlockPlan> {
        let unit_size = if self.output {
            output_prediction_unit_size(self.enable_tip_refinemv, self.plan.interpolation_filter)
        } else {
            prediction_unit_size(placed.luma_w(), placed.luma_h(), self.enable_tip_refinemv)
        };
        let use_optflow = unit_size == 8 && self.optflow_allowed;
        let block_w = placed.luma_w().min(
            self.frame_size
                .width()
                .saturating_sub(placed.luma_x())
                .next_multiple_of(unit_size),
        );
        let block_h = placed.luma_h().min(
            self.frame_size
                .height()
                .saturating_sub(placed.luma_y())
                .next_multiple_of(unit_size),
        );
        if block_w == 0 || block_h == 0 {
            return Err(DecodeHeaderStateError::InvalidBlockGeometry.into());
        }
        let unit_count = block_w
            .div_ceil(unit_size)
            .checked_mul(block_h.div_ceil(unit_size))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "TIP prediction unit count",
            })?;
        let mut plan = self.plan;
        if !use_optflow {
            (plan.optflow_distances, plan.optflow_sad_threshold) = (None, None);
        }
        let batch_chroma_x = placed.luma_x().max(placed.chroma_luma_x());
        let batch_chroma_y = placed.luma_y().max(placed.chroma_luma_y());
        let batch_chroma_end_x = placed.luma_x().saturating_add(block_w).min(
            placed
                .chroma_luma_x()
                .saturating_add(placed.chroma_luma_w()),
        );
        let batch_chroma_end_y = placed.luma_y().saturating_add(block_h).min(
            placed
                .chroma_luma_y()
                .saturating_add(placed.chroma_luma_h()),
        );
        Ok(TipBlockPlan {
            plan,
            output: self.output,
            use_optflow,
            unit_size,
            unit_count,
            block_w,
            block_h,
            batch_rect: mc::McBlockRect {
                luma_x: placed.luma_x(),
                luma_y: placed.luma_y(),
                luma_w: block_w,
                luma_h: block_h,
                chroma_luma_x: batch_chroma_x,
                chroma_luma_y: batch_chroma_y,
                chroma_luma_w: batch_chroma_end_x.saturating_sub(batch_chroma_x),
                chroma_luma_h: batch_chroma_end_y.saturating_sub(batch_chroma_y),
            },
            batch_has_chroma: placed.predict_chroma
                && batch_chroma_end_x > batch_chroma_x
                && batch_chroma_end_y > batch_chroma_y,
            frame_mi_rows: self.frame_size.height().div_ceil(4),
            frame_mi_cols: self.frame_size.width().div_ceil(4),
        })
    }
}

/// Fills `units` with the § 7.13.3.1 prediction units of a block that is not
/// batched.
fn build_units(
    units: &mut Vec<TipUnit>,
    plan: &TipBlockPlan,
    placed: &PlacedInterBlock,
    temporal: &TemporalMvContext,
) -> Result<()> {
    units.clear();
    units
        .try_reserve_exact(plan.unit_count)
        .map_err(|_| inter_allocation!("TIP prediction units"))?;
    for local_y in (0..plan.block_h).step_by(plan.unit_size) {
        let luma_y = placed.luma_y() + local_y;
        let candidate = temporal
            .tip_candidate_row(luma_y / 8, placed.block.mv)
            .ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        for local_x in (0..plan.block_w).step_by(plan.unit_size) {
            let luma_x = placed.luma_x() + local_x;
            let luma_w =
                prediction_unit_extent(plan.output, plan.block_w - local_x, plan.unit_size);
            let luma_h =
                prediction_unit_extent(plan.output, plan.block_h - local_y, plan.unit_size);
            let chroma_x = luma_x.max(placed.chroma_luma_x());
            let chroma_y = luma_y.max(placed.chroma_luma_y());
            let chroma_end_x = if plan.output {
                luma_x + luma_w
            } else {
                (luma_x + luma_w).min(placed.chroma_luma_x() + placed.chroma_luma_w())
            };
            let chroma_end_y = if plan.output {
                luma_y + luma_h
            } else {
                (luma_y + luma_h).min(placed.chroma_luma_y() + placed.chroma_luma_h())
            };
            let predict_chroma =
                placed.predict_chroma && chroma_end_x > chroma_x && chroma_end_y > chroma_y;
            let mvs = candidate(luma_x / 8)
                .ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            units.push(TipUnit {
                rect: mc::McBlockRect {
                    luma_x,
                    luma_y,
                    luma_w,
                    luma_h,
                    chroma_luma_x: chroma_x,
                    chroma_luma_y: chroma_y,
                    chroma_luma_w: chroma_end_x.saturating_sub(chroma_x),
                    chroma_luma_h: chroma_end_y.saturating_sub(chroma_y),
                },
                has_chroma: predict_chroma,
                mvs,
            });
        }
    }
    Ok(())
}

/// Fills `unit_mvs` with the candidates of a batched block's 8x8 units, or of
/// its first unit alone when `first_only`.
fn batch_candidates(
    unit_mvs: &mut Vec<[Mv; 2]>,
    plan: &TipBlockPlan,
    placed: &PlacedInterBlock,
    temporal: &TemporalMvContext,
    first_only: bool,
) -> Result<()> {
    unit_mvs.clear();
    unit_mvs
        .try_reserve_exact(if first_only { 1 } else { plan.unit_count })
        .map_err(|_| inter_allocation!("TIP prediction units"))?;
    let (x8, columns) = (placed.luma_x() / 8, plan.columns());
    let rows = if first_only { 1 } else { plan.block_h / 8 };
    for y8 in (placed.luma_y() / 8..).take(rows) {
        let candidate = temporal
            .tip_candidate_row(y8, placed.block.mv)
            .ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        for x8 in (x8..).take(if first_only { 1 } else { columns }) {
            unit_mvs.push(
                candidate(x8).ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?,
            );
        }
    }
    Ok(())
}

/// Derives one § 7.13.3.2 TIP block's motion: the optical-flow grid its units
/// share, and every unit's § 7.22 temporal record.
///
/// This reads reference samples but writes none, so it is the half a motion
/// resolution pass runs. The optical-flow shape stores its grid's refined
/// motion vectors, a refine-MV unit its own § 7.13.3.3 refinement (the § 5.20.4
/// `store_refined_mvs` `RefineMvs`), and every other shape the § 7.11.3
/// candidate the unit was built with.
#[allow(clippy::too_many_arguments)]
pub(super) fn motion<T: ReconSample>(
    scratch: &mut TipReconstructScratch,
    temporal_records: &mut Vec<TemporalMotionBlock>,
    sink: &mc::WorkspaceSink<'_, '_, T>,
    placed: &PlacedInterBlock,
    frame: &TipFramePlan,
    temporal: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    tile_offset: ByteOffset,
) -> Result<Option<mc::CompoundMotionGrid>> {
    let plan = &*scratch.motion_plan.insert(frame.block(placed)?);
    temporal_records
        .try_reserve(plan.unit_count)
        .map_err(|_| inter_allocation!("TIP temporal records"))?;
    if plan.batched() {
        batch_candidates(&mut scratch.unit_mvs, plan, placed, temporal, false)?;
        let grid = tip_motion_grid(
            &scratch.unit_mvs,
            &mut scratch.grid_candidates,
            &mut scratch.search,
            plan,
            sink,
            reference,
            tile_offset,
        )?;
        push_batch_records(temporal_records, plan, frame, &grid)?;
        return Ok(Some(grid));
    }
    build_units(&mut scratch.units, plan, placed, temporal)?;
    let held = plan
        .plan
        .use_refinemv
        .then(|| plan.hold(reference))
        .transpose()?;
    let refining = held
        .as_ref()
        .map(|held| held.prediction(&plan.plan))
        .transpose()?;
    scratch.whole_mvs.clear();
    let first_record = temporal_records.len();
    for unit in &scratch.units {
        let (stored_mvs, whole) = match refining.as_ref() {
            Some(prediction) => match prediction.block_params(unit).into_compound() {
                Some(compound) => {
                    let refined =
                        mc::compound_block_motion_grid(sink, &compound, None, tile_offset)?;
                    let whole = whole_sample_mvs(unit, refined.as_ref());
                    let stored =
                        refined.map_or(Ok(unit.mvs), |refined| refined.stored_mvs_at_index(0))?;
                    (stored, whole)
                }
                None => (unit.mvs, whole_sample_mvs(unit, None)),
            },
            None => (unit.mvs, whole_sample_mvs(unit, None)),
        };
        scratch.whole_mvs.push(whole);
        let record = plan.temporal_record(frame, &unit.rect, stored_mvs);
        TemporalMotionBlock::push_merged(temporal_records, first_record, record);
    }
    Ok(None)
}

/// Pushes one § 7.22 record per run of a batched row's units with equal stored
/// vectors: the records `push_merged` would grow unit by unit.
fn push_batch_records(
    records: &mut Vec<TemporalMotionBlock>,
    plan: &TipBlockPlan,
    frame: &TipFramePlan,
    grid: &mc::CompoundMotionGrid,
) -> Result<()> {
    let columns = plan.columns();
    for row_start in (0..plan.unit_count).step_by(columns.max(1)) {
        let mut run = (row_start, grid.stored_mvs_at_index(row_start)?);
        for index in row_start + 1..=row_start + columns {
            let next = if index < row_start + columns {
                Some(grid.stored_mvs_at_index(index)?)
            } else {
                None
            };
            if next == Some(run.1) {
                continue;
            }
            let mut rect = plan.unit_rect(run.0);
            rect.luma_w = (index - run.0) * 8;
            records.push(plan.temporal_record(frame, &rect, run.1));
            if let Some(mvs) = next {
                run = (index, mvs);
            }
        }
    }
    Ok(())
}

/// Builds the optical-flow motion grid the block's 8x8 units share.
fn tip_motion_grid<T: ReconSample>(
    unit_mvs: &[[Mv; 2]],
    candidates: &mut Vec<[Mv; 2]>,
    search: &mut mc::TipSearchScratch,
    plan: &TipBlockPlan,
    sink: &mc::WorkspaceSink<'_, '_, T>,
    reference: &InterReferenceState<T>,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundMotionGrid> {
    let held = plan.hold(reference)?;
    let prediction = held.prediction(&plan.plan)?;
    let first = *unit_mvs.first().ok_or(ReconError::ZeroDimension {
        field: "TIP compound batch",
    })?;
    let compound = prediction
        .block_params(&plan.batch_unit(first))
        .into_compound()
        .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
    mc::tip_batch_motion_grid(
        sink,
        &compound,
        plan.columns(),
        unit_mvs.len(),
        |index| (plan.unit_rect(index), unit_mvs[index]),
        tile_offset,
        core::mem::take(candidates),
        search,
    )
}

/// Keeps the largest candidate list a published unit hands back.
///
/// The list came from the scratch and returns to it, so the next batch's motion
/// grid is filled into the same allocation.
fn keep_candidates(best: &mut Vec<[Mv; 2]>, candidates: Vec<[Mv; 2]>) {
    if candidates.capacity() > best.capacity() {
        *best = candidates;
    }
}

fn publish_unit_outputs<T: ReconSample>(
    scratch: &TipReconstructScratch,
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    plan: &TipBlockPlan,
    grid: Option<mc::CompoundMotionGrid>,
    reference: &InterReferenceState<T>,
    tile_offset: ByteOffset,
) -> Result<Option<mc::CompoundMotionGrid>> {
    let held = plan.hold(reference)?;
    let prediction = held.prediction(&plan.plan)?;
    let mut run: Option<TipUnit> = None;
    for (index, unit) in scratch.units.iter().enumerate() {
        let params = prediction.block_params(unit);
        let known = scratch.whole_mvs.get(index);
        let (motion, mvs) = if let Some(&Some(mvs)) = known {
            (None, Some(mvs))
        } else {
            let motion = mc::inter_block_motion_grid(sink, params, None, tile_offset)?;
            let mvs = whole_sample_mvs(unit, motion.as_ref());
            (motion, mvs)
        };
        let Some(mvs) = mvs else {
            if let Some(run) = run.take() {
                predict_run(sink, &prediction, &run, tile_offset)?;
            }
            mc::predict_inter_block_from_grid(sink, &params, motion.as_ref(), tile_offset)?;
            continue;
        };
        let next = TipUnit {
            rect: unit.rect,
            has_chroma: unit.has_chroma,
            mvs,
        };
        if !run.as_mut().is_some_and(|run| run.extend(&next))
            && let Some(run) = run.replace(next)
        {
            predict_run(sink, &prediction, &run, tile_offset)?;
        }
    }
    if let Some(run) = run {
        predict_run(sink, &prediction, &run, tile_offset)?;
    }
    Ok(grid)
}

/// The vectors a unit predicts with when they land on whole luma and chroma
/// samples, so its prediction reads no neighbour and adjacent units with the
/// same vectors predict as one rectangle.
fn whole_sample_mvs(unit: &TipUnit, motion: Option<&mc::CompoundMotionGrid>) -> Option<[Mv; 2]> {
    let whole = |value: i32| value % 16 == 0;
    let mvs = match motion {
        None => unit.mvs,
        Some(grid) => {
            let cells = grid.uniform_mvs()?;
            if cells.iter().flatten().any(|&value| value % 32 != 0) {
                return None;
            }
            cells.map(|[row, col]| Mv {
                row: row / 2,
                col: col / 2,
            })
        }
    };
    let chroma_matches = !unit.has_chroma
        || (
            unit.rect.chroma_luma_x,
            unit.rect.chroma_luma_y,
            unit.rect.chroma_luma_w,
            unit.rect.chroma_luma_h,
        ) == (
            unit.rect.luma_x,
            unit.rect.luma_y,
            unit.rect.luma_w,
            unit.rect.luma_h,
        );
    (chroma_matches && mvs.iter().all(|mv| whole(mv.row) && whole(mv.col))).then_some(mvs)
}

impl TipUnit {
    /// Grows this run by the unit to its right when both predict alike.
    fn extend(&mut self, next: &TipUnit) -> bool {
        let fits = self.mvs == next.mvs
            && self.has_chroma == next.has_chroma
            && self.rect.luma_y == next.rect.luma_y
            && self.rect.luma_h == next.rect.luma_h
            && self.rect.luma_x + self.rect.luma_w == next.rect.luma_x
            && self.rect.luma_w + next.rect.luma_w <= mc::MAX_MC_BLOCK_DIM;
        if fits {
            self.rect.luma_w += next.rect.luma_w;
            self.rect.chroma_luma_w += next.rect.chroma_luma_w;
        }
        fits
    }
}

fn predict_run<T: ReconSample>(
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    prediction: &TipPrediction<'_, T>,
    run: &TipUnit,
    tile_offset: ByteOffset,
) -> Result<()> {
    let params = prediction
        .block_params(run)
        .with_refinemv(false)
        .with_refinemv_search(false);
    mc::predict_inter_block_from_grid(sink, &params, None, tile_offset)
}

pub(super) fn retire_motion_grid(
    scratch: &mut TipReconstructScratch,
    mut grid: Option<mc::CompoundMotionGrid>,
) {
    if let Some(grid) = grid.as_mut() {
        keep_candidates(&mut scratch.grid_candidates, grid.take_candidates());
    }
}

/// Reconstructs one § 7.13.3.2 TIP block into `sink` from the motion half's grid,
/// and from its plan and units when that half just ran on `scratch` (`fresh`).
///
/// A block whose units carry the § 7.13.3.1 optical-flow shape is predicted by
/// the fixed-unit batch kernel, which spawns no pool work and writes one
/// rectangle per plane, so every sink takes it. The per-unit fan-out is only
/// available for the frame sink: a task predicting into an out-of-order
/// surface holds a reference borrow across its prediction, and spawned work
/// that waits on that borrow would never run.
#[allow(clippy::too_many_arguments)]
pub(super) fn predict<T: ReconSample>(
    scratch: &mut TipReconstructScratch,
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    grid: Option<mc::CompoundMotionGrid>,
    fresh: bool,
    placed: &PlacedInterBlock,
    residual_blocks: &[InterResidualBlock],
    residual_coeffs: &[i32],
    frame: &TipFramePlan,
    temporal: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<Option<mc::CompoundMotionGrid>> {
    if !fresh || scratch.motion_plan.is_none() {
        let plan = scratch.motion_plan.insert(frame.block(placed)?);
        if plan.batched() {
            batch_candidates(&mut scratch.unit_mvs, plan, placed, temporal, true)?;
        } else {
            build_units(&mut scratch.units, plan, placed, temporal)?;
        }
        scratch.whole_mvs.clear();
    }
    let plan = scratch
        .motion_plan
        .as_ref()
        .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
    let grid = if plan.batched() {
        let held = plan.hold(reference)?;
        let first = *scratch.unit_mvs.first().ok_or(ReconError::ZeroDimension {
            field: "TIP compound batch",
        })?;
        let compound = held
            .prediction(&plan.plan)?
            .block_params(&plan.batch_unit(first))
            .into_compound()
            .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
        mc::predict_tip_batch_from_grid(
            sink,
            compound,
            plan.batch_rect,
            plan.batch_has_chroma,
            grid.ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?,
            tile_offset,
        )?
    } else {
        publish_unit_outputs(scratch, sink, plan, grid, reference, tile_offset)?
    };
    scratch.units.clear();
    if let Some(residual) = placed.block.residual.as_ref() {
        super::super::add_inter_residual_to_workspace(
            sink,
            residual,
            residual_blocks,
            residual_coeffs,
            qindex,
            luma_use_tcq,
            residual_use_ddt,
            false,
            bit_depth,
        )?;
    }
    Ok(grid)
}

/// `placed` moved onto `rect`, luma and chroma alike.
fn rect_placed(placed: &PlacedInterBlock, rect: splot_recon::PlaneRect) -> PlacedInterBlock {
    let mut band = placed.clone();
    band.luma = [rect.x(), rect.y(), rect.width(), rect.height()].map(|v| v as u32);
    band.chroma_luma = band.luma;
    band
}

/// Luma rows and columns per § 7.10.6 TIP-as-output prediction band.
///
/// The whole frame is one TIP block, so predicting it in one pass sized the
/// compound scratch at frame scale. Banding keeps that scratch bounded; both
/// sizes are multiples of both prediction unit sizes and of chroma
/// subsampling, so a band boundary never splits a unit. A scheduled band is
/// one motion-field band tall instead, see [`TipOutputStages::init`].
const TIP_OUTPUT_BAND_LUMA_ROWS: usize = 64;

const TIP_OUTPUT_BAND_LUMA_COLS: usize = 256;

pub(in crate::prediction::inter) fn reconstruct_output<T: ReconSample>(
    decode_scratch: &mut super::InterDecodeScratch<T>,
    mut recycled: splot_recon::FramePlaneSamples<T>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    geometry: super::super::FrameDecodeGeometry,
    offset: ByteOffset,
) -> Result<(DecodedFrame<T>, TemporalMotionField)> {
    let mut lease = crate::support::decode_buffers::DecodeBuffers::lend_temporal(
        decode_scratch.buffers.as_deref(),
    );
    let spares = TipSpares::default();
    spares.give(
        core::mem::replace(&mut lease.temporal, TemporalMvContext::empty()),
        core::mem::take(&mut decode_scratch.tip_output),
    );
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::from_workspace(
            CurrentFrameWorkspace::<T>::new_recycled_from(geometry.info(), &mut recycled)?,
            None,
        )?,
    );
    let stages = TipOutputStages::init(
        &spares,
        Arc::clone(&progress),
        sequence,
        core,
        reference,
        geometry,
        offset,
        None,
    )?;
    let mut motion_field = None;
    for band in 0..stages.band_count() {
        motion_field = stages
            .band(band, sequence, core, reference)?
            .or(motion_field);
    }
    let motion_field = motion_field.ok_or(DecodeHeaderStateError::IncompleteTipOutput)?;
    let (output, temporal) = stages.finish()?;
    let mut progress =
        Arc::into_inner(progress).ok_or(DecodeHeaderStateError::IncompleteTipOutput)?;
    progress
        .with_unpublished_workspace(|workspace| deblock_output(workspace, sequence, core))??;
    let frame = progress.freeze()?;
    lease.temporal = temporal;
    decode_scratch.tip_output = output;
    Ok((frame, motion_field))
}

/// A `TIP_FRAME_AS_OUTPUT` reconstruction split into row-scheduled steps:
/// [`init`](Self::init) lays out the bands (and projects the whole motion
/// field for a frame decoded in place), [`project`](Self::project) projects
/// one motion-field band of a scheduled frame and bounds the reference rows
/// of the bands in it, [`band`](Self::band) predicts one full-width band
/// into the frame's own rows and folds its records (a motion-field band
/// publishes once all its bands folded), and [`finish`](Self::finish) ends
/// the walk; [`deblock_output`] then runs before the freeze.
pub(crate) struct TipOutputStages<T: ReconSample> {
    temporal: TemporalMvContext,
    output: TipOutputScratch,
    info: splot_recon::DecodedFrameInfo,
    progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    /// The § 7.22 records every band folds in; a scheduled frame publishes
    /// its field through it band by band.
    motion: super::temporal::MotionFieldUnits,
    /// The band-by-band § 7.9 projection of a scheduled frame.
    plan: Option<super::super::find_mv_stack::TemporalBandPlan>,
    /// Bands whose records were folded.
    folded: core::sync::atomic::AtomicUsize,
    placed: PlacedInterBlock,
    /// Bands per motion-field band, whose records fold into it alone.
    parts: usize,
    band_w: usize,
    offset: ByteOffset,
}

impl<T: ReconSample> TipOutputStages<T> {
    /// Projects the references' motion into the frame and lays out its bands.
    ///
    /// # Errors
    /// Returns the header, projection or workspace diagnostic.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn init(
        spares: &TipSpares,
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        reference: &InterReferenceState<T>,
        geometry: super::super::FrameDecodeGeometry,
        offset: ByteOffset,
        publish: Option<MotionFieldHandle>,
    ) -> Result<Self> {
        let (mut temporal, mut output) = spares.take();
        let info = geometry.info();
        let inter = core
            .inter
            .as_ref()
            .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
        let ref_frame_idx = &inter.ref_frame_idx;
        let width = info.storage_luma_size().width();
        let height = info.storage_luma_size().height();
        let (mi_rows, mi_cols) = geometry.mi_dimensions();
        let sb_h4 = geometry.sb_h4();
        let projection_step = tmvp_projection_step(core);
        let current_order_hint = core
            .display_order_hint()
            .ok_or(DecodeHeaderStateError::MissingDisplayOrderHint)?;
        if sequence.partition.is_none() {
            return Err(DecodeHeaderStateError::IncompleteInterFrameTools.into());
        }
        let config = TemporalProjectionConfig {
            frame_size: (
                info.coded_luma_size().width(),
                info.coded_luma_size().height(),
            ),
            step: projection_step,
            unit_size8: tmvp_unit_size8(projection_step, sb_h4),
            enable_tip: sequence
                .inter
                .as_ref()
                .is_some_and(|tools| tools.enable_tip),
            enable_trajectory: false, // only § 7.12.2 reads TrajMv/TrajValid, and a TIP_FRAME_AS_OUTPUT frame parses no blocks
            reduced: sequence
                .inter
                .as_ref()
                .is_some_and(|tools| tools.reduced_ref_frame_mvs_mode),
            bru_ref: None,
        };
        let hints = super::super::find_mv_stack::reference_order_hints(
            ref_frame_idx,
            &reference.ref_valid,
            &reference.ref_order_hint,
        );
        let tip_pair =
            super::super::find_mv_stack::tip_reference_pair_from_hints(current_order_hint, &hints);
        let plan = if publish.is_some() {
            let (metadata, layouts) =
                scheduled_motion_field_inputs(&reference.ref_motion_fields, ref_frame_idx)?;
            let fill_holes = inter
                .allow_tip_hole_fill
                .ok_or(DecodeHeaderStateError::IncompleteInterFrameTools)?;
            let pair = tip_pair.ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            Some(
                temporal
                    .begin_banded_refresh(
                        geometry.motion_layout(),
                        current_order_hint,
                        config,
                        ref_frame_idx,
                        &reference.ref_valid,
                        &reference.ref_order_hint,
                        &metadata,
                        &layouts,
                        Some(pair),
                        true,
                        fill_holes,
                    )
                    .ok_or(DecodeHeaderStateError::InvalidInterTileSchedulingState)?,
            )
        } else {
            let ref_motion_fields = reference.resolve_motion_fields(ref_frame_idx)?;
            temporal.refresh_from_references(
                (mi_rows, mi_cols),
                current_order_hint,
                config,
                ref_frame_idx,
                &reference.ref_valid,
                &reference.ref_order_hint,
                &ref_motion_fields,
            )?;
            prepare_motion_field(&mut temporal, core, sb_h4, tip_pair)?;
            None
        };
        let global_mv = inter
            .tip_global_mv
            .ok_or(DecodeHeaderStateError::IncompleteTipOutput)?;
        let band_h = if publish.is_some() {
            geometry.motion_layout().band_rows8() * 8
        } else {
            TIP_OUTPUT_BAND_LUMA_ROWS
        }
        .min(height);
        let band_w = TIP_OUTPUT_BAND_LUMA_COLS.min(width);
        let placed = PlacedInterBlock {
            luma: [0, 0, band_w, band_h].map(|v| v as u32),
            chroma_luma: [0, 0, band_w, band_h].map(|v| v as u32),
            predict_chroma: sequence.general.chroma_format_idc != ChromaFormatIdc::Monochrome,
            sub8x8_chroma: false,
            interintra_chroma: false,
            interintra_edges: [crate::pipeline::reconstruct::IntraEdgeAvailability::new(true, true);
                2],
            block: InterBlock {
                ref_frame0: TIP_REF_FRAME,
                ref_frame1: None,
                mv: Mv {
                    row: global_mv.row,
                    col: global_mv.col,
                },
                mv1: Mv::ZERO,
                interp: ReconInterpolationFilter::EightTapSharp,
                warp_params: [None, None],
                bawp: BawpSyntax::default(),
                interintra: None,
                compound_blend: mc::CompoundBlend::default(),
                optflow_distances: None,
                residual: None,
            },
        };
        output.rects.clear();
        output.rows.clear();
        let columns = if publish.is_some() {
            width.div_ceil(band_w)
        } else {
            1
        };
        let strip_h = band_h
            .div_ceil(columns.min(band_h / 16).max(1))
            .next_multiple_of(16)
            .min(band_h);
        let parts = band_h.div_ceil(strip_h);
        for band_y in (0..height).step_by(band_h) {
            let band_end = (band_y + band_h).min(height);
            for strip_y in (band_y..band_end).step_by(strip_h) {
                let strip_end = (strip_y + strip_h).min(band_end);
                output.rects.push(splot_recon::PlaneRect::new(
                    0,
                    strip_y,
                    width,
                    strip_end - strip_y,
                )?);
                output.rows.push((strip_y, strip_end));
            }
        }
        progress.begin(&output.rows)?; // § 7.10.6 predicts every coded sample of these rows before the freeze
        let band_count = output.rects.len();
        if output.bands.len() < band_count {
            output.bands.resize_with(band_count, Default::default);
        }
        let mut motion = output.motion.take().unwrap_or_else(|| {
            super::temporal::MotionFieldUnits::new(TemporalMotionField::empty())
        });
        match publish {
            Some(handle) => {
                if height.div_ceil(band_h) != handle.layout().band_count() {
                    return Err(DecodeHeaderStateError::InvalidInterTemporalMotionState.into());
                }
                motion.reset_publishing(
                    geometry.motion_field_header(&hints),
                    band_count,
                    parts,
                    handle,
                )?;
            }
            None => motion.reset(geometry.new_motion_field(&hints).ok_or(
                ReconError::WorkspaceAllocationFailed {
                    plane: splot_recon::PlaneId::Y,
                    context: "TIP-output motion field",
                },
            )?),
        }
        Ok(Self {
            temporal,
            output,
            info,
            progress,
            motion,
            plan,
            folded: core::sync::atomic::AtomicUsize::new(0),
            placed,
            parts,
            band_w,
            offset,
        })
    }

    /// The bands the frame predicts, in raster order.
    pub(crate) fn band_count(&self) -> usize {
        self.output.rects.len()
    }

    /// The motion-field bands [`Self::project`] projects one at a time; zero
    /// when init projected the whole frame.
    pub(crate) fn projected_bands(&self) -> usize {
        self.plan
            .as_ref()
            .map_or(0, super::super::find_mv_stack::TemporalBandPlan::len)
    }

    /// Whether every reference band field band `index` projects from was
    /// published, so [`Self::project`] may run. § 7.9.8 keeps a projection in
    /// its source's TMVP row and § 7.10.3 to § 7.10.5 stay inside one TMVP
    /// unit, so a band reads only the same band of each reference field.
    pub(crate) fn projection_ready(
        &self,
        index: usize,
        reference: &InterReferenceState<T>,
    ) -> bool {
        self.plan
            .as_ref()
            .is_some_and(|plan| plan.sources_settled(index, &reference.ref_motion_fields))
    }

    /// Projects motion-field band `index` and hands `bounds` the reference
    /// rows each band in it reads, so a scheduler admits the band once
    /// those rows are published.
    ///
    /// # Errors
    /// Returns the projection's own diagnostic.
    pub(crate) fn project(
        &self,
        index: usize,
        core: &FrameHeaderCore,
        reference: &InterReferenceState<T>,
        mut bounds: impl FnMut(usize, super::row_gate::RowReferenceBounds),
    ) -> Result<()> {
        let plan = self
            .plan
            .as_ref()
            .ok_or(DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        super::tile::project_temporal_band(
            plan,
            &self.temporal,
            &reference.ref_motion_fields,
            index,
        )?;
        let inter = core
            .inter
            .as_ref()
            .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
        let gate = super::row_gate::RowReferenceGate::new(
            reference,
            core,
            &inter.ref_frame_idx,
            self.info,
            &self.temporal,
        );
        let first = index.saturating_mul(self.parts);
        for (band, rect) in self
            .output
            .rects
            .iter()
            .enumerate()
            .skip(first)
            .take(self.parts)
        {
            bounds(band, gate.tip_bounds(&rect_placed(&self.placed, *rect)));
        }
        Ok(())
    }

    /// Folds band `index`'s § 7.22 records into the field, which publishes
    /// the band's motion-field band once its last band lands.
    fn fold(&self, index: usize, records: &mut Vec<TemporalMotionBlock>) -> Result<()> {
        self.motion.fold_unit(index, records)?;
        records.clear();
        self.folded
            .fetch_add(1, core::sync::atomic::Ordering::AcqRel);
        self.motion.unit_landed_for(index);
        Ok(())
    }

    /// Predicts band `index` into the frame's rows it leases and folds its
    /// records. Bands are disjoint, so any number of them run at once.
    /// Returns the field of a frame that collects it, with the last band.
    ///
    /// # Errors
    /// Returns the prediction's own diagnostic.
    pub(crate) fn band(
        &self,
        index: usize,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        reference: &InterReferenceState<T>,
    ) -> Result<Option<TemporalMotionField>> {
        let (rect, band) = self.band_slot(index)?;
        let mut band = band.lock();
        let mut lease = self
            .progress
            .direct_stripe(index)
            .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
        let mut target = lease
            .take_target()
            .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
        let mut planes = {
            use splot_recon::PlaneId::{U, V, Y};
            [Y, U, V].map(|plane| target.take(plane))
        };
        let [y, u, v] = planes.each_mut().map(|plane| {
            plane.as_mut().map(|plane| {
                let stride = plane.stride();
                (plane.samples_mut(), stride)
            })
        });
        let mut surface = splot_recon::CurrentFrameRect::from_bands(self.info, rect, [y, u, v])?;
        let mut sink = mc::WorkspaceSink::Rect(&mut surface);
        self.predict_band(&mut band, rect, &mut sink, sequence, core, reference)?;
        self.fold(index, &mut band.records)?;
        Ok(
            (self.folded.load(core::sync::atomic::Ordering::Acquire) == self.output.rects.len())
                .then(|| self.motion.take_field())
                .flatten(),
        )
    }

    fn band_slot(
        &self,
        index: usize,
    ) -> Result<(splot_recon::PlaneRect, &parking_lot::Mutex<TipOutputBand>)> {
        let rect = *self
            .output
            .rects
            .get(index)
            .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
        let band = self
            .output
            .bands
            .get(index)
            .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
        Ok((rect, band))
    }

    fn predict_band(
        &self,
        band: &mut TipOutputBand,
        rect: splot_recon::PlaneRect,
        sink: &mut mc::WorkspaceSink<'_, '_, T>,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        reference: &InterReferenceState<T>,
    ) -> Result<()> {
        let inter = core
            .inter
            .as_ref()
            .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
        let ref_frame_idx = &inter.ref_frame_idx;
        band.records.clear();
        let width = self.info.storage_luma_size().width();
        let bit_depth = self.info.bit_depth();
        let mut placed = self.placed.clone();
        placed.luma[1] = rect.y() as u32;
        placed.luma[3] = rect.height() as u32;
        placed.chroma_luma[1] = rect.y() as u32;
        placed.chroma_luma[3] = rect.height() as u32;
        let frame = TipFramePlan::new(
            self.info,
            &self.temporal,
            sequence,
            core,
            ref_frame_idx,
            reference,
        )?;
        for band_x in (rect.x()..rect.x() + rect.width()).step_by(self.band_w) {
            let cols = self.band_w.min(width - band_x);
            placed.luma[0] = band_x as u32;
            placed.luma[2] = cols as u32;
            placed.chroma_luma[0] = band_x as u32;
            placed.chroma_luma[2] = cols as u32;
            let grid = motion(
                &mut band.scratch,
                &mut band.records,
                sink,
                &placed,
                &frame,
                &self.temporal,
                reference,
                self.offset,
            )?;
            let grid = predict(
                &mut band.scratch,
                sink,
                grid,
                true,
                &placed,
                &[],
                &[],
                &frame,
                &self.temporal,
                reference,
                0,
                false,
                false,
                bit_depth,
                self.offset,
            )?;
            retire_motion_grid(&mut band.scratch, grid);
        }
        Ok(())
    }

    /// Ends the walk once every band folded, handing back its scratch.
    ///
    /// # Errors
    /// Returns [`DecodeHeaderStateError::IncompleteTipOutput`] when a band
    /// did not fold.
    pub(crate) fn finish(self) -> Result<(TipOutputScratch, TemporalMvContext)> {
        let Self {
            temporal,
            mut output,
            mut motion,
            folded,
            ..
        } = self;
        if folded.into_inner() != output.rects.len() {
            return Err(DecodeHeaderStateError::IncompleteTipOutput.into());
        }
        motion.retire();
        output.motion = Some(motion);
        Ok((output, temporal))
    }
}

/// Deblocks the TIP unit edges of a frame written whole.
///
/// # Errors
/// Returns the deblock diagnostic.
pub(crate) fn deblock_output<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<()> {
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let bit_depth = workspace.info().bit_depth();
    if inter.apply_deblocking_filter_tip == Some(true) {
        let quant = core
            .quantization_params
            .ok_or(DecodeHeaderStateError::IncompleteTipOutput)?;
        let tq = sequence
            .transform_quant_entropy
            .as_ref()
            .ok_or(DecodeHeaderStateError::MissingSequenceTransformQuantEntropy)?;
        let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);
        let interpolation_filter = output_interpolation_filter(inter)?;
        let enable_tip_refinemv = sequence
            .inter
            .as_ref()
            .is_some_and(|tools| tools.enable_tip_refinemv);
        crate::filters::deblock::deblock_tip_frame(
            workspace,
            output_prediction_unit_size(enable_tip_refinemv, interpolation_filter),
            quant,
            seq_quant.base_uv_ac_delta_q,
            core.tile_info
                .as_ref()
                .map(|tile| (tile.mi_col_starts.as_ref(), tile.mi_row_starts.as_ref())),
            sequence
                .filter
                .is_some_and(|filter| filter.disable_loopfilters_across_tiles),
            bit_depth,
        )
        .map_err(|_| DecodeHeaderStateError::IncompleteTipOutput)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        TipHeldReferences, TipReferencePlan, output_interpolation_filter,
        output_prediction_unit_size, prediction_unit_extent, prediction_unit_size,
        tip_optflow_offsets_allowed, tip_optflow_weight_and_references_allowed,
        tip_prediction_controls, tip_references_unscaled, tip_refinemv_offsets_allowed,
        tip_uses_refinemv, tip_uses_two_references, tmvp_unit_size8,
    };
    use crate::prediction::inter::mc;
    use crate::prediction::inter::reference::HeldFrameSamples;
    use crate::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};
    use splot_core::headers::frame::{
        FrameSize, FrameType, InterControl, InterpolationFilter as FrameInterpolationFilter,
        TipFrameMode,
    };
    use splot_parallel::ThreadCount;
    use splot_recon::{
        BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, InterpolationFilter, OutputIndex,
        PixelFormat, PlaneId, PlaneRect, PlaneSize,
    };

    const TIP_FAMILIES_FIXTURE: &[u8] = include_bytes!(
        "../../../../../../tests/conformance/vectors/valid/syn-frame-tip-families-64x64.ivf"
    );

    #[test]
    fn tip_publication_is_byte_exact_at_one_and_four_workers()
    -> Result<(), Box<dyn std::error::Error>> {
        let decode = |threads| -> Result<Vec<u8>, crate::DecodeError> {
            let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(threads)))?;
            let mut output = Vec::new();
            context.decode_raw_bytes(
                TIP_FAMILIES_FIXTURE,
                DecodeOptions::default(),
                &mut output,
            )?;
            Ok(output)
        };

        assert_eq!(decode(1)?, decode(4)?);
        Ok(())
    }

    #[test]
    fn tip_reference_unit_size_follows_refinement_and_large_block_gates() {
        assert_eq!(prediction_unit_size(64, 32, false), 16);
        assert_eq!(prediction_unit_size(8, 32, false), 8);
        assert_eq!(prediction_unit_size(64, 32, true), 8);
        assert_eq!(prediction_unit_size(256, 256, true), 16);
    }

    #[test]
    fn invalid_tip_output_filters_are_typed_header_state_errors() {
        let mut inter = InterControl::default();
        for filter in [
            None,
            Some(FrameInterpolationFilter::Bilinear),
            Some(FrameInterpolationFilter::Switchable),
        ] {
            inter.tip_interpolation_filter = filter;
            assert!(matches!(
                output_interpolation_filter(&inter),
                Err(crate::DecodeError::HeaderState {
                    source: crate::DecodeHeaderStateError::IncompleteTipOutput
                })
            ));
        }
    }

    #[test]
    fn tip_prediction_controls_require_complete_bounded_tip_state() {
        let invalid = |inter: InterControl| {
            assert!(matches!(
                tip_prediction_controls(Some(&inter)),
                Err(crate::DecodeError::HeaderState {
                    source: crate::DecodeHeaderStateError::InvalidInterTipPredictionState
                })
            ));
        };
        assert!(matches!(
            tip_prediction_controls(None),
            Err(crate::DecodeError::HeaderState {
                source: crate::DecodeHeaderStateError::MissingInterControlRegion
            })
        ));
        let mut inter = InterControl::default();
        invalid(inter.clone());
        inter.tip_frame_mode = Some(TipFrameMode::AsRef);
        invalid(inter.clone());
        inter.tip_global_wtd_index = Some(0);
        invalid(inter.clone());
        inter.opfl_refine_type = Some(0);
        assert!(tip_prediction_controls(Some(&inter)).is_ok());
        inter.tip_global_wtd_index = Some(7);
        inter.opfl_refine_type = Some(2);
        inter.tip_frame_mode = Some(TipFrameMode::AsOutput);
        assert!(tip_prediction_controls(Some(&inter)).is_ok());
        inter.tip_frame_mode = Some(TipFrameMode::Disabled);
        invalid(inter.clone());
        inter.tip_frame_mode = Some(TipFrameMode::Other(3));
        invalid(inter.clone());
        inter.tip_frame_mode = Some(TipFrameMode::AsRef);
        inter.tip_global_wtd_index = Some(8);
        invalid(inter.clone());
        inter.tip_global_wtd_index = Some(0);
        inter.opfl_refine_type = Some(3);
        invalid(inter);
    }

    #[test]
    fn logical_compound_pair_survives_a_deduplicated_reference_borrow()
    -> Result<(), Box<dyn std::error::Error>> {
        let reference = tip_workspace()?.freeze()?;
        let held = TipHeldReferences {
            past: HeldFrameSamples::Settled(&reference),
            future: None,
        };
        let plan = TipReferencePlan {
            past: 0,
            future: Some(0),
            interpolation_filter: InterpolationFilter::EightTapSharp,
            blend: mc::CompoundBlend::default(),
            optflow_distances: None,
            use_refinemv: false,
            search_refinemv: false,
            optflow_sad_threshold: None,
        };

        assert!(held.prediction(&plan)?.reference1.is_some());
        Ok(())
    }

    /// A one-unit TIP block whose two flat predictions fall under the
    /// § 7.13.3.8 SAD gate keeps its candidates.
    #[test]
    fn one_unit_tip_block_under_the_optflow_sad_gate_keeps_its_candidates()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut reference = crate::prediction::inter::InterReferenceState::<u8>::empty()?;
        let mut store = splot_recon::ReferenceFrameStore::with_capacity(2)?;
        for slot in 0..2 {
            store.put(
                splot_recon::ReferenceSlot::new(slot)?,
                crate::pipeline::inflight::RefFrameSlot::completed(splot_recon::SharedFrame::new(
                    tip_workspace()?.freeze()?,
                )),
            )?;
        }
        reference.store = store;
        let rect = mc::McBlockRect::from_luma_rect(0, 0, 8, 8);
        let plan = super::TipBlockPlan {
            plan: TipReferencePlan {
                past: 0,
                future: Some(1),
                interpolation_filter: InterpolationFilter::EightTapSharp,
                blend: mc::CompoundBlend::default(),
                optflow_distances: Some([1, -1]),
                use_refinemv: false,
                search_refinemv: false,
                optflow_sad_threshold: Some(6),
            },
            output: false,
            use_optflow: true,
            unit_size: 8,
            unit_count: 1,
            block_w: 8,
            block_h: 8,
            batch_rect: rect,
            batch_has_chroma: false,
            frame_mi_rows: 2,
            frame_mi_cols: 2,
        };
        let mut scratch = super::TipReconstructScratch::default();
        scratch
            .unit_mvs
            .push([crate::prediction::inter::Mv::ZERO; 2]);
        let mut workspace = tip_workspace()?;
        let sink = mc::WorkspaceSink::Frame(&mut workspace);
        let grid = super::tip_motion_grid(
            &scratch.unit_mvs,
            &mut scratch.grid_candidates,
            &mut scratch.search,
            &plan,
            &sink,
            &reference,
            splot_core::span::ByteOffset::new(0),
        )?;
        assert_eq!(
            grid.stored_mvs_at_index(0)?,
            [crate::prediction::inter::Mv::ZERO; 2]
        );
        Ok(())
    }

    fn tip_workspace() -> splot_recon::Result<CurrentFrameWorkspace<u8>> {
        let size = PlaneSize::new(8, 8)?;
        let visible = PlaneRect::new(0, 0, 8, 8)?;
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            size,
            visible,
        )?;
        CurrentFrameWorkspace::new(info, 0)
    }

    #[test]
    fn tmvp_unit_size_uses_64_pixels_for_step_one_or_64_pixel_superblocks() {
        assert!(
            tmvp_unit_size8(1, 32) == 8
                && tmvp_unit_size8(2, 16) == 8
                && tmvp_unit_size8(2, 32) == 16
        );
    }

    #[test]
    fn tip_output_unit_size_requires_sharp_refinement() {
        assert_eq!(
            output_prediction_unit_size(true, InterpolationFilter::EightTapSharp),
            8
        );
        assert_eq!(
            output_prediction_unit_size(true, InterpolationFilter::EightTapSmooth),
            16
        );
        assert_eq!(
            output_prediction_unit_size(false, InterpolationFilter::EightTapSharp),
            16
        );
    }

    #[test]
    fn tip_output_edge_units_keep_nominal_prediction_size() {
        let output_height = prediction_unit_extent(true, 8, 16);
        let reference_height = prediction_unit_extent(false, 8, 16);
        let output = mc::McBlockRect::from_luma_rect(432, 1072, 16, output_height);
        let reference = mc::McBlockRect::from_luma_rect(432, 1072, 16, reference_height);

        assert_eq!(output.plane_rect(PlaneId::V, 1, 1).3, 8);
        assert_eq!(reference.plane_rect(PlaneId::V, 1, 1).3, 4);
    }

    #[test]
    fn tip_weight_sixteen_uses_only_the_past_reference() {
        assert!(tip_uses_two_references(8));
        assert!(!tip_uses_two_references(16));
    }

    #[test]
    fn tip_refinemv_requires_both_sequence_tools_and_equal_weight() {
        assert!(tip_uses_refinemv(false, true, true, 8));
        assert!(!tip_uses_refinemv(true, true, true, 8));
        assert!(!tip_uses_refinemv(false, false, true, 8));
        assert!(!tip_uses_refinemv(false, true, false, 8));
        assert!(!tip_uses_refinemv(false, true, true, 12));
        assert!(!tip_uses_refinemv(false, true, true, 16));
    }

    #[test]
    fn tip_refinemv_requires_symmetric_reference_offsets() {
        assert!(tip_refinemv_offsets_allowed(4, -4));
        assert!(!tip_refinemv_offsets_allowed(4, -5));
        assert!(!tip_refinemv_offsets_allowed(0, 0));
    }

    #[test]
    fn tip_optflow_accepts_unequal_opposite_direction_references() {
        assert!(tip_optflow_offsets_allowed(1, -2));
        assert!(!tip_optflow_offsets_allowed(1, 2));
    }

    #[test]
    fn tip_output_optflow_needs_references_on_both_sides() {
        assert!(tip_optflow_weight_and_references_allowed(true, 12, true));
        assert!(!tip_optflow_weight_and_references_allowed(true, 8, false));
        assert!(tip_optflow_weight_and_references_allowed(false, 8, true));
        assert!(!tip_optflow_weight_and_references_allowed(false, 12, true));
        assert!(!tip_optflow_weight_and_references_allowed(false, 8, false));
    }

    #[test]
    fn tip_refinemv_reference_gate_maps_slots_and_rejects_ineligible_state() {
        let allowed = |frame_type, frame_size, order_hints: &[u32], widths: &[u32]| {
            tip_references_unscaled(
                frame_type,
                frame_size,
                &[2, 0],
                order_hints,
                widths,
                &[352, 1, 352],
                [0, 1],
            )
        };
        let size = Some(FrameSize::new(352, 352));

        assert!(allowed(
            Some(FrameType::Inter),
            size,
            &[0, 0, 0],
            &[352, 1, 352]
        ));
        assert!(!allowed(
            Some(FrameType::Switch),
            size,
            &[0, 0, 0],
            &[352, 1, 352]
        ));
        assert!(!allowed(
            Some(FrameType::Inter),
            None,
            &[0, 0, 0],
            &[352, 1, 352]
        ));
        assert!(!allowed(
            Some(FrameType::Inter),
            size,
            &[u32::MAX, 0, 0],
            &[352, 1, 352]
        ));
        assert!(!allowed(Some(FrameType::Inter), size, &[], &[352, 1, 352]));
        assert!(!allowed(
            Some(FrameType::Inter),
            size,
            &[0, 0, 0],
            &[351, 1, 352]
        ));
        assert!(!tip_references_unscaled(
            Some(FrameType::Inter),
            size,
            &[2],
            &[0, 0, 0],
            &[352, 1, 352],
            &[352, 1, 352],
            [0, 1],
        ));
    }

    #[test]
    fn tip_refinemv_scale_gate_uses_spec_rounding() {
        assert!(tip_references_unscaled(
            Some(FrameType::Inter),
            Some(FrameSize::new(65_536, 65_536)),
            &[0, 1],
            &[0, 0],
            &[65_535, 65_536],
            &[65_535, 65_536],
            [0, 1],
        ));
    }
}
