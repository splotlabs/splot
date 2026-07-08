// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Deblock frontier tests for general intra decode.

use super::{TWO_SB_FIXTURE, assert_hash, assert_yuv420_frame, decode_eight, frame_hash};
use splot_recon::BitDepth;

const DEBLOCK_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-deblock-intra-128x64-q100.ivf"
);
const DEBLOCK_Q98_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-deblock-intra-128x64-q98.ivf"
);
const DEBLOCK_WIDE_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-2sb-deblockwide-intra-128x64-q100.ivf"
);
const DEBLOCK_GRID_Q100_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-grid-deblock-intra-128x128-q100.ivf"
);

#[test]
fn deblock_active_intra_frame_decodes_to_oracle() {
    let q100 = decode_eight(DEBLOCK_Q100_FIXTURE);
    assert_yuv420_frame(&q100, BitDepth::Eight, 128, 64);
    assert_hash(
        &q100,
        "a83cf84a6eab00d8c1e6aaf64e7aeba2049e7d1721a90147067ecc627f0aea0b",
    );
    let q98 = decode_eight(DEBLOCK_Q98_FIXTURE);
    assert_yuv420_frame(&q98, BitDepth::Eight, 128, 64);
    assert_hash(
        &q98,
        "3306f7ab5f192cc4f30f6c564e9b52a9d868de77ffd9fb913651cb58a8d8a3f1",
    );
    let wide = decode_eight(DEBLOCK_WIDE_Q100_FIXTURE);
    assert_yuv420_frame(&wide, BitDepth::Eight, 128, 64);
    assert_hash(
        &wide,
        "199c62093efa3b644fb4d519ae082516d9a4f9a77b13f116ddc62c49fa8648d7",
    );
    let grid = decode_eight(DEBLOCK_GRID_Q100_FIXTURE);
    assert_yuv420_frame(&grid, BitDepth::Eight, 128, 128);
    assert_hash(
        &grid,
        "242cb4df75288e96ab231c41f3bcb956aa0159d35a304e82c89e04814bd77ef0",
    );
}

#[test]
fn deblock_off_frame_is_byte_identical() {
    let frame = decode_eight(TWO_SB_FIXTURE);
    assert_eq!(
        frame_hash(&frame),
        "18ba32ffb8d818689cbded3dbd5c44602bb091c1f9750c1bb062e6f80498540f",
        "deblock-off fixture must stay byte-identical"
    );
}
