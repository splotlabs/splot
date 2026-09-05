// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::headers::frame::parse_tile_info;
    use crate::headers::sequence::{LevelIdx, Tier};
    use proptest::prelude::*;

    fn sb_flags(idx: u8) -> (bool, bool) {
        match idx % 3 {
            0 => (false, false),
            1 => (false, true),
            _ => (true, false),
        }
    }

    fn sb_size(idx: u8) -> SuperblockSize {
        match idx % 3 {
            0 => SuperblockSize::Block64x64,
            1 => SuperblockSize::Block128x128,
            _ => SuperblockSize::Block256x256,
        }
    }

    proptest! {
        /// Every parser-reachable `tile_info()` round-trips byte-exactly: parse arbitrary
        /// bits + gating, then re-emit and reparse to the identical model.
        #[test]
        fn tile_info_round_trips(
            data in proptest::collection::vec(any::<u8>(), 0..32),
            frame_width in 1u32..=8192,
            frame_height in 1u32..=8192,
            present in any::<bool>(),
            allow_change in any::<bool>(),
            has_params in any::<bool>(),
            uniform in any::<bool>(),
            seq_tile_cols_log2 in 0u8..=4,
            seq_tile_rows_log2 in 0u8..=4,
            seq_sb_cols in 0u32..=64,
            seq_sb_rows in 0u32..=64,
            sb in any::<u8>(),
            seq_sb in any::<u8>(),
            enable_avg_cdf in any::<bool>(),
            avg_cdf_type in 0u8..=3,
            tier_high in any::<bool>(),
            level in 0u8..=21,
            is_bridge in any::<bool>(),
            tip_frame_as_output in any::<bool>(),
            seq_sb_col_starts in proptest::collection::vec(0u32..=64, 0..=8),
            seq_sb_row_starts in proptest::collection::vec(0u32..=64, 0..=8),
        ) {
            let (use_256, use_128) = sb_flags(sb);
            let view = CoreSeqTileView {
                seq_tile_info_present_flag: present,
                allow_tile_info_change: allow_change,
                seq_tile_params: has_params.then_some(TileParams {
                    tile_cols: 1 << u32::from(seq_tile_cols_log2),
                    tile_rows: 1 << u32::from(seq_tile_rows_log2),
                    tile_cols_log2: seq_tile_cols_log2,
                    tile_rows_log2: seq_tile_rows_log2,
                    sb_cols: seq_sb_cols,
                    sb_rows: seq_sb_rows,
                    uniform_spacing: uniform,
                    covers_cols: true,
                    covers_rows: true,
                }),
                seq_sb_col_starts: std::sync::Arc::from(seq_sb_col_starts),
                seq_sb_row_starts: std::sync::Arc::from(seq_sb_row_starts),
                seq_sb_size: sb_size(seq_sb),
                use_256x256_superblock: use_256,
                use_128x128_superblock: use_128,
                enable_avg_cdf,
                avg_cdf_type,
                seq_tier: if tier_high { Tier::High } else { Tier::Main },
                seq_level_idx: LevelIdx::from_bits(level),
            };
            let frame = FrameSize::new(frame_width, frame_height);
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            if let Ok(info) =
                parse_tile_info(&mut reader, &view, frame, true, is_bridge, tip_frame_as_output)
            {
                let mut writer = BitWriter::new();
                write_tile_info(
                    &mut writer,
                    &info,
                    &view,
                    frame,
                    true,
                    is_bridge,
                    tip_frame_as_output,
                )
                .unwrap();
                let written = writer.into_bytes();
                let mut reparse = BitReader::new(&written, ByteOffset::new(0));
                let reparsed = parse_tile_info(
                    &mut reparse,
                    &view,
                    frame,
                    true,
                    is_bridge,
                    tip_frame_as_output,
                )
                .unwrap();
                prop_assert_eq!(reparsed, info);
            }
        }
    }
}
