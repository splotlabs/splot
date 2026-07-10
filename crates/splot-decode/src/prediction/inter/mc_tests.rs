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
fn dispatcher_zero_mv_copies_odd_reference_chroma_extents() {
    let reference = patterned_frame(9, 11);
    let mut workspace = workspace(9, 11);

    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::single(
            &reference,
            rect(0, 0, 9, 11),
            Mv::ZERO,
            InterpolationFilter::EightTap,
        ),
        ByteOffset::new(0),
    )
    .expect("odd single-reference dispatcher");

    let decoded = workspace.freeze().expect("freeze odd workspace");
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        assert_eq!(
            visible_samples(&decoded, plane),
            visible_samples(&reference, plane)
        );
    }
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
fn dispatcher_sub8x8_chroma_uses_only_the_first_reference() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let mut workspace = workspace(8, 8);

    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::compound_average(
            &reference0,
            &reference1,
            rect(0, 0, 8, 8),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTap,
            CompoundBlend::default(),
        )
        .with_sub8x8_chroma(true),
        ByteOffset::new(0),
    )
    .expect("sub8x8 chroma dispatcher");

    let decoded = workspace.freeze().expect("freeze sub8x8 workspace");
    assert_eq!(visible_samples(&decoded, PlaneId::Y), vec![60; 64]);
    assert_eq!(visible_samples(&decoded, PlaneId::U), vec![90; 16]);
    assert_eq!(visible_samples(&decoded, PlaneId::V), vec![120; 16]);
}

#[test]
fn dispatcher_sub8x8_chroma_drops_warp() {
    let reference = patterned_frame(16, 16);
    let mut warped = workspace(16, 16);
    let mut translational = workspace(16, 16);
    let rect = rect(0, 0, 8, 8);
    let mv = Mv::ZERO;

    motion_compensate_inter_block_into(
        &mut warped,
        InterBlockParams::single_warp(
            &reference,
            rect,
            mv,
            InterpolationFilter::EightTap,
            [1 << 16, 0, 1 << 16, 0, 0, 1 << 16],
        )
        .with_sub8x8_chroma(true),
        ByteOffset::new(0),
    )
    .expect("sub8x8 chroma warp dispatcher");
    motion_compensate_inter_block_into(
        &mut translational,
        InterBlockParams::single(&reference, rect, mv, InterpolationFilter::EightTap),
        ByteOffset::new(0),
    )
    .expect("translational dispatcher");

    let warped = warped.freeze().expect("freeze warped workspace");
    let translational = translational
        .freeze()
        .expect("freeze translational workspace");
    assert_ne!(
        visible_samples(&warped, PlaneId::Y),
        visible_samples(&translational, PlaneId::Y)
    );
    for plane in [PlaneId::U, PlaneId::V] {
        assert_eq!(
            visible_samples(&warped, plane),
            visible_samples(&translational, plane)
        );
    }
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
fn dispatcher_rebuilds_optflow_compound_planes_from_refined_grid() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let mut workspace = workspace(8, 8);
    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::compound_average(
            &reference0,
            &reference1,
            rect(0, 0, 8, 8),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTap,
            CompoundBlend::default(),
        )
        .with_optflow_distances(Some([1, -1])),
        ByteOffset::new(0),
    )
    .expect("optical-flow compound dispatcher");

    let decoded = workspace.freeze().expect("freeze optical-flow workspace");
    assert_eq!(visible_samples(&decoded, PlaneId::Y), vec![60; 64]);
    assert_eq!(visible_samples(&decoded, PlaneId::U), vec![100; 16]);
    assert_eq!(visible_samples(&decoded, PlaneId::V), vec![130; 16]);
}

#[test]
fn optflow_derivation_is_independent_of_the_final_interpolation_filter() {
    let width = 32;
    let height = 32;
    let pattern = |x: usize, y: usize| ((x * 19 + y * 37 + x * y * 3) % 251) as u8;
    let reference0_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern(x, y)))
        .collect();
    let reference1_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern((x + 2).min(width - 1), y)))
        .collect();
    let chroma = vec![128; width.div_ceil(2) * height.div_ceil(2)];
    let reference0 = frame(width, height, reference0_y, chroma.clone(), chroma.clone());
    let reference1 = frame(width, height, reference1_y, chroma.clone(), chroma);
    let derive = |interp| {
        let mut workspace = workspace(width, height);
        motion_compensate_inter_block_with_motion_grid_into(
            &mut workspace,
            InterBlockParams::compound_average(
                &reference0,
                &reference1,
                rect(8, 8, 8, 8),
                Mv { row: 3, col: 5 },
                Mv { row: -5, col: -3 },
                interp,
                CompoundBlend::default(),
            )
            .with_optflow_distances(Some([1, -1])),
            None,
            ByteOffset::new(0),
        )
        .expect("optical-flow dispatcher")
        .expect("optical-flow motion grid")
        .at_luma_offset(0, 0)
        .expect("optical-flow motion cell")
    };

    assert_eq!(
        derive(InterpolationFilter::EightTapSharp),
        derive(InterpolationFilter::Bilinear)
    );
}

#[test]
fn dispatcher_returns_tip_output_optflow_mvs_for_storage() {
    let reference0 = flat_frame(8, 8, 40, 90, 120);
    let reference1 = flat_frame(8, 8, 80, 110, 140);
    let mut workspace = workspace(8, 8);
    let mvs = motion_compensate_inter_block_with_optflow_mvs_into(
        &mut workspace,
        InterBlockParams::compound_average(
            &reference0,
            &reference1,
            rect(0, 0, 8, 8),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTap,
            CompoundBlend::default(),
        )
        .with_optflow_distances(Some([1, -1])),
        8,
        ByteOffset::new(0),
    )
    .expect("TIP output optical-flow dispatcher");

    assert_eq!(mvs, Some([Mv::ZERO; 2]));
}

#[test]
fn dispatcher_returns_default_refinemv_motion_grid() {
    let width = 32;
    let height = 32;
    let pattern = |x: usize, y: usize| ((x * 19 + y * 37 + x * y * 3) % 251) as u8;
    let reference0_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern(x, y)))
        .collect();
    let reference1_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern((x + 2).min(width - 1), y)))
        .collect();
    let chroma = vec![128; width.div_ceil(2) * height.div_ceil(2)];
    let reference0 = frame(width, height, reference0_y, chroma.clone(), chroma.clone());
    let reference1 = frame(width, height, reference1_y, chroma.clone(), chroma);
    let mut workspace = workspace(width, height);

    let grid = motion_compensate_inter_block_with_motion_grid_into(
        &mut workspace,
        InterBlockParams::compound_average(
            &reference0,
            &reference1,
            rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true),
        None,
        ByteOffset::new(0),
    )
    .expect("default refine-MV dispatcher")
    .expect("refine-MV motion grid");

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored refine-MVs"),
        [Mv { row: 0, col: 8 }, Mv { row: 0, col: -8 }]
    );
}

#[test]
fn dispatcher_skips_refinemv_search_without_disabling_refinemv() {
    let width = 32;
    let height = 32;
    let pattern = |x: usize, y: usize| ((x * 19 + y * 37 + x * y * 3) % 251) as u8;
    let reference0_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern(x, y)))
        .collect();
    let reference1_y: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| pattern((x + 2).min(width - 1), y)))
        .collect();
    let chroma = vec![128; width.div_ceil(2) * height.div_ceil(2)];
    let reference0 = frame(width, height, reference0_y, chroma.clone(), chroma.clone());
    let reference1 = frame(width, height, reference1_y, chroma.clone(), chroma);
    let mut workspace = workspace(width, height);

    let grid = motion_compensate_inter_block_with_motion_grid_into(
        &mut workspace,
        InterBlockParams::compound_average(
            &reference0,
            &reference1,
            rect(8, 8, 16, 16),
            Mv::ZERO,
            Mv::ZERO,
            InterpolationFilter::EightTapSharp,
            CompoundBlend::default(),
        )
        .with_refinemv(true)
        .with_refinemv_search(false),
        None,
        ByteOffset::new(0),
    )
    .expect("refine-MV dispatcher without search")
    .expect("refine-MV motion grid");

    assert_eq!(
        grid.temporal_mvs_at_luma_offset(0, 0)
            .expect("stored refine-MVs"),
        [Mv::ZERO, Mv::ZERO]
    );
}

#[test]
fn chroma_geometry_rounds_odd_luma_extents_up() {
    assert_eq!(
        rect(0, 0, 17, 19).plane_rect(PlaneId::U, 1, 1),
        (0, 0, 9, 10)
    );
    assert_eq!(rect(1, 1, 8, 8).plane_rect(PlaneId::V, 1, 1), (0, 0, 5, 5));
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

#[test]
fn warp_skips_prediction_units_beyond_the_current_frame() {
    let reference = patterned_frame(16, 8);
    let mut workspace = workspace(16, 8);

    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::single_warp(
            &reference,
            rect(8, 0, 16, 8),
            Mv::ZERO,
            InterpolationFilter::EightTap,
            crate::prediction::inter::find_mv_stack::DEFAULT_WARP_PARAMS,
        )
        .with_chroma(false),
        ByteOffset::new(0),
    )
    .expect("edge-clipped warp dispatcher");

    let decoded = workspace
        .freeze()
        .expect("freeze edge-clipped warp workspace");
    let y = visible_samples(&decoded, PlaneId::Y);
    let reference_y = visible_samples(&reference, PlaneId::Y);
    for row in 0..8 {
        assert_eq!(&y[row * 16..row * 16 + 8], &[0; 8]);
        assert_eq!(
            &y[row * 16 + 8..row * 16 + 16],
            &reference_y[row * 16 + 8..row * 16 + 16]
        );
    }
}

#[test]
fn extended_warp_skips_prediction_units_beyond_the_current_frame() {
    let reference = patterned_frame(24, 8);
    let mut workspace = workspace(16, 8);

    motion_compensate_inter_block_into(
        &mut workspace,
        InterBlockParams::single_warp(
            &reference,
            rect(12, 0, 8, 4),
            Mv::ZERO,
            InterpolationFilter::EightTap,
            crate::prediction::inter::find_mv_stack::DEFAULT_WARP_PARAMS,
        )
        .with_chroma(false),
        ByteOffset::new(0),
    )
    .expect("edge-clipped extended-warp dispatcher");

    let decoded = workspace
        .freeze()
        .expect("freeze edge-clipped extended-warp workspace");
    let y = visible_samples(&decoded, PlaneId::Y);
    let reference_y = visible_samples(&reference, PlaneId::Y);
    for row in 0..4 {
        assert_eq!(&y[row * 16..row * 16 + 12], &[0; 12]);
        assert_eq!(
            &y[row * 16 + 12..row * 16 + 16],
            &reference_y[row * 24 + 12..row * 24 + 16]
        );
    }
    assert!(y[4 * 16..].iter().all(|&sample| sample == 0));
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
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let u: Vec<u8> = (0..chroma_width * chroma_height)
        .map(|sample| 100 + u8::try_from(sample).expect("u sample"))
        .collect();
    let v: Vec<u8> = (0..chroma_width * chroma_height)
        .map(|sample| 150 + u8::try_from(sample).expect("v sample"))
        .collect();
    frame(width, height, y, u, v)
}

fn flat_frame(width: usize, height: usize, y: u8, u: u8, v: u8) -> DecodedFrame<u8> {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
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
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
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
