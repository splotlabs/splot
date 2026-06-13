// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn reserved_obu_with_all_zero_payload_is_an_error() {
    // size=2: reserved header 0x00 (obu_type=0) + an all-zero payload byte.
    let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0x00]);
    assert!(!report.is_conformant());
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-reserved/all-zero-payload"),
        "an all-zero reserved OBU payload must be an error (AV2 § 5.3)"
    );
}

#[test]
fn reserved_obu_with_nonzero_payload_is_conformant() {
    // size=2: reserved header 0x00 + non-zero payload. Reserved OBUs have no
    // defined payload syntax, so this is retained and ignored.
    let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0x40]);
    assert!(report.is_conformant());
}

#[test]
fn reserved_obu_with_nonzero_trailing_bits_shape_is_conformant() {
    // size=2: reserved header 0x00 + non-zero payload that is not a valid
    // trailing_bits pattern. Reserved OBUs are ignored except for the
    // all-zero-payload guard.
    let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0xC0]);
    assert!(report.is_conformant(), "report was: {report}");
}

#[test]
fn temporal_delimiter_payload_trailing_bits_are_validated() {
    // size=2: temporal delimiter header 0x08 + valid full-byte trailing_bits.
    let valid = Validator::new(false).validate_bytes(&[0x02, 0x08, 0x80]);
    assert!(valid.is_conformant(), "report was: {valid}");

    // Same OBU with missing trailing_one_bit.
    let invalid = Validator::new(false).validate_bytes(&[0x02, 0x08, 0x00]);
    assert!(
        invalid
            .errors()
            .any(|d| d.rule_id == "trailing-bits/missing-one-bit"),
        "report was: {invalid}"
    );
}

#[test]
fn sequence_header_payload_syntax_is_validated() {
    let mut bits = Bits::default();
    bits.uvlc(0); // seq_header_id
    bits.f(0, 5); // seq_profile_idc
    bits.bit(1); // single_picture_header_flag
    bits.f(0, 5); // seq_level_idx
    bits.uvlc(4); // invalid chroma_format_idc

    let data = annex_b_obu(0x04, &bits.into_bytes());
    let report = Validator::new(false).validate_bytes(&data);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "sequence-header/chroma-format-out-of-range"),
        "report was: {report}"
    );
}

#[test]
fn sequence_header_payload_eof_is_reported() {
    let report = Validator::new(false).validate_bytes(&[0x01, 0x04]);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "bitstream/parse-error"),
        "report was: {report}"
    );
}

#[test]
fn global_xlayer_requires_base_layers_is_flagged() {
    // size=2, header 0xA0 0x3F: OBU_METADATA_SHORT, ext, mlayer=1, xlayer=31
    // (global). A global xlayer requires base mlayer/tlayer (§ 6.2.2).
    let report = Validator::new(false).validate_bytes(&[0x02, 0xA0, 0x3F]);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-header/global-xlayer-requires-base-layers"),
        "report was: {report}"
    );
}

#[test]
fn global_xlayer_on_disallowed_type_is_flagged() {
    // size=2, header 0x84 0x1F: OBU_SEQUENCE_HEADER, ext, mlayer=0, xlayer=31.
    // The sequence header may not carry the global xlayer (§ 6.2.2).
    let report = Validator::new(false).validate_bytes(&[0x02, 0x84, 0x1F]);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-header/global-xlayer-allowed-types"),
        "report was: {report}"
    );
}

#[test]
fn base_layer_only_type_with_nonzero_layer_is_flagged() {
    // size=2, header 0x85 0x00: OBU_SEQUENCE_HEADER, ext, tlayer=1, mlayer=0,
    // xlayer=0. The sequence header must be base tlayer/mlayer (§ 6.2.2).
    let report = Validator::new(false).validate_bytes(&[0x02, 0x85, 0x00]);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-header/base-layer-only-types"),
        "report was: {report}"
    );
}

#[test]
fn temporal_layer_zero_only_type_is_flagged() {
    // size=1, header 0x11: OBU_CLOSED_LOOP_KEY, no extension, tlayer=1. Closed/
    // open-loop key, switch, and RAS frames must use tlayer 0 (§ 6.2.2).
    let report = Validator::new(false).validate_bytes(&[0x01, 0x11]);
    assert!(
        report
            .errors()
            .any(|d| d.rule_id == "obu-header/temporal-layer-zero-only-types"),
        "report was: {report}"
    );
}

#[test]
fn reserved_obu_type_emits_info_and_stays_conformant() {
    // size=2, header 0x68 (reserved obu_type 26) + non-zero payload 0x80.
    // Reserved types are ignored by decoders: informational, not an error.
    let report = Validator::new(false).validate_bytes(&[0x02, 0x68, 0x80]);
    assert!(report.is_conformant(), "report was: {report}");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == "obu-header/reserved-obu-type"),
        "report was: {report}"
    );
}
