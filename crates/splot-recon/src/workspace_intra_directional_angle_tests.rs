// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use crate::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, IntraDirectionalAngle,
    IntraDirectionalAngleEdge, IntraDirectionalAngleEdges, IntraMiddleDirectionalAngle,
    IntraMiddleDirectionalAngleEdges, IntraRectBlockSize, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize, ReconError, ReconSample, predict_intra_directional_angle_rect_into,
    predict_intra_middle_directional_angle_rect_into,
};

fn size(width: usize, height: usize) -> PlaneSize {
    PlaneSize::new(width, height).unwrap()
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(x, y, width, height).unwrap()
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

fn workspace<T: ReconSample>(
    bit_depth: BitDepth,
    width: usize,
    height: usize,
    fill: T,
) -> CurrentFrameWorkspace<T> {
    workspace_with_format(bit_depth, PixelFormat::Monochrome, width, height, fill)
}

fn workspace_with_format<T: ReconSample>(
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    fill: T,
) -> CurrentFrameWorkspace<T> {
    CurrentFrameWorkspace::new(
        info(
            bit_depth,
            pixel_format,
            size(width, height),
            rect(0, 0, width, height),
        ),
        fill,
    )
    .unwrap()
}

fn workspace_rect_samples<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane: PlaneId,
    rect: PlaneRect,
) -> Vec<T> {
    workspace
        .rect_rows(plane, rect)
        .unwrap()
        .flat_map(|row| row.iter().copied())
        .collect()
}

#[test]
fn workspace_predicts_one_sided_directional_angle_from_in_storage_above_edge() {
    let block = rect_block(2, 2);
    let target = rect(1, 1, block.width(), block.height());
    let above = [12, 20, 28, 36, 44, 52, 60, 68];

    for angle in [IntraDirectionalAngle::D45, IntraDirectionalAngle::D67] {
        let mut workspace =
            workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 10, 6, 0_u8);
        workspace
            .write_rect(PlaneId::U, rect(1, 0, above.len(), 1), &above, above.len())
            .unwrap();

        let mut expected = vec![0_u8; block.width() * block.height()];
        predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            block,
            angle,
            IntraDirectionalAngleEdges::above(&above),
            &mut expected,
            block.width(),
        )
        .unwrap();

        workspace
            .predict_intra_directional_angle_rect(PlaneId::U, 1, 1, block, angle)
            .unwrap();
        assert_eq!(
            workspace_rect_samples(&workspace, PlaneId::U, target),
            expected
        );
    }
}

#[test]
fn workspace_predicts_one_sided_directional_angle_from_in_storage_left_edge() {
    let block = rect_block(2, 2);
    let target = rect(1, 1, block.width(), block.height());
    let left = [9, 18, 27, 36, 45, 54, 63, 72];
    let mut workspace = workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 6, 10, 0_u8);
    workspace
        .write_rect(PlaneId::U, rect(0, 1, 1, left.len()), &left, 1)
        .unwrap();

    let mut expected = vec![0_u8; block.width() * block.height()];
    predict_intra_directional_angle_rect_into(
        BitDepth::Eight,
        block,
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleEdges::left(&left),
        &mut expected,
        block.width(),
    )
    .unwrap();

    workspace
        .predict_intra_directional_angle_rect(PlaneId::U, 1, 1, block, IntraDirectionalAngle::D203)
        .unwrap();
    assert_eq!(
        workspace_rect_samples(&workspace, PlaneId::U, target),
        expected
    );
}

#[test]
fn workspace_predicts_middle_directional_angle_from_in_storage_edges() {
    let block = rect_block(2, 2);
    let target = rect(1, 1, block.width(), block.height());
    let above = [11, 20, 30, 40, 50];
    let left = [11, 13, 17, 19, 23];

    for angle in [
        IntraMiddleDirectionalAngle::D113,
        IntraMiddleDirectionalAngle::D135,
        IntraMiddleDirectionalAngle::D157,
    ] {
        let mut workspace = workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 6, 6, 0_u8);
        workspace
            .write_rect(PlaneId::U, rect(0, 0, above.len(), 1), &above, above.len())
            .unwrap();
        workspace
            .write_rect(PlaneId::U, rect(0, 1, 1, left.len() - 1), &left[1..], 1)
            .unwrap();

        let mut expected = vec![0_u8; block.width() * block.height()];
        predict_intra_middle_directional_angle_rect_into(
            BitDepth::Eight,
            block,
            angle,
            IntraMiddleDirectionalAngleEdges::both(&left, &above),
            &mut expected,
            block.width(),
        )
        .unwrap();

        workspace
            .predict_intra_middle_directional_angle_rect(PlaneId::U, 1, 1, block, angle)
            .unwrap();
        assert_eq!(
            workspace_rect_samples(&workspace, PlaneId::U, target),
            expected
        );
    }
}

#[test]
fn workspace_directional_angle_accepts_10_bit_u16_samples() {
    let block = rect_block(2, 2);
    let target = rect(1, 1, block.width(), block.height());
    let above = [128_u16, 256, 384, 512, 640, 768, 896, 1000];
    let mut workspace = workspace_with_format(BitDepth::Ten, PixelFormat::Yuv444, 10, 6, 0_u16);
    workspace
        .write_rect(PlaneId::U, rect(1, 0, above.len(), 1), &above, above.len())
        .unwrap();

    let mut expected = vec![0_u16; block.width() * block.height()];
    predict_intra_directional_angle_rect_into(
        BitDepth::Ten,
        block,
        IntraDirectionalAngle::D67,
        IntraDirectionalAngleEdges::above(&above),
        &mut expected,
        block.width(),
    )
    .unwrap();

    workspace
        .predict_intra_directional_angle_rect(PlaneId::U, 1, 1, block, IntraDirectionalAngle::D67)
        .unwrap();
    assert_eq!(
        workspace_rect_samples(&workspace, PlaneId::U, target),
        expected
    );
}

#[test]
fn workspace_directional_angle_rejects_luma_until_idif_is_supported() {
    let block = rect_block(2, 2);
    let mut workspace = workspace(BitDepth::Eight, 8, 8, 0_u8);

    assert!(matches!(
        workspace.predict_intra_directional_angle_rect(
            PlaneId::Y,
            1,
            1,
            block,
            IntraDirectionalAngle::D45
        ),
        Err(
            ReconError::WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported {
                plane: PlaneId::Y,
                p_angle: 45,
                rect
            }
        ) if rect == PlaneRect::new(1, 1, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_middle_directional_angle_rect(
            PlaneId::Y,
            1,
            1,
            block,
            IntraMiddleDirectionalAngle::D135
        ),
        Err(
            ReconError::WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported {
                plane: PlaneId::Y,
                p_angle: 135,
                rect
            }
        ) if rect == PlaneRect::new(1, 1, 4, 4).unwrap()
    ));
}

#[test]
fn workspace_directional_angle_rejects_missing_prepared_edges() {
    let block = rect_block(2, 2);
    let mut workspace = workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 8, 8, 0_u8);

    assert!(matches!(
        workspace.predict_intra_directional_angle_rect(
            PlaneId::U,
            1,
            0,
            block,
            IntraDirectionalAngle::D45
        ),
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            p_angle: 45,
            edge: IntraDirectionalAngleEdge::Above,
            rect
        }) if rect == PlaneRect::new(1, 0, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_directional_angle_rect(
            PlaneId::U,
            1,
            1,
            block,
            IntraDirectionalAngle::D67
        ),
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            p_angle: 67,
            edge: IntraDirectionalAngleEdge::Above,
            rect
        }) if rect == PlaneRect::new(1, 1, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_directional_angle_rect(
            PlaneId::U,
            0,
            1,
            block,
            IntraDirectionalAngle::D203
        ),
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            p_angle: 203,
            edge: IntraDirectionalAngleEdge::Left,
            rect
        }) if rect == PlaneRect::new(0, 1, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_directional_angle_rect(
            PlaneId::U,
            1,
            1,
            block,
            IntraDirectionalAngle::D203
        ),
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            p_angle: 203,
            edge: IntraDirectionalAngleEdge::Left,
            rect
        }) if rect == PlaneRect::new(1, 1, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_middle_directional_angle_rect(
            PlaneId::U,
            0,
            1,
            block,
            IntraMiddleDirectionalAngle::D113
        ),
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            p_angle: 113,
            edge: IntraDirectionalAngleEdge::Left,
            rect
        }) if rect == PlaneRect::new(0, 1, 4, 4).unwrap()
    ));
    assert!(matches!(
        workspace.predict_intra_middle_directional_angle_rect(
            PlaneId::U,
            1,
            0,
            block,
            IntraMiddleDirectionalAngle::D135
        ),
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            p_angle: 135,
            edge: IntraDirectionalAngleEdge::Above,
            rect
        }) if rect == PlaneRect::new(1, 0, 4, 4).unwrap()
    ));
}

#[test]
fn workspace_directional_angle_rejects_missing_plane_and_out_of_bounds_target() {
    let block = rect_block(2, 2);
    let mut workspace = workspace(BitDepth::Eight, 8, 8, 0_u8);

    assert!(matches!(
        workspace.predict_intra_directional_angle_rect(
            PlaneId::U,
            1,
            1,
            block,
            IntraDirectionalAngle::D45
        ),
        Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
    ));
    let mut workspace = workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 8, 8, 0_u8);
    assert!(matches!(
        workspace.predict_intra_middle_directional_angle_rect(
            PlaneId::U,
            5,
            1,
            block,
            IntraMiddleDirectionalAngle::D135
        ),
        Err(ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::U,
            ..
        })
    ));
}

#[test]
fn workspace_directional_angle_invalid_edge_sample_does_not_mutate_target() {
    let block = rect_block(2, 2);
    let mut workspace = workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 10, 6, 7_u16);
    {
        let mut frame = workspace.as_frame_mut();
        let mut rows = frame.u_mut().unwrap().visible_rows_mut();
        rows.next().unwrap()[1] = 300;
    }
    // splot-copy-ok: snapshot workspace samples for no-mutation regression assertion
    let before = workspace.samples(PlaneId::U).unwrap().to_vec();

    assert!(matches!(
        workspace.predict_intra_directional_angle_rect(
            PlaneId::U,
            1,
            1,
            block,
            IntraDirectionalAngle::D45
        ),
        Err(ReconError::IntraDirectionalAngleSampleOutOfRange {
            edge: IntraDirectionalAngleEdge::Above,
            sample_index: 0,
            value: 300,
            max: 255
        })
    ));
    assert_eq!(workspace.samples(PlaneId::U).unwrap(), before);
}
