// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn conformant_temporal_delimiter() {
    let report = Validator::new(false).validate_bytes(&[0x01, 0x08]);
    assert!(report.is_conformant());
    assert_eq!(report.errors().count(), 0);
}

#[test]
fn conformant_temporal_delimiter_in_ivf_is_accepted() {
    let report = Validator::new(false).validate_bytes(&ivf_stream(&[&[0x01, 0x08]]));
    assert!(report.is_conformant(), "report was: {report}");
    assert_eq!(report.errors().count(), 0);
}

#[test]
fn malformed_ivf_frame_payload_is_a_diagnostic() {
    let mut data = ivf_stream(&[&[0x01, 0x08]]);
    data.extend_from_slice(&5u32.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());
    data.extend_from_slice(&[0x01, 0x08]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(!report.is_conformant());
    let diagnostic_offset = report
        .errors()
        .find(|d| d.rule_id == "ivf/truncated-frame-payload")
        .map(|d| d.byte_offset);
    assert_eq!(
        diagnostic_offset,
        Some(Some(splot_core::span::ByteOffset::new(data.len() as u64)))
    );
}

#[test]
fn trailing_partial_ivf_frame_header_after_complete_frame_is_tolerated() {
    let mut data = ivf_stream(&[&[0x01, 0x08]]);
    data.extend_from_slice(&1148u32.to_le_bytes());
    data.extend_from_slice(&6480u64.to_le_bytes()[..6]);
    let report = Validator::new(false).validate_bytes(&data);
    assert!(report.is_conformant(), "report was: {report}");
    assert_eq!(report.errors().count(), 0);
    let warning_offset = report
        .warnings()
        .find(|d| d.rule_id == "ivf/trailing-partial-frame-header")
        .map(|d| d.byte_offset);
    assert_eq!(
        warning_offset,
        Some(Some(splot_core::span::ByteOffset::new(data.len() as u64)))
    );
}

#[test]
fn annex_b_parse_error_inside_ivf_frame_is_a_bitstream_diagnostic() {
    let report = Validator::new(false).validate_bytes(&ivf_stream(&[&[0x05, 0x08]]));
    assert!(!report.is_conformant());
    let diagnostic_offset = report
        .errors()
        .find(|d| d.rule_id == "bitstream/parse-error")
        .map(|d| d.byte_offset);
    assert_eq!(
        diagnostic_offset,
        Some(Some(splot_core::span::ByteOffset::new(45)))
    );
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
