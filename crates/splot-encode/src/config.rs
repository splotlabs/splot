// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder configuration and its validation.

use core::num::NonZeroU32;

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, PixelFormat, PlaneSize};

use crate::error::{Error, Result};
use crate::preset::Preset;
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
    /// Chroma format; only [`PixelFormat::Yuv420`] is supported.
    pub chroma: PixelFormat,
    /// Frame rate as `(numerator, denominator)` frames per second.
    pub frame_rate: (u32, u32),
    /// Rate-control mode.
    pub rate_control: RateControlMode,
    /// Speed preset.
    pub preset: Preset,
    /// Worker-thread policy. It never changes the output bytes.
    pub threads: ThreadCount,
    /// Distance between key frames, in frames.
    pub keyint: NonZeroU32,
}

impl Config {
    /// Default `base_q_idx` for [`RateControlMode::ConstantQp`].
    pub const DEFAULT_QP: u16 = 80;
    /// Default key-frame interval.
    pub const DEFAULT_KEYINT: NonZeroU32 = match NonZeroU32::new(120) {
        Some(keyint) => keyint,
        None => NonZeroU32::MIN,
    };

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
            threads: ThreadCount::Auto,
            keyint: Self::DEFAULT_KEYINT,
        }
    }

    /// Checks that the encoder supports this configuration.
    ///
    /// # Errors
    /// Returns [`Error::InvalidDimensions`], [`Error::UnsupportedChroma`],
    /// [`Error::InvalidFrameRate`], [`Error::InvalidQp`], [`Error::InvalidCrf`]
    /// or [`Error::InvalidBitrate`].
    pub fn validate(&self) -> Result<()> {
        let dimension_ok = |value| (1..=MAX_FRAME_DIMENSION).contains(&value);
        if !dimension_ok(self.width) || !dimension_ok(self.height) {
            return Err(Error::InvalidDimensions {
                width: self.width,
                height: self.height,
            });
        }
        if self.chroma != PixelFormat::Yuv420 {
            return Err(Error::UnsupportedChroma {
                chroma: self.chroma,
            });
        }
        if self.frame_rate.0 == 0 || self.frame_rate.1 == 0 {
            return Err(Error::InvalidFrameRate {
                numerator: self.frame_rate.0,
                denominator: self.frame_rate.1,
            });
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

    /// Returns the Y, U and V plane sizes of one frame.
    ///
    /// # Errors
    /// Returns [`Error::InvalidDimensions`] for a zero dimension and
    /// [`Error::UnsupportedChroma`] for monochrome.
    pub fn plane_sizes(&self) -> Result<[PlaneSize; 3]> {
        let invalid = || Error::InvalidDimensions {
            width: self.width,
            height: self.height,
        };
        let luma =
            PlaneSize::new(self.width as usize, self.height as usize).map_err(|_| invalid())?;
        let chroma = self
            .chroma
            .chroma_size(luma)
            .map_err(|_| invalid())?
            .ok_or(Error::UnsupportedChroma {
                chroma: self.chroma,
            })?;
        Ok([luma, chroma, chroma])
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
        config.chroma = PixelFormat::Yuv444;
        assert!(matches!(
            config.validate(),
            Err(Error::UnsupportedChroma { .. })
        ));
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
        let [y, u, v] = Config::new(5, 3).plane_sizes()?;
        assert_eq!((y.width(), y.height()), (5, 3));
        assert_eq!((u.width(), u.height()), (3, 2));
        assert_eq!(u, v);
        Ok(())
    }
}
