// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use std::io;

use splot_parallel::ThreadCount;

use crate::test_support::{MINIMAL_FIXTURE, empty_avmenc_ivf, minimal_fixture_with_timebase};
use crate::{
    DecodeContext, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
    DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeRuntimeConfig, OUTPUT_ERROR_RULE_ID,
};

const MONO_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-mono-intra-64x64.ivf");

fn context(threads: ThreadCount) -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
}

fn expected_minimal_y4m() -> Vec<u8> {
    let mut bytes = b"YUV4MPEG2 W64 H64 F30:1 Ip A0:0 C420\nFRAME\n".to_vec();
    bytes.extend_from_slice(include_bytes!(
        "../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw"
    ));
    bytes
}

fn expected_mono_y4m_header() -> &'static [u8] {
    b"YUV4MPEG2 W64 H64 F1:1 Ip A0:0 Cmono\nFRAME\n"
}

#[test]
fn minimal_fixture_decodes_to_exact_y4m_bytes() {
    let mut bytes = Vec::new();

    context(ThreadCount::from(1usize))
        .decode_y4m_bytes(MINIMAL_FIXTURE, DecodeOptions::default(), &mut bytes)
        .unwrap();

    assert_eq!(bytes, expected_minimal_y4m());
}

#[test]
fn empty_ivf_y4m_fails_with_typed_empty_frame_set_error() {
    let input = empty_avmenc_ivf();
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::Max(0)),
    );
    let mut bytes = Vec::new();

    let error = context(ThreadCount::from(1usize))
        .decode_y4m_bytes(&input, options, &mut bytes)
        .unwrap_err();

    assert!(bytes.is_empty());
    assert!(matches!(
        error,
        DecodeError::Output { ref source }
            if source.operation() == crate::DecodeOutputOperation::SerializeY4m
                && source.source_kind() == "frame_set"
                && source.source_message().contains("at least one decoded frame")
    ));
}

#[test]
fn raw_annex_b_payload_y4m_fails_without_timebase() {
    let error = context(ThreadCount::from(1usize))
        .decode_y4m_bytes(&MINIMAL_FIXTURE[44..], DecodeOptions::default(), Vec::new())
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::UnsupportedFeature {
            unsupported
        } if unsupported.reason() == "annex_b_y4m_timebase"
    ));
}

#[test]
fn monochrome_fixture_decodes_to_luma_only_y4m_bytes() {
    let mut raw = Vec::new();
    let mut bytes = Vec::new();

    context(ThreadCount::from(1usize))
        .decode_raw_bytes(MONO_FIXTURE, DecodeOptions::default(), &mut raw)
        .unwrap();
    context(ThreadCount::from(1usize))
        .decode_y4m_bytes(MONO_FIXTURE, DecodeOptions::default(), &mut bytes)
        .unwrap();

    let header = expected_mono_y4m_header();
    assert_eq!(&bytes[..header.len()], header);
    assert_eq!(&bytes[header.len()..], raw.as_slice());
}

#[test]
fn zero_ivf_timebase_fails_as_y4m_output_diagnostic_before_writer_success() {
    for input in [
        minimal_fixture_with_timebase(0, 30),
        minimal_fixture_with_timebase(1, 0),
    ] {
        let mut bytes = Vec::new();

        let error = context(ThreadCount::from(1usize))
            .decode_y4m_bytes(&input, DecodeOptions::default(), &mut bytes)
            .unwrap_err();

        assert!(bytes.is_empty());
        assert!(matches!(
            error,
            DecodeError::Output {
                ref source
            } if source.operation().as_str() == "serialize_y4m"
                && source.source_kind() == "y4m"
        ));

        let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();
        assert_eq!(report.diagnostic.rule_id, OUTPUT_ERROR_RULE_ID);
        assert_eq!(report.diagnostic.spec_section, None);
        assert!(matches!(
            &report.details,
            DecodeDiagnosticDetails::OutputError(_)
        ));
        let DecodeDiagnosticDetails::OutputError(details) = report.details else {
            return;
        };
        assert_eq!(details.operation, "serialize_y4m");
        assert_eq!(details.source_kind, "y4m");
        assert!(details.source_message.contains("invalid Y4M frame rate"));
    }
}

#[test]
fn y4m_output_is_deterministic_across_thread_policies() {
    let decode = |threads| {
        let mut bytes = Vec::new();
        context(threads)
            .decode_y4m_bytes(MINIMAL_FIXTURE, DecodeOptions::default(), &mut bytes)
            .unwrap();
        bytes
    };
    let expected = expected_minimal_y4m();

    assert_eq!(decode(ThreadCount::from(1usize)), expected);
    assert_eq!(decode(ThreadCount::Auto), expected);
    assert_eq!(decode(ThreadCount::from(2usize)), expected);
}

#[test]
fn caller_writer_io_error_maps_to_output_diagnostic() {
    struct FailingWriter;

    impl io::Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed writer"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let error = context(ThreadCount::from(1usize))
        .decode_y4m_bytes(MINIMAL_FIXTURE, DecodeOptions::default(), FailingWriter)
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::Output {
            ref source
        } if source.operation().as_str() == "write_y4m_stream"
            && source.source_kind() == "io"
    ));

    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();
    assert_eq!(report.diagnostic.rule_id, OUTPUT_ERROR_RULE_ID);
    assert_eq!(report.diagnostic.spec_section, None);
    assert!(matches!(
        &report.details,
        DecodeDiagnosticDetails::OutputError(_)
    ));
    let DecodeDiagnosticDetails::OutputError(details) = report.details else {
        return;
    };
    assert_eq!(details.operation, "write_y4m_stream");
    assert_eq!(details.source_kind, "io");
    assert!(details.source_message.contains("closed writer"));
}
