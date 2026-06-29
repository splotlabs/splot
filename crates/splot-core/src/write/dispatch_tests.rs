// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


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
    use crate::hls::MultiFrameHeader;
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

    use crate::test_bits::Bits;

    /// Builds the §5.4.1 sequence-header **body** bytes for a minimal still-picture header (16x8,
    /// 4:2:0, BLOCK_64X64, no tile config, no film grain), then parses them into a `SequenceHeader`
    /// via `parse_sequence_header`. The body ends byte-aligned; the writer adds its own tail.
    fn still_picture_sequence_header() -> SequenceHeader {
        let mut bits = Bits::default();
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
        bits.bit(0); // use_256x256_superblock
        bits.bit(0); // use_128x128_superblock -> seqSbSize = BLOCK_64X64
        bits.bit(0); // enable_sdp
        bits.bit(0); // enable_ext_partitions
        bits.bit(0); // reduce_pb_aspect_ratio
        bits.bit(0); // enable_ext_seg -> MaxSegments = 8
        bits.bit(0); // seq_seg_info_present_flag
        bits.bit(0); // enable_dip
        bits.bit(0); // enable_intra_edge_filter
        bits.bit(0); // enable_mrls
        bits.bit(0); // enable_cfl_intra
        bits.f(0, 2); // cfl_ds_filter_index
        bits.bit(0); // enable_mhccp
        bits.bit(0); // enable_ibp
        bits.bit(0); // enable_refmvbank
        bits.bit(1); // disable_drl_reorder -> DRL_REORDER_DISABLED
        bits.bit(0); // seq_max_bvp_drl_bits_minus_1 = ns(3) -> 0
        bits.bit(0); // allow_frame_max_bvp_drl_bits
        bits.bit(0); // enable_bawp
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
        bits.bit(0); // disable_loopfilters_across_tiles
        bits.bit(0); // enable_cdef
        bits.bit(0); // enable_gdf
        bits.bit(0); // enable_restoration
        bits.bit(0); // enable_ccso
        bits.f(0, 2); // df_par_bits_minus_2
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
    fn reparse_payload(header: ObuHeader, payload: &[u8]) -> ParsedObu {
        let status = dispatch_obu_payload(header, payload, ByteOffset::new(0)).unwrap();
        match status {
            PayloadStatus::Parsed(parsed) => parsed,
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    /// Builds a no-extension OBU header for `obu_type` (tlayer 0), the natural header for the
    /// global-scope and base-layer round-trip fixtures here.
    fn header_for(obu_type: ObuType) -> ObuHeader {
        let header_byte = obu_type.raw() << 2;
        read_obu_header_from_slice(&[header_byte], ByteOffset::new(0)).unwrap()
    }

    include!("dispatch_roundtrip_tests.rs");


    #[test]
    fn complete_obu_sequence_header_round_trips_via_annexb() {
        let header = read_obu_header_from_slice(&[0x04], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::SequenceHeader);
        let payload = ParsedObu::SequenceHeader(Box::new(still_picture_sequence_header()));

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
        let reparsed = reparse_payload(parsed.obus[0].header, parsed.obus[0].payload);
        assert_eq!(reparsed, payload);

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
        let reparsed = reparse_payload(parsed.obus[0].header, parsed.obus[0].payload);
        assert_eq!(reparsed, payload);
    }

    #[test]
    fn complete_obu_group_uses_header_xlayer() {
        let header = read_obu_header_from_slice(&[0xA4, 0x1F], ByteOffset::new(0)).unwrap();
        assert_eq!(header.obu_type, ObuType::MetadataGroup);
        assert_eq!(header.extended_layer_id, GLOBAL_XLAYER_ID);

        let fixture = [0x00u8, 0x00, 0x04, 0x01, 0x80];
        let payload = reparse_payload(header, &fixture);

        let mut complete = BitWriter::new();
        write_complete_obu(&mut complete, &header, &payload, &[]).unwrap();
        let bytes = complete.into_bytes();
        let reparsed = reparse_payload(header, &bytes[2..]);
        assert_eq!(reparsed, payload);
    }


    #[test]
    fn non_canonical_sequence_header_reject_propagates() {
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
