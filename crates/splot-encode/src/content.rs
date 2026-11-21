// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! What the source declares about its samples, coded as a content
//! interpretation OBU (§ 5.15) in front of every key frame.

use splot_core::headers::content_interpretation::{
    AspectRatioInfo, ChromaSamplePosition as CodedChromaSamplePosition, ColorDescription,
    ContentInterpretation,
};
use splot_core::headers::frame::encode_content_interpretation_obu;
use splot_core::headers::sequence::TimingInfo;
use splot_recon::PixelFormat;

use crate::error::{Error, Result};

/// Where the chroma samples of a 4:2:0 or 4:2:2 picture sit against the luma
/// grid (`ci_chroma_sample_position_top`, § 6.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaSamplePosition {
    /// `CSP_LEFT`: on a luma column, between two luma rows in 4:2:0.
    Left = 0,
    /// `CSP_CENTER`: between two luma columns, between two luma rows in 4:2:0.
    Center = 1,
    /// `CSP_TOPLEFT`: on a luma sample; 4:2:0 only.
    TopLeft = 2,
    /// `CSP_TOP`: between two luma columns, on a luma row; 4:2:0 only.
    Top = 3,
    /// `CSP_BOTTOMLEFT`: on a luma column, on the lower luma row; 4:2:0 only.
    BottomLeft = 4,
    /// `CSP_BOTTOM`: between two luma columns, on the lower luma row; 4:2:0 only.
    Bottom = 5,
}

/// The description of the source content. The default describes nothing, and
/// the stream then has no content interpretation OBU. The encoder never
/// guesses a value and never converts samples to match one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentDescription {
    /// H.273 `ColourPrimaries` code point; [`Self::UNSPECIFIED`] by default.
    pub color_primaries: u8,
    /// H.273 `TransferCharacteristics` code point; [`Self::UNSPECIFIED`] by default.
    pub transfer_characteristics: u8,
    /// H.273 `MatrixCoefficients` code point; [`Self::UNSPECIFIED`] by default.
    pub matrix_coefficients: u8,
    /// Full-range samples; limited range by default (`ci_full_range_flag`).
    pub full_range: bool,
    /// Chroma sample position of a 4:2:0 or 4:2:2 source.
    pub chroma_sample_position: Option<ChromaSamplePosition>,
    /// Sample aspect ratio as `(width, height)`.
    pub sample_aspect_ratio: Option<(u32, u32)>,
    /// Codes the frame rate of the configuration as `timing_info()`
    /// (§ 5.4.12) with one tick per picture.
    pub timing_info: bool,
}

impl Default for ContentDescription {
    fn default() -> Self {
        Self {
            color_primaries: Self::UNSPECIFIED,
            transfer_characteristics: Self::UNSPECIFIED,
            matrix_coefficients: Self::UNSPECIFIED,
            full_range: false,
            chroma_sample_position: None,
            sample_aspect_ratio: None,
            timing_info: false,
        }
    }
}

impl ContentDescription {
    /// `CP_UNSPECIFIED`, `TC_UNSPECIFIED` and `MC_UNSPECIFIED` (§ 6.14).
    pub const UNSPECIFIED: u8 = 2;

    /// Checks the description against the chroma format `chroma`.
    pub(crate) fn validate(&self, chroma: PixelFormat) -> Result<()> {
        let invalid = |what| Err(Error::InvalidContentDescription { what });
        match (self.chroma_sample_position, chroma) {
            (None, _)
            | (Some(_), PixelFormat::Yuv420)
            | (
                Some(ChromaSamplePosition::Left | ChromaSamplePosition::Center),
                PixelFormat::Yuv422,
            ) => {}
            (Some(_), PixelFormat::Yuv422) => {
                return invalid("a 4:2:2 chroma sample position is left or center");
            }
            (Some(_), _) => {
                return invalid("a chroma sample position needs 4:2:0 or 4:2:2 input");
            }
        }
        if matches!(self.sample_aspect_ratio, Some((0, _) | (_, 0))) {
            return invalid("a sample aspect ratio has nonzero terms");
        }
        Ok(())
    }

    /// The Annex B content interpretation OBU of a progressive stream at
    /// `frame_rate`; empty when nothing is described.
    pub(crate) fn obu(&self, frame_rate: (u32, u32)) -> Result<Vec<u8>> {
        let code_points = (
            self.color_primaries,
            self.transfer_characteristics,
            self.matrix_coefficients,
        );
        let unspecified = (Self::UNSPECIFIED, Self::UNSPECIFIED, Self::UNSPECIFIED);
        let mut content = ContentInterpretation::progressive();
        content.color_description = (code_points != unspecified || self.full_range)
            .then(|| ColorDescription::from_code_points(code_points, self.full_range));
        content.chroma_sample_position =
            self.chroma_sample_position
                .map(|position| CodedChromaSamplePosition {
                    top: position as u32,
                    bottom: position as u32,
                });
        content.aspect_ratio = self
            .sample_aspect_ratio
            .map(|(width, height)| AspectRatioInfo::from_sample_aspect_ratio(width, height));
        content.timing_info = self.timing_info.then_some(TimingInfo {
            num_units_in_display_tick: frame_rate.1,
            time_scale: frame_rate.0,
            equal_picture_interval: true,
            num_ticks_per_picture_minus_1: Some(0),
        });
        if content == ContentInterpretation::progressive() {
            return Ok(Vec::new());
        }
        encode_content_interpretation_obu(&content)
            .map_err(crate::intra_frame::coding("content interpretation"))
    }
}
