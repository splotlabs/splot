// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::general_intra_tests::assert_eight_bit_oracle;

const HORIZONTAL_CHROMA_TILE_ORIGIN_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-cardinal-h-chroma-tile-origin-128x64-q1.ivf"
);

#[test]
fn horizontal_chroma_tile_origin_decodes_to_oracle() {
    assert_eight_bit_oracle(
        HORIZONTAL_CHROMA_TILE_ORIGIN_FIXTURE,
        (128, 64),
        (64, 32),
        "8efb0e30f50271df41154e6227cbd0ad6df2dc6405c7c29b5615f7ba188ebf6d",
    );
}
