// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coding decisions of one superblock, the input of the key-frame tile writer.

pub(crate) use splot_decode::encoder_syntax::Partition;
use splot_decode::encoder_syntax::TransformBlock;

use super::Result;

/// The partition tree of one superblock (§ 5.20.3.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SuperblockDecision {
    pub(crate) tree: PartitionTree,
}

/// One `decode_partition` call: a coded leaf or a split into child calls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PartitionTree {
    /// `PARTITION_NONE`.
    Leaf(BlockDecision),
    /// Any other partition, with one entry per § 5.20.3.1 child call in syntax
    /// order. Entries of children that start outside the frame are not coded.
    Split {
        kind: Partition,
        children: Vec<PartitionTree>,
    },
}

/// The intra prediction modes of § 5.20.5.5 (`YMode` and `UVMode` values).
#[expect(
    dead_code,
    reason = "the decision model names every AV2 value; the writer codes DC only"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraMode {
    Dc,
    Vertical,
    Horizontal,
    D45,
    D135,
    D113,
    D157,
    D203,
    D67,
    Smooth,
    SmoothVertical,
    SmoothHorizontal,
    Paeth,
}

/// The luma prediction of a block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaMode {
    pub(crate) mode: IntraMode,
    /// `AngleDeltaY`, `-3..=3`, zero for non-directional modes.
    pub(crate) angle_delta: i8,
}

/// The chroma prediction of a block.
#[expect(
    dead_code,
    reason = "the decision model names every AV2 value; the writer codes DC only"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChromaMode {
    /// A `UVMode` with `AngleDeltaUV`.
    Intra { mode: IntraMode, angle_delta: i8 },
    /// `UV_CFL_PRED` with explicit alphas.
    Cfl { alpha_u: i8, alpha_v: i8 },
}

/// The transform types of § 3 (`DCT_DCT` = 0 … `H_FLIPADST` = 15).
#[expect(
    dead_code,
    reason = "the decision model names every AV2 value; the writer codes DC only"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TxType {
    DctDct,
    AdstDct,
    DctAdst,
    AdstAdst,
    FlipadstDct,
    DctFlipadst,
    FlipadstFlipadst,
    AdstFlipadst,
    FlipadstAdst,
    Idtx,
    VDct,
    HDct,
    VAdst,
    HAdst,
    VFlipadst,
    HFlipadst,
}

/// The coded form of one transform block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformDecision {
    pub(crate) tx_type: TxType,
    /// Quantized levels in the row-major adjusted (at most 32x32) layout.
    pub(crate) levels: Vec<i32>,
}

/// Codes each transform block once the writer knows its prediction, because
/// intra prediction reads the reconstruction of the blocks before it.
pub(crate) trait ResidualCoder {
    /// Codes `tx`, whose row-major prediction is `prediction`.
    fn code(&mut self, tx: &TransformBlock, prediction: &[u16]) -> Result<TransformDecision>;
}

/// Block-level choices of one leaf; its transform blocks come from a [`ResidualCoder`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockDecision {
    pub(crate) y_mode: LumaMode,
    pub(crate) uv_mode: ChromaMode,
}

impl BlockDecision {
    pub(crate) const DC: Self = Self {
        y_mode: LumaMode {
            mode: IntraMode::Dc,
            angle_delta: 0,
        },
        uv_mode: ChromaMode::Intra {
            mode: IntraMode::Dc,
            angle_delta: 0,
        },
    };
}
