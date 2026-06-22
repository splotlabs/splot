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

/// The first bit-exact sub-pel inter fixture: frame 0 is a general-intra DC_PRED
/// half-cosine key frame; frame 1 is a single-reference NEWMV inter frame with an
/// EighthPel `(0, -4)` (a -1/2 luma-sample horizontal) sub-pel motion vector, a
/// SWITCHABLE `EIGHTTAP_SHARP` interpolation filter, and `skip == 1` (no residual).
/// avmdec `--rawvideo --i420` and `dav2d --demuxer ivf` decode the whole stream
/// byte-for-byte identically.
const TWO_FRAME_SUBPEL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-subpel-inter-64x64.ivf"
);

/// The first bit-exact inter-residual fixture: frame 0 is a general-intra DC_PRED
/// flat-100 key frame; frame 1 is a single-reference zero-MV inter frame with
/// `skip == 0` carrying a low-frequency §5.20.7.27 luma DCT_DCT residual (flat
/// chroma, no residual) added over the §7.13.3.18 zero-fraction copy of frame 0.
/// avmdec `--rawvideo --i420` and `dav2d --demuxer ivf` decode the whole stream
/// byte-for-byte identically (oracle MD5 `ab2b067aed48cf46035fa031cefb3ab1`).
const TWO_FRAME_RESIDUAL_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-residual-64x64.ivf"
);

/// The first bit-exact MULTI-BLOCK inter fixture: frame 0 is a general-intra
/// DC_PRED key frame (four flat 32x32 quadrants); frame 1 is a single-reference
/// inter frame whose 64x64 superblock is SPLIT into four 32x32 inter blocks.
/// Block 0 @ MI(0,0) is NEWMV with a non-zero MV (col 48 = +6 full pels); the
/// later three blocks are NEARMV that predict block 0's MV from the §7.11/§7.12
/// spatial-neighbour MV stack (find_mv_stack). All blocks are skip=1 (no
/// residual). avmdec `--rawvideo --i420` and `dav2d --demuxer ivf` decode the
/// whole stream byte-for-byte identically (oracle MD5
/// `e5b581a55433785c0071b635d5642083`). The OLD single-block inter decoder
/// rejected this fixture ("only supports a single top-left 64x64 inter block").
const TWO_FRAME_MVSTACK_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-mvstack-64x64.ivf"
);

// avmdec / dav2d both decode every plane of both frames to these flat values.
const FLAT_LUMA: u8 = 100;
const FLAT_CHROMA_U: u8 = 120;
const FLAT_CHROMA_V: u8 = 130;

fn decode_fixture(bytes: &[u8]) -> Vec<MinimalRuntimeFrame> {
    let options = DecodeOptions::default();
    let context =
        DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context");
    let plan = context.plan_bytes(bytes, options).expect("plan");
    decode_minimal_frames_from_plan(bytes, options, &plan).expect("decode")
}

fn decode_frames() -> Vec<MinimalRuntimeFrame> {
    decode_fixture(TWO_FRAME_INTER_FIXTURE)
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

/// The committed sub-pel fixture decodes a key frame + one sub-pel inter frame.
#[test]
fn subpel_fixture_decodes_two_frames() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_fixture(TWO_FRAME_SUBPEL_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the sub-pel stream decodes a key frame + one inter frame"
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
    }
}

/// The sub-pel inter frame is NOT a copy of the key frame: the § 7.13.3.18
/// interpolation-filter convolution over the decoded sub-pel motion vector
/// produces a fractionally-shifted prediction. This distinguishes the sub-pel MC
/// from the zero-MV straight copy.
#[test]
fn subpel_inter_frame_differs_from_key_frame() {
    let frames = decode_fixture(TWO_FRAME_SUBPEL_FIXTURE);
    let key = frames[0].frame();
    let inter = frames[1].frame();
    assert_ne!(
        key.y().samples(),
        inter.y().samples(),
        "the sub-pel inter luma must differ from the key luma (real fractional MC)"
    );
    // The fixture's content is purely horizontal, so the chroma planes are flat and
    // a horizontal sub-pel shift leaves them unchanged; only luma differs.
}

/// Regression pin for the per-frame decode hash of the sub-pel fixture. The raw
/// decoded output (both frames concatenated I420) matches avmdec `--rawvideo
/// --i420` and `dav2d --demuxer ivf` byte-for-byte (oracle MD5
/// `a0e82de3a95bb4b519c4c84ffa2ba816`, recorded in
/// `docs/LOCAL-REFERENCE-EVIDENCE.toml`); these `splot-dfh-sha256-v1` per-frame
/// hashes are splot's internal regression anchors for that bit-exact output.
#[test]
fn subpel_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(TWO_FRAME_SUBPEL_FIXTURE);
    let key_hash = splot_recon::DecodedFrameHashInput::new(frames[0].frame())
        .compute_hash()
        .to_hex();
    let inter_hash = splot_recon::DecodedFrameHashInput::new(frames[1].frame())
        .compute_hash()
        .to_hex();
    assert_eq!(
        key_hash, "8a6751d4517073bad0bbe71f4b5537df8e8b0bfee85fcd6af1ac2d5878dd59e8",
        "sub-pel key-frame hash"
    );
    assert_eq!(
        inter_hash, "4c2443d95b38cee9a574ba1166a1fe15d6f2b5d20de070001d31db15a661896e",
        "sub-pel inter-frame hash"
    );
    assert_ne!(key_hash, inter_hash, "the sub-pel frames must differ");
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

/// The committed inter-residual fixture decodes a key frame + one `skip == 0`
/// inter frame (a §5.20.7.27 coded residual over the zero-MV MC prediction).
#[test]
fn residual_fixture_decodes_two_frames() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_fixture(TWO_FRAME_RESIDUAL_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the residual stream decodes a key frame + one inter frame"
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
    }
}

/// The `skip == 0` inter frame is NOT a copy of the key frame: the §5.20.7.27
/// decoded residual (§7.14.4 dequant + §7.15.4 inverse transform + §7.14.3 add)
/// over the zero-MV §7.13.3.18 copy genuinely changes the luma. The chroma carries
/// no residual (the encoder coded a luma-only residual), so it stays flat and
/// equals the key chroma. This distinguishes the residual decode from the bare
/// zero-MV copy: if the residual were dropped, frame 1 would equal frame 0.
#[test]
fn residual_inter_frame_differs_from_key_frame() {
    let frames = decode_fixture(TWO_FRAME_RESIDUAL_FIXTURE);
    let key = frames[0].frame();
    let inter = frames[1].frame();
    // The key frame is flat (Y=100, U=120, V=130) per the avmdec/dav2d oracle.
    assert!(
        key.y().samples().iter().all(|&s| s == FLAT_LUMA),
        "key luma must be flat {FLAT_LUMA}"
    );
    assert_ne!(
        key.y().samples(),
        inter.y().samples(),
        "the residual inter luma must differ from the flat key luma (real residual)"
    );
    // The decoded residual is luma-only; the inter chroma equals the key chroma.
    assert_eq!(
        key.u().unwrap().samples(),
        inter.u().unwrap().samples(),
        "U: no chroma residual"
    );
    assert_eq!(
        key.v().unwrap().samples(),
        inter.v().unwrap().samples(),
        "V: no chroma residual"
    );
    assert!(
        inter
            .u()
            .unwrap()
            .samples()
            .iter()
            .all(|&s| s == FLAT_CHROMA_U),
        "inter U flat {FLAT_CHROMA_U}"
    );
    assert!(
        inter
            .v()
            .unwrap()
            .samples()
            .iter()
            .all(|&s| s == FLAT_CHROMA_V),
        "inter V flat {FLAT_CHROMA_V}"
    );
}

/// Regression pin for the per-frame decode hash of the inter-residual fixture.
/// The raw decoded output (both frames concatenated I420) matches avmdec
/// `--rawvideo --i420` and `dav2d --demuxer ivf` byte-for-byte (oracle MD5
/// `ab2b067aed48cf46035fa031cefb3ab1`, recorded in
/// `docs/LOCAL-REFERENCE-EVIDENCE.toml`); these `splot-dfh-sha256-v1` per-frame
/// hashes are splot's internal regression anchors for that bit-exact output.
#[test]
fn residual_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(TWO_FRAME_RESIDUAL_FIXTURE);
    let key_hash = splot_recon::DecodedFrameHashInput::new(frames[0].frame())
        .compute_hash()
        .to_hex();
    let inter_hash = splot_recon::DecodedFrameHashInput::new(frames[1].frame())
        .compute_hash()
        .to_hex();
    assert_eq!(
        key_hash, "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979",
        "residual key-frame hash"
    );
    assert_eq!(
        inter_hash, "6bc96c12710ebe225b994c8e70e253e7159cd3fe49da61de5ad2558c207e26d8",
        "residual inter-frame hash"
    );
    assert_ne!(
        key_hash, inter_hash,
        "the residual inter frame must differ from the key frame"
    );
}

/// The committed multi-block fixture decodes a key frame + one multi-block inter
/// frame (a §5.20.3 SPLIT into four 32x32 inter blocks).
#[test]
fn mvstack_fixture_decodes_two_frames() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_fixture(TWO_FRAME_MVSTACK_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the multi-block stream decodes a key frame + one inter frame"
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
    }
}

/// Regression pin for the per-frame decode hash of the multi-block MV-stack
/// fixture. The raw decoded output (both frames concatenated I420) matches avmdec
/// `--rawvideo --i420` and `dav2d --demuxer ivf` byte-for-byte (oracle MD5
/// `e5b581a55433785c0071b635d5642083`, recorded in
/// `docs/LOCAL-REFERENCE-EVIDENCE.toml`); these `splot-dfh-sha256-v1` per-frame
/// hashes are splot's internal regression anchors for that bit-exact output. The
/// inter frame is reconstructed from the §7.11/§7.12 neighbour-predicted MVs of
/// its four 32x32 blocks (block 0 NEWMV, the rest NEARMV reusing block 0's MV).
#[test]
fn mvstack_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(TWO_FRAME_MVSTACK_FIXTURE);
    let key_hash = splot_recon::DecodedFrameHashInput::new(frames[0].frame())
        .compute_hash()
        .to_hex();
    let inter_hash = splot_recon::DecodedFrameHashInput::new(frames[1].frame())
        .compute_hash()
        .to_hex();
    assert_eq!(
        key_hash, "37d5a851609575dcceec47aa4b53043fa04f36cb483c40925913b8adfd91504f",
        "multi-block key-frame hash"
    );
    assert_eq!(
        inter_hash, "b39afe593c1046b080efea9c8bf76242dba2a4965a556d7ed31bcf0fca444fc1",
        "multi-block inter-frame hash"
    );
    assert_ne!(
        key_hash, inter_hash,
        "the multi-block inter frame must differ from the key frame"
    );
}
