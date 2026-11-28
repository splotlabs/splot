// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The identity of a partition node within one coded frame. Decoder block
//! evidence and the encoder's decision trace name a node with the same value.

use splot_recon::PlaneRect;

use super::super::partition_allowed::PartitionTreeType;
use super::super::partition_traversal::TilePartitionCall;

/// The partition tree that owns a node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum BlockEvidenceTree {
    /// The shared tree.
    Shared,
    /// The separate luma tree.
    Luma,
    /// The separate chroma tree.
    Chroma,
}

impl From<PartitionTreeType> for BlockEvidenceTree {
    fn from(tree: PartitionTreeType) -> Self {
        match tree {
            PartitionTreeType::Shared => Self::Shared,
            PartitionTreeType::LumaPart => Self::Luma,
            PartitionTreeType::ChromaPart => Self::Chroma,
        }
    }
}

/// Identifies a node within one captured coded frame.
/// Keep its report's frame and input source identity to compare it across captures.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BlockEvidenceId {
    /// Zero-based tile index in raster order.
    pub tile: usize,
    /// The partition tree.
    pub tree: BlockEvidenceTree,
    /// The partition rectangle in coded luma samples, including edge padding.
    pub coded: PlaneRect,
}

impl BlockEvidenceId {
    /// Identifies a node by tile, tree and coded luma rectangle.
    #[must_use]
    pub const fn new(tile: usize, tree: BlockEvidenceTree, coded: PlaneRect) -> Self {
        Self { tile, tree, coded }
    }

    /// The identity of the `decode_partition` call `call` in tile `tile`.
    pub(crate) fn of_call(tile: usize, call: TilePartitionCall) -> Option<Self> {
        let [h, w] = call.b_size.num_4x4();
        let coded = PlaneRect::new(call.c() * 4, call.r() * 4, w * 4, h * 4).ok()?;
        Some(Self::new(tile, call.tree_type().into(), coded))
    }
}
