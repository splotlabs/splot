// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier Y4M adapter.
//!
//! Feature tracking: `DECODE-Y4M-RUNTIME-OUTPUT`.

use splot_core::ivf::IvfHeader;
use splot_recon::{
    BitDepth, DecodedFrame, PixelFormat, PlaneSize, ReconSample, Y4mFrameFormat, Y4mFrameHeader,
    Y4mFrameRate, Y4mStreamHeader, Y4mWriter,
};

use crate::error::{
    DecodeError, DecodeOutputError, DecodeOutputOperation, DecodeUnsupportedFeature, Result,
};
use crate::pipeline::PipelineDecodedFrame;
use crate::{
    DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeLimits, DecodeOptions, DecodeStreamPlan,
};

const MATRIX_ROW: &str = "decode-y4m-runtime-output";
const FEATURE_ID: &str = "DECODE-Y4M-RUNTIME-OUTPUT";
const SPEC_SECTION: &str = "7.1";
const REMEDIATION: &str = "Use an IVF input with a nonzero timebase for runtime Y4M output.";
const MINIMAL_Y4M_LUMA_WIDTH: usize = 64;
const MINIMAL_Y4M_LUMA_HEIGHT: usize = 64;
const MINIMAL_Y4M_CHROMA_WIDTH: u64 = 32;
const MINIMAL_Y4M_CHROMA_HEIGHT: u64 = 32;

pub(crate) fn encode_y4m_stream_from_plan(
    bytes: &[u8],
    options: &DecodeOptions,
    plan: &DecodeStreamPlan,
) -> Result<Vec<u8>> {
    let limits = options.limits();
    let outputs = crate::pipeline::decode_frames_from_plan_with_ivf_preflight(
        bytes,
        options,
        plan,
        |header| preflight_y4m_minimal_header(header, limits),
    )?;
    let first = outputs
        .first()
        .ok_or_else(|| DecodeError::UnsupportedFeature {
            unsupported: Box::new(DecodeUnsupportedFeature::new(
                "empty_decoded_frame_set",
                crate::pipeline::MINIMAL_INTRA_HASH_TIER_ID,
                MATRIX_ROW,
                FEATURE_ID,
                SPEC_SECTION,
                "runtime Y4M output requires at least one decoded frame",
                REMEDIATION,
                None,
            )),
        })?;
    ensure_y4m_timebase(first.frame_rate_numerator, first.frame_rate_denominator)?;
    let frame_rate = Y4mFrameRate::new(first.frame_rate_numerator, first.frame_rate_denominator)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;

    let mut y4m = Vec::new();
    match &first.frame {
        PipelineDecodedFrame::Eight(first_frame) => {
            write_y4m_stream(
                &mut y4m,
                first_frame,
                frame_rate,
                &outputs,
                |output| match &output.frame {
                    PipelineDecodedFrame::Eight(frame) => Some(frame),
                    PipelineDecodedFrame::Ten(_) => None,
                },
            )?;
        }
        PipelineDecodedFrame::Ten(first_frame) => {
            write_y4m_stream(
                &mut y4m,
                first_frame,
                frame_rate,
                &outputs,
                |output| match &output.frame {
                    PipelineDecodedFrame::Ten(frame) => Some(frame),
                    PipelineDecodedFrame::Eight(_) => None,
                },
            )?;
        }
    }

    options
        .limits()
        .ensure(DecodeLimitName::MaxOutputBytes, y4m.len() as u64)?;
    Ok(y4m)
}

/// Writes the Y4M stream header (derived from `first_frame`) and one `FRAME`
/// payload per displayed frame (§ 6.18) of the sample type `T`. `select` maps a
/// [`PipelineFrame`] to its `DecodedFrame<T>` for this stream's depth; a
/// frame of a different depth (`None`) is rejected with a structured diagnostic.
fn write_y4m_stream<T: ReconSample>(
    y4m: &mut Vec<u8>,
    first_frame: &DecodedFrame<T>,
    frame_rate: Y4mFrameRate,
    outputs: &[crate::pipeline::PipelineFrame],
    select: impl Fn(&crate::pipeline::PipelineFrame) -> Option<&DecodedFrame<T>>,
) -> Result<()> {
    let mut writer = Y4mWriter::from_frame(y4m, first_frame, frame_rate)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    for output in outputs {
        let frame = select(output).ok_or_else(|| {
            DecodeError::UnsupportedFeature {
                unsupported: Box::new(DecodeUnsupportedFeature::new(
                    "y4m_mixed_bit_depth_frames",
                    crate::pipeline::MINIMAL_INTRA_HASH_TIER_ID,
                    MATRIX_ROW,
                    FEATURE_ID,
                    SPEC_SECTION,
                    "runtime Y4M output requires every displayed frame to share the first frame's sample bit depth",
                    REMEDIATION,
                    None,
                )),
            }
        })?;
        writer.write_frame(frame).map_err(|source| {
            DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source)
        })?;
    }
    writer
        .flush()
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    Ok(())
}

fn preflight_y4m_minimal_header(header: IvfHeader, limits: DecodeLimits) -> Result<()> {
    ensure_y4m_timebase(header.timebase_denominator, header.timebase_numerator)?;
    let frame_rate = Y4mFrameRate::new(header.timebase_denominator, header.timebase_numerator)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    ensure_minimal_y4m_output_limit(limits, frame_rate)
}

fn ensure_y4m_timebase(numerator: u32, denominator: u32) -> Result<()> {
    if numerator == 0 || denominator == 0 {
        Err(DecodeError::UnsupportedFeature {
            unsupported: Box::new(DecodeUnsupportedFeature::new(
                "invalid_ivf_timebase",
                crate::pipeline::MINIMAL_INTRA_HASH_TIER_ID,
                MATRIX_ROW,
                FEATURE_ID,
                SPEC_SECTION,
                "runtime Y4M output requires a nonzero IVF timebase",
                REMEDIATION,
                None,
            )),
        })
    } else {
        Ok(())
    }
}

fn ensure_minimal_y4m_output_limit(limits: DecodeLimits, frame_rate: Y4mFrameRate) -> Result<()> {
    let luma_size = PlaneSize::new(MINIMAL_Y4M_LUMA_WIDTH, MINIMAL_Y4M_LUMA_HEIGHT)?;
    let frame_format = Y4mFrameFormat::new(luma_size, BitDepth::Eight, PixelFormat::Yuv420)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;
    let stream_header = Y4mStreamHeader::new(frame_format, frame_rate);
    let mut stream_header_bytes = Vec::new();
    stream_header
        .write_to(&mut stream_header_bytes)
        .map_err(|source| DecodeOutputError::y4m(DecodeOutputOperation::SerializeY4m, source))?;

    let luma_bytes = checked_mul(
        DecodeLimitName::MaxOutputBytes,
        MINIMAL_Y4M_LUMA_WIDTH as u64,
        MINIMAL_Y4M_LUMA_HEIGHT as u64,
    )?;
    let chroma_bytes = checked_mul(
        DecodeLimitName::MaxOutputBytes,
        MINIMAL_Y4M_CHROMA_WIDTH,
        MINIMAL_Y4M_CHROMA_HEIGHT,
    )?;
    let chroma_plane_bytes = checked_mul(DecodeLimitName::MaxOutputBytes, chroma_bytes, 2)?;
    let payload_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        luma_bytes,
        chroma_plane_bytes,
    )?;
    let headers_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        stream_header_bytes.len() as u64,
        Y4mFrameHeader::new().as_bytes().len() as u64,
    )?;
    let total_bytes = checked_add(
        DecodeLimitName::MaxOutputBytes,
        headers_bytes,
        payload_bytes,
    )?;
    limits.ensure(DecodeLimitName::MaxOutputBytes, total_bytes)?;

    Ok(())
}

fn checked_add(
    name: DecodeLimitName,
    left: u64,
    right: u64,
) -> core::result::Result<u64, DecodeLimitError> {
    left.checked_add(right)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Add,
            left,
            right,
        })
}

fn checked_mul(
    name: DecodeLimitName,
    left: u64,
    right: u64,
) -> core::result::Result<u64, DecodeLimitError> {
    left.checked_mul(right)
        .ok_or(DecodeLimitError::ArithmeticOverflow {
            name,
            op: DecodeLimitOp::Mul,
            left,
            right,
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io;

    use splot_parallel::ThreadCount;

    use crate::test_support::{MINIMAL_FIXTURE, minimal_fixture_with_timebase};
    use crate::{
        DecodeContext, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
        DecodeLimitName, DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeRuntimeConfig,
        OUTPUT_ERROR_RULE_ID, UNSUPPORTED_FEATURE_RULE_ID,
    };

    const BROAD_FIXTURE: &[u8] =
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-key-intra-64x64.ivf");

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

    #[test]
    fn minimal_fixture_decodes_to_exact_y4m_bytes() {
        let mut bytes = Vec::new();

        context(ThreadCount::from(1usize))
            .decode_y4m_bytes(MINIMAL_FIXTURE, DecodeOptions::default(), &mut bytes)
            .unwrap();

        assert_eq!(bytes, expected_minimal_y4m());
    }

    #[test]
    fn output_byte_limit_fails_before_writer_success() {
        let expected = expected_minimal_y4m();
        let options = DecodeOptions::new(
            DecodeLimits::default()
                .with_max_output_bytes(DecodeLimitThreshold::Max(expected.len() as u64 - 1)),
        );
        let mut bytes = Vec::new();

        let error = context(ThreadCount::from(1usize))
            .decode_y4m_bytes(MINIMAL_FIXTURE, options, &mut bytes)
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
    fn broader_fixture_fails_closed_as_unsupported_for_y4m() {
        let mut bytes = Vec::new();

        let error = context(ThreadCount::from(1usize))
            .decode_y4m_bytes(BROAD_FIXTURE, DecodeOptions::default(), &mut bytes)
            .unwrap_err();

        assert!(bytes.is_empty());
        assert!(matches!(
            error,
            DecodeError::UnsupportedFeature {
                unsupported
            } if unsupported.tier_id() == crate::pipeline::MINIMAL_INTRA_HASH_TIER_ID
        ));
    }

    #[test]
    fn zero_ivf_timebase_fails_as_source_diagnostic_before_y4m_serialization() {
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
                DecodeError::UnsupportedFeature {
                    ref unsupported
                } if unsupported.reason() == "invalid_ivf_timebase"
            ));

            let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();
            assert_eq!(report.diagnostic.rule_id, UNSUPPORTED_FEATURE_RULE_ID);
            assert_eq!(report.diagnostic.spec_section, Some("7.1"));
            assert!(matches!(
                &report.details,
                DecodeDiagnosticDetails::UnsupportedFeature(_)
            ));
            let DecodeDiagnosticDetails::UnsupportedFeature(details) = report.details else {
                return;
            };
            assert_eq!(details.unsupported_reason, "invalid_ivf_timebase");
            assert_eq!(details.tier_id, crate::pipeline::MINIMAL_INTRA_HASH_TIER_ID);
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
}
