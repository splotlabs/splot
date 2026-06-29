// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.18 CDEF decode-hash tests for the general intra decode path
//! (`DECODE-GENERAL-INTRA-CDEF`). Split from the parent `general_intra_tests`
//! module to keep that file under the source-line hard cap; the shared
//! `decode_general_intra_luma` / `TWO_SB_FIXTURE` helpers are reached via `super::`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::decode_general_intra_luma;

const CDEF_Q130_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdef-intra-128x64-q130.ivf"
);
const CDEF_Q120_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdef-intra-128x64-q120.ivf"
);
const CDEF_DEBLOCK_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdefdeblock-intra-128x64-q100.ivf"
);
const CDEF_UV_Q170_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-cdefuv-intra-128x64-q170.ivf"
);

#[test]
fn cdef_active_intra_frame_decodes_to_oracle() {
    use splot_recon::{BitDepth, PixelFormat, PlaneSize};

    let cases = [
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
