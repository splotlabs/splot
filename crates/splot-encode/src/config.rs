// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder configuration and its validation.

pub mod options;

use core::num::NonZeroU32;

use splot_core::headers::sequence::{ChromaFormatIdc, CroppingWindow, ProfileIdc};
use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, PixelFormat, PlaneSize};

use crate::content::{ContentDescription, KeyFrameMetadata};
use crate::error::{Error, Result};
use crate::preset::{Preset, PresetParams};
use crate::rate_control::{Crf, check_bitrate, check_cbr, max_qindex};

mod resolution;
pub use resolution::{PresetOrigin, ResolvedConfig, ResolvedPreset};

/// Largest frame dimension AV2 can signal: `max_frame_width_minus_1` has at most
/// 16 bits (§ 5.4.1, `docs/spec/av2/1.0.0/05-syntax-structures.md`).
pub const MAX_FRAME_DIMENSION: u32 = 1 << 16;

/// The quality metric the encoder tunes its choices for; each takes
/// [`Tune::overrides`] before [`Config::preset_overrides`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tune {
    /// PSNR: plain squared error everywhere, no overrides.
    #[default]
    Psnr,
    /// SSIM: `ssim_aq`, so superblocks move their lambda and qindex with the
    /// variance of their 16x16 luma blocks.
    Ssim,
    /// Visual quality: `ssim_aq` and `psy_rd`, so trials also keep the AC
    /// energy of the source.
    Vq,
}

impl Tune {
    /// The preset overrides of this tune.
    #[must_use]
    pub const fn overrides(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Psnr => &[],
            Self::Ssim => &[("ssim_aq", "30")],
            Self::Vq => &[("ssim_aq", "30"), ("psy_rd", "50")],
        }
    }
}

/// How the encoder picks the quantizer of each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RateControlMode {
    /// Every frame uses the same `base_q_idx`.
    ConstantQp {
        /// The frame-header `base_q_idx`, `1..=255` at 8 bits and `1..=303` at
        /// 10 bits (`MaxQ`, § 6.4.1 Table 6.3); `0` codes every frame lossless.
        qp: u16,
    },
    /// Constant quality with lookahead offsets; see [`crate::Crf`].
    Crf {
        /// The rate factor, `1..=63`; higher means fewer bits.
        crf: u8,
    },
    /// [`Self::Crf`] under a maximum bitrate: a decoder buffer that fills at
    /// `max_bitrate_kbps`, and a frame that would underflow it is coded
    /// again at a coarser quantizer before it is committed.
    CappedCrf {
        /// The rate factor, `1..=63`; higher means fewer bits.
        crf: u8,
        /// The bitrate the decoder buffer fills at, in kilobits per second.
        max_bitrate_kbps: u32,
        /// The buffer size in milliseconds at `max_bitrate_kbps`.
        buffer_ms: u32,
        /// The decoder buffer level before the first frame, in milliseconds
        /// at `max_bitrate_kbps`, `0..=buffer_ms`.
        initial_ms: u32,
    },
    /// Variable bitrate around a target average.
    Vbr {
        /// The target bitrate in kilobits per second.
        bitrate_kbps: u32,
    },
    /// Constant bitrate with a decoder buffer that no frame may underflow
    /// or overflow.
    Cbr {
        /// The target bitrate in kilobits per second.
        bitrate_kbps: u32,
        /// The buffer size in milliseconds at the target bitrate.
        buffer_ms: u32,
        /// The decoder buffer level before the first frame, in milliseconds
        /// at the target bitrate, `0..=buffer_ms`.
        initial_ms: u32,
    },
}

/// The tile layout of the frames (§ 5.18.7.3).
///
/// The encoder codes the tiles of a frame in parallel. A tile predicts and
/// adapts its CDFs without the other tiles, so more tiles cost compression.
/// `Default` is one tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TileLayout {
    /// `1 << cols_log2` tile columns by `1 << rows_log2` tile rows of uniform
    /// spacing, capped at one tile per superblock column and row.
    Uniform {
        /// Log2 of the tile column count, `0..=6`.
        cols_log2: u8,
        /// Log2 of the tile row count, `0..=6`.
        rows_log2: u8,
    },
    /// Coded tile sizes (`uniform_tile_spacing_flag = 0`) in superblocks of the
    /// sequence: the first columns and rows; one more column or row takes the
    /// rest of the frame when the sizes do not reach its edge. A preset with
    /// `seq_tile_info` above 0 refuses it.
    Explicit {
        /// Tile column widths.
        widths: Vec<u32>,
        /// Tile row heights.
        heights: Vec<u32>,
    },
}

impl Default for TileLayout {
    fn default() -> Self {
        Self::Uniform {
            cols_log2: 0,
            rows_log2: 0,
        }
    }
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
    /// What the source declares about its samples: colour, range, chroma
    /// sample position, sample aspect ratio, timing and HDR metadata. Nothing
    /// by default.
    pub content: ContentDescription,
    /// ICC profile, T.35 and user data payloads written in front of every
    /// key frame. None by default.
    pub key_frame_metadata: KeyFrameMetadata,
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
    /// Tile groups per frame: the tiles split in raster order into this many
    /// tile group OBUs of consecutive tiles (§ 5.19), at most one per tile.
    /// `1`, the default, codes one tile group.
    pub tile_groups: NonZeroU32,
    /// Distance between key frames, in frames.
    pub keyint: NonZeroU32,
    /// Fixed-ratio resize: every frame is coded at `8 / denominator` of the
    /// input size per axis, `9..=16`; `None` codes the input size. AV2 has no
    /// superres, so the stream decodes to the coded size.
    pub resize_denominator: Option<u8>,
    /// Cropping window, in luma samples of the coded frame (after
    /// [`Self::resize_denominator`]): the stream codes it in the sequence
    /// header (§ 5.4.1) and decodes to the window (§ 5.18.4.4). `None`, the
    /// default, codes no window.
    pub crop: Option<CroppingWindow>,
    /// The `seq_profile_idc` of the stream (Annex A.2 Table A.1). `None`, the
    /// default, codes the lowest profile that allows [`Self::chroma`]: 0 for
    /// 4:0:0 and 4:2:0, 3 for 4:2:2, 4 for 4:4:4.
    pub profile: Option<ProfileIdc>,
    /// Film grain: every key frame codes a grain model fitted to the noise of
    /// its source (none under a threshold), and every output frame applies it
    /// (§ 5.14, § 5.18.10.1). Off by default; TIP output frames stay off.
    pub film_grain: bool,
    /// CBR filler: when a temporal unit leaves the decoder buffer of
    /// [`RateControlMode::Cbr`] fuller than its size, the unit ends with one
    /// `OBU_PADDING` (§ 5.16) of the overflow, so the stream keeps the
    /// bitrate. Off by default; only CBR takes it.
    pub cbr_padding: bool,
    /// The metric the encoder tunes for; [`Tune::Psnr`] by default.
    pub tune: Tune,
    /// Fast decode: the preset takes [`Self::FAST_DECODE`] before
    /// [`Self::preset_overrides`]. Off by default.
    pub fast_decode: bool,
    /// Multi-frame header: each key frame unit codes one multi-frame header
    /// OBU (§ 5.7) after the sequence header, and every frame header refers
    /// to it with `cur_mfh_id = 1` (§ 5.18.2). Off by default.
    pub multi_frame_header: bool,
    /// The coded bits of each input frame in a first pass, in input order
    /// ([`Self::first_pass_config`]); [`RateControlMode::Vbr`] splits its
    /// budget over the frames in proportion to them. Empty (the default) is
    /// one pass; frames past the end take the mean budget.
    pub first_pass: Vec<u64>,
}

impl Config {
    /// Default `base_q_idx` for [`RateControlMode::ConstantQp`].
    pub const DEFAULT_QP: u16 = 80;

    /// Default rate factor for [`RateControlMode::Crf`], the mode of
    /// `splot encode` without a rate-control flag: its 8-bit base qindex is
    /// [`Self::DEFAULT_QP`].
    pub const DEFAULT_CRF: u8 = 20;
    /// Default key-frame interval.
    pub const DEFAULT_KEYINT: NonZeroU32 = match NonZeroU32::new(120) {
        Some(keyint) => keyint,
        None => NonZeroU32::MIN,
    };

    /// The `resize_denominator` range: down to half size, the largest step
    /// of the § 7.13.3.18 resampler.
    pub const RESIZE_DENOMINATORS: core::ops::RangeInclusive<u8> = 9..=16;

    /// The preset overrides of [`Self::fast_decode`]: 64x64 superblocks, no
    /// Wiener-NS or GDF, and at most two CDEF strength sets.
    pub const FAST_DECODE: [(&str, &str); 5] = [
        ("max_block_log2", "6"),
        ("wiener_ns", "0"),
        ("wiener_ns_chroma", "0"),
        ("gdf_candidates", "0"),
        ("cdef_strengths", "2"),
    ];

    /// Creates an 8-bit 4:2:0 configuration at 30 fps with default settings.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bit_depth: BitDepth::Eight,
            chroma: PixelFormat::Yuv420,
            frame_rate: (30, 1),
            content: ContentDescription::default(),
            key_frame_metadata: KeyFrameMetadata::default(),
            rate_control: RateControlMode::ConstantQp {
                qp: Self::DEFAULT_QP,
            },
            preset: Preset::default(),
            preset_overrides: Vec::new(),
            threads: ThreadCount::Auto,
            tiles: TileLayout::default(),
            tile_groups: NonZeroU32::MIN,
            keyint: Self::DEFAULT_KEYINT,
            resize_denominator: None,
            crop: None,
            profile: None,
            film_grain: false,
            cbr_padding: false,
            tune: Tune::Psnr,
            first_pass: Vec::new(),
            fast_decode: false,
            multi_frame_header: false,
        }
    }

    /// Checks that the encoder supports this configuration.
    ///
    /// # Errors
    /// Returns [`Error::InvalidDimensions`], [`Error::InvalidResize`],
    /// [`Error::InvalidCrop`], [`Error::InvalidProfile`],
    /// [`Error::InvalidFrameRate`], [`Error::InvalidContentDescription`],
    /// [`Error::InvalidMasteringDisplay`], [`Error::InvalidMetadata`],
    /// [`Error::InvalidTiles`], [`Error::InvalidQp`],
    /// [`Error::InvalidCrf`], [`Error::InvalidBitrate`], [`Error::LosslessWith`],
    /// [`Error::PaddingWithoutCbr`] or [`Error::Preset`].
    pub fn validate(&self) -> Result<()> {
        self.validated_presets(|_, _| {}).map(|_| ())
    }

    fn validated_presets(
        &self,
        record: impl FnMut(&str, PresetOrigin),
    ) -> Result<[PresetParams; 2]> {
        let lossless_refused = [
            (self.film_grain, "film grain"),
            (self.resize_denominator.is_some(), "resize"),
        ];
        if let Some(&(_, what)) = lossless_refused
            .iter()
            .find(|&&(on, _)| on && self.is_lossless())
        {
            return Err(Error::LosslessWith {
                what: what.to_owned(),
            });
        }
        let rows = [
            self.apply_preset(self.preset.params(), record)?,
            self.override_preset(self.preset.screen_params())?,
        ];
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
        let coded = self.coded();
        let chroma = ChromaFormatIdc::try_new(u32::from(self.chroma.chroma_format_idc()));
        if let Some(window) = self.crop
            && !chroma.is_some_and(|chroma| window.fits(coded.width, coded.height, chroma))
        {
            return Err(Error::InvalidCrop {
                window,
                width: coded.width,
                height: coded.height,
            });
        }
        if let Some(profile) = self.profile
            && !chroma.is_some_and(|chroma| profile.allows_chroma(chroma))
        {
            return Err(Error::InvalidProfile {
                profile,
                chroma: self.chroma,
            });
        }
        if self.frame_rate.0 == 0 || self.frame_rate.1 == 0 {
            return Err(Error::InvalidFrameRate {
                numerator: self.frame_rate.0,
                denominator: self.frame_rate.1,
            });
        }
        self.content.validate(self.chroma)?;
        self.key_frame_metadata.validate()?;
        let tiles_refused = match &self.tiles {
            TileLayout::Uniform {
                cols_log2,
                rows_log2,
            } => (*cols_log2.max(rows_log2) > TileLayout::MAX_LOG2)
                .then_some("more than 64 tile columns or rows"),
            TileLayout::Explicit { widths, heights } => {
                if widths.contains(&0) || heights.contains(&0) {
                    Some("a tile of 0 superblocks")
                } else if rows.iter().any(|row| row.seq_tile_info > 0) {
                    Some("seq_tile_info codes only a uniform layout")
                } else {
                    None
                }
            }
        };
        if let Some(reason) = tiles_refused {
            return Err(Error::InvalidTiles {
                tiles: self.tiles.clone(),
                reason,
            });
        }
        if self.cbr_padding && !matches!(self.rate_control, RateControlMode::Cbr { .. }) {
            return Err(Error::PaddingWithoutCbr);
        }
        match self.rate_control {
            RateControlMode::ConstantQp { qp } if qp > max_qindex(self.bit_depth) => {
                Err(Error::InvalidQp {
                    qp,
                    max: max_qindex(self.bit_depth),
                })
            }
            RateControlMode::Crf { crf } | RateControlMode::CappedCrf { crf, .. }
                if crf == 0 || crf > Crf::MAX =>
            {
                Err(Error::InvalidCrf { crf })
            }
            RateControlMode::Vbr { bitrate_kbps } => {
                check_bitrate(bitrate_kbps, None, self.frame_rate)
            }
            RateControlMode::Cbr {
                bitrate_kbps,
                buffer_ms,
                initial_ms,
            }
            | RateControlMode::CappedCrf {
                max_bitrate_kbps: bitrate_kbps,
                buffer_ms,
                initial_ms,
                ..
            } => check_cbr(bitrate_kbps, (buffer_ms, initial_ms), self.frame_rate),
            _ => Ok(()),
        }?;
        Ok(rows)
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

    /// The constant qindex of [`Self::first_pass_config`].
    pub const FIRST_PASS_QP: u16 = 128;

    /// The first pass of a two-pass VBR encode of this configuration: the
    /// frames at half size, at the constant qindex [`Self::FIRST_PASS_QP`],
    /// in the mini-GOPs that VBR codes (the preset's
    /// `bitrate_mini_gop_size`), so the first pass sees the second pass's
    /// prediction structure; no film grain, no cropping window and no first
    /// pass of its own.
    ///
    /// # Errors
    /// Returns [`Error::Preset`] when the preset overrides do not apply.
    pub fn first_pass_config(&self) -> Result<Self> {
        let params = self.override_preset(self.preset.params())?;
        let mut first = self.clone();
        first.resize_denominator = Some(16);
        first.rate_control = RateControlMode::ConstantQp {
            qp: Self::FIRST_PASS_QP,
        };
        first.film_grain = false;
        first.cbr_padding = false;
        first.crop = None;
        first.first_pass = Vec::new();
        first.preset_overrides.push((
            "mini_gop_size".to_owned(),
            params.bitrate_mini_gop_size.to_string(),
        ));
        Ok(first)
    }

    /// `row` with [`Tune::overrides`] of [`Self::tune`], then
    /// [`Self::FAST_DECODE`] (when [`Self::fast_decode`]), then
    /// [`Self::preset_overrides`] applied, checked by
    /// [`PresetParams::validate`].
    pub(crate) fn override_preset(&self, row: &PresetParams) -> Result<PresetParams> {
        self.apply_preset(row, |_, _| {})
    }

    fn apply_preset(
        &self,
        row: &PresetParams,
        mut record: impl FnMut(&str, PresetOrigin),
    ) -> Result<PresetParams> {
        let mut row = *row;
        for (name, value) in self.tune.overrides() {
            row.set(name, value)?;
            record(name, PresetOrigin::Tune);
        }
        if self.fast_decode {
            for (name, value) in Self::FAST_DECODE {
                row.set(name, value)?;
                record(name, PresetOrigin::FastDecode);
            }
        }
        for (name, value) in &self.preset_overrides {
            if self.is_lossless() && PresetParams::lossless_refuses(name, value) {
                return Err(Error::LosslessWith { what: name.clone() });
            }
            row.set(name, value)?;
            record(name, PresetOrigin::Explicit);
        }
        if self.is_lossless() {
            row.set_lossless_with(|name| record(name, PresetOrigin::Lossless));
        }
        row.validate()?;
        Ok(row)
    }

    /// Whether every frame is coded lossless: `ConstantQp` at `qp` 0.
    #[must_use]
    pub const fn is_lossless(&self) -> bool {
        matches!(self.rate_control, RateControlMode::ConstantQp { qp: 0 })
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

    /// `preset_overrides` win over the tune and fast-decode overrides:
    /// `--tune ssim` with `ssim_aq=0` codes the `--tune psnr` row, and
    /// `max_block_log2=7` undoes the 64x64 superblocks of `--fast-decode`.
    #[test]
    fn preset_overrides_win_over_tune_and_fast_decode() -> Result<()> {
        let mut config = Config::new(64, 64);
        let psnr = config.override_preset(config.preset.params())?;
        config.tune = Tune::Ssim;
        config.fast_decode = true;
        let tuned = config.override_preset(config.preset.params())?;
        assert_eq!((tuned.ssim_aq, tuned.max_block_log2), (30, 6));
        config.fast_decode = false;
        config.preset_overrides = vec![("ssim_aq".into(), "0".into())];
        assert_eq!(config.override_preset(config.preset.params())?, psnr);
        config.fast_decode = true;
        config
            .preset_overrides
            .push(("max_block_log2".into(), "7".into()));
        let row = config.override_preset(config.preset.params())?;
        assert_eq!(
            (row.ssim_aq, row.max_block_log2, row.gdf_candidates),
            (0, 7, 0)
        );
        Ok(())
    }

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
        config.tiles = TileLayout::Uniform {
            cols_log2: 0,
            rows_log2: TileLayout::MAX_LOG2 + 1,
        };
        assert!(matches!(config.validate(), Err(Error::InvalidTiles { .. })));
        for (widths, preset_set, reason) in [
            (vec![1, 0], None, "a tile of 0 superblocks"),
            (
                vec![1],
                Some(1),
                "seq_tile_info codes only a uniform layout",
            ),
        ] {
            config = Config::new(64, 64);
            config.tiles = TileLayout::Explicit {
                widths,
                heights: Vec::new(),
            };
            config.preset_overrides = preset_set
                .map(|value: u8| vec![("seq_tile_info".to_owned(), value.to_string())])
                .unwrap_or_default();
            assert!(
                matches!(config.validate(), Err(Error::InvalidTiles { reason: r, .. }) if r == reason)
            );
        }
        config = Config::new(64, 64);
        config.frame_rate = (30, 0);
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidFrameRate { .. })
        ));
        config = Config::new(64, 64);
        config.rate_control = RateControlMode::ConstantQp { qp: 0 };
        assert!(config.validate().is_ok() && config.is_lossless());
        let refused = |config: &Config| match config.validate() {
            Err(Error::LosslessWith { what }) => what,
            _ => String::new(),
        };
        for (name, value) in [
            ("segmentation", "1"),
            ("quant_matrices", "2"),
            ("y_dc_delta_q", "-3"),
            ("u_delta_q", "4"),
            ("v_delta_q", "1"),
        ] {
            let mut explicit = config.clone();
            explicit
                .preset_overrides
                .push((name.to_owned(), value.to_owned()));
            assert_eq!(refused(&explicit), name);
        }
        let mut grain = config.clone();
        grain.film_grain = true;
        assert_eq!(refused(&grain), "film grain");
        let mut resized = config.clone();
        resized.resize_denominator = Some(12);
        assert_eq!(refused(&resized), "resize");
        config
            .preset_overrides
            .push(("tcq".to_owned(), "1".to_owned()));
        let row = config.override_preset(config.preset.params());
        assert!(row.is_ok_and(|row| {
            !row.tcq.at(crate::preset::PresetContext::default()) && row.tpl_delta_q == 0
        }));
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
            initial_ms: 0,
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
            initial_ms: 0,
        };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidBitrate {
                what: "buffer_ms",
                ..
            })
        ));
        config.rate_control = RateControlMode::Cbr {
            bitrate_kbps: 100,
            buffer_ms: 500,
            initial_ms: 501,
        };
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidInitialBuffer {
                initial_ms: 501,
                buffer_ms: 500,
            })
        ));
        let capped = |crf, max_bitrate_kbps, (buffer_ms, initial_ms)| RateControlMode::CappedCrf {
            crf,
            max_bitrate_kbps,
            buffer_ms,
            initial_ms,
        };
        for (mode, what) in [
            (capped(30, 0, (500, 0)), "bitrate_kbps"),
            (capped(30, 100, (0, 0)), "buffer_ms"),
        ] {
            config.rate_control = mode;
            assert!(
                matches!(config.validate(), Err(Error::InvalidBitrate { what: w, .. }) if w == what)
            );
        }
        config.rate_control = capped(0, 100, (500, 250));
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidCrf { crf: 0 })
        ));
        config.rate_control = capped(30, 100, (500, 501));
        assert!(matches!(
            config.validate(),
            Err(Error::InvalidInitialBuffer {
                initial_ms: 501,
                ..
            })
        ));
        config.rate_control = capped(30, 100, (500, 500));
        assert!(config.validate().is_ok());
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
