// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use std::num::NonZeroU64;

use splot_parallel::ThreadCount;

use super::*;
use crate::test_support::{MINIMAL_FIXTURE, empty_avmenc_ivf, minimal_fixture_with_timebase};
use crate::{
    DecodeContext, DecodeError, DecodeLimitName, DecodeLimitThreshold, DecodeLimits,
    DecodeRuntimeConfig,
};

const MONO_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-mono-intra-64x64.ivf");
const ORDER_HINT_WRAP_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-orderhint-wrap-64x64.ivf");
const CROP_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-crop-intra-64x64-q80.ivf");
const GDF_PER_BLOCK_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-gdf-per-block-intra-128x128-q120.ivf"
);
const GDF_MULTI_TILE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-4tile-gdf-intra-128x128-q120.ivf"
);
const INTER_444_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2frame-inter-444-64x64-q128.ivf"
);
const REFERENCE_SCALING_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-2frame-refscale-inter-64x64-51x51-q80.ivf"
);
const DIRECTIONAL_TX_PARTITION_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-txpart-d135-intra-64x64-q100.ivf"
);
const RECT_NOEDGE_D113FOLLOW_CHROMA_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-rect-noedge-d113follow-chroma-64x64-q7.ivf"
);
const RECT_CHROMA_CHUNKS_444_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-rect-chroma-chunks-444-intra-128x64-q80.ivf"
);
const TEN_BIT_D45_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-d45-follow-intra-64x64-10bit-q128.ivf"
);
const TEN_BIT_D45_MONO_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-d45-mono-intra-64x64-10bit-q128.ivf"
);
const OUTPUT_EFFECT_MFH_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-output-mfh-64x64.ivf");
const OUTPUT_EFFECT_BRT_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-output-brt-64x64.ivf");
const OUTPUT_EFFECT_METADATA_GROUP_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-output-metadata-group-64x64.ivf"
);
const OUTPUT_EFFECT_METADATA_SHORT_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-output-metadata-short-prefix-suffix-64x64.ivf"
);
const OUTPUT_EFFECT_USER_QM_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-output-user-qm-64x64.ivf");
const OUTPUT_EFFECT_CI_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-output-ci-2frame-64x64.ivf");
const STANDALONE_OLK_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-standalone-olk-64x64.ivf");
const BRIDGE_CELU_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-bridge-celu-64x64.ivf");
const SCALED_BRIDGE_FIXTURE: &[u8] =
    include_bytes!("../../../../tests/conformance/vectors/valid/syn-bridge-scaled-box-64x64.ivf");
const SINGLE_PICTURE_BRIDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-bridge-single-picture-32x32.ivf"
);
const EXPECTED_DIGEST: &str = "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af";
const MONO_EXPECTED_DIGEST: &str =
    "2caad75a3f9a729187deee66d839eacf3a4705e62221cbdd6ece96c022334b6b";
const CROP_EXPECTED_DIGEST: &str =
    "db63b846c386f8b66acb6f4750abd436a4c2e5b9cbe63166d1c93f1fcc4e20b9";
const GDF_PER_BLOCK_EXPECTED_DIGEST: &str =
    "13af253aff07dbbdcd9cd9e904597376e5cdca89e4be02d6900419ad9dbaf599";
const GDF_MULTI_TILE_EXPECTED_DIGEST: &str =
    "3d74979b2e766b285c535c6ff68078b8175f8b835d6f8a4380f215cc19874107";
const INTER_444_EXPECTED_DIGEST: &str =
    "0a4ca451c7bf5d983e30b01f2d003aa3ba6f29281247fa1c924a1e60ab162fb8";
const REFERENCE_SCALING_EXPECTED_DIGESTS: [&str; 2] = [
    "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979",
    "eb2d49b9f7ab4b0d1ab105fd8fb3cfabea4ef49d27afcfb4a0761c604527e300",
];
const DIRECTIONAL_TX_PARTITION_EXPECTED_DIGEST: &str =
    "5fffbdc79140da104a1721ed649130f0a2409fadeeb58632cdba54a1add778a1";
const RECT_NOEDGE_D113FOLLOW_CHROMA_EXPECTED_DIGEST: &str =
    "f831430d302267653add61fbc5054c1f3ab193a10ab7529fbcda74be6cdff70e";
const RECT_CHROMA_CHUNKS_444_EXPECTED_DIGEST: &str =
    "7cbec0da6486a3ef302af168bcd58cdf5658acef05ffbfb64dbb14b068e6e3a6";
const TEN_BIT_D45_EXPECTED_DIGEST: &str =
    "176168529b5970cafe650525f68cdd34df4a9bd9cca1e784005248ec71aa7ed7";
const TEN_BIT_D45_MONO_EXPECTED_DIGEST: &str =
    "2efc7a2cdb39694755fb06ff31e70bc06b5711e26196d5a756daad43e546d6cc";

fn context(threads: ThreadCount) -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(threads)).unwrap()
}

fn assert_minimal_fixture_limit(limits: DecodeLimits, expected: DecodeLimitName) {
    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(MINIMAL_FIXTURE, DecodeOptions::new(limits))
        .unwrap_err();

    assert!(matches!(
        error,
        DecodeError::Limit { source } if source.name() == expected
    ));
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
fn output_effect_single_frame_fixtures_match_reference_hashes() {
    let fixtures = [
        (
            OUTPUT_EFFECT_MFH_FIXTURE,
            "e73a3b0168597953992650452b153d6d316f649254b2493864fb6d320a3d8f53",
        ),
        (
            OUTPUT_EFFECT_BRT_FIXTURE,
            "e73a3b0168597953992650452b153d6d316f649254b2493864fb6d320a3d8f53",
        ),
        (
            OUTPUT_EFFECT_METADATA_GROUP_FIXTURE,
            "e73a3b0168597953992650452b153d6d316f649254b2493864fb6d320a3d8f53",
        ),
        (
            OUTPUT_EFFECT_METADATA_SHORT_FIXTURE,
            "e73a3b0168597953992650452b153d6d316f649254b2493864fb6d320a3d8f53",
        ),
        (
            OUTPUT_EFFECT_USER_QM_FIXTURE,
            "0f43b4de99455e2cc4d1444290bdb07240f92f2a505211351ee0e3cb70826d86",
        ),
    ];

    for (fixture, expected) in fixtures {
        let report = context(ThreadCount::from(1usize))
            .decode_hash_report_bytes(fixture, DecodeOptions::default())
            .unwrap();
        assert_eq!(report.frames.len(), 1);
        assert_eq!(report.frames[0].hashes[0].digest_hex, expected);
    }
}

#[test]
fn content_interpretation_fixture_decodes_both_frames() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(OUTPUT_EFFECT_CI_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 2);
}

#[test]
fn empty_ivf_decodes_to_deterministic_empty_hash_report() {
    let input = empty_avmenc_ivf();
    for threads in [
        ThreadCount::from(1usize),
        ThreadCount::Auto,
        ThreadCount::from(2usize),
    ] {
        let report = context(threads)
            .decode_hash_report_bytes(&input, DecodeOptions::default())
            .unwrap();

        assert_eq!(report.contract_id, crate::DECODE_HASH_REPORT_CONTRACT_ID);
        assert_eq!(report.selected_output_variants.len(), 1);
        assert!(report.frames.is_empty());
    }
}

#[test]
fn truncated_empty_ivf_fails_as_malformed_source() {
    let input = empty_avmenc_ivf();
    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(&input[..input.len() - 1], DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn standalone_open_loop_key_matches_expected_output_hash() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(STANDALONE_OLK_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        "7ccdd8e95f70849d93650136562854419f79379ab897edca10fba785a61d46ab"
    );
}

#[test]
fn bridge_celu_matches_expected_output_hashes() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(BRIDGE_CELU_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 2);
    assert_eq!(
        report
            .frames
            .iter()
            .map(|frame| frame.hashes[0].digest_hex.as_str())
            .collect::<Vec<_>>(),
        [
            "ebf2ba02fa61281e66533bc142260d49971a96101442d7df7d099b1d2be3bad5",
            "66eba64a8824334395164d4db56990daa9f1928bbbb8959bc76107dd7f3cc6e8",
        ]
    );
}

#[test]
fn scaled_bridge_matches_expected_output_hashes() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(SCALED_BRIDGE_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 2);
    assert_eq!(
        report
            .frames
            .iter()
            .map(|frame| frame.hashes[0].digest_hex.as_str())
            .collect::<Vec<_>>(),
        [
            "6305864e33464444002180e560bbdef88ecf5fe09561a208e9f25c4163bcdf88",
            "d8e966fdea08175cd96050fac10cabf76c42ccb1ba9b39e1f389283641500f65",
        ]
    );
}

#[test]
fn single_picture_bridge_matches_expected_output_hashes() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(SINGLE_PICTURE_BRIDGE_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 2);
    assert_eq!(
        report
            .frames
            .iter()
            .map(|frame| frame.hashes[0].digest_hex.as_str())
            .collect::<Vec<_>>(),
        [
            "6305864e33464444002180e560bbdef88ecf5fe09561a208e9f25c4163bcdf88",
            "103a6bbd35c54def7f63a723a2a90f502763778f6e0d25603a02eac274bf8a3a",
        ]
    );
}

#[test]
fn crop_window_fixture_reports_and_hashes_only_visible_samples() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(CROP_FIXTURE, DecodeOptions::default())
        .unwrap();

    let frame = &report.frames[0];
    assert_eq!(frame.visible_luma_left, 2);
    assert_eq!(frame.visible_luma_top, 2);
    assert_eq!(frame.visible_luma_width, 60);
    assert_eq!(frame.visible_luma_height, 60);
    assert_eq!(frame.chroma_left, Some(1));
    assert_eq!(frame.chroma_top, Some(1));
    assert_eq!(frame.chroma_width, Some(30));
    assert_eq!(frame.chroma_height, Some(30));
    assert_eq!(frame.hashes[0].digest_hex, CROP_EXPECTED_DIGEST);
}

#[test]
fn per_block_gdf_fixture_matches_reference_output_hash() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(GDF_PER_BLOCK_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        GDF_PER_BLOCK_EXPECTED_DIGEST
    );
}

#[test]
fn per_block_gdf_fixture_eof_fails_closed() {
    let truncated = &GDF_PER_BLOCK_FIXTURE[..GDF_PER_BLOCK_FIXTURE.len() - 1];

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(truncated, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn multi_tile_gdf_fixture_matches_reference_output_hash() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(GDF_MULTI_TILE_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        GDF_MULTI_TILE_EXPECTED_DIGEST
    );
}

#[test]
fn multi_tile_gdf_fixture_eof_fails_closed() {
    let truncated = &GDF_MULTI_TILE_FIXTURE[..GDF_MULTI_TILE_FIXTURE.len() - 1];

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(truncated, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn inter_444_fixture_matches_reference_output_hash() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(INTER_444_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 2);
    for frame in report.frames {
        assert_eq!(frame.pixel_format, DecodeHashPixelFormat::Yuv444);
        assert_eq!(
            (frame.chroma_width, frame.chroma_height),
            (Some(64), Some(64))
        );
        assert_eq!(frame.hashes[0].digest_hex, INTER_444_EXPECTED_DIGEST);
    }
}

#[test]
fn inter_444_fixture_eof_fails_closed() {
    let truncated = &INTER_444_FIXTURE[..INTER_444_FIXTURE.len() - 1];

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(truncated, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn reference_scaling_fixture_matches_reference_output_hashes() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(REFERENCE_SCALING_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 2);
    assert_eq!(
        (
            report.frames[0].visible_luma_width,
            report.frames[0].visible_luma_height,
            report.frames[0].chroma_width,
            report.frames[0].chroma_height,
        ),
        (64, 64, Some(32), Some(32))
    );
    assert_eq!(
        (
            report.frames[1].visible_luma_width,
            report.frames[1].visible_luma_height,
            report.frames[1].chroma_width,
            report.frames[1].chroma_height,
        ),
        (51, 51, Some(26), Some(26))
    );
    for (frame, expected) in report.frames.iter().zip(REFERENCE_SCALING_EXPECTED_DIGESTS) {
        assert_eq!(frame.hashes[0].digest_hex, expected);
    }
}

#[test]
fn reference_scaling_fixture_eof_fails_closed() {
    let truncated = &REFERENCE_SCALING_FIXTURE[..REFERENCE_SCALING_FIXTURE.len() - 1];

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(truncated, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn directional_tx_partition_fixture_matches_reference_output_hash() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(DIRECTIONAL_TX_PARTITION_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        DIRECTIONAL_TX_PARTITION_EXPECTED_DIGEST
    );
}

#[test]
fn directional_tx_partition_fixture_eof_fails_closed() {
    let truncated = &DIRECTIONAL_TX_PARTITION_FIXTURE[..DIRECTIONAL_TX_PARTITION_FIXTURE.len() - 1];

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(truncated, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn rect_noedge_d113follow_chroma_fixture_matches_reference_output_hash() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(
            RECT_NOEDGE_D113FOLLOW_CHROMA_FIXTURE,
            DecodeOptions::default(),
        )
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        RECT_NOEDGE_D113FOLLOW_CHROMA_EXPECTED_DIGEST
    );
}

#[test]
fn rect_chroma_chunks_444_fixture_matches_reference_output_hash() {
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(RECT_CHROMA_CHUNKS_444_FIXTURE, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    let frame = &report.frames[0];
    assert_eq!(frame.pixel_format, DecodeHashPixelFormat::Yuv444);
    assert_eq!(
        (frame.chroma_width, frame.chroma_height),
        (Some(128), Some(64))
    );
    assert_eq!(
        frame.hashes[0].digest_hex,
        RECT_CHROMA_CHUNKS_444_EXPECTED_DIGEST
    );
}

#[test]
fn rect_chroma_chunks_444_fixture_eof_fails_closed() {
    let truncated = &RECT_CHROMA_CHUNKS_444_FIXTURE[..RECT_CHROMA_CHUNKS_444_FIXTURE.len() - 1];

    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(truncated, DecodeOptions::default())
        .unwrap_err();

    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}

#[test]
fn ten_bit_d45_fixtures_match_reference_output_hashes() {
    let cases = [
        (
            TEN_BIT_D45_FIXTURE,
            DecodeHashPixelFormat::Yuv420,
            TEN_BIT_D45_EXPECTED_DIGEST,
        ),
        (
            TEN_BIT_D45_MONO_FIXTURE,
            DecodeHashPixelFormat::Monochrome,
            TEN_BIT_D45_MONO_EXPECTED_DIGEST,
        ),
    ];

    for (fixture, pixel_format, expected_digest) in cases {
        let report = context(ThreadCount::from(1usize))
            .decode_hash_report_bytes(fixture, DecodeOptions::default())
            .unwrap();
        let frame = &report.frames[0];

        assert_eq!(report.frames.len(), 1);
        assert_eq!(frame.bit_depth, 10);
        assert_eq!(frame.pixel_format, pixel_format);
        assert_eq!(frame.hashes[0].digest_hex, expected_digest);
    }
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
fn raw_annex_b_payload_decodes_to_hash_report() {
    let ivf_payload = &MINIMAL_FIXTURE[44..];
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(ivf_payload, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames[0].hashes[0].digest_hex, EXPECTED_DIGEST);
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
fn partition_symbol_mutation_fails_closed_at_exit_symbol() {
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
        } if unsupported.reason() == "inter_exit_symbol"
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
fn hash_report_is_not_raw_output_byte_limited() {
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_output_bytes(DecodeLimitThreshold::Max(0)),
    );
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(MINIMAL_FIXTURE, options)
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    assert_eq!(report.frames[0].hashes[0].digest_hex, EXPECTED_DIGEST);
}

#[test]
fn long_hash_decode_keeps_reference_storage_bounded() {
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_reference_store_bytes(DecodeLimitThreshold::Max(110_592)),
    );
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(ORDER_HINT_WRAP_FIXTURE, options)
        .unwrap();

    assert_eq!(report.frames.len(), 121);
    assert_eq!(
        report.frames[119].hashes[0].digest_hex,
        "c6e91fa41a421a666ea3846bef406127d88c253b3d800a89bacabfd7ab0e4437"
    );
}

#[test]
fn long_hash_decode_rejects_live_store_cap_below_peak() {
    let options = DecodeOptions::new(
        DecodeLimits::default().with_max_reference_store_bytes(DecodeLimitThreshold::Max(110_591)),
    );
    let error = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(ORDER_HINT_WRAP_FIXTURE, options)
        .unwrap_err();

    assert!(matches!(error, DecodeError::Limit { .. }));
    let DecodeError::Limit { source } = error else {
        return;
    };
    let check = source.check().unwrap();
    assert_eq!(source.name(), DecodeLimitName::MaxReferenceStoreBytes);
    assert_eq!(check.actual(), 110_592);
    assert_eq!(check.threshold(), DecodeLimitThreshold::Max(110_591));
}

#[test]
fn hash_report_still_enforces_output_frame_limit() {
    assert_minimal_fixture_limit(
        DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::Max(0)),
        DecodeLimitName::MaxOutputFrames,
    );
}

#[test]
fn requested_hash_output_limit_emits_one_frame() {
    let options = DecodeOptions::default().with_output_frame_limit(NonZeroU64::new(1));
    let report = context(ThreadCount::from(1usize))
        .decode_hash_report_bytes(ORDER_HINT_WRAP_FIXTURE, options)
        .unwrap();

    assert_eq!(report.frames.len(), 1);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        "548b1f0574d2f692141da2f326aa5d94fb8af45d9f6af7f3ccec9e0326e3e4b6"
    );
}

#[test]
fn partition_frontier_limit_preserves_resource_limit() {
    assert_minimal_fixture_limit(
        DecodeLimits::default().with_max_tile_partition_steps(DecodeLimitThreshold::Max(0)),
        DecodeLimitName::MaxTilePartitionSteps,
    );
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
