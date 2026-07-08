// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_parallel::ThreadCount;

use super::*;
use crate::test_support::{MINIMAL_FIXTURE, minimal_fixture_with_timebase};
use crate::{
    DecodeContext, DecodeError, DecodeLimitName, DecodeLimitThreshold, DecodeLimits,
    DecodeRuntimeConfig,
};

const MONO_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-mono-intra-64x64.ivf");
const EXPECTED_DIGEST: &str = "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af";
const MONO_EXPECTED_DIGEST: &str =
    "2caad75a3f9a729187deee66d839eacf3a4705e62221cbdd6ece96c022334b6b";

fn context(threads: ThreadCount) -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
}

#[test]
fn minimal_fixture_decodes_to_hash_report() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(MINIMAL_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.contract_id, crate::DECODE_HASH_REPORT_CONTRACT_ID);
    assert_eq!(report.contract_version, 1);
    assert_eq!(report.selected_thread_policy, "1");
    assert_eq!(report.frames.len(), 1);
    let frame = &report.frames[0];
    assert_eq!(frame.output_index, 0);
    assert_eq!(frame.visible_luma_width, 64);
    assert_eq!(frame.visible_luma_height, 64);
    assert_eq!(frame.chroma_width, Some(32));
    assert_eq!(frame.chroma_height, Some(32));
    assert_eq!(frame.bit_depth, 8);
    assert_eq!(frame.pixel_format, DecodeHashPixelFormat::Yuv420);
    assert_eq!(frame.hashes.len(), 1);
    assert_eq!(frame.hashes[0].digest_hex, EXPECTED_DIGEST);
}

#[test]
fn zero_ivf_timebase_does_not_block_hash_output() {
    for input in [
        minimal_fixture_with_timebase(0, 30),
        minimal_fixture_with_timebase(1, 0),
    ] {
        let report = context(ThreadCount::from(1usize))
            .decode_hash_report_bytes(&input, DecodeOptions::default())
            .unwrap();

        assert_eq!(report.frames[0].hashes[0].digest_hex, EXPECTED_DIGEST);
    }
}

#[test]
fn hash_report_records_resolved_thread_count() {
    let context = context(ThreadCount::Auto);
    let resolved = context.threads().get().to_string();
    let report = context
        .decode_hash_report_bytes(MINIMAL_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.selected_thread_policy, resolved);
    assert_ne!(report.selected_thread_policy, "auto");
}

#[test]
fn monochrome_fixture_decodes_to_luma_only_hash_report() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(MONO_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    let frame = &report.frames[0];
    assert_eq!(frame.visible_luma_width, 64);
    assert_eq!(frame.visible_luma_height, 64);
    assert_eq!(frame.chroma_width, None);
    assert_eq!(frame.chroma_height, None);
    assert_eq!(frame.pixel_format, DecodeHashPixelFormat::Monochrome);
    assert_eq!(frame.hashes[0].digest_hex, MONO_EXPECTED_DIGEST);
}

#[test]
fn malformed_input_fails_before_hash_report() {
    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(&[0x01, 0x14, 0x05, 0x10], DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn raw_annex_b_payload_fails_closed_as_unsupported() {
    let ivf_payload = &MINIMAL_FIXTURE[44..];
    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(ivf_payload, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::UnsupportedFeature {
            unsupported
        } if unsupported.reason() == "non_ivf_input"
    ));
}

#[test]
fn tile_trace_mismatch_fails_closed_as_unsupported() {
    let mut bytes = MINIMAL_FIXTURE.to_vec();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(&bytes, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::UnsupportedFeature {
            unsupported
        } if unsupported.reason() == "inter_exit_symbol"
    ));
}

#[test]
fn partition_symbol_mutation_fails_closed_through_general_path() {
    let mut bytes = MINIMAL_FIXTURE.to_vec();
    let tile_start = bytes.len() - 2;
    bytes[tile_start] = 0xFF;

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(&bytes, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::UnsupportedFeature {
            unsupported
        } if unsupported.reason().starts_with("general_intra_")
    ));
}

#[test]
fn tile_payload_byte_mutations_return_typed_results() {
    let context = context(ThreadCount::from(1usize));
    let tile_payload_offsets = (MINIMAL_FIXTURE.len() - 2)..MINIMAL_FIXTURE.len();

    for offset in tile_payload_offsets {
        for value in u8::MIN..=u8::MAX {
            let mut bytes = MINIMAL_FIXTURE.to_vec();
            let original = bytes[offset];
            bytes[offset] = value;

            if let Ok(mut report) =
                context.decode_hash_report_bytes(&bytes, DecodeOptions::default())
            {
                assert_eq!(report.frames.len(), 1);
                let digest = report.frames.remove(0).hashes.remove(0).digest_hex;
                assert_eq!(digest.len(), 64);
                assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
                if value == original {
                    assert_eq!(digest, EXPECTED_DIGEST);
                }
            }
        }
    }
}

#[test]
fn output_byte_limit_fails_before_hash_report() {
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_output_bytes(DecodeLimitThreshold::Max(1)),
    );
    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(MINIMAL_FIXTURE, options)
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxOutputBytes
    ));
}

#[test]
fn partition_frontier_limit_preserves_resource_limit() {
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_tile_partition_steps(DecodeLimitThreshold::Max(0)),
    );
    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(MINIMAL_FIXTURE, options)
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxTilePartitionSteps
    ));
}

#[test]
fn decoded_hash_is_deterministic_across_thread_policies() {
    let digest = |threads| {
        context(threads)
            .decode_hash_report_bytes(MINIMAL_FIXTURE, DecodeOptions::default())
            .unwrap()
            .frames
            .remove(0)
            .hashes
            .remove(0)
            .digest_hex
    };

    assert_eq!(digest(ThreadCount::from(1usize)), EXPECTED_DIGEST);
    assert_eq!(digest(ThreadCount::Auto), EXPECTED_DIGEST);
    assert_eq!(digest(ThreadCount::from(2usize)), EXPECTED_DIGEST);
}
