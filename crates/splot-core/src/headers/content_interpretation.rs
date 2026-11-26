// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 content interpretation OBU syntax model (AV2 v1.0.0 § 5.15).
//!
//! `content_interpretation_obu()` carries per-embedded-layer presentation metadata
//! (scan type, color description, chroma sample position, aspect ratio) and, when
//! `ci_timing_info_present_flag` is set, the `timing_info()` structure shared with
//! [`crate::headers::sequence`]. This parser reads the full § 5.15 syntax; it never
//! skips unknown payload bits. Cross-embedded-layer timing consistency (§ 6.4.12)
//! and repeated-identity checks are handled by `splot-validate`.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::sequence::{TimingInfo, parse_timing_info_with_reader};
use crate::syntax::{NoopObserver, SyntaxObserver, SyntaxReader};

/// `ci_scan_type_idc` (AV2 v1.0.0 § 5.15 / § 6.14): how to interpret pictures in a
/// CVS in terms of progressive or interlace samples (0 unspecified, 1 progressive
/// frame, 2 interlace field, 3 interlace complementary field-pair).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanTypeIdc(u8);

impl ScanTypeIdc {
    /// Creates a scan-type id from a value already known to fit the 2-bit field.
    #[must_use]
    pub const fn from_bits(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw `ci_scan_type_idc` value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// `true` when `ci_scan_type_idc == 1` (progressive frame). When false, the
    /// content-interpretation OBU codes a separate bottom chroma sample position
    /// (AV2 § 5.15).
    #[must_use]
    pub const fn is_progressive_frame(self) -> bool {
        self.0 == 1
    }
}

/// The `ci_color_description_idc == 0` triple of H.273 code points (AV2 § 5.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorPrimariesTriple {
    /// `ci_color_primaries` (`f(8)`).
    pub color_primaries: u8,
    /// `ci_transfer_characteristics` (`f(8)`).
    pub transfer_characteristics: u8,
    /// `ci_matrix_coefficients` (`f(8)`).
    pub matrix_coefficients: u8,
}

/// `ci_color_description_present_flag` payload (AV2 § 5.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorDescription {
    /// `ci_color_description_idc` (`rg(2)`).
    pub color_description_idc: u32,
    /// Present only when `ci_color_description_idc == 0`.
    pub primaries: Option<ColorPrimariesTriple>,
    /// `ci_full_range_flag` (`f(1)`).
    pub full_range_flag: bool,
}

/// The unspecified `(CP_UNSPECIFIED, TC_UNSPECIFIED, MC_UNSPECIFIED)` color-primaries
/// triple (AV2 § 6.14: each constant is `2`). It is the default when no color
/// description is present and the derived value of any reserved color id.
const UNSPECIFIED_COLOR_PRIMARIES: ColorPrimariesTriple = triple(2, 2, 2);

const fn triple(
    color_primaries: u8,
    transfer_characteristics: u8,
    matrix_coefficients: u8,
) -> ColorPrimariesTriple {
    ColorPrimariesTriple {
        color_primaries,
        transfer_characteristics,
        matrix_coefficients,
    }
}

/// Derived color information for AV2 § 6.14 "same information" comparisons.
///
/// A content-interpretation OBU can encode the same color information in more than
/// one way (an explicit triple with `ci_color_description_idc == 0`, a Table 6.13
/// preset id, a reserved id, or by omitting the color description entirely — all of
/// which resolve to a defined value), so two layers carry the same information iff
/// their *derived* values match. See [`ColorDescription::derived`] and
/// [`ContentInterpretation::derived_color`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DerivedColorInfo {
    /// The H.273 code points: the explicit triple for `idc == 0`, the Table 6.13
    /// preset for `idc` in `1..=5`, or the unspecified `(2, 2, 2)` for a reserved
    /// `idc` (`6..=127`), which decoders ignore.
    pub code_points: ColorPrimariesTriple,
    /// `ci_full_range_flag`.
    pub full_range: bool,
}

impl DerivedColorInfo {
    /// The derived color information when no color description is present (AV2
    /// § 5.15: `ci_color_*` default to `*_UNSPECIFIED` and `ci_full_range_flag` to 0).
    pub const UNSPECIFIED: Self = Self {
        code_points: UNSPECIFIED_COLOR_PRIMARIES,
        full_range: false,
    };
}

impl ColorDescription {
    /// The shortest description of the H.273 `code_points` with
    /// `full_range_flag`: the Table 6.13 preset id whose triple they equal, else
    /// the explicit triple.
    #[must_use]
    pub fn from_code_points(code_points: ColorPrimariesTriple, full_range_flag: bool) -> Self {
        let preset = |color_description_idc| Self {
            color_description_idc,
            primaries: None,
            full_range_flag,
        };
        (1..=5)
            .map(preset)
            .find(|preset| preset.derived().code_points == code_points)
            .unwrap_or(Self {
                color_description_idc: 0,
                primaries: Some(code_points),
                full_range_flag,
            })
    }

    /// Returns whether two declarations contain different known information.
    ///
    /// The comparison uses the derived color and aspect ratio values (AV2
    /// § 6.14). Reserved bits and an undecidable aspect ratio do not establish
    /// differences.
    #[must_use]
    pub fn information_differs(&self, other: &Self) -> bool {
        self.scan_type_idc != other.scan_type_idc
            || self.chroma_sample_position != other.chroma_sample_position
            || self.timing_info != other.timing_info
            || self.derived_color() != other.derived_color()
            || matches!(
                (self.derived_sample_aspect_ratio(), other.derived_sample_aspect_ratio()),
                (Some(a), Some(b)) if a != b
            )
    }

    /// Returns the derived color information per AV2 § 6.14 (Table 6.13): the explicit
    /// values for `idc == 0`, the preset triple for `idc` in `1..=5`, or the
    /// unspecified `(2, 2, 2)` for a reserved `idc` (`6..=127`), which decoders ignore
    /// (so it carries the same color information as an absent or explicitly
    /// unspecified description).
    #[must_use]
    pub fn derived(&self) -> DerivedColorInfo {
        let code_points = match self.color_description_idc {
            0 => self.primaries.unwrap_or(UNSPECIFIED_COLOR_PRIMARIES),
            1 => triple(1, 1, 1),             // BT.709 SDR
            2 => triple(9, 16, 9),            // BT.2100 PQ
            3 => triple(9, 18, 9),            // BT.2100 HLG
            4 => triple(1, 13, 0),            // sRGB
            5 => triple(1, 13, 5),            // sYCC
            _ => UNSPECIFIED_COLOR_PRIMARIES, // 6..=127 reserved -> ignored by decoders
        };
        DerivedColorInfo {
            code_points,
            full_range: self.full_range_flag,
        }
    }
}

/// `ci_chroma_sample_position_present_flag` payload (AV2 § 5.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromaSamplePosition {
    /// `ci_chroma_sample_position_top` (`uvlc()`).
    pub top: u32,
    /// `ci_chroma_sample_position_bottom`; equals [`Self::top`] when
    /// `ci_scan_type_idc == 1`.
    pub bottom: u32,
}

/// Explicit sample aspect ratio when `ci_aspect_ratio_idc == 255` (AV2 § 5.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtendedSampleAspectRatio {
    /// `ci_sar_width` (`uvlc()`).
    pub sar_width: u32,
    /// `ci_sar_height` (`uvlc()`).
    pub sar_height: u32,
}

/// `(Aspect_Ratio_Width[i], Aspect_Ratio_Height[i])` for
/// `ci_aspect_ratio_idc` in `0..=16` (AV2 § 5.15).
const ASPECT_RATIOS: [(u32, u32); 17] = [
    (0, 0),
    (1, 1),
    (12, 11),
    (10, 11),
    (16, 11),
    (40, 33),
    (24, 11),
    (20, 11),
    (32, 11),
    (80, 33),
    (18, 11),
    (15, 11),
    (64, 33),
    (160, 99),
    (4, 3),
    (3, 2),
    (2, 1),
];

/// `ci_aspect_ratio_info_present_flag` payload (AV2 § 5.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AspectRatioInfo {
    /// `ci_aspect_ratio_idc` (`f(8)`).
    pub aspect_ratio_idc: u8,
    /// Present only when `ci_aspect_ratio_idc == 255`; otherwise the SAR is looked
    /// up in `Aspect_Ratio_Width`/`Aspect_Ratio_Height`.
    pub extended_sar: Option<ExtendedSampleAspectRatio>,
}

impl AspectRatioInfo {
    /// The shortest description of the sample aspect ratio `width:height`: the
    /// `ci_aspect_ratio_idc` in `1..=16` whose ratio it equals, else the explicit
    /// pair (AV2 § 5.15). A zero term is the unspecified ratio.
    #[must_use]
    pub fn from_sample_aspect_ratio(width: u32, height: u32) -> Self {
        let table = |aspect_ratio_idc| Self {
            aspect_ratio_idc,
            extended_sar: None,
        };
        let explicit = Self {
            aspect_ratio_idc: 255,
            extended_sar: Some(ExtendedSampleAspectRatio {
                sar_width: width,
                sar_height: height,
            }),
        };
        (1..=16)
            .map(table)
            .find(|table| table.derived_sar() == explicit.derived_sar())
            .unwrap_or(explicit)
    }

    /// Returns the derived sample aspect ratio as a *normalized* `(width, height)`
    /// ratio (AV2 § 5.15): the explicit SAR for `ci_aspect_ratio_idc == 255`, the
    /// `Aspect_Ratio_Width`/`Aspect_Ratio_Height` entry for `idc` in `0..=16`, or
    /// `None` for a reserved `idc` (`17..=254`, which is itself a § 6.14 conformance
    /// violation flagged elsewhere).
    ///
    /// § 5.15 defines the SAR as a ratio "in the same arbitrary units", so the pair
    /// is reduced by its greatest common divisor (e.g. `2:2` derives to `1:1`), and
    /// any pair with a zero dimension — which § 5.15 defines as unspecified — maps to
    /// the single canonical `(0, 0)`. This makes the value suitable for "same
    /// information" comparisons.
    #[must_use]
    pub fn derived_sar(&self) -> Option<(u32, u32)> {
        let raw = if self.aspect_ratio_idc == 255 {
            self.extended_sar.map(|s| (s.sar_width, s.sar_height))
        } else {
            ASPECT_RATIOS
                .get(usize::from(self.aspect_ratio_idc))
                .copied()
        };
        raw.map(|(w, h)| normalize_sample_aspect_ratio(w, h))
    }
}

/// Reduces a sample aspect ratio to lowest terms for "same information" comparison
/// (AV2 § 5.15). A zero in either dimension means the SAR is unspecified, mapped to
/// the canonical `(0, 0)`.
fn normalize_sample_aspect_ratio(width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let divisor = gcd(width, height);
    (width / divisor, height / divisor)
}

/// Greatest common divisor (Euclid's algorithm); `gcd(a, 0) == a`.
const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

/// Parsed `content_interpretation_obu()` syntax (AV2 v1.0.0 § 5.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContentInterpretation {
    /// `ci_scan_type_idc`.
    pub scan_type_idc: ScanTypeIdc,
    /// `ci_color_description_present_flag` payload, when present.
    pub color_description: Option<ColorDescription>,
    /// `ci_chroma_sample_position_present_flag` payload, when present.
    pub chroma_sample_position: Option<ChromaSamplePosition>,
    /// `ci_aspect_ratio_info_present_flag` payload, when present.
    pub aspect_ratio: Option<AspectRatioInfo>,
    /// `ci_timing_info_present_flag` payload, when present.
    pub timing_info: Option<TimingInfo>,
    /// `ci_reserved_2bit`; AV2 § 6.14 requires this to be 0 (the value is otherwise
    /// ignored by a decoder). The validator surfaces a non-zero value.
    pub reserved_2bit: u8,
}

impl ContentInterpretation {
    /// A progressive-frame OBU (`ci_scan_type_idc == 1`) with no optional payload.
    #[must_use]
    pub const fn progressive() -> Self {
        Self {
            scan_type_idc: ScanTypeIdc::from_bits(1),
            color_description: None,
            chroma_sample_position: None,
            aspect_ratio: None,
            timing_info: None,
            reserved_2bit: 0,
        }
    }

    /// Returns the derived color information, resolving an absent color description
    /// to its § 5.15 default (`*_UNSPECIFIED`, full range 0). Suitable for § 6.14
    /// "same information" comparisons across OBUs regardless of how (or whether) the
    /// color description is signalled.
    #[must_use]
    pub fn derived_color(&self) -> DerivedColorInfo {
        self.color_description
            .map_or(DerivedColorInfo::UNSPECIFIED, |c| c.derived())
    }

    /// Returns the derived sample aspect ratio (normalized, see
    /// [`AspectRatioInfo::derived_sar`]), resolving an absent aspect ratio to the
    /// unspecified `(0, 0)`. Returns `None` only for a reserved `ci_aspect_ratio_idc`
    /// (`17..=254`), which is itself a § 6.14 conformance violation flagged elsewhere.
    #[must_use]
    pub fn derived_sample_aspect_ratio(&self) -> Option<(u32, u32)> {
        self.aspect_ratio.map_or(Some((0, 0)), |a| a.derived_sar())
    }
}

/// Parses `content_interpretation_obu()` (AV2 v1.0.0 § 5.15).
///
/// The full syntax is read, including the optional color-description,
/// chroma-sample-position, aspect-ratio, and timing branches. The parser never
/// skips payload bits.
///
/// # Errors
/// Returns descriptor errors (`rg`/`uvlc`), the local timing-range
/// [`Error::InvalidSequenceHeader`](crate::error::Error::InvalidSequenceHeader)
/// violations from `parse_timing_info()`, or
/// [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload ends
/// mid-field.
pub fn parse_content_interpretation(reader: &mut BitReader<'_>) -> Result<ContentInterpretation> {
    let mut noop = NoopObserver;
    let mut sr = SyntaxReader::new(reader, &mut noop);
    parse_content_interpretation_with_reader(&mut sr)
}

/// Parses `content_interpretation_obu()` (AV2 v1.0.0 § 5.15) while reporting
/// syntax fields and bit spans to `observer`.
///
/// # Errors
/// Returns descriptor errors, [`Error::InvalidSequenceHeader`](crate::error::Error::InvalidSequenceHeader),
/// or [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof).
pub fn parse_content_interpretation_observed(
    reader: &mut BitReader<'_>,
    observer: &mut dyn SyntaxObserver,
) -> Result<ContentInterpretation> {
    let mut observer = observer;
    let mut sr = SyntaxReader::new(reader, &mut observer);
    parse_content_interpretation_with_reader(&mut sr)
}

pub(crate) fn parse_content_interpretation_with_reader<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
) -> Result<ContentInterpretation> {
    sr.in_struct("content_interpretation_obu", Some("5.15"), |sr| {
        let scan_type_idc = ScanTypeIdc::from_bits(sr.read_bits_u8("ci_scan_type_idc", 2)?);
        let color_description_present_flag = sr.read_flag("ci_color_description_present_flag")?;
        let chroma_sample_position_present_flag =
            sr.read_flag("ci_chroma_sample_position_present_flag")?;
        let aspect_ratio_info_present_flag = sr.read_flag("ci_aspect_ratio_info_present_flag")?;
        let timing_info_present_flag = sr.read_flag("ci_timing_info_present_flag")?;
        let reserved_2bit = sr.read_bits_u8("ci_reserved_2bit", 2)?;

        let color_description = if color_description_present_flag {
            let color_description_idc = sr.read_rg("ci_color_description_idc", 2)?;
            let primaries = if color_description_idc == 0 {
                Some(ColorPrimariesTriple {
                    color_primaries: sr.read_bits_u8("ci_color_primaries", 8)?,
                    transfer_characteristics: sr.read_bits_u8("ci_transfer_characteristics", 8)?,
                    matrix_coefficients: sr.read_bits_u8("ci_matrix_coefficients", 8)?,
                })
            } else {
                sr.absent("ci_color_primaries", "ci_color_description_idc != 0");
                sr.absent(
                    "ci_transfer_characteristics",
                    "ci_color_description_idc != 0",
                );
                sr.absent("ci_matrix_coefficients", "ci_color_description_idc != 0");
                None
            };
            let full_range_flag = sr.read_flag("ci_full_range_flag")?;
            Some(ColorDescription {
                color_description_idc,
                primaries,
                full_range_flag,
            })
        } else {
            sr.absent(
                "ci_color_description_idc",
                "!ci_color_description_present_flag",
            );
            None
        };

        let chroma_sample_position = if chroma_sample_position_present_flag {
            let top = sr.read_uvlc("ci_chroma_sample_position_top")?;
            let bottom = if scan_type_idc.is_progressive_frame() {
                sr.inferred_u64(
                    "ci_chroma_sample_position_bottom",
                    "ci_scan_type_idc == 1",
                    u64::from(top),
                );
                top
            } else {
                sr.read_uvlc("ci_chroma_sample_position_bottom")?
            };
            Some(ChromaSamplePosition { top, bottom })
        } else {
            sr.absent(
                "ci_chroma_sample_position_top",
                "!ci_chroma_sample_position_present_flag",
            );
            None
        };

        let aspect_ratio = if aspect_ratio_info_present_flag {
            let aspect_ratio_idc = sr.read_bits_u8("ci_aspect_ratio_idc", 8)?;
            let extended_sar = if aspect_ratio_idc == 255 {
                Some(ExtendedSampleAspectRatio {
                    sar_width: sr.read_uvlc("ci_sar_width")?,
                    sar_height: sr.read_uvlc("ci_sar_height")?,
                })
            } else {
                sr.absent("ci_sar_width", "ci_aspect_ratio_idc != 255");
                sr.absent("ci_sar_height", "ci_aspect_ratio_idc != 255");
                None
            };
            Some(AspectRatioInfo {
                aspect_ratio_idc,
                extended_sar,
            })
        } else {
            sr.absent("ci_aspect_ratio_idc", "!ci_aspect_ratio_info_present_flag");
            None
        };

        let timing_info = if timing_info_present_flag {
            Some(parse_timing_info_with_reader(sr)?)
        } else {
            None
        };

        Ok(ContentInterpretation {
            scan_type_idc,
            color_description,
            chroma_sample_position,
            aspect_ratio,
            timing_info,
            reserved_2bit,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    /// The six leading fixed fields with every optional-flag cleared.
    fn fixed_header(scan_type: u32, reserved_2bit: u32) -> Bits {
        let mut bits = Bits::default();
        bits.f(scan_type, 2); // ci_scan_type_idc
        bits.bit(0); // ci_color_description_present_flag
        bits.bit(0); // ci_chroma_sample_position_present_flag
        bits.bit(0); // ci_aspect_ratio_info_present_flag
        bits.bit(0); // ci_timing_info_present_flag
        bits.f(reserved_2bit, 2); // ci_reserved_2bit
        bits
    }

    fn timing_info_bits(
        display_tick: u32,
        time_scale: u32,
        equal_picture_interval: bool,
        num_ticks_minus_1: u32,
    ) -> Bits {
        let mut bits = Bits::default();
        bits.f(display_tick, 32);
        bits.f(time_scale, 32);
        bits.bit(u8::from(equal_picture_interval));
        if equal_picture_interval {
            bits.uvlc(num_ticks_minus_1);
        }
        bits
    }

    #[test]
    fn parses_with_all_optional_flags_false() {
        let data = fixed_header(0, 0).into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        assert_eq!(ci.scan_type_idc.get(), 0);
        assert_eq!(ci.color_description, None);
        assert_eq!(ci.chroma_sample_position, None);
        assert_eq!(ci.aspect_ratio, None);
        assert_eq!(ci.timing_info, None);
        assert_eq!(ci.reserved_2bit, 0);
    }

    #[test]
    fn parses_with_timing_info_present() {
        let mut bits = Bits::default();
        bits.f(1, 2); // ci_scan_type_idc = 1 (progressive)
        bits.bit(0); // color description absent
        bits.bit(0); // chroma sample position absent
        bits.bit(0); // aspect ratio absent
        bits.bit(1); // ci_timing_info_present_flag
        bits.f(0, 2); // ci_reserved_2bit
        let timing = timing_info_bits(1000, 30000, true, 1);
        bits.bits.extend(timing.bits);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        let timing = ci.timing_info.unwrap();
        assert_eq!(timing.num_units_in_display_tick, 1000);
        assert_eq!(timing.time_scale, 30000);
        assert!(timing.equal_picture_interval);
        assert_eq!(timing.num_ticks_per_picture_minus_1, Some(1));
    }

    #[test]
    fn parses_color_description_with_primaries_and_full_range() {
        let mut bits = Bits::default();
        bits.f(0, 2); // ci_scan_type_idc
        bits.bit(1); // ci_color_description_present_flag
        bits.bit(0); // chroma sample position absent
        bits.bit(0); // aspect ratio absent
        bits.bit(0); // timing absent
        bits.f(0, 2); // ci_reserved_2bit
        bits.rg(0, 2); // ci_color_description_idc = 0 -> primaries present
        bits.f(1, 8); // ci_color_primaries
        bits.f(13, 8); // ci_transfer_characteristics
        bits.f(6, 8); // ci_matrix_coefficients
        bits.bit(1); // ci_full_range_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        let color = ci.color_description.unwrap();
        assert_eq!(color.color_description_idc, 0);
        let primaries = color.primaries.unwrap();
        assert_eq!(primaries.color_primaries, 1);
        assert_eq!(primaries.transfer_characteristics, 13);
        assert_eq!(primaries.matrix_coefficients, 6);
        assert!(color.full_range_flag);
    }

    #[test]
    fn parses_color_description_nonzero_idc_skips_primaries() {
        let mut bits = Bits::default();
        bits.f(0, 2);
        bits.bit(1); // color description present
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        bits.f(0, 2);
        bits.rg(2, 2); // ci_color_description_idc = 2 -> no primaries triple
        bits.bit(0); // ci_full_range_flag
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        let color = ci.color_description.unwrap();
        assert_eq!(color.color_description_idc, 2);
        assert_eq!(color.primaries, None);
        assert!(!color.full_range_flag);
    }

    #[test]
    fn parses_chroma_sample_position_with_bottom_when_not_progressive() {
        let mut bits = Bits::default();
        bits.f(2, 2); // ci_scan_type_idc = 2 (interlace) -> bottom is coded
        bits.bit(0); // color description absent
        bits.bit(1); // ci_chroma_sample_position_present_flag
        bits.bit(0); // aspect ratio absent
        bits.bit(0); // timing absent
        bits.f(0, 2);
        bits.uvlc(2); // ci_chroma_sample_position_top
        bits.uvlc(5); // ci_chroma_sample_position_bottom
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        let chroma = ci.chroma_sample_position.unwrap();
        assert_eq!(chroma.top, 2);
        assert_eq!(chroma.bottom, 5);
    }

    #[test]
    fn chroma_sample_position_infers_bottom_when_progressive() {
        let mut bits = Bits::default();
        bits.f(1, 2); // ci_scan_type_idc = 1 (progressive) -> bottom inferred
        bits.bit(0);
        bits.bit(1); // chroma sample position present
        bits.bit(0);
        bits.bit(0);
        bits.f(0, 2);
        bits.uvlc(3); // top only
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        let chroma = ci.chroma_sample_position.unwrap();
        assert_eq!(chroma.top, 3);
        assert_eq!(chroma.bottom, 3);
    }

    #[test]
    fn parses_aspect_ratio_extended_sar_path() {
        let mut bits = Bits::default();
        bits.f(0, 2);
        bits.bit(0);
        bits.bit(0);
        bits.bit(1); // ci_aspect_ratio_info_present_flag
        bits.bit(0);
        bits.f(0, 2);
        bits.f(255, 8); // ci_aspect_ratio_idc = 255 -> extended SAR
        bits.uvlc(16); // ci_sar_width
        bits.uvlc(9); // ci_sar_height
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        let aspect = ci.aspect_ratio.unwrap();
        assert_eq!(aspect.aspect_ratio_idc, 255);
        let sar = aspect.extended_sar.unwrap();
        assert_eq!(sar.sar_width, 16);
        assert_eq!(sar.sar_height, 9);
    }

    #[test]
    fn parses_aspect_ratio_indexed_path() {
        let mut bits = Bits::default();
        bits.f(0, 2);
        bits.bit(0);
        bits.bit(0);
        bits.bit(1); // aspect ratio present
        bits.bit(0);
        bits.f(0, 2);
        bits.f(1, 8); // ci_aspect_ratio_idc = 1 -> no extended SAR
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        let aspect = ci.aspect_ratio.unwrap();
        assert_eq!(aspect.aspect_ratio_idc, 1);
        assert_eq!(aspect.extended_sar, None);
    }

    #[test]
    fn preserves_reserved_2bit_value() {
        let data = fixed_header(0, 0b10).into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let ci = parse_content_interpretation(&mut reader).unwrap();
        assert_eq!(ci.reserved_2bit, 0b10);
    }

    #[test]
    fn reports_eof_in_fixed_header() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_content_interpretation(&mut reader),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn reports_eof_inside_timing_info() {
        let mut bits = Bits::default();
        bits.f(0, 2);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(1); // timing present
        bits.f(0, 2);
        bits.f(0xFFFF, 16); // truncated num_units_in_display_tick (needs 32 bits)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_content_interpretation(&mut reader),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn derived_color_normalizes_presets_and_explicit() {
        let preset = ColorDescription {
            color_description_idc: 1,
            primaries: None,
            full_range_flag: false,
        };
        let explicit = ColorDescription {
            color_description_idc: 0,
            primaries: Some(ColorPrimariesTriple {
                color_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
            }),
            full_range_flag: false,
        };
        assert_eq!(preset.derived(), explicit.derived());
        assert_eq!(preset.derived().code_points, triple(1, 1, 1));

        let preset_triple = |idc: u32| {
            ColorDescription {
                color_description_idc: idc,
                primaries: None,
                full_range_flag: false,
            }
            .derived()
            .code_points
        };
        assert_eq!(preset_triple(2), triple(9, 16, 9)); // BT.2100 PQ
        assert_eq!(preset_triple(3), triple(9, 18, 9)); // BT.2100 HLG
        assert_eq!(preset_triple(4), triple(1, 13, 0)); // sRGB
        assert_eq!(preset_triple(5), triple(1, 13, 5)); // sYCC

        let bt2100_pq = ColorDescription {
            color_description_idc: 2,
            primaries: None,
            full_range_flag: false,
        };
        assert_ne!(preset.derived(), bt2100_pq.derived());

        let reserved = ColorDescription {
            color_description_idc: 50,
            primaries: None,
            full_range_flag: true,
        };
        assert_eq!(reserved.derived().code_points, triple(2, 2, 2));
        assert!(reserved.derived().full_range);

        let explicit_unspecified = ColorDescription {
            color_description_idc: 0,
            primaries: Some(ColorPrimariesTriple {
                color_primaries: 2,
                transfer_characteristics: 2,
                matrix_coefficients: 2,
            }),
            full_range_flag: true,
        };
        assert_eq!(reserved.derived(), explicit_unspecified.derived());
    }

    #[test]
    fn shortest_descriptions_keep_the_derived_values() {
        for (code_points, idc) in [
            (triple(1, 1, 1), 1),
            (triple(9, 16, 9), 2),
            (triple(1, 13, 5), 5),
        ] {
            let color = ColorDescription::from_code_points(code_points, true);
            assert_eq!((color.color_description_idc, color.primaries), (idc, None));
            assert_eq!(color.derived().code_points, code_points);
            assert!(color.full_range_flag);
        }
        let explicit = ColorDescription::from_code_points(triple(9, 16, 0), false);
        assert_eq!(explicit.color_description_idc, 0);
        assert_eq!(explicit.derived().code_points, triple(9, 16, 0));

        for (sar, idc) in [((1, 1), 1), ((8, 8), 1), ((80, 33), 9), ((4, 2), 16)] {
            let aspect = AspectRatioInfo::from_sample_aspect_ratio(sar.0, sar.1);
            assert_eq!((aspect.aspect_ratio_idc, aspect.extended_sar), (idc, None));
        }
        let explicit = AspectRatioInfo::from_sample_aspect_ratio(5, 7);
        assert_eq!(explicit.aspect_ratio_idc, 255);
        assert_eq!(explicit.derived_sar(), Some((5, 7)));

        let ci = ContentInterpretation::progressive();
        assert!(ci.scan_type_idc.is_progressive_frame());
        assert_eq!(ci.derived_color(), DerivedColorInfo::UNSPECIFIED);
    }

    #[test]
    fn derived_sar_normalizes_table_and_explicit() {
        let preset = AspectRatioInfo {
            aspect_ratio_idc: 1,
            extended_sar: None,
        };
        let explicit = AspectRatioInfo {
            aspect_ratio_idc: 255,
            extended_sar: Some(ExtendedSampleAspectRatio {
                sar_width: 1,
                sar_height: 1,
            }),
        };
        assert_eq!(preset.derived_sar(), Some((1, 1)));
        assert_eq!(preset.derived_sar(), explicit.derived_sar());

        let other = AspectRatioInfo {
            aspect_ratio_idc: 2,
            extended_sar: None,
        };
        assert_eq!(other.derived_sar(), Some((12, 11)));
        assert_ne!(preset.derived_sar(), other.derived_sar());

        let reserved = AspectRatioInfo {
            aspect_ratio_idc: 17,
            extended_sar: None,
        };
        assert_eq!(reserved.derived_sar(), None);

        let unreduced = AspectRatioInfo {
            aspect_ratio_idc: 255,
            extended_sar: Some(ExtendedSampleAspectRatio {
                sar_width: 2,
                sar_height: 2,
            }),
        };
        assert_eq!(unreduced.derived_sar(), Some((1, 1)));
        assert_eq!(unreduced.derived_sar(), preset.derived_sar());

        let explicit_zero = AspectRatioInfo {
            aspect_ratio_idc: 255,
            extended_sar: Some(ExtendedSampleAspectRatio {
                sar_width: 0,
                sar_height: 1,
            }),
        };
        let preset_zero = AspectRatioInfo {
            aspect_ratio_idc: 0,
            extended_sar: None,
        };
        assert_eq!(explicit_zero.derived_sar(), Some((0, 0)));
        assert_eq!(explicit_zero.derived_sar(), preset_zero.derived_sar());
    }

    #[test]
    fn propagates_timing_range_violation() {
        let mut bits = Bits::default();
        bits.f(0, 2);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(1); // timing present
        bits.f(0, 2);
        bits.f(0, 32); // num_units_in_display_tick = 0 -> § 6.4.12 violation
        bits.f(1, 32);
        bits.bit(0);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_content_interpretation(&mut reader),
            Err(Error::InvalidSequenceHeader { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The content-interpretation parser must never panic on arbitrary input.
        #[test]
        fn content_interpretation_parser_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..128),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_content_interpretation(&mut reader);
        }
    }
}
