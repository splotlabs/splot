// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Crate-private AV2 tile CDF selection boundary.
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.

use core::fmt;

use splot_core::symbol::CdfUpdateMode;
use splot_core::tables::cdf::{DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF};

pub(crate) const TILE_CDF_SELECTION_MATRIX_ROW: &str = "tile-cdf-selection-boundary";
pub(crate) const TILE_CDF_SELECTION_FEATURE_ID: &str = "DECODE-TILE-CDF-SELECTION-BOUNDARY";

const CDF_PROB_SCALE: i32 = 1 << 15;
const DO_SPLIT_PLANE_CONTEXTS: usize = 2;
const DO_SPLIT_CONTEXTS: usize = 64;
const DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS: usize = 1;
const DO_SQUARE_SPLIT_CONTEXTS: usize = 8;
const CDF_ROW_LEN: usize = 3;

type DoSplitCdfRows = [[[i32; CDF_ROW_LEN]; DO_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];
type DoSquareSplitCdfRows =
    [[[i32; CDF_ROW_LEN]; DO_SQUARE_SPLIT_CONTEXTS]; DO_SPLIT_PLANE_CONTEXTS];

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
        Ok(self.rows.row(selector)?.as_slice())
    }

    /// Provides closure-scoped mutable row access for `read_symbol(cdf)`.
    pub(crate) fn with_row_mut<R>(
        &mut self,
        selector: TileCdfSelector,
        f: impl FnOnce(&mut [i32]) -> R,
    ) -> Result<R, TileCdfError> {
        Ok(f(self.rows.row_mut(selector)?.as_mut_slice()))
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

/// Saved CDF subset used only to prove copy/average policy calculation.
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

    /// Applies the recorded copy/average decision for the supported subset.
    pub(crate) fn apply_tile(
        &mut self,
        tile_num: u32,
        tile: &TileCdfSubset,
        policy: TileCdfSavePolicy,
    ) {
        if policy.copy_cdf {
            self.rows = tile.rows.clone();
            return;
        }
        if policy.avg_cdf {
            self.rows
                .avg_from_tile(tile_num, &tile.rows, policy.num_log2);
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
    tile_cdfs: TileCdfSubset,
}

impl TileCdfWorkUnitBoundary {
    /// Creates tile-local CDF boundary metadata.
    #[must_use]
    pub(crate) const fn new(
        update_mode: CdfUpdateMode,
        save_policy: TileCdfSavePolicy,
        tile_cdfs: TileCdfSubset,
    ) -> Self {
        Self {
            update_mode,
            save_policy,
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
}

/// Supported CDF selectors for the first partition-entry boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileCdfSelector {
    /// `TileDoSplitCdf[PlaneStart][ctx]`.
    DoSplit {
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
}

/// Supported CDF arrays for error reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileCdfArray {
    /// `TileDoSplitCdf`.
    DoSplit,
    /// `TileDoSquareSplitCdf`.
    DoSquareSplit,
}

impl TileCdfArray {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DoSplit => "TileDoSplitCdf",
            Self::DoSquareSplit => "TileDoSquareSplitCdf",
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
    do_square_split: DoSquareSplitCdfRows,
}

impl TileCdfRows {
    fn from_defaults() -> Self {
        Self {
            do_split: DEFAULT_DO_SPLIT_CDF,
            do_square_split: DEFAULT_DO_SQUARE_SPLIT_CDF,
        }
    }

    fn row(&self, selector: TileCdfSelector) -> Result<&[i32; CDF_ROW_LEN], TileCdfError> {
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
                Ok(row)
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
                Ok(row)
            }
        }
    }

    fn row_mut(
        &mut self,
        selector: TileCdfSelector,
    ) -> Result<&mut [i32; CDF_ROW_LEN], TileCdfError> {
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
                Ok(row)
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
                Ok(row)
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
            for ctx in 0..DO_SQUARE_SPLIT_CONTEXTS {
                avg_cdf_row(
                    &mut self.do_square_split[plane][ctx],
                    &tile.do_square_split[plane][ctx],
                    tile_num,
                    num_log2,
                );
            }
        }
    }

    #[cfg(test)]
    pub(crate) const fn do_split(&self) -> &DoSplitCdfRows {
        &self.do_split
    }

    #[cfg(test)]
    pub(crate) const fn do_square_split(&self) -> &DoSquareSplitCdfRows {
        &self.do_square_split
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

fn avg_cdf_row(
    cdf: &mut [i32; CDF_ROW_LEN],
    tile_cdf: &[i32; CDF_ROW_LEN],
    tile_num: u32,
    num_log2: u8,
) {
    if tile_num == 0 {
        cdf[0] = CDF_PROB_SCALE;
        cdf[1] = tile_cdf[1];
        cdf[2] = 0;
    }
    let shift = u32::from(num_log2);
    cdf[0] -= (CDF_PROB_SCALE - tile_cdf[0]) >> shift;
    cdf[2] += tile_cdf[2] >> shift;
}

const fn floor_log2(value: u32) -> u32 {
    u32::BITS - 1 - value.leading_zeros()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use splot_core::span::ByteOffset;
    use splot_core::symbol::{SymbolDecoder, SymbolDecoderConfig};

    #[test]
    fn frame_cdf_subset_copies_generated_defaults_without_aliasing() {
        let frame = FrameCdfSubset::from_defaults();
        assert_eq!(frame.rows().do_split(), &DEFAULT_DO_SPLIT_CDF);
        assert_eq!(frame.rows().do_square_split(), &DEFAULT_DO_SQUARE_SPLIT_CDF);

        let mut tile = frame.tile_copy();
        tile.rows_mut().do_split[0][0][0] = 1234;

        assert_eq!(frame.rows().do_split()[0][0], DEFAULT_DO_SPLIT_CDF[0][0]);
        assert_ne!(
            tile.row(TileCdfSelector::DoSplit {
                plane_start: 0,
                ctx: 0
            })
            .unwrap(),
            DEFAULT_DO_SPLIT_CDF[0][0].as_slice()
        );
    }

    #[test]
    fn selector_returns_rows_and_bounds_errors() {
        let frame = FrameCdfSubset::from_defaults();
        let mut tile = frame.tile_copy();
        let row = tile
            .row(TileCdfSelector::DoSplit {
                plane_start: 0,
                ctx: 0,
            })
            .unwrap();
        assert_eq!(row, DEFAULT_DO_SPLIT_CDF[0][0].as_slice());
        assert_eq!(row.len(), CDF_ROW_LEN);

        let err = tile
            .with_row_mut(
                TileCdfSelector::DoSquareSplit {
                    plane_start: 1,
                    ctx: 0,
                },
                |_| (),
            )
            .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::DoSquareSplit,
                index_name: "plane_start",
                actual: 1,
                max_exclusive: 1,
            }
        );

        let err = tile
            .row(TileCdfSelector::DoSplit {
                plane_start: 0,
                ctx: 64,
            })
            .unwrap_err();
        assert_eq!(
            err,
            TileCdfError::SelectorOutOfRange {
                array: TileCdfArray::DoSplit,
                index_name: "ctx",
                actual: 64,
                max_exclusive: 64,
            }
        );
    }

    #[test]
    fn selected_row_hands_off_to_symbol_decoder_update_modes() {
        let frame = FrameCdfSubset::from_defaults();
        let selector = TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        };
        let payload = [0x80, 0x00];

        let mut enabled = frame.tile_copy();
        let before = enabled.row(selector).unwrap().to_vec();
        let mut symbol = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap();
        enabled
            .with_row_mut(selector, |row| symbol.read_symbol(row))
            .unwrap()
            .unwrap();
        assert_ne!(enabled.row(selector).unwrap(), before.as_slice());

        let mut disabled = frame.tile_copy();
        let before = disabled.row(selector).unwrap().to_vec();
        let mut symbol = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap();
        disabled
            .with_row_mut(selector, |row| symbol.read_symbol(row))
            .unwrap()
            .unwrap();
        assert_eq!(disabled.row(selector).unwrap(), before.as_slice());
    }

    #[test]
    fn cdf_save_policy_matches_spec() {
        let single =
            tile_cdf_save_policy(TileCdfPolicyInput::new(1, 1, false, false, 0), 0).unwrap();
        assert_eq!(single.num_log2(), 0);
        assert!(single.copy_cdf());
        assert!(!single.avg_cdf());

        let avg = tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, true, true, 0), 2).unwrap();
        assert_eq!(avg.num_log2(), 2);
        assert!(avg.avg_cdf());
        assert!(!avg.copy_cdf());

        let not_averaged =
            tile_cdf_save_policy(TileCdfPolicyInput::new(16, 1, true, true, 0), 8).unwrap();
        assert_eq!(not_averaged.num_log2(), 3);
        assert!(!not_averaged.avg_cdf());

        let context =
            tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, false, false, 3), 3).unwrap();
        assert!(context.copy_cdf());

        assert!(matches!(
            tile_cdf_save_policy(TileCdfPolicyInput::new(u32::MAX, 2, false, false, 0), 0),
            Err(TileCdfError::TileCountOverflow { .. })
        ));
        assert!(matches!(
            tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, false, false, 4), 0),
            Err(TileCdfError::ContextUpdateTileOutOfRange { .. })
        ));
    }

    #[test]
    fn saved_copy_and_average_are_exact_for_supported_subset() {
        let frame = FrameCdfSubset::from_defaults();
        let mut tile = frame.tile_copy();
        tile.rows_mut().do_split[0][0] = [20_000, 7, 4];
        tile.rows_mut().do_square_split[0][0] = [21_000, 6, 2];

        let mut saved = SavedCdfSubset::from_frame(&frame);
        saved.apply_tile(
            0,
            &tile,
            TileCdfSavePolicy {
                num_log2: 0,
                copy_cdf: true,
                avg_cdf: false,
            },
        );
        assert_eq!(saved.rows(), tile.rows());

        let mut saved = SavedCdfSubset::from_frame(&frame);
        saved.apply_tile(
            0,
            &tile,
            TileCdfSavePolicy {
                num_log2: 2,
                copy_cdf: false,
                avg_cdf: true,
            },
        );
        assert_eq!(saved.rows().do_split()[0][0], [29_576, 7, 1]);
        assert_eq!(saved.rows().do_square_split()[0][0], [29_826, 6, 0]);
    }
}
