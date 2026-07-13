// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Closed-loop-key sequence activation integration tests.

#![allow(clippy::unwrap_used)]

use core::num::NonZeroU64;

use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};
use splot_decode::{
    DecodeContext, DecodeError, DecodeLimitName, DecodeLimitThreshold, DecodeLimits, DecodeOptions,
    DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

const FIRST: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");
const COMPATIBLE_SECOND: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf");
const CROPPED_SECOND: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-crop-intra-64x64-q80.ivf");
const COMPATIBLE_TRANSITION: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-2seq-compatible-intra-64x64.obu");
const CROPPED_TRANSITION: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-2seq-crop-intra-64x64.obu");
const TEN_BIT_SECOND: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-10bit-q80.ivf");
const INTER_SECOND_SEQUENCE: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-2frame-flatstep-inter-y-dc-delta1-64x64-q80.ivf"
);
const FIRST_RAW: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw");
const IVF_PAYLOAD_OFFSET: usize = 44;
const FIRST_AVM_DIGEST: &str = "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af";
const CROPPED_AVM_DIGEST: &str = "db63b846c386f8b66acb6f4750abd436a4c2e5b9cbe63166d1c93f1fcc4e20b9";
const TEN_BIT_AVM_DIGEST: &str = "973eb3fc4b112c865f939dc1339824ca0b2a1522ca2b5ec70311afb459436e2d";
const SECOND_SEQUENCE_KEY_AVM_DIGEST: &str =
    "ebf2ba02fa61281e66533bc142260d49971a96101442d7df7d099b1d2be3bad5";
const SECOND_SEQUENCE_INTER_AVM_DIGEST: &str =
    "e73a3b0168597953992650452b153d6d316f649254b2493864fb6d320a3d8f53";

fn context() -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap()
}

fn repeated_sequence_annex_b(second: &[u8]) -> Vec<u8> {
    [&FIRST[IVF_PAYLOAD_OFFSET..], &second[IVF_PAYLOAD_OFFSET..]].concat()
}

fn repeated_sequence_ivf(second: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_ivf_header(&mut bytes, &IvfHeader::new(*b"AV02", 64, 64, 30, 1, 2)).unwrap();
    write_ivf_frame(&mut bytes, 0, &FIRST[IVF_PAYLOAD_OFFSET..]).unwrap();
    write_ivf_frame(&mut bytes, 1, &second[IVF_PAYLOAD_OFFSET..]).unwrap();
    bytes
}

fn repeated_sequence_with_inter() -> Vec<u8> {
    let mut bytes = FIRST[IVF_PAYLOAD_OFFSET..].to_vec();
    let mut cursor = 32;
    while cursor < INTER_SECOND_SEQUENCE.len() {
        let payload_len = u32::from_le_bytes(
            INTER_SECOND_SEQUENCE[cursor..cursor + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let payload_start = cursor + 12;
        let payload_end = payload_start + payload_len;
        bytes.extend_from_slice(&INTER_SECOND_SEQUENCE[payload_start..payload_end]);
        cursor = payload_end;
    }
    bytes
}

#[test]
fn changed_sequence_activates_at_clk_and_matches_reference_output() {
    let input = CROPPED_TRANSITION;

    let report = context()
        .decode_hash_report_bytes(input, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 2);
    assert_eq!(report.frames[0].visible_luma_width, 64);
    assert_eq!(report.frames[0].visible_luma_height, 64);
    assert_eq!(report.frames[0].hashes[0].digest_hex, FIRST_AVM_DIGEST);
    assert_eq!(report.frames[1].visible_luma_left, 2);
    assert_eq!(report.frames[1].visible_luma_top, 2);
    assert_eq!(report.frames[1].visible_luma_width, 60);
    assert_eq!(report.frames[1].visible_luma_height, 60);
    assert_eq!(report.frames[1].hashes[0].digest_hex, CROPPED_AVM_DIGEST);
}

#[test]
fn following_inter_frame_uses_the_new_sequence_and_references() {
    let input = repeated_sequence_with_inter();

    let report = context()
        .decode_hash_report_bytes(&input, DecodeOptions::default())
        .unwrap();

    assert_eq!(report.frames.len(), 3);
    assert_eq!(
        report.frames[1].hashes[0].digest_hex,
        SECOND_SEQUENCE_KEY_AVM_DIGEST
    );
    assert_eq!(
        report.frames[2].hashes[0].digest_hex,
        SECOND_SEQUENCE_INTER_AVM_DIGEST
    );

    let mut raw = Vec::new();
    context()
        .decode_raw_bytes(&input, DecodeOptions::default(), &mut raw)
        .unwrap();
    assert_eq!(raw.len(), 18_432);
}

#[test]
fn raw_output_allows_format_change_and_charges_both_frames() {
    let input = CROPPED_TRANSITION;
    let mut raw = Vec::new();

    context()
        .decode_raw_bytes(input, DecodeOptions::default(), &mut raw)
        .unwrap();

    assert_eq!(raw.len(), 11_544);

    let options = DecodeOptions::new(
        DecodeLimits::default()
            .with_max_output_bytes(DecodeLimitThreshold::Max(raw.len() as u64 - 1)),
    );
    let mut limited = Vec::new();
    let error = context()
        .decode_raw_bytes(input, options, &mut limited)
        .unwrap_err();
    assert!(limited.is_empty());
    assert!(matches!(
        error,
        DecodeError::Limit { source } if source.name() == DecodeLimitName::MaxOutputBytes
    ));
}

#[test]
fn compatible_sequence_change_writes_one_y4m_stream() {
    let input = repeated_sequence_ivf(COMPATIBLE_SECOND);
    let mut y4m = Vec::new();

    context()
        .decode_y4m_bytes(&input, DecodeOptions::default(), &mut y4m)
        .unwrap();

    assert!(y4m.starts_with(b"YUV4MPEG2 W64 H64 F30:1 Ip A0:0 C420\n"));
    assert_eq!(
        y4m.windows(b"FRAME\n".len())
            .filter(|window| *window == b"FRAME\n")
            .count(),
        2
    );

    let report = context()
        .decode_hash_report_bytes(COMPATIBLE_TRANSITION, DecodeOptions::default())
        .unwrap();
    assert_eq!(report.frames.len(), 2);
}

#[test]
fn incompatible_sequence_change_is_a_transactional_y4m_error() {
    let input = repeated_sequence_ivf(CROPPED_SECOND);
    let mut y4m = Vec::new();

    let error = context()
        .decode_y4m_bytes(&input, DecodeOptions::default(), &mut y4m)
        .unwrap_err();

    assert!(y4m.is_empty());
    assert!(matches!(
        error,
        DecodeError::Output { ref source }
            if source.operation().as_str() == "serialize_y4m"
                && source.source_kind() == "y4m"
                && source.source_message().contains("stream/frame mismatch")
    ));
}

#[test]
fn bit_depth_change_matches_reference_raw_and_is_rejected_by_y4m() {
    let annex_b = repeated_sequence_annex_b(TEN_BIT_SECOND);
    let report = context()
        .decode_hash_report_bytes(&annex_b, DecodeOptions::default())
        .unwrap();
    assert_eq!(report.frames.len(), 2);
    assert_eq!(report.frames[0].bit_depth, 8);
    assert_eq!(report.frames[1].bit_depth, 10);
    assert_eq!(report.frames[1].hashes[0].digest_hex, TEN_BIT_AVM_DIGEST);

    let mut raw = Vec::new();
    context()
        .decode_raw_bytes(&annex_b, DecodeOptions::default(), &mut raw)
        .unwrap();
    assert_eq!(raw.len(), 18_432);

    let mut y4m = Vec::new();
    let error = context()
        .decode_y4m_bytes(
            &repeated_sequence_ivf(TEN_BIT_SECOND),
            DecodeOptions::default(),
            &mut y4m,
        )
        .unwrap_err();
    assert!(y4m.is_empty());
    assert!(matches!(
        error,
        DecodeError::Output { ref source }
            if source.operation().as_str() == "serialize_y4m"
                && source.source_kind() == "frame_set"
                && source.source_message().contains("sample bit depth")
    ));
}

#[test]
fn output_limit_stops_before_the_next_sequence() {
    let input = CROPPED_TRANSITION;
    let options = DecodeOptions::default().with_output_frame_limit(NonZeroU64::new(1));
    let mut raw = Vec::new();

    context()
        .decode_raw_bytes(input, options, &mut raw)
        .unwrap();

    assert_eq!(raw, FIRST_RAW);
}

#[test]
fn truncated_repeated_sequence_fails_closed() {
    let input = CROPPED_TRANSITION;
    let mut raw = Vec::new();

    let error = context()
        .decode_raw_bytes(
            &input[..input.len() - 1],
            DecodeOptions::default(),
            &mut raw,
        )
        .unwrap_err();

    assert!(raw.is_empty());
    assert!(matches!(error, DecodeError::MalformedSource { .. }));
}
