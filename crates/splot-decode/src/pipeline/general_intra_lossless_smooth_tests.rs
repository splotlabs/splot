// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::general_intra_tests::assert_lossless_explicit_chroma_oracle;

const LOSSLESS_NONDC_CHROMA_SMOOTH_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-smooth-intra-64x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_SMOOTH_V_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-smooth-v-intra-64x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_SMOOTH_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-smooth-intra-64x64.ivf"
);

#[test]
fn lossless_nondc_chroma_smooth_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_NONDC_CHROMA_SMOOTH_FIXTURE,
        874,
        (64, 64),
        (32, 32),
        "lossless explicit Smooth",
        "3abbca14c97210383c916c30aee1bfd8a9d0ee4062ad60b8b2d0941ff71dd7d1",
    );
}

#[test]
fn lossless_nondc_chroma_smooth_v_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_NONDC_CHROMA_SMOOTH_V_FIXTURE,
        112,
        (64, 64),
        (32, 32),
        "lossless explicit SmoothV",
        "891f663879fa406d7488f76e97931a981086ae3d7c9a38bad4272af313856f54",
    );
}

#[test]
fn lossless_sdp_nondc_chroma_smooth_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_SDP_NONDC_CHROMA_SMOOTH_FIXTURE,
        874,
        (64, 64),
        (32, 32),
        "lossless SDP explicit Smooth",
        "3abbca14c97210383c916c30aee1bfd8a9d0ee4062ad60b8b2d0941ff71dd7d1",
    );
}
