// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Publication of decoded leaf facts into tile state.

use super::tree_walk::GeneralIntraTreeWalkError;
use super::{
    BlockSize, DecodeBlockFrontier, GeneralIntraLeafMode, IntraYMode, PartitionTreeType,
    TileBlockDecodedState, TileFscModeState, TileIntraJointModeState, TileIntraYModeState,
    TileLumaPaletteState, TileMiSizeState, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionTraversalError, TileUseDipState, TileUsesMrlsState, TileUvCflState,
    plane_range_for_tree_type, plane_subsampling,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_intra_leaf_state<E>(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    frontier: &DecodeBlockFrontier,
    leaf_mode: GeneralIntraLeafMode,
    sub_size: BlockSize,
    sb_mask: usize,
    joint_modes: &mut TileIntraJointModeState,
    fsc_modes: &mut TileFscModeState,
    use_dip: &mut TileUseDipState,
    uses_mrls: &mut TileUsesMrlsState,
    palette_y: &mut TileLumaPaletteState,
    uv_cfls: &mut TileUvCflState,
    y_modes: &mut TileIntraYModeState,
    block_decoded: &mut TileBlockDecodedState,
    mi_size_state: &mut TileMiSizeState,
) -> Result<(), GeneralIntraTreeWalkError<E>> {
    let tree_type = frontier.tree_type;
    let block_n4w = sub_size
        .num_4x4_wide()
        .map_err(TilePartitionTraversalError::from)?;
    let block_n4h = sub_size
        .num_4x4_high()
        .map_err(TilePartitionTraversalError::from)?;
    if let Some(uv_cfl) = leaf_mode.uv_cfl {
        let chroma_ref = frontier.chroma_ref_geometry();
        let chroma_n4w = chroma_ref
            .size()
            .num_4x4_wide()
            .map_err(TilePartitionTraversalError::from)?;
        let chroma_n4h = chroma_ref
            .size()
            .num_4x4_high()
            .map_err(TilePartitionTraversalError::from)?;
        uv_cfls.record_block(
            chroma_ref.row(),
            chroma_ref.col(),
            chroma_n4w,
            chroma_n4h,
            uv_cfl,
        );
    }
    if tree_type != PartitionTreeType::ChromaPart {
        if let Some(joint_mode) = leaf_mode.intra_joint_mode {
            let y_mode =
                leaf_mode
                    .y_mode
                    .ok_or(TilePartitionTraversalError::MissingIntraLumaModeState {
                        r: call.r,
                        c: call.c,
                    })?;
            let angle_delta_y = leaf_mode.angle_delta_y.ok_or(
                TilePartitionTraversalError::MissingIntraLumaModeState {
                    r: call.r,
                    c: call.c,
                },
            )?;
            let uses_mrls_value = leaf_mode.uses_mrls.ok_or(
                TilePartitionTraversalError::MissingIntraUsesMrlsState {
                    r: call.r,
                    c: call.c,
                },
            )?;
            let fsc_mode = leaf_mode.fsc_mode.ok_or(
                TilePartitionTraversalError::MissingIntraFscModeState {
                    r: call.r,
                    c: call.c,
                },
            )?;
            let use_dip_value =
                leaf_mode
                    .use_dip
                    .ok_or(TilePartitionTraversalError::MissingIntraUseDipState {
                        r: call.r,
                        c: call.c,
                    })?;
            joint_modes.record_block(call.r, call.c, block_n4w, block_n4h, joint_mode);
            fsc_modes.record_block(call.r, call.c, block_n4w, block_n4h, fsc_mode);
            use_dip.record_block(call.r, call.c, block_n4w, block_n4h, use_dip_value);
            uses_mrls.record_block(call.r, call.c, block_n4w, block_n4h, uses_mrls_value);
            palette_y.record_block(call.r, call.c, block_n4w, block_n4h, leaf_mode.palette_y);
            y_modes.record_block(call.r, call.c, block_n4w, block_n4h, y_mode, angle_delta_y);
        } else {
            if frame.frame_is_intra && !leaf_mode.is_intrabc() {
                return Err(GeneralIntraTreeWalkError::Traversal(
                    TilePartitionTraversalError::MissingIntraLumaModeState {
                        r: call.r,
                        c: call.c,
                    },
                ));
            }
            joint_modes.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
            fsc_modes.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
            use_dip.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
            uses_mrls.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
            palette_y.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
            if leaf_mode.is_intrabc() {
                y_modes.record_block(
                    call.r,
                    call.c,
                    block_n4w,
                    block_n4h,
                    IntraYMode::DC_PRED, // § 5.20.5.3 AV2 intraBC mode = DC_PRED (decodemv.c); an SDP chroma-part reads this collocated luma mode
                    0,
                );
            } else {
                y_modes.record_non_intra_block(call.r, call.c, block_n4w, block_n4h);
            }
        }
    }
    let sub_block_mi_row = call.r & sb_mask;
    let sub_block_mi_col = call.c & sb_mask;
    let (plane_start, plane_end) = plane_range_for_tree_type(tree_type, frame.num_planes);
    for plane in plane_start..plane_end {
        let (sub_x, sub_y) = plane_subsampling(frame, plane);
        block_decoded.set_block(
            plane,
            sub_block_mi_row,
            sub_block_mi_col,
            (block_n4w >> sub_x).max(1),
            (block_n4h >> sub_y).max(1),
        );
    }
    if tree_type != PartitionTreeType::ChromaPart {
        mi_size_state
            .update_luma_block(call.r, call.c, sub_size)
            .map_err(GeneralIntraTreeWalkError::MiSize)?;
    }
    if frontier.has_chroma || tree_type == PartitionTreeType::ChromaPart {
        let chroma_ref = call.chroma_ref_geometry();
        mi_size_state
            .update_chroma_block(chroma_ref.row, chroma_ref.col, chroma_ref.size)
            .map_err(GeneralIntraTreeWalkError::MiSize)?;
    }
    Ok(())
}
