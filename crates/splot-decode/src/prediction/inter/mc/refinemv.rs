// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::clip3;

use super::*;
use crate::prediction::inter::mv_scaling::derive_plane_scaling;
use crate::prediction::inter::read_mv::{MV_LOW, MV_UPP};

const REFINEMV_UNIT_SIZE: usize = 16;
const SEARCH_PADDING: i32 = 4 * 8;
const SEARCH_NEIGHBORS: [(i32, i32); 24] = [
    (-2, -2),
    (-2, -1),
    (-2, 0),
    (-2, 1),
    (-2, 2),
    (-1, -2),
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (-1, 2),
    (0, -2),
    (0, -1),
    (0, 1),
    (0, 2),
    (1, -2),
    (1, -1),
    (1, 0),
    (1, 1),
    (1, 2),
    (2, -2),
    (2, -1),
    (2, 0),
    (2, 1),
    (2, 2),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ReferenceAreaBounds {
    pub(super) first_x: i64,
    pub(super) first_y: i64,
    pub(super) last_x: i64,
    pub(super) last_y: i64,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reference_area_bounds(
    plane_x: i64,
    plane_y: i64,
    width: usize,
    height: usize,
    candidate: Mv,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
) -> ReferenceAreaBounds {
    let scaling = derive_plane_scaling(
        plane_x,
        plane_y,
        i64::from(candidate.row),
        i64::from(candidate.col),
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        width as i64,
        height as i64,
    );
    let x_padding = if width == 4 { (1, 2) } else { (3, 4) };
    let y_padding = if height == 4 { (1, 2) } else { (3, 4) };
    ReferenceAreaBounds {
        first_x: clip3(0, scaling.last_x, (scaling.start_x >> 10) - x_padding.0),
        first_y: clip3(0, scaling.last_y, (scaling.start_y >> 10) - y_padding.0),
        last_x: clip3(
            0,
            scaling.last_x,
            ((scaling.start_x + scaling.step_x * (width.saturating_sub(1)) as i64) >> 10)
                + x_padding.1,
        ),
        last_y: clip3(
            0,
            scaling.last_y,
            ((scaling.start_y + scaling.step_y * (height.saturating_sub(1)) as i64) >> 10)
                + y_padding.1,
        ),
    }
}

pub(super) fn compound_default_refinemv_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    offset: ByteOffset,
) -> Result<CompoundMotionGrid> {
    let columns = block.rect.luma_w.div_ceil(REFINEMV_UNIT_SIZE);
    let rows = block.rect.luma_h.div_ceil(REFINEMV_UNIT_SIZE);
    let cell_count = columns
        .checked_mul(rows)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "refine-MV motion-grid size",
        })?;
    let candidates = [block.mv0, block.mv1];
    let motion_cell = |local_x: usize, local_y: usize| -> Result<MotionCell> {
        let width = (block.rect.luma_w - local_x).min(REFINEMV_UNIT_SIZE);
        let height = (block.rect.luma_h - local_y).min(REFINEMV_UNIT_SIZE);
        let mut rect = block.rect;
        rect.luma_x += local_x;
        rect.luma_y += local_y;
        rect.luma_w = width;
        rect.luma_h = height;
        let mvs = if block.search_refinemv {
            search_refinemv(sink, block, rect, offset)?
        } else {
            candidates
        };
        Ok(MotionCell::from_refinemv(mvs))
    };
    if cell_count == 1 {
        return Ok(CompoundMotionGrid::from_single_refinemv(
            candidates,
            motion_cell(0, 0)?,
        ));
    }
    let mut cells = Vec::with_capacity(cell_count);
    for local_y in (0..block.rect.luma_h).step_by(REFINEMV_UNIT_SIZE) {
        for local_x in (0..block.rect.luma_w).step_by(REFINEMV_UNIT_SIZE) {
            cells.push(motion_cell(local_x, local_y)?);
        }
    }
    Ok(CompoundMotionGrid::from_refinemv(
        columns, candidates, cells,
    ))
}

fn search_refinemv<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    rect: McBlockRect,
    offset: ByteOffset,
) -> Result<[Mv; 2]> {
    let candidates = [block.mv0, block.mv1];
    if !search_range_allowed(candidates) {
        return Ok(candidates);
    }
    let prediction_width = rect
        .luma_w
        .checked_add(8)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "refine-MV prediction width",
        })?;
    let prediction_height = rect
        .luma_h
        .checked_add(8)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "refine-MV prediction height",
        })?;
    let mut prediction_rect = rect;
    prediction_rect.luma_w = prediction_width;
    prediction_rect.luma_h = prediction_height;
    let search_mv = |candidate: Mv| Mv {
        row: candidate.row - SEARCH_PADDING,
        col: candidate.col - SEARCH_PADDING,
    };
    let (dx, dy) = super::with_initial_luma_predictions(
        prediction_width,
        prediction_height,
        |pred0, pred1| {
            super::optflow::initial_luma_prediction(
                sink,
                block.reference0,
                prediction_rect,
                search_mv(candidates[0]),
                InterpolationFilter::Bilinear,
                Some((candidates[0], rect.luma_w, rect.luma_h)),
                offset,
                pred0,
            )?;
            super::optflow::initial_luma_prediction(
                sink,
                block.reference1,
                prediction_rect,
                search_mv(candidates[1]),
                InterpolationFilter::Bilinear,
                Some((candidates[1], rect.luma_w, rect.luma_h)),
                offset,
                pred1,
            )?;
            Ok(search_refinemv_offset(
                pred0,
                pred1,
                prediction_width,
                rect.luma_w,
                rect.luma_h,
                sink.info().bit_depth(),
                !block.refinemv_switchable,
            )?)
        },
    )?;
    Ok([
        Mv {
            row: candidates[0].row + dy * 8,
            col: candidates[0].col + dx * 8,
        },
        Mv {
            row: candidates[1].row - dy * 8,
            col: candidates[1].col - dx * 8,
        },
    ])
}

fn search_range_allowed(candidates: [Mv; 2]) -> bool {
    candidates.into_iter().all(|mv| {
        [mv.row, mv.col].into_iter().all(|component| {
            i64::from(component) - i64::from(SEARCH_PADDING) >= i64::from(MV_LOW + 1)
                && i64::from(component) + 2 * 8 <= i64::from(MV_UPP - 1)
        })
    })
}

fn search_refinemv_offset(
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    width: usize,
    height: usize,
    bit_depth: splot_recon::BitDepth,
    allow_center: bool,
) -> splot_recon::Result<(i32, i32)> {
    let sad_width = width.checked_add(4).ok_or(ReconError::ArithmeticOverflow {
        context: "refine-MV SAD width",
    })?;
    let sad_height = height
        .checked_add(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "refine-MV SAD height",
        })?;
    // AV2 § 7.13.3.6: `allowCentre = tipPred || !is_switchable_refinemv()`.
    let (mut best, mut best_sad, first_unchecked_neighbor) = if allow_center {
        let threshold = sad_width
            .checked_mul(sad_height)
            .and_then(|area| area.checked_mul(2))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "refine-MV SAD threshold",
            })? as u64;
        let center = refinemv_sad(pred0, pred1, stride, sad_width, sad_height, 0, 0, bit_depth)?;
        let biased_center = center - (center >> 3);
        if biased_center < threshold {
            return Ok((0, 0));
        }
        ((0, 0), biased_center, 0)
    } else {
        let (dy, dx) = SEARCH_NEIGHBORS[0];
        let sad = refinemv_sad(
            pred0, pred1, stride, sad_width, sad_height, dx, dy, bit_depth,
        )?;
        ((dx, dy), sad, 1)
    };
    for &(dy, dx) in &SEARCH_NEIGHBORS[first_unchecked_neighbor..] {
        let sad = refinemv_sad(
            pred0, pred1, stride, sad_width, sad_height, dx, dy, bit_depth,
        )?;
        if sad < best_sad {
            best_sad = sad;
            best = (dx, dy);
        }
    }
    Ok(best)
}

#[allow(clippy::too_many_arguments)]
fn refinemv_sad(
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    width: usize,
    height: usize,
    dx: i32,
    dy: i32,
    bit_depth: splot_recon::BitDepth,
) -> splot_recon::Result<u64> {
    let start0_x = usize::try_from(2 + dx).map_err(|_| ReconError::ArithmeticOverflow {
        context: "refine-MV SAD left offset",
    })?;
    let start0_y = usize::try_from(2 + dy).map_err(|_| ReconError::ArithmeticOverflow {
        context: "refine-MV SAD top offset",
    })?;
    let start1_x = usize::try_from(2 - dx).map_err(|_| ReconError::ArithmeticOverflow {
        context: "refine-MV SAD right offset",
    })?;
    let start1_y = usize::try_from(2 - dy).map_err(|_| ReconError::ArithmeticOverflow {
        context: "refine-MV SAD bottom offset",
    })?;
    let mut sad = 0u64;
    for row in (0..height).step_by(2) {
        for col in 0..width {
            let index0 = (start0_y + row)
                .checked_mul(stride)
                .and_then(|row| row.checked_add(start0_x + col))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "refine-MV SAD first index",
                })?;
            let index1 = (start1_y + row)
                .checked_mul(stride)
                .and_then(|row| row.checked_add(start1_x + col))
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "refine-MV SAD second index",
                })?;
            let left = *pred0.get(index0).ok_or(ReconError::ArithmeticOverflow {
                context: "refine-MV SAD first lookup",
            })?;
            let right = *pred1.get(index1).ok_or(ReconError::ArithmeticOverflow {
                context: "refine-MV SAD second lookup",
            })?;
            sad += u64::from(left.abs_diff(right));
        }
    }
    Ok(sad >> u32::from(bit_depth.bits().saturating_sub(8)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn default_search_keeps_a_low_sad_centre() {
        let prediction = vec![80u16; 24 * 24];
        assert_eq!(
            search_refinemv_offset(
                &prediction,
                &prediction,
                24,
                16,
                16,
                splot_recon::BitDepth::Eight,
                true,
            )
            .expect("centre search"),
            (0, 0)
        );
    }

    #[test]
    fn default_search_selects_opposing_full_pixel_offsets() {
        let sample = |y: i32, x: i32| ((y * 37 + x * 19 + y * x * 3).rem_euclid(256)) as u16;
        let pred0: Vec<u16> = (0..24)
            .flat_map(|y| (0..24).map(move |x| sample(y, x)))
            .collect();
        let pred1: Vec<u16> = (0..24)
            .flat_map(|y| (0..24).map(move |x| sample(y - 2, x + 2)))
            .collect();
        assert_eq!(
            search_refinemv_offset(
                &pred0,
                &pred1,
                24,
                16,
                16,
                splot_recon::BitDepth::Eight,
                true,
            )
            .expect("offset search"),
            (1, -1)
        );
    }

    #[test]
    fn switchable_search_rejects_low_sad_center_and_starts_with_first_neighbor() {
        let prediction = vec![80u16; 24 * 24];
        assert_eq!(
            search_refinemv_offset(
                &prediction,
                &prediction,
                24,
                16,
                16,
                splot_recon::BitDepth::Eight,
                false,
            )
            .expect("center-disabled search"),
            (-2, -2)
        );
    }

    #[test]
    fn reference_area_uses_the_refinemv_extension() {
        assert_eq!(
            reference_area_bounds(16, 16, 16, 16, Mv::ZERO, 0, 0, 16, 16),
            ReferenceAreaBounds {
                first_x: 13,
                first_y: 13,
                last_x: 35,
                last_y: 35,
            }
        );
    }
}
