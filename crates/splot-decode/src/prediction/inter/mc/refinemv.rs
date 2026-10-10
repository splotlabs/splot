// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::prediction::inter::mv_scaling::PlaneScaling;
use crate::prediction::inter::read_mv::{MV_LOW, MV_UPP};
use std::simd::{Simd, cmp::SimdOrd, num::SimdUint};

const REFINEMV_UNIT_SIZE: usize = 16;
pub(super) const TIP_PREDICTION_SIZE: usize = 16;
pub(super) const TIP_PREDICTION_AREA: usize = TIP_PREDICTION_SIZE * TIP_PREDICTION_SIZE;
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
    pub(super) first_x: i32,
    pub(super) first_y: i32,
    pub(super) last_x: i32,
    pub(super) last_y: i32,
}

#[allow(clippy::too_many_arguments)]
#[inline]
pub(super) fn reference_area_bounds(
    plane_x: i32,
    plane_y: i32,
    width: usize,
    height: usize,
    candidate: Mv,
    sub_x: u32,
    sub_y: u32,
    scaling: &PlaneScaling,
) -> ReferenceAreaBounds {
    let scaling = scaling.with_mv(plane_x, plane_y, candidate.row, candidate.col, sub_x, sub_y);
    let x_padding = if width == 4 { (1, 2) } else { (3, 4) };
    let y_padding = if height == 4 { (1, 2) } else { (3, 4) };
    let last_x = scaling.start_x + scaling.step_x * width.saturating_sub(1) as i32;
    let last_y = scaling.start_y + scaling.step_y * height.saturating_sub(1) as i32;
    ReferenceAreaBounds {
        first_x: ((scaling.start_x >> 10) - x_padding.0).clamp(0, scaling.last_x),
        first_y: ((scaling.start_y >> 10) - y_padding.0).clamp(0, scaling.last_y),
        last_x: ((last_x >> 10) + x_padding.1).clamp(0, scaling.last_x),
        last_y: ((last_y >> 10) + y_padding.1).clamp(0, scaling.last_y),
    }
}

pub(super) fn compound_default_refinemv_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
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
    let mut cells =
        super::optflow::take_motion_cells(cell_count, MotionCell::from_refinemv(candidates));
    let mut index = 0usize;
    for local_y in (0..block.rect.luma_h).step_by(REFINEMV_UNIT_SIZE) {
        for local_x in (0..block.rect.luma_w).step_by(REFINEMV_UNIT_SIZE) {
            cells[index] = motion_cell(local_x, local_y)?;
            index += 1;
        }
    }
    Ok(CompoundMotionGrid::from_refinemv(
        columns, candidates, cells,
    ))
}

#[inline(never)]
fn search_refinemv<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
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
    let predict = |center_only: bool, pred0: &mut [u16], pred1: &mut [u16]| {
        let references = [block.reference0, block.reference1];
        for ((reference, candidate), prediction) in
            references.into_iter().zip(candidates).zip([pred0, pred1])
        {
            let search_mv = Mv {
                row: candidate.row - SEARCH_PADDING,
                col: candidate.col - SEARCH_PADDING,
            };
            let area = Some((candidate, rect.luma_w, rect.luma_h));
            let filter = InterpolationFilter::Bilinear;
            if center_only {
                super::optflow::initial_luma_prediction::<_, 2>(
                    sink,
                    reference,
                    prediction_rect,
                    search_mv,
                    filter,
                    area,
                    offset,
                    false,
                    prediction,
                )?;
            } else {
                super::optflow::initial_luma_prediction::<_, 0>(
                    sink,
                    reference,
                    prediction_rect,
                    search_mv,
                    filter,
                    area,
                    offset,
                    false,
                    prediction,
                )?;
            }
        }
        Ok::<_, crate::error::DecodeError>(())
    };
    let bit_depth = sink.info().bit_depth();
    // AV2 § 7.13.3.6: `allowCentre = tipPred || !is_switchable_refinemv()`.
    let allow_center = !block.refinemv_switchable;
    let (dx, dy) = super::with_initial_luma_predictions(
        prediction_width,
        prediction_height,
        |pred0, pred1| {
            if allow_center {
                predict(true, pred0, pred1)?;
                let (stride, width, height) = (prediction_width, rect.luma_w, rect.luma_h);
                if refinemv_center_sad(pred0, pred1, stride, width, height, bit_depth)?.is_none() {
                    return Ok((0, 0));
                }
            }
            predict(false, pred0, pred1)?;
            Ok(search_refinemv_offset(
                pred0,
                pred1,
                prediction_width,
                rect.luma_w,
                rect.luma_h,
                bit_depth,
                allow_center,
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

/// Writes the cell of every full-pel TIP unit from unit `first` on whose
/// SADs on the references decide it, and returns whether it wrote any. The
/// other cells keep their uninitialized value for the full path.
#[inline(never)]
pub(super) fn tip_fullpel_cells<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    batch: &CompoundMcBlock<'_, T>,
    unit_at: &impl Fn(usize) -> (McBlockRect, [Mv; 2]),
    candidates: &[[Mv; 2]],
    (unit_size, offset): (usize, ByteOffset),
    first: usize,
    cells: &mut [MotionCell],
) -> Result<bool> {
    let mut views = None;
    let mut wrote = false;
    if batch.optflow_distances.is_none() {
        return Ok(wrote);
    }
    let candidates = candidates.get(first..).unwrap_or_default();
    for (index, (cell, &mvs)) in cells.iter_mut().zip(candidates).enumerate() {
        if !fullpel_candidates(mvs) {
            continue;
        }
        let unit = (unit_at(first + index).0, mvs);
        let Some(views) =
            views.get_or_insert_with(|| TipFullpelViews::new(sink, batch, unit, offset))
        else {
            break;
        };
        if let Some(fast) = views.motion_cell(sink, batch, unit, unit_size)? {
            *cell = fast;
            wrote = true;
        }
    }
    Ok(wrote)
}

/// Both luma reference views of one TIP batch, for units whose candidates
/// are full-pel: their bilinear initial predictions are the clamped
/// reference samples, so the § 7.13.3.6 and optical-flow SADs read the
/// references in place.
struct TipFullpelViews<'a, T: ReconSample> {
    views: [ReferencePlaneView<'a, T>; 2],
    /// The last column and storage row each reference's bounds clamp to.
    last: [(i32, i32); 2],
    shift: u32,
    max_sample: u16,
}

pub(super) fn fullpel_candidates(mvs: [Mv; 2]) -> bool {
    mvs.iter().all(|mv| (mv.row | mv.col).trailing_zeros() >= 3)
}

/// The reference sample each full-pel candidate moves `rect`'s origin to.
fn origins(rect: McBlockRect, mvs: [Mv; 2]) -> [(i32, i32); 2] {
    mvs.map(|mv| {
        (
            rect.luma_x as i32 + (mv.col >> 3),
            rect.luma_y as i32 + (mv.row >> 3),
        )
    })
}

impl<'a, T: ReconSample> TipFullpelViews<'a, T> {
    /// The views of an optical-flow batch with unscaled references, built at
    /// its first full-pel unit (`rect`, `mvs`), or `None`. Each view is
    /// published down to the last row that unit's fast path reads; every
    /// later read is bounded by the published rows.
    fn new(
        sink: &WorkspaceSink<'_, '_, T>,
        batch: &CompoundMcBlock<'a, T>,
        (rect, mvs): (McBlockRect, [Mv; 2]),
        offset: ByteOffset,
    ) -> Option<Self> {
        let frame_size = sink.info().coded_luma_size();
        let references = [batch.reference0, batch.reference1];
        if batch.optflow_distances.is_none()
            || references
                .iter()
                .any(|reference| reference.info().coded_luma_size() != frame_size)
        {
            return None;
        }
        let reach = if batch.search_refinemv && search_range_allowed(mvs) {
            8
        } else {
            7
        };
        let view = |reference: usize| {
            let last_row = rect.luma_y as i32 + (mvs[reference].row >> 3) + reach;
            let (view, _, _) = references[reference]
                .plane_view(PlaneId::Y, last_row, offset)
                .ok()?;
            Some(view)
        };
        let views = [view(0)?, view(1)?];
        let last = core::array::from_fn(|reference| {
            let storage = references[reference].info().storage_luma_size();
            (
                storage.width().min(views[reference].width()) as i32 - 1,
                storage.height() as i32 - 1,
            )
        });
        let bit_depth = sink.info().bit_depth();
        Some(Self {
            views,
            last,
            shift: u32::from(bit_depth.bits().saturating_sub(8)),
            max_sample: bit_depth.max_sample(),
        })
    }

    /// The motion cell of an 8x8 full-pel unit when the SADs on the
    /// references decide it, or `None` to build the initial predictions.
    fn motion_cell(
        &self,
        sink: &WorkspaceSink<'_, '_, T>,
        batch: &CompoundMcBlock<'_, T>,
        (rect, mvs): (McBlockRect, [Mv; 2]),
        unit_size: usize,
    ) -> Result<Option<MotionCell>> {
        let Some(distances) = batch.optflow_distances else {
            return Ok(None);
        };
        if (unit_size, rect.luma_w, rect.luma_h) != (8, 8, 8) || !fullpel_candidates(mvs) {
            return Ok(None);
        }
        if batch.search_refinemv && search_range_allowed(mvs) {
            return self.searched_cell(sink, batch, rect, mvs, distances);
        }
        self.unsearched_cell(sink, batch, rect, mvs, distances)
    }

    /// [`tip_refinemv_optflow_motion_cell`] when the centre SAD keeps the
    /// candidates. Only interior units, whose SAD rows and columns no bound
    /// clamps, take this path, so the optical flow reads the references too.
    fn searched_cell(
        &self,
        sink: &WorkspaceSink<'_, '_, T>,
        batch: &CompoundMcBlock<'_, T>,
        rect: McBlockRect,
        mvs: [Mv; 2],
        distances: [i32; 2],
    ) -> Result<Option<MotionCell>> {
        let origins = origins(rect, mvs);
        let origin = |reference: usize| {
            let (x, y) = origins[reference];
            let (last_x, last_y) = self.last[reference];
            (x >= 2 && y >= 2 && x + 9 <= last_x && y + 8 <= last_y)
                .then_some((x as usize, y as usize))
        };
        let (Some((x0, y0)), Some((x1, y1))) = (origin(0), origin(1)) else {
            return Ok(None);
        };
        let Some(center) = self.sad(
            [x0 - 2, x1 - 2],
            (0..6).map(|row| [y0 - 2 + 2 * row, y1 - 2 + 2 * row]),
            12,
        ) else {
            return Ok(None);
        };
        let center = center >> self.shift;
        if center - (center >> 3) >= 12 * 12 * 2 {
            return Ok(None);
        }
        let rows = [y0, y1].map(|y| core::array::from_fn(|row| y + row));
        self.optflow_cell(sink, batch, [x0, x1], rows, mvs, distances)
    }

    /// [`super::optflow::tip_unit_motion_cell`] for an unsearched unit. Its
    /// rows `Y..Y + 8` lie inside the refine window `Y - 3..=Y + 11`, so the
    /// copy clamps them to the plane alone, with or without that window.
    /// Units whose columns would clamp take the full path.
    fn unsearched_cell(
        &self,
        sink: &WorkspaceSink<'_, '_, T>,
        batch: &CompoundMcBlock<'_, T>,
        rect: McBlockRect,
        mvs: [Mv; 2],
        distances: [i32; 2],
    ) -> Result<Option<MotionCell>> {
        let mut xs = [0usize; 2];
        let mut rows = [[0usize; 8]; 2];
        for (reference, (x, y)) in origins(rect, mvs).into_iter().enumerate() {
            let (last_x, last_y) = self.last[reference];
            if x < 0 || x + 7 > last_x {
                return Ok(None);
            }
            xs[reference] = x as usize;
            rows[reference] =
                core::array::from_fn(|row| (y + row as i32).clamp(0, last_y) as usize);
        }
        self.optflow_cell(sink, batch, xs, rows, mvs, distances)
    }

    /// The cell of a unit whose 8x8 predictions are the reference rows `rows`
    /// from columns `xs`: the candidates when the optical-flow SAD skips the
    /// refinement, else the § 7.13.3.9 delta derived from the same samples.
    fn optflow_cell(
        &self,
        sink: &WorkspaceSink<'_, '_, T>,
        batch: &CompoundMcBlock<'_, T>,
        xs: [usize; 2],
        rows: [[usize; 8]; 2],
        mvs: [Mv; 2],
        distances: [i32; 2],
    ) -> Result<Option<MotionCell>> {
        let Some(sad) = self.sad(xs, (0..8).map(|row| [rows[0][row], rows[1][row]]), 8) else {
            return Ok(None);
        };
        if batch
            .optflow_sad_threshold
            .is_some_and(|threshold| sad >> self.shift < threshold)
        {
            return Ok(Some(MotionCell::from_refinemv(mvs)));
        }
        let mut predictions = [[0u16; 64]; 2];
        for (reference, prediction) in predictions.iter_mut().enumerate() {
            for (output, &row) in prediction.chunks_exact_mut(8).zip(&rows[reference]) {
                let Some(samples) = self.views[reference]
                    .readable_row(row)
                    .and_then(|samples| samples.get(xs[reference]..xs[reference] + 8))
                else {
                    return Ok(None);
                };
                for (output, sample) in output.iter_mut().zip(samples) {
                    *output = sample.to_u16().min(self.max_sample);
                }
            }
        }
        let delta = splot_recon::derive_optflow_mv_delta_8x8_strided_into(
            &predictions[0],
            0,
            &predictions[1],
            0,
            8,
            sink.info().bit_depth(),
            distances,
            &mut splot_recon::OptflowScratch::default(),
        )?;
        Ok(Some(MotionCell::from_optflow(mvs, delta)))
    }

    /// `Σ |ref0 - ref1|` of `width` (8 or 12) samples from columns `xs` over
    /// each pair of rows, or `None` when a row is not published.
    fn sad(
        &self,
        xs: [usize; 2],
        rows: impl Iterator<Item = [usize; 2]>,
        width: usize,
    ) -> Option<u32> {
        let mut sad8 = Simd::<u32, 8>::splat(0);
        let mut sad4 = Simd::<u32, 4>::splat(0);
        for rows in rows {
            let [left, right] = [0, 1].map(|reference| {
                self.views[reference]
                    .readable_row(rows[reference])?
                    .get(xs[reference]..xs[reference] + width)
            });
            let (left, right) = (left?, right?);
            sad8 += self
                .lanes::<8>(left)?
                .abs_diff(self.lanes::<8>(right)?)
                .cast();
            if width > 8 {
                sad4 += self
                    .lanes::<4>(&left[8..])?
                    .abs_diff(self.lanes::<4>(&right[8..])?)
                    .cast();
            }
        }
        Some(sad8.reduce_sum() + sad4.reduce_sum())
    }

    /// The first `N` samples as the copy writes them: clipped to the maximum.
    fn lanes<const N: usize>(&self, samples: &[T]) -> Option<Simd<u16, N>> {
        let samples = samples.get(..N)?;
        Some(match (T::u8_slice(samples), T::u16_slice(samples)) {
            (Some(samples), _) => Simd::<u8, N>::from_slice(samples).cast(),
            (_, Some(samples)) => Simd::from_slice(samples).simd_min(Simd::splat(self.max_sample)),
            _ => Simd::from_array(core::array::from_fn(|index| {
                samples[index].to_u16().min(self.max_sample)
            })),
        })
    }
}

pub(super) fn tip_refinemv_optflow_motion_cell<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    offset: ByteOffset,
    reuse_horizontal: [bool; 2],
    predictions: &mut [[u16; TIP_PREDICTION_AREA]; 2],
) -> Result<Option<MotionCell>> {
    const PREDICTION_SIZE: usize = TIP_PREDICTION_SIZE;
    const CENTER_SIZE: usize = 8;

    let Some(distances) = block.optflow_distances else {
        return Ok(None);
    };
    let candidates = [block.mv0, block.mv1];
    if !block.search_refinemv
        || block.rect.luma_w != 8
        || block.rect.luma_h != 8
        || !search_range_allowed(candidates)
    {
        return Ok(None);
    }
    let mut prediction_rect = block.rect;
    prediction_rect.luma_w = PREDICTION_SIZE;
    prediction_rect.luma_h = PREDICTION_SIZE;
    let search_mv = |candidate: Mv| Mv {
        row: candidate.row - SEARCH_PADDING,
        col: candidate.col - SEARCH_PADDING,
    };
    let [pred0, pred1] = predictions;
    super::optflow::initial_luma_prediction::<_, 0>(
        sink,
        block.reference0,
        prediction_rect,
        search_mv(candidates[0]),
        InterpolationFilter::Bilinear,
        Some((candidates[0], CENTER_SIZE, CENTER_SIZE)),
        offset,
        reuse_horizontal[0],
        pred0,
    )?;
    super::optflow::initial_luma_prediction::<_, 0>(
        sink,
        block.reference1,
        prediction_rect,
        search_mv(candidates[1]),
        InterpolationFilter::Bilinear,
        Some((candidates[1], CENTER_SIZE, CENTER_SIZE)),
        offset,
        reuse_horizontal[1],
        pred1,
    )?;
    let (dx, dy) = search_tip_refinemv_offset(pred0, pred1, sink.info().bit_depth());
    let base_mvs = [
        Mv {
            row: candidates[0].row + dy * 8,
            col: candidates[0].col + dx * 8,
        },
        Mv {
            row: candidates[1].row - dy * 8,
            col: candidates[1].col - dx * 8,
        },
    ];
    let start0 = usize::try_from((4 + dy) * PREDICTION_SIZE as i32 + 4 + dx).map_err(|_| {
        ReconError::ArithmeticOverflow {
            context: "TIP optical-flow predictor 0 offset",
        }
    })?;
    let start1 = usize::try_from((4 - dy) * PREDICTION_SIZE as i32 + 4 - dx).map_err(|_| {
        ReconError::ArithmeticOverflow {
            context: "TIP optical-flow predictor 1 offset",
        }
    })?;
    super::optflow::tip_optflow_motion_cell_strided(
        pred0,
        start0,
        pred1,
        start1,
        PREDICTION_SIZE,
        sink.info().bit_depth(),
        distances,
        block.optflow_sad_threshold,
        base_mvs,
    )
    .map(Some)
}

fn search_tip_refinemv_offset(
    pred0: &[u16; TIP_PREDICTION_AREA],
    pred1: &[u16; TIP_PREDICTION_AREA],
    bit_depth: splot_recon::BitDepth,
) -> (i32, i32) {
    let mut best = (0, 0);
    let center = tip_refinemv_sad(pred0, pred1, 0, 0, bit_depth);
    let mut best_sad = center - (center >> 3);
    if best_sad < 12 * 12 * 2 {
        return best;
    }
    for &(dy, dx) in &SEARCH_NEIGHBORS {
        let sad = tip_refinemv_sad(pred0, pred1, dx, dy, bit_depth);
        if sad < best_sad {
            best_sad = sad;
            best = (dx, dy);
        }
    }
    best
}

fn tip_refinemv_sad(
    pred0: &[u16; TIP_PREDICTION_AREA],
    pred1: &[u16; TIP_PREDICTION_AREA],
    dx: i32,
    dy: i32,
    bit_depth: splot_recon::BitDepth,
) -> u32 {
    let start0 = ((2 + dy) * TIP_PREDICTION_SIZE as i32 + 2 + dx) as usize;
    let start1 = ((2 - dy) * TIP_PREDICTION_SIZE as i32 + 2 - dx) as usize;
    let mut sad8 = Simd::<u32, 8>::splat(0);
    let mut sad4 = Simd::<u32, 4>::splat(0);
    for row in (0..12).step_by(2) {
        let left = &pred0[start0 + row * TIP_PREDICTION_SIZE..];
        let right = &pred1[start1 + row * TIP_PREDICTION_SIZE..];
        let left8 = Simd::<u16, 8>::from_slice(left);
        let right8 = Simd::<u16, 8>::from_slice(right);
        sad8 += (left8.simd_max(right8) - left8.simd_min(right8)).cast::<u32>();
        let left4 = Simd::<u16, 4>::from_slice(&left[8..]);
        let right4 = Simd::<u16, 4>::from_slice(&right[8..]);
        sad4 += (left4.simd_max(right4) - left4.simd_min(right4)).cast::<u32>();
    }
    (sad8.reduce_sum() + sad4.reduce_sum()) >> bit_depth.bits().saturating_sub(8)
}

fn search_range_allowed(candidates: [Mv; 2]) -> bool {
    candidates.into_iter().all(|mv| {
        [mv.row, mv.col].into_iter().all(|component| {
            (MV_LOW + 1 + SEARCH_PADDING..=MV_UPP - 1 - 2 * 8).contains(&component)
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
    let (sad_width, sad_height) = sad_extent(width, height)?;
    let (mut best, mut best_sad, first_unchecked_neighbor) = if allow_center {
        let Some(biased_center) =
            refinemv_center_sad(pred0, pred1, stride, width, height, bit_depth)?
        else {
            return Ok((0, 0));
        };
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

/// The `(width + 4) x (height + 4)` area each § 7.13.3.6 SAD compares.
fn sad_extent(width: usize, height: usize) -> splot_recon::Result<(usize, usize)> {
    let sad_width = width.checked_add(4).ok_or(ReconError::ArithmeticOverflow {
        context: "refine-MV SAD width",
    })?;
    let sad_height = height
        .checked_add(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "refine-MV SAD height",
        })?;
    Ok((sad_width, sad_height))
}

/// The biased § 7.13.3.6 centre SAD, or `None` when it keeps the
/// candidates. It reads only the predictions' centre area, two samples in
/// from each edge.
fn refinemv_center_sad(
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    width: usize,
    height: usize,
    bit_depth: splot_recon::BitDepth,
) -> splot_recon::Result<Option<u32>> {
    let (sad_width, sad_height) = sad_extent(width, height)?;
    let threshold = sad_width
        .checked_mul(sad_height)
        .and_then(|area| area.checked_mul(2))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "refine-MV SAD threshold",
        })? as u32;
    let center = refinemv_sad(pred0, pred1, stride, sad_width, sad_height, 0, 0, bit_depth)?;
    let biased_center = center - (center >> 3);
    Ok((biased_center >= threshold).then_some(biased_center))
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
) -> splot_recon::Result<u32> {
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
    let mut sad8 = Simd::<u32, 8>::splat(0);
    let mut sad4 = Simd::<u32, 4>::splat(0);
    let mut sad_tail = 0u32;
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    for row in (0..height).step_by(2) {
        let row_range = |start_y: usize, start_x: usize| {
            (start_y + row)
                .checked_mul(stride)
                .and_then(|row| row.checked_add(start_x))
                .and_then(|start| start.checked_add(width).map(|end| start..end))
        };
        let left = row_range(start0_y, start0_x)
            .and_then(|range| pred0.get(range))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "refine-MV SAD first lookup",
            })?;
        let right = row_range(start1_y, start1_x)
            .and_then(|range| pred1.get(range))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "refine-MV SAD second lookup",
            })?;
        let mut index = 0;
        while index + 8 <= width {
            let left = Simd::<u16, 8>::from_slice(&left[index..]);
            let right = Simd::<u16, 8>::from_slice(&right[index..]);
            sad8 += (left.simd_max(right) - left.simd_min(right)).cast::<u32>();
            index += 8;
        }
        if index + 4 <= width {
            let left = Simd::<u16, 4>::from_slice(&left[index..]);
            let right = Simd::<u16, 4>::from_slice(&right[index..]);
            sad4 += (left.simd_max(right) - left.simd_min(right)).cast::<u32>();
            index += 4;
        }
        for (&left, &right) in left[index..].iter().zip(&right[index..]) {
            sad_tail += u32::from(left.abs_diff(right));
        }
    }
    let sad = (sad8.reduce_sum() + sad4.reduce_sum() + sad_tail) >> downshift;
    Ok(sad)
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
        let scaling = crate::prediction::inter::mv_scaling::derive_plane_scaling(
            16, 16, 0, 0, 0, 0, 64, 64, 64, 64,
        );
        assert_eq!(
            reference_area_bounds(16, 16, 16, 16, Mv::ZERO, 0, 0, &scaling),
            ReferenceAreaBounds {
                first_x: 13,
                first_y: 13,
                last_x: 35,
                last_y: 35,
            }
        );
    }
}

#[cfg(test)]
#[path = "tip_fullpel_tests.rs"]
mod tip_fullpel_tests;
