// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::metadata::{
        MetadataHdrCll, MetadataIccProfile, MetadataTemporalPointInfo, MetadataUnknownRaw,
        MetadataUserDataUnregistered,
    };
    use crate::span::ByteOffset;
    use crate::types::GLOBAL_XLAYER_ID;

    /// Builds a `MetadataUnit` directly from a `metadata_type`, the declared `payload_size`, and a
    /// payload variant.
    fn unit(metadata_type: MetadataType, payload_size: usize, payload: MetadataPayload) -> MetadataUnit {
        MetadataUnit {
            metadata_type,
            payload_size,
            payload,
        }
    }

    /// Round-trips a `metadata_short_obu()` model: writes it, reparses, asserts model equality.
    fn short_round_trip(obu: &MetadataShortObu, passthrough: &[u8]) {
        let mut writer = BitWriter::new();
        write_metadata_short_obu(&mut writer, obu, passthrough).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_short(&bytes).unwrap();
        assert_eq!(&reparsed, obu);
    }

    fn short_obu(
        metadata_type: MetadataType,
        leb_bytes: u8,
        unit: Option<MetadataUnit>,
        cancel: bool,
    ) -> MetadataShortObu {
        MetadataShortObu {
            metadata_is_suffix: false,
            muh_layer_idc: 0,
            muh_cancel_flag: cancel,
            muh_persistence_idc: 0,
            metadata_type,
            metadata_type_leb128_bytes: leb_bytes,
            unit,
        }
    }


    #[test]
    fn short_hdr_cll_round_trips() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0x1234,
            max_fall: 0x5678,
        });
        let obu = short_obu(MetadataType::HdrCll, 1, Some(unit(MetadataType::HdrCll, 4, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_hdr_mdcv_round_trips() {
        let payload = MetadataPayload::HdrMdcv(MetadataHdrMdcv {
            primary_chromaticity_x: [10, 30, 50],
            primary_chromaticity_y: [20, 40, 60],
            white_point_chromaticity_x: 70,
            white_point_chromaticity_y: 80,
            luminance_max: 1_000_000,
            luminance_min: 5,
        });
        let obu = short_obu(MetadataType::HdrMdcv, 1, Some(unit(MetadataType::HdrMdcv, 24, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_itut_t35_without_extension_round_trips() {
        let payload = MetadataPayload::ItutT35(MetadataItutT35 {
            itu_t_t35_country_code: 0x01,
            itu_t_t35_country_code_extension_byte: None,
            payload_len: 3,
        });
        let obu = short_obu(MetadataType::ItutT35, 1, Some(unit(MetadataType::ItutT35, 4, payload)), false);
        short_round_trip(&obu, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn short_itut_t35_with_extension_round_trips() {
        let payload = MetadataPayload::ItutT35(MetadataItutT35 {
            itu_t_t35_country_code: 0xFF,
            itu_t_t35_country_code_extension_byte: Some(0x42),
            payload_len: 2,
        });
        let obu = short_obu(MetadataType::ItutT35, 1, Some(unit(MetadataType::ItutT35, 4, payload)), false);
        short_round_trip(&obu, &[0xAA, 0xBB]);
    }

    #[test]
    fn short_timecode_full_timestamp_round_trips() {
        let payload = MetadataPayload::Timecode(MetadataTimecode {
            counting_type: 0,
            full_timestamp_flag: true,
            discontinuity_flag: false,
            cnt_dropped_flag: false,
            n_frames: 7,
            seconds_value: Some(59),
            minutes_value: Some(58),
            hours_value: Some(23),
            time_offset_length: 0,
            time_offset_value: None,
        });
        let obu = short_obu(MetadataType::Timecode, 1, Some(unit(MetadataType::Timecode, 5, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_timecode_partial_with_offset_round_trips() {
        let payload = MetadataPayload::Timecode(MetadataTimecode {
            counting_type: 1,
            full_timestamp_flag: false,
            discontinuity_flag: false,
            cnt_dropped_flag: false,
            n_frames: 0,
            seconds_value: Some(30),
            minutes_value: None,
            hours_value: None,
            time_offset_length: 4,
            time_offset_value: Some(0b1010),
        });
        let obu = short_obu(MetadataType::Timecode, 1, Some(unit(MetadataType::Timecode, 5, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_scan_type_round_trips() {
        let payload = MetadataPayload::ScanType(MetadataScanType {
            mps_pic_struct_type: 12,
            mps_source_scan_type_idc: 1,
            mps_duplicate_flag: true,
        });
        let obu = short_obu(MetadataType::ScanType, 1, Some(unit(MetadataType::ScanType, 1, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_temporal_point_info_round_trips() {
        let payload = MetadataPayload::TemporalPointInfo(MetadataTemporalPointInfo {
            frame_presentation_time: 300,
        });
        let obu = short_obu(
            MetadataType::TemporalPointInfo,
            1,
            Some(unit(MetadataType::TemporalPointInfo, 3, payload)),
            false,
        );
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_decoded_frame_hash_per_plane_round_trips() {
        let payload = MetadataPayload::DecodedFrameHash(MetadataDecodedFrameHash {
            hash_type: 0,
            per_plane: true,
            has_grain: false,
            is_monochrome: false,
            reserved: 0,
            plane_hashes: vec![[0u8; 16], [1u8; 16], [2u8; 16]],
            frame_hash: None,
        });
        let obu = short_obu(
            MetadataType::DecodedFrameHash,
            1,
            Some(unit(MetadataType::DecodedFrameHash, 49, payload)),
            false,
        );
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_decoded_frame_hash_single_round_trips() {
        let payload = MetadataPayload::DecodedFrameHash(MetadataDecodedFrameHash {
            hash_type: 3,
            per_plane: false,
            has_grain: true,
            is_monochrome: false,
            reserved: 1,
            plane_hashes: vec![],
            frame_hash: Some([0xAB; 16]),
        });
        let obu = short_obu(
            MetadataType::DecodedFrameHash,
            1,
            Some(unit(MetadataType::DecodedFrameHash, 17, payload)),
            false,
        );
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_banding_hints_basic_round_trips() {
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: None,
        });
        let obu = short_obu(MetadataType::BandingHints, 1, Some(unit(MetadataType::BandingHints, 1, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_banding_hints_with_band_units_round_trips() {
        let detail = BandingHintsDetail {
            three_color_components_flag: false,
            components: vec![BandingComponent {
                banding_in_component_present_flag: true,
                max_band_width_minus_4: Some(5),
                max_band_step_minus_1: Some(2),
            }],
            band_units: Some(BandUnits {
                num_band_units_rows_minus_1: 1,
                num_band_units_cols_minus_1: 0,
                varying_size: None,
                banding_in_band_unit_present: vec![true, false],
            }),
        };
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: Some(detail),
        });
        let obu = short_obu(MetadataType::BandingHints, 1, Some(unit(MetadataType::BandingHints, 4, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_banding_hints_varying_size_round_trips() {
        let detail = BandingHintsDetail {
            three_color_components_flag: true,
            components: vec![
                BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                },
                BandingComponent {
                    banding_in_component_present_flag: true,
                    max_band_width_minus_4: Some(10),
                    max_band_step_minus_1: Some(3),
                },
                BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                },
            ],
            band_units: Some(BandUnits {
                num_band_units_rows_minus_1: 1,
                num_band_units_cols_minus_1: 1,
                varying_size: Some(VaryingBandUnits {
                    band_block_in_luma_samples: 4,
                    vert_size_in_band_blocks_minus_1: vec![1, 2],
                    horz_size_in_band_blocks_minus_1: vec![3, 4],
                }),
                banding_in_band_unit_present: vec![true, false, false, true],
            }),
        };
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: true,
            hints: Some(detail),
        });
        let obu = short_obu(MetadataType::BandingHints, 1, Some(unit(MetadataType::BandingHints, 8, payload)), false);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_icc_profile_round_trips() {
        let payload = MetadataPayload::IccProfile(MetadataIccProfile { payload_len: 12 });
        let obu = short_obu(MetadataType::IccProfile, 1, Some(unit(MetadataType::IccProfile, 12, payload)), false);
        short_round_trip(&obu, &[0xAB; 12]);
    }

    #[test]
    fn short_user_data_unregistered_round_trips() {
        let payload = MetadataPayload::UserDataUnregistered(MetadataUserDataUnregistered {
            uuid_iso_iec_11578: [7u8; 16],
            payload_len: 5,
        });
        let obu = short_obu(
            MetadataType::UserDataUnregistered,
            1,
            Some(unit(MetadataType::UserDataUnregistered, 21, payload)),
            false,
        );
        short_round_trip(&obu, &[0xCD; 5]);
    }

    #[test]
    fn short_unknown_raw_round_trips() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 3 });
        let obu = short_obu(MetadataType::Reserved(0), 1, Some(unit(MetadataType::Reserved(0), 3, payload)), false);
        short_round_trip(&obu, &[0xDE, 0xAD, 0xBE]);
    }

    #[test]
    fn short_cancel_round_trips() {
        let obu = short_obu(MetadataType::Timecode, 1, None, true);
        short_round_trip(&obu, &[]);
    }

    #[test]
    fn short_canonical_bytes_are_exact() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0x1234,
            max_fall: 0x5678,
        });
        let obu = short_obu(MetadataType::HdrCll, 1, Some(unit(MetadataType::HdrCll, 4, payload)), false);
        let mut writer = BitWriter::new();
        write_metadata_short_obu(&mut writer, &obu, &[]).unwrap();
        assert_eq!(writer.into_bytes(), vec![0x00, 0x01, 0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn short_non_minimal_leb_type_round_trips_byte_exact() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0,
            max_fall: 0,
        });
        let obu = short_obu(MetadataType::HdrCll, 2, Some(unit(MetadataType::HdrCll, 4, payload)), false);
        let mut writer = BitWriter::new();
        write_metadata_short_obu(&mut writer, &obu, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(bytes, vec![0x00, 0x81, 0x00, 0x00, 0x00, 0x00, 0x00]);
        let reparsed = reparse_short(&bytes).unwrap();
        assert_eq!(reparsed, obu);
    }


    #[test]
    fn short_layer_idc_out_of_domain_is_rejected() {
        let mut obu = short_obu(MetadataType::Timecode, 1, None, true);
        obu.muh_layer_idc = 8;
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "muh_field_domain" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn short_cancel_with_unit_is_rejected() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll { max_cll: 0, max_fall: 0 });
        let obu = short_obu(MetadataType::HdrCll, 1, Some(unit(MetadataType::HdrCll, 4, payload)), true);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "short_cancel_unit" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn short_non_cancel_without_unit_is_rejected() {
        let obu = short_obu(MetadataType::HdrCll, 1, None, false);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "short_cancel_unit" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn short_cancel_with_passthrough_is_rejected() {
        let obu = short_obu(MetadataType::Timecode, 1, None, true);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[0x00]),
            Err(WriteError::NonCanonicalMetadata { what: "passthrough_len" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn short_undersized_leb_len_is_rejected() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let obu = short_obu(MetadataType::Reserved(200), 1, Some(unit(MetadataType::Reserved(200), 0, payload)), false);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "metadata_type_leb_len" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn short_type_payload_mismatch_is_rejected() {
        let payload = MetadataPayload::ScanType(MetadataScanType {
            mps_pic_struct_type: 0,
            mps_source_scan_type_idc: 0,
            mps_duplicate_flag: false,
        });
        let obu = short_obu(MetadataType::HdrCll, 1, Some(unit(MetadataType::ScanType, 1, payload)), false);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "type_payload_mismatch" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn unit_payload_overflows_size_is_rejected() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll { max_cll: 0, max_fall: 0 });
        let u = unit(MetadataType::HdrCll, 2, payload);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_unit(&mut writer, &u, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "payload_overflows_size" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn unit_inconsistent_payload_size_is_rejected() {
        let payload = MetadataPayload::ItutT35(MetadataItutT35 {
            itu_t_t35_country_code: 0x01,
            itu_t_t35_country_code_extension_byte: None,
            payload_len: 3,
        });
        let u = unit(MetadataType::ItutT35, 5, payload);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_unit(&mut writer, &u, &[0xAA, 0xBB, 0xCC]),
            Err(WriteError::NonCanonicalMetadata { what: "unit_payload_size" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn unit_type_payload_mismatch_is_rejected() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll { max_cll: 0, max_fall: 0 });
        let u = unit(MetadataType::ScanType, 4, payload);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_unit(&mut writer, &u, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "type_payload_mismatch" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn payload_non_empty_passthrough_for_modeled_is_rejected() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll { max_cll: 0, max_fall: 0 });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[0x00]),
            Err(WriteError::NonCanonicalMetadata { what: "passthrough_len" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_passthrough_len_mismatch_is_rejected() {
        let payload = MetadataPayload::IccProfile(MetadataIccProfile { payload_len: 4 });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[0x00, 0x01]),
            Err(WriteError::NonCanonicalMetadata { what: "passthrough_len" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_itut_t35_extension_mismatch_is_rejected() {
        let payload = MetadataPayload::ItutT35(MetadataItutT35 {
            itu_t_t35_country_code: 0xFF,
            itu_t_t35_country_code_extension_byte: None,
            payload_len: 0,
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "itut_t35_extension" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_timecode_domain_is_rejected() {
        let payload = MetadataPayload::Timecode(MetadataTimecode {
            counting_type: 32, // f(5) domain is 0..=31
            full_timestamp_flag: false,
            discontinuity_flag: false,
            cnt_dropped_flag: false,
            n_frames: 0,
            seconds_value: None,
            minutes_value: None,
            hours_value: None,
            time_offset_length: 0,
            time_offset_value: None,
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "timecode_domain" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_timecode_presence_is_rejected() {
        let payload = MetadataPayload::Timecode(MetadataTimecode {
            counting_type: 0,
            full_timestamp_flag: true,
            discontinuity_flag: false,
            cnt_dropped_flag: false,
            n_frames: 0,
            seconds_value: Some(0),
            minutes_value: Some(0),
            hours_value: None,
            time_offset_length: 0,
            time_offset_value: None,
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "timecode_presence" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_scan_type_domain_is_rejected() {
        let payload = MetadataPayload::ScanType(MetadataScanType {
            mps_pic_struct_type: 0,
            mps_source_scan_type_idc: 4, // f(2) domain is 0..=3
            mps_duplicate_flag: false,
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "scan_type_domain" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_frame_hash_presence_is_rejected() {
        let payload = MetadataPayload::DecodedFrameHash(MetadataDecodedFrameHash {
            hash_type: 0,
            per_plane: true,
            has_grain: false,
            is_monochrome: false,
            reserved: 0,
            plane_hashes: vec![[0u8; 16], [0u8; 16], [0u8; 16]],
            frame_hash: Some([0u8; 16]),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "frame_hash_presence" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_frame_hash_domain_is_rejected() {
        let payload = MetadataPayload::DecodedFrameHash(MetadataDecodedFrameHash {
            hash_type: 16, // f(4) domain is 0..=15
            per_plane: false,
            has_grain: false,
            is_monochrome: false,
            reserved: 0,
            plane_hashes: vec![],
            frame_hash: Some([0u8; 16]),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "frame_hash_domain" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_banding_hints_presence_is_rejected() {
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: false,
            source_banding_present_flag: false,
            hints: Some(BandingHintsDetail {
                three_color_components_flag: false,
                components: vec![BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                }],
                band_units: None,
            }),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "banding_hints_presence" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_banding_component_count_is_rejected() {
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: Some(BandingHintsDetail {
                three_color_components_flag: true,
                components: vec![BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                }],
                band_units: None,
            }),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "banding_component_count" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_band_units_present_count_is_rejected() {
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: Some(BandingHintsDetail {
                three_color_components_flag: false,
                components: vec![BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                }],
                band_units: Some(BandUnits {
                    num_band_units_rows_minus_1: 1,
                    num_band_units_cols_minus_1: 0,
                    varying_size: None,
                    banding_in_band_unit_present: vec![true],
                }),
            }),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata { what: "band_units_present_count" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    fn group_round_trip(obu: &MetadataGroupObu, xlayer: ExtendedLayerId, passthrough: &[&[u8]]) {
        let mut writer = BitWriter::new();
        write_metadata_group_obu(&mut writer, obu, xlayer, passthrough).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_group(&bytes, xlayer).unwrap();
        assert_eq!(&reparsed, obu);
    }

    fn cancel_group_unit(metadata_type: MetadataType, header_size: u8) -> MetadataGroupUnit {
        MetadataGroupUnit {
            metadata_type,
            muh_header_size: header_size,
            muh_cancel_flag: true,
            muh_payload_size: None,
            muh_layer_idc: None,
            muh_persistence_idc: None,
            muh_priority: None,
            muh_reserved_zero_2bits: None,
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: usize::from(header_size),
            unit: None,
        }
    }

    /// A non-cancel HdrCll group unit. `header_size` must equal payload_size_bytes + 2 +
    /// layer_map_bytes + header_extension_len.
    fn hdr_cll_group_unit() -> MetadataGroupUnit {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0x1234,
            max_fall: 0x5678,
        });
        MetadataGroupUnit {
            metadata_type: MetadataType::HdrCll,
            muh_header_size: 3, // payload_size leb (1) + fixed 2 = 3
            muh_cancel_flag: false,
            muh_payload_size: Some(4),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::HdrCll, 4, payload)),
        }
    }

    #[test]
    fn group_single_cancelled_unit_round_trips() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![cancel_group_unit(MetadataType::Timecode, 0)],
        };
        group_round_trip(&obu, ExtendedLayerId::from_bits(0), &[&[]]);
    }

    #[test]
    fn group_single_hdr_cll_unit_round_trips() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![hdr_cll_group_unit()],
        };
        group_round_trip(&obu, ExtendedLayerId::from_bits(0), &[&[]]);
    }

    #[test]
    fn group_canonical_bytes_are_exact() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![hdr_cll_group_unit()],
        };
        let mut writer = BitWriter::new();
        write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]).unwrap();
        assert_eq!(
            writer.into_bytes(),
            vec![0x00, 0x00, 0x01, 0x06, 0x04, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78]
        );
    }

    #[test]
    fn group_local_mlayer_map_round_trips() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 4,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(LAYER_VALUES),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![0b0000_0110],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        group_round_trip(&obu, ExtendedLayerId::from_bits(2), &[&[]]);
    }

    #[test]
    fn group_global_xlayer_map_round_trips() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 8,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(LAYER_VALUES),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: Some(1), // bit 0 set
            muh_mlayer_maps: vec![0xAA],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        group_round_trip(&obu, GLOBAL_XLAYER_ID, &[&[]]);
    }

    #[test]
    fn group_header_extension_bytes_round_trip() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 4,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 1,
            unit: Some(unit(MetadataType::Reserved(0), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        group_round_trip(&obu, ExtendedLayerId::from_bits(0), &[&[]]);
    }

    #[test]
    fn group_two_units_round_trip() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 1,
            metadata_application_id: 2,
            units: vec![
                cancel_group_unit(MetadataType::Timecode, 0),
                hdr_cll_group_unit(),
            ],
        };
        group_round_trip(&obu, ExtendedLayerId::from_bits(0), &[&[], &[]]);
    }

    /// A non-cancel `UnknownRaw` group unit declaring a `raw_len`-byte opaque blob. `muh_header_size`
    /// = payload_size leb (1, for raw_len < 128) + fixed 2, no layer maps, no header extension.
    fn unknown_raw_group_unit(raw_len: usize) -> MetadataGroupUnit {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len });
        MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 3,
            muh_cancel_flag: false,
            muh_payload_size: Some(raw_len as u32),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), raw_len, payload)),
        }
    }

    #[test]
    fn group_flat_passthrough_splits_per_unit_blob_len() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![
                unknown_raw_group_unit(3),
                cancel_group_unit(MetadataType::Timecode, 0),
            ],
        };
        let xlayer = ExtendedLayerId::from_bits(0);
        let blob = [0xDEu8, 0xAD, 0xBE];

        let mut flat = BitWriter::new();
        write_metadata_group_obu_flat(&mut flat, &obu, xlayer, &blob).unwrap();
        let flat_bytes = flat.into_bytes();

        let mut split = BitWriter::new();
        write_metadata_group_obu(&mut split, &obu, xlayer, &[&blob[..], &[]]).unwrap();
        assert_eq!(flat_bytes, split.into_bytes(), "flat split != pre-split output");

        let reparsed = reparse_group(&flat_bytes, xlayer).unwrap();
        assert_eq!(&reparsed, &obu);
    }

    #[test]
    fn group_flat_passthrough_total_mismatch_is_rejected() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unknown_raw_group_unit(3)],
        };
        let mut writer = BitWriter::new();
        let err =
            write_metadata_group_obu_flat(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[0xDE, 0xAD])
                .unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMetadata { what } if what == "group_passthrough_len"),
            "expected group_passthrough_len, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn group_passthrough_count_mismatch_is_rejected() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![cancel_group_unit(MetadataType::Timecode, 0)],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[]),
            Err(WriteError::NonCanonicalMetadata { what: "group_passthrough_count" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_empty_units_is_rejected() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[]),
            Err(WriteError::NonCanonicalMetadata { what: "group_unit_count" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_header_domain_is_rejected() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 4, // f(2) domain is 0..=3
            metadata_application_id: 0,
            units: vec![cancel_group_unit(MetadataType::Timecode, 0)],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata { what: "group_header_domain" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_cancel_fields_present_is_rejected() {
        let mut unit = cancel_group_unit(MetadataType::Timecode, 0);
        unit.muh_payload_size = Some(0); // must be None on cancel
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata { what: "group_cancel_fields" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_cancel_header_size_mismatch_is_rejected() {
        let mut unit = cancel_group_unit(MetadataType::Timecode, 2);
        unit.header_extension_len = 1;
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata { what: "muh_header_size" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_header_size_underflow_is_rejected() {
        let mut unit = hdr_cll_group_unit();
        unit.muh_header_size = 1; // needs >= 3
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata { what: "muh_header_size" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_payload_size_leb_len_is_rejected() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 200 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 3,
            muh_cancel_flag: false,
            muh_payload_size: Some(200),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), 200, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[0u8; 200]]),
            Err(WriteError::NonCanonicalMetadata { what: "muh_payload_size_leb_len" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_layer_map_count_is_rejected() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 4,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(LAYER_VALUES),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![0x01, 0x02],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata { what: "layer_map_count" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_muh_payload_size_mismatch_is_rejected() {
        let mut unit = hdr_cll_group_unit();
        unit.muh_payload_size = Some(6); // unit payload_size is 4
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata { what: "muh_payload_size" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn leb128_with_len_pads_and_reparses() {
        for len in 1usize..=5 {
            let mut writer = BitWriter::new();
            write_leb128_with_len(&mut writer, 1, len, "test").unwrap();
            let bytes = writer.into_bytes();
            assert_eq!(bytes.len(), len, "leb len {len}");
            let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
            assert_eq!(reader.read_leb128().unwrap(), 1);
            assert_eq!(reader.byte_offset().get(), len as u64);
        }
    }

    #[test]
    fn leb128_with_len_rejects_undersized() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_leb128_with_len(&mut writer, 200, 1, "test"),
            Err(WriteError::NonCanonicalMetadata { what: "test" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn group_noncancel_missing_field_is_rejected() {
        let mut unit = hdr_cll_group_unit();
        unit.muh_priority = None;
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata {
                what: "group_noncancel_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_muh_header_size_domain_is_rejected() {
        let mut unit = cancel_group_unit(MetadataType::Timecode, 0);
        unit.muh_header_size = 200;
        unit.header_extension_len = 200;
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata {
                what: "muh_header_size_domain"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_global_missing_xlayer_map_is_rejected() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 8,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(LAYER_VALUES),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, GLOBAL_XLAYER_ID, &[&[]]),
            Err(WriteError::NonCanonicalMetadata {
                what: "layer_map_presence"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn group_reserved_named_metadata_type_is_rejected() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(7),
            muh_header_size: 3,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(7), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::NonCanonicalMetadata {
                what: "metadata_type_canonical"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn short_reserved_named_metadata_type_is_rejected() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let obu = short_obu(
            MetadataType::Reserved(5),
            1,
            Some(unit(MetadataType::Reserved(5), 0, payload)),
            false,
        );
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "metadata_type_canonical"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn payload_itut_t35_unexpected_extension_is_rejected() {
        let payload = MetadataPayload::ItutT35(MetadataItutT35 {
            itu_t_t35_country_code: 0x01,
            itu_t_t35_country_code_extension_byte: Some(0x42),
            payload_len: 0,
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "itut_t35_extension"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_banding_component_fields_missing_is_rejected() {
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: Some(BandingHintsDetail {
                three_color_components_flag: false,
                components: vec![BandingComponent {
                    banding_in_component_present_flag: true,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: Some(0),
                }],
                band_units: None,
            }),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "banding_component_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_banding_component_domain_is_rejected() {
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: Some(BandingHintsDetail {
                three_color_components_flag: false,
                components: vec![BandingComponent {
                    banding_in_component_present_flag: true,
                    max_band_width_minus_4: Some(64),
                    max_band_step_minus_1: Some(0),
                }],
                band_units: None,
            }),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "banding_component_domain"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_band_units_domain_is_rejected() {
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: Some(BandingHintsDetail {
                three_color_components_flag: false,
                components: vec![BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                }],
                band_units: Some(BandUnits {
                    num_band_units_rows_minus_1: 32,
                    num_band_units_cols_minus_1: 0,
                    varying_size: None,
                    banding_in_band_unit_present: vec![],
                }),
            }),
        });
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "band_units_domain"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    /// Builds a coding-present single-component BandingHints whose `band_units` carries the given
    /// `varying_size`, with `rows*cols` present flags so only the varying check can fire.
    fn banding_hints_with_varying(
        rows_minus_1: u8,
        cols_minus_1: u8,
        varying: VaryingBandUnits,
    ) -> MetadataPayload {
        let rows = usize::from(rows_minus_1) + 1;
        let cols = usize::from(cols_minus_1) + 1;
        MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: false,
            hints: Some(BandingHintsDetail {
                three_color_components_flag: false,
                components: vec![BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                }],
                band_units: Some(BandUnits {
                    num_band_units_rows_minus_1: rows_minus_1,
                    num_band_units_cols_minus_1: cols_minus_1,
                    varying_size: Some(varying),
                    banding_in_band_unit_present: vec![false; rows * cols],
                }),
            }),
        })
    }

    #[test]
    fn payload_band_units_varying_wrong_length_is_rejected() {
        let payload = banding_hints_with_varying(
            1,
            0,
            VaryingBandUnits {
                band_block_in_luma_samples: 0,
                vert_size_in_band_blocks_minus_1: vec![0, 0, 0],
                horz_size_in_band_blocks_minus_1: vec![0],
            },
        );
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "band_units_varying"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_band_units_varying_element_domain_is_rejected() {
        let payload = banding_hints_with_varying(
            0,
            0,
            VaryingBandUnits {
                band_block_in_luma_samples: 0,
                vert_size_in_band_blocks_minus_1: vec![32],
                horz_size_in_band_blocks_minus_1: vec![0],
            },
        );
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "band_units_varying"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn payload_band_units_varying_block_domain_is_rejected() {
        let payload = banding_hints_with_varying(
            0,
            0,
            VaryingBandUnits {
                band_block_in_luma_samples: 8,
                vert_size_in_band_blocks_minus_1: vec![0],
                horz_size_in_band_blocks_minus_1: vec![0],
            },
        );
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_payload(&mut writer, &payload, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "band_units_varying"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    #[cfg(target_pointer_width = "64")]
    fn unit_oversized_declared_payload_size_is_rejected() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0,
            max_fall: 0,
        });
        let u = unit(MetadataType::HdrCll, 1usize << 33, payload);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_unit(&mut writer, &u, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "unit_payload_size"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn unit_large_payload_size_pads_without_hang() {
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0x1234,
            max_fall: 0x5678,
        });
        let u = unit(MetadataType::HdrCll, 100_000, payload);
        let mut writer = BitWriter::new();
        write_metadata_unit(&mut writer, &u, &[]).unwrap();
        assert_eq!(writer.into_bytes().len(), 100_000);
    }


    #[test]
    fn group_timecode_unit_round_trips() {
        let payload = MetadataPayload::Timecode(MetadataTimecode {
            counting_type: 0,
            full_timestamp_flag: true,
            discontinuity_flag: false,
            cnt_dropped_flag: false,
            n_frames: 7,
            seconds_value: Some(59),
            minutes_value: Some(58),
            hours_value: Some(23),
            time_offset_length: 0,
            time_offset_value: None,
        });
        let header_size = (minimal_leb_len(5) + 2) as u8;
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Timecode,
            muh_header_size: header_size,
            muh_cancel_flag: false,
            muh_payload_size: Some(5),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Timecode, 5, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        group_round_trip(&obu, ExtendedLayerId::from_bits(0), &[&[]]);
    }

    #[test]
    fn group_varying_band_units_round_trips() {
        let detail = BandingHintsDetail {
            three_color_components_flag: true,
            components: vec![
                BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                },
                BandingComponent {
                    banding_in_component_present_flag: true,
                    max_band_width_minus_4: Some(10),
                    max_band_step_minus_1: Some(3),
                },
                BandingComponent {
                    banding_in_component_present_flag: false,
                    max_band_width_minus_4: None,
                    max_band_step_minus_1: None,
                },
            ],
            band_units: Some(BandUnits {
                num_band_units_rows_minus_1: 1,
                num_band_units_cols_minus_1: 1,
                varying_size: Some(VaryingBandUnits {
                    band_block_in_luma_samples: 4,
                    vert_size_in_band_blocks_minus_1: vec![1, 2],
                    horz_size_in_band_blocks_minus_1: vec![3, 4],
                }),
                banding_in_band_unit_present: vec![true, false, false, true],
            }),
        };
        let payload = MetadataPayload::BandingHints(MetadataBandingHints {
            coding_banding_present_flag: true,
            source_banding_present_flag: true,
            hints: Some(detail),
        });
        let payload_size = 8usize; // generous; the writer pads to this size
        let header_size = (minimal_leb_len(payload_size as u32) + 2) as u8;
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::BandingHints,
            muh_header_size: header_size,
            muh_cancel_flag: false,
            muh_payload_size: Some(payload_size as u32),
            muh_layer_idc: Some(0),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: None,
            muh_mlayer_maps: vec![],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::BandingHints, payload_size, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        group_round_trip(&obu, ExtendedLayerId::from_bits(0), &[&[]]);
    }

    #[test]
    fn group_multi_bit_global_xlayer_map_round_trips() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 10,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(LAYER_VALUES),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: Some(0b1011),
            muh_mlayer_maps: vec![0x11, 0x22, 0x44],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        group_round_trip(&obu, GLOBAL_XLAYER_ID, &[&[]]);
    }

    #[test]
    fn group_global_xlayer_map_high_bit_round_trips() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let unit = MetadataGroupUnit {
            metadata_type: MetadataType::Reserved(0),
            muh_header_size: 8,
            muh_cancel_flag: false,
            muh_payload_size: Some(0),
            muh_layer_idc: Some(LAYER_VALUES),
            muh_persistence_idc: Some(0),
            muh_priority: Some(0),
            muh_reserved_zero_2bits: Some(0),
            muh_xlayer_map: Some(1 << 30),
            muh_mlayer_maps: vec![0x55],
            header_extension_len: 0,
            unit: Some(unit(MetadataType::Reserved(0), 0, payload)),
        };
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        group_round_trip(&obu, GLOBAL_XLAYER_ID, &[&[]]);
    }

    #[test]
    fn group_non_minimal_muh_payload_size_byte_exact() {
        let mut unit = hdr_cll_group_unit();
        unit.muh_header_size = 4; // payload_size_bytes = 4 - 2 - 0 - 0 = 2
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![unit],
        };
        let xlayer = ExtendedLayerId::from_bits(0);
        let mut writer = BitWriter::new();
        write_metadata_group_obu(&mut writer, &obu, xlayer, &[&[]]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(
            bytes,
            vec![0x00, 0x00, 0x01, 0x08, 0x84, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78]
        );
        assert_eq!(&bytes[4..6], &[0x84, 0x00]);
        let reparsed = reparse_group(&bytes, xlayer).unwrap();
        assert_eq!(&reparsed, &obu);
    }


    #[test]
    fn short_unaligned_writer_is_rejected() {
        let obu = short_obu(MetadataType::Timecode, 1, None, true);
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        assert!(matches!(
            write_metadata_short_obu(&mut writer, &obu, &[]),
            Err(WriteError::WriterNotByteAligned)
        ));
        assert_eq!(writer.bit_len(), 1);
    }

    #[test]
    fn group_unaligned_writer_is_rejected() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![cancel_group_unit(MetadataType::Timecode, 0)],
        };
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[]]),
            Err(WriteError::WriterNotByteAligned)
        ));
        assert_eq!(writer.bit_len(), 1);
    }

    #[test]
    fn group_cancel_with_passthrough_is_rejected() {
        let obu = MetadataGroupObu {
            metadata_is_suffix: false,
            metadata_necessity_idc: 0,
            metadata_application_id: 0,
            units: vec![cancel_group_unit(MetadataType::Timecode, 0)],
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_group_obu(&mut writer, &obu, ExtendedLayerId::from_bits(0), &[&[0x00]]),
            Err(WriteError::NonCanonicalMetadata {
                what: "passthrough_len"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn unit_reserved_named_type_is_rejected() {
        let payload = MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 });
        let u = unit(MetadataType::Reserved(5), 0, payload);
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_metadata_unit(&mut writer, &u, &[]),
            Err(WriteError::NonCanonicalMetadata {
                what: "metadata_type_canonical"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }
}
