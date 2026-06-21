// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier runtime raw sample-byte adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.

use splot_recon::DecodedFrameHashInput;

use crate::error::{DecodeOutputError, DecodeOutputOperation, Result};
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn encode_raw_stream_from_plan(
    bitstream: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<u8>> {
    // Decode every displayed frame in output order and concatenate their raw visible
    // sample bytes (AV2 § 6.18). A single-frame intra stream emits one frame,
    // byte-identical to the prior single-frame behavior.
    let outputs =
        crate::runtime_minimal::decode_minimal_frames_from_plan(bitstream, options, plan)?;
    let mut bytes = Vec::new();
    for output in &outputs {
        let raw = DecodedFrameHashInput::new(&output.frame);
        bytes.try_reserve_exact(raw.byte_len()?).map_err(|source| {
            DecodeOutputError::io(
                DecodeOutputOperation::SerializeRaw,
                std::io::Error::other(format!("raw output allocation failed: {source}")),
            )
        })?;
        raw.write_to(&mut bytes)
            .map_err(|source| DecodeOutputError::io(DecodeOutputOperation::SerializeRaw, source))?;
    }
    options
        .limits()
        .ensure(crate::DecodeLimitName::MaxOutputBytes, bytes.len() as u64)?;
    Ok(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io;

    use splot_parallel::ThreadCount;

    use crate::{
        DecodeContext, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
        DecodeLimitName, DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeOutputOperation,
        DecodeRuntimeConfig, OUTPUT_ERROR_RULE_ID,
    };

    const MINIMAL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");
    const BROAD_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-key-intra-64x64.ivf");

    fn context(threads: ThreadCount) -> DecodeContext {
        DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
    }

    // The decoded raw planar output for the committed conformant luma-skip
    // fixture: luma is an all-zero (skipped) DC block (flat 128) while chroma
    // carries a real coded residual, so it is no longer flat. avmdec and dav2d
    // both decode the fixture to these exact bytes (see
    // docs/LOCAL-REFERENCE-EVIDENCE.toml); the reference is committed alongside
    // the fixture.
    fn expected_minimal_raw() -> Vec<u8> {
        include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw")
            .to_vec()
    }

    fn minimal_fixture_with_timebase(numerator: u32, denominator: u32) -> Vec<u8> {
        let mut bytes = MINIMAL_FIXTURE.to_vec();
        bytes[16..20].copy_from_slice(&denominator.to_le_bytes());
        bytes[20..24].copy_from_slice(&numerator.to_le_bytes());
        bytes
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
    fn broader_fixture_fails_closed_as_unsupported_for_raw() {
        let mut bytes = Vec::new();

        let error = context(ThreadCount::from(1usize))
            .decode_raw_bytes(BROAD_FIXTURE, DecodeOptions::default(), &mut bytes)
            .unwrap_err();

        assert!(bytes.is_empty());
        assert!(matches!(
            error,
            DecodeError::UnsupportedFeature {
                unsupported
            } if unsupported.tier_id() == crate::runtime_minimal::MINIMAL_INTRA_HASH_TIER_ID
        ));
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
        assert_eq!(
            report.diagnostic.matrix_row,
            "decode-minimal-raw-runtime-output"
        );
        assert_eq!(
            report.diagnostic.feature_id,
            "DECODE-MINIMAL-RAW-RUNTIME-OUTPUT"
        );
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
