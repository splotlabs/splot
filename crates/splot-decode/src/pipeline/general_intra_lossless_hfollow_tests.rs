// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::general_intra_tests::{
    assert_lossless_explicit_chroma_leftedge_pair_oracle, assert_lossless_explicit_chroma_oracle,
};

const LOSSLESS_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d45-leftedge-128x64.ivf"
);
const LOSSLESS_NONDC_CHROMA_HFOLLOW_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-hfollow-leftedge-128x64.ivf"
);
const LOSSLESS_SDP_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-chroma-d45-leftedge-128x64.ivf"
);

#[test]
fn lossless_nondc_chroma_d45_leftedge_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_leftedge_pair_oracle(
        LOSSLESS_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE,
        129,
        LOSSLESS_SDP_NONDC_CHROMA_D45_LEFTEDGE_FIXTURE,
        120,
        "D45",
        "56a0c73c398f6adb27194cb8d3908cea02791d63e79658c7552cc15a0752fc01",
    );
}

#[test]
fn lossless_nondc_chroma_hfollow_leftedge_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_NONDC_CHROMA_HFOLLOW_LEFTEDGE_FIXTURE,
        158,
        (128, 64),
        (64, 32),
        "lossless H-follow left-edge",
        "879dfd582fb00d3f068b5ac2f3eaf3a4c28449c8c763c18aa8645a5fde20448b",
    );
}
