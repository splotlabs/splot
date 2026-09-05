// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use std::sync::OnceLock;

#[path = "../support/output.rs"]
mod output;
use output::FailAfterBytes;
#[path = "../support/capture.rs"]
mod capture;
use capture::BoundedCaptureWriter;

use libfuzzer_sys::fuzz_target;
use splot_decode::{
    DecodeContext, DecodeError, DecodeOptions, DecodeOutputOperation, DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

#[path = "../support/decode_runtime.rs"]
mod decode_runtime;

const FIXTURE_MODE_FLAG: u8 = 0b1000_0000;
const FAILING_WRITER_FLAG: u8 = 0b0100_0000;
const MAX_RAW_INPUT_BYTES: usize = 4096;
const MAX_CAPTURE_BYTES: usize = 16 * 1024;
const Y4M_MAGIC: &[u8] = b"YUV4MPEG2 ";
const FRAME_MARKER: &[u8] = b"\nFRAME\n";
const MINIMAL_LUMA_BYTES: usize = 64 * 64;
const MINIMAL_CHROMA_BYTES: usize = 32 * 32;
const MINIMAL_PAYLOAD_BYTES: usize = MINIMAL_LUMA_BYTES + MINIMAL_CHROMA_BYTES * 2;
const MINIMAL_LUMA_SAMPLE: u8 = 128;
const MINIMAL_EXPECTED_RAW: &[u8] =
    include_bytes!("../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw");

static CONTEXT: OnceLock<Option<DecodeContext>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let (flags, payload) = match data.split_first() {
        Some((flags, payload)) => (*flags, payload),
        None => (0, &[][..]),
    };

    let Some(context) = CONTEXT
        .get_or_init(|| {
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).ok()
        })
        .as_ref()
    else {
        return;
    };

    let fixture_bytes;
    let bitstream = if flags & FIXTURE_MODE_FLAG == 0 {
        let len = payload.len().min(MAX_RAW_INPUT_BYTES);
        &payload[..len]
    } else {
        fixture_bytes = decode_runtime::mutated_minimal_fixture(payload);
        fixture_bytes.as_slice()
    };

    let options = DecodeOptions::new(decode_runtime::limits(flags, bitstream.len()));
    if flags & FAILING_WRITER_FLAG == 0 {
        let mut writer = BoundedCaptureWriter::new(MAX_CAPTURE_BYTES);
        if context
            .decode_y4m_bytes(bitstream, options, &mut writer)
            .is_ok()
        {
            if bitstream == decode_runtime::MINIMAL_FIXTURE {
                assert_minimal_y4m_shape(writer.bytes());
            }
        }
    } else {
        let mut writer = FailAfterBytes::new(usize::from(payload.first().copied().unwrap_or(0)));
        if let Err(DecodeError::Output { source }) =
            context.decode_y4m_bytes(bitstream, options, &mut writer)
        {
            assert_failing_writer_output_error(&source);
        }
    }
});

fn assert_failing_writer_output_error(source: &splot_decode::DecodeOutputError) {
    match source.source_kind() {
        "io" => assert_eq!(source.operation(), DecodeOutputOperation::WriteY4mStream),
        "y4m" | "frame_set" => assert_eq!(source.operation(), DecodeOutputOperation::SerializeY4m),
        kind => panic!("unexpected runtime Y4M output source kind {kind}"),
    }
}

fn assert_minimal_y4m_shape(bytes: &[u8]) {
    assert!(bytes.starts_with(Y4M_MAGIC));
    let Some(frame_marker_offset) = find_subslice(bytes, FRAME_MARKER) else {
        panic!("runtime Y4M output is missing FRAME header");
    };
    let header = &bytes[..frame_marker_offset];
    let payload_start = frame_marker_offset + FRAME_MARKER.len();
    let payload = &bytes[payload_start..];

    assert_header_token(header, b"W64");
    assert_header_token(header, b"H64");
    assert_header_token(header, b"Ip");
    assert_header_token(header, b"A0:0");
    assert_header_token(header, b"C420");
    assert_nonzero_frame_rate(header);
    assert_eq!(payload.len(), MINIMAL_PAYLOAD_BYTES);
    let (luma, _) = payload.split_at(MINIMAL_LUMA_BYTES);
    assert!(luma.iter().all(|sample| *sample == MINIMAL_LUMA_SAMPLE));
    assert_eq!(payload, MINIMAL_EXPECTED_RAW);
    assert!(find_subslice(payload, FRAME_MARKER).is_none());
}

fn assert_header_token(header: &[u8], token: &[u8]) {
    assert!(
        header
            .split(|byte| *byte == b' ')
            .any(|candidate| candidate == token),
        "runtime Y4M header is missing token {}",
        core::str::from_utf8(token).unwrap_or("<non-utf8>")
    );
}

fn assert_nonzero_frame_rate(header: &[u8]) {
    let Some(token) = header
        .split(|byte| *byte == b' ')
        .find(|candidate| candidate.starts_with(b"F"))
    else {
        panic!("runtime Y4M header is missing frame-rate token");
    };
    let Some((numerator, denominator)) = split_once_byte(&token[1..], b':') else {
        panic!("runtime Y4M frame-rate token is malformed");
    };
    let numerator = parse_ascii_u32(numerator).unwrap_or(0);
    let denominator = parse_ascii_u32(denominator).unwrap_or(0);
    assert_ne!(numerator, 0);
    assert_ne!(denominator, 0);
}

fn parse_ascii_u32(bytes: &[u8]) -> Option<u32> {
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    core::str::from_utf8(bytes).ok()?.parse().ok()
}

fn split_once_byte(bytes: &[u8], needle: u8) -> Option<(&[u8], &[u8])> {
    let index = bytes.iter().position(|byte| *byte == needle)?;
    Some((&bytes[..index], &bytes[index + 1..]))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}
