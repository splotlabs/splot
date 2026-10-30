// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder configuration and its validation.

use core::num::NonZeroU32;

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, PixelFormat, PlaneSize};

use crate::error::{Error, Result};
use crate::preset::{Preset, PresetParams};
use crate::rate_control::{Crf, check_bitrate, max_qindex};

/// Largest frame dimension AV2 can signal: `max_frame_width_minus_1` has at most
/// 16 bits (§ 5.4.1, `docs/spec/av2/1.0.0/05-syntax-structures.md`).
pub const MAX_FRAME_DIMENSION: u32 = 1 << 16;

/// How the encoder picks the quantizer of each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RateControlMode {
    /// Every frame uses the same `base_q_idx`.
    ConstantQp {
        /// The frame-header `base_q_idx`, `1..=255` at 8 bits and `1..=303` at
        /// 10 bits (`MaxQ`, § 6.4.1 Table 6.3).
        qp: u16,
    },
    /// Constant quality with lookahead offsets; see [`crate::Crf`].
    Crf {
        /// The rate factor, `1..=63`; higher means fewer bits.
        crf: u8,
    },
    /// Variable bitrate around a target average.
    Vbr {
        /// The target bitrate in kilobits per second.
        bitrate_kbps: u32,
    },
    /// Constant bitrate with a buffer that no frame may overflow.
    Cbr {
        /// The target bitrate in kilobits per second.
        bitrate_kbps: u32,
        /// The buffer size in milliseconds at the target bitrate.
        buffer_ms: u32,
    },
}

/// A uniform tile layout: `1 << cols_log2` tile columns by `1 << rows_log2`
/// tile rows (§ 5.18.7.3), capped at one tile per superblock column and row.
///
/// The encoder codes the tiles of a frame in parallel. A tile predicts and
/// adapts its CDFs without the other tiles, so more tiles cost compression.
/// `Default` is one tile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileLayout {
    /// Log2 of the tile column count, `0..=6`.
    pub cols_log2: u8,
    /// Log2 of the tile row count, `0..=6`.
    pub rows_log2: u8,
}

impl TileLayout {
    /// The largest `cols_log2` and `rows_log2`: `MAX_TILE_COLS` and
    /// `MAX_TILE_ROWS` are 64 (§ 3).
    pub const MAX_LOG2: u8 = 6;
}

/// The encoder configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// Frame width in luma samples.
    pub width: u32,
    /// Frame height in luma samples.
    pub height: u32,
    /// Sample bit depth.
    pub bit_depth: BitDepth,
    /// Chroma format: 4:2:0, 4:2:2, 4:4:4 or monochrome (`mono_chrome`, luma alone).
    pub chroma: PixelFormat,
    /// Frame rate as `(numerator, denominator)` frames per second.
    pub frame_rate: (u32, u32),
    /// Rate-control mode.
    pub rate_control: RateControlMode,
    /// Speed preset.
    pub preset: Preset,
    /// `(field, value)` overrides of the preset's [`PresetParams`], applied in
    /// order by [`PresetParams::set`]. They apply to the screen-content row
    /// too, so they win over the screen-content switch.
    pub preset_overrides: Vec<(String, String)>,
    /// Worker-thread policy. It never changes the output bytes.
    pub threads: ThreadCount,
    /// Tile layout of every frame. The thread count never changes it.
    pub tiles: TileLayout,
    /// Distance between key frames, in frames.
    pub keyint: NonZeroU32,
    /// Fixed-ratio resize: every frame is coded at `8 / denominator` of the
    /// input size per axis, `9..=16`; `None` codes the input size. AV2 has no
    /// superres, so the stream decodes to the coded size.
    pub resize_denominator: Option<u8>,
    /// Film grain: every key frame codes a grain model fitted to the noise of
    /// its source (none under a threshold), and every output frame applies it
    /// (§ 5.14, § 5.18.10.1). Off by default; TIP output frames stay off.
    pub film_grain: bool,
}

impl Config {
    /// Default `base_q_idx` for [`RateControlMode::ConstantQp`].
    pub const DEFAULT_QP: u16 = 80;
    /// Default key-frame interval.
    pub const DEFAULT_KEYINT: NonZeroU32 = match NonZeroU32::new(120) {
        Some(keyint) => keyint,
        None => NonZeroU32::MIN,
    };

    /// The `resize_denominator` range: down to half size, the largest step
    /// of the § 7.13.3.18 resampler.
    pub const RESIZE_DENOMINATORS: core::ops::RangeInclusive<u8> = 9..=16;

    /// Creates an 8-bit 4:2:0 configuration at 30 fps with default settings.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bit_depth: BitDepth::Eight,
            chroma: PixelFormat::Yuv420,
            frame_rate: (30, 1),
            rate_control: RateControlMode::ConstantQp {
                qp: Self::DEFAULT_QP,
            },
            preset: Preset::default(),
            preset_overrides: Vec::new(),
            threads: ThreadCount::Auto,
            tiles: TileLayout::default(),
            keyint: Self::DEFAULT_KEYINT,
            resize_denominator: None,
            film_grain: false,
        }
    }

    /// Checks that the encoder supports this configuration.
    ///
    /// # Errors
    /// Returns [`Error::InvalidDimensions`], [`Error::InvalidResize`],
    /// [`Error::InvalidFrameRate`], [`Error::InvalidTiles`], [`Error::InvalidQp`],
    /// [`Error::InvalidCrf`], [`Error::InvalidBitrate`] or [`Error::Preset`].
    pub fn validate(&self) -> Result<()> {
        self.override_preset(self.preset.params())?;
        self.override_preset(self.preset.screen_params())?;
        let dimension_ok = |value| (1..=MAX_FRAME_DIMENSION).contains(&value);
        if !dimension_ok(self.width) || !dimension_ok(self.height) {
            return Err(Error::InvalidDimensions {
                width: self.width,
                height: self.height,
            });
        }
        if let Some(denominator) = self.resize_denominator
            && !Self::RESIZE_DENOMINATORS.contains(&denominator)
        {
            return Err(Error::InvalidResize { denominator });
        }
        if self.frame_rate.0 == 0 || self.frame_rate.1 == 0 {
            return Err(Error::InvalidFrameRate {
                numerator: self.frame_rate.0,
                denominator: self.frame_rate.1,
            });
        }
        if self.tiles.cols_log2 > TileLayout::MAX_LOG2
            || self.tiles.rows_log2 > TileLayout::MAX_LOG2
        {
            return Err(Error::InvalidTiles { tiles: self.tiles });
        }
        match self.rate_control {
            RateControlMode::ConstantQp { qp } if qp == 0 || qp > max_qindex(self.bit_depth) => {
                Err(Error::InvalidQp {
                    qp,
                    max: max_qindex(self.bit_depth),
                })
            }
            RateControlMode::Crf { crf } if crf == 0 || crf > Crf::MAX => {
                Err(Error::InvalidCrf { crf })
            }
            RateControlMode::Vbr { bitrate_kbps } => {
                check_bitrate(bitrate_kbps, None, self.frame_rate)
            }
            RateControlMode::Cbr {
                bitrate_kbps,
                buffer_ms,
            } => check_bitrate(bitrate_kbps, Some(buffer_ms), self.frame_rate),
            _ => Ok(()),
        }
    }

    /// The configuration of the coded frames: the frame size after
    /// [`Self::resize_denominator`], which it clears. A denominator outside
    /// [`Self::RESIZE_DENOMINATORS`], which [`Self::validate`] rejects, keeps
    /// the input size.
    #[must_use]
    pub fn coded(&self) -> Self {
        let Some(denominator) = self
            .resize_denominator
            .filter(|denominator| Self::RESIZE_DENOMINATORS.contains(denominator))
        else {
            return self.clone();
        };
        let scale = |value: u32| {
            let scaled =
                (u64::from(value) * 8 + u64::from(denominator / 2)) / u64::from(denominator);
            u32::try_from(scaled).unwrap_or(value).max(1)
        };
        Self {
            width: scale(self.width),
            height: scale(self.height),
            resize_denominator: None,
            ..self.clone()
        }
    }

    /// `row` with [`Self::preset_overrides`] applied, checked by
    /// [`PresetParams::validate`].
    pub(crate) fn override_preset(&self, row: &PresetParams) -> Result<PresetParams> {
        let mut row = *row;
        for (name, value) in &self.preset_overrides {
            row.set(name, value)?;
        }
        row.validate()?;
        Ok(row)
    }

    /// Returns the Y, U and V plane sizes of one frame; monochrome has Y alone.
    ///
    /// # Errors
    /// Returns [`Error::InvalidDimensions`] for a zero dimension.
    pub fn plane_sizes(&self) -> Result<Vec<PlaneSize>> {
        let invalid = || Error::InvalidDimensions {
            width: self.width,
            height: self.height,
        };
        let luma =
            PlaneSize::new(self.width as usize, self.height as usize).map_err(|_| invalid())?;
        let chroma = self.chroma.chroma_size(luma).map_err(|_| invalid())?;
        Ok(chroma.map_or(vec![luma], |chroma| vec![luma, chroma, chroma]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(Config::new(64, 64).validate().is_ok());
    }

    #[test]
    fn rejects_unsupported_settings() {
        let mut config = Config::new(0, 64);
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidDimensions { .. })
        ));
        config.width = MAX_FRAME_DIMENSION + 1;
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidDimensions { .. })
        ));
        config = Config::new(64, 64);
        config.tiles.rows_log2 = TileLayout::MAX_LOG2 + 1;
        assert!(matches!(config.validate(), Err(Error::InvalidTiles { .. })));
        config = Config::new(64, 64);
        config.frame_rate = (30, 0);
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidFrameRate { .. })
        ));
        config = Config::new(64, 64);
        config.rate_control = RateControlMode::ConstantQp { qp: 0 };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidQp { qp: 0, .. })
        ));
        config.rate_control = RateControlMode::ConstantQp { qp: 256 };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidQp { qp: 256, max: 255 })
        ));
        config.bit_depth = BitDepth::Ten;
        assert!(config.validate().is_ok());
        config.rate_control = RateControlMode::ConstantQp { qp: 304 };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidQp { max: 303, .. })
        ));
        config.bit_depth = BitDepth::Eight;
        config.rate_control = RateControlMode::Crf { crf: 64 };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidCrf { crf: 64 })
        ));
        config.rate_control = RateControlMode::Cbr {
            bitrate_kbps: 100,
            buffer_ms: 0,
        };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidBitrate {
                what: "buffer_ms",
                ..
            })
        ));
        config.rate_control = RateControlMode::Vbr { bitrate_kbps: 0 };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidBitrate { .. })
        ));
        config.rate_control = RateControlMode::Vbr {
            bitrate_kbps: u32::MAX,
        };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidBitrate {
                what: "bitrate_kbps",
                ..
            })
        ));
        config.rate_control = RateControlMode::Cbr {
            bitrate_kbps: 100,
            buffer_ms: u32::MAX,
        };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidBitrate {
                what: "buffer_ms",
                ..
            })
        ));
        config.rate_control = RateControlMode::Crf { crf: 63 };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn odd_sizes_round_chroma_up() -> Result<()> {
        let mut config = Config::new(5, 3);
        let &[y, u, v] = config.plane_sizes()?.as_slice() else {
            unreachable!("4:2:0 has three planes")
        };
        assert_eq!((y.width(), y.height()), (5, 3));
        assert_eq!((u.width(), u.height()), (3, 2));
        assert_eq!(u, v);
        config.chroma = PixelFormat::Monochrome;
        assert!(config.validate().is_ok());
        assert_eq!(config.plane_sizes()?, [y]);
        Ok(())
    }

    #[test]
    fn resize_scales_the_coded_size_by_8_over_the_denominator() {
        let mut config = Config::new(160, 90);
        assert_eq!(config.coded(), config);
        for (denominator, size) in [(9, (142, 80)), (12, (107, 60)), (16, (80, 45))] {
            config.resize_denominator = Some(denominator);
            assert!(config.validate().is_ok());
            let coded = config.coded();
            assert_eq!(
                ((coded.width, coded.height), coded.resize_denominator),
                (size, None)
            );
        }
        config.width = 1;
        assert_eq!(config.coded().width, 1);
        config.width = 160;
        for denominator in [0, 8, 17] {
            config.resize_denominator = Some(denominator);
            assert_eq!((config.coded().width, config.coded().height), (160, 90));
            assert!(matches!(
                config.validate(),
                Err(Error::InvalidResize { denominator: d }) if d == denominator
            ));
        }
    }
}
