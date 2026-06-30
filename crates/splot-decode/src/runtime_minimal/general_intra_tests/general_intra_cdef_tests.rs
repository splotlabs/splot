// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDEF decode-hash tests for `DECODE-GENERAL-INTRA-CDEF`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::decode_general_intra_luma;
use splot_recon::{BitDepth, DecodedFrameHashInput, PixelFormat, PlaneSize};

struct CdefCase {
    name: &'static str,
    fixture: &'static [u8],
    frame_hash: &'static str,
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
            name: "cdef-q130",
            fixture: CDEF_Q130_FIXTURE,
            frame_hash: "5746153715e5537ae86879a9330048331e3bdf62246fb3fdd55a372cb8299cc9",
        },
        CdefCase {
            name: "cdef-q120",
            fixture: CDEF_Q120_FIXTURE,
            frame_hash: "5e8576f139db38fa6cf8c9d5015bf1a7b667a9255e84d1593222848728f02362",
        },
        CdefCase {
            name: "cdef-deblock-q100",
            fixture: CDEF_DEBLOCK_Q100_FIXTURE,
            frame_hash: "2915c2a65660fa1ff35c965f16e1c59d629ffc279539bfe9f502f7a03de2d23d",
        },
        CdefCase {
            name: "cdef-uv-q170",
            fixture: CDEF_UV_Q170_FIXTURE,
            frame_hash: "9b11d0effa3b93e84c63306e9ac865921e33f6e098cc35fbc472cbd6096ee3e6",
        },
    ];

    for case in &cases {
        assert_cdef_case(case);
    }
}

fn assert_cdef_case(case: &CdefCase) {
    let frame = decode_general_intra_luma(case.fixture);
    assert_eq!(frame.bit_depth(), BitDepth::Eight, "{}", case.name);
    assert_eq!(frame.pixel_format(), PixelFormat::Yuv420, "{}", case.name);
    assert_eq!(
        frame.y().visible_size(),
        PlaneSize::new(128, 64).unwrap(),
        "{}",
        case.name
    );
    let hash = DecodedFrameHashInput::new(&frame).compute_hash().to_hex();
    assert_eq!(hash, case.frame_hash, "{} must decode bit-exact", case.name);
}
