// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Property tests for the §5.17 metadata-OBU writers. The round-trip strategy constructs arbitrary
// valid (model, passthrough) pairs across all payload types and both OBU forms, writes them, and
// reparses to assert semantic equality; a second family feeds arbitrary (possibly invalid)
// constructed models to all four public writers and asserts they never panic and leave the writer
// untouched on Err.

// `include!`d into `crate::write::metadata` so `super::*` resolves to its writers and helpers.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod proptests {
    use super::*;
    use crate::headers::metadata::{
        MetadataHdrCll, MetadataIccProfile, MetadataTemporalPointInfo, MetadataUnknownRaw,
        MetadataUserDataUnregistered,
    };
    use crate::types::GLOBAL_XLAYER_ID;
    use proptest::prelude::*;

    /// An arbitrary valid (payload, passthrough, payload_size) triple across all 11 variants.
    fn arbitrary_payload() -> impl Strategy<Value = (MetadataPayload, Vec<u8>, usize)> {
        prop_oneof![
            // HdrCll: 4 bytes.
            (any::<u16>(), any::<u16>()).prop_map(|(max_cll, max_fall)| {
                (
                    MetadataPayload::HdrCll(MetadataHdrCll { max_cll, max_fall }),
                    Vec::new(),
                    4usize,
                )
            }),
            // HdrMdcv: 24 bytes.
            (
                any::<[u16; 3]>(),
                any::<[u16; 3]>(),
                any::<u16>(),
                any::<u16>(),
                any::<u32>(),
                any::<u32>(),
            )
                .prop_map(|(px, py, wx, wy, lmax, lmin)| {
                    (
                        MetadataPayload::HdrMdcv(MetadataHdrMdcv {
                            primary_chromaticity_x: px,
                            primary_chromaticity_y: py,
                            white_point_chromaticity_x: wx,
                            white_point_chromaticity_y: wy,
                            luminance_max: lmax,
                            luminance_min: lmin,
                        }),
                        Vec::new(),
                        24usize,
                    )
                }),
            // ItutT35: country code, optional extension, payload bytes.
            (
                any::<bool>(),
                proptest::collection::vec(any::<u8>(), 0..8),
            )
                .prop_map(|(ext, bytes)| {
                    let (country, extension, ext_len) = if ext {
                        (0xFFu8, Some(0x42u8), 1usize)
                    } else {
                        (0x01u8, None, 0usize)
                    };
                    let payload_len = bytes.len();
                    (
                        MetadataPayload::ItutT35(MetadataItutT35 {
                            itu_t_t35_country_code: country,
                            itu_t_t35_country_code_extension_byte: extension,
                            payload_len,
                        }),
                        bytes,
                        1 + ext_len + payload_len,
                    )
                }),
            // Timecode: a valid full-timestamp form (simple, in-domain).
            (0u8..32, any::<bool>(), any::<bool>(), 0u16..512, 0u8..64, 0u8..64, 0u8..32).prop_map(
                |(counting_type, disc, dropped, n_frames, s, m, h)| {
                    (
                        MetadataPayload::Timecode(MetadataTimecode {
                            counting_type,
                            full_timestamp_flag: true,
                            discontinuity_flag: disc,
                            cnt_dropped_flag: dropped,
                            n_frames,
                            seconds_value: Some(s),
                            minutes_value: Some(m),
                            hours_value: Some(h),
                            time_offset_length: 0,
                            time_offset_value: None,
                        }),
                        Vec::new(),
                        5usize, // 5+1+1+1+9 + 6+6+5 + 5 = 39 bits -> 5 bytes
                    )
                }
            ),
            // ScanType: 1 byte.
            (0u8..32, 0u8..4, any::<bool>()).prop_map(|(pic, idc, dup)| {
                (
                    MetadataPayload::ScanType(MetadataScanType {
                        mps_pic_struct_type: pic,
                        mps_source_scan_type_idc: idc,
                        mps_duplicate_flag: dup,
                    }),
                    Vec::new(),
                    1usize,
                )
            }),
            // TemporalPointInfo: leb128, padded to enough bytes.
            any::<u32>().prop_map(|t| {
                (
                    MetadataPayload::TemporalPointInfo(MetadataTemporalPointInfo {
                        frame_presentation_time: t,
                    }),
                    Vec::new(),
                    minimal_leb_len(t),
                )
            }),
            // DecodedFrameHash single: 1 + 16 = 17 bytes.
            (0u8..16, any::<bool>(), 0u8..2, any::<[u8; 16]>()).prop_map(
                |(hash_type, has_grain, reserved, hash)| {
                    (
                        MetadataPayload::DecodedFrameHash(MetadataDecodedFrameHash {
                            hash_type,
                            per_plane: false,
                            has_grain,
                            is_monochrome: false,
                            reserved,
                            plane_hashes: vec![],
                            frame_hash: Some(hash),
                        }),
                        Vec::new(),
                        17usize,
                    )
                }
            ),
            // BandingHints: a simple no-detail form.
            any::<bool>().prop_map(|source| {
                (
                    MetadataPayload::BandingHints(MetadataBandingHints {
                        coding_banding_present_flag: false,
                        source_banding_present_flag: source,
                        hints: None,
                    }),
                    Vec::new(),
                    1usize,
                )
            }),
            // IccProfile: passthrough only.
            proptest::collection::vec(any::<u8>(), 0..16).prop_map(|bytes| {
                let len = bytes.len();
                (
                    MetadataPayload::IccProfile(MetadataIccProfile { payload_len: len }),
                    bytes,
                    len,
                )
            }),
            // UserDataUnregistered: uuid + passthrough.
            (any::<[u8; 16]>(), proptest::collection::vec(any::<u8>(), 0..16)).prop_map(
                |(uuid, bytes)| {
                    let len = bytes.len();
                    (
                        MetadataPayload::UserDataUnregistered(MetadataUserDataUnregistered {
                            uuid_iso_iec_11578: uuid,
                            payload_len: len,
                        }),
                        bytes,
                        16 + len,
                    )
                }
            ),
            // UnknownRaw: passthrough only.
            proptest::collection::vec(any::<u8>(), 0..16).prop_map(|bytes| {
                let len = bytes.len();
                (
                    MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: len }),
                    bytes,
                    len,
                )
            }),
        ]
    }

    /// Returns the `metadata_type` that selects `payload`'s variant.
    fn type_of(payload: &MetadataPayload) -> MetadataType {
        match payload {
            MetadataPayload::HdrCll(_) => MetadataType::HdrCll,
            MetadataPayload::HdrMdcv(_) => MetadataType::HdrMdcv,
            MetadataPayload::ItutT35(_) => MetadataType::ItutT35,
            MetadataPayload::Timecode(_) => MetadataType::Timecode,
            MetadataPayload::DecodedFrameHash(_) => MetadataType::DecodedFrameHash,
            MetadataPayload::BandingHints(_) => MetadataType::BandingHints,
            MetadataPayload::IccProfile(_) => MetadataType::IccProfile,
            MetadataPayload::ScanType(_) => MetadataType::ScanType,
            MetadataPayload::TemporalPointInfo(_) => MetadataType::TemporalPointInfo,
            MetadataPayload::UserDataUnregistered(_) => MetadataType::UserDataUnregistered,
            MetadataPayload::UnknownRaw(_) => MetadataType::Reserved(0),
        }
    }

    proptest! {
        /// Every constructed (payload, passthrough) pair round-trips through the short OBU.
        #[test]
        fn short_obu_round_trips((payload, passthrough, payload_size) in arbitrary_payload()) {
            let metadata_type = type_of(&payload);
            let obu = MetadataShortObu {
                metadata_is_suffix: false,
                muh_layer_idc: 0,
                muh_cancel_flag: false,
                muh_persistence_idc: 0,
                metadata_type,
                metadata_type_leb128_bytes: minimal_leb_len(metadata_type.value()) as u8,
                unit: Some(MetadataUnit {
                    metadata_type,
                    payload_size,
                    payload,
                }),
            };
            let mut writer = BitWriter::new();
            write_metadata_short_obu(&mut writer, &obu, &passthrough).unwrap();
            let bytes = writer.into_bytes();
            let reparsed = reparse_short(&bytes).unwrap();
            prop_assert_eq!(reparsed, obu);
        }

        /// Every constructed payload round-trips through a non-cancel group unit, on both the
        /// local and global layer-map branches (layer_idc 0, no maps).
        #[test]
        fn group_obu_round_trips(
            (payload, passthrough, payload_size) in arbitrary_payload(),
            global in any::<bool>(),
        ) {
            let metadata_type = type_of(&payload);
            // payload_size_bytes is the minimal leb len of payload_size; header_size = that + 2.
            let payload_size_u32 = payload_size as u32;
            let header_size = (minimal_leb_len(payload_size_u32) + 2) as u8;
            let unit = MetadataGroupUnit {
                metadata_type,
                muh_header_size: header_size,
                muh_cancel_flag: false,
                muh_payload_size: Some(payload_size_u32),
                muh_layer_idc: Some(0),
                muh_persistence_idc: Some(0),
                muh_priority: Some(0),
                muh_reserved_zero_2bits: Some(0),
                muh_xlayer_map: None,
                muh_mlayer_maps: vec![],
                header_extension_len: 0,
                unit: Some(MetadataUnit { metadata_type, payload_size, payload }),
            };
            let obu = MetadataGroupObu {
                metadata_is_suffix: false,
                metadata_necessity_idc: 0,
                metadata_application_id: 0,
                units: vec![unit],
            };
            let xlayer = if global { GLOBAL_XLAYER_ID } else { ExtendedLayerId::from_bits(0) };
            let mut writer = BitWriter::new();
            let slice: &[u8] = &passthrough;
            write_metadata_group_obu(&mut writer, &obu, xlayer, &[slice]).unwrap();
            let bytes = writer.into_bytes();
            let reparsed = reparse_group(&bytes, xlayer).unwrap();
            prop_assert_eq!(reparsed, obu);
        }

        /// The short OBU writer never panics on an arbitrary (possibly invalid) model + passthrough,
        /// and leaves the writer empty on Err.
        #[test]
        fn short_writer_never_panics(
            is_suffix in any::<bool>(),
            layer_idc in any::<u8>(),
            cancel in any::<bool>(),
            persistence in any::<u8>(),
            type_value in any::<u32>(),
            leb_bytes in any::<u8>(),
            payload_size in 0usize..64,
            passthrough in proptest::collection::vec(any::<u8>(), 0..8),
        ) {
            let metadata_type = MetadataType::from_value(type_value);
            let obu = MetadataShortObu {
                metadata_is_suffix: is_suffix,
                muh_layer_idc: layer_idc,
                muh_cancel_flag: cancel,
                muh_persistence_idc: persistence,
                metadata_type,
                metadata_type_leb128_bytes: leb_bytes,
                unit: Some(MetadataUnit {
                    metadata_type,
                    payload_size,
                    payload: MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: payload_size }),
                }),
            };
            let mut writer = BitWriter::new();
            if write_metadata_short_obu(&mut writer, &obu, &passthrough).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }

        /// The payload writer never panics on arbitrary edge field values across constructed models.
        #[test]
        fn payload_writer_never_panics(
            counting_type in any::<u8>(),
            n_frames in any::<u16>(),
            time_offset_length in any::<u8>(),
            time_offset_value in proptest::option::of(any::<u32>()),
            pic in any::<u8>(),
            idc in any::<u8>(),
            hash_type in any::<u8>(),
            reserved in any::<u8>(),
        ) {
            // A timecode with out-of-domain values must be rejected, never panic.
            let timecode = MetadataPayload::Timecode(MetadataTimecode {
                counting_type,
                full_timestamp_flag: false,
                discontinuity_flag: false,
                cnt_dropped_flag: false,
                n_frames,
                seconds_value: None,
                minutes_value: None,
                hours_value: None,
                time_offset_length,
                time_offset_value,
            });
            let mut writer = BitWriter::new();
            if write_metadata_payload(&mut writer, &timecode, &[]).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }

            let scan = MetadataPayload::ScanType(MetadataScanType {
                mps_pic_struct_type: pic,
                mps_source_scan_type_idc: idc,
                mps_duplicate_flag: false,
            });
            let mut writer = BitWriter::new();
            if write_metadata_payload(&mut writer, &scan, &[]).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }

            let hash = MetadataPayload::DecodedFrameHash(MetadataDecodedFrameHash {
                hash_type,
                per_plane: false,
                has_grain: false,
                is_monochrome: false,
                reserved,
                plane_hashes: vec![],
                frame_hash: Some([0u8; 16]),
            });
            let mut writer = BitWriter::new();
            if write_metadata_payload(&mut writer, &hash, &[]).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }

        /// The unit and group writers never panic on arbitrary constructed group units.
        #[test]
        fn group_writer_never_panics(
            header_size in any::<u8>(),
            cancel in any::<bool>(),
            // Bounded: the test allocates `vec![0u8; payload_size]` below, so an unbounded u32
            // could OOM the runner. The u32-ceiling reject is covered directly by
            // `unit_oversized_declared_payload_size_is_rejected`.
            payload_size in proptest::option::of(0u32..256),
            layer_idc in proptest::option::of(any::<u8>()),
            global in any::<bool>(),
            ext_len in 0usize..8,
        ) {
            let metadata_type = MetadataType::Reserved(0);
            let unit = MetadataGroupUnit {
                metadata_type,
                muh_header_size: header_size,
                muh_cancel_flag: cancel,
                muh_payload_size: payload_size,
                muh_layer_idc: layer_idc,
                muh_persistence_idc: layer_idc,
                muh_priority: Some(0),
                muh_reserved_zero_2bits: Some(0),
                muh_xlayer_map: None,
                muh_mlayer_maps: vec![],
                header_extension_len: ext_len,
                unit: Some(MetadataUnit {
                    metadata_type,
                    payload_size: payload_size.unwrap_or(0) as usize,
                    payload: MetadataPayload::UnknownRaw(MetadataUnknownRaw {
                        raw_len: payload_size.unwrap_or(0) as usize,
                    }),
                }),
            };
            let obu = MetadataGroupObu {
                metadata_is_suffix: false,
                metadata_necessity_idc: 0,
                metadata_application_id: 0,
                units: vec![unit],
            };
            let xlayer = if global { GLOBAL_XLAYER_ID } else { ExtendedLayerId::from_bits(0) };
            let mut writer = BitWriter::new();
            let raw = vec![0u8; payload_size.unwrap_or(0) as usize];
            let slice: &[u8] = &raw;
            if write_metadata_group_obu(&mut writer, &obu, xlayer, &[slice]).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }

        /// A global-scope LAYER_VALUES group unit round-trips for an arbitrary muh_xlayer_map with
        /// one arbitrary mlayer byte per set bit in 0..31 — exercising the multi-bit map ordering
        /// and count logic that `group_obu_round_trips` (layer_idc fixed to 0) never reaches.
        #[test]
        fn group_layer_values_round_trips(
            xlayer_map in any::<u32>(),
            mlayer_seed in proptest::collection::vec(any::<u8>(), 31),
        ) {
            // One mlayer byte per set bit in 0..31 (the writer's iteration range), in bit order.
            let mlayer_maps: Vec<u8> = (0..31u32)
                .filter(|n| xlayer_map & (1 << n) != 0)
                .map(|n| mlayer_seed[n as usize])
                .collect();
            let set_bits = mlayer_maps.len();
            // header_size = payload_size leb (1) + fixed 2 + xlayer (4) + one byte per set bit.
            let header = 1 + 2 + 4 + set_bits;
            prop_assume!(header <= 127);
            let header_size = header as u8;
            let unit = MetadataGroupUnit {
                metadata_type: MetadataType::Reserved(0),
                muh_header_size: header_size,
                muh_cancel_flag: false,
                muh_payload_size: Some(0),
                muh_layer_idc: Some(LAYER_VALUES),
                muh_persistence_idc: Some(0),
                muh_priority: Some(0),
                muh_reserved_zero_2bits: Some(0),
                muh_xlayer_map: Some(xlayer_map),
                muh_mlayer_maps: mlayer_maps,
                header_extension_len: 0,
                unit: Some(MetadataUnit {
                    metadata_type: MetadataType::Reserved(0),
                    payload_size: 0,
                    payload: MetadataPayload::UnknownRaw(MetadataUnknownRaw { raw_len: 0 }),
                }),
            };
            let obu = MetadataGroupObu {
                metadata_is_suffix: false,
                metadata_necessity_idc: 0,
                metadata_application_id: 0,
                units: vec![unit],
            };
            let mut writer = BitWriter::new();
            write_metadata_group_obu(&mut writer, &obu, GLOBAL_XLAYER_ID, &[&[]]).unwrap();
            let bytes = writer.into_bytes();
            let reparsed = reparse_group(&bytes, GLOBAL_XLAYER_ID).unwrap();
            prop_assert_eq!(reparsed, obu);
        }

        /// The banding-hints writer never panics on arbitrary detail / band-unit / varying-size
        /// edge values, and leaves the writer empty on Err. This fuzzes the deepest recursion
        /// (BandingHintsDetail -> BandUnits -> VaryingBandUnits) with edge field values.
        #[test]
        fn banding_hints_writer_never_panics(
            coding in any::<bool>(),
            source in any::<bool>(),
            has_hints in any::<bool>(),
            three_color in any::<bool>(),
            present_flags in proptest::collection::vec(any::<bool>(), 0..5),
            width in proptest::option::of(any::<u8>()),
            step in proptest::option::of(any::<u8>()),
            has_band_units in any::<bool>(),
            rows_minus_1 in any::<u8>(),
            cols_minus_1 in any::<u8>(),
            has_varying in any::<bool>(),
            block in any::<u8>(),
            vert in proptest::collection::vec(any::<u8>(), 0..4),
            horz in proptest::collection::vec(any::<u8>(), 0..4),
            present_count in 0usize..6,
        ) {
            let components: Vec<BandingComponent> = present_flags
                .iter()
                .map(|&present| BandingComponent {
                    banding_in_component_present_flag: present,
                    max_band_width_minus_4: width,
                    max_band_step_minus_1: step,
                })
                .collect();
            let varying = has_varying.then(|| VaryingBandUnits {
                band_block_in_luma_samples: block,
                vert_size_in_band_blocks_minus_1: vert.clone(),
                horz_size_in_band_blocks_minus_1: horz.clone(),
            });
            let band_units = has_band_units.then(|| BandUnits {
                num_band_units_rows_minus_1: rows_minus_1,
                num_band_units_cols_minus_1: cols_minus_1,
                varying_size: varying,
                banding_in_band_unit_present: vec![false; present_count],
            });
            let hints = has_hints.then_some(BandingHintsDetail {
                three_color_components_flag: three_color,
                components,
                band_units,
            });
            let payload = MetadataPayload::BandingHints(MetadataBandingHints {
                coding_banding_present_flag: coding,
                source_banding_present_flag: source,
                hints,
            });
            let mut writer = BitWriter::new();
            if write_metadata_payload(&mut writer, &payload, &[]).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }
    }
}
