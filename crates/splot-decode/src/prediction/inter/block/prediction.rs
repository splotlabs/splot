// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::pipeline::reconstruct::IntraEdgeAvailability;

/// Positions in luma samples, which a `u32` frame size bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PlacedInterGeometry {
    pub(super) luma: [u32; 4],
    pub(super) chroma_luma: [u32; 4],
    pub(super) predict_chroma: bool,
    pub(super) sub8x8_chroma: bool,
    pub(super) interintra_chroma: bool,
    pub(super) interintra_edges: [IntraEdgeAvailability; 2],
}

impl PlacedInterGeometry {
    /// `block` placed at this geometry.
    pub(super) fn place(self, block: InterBlock) -> PlacedInterBlock {
        PlacedInterBlock {
            luma: self.luma,
            chroma_luma: self.chroma_luma,
            predict_chroma: self.predict_chroma,
            sub8x8_chroma: self.sub8x8_chroma,
            interintra_chroma: self.interintra_chroma,
            interintra_edges: self.interintra_edges,
            block,
        }
    }
}

pub(super) const fn leaf_predicts_chroma(chroma_planes: bool, luma_part: bool) -> bool {
    chroma_planes && !luma_part
}

/// § 7.11.3.1 disables compound chroma prediction in two cases: the § 5.20.4
/// `sub8x8Inter` mismatch between the block and its chroma reference, and any
/// thin 4xN / Nx4 block. TIP is excluded by its own caller.
pub(super) fn sub8x8_chroma_disables_compound(
    luma_size: BlockSize,
    chroma_size: BlockSize,
) -> bool {
    luma_size.index() != chroma_size.index() || is_thin_4xn_nx4_block(luma_size)
}

/// § 7.11.3.1 `is_thin_4xn_nx4_block`: a block with a four-sample side whose
/// long side reaches 16.
fn is_thin_4xn_nx4_block(size: BlockSize) -> bool {
    let (Ok(n4w), Ok(n4h)) = (size.num_4x4_wide(), size.num_4x4_high()) else {
        return false;
    };
    let (width, height) = (n4w * 4, n4h * 4);
    width.min(height) == 4 && width.max(height) >= 16
}

/// The placed geometry of a leaf of `work_unit`'s tile.
pub(super) fn inter_leaf_geometry(
    frontier: &DecodeBlockFrontier,
    work_unit: &DecodeTileWorkUnit,
    size4: (usize, usize),
    chroma_planes: bool,
) -> Result<PlacedInterGeometry> {
    let tile_start = (
        work_unit.mi_row_range().start as usize,
        work_unit.mi_col_range().start as usize,
    );
    placed_inter_geometry_at(frontier, tile_start, size4, chroma_planes)
}

/// A leaf's placed geometry in a tile that starts at 4x4 position `(row, col)`.
#[inline]
pub(super) fn placed_inter_geometry_at(
    frontier: &DecodeBlockFrontier,
    (tile_row_start, tile_col_start): (usize, usize),
    (n4w, n4h): (usize, usize),
    chroma_planes: bool,
) -> Result<PlacedInterGeometry> {
    let luma_x = frontier.c * 4;
    let luma_y = frontier.r * 4;
    let luma_w = n4w * 4;
    let luma_h = n4h * 4;
    let chroma_ref = frontier.chroma_ref_geometry();
    let (chroma_luma_x, chroma_luma_y, chroma_luma_w, chroma_luma_h) = if frontier.has_chroma {
        let chroma_n4w = chroma_ref
            .size()
            .num_4x4_wide()
            .map_err(|_| crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
        let chroma_n4h = chroma_ref
            .size()
            .num_4x4_high()
            .map_err(|_| crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
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
    let interintra_edges = [
        IntraEdgeAvailability::new(frontier.r > tile_row_start, frontier.c > tile_col_start),
        IntraEdgeAvailability::new(
            frontier.has_chroma && chroma_ref.row() > tile_row_start,
            frontier.has_chroma && chroma_ref.col() > tile_col_start,
        ),
    ];
    Ok(PlacedInterGeometry {
        luma: [luma_x, luma_y, luma_w, luma_h].map(|v| v as u32),
        chroma_luma: [chroma_luma_x, chroma_luma_y, chroma_luma_w, chroma_luma_h].map(|v| v as u32),
        predict_chroma,
        sub8x8_chroma: predict_chroma
            && sub8x8_chroma_disables_compound(frontier.b_size, chroma_ref.size()),
        interintra_chroma: frontier.has_chroma && !mixed_offset_chroma,
        interintra_edges,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_placed_inter_block<T: ReconSample>(
    interintra_scratch: &mut super::interintra::InterIntraScratch<T>,
    residual_scratch: &mut super::super::InterResidualReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    residual_blocks: &[InterResidualBlock],
    residual_coeffs: &[i32],
    motion: Option<mc::CompoundMotionGrid>,
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
        )?;
    }
    let held = super::super::hold_inter_block_references(ref_frame_idx, reference, placed)?;
    let motion = held.predict_into(
        &mut mc::WorkspaceSink::Frame(workspace),
        placed,
        rect,
        use_refinemv,
        refinemv_switchable,
        motion,
        tile_offset,
    )?;
    if placed.block.bawp.enabled {
        super::super::bawp::apply_bawp(
            workspace,
            held.reference0_samples()?,
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
                        placed.luma_w(),
                        placed.luma_h(),
                        usize::from(wedge_index),
                        prediction.sub_x,
                        prediction.sub_y,
                        samples,
                    ),
            };
            blend?;
        }
    }
    if let Some(residual) = placed.block.residual.as_ref() {
        super::super::add_inter_residual_to_workspace(
            residual_scratch,
            &mut mc::WorkspaceSink::Frame(workspace),
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
    Ok(motion)
}

/// Reconstructs one deferable inter block (no interintra, no BAWP, no
/// current-frame reads) into `sink`: motion compensation, then residual add.
#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_pure_inter_block<T: ReconSample>(
    sink: &mut mc::WorkspaceSink<'_, '_, T>,
    residual_scratch: &mut super::super::InterResidualReconScratch<T>,
    placed: &PlacedInterBlock,
    residual_blocks: &[InterResidualBlock],
    residual_coeffs: &[i32],
    motion: Option<mc::CompoundMotionGrid>,
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
    let held = super::super::hold_inter_block_references(ref_frame_idx, reference, placed)?;
    let motion = held.predict_into(
        sink,
        placed,
        placed.motion_compensation_rect(),
        use_refinemv,
        refinemv_switchable,
        motion,
        tile_offset,
    )?;
    if let Some(residual) = placed.block.residual.as_ref() {
        super::super::add_inter_residual_to_workspace(
            residual_scratch,
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
    Ok(motion)
}
