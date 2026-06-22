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

/// The first bit-exact MULTI-SUPERBLOCK inter fixture: a 128x64 frame is two
/// horizontally-adjacent 64x64 superblocks. Frame 0 is a general-intra DC_PRED key
/// frame (left SB flat 100, right SB flat 150, flat chroma); frame 1 is a
/// single-reference inter frame whose two superblocks are each a single 64x64 inter
/// block. SB0 @ MI(0,0) is NEWMV with a non-zero MV (col 48 = +6 full pels in
/// eighth-pel units); SB1 @ MI(0,16) — in the SECOND superblock — is NEARMV that
/// predicts SB0's MV across the superblock boundary from the frame-wide §7.11/§7.12
/// spatial-neighbour MV stack (find_mv_stack); both skip=1 (no residual). avmdec
/// `--rawvideo --i420` and `dav2d --demuxer ivf` decode the whole stream
/// byte-for-byte identically (oracle MD5 `477a993d671e93d37b92a0d368c238ff`,
/// 24576 bytes). The OLD single-64x64 inter decoder rejected this fixture
/// ("currently accepts only the verified 64x64 frame size").
const MULTI_SB_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-2sb-inter-128x64-q80.ivf");

/// The first bit-exact 2-D-GRID inter fixture: a 128x128 frame is a 2x2 grid of
/// 64x64 superblocks. Frame 0 is a general-intra DC_PRED key frame (four flat
/// 64x64 luma superblocks 100/150/80/200, flat chroma); frame 1 is a
/// single-reference inter frame whose four superblocks are each a single 64x64
/// inter block, all skip=1 (no residual). SB0 @ MI(0,0) is NEWMV (col 48 = +6
/// full pels, eighth-pel units, has_neighbour=false); SB1 @ MI(0,16), SB2 @
/// MI(16,0), and SB3 @ MI(16,16) are NEARMV that reconstruct SB0's MV from the
/// frame-wide §7.11/§7.12 spatial-neighbour MV stack — SB2 and SB3 (in the SECOND
/// superblock ROW) predict across the SB-ROW boundary, the exact case the
/// single-SB-row brick deferred. avmdec `--rawvideo --i420` and `dav2d --demuxer
/// ivf` decode the whole stream byte-for-byte identically (oracle MD5
/// `897bf67e72ec04cb7275fae08eab700c`, 49152 bytes). The single-SB-row inter
/// decoder rejected this fixture (`inter_unsupported_frame_size`).
const GRID_INTER_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-grid-inter-128x128-q80.ivf");

/// The first bit-exact DISTINCT-neighbour-MV inter fixture: a 64x64 frame is a
/// §5.20.3 SPLIT into four 32x32 inter blocks, each carrying a DIFFERENT motion
/// vector (UNLIKE the identical-MV mvstack fixtures whose stack collapses to one
/// entry). Frame 0 is a general-intra DC_PRED key frame (four flat 32x32 quadrants
/// 100/150/60/200, flat chroma); frame 1 shifts each quadrant by a distinct amount.
/// Block 0 @ MI(0,0) is NEWMV col 64 (+8 pel), block 1 @ MI(0,8) NEWMV col -32
/// (-4 pel), block 2 @ MI(8,0) NEWMV col 32 (+4 pel), and the interior block 3 @
/// MI(8,8) is NEARMV with RefMvIdx 1: its §7.12.2 spatial stack is
/// `[col 32 (LEFT = block 2), col -32 (ABOVE = block 1), col 64, col 0]`, so
/// RefMvIdx 1 reconstructs col -32 (the ABOVE neighbour) directly — pinning the
/// §7.12.2 left-before-above scan-point ORDERING and the §5.20.7.8 DRL slot-1
/// selection. Every leaf is 32x32 (Block_Width / Block_Height == 32, not > 32), so
/// the §7.12.2.20 large-block MVP combinations do not apply. avmdec
/// `--rawvideo --i420` and `dav2d --demuxer ivf` decode the whole stream
/// byte-for-byte identically (oracle MD5 `284e1450b42180f02de7415ab0367bfe`,
/// 12288 bytes).
const MVORDER_INTER_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2frame-inter-mvorder-64x64.ivf"
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

/// The committed multi-SUPERBLOCK fixture decodes a 128x64 key frame + one
/// 128x64 multi-superblock inter frame (two 64x64 superblocks).
#[test]
fn multi_sb_fixture_decodes_two_frames() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_fixture(MULTI_SB_INTER_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the multi-superblock stream decodes a key frame + one inter frame"
    );
    for (index, output) in frames.iter().enumerate() {
        let frame = output.frame();
        assert_eq!(frame.bit_depth(), BitDepth::Eight, "frame {index}");
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "frame {index}");
        assert_eq!(
            frame.y().visible_size(),
            PlaneSize::new(128, 64).unwrap(),
            "frame {index} is a 128x64 two-superblock frame"
        );
    }
}

/// Regression pin for the per-frame decode hash of the multi-superblock fixture.
/// The raw decoded output (both frames concatenated I420) matches avmdec
/// `--rawvideo --i420` and `dav2d --demuxer ivf` byte-for-byte (oracle MD5
/// `477a993d671e93d37b92a0d368c238ff`, recorded in
/// `docs/LOCAL-REFERENCE-EVIDENCE.toml`); these `splot-dfh-sha256-v1` per-frame
/// hashes are splot's internal regression anchors for that bit-exact output. The
/// inter frame is reconstructed from the §7.11/§7.12 neighbour-predicted MVs of
/// its two superblocks (SB0 NEWMV, SB1 NEARMV reusing SB0's MV across the
/// superblock boundary).
#[test]
fn multi_sb_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(MULTI_SB_INTER_FIXTURE);
    let key_hash = splot_recon::DecodedFrameHashInput::new(frames[0].frame())
        .compute_hash()
        .to_hex();
    let inter_hash = splot_recon::DecodedFrameHashInput::new(frames[1].frame())
        .compute_hash()
        .to_hex();
    assert_eq!(
        key_hash, "2dc3b82d7f75dd5f400474fbf370a9acc2e631f65e2cc1263d0ec0684b14da15",
        "multi-superblock key-frame hash"
    );
    assert_eq!(
        inter_hash, "dc9b4c4aef4e6dc1afa43ed16a93c17dd2fab9c1e61b5ab97dbae863d62a7ebd",
        "multi-superblock inter-frame hash"
    );
    assert_ne!(
        key_hash, inter_hash,
        "the multi-superblock inter frame must differ from the key frame (real cross-SB MV shift)"
    );
}

/// The committed 2-D-GRID fixture decodes a 128x128 key frame + one 128x128
/// 2x2-superblock-grid inter frame.
#[test]
fn grid_fixture_decodes_two_frames() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_fixture(GRID_INTER_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the 2-D-grid stream decodes a key frame + one inter frame"
    );
    for (index, output) in frames.iter().enumerate() {
        let frame = output.frame();
        assert_eq!(frame.bit_depth(), BitDepth::Eight, "frame {index}");
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "frame {index}");
        assert_eq!(
            frame.y().visible_size(),
            PlaneSize::new(128, 128).unwrap(),
            "frame {index} is a 128x128 2x2-superblock-grid frame"
        );
    }
}

/// Regression pin for the per-frame decode hash of the 2-D-grid fixture. The raw
/// decoded output (both frames concatenated I420) matches avmdec `--rawvideo
/// --i420` and `dav2d --demuxer ivf` byte-for-byte (oracle MD5
/// `897bf67e72ec04cb7275fae08eab700c`, 49152 bytes, recorded in
/// `docs/LOCAL-REFERENCE-EVIDENCE.toml`); these `splot-dfh-sha256-v1` per-frame
/// hashes are splot's internal regression anchors for that bit-exact output. The
/// inter frame is reconstructed from the §7.11/§7.12 neighbour-predicted MVs of
/// its four superblocks, two of which (SB2/SB3 in the second superblock ROW)
/// predict SB0's MV across the superblock-row boundary.
#[test]
fn grid_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(GRID_INTER_FIXTURE);
    let key_hash = splot_recon::DecodedFrameHashInput::new(frames[0].frame())
        .compute_hash()
        .to_hex();
    let inter_hash = splot_recon::DecodedFrameHashInput::new(frames[1].frame())
        .compute_hash()
        .to_hex();
    assert_eq!(
        key_hash, "5619e639914803867ca0bdeb12bff97e808788607f992c661a7bcfc0bea4911a",
        "2-D-grid key-frame hash"
    );
    assert_eq!(
        inter_hash, "f23ded7e9197d7c9b0a2fdc5cdc649c079cd1fb8a1c79e913b72fb74f0c502db",
        "2-D-grid inter-frame hash"
    );
    assert_ne!(
        key_hash, inter_hash,
        "the 2-D-grid inter frame must differ from the key frame (real cross-SB MV shift)"
    );
}

/// The committed DISTINCT-neighbour-MV fixture decodes a 64x64 key frame + one
/// 64x64 multi-block inter frame (a §5.20.3 SPLIT into four 32x32 inter blocks
/// with four DIFFERENT motion vectors).
#[test]
fn mvorder_fixture_decodes_two_frames() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_fixture(MVORDER_INTER_FIXTURE);
    assert_eq!(
        frames.len(),
        2,
        "the distinct-MV stream decodes a key frame + one inter frame"
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

/// Regression pin for the per-frame decode hash of the DISTINCT-neighbour-MV
/// fixture. The raw decoded output (both frames concatenated I420) matches avmdec
/// `--rawvideo --i420` and `dav2d --demuxer ivf` byte-for-byte (oracle MD5
/// `284e1450b42180f02de7415ab0367bfe`, 12288 bytes, recorded in
/// `docs/LOCAL-REFERENCE-EVIDENCE.toml`); these `splot-dfh-sha256-v1` per-frame
/// hashes are splot's internal regression anchors for that bit-exact output. The
/// interior block 3 @ MI(8,8) is NEARMV RefMvIdx 1 over a §7.12.2 spatial stack
/// whose slot 0 is its LEFT neighbour (col 32) and slot 1 is its ABOVE neighbour
/// (col -32); reconstructing col -32 confirms the left-before-above ordering. A
/// wrong stack order would reconstruct block 3 from col 32 and change this hash.
#[test]
fn mvorder_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(MVORDER_INTER_FIXTURE);
    let key_hash = splot_recon::DecodedFrameHashInput::new(frames[0].frame())
        .compute_hash()
        .to_hex();
    let inter_hash = splot_recon::DecodedFrameHashInput::new(frames[1].frame())
        .compute_hash()
        .to_hex();
    assert_eq!(
        key_hash, "3ddad4a90c482c106f9389ef55bc87beeaf772f4bec2041da4555bbd8deb6142",
        "distinct-MV key-frame hash"
    );
    assert_eq!(
        inter_hash, "3c2a8c85c4ba4be4fa82aecbefe92baa1567f2a9c45ea88f8275c21414480ad9",
        "distinct-MV inter-frame hash"
    );
    assert_ne!(
        key_hash, inter_hash,
        "the distinct-MV inter frame must differ from the key frame"
    );
}

/// The committed three-frame MULTI-REFERENCE fixture (DECODE-INTER-MULTIREF-RUNTIME):
/// frame 0 a flat DC_PRED intra key (luma 100), frame 1 a single-reference inter
/// block (§7.7 NumTotalRefs == 1, the key) reconstructing luma 160 and refreshing a
/// SECOND reference slot, and frame 2 an inter block over TWO valid references (§7.7
/// ref_frame_idx [0, 1]) whose §5.20.7.12 single_ref selects slot 1 (the retained
/// frame 1, luma 160), NOT the key (luma 100). Encoded with --cdf-update-mode=0 so
/// no CDF adaptation propagates. avmdec `--rawvideo --i420` and `dav2d --demuxer ivf
/// --muxer yuv` decode the whole stream byte-for-byte identically (oracle MD5
/// `861078138ab514bd847ccfe22ac44fa1`, 18432 bytes).
const MULTIREF_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf");

/// The committed multi-reference fixture decodes all THREE frames bit-exact.
#[test]
fn multiref_fixture_decodes_three_frames_bit_exact() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let frames = decode_fixture(MULTIREF_FIXTURE);
    assert_eq!(
        frames.len(),
        3,
        "the stream decodes a key frame + two inter frames"
    );
    // avmdec / dav2d both decode frame 0 to luma 100 / U 120 / V 130 and frames 1 and
    // 2 to luma 160 / U 90 / V 70.
    let expected: [(u8, u8, u8); 3] = [(100, 120, 130), (160, 90, 70), (160, 90, 70)];
    for (index, output) in frames.iter().enumerate() {
        let frame = output.frame();
        assert_eq!(frame.bit_depth(), BitDepth::Eight, "frame {index}");
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "frame {index}");
        assert_eq!(
            frame.y().visible_size(),
            PlaneSize::new(64, 64).unwrap(),
            "frame {index}"
        );
        let (y, u, v) = expected[index];
        assert!(
            frame.y().samples().iter().all(|&s| s == y),
            "frame {index} luma must be flat {y}"
        );
        assert!(
            frame.u().unwrap().samples().iter().all(|&s| s == u),
            "frame {index} U must be flat {u}"
        );
        assert!(
            frame.v().unwrap().samples().iter().all(|&s| s == v),
            "frame {index} V must be flat {v}"
        );
    }
}

/// THE asymmetric retention proof: frame 2 reads the RETAINED inter frame (frame 1,
/// slot 1) via the §5.20.7.12 single_ref read, NOT the key (slot 0). Frame 1 and
/// frame 2 reconstruct identically (luma 160), and both DIFFER from the key (luma
/// 100) — so a wrong slot-0 selection would reconstruct frame 2 to the key's 100 and
/// fail this test. This proves the §7.7 two-valid-slot map + §7.23 retention +
/// single_ref selection genuinely read frame 1's samples.
#[test]
fn multiref_frame2_reads_retained_inter_frame_not_key() {
    let frames = decode_fixture(MULTIREF_FIXTURE);
    let key = frames[0].frame();
    let inter1 = frames[1].frame();
    let inter2 = frames[2].frame();
    // Frame 2 == frame 1 (it copied the retained inter frame).
    assert_eq!(
        inter2.y().samples(),
        inter1.y().samples(),
        "frame 2 luma must equal the retained frame 1 (slot 1)"
    );
    assert_eq!(
        inter2.u().unwrap().samples(),
        inter1.u().unwrap().samples(),
        "frame 2 U must equal the retained frame 1 (slot 1)"
    );
    assert_eq!(
        inter2.v().unwrap().samples(),
        inter1.v().unwrap().samples(),
        "frame 2 V must equal the retained frame 1 (slot 1)"
    );
    // Frame 2 != the key: a wrong slot-0 (key) selection would have matched the key.
    assert_ne!(
        inter2.y().samples(),
        key.y().samples(),
        "frame 2 luma must DIFFER from the key (slot 0) — proving it read slot 1, not slot 0"
    );
}

/// Per-frame decode-hash regression pin for the multi-reference fixture.
#[test]
fn multiref_fixture_per_frame_hash_is_stable() {
    let frames = decode_fixture(MULTIREF_FIXTURE);
    let hash = |i: usize| {
        splot_recon::DecodedFrameHashInput::new(frames[i].frame())
            .compute_hash()
            .to_hex()
    };
    let key_hash = hash(0);
    let inter1_hash = hash(1);
    let inter2_hash = hash(2);
    assert_eq!(
        key_hash, "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979",
        "multi-reference key-frame hash"
    );
    assert_eq!(
        inter1_hash, "7dad863f3e72b5785012a4e0497e9eb0eab98281bec147f7fb81240aa5116e1b",
        "multi-reference frame-1 hash"
    );
    assert_eq!(
        inter2_hash, "7dad863f3e72b5785012a4e0497e9eb0eab98281bec147f7fb81240aa5116e1b",
        "multi-reference frame-2 hash (== retained frame 1)"
    );
    // Frames 1 and 2 reconstruct identically (frame 2 copied the retained frame 1);
    // both differ from the key.
    assert_eq!(inter1_hash, inter2_hash, "frame 2 == retained frame 1");
    assert_ne!(
        key_hash, inter1_hash,
        "the inter frames differ from the key"
    );
}
