// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::filters::source::DeblockedWindow;
use crate::filters::source::StripeInitialization::CopyAll;
use crate::test_support::yuv420_workspace as workspace_8bit;
use splot_recon::{
    CDEF_DIRECTIONS, CdefSampleTaps, CdefTap, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex,
    PixelFormat, PlaneSize, cdef_direction, cdef_filter_sample,
};

const UNAVAILABLE_TAP: CdefTap = CdefTap {
    value: 0,
    available: false,
};

struct CdefTapOffsets {
    primary: [[(isize, isize); 2]; 2],
    secondary: [[[(isize, isize); 2]; 2]; 2],
}

impl CdefTapOffsets {
    fn for_direction(dir: usize) -> Self {
        let offset = |dir: usize, k: usize, sign: isize| -> (isize, isize) {
            (
                sign * CDEF_DIRECTIONS[dir & 7][k][0] as isize,
                sign * CDEF_DIRECTIONS[dir & 7][k][1] as isize,
            )
        };
        let mut primary = [[(0isize, 0isize); 2]; 2];
        let mut secondary = [[[(0isize, 0isize); 2]; 2]; 2];
        for k in 0..2 {
            for (sign_index, sign) in [-1isize, 1].into_iter().enumerate() {
                primary[k][sign_index] = offset(dir, k, sign);
                for (dir_off_index, dir_off) in [6usize, 2].into_iter().enumerate() {
                    secondary[k][sign_index][dir_off_index] = offset(dir + dir_off, k, sign);
                }
            }
        }
        Self { primary, secondary }
    }
}

#[expect(clippy::too_many_arguments)]
fn gather_taps<T: ReconSample>(
    snap: FramePlane<'_, T>,
    offsets: &CdefTapOffsets,
    x: usize,
    y: usize,
    start_x: usize,
    start_y: usize,
    inside_x: usize,
    inside_y: usize,
    center: i32,
) -> CdefSampleTaps {
    let fetch = |(dy, dx): (isize, isize)| -> CdefTap {
        let y = y as isize + dy;
        let x = x as isize + dx;
        if x >= start_x as isize
            && y >= start_y as isize
            && (x as usize) < inside_x
            && (y as usize) < inside_y
        {
            match snap.row(y as usize).and_then(|row| row.get(x as usize)) {
                Some(value) => CdefTap {
                    value: i32::from(value.to_u16()),
                    available: true,
                },
                None => UNAVAILABLE_TAP,
            }
        } else {
            UNAVAILABLE_TAP
        }
    };

    let mut primary = [[UNAVAILABLE_TAP; 2]; 2];
    let mut secondary = [[[UNAVAILABLE_TAP; 2]; 2]; 2];
    for k in 0..2 {
        for sign_index in 0..2 {
            primary[k][sign_index] = fetch(offsets.primary[k][sign_index]);
            for (dir_off_index, tap) in secondary[k][sign_index].iter_mut().enumerate() {
                *tap = fetch(offsets.secondary[k][sign_index][dir_off_index]);
            }
        }
    }

    CdefSampleTaps {
        center,
        primary,
        secondary,
    }
}

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
    let frame = crate::test_support::with_auto_backend!(backend => cdef_stripe(
        backend,
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
    ))?;
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
                    .set_reconstructed_sample(plane, x, y, sample)
                    .map_err(|_| CdefError::Workspace)?;
            }
        }
    }
    Ok(())
}

/// The chroma-pair
/// gather interleaves `u` and `v` lane by lane, from either storage type.
#[test]
fn pair_gather_matches_the_scalar_copy() {
    let side = CHROMA_PAIR_SIDE + 2 * CDEF_TAP_REACH;
    let u: Vec<u8> = (0..side * 9).map(|i| (i * 7 % 256) as u8).collect();
    let v: Vec<u8> = (0..side * 9).map(|i| (i * 11 % 256) as u8).collect();
    let mut pad = [0u16; CDEF_PADDED_AREA];
    crate::test_support::with_auto_backend!(backend => {
        gather_chroma_pair(backend, &u, &v, 1, 9, &mut pad)
    })
    .unwrap();
    let wide = |x: &[u8]| x.iter().map(|&b| u16::from(b)).collect::<Vec<_>>();
    let mut pad16 = [0u16; CDEF_PADDED_AREA];
    crate::test_support::with_auto_backend!(backend => {
        gather_chroma_pair(backend, &wide(&u), &wide(&v), 1, 9, &mut pad16)
    })
    .unwrap();
    assert_eq!(pad16, pad);
    for row in 0..side {
        for col in 0..CHROMA_PAIR_SPAN {
            let at = row * CDEF_PAIR_STRIDE + 2 * col;
            let source = 1 + row * 9 + col;
            assert_eq!(
                pad[at..at + 2],
                [u16::from(u[source]), u16::from(v[source])]
            );
        }
    }
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
    let (before, after) = run_skip_grid_ripple(vec![1; 16 * 16]);
    assert_eq!(before, after, "all-skipped CDEF block bypasses filtering");
}

#[test]
fn skip_grid_filters_mixed_8x8() {
    let mut skip_values = vec![1; 16 * 16];
    skip_values[0] = 0;
    let (before, after) = run_skip_grid_ripple(skip_values);
    assert_ne!(before, after, "mixed CDEF block still filters");
}

fn run_skip_grid_ripple(skip_values: Vec<u8>) -> (Vec<u8>, Vec<u8>) {
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
    let lossless =
        crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(16, 16, &blocks, &[])
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
        r: r as u16,
        c: c as u16,
        prediction: crate::filters::deblock::DeblockPredictionUnit::new(r, c, 0),
        n4w: n4w as u8,
        n4h: n4h as u8,
        tx: 0,
        sub_pu_size: None,
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
            let ctx = CdefBlockCtx {
                r,
                c,
                mi_row_start: 0,
                mi_col_start: 0,
                params: CdefFrameParams {
                    y_pri: 0,
                    y_sec: 0,
                    uv_pri: 0,
                    uv_sec: 0,
                    damping: 4,
                },
                coeff_shift: 0,
                mi_rows: 16,
                mi_cols: 16,
                sub_x: 1,
                sub_y: 1,
                luma_lossless: false,
                chroma_lossless: false,
                fill_flat: [true; 3],
            };
            let filter = CdefBlockFilter {
                pri_str,
                sec_str,
                damping: 4,
                dir,
                coeff_shift: 0,
            };
            let mut pad = [0u16; CDEF_PADDED_AREA];
            let mut filtered = StripePlane::copy_from_into(snap, 0, 64, None, CopyAll).unwrap();
            crate::test_support::with_auto_backend!(backend => {
                compute_cdef_filter_plane::<_, u8>(backend, snap, &ctx, false, &filter, &mut pad, &mut filtered)
            })
            .unwrap();
            let offsets = CdefTapOffsets::for_direction(filter.dir);
            for i in 0..8 {
                for j in 0..8 {
                    let (x, y) = (x0 + j, y0 + i);
                    let center = snap.get(x as isize, y as isize).unwrap();
                    let taps = gather_taps(snap, &offsets, x, y, 0, 0, 64, 64, center);
                    let expected = cdef_filter_sample(
                        &taps,
                        filter.pri_str,
                        filter.sec_str,
                        filter.damping,
                        filter.coeff_shift,
                    )
                    .clamp(0, 255);
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
    let (_progress, mut rows) = crate::test_support::frontier_rows(striped);
    assert!(rows.publish_final_rows(128));
    let mut carry = DeblockedWindow::default();
    let leases = ranges
        .iter()
        .map(|&range| {
            let mut window = DeblockedWindow::default();
            window.fill(&mut rows, &mut carry, range, 10).unwrap();
            window
        })
        .collect::<Vec<_>>();
    let middle = leases[1].planes().unwrap().y;
    assert_eq!((middle.origin_y(), middle.end_y()), (46, 128));
    assert!(middle.row(54).is_some());
    assert!(middle.row(127).is_some());
    let frames = ranges
        .into_iter()
        .zip(&leases)
        .map(|((start, end), deblocked)| {
            crate::test_support::with_auto_backend!(backend => cdef_stripe(
                backend,
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
            ))
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
        assert_eq!(actual, full.samples(plane).unwrap(), "plane {plane:?}");
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

#[test]
fn stripe_identity_follows_strengths_and_skipped_blocks() {
    let params = |y_sec, uv_pri| CdefFrameParams {
        y_pri: 0,
        y_sec,
        uv_pri,
        uv_sec: 0,
        damping: 4,
    };
    let strengths = [params(0, 0), params(2, 0), params(0, 3)];
    let grid = CdefUnitGrid::new(2, 1, vec![Some(1), Some(2)]).unwrap();
    let identity = |strengths: Option<&[CdefFrameParams]>, skip: Option<&CdefSkipGrid>, rows| {
        cdef_stripe_identity(strengths, Some(&grid), skip, (32, 16), rows)
    };
    assert_eq!(
        cdef_stripe_identity(None, None, None, (32, 16), (0, 64)),
        [true; 2]
    );
    assert_eq!(identity(Some(&strengths[..1]), None, (0, 64)), [false; 2]);
    assert_eq!(identity(Some(&strengths), None, (0, 64)), [false, true]);
    assert_eq!(identity(Some(&strengths), None, (64, 128)), [true, false]);
    assert_eq!(identity(Some(&strengths), None, (0, 128)), [false; 2]);
    let mut skipped = vec![1; 32 * 16];
    let all = CdefSkipGrid::new(32, 16, skipped.clone()).unwrap();
    assert_eq!(identity(Some(&strengths), Some(&all), (0, 128)), [true; 2]);
    skipped[20 * 16 + 5] = 0;
    let one = CdefSkipGrid::new(32, 16, skipped).unwrap();
    assert_eq!(identity(Some(&strengths), Some(&one), (0, 64)), [true; 2]);
    assert_eq!(
        identity(Some(&strengths), Some(&one), (64, 128)),
        [true, false]
    );
}

/// One scenario of the § 7.18 frame reference.
struct ReferenceCase<'a> {
    format: PixelFormat,
    size: (usize, usize),
    strengths: &'a [CdefFrameParams],
    unit_index: fn(usize) -> Option<usize>,
    skip: Option<fn(usize, usize) -> bool>,
    lossless: &'a [(usize, usize, bool)],
    tile_starts: Option<(&'a [u32], &'a [u32])>,
}

/// § 7.18 over the whole frame, one sample at a time, from `cdef_direction`
/// and `cdef_filter_sample`; each plane as packed rows.
fn reference_cdef<T: ReconSample>(
    ws: &CurrentFrameWorkspace<T>,
    case: &ReferenceCase<'_>,
    lookup: &CdefBlockLookup<'_>,
    coeff_shift: u32,
) -> [Vec<u16>; 3] {
    let planes = [PlaneId::Y, PlaneId::U, PlaneId::V];
    let snaps = planes.map(|plane| FramePlane::new(ws, plane));
    let mut out = snaps.map(|snap| {
        snap.map_or_else(Vec::new, |snap| {
            (0..snap.frame_height())
                .flat_map(|y| snap.row(y).unwrap().iter().map(|s| s.to_u16()))
                .collect()
        })
    });
    let (mi_rows, mi_cols) = (lookup.mi_rows, lookup.mi_cols);
    let luma = snaps[0].unwrap();
    for r in (0..mi_rows).step_by(2) {
        for c in (0..mi_cols).step_by(2) {
            let Some(index) = lookup.grid.strength_for_mi(r, c).unwrap() else {
                continue;
            };
            let Some(blocks) = lookup.block_planes(r, c).unwrap() else {
                continue;
            };
            let params = case.strengths[index];
            let (x0, y0) = (c * MI_SIZE, r * MI_SIZE);
            let (bw, bh) = (
                8.min(luma.width().saturating_sub(x0)),
                8.min(luma.frame_height().saturating_sub(y0)),
            );
            if bw == 0 || bh == 0 {
                continue;
            }
            let (y_dir, var) = if params.y_pri == 0 && params.uv_pri == 0 {
                (0, 0)
            } else {
                let block = core::array::from_fn(|i| {
                    core::array::from_fn(|j| {
                        let row = luma.row(y0 + i.min(bh - 1)).unwrap();
                        i32::from(row[x0 + j.min(bw - 1)].to_u16() >> coeff_shift) - 128
                    })
                });
                cdef_direction(&block)
            };
            let [y_filter, uv_filter] =
                cdef_strengths(params, coeff_shift, (lookup.sub_x, lookup.sub_y))
                    .filters(y_dir, var);
            let rows = tile_span(lookup.tile_row_starts, r, mi_rows);
            let cols = tile_span(lookup.tile_col_starts, c, mi_cols);
            for (index, snap) in snaps.iter().enumerate() {
                let Some(snap) = *snap else { continue };
                let (filter, filtered) = if index == 0 {
                    (y_filter, blocks.luma)
                } else {
                    (uv_filter, blocks.chroma)
                };
                if !filtered || (filter.pri_str == 0 && filter.sec_str == 0) {
                    continue;
                }
                let (sx, sy) = if index == 0 {
                    (0, 0)
                } else {
                    (lookup.sub_x, lookup.sub_y)
                };
                let start = ((cols.0 * MI_SIZE) >> sx, (rows.0 * MI_SIZE) >> sy);
                let end = (
                    ((cols.1 * MI_SIZE) >> sx).min(snap.width()),
                    ((rows.1 * MI_SIZE) >> sy).min(snap.frame_height()),
                );
                let offsets = CdefTapOffsets::for_direction(filter.dir);
                let (px, py) = (x0 >> sx, y0 >> sy);
                for y in py..(py + (8 >> sy)).min(snap.frame_height()) {
                    for x in px..(px + (8 >> sx)).min(snap.width()) {
                        let center = i32::from(snap.row(y).unwrap()[x].to_u16());
                        let taps = gather_taps(
                            snap, &offsets, x, y, start.0, start.1, end.0, end.1, center,
                        );
                        out[index][y * snap.width() + x] = cdef_filter_sample(
                            &taps,
                            filter.pri_str,
                            filter.sec_str,
                            filter.damping,
                            filter.coeff_shift,
                        ) as u16;
                    }
                }
            }
        }
    }
    out
}

/// Fills `ws` with noise of a spread that changes per 16x16 tile, flat tiles
/// included, so each kernel mode and the flat paths run.
fn seed_reference_noise<T: ReconSample>(ws: &mut CurrentFrameWorkspace<T>, max: u16, seed: u32) {
    let mut state = seed;
    let spreads = [0u16, 1, 3, 9, 40, max];
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let Ok(size) = ws
            .plane(plane)
            .map(splot_recon::CurrentFramePlane::storage_size)
        else {
            continue;
        };
        for y in 0..size.height() {
            for x in 0..size.width() {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let spread = spreads[(x / 16 * 7 + y / 16 * 3 + plane.index()) % spreads.len()];
                let base = (max / 2).saturating_sub(spread / 2);
                let noise = if spread == 0 {
                    0
                } else {
                    (state >> 16) as u16 % (spread + 1)
                };
                let value = T::try_from_u16((base + noise).min(max)).ok().unwrap();
                ws.set_reconstructed_sample(plane, x, y, value).unwrap();
            }
        }
    }
}

fn run_reference_case<T: ReconSample>(case: &ReferenceCase<'_>, bit_depth: BitDepth, seed: u32) {
    let (width, height) = case.size;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        case.format,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    let mut ws = CurrentFrameWorkspace::<T>::new(info, T::default()).unwrap();
    let max = ((1u32 << bit_depth.bits()) - 1) as u16;
    seed_reference_noise(&mut ws, max, seed);
    let (mi_rows, mi_cols) = (height.div_ceil(8) * 2, width.div_ceil(8) * 2);
    let units = (
        mi_rows.div_ceil(CDEF_UNIT_MI),
        mi_cols.div_ceil(CDEF_UNIT_MI),
    );
    let grid = CdefUnitGrid::new(
        units.0,
        units.1,
        (0..units.0 * units.1).map(case.unit_index).collect(),
    )
    .unwrap();
    let skip = case.skip.map(|skip| {
        let values = (0..mi_rows * mi_cols)
            .map(|cell| u8::from(skip(cell / mi_cols, cell % mi_cols)))
            .collect();
        CdefSkipGrid::new(mi_rows, mi_cols, values).unwrap()
    });
    let blocks = |luma: bool| {
        case.lossless
            .iter()
            .filter(|block| block.2 == luma)
            .map(|&(r, c, _)| deblock_block(r, c, 2, 2, true))
            .collect::<Vec<_>>()
    };
    let lossless = (!case.lossless.is_empty()).then(|| {
        crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
            mi_rows,
            mi_cols,
            &blocks(true),
            &blocks(false),
        )
        .unwrap()
    });
    let subsampling = (
        usize::from(case.format.subsampling_x()),
        usize::from(case.format.subsampling_y()),
    );
    let coeff_shift = u32::from(bit_depth.bits()) - 8;
    let lookup = CdefBlockLookup {
        strengths: case.strengths,
        grid: &grid,
        tile_row_starts: case.tile_starts.map(|(rows, _)| rows),
        tile_col_starts: case.tile_starts.map(|(_, cols)| cols),
        skip_grid: skip.as_ref(),
        lossless_grid: lossless.as_ref(),
        mi_rows,
        mi_cols,
        sub_x: subsampling.0,
        sub_y: subsampling.1,
        has_chroma: !case.format.is_monochrome(),
        coeff_shift,
        fill_flat: [true; 3],
        units: &cdef_units(case.strengths, coeff_shift, subsampling),
    };
    let want = reference_cdef(&ws, case, &lookup, coeff_shift);
    let input = [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| {
        ws.plane(plane).map_or_else(
            |_| Vec::new(),
            |_| {
                crate::test_support::workspace_rows(&ws, plane)
                    .into_iter()
                    .map(ReconSample::to_u16)
                    .collect()
            },
        )
    });
    assert!(
        (0..3).all(|plane| want[plane].is_empty() || want[plane] != input[plane]),
        "the reference filters every plane"
    );
    let storage = ws.plane(PlaneId::Y).unwrap().storage_size().height();
    let frame = crate::test_support::with_auto_backend!(backend => cdef_stripe(
        backend,
        DeblockedPlanes::frame(&ws).unwrap(),
        Some(case.strengths),
        Some(&grid),
        skip.as_ref(),
        lossless.as_ref(),
        (mi_rows, mi_cols),
        subsampling,
        bit_depth,
        case.tile_starts,
        0,
        storage,
    ))
    .unwrap();
    let got = [
        Some(&frame.filtered_y),
        frame.filtered_u.as_ref(),
        frame.filtered_v.as_ref(),
    ]
    .map(|plane| {
        plane.map_or_else(Vec::new, |plane| {
            crate::test_support::stripe_rows(plane)
                .into_iter()
                .map(ReconSample::to_u16)
                .collect()
        })
    });
    for plane in 0..3 {
        assert!(
            got[plane] == want[plane],
            "plane {plane} {:?} {:?} {bit_depth:?} seed {seed}: first diff at {:?}",
            case.format,
            case.size,
            got[plane]
                .iter()
                .zip(&want[plane])
                .position(|(got, want)| got != want)
        );
    }
}

/// The stripe filter matches § 7.18 sample by sample at every segment
/// position class: frame edges and corners, partial blocks past the frame,
/// tile edges, skipped, lossless and flat blocks, every subsampling and both
/// bit depths.
#[test]
fn stripe_filter_matches_the_spec_reference() {
    let set = |y_pri, y_sec, uv_pri, uv_sec, damping| CdefFrameParams {
        y_pri,
        y_sec,
        uv_pri,
        uv_sec,
        damping,
    };
    let strengths = [
        set(4, 2, 3, 1, 4),
        set(15, 4, 15, 4, 6),
        set(0, 1, 0, 0, 3),
        set(7, 0, 0, 2, 5),
        set(1, 1, 2, 4, 3),
        set(0, 0, 5, 0, 4),
    ];
    let lossless = [(2, 4, true), (6, 10, false), (8, 2, true), (8, 2, false)];
    let formats = [
        PixelFormat::Yuv420,
        PixelFormat::Yuv422,
        PixelFormat::Yuv444,
        PixelFormat::Monochrome,
    ];
    for (case_index, format) in formats.into_iter().enumerate() {
        for size in [(144, 72), (70, 38), (132, 18)] {
            let cases = [
                ReferenceCase {
                    format,
                    size,
                    strengths: &strengths,
                    unit_index: |unit| Some(unit % 6),
                    skip: None,
                    lossless: &[],
                    tile_starts: None,
                },
                ReferenceCase {
                    format,
                    size,
                    strengths: &strengths,
                    unit_index: |unit| (unit % 7 != 3).then_some((unit + 1) % 6),
                    skip: Some(|r, c| (r / 2 + c / 2) % 3 == 0 || (r == 5 && c == 9)),
                    lossless: &lossless,
                    tile_starts: None,
                },
                ReferenceCase {
                    format,
                    size,
                    strengths: &strengths[..2],
                    unit_index: |unit| Some(unit % 2),
                    skip: Some(|_, c| c / 2 % 2 == 1),
                    lossless: &[],
                    tile_starts: Some((&[0, 4, 1000], &[0, 16, 24, 1000])),
                },
            ];
            for (index, case) in cases.iter().enumerate() {
                let seed = (case_index * 10 + index) as u32;
                run_reference_case::<u8>(case, BitDepth::Eight, seed);
                run_reference_case::<u16>(case, BitDepth::Ten, seed);
            }
        }
    }
}
