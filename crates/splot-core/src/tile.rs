// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 tile-partitioning helpers and the `tile_params()` syntax (AV2 v1.0.0
//! § 5.18.7.3, with the § 5.18.7.5 `uniform_spacing` and § 5.18.7.7 `tile_log2`
//! helpers and the § 9.3 / level-tier conversion tables).
//!
//! This is the reusable tile-partitioning foundation. It is wired into the sequence
//! tile config (§ 5.4.2); the frame-level `tile_info()` reuse path (§ 5.18.7.2) is
//! out of scope for this phase. The parser reads syntax only and performs no tile
//! decoding.

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
const fn num_4x4_blocks_wide(sb_size: SuperblockSize) -> u32 {
    match sb_size {
        SuperblockSize::Block64x64 => 16,
        SuperblockSize::Block128x128 => 32,
        SuperblockSize::Block256x256 => 64,
    }
}

/// `Mi_Width_Log2[sbSize]` for the three sequence superblock sizes (AV2 v1.0.0 § 9.3
/// conversion tables; confirmed against AVM `mi_size_wide_log2`).
const fn mi_width_log2(sb_size: SuperblockSize) -> u32 {
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

/// `Tile_Width_Scaling_Factor[seq_tier][seq_level_idx]`, or `None` for a reserved
/// level index (a level with no defined scaling factor).
fn tile_width_scaling_factor(tier: Tier, level_idx: u8) -> Option<u32> {
    let value = *TILE_WIDTH_SCALING_FACTOR[tier_index(tier)].get(level_idx as usize)?;
    (value != 0).then_some(value)
}

/// `Tile_Area_Scaling_Factor[seq_tier][seq_level_idx]`, or `None` for a reserved
/// level index.
fn tile_area_scaling_factor(tier: Tier, level_idx: u8) -> Option<u32> {
    let value = *TILE_AREA_SCALING_FACTOR[tier_index(tier)].get(level_idx as usize)?;
    (value != 0).then_some(value)
}

/// `tile_log2(blkSize, target)` (AV2 v1.0.0 § 5.18.7.7): the smallest `k` such that
/// `blkSize << k >= target`.
///
/// The shift is computed in `u64` and `k` is capped at 32 so a degenerate
/// `blkSize == 0` (which the spec loop would never terminate on) returns a bounded
/// value instead of looping forever; real call sites always pass `blkSize >= 1`.
#[must_use]
pub fn tile_log2(blk_size: u32, target: u32) -> u8 {
    let mut k = 0u32;
    while k < 32 && (u64::from(blk_size) << k) < u64::from(target) {
        k += 1;
    }
    // k <= 32 fits in u8.
    k as u8
}

/// Returns `true` when `1 << tile_log2` tiles fit within `sb_num` superblocks, i.e. a
/// uniform split at this log2 is non-degenerate (every requested tile gets at least
/// one superblock). This is a `splot` convenience predicate over the § 5.18.7.5
/// `uniform_spacing` behavior, not a named spec function.
#[must_use]
pub fn uniform_eligible(tile_log2: u8, sb_num: u32) -> bool {
    (1u64 << u32::from(tile_log2).min(63)) <= u64::from(sb_num)
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
#[must_use]
pub fn uniform_spacing(tile_log2: u8, mis: u32, sb_size: SuperblockSize) -> TileSpacing {
    let sb4x4 = num_4x4_blocks_wide(sb_size);
    let sb_shift = mi_width_log2(sb_size);
    let sbs = (mis + sb4x4 - 1) >> sb_shift;
    let full_sbs = mis >> sb_shift;
    let tile_log2 = u32::from(tile_log2).min(31);
    let tile_sb = full_sbs >> tile_log2;
    let extra_sbs = if tile_sb == 0 {
        sbs
    } else {
        full_sbs - (tile_sb << tile_log2)
    };

    let num_tiles = 1u64 << tile_log2;
    let mut starts = Vec::new();
    let mut start_sb = 0u32;
    let mut i = 0u32;
    while u64::from(i) < num_tiles && start_sb < sbs {
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

/// Parses `tile_params(frameWidth, frameHeight, uniformSbSize, sbSize, isBridge)`
/// (AV2 v1.0.0 § 5.18.7.3).
///
/// # Errors
/// Returns [`Error::Unimplemented`] when `seq_level_idx` is a reserved level index
/// (no defined scaling factor, so the bit layout is undefined — a non-conformant
/// stream). Returns descriptor errors or
/// [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload ends
/// mid-field.
pub fn parse_tile_params(reader: &mut BitReader<'_>, input: TileParamsInput) -> Result<TileParams> {
    let sb4x4 = num_4x4_blocks_wide(input.sb_size);
    let sb_shift = mi_width_log2(input.sb_size);
    let mi_cols = 2 * ((input.frame_width + 7) >> 3);
    let mi_rows = 2 * ((input.frame_height + 7) >> 3);
    let sb_cols = (mi_cols + sb4x4 - 1) >> sb_shift;
    let sb_rows = (mi_rows + sb4x4 - 1) >> sb_shift;

    let level_idx = input.seq_level_idx.get();
    let (max_tile_width_sb, mut max_tile_area_sb) = if level_idx != NO_LEVEL_IDX {
        // A reserved level has no defined scaling factor, so the bit layout cannot be
        // determined. Report it as unmodeled (the sequence tile config maps this to a
        // bounded status for the non-conformant level).
        let width_sf =
            tile_width_scaling_factor(input.seq_tier, level_idx).ok_or(Error::Unimplemented {
                feature: "AV2-5.18.7.3-TILE-PARAMS",
            })?;
        let area_sf =
            tile_area_scaling_factor(input.seq_tier, level_idx).ok_or(Error::Unimplemented {
                feature: "AV2-5.18.7.3-TILE-PARAMS",
            })?;
        let max_tile_width_sb = (width_sf * MAX_TILE_WIDTH) >> (sb_shift + 4);
        let max_tile_area_sb = (area_sf * MAX_TILE_AREA) >> (2 * (sb_shift + 2) + 2);
        (max_tile_width_sb, max_tile_area_sb)
    } else {
        (sb_cols, sb_cols * sb_rows)
    };

    let min_log2_tile_cols = tile_log2(max_tile_width_sb, sb_cols);
    let max_log2_tile_cols = tile_log2(1, sb_cols.min(MAX_TILE_COLS));
    let max_log2_tile_rows = tile_log2(1, sb_rows.min(MAX_TILE_ROWS));
    let min_log2_tiles =
        min_log2_tile_cols.max(tile_log2(max_tile_area_sb, sb_rows.saturating_mul(sb_cols)));

    let uniform = if input.is_bridge {
        true
    } else {
        reader.read_bit()? != 0
    };

    let (tile_cols, tile_cols_log2, tile_rows, covers_cols, covers_rows) = if uniform {
        // Column tiles.
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

        // Row tiles.
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
        // Uniform spacing always covers the frame.
        (tile_cols, tile_cols_log2, rows.count, true, true)
    } else {
        // Non-uniform columns.
        let mut widest_tile_sb = 1u32;
        let mut start_sb = 0u32;
        let mut tile_cols = 0u32;
        while start_sb < sb_cols {
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

        // Non-uniform rows.
        let mut start_sb = 0u32;
        let mut tile_rows = 0u32;
        while start_sb < sb_rows {
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
            covers_cols,
            covers_rows,
        )
    };

    let tile_rows_log2 = tile_log2(1, tile_rows);

    Ok(TileParams {
        tile_cols,
        tile_rows,
        tile_cols_log2,
        tile_rows_log2,
        sb_cols,
        sb_rows,
        uniform_spacing: uniform,
        covers_cols,
        covers_rows,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

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
        // Degenerate blkSize == 0 is bounded rather than looping forever.
        assert_eq!(tile_log2(0, 5), 32);
    }

    #[test]
    fn uniform_eligible_checks_tile_count_fits() {
        assert!(uniform_eligible(0, 1));
        assert!(uniform_eligible(1, 2));
        assert!(!uniform_eligible(1, 1));
        assert!(uniform_eligible(6, 64));
        assert!(!uniform_eligible(6, 63));
    }

    #[test]
    fn uniform_spacing_single_tile() {
        // miCols = 4 (16-wide frame), BLOCK_64X64 -> sbCols = 1, one tile at sb 0.
        let spacing = uniform_spacing(0, 4, SuperblockSize::Block64x64);
        assert_eq!(spacing.count, 1);
        assert_eq!(spacing.starts, vec![0]);
    }

    #[test]
    fn uniform_spacing_two_tiles() {
        // miCols = 32 (128-wide frame), BLOCK_64X64 -> sbCols = 2; tileLog2 = 1 splits
        // into two 1-superblock tiles at sb 0 and 1.
        let spacing = uniform_spacing(1, 32, SuperblockSize::Block64x64);
        assert_eq!(spacing.count, 2);
        assert_eq!(spacing.starts, vec![0, 1]);
    }

    #[test]
    fn parse_uniform_single_tile_reads_only_uniform_flag() {
        // 16x8 frame, BLOCK_64X64, level 0: minLog2TileCols == maxLog2TileCols == 0, so
        // no increment bits are read; just uniform_tile_spacing_flag = 1.
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
        // Same 16x8 frame, uniform_tile_spacing_flag = 0. sbCols = sbRows = 1, so each
        // ns() reads 0 bits (n == 1) and the single tile covers the frame.
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
        // 128x8 frame, BLOCK_64X64 -> sbCols = 2, sbRows = 1. Non-uniform: two
        // 1-superblock columns. uniform flag = 0, then ns(2) = 0 for the first column
        // (1 bit), the second column is implied (ns(1) reads 0 bits), and the single
        // row reads ns(1) (0 bits).
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
    fn reserved_level_is_unimplemented_without_reading_bits() {
        // seq_level_idx 22 is reserved (no defined scaling factor).
        let mut reader = BitReader::new(&[0xFF], ByteOffset::new(0));
        let mut tp = input(16, 8);
        tp.seq_level_idx = LevelIdx::from_bits(22);
        assert!(matches!(
            parse_tile_params(&mut reader, tp),
            Err(Error::Unimplemented {
                feature: "AV2-5.18.7.3-TILE-PARAMS"
            })
        ));
        // No bits were consumed (the reserved check precedes the uniform-flag read).
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn no_level_idx_uses_unconstrained_fallback() {
        // seq_level_idx 31 uses maxTileWidthSb = sbCols, maxTileAreaSb = sbCols*sbRows.
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
        /// `tile_params()` must never panic on arbitrary input or parameters.
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
    }
}
