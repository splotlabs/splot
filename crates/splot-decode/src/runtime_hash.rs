// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal-tier runtime hash adapter.
//!
//! Feature tracking: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

use core::num::NonZeroUsize;

use splot_recon::{DecodedFrame, DecodedFrameHashInput};

use crate::error::Result;
use crate::hash_report::{
    DecodeHashEntry, DecodeHashFrame, DecodeHashPixelFormat, DecodeHashReport,
};
use crate::{DecodeOptions, DecodeStreamPlan};

pub(crate) fn decode_hash_report_from_plan(
    bytes: &[u8],
    options: DecodeOptions,
    plan: &DecodeStreamPlan,
    resolved_threads: NonZeroUsize,
) -> Result<DecodeHashReport> {
    let output = crate::runtime_minimal::decode_minimal_frame_from_plan(bytes, options, plan)?;
    let hash = DecodedFrameHashInput::new(&output.frame).compute_hash();
    let report_frame = hash_frame_from_decoded(&output.frame, hash.to_hex());

    Ok(DecodeHashReport::raw_intermediate_output(
        resolved_threads.to_string(),
        vec![report_frame],
    ))
}

fn hash_frame_from_decoded(frame: &DecodedFrame<u8>, digest_hex: String) -> DecodeHashFrame {
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
    use crate::{
        DecodeContext, DecodeError, DecodeLimitName, DecodeLimitThreshold, DecodeLimits,
        DecodeRuntimeConfig, DecodeUnsupportedFeature,
    };

    const MINIMAL_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");
    const BROAD_FIXTURE: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-key-intra-64x64.ivf");
    const EXPECTED_DIGEST: &str =
        "cb11e05cb5da949c0e0f5b5a7cb310df35a96a22c45d1ada70d950859fe697d1";

    fn context(threads: ThreadCount) -> DecodeContext {
        DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
    }

    fn minimal_fixture_with_timebase(numerator: u32, denominator: u32) -> Vec<u8> {
        let mut bytes = MINIMAL_FIXTURE.to_vec();
        bytes[16..20].copy_from_slice(&denominator.to_le_bytes());
        bytes[20..24].copy_from_slice(&numerator.to_le_bytes());
        bytes
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
            } if unsupported.reason().starts_with("minimal_tile_")
                || unsupported.reason().ends_with("_all_zero_transform")
                || unsupported.reason().starts_with("intra_")
                || unsupported.reason() == "uv_mode_index"
        ));
    }

    #[test]
    fn partition_symbol_mutation_fails_through_frontier() {
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
            } if unsupported.reason() == "minimal_tile_partition_frontier"
        ));
    }

    #[test]
    fn tile_payload_byte_mutations_return_typed_results() {
        let context = context(ThreadCount::from(1usize));
        let tile_payload_offsets = (MINIMAL_FIXTURE.len() - 2)..MINIMAL_FIXTURE.len();

        for offset in tile_payload_offsets {
            for value in u8::MIN..=u8::MAX {
                let mut bytes = MINIMAL_FIXTURE.to_vec();
                bytes[offset] = value;

                if let Ok(mut report) =
                    context.decode_hash_report_bytes(&bytes, DecodeOptions::default())
                {
                    assert_eq!(report.frames.len(), 1);
                    let digest = report.frames.remove(0).hashes.remove(0).digest_hex;
                    assert_eq!(digest, EXPECTED_DIGEST);
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
