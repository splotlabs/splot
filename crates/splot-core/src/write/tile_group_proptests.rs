// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::tile_group::parse_tile_group_structure;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    prop_compose! {
        fn arbitrary_valid()(
            cols_log2 in 0u8..=6,
            rows_log2 in 0u8..=6,
            flag in any::<bool>(),
            frac_start in 0u32..=u32::MAX,
            frac_end in 0u32..=u32::MAX,
        )(
            num_tiles in 1u32..=(1u32 << (u32::from(cols_log2) + u32::from(rows_log2))),
            cols_log2 in Just(cols_log2),
            rows_log2 in Just(rows_log2),
            flag in Just(flag),
            frac_start in Just(frac_start),
            frac_end in Just(frac_end),
        ) -> (TileGroupLayout, TileGroupStructure) {
            let layout = TileGroupLayout::new(num_tiles, 1, cols_log2, rows_log2);

            let range_written = num_tiles > 1 && flag;
            let structure = if range_written {
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
                    prop_assert_eq!(writer.bit_len(), 0);
                }
            }
        }
    }
}


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod payload_proptests {
    use super::*;
    use crate::headers::tile_group::{TileFraming, parse_tile_group_framing};
    use proptest::prelude::*;

    /// Builds a conformant `tile_group_payload()` region for `tile_sizes` (last is the remainder
    /// tile) using `tile_size_bytes`, with deterministic per-tile data bytes.
    fn build_region(tile_sizes: &[u64], tile_size_bytes: u32) -> Vec<u8> {
        let mut region = Vec::new();
        let last = tile_sizes.len() - 1;
        for (i, &size) in tile_sizes.iter().enumerate() {
            if i != last {
                for j in 0..tile_size_bytes {
                    region.push((((size - 1) >> (j * 8)) & 0xFF) as u8);
                }
            }
            for b in 0..size {
                region.push(((i as u64 * 37 + b) & 0xFF) as u8);
            }
        }
        region
    }

    proptest! {
        /// Every conformant framing round-trips: build a region from arbitrary in-range tile sizes,
        /// parse it, write the framing back, and assert the bytes are byte-exact and a reparse is
        /// value-equal to the original framing.
        #[test]
        fn payload_round_trips(
            tile_size_bytes in 1u32..=4,
            raw_sizes in proptest::collection::vec(1u64..=256, 1..=5),
        ) {
            let tile_sizes = raw_sizes;
            let region = build_region(&tile_sizes, tile_size_bytes);
            let tg_end = (tile_sizes.len() - 1) as u32;
            let framing = parse_tile_group_framing(&region, 0, tg_end, tile_size_bytes, false);
            prop_assert_eq!(framing.defect, None);
            prop_assert_eq!(framing.tiles.len(), tile_sizes.len());

            let tile_data: Vec<&[u8]> = framing
                .tiles
                .iter()
                .map(|t| {
                    let start = t.tile_data_offset as usize;
                    let end = start + t.tile_size as usize;
                    &region[start..end]
                })
                .collect();

            let mut writer = BitWriter::new();
            write_tile_group_payload(&mut writer, &framing, &tile_data, tile_size_bytes, false)
                .unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(&bytes, &region);

            let reparsed = parse_tile_group_framing(&bytes, 0, tg_end, tile_size_bytes, false);
            prop_assert_eq!(reparsed, framing);
        }

        /// The writer never panics on an arbitrary constructed framing — incl. out-of-domain
        /// TileSizeBytes, zero-size tiles, count/length mismatches, is_bridge, and a defect — and a
        /// rejected write leaves `bit_len() == 0` (reject-before-write). On success the emitted bytes
        /// reparse to the same framing (the writer only accepts reproducible models).
        #[test]
        fn writer_never_panics(
            sizes in proptest::collection::vec(0u64..=400, 0..=6),
            tile_size_bytes in 0u32..=6,
            is_bridge in any::<bool>(),
            has_defect in any::<bool>(),
            extra_data in any::<bool>(),
        ) {
            let mut tiles = Vec::new();
            let mut offset = 0u64;
            let last = sizes.len().saturating_sub(1);
            for (i, &size) in sizes.iter().enumerate() {
                let sf = if i == last {
                    None
                } else {
                    let o = offset;
                    offset += u64::from(tile_size_bytes.min(8));
                    Some(o)
                };
                tiles.push(TileFraming {
                    tile_num: i as u32,
                    size_field_offset: sf,
                    tile_data_offset: offset,
                    tile_size: size,
                });
                offset += size;
            }
            let defect = if has_defect {
                Some(crate::headers::tile_group::TileFramingDefect::ZeroSizeTile {
                    tile_num: 0,
                    tile_data_offset: 0,
                })
            } else {
                None
            };
            let framing = TileGroupFraming { tiles, defect };

            let mut owned: Vec<Vec<u8>> = sizes.iter().map(|&s| vec![0u8; s as usize]).collect();
            if extra_data {
                owned.push(vec![0u8]);
            }
            let tile_data: Vec<&[u8]> = owned.iter().map(Vec::as_slice).collect();

            let mut writer = BitWriter::new();
            match write_tile_group_payload(
                &mut writer,
                &framing,
                &tile_data,
                tile_size_bytes,
                is_bridge,
            ) {
                Ok(()) => {
                    let bytes = writer.into_bytes();
                    let tg_end = (framing.tiles.len() - 1) as u32;
                    let reparsed =
                        parse_tile_group_framing(&bytes, 0, tg_end, tile_size_bytes, false);
                    prop_assert_eq!(reparsed, framing);
                }
                Err(_) => {
                    prop_assert_eq!(writer.bit_len(), 0);
                }
            }
        }
    }
}
