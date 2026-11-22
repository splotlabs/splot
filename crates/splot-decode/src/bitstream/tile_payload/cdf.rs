// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile CDF selection and lifecycle boundaries.

#[macro_use]
mod layout;

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
use std::sync::Arc;

use splot_core::headers::frame::MAX_CDEF_STRENGTH_SETS;
use splot_core::symbol::CdfUpdateMode;
use splot_core::tables::cdf::DEFAULT_BRU_MODE_CDF;
use splot_core::tables::cdf::{
    DEFAULT_CCSO_BLK_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH3_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH4_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH5_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH6_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH7_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH8_CDF,
    DEFAULT_CDEF_INDEX0_CDF, DEFAULT_DELTA_Q_CDF, DEFAULT_DO_EXT_PARTITION_CDF,
    DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF, DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF,
    DEFAULT_FSC_MODE_CDF, DEFAULT_INTRABC_CDF, DEFAULT_INTRABC_MODE_CDF,
    DEFAULT_INTRABC_PRECISION_CDF, DEFAULT_LOSSLESS_INTER_TX_TYPE_CDF,
    DEFAULT_LOSSLESS_TX_SIZE_CDF, DEFAULT_MORPH_PRED_CDF, DEFAULT_MRL_INDEX_CDF,
    DEFAULT_MRL_SEC_INDEX_CDF, DEFAULT_RECT_TYPE_CDF, DEFAULT_REGION_TYPE_CDF,
    DEFAULT_SEG_ID_EXT_FLAG_CDF, DEFAULT_SEGMENT_ID_CDF, DEFAULT_SEGMENT_ID_EXT_CDF,
    DEFAULT_SEGMENT_ID_PREDICTED_CDF, DEFAULT_TX_2OR3_PARTITION_TYPE_CDF,
    DEFAULT_TX_DO_PARTITION_CDF, DEFAULT_TX_PARTITION_TYPE_CDF,
    DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF, DEFAULT_USE_GDF_CDF,
};

pub(crate) use self::block_rows::BlockCdfRows;
pub use self::block_rows::MvCdfSelector;
pub(crate) use self::block_rows::{
    COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN, COMPOUND_MODE_SAME_REFS_CDF_ROW_LEN, EobPtSize,
    PB_MV_PRECISION_CDF_ROW_LEN,
};
pub(crate) use self::coeff_rows::{CoeffBaseRow, CoeffCdfRows, CoeffCdfSelector, CoeffLevelRows};
#[cfg(any(debug_assertions, fuzzing, test))]
pub(in crate::bitstream::tile_payload::cdf) use self::util::cdf_store_valid;
pub(in crate::bitstream::tile_payload::cdf) use self::util::{
    avg_cdf_store, blend_cdf_store, scale_cdf_store,
};
use self::util::{
    checked_context, checked_plane, checked_square_split_plane, floor_log2, tx_partition_type_array,
};
pub(crate) const CDF_PROB_SCALE: u32 = 1 << 15;
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
const LOSSLESS_TX_SIZE_GROUPS: usize = 4;
const LOSSLESS_TX_SIZE_IS_INTER_CONTEXTS: usize = 2;
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
const CDEF_INDEX_MINUS1_WITH8_ROW_LEN: usize = MAX_CDEF_STRENGTH_SETS;
const INTRABC_CONTEXTS: usize = 3;
const MORPH_PRED_CONTEXTS: usize = 3;
const MRL_INDEX_CONTEXTS: usize = 3;
const MRL_INDEX_ROW_LEN: usize = 5;
const MRL_SEC_INDEX_ROW_LEN: usize = 3;
const SEGMENT_ID_CONTEXTS: usize = 3;
const SEGMENT_ID_ROW_LEN: usize = 9;
const SEG_ID_EXT_FLAG_ROW_LEN: usize = 3;
const INTER_SDP_BSIZE_GROUPS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffCdfQContext {
    Q0,
    Q1,
    Q2,
    Q3,
}

impl CoeffCdfQContext {
    pub(crate) const fn from_base_q_idx(base_q_idx: u32) -> Self {
        match base_q_idx {
            0..=90 => Self::Q0,
            91..=140 => Self::Q1,
            141..=190 => Self::Q2,
            _ => Self::Q3,
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Q0 => 0,
            Self::Q1 => 1,
            Self::Q2 => 2,
            Self::Q3 => 3,
        }
    }
}

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
    pub(super) num_log2: u8,
    pub(super) copy_cdf: bool,
    pub(super) avg_cdf: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrameCdfSubset {
    rows: Box<TileCdfRows>,
}

impl FrameCdfSubset {
    /// The default bank of `base_q_idx` 0.
    #[must_use]
    pub(crate) fn from_defaults() -> Self {
        Self::default_for_base_q(0)
    }

    /// The default bank of `base_q_idx`: one bank holds the coefficient
    /// rows of the q context the frame reads (§ 6.17.2 `init_coeff_cdfs`).
    pub(crate) fn default_for_base_q(base_q_idx: u32) -> Self {
        Self {
            rows: Box::new(*default_tile_cdf_rows(base_q_idx)),
        }
    }

    pub(crate) fn reset_from(&mut self, source: &Self) {
        self.rows.clone_from(&source.rows);
    }

    pub(crate) fn reset_to_defaults(&mut self, base_q_idx: u32) {
        *self.rows = *default_tile_cdf_rows(base_q_idx);
    }

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        self.rows.blend_from_saved(&saved.rows);
        #[cfg(any(debug_assertions, fuzzing))]
        assert!(
            self.rows.rows_valid(),
            "a blended CDF row left the § 8.2.6 invariants"
        );
    }

    #[must_use]
    pub(crate) fn tile_copy(&self) -> TileCdfSubset {
        TileCdfSubset {
            rows: self.rows.clone(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfSubset {
    rows: Box<TileCdfRows>,
}

impl TileCdfSubset {
    pub(crate) fn row_mut(
        &mut self,
        selector: TileCdfSelector,
    ) -> Result<&mut [u16], TileCdfError> {
        self.rows.row_mut(selector)
    }

    pub(crate) fn reset_from_frame(&mut self, frame: &FrameCdfSubset) {
        self.rows.clone_from(&frame.rows);
    }

    /// Copies every row of `other` in place.
    pub(crate) fn copy_from(&mut self, other: &Self) {
        self.rows.clone_from(&other.rows);
    }

    /// Averages every row as the tile-average update averages four tiles:
    /// three of them these rows and one `other`.
    pub(crate) fn blend_three_to_one(&mut self, other: &Self) {
        let own = self.rows.clone();
        for (tile_num, rows) in (0..).zip([&*own, &*own, &*own, &*other.rows]) {
            self.rows.avg_from_tile(tile_num, rows, 2);
        }
    }

    /// The address and `u16` length of the row table, so the encoder facade
    /// can key a row by its offset.
    pub(crate) fn rows_span(&self) -> (usize, usize) {
        (
            core::ptr::from_ref::<TileCdfRows>(&self.rows).addr(),
            size_of::<TileCdfRows>() / size_of::<u16>(),
        )
    }

    #[allow(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn with_row_mut<R>(
        &mut self,
        selector: TileCdfSelector,
        f: impl FnOnce(&mut [u16]) -> R,
    ) -> Result<R, TileCdfError> {
        Ok(f(self.rows.row_mut(selector)?))
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(test)]
pub(crate) struct SavedCdfSubset {
    rows: Box<TileCdfRows>,
}

#[cfg(test)]
impl SavedCdfSubset {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn from_frame(frame: &FrameCdfSubset) -> Self {
        Self {
            rows: frame.rows.clone(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfWorkUnitBoundary {
    update_mode: CdfUpdateMode,
    save_policy: TileCdfSavePolicy,
    frame_cdfs: Arc<FrameCdfSubset>,
    tile_cdfs: TileCdfSubset,
}

impl TileCdfWorkUnitBoundary {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn new(
        update_mode: CdfUpdateMode,
        save_policy: TileCdfSavePolicy,
        frame_cdfs: Arc<FrameCdfSubset>,
    ) -> Self {
        Self::with_storage(update_mode, save_policy, frame_cdfs, None)
    }

    pub(crate) fn with_storage(
        update_mode: CdfUpdateMode,
        save_policy: TileCdfSavePolicy,
        frame_cdfs: Arc<FrameCdfSubset>,
        mut tile_cdfs: Option<TileCdfSubset>,
    ) -> Self {
        if let Some(tile_cdfs) = tile_cdfs.as_mut() {
            tile_cdfs.reset_from_frame(&frame_cdfs);
        }
        let tile_cdfs = tile_cdfs.unwrap_or_else(|| frame_cdfs.tile_copy());
        Self {
            update_mode,
            save_policy,
            frame_cdfs,
            tile_cdfs,
        }
    }

    pub(crate) fn into_storage(self) -> TileCdfSubset {
        self.tile_cdfs
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

    pub(crate) fn frame_cdfs_shared(&self) -> Arc<FrameCdfSubset> {
        Arc::clone(&self.frame_cdfs)
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
    LosslessTxSize {
        size_group: usize,
        is_inter: usize,
    },
    LosslessInterTxType,
    DeltaQ,
    UseGdf,
    BruMode,
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
    SegmentIdPredicted {
        ctx: usize,
    },
    RegionType {
        ctx: usize,
    },
    UseDpcmY,
    DpcmModeY,
    UseDpcmUv,
    DpcmModeUv,
    YModeSet,
    YModeIndex {
        ctx: usize,
    },
    YModeOffset {
        ctx: usize,
    },
    TxbSkip {
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
    UseDip {
        ctx: usize,
    },
    DipMode,
    VTxbSkip {
        ctx: usize,
    },
    EobExtra,
    EobPt {
        size: EobPtSize,
        eob_ctx: usize,
    },
    DcSign {
        plane_type: usize,
        group: usize,
        ctx: usize,
    },
    IsInter {
        ctx: usize,
    },
    SkipMode {
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
    SkipDrlMode {
        idx: usize,
    },
    TipMode {
        ctx: usize,
    },
    TipPredMode,
    TipDrlMode {
        idx: usize,
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
    JmvdScaleMode,
    JmvdAdaptiveScaleMode,
    CompoundModeNonJoint {
        ctx: usize,
    },
    CompoundModeSameRefs {
        ctx: usize,
    },
    CompoundType,
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
    UseOptflow {
        ctx: usize,
    },
    UseRefinemv {
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
    UsePcWiener,
    FlexRestorationType {
        tool: usize,
        plane: usize,
    },
    WienerNsLength {
        plane_ctx: usize,
    },
    WienerNsUvSym,
    WienerNsBase,
    /// Decode reads coefficient rows directly; tests address them here.
    #[cfg(test)]
    Coeff(CoeffCdfSelector),
}

macro_rules! tile_cdf_arrays {
    ($($variant:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) enum TileCdfArray {
            $($variant),+
        }

        crate::impl_reason_labels!(TileCdfArray {
            $($variant => $label),+
        });
    };
}

tile_cdf_arrays! {
    DoSplit => "TileDoSplitCdf",
    DoExtPartition => "TileDoExtPartitionCdf",
    DoSquareSplit => "TileDoSquareSplitCdf",
    RectType => "TileRectTypeCdf",
    DoUneven4WayPartition => "TileDoUneven4wayPartitionCdf",
    TxDoPartition => "TileTxDoPartitionCdf",
    Tx2Or3PartitionType => "TileTx2or3PartitionTypeCdf",
    TxPartitionType => "TileTxPartitionTypeCdf",
    TxPartitionTypeReduced => "TileTxPartitionTypeReducedCdf",
    LosslessTxSize => "TileLosslessTxSizeCdf",
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
    SegmentIdPredicted => "TileSegmentIdPredictedCdf",
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
    UseDip => "TileUseDipCdf",
    VTxbSkip => "TileVTxbSkipCdf",
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
    SkipMode => "TileSkipModeCdf",
    Skip => "TileSkipCdf",
    SingleMode => "TileSingleModeCdf",
    IsWarp => "TileIsWarpCdf",
    WarpIdx => "TileWarpIdxCdf",
    WarpPrecision => "TileWarpPrecisionCdf",
    WarpDeltaParamLow => "TileWarpDeltaParamLowCdf",
    WarpDeltaParamHigh => "TileWarpDeltaParamHighCdf",
    WarpInterIntra => "TileWarpInterIntraCdf",
    InterIntra => "TileInterIntraCdf",
    InterIntraMode => "TileInterIntraModeCdf",
    WedgeAngle => "TileWedgeAngleCdf",
    DrlMode => "TileDrlModeCdf",
    SkipDrlMode => "TileSkipDrlModeCdf",
    TipMode => "TileTipModeCdf",
    TipDrlMode => "TileTipDrlModeCdf",
    SingleRef => "TileSingleRefCdf",
    CompMode => "TileCompModeCdf",
    IsJoint => "TileIsJointCdf",
    CompoundModeNonJoint => "TileCompoundModeNonJointCdf",
    CompoundModeSameRefs => "TileCompoundModeSameRefsCdf",
    CompGroupIdx => "TileCompGroupIdxCdf",
    CwpIdx => "TileCwpIdxCdf",
    CompRef0 => "TileCompRef0Cdf",
    CompRef1 => "TileCompRef1Cdf",
    UseAmvd => "TileUseAmvdCdf",
    UseOptflow => "TileUseOptflowCdf",
    UseRefinemv => "TileUseRefinemvCdf",
    UseExtendWarp => "TileUseExtendWarpCdf",
    UseLocalWarp => "TileUseLocalWarpCdf",
    UseMostProbablePrecision => "TileUseMostProbablePrecisionCdf",
    PbMvPrecision => "TilePbMvPrecisionCdf",
    ExplicitBawp => "TileExplicitBawpCdf",
    AmvdIndex => "TileAmvdIndexCdf",
    JointShell6Class => "TileJointShell6ClassCdf",
    ShellOffsetLowClass => "TileShellOffsetLowClassCdf",
    ShellOffsetOtherClass => "TileShellOffsetOtherClassCdf",
    ColMvGreater => "TileColMvGreaterCdf",
    ColMvIndex => "TileColMvIndexCdf",
    InterpFilter => "TileInterpFilterCdf",
    FlexRestorationType => "TileFlexRestorationTypeCdf",
    WienerNsLength => "TileWienerNsLengthCdf",
    IdentityRowY => "TileIdentityRowYCdf",
    PaletteYColorIndex => "TilePaletteYColorIndexCdf",
}
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
    #[cfg(test)]
    #[error("{table} bSize={b_size} is outside 0..{max_exclusive}")]
    BlockSizeOutOfRange {
        table: &'static str,
        b_size: usize,
        max_exclusive: usize,
    },
    #[cfg(test)]
    #[error("{array}[{plane_start}][{index}] is unavailable; length {len}")]
    PartitionNeighborOutOfRange {
        array: &'static str,
        plane_start: usize,
        index: usize,
        len: usize,
    },
    #[error("tile CDF selector did not map to a tile-local or block selector")]
    UnexpectedSelector,
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

cdf_rows! {
    /// `Copy` so that resetting a boxed table is one copy into place, never a
    /// 48 KB temporary on the stack.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct TileCdfRows[TileCdfElement](q: CoeffCdfQContext) {
        do_split[DO_SPLIT_PLANE_CONTEXTS][DO_SPLIT_CONTEXTS]: CDF_ROW_LEN = DEFAULT_DO_SPLIT_CDF,
        do_ext_partition[DO_SPLIT_PLANE_CONTEXTS][DO_EXT_PARTITION_CONTEXTS]: CDF_ROW_LEN = DEFAULT_DO_EXT_PARTITION_CDF,
        do_square_split[DO_SPLIT_PLANE_CONTEXTS][DO_SQUARE_SPLIT_CONTEXTS]: CDF_ROW_LEN = DEFAULT_DO_SQUARE_SPLIT_CDF,
        rect_type[DO_SPLIT_PLANE_CONTEXTS][RECT_TYPE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_RECT_TYPE_CDF,
        do_uneven_4way_partition[DO_SPLIT_PLANE_CONTEXTS][DO_UNEVEN_4WAY_PARTITION_CONTEXTS]: CDF_ROW_LEN = DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF,
        tx_do_partition[TX_FSC_CONTEXTS][TX_IS_INTER_CONTEXTS][TXFM_SPLIT_GROUPS]: CDF_ROW_LEN = DEFAULT_TX_DO_PARTITION_CDF,
        tx_2or3_partition_type[TX_FSC_CONTEXTS][TX_IS_INTER_CONTEXTS][TX_2OR3_PARTITION_TYPE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_TX_2OR3_PARTITION_TYPE_CDF,
        tx_partition_type[TX_FSC_CONTEXTS][TX_IS_INTER_CONTEXTS][TX_PARTITION_TYPE_CONTEXTS]: TX_PARTITION_TYPE_ROW_LEN = DEFAULT_TX_PARTITION_TYPE_CDF,
        tx_partition_type_reduced[TX_FSC_CONTEXTS][TX_IS_INTER_CONTEXTS][TX_PARTITION_TYPE_CONTEXTS]: TX_PARTITION_TYPE_ROW_LEN = DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF,
        lossless_tx_size[LOSSLESS_TX_SIZE_GROUPS][LOSSLESS_TX_SIZE_IS_INTER_CONTEXTS]: CDF_ROW_LEN = DEFAULT_LOSSLESS_TX_SIZE_CDF,
        lossless_inter_tx_type: CDF_ROW_LEN = DEFAULT_LOSSLESS_INTER_TX_TYPE_CDF,
        delta_q: DELTA_Q_CDF_ROW_LEN = DEFAULT_DELTA_Q_CDF,
        use_gdf: CDF_ROW_LEN = DEFAULT_USE_GDF_CDF,
        bru_mode: 4 = DEFAULT_BRU_MODE_CDF,
        cdef_index0[CDEF_STRENGTH_INDEX0_CONTEXTS]: CDF_ROW_LEN = DEFAULT_CDEF_INDEX0_CDF,
        ccso_blk[CCSO_PLANES][CCSO_CONTEXTS]: CDF_ROW_LEN = DEFAULT_CCSO_BLK_CDF,
        cdef_index_minus1_with3: CDEF_INDEX_MINUS1_WITH3_ROW_LEN = DEFAULT_CDEF_INDEX_MINUS1_WITH3_CDF,
        cdef_index_minus1_with4: CDEF_INDEX_MINUS1_WITH4_ROW_LEN = DEFAULT_CDEF_INDEX_MINUS1_WITH4_CDF,
        cdef_index_minus1_with5: CDEF_INDEX_MINUS1_WITH5_ROW_LEN = DEFAULT_CDEF_INDEX_MINUS1_WITH5_CDF,
        cdef_index_minus1_with6: CDEF_INDEX_MINUS1_WITH6_ROW_LEN = DEFAULT_CDEF_INDEX_MINUS1_WITH6_CDF,
        cdef_index_minus1_with7: CDEF_INDEX_MINUS1_WITH7_ROW_LEN = DEFAULT_CDEF_INDEX_MINUS1_WITH7_CDF,
        cdef_index_minus1_with8: CDEF_INDEX_MINUS1_WITH8_ROW_LEN = DEFAULT_CDEF_INDEX_MINUS1_WITH8_CDF,
        intrabc[INTRABC_CONTEXTS]: CDF_ROW_LEN = DEFAULT_INTRABC_CDF,
        intrabc_mode: CDF_ROW_LEN = DEFAULT_INTRABC_MODE_CDF,
        intrabc_precision: CDF_ROW_LEN = DEFAULT_INTRABC_PRECISION_CDF,
        morph_pred[MORPH_PRED_CONTEXTS]: CDF_ROW_LEN = DEFAULT_MORPH_PRED_CDF,
        fsc_mode[FSC_MODE_CONTEXTS][FSC_BSIZE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_FSC_MODE_CDF,
        mrl_index[MRL_INDEX_CONTEXTS]: MRL_INDEX_ROW_LEN = DEFAULT_MRL_INDEX_CDF,
        mrl_sec_index[MRL_INDEX_CONTEXTS]: MRL_SEC_INDEX_ROW_LEN = DEFAULT_MRL_SEC_INDEX_CDF,
        seg_id_ext_flag[SEGMENT_ID_CONTEXTS]: SEG_ID_EXT_FLAG_ROW_LEN = DEFAULT_SEG_ID_EXT_FLAG_CDF,
        segment_id[SEGMENT_ID_CONTEXTS]: SEGMENT_ID_ROW_LEN = DEFAULT_SEGMENT_ID_CDF,
        segment_id_ext[SEGMENT_ID_CONTEXTS]: SEGMENT_ID_ROW_LEN = DEFAULT_SEGMENT_ID_EXT_CDF,
        segment_id_predicted[SEGMENT_ID_CONTEXTS]: CDF_ROW_LEN = DEFAULT_SEGMENT_ID_PREDICTED_CDF,
        region_type[INTER_SDP_BSIZE_GROUPS]: CDF_ROW_LEN = DEFAULT_REGION_TYPE_CDF,
        ..
        block: BlockCdfRows = BlockCdfRows::from_defaults(q),
    }
}

macro_rules! selected_cdf_row {
    ($rows:expr, $index:expr, $array:expr, $index_name:literal) => {{
        let max_exclusive = $rows.len();
        let row = $rows
            .get_mut($index)
            .ok_or(TileCdfError::SelectorOutOfRange {
                array: $array,
                index_name: $index_name,
                actual: $index,
                max_exclusive,
            })?;
        Ok(row.as_mut_slice())
    }};
}

macro_rules! partition_cdf_row {
    ($self:expr, $field:ident, $array:expr, $plane:expr, $ctx:expr) => {{
        let plane = $plane;
        selected_cdf_row!($self.$field()[plane], $ctx, $array, "ctx")
    }};
}

type TxPartitionTypeCdfRows = [[[[u16; TX_PARTITION_TYPE_ROW_LEN]; TX_PARTITION_TYPE_CONTEXTS];
    TX_IS_INTER_CONTEXTS]; TX_FSC_CONTEXTS];

fn tx_partition_rows_mut(rows: &mut TileCdfRows, reduced: bool) -> &mut TxPartitionTypeCdfRows {
    if reduced {
        rows.tx_partition_type_reduced()
    } else {
        rows.tx_partition_type()
    }
}

static DEFAULT_TILE_CDF_ROWS: [TileCdfRows; 4] = [
    TileCdfRows::from_defaults(CoeffCdfQContext::Q0),
    TileCdfRows::from_defaults(CoeffCdfQContext::Q1),
    TileCdfRows::from_defaults(CoeffCdfQContext::Q2),
    TileCdfRows::from_defaults(CoeffCdfQContext::Q3),
];

fn default_tile_cdf_rows(base_q_idx: u32) -> &'static TileCdfRows {
    &DEFAULT_TILE_CDF_ROWS[CoeffCdfQContext::from_base_q_idx(base_q_idx).index()]
}

impl TileCdfRows {
    #[allow(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    fn row_mut(&mut self, selector: TileCdfSelector) -> Result<&mut [u16], TileCdfError> {
        match selector {
            TileCdfSelector::DoSplit { plane_start, ctx } => {
                partition_cdf_row!(
                    self,
                    do_split,
                    TileCdfArray::DoSplit,
                    checked_plane(TileCdfArray::DoSplit, plane_start)?,
                    ctx
                )
            }
            TileCdfSelector::DoExtPartition { plane_start, ctx } => {
                partition_cdf_row!(
                    self,
                    do_ext_partition,
                    TileCdfArray::DoExtPartition,
                    checked_plane(TileCdfArray::DoExtPartition, plane_start)?,
                    ctx
                )
            }
            TileCdfSelector::DoSquareSplit { plane_start, ctx } => {
                partition_cdf_row!(
                    self,
                    do_square_split,
                    TileCdfArray::DoSquareSplit,
                    checked_square_split_plane(plane_start)?,
                    ctx
                )
            }
            TileCdfSelector::RectType { plane_start, ctx } => {
                partition_cdf_row!(
                    self,
                    rect_type,
                    TileCdfArray::RectType,
                    checked_plane(TileCdfArray::RectType, plane_start)?,
                    ctx
                )
            }
            TileCdfSelector::DoUneven4WayPartition { plane_start, ctx } => {
                partition_cdf_row!(
                    self,
                    do_uneven_4way_partition,
                    TileCdfArray::DoUneven4WayPartition,
                    checked_plane(TileCdfArray::DoUneven4WayPartition, plane_start)?,
                    ctx
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
                    self.tx_do_partition()[fsc_mode][is_inter],
                    txfm_split_group,
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
                    self.tx_2or3_partition_type()[fsc_mode][is_inter],
                    ctx,
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
                let rows = tx_partition_rows_mut(self, reduced);
                selected_cdf_row!(rows[fsc_mode][is_inter], ctx, array, "ctx")
            }
            TileCdfSelector::LosslessTxSize {
                size_group,
                is_inter,
            } => {
                let max_exclusive = self.lossless_tx_size().len();
                let group = self.lossless_tx_size().get_mut(size_group).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::LosslessTxSize,
                        index_name: "size_group",
                        actual: size_group,
                        max_exclusive,
                    },
                )?;
                let max_exclusive = group.len();
                let row = group
                    .get_mut(is_inter)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::LosslessTxSize,
                        index_name: "is_inter",
                        actual: is_inter,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::LosslessInterTxType => {
                Ok(self.lossless_inter_tx_type().as_mut_slice())
            }
            TileCdfSelector::DeltaQ => Ok(self.delta_q().as_mut_slice()),
            TileCdfSelector::UseGdf => Ok(self.use_gdf().as_mut_slice()),
            TileCdfSelector::BruMode => Ok(self.bru_mode().as_mut_slice()),
            TileCdfSelector::CdefIndex0 { ctx } => {
                selected_cdf_row!(self.cdef_index0(), ctx, TileCdfArray::CdefIndex0, "ctx")
            }
            TileCdfSelector::CcsoBlk { plane, ctx } => {
                let max_exclusive = self.ccso_blk().len();
                let plane_rows =
                    self.ccso_blk()
                        .get_mut(plane)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::CcsoBlk,
                            index_name: "plane",
                            actual: plane,
                            max_exclusive,
                        })?;
                let max_exclusive = plane_rows.len();
                let row = plane_rows
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CcsoBlk,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::CdefIndexMinus1 { strengths } => {
                cdef_index_minus1_row_mut(self, strengths)
            }
            TileCdfSelector::Intrabc { ctx } => {
                selected_cdf_row!(self.intrabc(), ctx, TileCdfArray::Intrabc, "ctx")
            }
            TileCdfSelector::IntrabcMode => Ok(self.intrabc_mode().as_mut_slice()),
            TileCdfSelector::IntrabcPrecision => Ok(self.intrabc_precision().as_mut_slice()),
            TileCdfSelector::MorphPred { ctx } => {
                selected_cdf_row!(self.morph_pred(), ctx, TileCdfArray::MorphPred, "ctx")
            }
            TileCdfSelector::FscMode { ctx, bsize_group } => {
                let ctx = checked_context(TileCdfArray::FscMode, "ctx", ctx, FSC_MODE_CONTEXTS)?;
                selected_cdf_row!(
                    self.fsc_mode()[ctx],
                    bsize_group,
                    TileCdfArray::FscMode,
                    "bsize_group"
                )
            }
            TileCdfSelector::MrlIndex { ctx } => {
                let ctx = checked_context(TileCdfArray::MrlIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok(self.mrl_index()[ctx].as_mut_slice())
            }
            TileCdfSelector::MrlSecIndex { ctx } => {
                let ctx =
                    checked_context(TileCdfArray::MrlSecIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok(self.mrl_sec_index()[ctx].as_mut_slice())
            }
            TileCdfSelector::SegIdExtFlag { ctx } => {
                let ctx =
                    checked_context(TileCdfArray::SegIdExtFlag, "ctx", ctx, SEGMENT_ID_CONTEXTS)?;
                Ok(self.seg_id_ext_flag()[ctx].as_mut_slice())
            }
            TileCdfSelector::SegmentId { ctx, ext } => {
                let ctx =
                    checked_context(TileCdfArray::SegmentId, "ctx", ctx, SEGMENT_ID_CONTEXTS)?;
                if ext {
                    Ok(self.segment_id_ext()[ctx].as_mut_slice())
                } else {
                    Ok(self.segment_id()[ctx].as_mut_slice())
                }
            }
            TileCdfSelector::SegmentIdPredicted { ctx } => {
                let ctx = checked_context(
                    TileCdfArray::SegmentIdPredicted,
                    "ctx",
                    ctx,
                    SEGMENT_ID_CONTEXTS,
                )?;
                Ok(self.segment_id_predicted()[ctx].as_mut_slice())
            }
            TileCdfSelector::RegionType { ctx } => {
                selected_cdf_row!(self.region_type(), ctx, TileCdfArray::RegionType, "ctx")
            }
            selector => self.block.row_mut(selector),
        }
    }

    fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        avg_cdf_store(
            &mut self.store,
            &tile.store,
            &Self::BOUNDS,
            tile_num,
            num_log2,
        );
        self.block.avg_from_tile(tile_num, &tile.block, num_log2);
    }

    fn blend_from_saved(&mut self, saved: &Self) {
        blend_cdf_store(&mut self.store, &saved.store, &Self::BOUNDS);
        self.block.blend_from_saved(&saved.block);
    }

    fn scale_counts_for_frame_end_update(&mut self) {
        scale_cdf_store(&mut self.store, &Self::BOUNDS);
        self.block.scale_counts_for_frame_end_update();
    }

    #[cfg(any(debug_assertions, fuzzing, test))]
    pub(crate) fn rows_valid(&self) -> bool {
        cdf_store_valid(&self.store, &Self::BOUNDS) && self.block.rows_valid()
    }
}

fn cdef_index_minus1_row_mut(
    rows: &mut TileCdfRows,
    strengths: usize,
) -> Result<&mut [u16], TileCdfError> {
    match strengths {
        3 => Ok(rows.cdef_index_minus1_with3().as_mut_slice()),
        4 => Ok(rows.cdef_index_minus1_with4().as_mut_slice()),
        5 => Ok(rows.cdef_index_minus1_with5().as_mut_slice()),
        6 => Ok(rows.cdef_index_minus1_with6().as_mut_slice()),
        7 => Ok(rows.cdef_index_minus1_with7().as_mut_slice()),
        MAX_CDEF_STRENGTH_SETS => Ok(rows.cdef_index_minus1_with8().as_mut_slice()),
        actual => Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CdefIndexMinus1,
            index_name: "strengths",
            actual,
            max_exclusive: MAX_CDEF_STRENGTH_SETS + 1,
        }),
    }
}

#[cfg(test)]
mod tests;
