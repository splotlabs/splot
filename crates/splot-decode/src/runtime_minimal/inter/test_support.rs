// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::obu::{ParsedObu, PayloadStatus};
use splot_core::span::ByteOffset;
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;

use crate::error::DecodeError;

#[derive(Clone, Copy, Debug)]
pub(super) struct UnsupportedFeatureExpectation {
    pub(super) reason: &'static str,
    pub(super) matrix_row: &'static str,
    pub(super) feature_id: &'static str,
    pub(super) spec_section: &'static str,
    pub(super) byte_offset: ByteOffset,
    pub(super) message_fragments: &'static [&'static str],
}

impl UnsupportedFeatureExpectation {
    pub(super) fn at_byte_offset(
        reason: &'static str,
        matrix_row: &'static str,
        feature_id: &'static str,
        spec_section: &'static str,
        byte_offset: ByteOffset,
        message_fragments: &'static [&'static str],
    ) -> Self {
        Self {
            reason,
            matrix_row,
            feature_id,
            spec_section,
            byte_offset,
            message_fragments,
        }
    }
}

pub(super) fn assert_unsupported_feature(
    error: DecodeError,
    context: &'static str,
    expected: UnsupportedFeatureExpectation,
) {
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("{context} must be an unsupported-feature error");
    };
    assert_eq!(unsupported.reason(), expected.reason);
    assert_eq!(unsupported.matrix_row(), expected.matrix_row);
    assert_eq!(unsupported.feature_id(), expected.feature_id);
    assert_eq!(unsupported.spec_section(), expected.spec_section);
    assert_eq!(unsupported.byte_offset(), Some(expected.byte_offset));
    for fragment in expected.message_fragments {
        assert!(
            unsupported.message().contains(fragment),
            "{context} message must contain {fragment:?}"
        );
    }
}

pub(super) fn fixture_sequence_and_key_core(bytes: &[u8]) -> (SequenceHeader, FrameHeaderCore) {
    let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(bytes) else {
        panic!("fixture is IVF");
    };
    assert!(parsed.error.is_none());
    assert!(parsed.warnings.is_empty());
    let obus = || parsed.frames.iter().flat_map(|frame| frame.obus.iter());
    let sequence = obus()
        .find_map(
            |envelope| match envelope.payload_status().expect("payload status") {
                PayloadStatus::Parsed(ParsedObu::SequenceHeader(sequence)) => {
                    Some((*sequence).clone())
                }
                _ => None,
            },
        )
        .expect("fixture carries a sequence header");
    let key = obus()
        .find(|envelope| envelope.header.obu_type == ObuType::ClosedLoopKey)
        .copied()
        .expect("fixture carries a closed-loop-key frame");
    let key_core = super::super::parse_frame_core(key, &sequence).expect("parse key core");
    (sequence, key_core)
}
