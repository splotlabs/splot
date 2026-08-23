// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::test_support::yuv420_workspace;
use splot_recon::{DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

fn bo_plane(offset_idx: u8) -> CcsoPlaneParams {
    CcsoPlaneParams {
        reuse_ccso: false,
        sb_reuse_ccso: false,
        ccso_ref_idx: None,
        ccso_planes: true,
        ccso_bo_only: Some(true),
        ccso_scale_idx: Some(0),
        ccso_quant_idx: Some(0),
        ccso_ext_filter: Some(0),
        ccso_edge_clf: Some(false),
        ccso_max_band_log2: Some(1),
        ccso_offset_idx: vec![offset_idx; 2],
    }
}

fn full_luma_grid(width: usize, height: usize) -> CcsoUnitGrid {
    let grid_cols = width.div_ceil(4);
    let grid_rows = height.div_ceil(4);
    let cells = grid_rows * grid_cols;
    CcsoUnitGrid::new(
        true,
        0,
        [true, false, false],
        [vec![1; cells], vec![0; cells], vec![0; cells]],
        grid_rows,
        grid_cols,
    )
    .unwrap()
}

fn edge_plane(
    ext_filter: u8,
    edge_clf: bool,
    max_band_log2: u8,
    offset_count: usize,
) -> CcsoPlaneParams {
    CcsoPlaneParams {
        reuse_ccso: false,
        sb_reuse_ccso: false,
        ccso_ref_idx: None,
        ccso_planes: true,
        ccso_bo_only: Some(false),
        ccso_scale_idx: Some(2),
        ccso_quant_idx: Some(1),
        ccso_ext_filter: Some(ext_filter),
        ccso_edge_clf: Some(edge_clf),
        ccso_max_band_log2: Some(max_band_log2),
        ccso_offset_idx: (0..offset_count).map(|i| (i % 8) as u8).collect(),
    }
}

fn full_grid(luma_width: usize, luma_height: usize) -> CcsoUnitGrid {
    let grid_cols = luma_width.div_ceil(4);
    let grid_rows = luma_height.div_ceil(4);
    let cells = grid_rows * grid_cols;
    CcsoUnitGrid::new(
        true,
        0,
        [true; 3],
        [vec![1; cells], vec![1; cells], vec![1; cells]],
        grid_rows,
        grid_cols,
    )
    .unwrap()
}

fn asymmetric_luma(width: usize, height: usize) -> Vec<u16> {
    let mut state = 0x0123_4567_89ab_cdefu64;
    (0..width * height)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) % 256) as u16
        })
        .collect()
}

fn workspace(pixel_format: PixelFormat, width: usize, height: usize) -> CurrentFrameWorkspace<u8> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        pixel_format,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    CurrentFrameWorkspace::new(info, 0).unwrap()
}

fn ref_luma(curr: &[u16], w: usize, h: usize, x: usize, y: usize) -> u16 {
    curr[y.min(h - 1) * w + x.min(w - 1)]
}

fn ref_luma_offset(
    curr: &[u16],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
    offset: (isize, isize),
) -> u16 {
    let sx = (x as isize + offset.0).clamp(0, (w - 1) as isize) as usize;
    let sy = (y as isize + offset.1).clamp(0, (h - 1) as isize) as usize;
    curr[sy * w + sx]
}

#[allow(clippy::too_many_arguments)]
fn reference_filtered_sample(
    pre: u8,
    curr: &[u16],
    lw: usize,
    lh: usize,
    x_luma: usize,
    y_luma: usize,
    params: &CcsoPlaneParams,
    bit_depth: BitDepth,
) -> u8 {
    let edge_clf = params.ccso_edge_clf.unwrap();
    let mei = if edge_clf { 2usize } else { 3 };
    let max_band = 1usize << params.ccso_max_band_log2.unwrap();
    let band_shift = bit_depth.bits() - params.ccso_max_band_log2.unwrap();
    let quant_step = i32::from(ccso_quant_step(
        params.ccso_scale_idx.unwrap(),
        params.ccso_quant_idx.unwrap(),
    ));
    let offsets = ccso_sample_offsets(params.ccso_ext_filter.unwrap()).unwrap();
    let center = ref_luma(curr, lw, lh, x_luma, y_luma);
    let band = usize::from(center >> band_shift);
    let s0 = ref_luma_offset(curr, lw, lh, x_luma, y_luma, offsets[0]);
    let s1 = ref_luma_offset(curr, lw, lh, x_luma, y_luma, offsets[1]);
    let cls0 = ccso_score(i32::from(s0) - i32::from(center), quant_step, edge_clf);
    let cls1 = ccso_score(i32::from(s1) - i32::from(center), quant_step, edge_clf);
    let index = (cls0 * mei + cls1) * max_band + band;
    let base = CCSO_OFFSET[usize::from(params.ccso_offset_idx[index])];
    let offset = base * (i32::from(params.ccso_scale_idx.unwrap()) + 1);
    (i32::from(pre) + offset).clamp(0, i32::from(bit_depth.max_sample())) as u8
}

#[test]
fn ccso_matches_per_sample_reference_for_edge_classifiers() {
    let luma_width = 18;
    let luma_height = 10;
    let curr_luma = asymmetric_luma(luma_width, luma_height);
    let grid = full_grid(luma_width, luma_height);
    for &(plane, ext_filter, edge_clf) in &[(0usize, 4u8, false), (1, 3, true), (2, 6, false)] {
        let sub = usize::from(plane > 0);
        let mei = if edge_clf { 2usize } else { 3 };
        let params = edge_plane(ext_filter, edge_clf, 2, mei * mei * 4);
        let mut workspace = yuv420_workspace(luma_width, luma_height, 0);
        let plane_id = plane_id(plane);
        let (pw, ph) = {
            let size = workspace.plane(plane_id).unwrap().storage_size();
            (size.width(), size.height())
        };
        let mut state = 0xdead_beef_cafe_f00du64;
        for y in 0..ph {
            for x in 0..pw {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                workspace
                    .set_reconstructed_sample(plane_id, x, y, ((state >> 33) % 256) as u8)
                    .unwrap();
            }
        }
        let pre = workspace.samples(plane_id).unwrap().to_vec();
        ccso_plane(
            &mut workspace,
            &curr_luma,
            luma_width,
            luma_height,
            plane,
            &params,
            &grid,
            None,
            BitDepth::Eight,
        )
        .unwrap();
        let post = workspace.samples(plane_id).unwrap();
        for y in 0..ph {
            for x in 0..pw {
                let expected = reference_filtered_sample(
                    pre[y * pw + x],
                    &curr_luma,
                    luma_width,
                    luma_height,
                    x << sub,
                    y << sub,
                    &params,
                    BitDepth::Eight,
                );
                assert_eq!(
                    post[y * pw + x],
                    expected,
                    "plane {plane} sample ({x}, {y}) ext_filter {ext_filter} edge_clf {edge_clf}"
                );
            }
        }
    }
}

#[test]
fn chroma_ccso_uses_frame_subsampling_for_yuv422() {
    let luma_width = 18;
    let luma_height = 10;
    let curr_luma = asymmetric_luma(luma_width, luma_height);
    let grid = full_grid(luma_width, luma_height);
    let params = edge_plane(0, false, 2, 36);
    let mut workspace = workspace(PixelFormat::Yuv422, luma_width, luma_height);
    let plane_id = PlaneId::U;
    let (pw, ph) = {
        let size = workspace.plane(plane_id).unwrap().storage_size();
        (size.width(), size.height())
    };
    for y in 0..ph {
        for x in 0..pw {
            workspace
                .set_reconstructed_sample(plane_id, x, y, ((x * 13 + y * 17) % 256) as u8)
                .unwrap();
        }
    }
    let pre = workspace.samples(plane_id).unwrap().to_vec();
    ccso_plane(
        &mut workspace,
        &curr_luma,
        luma_width,
        luma_height,
        1,
        &params,
        &grid,
        None,
        BitDepth::Eight,
    )
    .unwrap();
    let post = workspace.samples(plane_id).unwrap();
    for y in 0..ph {
        for x in 0..pw {
            let expected = reference_filtered_sample(
                pre[y * pw + x],
                &curr_luma,
                luma_width,
                luma_height,
                x << 1,
                y,
                &params,
                BitDepth::Eight,
            );
            assert_eq!(post[y * pw + x], expected, "sample ({x}, {y})");
        }
    }
}

#[test]
fn stripe_outputs_match_full_frame_across_restoration_boundaries() {
    let luma_width = 18;
    let luma_height = 128;
    let curr_luma = asymmetric_luma(luma_width, luma_height);
    let grid = full_grid(luma_width, luma_height);
    let params = edge_plane(4, false, 2, 36);

    for &(plane, sub_x, sub_y) in &[(0usize, 0usize, 0usize), (1, 1, 1), (2, 1, 1)] {
        let width = luma_width >> sub_x;
        let height = luma_height >> sub_y;
        let stripe_start = 56 >> sub_y;
        let stripe_end = 120 >> sub_y;
        let source: Vec<u16> = (0..width * height)
            .map(|index| ((index * 17 + plane * 29) & 255) as u16)
            .collect();
        let mut expected = StripePlane::from_samples(width, height, 0, source.clone()).unwrap();
        let mut actual = StripePlane::from_samples(
            width,
            height,
            stripe_start,
            source[stripe_start * width..stripe_end * width].to_vec(),
        )
        .unwrap();
        let prepared =
            prepare_ccso_plane(plane, &params, &grid, BitDepth::Eight, (sub_x, sub_y)).unwrap();

        ccso_apply(
            &mut expected,
            FramePlane::window(&curr_luma, luma_width, luma_height, 0, luma_height).unwrap(),
            plane,
            &prepared,
            &grid,
            None,
            None,
        )
        .unwrap();
        ccso_apply(
            &mut actual,
            FramePlane::window(&curr_luma, luma_width, luma_height, 0, luma_height).unwrap(),
            plane,
            &prepared,
            &grid,
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            actual.samples(),
            &expected.samples()[stripe_start * width..stripe_end * width],
            "plane {plane}"
        );
    }
}

#[test]
fn ccso_offset_lut_rejects_out_of_range_offset_index() {
    let mut params = edge_plane(0, false, 2, 36);
    params.ccso_offset_idx[7] = 8;
    assert!(matches!(
        ccso_offset_lut(&params, 36),
        Err(CcsoError::Params)
    ));
}

#[test]
fn luma_ccso_filters_partial_coded_edge_block() {
    let width = 18;
    let height = 10;
    let mut workspace = yuv420_workspace(width, height, 100);
    let curr_luma = vec![100u16; width * height];
    ccso_plane(
        &mut workspace,
        &curr_luma,
        width,
        height,
        0,
        &bo_plane(1),
        &full_luma_grid(width, height),
        None,
        BitDepth::Eight,
    )
    .unwrap();
    assert_eq!(
        workspace
            .reconstructed_sample(PlaneId::Y, width - 1, height - 1)
            .unwrap(),
        101,
        "CCSO must process the bottom-right partial coded block"
    );
}

#[test]
fn luma_ccso_preserves_lossless_4x4_samples() {
    let width = 8;
    let height = 8;
    let mut workspace = yuv420_workspace(width, height, 100);
    let curr_luma = vec![100u16; width * height];
    let lossless_block = crate::filters::deblock::DeblockBlock {
        r: 0,
        c: 0,
        luma_prediction: crate::filters::deblock::DeblockPredictionUnit {
            base_r: 0,
            base_c: 0,
            default_sub_pu_tx: 0,
        },
        chroma_prediction: crate::filters::deblock::DeblockPredictionUnit {
            base_r: 0,
            base_c: 0,
            default_sub_pu_tx: 0,
        },
        chroma_base_r: 0,
        chroma_base_c: 0,
        n4w: 1,
        n4h: 1,
        luma_tx: 0,
        chroma_tx: Some(0),
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 0,
        skip: false,
        lossless: true,
    };
    let lossless = crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
        2,
        2,
        &[lossless_block],
        [&[], &[]],
    )
    .unwrap();

    ccso_plane(
        &mut workspace,
        &curr_luma,
        width,
        height,
        0,
        &bo_plane(1),
        &full_luma_grid(width, height),
        Some(&lossless),
        BitDepth::Eight,
    )
    .unwrap();

    assert_eq!(
        workspace.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(),
        100
    );
    assert_eq!(
        workspace.reconstructed_sample(PlaneId::Y, 4, 0).unwrap(),
        101
    );
}

fn tile_ref_sample(
    pre: u16,
    curr: &[u16],
    lw: usize,
    at: (usize, usize),
    x_clamp: (usize, usize),
    y_clamp: (usize, usize),
    params: &CcsoPlaneParams,
) -> u16 {
    let bit_depth = BitDepth::Eight;
    let edge_clf = params.ccso_edge_clf.unwrap();
    let bo_only = params.ccso_bo_only.unwrap();
    let mei = if bo_only {
        1usize
    } else if edge_clf {
        2
    } else {
        3
    };
    let max_band = 1usize << params.ccso_max_band_log2.unwrap();
    let band_shift = bit_depth.bits() - params.ccso_max_band_log2.unwrap();
    let quant_step = i32::from(ccso_quant_step(
        params.ccso_scale_idx.unwrap(),
        params.ccso_quant_idx.unwrap(),
    ));
    let offsets = ccso_sample_offsets(params.ccso_ext_filter.unwrap()).unwrap();
    let tap = |dx: isize, dy: isize| -> i32 {
        let sx = (at.0 as isize + dx).clamp(x_clamp.0 as isize, x_clamp.1 as isize) as usize;
        let sy = (at.1 as isize + dy).clamp(y_clamp.0 as isize, y_clamp.1 as isize) as usize;
        i32::from(curr[sy * lw + sx])
    };
    let center = tap(0, 0);
    let band = (center >> band_shift) as usize;
    let (cls0, cls1) = if bo_only {
        (0, 0)
    } else {
        (
            ccso_score(
                tap(offsets[0].0, offsets[0].1) - center,
                quant_step,
                edge_clf,
            ),
            ccso_score(
                tap(offsets[1].0, offsets[1].1) - center,
                quant_step,
                edge_clf,
            ),
        )
    };
    let index = (cls0 * mei + cls1) * max_band + band;
    let base = CCSO_OFFSET[usize::from(params.ccso_offset_idx[index])];
    let offset = base * (i32::from(params.ccso_scale_idx.unwrap()) + 1);
    (i32::from(pre) + offset).clamp(0, i32::from(bit_depth.max_sample())) as u16
}

/// Filters one 8-bit luma plane and returns `(actual, tile-aware reference)`.
fn tiled_luma_ccso(
    params: &CcsoPlaneParams,
    shift: u32,
    luma: (usize, usize),
    tiles: Option<(&[u32], &[u32])>,
) -> (Vec<u16>, Vec<u16>) {
    let (lw, lh) = luma;
    let curr_luma = asymmetric_luma(lw, lh);
    let blk = 4usize << shift;
    let grid_cols = lw.div_ceil(blk);
    let grid_rows = lh.div_ceil(blk);
    let cells = grid_rows * grid_cols;
    let grid = CcsoUnitGrid::new(
        true,
        shift,
        [true, false, false],
        [vec![1; cells], vec![0; cells], vec![0; cells]],
        grid_rows,
        grid_cols,
    )
    .unwrap();
    let pre: Vec<u16> = (0..lw * lh).map(|i| ((i * 37 + 11) % 251) as u16).collect();
    let mut destination = StripePlane::from_samples(lw, lh, 0, pre.clone()).unwrap();
    let prepared = prepare_ccso_plane(0, params, &grid, BitDepth::Eight, (0, 0)).unwrap();
    ccso_apply(
        &mut destination,
        FramePlane::window(&curr_luma, lw, lh, 0, lh).unwrap(),
        0,
        &prepared,
        &grid,
        None,
        tiles,
    )
    .unwrap();
    let expected = (0..lw * lh)
        .map(|index| {
            let (x, y) = (index % lw, index / lw);
            let x_clamp =
                luma_tile_clamp(tiles.map(|(_, cols)| cols), x / blk * blk / MI_SIZE, lw - 1);
            let y_clamp =
                luma_tile_clamp(tiles.map(|(rows, _)| rows), y / blk * blk / MI_SIZE, lh - 1);
            tile_ref_sample(pre[index], &curr_luma, lw, (x, y), x_clamp, y_clamp, params)
        })
        .collect();
    (destination.samples().to_vec(), expected)
}

fn assert_luma_matches(actual: &[u16], expected: &[u16], lw: usize, label: &str) {
    for (index, (&got, &want)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            got,
            want,
            "{label}: luma sample ({}, {}) expected {want} got {got}",
            index % lw,
            index / lw
        );
    }
}

#[test]
fn ccso_honours_tile_column_clamp_on_left_tile_edge() {
    let params = edge_plane(1, false, 2, 36);
    let (actual, expected) = tiled_luma_ccso(&params, 3, (256, 8), Some((&[0, 2], &[0, 16, 64])));
    assert_luma_matches(&actual, &expected, 256, "ext_filter 1");
}

#[test]
fn ccso_honours_tile_column_clamp_for_diagonal_taps() {
    let params = edge_plane(2, false, 2, 36);
    let (actual, expected) = tiled_luma_ccso(&params, 3, (256, 8), Some((&[0, 2], &[0, 16, 64])));
    assert_luma_matches(&actual, &expected, 256, "ext_filter 2");
}

#[test]
fn ccso_bo_only_honours_tile_column_clamp() {
    let mut params = bo_plane(1);
    params.ccso_offset_idx = vec![1, 4];
    let (actual, expected) = tiled_luma_ccso(&params, 3, (128, 8), Some((&[0, 2], &[0, 20, 32])));
    assert_luma_matches(&actual, &expected, 128, "bo_only");
}

#[test]
fn ccso_single_tile_column_is_unchanged_by_the_tile_clamp() {
    let params = edge_plane(1, false, 2, 36);
    let (tiled, expected) = tiled_luma_ccso(&params, 3, (128, 8), Some((&[0, 2], &[0, 32])));
    let (untiled, _) = tiled_luma_ccso(&params, 3, (128, 8), None);
    assert_luma_matches(&tiled, &expected, 128, "single tile column");
    assert_eq!(
        tiled, untiled,
        "single tile column must match the untiled run"
    );
}
