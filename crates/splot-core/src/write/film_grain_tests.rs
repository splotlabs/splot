// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::film_grain::parse_film_grain;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    /// Parses a hand-built film-grain payload into a FilmGrainObu (asserts the parse succeeded), so
    /// the model is guaranteed parser-producible for the round-trip / reject tests.
    fn parse(bytes: &[u8]) -> FilmGrainObu {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_film_grain(&mut reader).unwrap()
    }

    /// Writes `fg`, reparses the emitted body, and asserts model equality (semantic round-trip; the
    /// writer re-derives minimal wire bit-widths, so byte-exactness is not asserted).
    fn round_trip(fg: &FilmGrainObu) {
        let mut writer = BitWriter::new();
        write_film_grain(&mut writer, fg).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
        let reparsed = parse_film_grain(&mut reader).unwrap();
        assert_eq!(&reparsed, fg);
    }

    /// Appends the smallest valid `film_grain_model()` for a non-monochrome, no-points config (no
    /// scaling points, `ar_coeff_lag == 0`).
    fn write_minimal_model(bits: &mut Bits) {
        bits.bit(0); // chroma_scaling_from_luma = 0
        bits.f(0, 4); // num_y_points = 0
        bits.f(0, 4); // num_cb_points = 0
        bits.f(0, 4); // num_cr_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0 -> numPosLuma = 0; no AR coeffs read
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range = 0 -> mc_identity inferred 0
        bits.bit(0); // film_grain_block_size
    }


    #[test]
    fn empty_update_flags_round_trips() {
        let mut bits = Bits::default();
        bits.f(0, 8);
        bits.uvlc(CHROMA_FORMAT_420);
        let fg = parse(&bits.into_bytes());
        assert!(fg.models.is_empty());
        round_trip(&fg);
    }

    #[test]
    fn monochrome_minimal_model_round_trips() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // slot 0
        bits.uvlc(CHROMA_FORMAT_400);
        bits.f(0, 4); // num_y_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range
        bits.bit(0); // film_grain_block_size
        let fg = parse(&bits.into_bytes());
        assert!(fg.monochrome);
        assert_eq!(fg.models.len(), 1);
        round_trip(&fg);
    }

    #[test]
    fn full_chroma_model_round_trips() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // slot 0
        bits.uvlc(CHROMA_FORMAT_420);
        bits.bit(0); // chroma_scaling_from_luma = 0 -> chroma points coded
        bits.f(2, 4); // num_y_points = 2
        bits.f(0, 3); // incr bits-1 = 0 -> bitsIncr = 1
        bits.f(0, 2); // scal bits-5 = 0 -> bitsScal = 5
        bits.f(1, 1); // point_y_value[0] = 1
        bits.f(3, 5); // point_y_scaling[0] = 3
        bits.f(1, 1); // increment -> value 2
        bits.f(4, 5); // scaling = 4
        bits.f(1, 4); // num_cb_points = 1
        bits.f(2, 3); // incr bits-1 = 2 -> bitsIncr = 3
        bits.f(1, 2); // scal bits-5 = 1 -> bitsScal = 6
        bits.f(5, 3); // point_cb_value[0] = 5
        bits.f(20, 6); // point_cb_scaling[0] = 20
        bits.f(1, 4); // num_cr_points = 1
        bits.f(0, 3); // bitsIncr = 1
        bits.f(0, 2); // bitsScal = 5
        bits.f(1, 1); // point_cr_value[0] = 1
        bits.f(7, 5); // point_cr_scaling[0] = 7
        bits.f(1, 2); // grain_scaling_minus_8
        bits.f(1, 2); // ar_coeff_lag = 1 -> numPosLuma = 4, numPosChroma = 5
        bits.f(0, 2); // bitsCoef = 5, midpoint 16
        bits.f(16, 5); // 0
        bits.f(17, 5); // 1
        bits.f(15, 5); // -1
        bits.f(31, 5); // 15
        bits.f(0, 2); // bitsCoef = 5
        for _ in 0..5 {
            bits.f(16, 5); // 0
        }
        bits.f(1, 2); // bitsCoef = 6, midpoint 32
        for _ in 0..5 {
            bits.f(32, 6); // 0
        }
        bits.f(2, 2); // ar_coeff_shift_minus_6
        bits.f(3, 2); // grain_scale_shift
        bits.f(100, 8);
        bits.f(200, 8);
        bits.f(300, 9);
        bits.f(50, 8);
        bits.f(60, 8);
        bits.f(511, 9);
        bits.bit(1); // overlap_flag
        bits.bit(0); // clip_to_restricted_range
        bits.bit(1); // film_grain_block_size
        let fg = parse(&bits.into_bytes());
        let model = &fg.models[0].model;
        assert_eq!(model.num_y_points, 2);
        assert_eq!(model.num_cb_points, 1);
        assert_eq!(model.num_cr_points, 1);
        assert_eq!(model.ar_coeffs_y, vec![0, 1, -1, 15]);
        assert_eq!(model.cb_offset, Some(300));
        assert_eq!(model.cr_offset, Some(511));
        round_trip(&fg);
    }

    #[test]
    fn chroma_scaling_from_luma_round_trips() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8); // slot 0
        bits.uvlc(CHROMA_FORMAT_444);
        bits.bit(1); // chroma_scaling_from_luma = 1
        bits.f(1, 4); // num_y_points = 1
        bits.f(0, 3); // bitsIncr = 1
        bits.f(0, 2); // bitsScal = 5
        bits.f(1, 1); // value 1
        bits.f(8, 5); // scaling 8
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(2, 2); // ar_coeff_lag = 2 -> numPosLuma = 12, numPosChroma = 13
        bits.f(0, 2); // bitsCoef = 5
        for _ in 0..12 {
            bits.f(16, 5);
        }
        bits.f(0, 2);
        for _ in 0..13 {
            bits.f(16, 5);
        }
        bits.f(0, 2);
        for _ in 0..13 {
            bits.f(16, 5);
        }
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range
        bits.bit(0); // film_grain_block_size
        let fg = parse(&bits.into_bytes());
        let model = &fg.models[0].model;
        assert!(model.chroma_scaling_from_luma);
        assert_eq!(model.num_cb_points, 0);
        assert_eq!(model.ar_coeffs_cb.len(), 13);
        assert_eq!(model.ar_coeffs_cr.len(), 13);
        round_trip(&fg);
    }

    #[test]
    fn clip_to_restricted_range_and_mc_identity_round_trips() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8);
        bits.uvlc(CHROMA_FORMAT_400); // monochrome
        bits.f(0, 4); // num_y_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(1); // overlap_flag
        bits.bit(1); // clip_to_restricted_range = 1 -> read mc_identity
        bits.bit(1); // mc_identity = 1
        bits.bit(1); // film_grain_block_size
        let fg = parse(&bits.into_bytes());
        let model = &fg.models[0].model;
        assert!(model.clip_to_restricted_range);
        assert!(model.mc_identity);
        round_trip(&fg);
    }

    #[test]
    fn multiple_slots_round_trip() {
        let mut bits = Bits::default();
        bits.f(0b0010_0010, 8); // slots 1 and 5
        bits.uvlc(CHROMA_FORMAT_420);
        write_minimal_model(&mut bits); // slot 1
        write_minimal_model(&mut bits); // slot 5
        let fg = parse(&bits.into_bytes());
        assert_eq!(fg.models.len(), 2);
        assert_eq!(fg.models[0].slot, 1);
        assert_eq!(fg.models[1].slot, 5);
        round_trip(&fg);
    }

    #[test]
    fn full_bit_width_range_round_trips() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8);
        bits.uvlc(CHROMA_FORMAT_420);
        bits.bit(0); // chroma_scaling_from_luma = 0
        bits.f(3, 4); // num_y_points = 3
        bits.f(7, 3); // incr bits-1 = 7 -> bitsIncr = 8
        bits.f(3, 2); // scal bits-5 = 3 -> bitsScal = 8
        bits.f(255, 8); // value 255
        bits.f(255, 8); // scaling 255
        bits.f(1, 8); // increment 1 -> value 256
        bits.f(0, 8); // scaling 0
        bits.f(200, 8); // increment 200 -> value 456
        bits.f(128, 8); // scaling 128
        bits.f(1, 4); // num_cb_points = 1
        bits.f(0, 3); // bitsIncr = 1
        bits.f(3, 2); // bitsScal = 8
        bits.f(1, 1); // value 1
        bits.f(255, 8); // scaling 255
        bits.f(0, 4); // num_cr_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(3, 2); // ar_coeff_lag = 3 -> numPosLuma = 24, numPosChroma = 25
        bits.f(3, 2); // bitsCoef = 8, midpoint 128
        bits.f(0, 8); // -128
        bits.f(255, 8); // 127
        for _ in 0..22 {
            bits.f(128, 8); // 0
        }
        bits.f(3, 2); // bitsCoef = 8
        for _ in 0..25 {
            bits.f(128, 8);
        }
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.f(255, 8);
        bits.f(0, 8);
        bits.f(0, 9);
        bits.bit(0); // overlap_flag
        bits.bit(0); // clip_to_restricted_range
        bits.bit(0); // film_grain_block_size
        let fg = parse(&bits.into_bytes());
        let model = &fg.models[0].model;
        assert_eq!(model.point_y[0].value, 255);
        assert_eq!(model.point_y[1].value, 256);
        assert_eq!(model.point_y[2].value, 456);
        assert_eq!(model.ar_coeffs_y[0], -128);
        assert_eq!(model.ar_coeffs_y[1], 127);
        round_trip(&fg);
    }

    #[test]
    fn out_of_range_chroma_idc_round_trips() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8);
        bits.uvlc(4);
        write_minimal_model(&mut bits);
        let fg = parse(&bits.into_bytes());
        assert_eq!(fg.chroma_idc, 4);
        assert!(!fg.monochrome);
        round_trip(&fg);
    }


    /// Builds a valid single-slot non-monochrome FilmGrainObu (one minimal model) for mutation.
    fn valid_single_slot() -> FilmGrainObu {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8);
        bits.uvlc(CHROMA_FORMAT_420);
        write_minimal_model(&mut bits);
        parse(&bits.into_bytes())
    }

    /// Asserts `write_film_grain(fg)` rejects with `NonCanonicalFilmGrain { what }` and writes nothing.
    fn assert_reject(fg: &FilmGrainObu, what: &str) {
        let mut writer = BitWriter::new();
        let err = write_film_grain(&mut writer, fg).unwrap_err();
        assert!(
            matches!(&err, WriteError::NonCanonicalFilmGrain { what: w } if *w == what),
            "expected NonCanonicalFilmGrain {{ {what} }}, got {err:?}"
        );
        assert_eq!(writer.bit_len(), 0, "reject left bits in the writer");
    }

    #[test]
    fn chroma_subsampling_mismatch_rejects() {
        let mut fg = valid_single_slot();
        fg.sub_x = !fg.sub_x; // disagrees with re-deriving from chroma_idc
        assert_reject(&fg, "chroma_subsampling");

        let mut fg2 = valid_single_slot();
        fg2.monochrome = true; // chroma_idc = 420 -> monochrome is false
        assert_reject(&fg2, "chroma_subsampling");
    }

    #[test]
    fn slot_vs_update_flags_mismatch_rejects() {
        let mut fg = valid_single_slot();
        fg.models[0].slot = 3;
        assert_reject(&fg, "slot_update_flags");

        let mut fg2 = valid_single_slot();
        fg2.models.clear();
        assert_reject(&fg2, "slot_update_flags");
    }

    #[test]
    fn non_monotonic_points_reject() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8);
        bits.uvlc(CHROMA_FORMAT_420);
        bits.bit(0);
        bits.f(2, 4); // num_y_points = 2
        bits.f(2, 3); // bitsIncr = 3
        bits.f(0, 2); // bitsScal = 5
        bits.f(5, 3); // value 5
        bits.f(3, 5); // scaling 3
        bits.f(1, 3); // increment -> value 6
        bits.f(4, 5); // scaling 4
        bits.f(0, 4); // num_cb_points = 0
        bits.f(0, 4); // num_cr_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0
        bits.f(0, 2); // bitsCoef = 5
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        let mut fg = parse(&bits.into_bytes());
        fg.models[0].model.point_y[1].value = 4;
        assert_reject(&fg, "non_monotonic_points");
    }

    #[test]
    fn num_y_points_len_mismatch_rejects() {
        let mut fg = valid_single_slot();
        fg.models[0].model.num_y_points = 0;
        fg.models[0].model.point_y.push(FilmGrainScalingPoint {
            value: 1,
            scaling: 2,
        });
        assert_reject(&fg, "num_y_points_len");
    }

    #[test]
    fn chroma_points_gate_reject_under_monochrome() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8);
        bits.uvlc(CHROMA_FORMAT_400);
        bits.f(0, 4); // num_y_points = 0
        bits.f(0, 2); // grain_scaling_minus_8
        bits.f(0, 2); // ar_coeff_lag = 0
        bits.f(0, 2); // ar_coeff_shift_minus_6
        bits.f(0, 2); // grain_scale_shift
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        let mut fg = parse(&bits.into_bytes());
        fg.models[0].model.num_cb_points = 1;
        fg.models[0].model.point_cb.push(FilmGrainScalingPoint {
            value: 1,
            scaling: 2,
        });
        assert_reject(&fg, "chroma_points_gate");
    }

    #[test]
    fn monochrome_chroma_scaling_reject() {
        let mut bits = Bits::default();
        bits.f(0b0000_0001, 8);
        bits.uvlc(CHROMA_FORMAT_400);
        bits.f(0, 4);
        bits.f(0, 2);
        bits.f(0, 2);
        bits.f(0, 2);
        bits.f(0, 2);
        bits.bit(0);
        bits.bit(0);
        bits.bit(0);
        let mut fg = parse(&bits.into_bytes());
        fg.models[0].model.chroma_scaling_from_luma = true;
        assert_reject(&fg, "monochrome_chroma_scaling");
    }

    #[test]
    fn ar_coeffs_y_len_mismatch_rejects() {
        let mut fg = valid_single_slot();
        fg.models[0].model.ar_coeffs_y.push(0);
        assert_reject(&fg, "ar_coeffs_y_len");
    }

    #[test]
    fn cb_mult_gate_mismatch_rejects() {
        let mut fg = valid_single_slot();
        fg.models[0].model.cb_mult = Some(7);
        fg.models[0].model.cb_luma_mult = Some(8);
        fg.models[0].model.cb_offset = Some(9);
        assert_reject(&fg, "cb_mult_gate");
    }

    #[test]
    fn mc_identity_without_clip_rejects() {
        let mut fg = valid_single_slot();
        fg.models[0].model.mc_identity = true;
        assert_reject(&fg, "mc_identity_clip");
    }

    #[test]
    fn point_increment_too_large_for_max_width_rejects() {
        let mut fg = valid_single_slot();
        fg.models[0].model.num_y_points = 1;
        fg.models[0].model.point_y = vec![FilmGrainScalingPoint {
            value: 256,
            scaling: 0,
        }];
        assert_reject(&fg, "point_increment_width");
    }

    #[test]
    fn unaligned_writer_rejects() {
        let mut writer = BitWriter::new();
        writer.write_bit(1).unwrap(); // leave the writer mid-byte
        let fg = valid_single_slot();
        let err = write_film_grain(&mut writer, &fg).unwrap_err();
        assert!(matches!(err, WriteError::WriterNotByteAligned));
        assert_eq!(writer.bit_len(), 1, "only the pre-existing stray bit remains");
    }
}
