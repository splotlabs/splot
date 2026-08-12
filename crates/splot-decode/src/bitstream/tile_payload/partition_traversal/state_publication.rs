// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Publication of decoded leaf facts into tile state.

use super::super::block_decoded_state::TileBlockDecodedState;
use super::tree_walk::GeneralIntraTreeWalkError;
use super::{
    BlockSize, DecodeBlockFrontier, GeneralIntraLeafMode, IntraYMode, PartitionTreeType,
    TileFscModeState, TileIntraJointModeState, TileIntraYModeState, TileLumaPaletteState,
    TileMiSizeState, TilePartitionCall, TilePartitionFrameFacts, TilePartitionTraversalError,
    TileUseDipState, TileUsesMrlsState, TileUvCflState, plane_range_for_tree_type,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodedLeafPublication {
    superblock_origin: [usize; 2],
    sub_block_origin: [usize; 2],
    block_size: BlockSize,
    tree_type: PartitionTreeType,
}

impl DecodedLeafPublication {
    pub(super) fn new(call: TilePartitionCall, sub_size: BlockSize, sb_mask: usize) -> Self {
        let sub_block_origin = [call.r & sb_mask, call.c & sb_mask];
        Self {
            superblock_origin: [
                call.r.saturating_sub(sub_block_origin[0]),
                call.c.saturating_sub(sub_block_origin[1]),
            ],
            sub_block_origin,
            block_size: sub_size,
            tree_type: call.tree_type,
        }
    }

    pub(crate) const fn superblock_origin(self) -> [usize; 2] {
        self.superblock_origin
    }

    pub(crate) const fn block_origin(self) -> [usize; 2] {
        [
            self.superblock_origin[0] + self.sub_block_origin[0],
            self.superblock_origin[1] + self.sub_block_origin[1],
        ]
    }

    /// The leaf's position and size in 4x4 units when it codes the luma plane —
    /// § 5.20.4.1 invokes the § 7.22 motion-field storage only for those leaves.
    pub(crate) fn luma_tree_block(self) -> Option<(usize, usize, usize, usize)> {
        if matches!(self.tree_type, PartitionTreeType::ChromaPart) {
            return None;
        }
        Some((
            self.superblock_origin[0] + self.sub_block_origin[0],
            self.superblock_origin[1] + self.sub_block_origin[1],
            self.block_size.num_4x4_wide().ok()?,
            self.block_size.num_4x4_high().ok()?,
        ))
    }

    pub(crate) fn prepare_block_decoded(
        self,
        block_decoded: &mut TileBlockDecodedState,
        current_superblock: &mut Option<[usize; 2]>,
    ) {
        if *current_superblock == Some(self.superblock_origin) {
            return;
        }
        block_decoded.clear_superblock(self.superblock_origin[0], self.superblock_origin[1]);
        *current_superblock = Some(self.superblock_origin);
    }

    pub(crate) fn publish_block_decoded(
        self,
        block_decoded: &mut TileBlockDecodedState,
    ) -> Result<(), TilePartitionTraversalError> {
        let block_size4 = [
            self.block_size.num_4x4_wide()?,
            self.block_size.num_4x4_high()?,
        ];
        let (plane_start, plane_end) =
            plane_range_for_tree_type(self.tree_type, block_decoded.num_planes());
        for plane in plane_start..plane_end {
            let (sub_x, sub_y) = block_decoded.subsampling(plane);
            block_decoded.set_block(
                plane,
                self.sub_block_origin[0],
                self.sub_block_origin[1],
                (block_size4[0] >> sub_x).max(1),
                (block_size4[1] >> sub_y).max(1),
            );
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_intra_leaf_state<E>(
    frame: TilePartitionFrameFacts,
    call: TilePartitionCall,
    frontier: &DecodeBlockFrontier,
    leaf_mode: GeneralIntraLeafMode,
    sub_size: BlockSize,
    joint_modes: &mut TileIntraJointModeState,
    fsc_modes: &mut TileFscModeState,
    use_dip: &mut TileUseDipState,
    uses_mrls: &mut TileUsesMrlsState,
    palette_y: &mut TileLumaPaletteState,
    uv_cfls: &mut TileUvCflState,
    y_modes: &mut TileIntraYModeState,
    mi_size_state: &mut TileMiSizeState,
) -> Result<(), GeneralIntraTreeWalkError<E>> {
    let tree_type = frontier.tree_type();
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
            let mrl =
                leaf_mode
                    .mrl
                    .ok_or(TilePartitionTraversalError::MissingIntraUsesMrlsState {
                        r: call.r,
                        c: call.c,
                    })?;
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
            uses_mrls.record_block(call.r, call.c, block_n4w, block_n4h, mrl);
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
            if leaf_mode.is_intrabc() {
                y_modes.record_block(
                    call.r,
                    call.c,
                    block_n4w,
                    block_n4h,
                    IntraYMode::Dc, // § 5.20.5.3 AV2 intraBC mode = DC_PRED (decodemv.c); an SDP chroma-part reads this collocated luma mode
                    0,
                );
            }
        }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn block_decoded_publication_clears_after_skipped_different_origin() {
        let mut state = TileBlockDecodedState::new(3, 1, 1, 16, 32, 32).unwrap();
        let mut current_superblock = None;
        let first = DecodedLeafPublication {
            superblock_origin: [0, 0],
            sub_block_origin: [0, 0],
            block_size: BlockSize::new(6).unwrap(),
            tree_type: PartitionTreeType::Shared,
        };

        first.prepare_block_decoded(&mut state, &mut current_superblock);
        first.publish_block_decoded(&mut state).unwrap();
        assert_eq!(state.count_top_right_avail(0, 1, 1, 1), 1);
        assert_eq!(state.count_top_right_avail(1, 1, 1, 1), 0);

        let skipped_precomputed = DecodedLeafPublication {
            superblock_origin: [0, 16],
            ..first
        };
        assert_eq!(skipped_precomputed.superblock_origin(), [0, 16]);
        let second = DecodedLeafPublication {
            superblock_origin: [16, 0],
            ..first
        };
        second.prepare_block_decoded(&mut state, &mut current_superblock);
        assert_eq!(state.count_top_right_avail(0, 1, 1, 1), 0);
        assert_eq!(current_superblock, Some([16, 0]));
    }
}
