// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_recon::{DecodedFrameInfo, OutputIndex, PlaneRect, PlaneSize};

use super::*;

fn workspace_sized(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
) -> CurrentFrameWorkspace<u8> {
    let luma = PlaneSize::new(width, height).unwrap();
    let visible = PlaneRect::new(0, 0, width, height).unwrap();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        pixel_format,
        luma,
        visible,
    )
    .unwrap();
    CurrentFrameWorkspace::new(info, 0).unwrap()
}

fn workspace(pixel_format: PixelFormat) -> CurrentFrameWorkspace<u8> {
    workspace_sized(pixel_format, 16, 16)
}

fn zero_block() -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: 0,
        use_tcq: false,
        lossless: false,
    }
}

fn derived_alpha() -> CflParams {
    CflParams {
        index: CflIndex::DerivedAlpha,
        alpha_u: 0,
        alpha_v: 0,
        mh_dir: None,
    }
}

#[test]
fn cfl_420_filter1_simd_matches_scalar_at_block_and_frame_edges() {
    let plane_width = 130;
    let plane_height = 9;
    let luma: Vec<u16> = (0..plane_width * plane_height)
        .map(|index| ((index * 17 + 11) & 1023) as u16)
        .collect();
    let average_q3 = 37;
    for (x, y, width, height) in [(0, 0, 32, 4), (1, 1, 64, 3), (31, 2, 32, 3), (50, 4, 16, 2)] {
        let mut actual = Vec::new();
        assert!(fill_cfl_luma_ac_420_filter1_u16(
            &luma,
            plane_width,
            plane_width,
            plane_height,
            x,
            y,
            width,
            height,
            average_q3,
            &mut actual,
        ));
        let mut expected = Vec::with_capacity(width * height);
        for row in 0..height {
            let luma_y = (2 * (y + row)).min(plane_height - 1);
            let next_y = (luma_y + 1).min(plane_height - 1);
            for col in 0..width {
                let luma_x = 2 * (x + col);
                let center = luma_x.min(plane_width - 1);
                let left = if col == 0 || luma_x.is_multiple_of(64) {
                    center
                } else {
                    luma_x.saturating_sub(1).min(plane_width - 1)
                };
                let right = (luma_x + 1).min(plane_width - 1);
                expected.push(
                    i32::from(luma[luma_y * plane_width + left])
                        + 2 * i32::from(luma[luma_y * plane_width + center])
                        + i32::from(luma[luma_y * plane_width + right])
                        + i32::from(luma[next_y * plane_width + left])
                        + 2 * i32::from(luma[next_y * plane_width + center])
                        + i32::from(luma[next_y * plane_width + right])
                        - average_q3,
                );
            }
        }
        assert_eq!(
            actual, expected,
            "x={x} y={y} width={width} height={height}"
        );
    }
}

#[test]
fn cfl_reconstruction_supports_non_420_chroma_geometry() {
    let mut scratch = crate::pipeline::general_intra::GeneralIntraReconScratch::default();
    let mut retained = None;
    for pixel_format in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let mut frame = workspace(pixel_format);

        reconstruct_general_intra_chroma_cfl_block_into(
            &mut scratch,
            &mut frame,
            &zero_block(),
            PlaneId::U,
            0,
            0,
            2,
            2,
            0,
            derived_alpha(),
            0,
            16,
            0,
            0,
            BitDepth::Eight,
        )
        .unwrap();

        assert_eq!(frame.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 128);
        let storage = (
            scratch.cfl_luma_ac.as_ptr(),
            scratch.cfl_prediction.as_ptr(),
        );
        if let Some(previous) = retained {
            assert_eq!(storage, previous);
        }
        retained = Some(storage);
    }
}

#[test]
fn cfl_luma_sample_uses_422_and_444_filters() {
    let luma = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    for (pixel_format, expected) in [(PixelFormat::Yuv422, 52), (PixelFormat::Yuv444, 40)] {
        let mut frame = workspace(pixel_format);
        frame
            .write_rect_block(
                PlaneId::Y,
                0,
                0,
                IntraRectBlockSize::new(2, 2).unwrap(),
                &luma,
            )
            .unwrap();

        assert_eq!(
            cfl_luma_q3(&frame, 1, 1, false, false, 0).unwrap(),
            expected
        );
    }
}

#[test]
fn mhccp_reference_extensions_use_luma_pixel_threshold_for_420() {
    let frame = workspace_sized(PixelFormat::Yuv420, 128, 128);

    let bottom_extended = mhccp_references(
        &frame,
        PlaneId::U,
        32,
        0,
        16,
        4,
        0,
        16,
        0,
        1,
        &mut Default::default(),
    )
    .unwrap();
    assert_eq!(
        (
            bottom_extended.width,
            bottom_extended.height,
            bottom_extended.above,
            bottom_extended.left
        ),
        (18, 8, 0, 2)
    );

    let right_extended = mhccp_references(
        &frame,
        PlaneId::U,
        0,
        4,
        4,
        16,
        0,
        16,
        1,
        0,
        &mut Default::default(),
    )
    .unwrap();
    assert_eq!(
        (
            right_extended.width,
            right_extended.height,
            right_extended.above,
            right_extended.left
        ),
        (8, 18, 2, 0)
    );

    let threshold_not_extended = mhccp_references(
        &frame,
        PlaneId::U,
        8,
        0,
        8,
        2,
        0,
        16,
        0,
        1,
        &mut Default::default(),
    )
    .unwrap();
    assert_eq!(threshold_not_extended.height, 2);
}

#[test]
fn mhccp_reference_extensions_use_non_420_subsampling() {
    for pixel_format in [PixelFormat::Yuv422, PixelFormat::Yuv444] {
        let frame = workspace_sized(pixel_format, 128, 128);
        let refs = mhccp_references(
            &frame,
            PlaneId::U,
            32,
            0,
            16,
            8,
            0,
            16,
            0,
            1,
            &mut Default::default(),
        )
        .unwrap();

        assert_eq!(
            (refs.width, refs.height, refs.above, refs.left),
            (18, 12, 0, 2)
        );
    }
}
