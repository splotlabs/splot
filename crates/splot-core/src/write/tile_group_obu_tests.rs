// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]
mod obu_tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        FrameHeaderParseStatus, FrameReferenceStateView, FrameType, init_core_from_prefix,
        parse_core_body, parse_frame_header_prefix,
    };
    use crate::headers::tile_group::{
        TileFraming, parse_tile_group_framing, parse_tile_group_prefix, parse_tile_group_structure,
    };
    use crate::span::ByteOffset;
    use crate::types::ObuType;


    fn base_seq() -> CoreSeqView {
        CoreSeqView::new_minimal_intra(4096, 2304).expect("4096x2304 is a valid maximum")
    }

    use crate::write::frame_header_core::tests::clk_direct_reference_bits as clk_frame_header_bits;

    /// Parses a frame-header body (activation prefix + `parse_core_body`) against the directly built
    /// `seq`, the same approach `frame_header_core_tests.rs::parse_core_body_for_test` uses. Reads
    /// from `reader` (so the caller can continue the stage-by-stage reparse afterward).
    fn parse_core_from(
        reader: &mut BitReader<'_>,
        obu_type: ObuType,
        first_pic: bool,
        seq: &CoreSeqView,
    ) -> FrameHeaderCore {
        let prefix = parse_frame_header_prefix(reader, obu_type, Some(first_pic)).unwrap();
        let mut core = init_core_from_prefix(&prefix, obu_type, first_pic);
        parse_core_body(
            reader,
            &mut core,
            seq,
            None,
            &FrameReferenceStateView::unknown(),
        )
        .unwrap();
        core
    }

    /// Builds a valid single-tile `FrameHeaderCore` + `CoreSeqView` by parsing the CLK fixture.
    fn valid_core() -> (FrameHeaderCore, CoreSeqView) {
        let seq = base_seq();
        let data = clk_frame_header_bits().into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let core = parse_core_from(&mut reader, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(
            core.status,
            FrameHeaderParseStatus::IntraHeaderComplete,
            "fixture must parse to IntraHeaderComplete"
        );
        (core, seq)
    }

    /// A `Complete` single-tile structure (no flag, inferred range 0..=0).
    fn single_tile_structure() -> TileGroupStructure {
        TileGroupStructure {
            tile_start_and_end_present_flag: false,
            tg_start: 0,
            tg_end: 0,
            outcome: TileGroupStructureOutcome::Complete,
            header_bytes: None,
            payload_size: None,
        }
    }

    #[test]
    fn whole_obu_round_trips_stage_by_stage() {
        let (core, seq) = valid_core();
        let layout = TileGroupLayout::new(1, 1, 0, 0);
        assert_eq!(layout.num_tiles, 1);
        let structure = single_tile_structure();

        let tile_bytes: Vec<u8> = (0u8..5).map(|b| b.wrapping_mul(37)).collect();
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        assert_eq!(framing.defect, None);
        assert_eq!(framing.tiles.len(), 1);
        assert_eq!(framing.tiles[0].tile_size, 5);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap();
        let bytes = writer.into_bytes();

        {
            let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
            let prefix =
                parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
            assert!(prefix.is_first_tile_group);
            assert!(prefix.frame_header_present_flag);
            assert!(prefix.frame_header.is_some());
        }

        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));

        assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group == 1");

        let reparsed_core = parse_core_from(&mut reader, ObuType::ClosedLoopKey, true, &seq);
        assert_eq!(
            reparsed_core.status,
            FrameHeaderParseStatus::IntraHeaderComplete
        );
        {
            let mut a = reparsed_core.clone();
            let mut b = core.clone();
            a.consumed_bits = 0;
            b.consumed_bits = 0;
            assert_eq!(a, b, "reparsed frame-header core != input core");
        }
        assert_eq!(reparsed_core.frame_type, Some(FrameType::Key));

        let sz = bytes.len() as u64;
        let parsed_structure = parse_tile_group_structure(&mut reader, layout, sz).unwrap();
        assert_eq!(parsed_structure.outcome, TileGroupStructureOutcome::Complete);
        assert_eq!(
            parsed_structure.tile_start_and_end_present_flag,
            structure.tile_start_and_end_present_flag
        );
        assert_eq!(parsed_structure.tg_start, structure.tg_start);
        assert_eq!(parsed_structure.tg_end, structure.tg_end);

        let header_bytes = parsed_structure.header_bytes.unwrap() as usize;
        let payload_size = parsed_structure.payload_size.unwrap() as usize;
        let region = &bytes[header_bytes..header_bytes + payload_size];
        let parsed_framing = parse_tile_group_framing(region, 0, 0, 1, false);
        assert_eq!(parsed_framing.defect, None);
        assert_eq!(parsed_framing, framing, "reparsed framing != input framing");
        let t = parsed_framing.tiles[0];
        let start = t.tile_data_offset as usize;
        let end = start + t.tile_size as usize;
        assert_eq!(&region[start..end], tile_bytes.as_slice());
    }

    #[test]
    fn reject_continuation_unsupported() {
        let (core, seq) = valid_core();
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            false, // is_first_tile_group == false
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "continuation_unsupported"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on continuation reject");
    }

    #[test]
    fn reject_unaligned_writer() {
        let (core, seq) = valid_core();
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap(); // one stray bit -> not byte-aligned
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(err, WriteError::WriterNotByteAligned);
        assert_eq!(writer.bit_len(), 1, "stray bit unchanged; composer wrote nothing");
    }

    #[test]
    fn reject_not_tile_group_obu() {
        let (mut core, seq) = valid_core();
        core.obu_type = ObuType::RegularSef; // is_tile_group() == false
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "not_tile_group_obu"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on not-tile-group reject");
    }

    #[test]
    fn reject_first_tg_start_not_zero() {
        let (core, seq) = valid_core();
        let mut structure = single_tile_structure();
        structure.tg_start = 1;
        structure.tg_end = 1;
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "first_tg_start_not_zero"
            }
        );
        assert_eq!(
            writer.bit_len(),
            0,
            "no bit written on first-tg-start reject"
        );
    }

    #[test]
    fn reject_missing_tile_info() {
        let (mut core, seq) = valid_core();
        core.tile_info = None;
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "missing_tile_info"
            }
        );
        assert_eq!(
            writer.bit_len(),
            0,
            "no bit written on missing-tile-info reject"
        );
    }

    #[test]
    fn reject_framing_range_mismatch() {
        let (core, seq) = valid_core();
        let structure = single_tile_structure(); // tg_start == tg_end == 0 -> expects 1 tile
        let region: Vec<u8> = vec![0x02, 10, 11, 12, 20, 21, 22, 23];
        let framing = parse_tile_group_framing(&region, 0, 1, 1, false);
        assert_eq!(framing.tiles.len(), 2);
        let tile_data: &[&[u8]] = &[&region[1..4], &region[4..8]];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "framing_range_mismatch"
            }
        );
        assert_eq!(
            writer.bit_len(),
            0,
            "no bit written on framing-range-mismatch reject"
        );
    }

    #[test]
    fn frame_header_sub_writer_reject_propagates() {
        let (mut core, seq) = valid_core();
        core.status = FrameHeaderParseStatus::ActivationFieldsOnly;
        let structure = single_tile_structure();
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader { what: "status" }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on frame-header reject");
    }

    #[test]
    fn structure_sub_writer_reject_propagates() {
        let (core, seq) = valid_core();
        let mut structure = single_tile_structure();
        structure.outcome = TileGroupStructureOutcome::Truncated;
        let tile_bytes = vec![0u8; 4];
        let framing = parse_tile_group_framing(&tile_bytes, 0, 0, 1, false);
        let tile_data: &[&[u8]] = &[&tile_bytes];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "incomplete_structure"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on structure reject");
    }

    #[test]
    fn payload_sub_writer_reject_propagates() {
        let (core, seq) = valid_core();
        let structure = single_tile_structure();
        let framing = TileGroupFraming {
            tiles: vec![TileFraming {
                tile_num: 0,
                size_field_offset: None,
                tile_data_offset: 0,
                tile_size: 0,
            }],
            defect: None,
        };
        let empty: &[u8] = &[];
        let tile_data: &[&[u8]] = &[empty];

        let mut writer = BitWriter::new();
        let err = write_tile_group_obu(
            &mut writer,
            &core,
            &seq,
            None,
            true,
            &structure,
            &framing,
            tile_data,
            true,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "zero_size_tile"
            }
        );
        assert_eq!(writer.bit_len(), 0, "no bit written on payload reject");
    }
}
