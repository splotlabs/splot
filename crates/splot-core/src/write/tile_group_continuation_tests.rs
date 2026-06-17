// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Round-trip and reject tests for the non-first (continuation) tile_group_obu() writer (§ 5.19 /
// § 5.20.1). `include!`d into `crate::write::tile_group` so `super::*` resolves to the composer.
//
// The continuation has no single parseable struct — the pieces (prefix, frame_header_copy() bits,
// structure, framing) are parsed/validated separately. The round-trip test builds the recorded first
// header + a structure + a framing, composes the OBU payload, then reparses it piece by piece
// (prefix -> recorded copy bits -> structure -> framing) and asserts each piece matches and the copy
// region is bit-identical. Reject tests assert reject-before-write (`bit_len() == 0`).

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod continuation_tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::tile_group::{
        TileFraming, parse_tile_group_framing, parse_tile_group_prefix, parse_tile_group_structure,
    };
    use crate::span::ByteOffset;
    use crate::types::ObuType;

    /// MSB-first bit builder mirroring the parser/writer test helpers.
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

    /// Builds a [`RecordedFrameHeaderBits`] of `num_bits` bits from `pattern` (left-aligned MSB-first).
    fn recorded_header(pattern: u32, num_bits: u32) -> RecordedFrameHeaderBits {
        let mut bits = Bits::default();
        bits.f(pattern, num_bits);
        let bytes = bits.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        RecordedFrameHeaderBits::record(&mut reader, u64::from(num_bits)).unwrap()
    }

    /// A 2-tile layout: tileBits (`TileColsLog2 + TileRowsLog2`) == 1, so a continuation covering the
    /// single last tile carries `tg_start = tg_end = 1`.
    fn two_tile_layout() -> TileGroupLayout {
        TileGroupLayout::new(2, 1, 1, 0)
    }

    fn last_tile_framing(tile_num: u32, tile_size: u64) -> TileGroupFraming {
        TileGroupFraming {
            tiles: vec![TileFraming {
                tile_num,
                size_field_offset: None, // the last tile reads no size field
                tile_data_offset: 0,
                tile_size,
            }],
            defect: None,
        }
    }

    fn structure(present: bool, tg_start: u32, tg_end: u32) -> TileGroupStructure {
        TileGroupStructure {
            tile_start_and_end_present_flag: present,
            tg_start,
            tg_end,
            outcome: TileGroupStructureOutcome::Complete,
            header_bytes: None, // parse-context; ignored by the writer
            payload_size: None,
        }
    }

    #[test]
    fn non_first_tile_group_round_trips() {
        // Continuation covering the last tile of a 2-tile frame, with a 10-bit frame_header_copy().
        let recorded = recorded_header(0b10_1100_1101, 10);
        let layout = two_tile_layout();
        let tile_bytes: &[u8] = &[0x11, 0x22, 0x33];
        let framing = last_tile_framing(1, tile_bytes.len() as u64);
        let structure = structure(true, 1, 1);

        let mut writer = BitWriter::new();
        write_tile_group_continuation_obu(
            &mut writer,
            Some(&recorded),
            true,
            layout,
            1,
            &structure,
            &framing,
            &[tile_bytes],
            false,
        )
        .unwrap();
        let bytes = writer.into_bytes();
        let sz = bytes.len() as u64;

        // Reparse piece by piece from one reader.
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let prefix = parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, None).unwrap();
        assert!(!prefix.is_first_tile_group);
        assert!(prefix.frame_header_present_flag);
        assert!(prefix.frame_header.is_none());

        // frame_header_copy(): exactly NumFrameHeaderBits bits, bit-identical to the recorded header.
        let copy = RecordedFrameHeaderBits::record(&mut reader, recorded.num_frame_header_bits())
            .unwrap();
        assert_eq!(copy, recorded, "frame_header_copy() is bit-identical");

        let reparsed_structure = parse_tile_group_structure(&mut reader, layout, sz).unwrap();
        assert_eq!(reparsed_structure.outcome, TileGroupStructureOutcome::Complete);
        assert_eq!(reparsed_structure.tg_start, 1);
        assert_eq!(reparsed_structure.tg_end, 1);
        assert!(reparsed_structure.tile_start_and_end_present_flag);

        // The framing region is the payload bytes after the byte-aligned header.
        let hb = reparsed_structure.header_bytes.unwrap() as usize;
        let reparsed_framing = parse_tile_group_framing(
            &bytes[hb..],
            reparsed_structure.tg_start,
            reparsed_structure.tg_end,
            1,
            false,
        );
        assert!(reparsed_framing.defect.is_none());
        assert_eq!(reparsed_framing.tiles.len(), 1);
        assert_eq!(reparsed_framing.tiles[0].tile_size, 3);
        assert_eq!(&bytes[hb..], tile_bytes, "tile data round-trips byte-exact");
    }

    #[test]
    fn non_first_tile_group_without_frame_header_round_trips() {
        // A continuation whose frame_header_present_flag is 0 (no frame_header_copy()).
        let layout = two_tile_layout();
        let tile_bytes: &[u8] = &[0xAB, 0xCD];
        let framing = last_tile_framing(1, tile_bytes.len() as u64);
        let structure = structure(true, 1, 1);

        let mut writer = BitWriter::new();
        write_tile_group_continuation_obu(
            &mut writer,
            None,
            false,
            layout,
            1,
            &structure,
            &framing,
            &[tile_bytes],
            false,
        )
        .unwrap();
        let bytes = writer.into_bytes();

        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let prefix = parse_tile_group_prefix(&mut reader, ObuType::ClosedLoopKey, None).unwrap();
        assert!(!prefix.is_first_tile_group);
        assert!(!prefix.frame_header_present_flag, "no frame_header_copy()");
        let reparsed_structure =
            parse_tile_group_structure(&mut reader, layout, bytes.len() as u64).unwrap();
        assert_eq!(reparsed_structure.tg_start, 1);
        let hb = reparsed_structure.header_bytes.unwrap() as usize;
        assert_eq!(&bytes[hb..], tile_bytes);
    }

    #[test]
    fn rejects_non_byte_aligned_writer() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap(); // now mid-byte
        let recorded = recorded_header(0b1, 1);
        let err = write_tile_group_continuation_obu(
            &mut writer,
            Some(&recorded),
            true,
            two_tile_layout(),
            1,
            &structure(true, 1, 1),
            &last_tile_framing(1, 1),
            &[&[0x00][..]],
            false,
        )
        .unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
    }

    #[test]
    fn rejects_frame_header_copy_gate_flag_without_bits() {
        // frame_header_present_flag set but no recorded copy bits supplied.
        let mut writer = BitWriter::new();
        let err = write_tile_group_continuation_obu(
            &mut writer,
            None,
            true,
            two_tile_layout(),
            1,
            &structure(true, 1, 1),
            &last_tile_framing(1, 1),
            &[&[0x00][..]],
            false,
        )
        .unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalTileGroup { what } if what == "frame_header_copy_gate")
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_frame_header_copy_gate_bits_without_flag() {
        // recorded copy bits supplied but frame_header_present_flag clear.
        let recorded = recorded_header(0b1, 1);
        let mut writer = BitWriter::new();
        let err = write_tile_group_continuation_obu(
            &mut writer,
            Some(&recorded),
            false,
            two_tile_layout(),
            1,
            &structure(true, 1, 1),
            &last_tile_framing(1, 1),
            &[&[0x00][..]],
            false,
        )
        .unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalTileGroup { what } if what == "frame_header_copy_gate")
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_framing_range_mismatch() {
        // The structure says one tile (tg_start == tg_end == 1) but the framing carries two records.
        let recorded = recorded_header(0b1, 1);
        let two_tiles = TileGroupFraming {
            tiles: vec![
                TileFraming {
                    tile_num: 1,
                    size_field_offset: Some(0),
                    tile_data_offset: 1,
                    tile_size: 1,
                },
                TileFraming {
                    tile_num: 2,
                    size_field_offset: None,
                    tile_data_offset: 2,
                    tile_size: 1,
                },
            ],
            defect: None,
        };
        let mut writer = BitWriter::new();
        let err = write_tile_group_continuation_obu(
            &mut writer,
            Some(&recorded),
            true,
            two_tile_layout(),
            1,
            &structure(true, 1, 1),
            &two_tiles,
            &[&[0x00][..], &[0x11][..]],
            false,
        )
        .unwrap_err();
        assert!(
            matches!(err, WriteError::NonCanonicalTileGroup { what } if what == "framing_range_mismatch")
        );
        assert_eq!(writer.bit_len(), 0);
    }
}
