// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::super::tests::frame_for;
use super::*;
use splot_recon::{BitDepth, DecodedFrame, PixelFormat};

const WIDTH: usize = 64;
const HEIGHT: usize = 32;

fn mv(row: i32, col: i32) -> Mv {
    Mv { row, col }
}

fn reference<T: ReconSample>(
    bit_depth: BitDepth,
    format: PixelFormat,
    seed: usize,
) -> DecodedFrame<T> {
    let max = usize::from(bit_depth.max_sample());
    let samples = |len: usize, salt: usize| -> Vec<T> {
        (0..len)
            .map(|i| {
                T::try_from_u16(((i * seed + salt * 7 + i / 5) % (max + 1)) as u16).expect("sample")
            })
            .collect()
    };
    let chroma =
        WIDTH.div_ceil(1 << format.subsampling_x()) * HEIGHT.div_ceil(1 << format.subsampling_y());
    frame_for(
        bit_depth,
        format,
        WIDTH,
        HEIGHT,
        samples(WIDTH * HEIGHT, 1),
        samples(chroma, 2),
        samples(chroma, 3),
    )
}

/// A TIP-shaped grid of 8x8 units over a `columns`-wide block.
fn grid(
    columns: usize,
    cells: &[[Mv; 2]],
    candidates: &[[Mv; 2]],
    merge: bool,
) -> CompoundMotionGrid {
    CompoundMotionGrid {
        unit_size: 8,
        columns,
        cells: MotionCells::Heap(
            cells
                .iter()
                .map(|&mvs| MotionCell::from_refinemv(mvs))
                .collect(),
        ),
        fullpel_runs: merge,
        refinemv_candidates: RefinemvCandidates::PerCell {
            candidates: candidates.to_vec(),
            unit_size: 8,
        },
    }
}

/// Predicts every plane of the block, with or without merged runs, and
/// returns the samples and the luma merge mask of every cell row.
fn predict<T: ReconSample + CompoundAverageOutput + Send>(
    references: &[DecodedFrame<T>; 2],
    rect: McBlockRect,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
) -> (Vec<u16>, Vec<u64>) {
    let info = references[0].info();
    let first = motion.cells.as_slice()[0].base_mvs;
    let block = InterBlockParams::compound_average(
        ReferenceSamples::settled(&references[0]),
        ReferenceSamples::settled(&references[1]),
        rect,
        first[0],
        first[1],
        InterpolationFilter::EightTap,
        CompoundBlend::default(),
    )
    .into_compound()
    .expect("compound block");
    let offset = ByteOffset::new(0);
    let mut samples = Vec::new();
    for (plane, sub_x, sub_y) in super::super::mc_planes(info.pixel_format()) {
        let (_, _, w, h) = rect.plane_rect(plane, sub_x, sub_y);
        let mut output = vec![T::default(); w * h];
        assert!(
            predict_motion_grid_compound_average_into(
                info,
                block,
                plane,
                sub_x,
                sub_y,
                motion,
                implicit_mask,
                CWP_EQUAL,
                offset,
                &mut output,
                w,
            )
            .expect("grid prediction")
        );
        samples.extend(output.iter().map(|sample| sample.to_u16()));
    }
    let prediction =
        super::super::compound_subpel_plane(info, block, PlaneId::Y, 0, 0, offset).expect("plane");
    let mut output = vec![T::default(); rect.luma_w * 8];
    let masks = (0..rect.luma_h / 8)
        .map(|cell_row| {
            predict_fullpel_runs(
                &prediction,
                motion.cells.as_slice(),
                motion.refinemv_candidate_slice(),
                cell_row * motion.columns,
                [cell_row * 8, 8, 8],
                (0, 0),
                (
                    info.bit_depth(),
                    InterpolationFilter::EightTap,
                    Some((8, 8)),
                ),
                (!implicit_mask, implicit_mask, CWP_EQUAL, (WIDTH, HEIGHT)),
                &mut [0; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
                &mut output,
                rect.luma_w,
            )
            .expect("runs")
        })
        .collect();
    (samples, masks)
}

/// Row 0 holds a three-cell still run, a two-cell shifted run and a partial
/// last cell; row 1 a shifted run that a changed candidate breaks, then
/// subpel and out-of-frame cells; row 2 runs that read past the right and
/// left edges of their still candidates' windows. The top-edge block's shifted
/// run reads above the frame, so the implicit mask keeps it per cell; the
/// left-edge block's row 1 run reads left of it.
fn merged_runs_match_per_cell<T: ReconSample + CompoundAverageOutput + Send>(
    bit_depth: BitDepth,
    format: PixelFormat,
) {
    let references = [
        reference::<T>(bit_depth, format, 13),
        reference::<T>(bit_depth, format, 29),
    ];
    let still = [mv(0, 0); 2];
    let shifted = [mv(16, -32), mv(-16, 32)];
    let subpel = [mv(3, -5), mv(-2, 7)];
    let outside = [mv(-64, 0), mv(64, 0)];
    let (right, left) = ([mv(0, 48); 2], [mv(0, -48); 2]);
    let cells = [
        still, still, still, shifted, shifted, shifted, //
        shifted, shifted, shifted, subpel, outside, outside, //
        right, right, left, left, still, still,
    ];
    let mut candidates = cells;
    candidates[8] = still;
    candidates[12..16].fill(still);
    let cases = [
        (
            McBlockRect::from_luma_rect(8, 8, 44, 24),
            [[0b1_1111, 0b11, 0]; 2],
        ),
        (
            McBlockRect::from_luma_rect(0, 4, 44, 24),
            [[0b1_1111, 0, 0]; 2],
        ),
        (
            McBlockRect::from_luma_rect(WIDTH - 44, 0, 44, 24),
            [[0b1_1111, 0b11, 0], [0b111, 0b11, 0]],
        ),
    ];
    for (rect, expected_masks) in cases {
        for (implicit_mask, expected_mask) in [false, true].into_iter().zip(expected_masks) {
            let (merged, mask) = predict(
                &references,
                rect,
                &grid(6, &cells, &candidates, true),
                implicit_mask,
            );
            let (per_cell, _) = predict(
                &references,
                rect,
                &grid(6, &cells, &candidates, false),
                implicit_mask,
            );
            assert_eq!(
                merged, per_cell,
                "{bit_depth:?} {format:?} {rect:?} mask {implicit_mask}"
            );
            assert_eq!(mask, expected_mask, "{bit_depth:?} {format:?} {rect:?}");
        }
    }
    let cells_of = |mvs: &[[Mv; 2]]| {
        mvs.iter()
            .map(|&mvs| MotionCell::from_refinemv(mvs))
            .collect::<Vec<_>>()
    };
    assert!(fullpel_run_pairs(&cells_of(&cells), 6));
    assert!(!fullpel_run_pairs(&cells_of(&[subpel, shifted]), 2));
}

#[test]
fn merged_fullpel_runs_match_per_cell_prediction() {
    merged_runs_match_per_cell::<u8>(BitDepth::Eight, PixelFormat::Yuv420);
    merged_runs_match_per_cell::<u16>(BitDepth::Ten, PixelFormat::Yuv420);
    merged_runs_match_per_cell::<u8>(BitDepth::Eight, PixelFormat::Yuv444);
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state >> 33
}

/// Two `width`-wide references: a noisy ramp, and the ramp with up to
/// `noise` added in the columns `noisy` selects.
fn noisy_ramps<T: ReconSample>(
    bit_depth: BitDepth,
    (width, height): (usize, usize),
    noise: u64,
    noisy: impl Fn(usize) -> bool,
    s: &mut u64,
) -> [DecodedFrame<T>; 2] {
    let max = u64::from(bit_depth.max_sample());
    let base: Vec<u64> = (0..width * height)
        .map(|i| ((i % width) as u64 * 5 + (i / width) as u64 * 3 + lcg(s) % 3) % (max + 1))
        .collect();
    let other = (0..base.len())
        .map(|i| base[i] + u64::from(noisy(i % width)) * (lcg(s) % (noise + 1)))
        .collect();
    [base, other].map(|luma| {
        let sample = |v: u64| T::try_from_u16(v.min(max) as u16).expect("sample");
        let chroma = vec![sample(64); (width / 2) * (height / 2)];
        frame_for(
            bit_depth,
            PixelFormat::Yuv420,
            width,
            height,
            luma.into_iter().map(sample).collect(),
            chroma.clone(),
            chroma,
        )
    })
}

/// The TIP grid of 8x8 units over the whole frame, and its per-unit cells.
fn tip_grid_and_cells<T: ReconSample>(
    references: &[DecodedFrame<T>; 2],
    mvs: &[[Mv; 2]],
    (threshold, refine, search): (Option<u32>, bool, bool),
    pool: Option<&splot_parallel::WorkerPool>,
) -> (CompoundMotionGrid, Vec<MotionCell>) {
    let info = references[0].info();
    let (width, height) = (
        info.coded_luma_size().width(),
        info.coded_luma_size().height(),
    );
    let columns = width / 8;
    let unit_rect = |i: usize| McBlockRect::from_luma_rect(i % columns * 8, i / columns * 8, 8, 8);
    let block = InterBlockParams::compound_average(
        ReferenceSamples::settled(&references[0]),
        ReferenceSamples::settled(&references[1]),
        unit_rect(0),
        mvs[0][0],
        mvs[0][1],
        InterpolationFilter::EightTap,
        CompoundBlend::default(),
    )
    .with_optflow_distances(Some([1, -1]))
    .with_optflow_sad_threshold(threshold)
    .with_refinemv(refine)
    .with_refinemv_search(search)
    .into_compound()
    .expect("compound block");
    let mut workspace = super::super::tests::workspace_for::<T>(
        info.bit_depth(),
        PixelFormat::Yuv420,
        width,
        height,
    );
    let sink = WorkspaceSink::Frame(&mut workspace);
    let offset = ByteOffset::new(0);
    let grid = || {
        let unit_at = |i: usize| (unit_rect(i), mvs[i]);
        tip_motion_grid(
            &sink,
            block,
            8,
            columns,
            mvs.len(),
            unit_at,
            offset,
            Vec::new(),
        )
        .expect("grid")
    };
    let grid = pool.map_or_else(grid, |pool| pool.install(grid));
    let cells = (0..mvs.len())
        .map(|i| {
            let mut unit = block;
            (unit.rect, unit.mv0, unit.mv1) = (unit_rect(i), mvs[i][0], mvs[i][1]);
            unit.has_chroma = false;
            super::super::refinemv::tip_refinemv_optflow_motion_cell(
                &sink,
                unit,
                offset,
                [false; 2],
                &mut [[0; 256]; 2],
            )
            .expect("refined cell")
            .map_or_else(|| tip_unit_motion_cell(&sink, unit, 8, offset), Ok)
            .expect("unit cell")
        })
        .collect();
    (grid, cells)
}

/// Checks a TIP grid against the per-unit cells, and its merged-run hint
/// against its cells.
fn check_tip_grid<T: ReconSample>(
    references: &[DecodedFrame<T>; 2],
    mvs: &[[Mv; 2]],
    modes: (Option<u32>, bool, bool),
    pool: Option<&splot_parallel::WorkerPool>,
    label: &str,
) {
    let bit_depth = references[0].info().bit_depth();
    let (grid, expected) = tip_grid_and_cells(references, mvs, modes, pool);
    assert_eq!(grid.cells.as_slice(), expected, "{bit_depth:?} {label}");
    let fullpel = mvs
        .iter()
        .any(|&mvs| super::super::refinemv::fullpel_candidates(mvs));
    let columns = references[0].info().coded_luma_size().width() / 8;
    assert_eq!(
        grid.fullpel_runs,
        fullpel && fullpel_run_pairs(grid.cells.as_slice(), columns),
        "{bit_depth:?} {label}"
    );
}

/// `count` units that repeat four mirrored candidates, mostly full-pel.
fn repeated_candidates(count: usize, s: &mut u64) -> Vec<[Mv; 2]> {
    let mut component = || (lcg(s) % 21) as i32 * 8 - 80 + i32::from(lcg(s).is_multiple_of(6)) * 3;
    let palette: Vec<[Mv; 2]> = (0..4)
        .map(|_| {
            let first = mv(component(), component());
            [first, mv(-first.row, -first.col)]
        })
        .collect();
    (0..count).map(|_| palette[(lcg(s) % 4) as usize]).collect()
}

/// Grids whose units repeat a few full-pel and subpel candidates, one of
/// 1024 units that takes the per-row parallel branch, and a still grid whose
/// interior units the full-pel pass decides between edge units that refine
/// on noise, so the unit after each run must not reuse the overlap of the
/// unit before it.
fn tip_grids_match_per_unit_cells<T: ReconSample>(bit_depth: BitDepth, mut s: u64) {
    let unit_count = WIDTH / 8 * (HEIGHT / 8);
    let edges =
        |s: &mut u64, size| noisy_ramps::<T>(bit_depth, size, 9, |x| x % 64 < 8 || x % 64 >= 56, s);
    let still = vec![[mv(0, 0); 2]; unit_count];
    let modes = (Some(6), true, true);
    check_tip_grid(
        &edges(&mut s, (WIDTH, HEIGHT)),
        &still,
        modes,
        None,
        "still",
    );
    let pool = splot_parallel::WorkerPool::new(splot_parallel::ThreadCount::Fixed(
        2.try_into().expect("two workers"),
    ))
    .expect("pool");
    let large = edges(&mut s, (256, 256));
    let still = vec![[mv(0, 0); 2]; 1024];
    check_tip_grid(&large, &still, modes, Some(&pool), "parallel still");
    let mvs = repeated_candidates(1024, &mut s);
    check_tip_grid(
        &large,
        &mvs,
        (Some(6), true, false),
        Some(&pool),
        "parallel",
    );
    for iter in 0..16 {
        let references = noisy_ramps::<T>(
            bit_depth,
            (WIDTH, HEIGHT),
            [0, 1, 3, 9][iter % 4],
            |_| true,
            &mut s,
        );
        let mvs = repeated_candidates(unit_count, &mut s);
        let modes = (
            [None, Some(6), Some(40)][iter % 3],
            iter % 4 != 3,
            iter % 2 == 0,
        );
        check_tip_grid(&references, &mvs, modes, None, &format!("grid {iter}"));
    }
}

#[test]
fn tip_motion_grids_match_per_unit_cells() {
    tip_grids_match_per_unit_cells::<u8>(BitDepth::Eight, 5);
    tip_grids_match_per_unit_cells::<u16>(BitDepth::Ten, 9);
}
