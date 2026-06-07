// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 bitstream validator: parse, then run the check registry.

use splot_core::Error;
use splot_core::annexb::{ObuEnvelope, parse_annex_b_obus_partial};

use crate::checks::{Check, default_checks, syntax_error_diagnostic};
use crate::context::ValidatorContext;
use crate::diagnostic::{Diagnostic, Severity, ValidationReport};
use crate::error_location::{error_bit_offset, error_offset};

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
        let mut context = ValidatorContext::default();
        for obu in &parsed.obus {
            context.observe_obu(obu, &mut report);
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
        annex_b_obu_with_header(&[header], payload)
    }

    fn annex_b_obu_with_header(header: &[u8], payload: &[u8]) -> Vec<u8> {
        let size = payload.len() + header.len();
        assert!(u8::try_from(size).is_ok());
        let mut data = Vec::with_capacity(payload.len() + header.len() + 1);
        data.push(size as u8);
        data.extend_from_slice(header);
        data.extend_from_slice(payload);
        data
    }

    fn layer_obu_header(obu_type: u8, tlayer: u8, mlayer: u8, xlayer: u8) -> [u8; 2] {
        [
            0x80 | (obu_type << 2) | (tlayer & 0b11),
            ((mlayer & 0b111) << 5) | (xlayer & 0b1_1111),
        ]
    }

    fn ceil_log2_u32(value: u32) -> u32 {
        if value <= 1 {
            0
        } else {
            u32::BITS - (value - 1).leading_zeros()
        }
    }

    fn sequence_header_payload(max_tlayer_id: u32, max_mlayer_id: u32) -> Vec<u8> {
        let mut bits = Bits::default();
        bits.uvlc(0); // seq_header_id
        bits.f(0, 5); // seq_profile_idc
        bits.bit(0); // single_picture_header_flag
        bits.f(0, 5); // seq_level_idx
        bits.uvlc(0); // chroma_format_idc
        bits.uvlc(0); // bit_depth_idc
        bits.f(0, 3); // seq_lcr_id
        bits.bit(0); // still_picture
        bits.f(max_tlayer_id, 2);
        bits.f(max_mlayer_id, 3);
        if max_mlayer_id > 0 {
            bits.f(0, ceil_log2_u32(max_mlayer_id + 1)); // seq_max_mlayer_cnt_minus_1
        }
        bits.bit(1); // monotonic_output_order_flag
        bits.f(3, 4); // frame_width_bits_minus_1
        bits.f(3, 4); // frame_height_bits_minus_1
        bits.f(15, 4); // max_frame_width_minus_1
        bits.f(7, 4); // max_frame_height_minus_1
        bits.bit(0); // seq_cropping_window_present_flag
        bits.bit(0); // seq_initial_display_delay_present_flag
        bits.bit(0); // decoder_model_info_present_flag
        if max_mlayer_id > 0 {
            bits.bit(0); // mlayer_dependency_present_flag
        }
        if max_tlayer_id > 0 {
            bits.bit(0); // tlayer_dependency_present_flag
        }
        bits.into_bytes()
    }

    fn stream_with_sequence_header(max_tlayer_id: u32, max_mlayer_id: u32) -> Vec<u8> {
        annex_b_obu(0x04, &sequence_header_payload(max_tlayer_id, max_mlayer_id))
    }

    fn sequence_header_obu_for_xlayer(
        xlayer: u8,
        max_tlayer_id: u32,
        max_mlayer_id: u32,
    ) -> Vec<u8> {
        let payload = sequence_header_payload(max_tlayer_id, max_mlayer_id);
        if xlayer == 0 {
            annex_b_obu(0x04, &payload)
        } else {
            annex_b_obu_with_header(&layer_obu_header(1, 0, 0, xlayer), &payload)
        }
    }

    fn temporal_delimiter_obu() -> Vec<u8> {
        annex_b_obu(0x08, &[])
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

    #[test]
    fn active_sequence_header_allows_following_obu_within_layer_limits() {
        let mut data = stream_with_sequence_header(1, 1);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report
                .errors()
                .any(|d| d.rule_id.starts_with("sequence-state/")),
            "report was: {report}"
        );
    }

    #[test]
    fn layer_obu_before_sequence_header_reports_missing_active_sequence() {
        let data = annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn non_activating_sequence_header_does_not_suppress_missing_active_sequence_error() {
        // 0x05 = OBU_SEQUENCE_HEADER at tlayer=1, so it parses but cannot activate.
        let mut data = annex_b_obu(0x05, &sequence_header_payload(1, 0));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/no-active-sequence-header"),
            "report was: {report}"
        );
    }

    #[test]
    fn temporal_unit_accepts_ascending_coded_xlayers() {
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 0), &[]));
        data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 1, 1), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            !report.errors().any(|d| d.rule_id.starts_with("obu-order/")),
            "report was: {report}"
        );
        assert!(report.is_conformant(), "report was: {report}");
    }

    #[test]
    fn temporal_unit_missing_delimiter_is_reported() {
        let data = annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]);
        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/temporal-unit-missing-delimiter"),
            "report was: {report}"
        );
    }

    #[test]
    fn global_hls_after_coded_layer_is_reported() {
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 0), &[]));
        data.extend(annex_b_obu_with_header(
            &layer_obu_header(16, 0, 0, 31),
            &[],
        ));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/global-hls-after-coded-layer"),
            "report was: {report}"
        );
    }

    #[test]
    fn coded_xlayers_must_ascend_within_temporal_unit() {
        let mut data = temporal_delimiter_obu();
        data.extend(sequence_header_obu_for_xlayer(1, 1, 1));
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 0, 1), &[]));
        data.extend(sequence_header_obu_for_xlayer(0, 1, 1));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/xlayer-order-not-ascending"),
            "report was: {report}"
        );
    }

    #[test]
    fn non_global_padding_outside_coded_layer_is_reported() {
        let mut data = temporal_delimiter_obu();
        data.extend(annex_b_obu_with_header(&layer_obu_header(25, 0, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "obu-order/padding-non-global-outside-coded-layer"),
            "report was: {report}"
        );
    }

    #[test]
    fn active_sequence_header_bounds_temporal_layer_id() {
        let mut data = stream_with_sequence_header(1, 1);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 2, 0, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn active_sequence_header_bounds_embedded_layer_id() {
        let mut data = stream_with_sequence_header(1, 1);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 0, 2, 0), &[]));

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/mlayer-exceeds-max"),
            "report was: {report}"
        );
    }

    #[test]
    fn stateful_diagnostics_from_prefix_survive_a_later_parse_error() {
        let mut data = stream_with_sequence_header(0, 0);
        data.extend(annex_b_obu_with_header(&layer_obu_header(6, 1, 0, 0), &[]));
        data.extend_from_slice(&[0x05, 0x08]);

        let report = Validator::new(false).validate_bytes(&data);
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "sequence-state/tlayer-exceeds-max"),
            "report was: {report}"
        );
        assert!(
            report
                .errors()
                .any(|d| d.rule_id == "bitstream/parse-error"),
            "report was: {report}"
        );
    }
}
