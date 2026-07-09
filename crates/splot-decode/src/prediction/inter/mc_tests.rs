// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::*;
use splot_recon::{
    DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane, PlaneSize,
    wedge_mask_plane_sample,
};

#[test]
fn dispatcher_zero_mv_copies_single_reference_planes() {
    let reference = patterned_frame(8, 8);
    let mut workspace = workspace(8, 8);

    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::single(
            &reference,
            rect(0, 0, 8, 8),
            Mv { row: 0, col: 0 },
            InterpolationFilter::EightTap,
        ),
        ByteOffset::new(0),
    )
    .expect("single-reference dispatcher");

    let decoded = workspace.freeze().expect("freeze dispatched workspace");
    assert_eq!(
        visible_samples(&decoded, PlaneId::Y),
        visible_samples(&reference, PlaneId::Y)
    );
    assert_eq!(
        visible_samples(&decoded, PlaneId::U),
        visible_samples(&reference, PlaneId::U)
    );
    assert_eq!(
        visible_samples(&decoded, PlaneId::V),
        visible_samples(&reference, PlaneId::V)
    );
}

#[test]
fn dispatcher_blends_compound_average_planes() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let samples = dispatch_compound_samples(&reference0, &reference1, CompoundBlend::default());
    assert_eq!(samples.0, vec![60; 64]);
    assert_eq!(samples.1, vec![100; 16]);
    assert_eq!(samples.2, vec![130; 16]);
}

#[test]
fn dispatcher_blends_compound_average_with_cwp_weight() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let samples = dispatch_compound_samples(
        &reference0,
        &reference1,
        CompoundBlend::default().average_with_cwp_weight(12),
    );
    assert_eq!(samples.0, vec![50; 64]);
    assert_eq!(samples.1, vec![95; 16]);
    assert_eq!(samples.2, vec![125; 16]);
}

#[test]
fn dispatcher_subsamples_luma_diff_weighted_mask_for_chroma() {
    let reference0 = flat_frame(8, 8, 100, 0, 0);
    let reference1 = flat_frame(8, 8, 100, 200, 200);
    let samples = dispatch_compound_samples(
        &reference0,
        &reference1,
        CompoundBlend::DiffWeighted { inverse: false },
    );
    assert_eq!(samples.0, vec![100; 64]);
    assert_eq!(samples.1, vec![81; 16]);
    assert_eq!(samples.2, vec![81; 16]);
}

#[test]
fn dispatcher_blends_compound_wedge_planes() {
    let reference0 = flat_frame(8, 8, 64, 64, 64);
    let reference1 = flat_frame(8, 8, 0, 0, 0);

    let samples = dispatch_compound_samples(
        &reference0,
        &reference1,
        CompoundBlend::Wedge {
            index: 0,
            sign: false,
        },
    );
    assert_eq!(
        u16::from(samples.0[0]),
        wedge_mask_plane_sample(8, 8, 0, false, 0, 0, 0, 0).expect("luma wedge mask")
    );
    assert_eq!(
        u16::from(samples.1[0]),
        wedge_mask_plane_sample(8, 8, 0, false, 1, 1, 0, 0).expect("chroma wedge mask")
    );

    let inverse = dispatch_compound_samples(
        &reference0,
        &reference1,
        CompoundBlend::Wedge {
            index: 0,
            sign: true,
        },
    );
    assert_eq!(
        u16::from(inverse.0[0]),
        wedge_mask_plane_sample(8, 8, 0, true, 0, 0, 0, 0).expect("inverse wedge mask")
    );
}

#[test]
fn dispatcher_uses_implicit_mask_for_offscreen_compound_refs() {
    let reference = patterned_frame(32, 8);
    let mut workspace = workspace(32, 8);

    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::compound_average(
            &reference,
            &reference,
            rect(0, 0, 32, 4),
            Mv { row: 0, col: 32 },
            Mv { row: 0, col: -128 },
            InterpolationFilter::EightTap,
            CompoundBlend::average_with_implicit_mask(true),
        ),
        ByteOffset::new(0),
    )
    .expect("implicit-mask compound dispatcher");

    let decoded = workspace.freeze().expect("freeze implicit mask workspace");
    let y = visible_samples(&decoded, PlaneId::Y);
    assert_eq!(y[0], 4);
    assert_eq!(y[20], 14);
    assert_eq!(y[28], 12);
}

fn workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u8> {
    let luma_size = PlaneSize::new(width, height).expect("luma size");
    let visible = PlaneRect::new(0, 0, width, height).expect("visible rect");
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        visible,
    )
    .expect("frame info");
    CurrentFrameWorkspace::new(info, 0).expect("workspace")
}

fn dispatch_compound_samples(
    reference0: &DecodedFrame<u8>,
    reference1: &DecodedFrame<u8>,
    blend: CompoundBlend,
) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut workspace = workspace(8, 8);
    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::compound_average(
            reference0,
            reference1,
            rect(0, 0, 8, 8),
            Mv { row: 0, col: 0 },
            Mv { row: 0, col: 0 },
            InterpolationFilter::EightTap,
            blend,
        ),
        ByteOffset::new(0),
    )
    .expect("compound dispatcher");

    let decoded = workspace.freeze().expect("freeze compound workspace");
    (
        visible_samples(&decoded, PlaneId::Y),
        visible_samples(&decoded, PlaneId::U),
        visible_samples(&decoded, PlaneId::V),
    )
}

fn patterned_frame(width: usize, height: usize) -> DecodedFrame<u8> {
    let y: Vec<u8> = (0..width * height)
        .map(|sample| u8::try_from(sample).expect("luma sample"))
        .collect();
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    let u: Vec<u8> = (0..chroma_width * chroma_height)
        .map(|sample| 100 + u8::try_from(sample).expect("u sample"))
        .collect();
    let v: Vec<u8> = (0..chroma_width * chroma_height)
        .map(|sample| 150 + u8::try_from(sample).expect("v sample"))
        .collect();
    frame(width, height, y, u, v)
}

fn flat_frame(width: usize, height: usize, y: u8, u: u8, v: u8) -> DecodedFrame<u8> {
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    frame(
        width,
        height,
        vec![y; width * height],
        vec![u; chroma_width * chroma_height],
        vec![v; chroma_width * chroma_height],
    )
}

fn frame(width: usize, height: usize, y: Vec<u8>, u: Vec<u8>, v: Vec<u8>) -> DecodedFrame<u8> {
    let luma_size = PlaneSize::new(width, height).expect("luma size");
    let luma_rect = PlaneRect::new(0, 0, width, height).expect("luma rect");
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    let chroma_size = PlaneSize::new(chroma_width, chroma_height).expect("chroma size");
    let chroma_rect = PlaneRect::new(0, 0, chroma_width, chroma_height).expect("chroma rect");
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        luma_size,
        luma_rect,
    )
    .expect("frame info");

    DecodedFrame::try_new(
        info,
        FramePlanes::new(
            plane(luma_size, width, luma_rect, y),
            Some(plane(chroma_size, chroma_width, chroma_rect, u)),
            Some(plane(chroma_size, chroma_width, chroma_rect, v)),
        ),
    )
    .expect("decoded frame")
}

fn plane(size: PlaneSize, stride: usize, visible: PlaneRect, samples: Vec<u8>) -> Plane<u8> {
    Plane::from_vec(size, stride, visible, samples).expect("plane")
}

fn visible_samples(frame: &DecodedFrame<u8>, plane: PlaneId) -> Vec<u8> {
    frame
        .plane(plane)
        .expect("frame plane")
        .visible_rows()
        .flat_map(|row| row.iter().copied())
        .collect()
}

const fn rect(luma_x: usize, luma_y: usize, luma_w: usize, luma_h: usize) -> McBlockRect {
    McBlockRect {
        luma_x,
        luma_y,
        luma_w,
        luma_h,
        chroma_luma_x: luma_x,
        chroma_luma_y: luma_y,
        chroma_luma_w: luma_w,
        chroma_luma_h: luma_h,
    }
}
