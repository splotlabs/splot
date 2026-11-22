// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Publication of decoded leaf facts into tile state.

use super::super::block_decoded_state::TileBlockDecodedState;
use super::tree_walk::GeneralIntraTreeWalkError;
use super::{
    BlockSize, DecodeBlockFrontier, GeneralIntraLeafMode, IntraFacts, PartitionTreeType,
    TileIntraFactsState, TileMiSizeState, TilePartitionCall, TilePartitionFrameFacts,
    TilePartitionTraversalError, plane_range_for_tree_type,
};

/// Leaf position in the superblock, measured in 4×4 cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodedLeafPublication {
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
            sub_block_origin: [sub[0] as u8, sub[1] as u8],
            block_size,
            tree_type,
        }
    }

    const fn sub_block_origin(self) -> [usize; 2] {
        [
            self.sub_block_origin[0] as usize,
            self.sub_block_origin[1] as usize,
        ]
    }

    /// The leaf's position and size in 4x4 units when it codes the luma plane —
    /// § 5.20.4.1 invokes the § 7.22 motion-field storage only for those leaves.
    pub(crate) fn luma_tree_block(
        self,
        [r, c]: [usize; 2],
    ) -> Option<(usize, usize, usize, usize)> {
        if matches!(self.tree_type, PartitionTreeType::ChromaPart) {
            return None;
        }
        let [sr, sc] = self.sub_block_origin();
        Some((
            r + sr,
            c + sc,
            self.block_size.num_4x4_wide().ok()?,
            self.block_size.num_4x4_high().ok()?,
        ))
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

#[expect(clippy::inline_always, reason = "measured: one call per decoded leaf")]
#[inline(always)]
pub(crate) fn publish_intra_leaf_state<E>(
    frame: &TilePartitionFrameFacts,
    call: &TilePartitionCall,
    frontier: &DecodeBlockFrontier,
    leaf_mode: GeneralIntraLeafMode,
    sub_size: BlockSize,
    intra_facts: &mut TileIntraFactsState,
    mi_size_state: &mut TileMiSizeState,
) -> Result<(), GeneralIntraTreeWalkError<E>> {
    let tree_type = frontier.tree_type();
    if tree_type != PartitionTreeType::ChromaPart {
        let facts = match leaf_mode.luma {
            Some(facts) => Some(facts),
            None if leaf_mode.is_intrabc() => Some(IntraFacts::intrabc()),
            None if frame.frame_is_intra => {
                return Err(GeneralIntraTreeWalkError::Traversal(
                    TilePartitionTraversalError::MissingIntraLumaModeState {
                        r: call.r(),
                        c: call.c(),
                    },
                ));
            }
            None => None,
        };
        if let Some(facts) = facts.filter(|_| tree_type == PartitionTreeType::LumaPart) {
            let extent = (
                sub_size
                    .num_4x4_wide()
                    .map_err(TilePartitionTraversalError::from)?,
                sub_size
                    .num_4x4_high()
                    .map_err(TilePartitionTraversalError::from)?,
            );
            intra_facts
                .grid_mut()
                .record_block((call.r(), call.c()), extent, facts);
        }
        mi_size_state
            .update_luma_block(call.r(), call.c(), sub_size, facts, leaf_mode.palette_y)
            .map_err(GeneralIntraTreeWalkError::MiSize)?;
    }
    if frontier.has_chroma || tree_type == PartitionTreeType::ChromaPart {
        let chroma_ref = frontier.chroma_ref_geometry();
        mi_size_state
            .update_chroma_block(
                chroma_ref.row(),
                chroma_ref.col(),
                chroma_ref.size(),
                leaf_mode.uv,
            )
            .map_err(GeneralIntraTreeWalkError::MiSize)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn leaf_publication_uses_the_owning_superblock_origin() {
        let size = BlockSize::new(6).unwrap();
        let publication = DecodedLeafPublication::at(21, 33, 15, size, PartitionTreeType::Shared);
        let mut state = TileBlockDecodedState::new(3, 1, 1, 16, 64, 64).unwrap();
        let origin = [16, 32];
        let (r, c, _, _) = publication.luma_tree_block(origin).unwrap();
        assert_eq!([r, c], [21, 33]);
        state.clear_superblock(origin[0], origin[1]);
        publication.publish_block_decoded(&mut state).unwrap();
        assert_eq!(state.count_top_right_avail(0, 0, 6, 1), 1);
        assert_eq!(state.count_top_right_avail(0, 0, 5, 1), 0);
    }
}
