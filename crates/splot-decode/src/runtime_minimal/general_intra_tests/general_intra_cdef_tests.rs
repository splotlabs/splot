// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDEF frontier tests for `DECODE-GENERAL-INTRA-CDEF`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{assert_decode_rejects, decode_general_intra_luma};
use splot_recon::{BitDepth, DecodedFrameHashInput, PixelFormat, PlaneSize};

struct CdefCase {
    fixture: &'static [u8],
    expected: CdefExpectation,
}

enum CdefExpectation {
    Hash(&'static str),
    Unsupported(&'static str),
}

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
        CdefCase {
            fixture: CDEF_Q130_FIXTURE,
            expected: CdefExpectation::Hash(
                "5746153715e5537ae86879a9330048331e3bdf62246fb3fdd55a372cb8299cc9",
            ),
        },
        CdefCase {
            fixture: CDEF_Q120_FIXTURE,
            expected: CdefExpectation::Hash(
                "5e8576f139db38fa6cf8c9d5015bf1a7b667a9255e84d1593222848728f02362",
            ),
        },
        CdefCase {
            fixture: CDEF_DEBLOCK_Q100_FIXTURE,
            expected: CdefExpectation::Unsupported("general_intra_transform_tool_residual"),
        },
        CdefCase {
            fixture: CDEF_UV_Q170_FIXTURE,
            expected: CdefExpectation::Hash(
                "9b11d0effa3b93e84c63306e9ac865921e33f6e098cc35fbc472cbd6096ee3e6",
            ),
        },
    ];

    for case in &cases {
        assert_cdef_case(case);
    }
}

fn assert_cdef_case(case: &CdefCase) {
    match case.expected {
        CdefExpectation::Hash(hash) => assert_cdef_hash(case.fixture, hash),
        CdefExpectation::Unsupported(reason) => assert_decode_rejects(case.fixture, reason),
    }
}

fn assert_cdef_hash(fixture: &[u8], frame_hash: &str) {
    let frame = decode_general_intra_luma(fixture);
    assert_eq!(frame.bit_depth(), BitDepth::Eight);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420);
    assert_eq!(frame.y().visible_size(), PlaneSize::new(128, 64).unwrap());
    let hash = DecodedFrameHashInput::new(&frame).compute_hash().to_hex();
    assert_eq!(hash, frame_hash);
}
