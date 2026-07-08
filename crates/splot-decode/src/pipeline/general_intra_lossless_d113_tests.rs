// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::general_intra_tests::assert_lossless_explicit_chroma_oracle;

const LOSSLESS_NONDC_CHROMA_D113_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-lossless-nondc-chroma-d113-intra-64x64.ivf"
);

#[test]
fn lossless_nondc_chroma_d113_frame_decodes_to_oracle() {
    assert_lossless_explicit_chroma_oracle(
        LOSSLESS_NONDC_CHROMA_D113_FIXTURE,
        342,
        (64, 64),
        (32, 32),
        "lossless explicit D113",
        "840bcce567e791d1b1b97b650c2f5e3b03da1660c0ba9a2415bb1abf334222dc",
    );
}
