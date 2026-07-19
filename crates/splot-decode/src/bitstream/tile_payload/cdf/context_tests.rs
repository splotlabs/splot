// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::tables::cdf::{
    DEFAULT_DO_EXT_PARTITION_CDF, DEFAULT_DO_SPLIT_CDF, DEFAULT_DO_SQUARE_SPLIT_CDF,
    DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF, DEFAULT_RECT_TYPE_CDF,
};

use super::super::{FrameCdfSubset, TileCdfError, TileCdfSelector};
use super::*;

const BLOCK_4X4: usize = 0;
const BLOCK_16X16: usize = 6;
const BLOCK_32X32: usize = 9;
const BLOCK_256X256: usize = BLOCK_256X256_INDEX;
const PARTITION_CONTEXT_ABOVE: [u8; BLOCK_SIZES] = [
    63, 63, 62, 62, 62, 60, 60, 60, 56, 56, 56, 48, 48, 48, 32, 32, 32, 0, 0, 63, 60, 62, 56, 60,
    48, 63, 56, 62, 48,
];
const PARTITION_CONTEXT_LEFT: [u8; BLOCK_SIZES] = [
    63, 62, 63, 62, 60, 62, 60, 56, 60, 56, 48, 56, 48, 32, 48, 32, 0, 32, 0, 60, 63, 56, 62, 48,
    60, 56, 63, 48, 62,
];

fn mi_grid(values: &[usize]) -> Vec<u8> {
    values
        .iter()
        .map(|&value| u8::try_from(value).unwrap())
        .collect()
}

fn do_split(plane_start: usize, ctx: usize) -> TileCdfSelector {
    TileCdfSelector::DoSplit { plane_start, ctx }
}

fn rect_type(plane_start: usize, ctx: usize) -> TileCdfSelector {
    TileCdfSelector::RectType { plane_start, ctx }
}

fn do_ext(plane_start: usize, ctx: usize) -> TileCdfSelector {
    TileCdfSelector::DoExtPartition { plane_start, ctx }
}

fn do_uneven(plane_start: usize, ctx: usize) -> TileCdfSelector {
    TileCdfSelector::DoUneven4WayPartition { plane_start, ctx }
}

fn do_square(plane_start: usize, ctx: usize) -> TileCdfSelector {
    TileCdfSelector::DoSquareSplit { plane_start, ctx }
}

fn above_partition_context(block_size: usize) -> u8 {
    PARTITION_CONTEXT_ABOVE[block_size]
}

fn left_partition_context(block_size: usize) -> u8 {
    PARTITION_CONTEXT_LEFT[block_size]
}

#[test]
fn derives_square_split_contexts_from_availability_gated_grid_neighbors() {
    let mi_sizes = mi_grid(&[BLOCK_256X256, BLOCK_4X4, BLOCK_4X4, BLOCK_256X256]);

    assert_eq!(
        SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, true, &mi_sizes, 2)
            .unwrap()
            .do_square_split_selector()
            .unwrap(),
        do_square(0, 3)
    );
    assert_eq!(
        SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, false, &mi_sizes, 2)
            .unwrap()
            .do_square_split_selector()
            .unwrap(),
        do_square(0, 1)
    );
    assert_eq!(
        SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, false, true, &mi_sizes, 2)
            .unwrap()
            .do_square_split_selector()
            .unwrap(),
        do_square(0, 2)
    );

    let empty_mi_sizes = Vec::new();
    assert_eq!(
        SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, false, &empty_mi_sizes, 0)
            .unwrap()
            .do_square_split_selector()
            .unwrap(),
        do_square(0, 0)
    );
}

#[test]
fn derives_square_split_block_256_bonus_contexts() {
    let mi_sizes = mi_grid(&[BLOCK_256X256, BLOCK_4X4, BLOCK_4X4, BLOCK_256X256]);

    assert_eq!(
        SquareSplitContextInput::new(BLOCK_256X256, 0, 1, 1, false, false, &mi_sizes, 2)
            .unwrap()
            .do_square_split_selector()
            .unwrap(),
        do_square(0, 4)
    );
    assert_eq!(
        SquareSplitContextInput::new(BLOCK_256X256, 0, 1, 1, true, true, &mi_sizes, 2)
            .unwrap()
            .do_square_split_selector()
            .unwrap(),
        do_square(0, 7)
    );
}

#[test]
fn derives_do_split_and_rect_type_contexts_from_neighbors() {
    let left0 = [left_partition_context(BLOCK_4X4)];
    let left1 = [left_partition_context(BLOCK_256X256)];
    let above0 = [above_partition_context(BLOCK_4X4)];
    let above1 = [above_partition_context(BLOCK_256X256)];
    let input =
        PartitionContextInput::new(BLOCK_16X16, 0, 0, 0, [&left0, &left1], [&above0, &above1])
            .unwrap();

    assert_eq!(input.do_split_selector().unwrap(), do_split(0, 7));
    assert_eq!(input.rect_type_selector().unwrap(), rect_type(0, 3));
}

#[test]
fn chroma_sdp_do_split_reads_plane_one_cdf_and_neighbor_array() {
    let left0 = [left_partition_context(BLOCK_4X4)];
    let left1 = [left_partition_context(BLOCK_256X256)];
    let above0 = [above_partition_context(BLOCK_4X4)];
    let above1 = [above_partition_context(BLOCK_256X256)];
    let input =
        PartitionContextInput::new(BLOCK_16X16, 1, 0, 0, [&left0, &left1], [&above0, &above1])
            .unwrap();

    assert_eq!(input.do_split_selector().unwrap(), do_split(1, 4));
    let luma =
        PartitionContextInput::new(BLOCK_16X16, 0, 0, 0, [&left0, &left1], [&above0, &above1])
            .unwrap();
    assert_eq!(luma.do_split_selector().unwrap(), do_split(0, 7));

    let empty_mi_sizes = Vec::new();
    assert!(matches!(
        SquareSplitContextInput::new(BLOCK_16X16, 1, 0, 0, false, false, &empty_mi_sizes, 0)
            .unwrap()
            .do_square_split_selector(),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSquareSplit,
            ..
        })
    ));
}

#[test]
fn derives_horizontal_ext_and_uneven_contexts_from_left_neighbors() {
    let mut left0 = [left_partition_context(BLOCK_256X256); 8];
    let left1 = [left_partition_context(BLOCK_256X256); 8];
    let above0 = [above_partition_context(BLOCK_256X256); 8];
    let above1 = [above_partition_context(BLOCK_256X256); 8];
    left0[1] = left_partition_context(BLOCK_16X16);
    left0[5] = left_partition_context(BLOCK_4X4);
    let input =
        PartitionContextInput::new(BLOCK_32X32, 0, 1, 0, [&left0, &left1], [&above0, &above1])
            .unwrap();

    assert_eq!(
        input
            .do_ext_partition_selector(RectPartitionType::Horz)
            .unwrap(),
        do_ext(0, 9)
    );
    assert_eq!(
        input
            .do_uneven_4way_partition_selector(RectPartitionType::Horz)
            .unwrap(),
        do_uneven(0, 9)
    );
}

#[test]
fn derives_vertical_ext_context_from_above_neighbors() {
    let left0 = [left_partition_context(BLOCK_256X256); 8];
    let left1 = [left_partition_context(BLOCK_256X256); 8];
    let mut above0 = [above_partition_context(BLOCK_256X256); 8];
    let above1 = [above_partition_context(BLOCK_256X256); 8];
    above0[2] = above_partition_context(BLOCK_16X16);
    above0[6] = above_partition_context(BLOCK_4X4);
    let input =
        PartitionContextInput::new(BLOCK_32X32, 0, 0, 2, [&left0, &left1], [&above0, &above1])
            .unwrap();

    assert_eq!(
        input
            .do_ext_partition_selector(RectPartitionType::Vert)
            .unwrap(),
        do_ext(0, 9)
    );
}

#[test]
fn rejects_invalid_context_inputs_before_table_use() {
    let empty: [u8; 0] = [];
    let err = PartitionContextInput::new(BLOCK_SIZES, 0, 0, 0, [&empty, &empty], [&empty, &empty])
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::BlockSizeOutOfRange {
            table: "bSize",
            b_size: BLOCK_SIZES,
            max_exclusive: BLOCK_SIZES,
        }
    );

    let left0 = [left_partition_context(BLOCK_4X4)];
    let left1 = [left_partition_context(BLOCK_4X4)];
    let above0 = [above_partition_context(BLOCK_4X4)];
    let above1 = [above_partition_context(BLOCK_4X4)];
    let err =
        PartitionContextInput::new(BLOCK_16X16, 2, 0, 0, [&left0, &left1], [&above0, &above1])
            .unwrap()
            .do_split_selector()
            .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSplit,
            index_name: "plane_start",
            actual: 2,
            max_exclusive: DO_SPLIT_PLANE_CONTEXTS,
        }
    );

    let err =
        PartitionContextInput::new(BLOCK_16X16, 0, 1, 0, [&left0, &left1], [&above0, &above1])
            .unwrap()
            .do_split_selector()
            .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionNeighborOutOfRange {
            array: "LeftMiSizes",
            plane_start: 0,
            index: 1,
            len: 1,
        }
    );
}

#[test]
fn rejects_square_split_invalid_inputs_before_grid_table_use() {
    let empty_mi_sizes = Vec::new();
    let err = SquareSplitContextInput::new(BLOCK_SIZES, 0, 0, 0, false, false, &empty_mi_sizes, 0)
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::BlockSizeOutOfRange {
            table: "bSize",
            b_size: BLOCK_SIZES,
            max_exclusive: BLOCK_SIZES,
        }
    );

    let err = SquareSplitContextInput::new(BLOCK_16X16, 1, 0, 0, false, false, &empty_mi_sizes, 0)
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSquareSplit,
            index_name: "plane_start",
            actual: 1,
            max_exclusive: 1,
        }
    );

    let err = SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, true, false, &empty_mi_sizes, 0)
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionGridCoordinateUnderflow {
            array: "MiSizes",
            plane_start: 0,
            coordinate: "r",
            actual: 0,
            subtract: 1,
        }
    );

    let err = SquareSplitContextInput::new(BLOCK_16X16, 0, 0, 0, false, true, &empty_mi_sizes, 0)
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionGridCoordinateUnderflow {
            array: "MiSizes",
            plane_start: 0,
            coordinate: "c",
            actual: 0,
            subtract: 1,
        }
    );
}

#[test]
fn rejects_square_split_missing_grid_cells_and_invalid_block_sizes() {
    let empty_grid = Vec::new();
    let one_cell_grid = mi_grid(&[BLOCK_4X4]);
    let err = SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 0, true, false, &empty_grid, 0)
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionGridRowOutOfRange {
            array: "MiSizes",
            plane_start: 0,
            row: 0,
            rows: 0,
        }
    );

    let err = SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, false, &one_cell_grid, 1)
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionGridColumnOutOfRange {
            array: "MiSizes",
            plane_start: 0,
            row: 0,
            col: 1,
            cols: 1,
        }
    );

    let invalid_grid = mi_grid(&[BLOCK_4X4, BLOCK_SIZES, BLOCK_4X4, BLOCK_4X4]);
    let err = SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, false, &invalid_grid, 2)
        .unwrap()
        .do_square_split_selector()
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionGridBlockSizeOutOfRange {
            array: "MiSizes",
            plane_start: 0,
            row: 0,
            col: 1,
            block_size: BLOCK_SIZES,
            max_exclusive: BLOCK_SIZES,
        }
    );
}

#[test]
fn rejects_second_half_and_neighbor_block_size_bounds() {
    let left0 = [left_partition_context(BLOCK_4X4); 4];
    let left1 = [left_partition_context(BLOCK_4X4); 4];
    let above0 = [above_partition_context(BLOCK_4X4); 4];
    let above1 = [above_partition_context(BLOCK_4X4); 4];
    let err =
        PartitionContextInput::new(BLOCK_32X32, 0, 0, 0, [&left0, &left1], [&above0, &above1])
            .unwrap()
            .do_ext_partition_selector(RectPartitionType::Vert)
            .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionNeighborOutOfRange {
            array: "AboveMiSizes",
            plane_start: 0,
            index: 4,
            len: 4,
        }
    );

    let bad_left0 = [64];
    let err = PartitionContextInput::new(
        BLOCK_16X16,
        0,
        0,
        0,
        [&bad_left0, &left1],
        [&above0, &above1],
    )
    .unwrap()
    .do_split_selector()
    .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::PartitionNeighborContextOutOfRange {
            array: "LeftMiSizes",
            plane_start: 0,
            index: 0,
            context: 64,
            max_exclusive: 64,
        }
    );
}

#[test]
fn derived_selectors_index_generated_default_rows() {
    let left0 = [left_partition_context(BLOCK_4X4); 8];
    let left1 = [left_partition_context(BLOCK_256X256); 8];
    let above0 = [above_partition_context(BLOCK_4X4); 8];
    let above1 = [above_partition_context(BLOCK_256X256); 8];
    let mi_sizes = mi_grid(&[BLOCK_256X256, BLOCK_4X4, BLOCK_4X4, BLOCK_256X256]);
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();

    let input =
        PartitionContextInput::new(BLOCK_16X16, 0, 0, 0, [&left0, &left1], [&above0, &above1])
            .unwrap();
    assert_eq!(
        tile.row(input.do_split_selector().unwrap()).unwrap(),
        DEFAULT_DO_SPLIT_CDF[0][7].as_slice()
    );
    assert_eq!(
        tile.row(input.rect_type_selector().unwrap()).unwrap(),
        DEFAULT_RECT_TYPE_CDF[0][3].as_slice()
    );
    let square_input =
        SquareSplitContextInput::new(BLOCK_16X16, 0, 1, 1, true, true, &mi_sizes, 2).unwrap();
    assert_eq!(
        tile.row(square_input.do_square_split_selector().unwrap())
            .unwrap(),
        DEFAULT_DO_SQUARE_SPLIT_CDF[0][3].as_slice()
    );

    let input =
        PartitionContextInput::new(BLOCK_32X32, 0, 0, 0, [&left0, &left1], [&above0, &above1])
            .unwrap();
    assert_eq!(
        tile.row(
            input
                .do_ext_partition_selector(RectPartitionType::Horz)
                .unwrap()
        )
        .unwrap(),
        DEFAULT_DO_EXT_PARTITION_CDF[0][11].as_slice()
    );
    assert_eq!(
        tile.row(
            input
                .do_uneven_4way_partition_selector(RectPartitionType::Horz)
                .unwrap()
        )
        .unwrap(),
        DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[0][11].as_slice()
    );
}
