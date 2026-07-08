// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::DecodeUnsupportedReason;

#[test]
fn raw_obu_limit_is_checked_before_parsing_next_obu() {
    let bytes = [0x01, 0x08, 0x05, 0x10];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_obus(crate::DecodeLimitThreshold::Max(1)),
    );

    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxObus
    ));
}

#[test]
fn unsupported_prefix_is_reported_before_later_obu_limit() {
    let bytes = [0x01, 0x14, 0x01, 0x08];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_obus(crate::DecodeLimitThreshold::Max(1)),
    );

    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::UnsupportedStructure {
            unsupported
        } if unsupported.reason() == DecodeUnsupportedReason::UnsupportedFrameObu
    ));
}

#[test]
fn malformed_suffix_is_reported_after_unsupported_prefix() {
    let bytes = [0x01, 0x14, 0x05, 0x10];

    let error = plan_byte_stream(&bytes, &DecodeOptions::default()).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::MalformedSource {
            issue
        } if issue.kind() == crate::DecodeSourceIssueKind::AnnexBParseError
    ));
}

#[test]
fn raw_frame_candidate_limit_is_checked_before_later_malformed_bytes() {
    let bytes = [0x01, 0x10, 0x01, 0x10, 0x05, 0x10];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_frames_to_decode(crate::DecodeLimitThreshold::Max(1)),
    );

    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxFramesToDecode
    ));
}

#[test]
fn raw_regular_tip_counts_toward_frame_candidate_limit() {
    let bytes = [0x01, 0x10, 0x01, 0x38];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_frames_to_decode(crate::DecodeLimitThreshold::Max(1)),
    );

    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxFramesToDecode
    ));
}
