// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::general_intra_tests::assert_eight_bit_oracle;

const D135_CHROMA_NONFULL_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-d135-chroma-nonfull-32x32-q7.ivf"
);

#[test]
fn d135_chroma_nonfull_no_neighbour_frame_decodes_to_oracle() {
    assert_eight_bit_oracle(
        D135_CHROMA_NONFULL_FIXTURE,
        (32, 32),
        (16, 16),
        "9a1caf04eeed5402053edc129525d145477fcac645984e84ea2ec735ca4d47fa",
    );
}
