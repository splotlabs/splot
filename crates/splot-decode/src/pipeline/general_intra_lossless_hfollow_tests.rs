// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::collections::BTreeSet;

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, DecodedFrameHashInput, PixelFormat, PlaneSize};

use super::general_intra_tests::{
    assert_lossless_directional_luma_oracle, assert_lossless_explicit_chroma_leftedge_pair_oracle,
    assert_lossless_explicit_chroma_oracle,
};
use super::*;
use crate::{DecodeContext, DecodeRuntimeConfig};

const LOSSLESS_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d45-leftedge-128x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_V_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-v-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_V_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-v-leftedge-128x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_VFOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-vfollow-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_VFOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-vfollow-leftedge-128x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_PAETH_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-paeth-leftedge-128x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_H_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-h-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_H_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-h-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_PAETH_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-paeth-leftedge-128x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_HFOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-hfollow-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_HFOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-hfollow-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d45-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D45FOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d45follow-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D135FOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d135follow-leftedge-128x64.ivf"
);

#[test]
fn lossless_nondc_chroma_d45_leftedge_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_leftedge_pair_oracle(
        LOSSLESS_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE,
        129,
        LOSSLESS_SDP_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE,
        120,
        "D45",
        "56a0c73c398f6adb27194cb8d3908cea02791d63e79658c7552cc15a0752fc01",
    );
}

#[test]
fn lossless_sdp_nondc_chroma_d45follow_leftedge_frame_decodes_to_oracle() {
    assert_lossless_directional_luma_oracle(
        LOSSLESS_SDP_NONDC_CHROMA_D45FOLLOW_LEFTEDGE_FIXTURE,
        76,
        128,
        64,
        64,
        32,
        "386bf9550c5623bc5eb0fba92f0985b2bd0f9d06c5fa991d32407f3b17f99c6f",
    );
}

#[test]
fn lossless_sdp_nondc_chroma_d135follow_leftedge_frame_decodes_to_oracle() {
    assert_lossless_directional_luma_oracle(
        LOSSLESS_SDP_NONDC_CHROMA_D135FOLLOW_LEFTEDGE_FIXTURE,
        125,
        128,
        64,
        64,
        32,
        "a79860d10164c5b4f17a10491a17280856a424b47846e90989af62d8f609bfa1",
    );
}

#[test]
fn lossless_nondc_chroma_v_leftedge_frame_decodes_to_oracle() {
    for (fixture, expected_len, label) in [
        (
            LOSSLESS_NONDC_CHROMA_V_LEFTEDGE_FIXTURE,
            127,
            "lossless V left-edge",
        ),
        (
            LOSSLESS_SDP_NONDC_CHROMA_V_LEFTEDGE_FIXTURE,
            127,
            "lossless SDP V left-edge",
        ),
    ] {
        assert_eq!(fixture.len(), expected_len, "{label}");
        let options = DecodeOptions::default();
        let context = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize)))
            .expect("context");
        let plan = context.plan_bytes(fixture, options).expect("plan");
        let decoded = context
            .pool()
            .install(|| decode_frame_from_plan(fixture, &options, &plan))
            .expect("decode");
        let PipelineDecodedFrame::Eight(frame) = decoded.frame else {
            panic!("fixture decoded as 10-bit");
        };

        assert_eq!(frame.bit_depth(), BitDepth::Eight, "{label}");
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "{label}");
        assert_eq!(
            frame.y().visible_size(),
            PlaneSize::new(128, 64).unwrap(),
            "{label}"
        );
        assert_eq!(
            frame.u().unwrap().visible_size(),
            PlaneSize::new(64, 32).unwrap(),
            "{label}"
        );
        assert_eq!(
            frame.v().unwrap().visible_size(),
            PlaneSize::new(64, 32).unwrap(),
            "{label}"
        );
        assert!(
            frame.y().samples().iter().all(|&sample| sample == 128),
            "{label}"
        );
        assert_eq!(
            frame
                .u()
                .unwrap()
                .samples()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            3,
            "{label}"
        );
        assert_eq!(
            frame
                .v()
                .unwrap()
                .samples()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            3,
            "{label}"
        );
        assert_eq!(
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
            "026ab3ce735e8d3c0a4b413ac6ab4c970631908887c1e7b7395cdf4464d72ea5",
            "{label}"
        );
    }
}

#[test]
fn lossless_nondc_chroma_ramped_leftedge_frames_decode_to_oracles() {
    for oracle in [
        LosslessRampedLeftedgeOracle {
            fixture: LOSSLESS_NONDC_CHROMA_H_LEFTEDGE_FIXTURE,
            expected_len: 238,
            u_samples: &[(0, 128), (63, 72)],
            v_samples: &[(0, 128), (63, 184)],
            expected_hash: "a5e8e56e191e9558ed8591013fa884fb573c320adea6aa6db475680a69167740",
        },
        LosslessRampedLeftedgeOracle {
            fixture: LOSSLESS_SDP_NONDC_CHROMA_H_LEFTEDGE_FIXTURE,
            expected_len: 238,
            u_samples: &[(0, 128), (63, 72)],
            v_samples: &[(0, 128), (63, 184)],
            expected_hash: "a5e8e56e191e9558ed8591013fa884fb573c320adea6aa6db475680a69167740",
        },
        LosslessRampedLeftedgeOracle {
            fixture: LOSSLESS_NONDC_CHROMA_VFOLLOW_LEFTEDGE_FIXTURE,
            expected_len: 230,
            u_samples: &[(0, 128), (32, 72), (63, 184)],
            v_samples: &[(0, 128), (32, 183), (63, 71)],
            expected_hash: "84b2a3c6212d5694b8914ca017b6dc7f6d6ae4876fc22582f2b924a5629e5304",
        },
        LosslessRampedLeftedgeOracle {
            fixture: LOSSLESS_SDP_NONDC_CHROMA_VFOLLOW_LEFTEDGE_FIXTURE,
            expected_len: 230,
            u_samples: &[(0, 128), (32, 72), (63, 184)],
            v_samples: &[(0, 128), (32, 183), (63, 71)],
            expected_hash: "84b2a3c6212d5694b8914ca017b6dc7f6d6ae4876fc22582f2b924a5629e5304",
        },
    ] {
        assert_lossless_ramped_leftedge_oracle(&oracle);
    }
}

struct LosslessRampedLeftedgeOracle {
    fixture: &'static [u8],
    expected_len: usize,
    u_samples: &'static [(usize, u8)],
    v_samples: &'static [(usize, u8)],
    expected_hash: &'static str,
}

fn assert_lossless_ramped_leftedge_oracle(oracle: &LosslessRampedLeftedgeOracle) {
    assert_eq!(oracle.fixture.len(), oracle.expected_len);
    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(oracle.fixture, options).expect("plan");
    let decoded = context
        .pool()
        .install(|| decode_frame_from_plan(oracle.fixture, &options, &plan))
        .expect("decode");
    let PipelineDecodedFrame::Eight(frame) = decoded.frame else {
        panic!("fixture decoded as 10-bit");
    };

    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
    assert_eq!(
        frame.u().unwrap().visible_size(),
        PlaneSize::new(64, 32).unwrap()
    );
    assert_eq!(
        frame.v().unwrap().visible_size(),
        PlaneSize::new(64, 32).unwrap()
    );
    assert!(
        frame
            .y()
            .samples()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            > 4
    );
    assert!(
        frame
            .u()
            .unwrap()
            .samples()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            > 4
    );
    assert!(
        frame
            .v()
            .unwrap()
            .samples()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            > 4
    );
    assert_eq!(frame.y().samples()[0], 128);
    assert_eq!(frame.y().samples()[64], 72);
    assert_eq!(frame.y().samples()[127], 184);

    let u_plane = frame.u().unwrap();
    for &(index, expected) in oracle.u_samples {
        assert_eq!(u_plane.samples()[index], expected);
    }

    let v_plane = frame.v().unwrap();
    for &(index, expected) in oracle.v_samples {
        assert_eq!(v_plane.samples()[index], expected);
    }

    assert_eq!(
        DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
        oracle.expected_hash
    );
}

#[test]
fn lossless_nondc_chroma_paeth_leftedge_frame_decodes_to_oracle() {
    for (fixture, expected_len, label) in [
        (
            LOSSLESS_NONDC_CHROMA_PAETH_LEFTEDGE_FIXTURE,
            122,
            "lossless Paeth left-edge",
        ),
        (
            LOSSLESS_SDP_NONDC_CHROMA_PAETH_LEFTEDGE_FIXTURE,
            121,
            "lossless SDP Paeth left-edge",
        ),
    ] {
        assert_lossless_explicit_chroma_oracle(
            fixture,
            expected_len,
            (128, 64),
            (64, 32),
            label,
            "b064c9c6fbeaac7b04e7c5cc4430f1af7a968488b9f2508127e82024f973fb96",
        );
    }
}

#[test]
fn lossless_nondc_chroma_hfollow_leftedge_frame_decodes_to_oracle() {
    for (fixture, label) in [
        (
            LOSSLESS_NONDC_CHROMA_HFOLLOW_LEFTEDGE_FIXTURE,
            "lossless H-follow left-edge",
        ),
        (
            LOSSLESS_SDP_NONDC_CHROMA_HFOLLOW_LEFTEDGE_FIXTURE,
            "lossless SDP H-follow left-edge",
        ),
    ] {
        assert_lossless_explicit_chroma_oracle(
            fixture,
            158,
            (128, 64),
            (64, 32),
            label,
            "879dfd582fb00d3f068b5ac2f3eaf3a4c28449c8c763c18aa8645a5fde20448b",
        );
    }
}
