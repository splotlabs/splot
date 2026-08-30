// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::filters::source::DeblockedSource;
use crate::test_support::yuv420_workspace as workspace_8bit;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneSize,
    cdef_filter_sample,
};

pub(super) fn constant_cdef_grid(
    mi_rows: usize,
    mi_cols: usize,
    value: usize,
) -> Result<CdefUnitGrid, CdefError> {
    let rows = mi_rows.div_ceil(CDEF_UNIT_MI);
    let cols = mi_cols.div_ceil(CDEF_UNIT_MI);
    let values_len = rows.checked_mul(cols).ok_or(CdefError::Geometry)?;
    CdefUnitGrid::new(rows, cols, vec![Some(value); values_len])
}

fn cdef_general_intra_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    params: CdefFrameParams,
    mi_rows: usize,
    mi_cols: usize,
    bit_depth: BitDepth,
) -> Result<(), CdefError> {
    let grid = constant_cdef_grid(mi_rows, mi_cols, 0)?;
    cdef_general_intra_frame_indexed(
        workspace,
        &[params],
        &grid,
        None,
        None,
        (mi_rows, mi_cols),
        bit_depth,
    )
}

fn cdef_general_intra_frame_indexed<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    strengths: &[CdefFrameParams],
    grid: &CdefUnitGrid,
    skip_grid: Option<&CdefSkipGrid>,
    lossless_grid: Option<&crate::filters::lossless::LosslessBlockGrid>,
    mi_grid: (usize, usize),
    bit_depth: BitDepth,
) -> Result<(), CdefError> {
    let height = workspace
        .plane(PlaneId::Y)
        .map_err(|_| CdefError::Workspace)?
        .storage_size()
        .height();
    let format = workspace.info().pixel_format();
    let frame = cdef_stripe(
        DeblockedPlanes::frame(workspace).ok_or(CdefError::Workspace)?,
        Some(strengths),
        Some(grid),
        skip_grid,
        lossless_grid,
        mi_grid,
        (
            usize::from(format.subsampling_x()),
            usize::from(format.subsampling_y()),
        ),
        bit_depth,
        None,
        0,
        height,
    )?;
    let CdefFrame {
        filtered_y,
        filtered_u,
        filtered_v,
        ..
    } = frame;
    for (plane, filtered) in [
        (PlaneId::Y, Some(filtered_y)),
        (PlaneId::U, filtered_u),
        (PlaneId::V, filtered_v),
    ] {
        let Some(filtered) = filtered else {
            continue;
        };
        let end = filtered.end_y().ok_or(CdefError::Geometry)?;
        for y in filtered.origin_y()..end {
            for (x, &sample) in filtered
                .row(y)
                .ok_or(CdefError::Workspace)?
                .iter()
                .enumerate()
            {
                workspace
                    .set_reconstructed_sample(
                        plane,
                        x,
                        y,
                        T::try_from_u16(sample).map_err(|_| CdefError::Workspace)?,
                    )
                    .map_err(|_| CdefError::Workspace)?;
            }
        }
    }
    Ok(())
}

#[test]
fn tap_reach_covers_direction_table() {
    let max_offset = CDEF_DIRECTIONS
        .iter()
        .flatten()
        .flatten()
        .map(|&offset| offset.unsigned_abs() as usize)
        .max()
        .unwrap();
    assert_eq!(CDEF_TAP_REACH, max_offset);
}

#[test]
fn flat_frame_is_unchanged() {
    let mut ws = workspace_8bit(64, 64, 100);
    cdef_general_intra_frame(
        &mut ws,
        CdefFrameParams {
            y_pri: 4,
            y_sec: 4,
            uv_pri: 0,
            uv_sec: 0,
            damping: 4,
        },
        16,
        16,
        BitDepth::Eight,
    )
    .unwrap();
    assert!(
        ws.samples(PlaneId::Y).unwrap().iter().all(|&s| s == 100),
        "flat luma unchanged"
    );
    assert!(
        ws.samples(PlaneId::U).unwrap().iter().all(|&s| s == 100),
        "flat chroma unchanged"
    );
}

#[test]
fn partial_coded_edge_blocks_do_not_exceed_plane_bounds() {
    let width = 18usize;
    let height = 10usize;
    let mut ws = workspace_8bit(width, height, 100);
    cdef_general_intra_frame(
        &mut ws,
        CdefFrameParams {
            y_pri: 4,
            y_sec: 4,
            uv_pri: 2,
            uv_sec: 4,
            damping: 4,
        },
        height.div_ceil(MI_SIZE),
        width.div_ceil(MI_SIZE),
        BitDepth::Eight,
    )
    .unwrap();
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        assert!(
            ws.samples(plane).unwrap().iter().all(|&s| s == 100),
            "flat partial-edge plane remains unchanged"
        );
    }
}

#[test]
fn yuv422_chroma_cdef_filters_full_vertical_block() {
    let mut ws = workspace(PixelFormat::Yuv422, 16, 8, 100);
    for y in 0..8 {
        for x in 0..8 {
            let value = if y % 2 == 0 { 130 } else { 126 };
            ws.set_reconstructed_sample(PlaneId::U, x, y, value)
                .unwrap();
        }
    }
    let before = ws.samples(PlaneId::U).unwrap().to_vec();
    cdef_general_intra_frame(
        &mut ws,
        CdefFrameParams {
            y_pri: 0,
            y_sec: 0,
            uv_pri: 2,
            uv_sec: 4,
            damping: 4,
        },
        2,
        4,
        BitDepth::Eight,
    )
    .unwrap();
    let after = ws.samples(PlaneId::U).unwrap();
    assert_ne!(
        &before[4 * 8..],
        &after[4 * 8..],
        "4:2:2 CDEF must cover the bottom half of the chroma block"
    );
}

#[test]
fn small_ringing_step_is_deringed_within_bounds() {
    let mut ws = workspace_8bit(64, 64, 100);
    seed_luma_ripple(&mut ws);
    let before = luma_8x8(&ws);
    cdef_general_intra_frame(&mut ws, cdef_ripple_params(), 16, 16, BitDepth::Eight).unwrap();
    let after = luma_8x8(&ws);
    assert_ne!(before, after, "the ripple block must be filtered (changed)");
    assert!(
        after.iter().all(|&s| (97..=103).contains(&s)),
        "deringed samples stay within the original [97, 103] band: {after:?}"
    );
    assert_eq!(
        ws.reconstructed_sample(PlaneId::Y, 40, 40).unwrap(),
        100,
        "far flat region untouched"
    );
}

#[test]
fn monochrome_cdef_filters_luma_without_chroma_planes() {
    let mut ws = workspace(PixelFormat::Monochrome, 64, 64, 100);
    seed_luma_ripple(&mut ws);
    let before = luma_8x8(&ws);
    cdef_general_intra_frame(&mut ws, cdef_ripple_params(), 16, 16, BitDepth::Eight).unwrap();
    let after = luma_8x8(&ws);
    assert_ne!(before, after, "monochrome luma is still filtered");
    assert!(ws.plane(PlaneId::U).is_err(), "monochrome has no U plane");
}

#[test]
fn skip_grid_leaves_all_skipped_8x8_unfiltered() {
    let (before, after) = run_skip_grid_ripple(vec![true; 16 * 16]);
    assert_eq!(before, after, "all-skipped CDEF block bypasses filtering");
}

#[test]
fn skip_grid_filters_mixed_8x8() {
    let mut skip_values = vec![true; 16 * 16];
    skip_values[0] = false;
    let (before, after) = run_skip_grid_ripple(skip_values);
    assert_ne!(before, after, "mixed CDEF block still filters");
}

fn run_skip_grid_ripple(skip_values: Vec<bool>) -> (Vec<u8>, Vec<u8>) {
    let mut ws = workspace_8bit(64, 64, 100);
    seed_luma_ripple(&mut ws);
    let before = luma_8x8(&ws);
    let grid = constant_cdef_grid(16, 16, 0).unwrap();
    let skip = CdefSkipGrid::new(16, 16, skip_values).unwrap();
    cdef_general_intra_frame_indexed(
        &mut ws,
        &[cdef_ripple_params()],
        &grid,
        Some(&skip),
        None,
        (16, 16),
        BitDepth::Eight,
    )
    .unwrap();
    (before, luma_8x8(&ws))
}

#[test]
fn lossless_grid_leaves_lossless_luma_8x8_unfiltered() {
    let mut ws = workspace_8bit(64, 64, 100);
    seed_luma_ripple(&mut ws);
    let before = luma_8x8(&ws);
    let grid = constant_cdef_grid(16, 16, 0).unwrap();
    let blocks = [deblock_block(0, 0, 2, 2, true)];
    let lossless = crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
        16,
        16,
        &blocks,
        [&[], &[]],
    )
    .unwrap();
    cdef_general_intra_frame_indexed(
        &mut ws,
        &[cdef_ripple_params()],
        &grid,
        None,
        Some(&lossless),
        (16, 16),
        BitDepth::Eight,
    )
    .unwrap();
    assert_eq!(before, luma_8x8(&ws));
}

pub(super) fn deblock_block(
    r: usize,
    c: usize,
    n4w: usize,
    n4h: usize,
    lossless: bool,
) -> crate::filters::deblock::DeblockBlock {
    crate::filters::deblock::DeblockBlock {
        r,
        c,
        luma_prediction: crate::filters::deblock::DeblockPredictionUnit {
            base_r: r,
            base_c: c,
            default_sub_pu_tx: 0,
        },
        chroma_prediction: crate::filters::deblock::DeblockPredictionUnit {
            base_r: r,
            base_c: c,
            default_sub_pu_tx: 0,
        },
        chroma_base_r: r,
        chroma_base_c: c,
        n4w,
        n4h,
        luma_tx: 0,
        chroma_tx: Some(0),
        sub_pu_size: None,
        chroma_transform_only: false,
        qindex: 0,
        skip: false,
        lossless,
    }
}

fn workspace(
    pixel_format: PixelFormat,
    width: usize,
    height: usize,
    fill: u8,
) -> CurrentFrameWorkspace<u8> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        pixel_format,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    CurrentFrameWorkspace::new(info, fill).unwrap()
}

fn seed_luma_ripple(ws: &mut CurrentFrameWorkspace<u8>) {
    for y in 0..8 {
        for x in 0..8 {
            let v = if (x + y) % 2 == 0 { 103 } else { 97 };
            ws.set_reconstructed_sample(PlaneId::Y, x, y, v).unwrap();
        }
    }
}

fn luma_8x8(ws: &CurrentFrameWorkspace<u8>) -> Vec<u8> {
    (0..8)
        .flat_map(|y| (0..8).map(move |x| (x, y)))
        .map(|(x, y)| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap())
        .collect()
}

const fn cdef_ripple_params() -> CdefFrameParams {
    CdefFrameParams {
        y_pri: 4,
        y_sec: 4,
        uv_pri: 0,
        uv_sec: 0,
        damping: 4,
    }
}

/// Asserts `compute_cdef_filter_plane` output for the 8x8 luma block at
/// mi-position `(r, c)` matches the per-sample `gather_taps` reference bit-for-bit
/// across every direction and strength pair. Seeds a non-flat luma region so the
/// filter is active. `r = c = 0` drives the gather-once edge path (taps off the
/// plane); an interior `(r, c)` drives the batched fast path.
fn assert_cdef_block_matches_per_sample_reference(
    r: usize,
    c: usize,
    seed: core::ops::Range<usize>,
) {
    let mut ws = workspace_8bit(64, 64, 100);
    for y in seed.clone() {
        for x in seed.clone() {
            let v = (60 + (x * 7 + y * 13) % 130) as u8;
            ws.set_reconstructed_sample(PlaneId::Y, x, y, v).unwrap();
        }
    }
    let snap = FramePlane::new(&ws, PlaneId::Y).unwrap();
    let (x0, y0) = (c * MI_SIZE, r * MI_SIZE);
    for dir in 0..8usize {
        for (pri_str, sec_str) in [(0, 3), (5, 0), (5, 3), (12, 4)] {
            let ctx = CdefFilterCtx {
                r,
                c,
                mi_row_start: 0,
                mi_col_start: 0,
                pri_str,
                sec_str,
                damping: 4,
                dir,
                sub: 0,
                coeff_shift: 0,
                max_sample: 255,
                mi_rows: 16,
                mi_cols: 16,
                frame_sub_x: 1,
                frame_sub_y: 1,
            };
            let mut pad = [0u16; CDEF_PADDED_AREA];
            let mut filtered = StripePlane::copy_from(snap, 0, 64).unwrap();
            compute_cdef_filter_plane::<u8>(snap, &ctx, &mut pad, &mut filtered).unwrap();
            let offsets = CdefTapOffsets::for_direction(ctx.dir);
            for i in 0..8 {
                for j in 0..8 {
                    let (x, y) = (x0 + j, y0 + i);
                    let center = snap.get(x as isize, y as isize).unwrap();
                    let taps = gather_taps(snap, &offsets, x, y, 0, 0, 64, 64, center);
                    let expected = cdef_filter_sample(
                        &taps,
                        ctx.pri_str,
                        ctx.sec_str,
                        ctx.damping,
                        ctx.coeff_shift,
                    )
                    .clamp(0, ctx.max_sample);
                    assert_eq!(
                        i32::from(filtered.row(y).unwrap()[x]),
                        expected,
                        "r={r} c={c} dir={dir} pri={pri_str} sec={sec_str} i={i} j={j}"
                    );
                }
            }
        }
    }
}

#[test]
fn interior_fast_path_matches_per_sample_reference() {
    assert_cdef_block_matches_per_sample_reference(6, 6, 20..36);
}

#[test]
fn edge_block_matches_per_sample_reference() {
    assert_cdef_block_matches_per_sample_reference(0, 0, 0..16);
}

#[test]
fn zero_strengths_elide_all_writes() {
    let mut ws = workspace_8bit(64, 64, 100);
    seed_luma_ripple(&mut ws);
    let before: Vec<u8> = ws.samples(PlaneId::Y).unwrap().to_vec();
    cdef_general_intra_frame(
        &mut ws,
        CdefFrameParams {
            y_pri: 0,
            y_sec: 0,
            uv_pri: 0,
            uv_sec: 0,
            damping: 4,
        },
        16,
        16,
        BitDepth::Eight,
    )
    .unwrap();
    assert_eq!(
        before,
        ws.samples(PlaneId::Y).unwrap(),
        "all-zero strengths leave the ripple untouched"
    );
}

#[test]
fn snapshot_get_bounds() {
    let ws = workspace_8bit(16, 16, 50);
    let snap = FramePlane::new(&ws, PlaneId::Y).unwrap();
    assert_eq!(snap.get(0, 0), Some(50));
    assert_eq!(snap.get(15, 15), Some(50));
    assert_eq!(snap.get(-1, 0), None, "negative x off-frame");
    assert_eq!(snap.get(16, 0), None, "x past width off-frame");
    assert_eq!(snap.get(0, 16), None, "y past height off-frame");
}

#[test]
fn stripe_frames_match_full_frame_across_restoration_boundaries() {
    let mut full = workspace_8bit(128, 128, 0);
    let mut striped = workspace_8bit(128, 128, 0);
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = full.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let sample = ((x * 7 + y * 13 + plane.index() * 31) & 255) as u8;
                full.set_reconstructed_sample(plane, x, y, sample).unwrap();
                striped
                    .set_reconstructed_sample(plane, x, y, sample)
                    .unwrap();
            }
        }
    }
    let params = CdefFrameParams {
        y_pri: 4,
        y_sec: 3,
        uv_pri: 2,
        uv_sec: 4,
        damping: 4,
    };
    let grid = constant_cdef_grid(32, 32, 0).unwrap();
    cdef_general_intra_frame_indexed(
        &mut full,
        &[params],
        &grid,
        None,
        None,
        (32, 32),
        BitDepth::Eight,
    )
    .unwrap();

    let ranges = [(0, 56), (56, 120), (120, 128)];
    let mut source = DeblockedSource::new(striped);
    assert!(source.publish_final_rows(128));
    let leases = ranges
        .iter()
        .map(|&(start, end)| source.lease(start, end, 10).unwrap())
        .collect::<Vec<_>>();
    let middle = leases[1].planes().unwrap().y;
    assert_eq!((middle.origin_y(), middle.end_y()), (46, 128));
    assert!(middle.row(54).is_some());
    assert!(middle.row(127).is_some());
    let frames = ranges
        .into_iter()
        .zip(&leases)
        .map(|((start, end), deblocked)| {
            cdef_stripe(
                deblocked.planes().unwrap(),
                Some(&[params]),
                Some(&grid),
                None,
                None,
                (32, 32),
                (1, 1),
                BitDepth::Eight,
                None,
                start,
                end,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let mut actual = Vec::new();
        for frame in &frames {
            let filtered = match plane {
                PlaneId::Y => Some(&frame.filtered_y),
                PlaneId::U => frame.filtered_u.as_ref(),
                PlaneId::V => frame.filtered_v.as_ref(),
            }
            .unwrap();
            actual.extend_from_slice(filtered.samples());
        }
        let expected = full
            .samples(plane)
            .unwrap()
            .iter()
            .copied()
            .map(u16::from)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "plane {plane:?}");
    }
}

fn workspace_chroma_ripple(row_varying: bool) -> CurrentFrameWorkspace<u8> {
    let mut ws = workspace_8bit(64, 64, 128);
    for plane in [PlaneId::U, PlaneId::V] {
        for y in 0..32usize {
            for x in 0..32usize {
                let v = if y % 2 == 0 { 130 } else { 126 };
                ws.set_reconstructed_sample(plane, x, y, v).unwrap();
            }
        }
    }
    for y in 0..8usize {
        for x in 0..8usize {
            let g = if row_varying { y } else { x } as i32;
            let v = (100 + g * 6).clamp(0, 255) as u8;
            ws.set_reconstructed_sample(PlaneId::Y, x, y, v).unwrap();
        }
    }
    ws
}

fn chroma_top_left_4x4(ws: &CurrentFrameWorkspace<u8>, plane: PlaneId) -> Vec<u8> {
    (0..4)
        .flat_map(|y| (0..4).map(move |x| (x, y)))
        .map(|(x, y)| ws.reconstructed_sample(plane, x, y).unwrap())
        .collect()
}

fn run_cdef(ws: &mut CurrentFrameWorkspace<u8>, uv_pri: i32, uv_sec: i32) {
    cdef_general_intra_frame(
        ws,
        CdefFrameParams {
            y_pri: 0,
            y_sec: 0,
            uv_pri,
            uv_sec,
            damping: 4,
        },
        16,
        16,
        BitDepth::Eight,
    )
    .unwrap();
}

#[test]
fn zero_uv_strengths_leave_chroma_untouched() {
    let before = workspace_chroma_ripple(true);
    let mut after = workspace_chroma_ripple(true);
    run_cdef(&mut after, 0, 0);
    for plane in [PlaneId::U, PlaneId::V] {
        assert_eq!(
            before.samples(plane).unwrap(),
            after.samples(plane).unwrap(),
            "uv strengths 0 -> chroma unchanged",
        );
    }
}

#[test]
fn nonzero_uv_strengths_dering_chroma_only() {
    let before = workspace_chroma_ripple(true);
    let mut after = workspace_chroma_ripple(true);
    run_cdef(&mut after, 2, 4);
    for plane in [PlaneId::U, PlaneId::V] {
        assert_ne!(
            before.samples(plane).unwrap(),
            after.samples(plane).unwrap(),
            "nonzero uv -> chroma derings (changes)",
        );
        assert!(
            after
                .samples(plane)
                .unwrap()
                .iter()
                .all(|&s| (126..=130).contains(&s)),
            "deringed chroma stays within the original [126, 130] band",
        );
    }
    assert_eq!(
        before.samples(PlaneId::Y).unwrap(),
        after.samples(PlaneId::Y).unwrap(),
        "uv strengths are chroma-only: luma untouched",
    );
}

#[test]
fn uv_dir_selection_tracks_luma_direction_only_when_uv_pri_nonzero() {
    let mut row_block = [[0i32; 8]; 8];
    let mut col_block = [[0i32; 8]; 8];
    for i in 0..8 {
        for j in 0..8 {
            row_block[i][j] = (100 + i as i32 * 6) - 128;
            col_block[i][j] = (100 + j as i32 * 6) - 128;
        }
    }
    let (row_dir, _) = cdef_direction(&row_block);
    let (col_dir, _) = cdef_direction(&col_block);
    assert_ne!(
        row_dir, col_dir,
        "the two luma blocks must select different yDirs to drive Cdef_Uv_Dir",
    );

    let mut horiz = workspace_chroma_ripple(true);
    let mut vert = workspace_chroma_ripple(false);
    run_cdef(&mut horiz, 2, 4);
    run_cdef(&mut vert, 2, 4);
    assert_ne!(
        chroma_top_left_4x4(&horiz, PlaneId::U),
        chroma_top_left_4x4(&vert, PlaneId::U),
        "uv_pri != 0: Cdef_Uv_Dir maps yDir to a primary chroma direction, so the \
         chroma output depends on the luma direction",
    );

    let mut horiz0 = workspace_chroma_ripple(true);
    let mut vert0 = workspace_chroma_ripple(false);
    run_cdef(&mut horiz0, 0, 4);
    run_cdef(&mut vert0, 0, 4);
    assert_eq!(
        chroma_top_left_4x4(&horiz0, PlaneId::U),
        chroma_top_left_4x4(&vert0, PlaneId::U),
        "uv_pri == 0: direction is forced to 0, so the luma direction is ignored",
    );
}
