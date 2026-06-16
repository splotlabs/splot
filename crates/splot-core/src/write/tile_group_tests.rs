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
        // == u32::MAX keeps the signaled range valid while letting tg_end span the full 32-bit field
        // just under NumTiles (§6.18 in-range), so the cap is exercised by a deterministic
        // round-trip (not only incidentally by the never-panics proptest).
        let layout = TileGroupLayout::new(u32::MAX, 1, 200, 200);
        assert_eq!(layout.num_tiles, u32::MAX);
        assert_eq!(layout.tile_bits(), 32);
        assert_round_trips(&structure(true, 0x1234_5678, u32::MAX - 1), layout);
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

    #[test]
    fn rejects_tg_out_of_range() {
        // A non-power-of-two grid (3 tiles) has tileBits == 2, so tg_end == 3 FITS f(2) but exceeds
        // the last tile index NumTiles-1 == 2 — the §6.18 out-of-range conformance refusal, distinct
        // from the f(tileBits) fit reject (which fires for tg_range above).
        let layout = TileGroupLayout::new(3, 1, 1, 1);
        assert_eq!(layout.num_tiles, 3);
        assert_eq!(layout.tile_bits(), 2);
        let mut writer = BitWriter::new();
        let err =
            write_tile_group_structure(&mut writer, &structure(true, 0, 3), layout).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "tg_out_of_range"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }
}

// Unit / reject tests for the §5.20.1 tile-group payload framing writer. The round-trip tests are
// parser-driven: build a conformant byte region by hand, parse it once to a known-good
// `TileGroupFraming` + per-tile `tile_data` slices, write that framing back into a fresh writer,
// then (a) assert the rewritten bytes are byte-exact with the original region and (b) reparse them
// and assert the `TileGroupFraming` is value-equal to the original (recomputed offsets + sizes +
// `defect == None`) and each tile's bytes round-trip exactly. Reject tests assert the typed error
// and that no bit was written (`bit_len() == 0`).

// `include!`d into `crate::write::tile_group` so `super::*` resolves to its writers and helpers.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod payload_tests {
    use super::*;
    use crate::headers::tile_group::parse_tile_group_framing;

    /// Encodes `tile_size_minus_1` as a `tile_size_bytes`-byte little-endian `le(n)` field
    /// (§ 4.11.5), the way a conformant tile group writes its tile size.
    fn le_size_field(tile_size_minus_1: u64, tile_size_bytes: u32) -> Vec<u8> {
        (0..tile_size_bytes)
            .map(|i| ((tile_size_minus_1 >> (i * 8)) & 0xFF) as u8)
            .collect()
    }

    /// Builds a conformant `tile_group_payload()` region for `tile_sizes` (in tile order; the LAST
    /// is the remainder tile with no size field), using `tile_size_bytes` for the size fields. Each
    /// tile's coded bytes are filled with a per-tile marker so the round-trip can be checked
    /// byte-exactly. Returns the region bytes.
    fn build_region(tile_sizes: &[u64], tile_size_bytes: u32) -> Vec<u8> {
        let mut region = Vec::new();
        let last = tile_sizes.len() - 1;
        for (i, &size) in tile_sizes.iter().enumerate() {
            if i != last {
                region.extend(le_size_field(size - 1, tile_size_bytes));
            }
            // Distinct, deterministic data bytes per tile (so two tiles are not byte-identical).
            for b in 0..size {
                region.push(((i as u64 * 37 + b) & 0xFF) as u8);
            }
        }
        region
    }

    /// Parser-driven round-trip: build a conformant region for `tile_sizes`/`tile_size_bytes`, parse
    /// it to a known-good framing, slice out each tile's coded bytes, write the framing back, and
    /// assert (a) the rewritten bytes equal the original region and (b) a reparse is value-equal to
    /// the original framing with each tile's bytes byte-exact.
    fn assert_round_trips(tile_sizes: &[u64], tile_size_bytes: u32) {
        let region = build_region(tile_sizes, tile_size_bytes);
        let tg_end = (tile_sizes.len() - 1) as u32;
        let framing = parse_tile_group_framing(&region, 0, tg_end, tile_size_bytes, false);
        assert_eq!(framing.defect, None, "fixture region must be conformant");
        assert_eq!(framing.tiles.len(), tile_sizes.len());

        // Slice each tile's coded bytes out of the parsed region at its recorded offsets.
        let tile_data: Vec<&[u8]> = framing
            .tiles
            .iter()
            .map(|t| {
                let start = t.tile_data_offset as usize;
                let end = start + t.tile_size as usize;
                &region[start..end]
            })
            .collect();

        // A fresh writer starts byte-aligned; write the framing back.
        let mut writer = BitWriter::new();
        write_tile_group_payload(&mut writer, &framing, &tile_data, tile_size_bytes, false).unwrap();
        let bytes = writer.into_bytes();

        // (a) The rewritten bytes are byte-exact with the original region.
        assert_eq!(bytes, region, "rewritten region must be byte-exact");

        // (b) A reparse is value-equal to the original framing (offsets recompute identically since
        // both lay out sequentially from 0, sizes match, defect == None).
        let reparsed = parse_tile_group_framing(&bytes, 0, tg_end, tile_size_bytes, false);
        assert_eq!(reparsed, framing, "reparsed framing must equal the original");

        // Each tile's coded bytes round-trip byte-exact at the recomputed offsets.
        for (i, t) in reparsed.tiles.iter().enumerate() {
            let start = t.tile_data_offset as usize;
            let end = start + t.tile_size as usize;
            assert_eq!(&bytes[start..end], tile_data[i], "tile {i} bytes round-trip");
        }
    }

    #[test]
    fn round_trips_single_tile_no_size_field() {
        // A lone tile is the last tile: it writes its coded bytes only, no size field.
        assert_round_trips(&[5], 1);
    }

    #[test]
    fn round_trips_two_tiles_one_size_field() {
        // Two tiles: tile0 writes a size field + data, tile1 (last) writes data only.
        assert_round_trips(&[3, 4], 1);
    }

    #[test]
    fn round_trips_tile_size_bytes_one() {
        assert_round_trips(&[2, 3, 1], 1);
    }

    #[test]
    fn round_trips_tile_size_bytes_two() {
        // A tileSize requiring two size-field bytes (300 - 1 == 299 == 0x012B).
        assert_round_trips(&[300, 7], 2);
    }

    #[test]
    fn round_trips_tile_size_bytes_four() {
        assert_round_trips(&[10, 70000, 4], 4);
    }

    #[test]
    fn round_trips_size_field_spans_full_width() {
        // TileSizeBytes == 1, tile_size == 256 -> tile_size_minus_1 == 255 == 0xFF, the full le(1)
        // width; the last tile takes the remainder.
        assert_round_trips(&[256, 8], 1);
    }

    #[test]
    fn tile_num_and_offsets_are_parse_context_not_reproduced() {
        // tile_num / size_field_offset / tile_data_offset are parse-context: the writer IGNORES them
        // (it lays tiles sequentially from the region start) and the parser recomputes them from
        // tg_start + the region. A framing with NON-sequential tile_num / bogus offsets still writes
        // the same bytes and reparses to the sequential values the parser derives — read(write(x)) is
        // semantic on tile_size, not on these derived fields (the §5.20.1 analogue of the structure
        // writer ignoring header_bytes / payload_size).
        let region = build_region(&[3, 4], 1);
        let mut framing = parse_tile_group_framing(&region, 0, 1, 1, false);
        // Perturb the derived parse-context fields to values the parser would never produce.
        framing.tiles[0].tile_num = 5;
        framing.tiles[1].tile_num = 6;
        framing.tiles[0].size_field_offset = Some(999);
        framing.tiles[0].tile_data_offset = 777;

        let tile_data: &[&[u8]] = &[&region[1..4], &region[4..8]];
        let mut writer = BitWriter::new();
        write_tile_group_payload(&mut writer, &framing, tile_data, 1, false).unwrap();
        let bytes = writer.into_bytes();
        // The coded bytes still round-trip byte-exact (the perturbed parse-context is ignored).
        assert_eq!(bytes, region);
        // The reparse recomputes sequential tile_num / offsets from tg_start == 0 — NOT the perturbed
        // values — confirming they are parse-context, not reproduced syntax.
        let reparsed = parse_tile_group_framing(&bytes, 0, 1, 1, false);
        assert_eq!(reparsed.tiles[0].tile_num, 0);
        assert_eq!(reparsed.tiles[1].tile_num, 1);
        assert_eq!(reparsed.tiles[0].size_field_offset, Some(0));
        assert_eq!(reparsed.defect, None);
    }

    /// A minimal conformant single-tile framing + matching one-slice `tile_data`, for the reject
    /// tests that need a baseline to perturb. (tile_size 4, one tile.)
    fn baseline() -> (TileGroupFraming, Vec<u8>) {
        let region = build_region(&[4], 1);
        let framing = parse_tile_group_framing(&region, 0, 0, 1, false);
        assert_eq!(framing.defect, None);
        (framing, region)
    }

    #[test]
    fn rejects_framing_defect() {
        // A truncated size field is a provable §5.20.1 defect; the framing carries Some(defect).
        let region = vec![0x01, 0x02]; // 2 bytes, TileSizeBytes == 3 -> size field truncated.
        let framing = parse_tile_group_framing(&region, 0, 1, 3, false);
        assert!(framing.defect.is_some());
        let mut writer = BitWriter::new();
        let err = write_tile_group_payload(&mut writer, &framing, &[], 3, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "framing_defect"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_bridge_unframeable() {
        let (framing, region) = baseline();
        let mut writer = BitWriter::new();
        // is_bridge == true: rejected before laying out tiles (bridge tiles record tile_size == 0).
        let err =
            write_tile_group_payload(&mut writer, &framing, &[&region], 1, true).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "bridge_unframeable"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_empty_framing() {
        let framing = TileGroupFraming {
            tiles: Vec::new(),
            defect: None,
        };
        let mut writer = BitWriter::new();
        let err = write_tile_group_payload(&mut writer, &framing, &[], 1, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "empty_framing"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_tile_data_count() {
        let (framing, _region) = baseline();
        // One tile but zero data slices.
        let mut writer = BitWriter::new();
        let err = write_tile_group_payload(&mut writer, &framing, &[], 1, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "tile_data_count"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_tile_size_bytes_domain() {
        let (framing, region) = baseline();
        let mut writer = BitWriter::new();
        // TileSizeBytes == 0 (below the 1..=4 domain).
        let err =
            write_tile_group_payload(&mut writer, &framing, &[&region], 0, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "tile_size_bytes_domain"
            }
        );
        assert_eq!(writer.bit_len(), 0);
        // TileSizeBytes == 5 (above the domain).
        let mut writer = BitWriter::new();
        let err =
            write_tile_group_payload(&mut writer, &framing, &[&region], 5, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "tile_size_bytes_domain"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_zero_size_tile() {
        // Hand-construct a framing with a zero-size tile (the parser only ever produces this as a
        // last-tile defect; here we force the reject directly).
        let framing = TileGroupFraming {
            tiles: vec![crate::headers::tile_group::TileFraming {
                tile_num: 0,
                size_field_offset: None,
                tile_data_offset: 0,
                tile_size: 0,
            }],
            defect: None,
        };
        let empty: &[u8] = &[];
        let mut writer = BitWriter::new();
        let err = write_tile_group_payload(&mut writer, &framing, &[empty], 1, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "zero_size_tile"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_tile_data_len() {
        let (framing, _region) = baseline();
        // The lone tile has tile_size == 4, but the supplied slice is 3 bytes.
        let wrong: &[u8] = &[0, 1, 2];
        let mut writer = BitWriter::new();
        let err = write_tile_group_payload(&mut writer, &framing, &[wrong], 1, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "tile_data_len"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn rejects_tile_size_field_overflow() {
        // A non-last tile whose tile_size - 1 does not fit le(TileSizeBytes). TileSizeBytes == 1
        // holds 0..=255 (tile_size 1..=256); tile_size == 257 -> tile_size_minus_1 == 256 overflows.
        let big: Vec<u8> = vec![0u8; 257];
        let small: Vec<u8> = vec![0u8; 2];
        let framing = TileGroupFraming {
            tiles: vec![
                crate::headers::tile_group::TileFraming {
                    tile_num: 0,
                    size_field_offset: Some(0),
                    tile_data_offset: 1,
                    tile_size: 257,
                },
                crate::headers::tile_group::TileFraming {
                    tile_num: 1,
                    size_field_offset: None,
                    tile_data_offset: 258,
                    tile_size: 2,
                },
            ],
            defect: None,
        };
        let mut writer = BitWriter::new();
        let err = write_tile_group_payload(
            &mut writer,
            &framing,
            &[big.as_slice(), small.as_slice()],
            1,
            false,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalTileGroup {
                what: "tile_size_field_overflow"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }


    #[test]
    fn rejects_writer_not_byte_aligned() {
        let (framing, region) = baseline();
        let mut writer = BitWriter::new();
        // Write a stray bit so the writer is mid-byte before the payload write.
        writer.write_bit(1).unwrap();
        assert_eq!(writer.bit_len(), 1);
        let err =
            write_tile_group_payload(&mut writer, &framing, &[&region], 1, false).unwrap_err();
        assert_eq!(err, WriteError::WriterNotByteAligned);
        // Reject-before-write: the stray bit is the only thing written; nothing was appended.
        assert_eq!(writer.bit_len(), 1);
    }
}
