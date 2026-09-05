// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::headers::frame::FrameHeaderParseStatus;
    use crate::headers::sequence::LevelIdx;
    use proptest::prelude::*;

    proptest! {
        /// Every parser-reachable intra frame header round-trips byte-exactly and semantically.
        #[test]
        fn intra_header_round_trips(
            type_idx in 0u8..3,
            first_pic in any::<bool>(),
            grain in any::<bool>(),
            short_refresh in any::<bool>(),
            monotonic in any::<bool>(),
            payload in proptest::collection::vec(any::<u8>(), 1..24),
        ) {
            let obu_type = match type_idx {
                0 => ObuType::ClosedLoopKey,
                1 => ObuType::OpenLoopKey,
                _ => ObuType::RegularTileGroup,
            };
            let seq = proptest_seq(grain, short_refresh, monotonic);
            let mut data = payload;
            data.extend_from_slice(&[0u8; 16]);

            let Ok(core) =
                parse_core_body_for_test(&data, obu_type, first_pic, &seq, None)
            else {
                return Ok(());
            };
            if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
                return Ok(());
            }

            let mut writer = BitWriter::new();
            write_frame_header_core(&mut writer, &core, &seq, None, first_pic).unwrap();
            let written = writer.into_bytes();

            let reparsed =
                parse_core_body_for_test(&written, obu_type, first_pic, &seq, None).unwrap();
            let mut a = reparsed.clone();
            let mut b = core.clone();
            a.consumed_bits = 0;
            b.consumed_bits = 0;
            prop_assert_eq!(a, b);

            let mut writer2 = BitWriter::new();
            write_frame_header_core(&mut writer2, &reparsed, &seq, None, first_pic).unwrap();
            prop_assert_eq!(writer2.into_bytes(), written);
        }

        /// The writer never panics for any core / sequence pair: a non-canonical or
        /// non-intra model returns `Err`, a canonical one returns `Ok`, never a panic.
        #[test]
        fn writer_never_panics(
            payload in proptest::collection::vec(any::<u8>(), 0..24),
            type_idx in any::<u8>(),
        ) {
            let obu_type = match type_idx % 4 {
                0 => ObuType::ClosedLoopKey,
                1 => ObuType::OpenLoopKey,
                2 => ObuType::RegularTileGroup,
                _ => ObuType::RegularSef,
            };
            let seq = proptest_seq(false, false, false);
            let mut data = payload;
            data.extend_from_slice(&[0u8; 16]);
            if let Ok(core) = parse_core_body_for_test(&data, obu_type, false, &seq, None) {
                let mut writer = BitWriter::new();
                let result = write_frame_header_core(&mut writer, &core, &seq, None, false);
                if result.is_err() {
                    prop_assert_eq!(writer.bit_len(), 0);
                }
            }
        }
    }

    /// A `base_seq()`-shaped view (OrderHintBits 4, NumRefFrames 8, screen content off,
    /// 12-bit dims, 4096x2304 max) with grain / short-refresh / monotonic toggles for the
    /// property generator.
    fn proptest_seq(grain: bool, short_refresh: bool, monotonic: bool) -> CoreSeqView {
        let mut seq = CoreSeqView::new_minimal_intra(4096, 2304).unwrap();
        seq.enable_short_refresh_frame_flags = short_refresh;
        seq.monotonic_output_order_flag = monotonic;
        seq.film_grain_params_present = Some(grain);
        seq.tile.seq_level_idx = LevelIdx::from_bits(0);
        seq
    }
}
