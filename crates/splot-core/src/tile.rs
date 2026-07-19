// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 tile-partitioning helpers and the `tile_params()` syntax (AV2 v1.0.0
//! § 5.18.7.3, with the § 5.18.7.5 `uniform_spacing` and § 5.18.7.7 `tile_log2`
//! helpers and the § 9.3 / level-tier conversion tables).
//!
//! This is the reusable tile-partitioning foundation. It is wired into the sequence
//! tile config (§ 5.4.2) and the frame-level `tile_info()` (§ 5.18.7.2,
//! [`crate::headers::frame`]), which also uses the § 5.18.7.4 `reuse_tile_params`
//! helper here. The parser reads syntax only and performs no tile decoding.

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::headers::sequence::{LevelIdx, SuperblockSize, Tier};

/// `MAX_TILE_COLS`: maximum number of tile columns (AV2 v1.0.0 § 3).
pub const MAX_TILE_COLS: u32 = 64;
/// `MAX_TILE_ROWS`: maximum number of tile rows (AV2 v1.0.0 § 3).
pub const MAX_TILE_ROWS: u32 = 64;
/// `MAX_TILE_WIDTH`: maximum width of a tile in luma samples (AV2 v1.0.0 § 3).
pub const MAX_TILE_WIDTH: u32 = 4096;
/// `MAX_TILE_AREA`: maximum area of a tile in luma samples (AV2 v1.0.0 § 3).
pub const MAX_TILE_AREA: u32 = 4096 * 2304;

/// `seq_level_idx` value reserved for the unconstrained (no-level) tile case
/// (AV2 v1.0.0 § 5.18.7.3: `if (seq_level_idx != 31)`).
const NO_LEVEL_IDX: u8 = 31;

/// `Num_4x4_Blocks_Wide[sbSize]` for the three sequence superblock sizes (AV2 v1.0.0
/// § 9.3 conversion tables; confirmed against AVM `mi_size_wide`).
#[must_use]
pub const fn num_4x4_blocks_wide(sb_size: SuperblockSize) -> u32 {
    match sb_size {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 => 32,
        SuperblockSize::Block256x256 => 64,
    }
}

/// `Mi_Width_Log2[sbSize]` for the three sequence superblock sizes (AV2 v1.0.0 § 9.3
/// conversion tables; confirmed against AVM `mi_size_wide_log2`).
#[must_use]
pub const fn mi_width_log2(sb_size: SuperblockSize) -> u32 {
    match sb_size {
        SuperblockSize::Block64x64 => 4,
        SuperblockSize::Block128x128 => 5,
        SuperblockSize::Block256x256 => 6,
    }
}

/// `Tile_Width_Scaling_Factor[2][31]` (AV2 v1.0.0 § A): indexed by `seq_tier`
/// (Main = 0, High = 1) and `seq_level_idx` (0..=30). Reserved level indices (22..=30)
/// are `0` and are treated as "no defined scaling".
const TILE_WIDTH_SCALING_FACTOR: [[u32; 31]; 2] = [
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 8, 8, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 8, 8, 8, 16, 16, 16, 16, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ],
];

/// `Tile_Area_Scaling_Factor[2][31]` (AV2 v1.0.0 § A): indexed by `seq_tier` and
/// `seq_level_idx` (0..=30). Reserved level indices (22..=30) are `0`.
const TILE_AREA_SCALING_FACTOR: [[u32; 31]; 2] = [
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 8, 8, 8, 8, 16, 16, 16, 16, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ],
    [
        4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 16, 16, 16, 16, 32, 32, 32, 32, 0, 0, 0, 0, 0, 0,
        0, 0, 0,
    ],
];

/// Returns the tier table index (Main = 0, High = 1).
const fn tier_index(tier: Tier) -> usize {
    match tier {
        Tier::Main => 0,
        Tier::High => 1,
    }
}

/// Returns the paired width/area scaling factors, or `None` for a reserved level.
pub(crate) fn tile_scaling_factors(tier: Tier, level_idx: u8) -> Option<(u32, u32)> {
    let tier = tier_index(tier);
    let width = *TILE_WIDTH_SCALING_FACTOR[tier].get(level_idx as usize)?;
    let area = *TILE_AREA_SCALING_FACTOR[tier].get(level_idx as usize)?;
    (width != 0 && area != 0).then_some((width, area))
}

/// `tile_log2(blkSize, target)` (AV2 v1.0.0 § 5.18.7.7): the smallest `k` such that
/// `blkSize << k >= target`.
///
/// `k` is capped at 32 so a degenerate `blkSize == 0` (which the spec loop would
/// never terminate on) returns a bounded value instead of looping forever; real
/// call sites always pass `blkSize >= 1`.
#[must_use]
pub fn tile_log2(blk_size: u32, target: u32) -> u8 {
    let mut k = 0u32;
    let mut scaled = blk_size;
    while k < 32 && scaled < target {
        scaled = scaled.saturating_mul(2);
        k += 1;
    }
    k as u8
}

/// The superblock column/row starts produced by `uniform_spacing()` (AV2 v1.0.0
/// § 5.18.7.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSpacing {
    /// Superblock start positions, one per tile.
    pub starts: Vec<u32>,
    /// Number of tiles produced (`i` at loop exit).
    pub count: u32,
}

/// `uniform_spacing(tileLog2, mis, sbSize)` (AV2 v1.0.0 § 5.18.7.5): distributes `mis`
/// mode-info units across `1 << tileLog2` roughly-equal tiles and returns their
/// superblock start positions and count.
///
/// AV2 syntax always passes a `mis` bounded by the sequence-header frame-dimension
/// limit (≤ ~131072 for the widest legal frame), but the rounding-up arithmetic uses
/// `saturating_add` so the function stays panic-free for an arbitrary `u32`.
#[must_use]
pub fn uniform_spacing(tile_log2: u8, mis: u32, sb_size: SuperblockSize) -> TileSpacing {
    let sb4x4 = num_4x4_blocks_wide(sb_size);
    let sb_shift = mi_width_log2(sb_size);
    let sbs = mis.saturating_add(sb4x4 - 1) >> sb_shift;
    let full_sbs = mis >> sb_shift;
    let tile_log2 = u32::from(tile_log2).min(31);
    let tile_sb = full_sbs >> tile_log2;
    let extra_sbs = if tile_sb == 0 {
        sbs
    } else {
        full_sbs - (tile_sb << tile_log2)
    };

    let num_tiles = 1u32 << tile_log2;
    let mut starts = Vec::with_capacity(num_tiles.min(sbs) as usize);
    let mut start_sb = 0u32;
    let mut i = 0u32;
    while i < num_tiles && start_sb < sbs {
        starts.push(start_sb);
        start_sb += tile_sb;
        if i < extra_sbs {
            start_sb += 1;
        }
        i += 1;
    }

    TileSpacing { starts, count: i }
}

/// Inputs to [`parse_tile_params`] (AV2 v1.0.0 § 5.18.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileParamsInput {
    /// `frameWidth` in luma samples.
    pub frame_width: u32,
    /// `frameHeight` in luma samples.
    pub frame_height: u32,
    /// `uniformSbSize` (the superblock size used for uniform spacing).
    pub uniform_sb_size: SuperblockSize,
    /// `sbSize` (the superblock size used for the column/row superblock grid).
    pub sb_size: SuperblockSize,
    /// `isBridge`: bridge frames infer `uniform_tile_spacing_flag = 1`.
    pub is_bridge: bool,
    /// `seq_tier`, used for the level/tier scaling tables.
    pub seq_tier: Tier,
    /// `seq_level_idx`, used for the level/tier scaling tables.
    pub seq_level_idx: LevelIdx,
}

/// Parsed `tile_params()` result (AV2 v1.0.0 § 5.18.7.3).
///
/// Holds the derived tile counts and log2 sizes, the superblock grid dimensions, the
/// uniform-spacing flag, and whether the column/row tile starts covered the frame
/// exactly (`covers_cols` / `covers_rows`). For any decodable stream the `ns()`-bounded
/// non-uniform loops make coverage exact, so the coverage flags are a defensive,
/// never-false-positive cross-check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileParams {
    /// `TileCols`.
    pub tile_cols: u32,
    /// `TileRows`.
    pub tile_rows: u32,
    /// `TileColsLog2`.
    pub tile_cols_log2: u8,
    /// `TileRowsLog2`.
    pub tile_rows_log2: u8,
    /// `sbCols`: frame width in superblocks.
    pub sb_cols: u32,
    /// `sbRows`: frame height in superblocks.
    pub sb_rows: u32,
    /// `uniform_tile_spacing_flag`.
    pub uniform_spacing: bool,
    /// Whether the tile column starts summed to exactly `sbCols` (always `true` for a
    /// uniform layout; the non-uniform `ns()` bound also guarantees it for decodable
    /// streams).
    pub covers_cols: bool,
    /// Whether the tile row starts summed to exactly `sbRows`.
    pub covers_rows: bool,
}

/// Full `tile_params()` result (AV2 v1.0.0 § 5.18.7.3): the [`TileParams`] summary
/// plus the superblock start arrays and the returned `sbShift`, which the
/// frame-level `tile_info()` (§ 5.18.7.2) needs to derive `MiColStarts` /
/// `MiRowStarts` (`MiColStarts[i] = sbColStarts[i] << sbShift2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileLayout {
    /// The derived tile counts, log2 sizes, grid dimensions, and coverage flags.
    pub params: TileParams,
    /// `sbColStarts[0..TileCols]`.
    pub sb_col_starts: Vec<u32>,
    /// `sbRowStarts[0..TileRows]`.
    pub sb_row_starts: Vec<u32>,
    /// The returned `sbShift` (`Mi_Width_Log2[uniformSbSize]` for a uniform layout —
    /// reassigned inside the uniform branch — else `Mi_Width_Log2[sbSize]`), the
    /// `sbShift2` of the § 5.18.7.2 caller.
    pub sb_shift2: u32,
}

/// Parses `tile_params(frameWidth, frameHeight, uniformSbSize, sbSize, isBridge)`
/// (AV2 v1.0.0 § 5.18.7.3), discarding the superblock start arrays.
///
/// Use [`parse_tile_layout`] when the start arrays and `sbShift` are needed (the
/// frame-level `tile_info()`, § 5.18.7.2).
///
/// # Errors
/// Returns [`Error::Unimplemented`] when `seq_level_idx` is a reserved level index
/// (no defined scaling factor, so the bit layout is undefined — a non-conformant
/// stream). Returns descriptor errors or
/// [`Error::UnexpectedEof`] if the payload ends
/// mid-field.
pub fn parse_tile_params(reader: &mut BitReader<'_>, input: TileParamsInput) -> Result<TileParams> {
    parse_tile_layout(reader, input).map(|layout| layout.params)
}

/// Parses `tile_params(frameWidth, frameHeight, uniformSbSize, sbSize, isBridge)`
/// (AV2 v1.0.0 § 5.18.7.3), keeping the superblock start arrays and the returned
/// `sbShift` for the § 5.18.7.2 `MiColStarts` / `MiRowStarts` derivation.
///
/// # Errors
/// Returns [`Error::Unimplemented`] when `seq_level_idx` is a reserved level index
/// (no defined scaling factor, so the bit layout is undefined — a non-conformant
/// stream). Returns descriptor errors or
/// [`Error::UnexpectedEof`] if the payload ends
/// mid-field.
pub fn parse_tile_layout(reader: &mut BitReader<'_>, input: TileParamsInput) -> Result<TileLayout> {
    let sb4x4 = num_4x4_blocks_wide(input.sb_size);
    let sb_shift = mi_width_log2(input.sb_size);
    let mi_cols = 2 * (input.frame_width.saturating_add(7) >> 3);
    let mi_rows = 2 * (input.frame_height.saturating_add(7) >> 3);
    let sb_cols = mi_cols.saturating_add(sb4x4 - 1) >> sb_shift;
    let sb_rows = mi_rows.saturating_add(sb4x4 - 1) >> sb_shift;

    let level_idx = input.seq_level_idx.get();
    let (max_tile_width_sb, mut max_tile_area_sb) = if level_idx != NO_LEVEL_IDX {
        let (width_sf, area_sf) =
            tile_scaling_factors(input.seq_tier, level_idx).ok_or(Error::Unimplemented {
                feature: "AV2-5.18.7.3-TILE-PARAMS",
            })?;
        let max_tile_width_sb = (width_sf * MAX_TILE_WIDTH) >> (sb_shift + 4);
        let max_tile_area_sb = (area_sf * MAX_TILE_AREA) >> (2 * (sb_shift + 2) + 2);
        (max_tile_width_sb, max_tile_area_sb)
    } else {
        (sb_cols, sb_cols.saturating_mul(sb_rows))
    };

    let min_log2_tile_cols = tile_log2(max_tile_width_sb, sb_cols);
    let max_log2_tile_cols = tile_log2(1, sb_cols.min(MAX_TILE_COLS));
    let max_log2_tile_rows = tile_log2(1, sb_rows.min(MAX_TILE_ROWS));
    let min_log2_tiles =
        min_log2_tile_cols.max(tile_log2(max_tile_area_sb, sb_rows.saturating_mul(sb_cols)));

    let uniform = if input.is_bridge {
        true
    } else {
        reader.read_flag()?
    };

    let (
        tile_cols,
        tile_cols_log2,
        tile_rows,
        sb_col_starts,
        sb_row_starts,
        covers_cols,
        covers_rows,
    ) = if uniform {
        let mut tile_cols_log2 = min_log2_tile_cols;
        if !input.is_bridge {
            while tile_cols_log2 < max_log2_tile_cols {
                if reader.read_bit()? == 1 {
                    tile_cols_log2 += 1;
                } else {
                    break;
                }
            }
        }
        let cols = uniform_spacing(tile_cols_log2, mi_cols, input.uniform_sb_size);
        let tile_cols = cols.count;
        let tile_cols_log2 = tile_log2(1, tile_cols);

        let min_log2_tile_rows = min_log2_tiles.saturating_sub(tile_cols_log2);
        let mut tile_rows_log2 = min_log2_tile_rows;
        if !input.is_bridge {
            while tile_rows_log2 < max_log2_tile_rows {
                if reader.read_bit()? == 1 {
                    tile_rows_log2 += 1;
                } else {
                    break;
                }
            }
        }
        let rows = uniform_spacing(tile_rows_log2, mi_rows, input.uniform_sb_size);
        (
            tile_cols,
            tile_cols_log2,
            rows.count,
            cols.starts,
            rows.starts,
            true,
            true,
        )
    } else {
        let mut widest_tile_sb = 1u32;
        let mut start_sb = 0u32;
        let mut tile_cols = 0u32;
        let mut sb_col_starts = Vec::new();
        while start_sb < sb_cols {
            sb_col_starts.push(start_sb);
            let n = (sb_cols - start_sb).min(max_tile_width_sb);
            let width_in_sbs_minus_1 = reader.read_ns(n)?;
            let size_sb = width_in_sbs_minus_1 + 1;
            widest_tile_sb = widest_tile_sb.max(size_sb);
            start_sb += size_sb;
            tile_cols += 1;
        }
        let covers_cols = start_sb == sb_cols;
        let tile_cols_log2 = tile_log2(1, tile_cols);

        if min_log2_tiles > 0 {
            max_tile_area_sb = sb_rows.saturating_mul(sb_cols) >> (u32::from(min_log2_tiles) + 1);
        } else {
            max_tile_area_sb = sb_rows.saturating_mul(sb_cols);
        }
        let max_tile_height_sb = (max_tile_area_sb / widest_tile_sb).max(1);

        let mut start_sb = 0u32;
        let mut tile_rows = 0u32;
        let mut sb_row_starts = Vec::new();
        while start_sb < sb_rows {
            sb_row_starts.push(start_sb);
            let max_height = (sb_rows - start_sb).min(max_tile_height_sb);
            let height_in_sbs_minus_1 = reader.read_ns(max_height)?;
            let size_sb = height_in_sbs_minus_1 + 1;
            start_sb += size_sb;
            tile_rows += 1;
        }
        let covers_rows = start_sb == sb_rows;
        (
            tile_cols,
            tile_cols_log2,
            tile_rows,
            sb_col_starts,
            sb_row_starts,
            covers_cols,
            covers_rows,
        )
    };

    let tile_rows_log2 = tile_log2(1, tile_rows);

    let sb_shift2 = if uniform {
        mi_width_log2(input.uniform_sb_size)
    } else {
        mi_width_log2(input.sb_size)
    };

    Ok(TileLayout {
        params: TileParams {
            tile_cols,
            tile_rows,
            tile_cols_log2,
            tile_rows_log2,
            sb_cols,
            sb_rows,
            uniform_spacing: uniform,
            covers_cols,
            covers_rows,
        },
        sb_col_starts,
        sb_row_starts,
        sb_shift2,
    })
}

/// Inputs to [`reuse_tile_params`] (AV2 v1.0.0 § 5.18.7.4): the stored sequence tile
/// layout (`SeqSbRowStarts`, `SeqTileRows`, `SeqTileRowsLog2`, `SeqSbColStarts`,
/// `SeqTileCols`, `SeqTileColsLog2` from § 5.18.7.2's call site), the superblock
/// sizes, and the frame `MiCols` / `MiRows` the spec function reads as globals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReuseTileParamsInput<'a> {
    /// `uniformSpacing` (`SeqUniformTileSpacingFlag` at the § 5.18.7.2 call site).
    pub uniform_spacing: bool,
    /// `sbRowStarts` (`SeqSbRowStarts`); only read by the non-uniform branch.
    pub seq_sb_row_starts: &'a [u32],
    /// `tileRows` (`SeqTileRows`); only read by the non-uniform branch.
    pub seq_tile_rows: u32,
    /// `tileRowsLog2` (`SeqTileRowsLog2`).
    pub seq_tile_rows_log2: u8,
    /// `sbColStarts` (`SeqSbColStarts`); only read by the non-uniform branch.
    pub seq_sb_col_starts: &'a [u32],
    /// `tileCols` (`SeqTileCols`); only read by the non-uniform branch.
    pub seq_tile_cols: u32,
    /// `tileColsLog2` (`SeqTileColsLog2`).
    pub seq_tile_cols_log2: u8,
    /// `seqSbSize` (`get_seq_sb_size()`, § 5.18.7.6).
    pub seq_sb_size: SuperblockSize,
    /// `sbSize` (the frame `SbSize`, § 5.18.2).
    pub sb_size: SuperblockSize,
    /// `MiCols` (§ 5.18.4.4 `compute_image_size()`).
    pub mi_cols: u32,
    /// `MiRows` (§ 5.18.4.4 `compute_image_size()`).
    pub mi_rows: u32,
}

/// Result of [`reuse_tile_params`] (AV2 v1.0.0 § 5.18.7.4), mirroring the spec's
/// `(sbRowStarts, tileRows, tileRowsLog2, sbColStarts, tileCols, tileColsLog2,
/// sbShift)` return tuple.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReuseTileParams {
    /// `sbRowStarts[0..tileRows]`.
    pub sb_row_starts: Vec<u32>,
    /// `tileRows`.
    pub tile_rows: u32,
    /// `tileRowsLog2`.
    pub tile_rows_log2: u8,
    /// `sbColStarts[0..tileCols]`.
    pub sb_col_starts: Vec<u32>,
    /// `tileCols`.
    pub tile_cols: u32,
    /// `tileColsLog2`.
    pub tile_cols_log2: u8,
    /// The returned `sbShift` (`Mi_Width_Log2[seqSbSize]` for a uniform layout, else
    /// `Mi_Width_Log2[sbSize]`), the `sbShift2` of the § 5.18.7.2 caller.
    pub sb_shift2: u32,
}

/// `reuse_tile_params(uniformSpacing, sbRowStarts, tileRows, tileRowsLog2,
/// sbColStarts, tileCols, tileColsLog2, seqSbSize, sbSize)` (AV2 v1.0.0
/// § 5.18.7.4): re-derives the frame tile layout from the stored sequence layout.
///
/// The uniform branch recomputes the start arrays via `uniform_spacing()`
/// (§ 5.18.7.5) at the frame `MiCols` / `MiRows`; the non-uniform branch passes the
/// stored start arrays and tile counts through. No bits are read.
#[must_use]
pub fn reuse_tile_params(input: ReuseTileParamsInput<'_>) -> ReuseTileParams {
    if input.uniform_spacing {
        let sb_shift2 = mi_width_log2(input.seq_sb_size);
        let cols = uniform_spacing(input.seq_tile_cols_log2, input.mi_cols, input.seq_sb_size);
        let rows = uniform_spacing(input.seq_tile_rows_log2, input.mi_rows, input.seq_sb_size);
        ReuseTileParams {
            tile_rows_log2: tile_log2(1, rows.count),
            tile_cols_log2: tile_log2(1, cols.count),
            tile_rows: rows.count,
            tile_cols: cols.count,
            sb_row_starts: rows.starts,
            sb_col_starts: cols.starts,
            sb_shift2,
        }
    } else {
        ReuseTileParams {
            sb_row_starts: input.seq_sb_row_starts.to_vec(),
            tile_rows: input.seq_tile_rows,
            tile_rows_log2: tile_log2(1, input.seq_tile_rows),
            sb_col_starts: input.seq_sb_col_starts.to_vec(),
            tile_cols: input.seq_tile_cols,
            tile_cols_log2: tile_log2(1, input.seq_tile_cols),
            sb_shift2: mi_width_log2(input.sb_size),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    fn input(frame_width: u32, frame_height: u32) -> TileParamsInput {
        TileParamsInput {
            frame_width,
            frame_height,
            uniform_sb_size: SuperblockSize::Block64x64,
            sb_size: SuperblockSize::Block64x64,
            is_bridge: false,
            seq_tier: Tier::Main,
            seq_level_idx: LevelIdx::from_bits(0),
        }
    }

    #[test]
    fn tile_log2_returns_smallest_k() {
        assert_eq!(tile_log2(64, 1), 0);
        assert_eq!(tile_log2(1, 1), 0);
        assert_eq!(tile_log2(1, 2), 1);
        assert_eq!(tile_log2(1, 3), 2);
        assert_eq!(tile_log2(1, 64), 6);
        assert_eq!(tile_log2(2, 8), 2);
        assert_eq!(tile_log2(0, 5), 32);
    }

    #[test]
    fn uniform_spacing_single_tile() {
        let spacing = uniform_spacing(0, 4, SuperblockSize::Block64x64);
        assert_eq!(spacing.count, 1);
        assert_eq!(spacing.starts, vec![0]);
    }

    #[test]
    fn uniform_spacing_two_tiles() {
        let spacing = uniform_spacing(1, 32, SuperblockSize::Block64x64);
        assert_eq!(spacing.count, 2);
        assert_eq!(spacing.starts, vec![0, 1]);
    }

    #[test]
    fn parse_uniform_single_tile_reads_only_uniform_flag() {
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_tile_params(&mut reader, input(16, 8)).unwrap();
        assert!(params.uniform_spacing);
        assert_eq!(params.tile_cols, 1);
        assert_eq!(params.tile_rows, 1);
        assert_eq!(params.sb_cols, 1);
        assert_eq!(params.sb_rows, 1);
        assert!(params.covers_cols);
        assert!(params.covers_rows);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn parse_non_uniform_single_tile() {
        let mut bits = Bits::default();
        bits.bit(0); // uniform_tile_spacing_flag = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_tile_params(&mut reader, input(16, 8)).unwrap();
        assert!(!params.uniform_spacing);
        assert_eq!(params.tile_cols, 1);
        assert_eq!(params.tile_rows, 1);
        assert!(params.covers_cols);
        assert!(params.covers_rows);
    }

    #[test]
    fn parse_non_uniform_two_columns() {
        let mut bits = Bits::default();
        bits.bit(0); // uniform_tile_spacing_flag = 0
        bits.bit(0); // ns(2) width_in_sbs_minus_1 = 0 -> first column 1 sb wide
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_tile_params(&mut reader, input(128, 8)).unwrap();
        assert!(!params.uniform_spacing);
        assert_eq!(params.sb_cols, 2);
        assert_eq!(params.tile_cols, 2);
        assert_eq!(params.tile_rows, 1);
        assert!(params.covers_cols);
        assert!(params.covers_rows);
        assert_eq!(reader.consumed_bits(), 2);
    }

    #[test]
    fn tile_layout_exposes_starts_and_sb_shift() {
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        bits.bit(1); // increment_tile_cols_log2 = 1
        bits.bit(0); // increment_tile_cols_log2 = 0
        bits.bit(0); // increment_tile_rows_log2 = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let layout = parse_tile_layout(&mut reader, input(256, 8)).unwrap();
        assert_eq!(layout.params.tile_cols, 2);
        assert_eq!(layout.params.tile_rows, 1);
        assert_eq!(layout.sb_col_starts, vec![0, 2]);
        assert_eq!(layout.sb_row_starts, vec![0]);
        assert_eq!(layout.sb_shift2, 4);
    }

    #[test]
    fn tile_layout_non_uniform_exposes_starts() {
        let mut bits = Bits::default();
        bits.bit(0); // uniform_tile_spacing_flag = 0
        bits.bit(0); // ns(2) width_in_sbs_minus_1 = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let layout = parse_tile_layout(&mut reader, input(128, 8)).unwrap();
        assert_eq!(layout.params.tile_cols, 2);
        assert_eq!(layout.sb_col_starts, vec![0, 1]);
        assert_eq!(layout.sb_row_starts, vec![0]);
        assert_eq!(layout.sb_shift2, 4);
    }

    #[test]
    fn reuse_tile_params_uniform_recomputes_at_frame_size() {
        let result = reuse_tile_params(ReuseTileParamsInput {
            uniform_spacing: true,
            seq_sb_row_starts: &[],
            seq_tile_rows: 0,
            seq_tile_rows_log2: 1,
            seq_sb_col_starts: &[],
            seq_tile_cols: 0,
            seq_tile_cols_log2: 1,
            seq_sb_size: SuperblockSize::Block64x64,
            sb_size: SuperblockSize::Block64x64,
            mi_cols: 64,
            mi_rows: 64,
        });
        assert_eq!(result.tile_cols, 2);
        assert_eq!(result.tile_rows, 2);
        assert_eq!(result.tile_cols_log2, 1);
        assert_eq!(result.tile_rows_log2, 1);
        assert_eq!(result.sb_col_starts, vec![0, 2]);
        assert_eq!(result.sb_row_starts, vec![0, 2]);
        assert_eq!(result.sb_shift2, 4);
    }

    #[test]
    fn reuse_tile_params_non_uniform_passes_starts_through() {
        let result = reuse_tile_params(ReuseTileParamsInput {
            uniform_spacing: false,
            seq_sb_row_starts: &[0, 3],
            seq_tile_rows: 2,
            seq_tile_rows_log2: 1,
            seq_sb_col_starts: &[0, 1, 2],
            seq_tile_cols: 3,
            seq_tile_cols_log2: 2,
            seq_sb_size: SuperblockSize::Block64x64,
            sb_size: SuperblockSize::Block128x128,
            mi_cols: 64,
            mi_rows: 64,
        });
        assert_eq!(result.tile_cols, 3);
        assert_eq!(result.tile_rows, 2);
        assert_eq!(result.tile_cols_log2, 2);
        assert_eq!(result.tile_rows_log2, 1);
        assert_eq!(result.sb_col_starts, vec![0, 1, 2]);
        assert_eq!(result.sb_row_starts, vec![0, 3]);
        assert_eq!(result.sb_shift2, 5);
    }

    #[test]
    fn reserved_level_is_unimplemented_without_reading_bits() {
        let mut reader = BitReader::new(&[0xFF], ByteOffset::new(0));
        let mut tp = input(16, 8);
        tp.seq_level_idx = LevelIdx::from_bits(22);
        assert!(matches!(
            parse_tile_params(&mut reader, tp),
            Err(Error::Unimplemented {
                feature: "AV2-5.18.7.3-TILE-PARAMS"
            })
        ));
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn no_level_idx_uses_unconstrained_fallback() {
        let mut bits = Bits::default();
        bits.bit(1); // uniform_tile_spacing_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let mut tp = input(16, 8);
        tp.seq_level_idx = LevelIdx::from_bits(31);
        let params = parse_tile_params(&mut reader, tp).unwrap();
        assert_eq!(params.tile_cols, 1);
        assert_eq!(params.tile_rows, 1);
    }

    #[test]
    fn reports_eof_without_panicking() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_tile_params(&mut reader, input(16, 8)),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn extreme_frame_dimensions_do_not_panic() {
        let mut tp = input(16, 8);
        tp.frame_width = u32::MAX;
        tp.frame_height = u32::MAX;
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_tile_params(&mut reader, tp),
            Err(Error::UnexpectedEof { .. })
        ));

        tp.seq_level_idx = LevelIdx::from_bits(31);
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_tile_params(&mut reader, tp),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn sb_size(idx: u8) -> SuperblockSize {
        match idx % 3 {
            0 => SuperblockSize::Block64x64,
            1 => SuperblockSize::Block128x128,
            _ => SuperblockSize::Block256x256,
        }
    }

    proptest! {
        /// `tile_params()` must never panic on arbitrary input or parameters across
        /// the AV2-legal frame-size domain. The grid arithmetic at the `u32` extreme
        /// is covered deterministically by `extreme_frame_dimensions_do_not_panic`.
        #[test]
        fn parse_tile_params_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            frame_width in 1u32..=65536,
            frame_height in 1u32..=65536,
            sb in any::<u8>(),
            level in 0u8..=31,
            tier_high in any::<bool>(),
            is_bridge in any::<bool>(),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_tile_params(&mut reader, TileParamsInput {
                frame_width,
                frame_height,
                uniform_sb_size: sb_size(sb),
                sb_size: sb_size(sb),
                is_bridge,
                seq_tier: if tier_high { Tier::High } else { Tier::Main },
                seq_level_idx: LevelIdx::from_bits(level),
            });
        }

        /// `tile_log2()` must never panic for any input (it caps its shift amount).
        /// `uniform_spacing()` must not panic for `mis` across the full `u32` range
        /// (its rounding-up arithmetic saturates). `tile_log2` for `uniform_spacing` is
        /// bounded here only to keep the produced start vector small — a large
        /// `tile_log2` is a misuse outside the AV2 domain.
        #[test]
        fn tile_helpers_never_panic(
            blk in any::<u32>(),
            target in any::<u32>(),
            tile_log2_small in 0u8..=8,
            mis in any::<u32>(),
            sb in any::<u8>(),
        ) {
            let _ = tile_log2(blk, target);
            let _ = uniform_spacing(tile_log2_small, mis, sb_size(sb));
        }
    }
}
