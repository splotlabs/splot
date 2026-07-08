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
        plane_tx_type: 0,
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
fn cfl_reconstruction_rejects_non_420_chroma_geometry() {
    let mut frame = workspace(PixelFormat::Yuv444);

    let error = reconstruct_general_intra_chroma_cfl_block_into(
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
    .unwrap_err();

    assert!(matches!(
        error,
        GeneralIntraResidualError::UnsupportedTransformToolResidual {
            reason: "general_intra_cfl_non_420_chroma"
        }
    ));
}

#[test]
fn mhccp_reference_extensions_use_luma_pixel_threshold_for_420() {
    let frame = workspace_sized(PixelFormat::Yuv420, 128, 128);

    let bottom_extended = mhccp_references(&frame, PlaneId::U, 32, 0, 16, 4, 0, 16, 0, 1).unwrap();
    assert_eq!(
        (
            bottom_extended.width,
            bottom_extended.height,
            bottom_extended.above,
            bottom_extended.left
        ),
        (18, 8, 0, 2)
    );

    let right_extended = mhccp_references(&frame, PlaneId::U, 0, 4, 4, 16, 0, 16, 1, 0).unwrap();
    assert_eq!(
        (
            right_extended.width,
            right_extended.height,
            right_extended.above,
            right_extended.left
        ),
        (8, 18, 2, 0)
    );

    let threshold_not_extended =
        mhccp_references(&frame, PlaneId::U, 8, 0, 8, 2, 0, 16, 0, 1).unwrap();
    assert_eq!(threshold_not_extended.height, 2);
}
