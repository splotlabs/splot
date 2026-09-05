// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// `OBU_QUANTIZATION_MATRIX` header byte: ext=0, type=22, tlayer=0.
pub(in crate::validator::tests) const QM_HEADER: u8 = 0x58;
/// `OBU_FILM_GRAIN` header byte: ext=0, type=23, tlayer=0.
pub(in crate::validator::tests) const FG_HEADER: u8 = 0x5C;

/// A complete, activating sequence header OBU for `obu_xlayer_id = 0` (so the
/// coded-frame-unit QM/FG OBUs that follow have an active sequence header).
pub(in crate::validator::tests) fn active_sequence_header_obu() -> Vec<u8> {
    annex_b_obu(0x04, &sequence_header_payload(0, 0))
}

/// A `quantizer_matrix_obu()` with `qm_bit_map == 0` (the reset/default path).
pub(in crate::validator::tests) fn qm_reset_obu() -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0, 15); // qm_bit_map = 0
    bits.bit(0); // qm_chroma_info_present_flag
    bits.bit(1); // trailing_one_bit (QM is non-extensible)
    bits.align();
    annex_b_obu(QM_HEADER, &bits.into_bytes())
}

/// A `quantizer_matrix_obu()` selecting a single `level` with its default matrix.
pub(in crate::validator::tests) fn qm_default_level_obu(level: u32) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(1 << level, 15); // qm_bit_map: set `level`
    bits.bit(0); // qm_chroma_info_present_flag = 0 -> 1 plane
    bits.bit(1); // qm_is_default_flag for `level`
    bits.bit(1); // trailing_one_bit
    bits.align();
    annex_b_obu(QM_HEADER, &bits.into_bytes())
}

/// Appends the smallest non-monochrome `film_grain_model()` (no scaling points,
/// `ar_coeff_lag == 0`).
pub(in crate::validator::tests) fn append_minimal_film_grain_model(bits: &mut Bits) {
    append_film_grain_model_with_points(bits, 0, 0, 0);
}

/// A `film_grain_obu()` with the given `update_flags` and (non-monochrome)
/// `chroma_idc`, with one minimal model per set update-flag bit.
pub(in crate::validator::tests) fn film_grain_obu_bytes(
    update_flags: u32,
    chroma_idc: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(update_flags, 8);
    bits.uvlc(chroma_idc);
    for _ in 0..update_flags.count_ones() {
        append_minimal_film_grain_model(&mut bits);
    }
    bits.bit(1); // trailing_one_bit (FG is non-extensible)
    bits.align();
    annex_b_obu(FG_HEADER, &bits.into_bytes())
}

pub(in crate::validator::tests) fn has_error(report: &ValidationReport, rule: &str) -> bool {
    report.errors().any(|d| d.rule_id == rule)
}

#[test]
fn qm_duplicate_reset_between_frames_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_reset_obu());
    data.extend(qm_reset_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "qm/duplicate-reset-between-frames"),
        "a second reset QM OBU must be flagged: {report}"
    );
}

#[test]
fn qm_single_reset_between_frames_is_conformant() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_reset_obu());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "qm/duplicate-reset-between-frames"),
        "a single reset QM OBU is conformant: {report}"
    );
}

#[test]
fn qm_duplicate_level_between_frames_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_default_level_obu(0));
    data.extend(qm_default_level_obu(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "qm/duplicate-level-between-frames"),
        "specifying QM level 0 twice must be flagged: {report}"
    );
}

#[test]
fn qm_distinct_levels_between_frames_is_conformant() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_default_level_obu(0));
    data.extend(qm_default_level_obu(1));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "qm/duplicate-level-between-frames"),
        "two distinct QM levels are conformant: {report}"
    );
}

#[test]
fn qm_duplicate_level_across_coded_frame_is_not_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_default_level_obu(0));
    data.extend(annex_b_obu(0x10, &[0xe0])); // OBU_CLOSED_LOOP_KEY (frame-bearing)
    data.extend(qm_default_level_obu(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "qm/duplicate-level-between-frames"),
        "the same level across a coded frame must not be flagged: {report}"
    );
}

#[test]
fn film_grain_zero_update_flags_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_obu_bytes(0, 0)); // fgm_update_flags == 0
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/update-flags-zero"),
        "fgm_update_flags == 0 must be flagged: {report}"
    );
}

#[test]
fn film_grain_chroma_idc_out_of_range_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_obu_bytes(0b0000_0001, 4)); // fgm_chroma_idc = 4 (> 3)
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/chroma-idc-out-of-range"),
        "fgm_chroma_idc > 3 must be flagged: {report}"
    );
}

#[test]
fn film_grain_duplicate_slot_in_coded_frame_unit_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_obu_bytes(0b0000_0001, 0));
    data.extend(film_grain_obu_bytes(0b0000_0001, 0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/duplicate-slot-in-coded-frame-unit"),
        "updating slot 0 twice in one coded frame unit must be flagged: {report}"
    );
}

#[test]
fn film_grain_distinct_slots_are_conformant() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0
    data.extend(film_grain_obu_bytes(0b0000_0010, 0)); // slot 1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "film-grain/duplicate-slot-in-coded-frame-unit"),
        "distinct film grain slots are conformant: {report}"
    );
    assert!(!has_error(&report, "film-grain/update-flags-zero"));
    assert!(!has_error(&report, "film-grain/chroma-idc-out-of-range"));
}

#[test]
fn qm_malformed_payload_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(QM_HEADER, &[0xFF]));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "bitstream/parse-error"),
        "a malformed quantizer matrix payload must be reported: {report}"
    );
}

#[test]
fn film_grain_malformed_payload_is_flagged() {
    let mut bits = Bits::default();
    bits.f(0b0000_0001, 8); // fgm_update_flags: slot 0
    bits.uvlc(2); // fgm_chroma_idc = 444 (non-monochrome)
    bits.bit(0); // chroma_scaling_from_luma
    bits.f(5, 4); // num_y_points = 5 -> point payload follows, but the input ends
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu(FG_HEADER, &bits.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "bitstream/parse-error"),
        "a malformed film-grain payload must be reported: {report}"
    );
}

#[test]
fn qm_quant_delta_out_of_range_is_flagged() {
    let mut bits = Bits::default();
    bits.f(1, 15); // qm_bit_map: level 0
    bits.bit(0); // 1 plane
    bits.bit(0); // qm_is_default_flag = 0
    bits.bit(0); // qm_8x8_is_symmetric = 0
    bits.uvlc(255); // svlc(128) -> quant_delta = 128 (out of range)
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(annex_b_obu(QM_HEADER, &bits.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "qm/quant-delta-out-of-range"),
        "an out-of-range quant_delta must be flagged: {report}"
    );
}

/// Appends `num` scaling points (per-point value increment 1, scaling 0), so the
/// points are strictly increasing.
pub(in crate::validator::tests) fn append_scaling_points(bits: &mut Bits, num: u32) {
    bits.f(num, 4);
    if num > 0 {
        bits.f(0, 3); // point_value_increment_bits_minus_1 = 0 -> bitsIncr = 1
        bits.f(0, 2); // point_scaling_bits_minus_5 = 0 -> bitsScal = 5
        for _ in 0..num {
            bits.f(1, 1); // value increment = 1
            bits.f(0, 5); // scaling = 0
        }
    }
}

/// Appends a non-monochrome `film_grain_model()` (chroma_scaling_from_luma = 0,
/// ar_coeff_lag = 0) with the given scaling-point counts.
pub(in crate::validator::tests) fn append_film_grain_model_with_points(
    bits: &mut Bits,
    num_y: u32,
    num_cb: u32,
    num_cr: u32,
) {
    bits.bit(0); // chroma_scaling_from_luma = 0
    append_scaling_points(bits, num_y);
    append_scaling_points(bits, num_cb);
    append_scaling_points(bits, num_cr);
    bits.f(0, 2); // grain_scaling_minus_8
    bits.f(0, 2); // ar_coeff_lag = 0 -> numPosLuma = 0
    let num_pos_chroma = u32::from(num_y > 0); // numPosLuma(0) + (num_y>0 ? 1 : 0)
    if num_y > 0 {
        bits.f(0, 2); // bits_per_ar_coeff_y_minus_5 (numPosLuma = 0 coeffs)
    }
    if num_cb > 0 {
        bits.f(0, 2); // bits_per_ar_coeff_cb_minus_5 -> bitsCoef = 5
        for _ in 0..num_pos_chroma {
            bits.f(16, 5); // ar_coeffs_cb (16 - 16 = 0)
        }
    }
    if num_cr > 0 {
        bits.f(0, 2);
        for _ in 0..num_pos_chroma {
            bits.f(16, 5);
        }
    }
    bits.f(0, 2); // ar_coeff_shift_minus_6
    bits.f(0, 2); // grain_scale_shift
    if num_cb > 0 {
        bits.f(0, 8); // cb_mult
        bits.f(0, 8); // cb_luma_mult
        bits.f(0, 9); // cb_offset
    }
    if num_cr > 0 {
        bits.f(0, 8); // cr_mult
        bits.f(0, 8); // cr_luma_mult
        bits.f(0, 9); // cr_offset
    }
    bits.bit(0); // overlap_flag
    bits.bit(0); // clip_to_restricted_range
    bits.bit(0); // film_grain_block_size
}

pub(in crate::validator::tests) fn film_grain_model_obu(
    chroma_idc: u32,
    num_y: u32,
    num_cb: u32,
    num_cr: u32,
) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.f(0b0000_0001, 8); // fgm_update_flags: slot 0
    bits.uvlc(chroma_idc);
    append_film_grain_model_with_points(&mut bits, num_y, num_cb, num_cr);
    bits.bit(1); // trailing_one_bit
    bits.align();
    annex_b_obu(FG_HEADER, &bits.into_bytes())
}

#[test]
fn film_grain_too_many_scaling_points_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_model_obu(2, 15, 0, 0)); // 4:4:4, 15 luma points
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/scaling-points-out-of-range"),
        "num_y_points > 14 must be flagged: {report}"
    );
}

#[test]
fn film_grain_420_unpaired_chroma_points_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_model_obu(0, 1, 0, 1)); // 4:2:0, cb=0 but cr=1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/chroma-points-not-paired"),
        "unpaired 4:2:0 chroma points must be flagged: {report}"
    );
}

#[test]
fn film_grain_paired_chroma_points_are_conformant() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_model_obu(0, 1, 1, 1)); // 4:2:0, cb=1 and cr=1
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        !has_error(&report, "film-grain/chroma-points-not-paired"),
        "paired 4:2:0 chroma points are conformant: {report}"
    );
    assert!(!has_error(
        &report,
        "film-grain/scaling-points-out-of-range"
    ));
    assert!(!has_error(
        &report,
        "film-grain/scaling-point-not-increasing"
    ));
}

#[test]
fn film_grain_non_increasing_scaling_point_is_flagged() {
    let mut bits = Bits::default();
    bits.f(0b0000_0001, 8); // slot 0
    bits.uvlc(2); // 4:4:4 (no chroma pairing constraint)
    bits.bit(0); // chroma_scaling_from_luma = 0
    bits.f(2, 4); // num_y_points = 2
    bits.f(0, 3); // point_value_increment_bits_minus_1 = 0 -> bitsIncr = 1
    bits.f(0, 2); // point_scaling_bits_minus_5 = 0 -> bitsScal = 5
    bits.f(1, 1); // point_y_value[0] increment = 1 -> value 1
    bits.f(0, 5); // point_y_scaling[0]
    bits.f(0, 1); // point_y_value[1] increment = 0 -> value stays 1 (not increasing)
    bits.f(0, 5); // point_y_scaling[1]
    bits.f(0, 4); // num_cb_points = 0
    bits.f(0, 4); // num_cr_points = 0
    bits.f(0, 2); // grain_scaling_minus_8
    bits.f(0, 2); // ar_coeff_lag = 0
    bits.f(0, 2); // bits_per_ar_coeff_y_minus_5 (numPosLuma = 0 coeffs; num_y > 0)
    bits.f(0, 2); // ar_coeff_shift_minus_6
    bits.f(0, 2); // grain_scale_shift
    bits.bit(0); // overlap_flag
    bits.bit(0); // clip_to_restricted_range
    bits.bit(0); // film_grain_block_size
    bits.bit(1); // trailing_one_bit
    bits.align();
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(annex_b_obu(FG_HEADER, &bits.into_bytes()));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/scaling-point-not-increasing"),
        "a non-increasing scaling point must be flagged: {report}"
    );
}

#[test]
fn qm_reset_then_level_definition_is_conformant() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_reset_obu());
    data.extend(qm_default_level_obu(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(!has_error(&report, "qm/duplicate-reset-between-frames"));
    assert!(
        !has_error(&report, "qm/duplicate-level-between-frames"),
        "a level definition after a reset is not a duplicate: {report}"
    );
}

#[test]
fn qm_duplicate_level_across_temporal_delimiter_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(qm_default_level_obu(0));
    data.extend(temporal_delimiter_obu()); // new temporal unit, but no coded frame
    data.extend(qm_default_level_obu(0));
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "qm/duplicate-level-between-frames"),
        "a level reused across a TD with no intervening frame must be flagged: {report}"
    );
}

#[test]
fn film_grain_duplicate_slot_across_temporal_delimiter_is_flagged() {
    let mut data = temporal_delimiter_obu();
    data.extend(active_sequence_header_obu());
    data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0
    data.extend(temporal_delimiter_obu()); // new temporal unit, but no coded frame
    data.extend(film_grain_obu_bytes(0b0000_0001, 0)); // slot 0 again
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        has_error(&report, "film-grain/duplicate-slot-in-coded-frame-unit"),
        "a slot reused across a TD with no intervening frame must be flagged: {report}"
    );
}

pub(in crate::validator::tests) fn has_warning(report: &ValidationReport, rule: &str) -> bool {
    report.warnings().any(|d| d.rule_id == rule)
}

/// A global `OBU_PADDING` (xlayer 31) carrying `payload`, after a temporal delimiter.
pub(in crate::validator::tests) fn global_padding_stream(payload: &[u8]) -> Vec<u8> {
    let mut data = temporal_delimiter_obu();
    data.extend(annex_b_obu_with_header(
        &layer_obu_header(25, 0, 0, 31),
        payload,
    ));
    data
}

#[test]
fn padding_all_zero_payload_is_flagged() {
    let report = Validator::new(false).validate_bytes(&global_padding_stream(&[0x00, 0x00]));
    assert!(
        has_error(&report, "padding/all-zero-payload"),
        "report was: {report}"
    );
}

#[test]
fn padding_invalid_trailing_bits_is_flagged() {
    let report = Validator::new(false).validate_bytes(&global_padding_stream(&[0x40]));
    assert!(
        has_error(&report, "padding/invalid-trailing-bits"),
        "report was: {report}"
    );
}

#[test]
fn padding_valid_payload_is_accepted() {
    let report = Validator::new(false).validate_bytes(&global_padding_stream(&[0xFF, 0x80]));
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("padding/")),
        "report was: {report}"
    );
}

#[test]
fn padding_empty_payload_is_accepted() {
    let report = Validator::new(false).validate_bytes(&global_padding_stream(&[]));
    assert!(
        !report.errors().any(|d| d.rule_id.starts_with("padding/")),
        "report was: {report}"
    );
}
