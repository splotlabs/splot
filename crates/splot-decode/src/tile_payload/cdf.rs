// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile CDF selection and lifecycle boundaries.
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY` and
//! `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.

pub(crate) mod block_context;
pub(crate) mod block_read;
mod block_rows;
pub(crate) mod context;
mod lifecycle;
pub(crate) mod partition_read;

use core::fmt;

use splot_core::symbol::CdfUpdateMode;
use splot_core::tables::cdf::{
    DEFAULT_DO_EXT_PARTITION_CDF, DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF,
    DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF, DEFAULT_RECT_TYPE_CDF,
};

use self::block_rows::{BlockCdfRows, BlockCdfSelector};

const CDF_PROB_SCALE: i32 = 1 << 15;
const DO_SPLIT_PLANE_CONTEXTS: usize = 2;
const DO_SPLIT_CONTEXTS: usize = 64;
const DO_EXT_PARTITION_CONTEXTS: usize = 64;
const DO_UNEVEN_4WAY_PARTITION_CONTEXTS: usize = 64;
const RECT_TYPE_CONTEXTS: usize = 64;
const DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS: usize = 1;
const DO_SQUARE_SPLIT_CONTEXTS: usize = 8;
const CDF_ROW_LEN: usize = 3;

type DoSplitCdfRows = [[[i32; CDF_ROW_LEN]; DO_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoExtPartitionCdfRows =
    [[[i32; CDF_ROW_LEN]; DO_EXT_PARTITION_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoSquareSplitCdfRows =
    [[[i32; CDF_ROW_LEN]; DO_SQUARE_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoUneven4WayPartitionCdfRows =
    [[[i32; CDF_ROW_LEN]; DO_UNEVEN_4WAY_PARTITION_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type RectTypeCdfRows = [[[i32; CDF_ROW_LEN]; RECT_TYPE_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];

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
    /// `TileYModeSetCdf` from AV2 § 8.3.2 for the minimal intra trace.
    YModeSet,
    /// `TileYModeIndexCdf[ctx]` from AV2 § 8.3.2 for the minimal intra trace.
    YModeIndex {
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
    /// `TileUvModeCflNotAllowedCdf[ctx]` from AV2 § 8.3.2.
    UvModeCflNotAllowed {
        /// Chroma mode context index.
        ctx: usize,
    },
    /// `TileVTxbSkipCdf[coeff_cdf_q_ctx][ctx]`.
    VTxbSkip {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// V-plane transform-skip context index.
        ctx: usize,
    },
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
    /// `TileYModeIndexCdf`.
    YModeIndex,
    /// `TileTxbSkipCdf`.
    TxbSkip,
    /// `TileUvModeCflNotAllowedCdf`.
    UvModeCflNotAllowed,
    /// `TileVTxbSkipCdf`.
    VTxbSkip,
}

impl TileCdfArray {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DoSplit => "TileDoSplitCdf",
            Self::DoExtPartition => "TileDoExtPartitionCdf",
            Self::DoSquareSplit => "TileDoSquareSplitCdf",
            Self::RectType => "TileRectTypeCdf",
            Self::DoUneven4WayPartition => "TileDoUneven4wayPartitionCdf",
            Self::YModeIndex => "TileYModeIndexCdf",
            Self::TxbSkip => "TileTxbSkipCdf",
            Self::UvModeCflNotAllowed => "TileUvModeCflNotAllowedCdf",
            Self::VTxbSkip => "TileVTxbSkipCdf",
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
            TileCdfSelector::YModeSet => self.block.row(BlockCdfSelector::YModeSet),
            TileCdfSelector::YModeIndex { ctx } => {
                self.block.row(BlockCdfSelector::YModeIndex { ctx })
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
            TileCdfSelector::UvModeCflNotAllowed { ctx } => self
                .block
                .row(BlockCdfSelector::UvModeCflNotAllowed { ctx }),
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => self.block.row(BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            }),
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
            TileCdfSelector::YModeSet => self.block.row_mut(BlockCdfSelector::YModeSet),
            TileCdfSelector::YModeIndex { ctx } => {
                self.block.row_mut(BlockCdfSelector::YModeIndex { ctx })
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
            TileCdfSelector::UvModeCflNotAllowed { ctx } => self
                .block
                .row_mut(BlockCdfSelector::UvModeCflNotAllowed { ctx }),
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => self.block.row_mut(BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            }),
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
    pub(crate) const fn uv_mode_cfl_not_allowed(&self) -> &block_rows::UvModeCflNotAllowedCdfRows {
        self.block.uv_mode_cfl_not_allowed()
    }

    #[cfg(test)]
    pub(crate) const fn v_txb_skip(&self) -> &block_rows::VTxbSkipCdfRows {
        self.block.v_txb_skip()
    }
}

fn checked_plane(array: TileCdfArray, plane_start: usize) -> Result<usize, TileCdfError> {
    if plane_start >= DO_SPLIT_PLANE_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "plane_start",
            actual: plane_start,
            max_exclusive: DO_SPLIT_PLANE_CONTEXTS,
        });
    }
    Ok(plane_start)
}

fn checked_square_split_plane(plane_start: usize) -> Result<usize, TileCdfError> {
    if plane_start >= DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSquareSplit,
            index_name: "plane_start",
            actual: plane_start,
            max_exclusive: DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS,
        });
    }
    Ok(plane_start)
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
