// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDEF frontier tests for `DECODE-GENERAL-INTRA-CDEF`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{assert_hash, assert_yuv420_frame, decode_eight};
use splot_recon::BitDepth;

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
    let cases = [
        (
            CDEF_Q130_FIXTURE,
            "5746153715e5537ae86879a9330048331e3bdf62246fb3fdd55a372cb8299cc9",
        ),
        (
            CDEF_Q120_FIXTURE,
            "5e8576f139db38fa6cf8c9d5015bf1a7b667a9255e84d1593222848728f02362",
        ),
        (
            CDEF_DEBLOCK_Q100_FIXTURE,
            "2915c2a65660fa1ff35c965f16e1c59d629ffc279539bfe9f502f7a03de2d23d",
        ),
        (
            CDEF_UV_Q170_FIXTURE,
            "9b11d0effa3b93e84c63306e9ac865921e33f6e098cc35fbc472cbd6096ee3e6",
        ),
    ];

    for (fixture, expected_hash) in cases {
        let frame = decode_eight(fixture);
        assert_yuv420_frame(&frame, BitDepth::Eight, 128, 64);
        assert_hash(&frame, expected_hash);
    }
}
