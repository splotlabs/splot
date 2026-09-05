// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Exhaustive tests for AV2 § 5.20.7.26 `Subsampled_Size`.

#![allow(clippy::unwrap_used)]

use super::*;

type SubsampledSizeTable = [[[usize; 2]; 2]; 29];

const BLOCK_16X16: usize = 6;
const BLOCK_16X32: usize = 7;
const BLOCK_32X16: usize = 8;
const BLOCK_32X32: usize = 9;
const BLOCK_32X64: usize = 10;
const BLOCK_64X32: usize = 11;
const BLOCK_128X128: usize = 15;
const BLOCK_256X256: usize = 18;
const BLOCK_16X64: usize = 23;
const BLOCK_64X16: usize = 24;
const BLOCK_32X4: usize = 26;
const BLOCK_8X64: usize = 27;
const BLOCK_64X8: usize = 28;
const BLOCK_INVALID: usize = 29;

const SUBSAMPLED_SIZE: SubsampledSizeTable = [
    [[BLOCK_4X4, BLOCK_4X4], [BLOCK_4X4, BLOCK_4X4]],
    [[BLOCK_4X8, BLOCK_4X4], [BLOCK_INVALID, BLOCK_4X4]],
    [[BLOCK_8X4, BLOCK_INVALID], [BLOCK_4X4, BLOCK_4X4]],
    [[BLOCK_8X8, BLOCK_8X4], [BLOCK_4X8, BLOCK_4X4]],
    [[BLOCK_8X16, BLOCK_8X8], [BLOCK_4X16, BLOCK_4X8]],
    [[BLOCK_16X8, BLOCK_16X4], [BLOCK_8X8, BLOCK_8X4]],
    [[BLOCK_16X16, BLOCK_16X8], [BLOCK_8X16, BLOCK_8X8]],
    [[BLOCK_16X32, BLOCK_16X16], [BLOCK_8X32, BLOCK_8X16]],
    [[BLOCK_32X16, BLOCK_32X8], [BLOCK_16X16, BLOCK_16X8]],
    [[BLOCK_32X32, BLOCK_32X16], [BLOCK_16X32, BLOCK_16X16]],
    [[BLOCK_32X64, BLOCK_32X32], [BLOCK_16X64, BLOCK_16X32]],
    [[BLOCK_64X32, BLOCK_64X16], [BLOCK_32X32, BLOCK_32X16]],
    [[BLOCK_64X64, BLOCK_64X32], [BLOCK_32X64, BLOCK_32X32]],
    [[BLOCK_64X128, BLOCK_64X64], [BLOCK_INVALID, BLOCK_32X64]],
    [[BLOCK_128X64, BLOCK_INVALID], [BLOCK_64X64, BLOCK_64X32]],
    [[BLOCK_128X128, BLOCK_128X64], [BLOCK_64X128, BLOCK_64X64]],
    [
        [BLOCK_128X256, BLOCK_128X128],
        [BLOCK_INVALID, BLOCK_64X128],
    ],
    [
        [BLOCK_256X128, BLOCK_INVALID],
        [BLOCK_128X128, BLOCK_128X64],
    ],
    [
        [BLOCK_256X256, BLOCK_256X128],
        [BLOCK_128X256, BLOCK_128X128],
    ],
    [[BLOCK_4X16, BLOCK_4X8], [BLOCK_INVALID, BLOCK_4X8]],
    [[BLOCK_16X4, BLOCK_INVALID], [BLOCK_8X4, BLOCK_8X4]],
    [[BLOCK_8X32, BLOCK_8X16], [BLOCK_4X32, BLOCK_4X16]],
    [[BLOCK_32X8, BLOCK_32X4], [BLOCK_16X8, BLOCK_16X4]],
    [[BLOCK_16X64, BLOCK_16X32], [BLOCK_8X64, BLOCK_8X32]],
    [[BLOCK_64X16, BLOCK_64X8], [BLOCK_32X16, BLOCK_32X8]],
    [[BLOCK_4X32, BLOCK_4X16], [BLOCK_INVALID, BLOCK_4X16]],
    [[BLOCK_32X4, BLOCK_INVALID], [BLOCK_16X4, BLOCK_16X4]],
    [[BLOCK_8X64, BLOCK_8X32], [BLOCK_INVALID, BLOCK_4X32]],
    [[BLOCK_64X8, BLOCK_INVALID], [BLOCK_32X8, BLOCK_32X4]],
];

#[test]
fn get_plane_residual_size_matches_subsampled_size_table() {
    for (b_size, subsampled) in SUBSAMPLED_SIZE.into_iter().enumerate() {
        for (subsampling_x, by_y) in subsampled.into_iter().enumerate() {
            for (subsampling_y, expected) in by_y.into_iter().enumerate() {
                let expected = (expected != BLOCK_INVALID).then_some(expected);
                let actual = get_plane_residual_size(
                    BlockSize::new(b_size).unwrap(),
                    1,
                    subsampling_x == 1,
                    subsampling_y == 1,
                )
                .unwrap()
                .map(BlockSize::index);
                assert_eq!(
                    actual, expected,
                    "bSize={b_size} SubsamplingX={subsampling_x} SubsamplingY={subsampling_y}"
                );
            }
        }
    }
}
