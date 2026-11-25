// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Static frame and tile limits from AV2 v1.0.0 Annex A.4.

use crate::headers::sequence::Tier;
use crate::hls::MultistreamDecoderOperation;
use crate::types::ExtendedLayerId;

/// Frame and tile maxima for a table-mapped level (Annex A.4).
#[expect(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLevelLimits {
    /// Maximum luma samples per picture, `MaxPicSize`.
    pub max_pic_size: u64,
    /// Maximum frame width in luma samples, `MaxHSize`.
    pub max_width: u32,
    /// Maximum frame height in luma samples, `MaxVSize`.
    pub max_height: u32,
    /// Maximum whole tiles per picture, `MaxTiles`.
    pub max_tiles: u32,
    /// Header-rate basis, `MaxHeaderRate`. The frame-header count limit also
    /// uses the sequence tier (Annex A.4).
    pub max_header_rate: u32,
    /// Maximum tile columns per picture, `MaxTileCols`.
    pub max_tile_cols: u32,
}

/// Looks up the static frame limits for a sequence level. Reserved levels
/// (22–30) and maximum parameters (31) have no row in Tables A.8 and A.9.
#[must_use]
pub fn level_limits(level_idx: u8) -> Option<FrameLevelLimits> {
    // Tables A.8 and A.9 (mirror lines 330–432), indexed by Table A.7 LevelIdx.
    // Columns are MaxPicSize, MaxHSize/MaxVSize, MaxTiles, and MaxTileCols.
    const LEVELS: [(u64, u32, u32, u32); 22] = [
        (147_456, 640, 8, 4),
        (278_784, 880, 8, 4),
        (665_856, 1360, 16, 6),
        (1_065_024, 1720, 16, 6),
        (2_359_296, 2560, 32, 8),
        (2_359_296, 2560, 32, 8),
        (8_912_896, 4975, 64, 8),
        (8_912_896, 4975, 64, 8),
        (8_912_896, 4975, 64, 8),
        (8_912_896, 4975, 64, 8),
        (35_651_584, 9951, 128, 16),
        (35_651_584, 9951, 128, 16),
        (35_651_584, 9951, 128, 16),
        (35_651_584, 9951, 128, 16),
        (142_606_336, 19902, 256, 32),
        (142_606_336, 19902, 256, 32),
        (142_606_336, 19902, 256, 32),
        (142_606_336, 19902, 256, 32),
        (530_841_600, 38400, 512, 64),
        (530_841_600, 38400, 512, 64),
        (530_841_600, 38400, 512, 64),
        (530_841_600, 38400, 512, 64),
    ];
    let &(max_pic_size, dimension, max_tiles, max_tile_cols) =
        LEVELS.get(usize::from(level_idx))?;
    Some(FrameLevelLimits {
        max_pic_size,
        max_width: dimension,
        max_height: dimension,
        max_tiles,
        max_tile_cols,
        max_header_rate: match level_idx {
            0..=3 => 150,
            4..=13 => 300,
            _ => 960,
        },
    })
}

impl MultistreamDecoderOperation {
    // Table column and ScaleFactorX as a numerator / denominator.
    fn allocation_scale(&self, xlayer: ExtendedLayerId) -> Option<(usize, u32, u32)> {
        let streams = self.sub_streams.get(..usize::from(self.sub_stream_count))?;
        if !streams.iter().any(|sub| sub.sub_xlayer_id == xlayer.get()) {
            return None;
        }
        if self.multistream_even_allocation_flag {
            return Some((1, 4, 1));
        }
        let large = streams.get(usize::from(self.multistream_large_picture_idc?))?;
        if xlayer.get() == large.sub_xlayer_id {
            Some((0, 3, 2))
        } else {
            Some((2, 9, 1))
        }
    }

    /// Computes the static frame and tile maxima in decoder mode 1 (Annex A.4).
    /// The native sequence level supplies the per-stream limits. The aggregate
    /// level and the stream's allocation can reduce them. Returns `None` for
    /// an undeclared layer or an unmapped sequence level.
    #[must_use]
    pub fn frame_level_limits(
        &self,
        sequence_level: u8,
        xlayer: ExtendedLayerId,
    ) -> Option<FrameLevelLimits> {
        let (column, numerator, denominator) = self.allocation_scale(xlayer)?;
        let mut limits = level_limits(sequence_level)?;
        if self.multistream_level_idx == 31 {
            return Some(limits);
        }
        let aggregate = level_limits(self.multistream_level_idx)?;
        limits.max_tiles = limits
            .max_tiles
            .min(aggregate.max_tiles * denominator / numerator);

        // Annex A.4 multistream table (mirror lines 475–512). The columns
        // contain (MaxVSizeX, MaxHSizeX, MaxTileColsX) for scales 1.5, 4, 9.
        // For the absent rows at aggregate levels 0–3, Splot follows pinned
        // AVM 457cd58681a747465661baccb1f32095bc5b7774, level.c,
        // level_to_sub_stream_level_index(): use the level-4.0 table cells.
        // MaxTilesX above still uses the signaled aggregate level.
        let row = match self.multistream_level_idx {
            0..=5 => [(1600, 896, 7), (960, 576, 4), (640, 384, 3)],
            6..=9 => [(2560, 1472, 7), (1920, 1088, 4), (1280, 768, 3)],
            10..=13 => [(5120, 2280, 13), (3840, 2176, 8), (2560, 1472, 5)],
            14..=17 => [(10240, 5760, 26), (7680, 4320, 16), (5120, 2880, 11)],
            _ => [(20480, 11520, 52), (15360, 8640, 32), (10240, 5760, 21)],
        };
        let (height, width, cols) = row[column];
        // MaxHeaderRateX is 132 for every row.
        limits.max_header_rate = limits.max_header_rate.min(132);
        limits.max_width = limits.max_width.min(width);
        limits.max_height = limits.max_height.min(height);
        limits.max_tile_cols = limits.max_tile_cols.min(cols);
        limits.max_pic_size = limits
            .max_pic_size
            .min(u64::from(width) * u64::from(height));
        Some(limits)
    }
}

/// An Annex A.4 rate in units per second. The fraction preserves the exact
/// result for uneven allocation, including values between whole units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelRate {
    /// Units in the interval.
    pub units: u64,
    /// Duration of the interval in seconds.
    pub seconds: u32,
}

/// Rate and compression limits for Annex E timing checks. A decoder needs
/// bitstream timing observations to check the rates against actual pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelRateLimits {
    /// Maximum display luma samples per second.
    pub display: LevelRate,
    /// Maximum decoded luma samples per second.
    pub decode: LevelRate,
    /// Maximum coded bits per second.
    pub bitrate: LevelRate,
    /// Minimum compression basis for the signaled tiers.
    pub min_compression_basis: u8,
}

impl MultistreamDecoderOperation {
    /// Computes the timing-dependent Annex A.4 level limits in decoder mode 1.
    /// Returns `None` for an unmapped sequence level or an undefined tier.
    #[must_use]
    pub fn rate_limits(
        &self,
        sequence_level: u8,
        sequence_tier: Tier,
        xlayer: ExtendedLayerId,
    ) -> Option<LevelRateLimits> {
        let (_, numerator, denominator) = self.allocation_scale(xlayer)?;
        let (display, decode, bitrate, min_compression_basis) =
            rate_table(sequence_level, sequence_tier)?;
        let native = |units| LevelRate { units, seconds: 1 };
        let mut limits = LevelRateLimits {
            display: native(display),
            decode: native(decode),
            bitrate: native(bitrate),
            min_compression_basis,
        };
        if self.multistream_level_idx == 31 {
            return Some(limits);
        }
        let (aggregate_display, aggregate_decode, aggregate_bitrate, aggregate_cr) = rate_table(
            self.multistream_level_idx,
            Tier::from_bit(self.multistream_tier),
        )?;
        let rate = |native_units, aggregate: u64| {
            let units = aggregate * u64::from(denominator);
            if units < native_units * u64::from(numerator) {
                LevelRate {
                    units,
                    seconds: numerator,
                }
            } else {
                native(native_units)
            }
        };
        limits.display = rate(display, aggregate_display);
        limits.decode = rate(decode, aggregate_decode);
        limits.bitrate = rate(bitrate, aggregate_bitrate);
        limits.min_compression_basis = min_compression_basis.max(aggregate_cr);
        Some(limits)
    }
}

fn rate_table(level: u8, tier: Tier) -> Option<(u64, u64, u64, u8)> {
    // Annex A.4 Tables A.8 and A.9, mirror lines 330–432. Columns are
    // display and decode samples/s, main and high bits/s, and MainCR.
    // HighCR is 4 wherever the high tier exists.
    const RATES: [(u64, u64, u64, u64, u8); 22] = [
        (4_423_680, 5_529_600, 1_500_000, 0, 2),
        (8_363_520, 10_454_400, 3_000_000, 0, 2),
        (19_975_680, 24_969_600, 6_000_000, 0, 2),
        (31_950_720, 39_938_400, 10_000_000, 0, 2),
        (70_778_880, 77_856_768, 12_000_000, 30_000_000, 4),
        (141_557_760, 155_713_536, 20_000_000, 50_000_000, 4),
        (267_386_880, 273_715_200, 30_000_000, 100_000_000, 6),
        (534_773_760, 547_430_400, 40_000_000, 160_000_000, 8),
        (1_069_547_520, 1_094_860_800, 60_000_000, 240_000_000, 8),
        (1_069_547_520, 1_176_502_272, 60_000_000, 240_000_000, 8),
        (1_069_547_520, 1_176_502_272, 60_000_000, 240_000_000, 8),
        (2_139_095_040, 2_189_721_600, 100_000_000, 480_000_000, 8),
        (4_278_190_080, 4_379_443_200, 160_000_000, 800_000_000, 8),
        (4_278_190_080, 4_706_009_088, 160_000_000, 800_000_000, 8),
        (4_278_190_080, 4_706_009_088, 160_000_000, 800_000_000, 8),
        (8_556_380_160, 8_758_886_400, 200_000_000, 960_000_000, 8),
        (
            17_112_760_320,
            17_517_772_800,
            320_000_000,
            1_600_000_000,
            8,
        ),
        (
            17_112_760_320,
            18_824_036_352,
            320_000_000,
            1_600_000_000,
            8,
        ),
        (
            17_112_760_320,
            18_824_036_352,
            320_000_000,
            1_600_000_000,
            8,
        ),
        (
            34_225_520_640,
            34_910_031_052,
            400_000_000,
            1_920_000_000,
            8,
        ),
        (
            68_451_041_280,
            69_820_062_105,
            640_000_000,
            3_200_000_000,
            8,
        ),
        (
            68_451_041_280,
            75_296_145_408,
            640_000_000,
            3_200_000_000,
            8,
        ),
    ];
    let &(display, decode, main_bps, high_bps, main_cr) = RATES.get(usize::from(level))?;
    match tier {
        Tier::Main => Some((display, decode, main_bps, main_cr)),
        Tier::High if high_bps != 0 => Some((display, decode, high_bps, 4)),
        Tier::High => None,
    }
}
