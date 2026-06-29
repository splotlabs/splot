// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Exhaustive tests for AV2 § 5.20.7.26 `Subsampled_Size`.

#![allow(clippy::unwrap_used)]

use super::*;

type ResidualTable = [[[Option<usize>; 2]; 2]; 29];

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

const SUBSAMPLED_SIZE: ResidualTable = [
    [
        [Some(BLOCK_4X4), Some(BLOCK_4X4)],
        [Some(BLOCK_4X4), Some(BLOCK_4X4)],
    ],
    [[Some(BLOCK_4X8), Some(BLOCK_4X4)], [None, Some(BLOCK_4X4)]],
    [[Some(BLOCK_8X4), None], [Some(BLOCK_4X4), Some(BLOCK_4X4)]],
    [
        [Some(BLOCK_8X8), Some(BLOCK_8X4)],
        [Some(BLOCK_4X8), Some(BLOCK_4X4)],
    ],
    [
        [Some(BLOCK_8X16), Some(BLOCK_8X8)],
        [Some(BLOCK_4X16), Some(BLOCK_4X8)],
    ],
    [
        [Some(BLOCK_16X8), Some(BLOCK_16X4)],
        [Some(BLOCK_8X8), Some(BLOCK_8X4)],
    ],
    [
        [Some(BLOCK_16X16), Some(BLOCK_16X8)],
        [Some(BLOCK_8X16), Some(BLOCK_8X8)],
    ],
    [
        [Some(BLOCK_16X32), Some(BLOCK_16X16)],
        [Some(BLOCK_8X32), Some(BLOCK_8X16)],
    ],
    [
        [Some(BLOCK_32X16), Some(BLOCK_32X8)],
        [Some(BLOCK_16X16), Some(BLOCK_16X8)],
    ],
    [
        [Some(BLOCK_32X32), Some(BLOCK_32X16)],
        [Some(BLOCK_16X32), Some(BLOCK_16X16)],
    ],
    [
        [Some(BLOCK_32X64), Some(BLOCK_32X32)],
        [Some(BLOCK_16X64), Some(BLOCK_16X32)],
    ],
    [
        [Some(BLOCK_64X32), Some(BLOCK_64X16)],
        [Some(BLOCK_32X32), Some(BLOCK_32X16)],
    ],
    [
        [Some(BLOCK_64X64), Some(BLOCK_64X32)],
        [Some(BLOCK_32X64), Some(BLOCK_32X32)],
    ],
    [
        [Some(BLOCK_64X128), Some(BLOCK_64X64)],
        [None, Some(BLOCK_32X64)],
    ],
    [
        [Some(BLOCK_128X64), None],
        [Some(BLOCK_64X64), Some(BLOCK_64X32)],
    ],
    [
        [Some(BLOCK_128X128), Some(BLOCK_128X64)],
        [Some(BLOCK_64X128), Some(BLOCK_64X64)],
    ],
    [
        [Some(BLOCK_128X256), Some(BLOCK_128X128)],
        [None, Some(BLOCK_64X128)],
    ],
    [
        [Some(BLOCK_256X128), None],
        [Some(BLOCK_128X128), Some(BLOCK_128X64)],
    ],
    [
        [Some(BLOCK_256X256), Some(BLOCK_256X128)],
        [Some(BLOCK_128X256), Some(BLOCK_128X128)],
    ],
    [[Some(BLOCK_4X16), Some(BLOCK_4X8)], [None, Some(BLOCK_4X8)]],
    [[Some(BLOCK_16X4), None], [Some(BLOCK_8X4), Some(BLOCK_8X4)]],
    [
        [Some(BLOCK_8X32), Some(BLOCK_8X16)],
        [Some(BLOCK_4X32), Some(BLOCK_4X16)],
    ],
    [
        [Some(BLOCK_32X8), Some(BLOCK_32X4)],
        [Some(BLOCK_16X8), Some(BLOCK_16X4)],
    ],
    [
        [Some(BLOCK_16X64), Some(BLOCK_16X32)],
        [Some(BLOCK_8X64), Some(BLOCK_8X32)],
    ],
    [
        [Some(BLOCK_64X16), Some(BLOCK_64X8)],
        [Some(BLOCK_32X16), Some(BLOCK_32X8)],
    ],
    [
        [Some(BLOCK_4X32), Some(BLOCK_4X16)],
        [None, Some(BLOCK_4X16)],
    ],
    [
        [Some(BLOCK_32X4), None],
        [Some(BLOCK_16X4), Some(BLOCK_16X4)],
    ],
    [
        [Some(BLOCK_8X64), Some(BLOCK_8X32)],
        [None, Some(BLOCK_4X32)],
    ],
    [
        [Some(BLOCK_64X8), None],
        [Some(BLOCK_32X8), Some(BLOCK_32X4)],
    ],
];

#[test]
fn get_plane_residual_size_matches_subsampled_size_table() {
    for (b_size, subsampled) in SUBSAMPLED_SIZE.into_iter().enumerate() {
        for (subsampling_x, by_y) in subsampled.into_iter().enumerate() {
            for (subsampling_y, expected) in by_y.into_iter().enumerate() {
                let actual = get_plane_residual_size(
                    BlockSize::new(b_size).unwrap(),
                    1,
                    subsampling_x == 1,
                    subsampling_y == 1,
                )
                .unwrap()
                .valid()
                .map(BlockSize::index);
                assert_eq!(
                    actual, expected,
                    "bSize={b_size} SubsamplingX={subsampling_x} SubsamplingY={subsampling_y}"
                );
            }
        }
    }
}
