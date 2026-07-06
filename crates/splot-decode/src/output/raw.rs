// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier raw sample-byte adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.

use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::pipeline::PipelineDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn encode_raw_stream_from_plan(
    bitstream: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<u8>> {
    let decode_started = crate::timing::start();
    let outputs = crate::pipeline::decode_frames_from_plan(bitstream, options, plan)?;
    crate::timing::report("runtime_decode", decode_started);
    let serialize_started = crate::timing::start();
    let mut bytes = Vec::new();
    for output in &outputs {
        match &output.frame {
            PipelineDecodedFrame::Eight(frame) => write_raw_frame(frame, &mut bytes)?,
            PipelineDecodedFrame::Ten(frame) => write_raw_frame(frame, &mut bytes)?,
        }
    }
    crate::timing::report("raw_serialize", serialize_started);
    options
        .limits()
        .ensure(crate::DecodeLimitName::MaxOutputBytes, bytes.len() as u64)?;
    Ok(bytes)
}

/// Appends one frame's raw visible sample bytes (§ 6.18) to `bytes`, reserving
/// the exact byte length first so the write cannot partially fill on OOM. The
/// splot-recon serializer is generic over the sample type, so the same path
/// serializes 8-bit (`u8`, one byte/sample) and 10-bit (`u16`, 16-bit-LE/sample).
fn write_raw_frame<T: ReconSample>(frame: &DecodedFrame<T>, bytes: &mut Vec<u8>) -> Result<()> {
    let raw = DecodedFrameHashInput::new(frame);
    bytes.try_reserve_exact(raw.byte_len()?).map_err(|source| {
        DecodeOutputError::io(
            DecodeOutputOperation::SerializeRaw,
            std::io::Error::other(format!("raw output allocation failed: {source}")),
        )
    })?;
    raw.write_to(bytes)
        .map_err(|source| DecodeOutputError::io(DecodeOutputOperation::SerializeRaw, source))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io;

    use splot_parallel::ThreadCount;

    use crate::test_support::{MINIMAL_FIXTURE, minimal_fixture_with_timebase};
    use crate::{
        DecodeContext, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
        DecodeLimitName, DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeOutputOperation,
        DecodeRuntimeConfig, OUTPUT_ERROR_RULE_ID,
    };

    const MONO_FIXTURE: &[u8] =
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-mono-intra-64x64.ivf");
    const MONO_LUMA_BYTES: usize = 64 * 64;

    fn context(threads: ThreadCount) -> DecodeContext {
        DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
    }

    fn expected_minimal_raw() -> Vec<u8> {
        include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw"
        )
        .to_vec()
    }

    #[test]
    fn minimal_fixture_decodes_to_exact_raw_bytes() {
        let mut bytes = Vec::new();

        context(ThreadCount::from(1usize))
            .decode_raw_bytes(MINIMAL_FIXTURE, DecodeOptions::default(), &mut bytes)
            .unwrap();

        assert_eq!(bytes, expected_minimal_raw());
    }

    #[test]
    fn zero_ivf_timebase_does_not_block_raw_output() {
        for input in [
            minimal_fixture_with_timebase(0, 30),
            minimal_fixture_with_timebase(1, 0),
        ] {
            let mut bytes = Vec::new();

            context(ThreadCount::from(1usize))
                .decode_raw_bytes(&input, DecodeOptions::default(), &mut bytes)
                .unwrap();

            assert_eq!(bytes, expected_minimal_raw());
        }
    }

    #[test]
    fn output_byte_limit_fails_before_writer_success() {
        let expected = expected_minimal_raw();
        let options = DecodeOptions::new(
            DecodeLimits::default()
                .with_max_output_bytes(DecodeLimitThreshold::Max(expected.len() as u64 - 1)),
        );
        let mut bytes = Vec::new();

        let error = context(ThreadCount::from(1usize))
            .decode_raw_bytes(MINIMAL_FIXTURE, options, &mut bytes)
            .unwrap_err();

        assert!(bytes.is_empty());
        assert!(matches!(
            error,
            DecodeError::Limit {
                source
            } if source.name() == DecodeLimitName::MaxOutputBytes
        ));
    }

    #[test]
    fn monochrome_fixture_decodes_to_luma_only_raw_bytes() {
        let mut bytes = Vec::new();

        context(ThreadCount::from(1usize))
            .decode_raw_bytes(MONO_FIXTURE, DecodeOptions::default(), &mut bytes)
            .unwrap();

        assert_eq!(bytes.len(), MONO_LUMA_BYTES);
        assert_eq!(&bytes[..12], &[0x4c; 12]);
        assert_eq!(&bytes[12..22], &[0x92; 10]);
        assert_eq!(&bytes[MONO_LUMA_BYTES - 10..], &[0xaa; 10]);
    }

    #[test]
    fn raw_output_is_deterministic_across_thread_policies() {
        let decode = |threads| {
            let mut bytes = Vec::new();
            context(threads)
                .decode_raw_bytes(MINIMAL_FIXTURE, DecodeOptions::default(), &mut bytes)
                .unwrap();
            bytes
        };
        let expected = expected_minimal_raw();

        assert_eq!(decode(ThreadCount::from(1usize)), expected);
        assert_eq!(decode(ThreadCount::Auto), expected);
        assert_eq!(decode(ThreadCount::from(2usize)), expected);
    }

    #[test]
    fn caller_writer_io_error_maps_to_raw_output_diagnostic() {
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
            .decode_raw_bytes(MINIMAL_FIXTURE, DecodeOptions::default(), FailingWriter)
            .unwrap_err();

        assert!(matches!(
            error,
            DecodeError::Output {
                ref source
            } if source.operation() == DecodeOutputOperation::WriteRawStream
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
        assert_eq!(details.operation, "write_raw_stream");
        assert_eq!(details.source_kind, "io");
        assert!(details.source_message.contains("closed writer"));
    }
}
