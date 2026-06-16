// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Property tests for the §5.19 tile-group structure writer: a constructed round-trip over an
// arbitrary VALID (layout, structure) pair (write -> reparse -> assert syntax fields), plus a
// "never panics" property over arbitrary field values (incl. huge num_tiles, out-of-range range,
// truncated outcome) asserting no panic and `bit_len() == 0` on Err.

// `include!`d into `crate::write::tile_group` so `super::*` resolves to its writer and helpers.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::tile_group::parse_tile_group_structure;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    // Builds an arbitrary VALID (layout, structure) the writer must accept and round-trip. The
    // layout uses AV2-legal log2 sizes (`0..=6` each) and a `num_tiles` consistent with the
    // signaled/inferred range path.
    prop_compose! {
        fn arbitrary_valid()(
            cols_log2 in 0u8..=6,
            rows_log2 in 0u8..=6,
            flag in any::<bool>(),
            frac_start in 0u32..=u32::MAX,
            frac_end in 0u32..=u32::MAX,
        )(
            // num_tiles in 1 ..= 2^(cols_log2 + rows_log2); a real layout has tiles within the
            // log2-derived grid, but the writer only needs num_tiles >= 1 and the range to be
            // consistent. Use the tileBits span as the tile-count ceiling.
            num_tiles in 1u32..=(1u32 << (u32::from(cols_log2) + u32::from(rows_log2))),
            cols_log2 in Just(cols_log2),
            rows_log2 in Just(rows_log2),
            flag in Just(flag),
            frac_start in Just(frac_start),
            frac_end in Just(frac_end),
        ) -> (TileGroupLayout, TileGroupStructure) {
            // Build a layout whose num_tiles is the chosen value; tile_cols/tile_rows only feed
            // the saturating product, so set cols = num_tiles, rows = 1.
            let layout = TileGroupLayout::new(num_tiles, 1, cols_log2, rows_log2);

            let range_written = num_tiles > 1 && flag;
            let structure = if range_written {
                // Pick tg_start <= tg_end, both in 0 .. num_tiles (the §6.18 in-range requirement;
                // num_tiles >= 2 here, and num_tiles <= 2^(cols_log2+rows_log2) == 2^tileBits, so the
                // values also fit f(tileBits)).
                let a = (u64::from(frac_start) % u64::from(num_tiles)) as u32;
                let b = (u64::from(frac_end) % u64::from(num_tiles)) as u32;
                let (tg_start, tg_end) = if a <= b { (a, b) } else { (b, a) };
                TileGroupStructure {
                    tile_start_and_end_present_flag: true,
                    tg_start,
                    tg_end,
                    outcome: TileGroupStructureOutcome::Complete,
                    header_bytes: None,
                    payload_size: None,
                }
            } else {
                // Inferred range: flag must be false (single tile can't signal it), range is the
                // default 0 .. num_tiles - 1.
                TileGroupStructure {
                    tile_start_and_end_present_flag: false,
                    tg_start: 0,
                    tg_end: num_tiles - 1,
                    outcome: TileGroupStructureOutcome::Complete,
                    header_bytes: None,
                    payload_size: None,
                }
            };
            (layout, structure)
        }
    }

    proptest! {
        /// Every valid (layout, structure) the writer accepts round-trips: the emitted bytes
        /// reparse to the same syntax fields with a Complete outcome.
        #[test]
        fn valid_structure_round_trips((layout, s) in arbitrary_valid()) {
            let mut writer = BitWriter::new();
            write_tile_group_structure(&mut writer, &s, layout).unwrap();
            let bytes = writer.into_bytes();
            let sz = bytes.len() as u64;
            let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
            let parsed = parse_tile_group_structure(&mut reader, layout, sz).unwrap();
            prop_assert_eq!(
                parsed.tile_start_and_end_present_flag,
                s.tile_start_and_end_present_flag
            );
            prop_assert_eq!(parsed.tg_start, s.tg_start);
            prop_assert_eq!(parsed.tg_end, s.tg_end);
            prop_assert_eq!(parsed.outcome, TileGroupStructureOutcome::Complete);
        }

        /// The writer never panics on an arbitrary constructed model — incl. huge num_tiles,
        /// out-of-range tg_start/tg_end, out-of-domain log2 sizes, and a Truncated outcome — and a
        /// rejected write leaves `bit_len() == 0` (reject-before-write).
        #[test]
        fn writer_never_panics(
            tile_cols in any::<u32>(),
            tile_rows in any::<u32>(),
            cols_log2 in any::<u8>(),
            rows_log2 in any::<u8>(),
            flag in any::<bool>(),
            tg_start in any::<u32>(),
            tg_end in any::<u32>(),
            truncated in any::<bool>(),
        ) {
            let layout = TileGroupLayout::new(tile_cols, tile_rows, cols_log2, rows_log2);
            let s = TileGroupStructure {
                tile_start_and_end_present_flag: flag,
                tg_start,
                tg_end,
                outcome: if truncated {
                    TileGroupStructureOutcome::Truncated
                } else {
                    TileGroupStructureOutcome::Complete
                },
                header_bytes: None,
                payload_size: None,
            };
            let mut writer = BitWriter::new();
            match write_tile_group_structure(&mut writer, &s, layout) {
                Ok(()) => {
                    // A successful write must reparse to the same syntax fields (the writer only
                    // accepts reproducible models).
                    let bytes = writer.into_bytes();
                    let sz = bytes.len() as u64;
                    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
                    let parsed =
                        parse_tile_group_structure(&mut reader, layout, sz).unwrap();
                    prop_assert_eq!(
                        parsed.tile_start_and_end_present_flag,
                        s.tile_start_and_end_present_flag
                    );
                    prop_assert_eq!(parsed.tg_start, s.tg_start);
                    prop_assert_eq!(parsed.tg_end, s.tg_end);
                    prop_assert_eq!(parsed.outcome, TileGroupStructureOutcome::Complete);
                }
                Err(_) => {
                    // Reject-before-write: no bit was emitted.
                    prop_assert_eq!(writer.bit_len(), 0);
                }
            }
        }
    }
}
