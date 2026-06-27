// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Per-OBU-type dispatch round-trip + passthrough-reject tests, split out of dispatch_tests.rs to
// keep both files under the advisory source-line limit. `include!`d INSIDE that file's `mod tests`,
// so `super::*`, the `Bits` helper, `header_for`, and `reparse_payload` all resolve to the parent.

    // ===================================================================================
    // Round-trips per written type (write_obu_payload)
    // ===================================================================================

    #[test]
    fn temporal_delimiter_payload_round_trips() {
        let header = header_for(ObuType::TemporalDelimiter);
        let payload = ParsedObu::TemporalDelimiter;
        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, header.obu_type.is_extensible_obu(), &[]).unwrap();
        let bytes = writer.into_bytes();
        // §5.5: a temporal delimiter has an empty payload and no tail.
        assert!(bytes.is_empty(), "temporal delimiter writes no payload bytes");
        let reparsed = reparse_payload(header, &bytes);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn sequence_header_payload_round_trips() {
        let header = header_for(ObuType::SequenceHeader);
        assert!(header.obu_type.is_extensible_obu());
        let payload = ParsedObu::SequenceHeader(Box::new(still_picture_sequence_header()));
        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, true, &[]).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_payload(header, &bytes);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn padding_payload_round_trips() {
        // §5.16: two arbitrary obu_padding_byte values (passthrough), then a trailing-bits byte.
        let passthrough = [0xDE, 0xADu8];
        let padding = PaddingObu {
            padding_len: 2,
            trailing_len: 1,
        };
        let payload = ParsedObu::Padding(padding);
        let header = header_for(ObuType::Padding);
        // Padding is non-extensible: it owns its own tail.
        assert!(!header.obu_type.is_extensible_obu());

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &passthrough).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(bytes, vec![0xDE, 0xAD, 0x80], "padding bytes + trailing_bits byte");
        let reparsed = reparse_payload(header, &bytes);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn padding_obu_payload_size_zero_and_one_edges() {
        let header = header_for(ObuType::Padding);
        // obuPayloadSize == 0: no padding bytes, no trailing bits.
        let empty = ParsedObu::Padding(PaddingObu {
            padding_len: 0,
            trailing_len: 0,
        });
        let mut w0 = BitWriter::new();
        write_obu_payload(&mut w0, &empty, false, &[]).unwrap();
        let b0 = w0.into_bytes();
        assert!(b0.is_empty(), "obuPayloadSize 0 writes nothing");
        assert_eq!(reparse_payload(header, &b0), empty);

        // obuPayloadSize == 1: no padding bytes, one trailing_bits() byte.
        let one = ParsedObu::Padding(PaddingObu {
            padding_len: 0,
            trailing_len: 1,
        });
        let mut w1 = BitWriter::new();
        write_obu_payload(&mut w1, &one, false, &[]).unwrap();
        let b1 = w1.into_bytes();
        assert_eq!(b1, vec![0x80], "obuPayloadSize 1 writes one trailing byte");
        assert_eq!(reparse_payload(header, &b1), one);
    }

    #[test]
    fn metadata_short_payload_round_trips() {
        // Build a cancelled short-metadata model directly (no passthrough), mirroring the
        // dispatch parser test in obu.rs.
        let obu = MetadataShortObu {
            metadata_is_suffix: false,
            muh_layer_idc: 0,
            muh_cancel_flag: true,
            muh_persistence_idc: 0,
            metadata_type: MetadataType::from_value(4),
            metadata_type_leb128_bytes: 1,
            unit: None,
        };
        // §5.2.1: metadata is non-extensible, so the tail is trailing_bits() only.
        let header = header_for(ObuType::MetadataShort);
        assert!(!header.obu_type.is_extensible_obu());
        let payload = ParsedObu::MetadataShort(Box::new(obu));

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_payload(header, &bytes);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn metadata_group_single_unit_payload_round_trips() {
        // Parse the obu.rs dispatch group fixture (one cancelled unit), slicing off the OBU
        // trailing byte the parser would have consumed, so the model is the writer's input.
        let header = header_for(ObuType::MetadataGroup);
        // Group with one cancelled unit then a trailing byte: [0x00, 0x00, 0x04, 0x01, 0x80].
        let fixture = [0x00u8, 0x00, 0x04, 0x01, 0x80];
        let parsed = reparse_payload(header, &fixture);
        let payload = parsed.clone();

        let mut writer = BitWriter::new();
        // Single-unit, non-global xlayer (header xlayer is 0 here) — the write_obu_payload path.
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_payload(header, &bytes);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn metadata_group_multi_unit_payload_round_trips() {
        // A two-unit group (both cancelled, so each needs an empty passthrough). The dispatch
        // splits the flat (empty) passthrough into one empty slice per unit, so a valid multi-unit
        // group round-trips through write_obu_payload rather than being rejected on unit count.
        let header = header_for(ObuType::MetadataGroup);
        // group header 0x00, cnt_minus_1 0x01 (2 units), unit [type 0x04, cancel 0x01] x2, tail 0x80.
        let fixture = [0x00u8, 0x01, 0x04, 0x01, 0x04, 0x01, 0x80];
        let payload = reparse_payload(header, &fixture);
        // Sanity: the fixture really is a two-unit group (else the test would not exercise the fix).
        match &payload {
            ParsedObu::MetadataGroup(obu) => assert_eq!(obu.units.len(), 2),
            other => panic!("expected a metadata group, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_payload(header, &bytes);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn buffer_removal_timing_payload_round_trips() {
        // §5.12 is not extensible; the dispatch routes it to write_buffer_removal_timing + the
        // trailing-bits tail (no longer Unimplemented). OPS-dependent form, one present + one absent.
        let header = header_for(ObuType::BufferRemovalTiming);
        let payload = ParsedObu::BufferRemovalTiming(BufferRemovalTiming::OperatingPointSet {
            br_ops_id: 3,
            br_ops_cnt: 2,
            op_times: vec![
                BufferRemovalOpTiming {
                    index: 0,
                    decoder_model_present: true,
                    br_time_op: Some(7),
                },
                BufferRemovalOpTiming {
                    index: 1,
                    decoder_model_present: false,
                    br_time_op: None,
                },
            ],
        });
        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);
    }

    #[test]
    fn msdo_payload_round_trips() {
        // §5.6 MSDO routes to write_msdo + the non-extensible tail (no longer Unimplemented). Even
        // allocation -> no large_picture_idc; two sub-streams (num_streams_minus_2 = 0).
        let header = header_for(ObuType::Msdo);
        let mut sub_streams = [SubStreamConfig {
            sub_xlayer_id: 0,
            sub_stream_max_profile: 0,
            sub_stream_max_level: 0,
            sub_stream_max_tier: 0,
        }; 9];
        sub_streams[0] = SubStreamConfig {
            sub_xlayer_id: 1,
            sub_stream_max_profile: 4,
            sub_stream_max_level: 3,
            sub_stream_max_tier: 0,
        };
        sub_streams[1] = SubStreamConfig {
            sub_xlayer_id: 2,
            sub_stream_max_profile: 3,
            sub_stream_max_level: 4,
            sub_stream_max_tier: 1,
        };
        let payload = ParsedObu::Msdo(MultistreamDecoderOperation {
            num_streams_minus_2: 0,
            multistream_profile_idc: ProfileIdc::from_bits(5),
            multistream_level_idx: 10,
            multistream_tier: 1,
            multistream_even_allocation_flag: true,
            multistream_large_picture_idc: None,
            sub_stream_count: 2,
            sub_streams,
            multistream_doh_constraint_flag: false,
        });
        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);
    }

    #[test]
    fn operating_point_set_payload_round_trips() {
        // §5.10/§5.11 OPS is extensible; the dispatch routes it to write_operating_point_set + the
        // extensible tail (obu_extension_flag = 0 + trailing_bits()). The OBU's obu_xlayer_id (global
        // vs local) selects the OPS syntax branch, so a global OPS must go through write_complete_obu
        // (which threads header.extended_layer_id == GLOBAL_XLAYER_ID), not write_obu_payload.
        //
        // Build a global reset OPS fixture by parsing a minimal OPS payload via dispatch (reset-only
        // OPS + the extensible tail), then write it back through write_complete_obu and reparse.
        let header = read_obu_header_from_slice(&[0xC8, 0x1F], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::OperatingPointSet);
        assert_eq!(header.extended_layer_id, GLOBAL_XLAYER_ID);
        assert!(header.obu_type.is_extensible_obu());
        let payload = reparse_payload(header, &[0x00, 0x40]);
        match &payload {
            ParsedObu::OperatingPointSet(ops) => {
                assert_eq!(ops.ops_cnt, 0, "fixture is a reset OPS");
                assert!(ops.is_global());
            }
            other => panic!("expected an operating point set, got {other:?}"),
        }

        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let bytes = complete.into_bytes();
        // bytes = header (2) ++ payload; reparse the payload region.
        let reparsed = reparse_payload(header, &bytes[2..]);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn operating_point_set_local_payload_round_trips() {
        // A local-xlayer OPS round-trips through write_obu_payload (header xlayer defaults to 0,
        // the local branch). Build a local active OPS (ops_cnt = 1, one explicit empty-map layer)
        // by parsing a hand-built payload, then write it back and reparse.
        let header = header_for(ObuType::OperatingPointSet);
        assert!(!header.extended_layer_id.is_global());
        // Local OPS body: reset=0 id=0 cnt=1 | priority(4)=0 intent(7)=0 | intent/ptl/color=0 |
        // reserved(2)=0 | ops_data_size leb128 | payload: dm=0 idd=0 mlayer_map(8)=0 | align | tail.
        let mut bits = Bits::default();
        bits.bit(0); // ops_reset_flag
        bits.f(0, 4); // ops_id
        bits.f(1, 3); // ops_cnt = 1
        bits.f(0, 4); // ops_priority
        bits.f(0, 7); // ops_intent
        bits.bit(0); // ops_intent_present_flag
        bits.bit(0); // ops_ptl_present_flag
        bits.bit(0); // ops_color_info_present_flag
        bits.f(0, 2); // ops_reserved_2bits (local)
        // payload body: dm=0, idd=0, ops_mlayer_map(8)=0 -> 1 byte + 2 bits aligns to 2 bytes.
        let mut body = Bits::default();
        body.bit(0); // decoder_model_present
        body.bit(0); // initial_display_delay_present
        body.f(0, 8); // ops_mlayer_map = 0
        // align body to a byte for opsBytes.
        while body.bits.len() % 8 != 0 {
            body.bit(0);
        }
        let ops_bytes = (body.bits.len() / 8) as u32;
        bits.f(ops_bytes, 8); // ops_data_size (single-byte leb128)
        bits.bits.extend_from_slice(&body.bits);
        // extensible OBU tail: obu_extension_flag = 0, then trailing_bits().
        bits.bit(0);
        bits.bit(1); // trailing_one_bit
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload_bytes = bits.into_bytes();
        let payload = reparse_payload(header, &payload_bytes);
        match &payload {
            ParsedObu::OperatingPointSet(ops) => {
                assert_eq!(ops.ops_cnt, 1);
                assert!(!ops.is_global());
            }
            other => panic!("expected an operating point set, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, true, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);
    }

    #[test]
    fn content_interpretation_payload_round_trips() {
        // §5.15 content interpretation is extensible; the dispatch routes it to
        // write_content_interpretation + the extensible tail (obu_extension_flag = 0 +
        // trailing_bits()). It carries no passthrough. Build a model with two optional structures
        // present (a preset color description and an indexed aspect ratio) to exercise the body.
        let header = header_for(ObuType::ContentInterpretation);
        assert!(header.obu_type.is_extensible_obu());
        let payload = ParsedObu::ContentInterpretation(ContentInterpretation {
            scan_type_idc: ScanTypeIdc::from_bits(1),
            color_description: Some(ColorDescription {
                color_description_idc: 1,
                primaries: None,
                full_range_flag: true,
            }),
            chroma_sample_position: None,
            aspect_ratio: Some(AspectRatioInfo {
                aspect_ratio_idc: 1,
                extended_sar: None,
            }),
            timing_info: None,
            reserved_2bit: 0,
        });
        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, true, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);

        // And it round-trips through write_complete_obu (header + payload + tail).
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        assert_eq!(reparse_payload(header, &complete_bytes[1..]), payload);
    }

    #[test]
    fn film_grain_payload_round_trips() {
        // §5.14 / §5.18.10.2 film grain is non-extensible; the dispatch routes it to
        // write_film_grain + the trailing-bits tail (no longer Unimplemented). Build a one-slot
        // model with luma scaling points and AR coeffs by hand, parse it via dispatch, then write it
        // back and reparse. The model is lossy versus the wire (bit-widths re-derived), so model
        // equality — not byte-exactness — is asserted.
        let header = header_for(ObuType::FilmGrain);
        assert!(!header.obu_type.is_extensible_obu());
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // fgm_update_flags: slot 0
        bits.uvlc(0); // fgm_chroma_idc = CHROMA_FORMAT_420
        bits.bit(0); // chroma_scaling_from_luma = 0
        bits.f(2, 4); // num_y_points = 2
        bits.f(0, 3); // bitsIncr = 1
        bits.f(0, 2); // bitsScal = 5
        bits.f(1, 1); // point_y_value[0] = 1
        bits.f(3, 5); // point_y_scaling[0] = 3
        bits.f(1, 1); // increment -> value 2
        bits.f(4, 5); // scaling 4
        bits.f(0, 4); // num_cb_points = 0
        bits.f(0, 4); // num_cr_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(1, 2); // ar_coeff_lag = 1 -> numPosLuma = 4
        bits.f(0, 2); // bitsCoef = 5, midpoint 16
        bits.f(16, 5); // 0
        bits.f(17, 5); // 1
        bits.f(15, 5); // -1
        bits.f(16, 5); // 0
        // num_cb/cr = 0 and chroma_scaling_from_luma = 0 -> no chroma AR coeffs.
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(1); // overlap_flag
        bits.bit(0); // clip_to_restricted_range
        bits.bit(0); // film_grain_block_size
        // trailing_bits(): a 1 marker then zero-pad to a byte boundary.
        bits.bit(1);
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload_bytes = bits.into_bytes();
        let payload = reparse_payload(header, &payload_bytes);
        match &payload {
            ParsedObu::FilmGrain(fg) => {
                assert_eq!(fg.models.len(), 1);
                assert_eq!(fg.models[0].model.num_y_points, 2);
            }
            other => panic!("expected a film grain OBU, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);

        // And it round-trips through write_complete_obu (header + payload + tail).
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        assert_eq!(reparse_payload(header, &complete_bytes[1..]), payload);
    }

    #[test]
    fn atlas_segment_payload_round_trips() {
        // §5.9 atlas segment is extensible; the dispatch routes it to write_atlas_segment + the
        // extensible tail (obu_extension_flag = 0 + trailing_bits()). It carries no passthrough.
        // Build a BASIC_ATLAS payload with signaled ids (two segments, a stream id) by hand, parse
        // it via dispatch, then write it back and reparse.
        let header = read_obu_header_from_slice(&[0xC4, 0x03], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::AtlasSegment);
        assert!(header.obu_type.is_extensible_obu());
        let mut bits = Bits::default();
        bits.f(3, 3); // atlas_segment_id
        bits.uvlc(1); // mode_idc = BASIC_ATLAS
        bits.bit(1); // ats_stream_id_present
        bits.uvlc(640); // ats_width
        bits.uvlc(480); // ats_height
        bits.uvlc(1); // ats_num_atlas_segments_minus_1 = 1 -> 2 segments
        for _ in 0..2 {
            bits.f(7, 5); // ats_input_stream_id
            bits.uvlc(0); // top_left_pos_x
            bits.uvlc(0); // top_left_pos_y
            bits.uvlc(100); // width
            bits.uvlc(100); // height
        }
        bits.bit(1); // ats_signaled_atlas_segment_ids_flag
        bits.f(10, 8); // ats_atlas_segment_id[0]
        bits.f(20, 8); // ats_atlas_segment_id[1]
        // extensible OBU tail: obu_extension_flag = 0, then trailing_bits().
        bits.bit(0);
        bits.bit(1); // trailing_one_bit
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload_bytes = bits.into_bytes();
        let payload = reparse_payload(header, &payload_bytes);
        match &payload {
            ParsedObu::AtlasSegment(atlas) => {
                assert_eq!(atlas.num_segments, 2);
                assert!(atlas.label.signaled_atlas_segment_ids);
            }
            other => panic!("expected an atlas segment OBU, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, true, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);

        // And it round-trips through write_complete_obu (header + payload + tail).
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        // header is two bytes (extension byte present for xlayer 3).
        assert_eq!(reparse_payload(header, &complete_bytes[2..]), payload);
    }

    #[test]
    fn multi_frame_header_payload_round_trips() {
        // §5.7 multi-frame header is extensible; the dispatch routes it to write_multi_frame_header
        // + the extensible tail (obu_extension_flag = 0 + trailing_bits()). It carries no
        // passthrough. Build a payload with frame size + deblocking update + seg_info(16) present by
        // hand, parse it via dispatch, then write it back and reparse.
        let header = header_for(ObuType::MultiFrameHeader);
        assert!(header.obu_type.is_extensible_obu());
        let mut bits = Bits::default();
        bits.uvlc(2); // mfh_seq_header_id
        bits.uvlc(1); // mfh_id_minus_1 -> mfhId = 2
        bits.bit(1); // mfh_frame_size_present_flag
        bits.f(3, 4); // mfh_frame_width_bits_minus_1 -> width_bits = 4
        bits.f(3, 4); // mfh_frame_height_bits_minus_1 -> height_bits = 4
        bits.f(15, 4); // mfh_frame_width_minus_1
        bits.f(7, 4); // mfh_frame_height_minus_1
        bits.bit(1); // mfh_deblocking_filter_update
        bits.bit(1); // mfh_apply_deblocking_filter[0]
        bits.bit(0); // [1]
        bits.bit(1); // [2]
        bits.bit(0); // [3]
        bits.bit(1); // mfh_seg_info_present_flag
        bits.bit(1); // mfh_ext_seg_flag -> seg_info(16)
        bits.bit(1); // mfh_allow_seg_info_change
        for _ in 0..(16 * 3) {
            bits.bit(0); // seg_info(16): all features disabled
        }
        // extensible OBU tail: obu_extension_flag = 0, then trailing_bits().
        bits.bit(0);
        bits.bit(1); // trailing_one_bit
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload_bytes = bits.into_bytes();
        let payload = reparse_payload(header, &payload_bytes);
        match &payload {
            ParsedObu::MultiFrameHeader(mfh) => {
                let mfh: &MultiFrameHeader = mfh;
                assert!(mfh.mfh_frame_size.is_some());
                assert_eq!(mfh.mfh_apply_deblocking_filter, [true, false, true, false]);
                assert_eq!(mfh.segment_info.as_ref().unwrap().num_segments, 16);
            }
            other => panic!("expected a multi-frame header OBU, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, true, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);

        // And it round-trips through write_complete_obu (header + payload + tail).
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        assert_eq!(reparse_payload(header, &complete_bytes[1..]), payload);
    }

    #[test]
    fn layer_config_record_global_payload_round_trips() {
        // §5.8 layer config record is extensible; the dispatch routes it to
        // write_layer_config_record + the extensible tail. The global/local branch is selected by the
        // header's obu_xlayer_id (here global xlayer 31 -> a global record), so a GLOBAL record only
        // round-trips through write_complete_obu (which threads the header); the header-less
        // write_obu_payload defaults obu_xlayer_id to the non-global 0 and so rejects a global record
        // (like the §5.10 OPS global path). It carries no passthrough.
        let header = read_obu_header_from_slice(&[0xC0, 0x1F], ByteOffset::new(0)).unwrap();
        assert!(header.extended_layer_id.is_global());
        assert!(header.obu_type.is_extensible_obu());
        let mut bits = Bits::default();
        // Minimal lcr_global_info(): id=1, map=0b1, all five flags 0, reserved fields 0.
        bits.f(1, 3); // lcr_global_config_record_id
        bits.f(1, 31); // lcr_xlayer_map
        bits.f(0, 5); // aggregate / ptl / payload / dependent / atlas-present flags
        bits.f(0, 7); // lcr_global_purpose_id
        bits.f(0, 2); // lcr_doh_constraint_flag / lcr_enforce_tile_alignment_flag
        bits.f(0, 3); // lcr_global_reserved_zero_3bits
        bits.f(0, 5); // lcr_global_reserved_zero_5bits
        // extensible OBU tail: obu_extension_flag = 0, then trailing_bits().
        bits.bit(0);
        bits.bit(1); // trailing_one_bit
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload_bytes = bits.into_bytes();
        let payload = reparse_payload(header, &payload_bytes);
        match &payload {
            ParsedObu::LayerConfigurationRecord(record) => {
                assert!(matches!(
                    record.as_ref(),
                    crate::headers::layer_config_record::LayerConfigurationRecord::Global(_)
                ));
            }
            other => panic!("expected a layer configuration record OBU, got {other:?}"),
        }

        // Round-trips through write_complete_obu (header + payload + tail). The global xlayer is
        // carried by the two-byte header (obu_extension_flag set), so skip both header bytes.
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        assert_eq!(reparse_payload(header, &complete_bytes[2..]), payload);
    }

    #[test]
    fn layer_config_record_local_payload_round_trips() {
        // A LOCAL record (header obu_xlayer_id = 0, non-global) round-trips through both the
        // header-less write_obu_payload (whose default xlayer 0 agrees with the local scope) and
        // write_complete_obu.
        let header = header_for(ObuType::LayerConfigurationRecord);
        assert!(!header.extended_layer_id.is_global());
        assert!(header.obu_type.is_extensible_obu());
        let mut bits = Bits::default();
        // Minimal lcr_local_info(xId=0): global_id=0, local_id=1, no ptl, no atlas, reserved 0.
        bits.f(0, 3); // lcr_global_id
        bits.f(1, 3); // lcr_local_id
        bits.bit(0); // lcr_profile_tier_level_info_present_flag
        bits.bit(0); // lcr_local_atlas_id_present_flag
        bits.f(0, 3); // lcr_local_reserved_zero_3bits
        bits.f(0, 5); // lcr_local_reserved_zero_5bits
        // lcr_xlayer_info(0, 0): four present flags clear, then byte_alignment().
        bits.f(0, 4);
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        // extensible OBU tail.
        bits.bit(0);
        bits.bit(1);
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload_bytes = bits.into_bytes();
        let payload = reparse_payload(header, &payload_bytes);
        match &payload {
            ParsedObu::LayerConfigurationRecord(record) => {
                assert!(matches!(
                    record.as_ref(),
                    crate::headers::layer_config_record::LayerConfigurationRecord::Local(_)
                ));
            }
            other => panic!("expected a layer configuration record OBU, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, true, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);

        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        assert_eq!(reparse_payload(header, &complete_bytes[1..]), payload);
    }

    #[test]
    fn layer_config_record_rejects_non_empty_passthrough() {
        let header = read_obu_header_from_slice(&[0xC0, 0x1F], ByteOffset::new(0)).unwrap();
        let payload = reparse_payload(header, &[0x20, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x40]);
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, true, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalLayerConfigRecord { what } if what == "passthrough"),
            "expected layer-config-record passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn quantization_matrix_payload_round_trips() {
        // §5.13 quantizer matrix is NOT extensible; the dispatch routes it to write_quantizer_matrix
        // + the generic (trailing_bits) tail. It carries no passthrough and no scope branch. Build a
        // default level-0 QM by hand, parse it via dispatch, then write it back and reparse.
        let header = read_obu_header_from_slice(&[0x58], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::QuantizationMatrix);
        assert!(!header.obu_type.is_extensible_obu());
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(0); // qm_chroma_info_present_flag = 0 (1 plane)
        bits.bit(1); // qm_is_default_flag = 1
        // non-extensible OBU tail: trailing_bits().
        bits.bit(1); // trailing_one_bit
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload_bytes = bits.into_bytes();
        let payload = reparse_payload(header, &payload_bytes);
        match &payload {
            ParsedObu::QuantizationMatrix(qm) => {
                assert!(!qm.is_reset());
                assert!(qm.levels[0].is_default);
            }
            other => panic!("expected a quantizer matrix OBU, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(reparse_payload(header, &bytes), payload);

        // And it round-trips through write_complete_obu (one-byte header + payload + tail).
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        assert_eq!(reparse_payload(header, &complete_bytes[1..]), payload);
    }

    #[test]
    fn quantization_matrix_rejects_non_empty_passthrough() {
        let header = read_obu_header_from_slice(&[0x58], ByteOffset::new(0)).unwrap();
        // Reset QM (qm_bit_map = 0, chroma = 0) then the trailing-bits byte.
        let payload = reparse_payload(header, &[0x00, 0x00, 0x80]);
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, false, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalQuantizationMatrix { what } if what == "passthrough"),
            "expected quantizer-matrix passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn multi_frame_header_rejects_non_empty_passthrough() {
        let header = header_for(ObuType::MultiFrameHeader);
        // Minimal MFH body (no frame size / deblocking / seg info) then the OBU tail.
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0); // obu_extension_flag
        bits.bit(1); // trailing_one_bit
        while bits.bits.len() % 8 != 0 {
            bits.bit(0);
        }
        let payload = reparse_payload(header, &bits.into_bytes());
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, true, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMultiFrameHeader { what } if what == "passthrough"),
            "expected multi-frame-header passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn atlas_segment_rejects_non_empty_passthrough() {
        let header = read_obu_header_from_slice(&[0xC4, 0x03], ByteOffset::new(0)).unwrap();
        // SINGLE_ATLAS, nominal dims, no signaled ids, then the OBU tail.
        let payload = reparse_payload(header, &[0x0F, 0x20]);
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, true, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalAtlasSegment { what } if what == "passthrough"),
            "expected atlas-segment passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn film_grain_rejects_non_empty_passthrough() {
        let header = header_for(ObuType::FilmGrain);
        let payload = reparse_payload(header, &[0x00, 0xC0]);
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, false, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalFilmGrain { what } if what == "passthrough"),
            "expected film-grain passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn content_interpretation_rejects_non_empty_passthrough() {
        let header = header_for(ObuType::ContentInterpretation);
        let payload = ParsedObu::ContentInterpretation(ContentInterpretation {
            scan_type_idc: ScanTypeIdc::from_bits(0),
            color_description: None,
            chroma_sample_position: None,
            aspect_ratio: None,
            timing_info: None,
            reserved_2bit: 0,
        });
        let mut writer = BitWriter::new();
        let err = write_complete_obu(&mut writer, &header, &payload, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalContentInterpretation { what } if what == "passthrough"),
            "expected passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn operating_point_set_rejects_non_empty_passthrough() {
        let header = read_obu_header_from_slice(&[0xC8, 0x1F], ByteOffset::new(0)).unwrap();
        let payload = reparse_payload(header, &[0x00, 0x40]);
        let mut writer = BitWriter::new();
        let err = write_complete_obu(&mut writer, &header, &payload, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalOperatingPointSet { what } if what == "passthrough"),
            "expected passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn buffer_removal_timing_rejects_non_empty_passthrough() {
        let payload =
            ParsedObu::BufferRemovalTiming(BufferRemovalTiming::ExtendedLayer { br_time: 1 });
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, false, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalBufferRemovalTiming { what } if what == "passthrough"),
            "expected passthrough reject, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }
