// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::{
    BitDepth, DecodedFrameHash, DecodedFrameHashInput, IntraDcEdge, IntraDcEdges, OutputIndex,
    PixelFormat, ReferenceFrameStore, ReferenceSlot, Y4mFrameRate, Y4mWriter,
    apply_intra_ibp_dc_rect, predict_intra_dc_rect_into,
};

fn size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height).unwrap()
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(x, y, width, height).unwrap()
}

fn square(log2_size: u8) -> IntraSquareBlockSize {
    IntraSquareBlockSize::new(log2_size).unwrap()
}

fn rect_block(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
    IntraRectBlockSize::new(log2_width, log2_height).unwrap()
}

fn assert_paeth_edge_unavailable(
    result: &Result<()>,
    edge: IntraPaethEdge,
    expected_rect: PlaneRect,
) {
    assert!(matches!(
        result,
        Err(ReconError::WorkspaceIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: actual_edge,
            rect
        }) if *actual_edge == edge && *rect == expected_rect
    ));
}

fn assert_directional_edge_unavailable(
    result: &Result<()>,
    p_angle: u16,
    edge: IntraDirectionalAngleEdge,
    expected_rect: PlaneRect,
) {
    assert!(matches!(
        result,
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            p_angle: actual_p_angle,
            edge: actual_edge,
            rect
        }) if *actual_p_angle == p_angle && *actual_edge == edge && *rect == expected_rect
    ));
}

fn assert_smooth_edge_unavailable(
    result: &Result<()>,
    edge: IntraSmoothEdge,
    expected_rect: PlaneRect,
) {
    assert!(matches!(
        result,
        Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: actual_edge,
            rect
        }) if *actual_edge == edge && *rect == expected_rect
    ));
}

fn info(
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    coded_luma_size: PlaneSize,
    visible_luma_rect: PlaneRect,
) -> DecodedFrameInfo {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        pixel_format,
        coded_luma_size,
        visible_luma_rect,
    )
    .unwrap()
}

fn monochrome_info(bit_depth: BitDepth, width: usize, height: usize) -> DecodedFrameInfo {
    info(
        bit_depth,
        PixelFormat::Monochrome,
        size(width, height),
        rect(0, 0, width, height),
    )
}

fn yuv420_info(width: usize, height: usize) -> DecodedFrameInfo {
    info(
        BitDepth::Eight,
        PixelFormat::Yuv420,
        size(width, height),
        rect(0, 0, width, height),
    )
}

#[test]
fn workspace_allocates_yuv420_planes_from_frame_info() {
    let workspace = CurrentFrameWorkspace::<u8>::new(yuv420_info(5, 3), 7).unwrap();

    assert_eq!(workspace.info().pixel_format(), PixelFormat::Yuv420);
    assert_eq!(
        workspace.plane(PlaneId::Y).unwrap().storage_size(),
        size(5, 3)
    );
    assert_eq!(
        workspace.plane(PlaneId::U).unwrap().storage_size(),
        size(3, 2)
    );
    assert_eq!(
        workspace.plane(PlaneId::V).unwrap().visible_rect(),
        rect(0, 0, 3, 2)
    );
    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &[7_u8; 15]);
    assert_eq!(workspace.samples(PlaneId::U).unwrap(), &[7_u8; 6]);
}

#[test]
fn workspace_reports_missing_chroma_for_monochrome() {
    let workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 4), 0).unwrap();

    assert!(matches!(
        workspace.plane(PlaneId::U),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
}

#[test]
fn workspace_fill_checks_plane_before_sample_range() {
    let mut workspace =
        CurrentFrameWorkspace::<u16>::new(monochrome_info(BitDepth::Eight, 4, 4), 0).unwrap();

    assert!(matches!(
        workspace.fill_rect(PlaneId::U, rect(0, 0, 1, 1), 300),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
}

#[test]
fn workspace_rejects_unsupported_storage_type_before_allocation() {
    assert!(matches!(
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Ten, 4, 4), 0),
        Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten
        })
    ));
}

#[test]
fn workspace_rejects_out_of_range_fill_sample() {
    assert!(matches!(
        CurrentFrameWorkspace::<u16>::new(monochrome_info(BitDepth::Eight, 4, 4), 300),
        Err(ReconError::SampleOutOfRange {
            plane: PlaneId::Y,
            sample_index: 0,
            value: 300,
            max: 255
        })
    ));
}

#[test]
fn workspace_rejects_overflowing_required_sample_count() {
    let err = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(usize::MAX, 2),
            rect(0, 0, 1, 1),
        ),
        0,
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ReconError::ArithmeticOverflow {
            context: "current-frame workspace plane required sample count"
        }
    ));
}

#[test]
fn workspace_rejects_overflowing_allocation_byte_count() {
    let err = CurrentFrameWorkspace::<u16>::new(
        info(
            BitDepth::Ten,
            PixelFormat::Monochrome,
            size((usize::MAX / 2) + 1, 1),
            rect(0, 0, 1, 1),
        ),
        0,
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ReconError::ArithmeticOverflow {
            context: "current-frame workspace plane allocation byte count"
        }
    ));
}

#[test]
fn workspace_rect_writes_are_bounded_and_preserve_other_samples() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 3), 1).unwrap();

    workspace
        .write_rect(PlaneId::Y, rect(1, 1, 2, 2), &[9, 8, 7, 6], 2)
        .unwrap();

    assert_eq!(
        workspace.samples(PlaneId::Y).unwrap(),
        &[1, 1, 1, 1, 1, 9, 8, 1, 1, 7, 6, 1]
    );
    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 2, 2))
        .unwrap()
        .collect();
    assert_eq!(rows, vec![&[9, 8][..], &[7, 6][..]]);
}

#[test]
fn workspace_rect_writes_reject_invalid_shape_and_bounds() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 4), 0).unwrap();

    assert!(matches!(
        workspace.write_rect(PlaneId::Y, rect(0, 0, 2, 2), &[1, 2, 3, 4], 1),
        Err(ReconError::WorkspaceWriteStrideTooSmall {
            plane: PlaneId::Y,
            stride_samples: 1,
            width: 2
        })
    ));
    assert!(matches!(
        workspace.write_square_block(PlaneId::Y, 0, 0, square(2), &[1; 15]),
        Err(ReconError::WorkspaceWriteLengthMismatch {
            plane: PlaneId::Y,
            expected: 16,
            actual: 15
        })
    ));
    assert!(matches!(
        workspace.write_rect_block(PlaneId::Y, 0, 0, rect_block(2, 3), &[1; 31]),
        Err(ReconError::WorkspaceWriteLengthMismatch {
            plane: PlaneId::Y,
            expected: 32,
            actual: 31
        })
    ));
    workspace
        .fill_rect(PlaneId::Y, rect(3, 0, 2, 1), 4)
        .unwrap();
    assert_eq!(
        workspace.samples(PlaneId::Y).unwrap(),
        &[0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    assert!(matches!(
        workspace.fill_rect(PlaneId::Y, rect(4, 0, 1, 1), 4),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
}

#[test]
fn workspace_write_rect_block_clamps_frame_edge_overhang_to_in_frame_samples() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 6, 4), 0).unwrap();

    let block: [u8; 16] = [
        10, 11, 12, 13, // row 0 (in-frame columns 10,11)
        14, 15, 16, 17, // row 1 (in-frame columns 14,15)
        18, 19, 20, 21, // out-of-frame rows below are dropped
        22, 23, 24, 25,
    ];
    workspace
        .write_rect_block(PlaneId::Y, 4, 2, rect_block(2, 2), &block)
        .unwrap();

    assert_eq!(
        workspace.samples(PlaneId::Y).unwrap(),
        &[
            0, 0, 0, 0, 0, 0, // row 0
            0, 0, 0, 0, 0, 0, // row 1
            0, 0, 0, 0, 10, 11, // row 2: in-frame block row 0
            0, 0, 0, 0, 14, 15, // row 3: in-frame block row 1
        ]
    );

    assert!(matches!(
        workspace.write_rect_block(PlaneId::Y, 6, 0, rect_block(2, 2), &block),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
    assert!(matches!(
        workspace.write_rect_block(PlaneId::Y, 0, 4, rect_block(2, 2), &block),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
}

#[test]
fn workspace_intra_edge_extends_partial_bottom_left_with_last_in_frame_sample() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 4), 0).unwrap();

    workspace
        .write_rect(PlaneId::Y, rect(3, 1, 1, 3), &[50, 60, 70], 1)
        .unwrap();

    workspace
        .write_rect(PlaneId::Y, rect(4, 0, 4, 1), &[80, 90, 100, 110], 4)
        .unwrap();

    let edges = workspace
        .intra_dc_edges_for_rect(PlaneId::Y, 4, 1, rect_block(2, 2))
        .unwrap();

    assert_eq!(
        edges.left_samples().unwrap(),
        &[50, 60, 70, 70],
        "partial bottom-edge left column must extend with the LAST in-frame sample"
    );
    assert_eq!(edges.above_samples().unwrap(), &[80, 90, 100, 110]);
}

#[test]
fn workspace_intra_edge_extends_partial_right_above_with_last_in_frame_sample() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 8), 0).unwrap();

    workspace
        .write_rect(PlaneId::Y, rect(1, 3, 3, 1), &[50, 60, 70], 3)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(0, 4, 1, 4), &[80, 90, 100, 110], 1)
        .unwrap();

    let edges = workspace
        .intra_dc_edges_for_rect(PlaneId::Y, 1, 4, rect_block(2, 2))
        .unwrap();

    assert_eq!(
        edges.above_samples().unwrap(),
        &[50, 60, 70, 70],
        "partial right-edge above row must extend with the LAST in-frame sample"
    );
    assert_eq!(edges.left_samples().unwrap(), &[80, 90, 100, 110]);
}

#[test]
fn workspace_copy_rect_within_plane_copies_luma_samples() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 5, 4), 0).unwrap();
    let initial: Vec<u8> = (0..20).collect();
    workspace
        .write_rect(PlaneId::Y, rect(0, 0, 5, 4), &initial, 5)
        .unwrap();

    workspace
        .copy_rect_within_plane(PlaneId::Y, rect(0, 0, 2, 2), rect(3, 2, 2, 2))
        .unwrap();

    assert_eq!(
        workspace.samples(PlaneId::Y).unwrap(),
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0, 1, 15, 16, 17, 5, 6
        ]
    );
}

#[test]
fn workspace_copy_rect_within_plane_snapshots_overlapping_source() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 6, 1), 0).unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(0, 0, 6, 1), &[0, 1, 2, 3, 4, 5], 6)
        .unwrap();

    workspace
        .copy_rect_within_plane(PlaneId::Y, rect(0, 0, 4, 1), rect(1, 0, 4, 1))
        .unwrap();

    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &[0, 0, 1, 2, 3, 5]);
}

#[test]
fn workspace_copy_rect_within_plane_rejects_invalid_inputs_without_mutation() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(yuv420_info(4, 4), 7).unwrap();
    workspace
        .write_rect(
            PlaneId::Y,
            rect(0, 0, 4, 4),
            &(0..16).collect::<Vec<u8>>(),
            4,
        )
        .unwrap();
    // splot-copy-ok: test snapshots workspace samples to prove invalid IntrABC copies are fail-atomic.
    let before = workspace.samples(PlaneId::Y).unwrap().to_vec();

    assert!(matches!(
        workspace.copy_rect_within_plane(PlaneId::Y, rect(0, 0, 2, 2), rect(2, 0, 1, 4)),
        Err(ReconError::WorkspaceCopyShapeMismatch {
            plane: PlaneId::Y,
            ..
        })
    ));
    assert!(matches!(
        workspace.copy_rect_within_plane(PlaneId::Y, rect(3, 0, 2, 1), rect(0, 0, 2, 1)),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
    assert!(matches!(
        workspace.copy_rect_within_plane(PlaneId::Y, rect(0, 0, 2, 1), rect(3, 0, 2, 1)),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));

    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), before.as_slice());
}

#[test]
fn workspace_copy_rect_within_plane_rejects_missing_plane() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 4), 3).unwrap();
    // splot-copy-ok: test snapshots workspace samples to prove missing-plane IntrABC copy is fail-atomic.
    let before = workspace.samples(PlaneId::Y).unwrap().to_vec();

    assert!(matches!(
        workspace.copy_rect_within_plane(PlaneId::U, rect(0, 0, 1, 1), rect(1, 1, 1, 1)),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), before.as_slice());
}

#[test]
fn workspace_write_rejects_out_of_range_samples_without_partial_write() {
    let mut workspace =
        CurrentFrameWorkspace::<u16>::new(monochrome_info(BitDepth::Eight, 2, 2), 1).unwrap();

    let err = workspace
        .write_rect(PlaneId::Y, rect(0, 0, 2, 1), &[7, 300], 2)
        .unwrap_err();
    assert!(matches!(
        err,
        ReconError::SampleOutOfRange {
            plane: PlaneId::Y,
            sample_index: 1,
            value: 300,
            max: 255
        }
    ));
    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &[1, 1, 1, 1]);
}

#[test]
fn workspace_extracts_edges_and_predicts_rectangular_dc() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 10), 0).unwrap();
    let block = rect_block(2, 3);

    workspace
        .write_rect(PlaneId::Y, rect(1, 1, 1, 8), &[0, 1, 2, 3, 4, 5, 6, 7], 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(2, 0, 4, 1), &[8, 9, 10, 11], 4)
        .unwrap();

    let edges = workspace
        .intra_dc_edges_for_rect(PlaneId::Y, 2, 1, block)
        .unwrap();
    assert_eq!(edges.left_samples(), Some(&[0, 1, 2, 3, 4, 5, 6, 7][..]));
    assert_eq!(edges.above_samples(), Some(&[8, 9, 10, 11][..]));

    workspace
        .predict_intra_dc_rect(PlaneId::Y, 2, 1, block)
        .unwrap();
    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(2, 1, 4, 8))
        .unwrap()
        .collect();
    assert_eq!(rows, vec![&[5, 5, 5, 5][..]; block.height()]);

    let frame = workspace.freeze().unwrap();
    let hash_input = DecodedFrameHashInput::new(&frame);
    assert_eq!(hash_input.byte_len().unwrap(), 80);
}

#[test]
fn workspace_rectangular_dc_clamps_overhang_and_rejects_out_of_frame_origin() {
    let workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();

    let edges = workspace
        .intra_dc_edges_for_rect(PlaneId::Y, 5, 1, rect_block(2, 3))
        .unwrap();
    assert_eq!(edges.left_samples().map(<[u8]>::len), Some(8));
    assert_eq!(edges.above_samples().map(<[u8]>::len), Some(4));

    assert!(matches!(
        workspace.intra_dc_edges_for_rect(PlaneId::Y, 0, 8, rect_block(2, 2)),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
}

#[test]
fn workspace_predicts_subsampled_dc_from_in_storage_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 66, 66), 0).unwrap();
    let block = rect_block(6, 6);
    let mut left = [200u8; 64];
    let mut above = [200u8; 64];
    for index in (0..64).step_by(2) {
        left[index] = 10;
        above[index] = 30;
    }

    workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, 64), &left, 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(1, 0, 64, 1), &above, 64)
        .unwrap();
    workspace
        .predict_intra_dc_subsampled_rect(PlaneId::Y, 1, 1, block)
        .unwrap();

    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 64, 64))
        .unwrap()
        .collect();
    assert!(
        rows.iter()
            .all(|row| row.iter().all(|sample| *sample == 20))
    );
}

#[test]
fn workspace_subsampled_dc_top_left_uses_midpoint_without_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u16>::new(monochrome_info(BitDepth::Ten, 4, 4), 0).unwrap();

    workspace
        .predict_intra_dc_subsampled_rect(PlaneId::Y, 0, 0, rect_block(2, 2))
        .unwrap();

    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &[512; 16]);
}

#[test]
fn workspace_subsampled_dc_uses_available_edge_without_synthesizing_missing_edge() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 5), 0).unwrap();

    workspace
        .write_rect(PlaneId::Y, rect(0, 0, 4, 1), &[40, 40, 40, 40], 4)
        .unwrap();
    workspace
        .predict_intra_dc_subsampled_rect(PlaneId::Y, 0, 1, rect_block(2, 2))
        .unwrap();

    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(0, 1, 4, 4))
        .unwrap()
        .collect();
    assert_eq!(rows, vec![&[40, 40, 40, 40][..]; 4]);
}

#[test]
fn workspace_subsampled_dc_rejects_missing_plane_and_out_of_bounds_target() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();

    assert!(matches!(
        workspace.predict_intra_dc_subsampled_rect(PlaneId::U, 0, 0, rect_block(2, 2)),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
    assert!(matches!(
        workspace.predict_intra_dc_subsampled_rect(PlaneId::Y, 5, 1, rect_block(2, 3)),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
}

#[test]
fn workspace_predicts_ibp_dc_from_in_storage_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 10, 10), 0).unwrap();
    let block = rect_block(3, 3);
    let left = [20u8, 24, 28, 32, 36, 40, 44, 48];
    let above = [160u8, 156, 152, 148, 144, 140, 136, 132];

    workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, 8), &left, 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(1, 0, 8, 1), &above, 8)
        .unwrap();
    workspace
        .predict_intra_ibp_dc_rect(PlaneId::Y, 1, 1, block)
        .unwrap();

    let mut expected = [0u8; 64];
    predict_intra_dc_rect_into(
        BitDepth::Eight,
        block,
        IntraDcEdges::both(&left, &above),
        &mut expected,
        8,
    )
    .unwrap();
    apply_intra_ibp_dc_rect(
        BitDepth::Eight,
        block,
        IntraDcEdges::both(&left, &above),
        &mut expected,
        8,
    )
    .unwrap();

    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 8, 8))
        .unwrap()
        .collect();
    for (row, expected_row) in rows.iter().zip(expected.chunks_exact(8)) {
        assert_eq!(*row, expected_row);
    }
}

#[test]
fn workspace_ibp_dc_top_left_uses_dc_midpoint_without_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 4), 0).unwrap();

    workspace
        .predict_intra_ibp_dc_rect(PlaneId::Y, 0, 0, rect_block(2, 2))
        .unwrap();

    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &[128; 16]);
}

#[test]
fn workspace_ibp_dc_rejects_missing_plane_and_out_of_bounds_target() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();

    assert!(matches!(
        workspace.predict_intra_ibp_dc_rect(PlaneId::U, 0, 0, rect_block(2, 2)),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
    assert!(matches!(
        workspace.predict_intra_ibp_dc_rect(PlaneId::Y, 5, 1, rect_block(2, 3)),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
}

#[test]
fn workspace_ibp_dc_invalid_edge_sample_does_not_mutate_target() {
    let mut workspace =
        CurrentFrameWorkspace::<u16>::new(monochrome_info(BitDepth::Eight, 6, 6), 7).unwrap();
    {
        let mut frame = workspace.as_frame_mut();
        let mut rows = frame.y_mut().visible_rows_mut();
        rows.next().unwrap()[1] = 300;
    }

    assert!(matches!(
        workspace.predict_intra_ibp_dc_rect(PlaneId::Y, 1, 1, rect_block(2, 2)),
        Err(ReconError::IntraPredictionSampleOutOfRange {
            edge: IntraDcEdge::Above,
            sample_index: 0,
            value: 300,
            max: 255
        })
    ));
    let rows: Vec<&[u16]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 4, 4))
        .unwrap()
        .collect();
    assert_eq!(rows, vec![&[7, 7, 7, 7][..]; 4]);
}

#[test]
fn workspace_predicts_rectangular_paeth_from_in_storage_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 6, 6), 0).unwrap();
    let block = rect_block(2, 2);

    workspace
        .write_rect(PlaneId::Y, rect(0, 0, 1, 1), &[10], 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, 4), &[30, 30, 30, 30], 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(1, 0, 4, 1), &[12, 12, 12, 12], 4)
        .unwrap();

    workspace
        .predict_intra_paeth_rect(PlaneId::Y, 1, 1, block)
        .unwrap();

    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 4, 4))
        .unwrap()
        .collect();
    assert_eq!(rows, vec![&[30, 30, 30, 30][..]; block.height()]);
}

#[test]
fn workspace_paeth_rejects_missing_prepared_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();
    let block = rect_block(2, 2);

    for (x, y, edge) in [(0, 1, IntraPaethEdge::Left), (1, 0, IntraPaethEdge::Above)] {
        let result = workspace.predict_intra_paeth_rect(PlaneId::Y, x, y, block);
        assert_paeth_edge_unavailable(&result, edge, rect(x, y, 4, 4));
    }
}

#[test]
fn workspace_predicts_cardinal_directional_from_in_storage_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 6, 6), 0).unwrap();
    let block = rect_block(2, 2);

    workspace
        .write_rect(PlaneId::Y, rect(1, 0, 4, 1), &[10, 20, 30, 40], 4)
        .unwrap();
    workspace
        .predict_intra_cardinal_directional_rect(
            PlaneId::Y,
            1,
            1,
            block,
            IntraCardinalDirection::Vertical,
        )
        .unwrap();

    let vertical_rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 4, 4))
        .unwrap()
        .collect();
    assert_eq!(vertical_rows, vec![&[10, 20, 30, 40][..]; 4]);

    workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, 4), &[3, 5, 7, 9], 1)
        .unwrap();
    workspace
        .predict_intra_cardinal_directional_rect(
            PlaneId::Y,
            1,
            1,
            block,
            IntraCardinalDirection::Horizontal,
        )
        .unwrap();

    let horizontal_rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 4, 4))
        .unwrap()
        .collect();
    assert_eq!(
        horizontal_rows,
        vec![
            &[3, 3, 3, 3][..],
            &[5, 5, 5, 5][..],
            &[7, 7, 7, 7][..],
            &[9, 9, 9, 9][..],
        ]
    );
}

#[test]
fn workspace_cardinal_directional_rejects_missing_prepared_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();
    let block = rect_block(2, 2);

    for (x, y, direction, edge) in [
        (
            1,
            0,
            IntraCardinalDirection::Vertical,
            IntraDirectionalAngleEdge::Above,
        ),
        (
            0,
            1,
            IntraCardinalDirection::Horizontal,
            IntraDirectionalAngleEdge::Left,
        ),
    ] {
        let result =
            workspace.predict_intra_cardinal_directional_rect(PlaneId::Y, x, y, block, direction);
        assert_directional_edge_unavailable(&result, direction.p_angle(), edge, rect(x, y, 4, 4));
    }
}

#[test]
fn workspace_predicts_rectangular_smooth_from_in_storage_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 6, 6), 0).unwrap();
    let block = rect_block(2, 2);

    workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, 5), &[20, 40, 60, 80, 100], 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(1, 0, 5, 1), &[10, 30, 50, 70, 90], 5)
        .unwrap();

    workspace
        .predict_intra_smooth_rect(PlaneId::Y, 1, 1, block, IntraSmoothMode::Smooth)
        .unwrap();

    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(1, 1, 4, 4))
        .unwrap()
        .collect();
    assert_eq!(
        rows,
        vec![
            &[26, 45, 64, 82][..],
            &[48, 62, 75, 87][..],
            &[70, 77, 85, 91][..],
            &[91, 92, 94, 95][..],
        ]
    );
}

#[test]
fn workspace_smooth_rejects_missing_prepared_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();
    let block = rect_block(2, 2);

    for (x, y, edge) in [
        (0, 1, IntraSmoothEdge::Left),
        (1, 0, IntraSmoothEdge::Above),
        (1, 4, IntraSmoothEdge::BottomLeft),
        (4, 1, IntraSmoothEdge::TopRight),
    ] {
        let result =
            workspace.predict_intra_smooth_rect(PlaneId::Y, x, y, block, IntraSmoothMode::Smooth);
        assert_smooth_edge_unavailable(&result, edge, rect(x, y, 4, 4));
    }
}

#[test]
fn workspace_extracts_edges_and_predicts_square_dc() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();
    let block = square(2);

    workspace
        .fill_rect(PlaneId::Y, rect(1, 2, 1, 4), 10)
        .unwrap();
    workspace
        .fill_rect(PlaneId::Y, rect(2, 1, 4, 1), 30)
        .unwrap();

    let edges = workspace
        .intra_dc_edges_for_square(PlaneId::Y, 2, 2, block)
        .unwrap();
    assert_eq!(edges.left_samples(), Some(&[10, 10, 10, 10][..]));
    assert_eq!(edges.above_samples(), Some(&[30, 30, 30, 30][..]));

    workspace
        .predict_intra_dc_square(PlaneId::Y, 2, 2, block)
        .unwrap();
    let rows: Vec<&[u8]> = workspace
        .rect_rows(PlaneId::Y, rect(2, 2, 4, 4))
        .unwrap()
        .collect();
    assert_eq!(rows, vec![&[20, 20, 20, 20][..]; block.side_len()]);
}

#[test]
fn workspace_top_left_square_dc_uses_midpoint_without_edges() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 4), 0).unwrap();
    workspace
        .predict_intra_dc_square(PlaneId::Y, 0, 0, square(2))
        .unwrap();

    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &[128; 16]);
}

#[test]
fn workspace_freezes_into_hash_y4m_and_reference_store_inputs() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(yuv420_info(4, 4), 8).unwrap();
    workspace
        .fill_rect(PlaneId::U, rect(0, 0, 2, 2), 16)
        .unwrap();
    workspace
        .fill_rect(PlaneId::V, rect(0, 0, 2, 2), 32)
        .unwrap();

    let frame = workspace.freeze().unwrap();
    let hash_input = DecodedFrameHashInput::new(&frame);
    assert_eq!(hash_input.byte_len().unwrap(), 24);
    assert_eq!(
        hash_input.compute_hash().as_bytes().len(),
        DecodedFrameHash::BYTE_LEN
    );

    let mut bytes = Vec::new();
    let mut writer =
        Y4mWriter::from_frame(&mut bytes, &frame, Y4mFrameRate::new(24, 1).unwrap()).unwrap();
    writer.write_frame(&frame).unwrap();
    writer.flush().unwrap();
    assert!(bytes.starts_with(b"YUV4MPEG2 W4 H4"));

    let mut store = ReferenceFrameStore::with_capacity(1).unwrap();
    let slot = ReferenceSlot::new(0).unwrap();
    let expected_index = frame.output_index();
    assert!(store.put(slot, frame).unwrap().is_none());
    assert_eq!(
        store.get(slot).unwrap().unwrap().output_index(),
        expected_index
    );
}

#[test]
fn workspace_reconstructed_sample_reads_arbitrary_in_storage_samples() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 3), 1).unwrap();

    workspace
        .write_rect(PlaneId::Y, rect(1, 1, 2, 2), &[9, 8, 7, 6], 2)
        .unwrap();

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 2, 1).unwrap(), 8);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 1, 2).unwrap(), 7);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 1);
}

#[test]
fn workspace_reconstructed_sample_rejects_out_of_bounds_and_missing_plane() {
    let workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 3), 1).unwrap();

    assert!(matches!(
        workspace.reconstructed_sample(PlaneId::Y, 4, 0),
        Err(ReconError::WorkspaceRectOutOfBounds { .. })
    ));
    assert!(matches!(
        workspace.reconstructed_sample(PlaneId::Y, 0, 3),
        Err(ReconError::WorkspaceRectOutOfBounds { .. })
    ));
    assert!(matches!(
        workspace.reconstructed_sample(PlaneId::U, 0, 0),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
}

#[test]
fn workspace_set_reconstructed_sample_writes_and_validates() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 4, 3), 1).unwrap();

    workspace
        .set_reconstructed_sample(PlaneId::Y, 2, 1, 200)
        .unwrap();
    assert_eq!(
        workspace.reconstructed_sample(PlaneId::Y, 2, 1).unwrap(),
        200
    );
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 1, 1).unwrap(), 1);
    workspace
        .set_reconstructed_sample(PlaneId::Y, 0, 0, 255)
        .unwrap();

    assert!(matches!(
        workspace.set_reconstructed_sample(PlaneId::Y, 4, 0, 5),
        Err(ReconError::WorkspaceRectOutOfBounds { .. })
    ));
    assert!(matches!(
        workspace.set_reconstructed_sample(PlaneId::Y, 0, 3, 5),
        Err(ReconError::WorkspaceRectOutOfBounds { .. })
    ));
    assert!(matches!(
        workspace.set_reconstructed_sample(PlaneId::U, 0, 0, 5),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
}
