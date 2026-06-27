// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

// Unit, byte-exact, and rejection tests for the § 5.18.6 / § 5.18.7.8 / § 5.18.2 frame
// quantization writers.

// `include!`d into `crate::write::frame_quant` so `super::*` resolves to its writers and
// private helpers (the property tests live in the sibling `frame_quant_proptests.rs`).

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
mod tests {
    use super::*;
    use crate::bitio::BitReader;
    use crate::headers::frame::{
        QmSetLevels, parse_delta_q_params, parse_lossless_info, parse_quantization_params,
        parse_setup_qm_params, read_delta_q,
    };
    use crate::segment::SegmentFeature;
    use crate::span::ByteOffset;
    use crate::test_support::{base_quant, seg_params};

    use crate::test_bits::Bits;

    fn quant_params(base_q_idx: u32) -> QuantizationParams {
        QuantizationParams {
            base_q_idx,
            delta_q_y_dc: 0,
            delta_q_u_dc: 0,
            delta_q_u_ac: 0,
            delta_q_v_dc: 0,
            delta_q_v_ac: 0,
            diff_uv_delta: false,
        }
    }

    fn qm_disabled() -> SetupQmParams {
        SetupQmParams {
            using_qmatrix: false,
            pic_qm_num_minus_1: 0,
            levels: [QmSetLevels::default(); MAX_PIC_QM_NUM],
        }
    }

    fn no_delta_q() -> DeltaQParams {
        DeltaQParams {
            delta_q_present: false,
            delta_q_res: 0,
        }
    }

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    // ----- write_read_delta_q (§ 5.18.6.3) -----

    fn roundtrip_delta_q(value: i32) {
        let mut writer = BitWriter::new();
        write_read_delta_q(&mut writer, value).unwrap();
        let bytes = writer.into_bytes();
        let parsed = read_delta_q(&mut reader(&bytes)).unwrap();
        assert_eq!(parsed, value, "read_delta_q round-trip for {value}");
    }

    #[test]
    fn read_delta_q_round_trips_full_domain() {
        for value in DELTA_Q_MIN..=DELTA_Q_MAX {
            roundtrip_delta_q(value);
        }
    }

    #[test]
    fn read_delta_q_zero_is_one_bit_canonical() {
        // Canonicalization 1: delta_q == 0 -> delta_coded f(1) = 0, no su.
        let mut writer = BitWriter::new();
        write_read_delta_q(&mut writer, 0).unwrap();
        assert_eq!(writer.bit_len(), 1);
        assert_eq!(writer.into_bytes(), vec![0b0000_0000]);
    }

    #[test]
    fn read_delta_q_nonzero_is_eight_bits() {
        // delta_coded f(1) = 1 then su(7).
        let mut writer = BitWriter::new();
        write_read_delta_q(&mut writer, 5).unwrap();
        assert_eq!(writer.bit_len(), 8);
        roundtrip_delta_q(5);
        roundtrip_delta_q(-64);
        roundtrip_delta_q(63);
    }

    #[test]
    fn read_delta_q_out_of_su_domain_rejected() {
        for bad in [64, -65, i32::MAX, i32::MIN] {
            let mut writer = BitWriter::new();
            let err = write_read_delta_q(&mut writer, bad).unwrap_err();
            assert_eq!(
                err,
                WriteError::ValueOutOfRange {
                    descriptor: "su",
                    value: i64::from(bad),
                }
            );
            assert_eq!(writer.bit_len(), 0);
        }
    }

    // ----- write_quantization_params (§ 5.18.6.1) -----

    /// Parse the hand-built bits with `parse_quantization_params`, then write the parsed
    /// model back and assert byte-exact round-trip (canonical fixtures).
    fn roundtrip_quant(bits: Bits, quant: &CoreSeqQuantView, tip: bool) {
        let data = bits.into_bytes();
        let mut rd = reader(&data);
        let params = parse_quantization_params(&mut rd, quant, tip).unwrap();
        let consumed = rd.consumed_bits();
        let mut writer = BitWriter::new();
        write_quantization_params(&mut writer, &params, quant, tip).unwrap();
        assert_eq!(writer.bit_len(), consumed, "bit length matches parser");
        let bytes = writer.into_bytes();
        let reparsed = parse_quantization_params(&mut reader(&bytes), quant, tip).unwrap();
        assert_eq!(reparsed, params);
    }

    #[test]
    fn quant_base_only_round_trips() {
        let mut bits = Bits::default();
        bits.f(100, 8);
        roundtrip_quant(bits, &base_quant(), false);
    }

    #[test]
    fn quant_9_bit_base_round_trips() {
        let quant = CoreSeqQuantView {
            bit_depth: 10,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(300, 9);
        roundtrip_quant(bits, &quant, false);
    }

    #[test]
    fn quant_y_dc_delta_round_trips() {
        let quant = CoreSeqQuantView {
            y_dc_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(50, 8);
        bits.bit(1);
        bits.su(-3, 7);
        roundtrip_quant(bits, &quant, false);
    }

    #[test]
    fn quant_shared_uv_delta_round_trips() {
        let quant = CoreSeqQuantView {
            uv_dc_delta_q_enabled: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(40, 8);
        bits.bit(1);
        bits.su(2, 7); // DeltaQUDc = 2
        bits.bit(1);
        bits.su(-5, 7); // DeltaQUAc = -5
        roundtrip_quant(bits, &quant, false);
    }

    #[test]
    fn quant_separate_uv_with_diff_round_trips() {
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            uv_dc_delta_q_enabled: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(40, 8);
        bits.bit(1); // diff_uv_delta
        bits.bit(1);
        bits.su(1, 7); // DeltaQUDc
        bits.bit(0); // DeltaQUAc not coded -> 0
        bits.bit(1);
        bits.su(-2, 7); // DeltaQVDc
        bits.bit(0); // DeltaQVAc not coded -> 0
        roundtrip_quant(bits, &quant, false);
    }

    #[test]
    fn quant_equal_ac_dc_q_round_trips() {
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            equal_ac_dc_q: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(30, 8);
        bits.bit(1); // diff_uv_delta
        bits.bit(1);
        bits.su(-4, 7); // DeltaQUAc = -4 -> DeltaQUDc = -4
        bits.bit(1);
        bits.su(6, 7); // DeltaQVAc = 6 -> DeltaQVDc = 6
        roundtrip_quant(bits, &quant, false);
    }

    #[test]
    fn quant_tip_frame_as_output_round_trips() {
        // With TIP_FRAME_AS_OUTPUT the Y DC read is skipped and uv_dc-only collapses the
        // chroma condition; only base_q_idx is written.
        let quant = CoreSeqQuantView {
            y_dc_delta_q_enabled: true,
            uv_dc_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(60, 8);
        roundtrip_quant(bits, &quant, true);
    }

    #[test]
    fn quant_monochrome_round_trips() {
        let quant = CoreSeqQuantView {
            num_planes: 1,
            separate_uv_delta_q: true,
            uv_dc_delta_q_enabled: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(77, 8);
        roundtrip_quant(bits, &quant, false);
    }

    #[test]
    fn quant_base_q_idx_too_wide_rejected() {
        // 8-bit view: base_q_idx must fit f(8); 256 does not.
        let params = quant_params(256);
        let mut writer = BitWriter::new();
        let err = write_quantization_params(&mut writer, &params, &base_quant(), false).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueTooWide {
                value: 256,
                width_bits: 8
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn quant_gated_off_y_dc_nonzero_rejected() {
        // y_dc_delta_q_enabled is false, so DeltaQYDc has no bitstream home.
        let mut params = quant_params(10);
        params.delta_q_y_dc = 3;
        let mut writer = BitWriter::new();
        let err = write_quantization_params(&mut writer, &params, &base_quant(), false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "delta_q_y_dc"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn quant_chroma_block_off_nonzero_rejected() {
        // No chroma reads enabled: any chroma delta / diff_uv_delta is non-canonical.
        let mut params = quant_params(10);
        params.delta_q_u_ac = 1;
        let mut writer = BitWriter::new();
        let err = write_quantization_params(&mut writer, &params, &base_quant(), false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "quant_chroma_delta"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn quant_diff_uv_delta_without_separate_rejected() {
        // diff_uv_delta is only coded when separate_uv_delta_q; setting it otherwise is bad.
        let quant = CoreSeqQuantView {
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut params = quant_params(10);
        params.diff_uv_delta = true;
        let mut writer = BitWriter::new();
        let err = write_quantization_params(&mut writer, &params, &quant, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "diff_uv_delta"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn quant_inferred_v_mismatch_rejected() {
        // !diff_uv_delta -> DeltaQV* must equal DeltaQU*; a mismatch is non-canonical.
        let quant = CoreSeqQuantView {
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut params = quant_params(10);
        params.delta_q_u_ac = 4;
        params.delta_q_v_ac = 5; // should be 4 (copied from U)
        params.delta_q_v_dc = 0;
        let mut writer = BitWriter::new();
        let err = write_quantization_params(&mut writer, &params, &quant, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "quant_v_inferred"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn quant_equal_ac_dc_q_mismatch_rejected() {
        // equal_ac_dc_q -> DeltaQUDc must equal DeltaQUAc (no DC read).
        let quant = CoreSeqQuantView {
            equal_ac_dc_q: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut params = quant_params(10);
        params.delta_q_u_ac = 4;
        params.delta_q_u_dc = 3; // should be 4
        params.delta_q_v_ac = 4;
        params.delta_q_v_dc = 4;
        let mut writer = BitWriter::new();
        let err = write_quantization_params(&mut writer, &params, &quant, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "delta_q_u_dc"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn quant_coded_delta_out_of_su_domain_rejected() {
        // A coded read_delta_q value outside [-64, 63] is rejected before any bit.
        let quant = CoreSeqQuantView {
            y_dc_delta_q_enabled: true,
            ..base_quant()
        };
        let mut params = quant_params(10);
        params.delta_q_y_dc = 100; // > 63
        let mut writer = BitWriter::new();
        let err = write_quantization_params(&mut writer, &params, &quant, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueOutOfRange {
                descriptor: "su",
                value: 100
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // ----- write_setup_qm_params (§ 5.18.6.2) -----

    fn roundtrip_qm(bits: Bits, quant: &CoreSeqQuantView, seg_enabled: bool) {
        let data = bits.into_bytes();
        let mut rd = reader(&data);
        let qm = parse_setup_qm_params(&mut rd, quant, seg_enabled).unwrap();
        let consumed = rd.consumed_bits();
        let mut writer = BitWriter::new();
        write_setup_qm_params(&mut writer, &qm, quant, seg_enabled).unwrap();
        assert_eq!(writer.bit_len(), consumed, "bit length matches parser");
        let bytes = writer.into_bytes();
        let reparsed = parse_setup_qm_params(&mut reader(&bytes), quant, seg_enabled).unwrap();
        assert_eq!(reparsed, qm);
    }

    #[test]
    fn qm_disabled_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // using_qmatrix = 0
        roundtrip_qm(bits, &base_quant(), true);
    }

    #[test]
    fn qm_no_segmentation_single_set_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1); // using_qmatrix
        bits.f(9, 4); // qm_y[0]
        bits.bit(1); // qm_uv_same_as_y
        roundtrip_qm(bits, &base_quant(), false);
    }

    #[test]
    fn qm_segmentation_three_sets_round_trips() {
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.bit(1);
        bits.f(2, 2); // pic_qm_num_minus_1 -> 3 sets
        bits.f(1, 4);
        bits.bit(1); // set 0 same_as_y
        bits.f(2, 4);
        bits.bit(0);
        bits.f(3, 4);
        bits.f(4, 4); // set 1
        bits.f(5, 4);
        bits.bit(0);
        bits.f(6, 4);
        bits.f(7, 4); // set 2
        roundtrip_qm(bits, &quant, true);
    }

    #[test]
    fn qm_shared_uv_round_trips() {
        // !separate_uv_delta_q with qm_u != qm_y: qm_v = qm_u inferred, no qm_v read.
        let mut bits = Bits::default();
        bits.bit(1);
        bits.f(8, 4); // qm_y
        bits.bit(0); // not same_as_y
        bits.f(2, 4); // qm_u (qm_v inferred = 2)
        roundtrip_qm(bits, &base_quant(), false);
    }

    #[test]
    fn qm_monochrome_round_trips() {
        let quant = CoreSeqQuantView {
            num_planes: 1,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.bit(1);
        bits.f(12, 4); // qm_y only
        roundtrip_qm(bits, &quant, false);
    }

    #[test]
    fn qm_same_as_y_canonicalization_collapses_explicit_form() {
        // A parser fed the explicit qm_uv_same_as_y == 0 form repeating qm_y produces a
        // level with qm_u == qm_v == qm_y; the writer re-emits the shorter same_as_y form,
        // which reparses to the identical level (semantic round-trip), but is fewer bits.
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.bit(1); // using_qmatrix
        bits.f(5, 4); // qm_y = 5
        bits.bit(0); // qm_uv_same_as_y = 0 (explicit, redundant)
        bits.f(5, 4); // qm_u = 5
        bits.f(5, 4); // qm_v = 5
        let data = bits.into_bytes();
        let parsed = parse_setup_qm_params(&mut reader(&data), &quant, false).unwrap();
        assert_eq!(
            parsed.levels[0],
            QmSetLevels {
                qm_y: 5,
                qm_u: 5,
                qm_v: 5
            }
        );
        // The writer emits the canonical same_as_y form: 1 + 4 + 1 = 6 bits, not 1+4+1+4+4.
        let mut writer = BitWriter::new();
        write_setup_qm_params(&mut writer, &parsed, &quant, false).unwrap();
        assert_eq!(writer.bit_len(), 6);
        let bytes = writer.into_bytes();
        let reparsed = parse_setup_qm_params(&mut reader(&bytes), &quant, false).unwrap();
        assert_eq!(reparsed, parsed);
    }

    #[test]
    fn qm_disabled_with_nonzero_pic_num_rejected() {
        let qm = SetupQmParams {
            using_qmatrix: false,
            pic_qm_num_minus_1: 1,
            levels: [QmSetLevels::default(); MAX_PIC_QM_NUM],
        };
        let mut writer = BitWriter::new();
        let err = write_setup_qm_params(&mut writer, &qm, &base_quant(), true).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "setup_qm_disabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn qm_nonzero_pic_num_without_segmentation_rejected() {
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 1,
            levels: [QmSetLevels::default(); MAX_PIC_QM_NUM],
        };
        let mut writer = BitWriter::new();
        let err = write_setup_qm_params(&mut writer, &qm, &base_quant(), false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "pic_qm_num_minus_1"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn qm_level_value_too_wide_rejected() {
        let mut qm = qm_disabled();
        qm.using_qmatrix = true;
        qm.levels[0] = QmSetLevels {
            qm_y: 16, // f(4) max is 15
            qm_u: 0,
            qm_v: 0,
        };
        let quant = CoreSeqQuantView {
            num_planes: 1,
            ..base_quant()
        };
        let mut writer = BitWriter::new();
        let err = write_setup_qm_params(&mut writer, &qm, &quant, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueTooWide {
                value: 16,
                width_bits: 4
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn qm_shared_uv_mismatch_rejected() {
        // !separate_uv_delta_q copies qm_v = qm_u; qm_v != qm_u is non-canonical.
        let mut qm = qm_disabled();
        qm.using_qmatrix = true;
        qm.levels[0] = QmSetLevels {
            qm_y: 1,
            qm_u: 2,
            qm_v: 3, // should equal qm_u
        };
        let mut writer = BitWriter::new();
        let err = write_setup_qm_params(&mut writer, &qm, &base_quant(), false).unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalFrameHeader { what: "qm_v" });
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn qm_monochrome_nonzero_chroma_rejected() {
        let mut qm = qm_disabled();
        qm.using_qmatrix = true;
        qm.levels[0] = QmSetLevels {
            qm_y: 1,
            qm_u: 2, // monochrome never reads chroma
            qm_v: 0,
        };
        let quant = CoreSeqQuantView {
            num_planes: 1,
            ..base_quant()
        };
        let mut writer = BitWriter::new();
        let err = write_setup_qm_params(&mut writer, &qm, &quant, false).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "qm_monochrome"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn qm_level_beyond_num_nonzero_rejected() {
        // pic_qm_num_minus_1 = 0 -> 1 level; a non-default level[1] could not be parsed.
        let mut qm = qm_disabled();
        qm.using_qmatrix = true;
        qm.levels[1] = QmSetLevels {
            qm_y: 3,
            qm_u: 3,
            qm_v: 3,
        };
        let mut writer = BitWriter::new();
        let err = write_setup_qm_params(&mut writer, &qm, &base_quant(), true).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "qm_level_beyond_num"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // ----- write_delta_q_params (§ 5.18.7.8) -----

    fn roundtrip_delta_q_params(bits: Bits, base_q_idx: u32) {
        let data = bits.into_bytes();
        let mut rd = reader(&data);
        let dq = parse_delta_q_params(&mut rd, base_q_idx).unwrap();
        let consumed = rd.consumed_bits();
        let mut writer = BitWriter::new();
        write_delta_q_params(&mut writer, &dq, base_q_idx).unwrap();
        assert_eq!(writer.bit_len(), consumed);
        let bytes = writer.into_bytes();
        let reparsed = parse_delta_q_params(&mut reader(&bytes), base_q_idx).unwrap();
        assert_eq!(reparsed, dq);
    }

    #[test]
    fn delta_q_params_zero_base_round_trips() {
        roundtrip_delta_q_params(Bits::default(), 0);
    }

    #[test]
    fn delta_q_params_absent_round_trips() {
        let mut bits = Bits::default();
        bits.bit(0); // delta_q_present = 0
        roundtrip_delta_q_params(bits, 10);
    }

    #[test]
    fn delta_q_params_present_round_trips() {
        let mut bits = Bits::default();
        bits.bit(1);
        bits.f(2, 2); // delta_q_res
        roundtrip_delta_q_params(bits, 10);
    }

    #[test]
    fn delta_q_present_with_zero_base_rejected() {
        let dq = DeltaQParams {
            delta_q_present: true,
            delta_q_res: 0,
        };
        let mut writer = BitWriter::new();
        let err = write_delta_q_params(&mut writer, &dq, 0).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "delta_q_present"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn delta_q_res_without_present_rejected() {
        let dq = DeltaQParams {
            delta_q_present: false,
            delta_q_res: 2,
        };
        let mut writer = BitWriter::new();
        let err = write_delta_q_params(&mut writer, &dq, 10).unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "delta_q_res"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn delta_q_res_too_wide_rejected() {
        let dq = DeltaQParams {
            delta_q_present: true,
            delta_q_res: 4, // f(2) max is 3
        };
        let mut writer = BitWriter::new();
        let err = write_delta_q_params(&mut writer, &dq, 10).unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueTooWide {
                value: 4,
                width_bits: 2
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    // ----- write_lossless_info (§ 5.18.2) -----

    #[allow(clippy::too_many_arguments)]
    fn roundtrip_lossless(
        bits: Bits,
        quant: &CoreSeqQuantView,
        quantization: &QuantizationParams,
        qm: &SetupQmParams,
        delta_q: DeltaQParams,
        segmentation: &SegmentationParams,
        max_segments: u8,
    ) {
        let data = bits.into_bytes();
        let mut rd = reader(&data);
        let info = parse_lossless_info(
            &mut rd,
            quant,
            quantization,
            qm,
            &delta_q,
            segmentation,
            max_segments,
        )
        .unwrap();
        let consumed = rd.consumed_bits();
        let mut writer = BitWriter::new();
        write_lossless_info(
            &mut writer,
            &info,
            quant,
            quantization,
            qm,
            &delta_q,
            segmentation,
            max_segments,
        )
        .unwrap();
        assert_eq!(writer.bit_len(), consumed, "bit length matches parser");
        let bytes = writer.into_bytes();
        let reparsed = parse_lossless_info(
            &mut reader(&bytes),
            quant,
            quantization,
            qm,
            &delta_q,
            segmentation,
            max_segments,
        )
        .unwrap();
        assert_eq!(reparsed, info);
    }

    #[test]
    fn lossless_all_coded_lossless_round_trips() {
        let quant = CoreSeqQuantView {
            enable_tcq: true,
            enable_parity_hiding: true,
            ..base_quant()
        };
        roundtrip_lossless(
            Bits::default(),
            &quant,
            &quant_params(0),
            &qm_disabled(),
            no_delta_q(),
            &seg_params(false),
            8,
        );
    }

    #[test]
    fn lossless_with_qm_index_round_trips() {
        // Segment 0 lossless via SEG_LVL_ALT_Q; segments 1..8 read qm_index f(1).
        let quant = CoreSeqQuantView {
            choose_tcq_per_frame: true,
            enable_parity_hiding: true,
            separate_uv_delta_q: true,
            ..base_quant()
        };
        let mut segmentation = seg_params(true);
        segmentation.features[0][0] = SegmentFeature {
            enabled: true,
            data: -40,
        };
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 1,
            levels: [
                QmSetLevels {
                    qm_y: 1,
                    qm_u: 2,
                    qm_v: 3,
                },
                QmSetLevels {
                    qm_y: 4,
                    qm_u: 5,
                    qm_v: 6,
                },
                QmSetLevels::default(),
                QmSetLevels::default(),
            ],
        };
        let mut bits = Bits::default();
        for i in 1..8 {
            bits.bit((i % 2) as u8); // qm_index for segments 1..8
        }
        bits.bit(0); // allow_tcq
        bits.bit(1); // allow_parity_hiding
        roundtrip_lossless(
            bits,
            &quant,
            &quant_params(40),
            &qm,
            no_delta_q(),
            &segmentation,
            8,
        );
    }

    #[test]
    fn lossless_qm_num_one_zero_bit_index_round_trips() {
        // qmNum == 1 -> CeilLog2 = 0 -> no qm_index bits, but the triple still validates.
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 0,
            levels: [
                QmSetLevels {
                    qm_y: 7,
                    qm_u: 8,
                    qm_v: 9,
                },
                QmSetLevels::default(),
                QmSetLevels::default(),
                QmSetLevels::default(),
            ],
        };
        // separate_uv_delta_q so the distinct qm_v in the level is a canonical QM table.
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            ..base_quant()
        };
        roundtrip_lossless(
            Bits::default(),
            &quant,
            &quant_params(40),
            &qm,
            no_delta_q(),
            &seg_params(false),
            16,
        );
    }

    #[test]
    fn lossless_allow_tcq_inferred_round_trips() {
        let quant = CoreSeqQuantView {
            enable_tcq: true,
            enable_parity_hiding: true,
            ..base_quant()
        };
        roundtrip_lossless(
            Bits::default(),
            &quant,
            &quant_params(40),
            &qm_disabled(),
            no_delta_q(),
            &seg_params(false),
            8,
        );
    }

    #[test]
    fn lossless_parity_hiding_read_round_trips() {
        // !CodedLossless, !choose_tcq (allow_tcq inferred enable_tcq = 0),
        // enable_parity_hiding -> allow_parity_hiding f(1).
        let quant = CoreSeqQuantView {
            enable_parity_hiding: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.bit(1); // allow_parity_hiding
        roundtrip_lossless(
            bits,
            &quant,
            &quant_params(40),
            &qm_disabled(),
            no_delta_q(),
            &seg_params(false),
            8,
        );
    }

    #[test]
    fn lossless_stored_lossless_array_mismatch_rejected() {
        // Constructed model: base_q_idx = 40 (not lossless) but lossless_array[0] = true.
        let mut info = parse_lossless_info(
            &mut reader(&[]),
            &base_quant(),
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        info.lossless_array[0] = true; // re-derivation says false
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &base_quant(),
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "lossless_array"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_qm_pic_num_out_of_range_rejected() {
        // A constructed qm whose pic_qm_num_minus_1 is beyond the f(2) field (0..=3) would
        // drive an over-wide qm_index field; reject before any bit even when
        // write_lossless_info is called without a prior write_setup_qm_params.
        let quant = base_quant();
        let good_qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 0,
            levels: [QmSetLevels::default(); MAX_PIC_QM_NUM],
        };
        // All-lossless (base_q_idx == 0): every segment stores [15, 15, 15], no qm_index bits.
        let data = Bits::default().into_bytes();
        let info = parse_lossless_info(
            &mut reader(&data),
            &quant,
            &quant_params(0),
            &good_qm,
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        let bad_qm = SetupQmParams {
            pic_qm_num_minus_1: 100,
            ..good_qm
        };
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &quant,
            &quant_params(0),
            &bad_qm,
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::ValueTooWide {
                value: 100,
                width_bits: 2
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_qm_disabled_nonzero_level_rejected() {
        // With using_qmatrix off the parser leaves seg_qm_levels zeroed; a stored non-zero
        // triple could not have been produced.
        let quant = base_quant();
        let data = Bits::default().into_bytes();
        let mut info = parse_lossless_info(
            &mut reader(&data),
            &quant,
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        info.seg_qm_levels[0] = [1, 2, 3];
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &quant,
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "seg_qm_level_disabled"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_tail_beyond_max_segments_rejected() {
        // The parser only touches segments 0..MaxSegments; a stored entry beyond it (here
        // lossless_array[8] with max_segments == 8) could not have been produced.
        let quant = base_quant();
        let data = Bits::default().into_bytes();
        let mut info = parse_lossless_info(
            &mut reader(&data),
            &quant,
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        info.lossless_array[8] = true;
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &quant,
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "lossless_tail"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_qm_index_no_match_rejected() {
        // Constructed model: a non-lossless segment stores a triple matching no level set.
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 0,
            levels: [
                QmSetLevels {
                    qm_y: 1,
                    qm_u: 1,
                    qm_v: 1,
                },
                QmSetLevels::default(),
                QmSetLevels::default(),
                QmSetLevels::default(),
            ],
        };
        let mut info = parse_lossless_info(
            &mut reader(&[]),
            &base_quant(),
            &quant_params(40),
            &qm,
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        // Tamper a stored level triple to one no level set can reproduce.
        info.seg_qm_levels[0] = [9, 9, 9];
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &base_quant(),
            &quant_params(40),
            &qm,
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "seg_qm_level"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_segment_wrong_level_15_rejected() {
        // A lossless segment must store [15, 15, 15]; a constructed mismatch is rejected.
        let quant = CoreSeqQuantView {
            enable_parity_hiding: true,
            ..base_quant()
        };
        let mut segmentation = seg_params(true);
        // Segment 0 lossless (base_q_idx 0, feature off); others lossless too.
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 0,
            levels: [
                QmSetLevels {
                    qm_y: 2,
                    qm_u: 2,
                    qm_v: 2,
                },
                QmSetLevels::default(),
                QmSetLevels::default(),
                QmSetLevels::default(),
            ],
        };
        // Force one segment non-lossless via a positive ALT_Q so it has a qm_index, but make
        // segment 0 lossless and tamper its stored [15,15,15].
        segmentation.features[1][0] = SegmentFeature {
            enabled: true,
            data: 5,
        };
        let mut info = parse_lossless_info(
            &mut reader(&[0u8]),
            &quant,
            &quant_params(0),
            &qm,
            &no_delta_q(),
            &segmentation,
            8,
        )
        .unwrap();
        assert!(info.lossless_array[0]);
        info.seg_qm_levels[0] = [14, 15, 15]; // should be [15, 15, 15]
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &quant,
            &quant_params(0),
            &qm,
            &no_delta_q(),
            &segmentation,
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "seg_qm_level_lossless"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_coded_lossless_mismatch_rejected() {
        let mut info = parse_lossless_info(
            &mut reader(&[]),
            &base_quant(),
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        assert!(info.coded_lossless);
        info.coded_lossless = false; // re-derivation says true
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &base_quant(),
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "coded_lossless"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_has_lossless_segment_mismatch_rejected() {
        let mut info = parse_lossless_info(
            &mut reader(&[]),
            &base_quant(),
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        info.has_lossless_segment = false; // re-derivation says true
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &base_quant(),
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "has_lossless_segment"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_allow_tcq_inferred_mismatch_rejected() {
        // !choose_tcq_per_frame -> allow_tcq inferred enable_tcq (false here); true is bad.
        let mut info = parse_lossless_info(
            &mut reader(&[]),
            &base_quant(),
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        info.allow_tcq = true; // inferred enable_tcq = false
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &base_quant(),
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(err, WriteError::NonCanonicalFrameHeader { what: "allow_tcq" });
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_allow_parity_hiding_inferred_mismatch_rejected() {
        // enable_parity_hiding = false -> allow_parity_hiding inferred false; true is bad.
        let mut info = parse_lossless_info(
            &mut reader(&[]),
            &base_quant(),
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        info.allow_parity_hiding = true;
        let mut writer = BitWriter::new();
        let err = write_lossless_info(
            &mut writer,
            &info,
            &base_quant(),
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            WriteError::NonCanonicalFrameHeader {
                what: "allow_parity_hiding"
            }
        );
        assert_eq!(writer.bit_len(), 0);
    }

    #[test]
    fn lossless_hostile_max_segments_does_not_panic() {
        // max_segments = 255 must not index out of bounds (the writer caps at MAX_SEGMENTS).
        let info = parse_lossless_info(
            &mut reader(&[]),
            &base_quant(),
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            255,
        )
        .unwrap();
        let mut writer = BitWriter::new();
        write_lossless_info(
            &mut writer,
            &info,
            &base_quant(),
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            255,
        )
        .unwrap();
    }
}
