// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 bitstream validator: parse, then run the check registry.

use splot_core::Error;
use splot_core::annexb::{ObuEnvelope, parse_annex_b_obus_partial};
use splot_core::span::{BitOffset, ByteOffset};

use crate::checks::{Check, default_checks, syntax_error_diagnostic};
use crate::diagnostic::{Diagnostic, Severity, ValidationReport};

/// Validates AV2 length-delimited bitstreams and produces a [`ValidationReport`].
#[derive(Debug, Clone, Copy)]
pub struct Validator {
    /// When `true`, [`Validator::is_acceptable`] treats a report with warnings
    /// (not just errors) as a conformance failure. The set of diagnostics produced
    /// by [`Validator::validate_bytes`] is unaffected.
    pub strict: bool,
}

impl Validator {
    /// Creates a validator.
    #[must_use]
    pub fn new(strict: bool) -> Self {
        Self { strict }
    }

    /// Returns `true` if `report` passes under this validator's strictness.
    ///
    /// A report always fails if it contains any [`Severity::Error`]; in
    /// [`Validator::strict`] mode it additionally fails if it contains any warning.
    /// This is the single source of truth for pass/fail (the CLI's exit status
    /// uses it).
    #[must_use]
    pub fn is_acceptable(&self, report: &ValidationReport) -> bool {
        report.is_conformant() && !(self.strict && report.warnings().next().is_some())
    }

    /// Validates `data` as an AV2 Annex B bitstream.
    ///
    /// A malformed bitstream is reported as one or more [`Severity::Error`]
    /// diagnostics, never as a panic or an `Err`.
    #[must_use]
    pub fn validate_bytes(&self, data: &[u8]) -> ValidationReport {
        let mut report = ValidationReport::new();
        // Parse the whole stream, keeping OBUs parsed before any later structural
        // error so their conformance diagnostics are not lost.
        let parsed = parse_annex_b_obus_partial(data);
        let checks = default_checks();
        for obu in &parsed.obus {
            run_checks(&checks, obu, &mut report);
        }
        if let Some(error) = parsed.error {
            report.push(parse_error_diagnostic(&error));
        }
        report
    }
}

fn run_checks(checks: &[Box<dyn Check>], obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
    for check in checks {
        check.run(obu, report);
    }
}

fn parse_error_diagnostic(error: &Error) -> Diagnostic {
    if let Some(diagnostic) = syntax_error_diagnostic(error) {
        return diagnostic;
    }

    let mut diagnostic =
        Diagnostic::new(Severity::Error, "bitstream/parse-error", error.to_string())
            .with_spec_section("Annex B");
    if let Some(offset) = error_offset(error) {
        diagnostic = diagnostic.with_byte_offset(offset);
    }
    if let Some(bit_offset) = error_bit_offset(error) {
        diagnostic = diagnostic.with_bit_offset(bit_offset);
    }
    diagnostic
}

fn error_offset(error: &Error) -> Option<ByteOffset> {
    match error {
        Error::UnexpectedEof { offset, .. }
        | Error::InvalidLeb128 { offset, .. }
        | Error::InvalidUvlc { offset, .. }
        | Error::InvalidNs { offset, .. }
        | Error::InvalidObuHeader { offset, .. }
        | Error::InvalidTrailingBits { offset, .. }
        | Error::InvalidByteAlignment { offset, .. }
        | Error::InvalidSequenceHeader { offset, .. }
        | Error::ObuSizeOutOfRange { offset, .. }
        | Error::ObuPayloadOutOfRange { offset, .. } => Some(*offset),
        _ => None,
    }
}

fn error_bit_offset(error: &Error) -> Option<BitOffset> {
    match error {
        Error::InvalidUvlc { bit_offset, .. }
        | Error::InvalidNs { bit_offset, .. }
        | Error::InvalidTrailingBits { bit_offset, .. }
        | Error::InvalidByteAlignment { bit_offset, .. }
        | Error::InvalidSequenceHeader { bit_offset, .. } => Some(*bit_offset),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    fn annex_b_obu(header: u8, payload: &[u8]) -> Vec<u8> {
        let size = (payload.len() + 1) as u8;
        let mut data = Vec::with_capacity(payload.len() + 2);
        data.push(size);
        data.push(header);
        data.extend_from_slice(payload);
        data
    }

    #[test]
    fn conformant_temporal_delimiter() {
        let report = Validator::new(false).validate_bytes(&[0x01, 0x08]);
        assert!(report.is_conformant());
        assert_eq!(report.errors().count(), 0);
    }

    #[test]
    fn temporal_delimiter_without_global_xlayer_is_flagged() {
        // size=2, header 0x88 0x05: TemporalDelimiter with extension, xlayer=5 (not global).
        let report = Validator::new(false).validate_bytes(&[0x02, 0x88, 0x05]);
        assert!(!report.is_conformant());
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/global-xlayer-required")
        );
    }

    #[test]
    fn parse_error_becomes_a_single_error_diagnostic() {
        let report = Validator::new(false).validate_bytes(&[0x00]);
        assert!(!report.is_conformant());
        assert_eq!(report.errors().count(), 1);
        assert!(report.diagnostics[0].byte_offset.is_some());
    }

    #[test]
    fn report_display_reports_status() {
        let report = Validator::new(false).validate_bytes(&[0x02, 0x88, 0x05]);
        assert!(report.to_string().contains("ERROR"));
    }

    #[test]
    fn diagnostics_from_prefix_survive_a_later_parse_error() {
        // OBU #0: TemporalDelimiter with extension, xlayer=5 (a §6.2.2 violation).
        // OBU #1: truncated (declares 5 bytes, only 1 present).
        let report = Validator::new(false).validate_bytes(&[0x02, 0x88, 0x05, 0x05, 0x08]);
        assert!(!report.is_conformant());
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-header/global-xlayer-required"),
            "expected the conformance error from the parseable prefix"
        );
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "expected the parse error for the truncated tail"
        );
    }

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
        // size=2: reserved header 0x00 + payload 0x80 (trailing_one_bit = 1).
        let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0x80]);
        assert!(report.is_conformant());
    }

    #[test]
    fn reserved_obu_with_nonzero_bad_trailing_zero_bit_is_flagged() {
        // size=2: reserved header 0x00 + payload 0xC0. The first trailing bit is
        // valid, but the next padding bit is 1.
        let report = Validator::new(false).validate_bytes(&[0x02, 0x00, 0xC0]);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "trailing-bits/zero-bit-not-zero"),
            "report was: {report}"
        );
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
}
