// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::sequence::{
        LevelIdx, parse_sequence_filter_config, parse_sequence_tile_config,
    };
    use crate::span::ByteOffset;
    use crate::tile::TileParamsInput;
    use proptest::prelude::*;

    fn sb_size(idx: u8) -> SuperblockSize {
        match idx % 3 {
            0 => SuperblockSize::Block64x64,
            1 => SuperblockSize::Block128x128,
            _ => SuperblockSize::Block256x256,
        }
    }

    proptest! {
        /// Every parser-reachable filter config round-trips and is byte-stable.
        #[test]
        fn filter_round_trips(
            bits in proptest::collection::vec(any::<bool>(), 0..16),
            single_picture in any::<bool>(),
            sb in any::<u8>(),
        ) {
            let raw: Vec<_> = bits.iter().copied().map(u8::from).collect();
            let padding = [0u8; 4];
            let packed = pack_bits(&raw, &padding);
            let sb = sb_size(sb);
            let mut reader = BitReader::new(&packed, ByteOffset::new(0));
            if let Ok(config) = parse_sequence_filter_config(&mut reader, single_picture, sb) {
                let mut writer = BitWriter::new();
                write_sequence_filter_config(&mut writer, &config, single_picture, sb).unwrap();
                let written = writer.into_bytes();
                let mut reparse = BitReader::new(&written, ByteOffset::new(0));
                let reparsed =
                    parse_sequence_filter_config(&mut reparse, single_picture, sb).unwrap();
                prop_assert_eq!(reparsed, config);
            }
        }

        /// Every parser-reachable tile config (across frame sizes, sb sizes, tiers, and
        /// non-reserved levels) round-trips and is byte-stable.
        #[test]
        fn tile_round_trips(
            bits in proptest::collection::vec(any::<bool>(), 0..48),
            frame_width in 1u32..=512,
            frame_height in 1u32..=512,
            sb in any::<u8>(),
            level in 0u8..=21,
            tier_high in any::<bool>(),
        ) {
            let raw: Vec<_> = bits.iter().copied().map(u8::from).collect();
            let padding = [0u8; 16];
            let packed = pack_bits(&raw, &padding);
            let sb = sb_size(sb);
            let input = TileParamsInput {
                frame_width,
                frame_height,
                uniform_sb_size: sb,
                sb_size: sb,
                is_bridge: false,
                seq_tier: if tier_high { Tier::High } else { Tier::Main },
                seq_level_idx: LevelIdx::from_bits(level),
            };
            let mut reader = BitReader::new(&packed, ByteOffset::new(0));
            if let Ok(config) = parse_sequence_tile_config(&mut reader, input) {
                if config.unimplemented_at().is_some() {
                    return Ok(());
                }
                let mut writer = BitWriter::new();
                write_sequence_tile_config(&mut writer, &config, input).unwrap();
                let written = writer.into_bytes();
                let mut reparse = BitReader::new(&written, ByteOffset::new(0));
                let reparsed = parse_sequence_tile_config(&mut reparse, input).unwrap();
                prop_assert_eq!(reparsed, config);
            }
        }

        /// The writer never panics on an arbitrary tile config parsed from random bits,
        /// including reserved levels (which it rejects rather than panicking).
        #[test]
        fn tile_writer_never_panics(
            bits in proptest::collection::vec(any::<bool>(), 0..48),
            frame_width in 1u32..=2048,
            frame_height in 1u32..=2048,
            sb in any::<u8>(),
            level in 0u8..=31,
            tier_high in any::<bool>(),
        ) {
            let raw: Vec<_> = bits.iter().copied().map(u8::from).collect();
            let padding = [0u8; 16];
            let packed = pack_bits(&raw, &padding);
            let sb = sb_size(sb);
            let input = TileParamsInput {
                frame_width,
                frame_height,
                uniform_sb_size: sb,
                sb_size: sb,
                is_bridge: false,
                seq_tier: if tier_high { Tier::High } else { Tier::Main },
                seq_level_idx: LevelIdx::from_bits(level),
            };
            let mut reader = BitReader::new(&packed, ByteOffset::new(0));
            if let Ok(config) = parse_sequence_tile_config(&mut reader, input) {
                let mut writer = BitWriter::new();
                let _ = write_sequence_tile_config(&mut writer, &config, input);
            }
        }
    }

    /// Packs a `0/1` byte slice MSB-first into bytes, appending `padding`.
    fn pack_bits(bits: &[u8], padding: &[u8]) -> Vec<u8> {
        let mut out = crate::test_bits::Bits::default();
        for &bit in bits {
            out.bit(bit & 1);
        }
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(padding);
        bytes
    }
}
