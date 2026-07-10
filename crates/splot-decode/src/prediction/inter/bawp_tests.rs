// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, FramePlanes,
    InterpolationFilter, OutputIndex, PixelFormat, Plane, PlaneId, PlaneRect, PlaneSize,
};

use super::{apply_bawp, apply_intrabc_morph_pred, bawp_template_counts};
use crate::prediction::inter::{BawpSyntax, InterBlock, Mv, PlacedInterBlock, mc::CompoundBlend};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn template_counts_follow_the_clamped_size_table() {
    for (case, expected) in [
        ((16, 16, true, true, true), (16, 16, 16, 16)),
        ((12, 16, true, true, true), (8, 16, 8, 8)),
        ((16, 12, true, true, true), (16, 8, 8, 8)),
        ((4, 4, true, true, true), (4, 4, 4, 4)),
        ((16, 4, true, true, true), (16, 4, 16, 0)),
        ((4, 16, true, true, true), (4, 16, 0, 16)),
        ((32, 8, true, true, false), (16, 8, 16, 0)),
        ((8, 32, true, false, true), (8, 16, 0, 16)),
        ((32, 32, false, true, true), (8, 8, 8, 8)),
        ((12, 12, false, true, true), (8, 8, 8, 8)),
        ((64, 64, true, false, false), (16, 16, 0, 0)),
    ] {
        let (bw, bh, luma, up, left) = case;
        assert_eq!(
            bawp_template_counts(bw, bh, luma, up, left),
            expected,
            "bw={bw} bh={bh} luma={luma} up={up} left={left}"
        );
    }
}

#[test]
fn intrabc_morph_pred_skips_unavailable_top_left_template() -> TestResult {
    let mut workspace = workspace(8, 8, 77)?;
    let target = PlaneRect::new(0, 0, 4, 4)?;

    apply_intrabc_morph_pred(
        &mut workspace,
        target,
        Mv { row: 0, col: 0 },
        splot_core::span::ByteOffset::new(0),
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 0, 0)?, 77);
    Ok(())
}

#[test]
fn inter_bawp_skips_unavailable_top_left_reference_template() -> TestResult {
    let mut workspace = workspace(8, 8, 91)?;
    let reference = frame(8, 8, vec![12; 64], vec![34; 16], vec![56; 16])?;
    let placed = placed_luma_block(0, 0, 4, 4);

    apply_bawp(
        &mut workspace,
        &reference,
        &placed,
        BawpSyntax {
            enabled: true,
            ..BawpSyntax::default()
        },
        Mv { row: 0, col: 0 },
        splot_core::span::ByteOffset::new(0),
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 0, 0)?, 91);
    Ok(())
}

#[test]
fn intrabc_morph_pred_applies_large_luma_block() -> TestResult {
    let mut workspace = workspace(160, 160, 40)?;
    for col in 24..40 {
        workspace.set_reconstructed_sample(PlaneId::Y, col, 23, 20)?;
    }
    for row in 24..40 {
        workspace.set_reconstructed_sample(PlaneId::Y, 23, row, 20)?;
    }

    apply_intrabc_morph_pred(
        &mut workspace,
        PlaneRect::new(16, 16, 128, 128)?,
        Mv { row: 64, col: 64 },
        splot_core::span::ByteOffset::new(0),
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 16, 16)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 143, 143)?, 60);
    Ok(())
}

#[test]
fn inter_bawp_applies_large_luma_block() -> TestResult {
    let mut workspace = workspace(160, 160, 40)?;
    let reference = frame(
        160,
        160,
        vec![20; 160 * 160],
        vec![20; 80 * 80],
        vec![20; 80 * 80],
    )?;

    apply_bawp(
        &mut workspace,
        &reference,
        &placed_luma_block(16, 16, 128, 128),
        BawpSyntax {
            enabled: true,
            ..BawpSyntax::default()
        },
        Mv { row: 0, col: 0 },
        splot_core::span::ByteOffset::new(0),
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 16, 16)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 143, 143)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 15, 15)?, 40);
    Ok(())
}

fn workspace(width: usize, height: usize, fill: u8) -> TestResult<CurrentFrameWorkspace<u8>> {
    let luma_size = PlaneSize::new(width, height)?;
    let visible = PlaneRect::new(0, 0, width, height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        visible,
    )?;
    Ok(CurrentFrameWorkspace::new(info, fill)?)
}

fn frame(
    width: usize,
    height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
) -> TestResult<DecodedFrame<u8>> {
    let luma_size = PlaneSize::new(width, height)?;
    let luma_rect = PlaneRect::new(0, 0, width, height)?;
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    let chroma_size = PlaneSize::new(chroma_width, chroma_height)?;
    let chroma_rect = PlaneRect::new(0, 0, chroma_width, chroma_height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        luma_rect,
    )?;

    Ok(DecodedFrame::try_new(
        info,
        FramePlanes::new(
            plane(luma_size, width, luma_rect, y)?,
            Some(plane(chroma_size, chroma_width, chroma_rect, u)?),
            Some(plane(chroma_size, chroma_width, chroma_rect, v)?),
        ),
    )?)
}

fn plane(
    size: PlaneSize,
    stride: usize,
    visible: PlaneRect,
    samples: Vec<u8>,
) -> TestResult<Plane<u8>> {
    Ok(Plane::from_vec(size, stride, visible, samples)?)
}

fn placed_luma_block(x: usize, y: usize, width: usize, height: usize) -> PlacedInterBlock {
    PlacedInterBlock {
        luma_x: x,
        luma_y: y,
        luma_w: width,
        luma_h: height,
        chroma_luma_x: x,
        chroma_luma_y: y,
        chroma_luma_w: width,
        chroma_luma_h: height,
        predict_chroma: false,
        chroma_first_reference_only: false,
        interintra_chroma: false,
        block: InterBlock {
            ref_frame0: 0,
            ref_frame1: None,
            mv: Mv { row: 0, col: 0 },
            mv1: Mv { row: 0, col: 0 },
            interp: InterpolationFilter::EightTap,
            warp_params: [None, None],
            bawp: BawpSyntax::default(),
            interintra: None,
            compound_blend: CompoundBlend::default(),
            optflow_distances: None,
            residual: None,
        },
    }
}
