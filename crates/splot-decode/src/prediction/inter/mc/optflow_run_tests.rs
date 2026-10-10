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
        run_pairs: core::sync::atomic::AtomicU8::new(u8::from(!merge)),
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
    assert!(grid(6, &cells, &candidates, true).has_run_pairs());
    assert!(!grid(2, &[subpel, shifted], &[subpel, shifted], true).has_run_pairs());
}

#[test]
fn merged_fullpel_runs_match_per_cell_prediction() {
    merged_runs_match_per_cell::<u8>(BitDepth::Eight, PixelFormat::Yuv420);
    merged_runs_match_per_cell::<u16>(BitDepth::Ten, PixelFormat::Yuv420);
    merged_runs_match_per_cell::<u8>(BitDepth::Eight, PixelFormat::Yuv444);
}
