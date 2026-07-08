// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::obu::{ParsedObu, PayloadStatus};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;

pub(crate) fn fixture_sequence_and_key_core(bytes: &[u8]) -> (SequenceHeader, FrameHeaderCore) {
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
    let key_core = crate::pipeline::parse_frame_core(key, &sequence).expect("parse key core");
    (sequence, key_core)
}
