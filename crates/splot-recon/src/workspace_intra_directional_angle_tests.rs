// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use crate::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, IntraDirectionalAngle,
    IntraDirectionalAngleEdge, IntraDirectionalAngleEdges, IntraDirectionalAngleIdifEdges,
    IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges, IntraRectBlockSize, OutputIndex,
    PixelFormat, PlaneId, PlaneRect, PlaneSize, ReconError, ReconSample,
    predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
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

fn one_sided_idif_edge<T: Copy>(corner: T, edge: &[T]) -> Vec<T> {
    let mut idif = vec![corner, corner];
    // splot-copy-ok: materialize the independent expected IDIF edge in test storage
    idif.extend_from_slice(edge);
    // splot-copy-ok: materialize the independent expected IDIF tail in test storage
    idif.extend_from_slice(&[edge[edge.len() - 1]; 2]);
    idif
}

fn predict_middle_d135_with_edges(
    workspace: &mut CurrentFrameWorkspace<u8>,
    plane: PlaneId,
    block: IntraRectBlockSize,
    corner: u8,
    above: &[u8],
    left: &[u8],
) {
    workspace
        .write_rect(plane, rect(0, 0, 1, 1), &[corner], 1)
        .unwrap();
    workspace
        .write_rect(plane, rect(1, 0, above.len(), 1), above, above.len())
        .unwrap();
    for (i, sample) in left.iter().enumerate() {
        workspace
            .write_rect(plane, rect(0, 1 + i, 1, 1), &[*sample], 1)
            .unwrap();
    }
    workspace
        .predict_intra_middle_directional_angle_rect(
            plane,
            1,
            1,
            block,
            IntraMiddleDirectionalAngle::D135,
        )
        .unwrap();
}

fn assert_edge_unavailable(
    result: &Result<(), ReconError>,
    plane: PlaneId,
    p_angle: u16,
    edge: IntraDirectionalAngleEdge,
    expected_rect: PlaneRect,
) {
    assert!(matches!(
        result,
        Err(ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: actual_plane,
            p_angle: actual_p_angle,
            edge: actual_edge,
            rect
        }) if *actual_plane == plane
            && *actual_p_angle == p_angle
            && *actual_edge == edge
            && *rect == expected_rect
    ));
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
    let corner = 64_u16;
    let above = [128_u16, 256, 384, 512, 640, 768, 896, 1000];
    let idif = one_sided_idif_edge(corner, &above);
    let mut workspace = workspace(BitDepth::Ten, 10, 6, 0_u16);
    workspace
        .write_rect(PlaneId::Y, rect(0, 0, 1, 1), &[corner], 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(1, 0, above.len(), 1), &above, above.len())
        .unwrap();

    let mut expected = vec![0_u16; block.width() * block.height()];
    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Ten,
        block,
        IntraDirectionalAngle::D67,
        IntraDirectionalAngleIdifEdges::above(&idif),
        &mut expected,
        block.width(),
    )
    .unwrap();

    workspace
        .predict_intra_directional_angle_rect(PlaneId::Y, 1, 1, block, IntraDirectionalAngle::D67)
        .unwrap();
    assert_eq!(
        workspace_rect_samples(&workspace, PlaneId::Y, target),
        expected
    );
}

#[test]
fn workspace_predicts_luma_one_sided_idif_from_in_storage_above_edge() {
    let block = rect_block(2, 2);
    let target = rect(1, 1, block.width(), block.height());
    let corner = 7_u8;
    let above = [12, 20, 28, 36, 44, 52, 60, 68];
    let idif = one_sided_idif_edge(corner, &above);

    for angle in [IntraDirectionalAngle::D45, IntraDirectionalAngle::D67] {
        let mut workspace = workspace(BitDepth::Eight, 10, 6, 0_u8);
        workspace
            .write_rect(PlaneId::Y, rect(0, 0, 1, 1), &[corner], 1)
            .unwrap();
        workspace
            .write_rect(PlaneId::Y, rect(1, 0, above.len(), 1), &above, above.len())
            .unwrap();

        let mut expected = vec![0_u8; block.width() * block.height()];
        predict_intra_directional_angle_rect_one_sided_idif_into(
            BitDepth::Eight,
            block,
            angle,
            IntraDirectionalAngleIdifEdges::above(&idif),
            &mut expected,
            block.width(),
        )
        .unwrap();

        workspace
            .predict_intra_directional_angle_rect(PlaneId::Y, 1, 1, block, angle)
            .unwrap();
        assert_eq!(
            workspace_rect_samples(&workspace, PlaneId::Y, target),
            expected
        );
    }
}

#[test]
fn workspace_predicts_luma_one_sided_idif_from_in_storage_left_edge() {
    let block = rect_block(2, 2);
    let target = rect(1, 1, block.width(), block.height());
    let corner = 5_u8;
    let left = [10, 20, 30, 40, 50, 60, 70, 80];
    let idif = one_sided_idif_edge(corner, &left);
    let mut workspace = workspace(BitDepth::Eight, 6, 10, 0_u8);
    workspace
        .write_rect(PlaneId::Y, rect(0, 0, 1, 1), &[corner], 1)
        .unwrap();
    workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, left.len()), &left, 1)
        .unwrap();

    let mut expected = vec![0_u8; block.width() * block.height()];
    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        block,
        IntraDirectionalAngle::D203,
        IntraDirectionalAngleIdifEdges::left(&idif),
        &mut expected,
        block.width(),
    )
    .unwrap();

    workspace
        .predict_intra_directional_angle_rect(PlaneId::Y, 1, 1, block, IntraDirectionalAngle::D203)
        .unwrap();
    assert_eq!(
        workspace_rect_samples(&workspace, PlaneId::Y, target),
        [
            13, 17, 21, 25, 24, 28, 31, 35, 34, 38, 41, 45, 44, 48, 51, 55
        ]
    );
    assert_eq!(
        expected,
        workspace_rect_samples(&workspace, PlaneId::Y, target)
    );
}

#[test]
fn workspace_luma_one_sided_idif_uses_own_edge_sample_when_corner_is_out_of_frame() {
    let block = rect_block(2, 2);
    let target = rect(0, 1, block.width(), block.height());
    let above = [12, 20, 28, 36, 44, 52, 60, 68];
    let idif = one_sided_idif_edge(above[0], &above);
    let mut workspace = workspace(BitDepth::Eight, 8, 6, 0_u8);
    workspace
        .write_rect(PlaneId::Y, rect(0, 0, above.len(), 1), &above, above.len())
        .unwrap();

    let mut expected = vec![0_u8; block.width() * block.height()];
    predict_intra_directional_angle_rect_one_sided_idif_into(
        BitDepth::Eight,
        block,
        IntraDirectionalAngle::D67,
        IntraDirectionalAngleIdifEdges::above(&idif),
        &mut expected,
        block.width(),
    )
    .unwrap();

    workspace
        .predict_intra_directional_angle_rect(PlaneId::Y, 0, 1, block, IntraDirectionalAngle::D67)
        .unwrap();
    assert_eq!(
        workspace_rect_samples(&workspace, PlaneId::Y, target),
        expected
    );
}

#[test]
fn workspace_middle_directional_luma_d135_idif_matches_chroma_bilinear() {
    let block = rect_block(2, 2);
    let target = rect(1, 1, block.width(), block.height());
    let above_row = [40_u8, 48, 56, 64];
    let left_col = [50_u8, 58, 66, 74];
    let corner = 32_u8;

    let mut luma = workspace(BitDepth::Eight, 8, 8, 0_u8);
    predict_middle_d135_with_edges(&mut luma, PlaneId::Y, block, corner, &above_row, &left_col);

    let mut chroma = workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 8, 8, 0_u8);
    predict_middle_d135_with_edges(
        &mut chroma,
        PlaneId::U,
        block,
        corner,
        &above_row,
        &left_col,
    );

    assert_eq!(
        workspace_rect_samples(&luma, PlaneId::Y, target),
        workspace_rect_samples(&chroma, PlaneId::U, target),
    );
}

#[test]
fn workspace_directional_angle_rejects_missing_prepared_edges() {
    let block = rect_block(2, 2);
    let mut workspace = workspace_with_format(BitDepth::Eight, PixelFormat::Yuv444, 8, 8, 0_u8);

    for (x, y, angle, edge) in [
        (
            1,
            0,
            IntraDirectionalAngle::D45,
            IntraDirectionalAngleEdge::Above,
        ),
        (
            1,
            1,
            IntraDirectionalAngle::D67,
            IntraDirectionalAngleEdge::Above,
        ),
        (
            0,
            1,
            IntraDirectionalAngle::D203,
            IntraDirectionalAngleEdge::Left,
        ),
        (
            1,
            1,
            IntraDirectionalAngle::D203,
            IntraDirectionalAngleEdge::Left,
        ),
    ] {
        let result = workspace.predict_intra_directional_angle_rect(PlaneId::U, x, y, block, angle);
        assert_edge_unavailable(
            &result,
            PlaneId::U,
            angle.p_angle(),
            edge,
            rect(x, y, block.width(), block.height()),
        );
    }
    for (x, y, angle, edge) in [
        (
            0,
            1,
            IntraMiddleDirectionalAngle::D113,
            IntraDirectionalAngleEdge::Left,
        ),
        (
            1,
            0,
            IntraMiddleDirectionalAngle::D135,
            IntraDirectionalAngleEdge::Above,
        ),
    ] {
        let result =
            workspace.predict_intra_middle_directional_angle_rect(PlaneId::U, x, y, block, angle);
        assert_edge_unavailable(
            &result,
            PlaneId::U,
            angle.p_angle(),
            edge,
            rect(x, y, block.width(), block.height()),
        );
    }
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
