// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier runtime hash adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

use core::num::NonZeroUsize;

use splot_recon::{DecodedFrame, DecodedFrameHashInput, ReconSample};

use crate::error::Result;
use crate::hash_report::{
    DecodeHashEntry, DecodeHashFrame, DecodeHashPixelFormat, DecodeHashReport,
};
use crate::runtime_minimal::MinimalRuntimeDecodedFrame;
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn decode_hash_report_from_plan(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
    resolved_threads: NonZeroUsize,
) -> Result<DecodeHashReport> {
    let outputs = crate::runtime_minimal::decode_minimal_frames_from_plan(bytes, options, plan)?;
    let mut report_frames = Vec::with_capacity(outputs.len());
    for output in &outputs {
        let report_frame = match &output.frame {
            MinimalRuntimeDecodedFrame::Eight(frame) => {
                let hash = DecodedFrameHashInput::new(frame).compute_hash();
                hash_frame_from_decoded(frame, hash.to_hex())
            }
            MinimalRuntimeDecodedFrame::Ten(frame) => {
                let hash = DecodedFrameHashInput::new(frame).compute_hash();
                hash_frame_from_decoded(frame, hash.to_hex())
            }
        };
        report_frames.push(report_frame);
    }

    Ok(DecodeHashReport::raw_intermediate_output(
        resolved_threads.to_string(),
        report_frames,
    ))
}

fn hash_frame_from_decoded<T: ReconSample>(
    frame: &DecodedFrame<T>,
    digest_hex: String,
) -> DecodeHashFrame {
    let visible = frame.visible_luma_rect();
    let chroma = frame
        .pixel_format()
        .chroma_size(visible.size())
        .ok()
        .flatten();
    DecodeHashFrame {
        output_index: frame.output_index().get(),
        visible_luma_left: visible.x() as u32,
        visible_luma_top: visible.y() as u32,
        visible_luma_width: visible.width() as u32,
        visible_luma_height: visible.height() as u32,
        chroma_left: chroma.map(|_| (visible.x() / 2) as u32),
        chroma_top: chroma.map(|_| (visible.y() / 2) as u32),
        chroma_width: chroma.map(|size| size.width() as u32),
        chroma_height: chroma.map(|size| size.height() as u32),
        bit_depth: frame.bit_depth().bits(),
        pixel_format: DecodeHashPixelFormat::Yuv420,
        hashes: vec![DecodeHashEntry::raw_intermediate_output_sha256(digest_hex)],
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use splot_parallel::ThreadCount;

    use super::*;
    use crate::runtime_test_support::{MINIMAL_FIXTURE, minimal_fixture_with_timebase};
    use crate::{
        DecodeContext, DecodeError, DecodeLimitName, DecodeLimitThreshold, DecodeLimits,
        DecodeRuntimeConfig, DecodeUnsupportedFeature,
    };

    const BROAD_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-key-intra-64x64.ivf");
    const EXPECTED_DIGEST: &str =
        "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af";

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
    fn broader_fixture_fails_closed_as_unsupported() {
        let error = context(ThreadCount::from(1usize))
            .decode_hash_report_bytes(BROAD_FIXTURE, DecodeOptions::default())
            .unwrap_err();

        assert!(matches!(
            error,
            DecodeError::UnsupportedFeature {
                unsupported
            } if unsupported.tier_id() == crate::runtime_minimal::MINIMAL_INTRA_HASH_TIER_ID
        ));
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
    fn sequence_chroma_tools_fail_closed_before_hash() -> core::result::Result<(), String> {
        for reason in ["unsupported_cfl_intra", "unsupported_mhccp"] {
            let unsupported = one_bit_mutation_rejected_with_reason(reason)?;
            assert_eq!(unsupported.reason(), reason);
            assert_eq!(
                unsupported.tier_id(),
                crate::runtime_minimal::MINIMAL_INTRA_HASH_TIER_ID
            );
        }
        Ok(())
    }

    fn one_bit_mutation_rejected_with_reason(
        reason: &'static str,
    ) -> core::result::Result<DecodeUnsupportedFeature, String> {
        let context = context(ThreadCount::from(1usize));
        for byte_index in 0..MINIMAL_FIXTURE.len() {
            for bit_index in 0..8 {
                let mut bytes = MINIMAL_FIXTURE.to_vec();
                bytes[byte_index] ^= 1 << bit_index;

                if let Err(DecodeError::UnsupportedFeature { unsupported }) =
                    context.decode_hash_report_bytes(&bytes, DecodeOptions::default())
                    && unsupported.reason() == reason
                {
                    return Ok(unsupported.as_ref().clone());
                }
            }
        }
        Err(format!("no single-bit mutation reached {reason}"))
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
            } if unsupported.reason().starts_with("general_intra_")
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
}
