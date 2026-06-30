// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>


#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::quantizer_matrix::parse_quantizer_matrix;
    use crate::span::ByteOffset;
    use crate::test_bits::Bits;

    fn parse(bytes: &[u8]) -> QuantizerMatrixObu {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_quantizer_matrix(&mut reader).unwrap()
    }

    fn write_model(model: &QuantizerMatrixObu) -> Vec<u8> {
        let mut writer = BitWriter::new();
        write_quantizer_matrix(&mut writer, model).unwrap();
        writer.into_bytes()
    }

    /// Parses a hand-built body, writes the model back, and asserts the re-emission reparses to the
    /// same model (semantic round-trip — the canonicalizing writer does not guarantee byte-exactness).
    fn round_trip(body: &[u8]) -> QuantizerMatrixObu {
        let model = parse(body);
        let bytes = write_model(&model);
        assert_eq!(parse(&bytes), model, "semantic round-trip");
        model
    }

    /// Like [`round_trip`] but also asserts byte-exactness (valid only when `body` is already in the
    /// writer's canonical long form).
    fn round_trip_byte_exact(body: &[u8]) -> QuantizerMatrixObu {
        let model = round_trip(body);
        assert_eq!(&write_model(&model), body, "byte-exact (already long form)");
        model
    }

    fn write_err(model: &QuantizerMatrixObu) -> WriteError {
        let mut writer = BitWriter::new();
        let err = write_quantizer_matrix(&mut writer, model).unwrap_err();
        assert_eq!(writer.bit_len(), 0, "a rejected model writes no bit");
        err
    }

    fn what(err: &WriteError) -> &'static str {
        match err {
            WriteError::NonCanonicalQuantizationMatrix { what } => what,
            other => panic!("expected NonCanonicalQuantizationMatrix, got {other:?}"),
        }
    }

    /// A non-default level-0 prefix: `qm_bit_map` = level 0 only, `qm_chroma_info_present_flag` =
    /// `chroma`, `qm_is_default_flag` = 0.
    fn user_defined_level0_prefix(chroma: bool) -> Bits {
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(u8::from(chroma)); // qm_chroma_info_present_flag
        bits.bit(0); // qm_is_default_flag = 0
        bits
    }

    /// Appends one long-form plane (skip flags 0, then `w*h` `svlc(delta)` deltas) for transform `t`
    /// at `plane_idx` — the exact shape the writer emits.
    fn long_form_plane(bits: &mut Bits, t: usize, plane_idx: usize, delta: i32, w: usize, h: usize) {
        if plane_idx > 0 {
            bits.bit(0); // qm_copy_from_previous_plane = 0
        }
        if t == 0 || t == 2 {
            bits.bit(0); // qm_8x8_is_symmetric / qm_4x8_is_transpose_of_8x4 = 0
        }
        bits.deltas(delta, w * h);
    }

    /// A full long-form single-plane user-defined level-0 body, every cell `svlc(delta)`.
    fn long_form_level0_body(delta: i32) -> Vec<u8> {
        let mut bits = user_defined_level0_prefix(false);
        long_form_plane(&mut bits, 0, 0, delta, 8, 8);
        long_form_plane(&mut bits, 1, 0, delta, 8, 4);
        long_form_plane(&mut bits, 2, 0, delta, 4, 8);
        bits.into_bytes()
    }

    /// Appends one long-form plane whose every coefficient is the constant `value` (`1..=159`): the
    /// first scan cell carries `svlc(value - 32)` (from the initial quant 32), the rest `svlc(0)`.
    /// This is exactly the writer's canonical encoding of a constant plane, so it stays byte-exact and
    /// — for `value < 32` — exercises a negative `quant_delta` without tripping the `quant2 == 0`
    /// repeat sentinel.
    fn long_form_constant_plane(bits: &mut Bits, t: usize, plane_idx: usize, value: u8, w: usize, h: usize) {
        if plane_idx > 0 {
            bits.bit(0);
        }
        if t == 0 || t == 2 {
            bits.bit(0);
        }
        bits.svlc(i32::from(value) - 32); // scan cell 0: quant 32 -> value
        bits.deltas(0, w * h - 1); // remaining cells stay at value
    }


    #[test]
    fn diagonal_scan_matches_av2_oracle_order() {
        assert_eq!(
            diagonal_scan_2d(8, 8)[..10],
            [0, 8, 1, 16, 9, 2, 24, 17, 10, 3]
        );
        assert_eq!(diagonal_scan_2d(8, 4)[..6], [0, 8, 1, 16, 9, 2]);
        assert_eq!(
            diagonal_scan_2d(4, 8)[..10],
            [0, 4, 1, 8, 5, 2, 12, 9, 6, 3]
        );
    }


    #[test]
    fn reset_obu_round_trips_byte_exact() {
        let mut bits = Bits::default();
        bits.f(0, 15); // qm_bit_map == 0 (reset)
        bits.bit(0); // qm_chroma_info_present_flag
        let qm = round_trip_byte_exact(&bits.into_bytes());
        assert!(qm.is_reset());
        assert!(qm.levels.is_empty());
    }

    #[test]
    fn default_level_round_trips_byte_exact() {
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(0); // 1 plane
        bits.bit(1); // qm_is_default_flag = 1
        let qm = round_trip_byte_exact(&bits.into_bytes());
        assert!(qm.levels[0].is_default);
        assert!(qm.levels[0].matrices.is_none());
    }

    #[test]
    fn long_form_user_defined_round_trips_byte_exact() {
        let qm = round_trip_byte_exact(&long_form_level0_body(1));
        let matrices = qm.levels[0].matrices.as_ref().unwrap();
        assert_eq!(matrices.len(), 3);
        let v = &matrices[0].planes[0].values;
        assert_eq!(v[diagonal_scan_2d(8, 8)[0]], 33);
        assert_eq!(v[diagonal_scan_2d(8, 8)[63]], 96);
    }

    #[test]
    fn long_form_sub32_constant_round_trips_byte_exact() {
        let mut bits = user_defined_level0_prefix(false);
        long_form_constant_plane(&mut bits, 0, 0, 31, 8, 8);
        long_form_constant_plane(&mut bits, 1, 0, 31, 8, 4);
        long_form_constant_plane(&mut bits, 2, 0, 31, 4, 8);
        let qm = round_trip_byte_exact(&bits.into_bytes());
        let v = &qm.levels[0].matrices.as_ref().unwrap()[0].planes[0].values;
        assert!(v.iter().all(|&c| c == 31));
    }

    #[test]
    fn three_plane_long_form_round_trips_byte_exact() {
        let mut bits = user_defined_level0_prefix(true); // 3 planes
        for t in 0..3 {
            let (w, h) = match t {
                0 => (8, 8),
                1 => (8, 4),
                _ => (4, 8),
            };
            for plane_idx in 0..3 {
                long_form_plane(&mut bits, t, plane_idx, 1, w, h);
            }
        }
        let qm = round_trip_byte_exact(&bits.into_bytes());
        assert_eq!(qm.num_planes, 3);
        assert_eq!(qm.levels[0].matrices.as_ref().unwrap()[0].planes.len(), 3);
    }

    #[test]
    fn multi_level_round_trips() {
        let mut bits = Bits::default();
        bits.f(0b101, 15); // qm_bit_map: levels 0 and 2
        bits.bit(0); // 1 plane
        bits.bit(1); // level 0: qm_is_default_flag = 1
        bits.bit(0); // level 2: qm_is_default_flag = 0
        long_form_plane(&mut bits, 0, 0, 1, 8, 8);
        long_form_plane(&mut bits, 1, 0, 1, 8, 4);
        long_form_plane(&mut bits, 2, 0, 1, 4, 8);
        let qm = round_trip_byte_exact(&bits.into_bytes());
        assert_eq!(qm.levels.len(), 2);
        assert_eq!(qm.levels[0].level, 0);
        assert!(qm.levels[0].is_default);
        assert_eq!(qm.levels[1].level, 2);
        assert!(!qm.levels[1].is_default);
    }

    #[test]
    fn symmetric_8x8_canonicalizes_and_round_trips() {
        let mut bits = user_defined_level0_prefix(false);
        bits.bit(1); // qm_8x8_is_symmetric -> only lower-triangle (col<=row) deltas
        let lower_tri = (0..8).map(|r| r + 1).sum::<usize>(); // 36 cells
        bits.deltas(0, lower_tri);
        long_form_plane(&mut bits, 1, 0, 0, 8, 4);
        bits.bit(1); // t==2: qm_4x8_is_transpose_of_8x4
        let qm = round_trip(&bits.into_bytes()); // semantic only (canonicalized)
        let m = qm.levels[0].matrices.as_ref().unwrap();
        assert!(m[0].planes[0].values.iter().all(|&v| v == 32)); // symmetric flat -> all 32
        assert!(m[2].planes[0].values.iter().all(|&v| v == 32)); // transpose of flat 8x4 -> all 32
    }

    #[test]
    fn transpose_4x8_canonicalizes_and_round_trips() {
        let mut bits = user_defined_level0_prefix(false);
        long_form_plane(&mut bits, 0, 0, 1, 8, 8);
        long_form_plane(&mut bits, 1, 0, 1, 8, 4);
        bits.bit(1); // t==2: qm_4x8_is_transpose_of_8x4
        let qm = round_trip(&bits.into_bytes());
        let m = qm.levels[0].matrices.as_ref().unwrap();
        let tx8x4 = &m[1].planes[0].values;
        let tx4x8 = &m[2].planes[0].values;
        for i in 0..8 {
            for j in 0..4 {
                assert_eq!(tx4x8[i * 4 + j], tx8x4[j * 8 + i]);
            }
        }
    }

    #[test]
    fn copy_previous_plane_canonicalizes_and_round_trips() {
        let mut bits = user_defined_level0_prefix(true); // 3 planes
        long_form_plane(&mut bits, 0, 0, 1, 8, 8);
        bits.bit(1); // plane 1: qm_copy_from_previous_plane
        bits.bit(1); // plane 2: qm_copy_from_previous_plane
        long_form_plane(&mut bits, 1, 0, 1, 8, 4);
        bits.bit(1);
        bits.bit(1);
        long_form_plane(&mut bits, 2, 0, 1, 4, 8);
        bits.bit(1);
        bits.bit(1);
        let qm = round_trip(&bits.into_bytes());
        let m = qm.levels[0].matrices.as_ref().unwrap();
        assert_eq!(m[0].planes[1].values, m[0].planes[0].values);
        assert_eq!(m[0].planes[2].values, m[0].planes[0].values);
    }

    #[test]
    fn coefficient_repeat_canonicalizes_and_round_trips() {
        let mut bits = user_defined_level0_prefix(false);
        bits.bit(0); // qm_8x8_is_symmetric = 0
        bits.svlc(8); // 32 + 8 = 40
        bits.svlc(-40); // (40 - 40) & 255 = 0 -> repeat; remaining cells keep 40
        long_form_plane(&mut bits, 1, 0, 0, 8, 4);
        bits.bit(0); // t==2: not a transpose
        bits.deltas(0, 32);
        let qm = round_trip(&bits.into_bytes());
        let v = &qm.levels[0].matrices.as_ref().unwrap()[0].planes[0].values;
        assert!(v.iter().all(|&c| c == 40), "coefficient repeat fills 40");
    }


    #[test]
    fn rejects_num_planes_mismatch() {
        let mut bits = Bits::default();
        bits.f(0, 15);
        bits.bit(0); // chroma absent -> num_planes 1
        let mut qm = parse(&bits.into_bytes());
        qm.num_planes = 3; // disagrees with chroma_info_present == false
        assert_eq!(what(&write_err(&qm)), "num_planes");
    }

    #[test]
    fn rejects_level_count_mismatch() {
        let qm_body = long_form_level0_body(1);
        let mut qm = parse(&qm_body);
        qm.qm_bit_map = 0b11; // two set bits now, but only one level
        assert_eq!(what(&write_err(&qm)), "level_count");
    }

    #[test]
    fn rejects_level_index_mismatch() {
        let mut qm = parse(&long_form_level0_body(1));
        qm.levels[0].level = 5; // != the only set bit (0)
        assert_eq!(what(&write_err(&qm)), "level_index");
    }

    #[test]
    fn rejects_is_default_gate() {
        let mut bits = Bits::default();
        bits.f(1, 15);
        bits.bit(0);
        bits.bit(1); // default level -> matrices None
        let mut qm = parse(&bits.into_bytes());
        qm.levels[0].is_default = false; // now (false, None) -> parser-unproducible
        assert_eq!(what(&write_err(&qm)), "is_default_gate");
    }

    #[test]
    fn rejects_transform_count() {
        let mut qm = parse(&long_form_level0_body(1));
        qm.levels[0].matrices.as_mut().unwrap().truncate(2);
        assert_eq!(what(&write_err(&qm)), "transform_count");
    }

    #[test]
    fn rejects_transform_order() {
        let mut qm = parse(&long_form_level0_body(1));
        qm.levels[0].matrices.as_mut().unwrap()[0].transform = FundamentalQmTransform::Tx8x4;
        assert_eq!(what(&write_err(&qm)), "transform_order");
    }

    #[test]
    fn rejects_plane_count() {
        let mut qm = parse(&long_form_level0_body(1));
        let plane = qm.levels[0].matrices.as_ref().unwrap()[0].planes[0].clone();
        qm.levels[0].matrices.as_mut().unwrap()[0].planes.push(plane); // len 2 != num_planes 1
        assert_eq!(what(&write_err(&qm)), "plane_count");
    }

    #[test]
    fn rejects_plane_dimensions() {
        let mut qm = parse(&long_form_level0_body(1));
        qm.levels[0].matrices.as_mut().unwrap()[0].planes[0].width = 4; // TX_8X8 must be 8 wide
        assert_eq!(what(&write_err(&qm)), "plane_dimensions");
    }

    #[test]
    fn rejects_plane_value_count() {
        let mut qm = parse(&long_form_level0_body(1));
        qm.levels[0].matrices.as_mut().unwrap()[0].planes[0]
            .values
            .push(1); // 65 != 64
        assert_eq!(what(&write_err(&qm)), "plane_value_count");
    }

    #[test]
    fn rejects_zero_coefficient() {
        let mut qm = parse(&long_form_level0_body(1));
        qm.levels[0].matrices.as_mut().unwrap()[0].planes[0].values[0] = 0; // unrepresentable
        assert_eq!(what(&write_err(&qm)), "coefficient_zero");
    }
}
