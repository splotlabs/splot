// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, DecodedFrameHashInput, PixelFormat, PlaneSize};

use super::general_intra_tests::assert_lossless_explicit_chroma_oracle;
use super::*;
use crate::{DecodeContext, DecodeRuntimeConfig};

const LOSSLESS_NONDC_LUMA_D67_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-luma-d67-intra-64x64.ivf"
);
const LOSSLESS_NONDC_LUMA_D203_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-luma-d203-intra-64x64.ivf"
);
const LOSSLESS_NONDC_LUMA_D203_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-luma-d203-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_NONDC_LUMA_D67_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-luma-d67-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_NONDC_LUMA_D113_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-luma-d113-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_NONDC_LUMA_D135_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-luma-d135-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_NONDC_LUMA_D157_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-luma-d157-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_LUMA_D135_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d135-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_LUMA_D45_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d45-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_LUMA_D67_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d67-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_LUMA_D113_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d113-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_LUMA_D157_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d157-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_LUMA_D203_CHROMA_FOLLOW_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d203-chroma-follow-intra-64x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_D157_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d157-intra-64x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_D67_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d67-intra-64x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_D67_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d67-leftedge-128x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_D67FOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d67follow-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D67_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d67-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D67FOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d67follow-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D157_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d157-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D67_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d67-intra-64x64.ivf"
);

fn assert_lossless_yuv420_oracle(
    fixture: &[u8],
    expected_len: usize,
    frame_size: (usize, usize),
    chroma_size: (usize, usize),
    expected_hash: &str,
) {
    assert_eq!(fixture.len(), expected_len);
    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(fixture, options).expect("plan");
    let decoded = context
        .pool()
        .install(|| decode_frame_from_plan(fixture, &options, &plan))
        .expect("decode");
    let PipelineDecodedFrame::Eight(frame) = decoded.frame else {
        panic!("fixture decoded as 10-bit");
    };

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(
        frame.y().visible_size(),
        PlaneSize::new(frame_size.0, frame_size.1).unwrap()
    );
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(chroma_size.0, chroma_size.1).unwrap()
    );
    assert_eq!(
        frame.v().unwrap().visible_size(),
        PlaneSize::new(chroma_size.0, chroma_size.1).unwrap()
    );
    assert_eq!(
        DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
        expected_hash
    );
}

#[test]
fn lossless_nondc_luma_d67_and_lossless_nondc_chroma_d157_frames_decode_to_oracle() {
    let cases: &[(&[u8], usize, &str)] = &[
        (
            LOSSLESS_NONDC_LUMA_D67_FIXTURE,
            140,
            "06f8dbad33bd1c4e903d36cc3f042fceb1c2c4094f9ff50c08b4c1f08ba81595",
        ),
        (
            LOSSLESS_NONDC_CHROMA_D157_FIXTURE,
            552,
            "00e00b20f057056ee63684a752d60e4a158da1e466720069d81639e88887030d",
        ),
        (
            LOSSLESS_SDP_NONDC_CHROMA_D157_FIXTURE,
            552,
            "00e00b20f057056ee63684a752d60e4a158da1e466720069d81639e88887030d",
        ),
    ];
    for &(fixture, expected_len, expected_hash) in cases {
        assert_lossless_yuv420_oracle(fixture, expected_len, (64, 64), (32, 32), expected_hash);
    }
}

#[test]
fn lossless_sdp_nondc_chroma_d157_frame_decodes_to_oracle() {
    lossless_nondc_luma_d67_and_lossless_nondc_chroma_d157_frames_decode_to_oracle();
}

#[test]
fn lossless_nondc_luma_d203_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_NONDC_LUMA_D203_FIXTURE,
        1493,
        (64, 64),
        (32, 32),
        "547050c8d8b70f5eac7a44475fba961363945f35c490fe1554e129f6cd349662",
    );
}

#[test]
fn lossless_nondc_luma_d203_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_NONDC_LUMA_D203_CHROMA_FOLLOW_FIXTURE,
        1821,
        (64, 64),
        (32, 32),
        "1aa978acb73d6bc314b779a49f4aaecafba12978ecf290673fa3ed30c54e735a",
    );
}

#[test]
fn lossless_nondc_luma_d67_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_NONDC_LUMA_D67_CHROMA_FOLLOW_FIXTURE,
        168,
        (64, 64),
        (32, 32),
        "1e06306e7d131ffc620e1969987c4f4b7dbae673c68e56e8a462a65c338c4576",
    );
}

#[test]
fn lossless_sdp_nondc_luma_d45_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_SDP_NONDC_LUMA_D45_CHROMA_FOLLOW_FIXTURE,
        123,
        (64, 64),
        (32, 32),
        "52c9fe30861467fdb7a1897bd266e21e8edd4c6ddb04ef534442849125c5d925",
    );
}

#[test]
fn lossless_nondc_luma_d113_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_NONDC_LUMA_D113_CHROMA_FOLLOW_FIXTURE,
        218,
        (64, 64),
        (32, 32),
        "8f2f920f4daf0e1c057d0ae7d51ba56508b6a5f552dd3459bba97677ba7aafd8",
    );
}

#[test]
fn lossless_sdp_nondc_luma_d67_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_SDP_NONDC_LUMA_D67_CHROMA_FOLLOW_FIXTURE,
        168,
        (64, 64),
        (32, 32),
        "1e06306e7d131ffc620e1969987c4f4b7dbae673c68e56e8a462a65c338c4576",
    );
}

#[test]
fn lossless_nondc_luma_d135_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_NONDC_LUMA_D135_CHROMA_FOLLOW_FIXTURE,
        70,
        (64, 64),
        (32, 32),
        "5fffbdc79140da104a1721ed649130f0a2409fadeeb58632cdba54a1add778a1",
    );
}

#[test]
fn lossless_nondc_luma_d157_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_NONDC_LUMA_D157_CHROMA_FOLLOW_FIXTURE,
        503,
        (64, 64),
        (32, 32),
        "3b73634d41e76e87cfdd22fc75b11a2e9f187b96434f80915f18054aac8a7c9c",
    );
}

#[test]
fn lossless_sdp_nondc_luma_d203_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_SDP_NONDC_LUMA_D203_CHROMA_FOLLOW_FIXTURE,
        1821,
        (64, 64),
        (32, 32),
        "1aa978acb73d6bc314b779a49f4aaecafba12978ecf290673fa3ed30c54e735a",
    );
}

#[test]
fn lossless_sdp_nondc_luma_d135_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_SDP_NONDC_LUMA_D135_CHROMA_FOLLOW_FIXTURE,
        70,
        (64, 64),
        (32, 32),
        "5fffbdc79140da104a1721ed649130f0a2409fadeeb58632cdba54a1add778a1",
    );
}

#[test]
fn lossless_sdp_nondc_luma_d157_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_SDP_NONDC_LUMA_D157_CHROMA_FOLLOW_FIXTURE,
        496,
        (64, 64),
        (32, 32),
        "3b73634d41e76e87cfdd22fc75b11a2e9f187b96434f80915f18054aac8a7c9c",
    );
}

#[test]
fn lossless_sdp_nondc_luma_d113_chroma_follow_frame_decodes_to_oracle() {
    assert_lossless_yuv420_oracle(
        LOSSLESS_SDP_NONDC_LUMA_D113_CHROMA_FOLLOW_FIXTURE,
        218,
        (64, 64),
        (32, 32),
        "8f2f920f4daf0e1c057d0ae7d51ba56508b6a5f552dd3459bba97677ba7aafd8",
    );
}

#[test]
fn lossless_nondc_chroma_d67_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_NONDC_CHROMA_D67_FIXTURE,
        304,
        (64, 64),
        (32, 32),
        "lossless explicit D67",
        "bd031b83ebb53396538bcfdebe5c2fe5a186e8d75a5842fabdcad123039f7b3b",
    );
}

#[test]
fn lossless_nondc_chroma_d67_leftedge_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_NONDC_CHROMA_D67_LEFTEDGE_FIXTURE,
        217,
        (128, 64),
        (64, 32),
        "lossless explicit D67 left-edge",
        "c8ea02e937ad6ca39ab558192a831e71832fbda95ec2f249f5c138ece880a591",
    );
}

#[test]
fn lossless_sdp_nondc_chroma_d67_leftedge_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_SDP_NONDC_CHROMA_D67_LEFTEDGE_FIXTURE,
        197,
        (128, 64),
        (64, 32),
        "lossless SDP explicit D67 left-edge",
        "c8ea02e937ad6ca39ab558192a831e71832fbda95ec2f249f5c138ece880a591",
    );
}

#[test]
fn lossless_nondc_chroma_d67follow_leftedge_frame_decodes_to_oracle() {
    assert_lossless_d67follow_leftedge_oracle(
        LOSSLESS_NONDC_CHROMA_D67FOLLOW_LEFTEDGE_FIXTURE,
        180,
    );
}

#[test]
fn lossless_sdp_nondc_chroma_d67follow_leftedge_frame_decodes_to_oracle() {
    assert_lossless_d67follow_leftedge_oracle(
        LOSSLESS_SDP_NONDC_CHROMA_D67FOLLOW_LEFTEDGE_FIXTURE,
        168,
    );
}

fn assert_lossless_d67follow_leftedge_oracle(fixture: &[u8], expected_len: usize) {
    assert_lossless_yuv420_oracle(
        fixture,
        expected_len,
        (128, 64),
        (64, 32),
        "318dce0d8ed8771aa79709051ee18e72cdb7b999c05f27095fe1a6cc611ba6ee",
    );
}

#[test]
fn lossless_sdp_nondc_chroma_d67_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_SDP_NONDC_CHROMA_D67_FIXTURE,
        304,
        (64, 64),
        (32, 32),
        "lossless SDP explicit D67",
        "bd031b83ebb53396538bcfdebe5c2fe5a186e8d75a5842fabdcad123039f7b3b",
    );
}
