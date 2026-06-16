// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Round-trip, Unimplemented, and reject-propagation tests for the unified complete-OBU writer.
// `include!`d into `crate::write::dispatch` so `super::*` resolves to its writers and helpers.
//
// Each round-trip builds a `ParsedObu` (directly, or by parsing a hand-built OBU payload via the
// parser/`dispatch_obu_payload`), writes it with `write_obu_payload` / `write_complete_obu`,
// reparses the emitted bytes, and asserts the reparsed `ParsedObu` equals the original. The
// Unimplemented and reject tests assert the typed error and that no bit was written
// (`bit_len() == 0`).

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::buffer_removal_timing::{BufferRemovalOpTiming, BufferRemovalTiming};
    use crate::headers::content_interpretation::{
        AspectRatioInfo, ColorDescription, ContentInterpretation, ScanTypeIdc,
    };
    use crate::headers::sequence::ProfileIdc;
    use crate::hls::{MultistreamDecoderOperation, SubStreamConfig};
    use crate::headers::metadata::{
        MetadataHdrCll, MetadataPayload, MetadataShortObu, MetadataType, MetadataUnit,
        parse_metadata_short,
    };
    use crate::headers::sequence::{SequenceHeader, parse_sequence_header};
    use crate::obu::{
        ObuHeader, PayloadStatus, dispatch_obu_payload, read_obu_header_from_slice,
    };
    use crate::span::ByteOffset;
    use crate::types::{GLOBAL_XLAYER_ID, ObuType};
    use crate::write::bit_writer::BitWriter;
    use crate::write::error::WriteError;
    use crate::write::obu::write_annexb_obu;

    /// MSB-first bit builder, mirroring the `Bits` helper used across the §5.4 writer tests, so
    /// this module reuses the same hand-built, spec-grounded sequence-header fixture.
    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    /// Builds the §5.4.1 sequence-header **body** bytes for a minimal still-picture header (16x8,
    /// 4:2:0, BLOCK_64X64, no tile config, no film grain), then parses them into a `SequenceHeader`
    /// via `parse_sequence_header`. The body ends byte-aligned; the writer adds its own tail.
    fn still_picture_sequence_header() -> SequenceHeader {
        let mut bits = Bits::default();
        // general (single_picture_header_flag = 1, chroma 4:2:0)
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(1); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx (single picture -> no seq_tier)
        bits.uvlc(0); // chroma_format_idc = CHROMA_FORMAT_420
        bits.uvlc(0); // bit_depth_idc
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1 -> 16
        bits.f(7, 4); // max_frame_height_minus_1 -> 8
        bits.bit(0); // seq_cropping_window_present_flag
        // sequence_partition_config (not monochrome, single picture)
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock -> seqSbSize = BLOCK_64X64
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        // sequence_segment_config
        bits.bit(0); // enable_ext_seg -> MaxSegments = 8
        bits.bit(0); // seq_seg_info_present_flag
        // sequence_intra_config (not monochrome)
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        // sequence_inter_config (single_picture_header_flag branch)
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
        // sequence_scc_config (single picture -> no signalled bits)
        // sequence_transform_quant_entropy_config (not monochrome, single picture)
        bits.bit(0); // enable_fsc
        bits.bit(0); // enable_idtx_intra
        bits.bit(0); // enable_intra_ist
        bits.bit(0); // enable_inter_ist
        bits.bit(0); // enable_chroma_dctonly
        bits.bit(0); // reduced_tx_part_set
        bits.bit(0); // enable_cctx
        bits.bit(0); // enable_tcq
        bits.bit(0); // enable_parity_hiding
        bits.bit(0); // separate_uv_delta_q
        bits.bit(1); // equal_ac_dc_q -> skip y/uv dc delta reads
        bits.f(0, 5); // base_uv_ac_delta_q
        bits.bit(0); // uv_ac_delta_q_enabled
        // sequence_filter_config (single picture, seqSbSize = BLOCK_64X64)
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.f(0, 2); // df_par_bits_minus_2
        // §5.4.2 tile config + film grain
        bits.bit(0); // seq_tile_info_present_flag
        bits.bit(0); // film_grain_params_present
        let body = bits.into_bytes();
        let mut reader = BitReader::new(&body, ByteOffset::new(0));
        let header = parse_sequence_header(&mut reader).unwrap();
        assert!(header.is_fully_parsed(), "fixture header is fully parsed");
        header
    }

    /// Reparses a written OBU payload through the stateless dispatcher and returns the `ParsedObu`,
    /// asserting the parse fully succeeded (no `Opaque` / `PrefixParsed` / `Unimplemented`).
    fn reparse_payload(header: &ObuHeader, payload: &[u8]) -> ParsedObu {
        let status = dispatch_obu_payload(*header, payload, ByteOffset::new(0)).unwrap();
        match status {
            PayloadStatus::Parsed(parsed) => parsed,
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    /// Builds a no-extension OBU header for `obu_type` (tlayer 0), the natural header for the
    /// global-scope and base-layer round-trip fixtures here.
    fn header_for(obu_type: ObuType) -> ObuHeader {
        // obu_header() = ext(0) | type(5) | tlayer(0): the type occupies bits 6..2.
        let header_byte = obu_type.raw() << 2;
        read_obu_header_from_slice(&[header_byte], ByteOffset::new(0)).unwrap()
    }

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
        let reparsed = reparse_payload(&header, &bytes);
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
        let reparsed = reparse_payload(&header, &bytes);
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
        let reparsed = reparse_payload(&header, &bytes);
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
        assert_eq!(reparse_payload(&header, &b0), empty);

        // obuPayloadSize == 1: no padding bytes, one trailing_bits() byte.
        let one = ParsedObu::Padding(PaddingObu {
            padding_len: 0,
            trailing_len: 1,
        });
        let mut w1 = BitWriter::new();
        write_obu_payload(&mut w1, &one, false, &[]).unwrap();
        let b1 = w1.into_bytes();
        assert_eq!(b1, vec![0x80], "obuPayloadSize 1 writes one trailing byte");
        assert_eq!(reparse_payload(&header, &b1), one);
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
        let reparsed = reparse_payload(&header, &bytes);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn metadata_group_single_unit_payload_round_trips() {
        // Parse the obu.rs dispatch group fixture (one cancelled unit), slicing off the OBU
        // trailing byte the parser would have consumed, so the model is the writer's input.
        let header = header_for(ObuType::MetadataGroup);
        // Group with one cancelled unit then a trailing byte: [0x00, 0x00, 0x04, 0x01, 0x80].
        let fixture = [0x00u8, 0x00, 0x04, 0x01, 0x80];
        let parsed = reparse_payload(&header, &fixture);
        let payload = parsed.clone();

        let mut writer = BitWriter::new();
        // Single-unit, non-global xlayer (header xlayer is 0 here) — the write_obu_payload path.
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_payload(&header, &bytes);
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
        let payload = reparse_payload(&header, &fixture);
        // Sanity: the fixture really is a two-unit group (else the test would not exercise the fix).
        match &payload {
            ParsedObu::MetadataGroup(obu) => assert_eq!(obu.units.len(), 2),
            other => panic!("expected a metadata group, got {other:?}"),
        }

        let mut writer = BitWriter::new();
        write_obu_payload(&mut writer, &payload, false, &[]).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = reparse_payload(&header, &bytes);
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
        assert_eq!(reparse_payload(&header, &bytes), payload);
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
        assert_eq!(reparse_payload(&header, &bytes), payload);
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
        let payload = reparse_payload(&header, &[0x00, 0x40]);
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
        let reparsed = reparse_payload(&header, &bytes[2..]);
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
        let payload = reparse_payload(&header, &payload_bytes);
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
        assert_eq!(reparse_payload(&header, &bytes), payload);
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
        assert_eq!(reparse_payload(&header, &bytes), payload);

        // And it round-trips through write_complete_obu (header + payload + tail).
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        assert_eq!(reparse_payload(&header, &complete_bytes[1..]), payload);
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
        let payload = reparse_payload(&header, &[0x00, 0x40]);
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

    // ===================================================================================
    // write_complete_obu round-trips (framed through write_annexb_obu)
    // ===================================================================================

    #[test]
    fn complete_obu_sequence_header_round_trips_via_annexb() {
        let header = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::SequenceHeader);
        let payload = ParsedObu::SequenceHeader(Box::new(still_picture_sequence_header()));

        // write_complete_obu emits header + payload + tail; capture the payload bytes alone to
        // frame them with write_annexb_obu (which prepends leb128(num_bytes_in_obu) + header).
        let mut payload_writer = BitWriter::new();
        write_obu_payload(&mut payload_writer, &payload, true, &[]).unwrap();
        let payload_bytes = payload_writer.into_bytes();

        let mut framed = BitWriter::new();
        write_annexb_obu(&mut framed, &header, &payload_bytes).unwrap();
        let bytes = framed.into_bytes();

        let parsed = crate::annexb::parse_annex_b_obus_partial(&bytes);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.obus.len(), 1);
        assert_eq!(parsed.obus[0].header, header);
        let reparsed = reparse_payload(&parsed.obus[0].header, parsed.obus[0].payload);
        assert_eq!(reparsed, payload);

        // And write_complete_obu produces exactly header-bytes ++ payload-bytes.
        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let complete_bytes = complete.into_bytes();
        let mut expected = BitWriter::new();
        write_obu_payload(&mut expected, &payload, true, &[]).unwrap();
        let expected_payload = expected.into_bytes();
        assert_eq!(&complete_bytes[..1], &[0x04]); // the §5.2.2 header byte
        assert_eq!(&complete_bytes[1..], &expected_payload[..]);
    }

    #[test]
    fn complete_obu_temporal_delimiter_round_trips_via_annexb() {
        let header = read_obu_header_from_slice(&[0x08], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::TemporalDelimiter);
        let payload = ParsedObu::TemporalDelimiter;

        let mut framed = BitWriter::new();
        write_annexb_obu(&mut framed, &header, &[]).unwrap();
        let bytes = framed.into_bytes();
        assert_eq!(bytes, vec![0x01, 0x08], "leb128(1) + TD header, empty payload");
        let parsed = crate::annexb::parse_annex_b_obus_partial(&bytes);
        assert!(parsed.error.is_none());
        let reparsed = reparse_payload(&parsed.obus[0].header, parsed.obus[0].payload);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn complete_obu_group_uses_header_xlayer() {
        // A global-xlayer (31) metadata-group header takes the §6.16.3 global layer-map branch.
        // write_complete_obu threads header.extended_layer_id, so a global-branch group writes
        // and round-trips through write_complete_obu even though write_obu_payload alone defaults
        // to the local branch.
        let header = read_obu_header_from_slice(&[0xA4, 0x1F], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::MetadataGroup);
        assert_eq!(header.extended_layer_id, GLOBAL_XLAYER_ID);

        // One cancelled unit (no layer maps regardless of branch) then a trailing byte.
        let fixture = [0x00u8, 0x00, 0x04, 0x01, 0x80];
        let payload = reparse_payload(&header, &fixture);

        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let bytes = complete.into_bytes();
        // bytes = header (2) ++ payload; reparse the payload region.
        let reparsed = reparse_payload(&header, &bytes[2..]);
        assert_eq!(reparsed, payload);
    }

    // ===================================================================================
    // Unimplemented stubs (>= 2 unwritten types)
    // ===================================================================================

    #[test]
    fn unimplemented_types_return_typed_stub_without_writing() {
        // Build the unwritten payloads by parsing minimal OBU payloads via dispatch, then assert
        // write_obu_payload returns Unimplemented with the matrix Feature ID and writes nothing.

        // OBU_QUANTIZATION_MATRIX (non-extensible): qm_bit_map(15) + chroma flag(1) + trailing.
        let qm_header = read_obu_header_from_slice(&[0x58], ByteOffset::new(0)).unwrap();
        let qm = reparse_payload(&qm_header, &[0x00, 0x00, 0x80]);
        assert_eq!(qm.feature_id(), "AV2-5.13-QUANTIZATION-MATRIX");

        // OBU_FILM_GRAIN (non-extensible).
        let fg_header = read_obu_header_from_slice(&[0x5C], ByteOffset::new(0)).unwrap();
        let fg = reparse_payload(&fg_header, &[0x00, 0xC0]);
        assert_eq!(fg.feature_id(), "AV2-5.14-FILM-GRAIN");

        for (payload, header) in [(&qm, &qm_header), (&fg, &fg_header)] {
            let mut writer = BitWriter::new();
            let err = write_obu_payload(
                &mut writer,
                payload,
                header.obu_type.is_extensible_obu(),
                &[],
            )
            .unwrap_err();
            assert!(
                matches!(err, WriteError::Unimplemented { feature } if feature == payload.feature_id()),
                "expected Unimplemented {{ {} }}, got {err:?}",
                payload.feature_id()
            );
            assert_eq!(writer.bit_len(), 0, "no bits written for an unimplemented type");

            // write_complete_obu propagates the same stub and leaves the writer clean (no
            // stray OBU-header byte).
            let mut complete = BitWriter::new();
            let cerr = write_complete_obu(&mut complete, header, payload, &[]).unwrap_err();
            assert!(matches!(cerr, WriteError::Unimplemented { .. }));
            assert_eq!(complete.bit_len(), 0, "no header byte written on reject");
        }
    }

    // ===================================================================================
    // Sub-writer reject propagation (bit_len() == 0)
    // ===================================================================================

    #[test]
    fn non_canonical_sequence_header_reject_propagates() {
        // An unwritable sequence header (unimplemented_at set) makes write_sequence_header reject
        // with UnwritableSequenceHeader; the dispatch must propagate it and write nothing.
        let mut header = still_picture_sequence_header();
        header.unimplemented_at = Some("AV2-5.4.2-SEQUENCE-TILE-CONFIG");
        let payload = ParsedObu::SequenceHeader(Box::new(header));

        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, true, &[]).unwrap_err();
        assert!(
            matches!(err, WriteError::UnwritableSequenceHeader { .. }),
            "expected UnwritableSequenceHeader, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "sub-writer reject left bits in the writer");
    }

    #[test]
    fn non_canonical_metadata_reject_propagates() {
        // A cancelled short OBU carrying a unit is non-canonical (§5.17.2); the metadata writer
        // rejects it (`short_cancel_unit`) and the dispatch must propagate, writing nothing.
        let bytes = [0x08u8, 0x04, 0x80];
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let mut obu = parse_metadata_short(&mut reader, bytes.len()).unwrap();
        // Force the non-canonical combination: cancelled but a unit present.
        obu.muh_cancel_flag = true;
        obu.unit = Some(MetadataUnit {
            metadata_type: MetadataType::HdrCll,
            payload_size: 4,
            payload: MetadataPayload::HdrCll(MetadataHdrCll {
                max_cll: 0,
                max_fall: 0,
            }),
        });
        let payload = ParsedObu::MetadataShort(Box::new(obu));

        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, false, &[]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMetadata { .. }),
            "expected NonCanonicalMetadata, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "metadata reject left bits in the writer");
    }

    #[test]
    fn padding_passthrough_mismatch_rejects_without_writing() {
        // padding_len says 2 but only 1 passthrough byte is supplied.
        let payload = ParsedObu::Padding(PaddingObu {
            padding_len: 2,
            trailing_len: 1,
        });
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, false, &[0xDE]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMetadata { what } if what == "padding_passthrough_len"),
            "expected padding_passthrough_len, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn padding_trailing_len_zero_with_padding_bytes_rejects_without_writing() {
        // §5.16: the parser splits at the last non-zero byte, so a non-empty payload always
        // has trailing_len >= 1. `trailing_len == 0` with `padding_len > 0` is a hand-built
        // model the parser never emits (its bytes would reparse as InvalidPadding), so the
        // writer must reject it before any bit rather than emit a non-round-tripping stream.
        let payload = ParsedObu::Padding(PaddingObu {
            padding_len: 3,
            trailing_len: 0,
        });
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, false, &[0xDE, 0xAD, 0xBE]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMetadata { what } if what == "padding_trailing_len"),
            "expected padding_trailing_len, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "padding reject left bits in the writer");
    }

    #[test]
    fn temporal_delimiter_rejects_non_empty_passthrough() {
        let payload = ParsedObu::TemporalDelimiter;
        let mut writer = BitWriter::new();
        let err = write_obu_payload(&mut writer, &payload, false, &[0x00]).unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalMetadata { what } if what == "temporal_delimiter_passthrough"),
            "expected temporal_delimiter_passthrough, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn complete_obu_rejects_unaligned_writer() {
        let header = read_obu_header_from_slice(&[0x08], ByteOffset::new(0)).unwrap();
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap();
        let err =
            write_complete_obu(&mut writer, &header, &ParsedObu::TemporalDelimiter, &[]).unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1, "unaligned reject left the writer untouched");
    }

    #[test]
    fn complete_obu_rejects_mismatched_header_payload() {
        // The §5.2.1 OBU dispatch routes one obu_type to one payload syntax, so a SequenceHeader
        // header paired with a TemporalDelimiter payload is a pair the parser could never produce
        // (it would reparse as a sequence header). Reject before any bit.
        let header = header_for(ObuType::SequenceHeader);
        let mut writer = BitWriter::new();
        let err = write_complete_obu(&mut writer, &header, &ParsedObu::TemporalDelimiter, &[])
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::ObuTypePayloadMismatch {
                payload: "temporal_delimiter_obu"
            }
        );
        assert_eq!(writer.bit_len(), 0, "mismatch reject left the writer untouched");
    }
}
