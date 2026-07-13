// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::general_intra_tests::assert_eight_bit_oracle;

const D157_CHROMA_NONFULL_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-d157-chroma-nonfull-32x32-q1.ivf"
);

#[test]
fn d157_chroma_nonfull_no_neighbour_frame_decodes_to_oracle() {
    assert_eight_bit_oracle(
        D157_CHROMA_NONFULL_FIXTURE,
        (32, 32),
        (16, 16),
        "9084f7311718d3558b0e9b5ed09315a989f39b55b585d3e387fe878c436d40e7",
    );
}
