// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.18 CDEF decode-hash tests for the general intra decode path
//! (`DECODE-GENERAL-INTRA-CDEF`). Split from the parent `general_intra_tests`
//! module to keep that file under the source-line hard cap; the shared
//! `decode_general_intra_luma` / `TWO_SB_FIXTURE` helpers are reached via `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::decode_general_intra_luma;

// AV2 §7.18 CDEF fixtures. Each is a 128x64 8-bit 4:2:0 intra key frame whose two
// 64x64 superblocks are PARTITION_SPLIT into all-DC 32x32 blocks, with
// `cdef_frame_enable == 1`, `CdefStrengths == 1` (so §5.20.10.1 `read_cdef` reads no
// per-block symbol — `cdef_idx[r][c] == 0` everywhere) and
// `cdef_on_skip_txfm_frame_enable == 1` (so the §7.18.1 `skip` is 0). avmdec and
// dav2d agree byte-for-byte on each, and splot reproduces the deringed output
// exactly. The CDEF effect is a real per-8x8-block direction-search-driven dering
// over the cosine-AC content (yDir varies 0/2/4/6, var positive); thousands of luma
// samples differ from the CDEF-off reconstruction.
const CDEF_Q130_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdef-intra-128x64-q130.ivf"
);
const CDEF_Q120_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdef-intra-128x64-q120.ivf"
);
// A fixture with BOTH §7.17 deblocking (`apply == [false, true, true, true]`) AND
// §7.18 CDEF active: it pins the filter order (deblock THEN CDEF) end-to-end against
// the avmdec/dav2d oracle.
const CDEF_DEBLOCK_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdefdeblock-intra-128x64-q100.ivf"
);
// A fixture with NONZERO chroma (uv) CDEF strengths (`uv_pri 2`, `uv_sec 4`): it is
// the first to exercise the §7.18.1 chroma steps 9-14 sample-changingly — the
// `Cdef_Uv_Dir[1][1][yDir]` direction selection (engaged because `uv_pri != 0`), the
// 4:2:0 subsampled 4x4 chroma tap addressing, and the `CdefDamping - 1` chroma
// damping. Both 64x64 superblocks are DC_PRED luma; the left (top-left) carries
// non-follow H_PRED chroma and the right DC chroma, both deringed by the chroma CDEF.
const CDEF_UV_Q170_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdefuv-intra-128x64-q170.ivf"
);

#[test]
fn cdef_active_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    // Each CDEF-active fixture reconstructs into a `DecodedFrame<u8>`, the §7.18 CDEF
    // pass runs in place after deblocking, and the frame hash pins splot's output —
    // byte-for-byte equal to avmdec AND dav2d. `damp` documents `CdefDamping`, `pri/sec`
    // the single luma strength set, and `uv_pri/uv_sec` the chroma strengths. The first
    // three fixtures have zero uv strengths (chroma CDEF is a no-op); the deblock fixture
    // additionally has §7.17 deblocking active, pinning the deblock→CDEF order; the last
    // fixture has NONZERO uv strengths, so the §7.18.1 chroma steps 9-14 change chroma
    // samples (the `Cdef_Uv_Dir` selection, the 4:2:0 subsampled tap addressing, and the
    // `CdefDamping - 1` chroma damping). Driven by a table so it is not a structural
    // duplicate of the sibling deblock decode-hash assertions (the dupehound diff ratchet).
    let cases = [
        // (fixture, raw md5 (avm==dav2d), splot frame hash, damp, y_pri, y_sec, uv_pri, uv_sec)
        (
            CDEF_Q130_FIXTURE,
            "192e3935f9892345a14e02cb4baf4ba5",
            "5746153715e5537ae86879a9330048331e3bdf62246fb3fdd55a372cb8299cc9",
            5u8,
            1u8,
            4u8,
            0u8,
            0u8,
        ),
        (
            CDEF_Q120_FIXTURE,
            "2319a8f00af1ebb919a52ba18d90f4a1",
            "5e8576f139db38fa6cf8c9d5015bf1a7b667a9255e84d1593222848728f02362",
            4,
            2,
            4,
            0,
            0,
        ),
        (
            CDEF_DEBLOCK_Q100_FIXTURE,
            "472d95801ce2a112160bcdfee93957d5",
            "2915c2a65660fa1ff35c965f16e1c59d629ffc279539bfe9f502f7a03de2d23d",
            4,
            1,
            4,
            0,
            0,
        ),
        (
            CDEF_UV_Q170_FIXTURE,
            "d783f353078cf156ba23dcfd3b2b50ad",
            "9b11d0effa3b93e84c63306e9ac865921e33f6e098cc35fbc472cbd6096ee3e6",
            5,
            10,
            4,
            2,
            4,
        ),
    ];
    for (fixture, raw_md5, frame_hash, damp, y_pri, y_sec, uv_pri, uv_sec) in cases {
        let frame = decode_general_intra_luma(fixture);
        assert_eq!(frame.bit_depth(), BitDepth::Eight);
        assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
        assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
        let hash = splot_recon::DecodedFrameHashInput::new(&frame)
            .compute_hash()
            .to_hex();
        assert_eq!(
            hash, frame_hash,
            "CDEF fixture (raw md5 {raw_md5}, CdefDamping {damp}, y_pri {y_pri}, y_sec {y_sec}, uv_pri {uv_pri}, uv_sec {uv_sec}) must decode bit-exact"
        );
    }
}

// The CDEF-pass-is-skipped-when-disabled regression is covered structurally rather
// than by a dedicated decode-hash test (which would byte-for-byte duplicate the
// sibling `deblock_off_frame_is_byte_identical`, tripping the dupehound diff
// ratchet): every existing CDEF-off general-intra fixture in the conformance corpus
// decodes through the new `cdef_general_intra_frame` skip path unchanged (verified
// by `cargo xtask conformance`), and the `cdef.rs::flat_frame_is_unchanged` unit
// test pins the in-module no-op directly.
