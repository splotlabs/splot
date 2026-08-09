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
use splot_core::span::ByteOffset;

#[test]
fn internal_state_error_stays_operational() {
    let error = DecodeError::InternalState {
        reason: "inter_test_invariant",
        byte_offset: ByteOffset::new(7),
    };

    assert!(DecodeDiagnosticReport::from_decode_error(&error).is_none());
    assert_eq!(
        error.to_string(),
        "internal decode state `inter_test_invariant` failed at byte 7"
    );
}

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
    let error = plan_byte_stream(&[0x01, 0x50], &DecodeOptions::default()).unwrap_err();

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, UNSUPPORTED_FEATURE_RULE_ID);
    assert_eq!(report.diagnostic.severity, DecodeSeverity::Error);
    assert_eq!(report.diagnostic.spec_section, Some("7.1"));
    let DecodeDiagnosticDetails::UnsupportedStructure(details) = report.details else {
        panic!("expected unsupported-structure details");
    };
    assert_eq!(
        details.unsupported_reason,
        DecodeUnsupportedReason::MultistreamSelection.as_str()
    );
    assert_eq!(details.obu_type, "OBU_MSDO");
    assert_eq!(details.byte_offset, 1);
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

#[test]
fn frame_set_output_error_report_uses_stable_source_kind() {
    let error = DecodeError::Output {
        source: DecodeOutputError::invalid_frame_set(
            DecodeOutputOperation::SerializeY4m,
            "runtime Y4M output requires at least one decoded frame",
        ),
    };

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();

    assert_eq!(report.diagnostic.rule_id, OUTPUT_ERROR_RULE_ID);
    let DecodeDiagnosticDetails::OutputError(details) = report.details else {
        panic!("expected output-error details");
    };
    assert_eq!(details.operation, "serialize_y4m");
    assert_eq!(details.source_kind, "frame_set");
    assert_eq!(
        details.source_message,
        "runtime Y4M output requires at least one decoded frame"
    );
}
