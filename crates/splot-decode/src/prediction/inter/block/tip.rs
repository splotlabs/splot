// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::DecodeHeaderStateError;
use crate::prediction::inter::InterResidualReconScratch;
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_parallel::prelude::*;
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
    metadata: Option<Box<mc::CompoundBlockMetadata>>,
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

/// The reference pair and § 7.13.5 prediction settings one TIP block reads
/// through, resolved once so the borrows can be retaken per unit batch.
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

/// One batch of TIP prediction units' borrow of the block's reference pair.
struct TipHeldReferences<'a, T: ReconSample> {
    past: HeldFrameSamples<'a, T>,
    /// The future reference's borrow, absent when it names the past slot.
    future: Option<HeldFrameSamples<'a, T>>,
}

impl TipReferencePlan {
    /// Borrows the reference pair for one batch of prediction units.
    ///
    /// A still-filtering reference is readable only while the borrow lives, and
    /// that borrow holds the reference's shared workspace lock, so the batch —
    /// not the whole TIP block — is the unit of the hold: the § 7.2 filter phase
    /// publishing that same frame's later stripes waits out one batch instead of
    /// every unit the block covers.
    fn hold<'a, T: ReconSample>(
        &self,
        reference: &'a InterReferenceState<T>,
    ) -> Result<TipHeldReferences<'a, T>> {
        let (past, future) = super::super::hold_reference_pair(reference, self.past, self.future)?;
        Ok(TipHeldReferences { past, future })
    }
}

impl<T: ReconSample> TipHeldReferences<'_, T> {
    /// Whether both borrows name settled frames, which hold no lock and never
    /// unsettle, so one borrow covers every unit the block predicts.
    const fn settled(&self) -> bool {
        matches!(self.past, HeldFrameSamples::Settled(_))
            && !matches!(self.future, Some(HeldFrameSamples::Filtering(_)))
    }

    /// Resolves the borrow into the samples one batch's units read.
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
pub(super) struct TipReconstructScratch<T: ReconSample> {
    units: Vec<TipUnit>,
    output_samples: Vec<T>,
}

const fn tip_uses_two_references(weight: i16) -> bool {
    weight != TIP_SINGLE_WEIGHT
}

const fn tip_temporal_mvs(
    use_optflow: bool,
    candidate: [Mv; 2],
    refined: Option<[Mv; 2]>,
) -> [Mv; 2] {
    if use_optflow {
        match refined {
            Some(refined) => refined,
            None => candidate,
        }
    } else {
        candidate
    }
}

const fn tip_uses_refinemv(
    output: bool,
    enable_refinemv: bool,
    enable_tip_refinemv: bool,
    weight: i16,
) -> bool {
    !output && enable_refinemv && enable_tip_refinemv && weight == mc::CWP_EQUAL
}

#[doc = "AV2 § 5.20.7.17 equal-distance reference gate for refine-MV search."]
const fn tip_refinemv_offsets_allowed(past_offset: i32, future_offset: i32) -> bool {
    past_offset != 0 && past_offset == -future_offset
}

#[doc = "AV2 § 5.20.7.14 and § 5.20.7.17 reference gate for refine-MV search."]
fn tip_refinemv_references_allowed(
    frame_type: Option<FrameType>,
    frame_size: Option<splot_core::headers::frame::FrameSize>,
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    ref_frame_width: &[u32],
    ref_frame_height: &[u32],
    references: [(i8, i32); 2],
) -> bool {
    if !tip_refinemv_offsets_allowed(references[0].1, references[1].1) {
        return false;
    }
    tip_references_unscaled(
        frame_type,
        frame_size,
        ref_frame_idx,
        ref_order_hint,
        ref_frame_width,
        ref_frame_height,
        [references[0].0, references[1].0],
    )
}

#[doc = "AV2 § 5.20.7.14 opposite-direction reference gate for optical flow."]
fn tip_optflow_references_allowed(
    frame_type: Option<FrameType>,
    frame_size: Option<splot_core::headers::frame::FrameSize>,
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    ref_frame_width: &[u32],
    ref_frame_height: &[u32],
    references: [(i8, i32); 2],
) -> bool {
    if (references[0].1 <= 0) == (references[1].1 <= 0) {
        return false;
    }
    tip_references_unscaled(
        frame_type,
        frame_size,
        ref_frame_idx,
        ref_order_hint,
        ref_frame_width,
        ref_frame_height,
        [references[0].0, references[1].0],
    )
}

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

fn compute_parallel_outputs<T: ReconSample>(
    sink: &mc::WorkspaceSink<'_, '_, T>,
    units: &mut [TipUnit],
    output_samples: &mut [T],
    output_stride: usize,
    prediction: &TipPrediction<'_, T>,
    tile_offset: ByteOffset,
) -> Result<()> {
    if output_stride == 0 {
        return Err(ReconError::ZeroDimension {
            field: "TIP compound output stride",
        }
        .into());
    }
    let expected =
        units
            .len()
            .checked_mul(output_stride)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "TIP compound output arena length",
            })?;
    if output_samples.len() != expected {
        return Err(ReconError::BufferLengthMismatch {
            expected,
            actual: output_samples.len(),
        }
        .into());
    }
    output_samples
        .par_chunks_mut(output_stride)
        .zip(units.par_iter_mut())
        .try_for_each(|(samples, unit)| {
            let compound = prediction
                .block_params(unit)
                .into_compound()
                .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
            let motion = mc::compound_block_motion_grid(
                sink,
                compound,
                prediction.optflow_distances.is_some().then_some(8),
                tile_offset,
            )?;
            let metadata =
                mc::predict_compound_from_grid(sink, compound, motion, tile_offset, samples)?;
            if prediction.optflow_distances.is_some() {
                unit.mvs = tip_temporal_mvs(true, unit.mvs, metadata.stored_mvs_at_origin()?);
            }
            unit.metadata = Some(Box::new(metadata));
            Ok(())
        })
}

fn compute_batched_output<T: ReconSample>(
    sink: &mc::WorkspaceSink<'_, '_, T>,
    units: &[TipUnit],
    output_samples: &mut [T],
    prediction: &TipPrediction<'_, T>,
    plan: &TipBlockPlan,
    motion: mc::CompoundMotionGrid,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundBlockMetadata> {
    let first = units.first().ok_or(ReconError::ZeroDimension {
        field: "TIP compound batch",
    })?;
    let compound = prediction
        .block_params(first)
        .into_compound()
        .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
    mc::predict_tip_batch_from_grid(
        sink,
        compound,
        plan.batch_rect,
        plan.batch_has_chroma,
        motion,
        tile_offset,
        output_samples,
    )
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
    tile_offset: ByteOffset,
) -> Result<bool> {
    let (n4w, n4h) = n4;
    if tip_frame_mode == TipFrameMode::Disabled || !allowed_for_block(frontier, n4w, n4h) {
        return Ok(false);
    }
    let tip_ref = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::TipMode {
                ctx: neighbour_ctx.tip_mode_ctx(),
            },
            symbols,
        )
        .map_err(|error| symbol_read_error(error, tile_offset))?;
    Ok(tip_ref.get() != 0)
}

fn allowed_for_block(frontier: &DecodeBlockFrontier, n4w: usize, n4h: usize) -> bool {
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

/// The § 7.13.5 settings both halves of one TIP block derive from.
///
/// Deriving them is header arithmetic over the frame's TIP controls, so each
/// half derives its own copy rather than carrying one across the seam.
struct TipBlockPlan {
    references: TipReferencePair,
    plan: TipReferencePlan,
    two_references: bool,
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
    /// Borrows the block's reference pair.
    fn hold<'a, T: ReconSample>(
        &self,
        reference: &'a InterReferenceState<T>,
    ) -> Result<TipHeldReferences<'a, T>> {
        self.plan.hold(reference)
    }

    /// One unit's § 7.22 temporal motion record.
    fn temporal_record<T: ReconSample>(
        &self,
        reference: &InterReferenceState<T>,
        ref_frame_idx: &[u32],
        current_order_hint: u32,
        unit: &TipUnit,
        stored_mvs: [Mv; 2],
    ) -> TemporalMotionBlock {
        super::temporal::temporal_motion_block(
            reference,
            ref_frame_idx,
            unit.rect.luma_y / 4,
            unit.rect.luma_x / 4,
            unit.rect.luma_w.div_ceil(4),
            unit.rect.luma_h.div_ceil(4),
            self.frame_mi_rows,
            self.frame_mi_cols,
            current_order_hint,
            self.references.past_ref,
            self.two_references.then_some(self.references.future_ref),
            stored_mvs[0],
            stored_mvs[1],
            [None, None],
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn tip_block_plan<T: ReconSample>(
    info: splot_recon::DecodedFrameInfo,
    placed: &PlacedInterBlock,
    temporal: &TemporalMvContext,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
) -> Result<TipBlockPlan> {
    let references = temporal
        .tip_references()
        .ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
    let (inter, mode, weight, opfl_refine_type) = tip_prediction_controls(core.inter.as_ref())?;
    let implicit_mask = sequence
        .inter
        .as_ref()
        .is_some_and(|tools| tools.enable_imp_msk_bld);
    let blend = mc::CompoundBlend::average_with_implicit_mask(implicit_mask)
        .average_with_cwp_weight(weight);
    let two_references = tip_uses_two_references(weight);
    let enable_tip_refinemv = sequence
        .inter
        .as_ref()
        .is_some_and(|tools| tools.enable_tip_refinemv);
    let output = mode == TipFrameMode::AsOutput;
    let offsets = [
        (references.past_ref, references.past_offset),
        (references.future_ref, references.future_offset),
    ];
    let refined_references_allowed = tip_refinemv_references_allowed(
        core.frame_type,
        core.frame_size,
        ref_frame_idx,
        &reference.ref_order_hint,
        &reference.ref_frame_width,
        &reference.ref_frame_height,
        offsets,
    );
    let use_refinemv = sequence.inter.as_ref().is_some_and(|tools| {
        tip_uses_refinemv(
            output,
            tools.enable_refinemv,
            tools.enable_tip_refinemv,
            weight,
        )
    });
    let search_refinemv = use_refinemv && refined_references_allowed;
    let interpolation_filter = if output {
        output_interpolation_filter(inter)?
    } else {
        ReconInterpolationFilter::EightTapSharp
    };
    let unit_size = if output {
        output_prediction_unit_size(enable_tip_refinemv, interpolation_filter)
    } else {
        prediction_unit_size(placed.luma_w, placed.luma_h, enable_tip_refinemv)
    };
    let use_optflow = unit_size == 8
        && opfl_refine_type != 0
        && enable_tip_refinemv
        && interpolation_filter == ReconInterpolationFilter::EightTapSharp
        && two_references
        && (output
            || weight == mc::CWP_EQUAL
                && tip_optflow_references_allowed(
                    core.frame_type,
                    core.frame_size,
                    ref_frame_idx,
                    &reference.ref_order_hint,
                    &reference.ref_frame_width,
                    &reference.ref_frame_height,
                    offsets,
                ));
    let frame_size = info.coded_luma_size();
    let block_w = placed
        .luma_w
        .min(frame_size.width().saturating_sub(placed.luma_x));
    let block_h = placed
        .luma_h
        .min(frame_size.height().saturating_sub(placed.luma_y));
    if block_w == 0 || block_h == 0 {
        return Err(DecodeHeaderStateError::InvalidBlockGeometry.into());
    }
    let unit_count = block_w
        .div_ceil(unit_size)
        .checked_mul(block_h.div_ceil(unit_size))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "TIP prediction unit count",
        })?;
    let past = super::super::block_reference_slot(ref_frame_idx, references.past_ref)?;
    let future = two_references
        .then(|| super::super::block_reference_slot(ref_frame_idx, references.future_ref))
        .transpose()?;
    let plan = TipReferencePlan {
        past,
        future,
        interpolation_filter,
        blend,
        optflow_distances: use_optflow
            .then_some([references.past_offset, references.future_offset]),
        use_refinemv,
        search_refinemv,
        optflow_sad_threshold: use_optflow.then_some(if output { 15 } else { 6 }),
    };
    let batch_chroma_x = placed.luma_x.max(placed.chroma_luma_x);
    let batch_chroma_y = placed.luma_y.max(placed.chroma_luma_y);
    let batch_chroma_end_x = placed
        .luma_x
        .saturating_add(block_w)
        .min(placed.chroma_luma_x.saturating_add(placed.chroma_luma_w));
    let batch_chroma_end_y = placed
        .luma_y
        .saturating_add(block_h)
        .min(placed.chroma_luma_y.saturating_add(placed.chroma_luma_h));
    Ok(TipBlockPlan {
        references,
        plan,
        two_references,
        output,
        use_optflow,
        unit_size,
        unit_count,
        block_w,
        block_h,
        batch_rect: mc::McBlockRect {
            luma_x: placed.luma_x,
            luma_y: placed.luma_y,
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
        frame_mi_rows: frame_size.height().div_ceil(4),
        frame_mi_cols: frame_size.width().div_ceil(4),
    })
}

/// Fills `scratch.units` with the block's § 7.13.3.1 prediction units, or with
/// the first one alone when `first_only`.
///
/// The batch kernel reads the block's geometry off its first unit and samples
/// every other one through the motion grid, so its prediction half rebuilds one
/// unit where the motion half built them all.
fn build_units<T: ReconSample>(
    scratch: &mut TipReconstructScratch<T>,
    plan: &TipBlockPlan,
    placed: &PlacedInterBlock,
    temporal: &TemporalMvContext,
    first_only: bool,
) -> Result<()> {
    scratch.units.clear();
    scratch.output_samples.clear();
    scratch
        .units
        .try_reserve_exact(if first_only { 1 } else { plan.unit_count })
        .map_err(|_| inter_allocation!("TIP prediction units"))?;
    for local_y in (0..plan.block_h).step_by(plan.unit_size) {
        for local_x in (0..plan.block_w).step_by(plan.unit_size) {
            if first_only && !scratch.units.is_empty() {
                break;
            }
            let luma_x = placed.luma_x + local_x;
            let luma_y = placed.luma_y + local_y;
            let luma_w =
                prediction_unit_extent(plan.output, plan.block_w - local_x, plan.unit_size);
            let luma_h =
                prediction_unit_extent(plan.output, plan.block_h - local_y, plan.unit_size);
            let chroma_x = luma_x.max(placed.chroma_luma_x);
            let chroma_y = luma_y.max(placed.chroma_luma_y);
            let chroma_end_x = if plan.output {
                luma_x + luma_w
            } else {
                (luma_x + luma_w).min(placed.chroma_luma_x + placed.chroma_luma_w)
            };
            let chroma_end_y = if plan.output {
                luma_y + luma_h
            } else {
                (luma_y + luma_h).min(placed.chroma_luma_y + placed.chroma_luma_h)
            };
            let predict_chroma =
                placed.predict_chroma && chroma_end_x > chroma_x && chroma_end_y > chroma_y;
            let mvs = temporal
                .tip_candidate(luma_y / 8, luma_x / 8, placed.block.mv)
                .ok_or(DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            scratch.units.push(TipUnit {
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
                metadata: None,
            });
        }
    }
    Ok(())
}

/// Derives one § 7.13.5 TIP block's motion: the optical-flow grid its units
/// share, and every unit's § 7.22 temporal record.
///
/// This reads reference samples but writes none, so it is the half a motion
/// resolution pass runs. Only the optical-flow shape refines a unit's stored
/// motion vectors; every other shape stores the § 7.11.3 candidate the unit was
/// built with, and derives no grid at all.
#[allow(clippy::too_many_arguments)]
pub(super) fn motion<T: ReconSample>(
    scratch: &mut TipReconstructScratch<T>,
    temporal_records: &mut Vec<TemporalMotionBlock>,
    sink: &mc::WorkspaceSink<'_, '_, T>,
    placed: &PlacedInterBlock,
    temporal: &TemporalMvContext,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    tile_offset: ByteOffset,
) -> Result<Option<mc::CompoundMotionGrid>> {
    let plan = tip_block_plan(
        sink.info(),
        placed,
        temporal,
        sequence,
        core,
        ref_frame_idx,
        reference,
    )?;
    build_units(scratch, &plan, placed, temporal, false)?;
    let grid = plan
        .use_optflow
        .then(|| tip_motion_grid(scratch, &plan, sink, reference, tile_offset))
        .transpose()?;
    temporal_records
        .try_reserve(plan.unit_count)
        .map_err(|_| inter_allocation!("TIP temporal records"))?;
    let current_order_hint = core
        .display_order_hint()
        .ok_or(DecodeHeaderStateError::MissingDisplayOrderHint)?;
    for (index, unit) in scratch.units.iter().enumerate() {
        let stored_mvs = match grid.as_ref() {
            Some(grid) => grid.stored_mvs_at_index(index)?,
            None => unit.mvs,
        };
        temporal_records.push(plan.temporal_record(
            reference,
            ref_frame_idx,
            current_order_hint,
            unit,
            stored_mvs,
        ));
    }
    scratch.units.clear();
    Ok(grid)
}

/// Builds the optical-flow motion grid the block's units share.
fn tip_motion_grid<T: ReconSample>(
    scratch: &TipReconstructScratch<T>,
    plan: &TipBlockPlan,
    sink: &mc::WorkspaceSink<'_, '_, T>,
    reference: &InterReferenceState<T>,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundMotionGrid> {
    let held = plan.hold(reference)?;
    let prediction = held.prediction(&plan.plan)?;
    let first = scratch.units.first().ok_or(ReconError::ZeroDimension {
        field: "TIP compound batch",
    })?;
    let compound = prediction
        .block_params(first)
        .into_compound()
        .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
    if scratch.units.len() == 1 {
        return mc::compound_block_motion_grid(sink, compound, Some(8), tile_offset)?
            .ok_or_else(|| DecodeHeaderStateError::InvalidInterTipPredictionState.into());
    }
    mc::tip_batch_motion_grid(
        sink,
        compound,
        plan.block_w.div_ceil(plan.unit_size),
        scratch.units.len(),
        |index| {
            let unit = &scratch.units[index];
            (unit.rect, unit.mvs)
        },
        tile_offset,
    )
}

/// Writes the units the batch kernel did not cover, one reference borrow per
/// batch of units so a still-filtering reference is held for as little as the
/// § 7.2 filter phase publishing its later stripes can wait out.
/// Publishes finished unit samples into disjoint horizontal bands of the frame.
///
/// On the common path every unit already carries its own samples, so
/// publication is a pure disjoint scatter: each unit writes its own rectangle
/// and reads none. Units tile the frame, so grouping them by luma row yields
/// full-width bands whose rectangles are disjoint, which `rect_surfaces` proves
/// before handing out one exclusive surface each. Walking thousands of units on
/// one worker instead leaves the pool draining behind a § 7.10.6 output frame.
/// How many disjoint luma bands the finished units cover, or zero when any
/// unit still owes its prediction.
///
/// One band is the whole frame's worth of work in a single job, so the band
/// path would pay for its rect partition and pool entry and win nothing.
fn published_band_count<T: ReconSample>(scratch: &TipReconstructScratch<T>) -> usize {
    let mut bands = 0;
    let mut last = None;
    for unit in &scratch.units {
        let Some(metadata) = unit.metadata.as_ref() else {
            return 0;
        };
        let (_, y, _, _) = metadata.luma_rect();
        if last != Some(y) {
            bands += 1;
            last = Some(y);
        }
    }
    bands
}

fn publish_units_by_band<T: ReconSample>(
    scratch: &mut TipReconstructScratch<T>,
    workspace: &mut splot_recon::CurrentFrameWorkspace<T>,
    output_stride: usize,
) -> Result<()> {
    let height = workspace.info().coded_luma_size().height();
    let width = workspace.info().coded_luma_size().width();
    let mut bands: Vec<(usize, usize)> = Vec::new();
    let mut members: Vec<Vec<usize>> = Vec::new();
    for (index, unit) in scratch.units.iter().enumerate() {
        let Some(metadata) = unit.metadata.as_ref() else {
            return Err(DecodeHeaderStateError::InvalidInterTipPredictionState.into());
        };
        let (_, y, _, h) = metadata.luma_rect();
        let end = y.saturating_add(h).min(height);
        if let Some(last) = bands.last_mut()
            && last.0 == y
        {
            last.1 = last.1.max(end);
            if let Some(group) = members.last_mut() {
                group.push(index);
            }
            continue;
        }
        bands.push((y, end));
        members.push(vec![index]);
    }
    if bands.is_empty() {
        return Ok(());
    }
    let rects = bands
        .iter()
        .map(|(start, end)| splot_recon::PlaneRect::new(0, *start, width, end - start))
        .collect::<splot_recon::Result<Vec<_>>>()?;
    let surfaces = workspace.rect_surfaces(&rects)?;
    let chunks: Vec<&[T]> = scratch.output_samples.chunks_exact(output_stride).collect();
    let units = &scratch.units;
    surfaces
        .into_par_iter()
        .zip(members.into_par_iter())
        .try_for_each(|(mut surface, group)| -> Result<()> {
            let mut sink = mc::WorkspaceSink::Rect(&mut surface);
            for index in group {
                let metadata = units[index]
                    .metadata
                    .as_ref()
                    .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
                let samples = chunks.get(index).ok_or(ReconError::BufferLengthMismatch {
                    expected: output_stride,
                    actual: 0,
                })?;
                metadata.publish(samples, &mut sink)?;
            }
            Ok(())
        })?;
    for unit in &mut scratch.units {
        unit.metadata = None;
    }
    Ok(())
}

fn publish_unit_outputs<T: ReconSample>(
    scratch: &mut TipReconstructScratch<T>,
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    plan: &TipBlockPlan,
    mut grid: Option<mc::CompoundMotionGrid>,
    output_stride: usize,
    reference: &InterReferenceState<T>,
    tile_offset: ByteOffset,
) -> Result<()> {
    if splot_parallel::on_worker_pool()
        && published_band_count(scratch) > 1
        && let mc::WorkspaceSink::Frame(workspace) = sink
    {
        return publish_units_by_band(scratch, workspace, output_stride);
    }
    let mut output_chunks = scratch.output_samples.chunks_exact(output_stride);
    let mut units_per_hold = scratch.units.len().max(1);
    if scratch.units.iter().any(|unit| unit.metadata.is_none()) && !plan.hold(reference)?.settled()
    {
        units_per_hold = plan.block_w.div_ceil(plan.unit_size).max(1);
    }
    for batch in scratch.units.chunks_mut(units_per_hold) {
        let held = batch
            .iter()
            .any(|unit| unit.metadata.is_none())
            .then(|| plan.hold(reference))
            .transpose()?;
        let prediction = held
            .as_ref()
            .map(|held| held.prediction(&plan.plan))
            .transpose()?;
        for unit in batch {
            if let Some(metadata) = unit.metadata.take() {
                let samples = output_chunks
                    .next()
                    .ok_or(ReconError::BufferLengthMismatch {
                        expected: output_stride,
                        actual: 0,
                    })?;
                metadata.publish(samples, sink)?;
                continue;
            }
            let params = prediction
                .as_ref()
                .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?
                .block_params(unit);
            if plan.use_optflow {
                let compound = params
                    .into_compound()
                    .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?;
                mc::predict_compound_average_block(sink, compound, grid.take(), tile_offset)?
                    .publish(sink)?;
            } else {
                mc::motion_compensate_inter_block_into(sink, params, tile_offset)?;
            }
        }
    }
    Ok(())
}

fn resize_output_samples<T: ReconSample>(samples: &mut Vec<T>, len: usize) -> Result<()> {
    samples
        .try_reserve(len.saturating_sub(samples.len()))
        .map_err(|_| inter_allocation!("TIP compound output samples"))?;
    samples.resize(len, T::default());
    Ok(())
}

/// Reconstructs one § 7.13.5 TIP block into `sink` from the motion half's grid.
///
/// A block whose units carry the § 7.13.3.1 optical-flow shape is predicted by
/// the fixed-unit batch kernel, which spawns no pool work and writes one
/// rectangle per plane, so every sink takes it. The per-unit fan-out is only
/// available for the frame sink: a task predicting into an out-of-order
/// surface holds a reference borrow across its prediction, and spawned work
/// that waits on that borrow would never run.
#[allow(clippy::too_many_arguments)]
pub(super) fn predict<T: ReconSample>(
    scratch: &mut TipReconstructScratch<T>,
    residual_scratch: &mut InterResidualReconScratch<T>,
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    grid: Option<mc::CompoundMotionGrid>,
    placed: &PlacedInterBlock,
    residual_blocks: &[InterResidualBlock],
    temporal: &TemporalMvContext,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<()> {
    let plan = tip_block_plan(
        sink.info(),
        placed,
        temporal,
        sequence,
        core,
        ref_frame_idx,
        reference,
    )?;
    let batched_output = plan.use_optflow && plan.unit_count > 1;
    build_units(scratch, &plan, placed, temporal, batched_output)?;
    let parallel_output = !plan.use_optflow
        && matches!(sink, mc::WorkspaceSink::Frame(_))
        && plan.two_references
        && splot_parallel::on_worker_pool();
    let output_stride = mc::mc_planes(sink.info().pixel_format())
        .into_iter()
        .map(|(_, sub_x, sub_y)| (plan.unit_size >> sub_x) * (plan.unit_size >> sub_y))
        .sum::<usize>();
    if parallel_output || batched_output {
        let arena_len =
            plan.unit_count
                .checked_mul(output_stride)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "TIP compound output arena length",
                })?;
        resize_output_samples(&mut scratch.output_samples, arena_len)?;
    }
    let mut grid = grid;
    let batch_metadata = if batched_output {
        let held = plan.hold(reference)?;
        Some(compute_batched_output(
            sink,
            &scratch.units,
            &mut scratch.output_samples,
            &held.prediction(&plan.plan)?,
            &plan,
            grid.take()
                .ok_or(DecodeHeaderStateError::InvalidInterTipPredictionState)?,
            tile_offset,
        )?)
    } else {
        None
    };
    if parallel_output {
        let held = plan.plan.hold(reference)?;
        compute_parallel_outputs(
            sink,
            &mut scratch.units,
            &mut scratch.output_samples,
            output_stride,
            &held.prediction(&plan.plan)?,
            tile_offset,
        )?;
    }
    if let Some(metadata) = batch_metadata.as_ref() {
        metadata.publish(&scratch.output_samples, sink)?;
    } else {
        publish_unit_outputs(
            scratch,
            sink,
            &plan,
            grid,
            output_stride,
            reference,
            tile_offset,
        )?;
    }
    scratch.units.clear();
    if let Some(residual) = placed.block.residual.as_ref() {
        super::super::add_inter_residual_to_workspace(
            residual_scratch,
            sink,
            residual,
            residual_blocks,
            qindex,
            luma_use_tcq,
            residual_use_ddt,
            false,
            bit_depth,
        )?;
    }
    Ok(())
}

/// Luma rows per § 7.10.6 TIP-as-output prediction band.
///
/// The whole frame is one TIP block, so predicting it in one pass sized the
/// compound scratch at frame scale. Banding keeps that scratch bounded; 64 is a
/// multiple of both prediction unit sizes and of chroma subsampling, so a band
/// boundary never splits a unit.
const TIP_OUTPUT_BAND_LUMA_ROWS: usize = 64;

pub(in crate::prediction::inter) fn reconstruct_output<T: ReconSample>(
    decode_scratch: &mut super::InterDecodeScratch<T>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    geometry: super::super::FrameDecodeGeometry,
    offset: ByteOffset,
) -> Result<(DecodedFrame<T>, TemporalMotionField)> {
    let info = geometry.info();
    let bit_depth = info.bit_depth();
    let inter = core
        .inter
        .as_ref()
        .ok_or(DecodeHeaderStateError::MissingInterControlRegion)?;
    let ref_frame_idx = &inter.ref_frame_idx;
    let width = info.coded_luma_size().width();
    let height = info.coded_luma_size().height();
    let (mi_rows, mi_cols) = geometry.mi_dimensions();
    let sb_h4 = geometry.sb_h4();
    let projection_step = tmvp_projection_step(core);
    let current_order_hint = core
        .display_order_hint()
        .ok_or(DecodeHeaderStateError::MissingDisplayOrderHint)?;
    if sequence.partition.is_none() {
        return Err(DecodeHeaderStateError::IncompleteInterFrameTools.into());
    }
    let ref_motion_fields = reference.resolve_motion_fields(ref_frame_idx)?;
    let temporal = decode_scratch
        .temporal_context
        .get_or_insert_with(TemporalMvContext::empty);
    temporal.refresh_from_references(
        (mi_rows, mi_cols),
        current_order_hint,
        TemporalProjectionConfig {
            frame_size: (width, height),
            step: projection_step,
            unit_size8: tmvp_unit_size8(projection_step, sb_h4),
            enable_tip: sequence
                .inter
                .as_ref()
                .is_some_and(|tools| tools.enable_tip),
            enable_trajectory: sequence
                .inter
                .as_ref()
                .is_some_and(|tools| tools.enable_mv_traj),
            reduced: sequence
                .inter
                .as_ref()
                .is_some_and(|tools| tools.reduced_ref_frame_mvs_mode),
        },
        ref_frame_idx,
        &reference.ref_valid,
        &reference.ref_order_hint,
        &ref_motion_fields,
    )?;
    let tip_pair = super::super::find_mv_stack::tip_reference_pair_from_hints(
        current_order_hint,
        temporal.reference_order_hints(),
    );
    prepare_motion_field(temporal, core, sb_h4, tip_pair)?;
    let global_mv = inter
        .tip_global_mv
        .ok_or(DecodeHeaderStateError::IncompleteTipOutput)?;
    let mut workspace = CurrentFrameWorkspace::<T>::new_recycled(info)?; // § 7.10.6 predicts every coded sample of the frame below before `freeze`
    let mut motion_field = geometry
        .new_motion_field(temporal.reference_order_hints())
        .ok_or(ReconError::WorkspaceAllocationFailed {
            plane: splot_recon::PlaneId::Y,
            context: "TIP-output motion field",
        })?;
    let band_h = TIP_OUTPUT_BAND_LUMA_ROWS.min(height);
    let mut placed = PlacedInterBlock {
        luma_x: 0,
        luma_y: 0,
        luma_w: width,
        luma_h: band_h,
        chroma_luma_x: 0,
        chroma_luma_y: 0,
        chroma_luma_w: width,
        chroma_luma_h: band_h,
        predict_chroma: sequence.general.chroma_format_idc != ChromaFormatIdc::Monochrome,
        sub8x8_chroma: false,
        interintra_chroma: false,
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
    let mut scratch = TipReconstructScratch::default();
    let mut residual_scratch = InterResidualReconScratch::default();
    let mut temporal_records = Vec::new();
    let mut sink = mc::WorkspaceSink::Frame(&mut workspace);
    let mut band_y = 0;
    while band_y < height {
        let rows = band_h.min(height - band_y);
        placed.luma_y = band_y;
        placed.luma_h = rows;
        placed.chroma_luma_y = band_y;
        placed.chroma_luma_h = rows;
        let grid = motion(
            &mut scratch,
            &mut temporal_records,
            &sink,
            &placed,
            temporal,
            sequence,
            core,
            ref_frame_idx,
            reference,
            offset,
        )?;
        predict(
            &mut scratch,
            &mut residual_scratch,
            &mut sink,
            grid,
            &placed,
            &[],
            temporal,
            sequence,
            core,
            ref_frame_idx,
            reference,
            0,
            false,
            false,
            bit_depth,
            offset,
        )?;
        band_y += rows;
    }
    super::temporal::commit_temporal_motion_blocks(&mut motion_field, &temporal_records);
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
            &mut workspace,
            output_prediction_unit_size(enable_tip_refinemv, interpolation_filter),
            quant,
            seq_quant.base_uv_ac_delta_q,
            core.tile_info
                .as_ref()
                .map(|tile| (tile.mi_col_starts.as_slice(), tile.mi_row_starts.as_slice())),
            sequence
                .filter
                .is_some_and(|filter| filter.disable_loopfilters_across_tiles),
            bit_depth,
        )
        .map_err(|_| DecodeHeaderStateError::IncompleteTipOutput)?;
    }
    Ok((workspace.freeze()?, motion_field))
}

#[cfg(test)]
mod tests {
    use super::{
        TipHeldReferences, TipPrediction, TipReferencePlan, TipUnit, compute_parallel_outputs,
        output_interpolation_filter, output_prediction_unit_size, prediction_unit_extent,
        prediction_unit_size, resize_output_samples, tip_optflow_references_allowed,
        tip_prediction_controls, tip_refinemv_offsets_allowed, tip_refinemv_references_allowed,
        tip_temporal_mvs, tip_uses_refinemv, tip_uses_two_references, tmvp_unit_size8,
    };
    use crate::prediction::inter::reference::{HeldFrameSamples, ReferenceSamples};
    use crate::prediction::inter::{Mv, mc};
    use crate::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};
    use splot_core::headers::frame::{
        FrameSize, FrameType, InterControl, InterpolationFilter as FrameInterpolationFilter,
        TipFrameMode,
    };
    use splot_core::span::ByteOffset;
    use splot_parallel::{ThreadCount, WorkerPool};
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
    fn tip_output_sample_allocation_failure_is_typed() {
        let mut samples = Vec::<u8>::new();
        assert!(matches!(
            resize_output_samples(&mut samples, usize::MAX),
            Err(crate::DecodeError::Reconstruction {
                source: splot_recon::ReconError::WorkspaceAllocationFailed {
                    plane: PlaneId::Y,
                    context: "TIP compound output samples"
                }
            })
        ));
        assert!(samples.is_empty());
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

    #[test]
    fn tip_parallel_output_error_precedes_publication() -> Result<(), Box<dyn std::error::Error>> {
        let reference = tip_workspace()?.freeze()?;
        let mut workspace = tip_workspace()?;
        let mut units = [TipUnit {
            rect: mc::McBlockRect::from_luma_rect(0, 0, 8, 8),
            has_chroma: true,
            mvs: [Mv::ZERO; 2],
            metadata: None,
        }];
        let prediction = TipPrediction {
            reference0: ReferenceSamples::settled(&reference),
            reference1: None,
            interpolation_filter: InterpolationFilter::EightTap,
            blend: mc::CompoundBlend::default(),
            optflow_distances: None,
            use_refinemv: false,
            search_refinemv: false,
            optflow_sad_threshold: None,
        };
        let mut output = [7u8; 96];
        let pool = WorkerPool::new(ThreadCount::Fixed(2.try_into()?))?;
        let result = {
            let sink = mc::WorkspaceSink::Frame(&mut workspace);
            pool.install(|| {
                compute_parallel_outputs(
                    &sink,
                    &mut units,
                    &mut output,
                    96,
                    &prediction,
                    ByteOffset::new(0),
                )
            })
        };

        assert!(result.is_err());
        assert!(units[0].metadata.is_none());
        assert_eq!(output, [7; 96]);
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
        let allowed = |offsets: (i32, i32)| {
            tip_optflow_references_allowed(
                Some(FrameType::Inter),
                Some(FrameSize::new(64, 64)),
                &[0, 1],
                &[1, 2],
                &[64, 64],
                &[64, 64],
                [(0, offsets.0), (1, offsets.1)],
            )
        };

        assert!(allowed((1, -2)));
        assert!(!allowed((1, 2)));
        assert!(!tip_optflow_references_allowed(
            Some(FrameType::Inter),
            Some(FrameSize::new(64, 64)),
            &[0, 1],
            &[1, 2],
            &[63, 64],
            &[64, 64],
            [(0, 1), (1, -2)],
        ));
    }

    #[test]
    fn tip_temporal_storage_uses_refined_mvs_only_with_optflow() {
        let candidate = [Mv { row: 1, col: 2 }, Mv { row: 3, col: 4 }];
        let refined = [Mv { row: 5, col: 6 }, Mv { row: 7, col: 8 }];

        assert_eq!(tip_temporal_mvs(false, candidate, Some(refined)), candidate);
        assert_eq!(tip_temporal_mvs(true, candidate, Some(refined)), refined);
        assert_eq!(tip_temporal_mvs(true, candidate, None), candidate);
    }

    #[test]
    fn tip_refinemv_reference_gate_maps_slots_and_rejects_ineligible_state() {
        let allowed = |frame_type, frame_size, order_hints: &[u32], widths: &[u32]| {
            tip_refinemv_references_allowed(
                frame_type,
                frame_size,
                &[2, 0],
                order_hints,
                widths,
                &[352, 1, 352],
                [(0, 4), (1, -4)],
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
        assert!(!tip_refinemv_references_allowed(
            Some(FrameType::Inter),
            size,
            &[2],
            &[0, 0, 0],
            &[352, 1, 352],
            &[352, 1, 352],
            [(0, 4), (1, -4)],
        ));
    }

    #[test]
    fn tip_refinemv_scale_gate_uses_spec_rounding() {
        assert!(tip_refinemv_references_allowed(
            Some(FrameType::Inter),
            Some(FrameSize::new(65_536, 65_536)),
            &[0, 1],
            &[0, 0],
            &[65_535, 65_536],
            &[65_535, 65_536],
            [(0, 4), (1, -4)],
        ));
    }
}
