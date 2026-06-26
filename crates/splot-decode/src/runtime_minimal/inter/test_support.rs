// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared inter-runtime test fixtures reused across the `inter` test submodules.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::obu::{ParsedObu, PayloadStatus};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;

/// Parses `bytes` (an IVF fixture) and returns its sequence header and the parsed
/// frame-header core of the first closed-loop-key frame.
pub(super) fn fixture_sequence_and_key_core(bytes: &[u8]) -> (SequenceHeader, FrameHeaderCore) {
    let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(bytes) else {
        panic!("fixture is IVF");
    };
    assert!(parsed.error.is_none());
    assert!(parsed.warnings.is_empty());
    let sequence = parsed
        .frames
        .iter()
        .flat_map(|frame| frame.obus.iter())
        .find_map(
            |envelope| match envelope.payload_status().expect("payload status") {
                PayloadStatus::Parsed(ParsedObu::SequenceHeader(sequence)) => {
                    Some((*sequence).clone())
                }
                _ => None,
            },
        )
        .expect("fixture carries a sequence header");
    let key = parsed
        .frames
        .iter()
        .flat_map(|frame| frame.obus.iter())
        .find(|envelope| envelope.header.obu_type == ObuType::ClosedLoopKey)
        .copied()
        .expect("fixture carries a closed-loop-key frame");
    let key_core = super::super::parse_frame_core(key, &sequence).expect("parse key core");
    (sequence, key_core)
}
