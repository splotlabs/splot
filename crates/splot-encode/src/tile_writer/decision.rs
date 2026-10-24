// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coding decisions of one superblock, the input of the key-frame tile writer.

pub(crate) use splot_decode::encoder_syntax::{
    CompoundMode, CompoundModes, InterMode, InterModes, InterPrediction, IntraMode, IntraModes,
    Ist, MotionVector, Partition, TxPartition, TxType,
};
use splot_decode::encoder_syntax::{IntraTileSyntax, TransformBlock};

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
    /// `PARTITION_NONE` with its choices.
    Leaf(LeafChoice),
    /// Any other partition, with one entry per § 5.20.3.1 child call in syntax
    /// order. Entries of children that start outside the frame are not coded.
    Split {
        kind: Partition,
        children: Vec<PartitionTree>,
    },
}

/// The modes and the luma transform partition of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LeafChoice {
    /// The intra modes; an inter leaf ignores them.
    pub(crate) modes: IntraModes,
    /// One of the leaf's `luma_tx_partitions`.
    pub(crate) tx_partition: TxPartition,
    /// `Some` for an inter leaf.
    pub(crate) inter: Option<InterPrediction>,
}

impl LeafChoice {
    /// `DC_PRED` with the largest transform.
    pub(crate) const DC: Self = Self::largest(IntraModes::DC);

    /// `modes` with the largest transform.
    pub(crate) const fn largest(modes: IntraModes) -> Self {
        Self {
            modes,
            tx_partition: TxPartition::None,
            inter: None,
        }
    }

    /// The inter leaf of `prediction`.
    pub(crate) const fn inter(prediction: InterPrediction) -> Self {
        Self {
            inter: Some(prediction),
            ..Self::DC
        }
    }
}

/// The coded form of one transform block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformDecision {
    /// The type the levels use: the block's `tx_type` from
    /// [`ResidualCoder::code`], or one of the offered types from
    /// [`ResidualCoder::code_luma`].
    pub(crate) tx_type: TxType,
    /// The secondary transform the levels use: the block's `ist`.
    pub(crate) ist: Ist,
    /// Quantized levels in the row-major adjusted (at most 32x32) layout.
    pub(crate) levels: Vec<i32>,
}

/// Codes each transform block once the writer knows its prediction, because
/// intra prediction reads the reconstruction of the blocks before it.
///
/// The decoder infers `DCT_DCT` for a luma block with one coded coefficient
/// (§ 5.20.8.2). When a [`Self::code_luma`] decision has `eob == 1` with
/// another type, the writer codes the block again through [`Self::code`] with
/// `DCT_DCT`; all-zero levels simply take `DCT_DCT`.
pub(crate) trait ResidualCoder {
    /// Codes `tx` with `tx.tx_type`, whose row-major prediction is
    /// `prediction`; `syntax` holds the CDF rows its coefficients will use.
    fn code(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
    ) -> Result<TransformDecision>;

    /// Codes luma block `tx` with one of `tx_types`, the types the block may
    /// code (`DCT_DCT` first); the default codes `tx.tx_type` through [`Self::code`].
    fn code_luma(
        &mut self,
        syntax: &mut IntraTileSyntax,
        tx: &TransformBlock,
        prediction: &[u16],
        _tx_types: &[TxType],
    ) -> Result<TransformDecision> {
        self.code(syntax, tx, prediction)
    }
}
