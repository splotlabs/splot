// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit / reject tests for the §5.19 tile-group structure writer. Round-trips write a constructed
// structure into a fresh writer, then reparse the emitted bytes via parse_tile_group_structure and
// assert the syntax fields match and outcome == Complete; reject tests assert the typed error and
// that no bit was written (`bit_len() == 0`).

// `include!`d into `crate::write::tile_group` so `super::*` resolves to its writer and helpers.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::tile_group::parse_tile_group_structure;
    use crate::span::ByteOffset;

    /// Builds a `Complete` structure with the given fields and `None` parse-context artifacts (the
    /// writer ignores `header_bytes` / `payload_size`).
    fn structure(flag: bool, tg_start: u32, tg_end: u32) -> TileGroupStructure {
        TileGroupStructure {
            tile_start_and_end_present_flag: flag,
            tg_start,
            tg_end,
            outcome: TileGroupStructureOutcome::Complete,
            header_bytes: None,
            payload_size: None,
        }
    }

    /// Writes `s` for `layout` into a fresh writer, reparses the emitted bytes, and asserts the
    /// syntax fields round-trip and the reparse is `Complete`. `sz` is the OBU payload size the
    /// reparse is told (it only affects the parse-context fields, which this slice does not own).
    fn assert_round_trips(s: &TileGroupStructure, layout: TileGroupLayout) {
        let mut writer = BitWriter::new();
        write_tile_group_structure(&mut writer, s, layout).unwrap();
        let bytes = writer.into_bytes();
        // A zero-bit single-tile structure emits no bytes (align_to_byte() is a no-op at nbits==0,
        // so into_bytes() is empty and the reparse runs with sz == 0); once any bit is written,
        // byte_alignment() rounds up to a whole byte. `bytes.len()` gives the reparse the exact size
        // either way.
        let sz = bytes.len() as u64;
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let parsed = parse_tile_group_structure(&mut reader, layout, sz).unwrap();
        assert_eq!(
            parsed.tile_start_and_end_present_flag,
            s.tile_start_and_end_present_flag
        );
        assert_eq!(parsed.tg_start, s.tg_start);
        assert_eq!(parsed.tg_end, s.tg_end);
        assert_eq!(parsed.outcome, TileGroupStructureOutcome::Complete);
    }

    #[test]
    fn round_trips_single_tile_inferred_range() {
        // NumTiles == 1: no flag, inferred tg_start = 0, tg_end = 0; byte_alignment() pads to a byte.
        let layout = TileGroupLayout::new(1, 1, 0, 0);
        assert_eq!(layout.num_tiles, 1);
        assert_round_trips(&structure(false, 0, 0), layout);
    }

    #[test]
    fn round_trips_multi_tile_flag_clear_inferred_range() {
        // 2x2 -> NumTiles == 4, tileBits == 2. flag = 0 -> no range bits, inferred 0 .. 3.
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        assert_eq!(layout.num_tiles, 4);
        assert_eq!(layout.tile_bits(), 2);
        assert_round_trips(&structure(false, 0, 3), layout);
    }

    #[test]
    fn round_trips_multi_tile_flag_set_explicit_range() {
        // 2x2 -> tileBits == 2; flag = 1, explicit tg_start = 1, tg_end = 2.
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        assert_round_trips(&structure(true, 1, 2), layout);
    }

    #[test]
    fn round_trips_explicit_range_tile_bits_boundary_one() {
        // tileBits == 1 (2x1 layout, cols_log2 = 1, rows_log2 = 0): the narrowest signaled range.
        let layout = TileGroupLayout::new(2, 1, 1, 0);
        assert_eq!(layout.num_tiles, 2);
        assert_eq!(layout.tile_bits(), 1);
        // The only valid tg_end >= tg_start pair that fits f(1): tg_start = 0, tg_end = 1.
        assert_round_trips(&structure(true, 0, 1), layout);
        // And the degenerate single-tile-group range tg_start = tg_end = 1 (still fits f(1)).
        assert_round_trips(&structure(true, 1, 1), layout);
    }

    #[test]
    fn round_trips_explicit_range_wider_tile_bits() {
        // 8x8 -> NumTiles == 64, cols_log2 = 3, rows_log2 = 3 -> tileBits == 6. Exercise a wide
        // range incl. the top index 63 (== 2^6 - 1).
        let layout = TileGroupLayout::new(8, 8, 3, 3);
        assert_eq!(layout.num_tiles, 64);
        assert_eq!(layout.tile_bits(), 6);
        assert_round_trips(&structure(true, 5, 63), layout);
    }

    #[test]
    fn round_trips_explicit_range_at_tile_bits_cap() {
        // tile_bits() caps at 32 when TileColsLog2 + TileRowsLog2 >= 32; the writer special-cases
        // this boundary (the u64 `1 << tile_bits` fit bound and the f(32) `write_bits`). num_tiles
        // == 2 keeps the range signaled. Cover the full 32-bit field incl. the maximum index
        // u32::MAX, so the cap is exercised by a deterministic round-trip (not only incidentally by
        // the never-panics proptest).
        let layout = TileGroupLayout::new(2, 1, 200, 200);
        assert_eq!(layout.num_tiles, 2);
        assert_eq!(layout.tile_bits(), 32);
        assert_round_trips(&structure(true, 0x1234_5678, u32::MAX), layout);
    }

    #[test]
    fn rejects_incomplete_structure() {
        let layout = TileGroupLayout::new(1, 1, 0, 0);
        let mut s = structure(false, 0, 0);
        s.outcome = TileGroupStructureOutcome::Truncated;
        let mut writer = BitWriter::new();
        let err = write_tile_group_structure(&mut writer, &s, layout).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "incomplete_structure"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_degenerate_layout() {
        // NumTiles == 0 (a 0x0 layout). The flag/range are irrelevant; the layout is rejected first.
        let layout = TileGroupLayout::new(0, 0, 0, 0);
        assert_eq!(layout.num_tiles, 0);
        let mut writer = BitWriter::new();
        let err = write_tile_group_structure(&mut writer, &structure(false, 0, 0), layout)
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "degenerate_layout"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_inferred_range_mismatch() {
        // Multi-tile, flag clear, but tg_end != NumTiles - 1: the inferred default would be
        // 0 .. 3, so the model could not round-trip.
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        let mut writer = BitWriter::new();
        let err = write_tile_group_structure(&mut writer, &structure(false, 0, 2), layout)
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "inferred_range_mismatch"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_flag_without_multi_tile() {
        // NumTiles == 1 but the flag is set: the parser never reads a flag for a single tile.
        let layout = TileGroupLayout::new(1, 1, 0, 0);
        let mut writer = BitWriter::new();
        let err = write_tile_group_structure(&mut writer, &structure(true, 0, 0), layout)
            .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "flag_without_multi_tile"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_tg_range_inverted() {
        // Signaled range (NumTiles > 1 && flag) with tg_end < tg_start.
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        let mut writer = BitWriter::new();
        let err = write_tile_group_structure(&mut writer, &structure(true, 2, 1), layout)
            .unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalTileGroup { what: "tg_range" });
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_tg_range_out_of_field() {
        // Signaled range with tg_start >= 1 << tileBits: tileBits == 2 here, so tg_start = 4 does
        // not fit f(2).
        let layout = TileGroupLayout::new(2, 2, 1, 1);
        assert_eq!(layout.tile_bits(), 2);
        let mut writer = BitWriter::new();
        let err = write_tile_group_structure(&mut writer, &structure(true, 4, 5), layout)
            .unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalTileGroup { what: "tg_range" });
        assert_eq!(writer.bit_len(), 0);
    }
}
