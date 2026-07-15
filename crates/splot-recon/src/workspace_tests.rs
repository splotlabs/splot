// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use splot_core::headers::sequence::SuperblockSize;

use crate::{
    BitDepth, DecodedFrameHash, DecodedFrameHashInput, IntraDcEdge, IntraDcEdges, OutputIndex,
    PixelFormat, ReferenceFrameStore, ReferenceSlot, Y4mFrameRate, Y4mWriter,
    apply_intra_ibp_dc_rect, predict_intra_dc_rect_into, reconstruct_add_residual,
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

fn pixel_format_info(pixel_format: PixelFormat, width: usize, height: usize) -> DecodedFrameInfo {
    info(
        BitDepth::Eight,
        pixel_format,
        size(width, height),
        rect(0, 0, width, height),
    )
}

fn band_rects(pixel_format: PixelFormat) -> Vec<(PlaneRect, Option<PlaneRect>, Option<PlaneRect>)> {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(pixel_format_info(pixel_format, 10, 130), 0).unwrap();
    workspace
        .sb_row_bands(SuperblockSize::Block64x64)
        .map(|band| {
            let y_rect = band.luma_rect();
            let (_, u, v) = band.into_planes();
            (
                y_rect,
                u.as_ref().map(CurrentFramePlaneRowBand::rect),
                v.as_ref().map(CurrentFramePlaneRowBand::rect),
            )
        })
        .collect()
}

#[test]
fn workspace_sb_row_bands_preserve_plane_geometry() {
    let y = [rect(0, 0, 10, 64), rect(0, 64, 10, 64), rect(0, 128, 10, 2)];
    let chroma_420 = [rect(0, 0, 5, 32), rect(0, 32, 5, 32), rect(0, 64, 5, 1)];
    let chroma_422 = [rect(0, 0, 5, 64), rect(0, 64, 5, 64), rect(0, 128, 5, 2)];

    assert_eq!(
        band_rects(PixelFormat::Monochrome),
        y.map(|rect| (rect, None, None))
    );
    assert_eq!(
        band_rects(PixelFormat::Yuv420),
        y.into_iter()
            .zip(chroma_420)
            .map(|(y, uv)| (y, Some(uv), Some(uv)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        band_rects(PixelFormat::Yuv422),
        y.into_iter()
            .zip(chroma_422)
            .map(|(y, uv)| (y, Some(uv), Some(uv)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        band_rects(PixelFormat::Yuv444),
        y.map(|rect| (rect, Some(rect), Some(rect)))
    );
}

#[test]
fn workspace_sb_row_bands_follow_every_superblock_size() {
    for (sb_size, expected) in [
        (SuperblockSize::Block64x64, vec![64, 64, 64, 64, 44]),
        (SuperblockSize::Block128x128, vec![128, 128, 44]),
        (SuperblockSize::Block256x256, vec![256, 44]),
    ] {
        let mut workspace =
            CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 300), 0).unwrap();
        let heights = workspace
            .sb_row_bands(sb_size)
            .map(|band| band.luma_rect().height())
            .collect::<Vec<_>>();

        assert_eq!(heights, expected);
    }
}

#[cfg(test)]
#[test]
fn workspace_sb_row_bands_are_disjoint_and_keep_global_origins() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(yuv420_info(8, 130), 0).unwrap();
    let bands = workspace
        .sb_row_bands(SuperblockSize::Block64x64)
        .collect::<Vec<_>>();

    std::thread::scope(|scope| {
        for band in bands {
            scope.spawn(move || {
                let (y, u, v) = band.into_planes();
                for mut plane in [Some(y), u, v].into_iter().flatten() {
                    let rect = plane.rect();
                    let stride = plane.stride_samples();
                    for (local_y, row) in plane.samples_mut().chunks_mut(stride).enumerate() {
                        row.fill(u8::try_from(rect.y() + local_y).unwrap());
                    }
                }
            });
        }
    });

    for y in 0..130 {
        assert_eq!(
            workspace.reconstructed_sample(PlaneId::Y, 3, y).unwrap(),
            u8::try_from(y).unwrap()
        );
    }
    for y in 0..65 {
        let expected = u8::try_from(y).unwrap();
        assert_eq!(
            workspace.reconstructed_sample(PlaneId::U, 2, y).unwrap(),
            expected
        );
        assert_eq!(
            workspace.reconstructed_sample(PlaneId::V, 2, y).unwrap(),
            expected
        );
    }
}

#[test]
fn intra_prediction_scratch_is_task_local_and_reusable() {
    let mut first = IntraPredictionScratch::<u8>::new();
    let mut second = IntraPredictionScratch::<u8>::new();
    let first_buffer = first
        .take_buffer(IntraPredictionScratchBuffer::Primary, PlaneId::Y, 64, 7)
        .unwrap();
    let first_pointer = first_buffer.as_ptr();
    first.recycle_buffer(IntraPredictionScratchBuffer::Primary, first_buffer);

    let second_buffer = second
        .take_buffer(IntraPredictionScratchBuffer::Primary, PlaneId::Y, 64, 9)
        .unwrap();
    assert_ne!(second_buffer.as_ptr(), first_pointer);
    assert_eq!(second_buffer, vec![9; 64]);

    let first_buffer = first
        .take_buffer(IntraPredictionScratchBuffer::Primary, PlaneId::Y, 16, 11)
        .unwrap();
    assert_eq!(first_buffer.as_ptr(), first_pointer);
    assert_eq!(first_buffer, vec![11; 16]);
}

fn assert_row_surface_matches_frame<T>(bit_depth: BitDepth, pixel_format: PixelFormat)
where
    T: ReconSample + core::fmt::Debug + Eq,
{
    let frame_info = info(bit_depth, pixel_format, size(9, 67), rect(0, 0, 9, 67));
    let mut frame_workspace = CurrentFrameWorkspace::<T>::new(frame_info, T::default()).unwrap();
    let mut row_workspace = CurrentFrameWorkspace::<T>::new(frame_info, T::default()).unwrap();
    let mut row_band = row_workspace
        .sb_row_bands(SuperblockSize::Block64x64)
        .nth(1)
        .unwrap();
    let planes = if pixel_format == PixelFormat::Monochrome {
        &[(PlaneId::Y, 0_u8)][..]
    } else {
        &[
            (PlaneId::Y, 0_u8),
            (PlaneId::U, pixel_format.subsampling_y()),
            (PlaneId::V, pixel_format.subsampling_y()),
        ][..]
    };
    {
        let mut frame_surface = CurrentFrameSurface::Frame(&mut frame_workspace);
        let mut row_surface = CurrentFrameSurface::Row(&mut row_band);

        assert_eq!(row_surface.info(), frame_surface.info());
        for &(plane, sub_y) in planes {
            let storage = row_surface.plane_storage_size(plane).unwrap();
            assert_eq!(frame_surface.plane_storage_size(plane).unwrap(), storage);
            let y = 64 >> sub_y;
            let target = rect(0, y, storage.width(), storage.height() - y);
            let base = if bit_depth == BitDepth::Ten { 600 } else { 40 };
            let samples = (0..target.width() * target.height())
                .map(|index| T::try_from_u16(base + u16::try_from(index % 31).unwrap()).unwrap())
                .collect::<Vec<_>>();

            frame_surface
                .write_rect(plane, target, &samples, target.width())
                .unwrap();
            row_surface
                .write_rect(plane, target, &samples, target.width())
                .unwrap();
            assert_eq!(
                row_surface
                    .rect_rows(plane, target)
                    .unwrap()
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>(),
                samples
            );
        }
        if pixel_format == PixelFormat::Monochrome {
            assert!(matches!(
                row_surface.plane_storage_size(PlaneId::U),
                Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U })
            ));
        }
    }
    for &(plane, _) in planes {
        assert_eq!(
            row_workspace.samples(plane).unwrap(),
            frame_workspace.samples(plane).unwrap()
        );
    }
}

#[test]
fn row_surface_matches_frame_for_every_pixel_format_and_sample_type() {
    for pixel_format in [
        PixelFormat::Monochrome,
        PixelFormat::Yuv420,
        PixelFormat::Yuv422,
        PixelFormat::Yuv444,
    ] {
        assert_row_surface_matches_frame::<u8>(BitDepth::Eight, pixel_format);
        assert_row_surface_matches_frame::<u16>(BitDepth::Ten, pixel_format);
    }
}

#[test]
fn row_surface_clips_frame_edge_and_rejects_cross_band_access_atomically() {
    let mut workspace = CurrentFrameWorkspace::<u8>::new(yuv420_info(9, 67), 3).unwrap();
    let mut row_band = workspace
        .sb_row_bands(SuperblockSize::Block64x64)
        .nth(1)
        .unwrap();
    {
        let mut surface = CurrentFrameSurface::Row(&mut row_band);

        let cross_band = rect(0, 63, 4, 4);
        assert!(matches!(
            surface.write_rect_block(PlaneId::Y, 0, 63, rect_block(2, 2), &[9; 16]),
            Err(ReconError::WorkspaceRowBandRectOutOfBounds {
                plane: PlaneId::Y,
                band,
                rect: rejected,
            }) if band == rect(0, 64, 9, 3) && rejected == cross_band
        ));
        assert!(matches!(
            surface.rect_rows(PlaneId::U, rect(0, 31, 2, 2)),
            Err(ReconError::WorkspaceRowBandRectOutOfBounds {
                plane: PlaneId::U,
                ..
            })
        ));

        surface
            .write_rect(PlaneId::Y, rect(7, 65, 4, 4), &[8; 16], 4)
            .unwrap();
    }

    assert_eq!(
        workspace.reconstructed_sample(PlaneId::Y, 0, 63).unwrap(),
        3
    );
    for y in 65..67 {
        assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 7, y).unwrap(), 8);
        assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 8, y).unwrap(), 8);
    }
    assert!(
        workspace
            .samples(PlaneId::U)
            .unwrap()
            .iter()
            .all(|&sample| sample == 3)
    );
}

fn assert_surface_add_residual_matches_reference<T>(bit_depth: BitDepth)
where
    T: ReconSample + core::fmt::Debug + Eq,
{
    let mut workspace =
        CurrentFrameWorkspace::<T>::new(monochrome_info(bit_depth, 4, 4), T::default()).unwrap();
    let prediction = core::array::from_fn::<_, 16, _>(|index| {
        T::try_from_u16([0, 1, 127, 254, 255][index % 5].min(bit_depth.max_sample())).unwrap()
    });
    let residual =
        core::array::from_fn::<_, 16, _>(|index| [i32::MIN, -2, 0, 2, i32::MAX][index % 5]);
    workspace
        .write_rect(PlaneId::Y, rect(0, 0, 4, 4), &prediction, 4)
        .unwrap();
    let mut expected = [T::default(); 16];
    reconstruct_add_residual(&prediction, &residual, bit_depth, &mut expected).unwrap();

    CurrentFrameSurface::Frame(&mut workspace)
        .add_residual_rect_block(PlaneId::Y, 0, 0, rect_block(2, 2), &residual)
        .unwrap();

    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), &expected);
    assert_eq!(expected[0].to_u16(), 0);
    assert_eq!(expected[4].to_u16(), bit_depth.max_sample());
}

#[test]
fn surface_add_residual_matches_buffer_reference_u8_and_u16() {
    assert_surface_add_residual_matches_reference::<u8>(BitDepth::Eight);
    assert_surface_add_residual_matches_reference::<u16>(BitDepth::Ten);
}

#[test]
fn row_surface_add_residual_matches_clipped_frame_stride_and_is_atomic() {
    let info = monochrome_info(BitDepth::Eight, 8, 67);
    let mut frame = CurrentFrameWorkspace::<u8>::new(info, 50).unwrap();
    let mut rows = CurrentFrameWorkspace::<u8>::new(info, 50).unwrap();
    let residual = core::array::from_fn::<_, 16, _>(|index| index as i32 + 1);
    CurrentFrameSurface::Frame(&mut frame)
        .add_residual_rect_block(PlaneId::Y, 6, 65, rect_block(2, 2), &residual)
        .unwrap();
    {
        let mut row_band = rows
            .sb_row_bands(SuperblockSize::Block64x64)
            .nth(1)
            .unwrap();
        CurrentFrameSurface::Row(&mut row_band)
            .add_residual_rect_block(PlaneId::Y, 6, 65, rect_block(2, 2), &residual)
            .unwrap();
    }

    assert_eq!(
        rows.samples(PlaneId::Y).unwrap(),
        frame.samples(PlaneId::Y).unwrap()
    );
    assert_eq!(frame.reconstructed_sample(PlaneId::Y, 6, 65).unwrap(), 51);
    assert_eq!(frame.reconstructed_sample(PlaneId::Y, 7, 65).unwrap(), 52);
    assert_eq!(frame.reconstructed_sample(PlaneId::Y, 6, 66).unwrap(), 55);
    assert_eq!(frame.reconstructed_sample(PlaneId::Y, 7, 66).unwrap(), 56);
    assert_eq!(frame.reconstructed_sample(PlaneId::Y, 5, 65).unwrap(), 50);

    {
        let mut row_band = rows
            .sb_row_bands(SuperblockSize::Block64x64)
            .nth(1)
            .unwrap();
        assert!(matches!(
            CurrentFrameSurface::Row(&mut row_band).add_residual_rect_block(
                PlaneId::Y,
                0,
                63,
                rect_block(2, 2),
                &residual,
            ),
            Err(ReconError::WorkspaceRowBandRectOutOfBounds { .. })
        ));
    }
    assert_eq!(
        rows.samples(PlaneId::Y).unwrap(),
        frame.samples(PlaneId::Y).unwrap()
    );
}

#[test]
fn surface_add_residual_rejects_inputs_atomically() {
    let mut workspace =
        CurrentFrameWorkspace::<u16>::new(monochrome_info(BitDepth::Eight, 4, 4), 20).unwrap();
    assert!(matches!(
        CurrentFrameSurface::Frame(&mut workspace).add_residual_rect_block(
            PlaneId::Y,
            0,
            0,
            rect_block(2, 2),
            &[1; 15],
        ),
        Err(ReconError::WorkspaceWriteLengthMismatch { .. })
    ));
    assert!(
        workspace
            .samples(PlaneId::Y)
            .unwrap()
            .iter()
            .all(|&sample| sample == 20)
    );

    workspace.as_frame_mut().y_mut().samples_mut()[15] = 300;
    assert!(matches!(
        CurrentFrameSurface::Frame(&mut workspace).add_residual_rect_block(
            PlaneId::Y,
            0,
            0,
            rect_block(2, 2),
            &[1; 16],
        ),
        Err(ReconError::ReconstructPredictionOutOfRange {
            sample_index: 15,
            value: 300,
            max: 255,
        })
    ));
    let mut expected = [20; 16];
    expected[15] = 300;
    assert_eq!(workspace.samples(PlaneId::Y).unwrap(), expected);
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
fn workspace_intra_edges_hold_maximum_u8_and_u16_blocks_inline() {
    let u8_left = [11_u8; 64];
    let u8_above = [22_u8; 64];
    let mut u8_workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 65, 65), 0).unwrap();
    u8_workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, 64), &u8_left, 1)
        .unwrap();
    u8_workspace
        .write_rect(PlaneId::Y, rect(1, 0, 64, 1), &u8_above, 64)
        .unwrap();
    let u8_edges = u8_workspace
        .intra_dc_edges_for_rect(PlaneId::Y, 1, 1, rect_block(6, 6))
        .unwrap();
    assert_eq!(u8_edges.left_samples(), Some(u8_left.as_slice()));
    assert_eq!(u8_edges.above_samples(), Some(u8_above.as_slice()));

    let u16_left = [300_u16; 64];
    let u16_above = [700_u16; 64];
    let mut u16_workspace =
        CurrentFrameWorkspace::<u16>::new(monochrome_info(BitDepth::Ten, 65, 65), 0).unwrap();
    u16_workspace
        .write_rect(PlaneId::Y, rect(0, 1, 1, 64), &u16_left, 1)
        .unwrap();
    u16_workspace
        .write_rect(PlaneId::Y, rect(1, 0, 64, 1), &u16_above, 64)
        .unwrap();
    let u16_edges = u16_workspace
        .intra_dc_edges_for_rect(PlaneId::Y, 1, 1, rect_block(6, 6))
        .unwrap()
        .clone();
    assert_eq!(u16_edges.left_samples(), Some(u16_left.as_slice()));
    assert_eq!(u16_edges.above_samples(), Some(u16_above.as_slice()));
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
fn workspace_reuses_two_intra_prediction_buffers() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();
    let primary = workspace
        .take_intra_prediction_buffer(
            IntraPredictionScratchBuffer::Primary,
            PlaneId::Y,
            64 * 64,
            7,
        )
        .unwrap();
    let primary_ptr = primary.as_ptr();
    let secondary = workspace
        .take_intra_prediction_buffer(
            IntraPredictionScratchBuffer::Secondary,
            PlaneId::Y,
            64 * 64,
            9,
        )
        .unwrap();
    let secondary_ptr = secondary.as_ptr();
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, primary);
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, secondary);

    let primary = workspace
        .take_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, PlaneId::Y, 16, 11)
        .unwrap();
    let secondary = workspace
        .take_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, PlaneId::Y, 16, 13)
        .unwrap();

    assert_eq!(primary.as_ptr(), primary_ptr);
    assert_eq!(secondary.as_ptr(), secondary_ptr);
    assert_eq!(primary, vec![11; 16]);
    assert_eq!(secondary, vec![13; 16]);
}

#[test]
fn workspace_rejects_oversized_intra_prediction_scratch() {
    let mut workspace =
        CurrentFrameWorkspace::<u8>::new(monochrome_info(BitDepth::Eight, 8, 8), 0).unwrap();

    assert!(matches!(
        workspace.take_intra_prediction_buffer(
            IntraPredictionScratchBuffer::Primary,
            PlaneId::Y,
            64 * 64 + 1,
            0,
        ),
        Err(ReconError::WorkspaceIntraPredictionScratchTooLarge {
            sample_count: 4097,
            max_sample_count: 4096,
        })
    ));
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
