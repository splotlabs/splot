// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coding decisions of one superblock, the input of the key-frame tile writer.

pub(crate) use splot_decode::encoder_syntax::{
    ChromaMode, IntraMode, IntraModes, Partition, TxPartition, TxType,
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
    /// The type the levels use: the block's `tx_type` from
    /// [`ResidualCoder::code`], or one of the offered types from
    /// [`ResidualCoder::code_luma`].
    pub(crate) tx_type: TxType,
    /// Quantized levels in the row-major adjusted (at most 32x32) layout.
    pub(crate) levels: Vec<i32>,
}

/// Chooses each leaf's modes and codes each transform block once the writer
/// knows its prediction, because intra prediction reads the reconstruction of
/// the blocks before it.
///
/// The writer codes a leaf's luma blocks, then its chroma blocks, then writes
/// the leaf's symbols in syntax order. The mode and partition hooks may plan,
/// predict, and reconstruct candidates through `syntax` if they restore it; the
/// writer replans the leaf afterwards.
///
/// The decoder infers `DCT_DCT` for a luma block with one coded coefficient
/// (§ 5.20.8.2). When a [`Self::code_luma`] decision has `eob == 1` with
/// another type, the writer codes the block again through [`Self::code`] with
/// `DCT_DCT`; all-zero levels simply take `DCT_DCT`.
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

    /// The luma transform partition of `leaf` under `modes`, one of
    /// `syntax.luma_tx_partitions(leaf)`; the default splits nothing.
    fn tx_partition(
        &mut self,
        _syntax: &mut IntraTileSyntax,
        _leaf: &LeafBlock,
        _modes: IntraModes,
    ) -> Result<TxPartition> {
        Ok(TxPartition::None)
    }

    /// Codes luma block `tx` with one of `tx_types`, the types the block may
    /// code (`DCT_DCT` first); the default codes `tx.tx_type` through [`Self::code`].
    fn code_luma(
        &mut self,
        tx: &TransformBlock,
        prediction: &[u16],
        _tx_types: &[TxType],
    ) -> Result<TransformDecision> {
        self.code(tx, prediction)
    }

    /// Codes `tx` with `tx.tx_type`; its row-major prediction is `prediction`.
    fn code(&mut self, tx: &TransformBlock, prediction: &[u16]) -> Result<TransformDecision>;
}
