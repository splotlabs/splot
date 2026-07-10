// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[doc = "AV2 § 7.13.3.1 Tip_Weighting_Factor."]
const TIP_WEIGHTING_FACTORS: [i16; 8] = [8, 12, 16, 18, 20, 4, 6, -4];

#[doc = "AV2 § 7.13.3.1 tipSize selection for TIP prediction."]
const fn prediction_unit_size(width: usize, height: usize, enable_tip_refinemv: bool) -> usize {
    if (!enable_tip_refinemv && width >= 16 && height >= 16) || (width >= 256 && height >= 256) {
        16
    } else {
        8
    }
}

pub(super) fn prepare_motion_field(
    temporal: &mut TemporalMvContext,
    core: &FrameHeaderCore,
    sb_h4: usize,
) {
    let Some(inter) = core
        .inter
        .as_ref()
        .filter(|inter| inter.tip_frame_mode != Some(TipFrameMode::Disabled))
    else {
        return;
    };
    let projection_step = usize::from(inter.tmvp_sample_step_minus_1.unwrap_or(false)) + 1;
    _ = temporal.prepare_tip(
        projection_step,
        (sb_h4 / 2).min(16),
        inter.allow_tip_hole_fill.unwrap_or(false),
    );
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
    let enable_tip_refinemv = sequence
        .inter
        .as_ref()
        .is_some_and(|tools| tools.enable_tip_refinemv);
    let unit_size = prediction_unit_size(placed.luma_w, placed.luma_h, enable_tip_refinemv);
    let use_optflow = unit_size == 8
        && weight == mc::CWP_EQUAL
        && inter.opfl_refine_type.unwrap_or(0) != 0
        && enable_tip_refinemv;
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
                    ref_frame1: Some(references.future_ref),
                    mv: mvs[0],
                    mv1: mvs[1],
                    interp: ReconInterpolationFilter::EightTapSharp,
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
            mc::motion_compensate_inter_block_into(workspace, params, tile_offset)?;
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

#[cfg(test)]
mod tests {
    use super::prediction_unit_size;

    #[test]
    fn tip_reference_unit_size_follows_refinement_and_large_block_gates() {
        assert_eq!(prediction_unit_size(64, 32, false), 16);
        assert_eq!(prediction_unit_size(8, 32, false), 8);
        assert_eq!(prediction_unit_size(64, 32, true), 8);
        assert_eq!(prediction_unit_size(256, 256, true), 16);
    }
}
