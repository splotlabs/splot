// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, DecodedFrameHashInput, PixelFormat, PlaneSize};

use super::general_intra_tests::assert_lossless_explicit_chroma_oracle;
use super::*;
use crate::{DecodeContext, DecodeRuntimeConfig};

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
const LOSSLESS_SDP_NONDC_CHROMA_D157_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d157-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D67_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d67-intra-64x64.ivf"
);

#[test]
fn lossless_nondc_chroma_d157_frame_decodes_to_oracle() {
    for fixture in [
        LOSSLESS_NONDC_CHROMA_D157_FIXTURE,
        LOSSLESS_SDP_NONDC_CHROMA_D157_FIXTURE,
    ] {
        assert_eq!(fixture.len(), 552);
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

        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(64, 64).unwrap());
        assert_eq!(
            frame.u().unwrap().visible_size(),
            PlaneSize::new(32, 32).unwrap()
        );
        assert_eq!(
            frame.v().unwrap().visible_size(),
            PlaneSize::new(32, 32).unwrap()
        );
        assert_eq!(
            DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
            "00e00b20f057056ee63684a752d60e4a158da1e466720069d81639e88887030d"
        );
    }
}

#[test]
fn lossless_sdp_nondc_chroma_d157_frame_decodes_to_oracle() {
    lossless_nondc_chroma_d157_frame_decodes_to_oracle();
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
fn lossless_nondc_chroma_d67follow_leftedge_frame_decodes_to_oracle() {
    assert_eq!(LOSSLESS_NONDC_CHROMA_D67FOLLOW_LEFTEDGE_FIXTURE.len(), 180);
    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context
        .plan_bytes(LOSSLESS_NONDC_CHROMA_D67FOLLOW_LEFTEDGE_FIXTURE, options)
        .expect("plan");
    let decoded = context
        .pool()
        .install(|| {
            decode_frame_from_plan(
                LOSSLESS_NONDC_CHROMA_D67FOLLOW_LEFTEDGE_FIXTURE,
                &options,
                &plan,
            )
        })
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
    assert_eq!(
        DecodedFrameHashInput::new(&frame).compute_hash().to_hex(),
        "318dce0d8ed8771aa79709051ee18e72cdb7b999c05f27095fe1a6cc611ba6ee"
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
