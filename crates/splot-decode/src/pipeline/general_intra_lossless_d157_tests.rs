// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_parallel::ThreadCount;
use splot_recon::{BitDepth, DecodedFrameHashInput, PixelFormat, PlaneSize};

use super::*;
use crate::{DecodeContext, DecodeRuntimeConfig};

const LOSSLESS_NONDC_CHROMA_D157_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d157-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D157_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d157-intra-64x64.ivf"
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
