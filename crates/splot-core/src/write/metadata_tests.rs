// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit / reject tests for the §5.17 metadata-OBU writers. Round-trips construct a model (often by
// parsing a hand-built byte vector and slicing out the passthrough region), re-emit it via the
// writer, and assert the reparse equals the original model (and byte-exactness for canonical
// inputs). Reject tests assert the typed error and that no bit was written (`bit_len() == 0`).

// `include!`d into `crate::write::metadata` so `super::*` resolves to its writers and helpers.

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

    // ===== short OBU: per-payload round-trips =====

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
        // payload_size = 1 (country code) + 3 (payload).
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
        // payload_size = 1 (country code) + 1 (extension) + 2 (payload).
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
        // 5+1+1+1+9 + 6+6+5 + 5 = 39 bits -> 5 bytes.
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
        // leb128(300) is 2 bytes; pad to a 3-byte unit so padding is exercised.
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
        // 1 byte flags + 3*16 hash bytes = 49 bytes.
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
        // The exact unit size is whatever fits the bits; use a generous size (the writer pads).
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
        // is_suffix=0, layer_idc=0, cancel=0, persistence=0 -> 0x00; type=HdrCll leb128 = 0x01;
        // unit = 0x12 0x34 0x56 0x78 (max_cll/max_fall).
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
        // metadata_type = 1 (HdrCll) but coded in 2 leb128 bytes (0x81 0x00): non-minimal.
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0,
            max_fall: 0,
        });
        let obu = short_obu(MetadataType::HdrCll, 2, Some(unit(MetadataType::HdrCll, 4, payload)), false);
        let mut writer = BitWriter::new();
        write_metadata_short_obu(&mut writer, &obu, &[]).unwrap();
        let bytes = writer.into_bytes();
        // 0x00 header, 0x81 0x00 (non-minimal leb128 of 1), then the 4 unit bytes.
        assert_eq!(bytes, vec![0x00, 0x81, 0x00, 0x00, 0x00, 0x00, 0x00]);
        let reparsed = reparse_short(&bytes).unwrap();
        assert_eq!(reparsed, obu);
    }

    // ===== short OBU: reject paths =====

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
        // metadata_type = 200 needs 2 leb128 bytes; stored leb_bytes = 1 cannot encode it.
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
        // OBU metadata_type HdrCll but the unit carries a ScanType payload.
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

    // ===== metadata_unit reject paths =====

    #[test]
    fn unit_payload_overflows_size_is_rejected() {
        // HdrCll needs 4 bytes but the declared payload_size is 2.
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
        // ItutT35 payload_size must equal 1 + ext + payload_len = 1 + 0 + 3 = 4, but declared 5.
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

    // ===== payload reject paths =====

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
        // country code 0xFF but no extension byte modeled.
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
        // full_timestamp_flag but hours missing.
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
        // per_plane but a frame_hash is also present.
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
        // coding_banding_present_flag = false but hints present.
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
        // three_color_components_flag implies 3 components but only 1 modeled.
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
        // 2 rows * 1 col = 2 band units but only 1 present flag modeled.
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

    // ===== group OBU round-trips =====

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
        // group header 0x00, cnt_minus_1 0x00, type 0x01, header byte (3<<1)=0x06, payload_size
        // 0x04, fixed 0x00 0x00, hdr_cll 0x12 0x34 0x56 0x78.
        assert_eq!(
            writer.into_bytes(),
            vec![0x00, 0x00, 0x01, 0x06, 0x04, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78]
        );
    }

    #[test]
    fn group_local_mlayer_map_round_trips() {
        // layer_idc = LAYER_VALUES on a local OBU -> a single mlayer map byte. header_size =
        // payload_size leb (1) + fixed 2 + 1 mlayer = 4. type=Reserved(0) -> UnknownRaw, size 0.
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
        // Global OBU, layer_idc = LAYER_VALUES -> xlayer_map f(32) + one mlayer per set bit.
        // header_size = payload_size leb (1) + 2 + 4 (xlayer) + 1 (one set bit) = 8.
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
        // header_size = 4 -> 1 extension byte after the fixed header (payload_size leb (1) +
        // fixed 2 + 1 extension), no layer maps.
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

    // ===== group OBU reject paths =====

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
        // header_extension_len must equal muh_header_size on cancel.
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
        // header_size too small to cover payload_size leb (1) + fixed 2.
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
        // payload_size = 200 needs 2 leb bytes, but header_size only budgets 1 for it (3 = 1 + 2).
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
        // local LAYER_VALUES requires exactly 1 mlayer map; supply 2.
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
        // unit.payload_size disagrees with muh_payload_size.
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
        // value 1 in 1..=5 byte encodings reparses to 1 and consumes exactly `len` bytes.
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
        // value 200 needs 2 groups; len 1 is too small.
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_leb128_with_len(&mut writer, 200, 1, "test"),
            Err(WriteError::NonCanonicalMetadata { what: "test" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    // ===== group OBU: additional reject paths (adversarial coverage) =====

    #[test]
    fn group_noncancel_missing_field_is_rejected() {
        // A non-cancel unit with one required muh_* field absent is rejected before any write.
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
        // muh_header_size is f(7); 200 overflows the field and is rejected up front.
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
        // Global OBU, LAYER_VALUES, but no muh_xlayer_map: the global branch rejects the missing
        // map (layer_map_presence) before the header-size arithmetic runs.
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
        // Reserved(7) re-maps to IccProfile on reparse, so the group unit type is non-canonical.
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

    // ===== short OBU: additional reject paths (adversarial coverage) =====

    #[test]
    fn short_reserved_named_metadata_type_is_rejected() {
        // Reserved(5) re-maps to DecodedFrameHash on reparse, so it could never have been parsed.
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

    // ===== payload: additional reject paths (adversarial coverage) =====

    #[test]
    fn payload_itut_t35_unexpected_extension_is_rejected() {
        // The other direction of itut_t35_extension: a non-0xFF country code with an extension
        // byte modeled.
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
        // banding_in_component_present_flag set but the gated fields are absent.
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
        // max_band_width_minus_4 = 64 is out of the f(6) domain (0..=63).
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
        // num_band_units_rows_minus_1 = 32 is out of the f(5) domain (0..=31).
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
        // vert vector length (3) disagrees with rows (2).
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
        // A varying element of 32 is out of the f(5) domain (0..=31).
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
        // band_block_in_luma_samples = 8 is out of the f(3) domain (0..=7).
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

    // ===== metadata_unit: over-large declared size cap (no hang) =====

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn unit_oversized_declared_payload_size_is_rejected() {
        // A declared payload_size beyond u32::MAX could never have been parsed; it must be
        // rejected before the pad loop (proving no unbounded padding is attempted).
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
        // A 100_000-byte declared size with a 4-byte typed payload pads to exactly the declared
        // size; the padding is bounded by the u32 payload_size cap, so a constructed model can
        // never drive it unbounded.
        let payload = MetadataPayload::HdrCll(MetadataHdrCll {
            max_cll: 0x1234,
            max_fall: 0x5678,
        });
        let u = unit(MetadataType::HdrCll, 100_000, payload);
        let mut writer = BitWriter::new();
        write_metadata_unit(&mut writer, &u, &[]).unwrap();
        assert_eq!(writer.into_bytes().len(), 100_000);
    }

    // ===== group OBU: additional round-trips (adversarial coverage) =====

    #[test]
    fn group_timecode_unit_round_trips() {
        // A non-cancel group unit carrying a full-timestamp Timecode (39 bits = 5 bytes).
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
        // Reuse the varying-size BandingHints detail from the short round-trip test inside a
        // non-cancel group unit. payload_size is over-declared; the writer pads to it.
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
        // Global OBU, LAYER_VALUES, xlayer_map with bits 0, 1, 3 set -> 3 mlayer map bytes.
        // header_size = payload_size leb (1) + fixed 2 + xlayer (4) + 3 mlayer = 10.
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
        // Bit 30 is the highest bit the writer iterates (0..31), so it still yields one mlayer
        // byte; bit 31 would be ignored. header_size = 1 + 2 + 4 (xlayer) + 1 (one set bit) = 8.
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
        // muh_header_size = 4 with no layer maps / extension forces payload_size_bytes = 2, a
        // NON-minimal 2-byte leb for the value 4 (0x84 0x00).
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
        // group header 0x00, cnt_minus_1 0x00, type 0x01, header byte (4<<1)=0x08, then the
        // non-minimal 2-byte muh_payload_size leb 0x84 0x00, fixed 0x00 0x00, hdr_cll bytes.
        assert_eq!(
            bytes,
            vec![0x00, 0x00, 0x01, 0x08, 0x84, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78]
        );
        // The muh_payload_size leb sits at indices 4..6.
        assert_eq!(&bytes[4..6], &[0x84, 0x00]);
        let reparsed = reparse_group(&bytes, xlayer).unwrap();
        assert_eq!(&reparsed, &obu);
    }

    // ===== review-driven reject guards =====

    #[test]
    fn short_unaligned_writer_is_rejected() {
        // The metadata OBU payload must start byte-aligned; a mid-byte writer is rejected and the
        // writer is left untouched (still holding only the pre-existing partial bit).
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
        // A cancelled group unit carries no metadata_unit, so supplied opaque bytes are rejected
        // rather than silently dropped (matching the short OBU's cancel arm).
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
        // A direct write_metadata_unit caller must also be guarded: Reserved(5) re-maps to a named
        // type on reparse, so it could never have been parsed.
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
