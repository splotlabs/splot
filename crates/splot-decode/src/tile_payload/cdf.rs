// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile CDF selection and lifecycle boundaries.
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY` and
//! `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.

pub(crate) mod block_context;
pub(crate) mod block_read;
mod block_rows;
pub(crate) mod coeff_context;
mod coeff_rows;
pub(crate) mod context;
mod lifecycle;
pub(crate) mod partition_read;

use core::fmt;

use splot_core::symbol::CdfUpdateMode;
use splot_core::tables::cdf::{
    DEFAULT_CCSO_BLK_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH3_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH4_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH5_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH6_CDF,
    DEFAULT_CDEF_INDEX_MINUS1_WITH7_CDF, DEFAULT_CDEF_INDEX_MINUS1_WITH8_CDF,
    DEFAULT_CDEF_INDEX0_CDF, DEFAULT_DELTA_Q_CDF, DEFAULT_DO_EXT_PARTITION_CDF,
    DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF, DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF,
    DEFAULT_FSC_MODE_CDF, DEFAULT_INTRABC_CDF, DEFAULT_INTRABC_MODE_CDF,
    DEFAULT_INTRABC_PRECISION_CDF, DEFAULT_MRL_INDEX_CDF, DEFAULT_MRL_SEC_INDEX_CDF,
    DEFAULT_RECT_TYPE_CDF, DEFAULT_TX_2OR3_PARTITION_TYPE_CDF, DEFAULT_TX_DO_PARTITION_CDF,
    DEFAULT_TX_PARTITION_TYPE_CDF, DEFAULT_TX_PARTITION_TYPE_REDUCED_CDF,
};

use self::block_rows::{BlockCdfRows, BlockCdfSelector};
pub(crate) use self::coeff_rows::CoeffCdfSelector;
// Re-exported at crate visibility so sibling decode code (e.g. the future
// `coeffs()` consumer in `block_symbol.rs`) can name the `eob_pt` size class to
// construct the `pub(crate)` `TileCdfSelector::EobPt` variant.
pub(crate) use self::block_rows::{EobPtSize, MvCdfSelector};

const CDF_PROB_SCALE: i32 = 1 << 15;
const DO_SPLIT_PLANE_CONTEXTS: usize = 2;
/// § 8.3.2: `do_square_split` `PlaneStart` is fixed at 0, so only one plane is
/// valid for it (tighter than the shared 2-plane partition CDF array bound).
const DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS: usize = 1;
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
const MRL_INDEX_CONTEXTS: usize = 3;
const MRL_INDEX_ROW_LEN: usize = 5;
const MRL_SEC_INDEX_ROW_LEN: usize = 3;

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
type MrlIndexCdfRows = [[i32; MRL_INDEX_ROW_LEN]; MRL_INDEX_CONTEXTS];
type MrlSecIndexCdfRows = [[i32; MRL_SEC_INDEX_ROW_LEN]; MRL_INDEX_CONTEXTS];

/// Inputs for AV2 § 8.2.4 tile CDF copy/average policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfPolicyInput {
    tile_cols: u32,
    tile_rows: u32,
    enable_avg_cdf: bool,
    avg_cdf_type: bool,
    context_update_tile_id: u32,
}

impl TileCdfPolicyInput {
    /// Creates tile CDF policy facts from parsed frame/sequence tile info.
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

    /// Default single-tile policy used by current crate-private tests.
    #[must_use]
    pub(crate) const fn single_tile_default() -> Self {
        Self::new(1, 1, false, false, 0)
    }

    /// Returns a copy using the authoritative parsed tile-grid dimensions.
    #[must_use]
    pub(crate) const fn with_tile_grid(mut self, tile_cols: u32, tile_rows: u32) -> Self {
        self.tile_cols = tile_cols;
        self.tile_rows = tile_rows;
        self
    }
}

/// AV2 § 8.2.4 copy/average decision for one tile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfSavePolicy {
    num_log2: u8,
    copy_cdf: bool,
    avg_cdf: bool,
}

impl TileCdfSavePolicy {
    /// `numLog2` from AV2 § 8.2.4.
    #[must_use]
    pub(crate) const fn num_log2(self) -> u8 {
        self.num_log2
    }

    /// Whether this tile copies final Tile CDF rows into Saved CDF rows.
    #[must_use]
    pub(crate) const fn copy_cdf(self) -> bool {
        self.copy_cdf
    }

    /// Whether this tile averages final Tile CDF rows into Saved CDF rows.
    #[must_use]
    pub(crate) const fn avg_cdf(self) -> bool {
        self.avg_cdf
    }
}

/// Frame-level CDF subset for the current boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrameCdfSubset {
    rows: TileCdfRows,
}

impl FrameCdfSubset {
    /// Copies the supported subset from generated AV2 § 9.3 defaults.
    #[must_use]
    pub(crate) fn from_defaults() -> Self {
        Self {
            rows: TileCdfRows::from_defaults(),
        }
    }

    /// Copies frame CDF rows into tile-local CDF rows for `init_symbol`.
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

/// Tile-local mutable CDF subset for § 8.3 row selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfSubset {
    rows: TileCdfRows,
}

impl TileCdfSubset {
    /// Returns an immutable row for tests and metadata checks.
    pub(crate) fn row(&self, selector: TileCdfSelector) -> Result<&[i32], TileCdfError> {
        self.rows.row(selector)
    }

    /// Provides closure-scoped mutable row access for `read_symbol(cdf)`.
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

/// Saved CDF subset used for supported Tile-to-Saved lifecycle state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SavedCdfSubset {
    rows: TileCdfRows,
}

impl SavedCdfSubset {
    /// Starts saved rows from the frame-level subset.
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

/// Boundary metadata attached to one tile work unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCdfWorkUnitBoundary {
    update_mode: CdfUpdateMode,
    save_policy: TileCdfSavePolicy,
    frame_cdfs: FrameCdfSubset,
    saved_cdfs: SavedCdfSubset,
    tile_cdfs: TileCdfSubset,
}

impl TileCdfWorkUnitBoundary {
    /// Creates work-unit CDF boundary state from tile-local rows.
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

    /// CDF update mode that future syntax reads must use.
    #[must_use]
    pub(crate) const fn update_mode(&self) -> CdfUpdateMode {
        self.update_mode
    }

    /// Copy/average policy recorded for this tile.
    #[must_use]
    pub(crate) const fn save_policy(&self) -> TileCdfSavePolicy {
        self.save_policy
    }

    /// Tile-local CDF subset.
    #[must_use]
    pub(crate) const fn tile_cdfs(&self) -> &TileCdfSubset {
        &self.tile_cdfs
    }

    /// Mutable tile-local CDF subset for future `decode_tile()` symbol reads.
    pub(crate) fn tile_cdfs_mut(&mut self) -> &mut TileCdfSubset {
        &mut self.tile_cdfs
    }

    #[cfg(test)]
    pub(crate) const fn frame_cdfs(&self) -> &FrameCdfSubset {
        &self.frame_cdfs
    }

    #[cfg(test)]
    pub(crate) const fn saved_cdfs(&self) -> &SavedCdfSubset {
        &self.saved_cdfs
    }
}

/// Supported CDF selectors for the partition-entry boundary subset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileCdfSelector {
    /// `TileDoSplitCdf[PlaneStart][ctx]`.
    DoSplit {
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Partition context index.
        ctx: usize,
    },
    /// `TileDoExtPartitionCdf[PlaneStart][ctx]`.
    DoExtPartition {
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Partition context index.
        ctx: usize,
    },
    /// `TileDoSquareSplitCdf[0][ctx]`; AV2 § 8.3.2 fixes `PlaneStart` at 0.
    DoSquareSplit {
        /// `PlaneStart` partition-structure context; must be 0 for this selector.
        plane_start: usize,
        /// Square-split context index.
        ctx: usize,
    },
    /// `TileRectTypeCdf[PlaneStart][ctx]` from AV2 § 8.3.2
    /// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`).
    RectType {
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Rectangular-partition context index.
        ctx: usize,
    },
    /// `TileDoUneven4wayPartitionCdf[PlaneStart][ctx]`.
    DoUneven4WayPartition {
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Partition context index.
        ctx: usize,
    },
    /// `TileTxDoPartitionCdf[fsc_mode][is_inter][txfmSplitGroup]`.
    TxDoPartition {
        /// `fsc_mode` context.
        fsc_mode: usize,
        /// `is_inter` context.
        is_inter: usize,
        /// `Size_To_Tx_Part_Group_Lookup[MiSize]`.
        txfm_split_group: usize,
    },
    /// `TileTx2or3PartitionTypeCdf[fsc_mode][is_inter][ctx]`.
    Tx2Or3PartitionType {
        /// `fsc_mode` context.
        fsc_mode: usize,
        /// `is_inter` context.
        is_inter: usize,
        /// `Size_To_Tx_Type_Group_Vert_Or_Horz[MiSize] - 1`.
        ctx: usize,
    },
    /// `TileTxPartitionTypeCdf` or reduced variant selected by `reduced`.
    TxPartitionType {
        /// `fsc_mode` context.
        fsc_mode: usize,
        /// `is_inter` context.
        is_inter: usize,
        /// `Size_To_Tx_Type_Group_Vert_And_Horz[MiSize]`.
        ctx: usize,
        /// Selects `TileTxPartitionTypeReducedCdf` when true.
        reduced: bool,
    },
    /// `TileDeltaQCdf` from AV2 § 8.3.2.
    DeltaQ,
    /// `TileCdefIndex0Cdf[ctx]` from AV2 § 8.3.2.
    CdefIndex0 {
        /// CDEF neighbour context (`0..CDEF_STRENGTH_INDEX0_CTX`).
        ctx: usize,
    },
    /// `TileCcsoBlkCdf[plane][ctx]` from AV2 § 8.3.2 (per-block `ccso_blk`, § 5.20.10.2).
    CcsoBlk {
        /// Plane index (`0..CCSO_PLANES`).
        plane: usize,
        /// CCSO neighbour context (`0..CCSO_CONTEXTS`).
        ctx: usize,
    },
    /// `TileCdefIndexMinus1With<CdefStrengths>Cdf` from AV2 § 8.3.2.
    CdefIndexMinus1 {
        /// Parsed frame `CdefStrengths` value (`3..=8`).
        strengths: usize,
    },
    /// `TileIntrabcCdf[ctx]` from AV2 § 8.3.2.
    Intrabc {
        /// Intra block-copy neighbour context (`0..INTRABC_CONTEXTS`).
        ctx: usize,
    },
    /// `TileIntrabcModeCdf` from AV2 § 8.3.2.
    IntrabcMode,
    /// `TileIntrabcPrecisionCdf` from AV2 § 8.3.2.
    IntrabcPrecision,
    /// `TileFscModeCdf[ctx][Fsc_Bsize_Groups[MiSize]]` from AV2 § 8.3.2.
    FscMode {
        /// FSC neighbour context (`0..FSC_MODE_CONTEXTS`).
        ctx: usize,
        /// `Fsc_Bsize_Groups[MiSize]`.
        bsize_group: usize,
    },
    /// `TileMrlIndexCdf[ctx]` from AV2 § 8.3.2.
    MrlIndex {
        /// MRL neighbour context (`0..MRL_INDEX_CONTEXTS`).
        ctx: usize,
    },
    /// `TileMrlSecIndexCdf[ctx]` from AV2 § 8.3.2.
    MrlSecIndex {
        /// MRL neighbour context (`0..MRL_INDEX_CONTEXTS`).
        ctx: usize,
    },
    /// `TileYModeSetCdf` from AV2 § 8.3.2 for the minimal intra trace.
    YModeSet,
    /// `TileYModeIndexCdf[ctx]` from AV2 § 8.3.2 for the minimal intra trace.
    YModeIndex {
        /// Intra mode context index.
        ctx: usize,
    },
    /// `TileYModeOffsetCdf[ctx]` from AV2 § 8.3.2 (the § 5.20.5.3 `y_mode_offset`
    /// escape symbol; shares the `y_mode_index` context derivation).
    YModeOffset {
        /// Intra mode context index.
        ctx: usize,
    },
    /// `TileTxbSkipCdf[coeff_cdf_q_ctx][plane_type][tx_size][ctx]`.
    ///
    /// This selector exposes only the generated-default row shape needed by
    /// the current minimal block-symbol trace; it is not a broad § 8.3
    /// transform-syntax context derivation API.
    TxbSkip {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Plane type context.
        plane_type: usize,
        /// Transform-size context.
        tx_size: usize,
        /// Transform-skip context index.
        ctx: usize,
    },
    /// `TileIntraTxTypeSet1Cdf[Tx_Size_Sqr[txSz]]` from AV2 § 8.3.2 Table 8.2.
    IntraTxTypeSet1 {
        /// `Tx_Size_Sqr[txSz]`.
        tx_size_sqr: usize,
    },
    /// `TileIntraTxTypeSet2Cdf[Tx_Size_Sqr[txSz]]` from AV2 § 8.3.2 Table 8.2.
    IntraTxTypeSet2 {
        /// `Tx_Size_Sqr[txSz]`.
        tx_size_sqr: usize,
    },
    /// `TileIsLongSideDctCdf[is_inter]` from AV2 § 8.3.2.
    IsLongSideDct {
        /// `is_inter`.
        is_inter: usize,
    },
    /// `TileIntraTxTypeLongCdf[Tx_Size_Sqr[txSz]]` from AV2 § 8.3.2 Table 8.2.
    IntraTxTypeLong {
        /// `Tx_Size_Sqr[txSz]`.
        tx_size_sqr: usize,
    },
    /// `TileSecTxTypeCdf[is_inter][Tx_Size_Sqr[txSz]]` from AV2 § 8.3.2.
    SecTxType {
        /// `is_inter`.
        is_inter: usize,
        /// `Tx_Size_Sqr[txSz]`.
        tx_size_sqr: usize,
    },
    /// `TileMostProbableStxSetCdf` from AV2 § 8.3.2.
    MostProbableStxSet,
    /// `TileMostProbableStxSetAdstCdf` from AV2 § 8.3.2.
    MostProbableStxSetAdst,
    /// `TileCctxTypeCdf` from AV2 § 8.3.2.
    CctxType,
    /// `TileUvModeCflNotAllowedCdf[ctx]` from AV2 § 8.3.2.
    UvModeCflNotAllowed {
        /// Chroma mode context index.
        ctx: usize,
    },
    /// `TileIsCflCdf[ctx]` from AV2 § 8.3.2.
    IsCfl {
        /// CfL neighbour context index.
        ctx: usize,
    },
    /// `TileCflIndexCdf` from AV2 § 8.3.2.
    CflIndex,
    /// `TileCflSignCdf` from AV2 § 8.3.2.
    CflSign,
    /// `TileCflAlphaCdf[ctx]` from AV2 § 8.3.2.
    CflAlpha {
        /// CfL alpha context index.
        ctx: usize,
    },
    /// `TileCflMhccpCdf` from AV2 § 8.3.2.
    CflMhccp,
    /// `TileCflMhDirCdf[Size_Group[MiSize]]` from AV2 § 8.3.2.
    CflMhDir {
        /// `Size_Group[MiSize]`.
        size_group: usize,
    },
    /// `TileVTxbSkipCdf[coeff_cdf_q_ctx][ctx]`.
    VTxbSkip {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// V-plane transform-skip context index.
        ctx: usize,
    },
    /// `TileEobExtraCdf[coeff_cdf_q_ctx]` (AV2 § 8.3.2: context-free selection).
    EobExtra {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
    },
    /// `TileEobPt<size>Cdf[coeff_cdf_q_ctx][eobCtx]` (AV2 § 8.3.2).
    EobPt {
        /// Transform-size class selecting the `eob_pt` family bank.
        size: EobPtSize,
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// `eobCtx = (plane > 0) ? 2 : is_inter`.
        eob_ctx: usize,
    },
    /// `TileDcSignCdf[coeff_cdf_q_ctx][plane_type][group][ctx]` (AV2 § 8.3.2).
    DcSign {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Plane type context (luma vs chroma).
        plane_type: usize,
        /// `isHidden` group.
        group: usize,
        /// DC-sign context (caller-resolved from the Above/Left DC contexts).
        ctx: usize,
    },
    /// `TileIsInterCdf[ctx]` (AV2 § 8.3.2): the `read_is_inter` decision.
    IsInter {
        /// `is_inter` context index.
        ctx: usize,
    },
    /// `TileSkipCdf[ctx]` (AV2 § 8.3.2): the `read_skip` decision.
    Skip {
        /// `skip_flag` context index.
        ctx: usize,
    },
    /// `TileSingleModeCdf[NewMvContext]` (AV2 § 8.3.2): the single-reference inter
    /// `single_mode` symbol.
    SingleMode {
        /// `NewMvContext`.
        ctx: usize,
    },
    /// `TileDrlModeCdf[Min(idx, 2)][NewMvContext]` (AV2 § 8.3.2): the §5.20.7.8
    /// `read_drl_idx` `drl_mode` symbol.
    DrlMode {
        /// `Min(idx, 2)` index bank.
        idx: usize,
        /// `NewMvContext`.
        ctx: usize,
    },
    /// `TileSingleRefCdf[ctx][ref]` (AV2 § 8.3.2): the §5.20.7.12 `read_single_ref`
    /// binary `single_ref` symbol.
    SingleRef {
        /// §8.3.2 neighbour-derived single_ref context (same derivation as
        /// `comp_ref`).
        ctx: usize,
        /// The §5.20.7.12 loop counter `ref`.
        ref_idx: usize,
    },
    /// `TileCompModeCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.10 `comp_mode` symbol.
    CompMode {
        /// §8.3.2 compound-reference mode context.
        ctx: usize,
    },
    /// `TileIsJointCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.6 `is_joint` symbol.
    IsJoint {
        /// §8.3.2 `is_joint` context.
        ctx: usize,
    },
    /// `TileCompoundModeNonJointCdf[NewMvContext]` (AV2 § 8.3.2): the
    /// §5.20.7.6 non-joint compound mode symbol.
    CompoundModeNonJoint {
        /// `NewMvContext`.
        ctx: usize,
    },
    /// `TileCompGroupIdxCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.16
    /// `comp_group_idx` symbol.
    CompGroupIdx {
        /// `comp_group_idx` context.
        ctx: usize,
    },
    /// `TileCwpIdxCdf[idx]` (AV2 § 8.3.2): the §5.20.7.6 `cwp_idx` symbol.
    CwpIdx {
        /// CWP truncated-unary index.
        idx: usize,
    },
    /// `TileCompRef0Cdf[ctx][ref]` (AV2 § 8.3.2): the §5.20.7.11 `comp_ref`
    /// symbol before any compound reference has been found.
    CompRef0 {
        /// §8.3.2 neighbour-derived comp_ref context.
        ctx: usize,
        /// The §5.20.7.11 loop counter `ref`.
        ref_idx: usize,
    },
    /// `TileCompRef1Cdf[ctx][bitType][ref]` (AV2 § 8.3.2): the §5.20.7.11
    /// `comp_ref` symbol after the first compound reference has been found.
    CompRef1 {
        /// §8.3.2 neighbour-derived comp_ref context.
        ctx: usize,
        /// Same-side/opposite-side bit type.
        bit_type: usize,
        /// The §5.20.7.11 loop counter `ref`.
        ref_idx: usize,
    },
    /// AV2 § 5.20.7.20 SHELL-coded motion-vector CDF rows.
    ReadMv(MvCdfSelector),
    /// `TileInterpFilterCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.6 SWITCHABLE
    /// `interp_filter` symbol.
    InterpFilter {
        /// The §8.3.2 interp-filter context.
        ctx: usize,
    },
    /// `TileUseWienerNsCdf` (AV2 §8.3.2): the §5.20.10.5 `use_wiener_ns`
    /// binary symbol.
    UseWienerNs,
    /// `TileWienerNsLengthCdf[Min(plane, 1)]` (AV2 §8.3.2): the
    /// §5.20.10.6 `wiener_ns_length` binary symbol.
    WienerNsLength {
        /// `Min(plane, 1)`.
        plane_ctx: usize,
    },
    /// `TileWienerNsUvSymCdf` (AV2 §8.3.2): the §5.20.10.6
    /// `wiener_ns_uv_sym` binary symbol.
    WienerNsUvSym,
    /// `TileWienerNsBaseCdf` (AV2 §8.3.2): the §5.20.10.6
    /// `wiener_ns_base` symbol used by `decode_4part`.
    WienerNsBase,
    /// Coefficient base/base-EOB/base-range and IDTX CDF rows.
    Coeff(CoeffCdfSelector),
}

/// Supported CDF arrays for error reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileCdfArray {
    /// `TileDoSplitCdf`.
    DoSplit,
    /// `TileDoExtPartitionCdf`.
    DoExtPartition,
    /// `TileDoSquareSplitCdf`.
    DoSquareSplit,
    /// `TileRectTypeCdf`.
    RectType,
    /// `TileDoUneven4wayPartitionCdf`.
    DoUneven4WayPartition,
    /// `TileTxDoPartitionCdf`.
    TxDoPartition,
    /// `TileTx2or3PartitionTypeCdf`.
    Tx2Or3PartitionType,
    /// `TileTxPartitionTypeCdf`.
    TxPartitionType,
    /// `TileTxPartitionTypeReducedCdf`.
    TxPartitionTypeReduced,
    /// `TileCdefIndex0Cdf`.
    CdefIndex0,
    /// `TileCcsoBlkCdf`.
    CcsoBlk,
    /// `TileCdefIndexMinus1With<CdefStrengths>Cdf`.
    CdefIndexMinus1,
    /// `TileIntrabcCdf`.
    Intrabc,
    /// `TileFscModeCdf`.
    FscMode,
    /// `TileMrlIndexCdf`.
    MrlIndex,
    /// `TileMrlSecIndexCdf`.
    MrlSecIndex,
    /// `TileYModeIndexCdf`.
    YModeIndex,
    /// `TileYModeOffsetCdf`.
    YModeOffset,
    /// `TileTxbSkipCdf`.
    TxbSkip,
    /// `TileIntraTxTypeSet1Cdf`.
    IntraTxTypeSet1,
    /// `TileIntraTxTypeSet2Cdf`.
    IntraTxTypeSet2,
    /// `TileIsLongSideDctCdf`.
    IsLongSideDct,
    /// `TileIntraTxTypeLongCdf`.
    IntraTxTypeLong,
    /// `TileSecTxTypeCdf`.
    SecTxType,
    /// `TileUvModeCflNotAllowedCdf`.
    UvModeCflNotAllowed,
    /// `TileIsCflCdf`.
    IsCfl,
    /// `TileCflAlphaCdf`.
    CflAlpha,
    /// `TileCflMhDirCdf`.
    CflMhDir,
    /// `TileVTxbSkipCdf`.
    VTxbSkip,
    /// `TileEobExtraCdf`.
    EobExtra,
    /// `TileEobPt<size>Cdf` family.
    EobPt,
    /// `TileDcSignCdf`.
    DcSign,
    /// `TileCoeffBaseCdf`.
    CoeffBase,
    /// `TileCoeffBasePhCdf`.
    CoeffBasePh,
    /// `TileCoeffBaseUvCdf`.
    CoeffBaseUv,
    /// `TileCoeffBaseLfCdf`.
    CoeffBaseLf,
    /// `TileCoeffBaseLfUvCdf`.
    CoeffBaseLfUv,
    /// `TileCoeffBaseEobCdf`.
    CoeffBaseEob,
    /// `TileCoeffBaseEobUvCdf`.
    CoeffBaseEobUv,
    /// `TileCoeffBaseBobCdf`.
    CoeffBaseBob,
    /// `TileCoeffBaseIdtxCdf`.
    CoeffBaseIdtx,
    /// `TileCoeffBaseLfEobCdf`.
    CoeffBaseLfEob,
    /// `TileCoeffBaseLfEobUvCdf`.
    CoeffBaseLfEobUv,
    /// `TileCoeffBrCdf`.
    CoeffBr,
    /// `TileCoeffBrUvCdf`.
    CoeffBrUv,
    /// `TileCoeffBrLfCdf`.
    CoeffBrLf,
    /// `TileCoeffBrIdtxCdf`.
    CoeffBrIdtx,
    /// `TileIdtxSignCdf`.
    IdtxSign,
    /// `TileIsInterCdf`.
    IsInter,
    /// `TileSkipCdf`.
    Skip,
    /// `TileSingleModeCdf`.
    SingleMode,
    /// `TileDrlModeCdf`.
    DrlMode,
    /// `TileSingleRefCdf`.
    SingleRef,
    /// `TileCompModeCdf`.
    CompMode,
    /// `TileIsJointCdf`.
    IsJoint,
    /// `TileCompoundModeNonJointCdf`.
    CompoundModeNonJoint,
    /// `TileCompGroupIdxCdf`.
    CompGroupIdx,
    /// `TileCwpIdxCdf`.
    CwpIdx,
    /// `TileCompRef1Cdf`.
    CompRef1,
    /// `TileCompRef0Cdf`.
    CompRef0,
    /// `TileJointShell6ClassCdf` (the EighthPel P == 6 `shell_class` bank pair).
    JointShell6Class,
    /// `TileShellOffsetLowClassCdf`.
    ShellOffsetLowClass,
    /// `TileShellOffsetOtherClassCdf`.
    ShellOffsetOtherClass,
    /// `TileColMvGreaterCdf`.
    ColMvGreater,
    /// `TileColMvIndexCdf`.
    ColMvIndex,
    /// `TileInterpFilterCdf`.
    InterpFilter,
    /// `TileWienerNsLengthCdf`.
    WienerNsLength,
}

impl TileCdfArray {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DoSplit => "TileDoSplitCdf",
            Self::DoExtPartition => "TileDoExtPartitionCdf",
            Self::DoSquareSplit => "TileDoSquareSplitCdf",
            Self::RectType => "TileRectTypeCdf",
            Self::DoUneven4WayPartition => "TileDoUneven4wayPartitionCdf",
            Self::TxDoPartition => "TileTxDoPartitionCdf",
            Self::Tx2Or3PartitionType => "TileTx2or3PartitionTypeCdf",
            Self::TxPartitionType => "TileTxPartitionTypeCdf",
            Self::TxPartitionTypeReduced => "TileTxPartitionTypeReducedCdf",
            Self::CdefIndex0 => "TileCdefIndex0Cdf",
            Self::CcsoBlk => "TileCcsoBlkCdf",
            Self::CdefIndexMinus1 => "TileCdefIndexMinus1Cdf",
            Self::Intrabc => "TileIntrabcCdf",
            Self::FscMode => "TileFscModeCdf",
            Self::MrlIndex => "TileMrlIndexCdf",
            Self::MrlSecIndex => "TileMrlSecIndexCdf",
            Self::YModeIndex => "TileYModeIndexCdf",
            Self::YModeOffset => "TileYModeOffsetCdf",
            Self::TxbSkip => "TileTxbSkipCdf",
            Self::IntraTxTypeSet1 => "TileIntraTxTypeSet1Cdf",
            Self::IntraTxTypeSet2 => "TileIntraTxTypeSet2Cdf",
            Self::IsLongSideDct => "TileIsLongSideDctCdf",
            Self::IntraTxTypeLong => "TileIntraTxTypeLongCdf",
            Self::SecTxType => "TileSecTxTypeCdf",
            Self::UvModeCflNotAllowed => "TileUvModeCflNotAllowedCdf",
            Self::IsCfl => "TileIsCflCdf",
            Self::CflAlpha => "TileCflAlphaCdf",
            Self::CflMhDir => "TileCflMhDirCdf",
            Self::VTxbSkip => "TileVTxbSkipCdf",
            Self::EobExtra => "TileEobExtraCdf",
            Self::EobPt => "TileEobPtCdf",
            Self::DcSign => "TileDcSignCdf",
            Self::CoeffBase => "TileCoeffBaseCdf",
            Self::CoeffBasePh => "TileCoeffBasePhCdf",
            Self::CoeffBaseUv => "TileCoeffBaseUvCdf",
            Self::CoeffBaseLf => "TileCoeffBaseLfCdf",
            Self::CoeffBaseLfUv => "TileCoeffBaseLfUvCdf",
            Self::CoeffBaseEob => "TileCoeffBaseEobCdf",
            Self::CoeffBaseEobUv => "TileCoeffBaseEobUvCdf",
            Self::CoeffBaseBob => "TileCoeffBaseBobCdf",
            Self::CoeffBaseIdtx => "TileCoeffBaseIdtxCdf",
            Self::CoeffBaseLfEob => "TileCoeffBaseLfEobCdf",
            Self::CoeffBaseLfEobUv => "TileCoeffBaseLfEobUvCdf",
            Self::CoeffBr => "TileCoeffBrCdf",
            Self::CoeffBrUv => "TileCoeffBrUvCdf",
            Self::CoeffBrLf => "TileCoeffBrLfCdf",
            Self::CoeffBrIdtx => "TileCoeffBrIdtxCdf",
            Self::IdtxSign => "TileIdtxSignCdf",
            Self::IsInter => "TileIsInterCdf",
            Self::Skip => "TileSkipCdf",
            Self::SingleMode => "TileSingleModeCdf",
            Self::DrlMode => "TileDrlModeCdf",
            Self::SingleRef => "TileSingleRefCdf",
            Self::CompMode => "TileCompModeCdf",
            Self::IsJoint => "TileIsJointCdf",
            Self::CompoundModeNonJoint => "TileCompoundModeNonJointCdf",
            Self::CompGroupIdx => "TileCompGroupIdxCdf",
            Self::CwpIdx => "TileCwpIdxCdf",
            Self::CompRef0 => "TileCompRef0Cdf",
            Self::CompRef1 => "TileCompRef1Cdf",
            Self::JointShell6Class => "TileJointShell6ClassCdf",
            Self::ShellOffsetLowClass => "TileShellOffsetLowClassCdf",
            Self::ShellOffsetOtherClass => "TileShellOffsetOtherClassCdf",
            Self::ColMvGreater => "TileColMvGreaterCdf",
            Self::ColMvIndex => "TileColMvIndexCdf",
            Self::InterpFilter => "TileInterpFilterCdf",
            Self::WienerNsLength => "TileWienerNsLengthCdf",
        }
    }
}

/// Tile CDF boundary error.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum TileCdfError {
    /// Selector context exceeded the supported array dimensions.
    #[error("{array} selector {index_name}={actual} is outside 0..{max_exclusive}")]
    SelectorOutOfRange {
        /// CDF array being selected.
        array: TileCdfArray,
        /// Index name.
        index_name: &'static str,
        /// Actual supplied index.
        actual: usize,
        /// Exclusive upper bound.
        max_exclusive: usize,
    },
    /// `TileCols * TileRows` overflowed the bounded policy type.
    #[error("tile count overflow for TileCols={tile_cols}, TileRows={tile_rows}")]
    TileCountOverflow {
        /// Tile columns.
        tile_cols: u32,
        /// Tile rows.
        tile_rows: u32,
    },
    /// `TileCols * TileRows` was zero.
    #[error("tile count must be nonzero, got TileCols={tile_cols}, TileRows={tile_rows}")]
    InvalidTileCount {
        /// Tile columns.
        tile_cols: u32,
        /// Tile rows.
        tile_rows: u32,
    },
    /// The current tile number is outside the tile grid.
    #[error("TileNum={tile_num} is outside tile count {tile_count}")]
    TileNumOutOfRange {
        /// Tile number.
        tile_num: u32,
        /// Tile count.
        tile_count: u32,
    },
    /// `context_update_tile_id` is outside the tile grid.
    #[error("context_update_tile_id={context_update_tile_id} is outside tile count {tile_count}")]
    ContextUpdateTileOutOfRange {
        /// Context update tile id.
        context_update_tile_id: u32,
        /// Tile count.
        tile_count: u32,
    },
    /// `bSize` exceeded the generated block-size table dimensions.
    #[error("{table} bSize={b_size} is outside 0..{max_exclusive}")]
    BlockSizeOutOfRange {
        /// Table or context being indexed by `bSize`.
        table: &'static str,
        /// Actual supplied `bSize`.
        b_size: usize,
        /// Exclusive upper bound.
        max_exclusive: usize,
    },
    /// A left or above neighbor slot was not available.
    #[error("{array}[{plane_start}][{index}] is unavailable; length {len}")]
    PartitionNeighborOutOfRange {
        /// Neighbor array being indexed.
        array: &'static str,
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Requested neighbor index.
        index: usize,
        /// Available neighbor-array length.
        len: usize,
    },
    /// A left or above neighbor slot contained an invalid block-size index.
    #[error(
        "{array}[{plane_start}][{index}] block size {block_size} is outside 0..{max_exclusive}"
    )]
    PartitionNeighborBlockSizeOutOfRange {
        /// Neighbor array being indexed.
        array: &'static str,
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Requested neighbor index.
        index: usize,
        /// Invalid neighbor block-size index.
        block_size: usize,
        /// Exclusive upper bound.
        max_exclusive: usize,
    },
    /// A partition grid coordinate would underflow before lookup.
    #[error(
        "{array}[{plane_start}] {coordinate} coordinate underflow deriving {actual}-{subtract}"
    )]
    PartitionGridCoordinateUnderflow {
        /// Grid array being indexed.
        array: &'static str,
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Coordinate name.
        coordinate: &'static str,
        /// Actual supplied coordinate.
        actual: usize,
        /// Amount subtracted from the coordinate.
        subtract: usize,
    },
    /// A partition grid row was not available.
    #[error("{array}[{plane_start}][{row}] row is unavailable; rows {rows}")]
    PartitionGridRowOutOfRange {
        /// Grid array being indexed.
        array: &'static str,
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Requested row index.
        row: usize,
        /// Available row count.
        rows: usize,
    },
    /// A partition grid column was not available.
    #[error("{array}[{plane_start}][{row}][{col}] column is unavailable; columns {cols}")]
    PartitionGridColumnOutOfRange {
        /// Grid array being indexed.
        array: &'static str,
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Requested row index.
        row: usize,
        /// Requested column index.
        col: usize,
        /// Available column count in the requested row.
        cols: usize,
    },
    /// A partition grid cell contained an invalid block-size index.
    #[error(
        "{array}[{plane_start}][{row}][{col}] block size {block_size} is outside 0..{max_exclusive}"
    )]
    PartitionGridBlockSizeOutOfRange {
        /// Grid array being indexed.
        array: &'static str,
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Requested row index.
        row: usize,
        /// Requested column index.
        col: usize,
        /// Invalid grid block-size index.
        block_size: usize,
        /// Exclusive upper bound.
        max_exclusive: usize,
    },
    /// Extended-partition second-neighbor index arithmetic overflowed.
    #[error("{array}[{plane_start}] index overflow deriving {base}+{offset}")]
    PartitionNeighborIndexOverflow {
        /// Neighbor array being indexed.
        array: &'static str,
        /// `PlaneStart` partition-structure context.
        plane_start: usize,
        /// Base neighbor index.
        base: usize,
        /// Derived second-half offset.
        offset: usize,
    },
    /// A generated conversion table entry was negative where a context index is required.
    #[error("{table}[{b_size}] value {value} cannot be represented as a context index")]
    ConversionTableValueOutOfRange {
        /// Generated conversion table being converted.
        table: &'static str,
        /// Block-size index.
        b_size: usize,
        /// Generated table value.
        value: i32,
    },
}

impl fmt::Display for TileCdfArray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Computes AV2 § 8.2.4 copy/average policy for one tile.
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
    fsc_mode: FscModeCdfRows,
    mrl_index: MrlIndexCdfRows,
    mrl_sec_index: MrlSecIndexCdfRows,
    block: BlockCdfRows,
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
            fsc_mode: DEFAULT_FSC_MODE_CDF,
            mrl_index: DEFAULT_MRL_INDEX_CDF,
            mrl_sec_index: DEFAULT_MRL_SEC_INDEX_CDF,
            block: BlockCdfRows::from_defaults(),
        }
    }

    fn row(&self, selector: TileCdfSelector) -> Result<&[i32], TileCdfError> {
        match selector {
            TileCdfSelector::DoSplit { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoSplit, plane_start)?;
                let row =
                    self.do_split[plane]
                        .get(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::DoSplit,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive: DO_SPLIT_CONTEXTS,
                        })?;
                Ok(row.as_slice())
            }
            TileCdfSelector::DoExtPartition { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoExtPartition, plane_start)?;
                let row = self.do_ext_partition[plane].get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DoExtPartition,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: DO_EXT_PARTITION_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            TileCdfSelector::DoSquareSplit { plane_start, ctx } => {
                let plane = checked_square_split_plane(plane_start)?;
                let row = self.do_square_split[plane].get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DoSquareSplit,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: DO_SQUARE_SPLIT_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            TileCdfSelector::RectType { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::RectType, plane_start)?;
                let row =
                    self.rect_type[plane]
                        .get(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::RectType,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive: RECT_TYPE_CONTEXTS,
                        })?;
                Ok(row.as_slice())
            }
            TileCdfSelector::DoUneven4WayPartition { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoUneven4WayPartition, plane_start)?;
                let row = self.do_uneven_4way_partition[plane].get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DoUneven4WayPartition,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: DO_UNEVEN_4WAY_PARTITION_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
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
                let row = self.tx_do_partition[fsc_mode][is_inter]
                    .get(txfm_split_group)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::TxDoPartition,
                        index_name: "txfm_split_group",
                        actual: txfm_split_group,
                        max_exclusive: TXFM_SPLIT_GROUPS,
                    })?;
                Ok(row.as_slice())
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
                let row = self.tx_2or3_partition_type[fsc_mode][is_inter]
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::Tx2Or3PartitionType,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: TX_2OR3_PARTITION_TYPE_CONTEXTS,
                    })?;
                Ok(row.as_slice())
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
                let rows = if reduced {
                    &self.tx_partition_type_reduced
                } else {
                    &self.tx_partition_type
                };
                let row =
                    rows[fsc_mode][is_inter]
                        .get(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive: TX_PARTITION_TYPE_CONTEXTS,
                        })?;
                Ok(row.as_slice())
            }
            TileCdfSelector::DeltaQ => Ok(self.delta_q.as_slice()),
            TileCdfSelector::CdefIndex0 { ctx } => {
                let row = self
                    .cdef_index0
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CdefIndex0,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: CDEF_STRENGTH_INDEX0_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            TileCdfSelector::CcsoBlk { plane, ctx } => {
                let plane_rows =
                    self.ccso_blk
                        .get(plane)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::CcsoBlk,
                            index_name: "plane",
                            actual: plane,
                            max_exclusive: CCSO_PLANES,
                        })?;
                let row = plane_rows
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CcsoBlk,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: CCSO_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            TileCdfSelector::CdefIndexMinus1 { strengths } => {
                cdef_index_minus1_row(self, strengths)
            }
            TileCdfSelector::Intrabc { ctx } => {
                let row = self
                    .intrabc
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::Intrabc,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: INTRABC_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            TileCdfSelector::IntrabcMode => Ok(self.intrabc_mode.as_slice()),
            TileCdfSelector::IntrabcPrecision => Ok(self.intrabc_precision.as_slice()),
            TileCdfSelector::FscMode { ctx, bsize_group } => {
                let ctx = checked_context(TileCdfArray::FscMode, "ctx", ctx, FSC_MODE_CONTEXTS)?;
                let row = self.fsc_mode[ctx].get(bsize_group).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::FscMode,
                        index_name: "bsize_group",
                        actual: bsize_group,
                        max_exclusive: FSC_BSIZE_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            TileCdfSelector::MrlIndex { ctx } => {
                let ctx = checked_context(TileCdfArray::MrlIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok(self.mrl_index[ctx].as_slice())
            }
            TileCdfSelector::MrlSecIndex { ctx } => {
                let ctx =
                    checked_context(TileCdfArray::MrlSecIndex, "ctx", ctx, MRL_INDEX_CONTEXTS)?;
                Ok(self.mrl_sec_index[ctx].as_slice())
            }
            TileCdfSelector::YModeSet => self.block.row(BlockCdfSelector::YModeSet),
            TileCdfSelector::YModeIndex { ctx } => {
                self.block.row(BlockCdfSelector::YModeIndex { ctx })
            }
            TileCdfSelector::YModeOffset { ctx } => {
                self.block.row(BlockCdfSelector::YModeOffset { ctx })
            }
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            } => self.block.row(BlockCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            }),
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr } => self
                .block
                .row(BlockCdfSelector::IntraTxTypeSet1 { tx_size_sqr }),
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr } => self
                .block
                .row(BlockCdfSelector::IntraTxTypeSet2 { tx_size_sqr }),
            TileCdfSelector::IsLongSideDct { is_inter } => {
                self.block.row(BlockCdfSelector::IsLongSideDct { is_inter })
            }
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr } => self
                .block
                .row(BlockCdfSelector::IntraTxTypeLong { tx_size_sqr }),
            TileCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            } => self.block.row(BlockCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            }),
            TileCdfSelector::MostProbableStxSet => {
                self.block.row(BlockCdfSelector::MostProbableStxSet)
            }
            TileCdfSelector::MostProbableStxSetAdst => {
                self.block.row(BlockCdfSelector::MostProbableStxSetAdst)
            }
            TileCdfSelector::CctxType => self.block.row(BlockCdfSelector::CctxType),
            TileCdfSelector::UvModeCflNotAllowed { ctx } => self
                .block
                .row(BlockCdfSelector::UvModeCflNotAllowed { ctx }),
            TileCdfSelector::IsCfl { ctx } => self.block.row(BlockCdfSelector::IsCfl { ctx }),
            TileCdfSelector::CflIndex => self.block.row(BlockCdfSelector::CflIndex),
            TileCdfSelector::CflSign => self.block.row(BlockCdfSelector::CflSign),
            TileCdfSelector::CflAlpha { ctx } => self.block.row(BlockCdfSelector::CflAlpha { ctx }),
            TileCdfSelector::CflMhccp => self.block.row(BlockCdfSelector::CflMhccp),
            TileCdfSelector::CflMhDir { size_group } => {
                self.block.row(BlockCdfSelector::CflMhDir { size_group })
            }
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => self.block.row(BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            }),
            TileCdfSelector::EobExtra { coeff_cdf_q_ctx } => self
                .block
                .row(BlockCdfSelector::EobExtra { coeff_cdf_q_ctx }),
            TileCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => self.block.row(BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            }),
            TileCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            } => self.block.row(BlockCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            }),
            TileCdfSelector::IsInter { ctx } => self.block.row(BlockCdfSelector::IsInter { ctx }),
            TileCdfSelector::Skip { ctx } => self.block.row(BlockCdfSelector::Skip { ctx }),
            TileCdfSelector::SingleMode { ctx } => {
                self.block.row(BlockCdfSelector::SingleMode { ctx })
            }
            TileCdfSelector::DrlMode { idx, ctx } => {
                self.block.row(BlockCdfSelector::DrlMode { idx, ctx })
            }
            TileCdfSelector::SingleRef { ctx, ref_idx } => {
                self.block.row(BlockCdfSelector::SingleRef { ctx, ref_idx })
            }
            TileCdfSelector::CompMode { ctx } => self.block.row(BlockCdfSelector::CompMode { ctx }),
            TileCdfSelector::IsJoint { ctx } => self.block.row(BlockCdfSelector::IsJoint { ctx }),
            TileCdfSelector::CompoundModeNonJoint { ctx } => self
                .block
                .row(BlockCdfSelector::CompoundModeNonJoint { ctx }),
            TileCdfSelector::CompGroupIdx { ctx } => {
                self.block.row(BlockCdfSelector::CompGroupIdx { ctx })
            }
            TileCdfSelector::CwpIdx { idx } => self.block.row(BlockCdfSelector::CwpIdx { idx }),
            TileCdfSelector::CompRef0 { ctx, ref_idx } => {
                self.block.row(BlockCdfSelector::CompRef0 { ctx, ref_idx })
            }
            TileCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => self.block.row(BlockCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            }),
            TileCdfSelector::ReadMv(selector) => self.block.row(BlockCdfSelector::ReadMv(selector)),
            TileCdfSelector::InterpFilter { ctx } => {
                self.block.row(BlockCdfSelector::InterpFilter { ctx })
            }
            TileCdfSelector::UseWienerNs => self.block.row(BlockCdfSelector::UseWienerNs),
            TileCdfSelector::WienerNsLength { plane_ctx } => self
                .block
                .row(BlockCdfSelector::WienerNsLength { plane_ctx }),
            TileCdfSelector::WienerNsUvSym => self.block.row(BlockCdfSelector::WienerNsUvSym),
            TileCdfSelector::WienerNsBase => self.block.row(BlockCdfSelector::WienerNsBase),
            TileCdfSelector::Coeff(selector) => self.block.row(BlockCdfSelector::Coeff(selector)),
        }
    }

    fn row_mut(&mut self, selector: TileCdfSelector) -> Result<&mut [i32], TileCdfError> {
        match selector {
            TileCdfSelector::DoSplit { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoSplit, plane_start)?;
                let max_exclusive = self.do_split[plane].len();
                let row =
                    self.do_split[plane]
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::DoSplit,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::DoExtPartition { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoExtPartition, plane_start)?;
                let max_exclusive = self.do_ext_partition[plane].len();
                let row = self.do_ext_partition[plane].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DoExtPartition,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::DoSquareSplit { plane_start, ctx } => {
                let plane = checked_square_split_plane(plane_start)?;
                let max_exclusive = self.do_square_split[plane].len();
                let row = self.do_square_split[plane].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DoSquareSplit,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::RectType { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::RectType, plane_start)?;
                let max_exclusive = self.rect_type[plane].len();
                let row =
                    self.rect_type[plane]
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::RectType,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::DoUneven4WayPartition { plane_start, ctx } => {
                let plane = checked_plane(TileCdfArray::DoUneven4WayPartition, plane_start)?;
                let max_exclusive = self.do_uneven_4way_partition[plane].len();
                let row = self.do_uneven_4way_partition[plane].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DoUneven4WayPartition,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
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
                let max_exclusive = self.tx_do_partition[fsc_mode][is_inter].len();
                let row = self.tx_do_partition[fsc_mode][is_inter]
                    .get_mut(txfm_split_group)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::TxDoPartition,
                        index_name: "txfm_split_group",
                        actual: txfm_split_group,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
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
                let max_exclusive = self.tx_2or3_partition_type[fsc_mode][is_inter].len();
                let row = self.tx_2or3_partition_type[fsc_mode][is_inter]
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::Tx2Or3PartitionType,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
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
                let rows = if reduced {
                    &mut self.tx_partition_type_reduced
                } else {
                    &mut self.tx_partition_type
                };
                let max_exclusive = rows[fsc_mode][is_inter].len();
                let row = rows[fsc_mode][is_inter].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::DeltaQ => Ok(self.delta_q.as_mut_slice()),
            TileCdfSelector::CdefIndex0 { ctx } => {
                let max_exclusive = self.cdef_index0.len();
                let row =
                    self.cdef_index0
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::CdefIndex0,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::CcsoBlk { plane, ctx } => {
                let plane_rows =
                    self.ccso_blk
                        .get_mut(plane)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::CcsoBlk,
                            index_name: "plane",
                            actual: plane,
                            max_exclusive: CCSO_PLANES,
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
                let max_exclusive = self.intrabc.len();
                let row = self
                    .intrabc
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::Intrabc,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            TileCdfSelector::IntrabcMode => Ok(self.intrabc_mode.as_mut_slice()),
            TileCdfSelector::IntrabcPrecision => Ok(self.intrabc_precision.as_mut_slice()),
            TileCdfSelector::FscMode { ctx, bsize_group } => {
                let ctx = checked_context(TileCdfArray::FscMode, "ctx", ctx, FSC_MODE_CONTEXTS)?;
                let max_exclusive = self.fsc_mode[ctx].len();
                let row = self.fsc_mode[ctx].get_mut(bsize_group).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::FscMode,
                        index_name: "bsize_group",
                        actual: bsize_group,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
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
            TileCdfSelector::YModeSet => self.block.row_mut(BlockCdfSelector::YModeSet),
            TileCdfSelector::YModeIndex { ctx } => {
                self.block.row_mut(BlockCdfSelector::YModeIndex { ctx })
            }
            TileCdfSelector::YModeOffset { ctx } => {
                self.block.row_mut(BlockCdfSelector::YModeOffset { ctx })
            }
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            } => self.block.row_mut(BlockCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            }),
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr } => self
                .block
                .row_mut(BlockCdfSelector::IntraTxTypeSet1 { tx_size_sqr }),
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr } => self
                .block
                .row_mut(BlockCdfSelector::IntraTxTypeSet2 { tx_size_sqr }),
            TileCdfSelector::IsLongSideDct { is_inter } => self
                .block
                .row_mut(BlockCdfSelector::IsLongSideDct { is_inter }),
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr } => self
                .block
                .row_mut(BlockCdfSelector::IntraTxTypeLong { tx_size_sqr }),
            TileCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            } => self.block.row_mut(BlockCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            }),
            TileCdfSelector::MostProbableStxSet => {
                self.block.row_mut(BlockCdfSelector::MostProbableStxSet)
            }
            TileCdfSelector::MostProbableStxSetAdst => {
                self.block.row_mut(BlockCdfSelector::MostProbableStxSetAdst)
            }
            TileCdfSelector::CctxType => self.block.row_mut(BlockCdfSelector::CctxType),
            TileCdfSelector::UvModeCflNotAllowed { ctx } => self
                .block
                .row_mut(BlockCdfSelector::UvModeCflNotAllowed { ctx }),
            TileCdfSelector::IsCfl { ctx } => self.block.row_mut(BlockCdfSelector::IsCfl { ctx }),
            TileCdfSelector::CflIndex => self.block.row_mut(BlockCdfSelector::CflIndex),
            TileCdfSelector::CflSign => self.block.row_mut(BlockCdfSelector::CflSign),
            TileCdfSelector::CflAlpha { ctx } => {
                self.block.row_mut(BlockCdfSelector::CflAlpha { ctx })
            }
            TileCdfSelector::CflMhccp => self.block.row_mut(BlockCdfSelector::CflMhccp),
            TileCdfSelector::CflMhDir { size_group } => self
                .block
                .row_mut(BlockCdfSelector::CflMhDir { size_group }),
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => self.block.row_mut(BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            }),
            TileCdfSelector::EobExtra { coeff_cdf_q_ctx } => self
                .block
                .row_mut(BlockCdfSelector::EobExtra { coeff_cdf_q_ctx }),
            TileCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => self.block.row_mut(BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            }),
            TileCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            } => self.block.row_mut(BlockCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            }),
            TileCdfSelector::IsInter { ctx } => {
                self.block.row_mut(BlockCdfSelector::IsInter { ctx })
            }
            TileCdfSelector::Skip { ctx } => self.block.row_mut(BlockCdfSelector::Skip { ctx }),
            TileCdfSelector::SingleMode { ctx } => {
                self.block.row_mut(BlockCdfSelector::SingleMode { ctx })
            }
            TileCdfSelector::DrlMode { idx, ctx } => {
                self.block.row_mut(BlockCdfSelector::DrlMode { idx, ctx })
            }
            TileCdfSelector::SingleRef { ctx, ref_idx } => self
                .block
                .row_mut(BlockCdfSelector::SingleRef { ctx, ref_idx }),
            TileCdfSelector::CompMode { ctx } => {
                self.block.row_mut(BlockCdfSelector::CompMode { ctx })
            }
            TileCdfSelector::IsJoint { ctx } => {
                self.block.row_mut(BlockCdfSelector::IsJoint { ctx })
            }
            TileCdfSelector::CompoundModeNonJoint { ctx } => self
                .block
                .row_mut(BlockCdfSelector::CompoundModeNonJoint { ctx }),
            TileCdfSelector::CompGroupIdx { ctx } => {
                self.block.row_mut(BlockCdfSelector::CompGroupIdx { ctx })
            }
            TileCdfSelector::CwpIdx { idx } => self.block.row_mut(BlockCdfSelector::CwpIdx { idx }),
            TileCdfSelector::CompRef0 { ctx, ref_idx } => self
                .block
                .row_mut(BlockCdfSelector::CompRef0 { ctx, ref_idx }),
            TileCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => self.block.row_mut(BlockCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            }),
            TileCdfSelector::ReadMv(selector) => {
                self.block.row_mut(BlockCdfSelector::ReadMv(selector))
            }
            TileCdfSelector::InterpFilter { ctx } => {
                self.block.row_mut(BlockCdfSelector::InterpFilter { ctx })
            }
            TileCdfSelector::UseWienerNs => self.block.row_mut(BlockCdfSelector::UseWienerNs),
            TileCdfSelector::WienerNsLength { plane_ctx } => self
                .block
                .row_mut(BlockCdfSelector::WienerNsLength { plane_ctx }),
            TileCdfSelector::WienerNsUvSym => self.block.row_mut(BlockCdfSelector::WienerNsUvSym),
            TileCdfSelector::WienerNsBase => self.block.row_mut(BlockCdfSelector::WienerNsBase),
            TileCdfSelector::Coeff(selector) => {
                self.block.row_mut(BlockCdfSelector::Coeff(selector))
            }
        }
    }

    fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        for plane in 0..DO_SPLIT_PLANE_CONTEXTS {
            for ctx in 0..DO_SPLIT_CONTEXTS {
                avg_cdf_row(
                    &mut self.do_split[plane][ctx],
                    &tile.do_split[plane][ctx],
                    tile_num,
                    num_log2,
                );
            }
            for ctx in 0..DO_EXT_PARTITION_CONTEXTS {
                avg_cdf_row(
                    &mut self.do_ext_partition[plane][ctx],
                    &tile.do_ext_partition[plane][ctx],
                    tile_num,
                    num_log2,
                );
            }
            for ctx in 0..DO_SQUARE_SPLIT_CONTEXTS {
                avg_cdf_row(
                    &mut self.do_square_split[plane][ctx],
                    &tile.do_square_split[plane][ctx],
                    tile_num,
                    num_log2,
                );
            }
            for ctx in 0..RECT_TYPE_CONTEXTS {
                avg_cdf_row(
                    &mut self.rect_type[plane][ctx],
                    &tile.rect_type[plane][ctx],
                    tile_num,
                    num_log2,
                );
            }
            for ctx in 0..DO_UNEVEN_4WAY_PARTITION_CONTEXTS {
                avg_cdf_row(
                    &mut self.do_uneven_4way_partition[plane][ctx],
                    &tile.do_uneven_4way_partition[plane][ctx],
                    tile_num,
                    num_log2,
                );
            }
        }
        for fsc_mode in 0..TX_FSC_CONTEXTS {
            for is_inter in 0..TX_IS_INTER_CONTEXTS {
                for ctx in 0..TXFM_SPLIT_GROUPS {
                    avg_cdf_row(
                        &mut self.tx_do_partition[fsc_mode][is_inter][ctx],
                        &tile.tx_do_partition[fsc_mode][is_inter][ctx],
                        tile_num,
                        num_log2,
                    );
                }
                for ctx in 0..TX_2OR3_PARTITION_TYPE_CONTEXTS {
                    avg_cdf_row(
                        &mut self.tx_2or3_partition_type[fsc_mode][is_inter][ctx],
                        &tile.tx_2or3_partition_type[fsc_mode][is_inter][ctx],
                        tile_num,
                        num_log2,
                    );
                }
                for ctx in 0..TX_PARTITION_TYPE_CONTEXTS {
                    avg_cdf_row(
                        &mut self.tx_partition_type[fsc_mode][is_inter][ctx],
                        &tile.tx_partition_type[fsc_mode][is_inter][ctx],
                        tile_num,
                        num_log2,
                    );
                    avg_cdf_row(
                        &mut self.tx_partition_type_reduced[fsc_mode][is_inter][ctx],
                        &tile.tx_partition_type_reduced[fsc_mode][is_inter][ctx],
                        tile_num,
                        num_log2,
                    );
                }
            }
        }
        avg_cdf_row(&mut self.delta_q, &tile.delta_q, tile_num, num_log2);
        for ctx in 0..CDEF_STRENGTH_INDEX0_CONTEXTS {
            avg_cdf_row(
                &mut self.cdef_index0[ctx],
                &tile.cdef_index0[ctx],
                tile_num,
                num_log2,
            );
        }
        for plane in 0..CCSO_PLANES {
            for ctx in 0..CCSO_CONTEXTS {
                avg_cdf_row(
                    &mut self.ccso_blk[plane][ctx],
                    &tile.ccso_blk[plane][ctx],
                    tile_num,
                    num_log2,
                );
            }
        }
        avg_cdf_row(
            &mut self.cdef_index_minus1_with3,
            &tile.cdef_index_minus1_with3,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.cdef_index_minus1_with4,
            &tile.cdef_index_minus1_with4,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.cdef_index_minus1_with5,
            &tile.cdef_index_minus1_with5,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.cdef_index_minus1_with6,
            &tile.cdef_index_minus1_with6,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.cdef_index_minus1_with7,
            &tile.cdef_index_minus1_with7,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.cdef_index_minus1_with8,
            &tile.cdef_index_minus1_with8,
            tile_num,
            num_log2,
        );
        for ctx in 0..INTRABC_CONTEXTS {
            avg_cdf_row(
                &mut self.intrabc[ctx],
                &tile.intrabc[ctx],
                tile_num,
                num_log2,
            );
        }
        avg_cdf_row(
            &mut self.intrabc_mode,
            &tile.intrabc_mode,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.intrabc_precision,
            &tile.intrabc_precision,
            tile_num,
            num_log2,
        );
        for ctx in 0..FSC_MODE_CONTEXTS {
            for bsize_group in 0..FSC_BSIZE_CONTEXTS {
                avg_cdf_row(
                    &mut self.fsc_mode[ctx][bsize_group],
                    &tile.fsc_mode[ctx][bsize_group],
                    tile_num,
                    num_log2,
                );
            }
        }
        for ctx in 0..MRL_INDEX_CONTEXTS {
            avg_cdf_row(
                &mut self.mrl_index[ctx],
                &tile.mrl_index[ctx],
                tile_num,
                num_log2,
            );
            avg_cdf_row(
                &mut self.mrl_sec_index[ctx],
                &tile.mrl_sec_index[ctx],
                tile_num,
                num_log2,
            );
        }
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

fn checked_plane(array: TileCdfArray, plane_start: usize) -> Result<usize, TileCdfError> {
    checked_plane_within(array, plane_start, DO_SPLIT_PLANE_CONTEXTS)
}

/// § 8.3.2 fixes `do_square_split` `PlaneStart` at 0 (the chroma partition is
/// forced for the large block sizes where it is read), so only plane 0 is valid
/// for that selector — tighter than the shared 2-plane partition CDF array bound.
fn checked_square_split_plane(plane_start: usize) -> Result<usize, TileCdfError> {
    checked_plane_within(
        TileCdfArray::DoSquareSplit,
        plane_start,
        DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS,
    )
}

fn checked_plane_within(
    array: TileCdfArray,
    plane_start: usize,
    max_exclusive: usize,
) -> Result<usize, TileCdfError> {
    if plane_start >= max_exclusive {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "plane_start",
            actual: plane_start,
            max_exclusive,
        });
    }
    Ok(plane_start)
}

fn checked_context(
    array: TileCdfArray,
    index_name: &'static str,
    actual: usize,
    max_exclusive: usize,
) -> Result<usize, TileCdfError> {
    if actual >= max_exclusive {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name,
            actual,
            max_exclusive,
        });
    }
    Ok(actual)
}

const fn tx_partition_type_array(reduced: bool) -> TileCdfArray {
    if reduced {
        TileCdfArray::TxPartitionTypeReduced
    } else {
        TileCdfArray::TxPartitionType
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

fn avg_cdf_row<const N: usize>(
    cdf: &mut [i32; N],
    tile_cdf: &[i32; N],
    tile_num: u32,
    num_log2: u8,
) {
    if tile_num == 0 {
        for value in &mut cdf[..N - 2] {
            *value = CDF_PROB_SCALE;
        }
        cdf[N - 2] = tile_cdf[N - 2];
        cdf[N - 1] = 0;
    }
    let shift = u32::from(num_log2);
    for i in 0..N - 2 {
        cdf[i] -= (CDF_PROB_SCALE - tile_cdf[i]) >> shift;
    }
    cdf[N - 1] += tile_cdf[N - 1] >> shift;
}

fn scale_cdf_count<const N: usize>(cdf: &mut [i32; N]) {
    cdf[N - 1] = cdf[N - 1].saturating_mul(3) >> 2;
}

const fn floor_log2(value: u32) -> u32 {
    u32::BITS - 1 - value.leading_zeros()
}

#[cfg(test)]
mod tests;
