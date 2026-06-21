// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! First-inter-frame decode tests for the shared minimal-tier runtime.
//!
//! The committed `syn-2frame-inter-64x64.ivf` is the verified target: frame 0 is an
//! `OBU_CLOSED_LOOP_KEY` intra key frame, frame 1 is an `OBU_REGULAR_TILE_GROUP`
//! inter frame (single reference, `is_inter == 1`, `skip == 1`, the single-reference
//! zero-MV NEARMV mode, no residual). avmdec `--rawvideo --i420` and
//! `dav2d --demuxer ivf` decode the whole stream byte-for-byte identically
//! (decoded-output md5 `4e1bd39f0b541ef1f479cff049e6985c`, 12288 bytes; frame 1 == a
//! straight copy of frame 0 via § 7.13.3.18 zero-fraction motion compensation).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_parallel::ThreadCount;

use super::super::{MinimalRuntimeFrame, decode_minimal_frames_from_plan};
use crate::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};

const TWO_FRAME_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf");

// avmdec / dav2d both decode every plane of both frames to these flat values.
const FLAT_LUMA: u8 = 100;
const FLAT_CHROMA_U: u8 = 120;
const FLAT_CHROMA_V: u8 = 130;

fn decode_frames() -> Vec<MinimalRuntimeFrame> {
    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context
        .plan_bytes(TWO_FRAME_INTER_FIXTURE, options)
        .expect("plan");
    decode_minimal_frames_from_plan(TWO_FRAME_INTER_FIXTURE, options, &plan).expect("decode")
}

#[test]
fn two_frame_inter_fixture_decodes_both_frames_bit_exact() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_frames();
    assert_eq!(
        frames.len(),
        2,
        "the stream decodes a key frame + one inter frame"
    );

    for (index, output) in frames.iter().enumerate() {
        let frame = output.frame();
        assert_eq!(frame.bit_depth(), BitDepth::Eight, "frame {index}");
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "frame {index}");
        assert_eq!(
            frame.y().visible_size(),
            PlaneSize::new(64, 64).unwrap(),
            "frame {index}"
        );
        assert!(
            frame.y().samples().iter().all(|&s| s == FLAT_LUMA),
            "frame {index} luma must be flat {FLAT_LUMA}"
        );
        assert!(
            frame
                .u()
                .unwrap()
                .samples()
                .iter()
                .all(|&s| s == FLAT_CHROMA_U),
            "frame {index} U must be flat {FLAT_CHROMA_U}"
        );
        assert!(
            frame
                .v()
                .unwrap()
                .samples()
                .iter()
                .all(|&s| s == FLAT_CHROMA_V),
            "frame {index} V must be flat {FLAT_CHROMA_V}"
        );
    }
}

#[test]
fn inter_frame_is_a_bit_exact_copy_of_the_key_frame() {
    // §7.13.3.18 zero-fraction motion compensation reduces to a straight copy of the
    // co-located key block, so frame 1's planes must be byte-identical to frame 0's
    // (avmdec/dav2d agree: frame 1 == a copy of frame 0).
    let frames = decode_frames();
    let key = frames[0].frame();
    let inter = frames[1].frame();
    assert_eq!(key.y().samples(), inter.y().samples(), "luma copy");
    assert_eq!(
        key.u().unwrap().samples(),
        inter.u().unwrap().samples(),
        "U copy"
    );
    assert_eq!(
        key.v().unwrap().samples(),
        inter.v().unwrap().samples(),
        "V copy"
    );
}

#[test]
fn two_frame_inter_fixture_per_frame_hash_is_stable() {
    // Regression pin for the per-frame decode hash. The flat-plane / copy tests above
    // are the avmdec/dav2d oracle anchors. The inter frame is a copy of the key
    // frame, so their per-frame hashes match.
    let frames = decode_frames();
    let key_hash = splot_recon::DecodedFrameHashInput::new(frames[0].frame())
        .compute_hash()
        .to_hex();
    let inter_hash = splot_recon::DecodedFrameHashInput::new(frames[1].frame())
        .compute_hash()
        .to_hex();
    assert_eq!(key_hash, inter_hash, "inter frame hash == key frame hash");
}
