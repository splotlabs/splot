// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_parallel::prelude::*;
use splot_recon::{DecodedFrame, PixelFormat};

#[doc = "AV2 § 7.13.3.1 Tip_Weighting_Factor."]
const TIP_WEIGHTING_FACTORS: [i16; 8] = [8, 12, 16, 18, 20, 4, 6, -4];
const TIP_SINGLE_WEIGHT: i16 = 16;

#[derive(Clone, Copy)]
struct TipUnit<'a, T: ReconSample> {
    params: mc::InterBlockParams<'a, T>,
    mvs: [Mv; 2],
    luma_x: usize,
    luma_y: usize,
    luma_w: usize,
    luma_h: usize,
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
    if frame_type == Some(FrameType::Switch)
        || !tip_refinemv_offsets_allowed(references[0].1, references[1].1)
    {
        return false;
    }
    let Some(frame_size) = frame_size else {
        return false;
    };
    let no_scale = 1_u64 << 14;
    references.into_iter().all(|(ref_frame, _)| {
        usize::try_from(ref_frame)
            .ok()
            .and_then(|index| ref_frame_idx.get(index))
            .and_then(|&slot| usize::try_from(slot).ok())
            .is_some_and(|slot| {
                let scale = |reference: &[u32], current: u32| {
                    current != 0
                        && reference.get(slot).is_some_and(|&dimension| {
                            ((u64::from(dimension) << 14) + u64::from(current / 2))
                                / u64::from(current)
                                == no_scale
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
/// One TIP prediction unit's temporal-motion contribution, recorded by the
/// caller once the unit's (possibly optical-flow refined) MVs are known.
#[derive(Clone, Copy, Debug)]
pub(super) struct TipTemporalRecord {
    pub(super) mi_row: usize,
    pub(super) mi_col: usize,
    pub(super) n4w: usize,
    pub(super) n4h: usize,
    pub(super) ref_frame0: i8,
    pub(super) ref_frame1: Option<i8>,
    pub(super) mvs: [Mv; 2],
}

pub(super) fn apply_tip_temporal_records<T: ReconSample>(
    motion_field: &mut TemporalMotionField,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    frame_mi_rows: usize,
    frame_mi_cols: usize,
    current_order_hint: u32,
    records: &[TipTemporalRecord],
) {
    for record in records {
        super::temporal::record_temporal_motion_block(
            motion_field,
            reference,
            ref_frame_idx,
            record.mi_row,
            record.mi_col,
            record.n4w,
            record.n4h,
            frame_mi_rows,
            frame_mi_cols,
            current_order_hint,
            record.ref_frame0,
            record.ref_frame1,
            record.mvs[0],
            record.mvs[1],
            [None, None],
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct<T: ReconSample>(
    sink: &mut mc::WorkspaceSink<'_, T>,
    placed: &PlacedInterBlock,
    temporal: &TemporalMvContext,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<Vec<TipTemporalRecord>> {
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
    let use_refinemv = sequence.inter.as_ref().is_some_and(|tools| {
        tip_uses_refinemv(
            output,
            tools.enable_refinemv,
            tools.enable_tip_refinemv,
            weight,
        )
    });
    let search_refinemv = use_refinemv
        && tip_refinemv_references_allowed(
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
        && (output || weight == mc::CWP_EQUAL);
    let mut records = Vec::new();
    let frame_size = sink.info().coded_luma_size();
    let block_w = placed
        .luma_w
        .min(frame_size.width().saturating_sub(placed.luma_x));
    let block_h = placed
        .luma_h
        .min(frame_size.height().saturating_sub(placed.luma_y));

    let mut units = Vec::new();
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
            let unit = PlacedInterBlock {
                luma_x,
                luma_y,
                luma_w,
                luma_h,
                chroma_luma_x: chroma_x,
                chroma_luma_y: chroma_y,
                chroma_luma_w: chroma_end_x.saturating_sub(chroma_x),
                chroma_luma_h: chroma_end_y.saturating_sub(chroma_y),
                predict_chroma,
                sub8x8_chroma: false,
                interintra_chroma: false,
                block: InterBlock {
                    ref_frame0: references.past_ref,
                    ref_frame1: two_references.then_some(references.future_ref),
                    mv: mvs[0],
                    mv1: mvs[1],
                    interp: interpolation_filter,
                    warp_params: [None, None],
                    bawp: BawpSyntax::default(),
                    interintra: None,
                    compound_blend: blend,
                    optflow_distances: use_optflow
                        .then_some([references.past_offset, references.future_offset]),
                    residual: None,
                },
            };
            let rect = mc::McBlockRect {
                luma_x,
                luma_y,
                luma_w,
                luma_h,
                chroma_luma_x: unit.chroma_luma_x,
                chroma_luma_y: unit.chroma_luma_y,
                chroma_luma_w: unit.chroma_luma_w,
                chroma_luma_h: unit.chroma_luma_h,
            };
            let params = super::super::resolve_inter_block_params(
                ref_frame_idx,
                reference,
                &unit,
                rect,
                tile_offset,
            )?
            .with_refinemv(use_refinemv)
            .with_refinemv_search(search_refinemv)
            .with_optflow_sad_threshold(use_optflow.then_some(if output { 15 } else { 6 }));
            units.push(TipUnit {
                params,
                mvs,
                luma_x,
                luma_y,
                luma_w,
                luma_h,
            });
        }
    }
    let outputs = if two_references && splot_parallel::on_worker_pool() {
        let shared: &mc::WorkspaceSink<'_, T> = sink;
        let results: Vec<_> = units
            .par_iter()
            .map(|unit| {
                let compound = unit
                    .params
                    .into_compound()
                    .ok_or_else(|| tip_reference_pair_error(tile_offset))?;
                mc::predict_compound_average_block(
                    shared,
                    compound,
                    use_optflow.then_some(8),
                    tile_offset,
                )
                .map(Some)
            })
            .collect();
        results.into_iter().collect::<Result<Vec<_>>>()?
    } else {
        (0..units.len()).map(|_| None).collect()
    };
    for (unit, output) in units.into_iter().zip(outputs) {
        let stored_mvs = if let Some(output) = output {
            let refined_mvs = if use_optflow {
                output.stored_mvs_at_origin()?
            } else {
                None
            };
            let stored_mvs = tip_temporal_mvs(use_optflow, unit.mvs, refined_mvs);
            output.publish(sink)?;
            stored_mvs
        } else if use_optflow {
            mc::motion_compensate_inter_block_with_optflow_mvs_into(
                sink,
                unit.params,
                8,
                tile_offset,
            )?
            .unwrap_or(unit.mvs)
        } else {
            mc::motion_compensate_inter_block_into(sink, unit.params, tile_offset)?;
            unit.mvs
        };
        records.push(TipTemporalRecord {
            mi_row: unit.luma_y / 4,
            mi_col: unit.luma_x / 4,
            n4w: unit.luma_w.div_ceil(4),
            n4h: unit.luma_h.div_ceil(4),
            ref_frame0: references.past_ref,
            ref_frame1: two_references.then_some(references.future_ref),
            mvs: stored_mvs,
        });
    }
    if let Some(residual) = placed.block.residual.as_ref() {
        super::super::add_inter_residual_to_workspace(
            sink,
            residual,
            qindex,
            luma_use_tcq,
            residual_use_ddt,
            false,
            bit_depth,
            tile_offset,
        )?;
    }
    Ok(records)
}

pub(in crate::prediction::inter) fn reconstruct_output<T: ReconSample>(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    reference: &InterReferenceState<'_, T>,
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
    if inter.apply_deblocking_filter_tip == Some(true)
        && core
            .tile_info
            .as_ref()
            .is_none_or(|tile| tile.tile_cols != 1 || tile.tile_rows != 1)
    {
        return Err(inter_cap!(
            "tip_output_multi_tile_deblocking",
            offset,
            "inter.tip_output.multi_tile_deblocking",
            "7.10.6"
        ));
    }
    let ref_frame_idx = &inter.ref_frame_idx;
    let width = usize::try_from(frame_size.width)
        .map_err(|_| missing("unsupported capability: inter.tip_output.frame_dimensions"))?;
    let height = usize::try_from(frame_size.height)
        .map_err(|_| missing("unsupported capability: inter.tip_output.frame_dimensions"))?;
    let (mi_rows, mi_cols) = (height.div_ceil(4), width.div_ceil(4));
    let sb_h4 = super::superblock_h4(sequence, core)
        .ok_or_else(|| missing("missing required input: inter.tip_output.superblock_size"))?;
    let projection_step = tmvp_projection_step(core);
    let mut temporal = TemporalMvContext::from_references(
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
        &reference.ref_motion_fields,
    )
    .ok_or_else(|| missing("missing required input: inter.tip_output.temporal_context"))?;
    prepare_motion_field(&mut temporal, core, sb_h4);
    let global_mv = inter
        .tip_global_mv
        .ok_or_else(|| missing("missing required input: inter.tip_output.global_mv"))?;
    let mut workspace = crate::pipeline::reconstruct::new_general_intra_workspace::<T>(
        width,
        height,
        bit_depth,
        PixelFormat::from_av2_chroma_format_idc(sequence.general.chroma_format_idc.get())?,
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
    let records = reconstruct(
        &mut mc::WorkspaceSink::Frame(&mut workspace),
        &placed,
        &temporal,
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
    let coded = workspace.info().coded_luma_size();
    apply_tip_temporal_records(
        &mut motion_field,
        reference,
        ref_frame_idx,
        coded.height().div_ceil(4),
        coded.width().div_ceil(4),
        core.display_order_hint().unwrap_or(0),
        &records,
    );
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
            bit_depth,
        )
        .map_err(|_| missing("unsupported capability: inter.tip_output.deblocking"))?;
    }
    Ok((workspace.freeze()?, motion_field))
}

#[cfg(test)]
mod tests {
    use super::{
        output_prediction_unit_size, prediction_unit_size, tip_refinemv_offsets_allowed,
        tip_refinemv_references_allowed, tip_temporal_mvs, tip_uses_refinemv,
        tip_uses_two_references, tmvp_unit_size8,
    };
    use crate::prediction::inter::Mv;
    use splot_core::headers::frame::{FrameSize, FrameType};
    use splot_recon::InterpolationFilter;

    #[test]
    fn tip_reference_unit_size_follows_refinement_and_large_block_gates() {
        assert_eq!(prediction_unit_size(64, 32, false), 16);
        assert_eq!(prediction_unit_size(8, 32, false), 8);
        assert_eq!(prediction_unit_size(64, 32, true), 8);
        assert_eq!(prediction_unit_size(256, 256, true), 16);
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
