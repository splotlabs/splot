// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::DecodeUnsupportedReason;
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};

#[test]
fn prepared_ivf_keeps_obus_in_one_flat_arena() {
    let mut bytes = Vec::new();
    write_ivf_header(&mut bytes, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 2)).unwrap();
    write_ivf_frame(&mut bytes, 0, &[0x01, 0x08, 0x01, 0x04]).unwrap();
    write_ivf_frame(&mut bytes, 1, &[0x01, 0x10]).unwrap();

    let prepared = prepare_byte_stream(&bytes, &DecodeOptions::default()).unwrap();
    assert!(matches!(prepared.parsed(), FlatParsedBitstream::Ivf(_)));
    let FlatParsedBitstream::Ivf(ivf) = prepared.parsed() else {
        return;
    };

    assert_eq!(prepared.plan().obu_count(), 3);
    assert_eq!(ivf.obus.len(), 3);
    assert_eq!(ivf.frames[0].obus, 0..2);
    assert_eq!(ivf.frames[1].obus, 2..3);
    assert_eq!(ivf.frame_obus(&ivf.frames[0]).len(), 2);
    assert_eq!(ivf.frame_obus(&ivf.frames[1]).len(), 1);
}

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

    let bytes = [0x01, 0x50, 0x01, 0x08];
    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::UnsupportedStructure {
            unsupported
        } if unsupported.reason() == DecodeUnsupportedReason::MultistreamSelection
    ));
}

#[test]
fn malformed_suffix_is_reported_after_unsupported_prefix() {
    for bytes in [[0x01, 0x14, 0x05, 0x10], [0x01, 0x1D, 0x05, 0x10]] {
        let error = plan_byte_stream(&bytes, &DecodeOptions::default()).unwrap_err();

        assert!(matches!(
            error,
            crate::DecodeError::MalformedSource {
                issue
            } if issue.kind() == crate::DecodeSourceIssueKind::AnnexBParseError
        ));
    }
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
