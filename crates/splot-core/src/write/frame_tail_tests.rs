// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit / reject tests for the §5.18.8.1 / §5.18.10.1 / §5.18.2 intra-tail writers. Round-trips
// drive the parser on hand-built bits, re-emit via the writer, and reparse; reject tests assert
// the typed error and that no bit was written (`bit_len() == 0`).

// `include!`d into `crate::write::frame_tail` so `super::*` resolves to its writers and helpers.

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{parse_film_grain_config, parse_intra_tail, read_tx_mode};
    use crate::span::ByteOffset;

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    /// An output intra frame with grain present and not single-picture (apply_grain coded).
    fn base_input() -> FrameTailInput {
        FrameTailInput {
            coded_lossless: false,
            film_grain_params_present: true,
            single_picture_header_flag: false,
            immediate_output_frame: true,
            implicit_output_frame: false,
        }
    }

    // ===== tx_mode (§ 5.18.8.1) =====

    #[test]
    fn tx_mode_lossless_only_4x4_writes_no_bit() {
        let mut writer = BitWriter::new();
        write_tx_mode(&mut writer, TxMode::Only4x4, true).unwrap();
        assert_eq!(writer.bit_len(), 0);
        // ONLY_4X4 reparses (lossless reads no bit).
        assert_eq!(read_tx_mode(&mut reader(&[]), true).unwrap(), TxMode::Only4x4);
    }

    #[test]
    fn tx_mode_largest_and_select_round_trip() {
        for mode in [TxMode::Largest, TxMode::Select] {
            let mut writer = BitWriter::new();
            write_tx_mode(&mut writer, mode, false).unwrap();
            let bytes = writer.into_bytes();
            assert_eq!(read_tx_mode(&mut reader(&bytes), false).unwrap(), mode);
        }
    }

    #[test]
    fn tx_mode_lossless_non_only_4x4_is_rejected() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_tx_mode(&mut writer, TxMode::Select, true),
            Err(WriteError::NonCanonicalFrameHeader { what: "tx_mode" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn tx_mode_non_lossless_only_4x4_is_rejected() {
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_tx_mode(&mut writer, TxMode::Only4x4, false),
            Err(WriteError::NonCanonicalFrameHeader { what: "tx_mode" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    // ===== film_grain_config (§ 5.18.10.1) =====

    fn fg_round_trip(fg: &FilmGrainConfig, input: &FrameTailInput) {
        let mut writer = BitWriter::new();
        write_film_grain_config(&mut writer, fg, input).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = parse_film_grain_config(&mut reader(&bytes), input).unwrap();
        assert_eq!(&reparsed, fg);
    }

    #[test]
    fn film_grain_gated_off_round_trips() {
        // Grain not present -> apply_grain inferred 0, no bit.
        let input = FrameTailInput {
            film_grain_params_present: false,
            ..base_input()
        };
        let fg = FilmGrainConfig {
            apply_grain: false,
            fgm_id: None,
            grain_seed: None,
        };
        fg_round_trip(&fg, &input);
        let mut writer = BitWriter::new();
        write_film_grain_config(&mut writer, &fg, &input).unwrap();
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn film_grain_not_output_frame_round_trips() {
        // Neither output flag set -> apply_grain inferred 0.
        let input = FrameTailInput {
            immediate_output_frame: false,
            implicit_output_frame: false,
            ..base_input()
        };
        let fg = FilmGrainConfig {
            apply_grain: false,
            fgm_id: None,
            grain_seed: None,
        };
        fg_round_trip(&fg, &input);
    }

    #[test]
    fn film_grain_single_picture_infers_apply_round_trips() {
        let input = FrameTailInput {
            single_picture_header_flag: true,
            ..base_input()
        };
        let fg = FilmGrainConfig {
            apply_grain: true,
            fgm_id: Some(5),
            grain_seed: Some(40000),
        };
        fg_round_trip(&fg, &input);
    }

    #[test]
    fn film_grain_coded_apply_true_round_trips() {
        let fg = FilmGrainConfig {
            apply_grain: true,
            fgm_id: Some(7),
            grain_seed: Some(65535),
        };
        fg_round_trip(&fg, &base_input());
    }

    #[test]
    fn film_grain_coded_apply_false_round_trips() {
        let fg = FilmGrainConfig {
            apply_grain: false,
            fgm_id: None,
            grain_seed: None,
        };
        fg_round_trip(&fg, &base_input());
    }

    #[test]
    fn film_grain_apply_true_when_gated_off_is_rejected() {
        let input = FrameTailInput {
            film_grain_params_present: false,
            ..base_input()
        };
        let fg = FilmGrainConfig {
            apply_grain: true,
            fgm_id: Some(0),
            grain_seed: Some(0),
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_film_grain_config(&mut writer, &fg, &input),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_apply_grain"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn film_grain_apply_false_when_single_picture_is_rejected() {
        let input = FrameTailInput {
            single_picture_header_flag: true,
            ..base_input()
        };
        let fg = FilmGrainConfig {
            apply_grain: false,
            fgm_id: None,
            grain_seed: None,
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_film_grain_config(&mut writer, &fg, &input),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_apply_grain"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn film_grain_apply_with_missing_field_is_rejected() {
        let fg = FilmGrainConfig {
            apply_grain: true,
            fgm_id: None, // must be Some when apply_grain
            grain_seed: Some(1),
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_film_grain_config(&mut writer, &fg, &base_input()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn film_grain_field_present_when_not_apply_is_rejected() {
        let fg = FilmGrainConfig {
            apply_grain: false,
            fgm_id: Some(0), // must be None when !apply_grain
            grain_seed: None,
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_film_grain_config(&mut writer, &fg, &base_input()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn film_grain_out_of_domain_fgm_id_is_rejected() {
        let fg = FilmGrainConfig {
            apply_grain: true,
            fgm_id: Some(8), // f(3) domain is 0..=7
            grain_seed: Some(0),
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_film_grain_config(&mut writer, &fg, &base_input()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_fgm_id"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    // ===== intra tail (§ 5.18.2) =====

    fn tail_round_trip(tail: &FrameHeaderTail, input: &FrameTailInput) {
        let mut writer = BitWriter::new();
        write_intra_tail(&mut writer, tail, input).unwrap();
        let bytes = writer.into_bytes();
        let reparsed = parse_intra_tail(&mut reader(&bytes), input).unwrap();
        assert_eq!(&reparsed, tail);
    }

    fn base_tail() -> FrameHeaderTail {
        FrameHeaderTail {
            tx_mode: TxMode::Select,
            reference_select: false,
            skip_mode_present: false,
            allow_bawp: false,
            allow_warpmv_mode: false,
            reduced_tx_set: 2,
            use_global_motion: false,
            film_grain: FilmGrainConfig {
                apply_grain: true,
                fgm_id: Some(3),
                grain_seed: Some(12345),
            },
        }
    }

    #[test]
    fn intra_tail_non_lossless_round_trips() {
        tail_round_trip(&base_tail(), &base_input());
    }

    #[test]
    fn intra_tail_lossless_round_trips() {
        let input = FrameTailInput {
            coded_lossless: true,
            ..base_input()
        };
        let tail = FrameHeaderTail {
            tx_mode: TxMode::Only4x4,
            ..base_tail()
        };
        tail_round_trip(&tail, &input);
    }

    #[test]
    fn intra_tail_grain_absent_round_trips() {
        let input = FrameTailInput {
            film_grain_params_present: false,
            ..base_input()
        };
        let tail = FrameHeaderTail {
            film_grain: FilmGrainConfig {
                apply_grain: false,
                fgm_id: None,
                grain_seed: None,
            },
            ..base_tail()
        };
        tail_round_trip(&tail, &input);
    }

    #[test]
    fn intra_tail_inference_true_is_rejected() {
        let tail = FrameHeaderTail {
            allow_warpmv_mode: true, // inferred false on the intra path
            ..base_tail()
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_intra_tail(&mut writer, &tail, &base_input()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "intra_tail_inference"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn intra_tail_out_of_domain_reduced_tx_set_is_rejected() {
        let tail = FrameHeaderTail {
            reduced_tx_set: 4, // f(2) domain is 0..=3
            ..base_tail()
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_intra_tail(&mut writer, &tail, &base_input()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "reduced_tx_set"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn intra_tail_tx_mode_mismatch_is_rejected_before_any_bit() {
        // Non-lossless model carrying ONLY_4X4 — the intra-tail check must reject before any bit.
        let tail = FrameHeaderTail {
            tx_mode: TxMode::Only4x4,
            ..base_tail()
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_intra_tail(&mut writer, &tail, &base_input()),
            Err(WriteError::NonCanonicalFrameHeader { what: "tx_mode" })
        ));
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn intra_tail_bad_film_grain_rejects_before_any_bit() {
        // The tx_mode and reduced_tx_set are valid, but the film_grain model is not (apply_grain
        // with a missing fgm_id). check_intra_tail_encodable must reject the whole tail BEFORE
        // write_intra_tail emits the tx_mode / reduced_tx_set bits — otherwise this would be a
        // partial buffer (bit_len() > 0 on Err).
        let tail = FrameHeaderTail {
            film_grain: FilmGrainConfig {
                apply_grain: true,
                fgm_id: None,
                grain_seed: Some(1),
            },
            ..base_tail()
        };
        let mut writer = BitWriter::new();
        assert!(matches!(
            write_intra_tail(&mut writer, &tail, &base_input()),
            Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_fields"
            })
        ));
        assert_eq!(writer.bit_len(), 0);
    }
}
