// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::prediction::inter::InterResidualReconScratch;
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_parallel::prelude::*;
use splot_recon::{DecodedFrame, PixelFormat, ReconError};

use super::super::reference::{HeldFrameSamples, ReferenceSamples};

use super::super::find_mv_stack::TemporalMotionBlock;

#[doc = "AV2 § 7.13.3.1 Tip_Weighting_Factor."]
const TIP_WEIGHTING_FACTORS: [i16; 8] = [8, 12, 16, 18, 20, 4, 6, -4];
const TIP_SINGLE_WEIGHT: i16 = 16;

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
    compound: bool,
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
        tile_offset: ByteOffset,
    ) -> Result<TipHeldReferences<'a, T>> {
        Ok(TipHeldReferences {
            past: super::super::hold_reference_slot(reference, self.past, tile_offset)?,
            future: match self.future {
                Some(slot) if slot != self.past => Some(super::super::hold_reference_slot(
                    reference,
                    slot,
                    tile_offset,
                )?),
                _ => None,
            },
            compound: self.future.is_some(),
        })
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
        Ok(TipPrediction {
            reference0: self.past.samples()?,
            reference1: self
                .compound
                .then(|| self.future.as_ref().unwrap_or(&self.past).samples())
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

fn tip_reference_pair_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_missing!(
        "inter_tip_reference_pair",
        tile_offset,
        "inter.tip.closest_past_and_future",
        SPEC_MODE_INFO
    )
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

#[allow(clippy::too_many_arguments)]
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
                .ok_or_else(|| tip_reference_pair_error(tile_offset))?;
            let metadata = mc::predict_compound_average_block_into(
                sink,
                compound,
                prediction.optflow_distances.is_some().then_some(8),
                tile_offset,
                samples,
            )?;
            if prediction.optflow_distances.is_some() {
                unit.mvs = tip_temporal_mvs(true, unit.mvs, metadata.stored_mvs_at_origin()?);
            }
            unit.metadata = Some(Box::new(metadata));
            Ok(())
        })
}

#[allow(clippy::too_many_arguments)]
fn compute_batched_output<T: ReconSample>(
    sink: &mc::WorkspaceSink<'_, '_, T>,
    units: &[TipUnit],
    output_samples: &mut [T],
    prediction: &TipPrediction<'_, T>,
    batch_rect: mc::McBlockRect,
    batch_has_chroma: bool,
    columns: usize,
    tile_offset: ByteOffset,
) -> Result<mc::CompoundBlockMetadata> {
    let first = units.first().ok_or(ReconError::ZeroDimension {
        field: "TIP compound batch",
    })?;
    let compound = prediction
        .block_params(first)
        .into_compound()
        .ok_or_else(|| tip_reference_pair_error(tile_offset))?;
    mc::predict_tip_compound_batch_into(
        sink,
        compound,
        batch_rect,
        batch_has_chroma,
        columns,
        units.iter().map(|unit| (unit.rect, unit.mvs)),
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

fn output_interpolation_filter(
    inter: &splot_core::headers::frame::InterControl,
    offset: ByteOffset,
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
        _ => Err(inter_cap!(
            "tip_output_interpolation_filter",
            offset,
            "inter.tip_output.interpolation_filter",
            "7.10.6"
        )),
    }
}

pub(super) fn prepare_motion_field(
    temporal: &mut TemporalMvContext,
    core: &FrameHeaderCore,
    sb_h4: usize,
) {
    let Some(inter) = core.inter.as_ref() else {
        return;
    };
    let projection_step = tmvp_projection_step(core);
    let tmvp_unit_size8 = tmvp_unit_size8(projection_step, sb_h4);
    if inter.tip_frame_mode == Some(TipFrameMode::Disabled) {
        temporal.fill_sampling_gaps(projection_step, tmvp_unit_size8);
        return;
    }
    _ = temporal.prepare_tip(
        projection_step,
        tmvp_unit_size8,
        inter.allow_tip_hole_fill.unwrap_or(false),
    );
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
        .map_err(|_| symbol_read_error(tile_offset))?;
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

#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct<T: ReconSample>(
    scratch: &mut TipReconstructScratch<T>,
    residual_scratch: &mut InterResidualReconScratch<T>,
    temporal_records: &mut Vec<TemporalMotionBlock>,
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    allow_unit_parallelism: bool,
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
    let references = temporal
        .tip_references()
        .ok_or_else(|| tip_reference_pair_error(tile_offset))?;
    let inter = core.inter.as_ref().ok_or_else(|| {
        inter_missing!(
            "inter_tip_control",
            tile_offset,
            "inter.tip.control",
            SPEC_MODE_INFO
        )
    })?;
    let weight_index = usize::from(inter.tip_global_wtd_index.unwrap_or(0));
    let weight = TIP_WEIGHTING_FACTORS
        .get(weight_index)
        .copied()
        .ok_or_else(|| {
            inter_cap!(
                "inter_tip_weight_index",
                tile_offset,
                "inter.tip.global_weight_index",
                SPEC_MODE_INFO
            )
        })?;
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
    let output = inter.tip_frame_mode == Some(TipFrameMode::AsOutput);
    let refined_references_allowed = tip_refinemv_references_allowed(
        core.frame_type,
        core.frame_size,
        ref_frame_idx,
        &reference.ref_order_hint,
        &reference.ref_frame_width,
        &reference.ref_frame_height,
        [
            (references.past_ref, references.past_offset),
            (references.future_ref, references.future_offset),
        ],
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
        output_interpolation_filter(inter, tile_offset)?
    } else {
        ReconInterpolationFilter::EightTapSharp
    };
    let unit_size = if output {
        output_prediction_unit_size(enable_tip_refinemv, interpolation_filter)
    } else {
        prediction_unit_size(placed.luma_w, placed.luma_h, enable_tip_refinemv)
    };
    let use_optflow = unit_size == 8
        && inter.opfl_refine_type.unwrap_or(0) != 0
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
                    [
                        (references.past_ref, references.past_offset),
                        (references.future_ref, references.future_offset),
                    ],
                ));
    let frame_size = sink.info().coded_luma_size();
    let frame_mi_rows = frame_size.height().div_ceil(4);
    let frame_mi_cols = frame_size.width().div_ceil(4);
    let block_w = placed
        .luma_w
        .min(frame_size.width().saturating_sub(placed.luma_x));
    let block_h = placed
        .luma_h
        .min(frame_size.height().saturating_sub(placed.luma_y));

    let unit_count = block_w
        .div_ceil(unit_size)
        .checked_mul(block_h.div_ceil(unit_size))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "TIP prediction unit count",
        })?;
    scratch.units.clear();
    scratch.output_samples.clear();
    scratch.units.try_reserve_exact(unit_count).map_err(|_| {
        inter_cap!(
            "inter_tip_unit_allocation",
            tile_offset,
            "inter.tip.prediction_unit_allocation",
            "7.13.3.1"
        )
    })?;
    let units_timer = crate::timing::start();
    let plan = (block_w > 0 && block_h > 0)
        .then(|| {
            let past = super::super::block_reference_slot(
                ref_frame_idx,
                references.past_ref,
                tile_offset,
            )?;
            let future = two_references
                .then(|| {
                    super::super::block_reference_slot(
                        ref_frame_idx,
                        references.future_ref,
                        tile_offset,
                    )
                })
                .transpose()?;
            Ok::<_, crate::error::DecodeError>(TipReferencePlan {
                past,
                future,
                interpolation_filter,
                blend,
                optflow_distances: use_optflow
                    .then_some([references.past_offset, references.future_offset]),
                use_refinemv,
                search_refinemv,
                optflow_sad_threshold: use_optflow.then_some(if output { 15 } else { 6 }),
            })
        })
        .transpose()?;
    for local_y in (0..block_h).step_by(unit_size) {
        for local_x in (0..block_w).step_by(unit_size) {
            let luma_x = placed.luma_x + local_x;
            let luma_y = placed.luma_y + local_y;
            let luma_w = (block_w - local_x).min(unit_size);
            let luma_h = (block_h - local_y).min(unit_size);
            let chroma_x = luma_x.max(placed.chroma_luma_x);
            let chroma_y = luma_y.max(placed.chroma_luma_y);
            let chroma_end_x = (luma_x + luma_w).min(placed.chroma_luma_x + placed.chroma_luma_w);
            let chroma_end_y = (luma_y + luma_h).min(placed.chroma_luma_y + placed.chroma_luma_h);
            let predict_chroma =
                placed.predict_chroma && chroma_end_x > chroma_x && chroma_end_y > chroma_y;
            let mvs = temporal
                .tip_candidate(luma_y / 8, luma_x / 8, placed.block.mv)
                .ok_or_else(|| {
                    inter_missing!(
                        "inter_tip_motion_field",
                        tile_offset,
                        "inter.tip.motion_field",
                        SPEC_MODE_INFO
                    )
                })?;
            let rect = mc::McBlockRect {
                luma_x,
                luma_y,
                luma_w,
                luma_h,
                chroma_luma_x: chroma_x,
                chroma_luma_y: chroma_y,
                chroma_luma_w: chroma_end_x.saturating_sub(chroma_x),
                chroma_luma_h: chroma_end_y.saturating_sub(chroma_y),
            };
            scratch.units.push(TipUnit {
                rect,
                has_chroma: predict_chroma,
                mvs,
                metadata: None,
            });
        }
    }
    if units_timer.is_some() {
        crate::timing::report_detail(
            "inter_tip_units",
            units_timer,
            &format!(
                "units={} columns={} width={block_w} height={block_h} unit_size={unit_size}",
                scratch.units.len(),
                block_w.div_ceil(unit_size)
            ),
        );
    }
    let parallel_output =
        allow_unit_parallelism && two_references && splot_parallel::on_worker_pool();
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
    let batch_has_chroma = placed.predict_chroma
        && batch_chroma_end_x > batch_chroma_x
        && batch_chroma_end_y > batch_chroma_y;
    let batch_rect = mc::McBlockRect {
        luma_x: placed.luma_x,
        luma_y: placed.luma_y,
        luma_w: block_w,
        luma_h: block_h,
        chroma_luma_x: batch_chroma_x,
        chroma_luma_y: batch_chroma_y,
        chroma_luma_w: batch_chroma_end_x.saturating_sub(batch_chroma_x),
        chroma_luma_h: batch_chroma_end_y.saturating_sub(batch_chroma_y),
    };
    let batched_output = parallel_output
        && use_optflow
        && unit_size == 8
        && scratch.units.len() > 1
        && splot_parallel::current_pool_width() == 1;
    let output_stride = mc::mc_planes(sink.info().pixel_format())
        .into_iter()
        .map(|(_, sub_x, sub_y)| (unit_size >> sub_x) * (unit_size >> sub_y))
        .sum::<usize>();
    if parallel_output {
        let arena_len = scratch.units.len().checked_mul(output_stride).ok_or(
            ReconError::ArithmeticOverflow {
                context: "TIP compound output arena length",
            },
        )?;
        scratch.output_samples.resize(arena_len, T::default());
    }
    let prediction_timer = crate::timing::start();
    let batch_metadata = if batched_output {
        let plan = plan
            .as_ref()
            .ok_or_else(|| tip_reference_pair_error(tile_offset))?;
        let held = plan.hold(reference, tile_offset)?;
        Some(compute_batched_output(
            sink,
            &scratch.units,
            &mut scratch.output_samples,
            &held.prediction(plan)?,
            batch_rect,
            batch_has_chroma,
            block_w.div_ceil(unit_size),
            tile_offset,
        )?)
    } else {
        None
    };
    if parallel_output
        && !batched_output
        && let Some(plan) = plan.as_ref()
    {
        let held = plan.hold(reference, tile_offset)?;
        compute_parallel_outputs(
            sink,
            &mut scratch.units,
            &mut scratch.output_samples,
            output_stride,
            &held.prediction(plan)?,
            tile_offset,
        )?;
    }
    crate::timing::report("inter_tip_prediction", prediction_timer);
    let publish_timer = crate::timing::start();
    temporal_records.try_reserve(unit_count).map_err(|_| {
        inter_cap!(
            "inter_tip_temporal_record_allocation",
            tile_offset,
            "inter.tip.temporal_record_allocation",
            "7.22"
        )
    })?;
    if let Some(metadata) = batch_metadata.as_ref() {
        metadata.publish(&scratch.output_samples, sink)?;
    }
    let mut output_chunks = scratch.output_samples.chunks_exact(output_stride);
    let mut units_per_hold = scratch.units.len().max(1);
    if batch_metadata.is_none()
        && scratch.units.iter().any(|unit| unit.metadata.is_none())
        && let Some(plan) = plan.as_ref()
        && !plan.hold(reference, tile_offset)?.settled()
    {
        units_per_hold = block_w.div_ceil(unit_size).max(1);
    }
    let mut index = 0usize;
    for batch in scratch.units.chunks_mut(units_per_hold) {
        let held = (batch_metadata.is_none() && batch.iter().any(|unit| unit.metadata.is_none()))
            .then(|| {
                plan.as_ref()
                    .ok_or_else(|| tip_reference_pair_error(tile_offset))
                    .and_then(|plan| plan.hold(reference, tile_offset))
            })
            .transpose()?;
        let prediction = held
            .as_ref()
            .zip(plan.as_ref())
            .map(|(held, plan)| held.prediction(plan))
            .transpose()?;
        for unit in batch {
            let stored_mvs = if let Some(metadata) = batch_metadata.as_ref() {
                metadata.stored_mvs_at_index(index)?.unwrap_or(unit.mvs)
            } else if let Some(metadata) = unit.metadata.take() {
                let samples = output_chunks
                    .next()
                    .ok_or(ReconError::BufferLengthMismatch {
                        expected: output_stride,
                        actual: 0,
                    })?;
                metadata.publish(samples, sink)?;
                unit.mvs
            } else if use_optflow {
                let params = prediction
                    .as_ref()
                    .ok_or_else(|| tip_reference_pair_error(tile_offset))?
                    .block_params(unit);
                mc::motion_compensate_inter_block_with_optflow_mvs_into(
                    sink,
                    params,
                    8,
                    tile_offset,
                )?
                .unwrap_or(unit.mvs)
            } else {
                let params = prediction
                    .as_ref()
                    .ok_or_else(|| tip_reference_pair_error(tile_offset))?
                    .block_params(unit);
                mc::motion_compensate_inter_block_into(sink, params, tile_offset)?;
                unit.mvs
            };
            temporal_records.push(super::temporal::temporal_motion_block(
                reference,
                ref_frame_idx,
                unit.rect.luma_y / 4,
                unit.rect.luma_x / 4,
                unit.rect.luma_w.div_ceil(4),
                unit.rect.luma_h.div_ceil(4),
                frame_mi_rows,
                frame_mi_cols,
                core.display_order_hint().unwrap_or(0),
                references.past_ref,
                two_references.then_some(references.future_ref),
                stored_mvs[0],
                stored_mvs[1],
                [None, None],
            ));
            index += 1;
        }
    }
    scratch.units.clear();
    crate::timing::report("inter_tip_publish", publish_timer);
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
            tile_offset,
        )?;
    }
    Ok(())
}

pub(in crate::prediction::inter) fn reconstruct_output<T: ReconSample>(
    decode_scratch: &mut super::InterDecodeScratch<T>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<T>,
    bit_depth: BitDepth,
    offset: ByteOffset,
) -> Result<(DecodedFrame<T>, TemporalMotionField)> {
    let missing = |message| unsupported_at("tip_output_state", offset, message, "7.10.6");
    let frame_size = core
        .frame_size
        .ok_or_else(|| missing("missing required input: inter.tip_output.frame_size"))?;
    let inter = core
        .inter
        .as_ref()
        .ok_or_else(|| missing("missing required input: inter.tip_output.control"))?;
    let ref_frame_idx = &inter.ref_frame_idx;
    let width = usize::try_from(frame_size.width)
        .map_err(|_| missing("unsupported capability: inter.tip_output.frame_dimensions"))?;
    let height = usize::try_from(frame_size.height)
        .map_err(|_| missing("unsupported capability: inter.tip_output.frame_dimensions"))?;
    let (mi_rows, mi_cols) = (height.div_ceil(4), width.div_ceil(4));
    let sb_h4 = super::superblock_h4(sequence, core)
        .ok_or_else(|| missing("missing required input: inter.tip_output.superblock_size"))?;
    let projection_step = tmvp_projection_step(core);
    let ref_motion_fields = reference.resolve_motion_fields().ok_or_else(|| {
        missing("missing required input: inter.tip_output.reference_motion_field")
    })?;
    let temporal = decode_scratch
        .temporal_context
        .get_or_insert_with(TemporalMvContext::empty);
    temporal
        .refresh_from_references(
            (mi_rows, mi_cols),
            core.display_order_hint().unwrap_or(0),
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
        )
        .ok_or_else(|| missing("missing required input: inter.tip_output.temporal_context"))?;
    prepare_motion_field(temporal, core, sb_h4);
    let global_mv = inter
        .tip_global_mv
        .ok_or_else(|| missing("missing required input: inter.tip_output.global_mv"))?;
    let visible_luma_rect =
        crate::pipeline::derive_visible_luma_rect(sequence, frame_size.width, frame_size.height)?;
    let mut workspace =
        crate::pipeline::reconstruct::new_general_intra_workspace_with_visible_rect::<T>(
            width,
            height,
            bit_depth,
            PixelFormat::from_av2_chroma_format_idc(sequence.general.chroma_format_idc.get())?,
            visible_luma_rect,
        )?;
    let mut motion_field = TemporalMotionField::new(mi_rows, mi_cols)
        .ok_or_else(|| missing("unsupported capability: inter.tip_output.motion_field"))?;
    motion_field.set_reference_metadata(true, (width, height), temporal.reference_order_hints());
    let placed = PlacedInterBlock {
        luma_x: 0,
        luma_y: 0,
        luma_w: width,
        luma_h: height,
        chroma_luma_x: 0,
        chroma_luma_y: 0,
        chroma_luma_w: width,
        chroma_luma_h: height,
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
    reconstruct(
        &mut scratch,
        &mut residual_scratch,
        &mut temporal_records,
        &mut mc::WorkspaceSink::Frame(&mut workspace),
        true,
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
    super::temporal::commit_temporal_motion_blocks(&mut motion_field, &temporal_records);
    if inter.apply_deblocking_filter_tip == Some(true) {
        let quant = core
            .quantization_params
            .ok_or_else(|| missing("missing required input: inter.tip_output.quantizer"))?;
        let tq = sequence.transform_quant_entropy.as_ref().ok_or_else(|| {
            missing("missing required input: inter.tip_output.sequence_quantizer")
        })?;
        let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);
        let interpolation_filter = output_interpolation_filter(inter, offset)?;
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
        .map_err(|_| missing("unsupported capability: inter.tip_output.deblocking"))?;
    }
    Ok((workspace.freeze()?, motion_field))
}

#[cfg(test)]
mod tests {
    use super::{
        TipPrediction, TipUnit, compute_parallel_outputs, output_prediction_unit_size,
        prediction_unit_size, tip_optflow_references_allowed, tip_refinemv_offsets_allowed,
        tip_refinemv_references_allowed, tip_temporal_mvs, tip_uses_refinemv,
        tip_uses_two_references, tmvp_unit_size8,
    };
    use crate::prediction::inter::reference::ReferenceSamples;
    use crate::prediction::inter::{Mv, mc};
    use splot_core::headers::frame::{FrameSize, FrameType};
    use splot_core::span::ByteOffset;
    use splot_parallel::{ThreadCount, WorkerPool};
    use splot_recon::{
        BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, InterpolationFilter, OutputIndex,
        PixelFormat, PlaneRect, PlaneSize,
    };

    #[test]
    fn tip_reference_unit_size_follows_refinement_and_large_block_gates() {
        assert_eq!(prediction_unit_size(64, 32, false), 16);
        assert_eq!(prediction_unit_size(8, 32, false), 8);
        assert_eq!(prediction_unit_size(64, 32, true), 8);
        assert_eq!(prediction_unit_size(256, 256, true), 16);
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
