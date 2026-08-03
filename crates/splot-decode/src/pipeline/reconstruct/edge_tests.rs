// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Edge-extension regression tests for general intra reconstruction.

use super::*;

#[test]
fn horizontal_cardinal_mrl_tx32_clamps_bottom_edge_rows() {
    let mut workspace =
        new_general_intra_workspace::<u8>(64, 32, BitDepth::Eight, PixelFormat::Yuv420).unwrap();
    let x = 32;
    let y = 8;
    let mrl_index = 3;
    let left_col = x - 1 - mrl_index;
    for row in 0..32 {
        workspace
            .set_reconstructed_sample(PlaneId::Y, left_col, row, (40 + row) as u8)
            .unwrap();
    }
    let block_size = IntraRectBlockSize::new(5, 5).unwrap();
    let mut prediction = vec![0; block_size.sample_count()];

    super::super::one_sided::cardinal_mrl_luma_prediction_into(
        &workspace,
        IntraCardinalDirection::Horizontal,
        x,
        y,
        block_size,
        mrl_index,
        mrl_index,
        IntraEdgeAvailability::all(),
        BitDepth::Eight,
        &mut prediction,
    )
    .unwrap();

    for row in 0..24 {
        assert_eq!(&prediction[row * 32..(row + 1) * 32], &[48 + row as u8; 32]);
    }
    for row in 24..32 {
        assert_eq!(&prediction[row * 32..(row + 1) * 32], &[71; 32]);
    }
}

#[test]
fn vertical_cardinal_mrl_tx32_clamps_right_edge_columns() {
    let mut workspace =
        new_general_intra_workspace::<u8>(32, 64, BitDepth::Eight, PixelFormat::Yuv420).unwrap();
    let x = 8;
    let y = 32;
    let mrl_index = 3;
    let above_row = y - 1 - mrl_index;
    for column in 0..32 {
        workspace
            .set_reconstructed_sample(PlaneId::Y, column, above_row, (40 + column) as u8)
            .unwrap();
    }
    let block_size = IntraRectBlockSize::new(5, 5).unwrap();
    let mut prediction = vec![0; block_size.sample_count()];

    super::super::one_sided::cardinal_mrl_luma_prediction_into(
        &workspace,
        IntraCardinalDirection::Vertical,
        x,
        y,
        block_size,
        mrl_index,
        mrl_index,
        IntraEdgeAvailability::all(),
        BitDepth::Eight,
        &mut prediction,
    )
    .unwrap();

    for row in 0..32 {
        for column in 0..24 {
            assert_eq!(prediction[row * 32 + column], 48 + column as u8);
        }
        assert_eq!(&prediction[row * 32 + 24..(row + 1) * 32], &[71; 8]);
    }
}

#[test]
fn chroma_d135_top_row_filters_available_left_edge() {
    let mut ws =
        new_general_intra_workspace::<u16>(192, 128, BitDepth::Ten, PixelFormat::Yuv420).unwrap();
    for row in 0..64 {
        let value = match row {
            0..54 => 512,
            54 => 513,
            _ => 514,
        };
        ws.set_reconstructed_sample(PlaneId::U, 31, row, value)
            .unwrap();
    }

    reconstruct_general_intra_middle_neighbour_rect_block_into(
        &mut ws,
        &all_zero_luma_block(),
        135,
        PlaneId::U,
        32,
        0,
        6,
        6,
        0,
        false,
        None,
        None,
        BitDepth::Ten,
        MiddleEdgeAvailability {
            above: false,
            left: true,
        },
        TwoSidedMiddleEdgeFilters {
            above: OneSidedEdgeFilter::default(),
            left: OneSidedEdgeFilter {
                strength: 3,
                num_px: 65,
                corner_opposite: None,
            },
        },
    )
    .unwrap();

    assert_eq!(ws.reconstructed_sample(PlaneId::U, 33, 55).unwrap(), 513);
}
