// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile CDF selection and lifecycle boundaries.

/// Reinterprets a nested CDF-row array as one contiguous slice of rows; each
/// trailing `flatten` token peels one nesting level, mirroring the iterator
/// `.flatten()` chains these lifecycle macros previously built.
macro_rules! flat_cdf_rows {
    ($e:expr) => { &$e[..] };
    ($e:expr, $f:ident $(, $rest:ident)*) => { flat_cdf_rows!($e.as_flattened() $(, $rest)*) };
}
macro_rules! flat_cdf_rows_mut {
    ($e:expr) => { &mut $e[..] };
    ($e:expr, $f:ident $(, $rest:ident)*) => { flat_cdf_rows_mut!($e.as_flattened_mut() $(, $rest)*) };
}

macro_rules! tile_cdf_common_count_rows {
    ($row:ident, $rows:ident) => {
        $rows!(do_split.flatten());
        $rows!(do_ext_partition.flatten());
        $rows!(do_square_split.flatten());
        $rows!(rect_type.flatten());
        $rows!(do_uneven_4way_partition.flatten());
        $rows!(tx_do_partition.flatten().flatten());
        $rows!(tx_2or3_partition_type.flatten().flatten());
        $rows!(tx_partition_type.flatten().flatten());
        $rows!(tx_partition_type_reduced.flatten().flatten());
        $rows!(lossless_tx_size.flatten());
        $row!(lossless_inter_tx_type);
        $row!(delta_q);
        $row!(use_gdf);
        $rows!(cdef_index0);
        $rows!(ccso_blk.flatten());
        $row!(cdef_index_minus1_with3);
        $row!(cdef_index_minus1_with4);
        $row!(cdef_index_minus1_with5);
        $row!(cdef_index_minus1_with6);
        $row!(cdef_index_minus1_with7);
        $row!(cdef_index_minus1_with8);
        $rows!(intrabc);
        $row!(intrabc_mode);
        $row!(intrabc_precision);
        $rows!(morph_pred);
        $rows!(fsc_mode.flatten());
        $rows!(mrl_index);
        $rows!(mrl_sec_index);
        $rows!(region_type);
    };
}

macro_rules! tile_cdf_saved_rows {
    ($row:ident, $rows:ident) => {
        tile_cdf_common_count_rows!($row, $rows);
        $rows!(seg_id_ext_flag);
        $rows!(segment_id);
        $rows!(segment_id_ext);
        $rows!(segment_id_predicted);
    };
}

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

use self::block_rows::BlockCdfRows;
pub(crate) use self::block_rows::{
    COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN, COMPOUND_MODE_SAME_REFS_CDF_ROW_LEN, EobPtSize,
    MvCdfSelector,
};
pub(crate) use self::coeff_rows::CoeffCdfSelector;
pub(in crate::bitstream::tile_payload::cdf) use self::util::{
    avg_cdf_row, avg_cdf_rows, blend_cdf_row, blend_cdf_rows, scale_cdf_count, scale_cdf_rows,
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

pub(crate) const fn coeff_cdf_q_ctx_from_base_q_idx(base_q_idx: u32) -> usize {
    CoeffCdfQContext::from_base_q_idx(base_q_idx).index()
}

type DoSplitCdfRows = [[[u16; CDF_ROW_LEN]; DO_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoExtPartitionCdfRows =
    [[[u16; CDF_ROW_LEN]; DO_EXT_PARTITION_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoSquareSplitCdfRows =
    [[[u16; CDF_ROW_LEN]; DO_SQUARE_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoUneven4WayPartitionCdfRows =
    [[[u16; CDF_ROW_LEN]; DO_UNEVEN_4WAY_PARTITION_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type RectTypeCdfRows = [[[u16; CDF_ROW_LEN]; RECT_TYPE_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type TxDoPartitionCdfRows =
    [[[[u16; CDF_ROW_LEN]; TXFM_SPLIT_GROUPS]; TX_IS_INTER_CONTEXTS]; TX_FSC_CONTEXTS];
type Tx2Or3PartitionTypeCdfRows = [[[[u16; CDF_ROW_LEN]; TX_2OR3_PARTITION_TYPE_CONTEXTS];
    TX_IS_INTER_CONTEXTS]; TX_FSC_CONTEXTS];
type TxPartitionTypeCdfRows = [[[[u16; TX_PARTITION_TYPE_ROW_LEN]; TX_PARTITION_TYPE_CONTEXTS];
    TX_IS_INTER_CONTEXTS]; TX_FSC_CONTEXTS];
type LosslessTxSizeCdfRows =
    [[[u16; CDF_ROW_LEN]; LOSSLESS_TX_SIZE_IS_INTER_CONTEXTS]; LOSSLESS_TX_SIZE_GROUPS];
type LosslessInterTxTypeCdfRow = [u16; CDF_ROW_LEN];
type DeltaQCdfRow = [u16; DELTA_Q_CDF_ROW_LEN];
type FscModeCdfRows = [[[u16; CDF_ROW_LEN]; FSC_BSIZE_CONTEXTS]; FSC_MODE_CONTEXTS];
type CdefIndex0CdfRows = [[u16; CDF_ROW_LEN]; CDEF_STRENGTH_INDEX0_CONTEXTS];
type CcsoBlkCdfRows = [[[u16; CDF_ROW_LEN]; CCSO_CONTEXTS]; CCSO_PLANES];
type CdefIndexMinus1With3CdfRow = [u16; CDEF_INDEX_MINUS1_WITH3_ROW_LEN];
type CdefIndexMinus1With4CdfRow = [u16; CDEF_INDEX_MINUS1_WITH4_ROW_LEN];
type CdefIndexMinus1With5CdfRow = [u16; CDEF_INDEX_MINUS1_WITH5_ROW_LEN];
type CdefIndexMinus1With6CdfRow = [u16; CDEF_INDEX_MINUS1_WITH6_ROW_LEN];
type CdefIndexMinus1With7CdfRow = [u16; CDEF_INDEX_MINUS1_WITH7_ROW_LEN];
type CdefIndexMinus1With8CdfRow = [u16; CDEF_INDEX_MINUS1_WITH8_ROW_LEN];
type IntrabcCdfRows = [[u16; CDF_ROW_LEN]; INTRABC_CONTEXTS];
type IntrabcModeCdfRow = [u16; CDF_ROW_LEN];
type IntrabcPrecisionCdfRow = [u16; CDF_ROW_LEN];
type MorphPredCdfRows = [[u16; CDF_ROW_LEN]; MORPH_PRED_CONTEXTS];
type MrlIndexCdfRows = [[u16; MRL_INDEX_ROW_LEN]; MRL_INDEX_CONTEXTS];
type MrlSecIndexCdfRows = [[u16; MRL_SEC_INDEX_ROW_LEN]; MRL_INDEX_CONTEXTS];
type SegmentIdCdfRows = [[u16; SEGMENT_ID_ROW_LEN]; SEGMENT_ID_CONTEXTS];
type SegIdExtFlagCdfRows = [[u16; SEG_ID_EXT_FLAG_ROW_LEN]; SEGMENT_ID_CONTEXTS];
type SegmentIdPredictedCdfRows = [[u16; CDF_ROW_LEN]; SEGMENT_ID_CONTEXTS];
type RegionTypeCdfRows = [[u16; CDF_ROW_LEN]; INTER_SDP_BSIZE_GROUPS];
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
    #[must_use]
    pub(crate) fn from_defaults() -> Self {
        Self {
            rows: Box::new(TileCdfRows::from_defaults()),
        }
    }

    pub(crate) fn default_for_base_q(base_q_idx: u32) -> Self {
        let mut cdfs = Self::from_defaults();
        cdfs.rows
            .block
            .replicate_bounded_coeff_q_context(CoeffCdfQContext::from_base_q_idx(base_q_idx));
        cdfs
    }

    pub(crate) fn replicate_coeff_q_context_for_base_q(&mut self, base_q_idx: u32) {
        self.rows
            .block
            .replicate_bounded_coeff_q_context(CoeffCdfQContext::from_base_q_idx(base_q_idx));
    }

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        self.rows.blend_from_saved(&saved.rows);
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
    #[inline]
    pub(crate) fn with_row_mut<R>(
        &mut self,
        selector: TileCdfSelector,
        f: impl FnOnce(&mut [u16]) -> R,
    ) -> Result<R, TileCdfError> {
        Ok(f(self.rows.row_mut(selector)?))
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SavedCdfSubset {
    rows: Box<TileCdfRows>,
}

impl SavedCdfSubset {
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
    #[must_use]
    pub(crate) fn new(
        update_mode: CdfUpdateMode,
        save_policy: TileCdfSavePolicy,
        frame_cdfs: Arc<FrameCdfSubset>,
    ) -> Self {
        let tile_cdfs = frame_cdfs.tile_copy();
        Self {
            update_mode,
            save_policy,
            frame_cdfs,
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
    UseDip {
        ctx: usize,
    },
    DipMode,
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
    lossless_tx_size: LosslessTxSizeCdfRows,
    lossless_inter_tx_type: LosslessInterTxTypeCdfRow,
    delta_q: DeltaQCdfRow,
    use_gdf: [u16; CDF_ROW_LEN],
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
    segment_id_predicted: SegmentIdPredictedCdfRows,
    region_type: RegionTypeCdfRows,
    block: BlockCdfRows,
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
        selected_cdf_row!($self.$field[plane], $ctx, $array, "ctx")
    }};
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
            lossless_tx_size: DEFAULT_LOSSLESS_TX_SIZE_CDF,
            lossless_inter_tx_type: DEFAULT_LOSSLESS_INTER_TX_TYPE_CDF,
            delta_q: DEFAULT_DELTA_Q_CDF,
            use_gdf: DEFAULT_USE_GDF_CDF,
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
            segment_id_predicted: DEFAULT_SEGMENT_ID_PREDICTED_CDF,
            region_type: DEFAULT_REGION_TYPE_CDF,
            block: BlockCdfRows::from_defaults(),
        }
    }

    #[inline]
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
                    self.tx_do_partition[fsc_mode][is_inter],
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
                    self.tx_2or3_partition_type[fsc_mode][is_inter],
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
                let max_exclusive = self.lossless_tx_size.len();
                let group = self.lossless_tx_size.get_mut(size_group).ok_or(
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
            TileCdfSelector::LosslessInterTxType => Ok(self.lossless_inter_tx_type.as_mut_slice()),
            TileCdfSelector::DeltaQ => Ok(self.delta_q.as_mut_slice()),
            TileCdfSelector::UseGdf => Ok(self.use_gdf.as_mut_slice()),
            TileCdfSelector::CdefIndex0 { ctx } => {
                selected_cdf_row!(self.cdef_index0, ctx, TileCdfArray::CdefIndex0, "ctx")
            }
            TileCdfSelector::CcsoBlk { plane, ctx } => {
                let max_exclusive = self.ccso_blk.len();
                let plane_rows =
                    self.ccso_blk
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
                selected_cdf_row!(self.intrabc, ctx, TileCdfArray::Intrabc, "ctx")
            }
            TileCdfSelector::IntrabcMode => Ok(self.intrabc_mode.as_mut_slice()),
            TileCdfSelector::IntrabcPrecision => Ok(self.intrabc_precision.as_mut_slice()),
            TileCdfSelector::MorphPred { ctx } => {
                selected_cdf_row!(self.morph_pred, ctx, TileCdfArray::MorphPred, "ctx")
            }
            TileCdfSelector::FscMode { ctx, bsize_group } => {
                let ctx = checked_context(TileCdfArray::FscMode, "ctx", ctx, FSC_MODE_CONTEXTS)?;
                selected_cdf_row!(
                    self.fsc_mode[ctx],
                    bsize_group,
                    TileCdfArray::FscMode,
                    "bsize_group"
                )
            }
            TileCdfSelector::MrlIndex { ctx } => {
                let ctx = checked_context(TileCdfArray::MrlIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok(self.mrl_index[ctx].as_mut_slice())
            }
            TileCdfSelector::MrlSecIndex { ctx } => {
                let ctx =
                    checked_context(TileCdfArray::MrlSecIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok(self.mrl_sec_index[ctx].as_mut_slice())
            }
            TileCdfSelector::SegIdExtFlag { ctx } => {
                let ctx =
                    checked_context(TileCdfArray::SegIdExtFlag, "ctx", ctx, SEGMENT_ID_CONTEXTS)?;
                Ok(self.seg_id_ext_flag[ctx].as_mut_slice())
            }
            TileCdfSelector::SegmentId { ctx, ext } => {
                let ctx =
                    checked_context(TileCdfArray::SegmentId, "ctx", ctx, SEGMENT_ID_CONTEXTS)?;
                if ext {
                    Ok(self.segment_id_ext[ctx].as_mut_slice())
                } else {
                    Ok(self.segment_id[ctx].as_mut_slice())
                }
            }
            TileCdfSelector::SegmentIdPredicted { ctx } => {
                let ctx = checked_context(
                    TileCdfArray::SegmentIdPredicted,
                    "ctx",
                    ctx,
                    SEGMENT_ID_CONTEXTS,
                )?;
                Ok(self.segment_id_predicted[ctx].as_mut_slice())
            }
            TileCdfSelector::RegionType { ctx } => {
                selected_cdf_row!(self.region_type, ctx, TileCdfArray::RegionType, "ctx")
            }
            selector => self.block.row_mut(selector),
        }
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
                    flat_cdf_rows_mut!(self.$field $(, $flatten)*),
                    flat_cdf_rows!(tile.$field $(, $flatten)*),
                    tile_num,
                    num_log2,
                );
            };
        }

        tile_cdf_saved_rows!(avg_row, avg_rows);
        self.block.avg_from_tile(tile_num, &tile.block, num_log2);
    }

    fn blend_from_saved(&mut self, saved: &Self) {
        macro_rules! blend_row {
            ($field:ident) => {
                blend_cdf_row(&mut self.$field, &saved.$field);
            };
        }
        macro_rules! blend_rows {
            ($field:ident $(. $flatten:ident())*) => {
                blend_cdf_rows(
                    flat_cdf_rows_mut!(self.$field $(, $flatten)*),
                    flat_cdf_rows!(saved.$field $(, $flatten)*),
                );
            };
        }

        tile_cdf_saved_rows!(blend_row, blend_rows);
        self.block.blend_from_saved(&saved.block);
    }
}

fn cdef_index_minus1_row_mut(
    rows: &mut TileCdfRows,
    strengths: usize,
) -> Result<&mut [u16], TileCdfError> {
    match strengths {
        3 => Ok(rows.cdef_index_minus1_with3.as_mut_slice()),
        4 => Ok(rows.cdef_index_minus1_with4.as_mut_slice()),
        5 => Ok(rows.cdef_index_minus1_with5.as_mut_slice()),
        6 => Ok(rows.cdef_index_minus1_with6.as_mut_slice()),
        7 => Ok(rows.cdef_index_minus1_with7.as_mut_slice()),
        MAX_CDEF_STRENGTH_SETS => Ok(rows.cdef_index_minus1_with8.as_mut_slice()),
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
