// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! What the source declares about its samples, coded in front of every key
//! frame as a content interpretation OBU (§ 5.15) and HDR metadata OBUs
//! (§ 5.17.5, § 5.17.6).

use splot_core::headers::content_interpretation::{
    AspectRatioInfo, ChromaSamplePosition as CodedChromaSamplePosition, ColorDescription,
    ColorPrimariesTriple, ContentInterpretation, DerivedColorInfo,
};
use splot_core::headers::frame::{
    encode_content_interpretation_obu, encode_hdr_cll_obu, encode_hdr_mdcv_obu,
};
pub use splot_core::headers::metadata::MetadataHdrCll as ContentLightLevel;
use splot_core::headers::metadata::MetadataHdrMdcv;
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

/// The colour volume of the mastering display (`metadata_hdr_mdcv()`,
/// § 6.16.6), in the integer units of SMPTE ST 2086. A value outside the
/// range the specification gives for it means unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasteringDisplay {
    /// CIE 1931 `(x, y)` of the green, blue and red primaries, in units of
    /// 0.00002.
    pub primaries: [(u16, u16); 3],
    /// CIE 1931 `(x, y)` of the white point, in units of 0.00002.
    pub white_point: (u16, u16),
    /// Nominal maximum luminance in units of 0.0001 cd/m².
    pub luminance_max: u32,
    /// Nominal minimum luminance in units of 0.0001 cd/m².
    pub luminance_min: u32,
}

/// The description of the source content. The default describes nothing, and
/// the stream then has no content interpretation or HDR metadata OBU. The
/// encoder never guesses a value and never converts samples to match one.
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
    /// Mastering display colour volume of HDR content.
    pub mastering_display: Option<MasteringDisplay>,
    /// Content light level of HDR content: upper bounds in cd/m² of any
    /// sample (MaxCLL) and of a frame average (MaxFALL), § 6.16.5; 0 gives
    /// no bound.
    pub content_light_level: Option<ContentLightLevel>,
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
            mastering_display: None,
            content_light_level: None,
        }
    }
}

impl ContentDescription {
    /// `CP_UNSPECIFIED`, `TC_UNSPECIFIED` and `MC_UNSPECIFIED` (§ 6.14).
    pub const UNSPECIFIED: u8 = 2;

    /// Checks the description against the chroma format `chroma` and the
    /// § 6.16.6 rule that a display maximum of 50000 forbids that minimum.
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
        if self
            .mastering_display
            .is_some_and(|display| (display.luminance_max, display.luminance_min) == (50000, 50000))
        {
            return invalid("a mastering display maximum of 50000 needs another minimum");
        }
        Ok(())
    }

    /// The Annex B OBUs of a progressive stream at `frame_rate`, in the order
    /// of § 7.3.3: the content interpretation OBU, the multi-frame header
    /// OBUs `mfh`, then the mastering display and content light level
    /// metadata OBUs. Empty when nothing is described and `mfh` is empty.
    pub(crate) fn obus(&self, frame_rate: (u32, u32), mfh: &[u8]) -> Result<Vec<u8>> {
        let code_points = ColorPrimariesTriple {
            color_primaries: self.color_primaries,
            transfer_characteristics: self.transfer_characteristics,
            matrix_coefficients: self.matrix_coefficients,
        };
        let mut content = ContentInterpretation::progressive();
        content.color_description = (code_points != DerivedColorInfo::UNSPECIFIED.code_points
            || self.full_range)
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
        let mut obus = Vec::new();
        if content != ContentInterpretation::progressive() {
            obus = encode_content_interpretation_obu(&content)
                .map_err(crate::intra_frame::coding("content interpretation"))?;
        }
        obus.extend_from_slice(mfh);
        if let Some(display) = self.mastering_display {
            let [(gx, gy), (bx, by), (rx, ry)] = display.primaries;
            let obu = encode_hdr_mdcv_obu(MetadataHdrMdcv {
                primary_chromaticity_x: [gx, bx, rx],
                primary_chromaticity_y: [gy, by, ry],
                white_point_chromaticity_x: display.white_point.0,
                white_point_chromaticity_y: display.white_point.1,
                luminance_max: display.luminance_max,
                luminance_min: display.luminance_min,
            });
            obus.extend(obu.map_err(crate::intra_frame::coding("mastering display"))?);
        }
        if let Some(light_level) = self.content_light_level {
            let obu = encode_hdr_cll_obu(light_level);
            obus.extend(obu.map_err(crate::intra_frame::coding("content light level"))?);
        }
        Ok(obus)
    }
}
