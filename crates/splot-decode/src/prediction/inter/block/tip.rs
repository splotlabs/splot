// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_recon::{DecodedFrame, PixelFormat};

#[doc = "AV2 § 7.13.3.1 Tip_Weighting_Factor."]
const TIP_WEIGHTING_FACTORS: [i16; 8] = [8, 12, 16, 18, 20, 4, 6, -4];
const TIP_SINGLE_WEIGHT: i16 = 16;

const fn tip_uses_two_references(weight: i16) -> bool {
    weight != TIP_SINGLE_WEIGHT
}

#[doc = "AV2 § 7.13.3.1 tipSize selection for TIP prediction."]
const fn prediction_unit_size(width: usize, height: usize, enable_tip_refinemv: bool) -> usize {
    if (!enable_tip_refinemv && width >= 16 && height >= 16) || (width >= 256 && height >= 256) {
        16
    } else {
        8
    }
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
    let tmvp_unit_size8 = if projection_step == 1 {
        8
    } else {
        (sb_h4 / 2).min(16)
    };
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
    workspace: &mut CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    temporal: &TemporalMvContext,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<'_, T>,
    mut output_motion_field: Option<&mut TemporalMotionField>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<()> {
    let references = temporal.tip_references().ok_or_else(|| {
        inter_missing!(
            "inter_tip_reference_pair",
            tile_offset,
            "inter.tip.closest_past_and_future",
            SPEC_MODE_INFO
        )
    })?;
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
    let frame_size = workspace.info().coded_luma_size();
    let block_w = placed
        .luma_w
        .min(frame_size.width().saturating_sub(placed.luma_x));
    let block_h = placed
        .luma_h
        .min(frame_size.height().saturating_sub(placed.luma_y));

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
            let has_chroma =
                placed.has_chroma && chroma_end_x > chroma_x && chroma_end_y > chroma_y;
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
                has_chroma,
                interintra_chroma: false,
                block: InterBlock {
                    ref_frame0: references.past_ref,
                    ref_frame1: two_references.then_some(references.future_ref),
                    mv: mvs[0],
                    mv1: mvs[1],
                    interp: interpolation_filter,
                    warp_params: None,
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
            )?;
            let stored_mvs = if use_optflow {
                mc::motion_compensate_inter_block_with_optflow_mvs_into(
                    workspace,
                    params,
                    8,
                    tile_offset,
                )?
                .unwrap_or(mvs)
            } else {
                mc::motion_compensate_inter_block_into(workspace, params, tile_offset)?;
                mvs
            };
            if let Some(motion_field) = output_motion_field.as_deref_mut() {
                super::temporal::record_temporal_motion_block(
                    motion_field,
                    reference,
                    ref_frame_idx,
                    luma_y / 4,
                    luma_x / 4,
                    luma_w.div_ceil(4),
                    luma_h.div_ceil(4),
                    frame_size.height().div_ceil(4),
                    frame_size.width().div_ceil(4),
                    core.order_hint_lsb.unwrap_or(0),
                    references.past_ref,
                    two_references.then_some(references.future_ref),
                    stored_mvs[0],
                    stored_mvs[1],
                    None,
                );
            }
        }
    }
    if let Some(residual) = placed.block.residual.as_ref() {
        super::super::add_inter_residual_to_workspace(
            workspace,
            residual,
            qindex,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            tile_offset,
        )?;
    }
    Ok(())
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
    let mut temporal = TemporalMvContext::from_references(
        (mi_rows, mi_cols),
        core.order_hint_lsb.unwrap_or(0),
        TemporalProjectionConfig {
            frame_size: (width, height),
            step: tmvp_projection_step(core),
            enable_tip: sequence
                .inter
                .as_ref()
                .is_some_and(|tools| tools.enable_tip),
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
    let sb_h4 = super::superblock_h4(sequence, core)
        .ok_or_else(|| missing("missing required input: inter.tip_output.superblock_size"))?;
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
        has_chroma: sequence.general.chroma_format_idc != ChromaFormatIdc::Monochrome,
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
            warp_params: None,
            bawp: BawpSyntax::default(),
            interintra: None,
            compound_blend: mc::CompoundBlend::default(),
            optflow_distances: None,
            residual: None,
        },
    };
    reconstruct(
        &mut workspace,
        &placed,
        &temporal,
        sequence,
        core,
        ref_frame_idx,
        reference,
        Some(&mut motion_field),
        0,
        false,
        false,
        bit_depth,
        offset,
    )?;
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
    use super::{output_prediction_unit_size, prediction_unit_size, tip_uses_two_references};
    use splot_recon::InterpolationFilter;

    #[test]
    fn tip_reference_unit_size_follows_refinement_and_large_block_gates() {
        assert_eq!(prediction_unit_size(64, 32, false), 16);
        assert_eq!(prediction_unit_size(8, 32, false), 8);
        assert_eq!(prediction_unit_size(64, 32, true), 8);
        assert_eq!(prediction_unit_size(256, 256, true), 16);
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
}
