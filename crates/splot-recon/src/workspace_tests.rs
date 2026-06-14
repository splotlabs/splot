// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::{
    BitDepth, DecodedFrameHash, DecodedFrameHashInput, OutputIndex, PixelFormat,
    ReferenceFrameStore, ReferenceSlot, Y4mFrameRate, Y4mWriter,
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

#[test]
fn workspace_allocates_yuv420_planes_from_frame_info() {
    let workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Yuv420,
            size(5, 3),
            rect(0, 0, 5, 3),
        ),
        7,
    )
    .unwrap();

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
    let workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(4, 4),
            rect(0, 0, 4, 4),
        ),
        0,
    )
    .unwrap();

    assert!(matches!(
        workspace.plane(PlaneId::U),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
}

#[test]
fn workspace_fill_checks_plane_before_sample_range() {
    let mut workspace = CurrentFrameWorkspace::<u16>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(4, 4),
            rect(0, 0, 4, 4),
        ),
        0,
    )
    .unwrap();

    assert!(matches!(
        workspace.fill_rect(PlaneId::U, rect(0, 0, 1, 1), 300),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
}

#[test]
fn workspace_rejects_unsupported_storage_type_before_allocation() {
    assert!(matches!(
        CurrentFrameWorkspace::<u8>::new(
            info(
                BitDepth::Ten,
                PixelFormat::Monochrome,
                size(4, 4),
                rect(0, 0, 4, 4),
            ),
            0,
        ),
        Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten
        })
    ));
}

#[test]
fn workspace_rejects_out_of_range_fill_sample() {
    assert!(matches!(
        CurrentFrameWorkspace::<u16>::new(
            info(
                BitDepth::Eight,
                PixelFormat::Monochrome,
                size(4, 4),
                rect(0, 0, 4, 4),
            ),
            300,
        ),
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
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(4, 3),
            rect(0, 0, 4, 3),
        ),
        1,
    )
    .unwrap();

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
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(4, 4),
            rect(0, 0, 4, 4),
        ),
        0,
    )
    .unwrap();

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
    assert!(matches!(
        workspace.fill_rect(PlaneId::Y, rect(3, 0, 2, 1), 4),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
}

#[test]
fn workspace_write_rejects_out_of_range_samples_without_partial_write() {
    let mut workspace = CurrentFrameWorkspace::<u16>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(2, 2),
            rect(0, 0, 2, 2),
        ),
        1,
    )
    .unwrap();

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
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(8, 10),
            rect(0, 0, 8, 10),
        ),
        0,
    )
    .unwrap();
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
fn workspace_rectangular_dc_rejects_out_of_bounds_target() {
    let workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(8, 8),
            rect(0, 0, 8, 8),
        ),
        0,
    )
    .unwrap();

    assert!(matches!(
        workspace.intra_dc_edges_for_rect(PlaneId::Y, 5, 1, rect_block(2, 3)),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::Y,
            ..
        })
    ));
}

#[test]
fn workspace_predicts_rectangular_paeth_from_in_storage_edges() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(6, 6),
            rect(0, 0, 6, 6),
        ),
        0,
    )
    .unwrap();
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
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(8, 8),
            rect(0, 0, 8, 8),
        ),
        0,
    )
    .unwrap();
    let block = rect_block(2, 2);

    assert!(matches!(
        workspace.predict_intra_paeth_rect(PlaneId::Y, 0, 1, block),
        Err(ReconError::WorkspaceIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: IntraPaethEdge::Left,
            rect
        }) if rect == PlaneRect::new(0, 1, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_paeth_rect(PlaneId::Y, 1, 0, block),
        Err(ReconError::WorkspaceIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: IntraPaethEdge::Above,
            rect
        }) if rect == PlaneRect::new(1, 0, 4, 4).unwrap()
    ));
}

#[test]
fn workspace_predicts_rectangular_smooth_from_in_storage_edges() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(6, 6),
            rect(0, 0, 6, 6),
        ),
        0,
    )
    .unwrap();
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
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(8, 8),
            rect(0, 0, 8, 8),
        ),
        0,
    )
    .unwrap();
    let block = rect_block(2, 2);

    assert!(matches!(
        workspace.predict_intra_smooth_rect(
            PlaneId::Y,
            0,
            1,
            block,
            IntraSmoothMode::Smooth
        ),
        Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: IntraSmoothEdge::Left,
            rect
        }) if rect == PlaneRect::new(0, 1, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_smooth_rect(
            PlaneId::Y,
            1,
            0,
            block,
            IntraSmoothMode::Smooth
        ),
        Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: IntraSmoothEdge::Above,
            rect
        }) if rect == PlaneRect::new(1, 0, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_smooth_rect(
            PlaneId::Y,
            1,
            4,
            block,
            IntraSmoothMode::Smooth
        ),
        Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: IntraSmoothEdge::BottomLeft,
            rect
        }) if rect == PlaneRect::new(1, 4, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_smooth_rect(
            PlaneId::Y,
            4,
            1,
            block,
            IntraSmoothMode::Smooth
        ),
        Err(ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
            plane: PlaneId::Y,
            edge: IntraSmoothEdge::TopRight,
            rect
        }) if rect == PlaneRect::new(4, 1, 4, 4).unwrap()
    ));
}

#[test]
fn workspace_extracts_edges_and_predicts_square_dc() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(8, 8),
            rect(0, 0, 8, 8),
        ),
        0,
    )
    .unwrap();
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
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Monochrome,
            size(4, 4),
            rect(0, 0, 4, 4),
        ),
        0,
    )
    .unwrap();
    workspace
        .predict_intra_dc_square(PlaneId::Y, 0, 0, square(2))
        .unwrap();

    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &[128; 16]);
}

#[test]
fn workspace_freezes_into_hash_y4m_and_reference_store_inputs() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(
        info(
            BitDepth::Eight,
            PixelFormat::Yuv420,
            size(4, 4),
            rect(0, 0, 4, 4),
        ),
        8,
    )
    .unwrap();
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
    assert!(store.put(slot, frame.clone()).unwrap().is_none());
    assert_eq!(
        store.get(slot).unwrap().unwrap().output_index(),
        frame.output_index()
    );
}
