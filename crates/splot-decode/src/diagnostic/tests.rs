// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use crate::bitstream::byte_stream::plan_byte_stream;
use crate::{
    DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError, DecodeLimitName,
    DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeOutputError, DecodeOutputOperation,
    DecodeSeverity, DecodeSourceIssueKind, DecodeUnsupportedReason, MALFORMED_SOURCE_RULE_ID,
    OUTPUT_ERROR_RULE_ID, RESOURCE_LIMIT_RULE_ID, UNSUPPORTED_FEATURE_RULE_ID,
};

#[test]
fn malformed_source_report_has_stable_fields() {
    let error = plan_byte_stream(&[0x05, 0x10], &DecodeOptions::default()).unwrap_err();

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, MALFORMED_SOURCE_RULE_ID);
    assert_eq!(report.diagnostic.severity, DecodeSeverity::Error);
    assert_eq!(report.diagnostic.spec_section, None);
    let DecodeDiagnosticDetails::MalformedSource(details) = report.details else {
        panic!("expected malformed-source details");
    };
    assert_eq!(
        details.source_issue_kind,
        DecodeSourceIssueKind::AnnexBParseError.as_str()
    );
    assert!(details.byte_offset.is_some());
    assert!(!details.parser_message.is_empty());
}

#[test]
fn resource_limit_report_has_measured_values() {
    let options =
        DecodeOptions::new(DecodeLimits::unlimited().with_max_obus(DecodeLimitThreshold::Max(0)));
    let error = plan_byte_stream(&[0x01, 0x08], &options).unwrap_err();

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, RESOURCE_LIMIT_RULE_ID);
    assert_eq!(report.diagnostic.severity, DecodeSeverity::Error);
    assert_eq!(report.diagnostic.spec_section, Some("5.2.1"));
    let DecodeDiagnosticDetails::ResourceLimit(details) = report.details else {
        panic!("expected resource-limit details");
    };
    assert_eq!(details.limit_name, DecodeLimitName::MaxObus.as_str());
    assert_eq!(details.limit, Some(0));
    assert_eq!(details.actual, Some(1));
    assert_eq!(details.unit, "count");
    assert_eq!(details.byte_offset, None);
    assert_eq!(details.bit_offset, None);
}

#[test]
fn lr_source_read_resource_limit_cites_source_read_process() {
    let source = DecodeLimits::zero()
        .ensure(DecodeLimitName::MaxLoopRestorationSourceReads, 1)
        .unwrap_err();
    let error = DecodeError::Limit { source };

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, RESOURCE_LIMIT_RULE_ID);
    assert_eq!(report.diagnostic.spec_section, Some("7.20.2"));
    let DecodeDiagnosticDetails::ResourceLimit(details) = report.details else {
        panic!("expected resource-limit details");
    };
    assert_eq!(
        details.limit_name,
        DecodeLimitName::MaxLoopRestorationSourceReads.as_str()
    );
}

#[test]
fn unsupported_structure_report_uses_planner_metadata() {
    let error = plan_byte_stream(&[0x01, 0x14], &DecodeOptions::default()).unwrap_err();

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, UNSUPPORTED_FEATURE_RULE_ID);
    assert_eq!(report.diagnostic.severity, DecodeSeverity::Error);
    assert_eq!(report.diagnostic.spec_section, Some("5.2.1"));
    let DecodeDiagnosticDetails::UnsupportedStructure(details) = report.details else {
        panic!("expected unsupported-structure details");
    };
    assert_eq!(
        details.unsupported_reason,
        DecodeUnsupportedReason::UnsupportedFrameObu.as_str()
    );
    assert_eq!(details.obu_type, "OBU_OPEN_LOOP_KEY");
    assert_eq!(details.byte_offset, 1);
}

#[test]
fn runtime_unsupported_report_summarizes_successful_plan() {
    let plan = plan_byte_stream(&[0x01, 0x10], &DecodeOptions::default()).unwrap();

    let report = DecodeDiagnosticReport::runtime_unsupported(&plan);

    assert_eq!(report.diagnostic.rule_id, UNSUPPORTED_FEATURE_RULE_ID);
    assert_eq!(report.diagnostic.spec_section, Some("7.1"));
    let DecodeDiagnosticDetails::RuntimeUnsupported(summary) = report.details else {
        panic!("expected runtime-unsupported summary");
    };
    assert_eq!(summary.bitstream_format, "annex_b");
    assert_eq!(summary.input_len_bytes, 2);
    assert_eq!(summary.obu_count, 1);
    assert_eq!(summary.frame_candidate_count, 1);
    assert_eq!(summary.source_warning_count, 0);
    assert_eq!(summary.selected_temporal_layer_id, 0);
    assert_eq!(summary.selected_embedded_layer_id, 0);
    assert_eq!(summary.selected_extended_layer_id, 0);
}

#[test]
fn output_error_report_has_stable_operation_details() {
    let error = DecodeError::Output {
        source: DecodeOutputError::io(
            DecodeOutputOperation::WriteY4mStream,
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed writer"),
        ),
    };

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, OUTPUT_ERROR_RULE_ID);
    assert_eq!(report.diagnostic.severity, DecodeSeverity::Error);
    assert_eq!(report.diagnostic.spec_section, None);
    let DecodeDiagnosticDetails::OutputError(details) = report.details else {
        panic!("expected output-error details");
    };
    assert_eq!(details.operation, "write_y4m_stream");
    assert_eq!(details.source_kind, "io");
    assert_eq!(details.source_message, "closed writer");
}

#[test]
fn raw_output_error_report_uses_raw_support_row() {
    let error = DecodeError::Output {
        source: DecodeOutputError::io(
            DecodeOutputOperation::WriteRawStream,
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed writer"),
        ),
    };

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, OUTPUT_ERROR_RULE_ID);
    assert_eq!(report.diagnostic.severity, DecodeSeverity::Error);
    assert_eq!(report.diagnostic.spec_section, None);
    let DecodeDiagnosticDetails::OutputError(details) = report.details else {
        panic!("expected output-error details");
    };
    assert_eq!(details.operation, "write_raw_stream");
    assert_eq!(details.source_kind, "io");
    assert_eq!(details.source_message, "closed writer");
}
