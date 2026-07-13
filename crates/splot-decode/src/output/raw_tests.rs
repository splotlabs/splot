// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use std::io;

use splot_core::bitio::BitReader;
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::ivf::{IVF_FRAME_HEADER_SIZE, IVF_HEADER_SIZE};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;
use splot_parallel::ThreadCount;

use crate::test_support::{MINIMAL_FIXTURE, empty_avmenc_ivf, minimal_fixture_with_timebase};
use crate::{
    DecodeContext, DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeError,
    DecodeLayerSelection, DecodeLimitName, DecodeLimitThreshold, DecodeLimits, DecodeOptions,
    DecodeOutputOperation, DecodePlannedObuRole, DecodeRuntimeConfig, DecodeSourceIssueKind,
    OUTPUT_ERROR_RULE_ID,
};

const MONO_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-mono-intra-64x64.ivf");
const TEMPORAL_LAYER_DECLARATION_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-tlayer2-base-only-intra-64x64.ivf"
);
const TEMPORAL_LAYER_STREAM_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-tlayer2-base-and-enhancement-64x64.obu"
);
const SEF_FAMILIES_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-frame-sef-families-64x64.ivf");
const MONO_LUMA_BYTES: usize = 64 * 64;
const TEMPORAL_LAYER_DECLARATION_AVM_DIGEST: &str =
    "ae245dff6e5b9272ba039d820c58fc77ed9d1184f031fa103b0ee101914eff32";
const TEMPORAL_LAYER_STREAM_BASE_AVM_DIGEST: &str =
    "ce03721c5041e45d02106592c6c55db2bcda0d651568729b92f4ef5d4a665774";

fn context(threads: ThreadCount) -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
}

fn expected_minimal_raw() -> Vec<u8> {
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw")
        .to_vec()
}

fn minimal_fixture_with_advisory_metadata_and_empty_record() -> Vec<u8> {
    let header_len = usize::from(IVF_HEADER_SIZE);
    let mut input = MINIMAL_FIXTURE[..header_len].to_vec();
    input[12..16].fill(0);
    input[24..28].copy_from_slice(&9u32.to_le_bytes());
    input.extend_from_slice(&[0; IVF_FRAME_HEADER_SIZE]);
    input.extend_from_slice(&MINIMAL_FIXTURE[header_len..]);
    input
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
fn shared_show_existing_outputs_do_not_consume_reference_store_bytes() {
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_reference_store_bytes(DecodeLimitThreshold::Max(36_864)),
    );
    let mut bytes = Vec::new();

    context(ThreadCount::from(1usize))
        .decode_raw_bytes(SEF_FAMILIES_FIXTURE, options, &mut bytes)
        .unwrap();

    assert_eq!(bytes.len(), 24_576);
}

#[test]
fn advisory_ivf_metadata_and_empty_record_preserve_raw_output() {
    let mut bytes = Vec::new();

    context(ThreadCount::from(1usize))
        .decode_raw_bytes(
            &minimal_fixture_with_advisory_metadata_and_empty_record(),
            DecodeOptions::default(),
            &mut bytes,
        )
        .unwrap();

    assert_eq!(bytes, expected_minimal_raw());
}

#[test]
fn empty_ivf_record_is_charged_to_the_record_limit() {
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_ivf_frame_records(DecodeLimitThreshold::Max(1)),
    );
    let mut bytes = Vec::new();

    let error = context(ThreadCount::from(1usize))
        .decode_raw_bytes(
            &minimal_fixture_with_advisory_metadata_and_empty_record(),
            options,
            &mut bytes,
        )
        .unwrap_err();

    assert!(bytes.is_empty());
    assert!(matches!(
        error,
        DecodeError::Limit { source } if source.name() == DecodeLimitName::MaxIvfFrameRecords
    ));
}

#[test]
fn non_av2_ivf_codec_is_a_typed_source_rejection() {
    let mut input = MINIMAL_FIXTURE.to_vec();
    input[8..12].copy_from_slice(b"VP90");
    let mut bytes = Vec::new();

    let error = context(ThreadCount::from(1usize))
        .decode_raw_bytes(&input, DecodeOptions::default(), &mut bytes)
        .unwrap_err();

    assert!(bytes.is_empty());
    let report = DecodeDiagnosticReport::from_decode_error(&error).unwrap();
    assert!(matches!(
        report.details,
        DecodeDiagnosticDetails::MalformedSource(_)
    ));
    let DecodeDiagnosticDetails::MalformedSource(details) = report.details else {
        return;
    };
    assert_eq!(
        details.source_issue_kind,
        DecodeSourceIssueKind::IvfUnsupportedCodec.as_str()
    );
    assert_eq!(details.parser_rule_id, Some("decode/unsupported-ivf-codec"));
    assert!(matches!(
        error,
        DecodeError::MalformedSource { issue }
            if issue.kind() == DecodeSourceIssueKind::IvfUnsupportedCodec
                && issue.rule_id() == Some("decode/unsupported-ivf-codec")
                && issue.offset() == Some(splot_core::span::ByteOffset::new(8))
    ));
}

#[test]
fn trailing_partial_ivf_header_preserves_complete_frame_output() {
    let mut input = MINIMAL_FIXTURE.to_vec();
    input.push(0xAA);
    let mut bytes = Vec::new();

    context(ThreadCount::from(1usize))
        .decode_raw_bytes(&input, DecodeOptions::default(), &mut bytes)
        .unwrap();

    assert_eq!(bytes, expected_minimal_raw());
}

#[test]
fn truncated_ivf_payload_fails_before_raw_output() {
    let mut input = MINIMAL_FIXTURE.to_vec();
    let declared_size = u32::from_le_bytes(input[32..36].try_into().unwrap());
    input[32..36].copy_from_slice(&(declared_size + 1).to_le_bytes());
    let mut bytes = Vec::new();

    let error = context(ThreadCount::from(1usize))
        .decode_raw_bytes(&input, DecodeOptions::default(), &mut bytes)
        .unwrap_err();

    assert!(bytes.is_empty());
    assert!(matches!(
        error,
        DecodeError::MalformedSource { issue }
            if issue.kind() == DecodeSourceIssueKind::IvfContainerError
                && issue.rule_id().is_some()
    ));
}

#[test]
fn empty_ivf_decodes_to_zero_raw_bytes_with_zero_output_limits() {
    let input = empty_avmenc_ivf();
    let limits = DecodeLimits::default()
        .with_max_obus(DecodeLimitThreshold::Max(0))
        .with_max_ivf_frame_records(DecodeLimitThreshold::Max(0))
        .with_max_frames_to_decode(DecodeLimitThreshold::Max(0))
        .with_max_output_frames(DecodeLimitThreshold::Max(0))
        .with_max_output_bytes(DecodeLimitThreshold::Max(0));

    for threads in [
        ThreadCount::from(1usize),
        ThreadCount::Auto,
        ThreadCount::from(2usize),
    ] {
        let mut bytes = Vec::new();
        context(threads)
            .decode_raw_bytes(&input, DecodeOptions::new(limits), &mut bytes)
            .unwrap();
        assert!(bytes.is_empty());
    }
}

#[test]
fn raw_annex_b_payload_decodes_to_exact_raw_bytes() {
    let mut bytes = Vec::new();

    context(ThreadCount::from(1usize))
        .decode_raw_bytes(&MINIMAL_FIXTURE[44..], DecodeOptions::default(), &mut bytes)
        .unwrap();

    assert_eq!(bytes, expected_minimal_raw());
}

#[test]
fn higher_declared_temporal_layer_decodes_selected_base_layer_to_reference_output() {
    let context = context(ThreadCount::from(1usize));
    let plan = context
        .plan_bytes(TEMPORAL_LAYER_DECLARATION_FIXTURE, DecodeOptions::default())
        .unwrap();
    assert_eq!(plan.selected_layer(), DecodeLayerSelection::base());
    assert_eq!(plan.obu_count(), 3);
    assert_eq!(plan.frame_candidate_count(), 1);
    assert!(plan.frame_candidates_all().all(|candidate| {
        candidate.header().temporal_layer_id.get() == 0
            && candidate.header().embedded_layer_id.get() == 0
            && candidate.header().extended_layer_id.get() == 0
    }));

    let parsed = parse_bitstream_partial(TEMPORAL_LAYER_DECLARATION_FIXTURE);
    assert!(matches!(parsed, ParsedBitstream::Ivf(_)));
    let ParsedBitstream::Ivf(parsed) = parsed else {
        return;
    };
    let sequence = parsed
        .frames
        .iter()
        .flat_map(|frame| &frame.obus)
        .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
        .unwrap();
    let mut reader = BitReader::new(sequence.payload, sequence.payload_offset());
    let sequence = parse_sequence_header(&mut reader).unwrap();
    assert_eq!(sequence.general.max_tlayer_id.get(), 1);
    assert_eq!(sequence.general.max_mlayer_id.get(), 0);

    let report = context
        .decode_hash_report_bytes(TEMPORAL_LAYER_DECLARATION_FIXTURE, DecodeOptions::default())
        .unwrap();
    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        TEMPORAL_LAYER_DECLARATION_AVM_DIGEST
    );

    let mut raw = Vec::new();
    context
        .decode_raw_bytes(
            TEMPORAL_LAYER_DECLARATION_FIXTURE,
            DecodeOptions::default(),
            &mut raw,
        )
        .unwrap();
    assert_eq!(raw.len(), 64 * 64 + 2 * 32 * 32);
}

#[test]
fn non_base_temporal_layer_is_retained_without_runtime_output() {
    let context = context(ThreadCount::from(1usize));
    let plan = context
        .plan_bytes(TEMPORAL_LAYER_STREAM_FIXTURE, DecodeOptions::default())
        .unwrap();
    let unselected: Vec<_> = plan
        .obus()
        .filter(|obu| obu.role() == DecodePlannedObuRole::UnselectedLayer)
        .collect();

    assert_eq!(plan.obu_count(), 5);
    assert_eq!(plan.frame_candidate_count(), 1);
    assert_eq!(unselected.len(), 1);
    assert_eq!(unselected[0].header().temporal_layer_id.get(), 1);

    let report = context
        .decode_hash_report_bytes(TEMPORAL_LAYER_STREAM_FIXTURE, DecodeOptions::default())
        .unwrap();
    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        TEMPORAL_LAYER_STREAM_BASE_AVM_DIGEST
    );

    let mut raw = Vec::new();
    context
        .decode_raw_bytes(
            TEMPORAL_LAYER_STREAM_FIXTURE,
            DecodeOptions::default(),
            &mut raw,
        )
        .unwrap();
    assert_eq!(raw.len(), 64 * 64 + 2 * 32 * 32);
}

#[test]
fn higher_declared_temporal_layer_fixture_eof_fails_closed() {
    let truncated =
        &TEMPORAL_LAYER_DECLARATION_FIXTURE[..TEMPORAL_LAYER_DECLARATION_FIXTURE.len() - 1];

    let error = context(ThreadCount::from(1usize))
        .plan_bytes(truncated, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
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
