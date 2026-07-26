// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PlacedInterGeometry {
    pub(super) luma_x: usize,
    pub(super) luma_y: usize,
    pub(super) luma_w: usize,
    pub(super) luma_h: usize,
    pub(super) chroma_luma_x: usize,
    pub(super) chroma_luma_y: usize,
    pub(super) chroma_luma_w: usize,
    pub(super) chroma_luma_h: usize,
    pub(super) predict_chroma: bool,
    pub(super) sub8x8_chroma: bool,
    pub(super) interintra_chroma: bool,
}

pub(super) const fn leaf_predicts_chroma(chroma_planes: bool, luma_part: bool) -> bool {
    chroma_planes && !luma_part
}

pub(super) const fn sub8x8_chroma_disables_compound(
    luma_size: BlockSize,
    chroma_size: BlockSize,
) -> bool {
    luma_size.index() != chroma_size.index()
}

pub(super) fn placed_inter_geometry(
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    chroma_planes: bool,
    tile_offset: ByteOffset,
) -> Result<PlacedInterGeometry> {
    let luma_x = frontier.c * 4;
    let luma_y = frontier.r * 4;
    let luma_w = n4w * 4;
    let luma_h = n4h * 4;
    let (chroma_luma_x, chroma_luma_y, chroma_luma_w, chroma_luma_h) = if frontier.has_chroma {
        let chroma_ref = frontier.chroma_ref_geometry();
        let chroma_n4w = chroma_ref.size().num_4x4_wide().map_err(|_| {
            inter_diag!(
                "inter_chroma_ref_width",
                tile_offset,
                "invalid inter chroma reference width",
                "5.20.4.1"
            )
        })?;
        let chroma_n4h = chroma_ref.size().num_4x4_high().map_err(|_| {
            inter_diag!(
                "inter_chroma_ref_height",
                tile_offset,
                "invalid inter chroma reference height",
                "5.20.4.1"
            )
        })?;
        (
            chroma_ref.col() * 4,
            chroma_ref.row() * 4,
            chroma_n4w * 4,
            chroma_n4h * 4,
        )
    } else {
        (luma_x, luma_y, luma_w, luma_h)
    };
    let mixed_offset_chroma = !frontier.is_luma_part()
        && !frontier.is_chroma_part()
        && frontier.is_mixed_region()
        && frontier.chroma_offset;
    let predict_chroma = leaf_predicts_chroma(chroma_planes, frontier.is_luma_part());
    Ok(PlacedInterGeometry {
        luma_x,
        luma_y,
        luma_w,
        luma_h,
        chroma_luma_x,
        chroma_luma_y,
        chroma_luma_w,
        chroma_luma_h,
        predict_chroma,
        sub8x8_chroma: predict_chroma
            && sub8x8_chroma_disables_compound(
                frontier.b_size,
                frontier.chroma_ref_geometry().size(),
            ),
        interintra_chroma: frontier.has_chroma && !mixed_offset_chroma,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_placed_inter_block<T: ReconSample>(
    interintra_scratch: &mut super::interintra::InterIntraScratch<T>,
    residual_scratch: &mut super::super::InterResidualReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    residual_blocks: &[InterResidualBlock],
    use_refinemv: bool,
    refinemv_switchable: bool,
    block_decoded: &TileBlockDecodedState,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    enable_ibp: bool,
    tile_offset: ByteOffset,
) -> Result<Option<mc::CompoundMotionGrid>> {
    let rect = placed.motion_compensation_rect();
    if let Some(prediction) = placed.block.interintra {
        super::predict_interintra_planes(
            interintra_scratch,
            workspace,
            placed,
            block_decoded,
            prediction.mode(),
            enable_ibp,
            bit_depth,
            tile_offset,
        )?;
    }
    let block_params = super::super::resolve_inter_block_params(
        ref_frame_idx,
        reference,
        placed,
        rect,
        tile_offset,
    )?
    .with_refinemv(use_refinemv)
    .with_switchable_refinemv(refinemv_switchable);
    let motion_grid = mc::motion_compensate_inter_block_with_motion_grid_into(
        &mut mc::WorkspaceSink::Frame(workspace),
        block_params,
        None,
        tile_offset,
    )?;
    if placed.block.bawp.enabled {
        let slot = usize::try_from(placed.block.ref_frame0)
            .ok()
            .and_then(|list_ref| ref_frame_idx.get(list_ref).copied())
            .ok_or_else(|| {
                inter_missing!(
                    "inter_missing_bawp_reference_slot",
                    tile_offset,
                    "inter.bawp.reference_frame",
                    super::super::SPEC_REFERENCE
                )
            })?;
        let ref_frame = reference.frame_for_slot(slot).ok_or_else(|| {
            inter_missing!(
                "inter_missing_bawp_reference_frame",
                tile_offset,
                "inter.bawp.reference_frame",
                super::super::SPEC_REFERENCE
            )
        })?;
        super::super::bawp::apply_bawp(
            workspace,
            ref_frame,
            placed,
            placed.block.bawp,
            placed.block.mv,
            tile_offset,
        )?;
    }
    if let Some(interintra) = placed.block.interintra {
        for (prediction, samples) in interintra_scratch.planes() {
            let blend = match interintra {
                InterIntraPrediction::SmoothMask { mode } => workspace
                    .blend_smooth_interintra_rect(
                        prediction.plane,
                        prediction.x,
                        prediction.y,
                        prediction.size,
                        mode,
                        samples,
                    ),
                InterIntraPrediction::WedgeMask { wedge_index, .. } => workspace
                    .blend_wedge_interintra_rect(
                        prediction.plane,
                        prediction.x,
                        prediction.y,
                        prediction.size,
                        placed.luma_w,
                        placed.luma_h,
                        usize::from(wedge_index),
                        prediction.sub_x,
                        prediction.sub_y,
                        samples,
                    ),
            };
            blend.map_err(|_| {
                inter_diag!(
                    "inter_interintra_blend",
                    tile_offset,
                    "interintra blend failed",
                    "7.13.3.30"
                )
            })?;
        }
    }
    if let Some(residual) = placed.block.residual.as_ref() {
        super::super::add_inter_residual_to_workspace(
            residual_scratch,
            &mut mc::WorkspaceSink::Frame(workspace),
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
    Ok(motion_grid)
}

/// Reconstructs one deferable inter block (no interintra, no BAWP, no
/// current-frame reads) into `sink`: motion compensation, then residual add.
#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_pure_inter_block<T: ReconSample>(
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    residual_scratch: &mut super::super::InterResidualReconScratch<T>,
    placed: &PlacedInterBlock,
    residual_blocks: &[InterResidualBlock],
    use_refinemv: bool,
    refinemv_switchable: bool,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    qindex: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<Option<mc::CompoundMotionGrid>> {
    let block_params = super::super::resolve_inter_block_params(
        ref_frame_idx,
        reference,
        placed,
        placed.motion_compensation_rect(),
        tile_offset,
    )?
    .with_refinemv(use_refinemv)
    .with_switchable_refinemv(refinemv_switchable);
    let motion_grid = mc::motion_compensate_inter_block_with_motion_grid_into(
        sink,
        block_params,
        None,
        tile_offset,
    )?;
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
    Ok(motion_grid)
}
