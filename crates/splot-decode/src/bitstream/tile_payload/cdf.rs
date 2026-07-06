// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile CDF selection and lifecycle boundaries.

pub(crate) mod block_context;
pub(crate) mod block_read;
mod block_rows;
pub(crate) mod coeff_context;
mod coeff_rows;
pub(crate) mod context;
mod lifecycle;
pub(crate) mod partition_read;
mod util;

use core::fmt;

use splot_core::symbol::CdfUpdateMode;
use splot_core::tables::cdf::{
    DEFAULT_CCSO_BLK_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH3_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH4_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH5_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH6_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH7_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH8_CDF,
    DEFAULT_CDEF_INDEX0_CDF, DEFAULT_DELTA_Q_CDF, DEFAULT_DO_EXT_PARTITION_CDF,
    DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF, DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF,
    DEFAULT_FSC_MODE_CDF, DEFAULT_INTRABC_CDF, DEFAULT_INTRABC_MODE_CDF,
    DEFAULT_INTRABC_PRECISION_CDF, DEFAULT_MORPH_PRED_CDF, DEFAULT_MRL_INDEX_CDF,
    DEFAULT_MRL_SEC_INDEX_CDF, DEFAULT_RECT_TYPE_CDF, DEFAULT_REGION_TYPE_CDF,
    DEFAULT_SEG_ID_EXT_FLAG_CDF, DEFAULT_SEGMENT_ID_CDF, DEFAULT_SEGMENT_ID_EXT_CDF,
    DEFAULT_TX_2OR3_PARTITION_TYPE_CDF, DEFAULT_TX_DO_PARTITION_CDF, DEFAULT_TX_PARTITION_TYPE_CDF,
    DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF,
};

use self::block_rows::{BlockCdfRows, BlockCdfSelector};
pub(crate) use self::block_rows::{EobPtSize, MvCdfSelector};
pub(crate) use self::coeff_rows::CoeffCdfSelector;
pub(in crate::bitstream::tile_payload::cdf) use self::util::{
    avg_cdf_row, avg_cdf_rows, scale_cdf_count, scale_cdf_rows,
};
use self::util::{
    checked_context, checked_plane, checked_square_split_plane, floor_log2, tx_partition_type_array,
};
use super::coeff_loop::use_fsc_branch::coeff_cdf_q_ctx_from_base_q_idx;

pub(crate) const CDF_PROB_SCALE: i32 = 1 << 15;
pub(crate) const DO_SPLIT_PLANE_CONTEXTS: usize = 2;
pub(crate) const DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS: usize = 1;
const DO_SPLIT_CONTEXTS: usize = 64;
const DO_EXT_PARTITION_CONTEXTS: usize = 64;
const DO_UNEVEN_4WAY_PARTITION_CONTEXTS: usize = 64;
const RECT_TYPE_CONTEXTS: usize = 64;
const DO_SQUARE_SPLIT_CONTEXTS: usize = 8;
const CDF_ROW_LEN: usize = 3;
const TX_FSC_CONTEXTS: usize = 2;
const TX_IS_INTER_CONTEXTS: usize = 2;
const TXFM_SPLIT_GROUPS: usize = 9;
const TX_2OR3_PARTITION_TYPE_CONTEXTS: usize = 2;
const TX_PARTITION_TYPE_CONTEXTS: usize = 14;
const TX_PARTITION_TYPE_ROW_LEN: usize = 8;
const DELTA_Q_CDF_ROW_LEN: usize = 9;
const FSC_MODE_CONTEXTS: usize = 4;
const FSC_BSIZE_CONTEXTS: usize = 6;
const CDEF_STRENGTH_INDEX0_CONTEXTS: usize = 4;
const CCSO_PLANES: usize = 3;
const CCSO_CONTEXTS: usize = 4;
const CDEF_INDEX_MINUS1_WITH3_ROW_LEN: usize = 3;
const CDEF_INDEX_MINUS1_WITH4_ROW_LEN: usize = 4;
const CDEF_INDEX_MINUS1_WITH5_ROW_LEN: usize = 5;
const CDEF_INDEX_MINUS1_WITH6_ROW_LEN: usize = 6;
const CDEF_INDEX_MINUS1_WITH7_ROW_LEN: usize = 7;
const CDEF_INDEX_MINUS1_WITH8_ROW_LEN: usize = 8;
const INTRABC_CONTEXTS: usize = 3;
const MORPH_PRED_CONTEXTS: usize = 3;
const MRL_INDEX_CONTEXTS: usize = 3;
const MRL_INDEX_ROW_LEN: usize = 5;
const MRL_SEC_INDEX_ROW_LEN: usize = 3;
const SEGMENT_ID_CONTEXTS: usize = 3;
const SEGMENT_ID_ROW_LEN: usize = 9;
const SEG_ID_EXT_FLAG_ROW_LEN: usize = 3;
const INTER_SDP_BSIZE_GROUPS: usize = 4;

type DoSplitCdfRows = [[[i32; CDF_ROW_LEN]; DO_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoExtPartitionCdfRows =
    [[[i32; CDF_ROW_LEN]; DO_EXT_PARTITION_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoSquareSplitCdfRows =
    [[[i32; CDF_ROW_LEN]; DO_SQUARE_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoUneven4WayPartitionCdfRows =
    [[[i32; CDF_ROW_LEN]; DO_UNEVEN_4WAY_PARTITION_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type RectTypeCdfRows = [[[i32; CDF_ROW_LEN]; RECT_TYPE_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type TxDoPartitionCdfRows =
    [[[[i32; CDF_ROW_LEN]; TXFM_SPLIT_GROUPS]; TX_IS_INTER_CONTEXTS]; TX_FSC_CONTEXTS];
type Tx2Or3PartitionTypeCdfRows = [[[[i32; CDF_ROW_LEN]; TX_2OR3_PARTITION_TYPE_CONTEXTS];
    TX_IS_INTER_CONTEXTS]; TX_FSC_CONTEXTS];
type TxPartitionTypeCdfRows = [[[[i32; TX_PARTITION_TYPE_ROW_LEN]; TX_PARTITION_TYPE_CONTEXTS];
    TX_IS_INTER_CONTEXTS]; TX_FSC_CONTEXTS];
type DeltaQCdfRow = [i32; DELTA_Q_CDF_ROW_LEN];
type FscModeCdfRows = [[[i32; CDF_ROW_LEN]; FSC_BSIZE_CONTEXTS]; FSC_MODE_CONTEXTS];
type CdefIndex0CdfRows = [[i32; CDF_ROW_LEN]; CDEF_STRENGTH_INDEX0_CONTEXTS];
type CcsoBlkCdfRows = [[[i32; CDF_ROW_LEN]; CCSO_CONTEXTS]; CCSO_PLANES];
type CdefIndexMinus1With3CdfRow = [i32; CDEF_INDEX_MINUS1_WITH3_ROW_LEN];
type CdefIndexMinus1With4CdfRow = [i32; CDEF_INDEX_MINUS1_WITH4_ROW_LEN];
type CdefIndexMinus1With5CdfRow = [i32; CDEF_INDEX_MINUS1_WITH5_ROW_LEN];
type CdefIndexMinus1With6CdfRow = [i32; CDEF_INDEX_MINUS1_WITH6_ROW_LEN];
type CdefIndexMinus1With7CdfRow = [i32; CDEF_INDEX_MINUS1_WITH7_ROW_LEN];
type CdefIndexMinus1With8CdfRow = [i32; CDEF_INDEX_MINUS1_WITH8_ROW_LEN];
type IntrabcCdfRows = [[i32; CDF_ROW_LEN]; INTRABC_CONTEXTS];
type IntrabcModeCdfRow = [i32; CDF_ROW_LEN];
type IntrabcPrecisionCdfRow = [i32; CDF_ROW_LEN];
type MorphPredCdfRows = [[i32; CDF_ROW_LEN]; MORPH_PRED_CONTEXTS];
type MrlIndexCdfRows = [[i32; MRL_INDEX_ROW_LEN]; MRL_INDEX_CONTEXTS];
type MrlSecIndexCdfRows = [[i32; MRL_SEC_INDEX_ROW_LEN]; MRL_INDEX_CONTEXTS];
type SegmentIdCdfRows = [[i32; SEGMENT_ID_ROW_LEN]; SEGMENT_ID_CONTEXTS];
type SegIdExtFlagCdfRows = [[i32; SEG_ID_EXT_FLAG_ROW_LEN]; SEGMENT_ID_CONTEXTS];
type RegionTypeCdfRows = [[i32; CDF_ROW_LEN]; INTER_SDP_BSIZE_GROUPS];
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfPolicyInput {
    tile_cols: u32,
    tile_rows: u32,
    enable_avg_cdf: bool,
    avg_cdf_type: bool,
    context_update_tile_id: u32,
}

impl TileCdfPolicyInput {
    #[must_use]
    pub(crate) const fn new(
        tile_cols: u32,
        tile_rows: u32,
        enable_avg_cdf: bool,
        avg_cdf_type: bool,
        context_update_tile_id: u32,
    ) -> Self {
        Self {
            tile_cols,
            tile_rows,
            enable_avg_cdf,
            avg_cdf_type,
            context_update_tile_id,
        }
    }
    #[must_use]
    pub(crate) const fn single_tile_default() -> Self {
        Self::new(1, 1, false, false, 0)
    }
    #[must_use]
    pub(crate) const fn with_tile_grid(mut self, tile_cols: u32, tile_rows: u32) -> Self {
        self.tile_cols = tile_cols;
        self.tile_rows = tile_rows;
        self
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfSavePolicy {
    num_log2: u8,
    copy_cdf: bool,
    avg_cdf: bool,
}

impl TileCdfSavePolicy {
    #[must_use]
    pub(crate) const fn num_log2(self) -> u8 {
        self.num_log2
    }
    #[must_use]
    pub(crate) const fn copy_cdf(self) -> bool {
        self.copy_cdf
    }
    #[must_use]
    pub(crate) const fn avg_cdf(self) -> bool {
        self.avg_cdf
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrameCdfSubset {
    rows: TileCdfRows,
}

impl FrameCdfSubset {
    #[must_use]
    pub(crate) fn from_defaults() -> Self {
        Self {
            rows: TileCdfRows::from_defaults(),
        }
    }

    pub(crate) fn default_for_base_q(base_q_idx: u32) -> Result<Self, TileCdfError> {
        let mut cdfs = Self::from_defaults();
        cdfs.rows
            .block
            .replicate_coeff_q_context(coeff_cdf_q_ctx_from_base_q_idx(base_q_idx))?;
        Ok(cdfs)
    }

    #[must_use]
    pub(crate) fn tile_copy(&self) -> TileCdfSubset {
        TileCdfSubset {
            rows: self.rows.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn rows(&self) -> &TileCdfRows {
        &self.rows
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfSubset {
    rows: TileCdfRows,
}

impl TileCdfSubset {
    pub(crate) fn row(&self, selector: TileCdfSelector) -> Result<&[i32], TileCdfError> {
        self.rows.row(selector)
    }
    pub(crate) fn with_row_mut<R>(
        &mut self,
        selector: TileCdfSelector,
        f: impl FnOnce(&mut [i32]) -> R,
    ) -> Result<R, TileCdfError> {
        Ok(f(self.rows.row_mut(selector)?))
    }

    #[cfg(test)]
    pub(crate) const fn rows(&self) -> &TileCdfRows {
        &self.rows
    }

    #[cfg(test)]
    pub(crate) fn rows_mut(&mut self) -> &mut TileCdfRows {
        &mut self.rows
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SavedCdfSubset {
    rows: TileCdfRows,
}

impl SavedCdfSubset {
    #[must_use]
    pub(crate) fn from_frame(frame: &FrameCdfSubset) -> Self {
        Self {
            rows: frame.rows.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) const fn rows(&self) -> &TileCdfRows {
        &self.rows
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfWorkUnitBoundary {
    update_mode: CdfUpdateMode,
    save_policy: TileCdfSavePolicy,
    frame_cdfs: FrameCdfSubset,
    saved_cdfs: SavedCdfSubset,
    tile_cdfs: TileCdfSubset,
}

impl TileCdfWorkUnitBoundary {
    #[must_use]
    pub(crate) fn new(
        update_mode: CdfUpdateMode,
        save_policy: TileCdfSavePolicy,
        frame_cdfs: FrameCdfSubset,
    ) -> Self {
        let saved_cdfs = SavedCdfSubset::from_frame(&frame_cdfs);
        let tile_cdfs = frame_cdfs.tile_copy();
        Self {
            update_mode,
            save_policy,
            frame_cdfs,
            saved_cdfs,
            tile_cdfs,
        }
    }
    #[must_use]
    pub(crate) const fn update_mode(&self) -> CdfUpdateMode {
        self.update_mode
    }
    #[must_use]
    pub(crate) const fn save_policy(&self) -> TileCdfSavePolicy {
        self.save_policy
    }
    #[must_use]
    pub(crate) const fn tile_cdfs(&self) -> &TileCdfSubset {
        &self.tile_cdfs
    }
    pub(crate) fn tile_cdfs_mut(&mut self) -> &mut TileCdfSubset {
        &mut self.tile_cdfs
    }

    #[cfg(test)]
    pub(crate) const fn frame_cdfs(&self) -> &FrameCdfSubset {
        &self.frame_cdfs
    }

    pub(crate) fn frame_cdfs_clone(&self) -> FrameCdfSubset {
        self.frame_cdfs.clone()
    }

    #[cfg(test)]
    pub(crate) const fn saved_cdfs(&self) -> &SavedCdfSubset {
        &self.saved_cdfs
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileCdfSelector {
    DoSplit {
        plane_start: usize,
        ctx: usize,
    },
    DoExtPartition {
        plane_start: usize,
        ctx: usize,
    },
    DoSquareSplit {
        plane_start: usize,
        ctx: usize,
    },
    RectType {
        plane_start: usize,
        ctx: usize,
    },
    DoUneven4WayPartition {
        plane_start: usize,
        ctx: usize,
    },
    TxDoPartition {
        fsc_mode: usize,
        is_inter: usize,
        txfm_split_group: usize,
    },
    Tx2Or3PartitionType {
        fsc_mode: usize,
        is_inter: usize,
        ctx: usize,
    },
    TxPartitionType {
        fsc_mode: usize,
        is_inter: usize,
        ctx: usize,
        reduced: bool,
    },
    DeltaQ,
    CdefIndex0 {
        ctx: usize,
    },
    CcsoBlk {
        plane: usize,
        ctx: usize,
    },
    CdefIndexMinus1 {
        strengths: usize,
    },
    Intrabc {
        ctx: usize,
    },
    IntrabcMode,
    IntrabcPrecision,
    MorphPred {
        ctx: usize,
    },
    FscMode {
        ctx: usize,
        bsize_group: usize,
    },
    MrlIndex {
        ctx: usize,
    },
    MrlSecIndex {
        ctx: usize,
    },
    SegIdExtFlag {
        ctx: usize,
    },
    SegmentId {
        ctx: usize,
        ext: bool,
    },
    RegionType {
        ctx: usize,
    },
    YModeSet,
    YModeIndex {
        ctx: usize,
    },
    YModeOffset {
        ctx: usize,
    },
    TxbSkip {
        coeff_cdf_q_ctx: usize,
        plane_type: usize,
        tx_size: usize,
        ctx: usize,
    },
    IntraTxTypeSet1 {
        tx_size_sqr: usize,
    },
    IntraTxTypeSet2 {
        tx_size_sqr: usize,
    },
    IsLongSideDct {
        is_inter: usize,
    },
    IntraTxTypeLong {
        tx_size_sqr: usize,
    },
    InterTxTypeLong {
        ctx: usize,
        tx_size_sqr: usize,
    },
    InterTxTypeSet1 {
        ctx: usize,
        tx_size_sqr: usize,
    },
    InterTxTypeSet2 {
        ctx: usize,
    },
    InterTxTypeIndexSet1 {
        ctx: usize,
    },
    InterTxTypeIndexSet2 {
        ctx: usize,
    },
    InterTxTypeOffsetSet1 {
        ctx: usize,
    },
    InterTxTypeOffsetSet2 {
        ctx: usize,
    },
    InterTxTypeSet3 {
        ctx: usize,
        tx_size_sqr: usize,
    },
    InterTxTypeSet4 {
        ctx: usize,
        tx_size_sqr: usize,
    },
    SecTxType {
        is_inter: usize,
        tx_size_sqr: usize,
    },
    MostProbableStxSet,
    MostProbableStxSetAdst,
    CctxType,
    PaletteYMode,
    PaletteYSize,
    IdentityRowY {
        ctx: usize,
    },
    PaletteYColorIndex {
        palette_size: usize,
        ctx: usize,
    },
    UvModeCflNotAllowed {
        ctx: usize,
    },
    IsCfl {
        ctx: usize,
    },
    CflIndex,
    CflSign,
    CflAlpha {
        ctx: usize,
    },
    CflMhccp,
    CflMhDir {
        size_group: usize,
    },
    VTxbSkip {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    EobExtra {
        coeff_cdf_q_ctx: usize,
    },
    EobPt {
        size: EobPtSize,
        coeff_cdf_q_ctx: usize,
        eob_ctx: usize,
    },
    DcSign {
        coeff_cdf_q_ctx: usize,
        plane_type: usize,
        group: usize,
        ctx: usize,
    },
    IsInter {
        ctx: usize,
    },
    Skip {
        ctx: usize,
    },
    SingleMode {
        ctx: usize,
    },
    IsWarp {
        ctx: usize,
    },
    WarpMv,
    WarpIdx {
        ctx: usize,
    },
    WarpWithMvd,
    WarpPrecision {
        block_size: usize,
    },
    WarpDeltaParamLow {
        index_type: usize,
    },
    WarpDeltaParamHigh {
        index_type: usize,
    },
    WarpDeltaParamSign,
    WarpInterIntra {
        bsize_group: usize,
    },
    InterIntra {
        bsize_group: usize,
    },
    InterIntraMode {
        bsize_group: usize,
    },
    WedgeInterIntra,
    WedgeQuad,
    WedgeAngle {
        quad: usize,
    },
    WedgeDist1,
    WedgeDist2,
    DrlMode {
        idx: usize,
        ctx: usize,
    },
    SingleRef {
        ctx: usize,
        ref_idx: usize,
    },
    CompMode {
        ctx: usize,
    },
    IsJoint {
        ctx: usize,
    },
    CompoundModeNonJoint {
        ctx: usize,
    },
    CompGroupIdx {
        ctx: usize,
    },
    CwpIdx {
        idx: usize,
    },
    CompRef0 {
        ctx: usize,
        ref_idx: usize,
    },
    CompRef1 {
        ctx: usize,
        bit_type: usize,
        ref_idx: usize,
    },
    ReadMv(MvCdfSelector),
    InterpFilter {
        ctx: usize,
    },
    UseAmvd {
        index: usize,
        ctx: usize,
    },
    UseExtendWarp {
        ctx: usize,
    },
    UseLocalWarp {
        ctx: usize,
    },
    UseMostProbablePrecision {
        ctx: usize,
    },
    PbMvPrecision {
        ctx: usize,
        frame_ctx: usize,
    },
    UseBawp,
    UseBawpChroma,
    ExplicitBawp {
        ctx: usize,
    },
    ExplicitBawpScale,
    UseWienerNs,
    WienerNsLength {
        plane_ctx: usize,
    },
    WienerNsUvSym,
    WienerNsBase,
    Coeff(CoeffCdfSelector),
}
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileCdfArray {
    DoSplit,
    DoExtPartition,
    DoSquareSplit,
    RectType,
    DoUneven4WayPartition,
    TxDoPartition,
    Tx2Or3PartitionType,
    TxPartitionType,
    TxPartitionTypeReduced,
    CdefIndex0,
    CcsoBlk,
    CdefIndexMinus1,
    Intrabc,
    MorphPred,
    FscMode,
    MrlIndex,
    MrlSecIndex,
    SegIdExtFlag,
    SegmentId,
    SegmentIdExt,
    RegionType,
    YModeIndex,
    YModeOffset,
    TxbSkip,
    IntraTxTypeSet1,
    IntraTxTypeSet2,
    IsLongSideDct,
    IntraTxTypeLong,
    InterTxTypeLong,
    InterTxTypeSet1,
    InterTxTypeSet2,
    InterTxTypeIndexSet1,
    InterTxTypeIndexSet2,
    InterTxTypeOffsetSet1,
    InterTxTypeOffsetSet2,
    InterTxTypeSet3,
    InterTxTypeSet4,
    SecTxType,
    UvModeCflNotAllowed,
    IsCfl,
    CflAlpha,
    CflMhDir,
    VTxbSkip,
    EobExtra,
    EobPt,
    DcSign,
    CoeffBase,
    CoeffBasePh,
    CoeffBaseUv,
    CoeffBaseLf,
    CoeffBaseLfUv,
    CoeffBaseEob,
    CoeffBaseEobUv,
    CoeffBaseBob,
    CoeffBaseIdtx,
    CoeffBaseLfEob,
    CoeffBaseLfEobUv,
    CoeffBr,
    CoeffBrUv,
    CoeffBrLf,
    CoeffBrIdtx,
    IdtxSign,
    IsInter,
    Skip,
    SingleMode,
    IsWarp,
    WarpMv,
    WarpIdx,
    WarpWithMvd,
    WarpPrecision,
    WarpDeltaParamLow,
    WarpDeltaParamHigh,
    WarpDeltaParamSign,
    WarpInterIntra,
    InterIntra,
    InterIntraMode,
    WedgeInterIntra,
    WedgeQuad,
    WedgeAngle,
    WedgeDist1,
    WedgeDist2,
    DrlMode,
    SingleRef,
    CompMode,
    IsJoint,
    CompoundModeNonJoint,
    CompGroupIdx,
    CwpIdx,
    CompRef1,
    CompRef0,
    UseAmvd,
    UseExtendWarp,
    UseLocalWarp,
    UseMostProbablePrecision,
    PbMvPrecision,
    ExplicitBawp,
    AmvdJoint,
    AmvdIndex,
    JointShell6Class,
    ShellOffsetLowClass,
    ShellOffsetOtherClass,
    ColMvGreater,
    ColMvIndex,
    InterpFilter,
    WienerNsLength,
    PaletteYSize,
    IdentityRowY,
    PaletteYColorIndex,
}

crate::impl_reason_labels!(TileCdfArray {
    DoSplit => "TileDoSplitCdf",
    DoExtPartition => "TileDoExtPartitionCdf",
    DoSquareSplit => "TileDoSquareSplitCdf",
    RectType => "TileRectTypeCdf",
    DoUneven4WayPartition => "TileDoUneven4wayPartitionCdf",
    TxDoPartition => "TileTxDoPartitionCdf",
    Tx2Or3PartitionType => "TileTx2or3PartitionTypeCdf",
    TxPartitionType => "TileTxPartitionTypeCdf",
    TxPartitionTypeReduced => "TileTxPartitionTypeReducedCdf",
    CdefIndex0 => "TileCdefIndex0Cdf",
    CcsoBlk => "TileCcsoBlkCdf",
    CdefIndexMinus1 => "TileCdefIndexMinus1Cdf",
    Intrabc => "TileIntrabcCdf",
    MorphPred => "TileMorphPredCdf",
    FscMode => "TileFscModeCdf",
    MrlIndex => "TileMrlIndexCdf",
    MrlSecIndex => "TileMrlSecIndexCdf",
    SegIdExtFlag => "TileSegIdExtFlagCdf",
    SegmentId => "TileSegmentIdCdf",
    SegmentIdExt => "TileSegmentIdExtCdf",
    RegionType => "TileRegionTypeCdf",
    YModeIndex => "TileYModeIndexCdf",
    YModeOffset => "TileYModeOffsetCdf",
    TxbSkip => "TileTxbSkipCdf",
    IntraTxTypeSet1 => "TileIntraTxTypeSet1Cdf",
    IntraTxTypeSet2 => "TileIntraTxTypeSet2Cdf",
    IsLongSideDct => "TileIsLongSideDctCdf",
    IntraTxTypeLong => "TileIntraTxTypeLongCdf",
    InterTxTypeLong => "TileInterTxTypeLongCdf",
    InterTxTypeSet1 => "TileInterTxTypeSet1Cdf",
    InterTxTypeSet2 => "TileInterTxTypeSet2Cdf",
    InterTxTypeIndexSet1 => "TileInterTxTypeIndexSet1Cdf",
    InterTxTypeIndexSet2 => "TileInterTxTypeIndexSet2Cdf",
    InterTxTypeOffsetSet1 => "TileInterTxTypeOffsetSet1Cdf",
    InterTxTypeOffsetSet2 => "TileInterTxTypeOffsetSet2Cdf",
    InterTxTypeSet3 => "TileInterTxTypeSet3Cdf",
    InterTxTypeSet4 => "TileInterTxTypeSet4Cdf",
    SecTxType => "TileSecTxTypeCdf",
    UvModeCflNotAllowed => "TileUvModeCflNotAllowedCdf",
    IsCfl => "TileIsCflCdf",
    CflAlpha => "TileCflAlphaCdf",
    CflMhDir => "TileCflMhDirCdf",
    VTxbSkip => "TileVTxbSkipCdf",
    EobExtra => "TileEobExtraCdf",
    EobPt => "TileEobPtCdf",
    DcSign => "TileDcSignCdf",
    CoeffBase => "TileCoeffBaseCdf",
    CoeffBasePh => "TileCoeffBasePhCdf",
    CoeffBaseUv => "TileCoeffBaseUvCdf",
    CoeffBaseLf => "TileCoeffBaseLfCdf",
    CoeffBaseLfUv => "TileCoeffBaseLfUvCdf",
    CoeffBaseEob => "TileCoeffBaseEobCdf",
    CoeffBaseEobUv => "TileCoeffBaseEobUvCdf",
    CoeffBaseBob => "TileCoeffBaseBobCdf",
    CoeffBaseIdtx => "TileCoeffBaseIdtxCdf",
    CoeffBaseLfEob => "TileCoeffBaseLfEobCdf",
    CoeffBaseLfEobUv => "TileCoeffBaseLfEobUvCdf",
    CoeffBr => "TileCoeffBrCdf",
    CoeffBrUv => "TileCoeffBrUvCdf",
    CoeffBrLf => "TileCoeffBrLfCdf",
    CoeffBrIdtx => "TileCoeffBrIdtxCdf",
    IdtxSign => "TileIdtxSignCdf",
    IsInter => "TileIsInterCdf",
    Skip => "TileSkipCdf",
    SingleMode => "TileSingleModeCdf",
    IsWarp => "TileIsWarpCdf",
    WarpMv => "TileWarpMvCdf",
    WarpIdx => "TileWarpIdxCdf",
    WarpWithMvd => "TileWarpWithMvdCdf",
    WarpPrecision => "TileWarpPrecisionCdf",
    WarpDeltaParamLow => "TileWarpDeltaParamLowCdf",
    WarpDeltaParamHigh => "TileWarpDeltaParamHighCdf",
    WarpDeltaParamSign => "TileWarpDeltaParamSignCdf",
    WarpInterIntra => "TileWarpInterIntraCdf",
    InterIntra => "TileInterIntraCdf",
    InterIntraMode => "TileInterIntraModeCdf",
    WedgeInterIntra => "TileWedgeInterIntraCdf",
    WedgeQuad => "TileWedgeQuadCdf",
    WedgeAngle => "TileWedgeAngleCdf",
    WedgeDist1 => "TileWedgeDist1Cdf",
    WedgeDist2 => "TileWedgeDist2Cdf",
    DrlMode => "TileDrlModeCdf",
    SingleRef => "TileSingleRefCdf",
    CompMode => "TileCompModeCdf",
    IsJoint => "TileIsJointCdf",
    CompoundModeNonJoint => "TileCompoundModeNonJointCdf",
    CompGroupIdx => "TileCompGroupIdxCdf",
    CwpIdx => "TileCwpIdxCdf",
    CompRef0 => "TileCompRef0Cdf",
    CompRef1 => "TileCompRef1Cdf",
    UseAmvd => "TileUseAmvdCdf",
    UseExtendWarp => "TileUseExtendWarpCdf",
    UseLocalWarp => "TileUseLocalWarpCdf",
    UseMostProbablePrecision => "TileUseMostProbablePrecisionCdf",
    PbMvPrecision => "TilePbMvPrecisionCdf",
    ExplicitBawp => "TileExplicitBawpCdf",
    AmvdJoint => "TileAmvdJointCdf",
    AmvdIndex => "TileAmvdIndexCdf",
    JointShell6Class => "TileJointShell6ClassCdf",
    ShellOffsetLowClass => "TileShellOffsetLowClassCdf",
    ShellOffsetOtherClass => "TileShellOffsetOtherClassCdf",
    ColMvGreater => "TileColMvGreaterCdf",
    ColMvIndex => "TileColMvIndexCdf",
    InterpFilter => "TileInterpFilterCdf",
    WienerNsLength => "TileWienerNsLengthCdf",
    PaletteYSize => "TilePaletteYSizeCdf",
    IdentityRowY => "TileIdentityRowYCdf",
    PaletteYColorIndex => "TilePaletteYColorIndexCdf",
});
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum TileCdfError {
    #[error("{array} selector {index_name}={actual} is outside 0..{max_exclusive}")]
    SelectorOutOfRange {
        array: TileCdfArray,
        index_name: &'static str,
        actual: usize,
        max_exclusive: usize,
    },
    #[error("tile count overflow for TileCols={tile_cols}, TileRows={tile_rows}")]
    TileCountOverflow { tile_cols: u32, tile_rows: u32 },
    #[error("tile count must be nonzero, got TileCols={tile_cols}, TileRows={tile_rows}")]
    InvalidTileCount { tile_cols: u32, tile_rows: u32 },
    #[error("TileNum={tile_num} is outside tile count {tile_count}")]
    TileNumOutOfRange { tile_num: u32, tile_count: u32 },
    #[error("context_update_tile_id={context_update_tile_id} is outside tile count {tile_count}")]
    ContextUpdateTileOutOfRange {
        context_update_tile_id: u32,
        tile_count: u32,
    },
    #[error("{table} bSize={b_size} is outside 0..{max_exclusive}")]
    BlockSizeOutOfRange {
        table: &'static str,
        b_size: usize,
        max_exclusive: usize,
    },
    #[error("{array}[{plane_start}][{index}] is unavailable; length {len}")]
    PartitionNeighborOutOfRange {
        array: &'static str,
        plane_start: usize,
        index: usize,
        len: usize,
    },
    #[error(
        "{array}[{plane_start}][{index}] block size {block_size} is outside 0..{max_exclusive}"
    )]
    #[allow(dead_code)]
    PartitionNeighborBlockSizeOutOfRange {
        array: &'static str,
        plane_start: usize,
        index: usize,
        block_size: usize,
        max_exclusive: usize,
    },
    #[error(
        "{array}[{plane_start}][{index}] partition context {context} is outside 0..{max_exclusive}"
    )]
    PartitionNeighborContextOutOfRange {
        array: &'static str,
        plane_start: usize,
        index: usize,
        context: usize,
        max_exclusive: usize,
    },
    #[error(
        "{array}[{plane_start}] {coordinate} coordinate underflow deriving {actual}-{subtract}"
    )]
    PartitionGridCoordinateUnderflow {
        array: &'static str,
        plane_start: usize,
        coordinate: &'static str,
        actual: usize,
        subtract: usize,
    },
    #[error("{array}[{plane_start}][{row}] row is unavailable; rows {rows}")]
    PartitionGridRowOutOfRange {
        array: &'static str,
        plane_start: usize,
        row: usize,
        rows: usize,
    },
    #[error("{array}[{plane_start}][{row}][{col}] column is unavailable; columns {cols}")]
    PartitionGridColumnOutOfRange {
        array: &'static str,
        plane_start: usize,
        row: usize,
        col: usize,
        cols: usize,
    },
    #[error(
        "{array}[{plane_start}][{row}][{col}] block size {block_size} is outside 0..{max_exclusive}"
    )]
    PartitionGridBlockSizeOutOfRange {
        array: &'static str,
        plane_start: usize,
        row: usize,
        col: usize,
        block_size: usize,
        max_exclusive: usize,
    },
    #[error("{array}[{plane_start}] index overflow deriving {base}+{offset}")]
    PartitionNeighborIndexOverflow {
        array: &'static str,
        plane_start: usize,
        base: usize,
        offset: usize,
    },
    #[error("{table}[{b_size}] value {value} cannot be represented as a context index")]
    ConversionTableValueOutOfRange {
        table: &'static str,
        b_size: usize,
        value: i32,
    },
}

impl fmt::Display for TileCdfArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
pub(crate) fn tile_cdf_save_policy(
    input: TileCdfPolicyInput,
    tile_num: u32,
) -> Result<TileCdfSavePolicy, TileCdfError> {
    let tile_count =
        input
            .tile_cols
            .checked_mul(input.tile_rows)
            .ok_or(TileCdfError::TileCountOverflow {
                tile_cols: input.tile_cols,
                tile_rows: input.tile_rows,
            })?;
    if tile_count == 0 {
        return Err(TileCdfError::InvalidTileCount {
            tile_cols: input.tile_cols,
            tile_rows: input.tile_rows,
        });
    }
    if tile_num >= tile_count {
        return Err(TileCdfError::TileNumOutOfRange {
            tile_num,
            tile_count,
        });
    }

    let num_log2 = floor_log2(tile_count).min(3) as u8;
    let mut copy_cdf = false;
    let mut avg_cdf = false;
    if input.enable_avg_cdf && input.avg_cdf_type {
        avg_cdf = tile_num < (1u32 << num_log2);
    } else {
        if input.context_update_tile_id >= tile_count {
            return Err(TileCdfError::ContextUpdateTileOutOfRange {
                context_update_tile_id: input.context_update_tile_id,
                tile_count,
            });
        }
        copy_cdf = tile_num == input.context_update_tile_id;
    }

    Ok(TileCdfSavePolicy {
        num_log2,
        copy_cdf,
        avg_cdf,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfRows {
    do_split: DoSplitCdfRows,
    do_ext_partition: DoExtPartitionCdfRows,
    do_square_split: DoSquareSplitCdfRows,
    rect_type: RectTypeCdfRows,
    do_uneven_4way_partition: DoUneven4WayPartitionCdfRows,
    tx_do_partition: TxDoPartitionCdfRows,
    tx_2or3_partition_type: Tx2Or3PartitionTypeCdfRows,
    tx_partition_type: TxPartitionTypeCdfRows,
    tx_partition_type_reduced: TxPartitionTypeCdfRows,
    delta_q: DeltaQCdfRow,
    cdef_index0: CdefIndex0CdfRows,
    ccso_blk: CcsoBlkCdfRows,
    cdef_index_minus1_with3: CdefIndexMinus1With3CdfRow,
    cdef_index_minus1_with4: CdefIndexMinus1With4CdfRow,
    cdef_index_minus1_with5: CdefIndexMinus1With5CdfRow,
    cdef_index_minus1_with6: CdefIndexMinus1With6CdfRow,
    cdef_index_minus1_with7: CdefIndexMinus1With7CdfRow,
    cdef_index_minus1_with8: CdefIndexMinus1With8CdfRow,
    intrabc: IntrabcCdfRows,
    intrabc_mode: IntrabcModeCdfRow,
    intrabc_precision: IntrabcPrecisionCdfRow,
    morph_pred: MorphPredCdfRows,
    fsc_mode: FscModeCdfRows,
    mrl_index: MrlIndexCdfRows,
    mrl_sec_index: MrlSecIndexCdfRows,
    seg_id_ext_flag: SegIdExtFlagCdfRows,
    segment_id: SegmentIdCdfRows,
    segment_id_ext: SegmentIdCdfRows,
    region_type: RegionTypeCdfRows,
    block: BlockCdfRows,
}

macro_rules! selected_cdf_row {
    ($rows:expr, $index:expr, $get:ident, $as_slice:ident, $array:expr, $index_name:literal) => {{
        let max_exclusive = $rows.len();
        let row = $rows.$get($index).ok_or(TileCdfError::SelectorOutOfRange {
            array: $array,
            index_name: $index_name,
            actual: $index,
            max_exclusive,
        })?;
        Ok(row.$as_slice())
    }};
}

macro_rules! tile_cdf_row {
    (
        $self:expr,
        $selector:expr,
        $get:ident,
        $as_slice:ident,
        $block_row:ident,
        $tx_partition_rows:ident,
        $cdef_index_minus1_row:ident
    ) => {
        match $selector {
            TileCdfSelector::DoSplit { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoSplit, plane_start)?;
                selected_cdf_row!(
                    $self.do_split[plane],
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::DoSplit,
                    "ctx"
                )
            }
            TileCdfSelector::DoExtPartition { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoExtPartition, plane_start)?;
                selected_cdf_row!(
                    $self.do_ext_partition[plane],
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::DoExtPartition,
                    "ctx"
                )
            }
            TileCdfSelector::DoSquareSplit { plane_start, ctx } => {
                let plane = checked_square_split_plane(plane_start)?;
                selected_cdf_row!(
                    $self.do_square_split[plane],
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::DoSquareSplit,
                    "ctx"
                )
            }
            TileCdfSelector::RectType { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::RectType, plane_start)?;
                selected_cdf_row!(
                    $self.rect_type[plane],
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::RectType,
                    "ctx"
                )
            }
            TileCdfSelector::DoUneven4WayPartition { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoUneven4WayPartition, plane_start)?;
                selected_cdf_row!(
                    $self.do_uneven_4way_partition[plane],
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::DoUneven4WayPartition,
                    "ctx"
                )
            }
            TileCdfSelector::TxDoPartition {
                fsc_mode,
                is_inter,
                txfm_split_group,
            } => {
                let fsc_mode = checked_context(
                    TileCdfArray::TxDoPartition,
                    "fsc_mode",
                    fsc_mode,
                    TX_FSC_CONTEXTS,
                )?;
                let is_inter = checked_context(
                    TileCdfArray::TxDoPartition,
                    "is_inter",
                    is_inter,
                    TX_IS_INTER_CONTEXTS,
                )?;
                selected_cdf_row!(
                    $self.tx_do_partition[fsc_mode][is_inter],
                    txfm_split_group,
                    $get,
                    $as_slice,
                    TileCdfArray::TxDoPartition,
                    "txfm_split_group"
                )
            }
            TileCdfSelector::Tx2Or3PartitionType {
                fsc_mode,
                is_inter,
                ctx,
            } => {
                let fsc_mode = checked_context(
                    TileCdfArray::Tx2Or3PartitionType,
                    "fsc_mode",
                    fsc_mode,
                    TX_FSC_CONTEXTS,
                )?;
                let is_inter = checked_context(
                    TileCdfArray::Tx2Or3PartitionType,
                    "is_inter",
                    is_inter,
                    TX_IS_INTER_CONTEXTS,
                )?;
                selected_cdf_row!(
                    $self.tx_2or3_partition_type[fsc_mode][is_inter],
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::Tx2Or3PartitionType,
                    "ctx"
                )
            }
            TileCdfSelector::TxPartitionType {
                fsc_mode,
                is_inter,
                ctx,
                reduced,
            } => {
                let array = tx_partition_type_array(reduced);
                let fsc_mode = checked_context(array, "fsc_mode", fsc_mode, TX_FSC_CONTEXTS)?;
                let is_inter = checked_context(array, "is_inter", is_inter, TX_IS_INTER_CONTEXTS)?;
                let rows = $tx_partition_rows($self, reduced);
                selected_cdf_row!(rows[fsc_mode][is_inter], ctx, $get, $as_slice, array, "ctx")
            }
            TileCdfSelector::DeltaQ => Ok($self.delta_q.$as_slice()),
            TileCdfSelector::CdefIndex0 { ctx } => {
                selected_cdf_row!(
                    $self.cdef_index0,
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::CdefIndex0,
                    "ctx"
                )
            }
            TileCdfSelector::CcsoBlk { plane, ctx } => {
                let max_exclusive = $self.ccso_blk.len();
                let plane_rows =
                    $self
                        .ccso_blk
                        .$get(plane)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::CcsoBlk,
                            index_name: "plane",
                            actual: plane,
                            max_exclusive,
                        })?;
                let max_exclusive = plane_rows.len();
                let row = plane_rows
                    .$get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CcsoBlk,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.$as_slice())
            }
            TileCdfSelector::CdefIndexMinus1 { strengths } => {
                $cdef_index_minus1_row($self, strengths)
            }
            TileCdfSelector::Intrabc { ctx } => {
                selected_cdf_row!(
                    $self.intrabc,
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::Intrabc,
                    "ctx"
                )
            }
            TileCdfSelector::IntrabcMode => Ok($self.intrabc_mode.$as_slice()),
            TileCdfSelector::IntrabcPrecision => Ok($self.intrabc_precision.$as_slice()),
            TileCdfSelector::MorphPred { ctx } => {
                selected_cdf_row!(
                    $self.morph_pred,
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::MorphPred,
                    "ctx"
                )
            }
            TileCdfSelector::FscMode { ctx, bsize_group } => {
                let ctx = checked_context(TileCdfArray::FscMode, "ctx", ctx, FSC_MODE_CONTEXTS)?;
                selected_cdf_row!(
                    $self.fsc_mode[ctx],
                    bsize_group,
                    $get,
                    $as_slice,
                    TileCdfArray::FscMode,
                    "bsize_group"
                )
            }
            TileCdfSelector::MrlIndex { ctx } => {
                let ctx = checked_context(TileCdfArray::MrlIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok($self.mrl_index[ctx].$as_slice())
            }
            TileCdfSelector::MrlSecIndex { ctx } => {
                let ctx =
                    checked_context(TileCdfArray::MrlSecIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok($self.mrl_sec_index[ctx].$as_slice())
            }
            TileCdfSelector::SegIdExtFlag { ctx } => {
                let ctx =
                    checked_context(TileCdfArray::SegIdExtFlag, "ctx", ctx, SEGMENT_ID_CONTEXTS)?;
                Ok($self.seg_id_ext_flag[ctx].$as_slice())
            }
            TileCdfSelector::SegmentId { ctx, ext } => {
                let ctx =
                    checked_context(TileCdfArray::SegmentId, "ctx", ctx, SEGMENT_ID_CONTEXTS)?;
                if ext {
                    Ok($self.segment_id_ext[ctx].$as_slice())
                } else {
                    Ok($self.segment_id[ctx].$as_slice())
                }
            }
            TileCdfSelector::RegionType { ctx } => {
                selected_cdf_row!(
                    $self.region_type,
                    ctx,
                    $get,
                    $as_slice,
                    TileCdfArray::RegionType,
                    "ctx"
                )
            }
            TileCdfSelector::YModeSet => $self.block.$block_row(BlockCdfSelector::YModeSet),
            TileCdfSelector::YModeIndex { ctx } => {
                $self.block.$block_row(BlockCdfSelector::YModeIndex { ctx })
            }
            TileCdfSelector::YModeOffset { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::YModeOffset { ctx }),
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            } => $self.block.$block_row(BlockCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            }),
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr } => $self
                .block
                .$block_row(BlockCdfSelector::IntraTxTypeSet1 { tx_size_sqr }),
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr } => $self
                .block
                .$block_row(BlockCdfSelector::IntraTxTypeSet2 { tx_size_sqr }),
            TileCdfSelector::IsLongSideDct { is_inter } => $self
                .block
                .$block_row(BlockCdfSelector::IsLongSideDct { is_inter }),
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr } => $self
                .block
                .$block_row(BlockCdfSelector::IntraTxTypeLong { tx_size_sqr }),
            TileCdfSelector::InterTxTypeLong { ctx, tx_size_sqr } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeLong { ctx, tx_size_sqr }),
            TileCdfSelector::InterTxTypeSet1 { ctx, tx_size_sqr } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeSet1 { ctx, tx_size_sqr }),
            TileCdfSelector::InterTxTypeSet2 { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeSet2 { ctx }),
            TileCdfSelector::InterTxTypeIndexSet1 { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeIndexSet1 { ctx }),
            TileCdfSelector::InterTxTypeIndexSet2 { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeIndexSet2 { ctx }),
            TileCdfSelector::InterTxTypeOffsetSet1 { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeOffsetSet1 { ctx }),
            TileCdfSelector::InterTxTypeOffsetSet2 { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeOffsetSet2 { ctx }),
            TileCdfSelector::InterTxTypeSet3 { ctx, tx_size_sqr } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeSet3 { ctx, tx_size_sqr }),
            TileCdfSelector::InterTxTypeSet4 { ctx, tx_size_sqr } => $self
                .block
                .$block_row(BlockCdfSelector::InterTxTypeSet4 { ctx, tx_size_sqr }),
            TileCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            } => $self.block.$block_row(BlockCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            }),
            TileCdfSelector::MostProbableStxSet => {
                $self.block.$block_row(BlockCdfSelector::MostProbableStxSet)
            }
            TileCdfSelector::MostProbableStxSetAdst => $self
                .block
                .$block_row(BlockCdfSelector::MostProbableStxSetAdst),
            TileCdfSelector::CctxType => $self.block.$block_row(BlockCdfSelector::CctxType),
            TileCdfSelector::PaletteYMode => $self.block.$block_row(BlockCdfSelector::PaletteYMode),
            TileCdfSelector::PaletteYSize => $self.block.$block_row(BlockCdfSelector::PaletteYSize),
            TileCdfSelector::IdentityRowY { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::IdentityRowY { ctx }),
            TileCdfSelector::PaletteYColorIndex { palette_size, ctx } => $self
                .block
                .$block_row(BlockCdfSelector::PaletteYColorIndex { palette_size, ctx }),
            TileCdfSelector::UvModeCflNotAllowed { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::UvModeCflNotAllowed { ctx }),
            TileCdfSelector::IsCfl { ctx } => {
                $self.block.$block_row(BlockCdfSelector::IsCfl { ctx })
            }
            TileCdfSelector::CflIndex => $self.block.$block_row(BlockCdfSelector::CflIndex),
            TileCdfSelector::CflSign => $self.block.$block_row(BlockCdfSelector::CflSign),
            TileCdfSelector::CflAlpha { ctx } => {
                $self.block.$block_row(BlockCdfSelector::CflAlpha { ctx })
            }
            TileCdfSelector::CflMhccp => $self.block.$block_row(BlockCdfSelector::CflMhccp),
            TileCdfSelector::CflMhDir { size_group } => $self
                .block
                .$block_row(BlockCdfSelector::CflMhDir { size_group }),
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => $self.block.$block_row(BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            }),
            TileCdfSelector::EobExtra { coeff_cdf_q_ctx } => $self
                .block
                .$block_row(BlockCdfSelector::EobExtra { coeff_cdf_q_ctx }),
            TileCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => $self.block.$block_row(BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            }),
            TileCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            } => $self.block.$block_row(BlockCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            }),
            TileCdfSelector::IsInter { ctx } => {
                $self.block.$block_row(BlockCdfSelector::IsInter { ctx })
            }
            TileCdfSelector::Skip { ctx } => $self.block.$block_row(BlockCdfSelector::Skip { ctx }),
            TileCdfSelector::SingleMode { ctx } => {
                $self.block.$block_row(BlockCdfSelector::SingleMode { ctx })
            }
            TileCdfSelector::IsWarp { ctx } => {
                $self.block.$block_row(BlockCdfSelector::IsWarp { ctx })
            }
            TileCdfSelector::WarpMv => $self.block.$block_row(BlockCdfSelector::WarpMv),
            TileCdfSelector::WarpIdx { ctx } => {
                $self.block.$block_row(BlockCdfSelector::WarpIdx { ctx })
            }
            TileCdfSelector::WarpWithMvd => $self.block.$block_row(BlockCdfSelector::WarpWithMvd),
            TileCdfSelector::WarpPrecision { block_size } => $self
                .block
                .$block_row(BlockCdfSelector::WarpPrecision { block_size }),
            TileCdfSelector::WarpDeltaParamLow { index_type } => $self
                .block
                .$block_row(BlockCdfSelector::WarpDeltaParamLow { index_type }),
            TileCdfSelector::WarpDeltaParamHigh { index_type } => $self
                .block
                .$block_row(BlockCdfSelector::WarpDeltaParamHigh { index_type }),
            TileCdfSelector::WarpDeltaParamSign => {
                $self.block.$block_row(BlockCdfSelector::WarpDeltaParamSign)
            }
            TileCdfSelector::WarpInterIntra { bsize_group } => $self
                .block
                .$block_row(BlockCdfSelector::WarpInterIntra { bsize_group }),
            TileCdfSelector::InterIntra { bsize_group } => $self
                .block
                .$block_row(BlockCdfSelector::InterIntra { bsize_group }),
            TileCdfSelector::InterIntraMode { bsize_group } => $self
                .block
                .$block_row(BlockCdfSelector::InterIntraMode { bsize_group }),
            TileCdfSelector::WedgeInterIntra => {
                $self.block.$block_row(BlockCdfSelector::WedgeInterIntra)
            }
            TileCdfSelector::WedgeQuad => $self.block.$block_row(BlockCdfSelector::WedgeQuad),
            TileCdfSelector::WedgeAngle { quad } => $self
                .block
                .$block_row(BlockCdfSelector::WedgeAngle { quad }),
            TileCdfSelector::WedgeDist1 => $self.block.$block_row(BlockCdfSelector::WedgeDist1),
            TileCdfSelector::WedgeDist2 => $self.block.$block_row(BlockCdfSelector::WedgeDist2),
            TileCdfSelector::DrlMode { idx, ctx } => $self
                .block
                .$block_row(BlockCdfSelector::DrlMode { idx, ctx }),
            TileCdfSelector::SingleRef { ctx, ref_idx } => $self
                .block
                .$block_row(BlockCdfSelector::SingleRef { ctx, ref_idx }),
            TileCdfSelector::CompMode { ctx } => {
                $self.block.$block_row(BlockCdfSelector::CompMode { ctx })
            }
            TileCdfSelector::IsJoint { ctx } => {
                $self.block.$block_row(BlockCdfSelector::IsJoint { ctx })
            }
            TileCdfSelector::CompoundModeNonJoint { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::CompoundModeNonJoint { ctx }),
            TileCdfSelector::CompGroupIdx { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::CompGroupIdx { ctx }),
            TileCdfSelector::CwpIdx { idx } => {
                $self.block.$block_row(BlockCdfSelector::CwpIdx { idx })
            }
            TileCdfSelector::CompRef0 { ctx, ref_idx } => $self
                .block
                .$block_row(BlockCdfSelector::CompRef0 { ctx, ref_idx }),
            TileCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => $self.block.$block_row(BlockCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            }),
            TileCdfSelector::ReadMv(selector) => {
                $self.block.$block_row(BlockCdfSelector::ReadMv(selector))
            }
            TileCdfSelector::InterpFilter { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::InterpFilter { ctx }),
            TileCdfSelector::UseAmvd { index, ctx } => $self
                .block
                .$block_row(BlockCdfSelector::UseAmvd { index, ctx }),
            TileCdfSelector::UseExtendWarp { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::UseExtendWarp { ctx }),
            TileCdfSelector::UseLocalWarp { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::UseLocalWarp { ctx }),
            TileCdfSelector::UseMostProbablePrecision { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::UseMostProbablePrecision { ctx }),
            TileCdfSelector::PbMvPrecision { ctx, frame_ctx } => $self
                .block
                .$block_row(BlockCdfSelector::PbMvPrecision { ctx, frame_ctx }),
            TileCdfSelector::UseBawp => $self.block.$block_row(BlockCdfSelector::UseBawp),
            TileCdfSelector::UseBawpChroma => {
                $self.block.$block_row(BlockCdfSelector::UseBawpChroma)
            }
            TileCdfSelector::ExplicitBawp { ctx } => $self
                .block
                .$block_row(BlockCdfSelector::ExplicitBawp { ctx }),
            TileCdfSelector::ExplicitBawpScale => {
                $self.block.$block_row(BlockCdfSelector::ExplicitBawpScale)
            }
            TileCdfSelector::UseWienerNs => $self.block.$block_row(BlockCdfSelector::UseWienerNs),
            TileCdfSelector::WienerNsLength { plane_ctx } => $self
                .block
                .$block_row(BlockCdfSelector::WienerNsLength { plane_ctx }),
            TileCdfSelector::WienerNsUvSym => {
                $self.block.$block_row(BlockCdfSelector::WienerNsUvSym)
            }
            TileCdfSelector::WienerNsBase => $self.block.$block_row(BlockCdfSelector::WienerNsBase),
            TileCdfSelector::Coeff(selector) => {
                $self.block.$block_row(BlockCdfSelector::Coeff(selector))
            }
        }
    };
}

fn tx_partition_rows(rows: &TileCdfRows, reduced: bool) -> &TxPartitionTypeCdfRows {
    if reduced {
        &rows.tx_partition_type_reduced
    } else {
        &rows.tx_partition_type
    }
}

fn tx_partition_rows_mut(rows: &mut TileCdfRows, reduced: bool) -> &mut TxPartitionTypeCdfRows {
    if reduced {
        &mut rows.tx_partition_type_reduced
    } else {
        &mut rows.tx_partition_type
    }
}

impl TileCdfRows {
    fn from_defaults() -> Self {
        Self {
            do_split: DEFAULT_DO_SPLIT_CDF,
            do_ext_partition: DEFAULT_DO_EXT_PARTITION_CDF,
            do_square_split: DEFAULT_DO_SQUARE_SPLIT_CDF,
            rect_type: DEFAULT_RECT_TYPE_CDF,
            do_uneven_4way_partition: DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF,
            tx_do_partition: DEFAULT_TX_DO_PARTITION_CDF,
            tx_2or3_partition_type: DEFAULT_TX_2OR3_PARTITION_TYPE_CDF,
            tx_partition_type: DEFAULT_TX_PARTITION_TYPE_CDF,
            tx_partition_type_reduced: DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF,
            delta_q: DEFAULT_DELTA_Q_CDF,
            cdef_index0: DEFAULT_CDEF_INDEX0_CDF,
            ccso_blk: DEFAULT_CCSO_BLK_CDF,
            cdef_index_minus1_with3: DEFAULT_CDEF_INDEX_MINUS1_WITH3_CDF,
            cdef_index_minus1_with4: DEFAULT_CDEF_INDEX_MINUS1_WITH4_CDF,
            cdef_index_minus1_with5: DEFAULT_CDEF_INDEX_MINUS1_WITH5_CDF,
            cdef_index_minus1_with6: DEFAULT_CDEF_INDEX_MINUS1_WITH6_CDF,
            cdef_index_minus1_with7: DEFAULT_CDEF_INDEX_MINUS1_WITH7_CDF,
            cdef_index_minus1_with8: DEFAULT_CDEF_INDEX_MINUS1_WITH8_CDF,
            intrabc: DEFAULT_INTRABC_CDF,
            intrabc_mode: DEFAULT_INTRABC_MODE_CDF,
            intrabc_precision: DEFAULT_INTRABC_PRECISION_CDF,
            morph_pred: DEFAULT_MORPH_PRED_CDF,
            fsc_mode: DEFAULT_FSC_MODE_CDF,
            mrl_index: DEFAULT_MRL_INDEX_CDF,
            mrl_sec_index: DEFAULT_MRL_SEC_INDEX_CDF,
            seg_id_ext_flag: DEFAULT_SEG_ID_EXT_FLAG_CDF,
            segment_id: DEFAULT_SEGMENT_ID_CDF,
            segment_id_ext: DEFAULT_SEGMENT_ID_EXT_CDF,
            region_type: DEFAULT_REGION_TYPE_CDF,
            block: BlockCdfRows::from_defaults(),
        }
    }

    fn row(&self, selector: TileCdfSelector) -> Result<&[i32], TileCdfError> {
        tile_cdf_row!(
            self,
            selector,
            get,
            as_slice,
            row,
            tx_partition_rows,
            cdef_index_minus1_row
        )
    }

    fn row_mut(&mut self, selector: TileCdfSelector) -> Result<&mut [i32], TileCdfError> {
        tile_cdf_row!(
            self,
            selector,
            get_mut,
            as_mut_slice,
            row_mut,
            tx_partition_rows_mut,
            cdef_index_minus1_row_mut
        )
    }

    fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        macro_rules! avg_row {
            ($field:ident) => {
                avg_cdf_row(&mut self.$field, &tile.$field, tile_num, num_log2);
            };
        }
        macro_rules! avg_rows {
            ($field:ident $(. $flatten:ident())*) => {
                avg_cdf_rows(
                    self.$field.iter_mut()$(.$flatten())*,
                    tile.$field.iter()$(.$flatten())*,
                    tile_num,
                    num_log2,
                );
            };
        }

        avg_rows!(do_split.flatten());
        avg_rows!(do_ext_partition.flatten());
        avg_rows!(do_square_split.flatten());
        avg_rows!(rect_type.flatten());
        avg_rows!(do_uneven_4way_partition.flatten());
        avg_rows!(tx_do_partition.flatten().flatten());
        avg_rows!(tx_2or3_partition_type.flatten().flatten());
        avg_rows!(tx_partition_type.flatten().flatten());
        avg_rows!(tx_partition_type_reduced.flatten().flatten());
        avg_row!(delta_q);
        avg_rows!(cdef_index0);
        avg_rows!(ccso_blk.flatten());
        avg_row!(cdef_index_minus1_with3);
        avg_row!(cdef_index_minus1_with4);
        avg_row!(cdef_index_minus1_with5);
        avg_row!(cdef_index_minus1_with6);
        avg_row!(cdef_index_minus1_with7);
        avg_row!(cdef_index_minus1_with8);
        avg_rows!(intrabc);
        avg_row!(intrabc_mode);
        avg_row!(intrabc_precision);
        avg_rows!(morph_pred);
        avg_rows!(fsc_mode.flatten());
        avg_rows!(mrl_index);
        avg_rows!(mrl_sec_index);
        avg_rows!(seg_id_ext_flag);
        avg_rows!(segment_id);
        avg_rows!(segment_id_ext);
        avg_rows!(region_type);
        self.block.avg_from_tile(tile_num, &tile.block, num_log2);
    }

    #[cfg(test)]
    pub(crate) const fn do_split(&self) -> &DoSplitCdfRows {
        &self.do_split
    }

    #[cfg(test)]
    pub(crate) const fn do_ext_partition(&self) -> &DoExtPartitionCdfRows {
        &self.do_ext_partition
    }

    #[cfg(test)]
    pub(crate) const fn do_square_split(&self) -> &DoSquareSplitCdfRows {
        &self.do_square_split
    }

    #[cfg(test)]
    pub(crate) const fn rect_type(&self) -> &RectTypeCdfRows {
        &self.rect_type
    }

    #[cfg(test)]
    pub(crate) const fn do_uneven_4way_partition(&self) -> &DoUneven4WayPartitionCdfRows {
        &self.do_uneven_4way_partition
    }

    #[cfg(test)]
    pub(crate) const fn tx_do_partition(&self) -> &TxDoPartitionCdfRows {
        &self.tx_do_partition
    }

    #[cfg(test)]
    pub(crate) const fn tx_2or3_partition_type(&self) -> &Tx2Or3PartitionTypeCdfRows {
        &self.tx_2or3_partition_type
    }

    #[cfg(test)]
    pub(crate) const fn tx_partition_type(&self) -> &TxPartitionTypeCdfRows {
        &self.tx_partition_type
    }

    #[cfg(test)]
    pub(crate) const fn tx_partition_type_reduced(&self) -> &TxPartitionTypeCdfRows {
        &self.tx_partition_type_reduced
    }

    #[cfg(test)]
    pub(crate) const fn delta_q(&self) -> &DeltaQCdfRow {
        &self.delta_q
    }

    #[cfg(test)]
    pub(crate) const fn intrabc_mode(&self) -> &IntrabcModeCdfRow {
        &self.intrabc_mode
    }

    #[cfg(test)]
    pub(crate) const fn intrabc_precision(&self) -> &IntrabcPrecisionCdfRow {
        &self.intrabc_precision
    }

    #[cfg(test)]
    pub(crate) const fn morph_pred(&self) -> &MorphPredCdfRows {
        &self.morph_pred
    }

    #[cfg(test)]
    pub(crate) const fn fsc_mode(&self) -> &FscModeCdfRows {
        &self.fsc_mode
    }

    #[cfg(test)]
    pub(crate) const fn mrl_index(&self) -> &MrlIndexCdfRows {
        &self.mrl_index
    }

    #[cfg(test)]
    pub(crate) const fn mrl_sec_index(&self) -> &MrlSecIndexCdfRows {
        &self.mrl_sec_index
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) const fn region_type(&self) -> &RegionTypeCdfRows {
        &self.region_type
    }

    #[cfg(test)]
    pub(crate) const fn y_mode_set(&self) -> &block_rows::YModeSetCdfRow {
        self.block.y_mode_set()
    }

    #[cfg(test)]
    pub(crate) const fn y_mode_index(&self) -> &block_rows::YModeIndexCdfRows {
        self.block.y_mode_index()
    }

    #[cfg(test)]
    pub(crate) const fn txb_skip(&self) -> &block_rows::TxbSkipCdfRows {
        self.block.txb_skip()
    }

    #[cfg(test)]
    pub(crate) const fn is_long_side_dct(&self) -> &block_rows::IsLongSideDctCdfRows {
        self.block.is_long_side_dct()
    }

    #[cfg(test)]
    pub(crate) const fn intra_tx_type_long(&self) -> &block_rows::IntraTxTypeLongCdfRows {
        self.block.intra_tx_type_long()
    }

    #[cfg(test)]
    pub(crate) const fn intra_tx_type_set1(&self) -> &block_rows::IntraTxTypeSet1CdfRows {
        self.block.intra_tx_type_set1()
    }

    #[cfg(test)]
    pub(crate) const fn intra_tx_type_set2(&self) -> &block_rows::IntraTxTypeSet2CdfRows {
        self.block.intra_tx_type_set2()
    }

    #[cfg(test)]
    pub(crate) const fn sec_tx_type(&self) -> &block_rows::SecTxTypeCdfRows {
        self.block.sec_tx_type()
    }

    #[cfg(test)]
    pub(crate) const fn most_probable_stx_set(&self) -> &block_rows::MostProbableStxSetCdfRow {
        self.block.most_probable_stx_set()
    }

    #[cfg(test)]
    pub(crate) const fn most_probable_stx_set_adst(
        &self,
    ) -> &block_rows::MostProbableStxSetAdstCdfRow {
        self.block.most_probable_stx_set_adst()
    }

    #[cfg(test)]
    pub(crate) const fn cctx_type(&self) -> &block_rows::CctxTypeCdfRow {
        self.block.cctx_type()
    }

    #[cfg(test)]
    pub(crate) const fn palette_y_mode(&self) -> &block_rows::PaletteYModeCdfRow {
        self.block.palette_y_mode()
    }

    #[cfg(test)]
    pub(crate) const fn uv_mode_cfl_not_allowed(&self) -> &block_rows::UvModeCflNotAllowedCdfRows {
        self.block.uv_mode_cfl_not_allowed()
    }

    #[cfg(test)]
    pub(crate) const fn is_cfl(&self) -> &block_rows::IsCflCdfRows {
        self.block.is_cfl()
    }

    #[cfg(test)]
    pub(crate) const fn cfl_index(&self) -> &block_rows::CflIndexCdfRow {
        self.block.cfl_index()
    }

    #[cfg(test)]
    pub(crate) const fn cfl_sign(&self) -> &block_rows::CflSignCdfRow {
        self.block.cfl_sign()
    }

    #[cfg(test)]
    pub(crate) const fn cfl_alpha(&self) -> &block_rows::CflAlphaCdfRows {
        self.block.cfl_alpha()
    }

    #[cfg(test)]
    pub(crate) const fn cfl_mhccp(&self) -> &block_rows::CflMhccpCdfRow {
        self.block.cfl_mhccp()
    }

    #[cfg(test)]
    pub(crate) const fn cfl_mh_dir(&self) -> &block_rows::CflMhDirCdfRows {
        self.block.cfl_mh_dir()
    }

    #[cfg(test)]
    pub(crate) const fn v_txb_skip(&self) -> &block_rows::VTxbSkipCdfRows {
        self.block.v_txb_skip()
    }

    #[cfg(test)]
    pub(crate) const fn eob_extra(&self) -> &block_rows::EobExtraCdfRows {
        self.block.eob_extra()
    }

    #[cfg(test)]
    pub(crate) const fn comp_mode(&self) -> &block_rows::CompModeCdfRows {
        self.block.comp_mode()
    }

    #[cfg(test)]
    pub(crate) const fn is_joint(&self) -> &block_rows::IsJointCdfRows {
        self.block.is_joint()
    }

    #[cfg(test)]
    pub(crate) const fn compound_mode_non_joint(&self) -> &block_rows::CompoundModeNonJointCdfRows {
        self.block.compound_mode_non_joint()
    }

    #[cfg(test)]
    pub(crate) const fn comp_group_idx(&self) -> &block_rows::CompGroupIdxCdfRows {
        self.block.comp_group_idx()
    }

    #[cfg(test)]
    pub(crate) const fn cwp_idx(&self) -> &block_rows::CwpIdxCdfRows {
        self.block.cwp_idx()
    }

    #[cfg(test)]
    pub(crate) const fn comp_ref0(&self) -> &block_rows::CompRef0CdfRows {
        self.block.comp_ref0()
    }

    #[cfg(test)]
    pub(crate) const fn comp_ref1(&self) -> &block_rows::CompRef1CdfRows {
        self.block.comp_ref1()
    }

    #[cfg(test)]
    pub(crate) const fn use_wiener_ns(&self) -> &block_rows::UseWienerNsCdfRow {
        self.block.use_wiener_ns()
    }

    #[cfg(test)]
    pub(crate) const fn wiener_ns_length(&self) -> &block_rows::WienerNsLengthCdfRows {
        self.block.wiener_ns_length()
    }

    #[cfg(test)]
    pub(crate) const fn wiener_ns_uv_sym(&self) -> &block_rows::WienerNsUvSymCdfRow {
        self.block.wiener_ns_uv_sym()
    }

    #[cfg(test)]
    pub(crate) const fn wiener_ns_base(&self) -> &block_rows::WienerNsBaseCdfRow {
        self.block.wiener_ns_base()
    }
}

fn cdef_index_minus1_row(rows: &TileCdfRows, strengths: usize) -> Result<&[i32], TileCdfError> {
    match strengths {
        3 => Ok(rows.cdef_index_minus1_with3.as_slice()),
        4 => Ok(rows.cdef_index_minus1_with4.as_slice()),
        5 => Ok(rows.cdef_index_minus1_with5.as_slice()),
        6 => Ok(rows.cdef_index_minus1_with6.as_slice()),
        7 => Ok(rows.cdef_index_minus1_with7.as_slice()),
        8 => Ok(rows.cdef_index_minus1_with8.as_slice()),
        actual => Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CdefIndexMinus1,
            index_name: "strengths",
            actual,
            max_exclusive: 9,
        }),
    }
}

fn cdef_index_minus1_row_mut(
    rows: &mut TileCdfRows,
    strengths: usize,
) -> Result<&mut [i32], TileCdfError> {
    match strengths {
        3 => Ok(rows.cdef_index_minus1_with3.as_mut_slice()),
        4 => Ok(rows.cdef_index_minus1_with4.as_mut_slice()),
        5 => Ok(rows.cdef_index_minus1_with5.as_mut_slice()),
        6 => Ok(rows.cdef_index_minus1_with6.as_mut_slice()),
        7 => Ok(rows.cdef_index_minus1_with7.as_mut_slice()),
        8 => Ok(rows.cdef_index_minus1_with8.as_mut_slice()),
        actual => Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CdefIndexMinus1,
            index_name: "strengths",
            actual,
            max_exclusive: 9,
        }),
    }
}

#[cfg(test)]
mod tests;
