// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Publication of decoded leaf facts into tile state.

use super::super::block_decoded_state::TileBlockDecodedState;
use super::tree_walk::GeneralIntraTreeWalkError;
use super::{
    BlockSize, DecodeBlockFrontier, GeneralIntraLeafMode, IntraFacts, PartitionTreeType,
    TileIntraFactsState, TileMiSizeState, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionTraversalError, TileUvCflState, plane_range_for_tree_type,
};

/// Mode-info positions are below `MiRows`/`MiCols`, which a `u32` frame size
/// bounds, and a position inside a superblock is below 64.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodedLeafPublication {
    superblock_origin: [u32; 2],
    sub_block_origin: [u8; 2],
    block_size: BlockSize,
    tree_type: PartitionTreeType,
}

impl DecodedLeafPublication {
    pub(crate) fn new(call: TilePartitionCall, sub_size: BlockSize, sb_mask: usize) -> Self {
        Self::at(call.r(), call.c(), sb_mask, sub_size, call.tree_type)
    }

    /// The publication the walk makes for `frontier`'s leaf.
    pub(crate) const fn from_frontier(frontier: &DecodeBlockFrontier, sb_mask: usize) -> Self {
        Self::at(
            frontier.r,
            frontier.c,
            sb_mask,
            frontier.b_size,
            frontier.tree_type(),
        )
    }

    const fn at(
        r: usize,
        c: usize,
        sb_mask: usize,
        block_size: BlockSize,
        tree_type: PartitionTreeType,
    ) -> Self {
        let sub = [r & sb_mask, c & sb_mask];
        Self {
            superblock_origin: [(r - sub[0]) as u32, (c - sub[1]) as u32],
            sub_block_origin: [sub[0] as u8, sub[1] as u8],
            block_size,
            tree_type,
        }
    }

    pub(crate) const fn superblock_origin(self) -> [usize; 2] {
        [
            self.superblock_origin[0] as usize,
            self.superblock_origin[1] as usize,
        ]
    }

    const fn sub_block_origin(self) -> [usize; 2] {
        [
            self.sub_block_origin[0] as usize,
            self.sub_block_origin[1] as usize,
        ]
    }

    pub(crate) const fn block_origin(self) -> [usize; 2] {
        let ([r, c], [sr, sc]) = (self.superblock_origin(), self.sub_block_origin());
        [r + sr, c + sc]
    }

    /// The leaf's position and size in 4x4 units when it codes the luma plane —
    /// § 5.20.4.1 invokes the § 7.22 motion-field storage only for those leaves.
    pub(crate) fn luma_tree_block(self) -> Option<(usize, usize, usize, usize)> {
        if matches!(self.tree_type, PartitionTreeType::ChromaPart) {
            return None;
        }
        let [mi_row, mi_col] = self.block_origin();
        Some((
            mi_row,
            mi_col,
            self.block_size.num_4x4_wide().ok()?,
            self.block_size.num_4x4_high().ok()?,
        ))
    }

    pub(crate) fn prepare_block_decoded(
        self,
        block_decoded: &mut TileBlockDecodedState,
        current_superblock: &mut Option<[usize; 2]>,
    ) {
        let origin = self.superblock_origin();
        if *current_superblock == Some(origin) {
            return;
        }
        block_decoded.clear_superblock(origin[0], origin[1]);
        *current_superblock = Some(origin);
    }

    pub(crate) fn publish_block_decoded(
        self,
        block_decoded: &mut TileBlockDecodedState,
    ) -> Result<(), TilePartitionTraversalError> {
        let (plane_start, plane_end) =
            plane_range_for_tree_type(self.tree_type, block_decoded.num_planes());
        block_decoded.set_leaf(
            plane_start..plane_end,
            self.sub_block_origin()[0],
            self.sub_block_origin()[1],
            self.block_size.num_4x4_wide()?,
            self.block_size.num_4x4_high()?,
        );
        Ok(())
    }
}

#[allow(
    clippy::too_many_arguments,
    clippy::inline_always,
    reason = "measured: one call per decoded leaf"
)]
#[inline(always)]
pub(crate) fn publish_intra_leaf_state<E>(
    frame: &TilePartitionFrameFacts,
    call: &TilePartitionCall,
    frontier: &DecodeBlockFrontier,
    leaf_mode: GeneralIntraLeafMode,
    sub_size: BlockSize,
    intra_facts: &mut TileIntraFactsState,
    uv_cfls: &mut TileUvCflState,
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
            let Some(y_mode) = leaf_mode.y_mode else {
                return Err(TilePartitionTraversalError::MissingIntraLumaModeState {
                    r: call.r(),
                    c: call.c(),
                }
                .into());
            };
            let Some(angle_delta_y) = leaf_mode.angle_delta_y else {
                return Err(TilePartitionTraversalError::MissingIntraLumaModeState {
                    r: call.r(),
                    c: call.c(),
                }
                .into());
            };
            let Some(mrl) = leaf_mode.mrl else {
                return Err(TilePartitionTraversalError::MissingIntraUsesMrlsState {
                    r: call.r(),
                    c: call.c(),
                }
                .into());
            };
            let Some(fsc_mode) = leaf_mode.fsc_mode else {
                return Err(TilePartitionTraversalError::MissingIntraFscModeState {
                    r: call.r(),
                    c: call.c(),
                }
                .into());
            };
            let Some(use_dip_value) = leaf_mode.use_dip else {
                return Err(TilePartitionTraversalError::MissingIntraUseDipState {
                    r: call.r(),
                    c: call.c(),
                }
                .into());
            };
            intra_facts.record_leaf(
                (call.r(), call.c()),
                (block_n4w, block_n4h),
                IntraFacts::luma(
                    joint_mode,
                    y_mode,
                    angle_delta_y,
                    fsc_mode,
                    use_dip_value,
                    mrl,
                ),
                leaf_mode.palette_y,
            );
        } else {
            if frame.frame_is_intra && !leaf_mode.is_intrabc() {
                return Err(GeneralIntraTreeWalkError::Traversal(
                    TilePartitionTraversalError::MissingIntraLumaModeState {
                        r: call.r(),
                        c: call.c(),
                    },
                ));
            }
            if leaf_mode.is_intrabc() {
                intra_facts.record_leaf(
                    (call.r(), call.c()),
                    (block_n4w, block_n4h),
                    IntraFacts::intrabc(),
                    None,
                );
            }
        }
    }
    if tree_type != PartitionTreeType::ChromaPart {
        mi_size_state
            .update_luma_block(call.r(), call.c(), sub_size)
            .map_err(GeneralIntraTreeWalkError::MiSize)?;
    }
    if frontier.has_chroma || tree_type == PartitionTreeType::ChromaPart {
        let chroma_ref = call.chroma_ref_geometry();
        mi_size_state
            .update_chroma_block(chroma_ref.row(), chroma_ref.col(), chroma_ref.size())
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
