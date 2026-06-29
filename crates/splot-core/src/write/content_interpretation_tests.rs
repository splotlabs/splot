// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::content_interpretation::{
        ColorPrimariesTriple, ExtendedSampleAspectRatio, ScanTypeIdc, parse_content_interpretation,
    };
    use crate::span::ByteOffset;

    /// Writes a content-interpretation body and reparses it, asserting model equality. The body is
    /// variable-width; the parser reads exactly the body bits and ignores the zero byte-padding
    /// `into_bytes` adds after a byte-aligned payload.
    fn round_trip(ci: &ContentInterpretation) {
        let mut writer = BitWriter::new();
        write_content_interpretation(&mut writer, ci).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let reparsed = parse_content_interpretation(&mut reader).unwrap();
        assert_eq!(&reparsed, ci);
    }

    /// A content-interpretation model with every optional structure absent.
    fn minimal(scan_type: u8, reserved_2bit: u8) -> ContentInterpretation {
        ContentInterpretation {
            scan_type_idc: ScanTypeIdc::from_bits(scan_type),
            color_description: None,
            chroma_sample_position: None,
            aspect_ratio: None,
            timing_info: None,
            reserved_2bit,
        }
    }


    #[test]
    fn minimal_all_options_absent_round_trips() {
        round_trip(&minimal(0, 0));
        round_trip(&minimal(3, 0));
    }

    #[test]
    fn color_description_idc_zero_explicit_triple_round_trips() {
        let mut ci = minimal(0, 0);
        ci.color_description = Some(ColorDescription {
            color_description_idc: 0,
            primaries: Some(ColorPrimariesTriple {
                color_primaries: 1,
                transfer_characteristics: 13,
                matrix_coefficients: 6,
            }),
            full_range_flag: true,
        });
        round_trip(&ci);
    }

    #[test]
    fn color_description_preset_idc_round_trips() {
        let mut ci = minimal(0, 0);
        ci.color_description = Some(ColorDescription {
            color_description_idc: 2,
            primaries: None,
            full_range_flag: false,
        });
        round_trip(&ci);
    }

    #[test]
    fn reserved_color_description_idc_round_trips_verbatim() {
        let mut ci = minimal(0, 0);
        ci.color_description = Some(ColorDescription {
            color_description_idc: 100,
            primaries: None,
            full_range_flag: true,
        });
        round_trip(&ci);
    }

    #[test]
    fn chroma_sample_position_interlace_codes_bottom_round_trips() {
        let mut ci = minimal(2, 0);
        ci.chroma_sample_position = Some(ChromaSamplePosition { top: 2, bottom: 5 });
        round_trip(&ci);
    }

    #[test]
    fn chroma_sample_position_progressive_infers_bottom_round_trips() {
        let mut ci = minimal(1, 0);
        ci.chroma_sample_position = Some(ChromaSamplePosition { top: 3, bottom: 3 });
        round_trip(&ci);
    }

    #[test]
    fn aspect_ratio_extended_sar_round_trips() {
        let mut ci = minimal(0, 0);
        ci.aspect_ratio = Some(AspectRatioInfo {
            aspect_ratio_idc: 255,
            extended_sar: Some(ExtendedSampleAspectRatio {
                sar_width: 16,
                sar_height: 9,
            }),
        });
        round_trip(&ci);
    }

    #[test]
    fn aspect_ratio_indexed_idc_round_trips() {
        let mut ci = minimal(0, 0);
        ci.aspect_ratio = Some(AspectRatioInfo {
            aspect_ratio_idc: 1,
            extended_sar: None,
        });
        round_trip(&ci);
    }

    #[test]
    fn reserved_aspect_ratio_idc_round_trips_verbatim() {
        let mut ci = minimal(0, 0);
        ci.aspect_ratio = Some(AspectRatioInfo {
            aspect_ratio_idc: 200,
            extended_sar: None,
        });
        round_trip(&ci);
    }

    #[test]
    fn nonzero_reserved_2bit_round_trips_verbatim() {
        for reserved in 0u8..=3 {
            round_trip(&minimal(0, reserved));
        }
    }

    #[test]
    fn timing_info_equal_picture_interval_round_trips() {
        let mut ci = minimal(1, 0);
        ci.timing_info = Some(TimingInfo {
            num_units_in_display_tick: 1000,
            time_scale: 30000,
            equal_picture_interval: true,
            num_ticks_per_picture_minus_1: Some(1),
        });
        round_trip(&ci);
    }

    #[test]
    fn timing_info_unequal_picture_interval_round_trips() {
        let mut ci = minimal(0, 0);
        ci.timing_info = Some(TimingInfo {
            num_units_in_display_tick: 24,
            time_scale: 1,
            equal_picture_interval: false,
            num_ticks_per_picture_minus_1: None,
        });
        round_trip(&ci);
    }

    #[test]
    fn all_structures_present_round_trips() {
        let ci = ContentInterpretation {
            scan_type_idc: ScanTypeIdc::from_bits(2),
            color_description: Some(ColorDescription {
                color_description_idc: 0,
                primaries: Some(ColorPrimariesTriple {
                    color_primaries: 9,
                    transfer_characteristics: 16,
                    matrix_coefficients: 9,
                }),
                full_range_flag: false,
            }),
            chroma_sample_position: Some(ChromaSamplePosition { top: 1, bottom: 4 }),
            aspect_ratio: Some(AspectRatioInfo {
                aspect_ratio_idc: 255,
                extended_sar: Some(ExtendedSampleAspectRatio {
                    sar_width: 4,
                    sar_height: 3,
                }),
            }),
            timing_info: Some(TimingInfo {
                num_units_in_display_tick: 1001,
                time_scale: 60000,
                equal_picture_interval: true,
                num_ticks_per_picture_minus_1: Some(0),
            }),
            reserved_2bit: 1,
        };
        round_trip(&ci);
    }


    /// Asserts `write_content_interpretation` rejects `ci` with the given `what` and writes nothing.
    fn reject(ci: &ContentInterpretation, what: &str) {
        let mut writer = BitWriter::new();
        let err = write_content_interpretation(&mut writer, ci).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalContentInterpretation { what: w } if w == what),
            "expected {what} reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "reject left bits in the writer");
    }

    #[test]
    fn rejects_unaligned_writer() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        let err = write_content_interpretation(&mut writer, &minimal(0, 0)).unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1, "unaligned reject left the writer untouched");
    }

    #[test]
    fn rejects_primaries_present_with_nonzero_idc() {
        let mut ci = minimal(0, 0);
        ci.color_description = Some(ColorDescription {
            color_description_idc: 1,
            primaries: Some(ColorPrimariesTriple {
                color_primaries: 1,
                transfer_characteristics: 1,
                matrix_coefficients: 1,
            }),
            full_range_flag: false,
        });
        reject(&ci, "color_primaries_idc");
    }

    #[test]
    fn rejects_primaries_absent_with_zero_idc() {
        let mut ci = minimal(0, 0);
        ci.color_description = Some(ColorDescription {
            color_description_idc: 0,
            primaries: None,
            full_range_flag: false,
        });
        reject(&ci, "color_primaries_idc");
    }

    #[test]
    fn rejects_progressive_chroma_with_differing_bottom() {
        let mut ci = minimal(1, 0);
        ci.chroma_sample_position = Some(ChromaSamplePosition { top: 3, bottom: 7 });
        reject(&ci, "chroma_bottom_progressive");
    }

    #[test]
    fn rejects_extended_sar_present_with_non_255_idc() {
        let mut ci = minimal(0, 0);
        ci.aspect_ratio = Some(AspectRatioInfo {
            aspect_ratio_idc: 1,
            extended_sar: Some(ExtendedSampleAspectRatio {
                sar_width: 16,
                sar_height: 9,
            }),
        });
        reject(&ci, "extended_sar_idc");
    }

    #[test]
    fn rejects_extended_sar_absent_with_255_idc() {
        let mut ci = minimal(0, 0);
        ci.aspect_ratio = Some(AspectRatioInfo {
            aspect_ratio_idc: 255,
            extended_sar: None,
        });
        reject(&ci, "extended_sar_idc");
    }

    #[test]
    fn rejects_timing_ticks_present_without_equal_interval() {
        let mut ci = minimal(0, 0);
        ci.timing_info = Some(TimingInfo {
            num_units_in_display_tick: 1,
            time_scale: 1,
            equal_picture_interval: false,
            num_ticks_per_picture_minus_1: Some(5),
        });
        reject(&ci, "timing_num_ticks_gate");
    }

    #[test]
    fn rejects_timing_ticks_absent_with_equal_interval() {
        let mut ci = minimal(0, 0);
        ci.timing_info = Some(TimingInfo {
            num_units_in_display_tick: 1,
            time_scale: 1,
            equal_picture_interval: true,
            num_ticks_per_picture_minus_1: None,
        });
        reject(&ci, "timing_num_ticks_gate");
    }

    #[test]
    fn rejects_zero_num_units_in_display_tick() {
        let mut ci = minimal(0, 0);
        ci.timing_info = Some(TimingInfo {
            num_units_in_display_tick: 0,
            time_scale: 1,
            equal_picture_interval: false,
            num_ticks_per_picture_minus_1: None,
        });
        reject(&ci, "timing_display_tick_zero");
    }

    #[test]
    fn rejects_zero_time_scale() {
        let mut ci = minimal(0, 0);
        ci.timing_info = Some(TimingInfo {
            num_units_in_display_tick: 1,
            time_scale: 0,
            equal_picture_interval: false,
            num_ticks_per_picture_minus_1: None,
        });
        reject(&ci, "timing_time_scale_zero");
    }
}
