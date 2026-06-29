// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{parse_film_grain_config, parse_intra_tail, read_tx_mode};
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn pack(bits: &[bool]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (i, b) in chunk.iter().enumerate() {
                byte |= u8::from(*b) << (7 - i);
            }
            out.push(byte);
        }
        out.extend_from_slice(&[0u8; 8]); // pad so the parser never hits EOF mid-field
        out
    }

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    prop_compose! {
        fn arbitrary_input()(
            coded_lossless in any::<bool>(),
            film_grain_params_present in any::<bool>(),
            single_picture_header_flag in any::<bool>(),
            immediate_output_frame in any::<bool>(),
            implicit_output_frame in any::<bool>(),
        ) -> FrameTailInput {
            FrameTailInput {
                coded_lossless,
                film_grain_params_present,
                single_picture_header_flag,
                immediate_output_frame,
                implicit_output_frame,
            }
        }
    }

    fn arbitrary_film_grain() -> impl Strategy<Value = FilmGrainConfig> {
        (any::<bool>(), 0u8..16, any::<u16>()).prop_map(|(apply_grain, fgm_id, grain_seed)| {
            FilmGrainConfig {
                apply_grain,
                fgm_id: if apply_grain { Some(fgm_id) } else { None },
                grain_seed: if apply_grain { Some(grain_seed) } else { None },
            }
        })
    }

    proptest! {
        /// Every parser-reachable read_tx_mode round-trips.
        #[test]
        fn tx_mode_round_trips(coded_lossless in any::<bool>(), bit in any::<bool>()) {
            let packed = pack(&[bit]);
            if let Ok(mode) = read_tx_mode(&mut reader(&packed), coded_lossless) {
                let mut writer = BitWriter::new();
                write_tx_mode(&mut writer, mode, coded_lossless).unwrap();
                let written = writer.into_bytes();
                let reparsed = read_tx_mode(&mut reader(&written), coded_lossless).unwrap();
                prop_assert_eq!(reparsed, mode);
            }
        }

        /// Every parser-reachable film_grain_config round-trips.
        #[test]
        fn film_grain_round_trips(
            input in arbitrary_input(),
            bits in proptest::collection::vec(any::<bool>(), 0..32),
        ) {
            let packed = pack(&bits);
            if let Ok(fg) = parse_film_grain_config(&mut reader(&packed), &input) {
                let mut writer = BitWriter::new();
                write_film_grain_config(&mut writer, &fg, &input).unwrap();
                let written = writer.into_bytes();
                let reparsed = parse_film_grain_config(&mut reader(&written), &input).unwrap();
                prop_assert_eq!(reparsed, fg);
            }
        }

        /// Every parser-reachable intra tail round-trips.
        #[test]
        fn intra_tail_round_trips(
            input in arbitrary_input(),
            bits in proptest::collection::vec(any::<bool>(), 0..32),
        ) {
            let packed = pack(&bits);
            if let Ok(tail) = parse_intra_tail(&mut reader(&packed), &input) {
                let mut writer = BitWriter::new();
                write_intra_tail(&mut writer, &tail, &input).unwrap();
                let written = writer.into_bytes();
                let reparsed = parse_intra_tail(&mut reader(&written), &input).unwrap();
                prop_assert_eq!(reparsed, tail);
            }
        }

        /// The film-grain writer never panics on an arbitrary (possibly invalid) model + gating,
        /// and on Err leaves the writer empty.
        #[test]
        fn film_grain_writer_never_panics_on_constructed_models(
            input in arbitrary_input(),
            fg in arbitrary_film_grain(),
        ) {
            let mut writer = BitWriter::new();
            if write_film_grain_config(&mut writer, &fg, &input).is_err() {
                prop_assert_eq!(writer.bit_len(), 0);
            }
        }
    }
}
