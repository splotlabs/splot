// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! What the source declares about its samples, coded in front of every key
//! frame as a content interpretation OBU (§ 5.15) and metadata OBUs
//! (§ 5.17).

use splot_core::headers::content_interpretation::{
    AspectRatioInfo, ChromaSamplePosition as CodedChromaSamplePosition, ColorDescription,
    ColorPrimariesTriple, ContentInterpretation, DerivedColorInfo,
};
use splot_core::headers::frame::{encode_content_interpretation_obu, encode_metadata_obu};
pub use splot_core::headers::metadata::MetadataHdrCll as ContentLightLevel;
use splot_core::headers::metadata::{
    MetadataHdrMdcv, MetadataIccProfile, MetadataItutT35, MetadataPayload, MetadataType,
    MetadataUserDataUnregistered,
};
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
/// § 6.16.6), in the integer units of SMPTE ST 2086. Each field is in its
/// § 6.16.6 range, or 0 for unknown.
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

impl MasteringDisplay {
    /// Checks each field against its § 6.16.6 range (allowing 0 for unknown)
    /// and the rule that a maximum of 50000 forbids a minimum of 50000.
    pub(crate) fn validate(&self) -> Result<()> {
        const PRIMARY_X: [&str; 3] = [
            "primary_chromaticity_x[0]",
            "primary_chromaticity_x[1]",
            "primary_chromaticity_x[2]",
        ];
        const PRIMARY_Y: [&str; 3] = [
            "primary_chromaticity_y[0]",
            "primary_chromaticity_y[1]",
            "primary_chromaticity_y[2]",
        ];
        let check = |field, value: u32, min, max| {
            if value != 0 && !(min..=max).contains(&value) {
                return Err(Error::InvalidMasteringDisplay {
                    field,
                    value,
                    min,
                    max,
                });
            }
            Ok(())
        };
        for (i, &(x, y)) in self.primaries.iter().enumerate() {
            check(PRIMARY_X[i], u32::from(x), 5, 37_000)?;
            check(PRIMARY_Y[i], u32::from(y), 5, 42_000)?;
        }
        check(
            "white_point_chromaticity_x",
            u32::from(self.white_point.0),
            5,
            37_000,
        )?;
        check(
            "white_point_chromaticity_y",
            u32::from(self.white_point.1),
            5,
            42_000,
        )?;
        check("luminance_max", self.luminance_max, 50_000, 100_000_000)?;
        check("luminance_min", self.luminance_min, 1, 50_000)?;
        if (self.luminance_max, self.luminance_min) == (50_000, 50_000) {
            return Err(Error::InvalidContentDescription {
                what: "a mastering display maximum of 50000 needs another minimum",
            });
        }
        Ok(())
    }
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
    /// § 6.16.6 mastering display rules.
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
        if let Some(display) = self.mastering_display {
            display.validate()?;
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
            let display = MetadataHdrMdcv {
                primary_chromaticity_x: [gx, bx, rx],
                primary_chromaticity_y: [gy, by, ry],
                white_point_chromaticity_x: display.white_point.0,
                white_point_chromaticity_y: display.white_point.1,
                luminance_max: display.luminance_max,
                luminance_min: display.luminance_min,
            };
            let payload = MetadataPayload::HdrMdcv(display);
            let obu = encode_metadata_obu(MetadataType::HdrMdcv, payload, &[]);
            obus.extend(obu.map_err(crate::intra_frame::coding("mastering display"))?);
        }
        if let Some(light_level) = self.content_light_level {
            let payload = MetadataPayload::HdrCll(light_level);
            let obu = encode_metadata_obu(MetadataType::HdrCll, payload, &[]);
            obus.extend(obu.map_err(crate::intra_frame::coding("content light level"))?);
        }
        Ok(obus)
    }
}

/// Metadata payloads that the encoder copies unchanged into a prefix metadata
/// OBU in front of every key frame, after the HDR metadata (§ 5.17, § 7.3.3).
/// None by default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyFrameMetadata {
    /// The ICC profile bytes of `metadata_icc_profile()` (§ 5.17.9).
    pub icc_profile: Option<Vec<u8>>,
    /// The bytes of `metadata_itut_t35()` (§ 5.17.4): the country code, an
    /// extension byte when the code is 0xFF, then the payload.
    pub itut_t35: Option<Vec<u8>>,
    /// The bytes of `metadata_user_data_unregistered()` (§ 5.17.13): a
    /// 16-byte UUID, then the payload.
    pub user_data_unregistered: Option<Vec<u8>>,
}

impl KeyFrameMetadata {
    /// Checks that each payload has the fields its syntax starts with: the
    /// 128-byte header of an ICC profile (ISO 15076-1, § 6.16.9), a T.35
    /// country code (with its extension byte) and terminal provider code
    /// (§ 6.16.4), and a user data UUID.
    pub(crate) fn validate(&self) -> Result<()> {
        let invalid = |what| Err(Error::InvalidMetadata { what });
        if self.icc_profile.as_ref().is_some_and(|icc| icc.len() < 128) {
            return invalid("an ICC profile starts with its 128-byte header (§ 6.16.9)");
        }
        match self.itut_t35.as_deref() {
            Some([]) => return invalid("T.35 data starts with a country code"),
            Some([0xFF]) => return invalid("T.35 country code 0xFF has an extension byte"),
            Some([_] | [0xFF, _]) => {
                return invalid("T.35 data has a terminal provider code after the country code");
            }
            _ => {}
        }
        if self
            .user_data_unregistered
            .as_ref()
            .is_some_and(|data| data.len() < 16)
        {
            return invalid("user data starts with a 16-byte UUID");
        }
        Ok(())
    }

    /// The Annex B metadata OBUs: ICC profile, T.35, then user data.
    pub(crate) fn obus(&self) -> Result<Vec<u8>> {
        let mut obus = Vec::new();
        let mut push = |metadata_type, payload, passthrough: &[u8], what| -> Result<()> {
            let obu = encode_metadata_obu(metadata_type, payload, passthrough);
            obus.extend(obu.map_err(crate::intra_frame::coding(what))?);
            Ok(())
        };
        if let Some(icc) = &self.icc_profile {
            let payload_len = icc.len();
            let payload = MetadataPayload::IccProfile(MetadataIccProfile { payload_len });
            push(MetadataType::IccProfile, payload, icc, "ICC profile")?;
        }
        if let Some(&[country_code, ref rest @ ..]) = self.itut_t35.as_deref() {
            let (extension, payload) = match rest {
                [extension, payload @ ..] if country_code == 0xFF => (Some(*extension), payload),
                _ => (None, rest),
            };
            let payload_t35 = MetadataPayload::ItutT35(MetadataItutT35 {
                itu_t_t35_country_code: country_code,
                itu_t_t35_country_code_extension_byte: extension,
                payload_len: payload.len(),
            });
            push(MetadataType::ItutT35, payload_t35, payload, "T.35")?;
        }
        if let Some((uuid, payload)) = self
            .user_data_unregistered
            .as_deref()
            .and_then(|data| data.split_first_chunk::<16>())
        {
            let payload_user =
                MetadataPayload::UserDataUnregistered(MetadataUserDataUnregistered {
                    uuid_iso_iec_11578: *uuid,
                    payload_len: payload.len(),
                });
            push(
                MetadataType::UserDataUnregistered,
                payload_user,
                payload,
                "user data",
            )?;
        }
        Ok(obus)
    }
}
