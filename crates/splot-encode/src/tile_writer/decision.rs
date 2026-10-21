// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coding decisions of one superblock, the input of the key-frame tile writer.

pub(crate) use splot_decode::encoder_syntax::{
    ChromaMode, IntraMode, IntraModes, Partition, TxType,
};
use splot_decode::encoder_syntax::{IntraTileSyntax, LeafBlock, TransformBlock};

use super::Result;

/// Every `YMode` and non-CFL `UVMode` value, in value order.
pub(crate) const INTRA_MODES: [IntraMode; 13] = [
    IntraMode::Dc,
    IntraMode::Vertical,
    IntraMode::Horizontal,
    IntraMode::D45,
    IntraMode::D135,
    IntraMode::D113,
    IntraMode::D157,
    IntraMode::D203,
    IntraMode::D67,
    IntraMode::Smooth,
    IntraMode::SmoothVertical,
    IntraMode::SmoothHorizontal,
    IntraMode::Paeth,
];

/// The partition tree of one superblock (§ 5.20.3.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SuperblockDecision {
    pub(crate) tree: PartitionTree,
}

/// One `decode_partition` call: a coded leaf or a split into child calls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PartitionTree {
    /// `PARTITION_NONE` with its planned modes.
    Leaf(IntraModes),
    /// Any other partition, with one entry per § 5.20.3.1 child call in syntax
    /// order. Entries of children that start outside the frame are not coded.
    Split {
        kind: Partition,
        children: Vec<PartitionTree>,
    },
}

/// The coded form of one transform block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformDecision {
    /// Must equal the block's `tx_type`: the writer codes no transform type.
    pub(crate) tx_type: TxType,
    /// Quantized levels in the row-major adjusted (at most 32x32) layout.
    pub(crate) levels: Vec<i32>,
}

/// Chooses each leaf's modes and codes each transform block once the writer
/// knows its prediction, because intra prediction reads the reconstruction of
/// the blocks before it.
///
/// The writer codes a leaf's luma blocks, then its chroma blocks, then writes
/// the leaf's symbols in syntax order. The mode hooks may plan and predict
/// candidates through `syntax`; the writer replans the leaf afterwards.
pub(crate) trait ResidualCoder {
    /// The luma modes of `leaf`; the default keeps `planned`.
    fn luma_modes(
        &mut self,
        _syntax: &mut IntraTileSyntax,
        _leaf: &LeafBlock,
        planned: IntraModes,
    ) -> Result<IntraModes> {
        Ok(planned)
    }

    /// The chroma mode of `leaf`, chosen after its luma is reconstructed; the
    /// default keeps the one in `modes`.
    fn chroma_mode(
        &mut self,
        _syntax: &mut IntraTileSyntax,
        _leaf: &LeafBlock,
        modes: IntraModes,
    ) -> Result<ChromaMode> {
        Ok(modes.uv_mode)
    }

    /// Codes `tx`, whose row-major prediction is `prediction`.
    fn code(&mut self, tx: &TransformBlock, prediction: &[u16]) -> Result<TransformDecision>;
}
