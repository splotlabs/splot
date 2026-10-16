// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::prediction::inter::mv_scaling::PlaneScaling;
use crate::prediction::inter::read_mv::{MV_LOW, MV_UPP};
use std::simd::{Simd, cmp::SimdOrd, num::SimdUint, simd_swizzle};

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
    let x_padding = if width == 4 { (1, 2) } else { (3, 4) };
    let y_padding = if height == 4 { (1, 2) } else { (3, 4) };
    if !scaling.is_scaled() {
        // The unscaled start is `((pos << 4) + mv_offset) << 6 | 32`, so its
        // integer sample is the 1/16 position shifted down by four.
        let first = |pos: i32, mv: i32, sub: u32| ((pos << 4) + ((2 * mv) >> sub)) >> 4;
        let (x, y) = (
            first(plane_x, candidate.col, sub_x),
            first(plane_y, candidate.row, sub_y),
        );
        let clamp = |value: i32, last: i32| value.max(0).min(last);
        return ReferenceAreaBounds {
            first_x: clamp(x - x_padding.0, scaling.last_x),
            first_y: clamp(y - y_padding.0, scaling.last_y),
            last_x: clamp(x + width as i32 - 1 + x_padding.1, scaling.last_x),
            last_y: clamp(y + height as i32 - 1 + y_padding.1, scaling.last_y),
        };
    }
    let scaling = scaling.with_mv(plane_x, plane_y, candidate.row, candidate.col, sub_x, sub_y);
    let last_x = scaling.start_x + scaling.step_x * width.saturating_sub(1) as i32;
    let last_y = scaling.start_y + scaling.step_y * height.saturating_sub(1) as i32;
    ReferenceAreaBounds {
        first_x: ((scaling.start_x >> 10) - x_padding.0).clamp(0, scaling.last_x),
        first_y: ((scaling.start_y >> 10) - y_padding.0).clamp(0, scaling.last_y),
        last_x: ((last_x >> 10) + x_padding.1).clamp(0, scaling.last_x),
        last_y: ((last_y >> 10) + y_padding.1).clamp(0, scaling.last_y),
    }
}

/// The `N`x`N` bilinear prediction (`N` 16 or 24) of a § 7.13.3.6 search
/// window whose clip bounds are its inner `N - 1` reference samples on each
/// axis, written at stride `N`, or `false` without writing for any other
/// block. The clamped outer column and row read their inner neighbour twice,
/// so every row is filtered horizontally once and serves two output rows.
pub(super) fn bilinear_search_window<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> bool {
    match params.w {
        16 => search_window::<T, 16, 2, 15>(view, params, output),
        24 => search_window::<T, 24, 3, 23>(view, params, output),
        _ => false,
    }
}

/// [`bilinear_search_window`] of an `N`-sample window: `G = N / 8` lane
/// groups per row over the `M = N - 1` inner samples.
#[inline(never)]
fn search_window<T: ReconSample, const N: usize, const G: usize, const M: usize>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> bool {
    let (x0, y0) = (params.start_x >> 10, params.start_y >> 10);
    let last = M as i32;
    if params.interp != InterpolationFilter::Bilinear
        || (params.step_x, params.step_y, params.w, params.h) != (1 << 10, 1 << 10, N, N)
        || [params.first_x, params.last_x, params.first_y, params.last_y]
            != [x0 + 1, x0 + last, y0 + 1, y0 + last]
        || x0 + 1 < 0
        || y0 + 1 < 0
    {
        return false;
    }
    let (Some(output), Some(spans)) = (
        output.get_mut(..N * N),
        view.readable_spans::<M>(
            core::array::from_fn(|row| (y0 + 1) as usize + row),
            (x0 + 1) as usize,
            M,
        ),
    ) else {
        return false;
    };
    let lanes = |samples: &[T]| -> Simd<u16, 8> {
        match (T::u8_slice(samples), T::u16_slice(samples)) {
            (Some(samples), _) => Simd::<u8, 8>::from_slice(samples).cast(),
            (_, Some(samples)) => Simd::from_slice(samples),
            _ => Simd::from_array(core::array::from_fn(|lane| samples[lane].to_u16())),
        }
    };
    let h = ((params.start_x >> 6) & 15) as u16;
    let v = ((params.start_y >> 6) & 15) as u16;
    let horizontal = |span: &[T]| -> [Simd<u16, 8>; G] {
        let (h0, h1) = (Simd::splat(16 - h), Simd::splat(h));
        let mut groups = [Simd::splat(0); G];
        for (group, filtered) in groups.iter_mut().enumerate() {
            let (left, right) = if group == 0 {
                let samples = lanes(&span[..8]);
                (simd_swizzle!(samples, [0, 0, 1, 2, 3, 4, 5, 6]), samples)
            } else if group == G - 1 {
                let samples = lanes(&span[8 * group - 1..]);
                (samples, simd_swizzle!(samples, [1, 2, 3, 4, 5, 6, 7, 7]))
            } else {
                (lanes(&span[8 * group - 1..]), lanes(&span[8 * group..]))
            };
            *filtered = left * h0 + right * h1;
        }
        groups
    };
    let max_sample = Simd::splat(params.bit_depth.max_sample());
    let mut write = |row: usize, top: [Simd<u16, 8>; G], bottom: [Simd<u16, 8>; G]| {
        for group in 0..G {
            // Both factors widened from u16 lanes lower to `umull`/`umlal`.
            let weight = |weight: u16| Simd::<u16, 8>::splat(weight).cast::<u32>();
            let sum =
                top[group].cast::<u32>() * weight(16 - v) + bottom[group].cast::<u32>() * weight(v);
            ((sum + Simd::splat(128)) >> 8)
                .cast::<u16>()
                .simd_min(max_sample)
                .copy_to_slice(&mut output[row * N + 8 * group..]);
        }
    };
    let mut top = horizontal(spans[0]);
    write(0, top, top);
    for (row, span) in spans.iter().enumerate().skip(1) {
        let bottom = horizontal(span);
        write(row, top, bottom);
        top = bottom;
    }
    write(N - 1, top, top);
    true
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
    let mut luma = super::optflow::LumaReference::pair(sink.info(), &block, offset);
    let mut predict = |center_only: bool, pred0: &mut [u16], pred1: &mut [u16]| {
        for ((luma, candidate), prediction) in luma.iter_mut().zip(candidates).zip([pred0, pred1]) {
            let search_mv = Mv {
                row: candidate.row - SEARCH_PADDING,
                col: candidate.col - SEARCH_PADDING,
            };
            let area = Some((candidate, rect.luma_w, rect.luma_h));
            let filter = InterpolationFilter::Bilinear;
            if center_only {
                super::optflow::initial_luma_prediction::<_, 2>(
                    luma,
                    prediction_rect,
                    search_mv,
                    filter,
                    area,
                    None,
                    prediction,
                )?;
            } else {
                super::optflow::initial_luma_prediction::<_, 0>(
                    luma,
                    prediction_rect,
                    search_mv,
                    filter,
                    area,
                    None,
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
    /// candidates. Only units whose SAD columns no bound clamps take this
    /// path. The SAD rows `Y - 2..=Y + 8` lie inside the refine window, so the
    /// copy clamps them to the plane alone, as in [`Self::unsearched_cell`].
    fn searched_cell(
        &self,
        sink: &WorkspaceSink<'_, '_, T>,
        batch: &CompoundMcBlock<'_, T>,
        rect: McBlockRect,
        mvs: [Mv; 2],
        distances: [i32; 2],
    ) -> Result<Option<MotionCell>> {
        let mut xs = [0usize; 2];
        let mut center_rows = [[0usize; 6]; 2];
        let mut rows = [[0usize; 8]; 2];
        for (reference, (x, y)) in origins(rect, mvs).into_iter().enumerate() {
            let (last_x, last_y) = self.last[reference];
            if x < 2 || x + 9 > last_x {
                return Ok(None);
            }
            xs[reference] = x as usize;
            let row = |offset: i32| (y + offset).clamp(0, last_y) as usize;
            center_rows[reference] = core::array::from_fn(|k| row(2 * k as i32 - 2));
            rows[reference] = core::array::from_fn(|k| row(k as i32));
        }
        let Some(center) = self
            .spans(xs.map(|x| x - 2), center_rows, 12)
            .and_then(|spans| self.sad(&spans))
        else {
            return Ok(None);
        };
        let center = center >> self.shift;
        if center - (center >> 3) >= 12 * 12 * 2 {
            return Ok(None);
        }
        self.optflow_cell(sink, batch, xs, rows, mvs, distances)
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
        let Some(spans) = self.spans(xs, rows, 8) else {
            return Ok(None);
        };
        let Some(sad) = self.sad(&spans) else {
            return Ok(None);
        };
        if batch
            .optflow_sad_threshold
            .is_some_and(|threshold| sad >> self.shift < threshold)
        {
            return Ok(Some(MotionCell::from_refinemv(mvs)));
        }
        let mut predictions = [[0u16; 64]; 2];
        for (prediction, spans) in predictions.iter_mut().zip(&spans) {
            for (output, samples) in prediction.chunks_exact_mut(8).zip(spans) {
                for (output, sample) in output.iter_mut().zip(*samples) {
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
        )?;
        Ok(Some(MotionCell::from_optflow(mvs, delta)))
    }

    /// The `width`-sample spans from columns `xs` of each reference's `rows`,
    /// or `None` when a row is not published.
    fn spans<const N: usize>(
        &self,
        xs: [usize; 2],
        rows: [[usize; N]; 2],
        width: usize,
    ) -> Option<[[&[T]; N]; 2]> {
        let [first, second] = [0, 1].map(|reference| {
            self.views[reference].readable_spans(rows[reference], xs[reference], width)
        });
        Some([first?, second?])
    }

    /// `Σ |ref0 - ref1|` over pairs of 8- or 12-sample spans.
    fn sad<const N: usize>(&self, spans: &[[&[T]; N]; 2]) -> Option<u32> {
        let mut sad8 = Simd::<u32, 8>::splat(0);
        let mut sad4 = Simd::<u32, 4>::splat(0);
        for (&left, &right) in spans[0].iter().zip(&spans[1]) {
            sad8 += self
                .lanes::<8>(left)?
                .abs_diff(self.lanes::<8>(right)?)
                .cast();
            if left.len() > 8 {
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

/// Refines one 8x8 TIP unit. The centre SAD and the centre optical flow read
/// only the 12x12 centre of the 16x16 bilinear search area, so the centre is
/// predicted first and the whole area only when the search goes on.
pub(super) fn tip_refinemv_optflow_motion_cell<'a, T: ReconSample>(
    block: CompoundMcBlock<'_, T>,
    predictions: &mut [[u16; TIP_PREDICTION_AREA]; 2],
    luma: &mut [super::optflow::LumaReference<'a, T>; 2],
    carries: &mut [splot_recon::BilinearOverlapCarry<'a, T>; 2],
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
    for (((luma, carry), candidate), prediction) in luma
        .iter_mut()
        .zip(carries.iter_mut())
        .zip(candidates)
        .zip([&mut *pred0, &mut *pred1])
    {
        super::optflow::initial_luma_prediction::<_, 2>(
            luma,
            prediction_rect,
            search_mv(candidate),
            InterpolationFilter::Bilinear,
            Some((candidate, CENTER_SIZE, CENTER_SIZE)),
            Some(carry),
            prediction,
        )?;
    }
    let bit_depth = luma[0].bit_depth();
    let center = tip_refinemv_sad(pred0, pred1, 0, 0, bit_depth, u32::MAX);
    let center_sad = center - (center >> 3);
    let (dx, dy) = if center_sad < 12 * 12 * 2 {
        (0, 0)
    } else {
        for ((luma, candidate), prediction) in luma
            .iter_mut()
            .zip(candidates)
            .zip([&mut *pred0, &mut *pred1])
        {
            super::optflow::initial_luma_prediction::<_, 0>(
                luma,
                prediction_rect,
                search_mv(candidate),
                InterpolationFilter::Bilinear,
                Some((candidate, CENTER_SIZE, CENTER_SIZE)),
                None,
                prediction,
            )?;
        }
        search_tip_refinemv_offset(pred0, pred1, center_sad, bit_depth)
    };
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
        bit_depth,
        distances,
        block.optflow_sad_threshold,
        base_mvs,
    )
    .map(Some)
}

/// The § 7.13.3.6 neighbour search of a unit whose biased centre SAD
/// `best_sad` did not keep the centre.
fn search_tip_refinemv_offset(
    pred0: &[u16; TIP_PREDICTION_AREA],
    pred1: &[u16; TIP_PREDICTION_AREA],
    mut best_sad: u32,
    bit_depth: splot_recon::BitDepth,
) -> (i32, i32) {
    let mut best = (0, 0);
    for &(dy, dx) in &SEARCH_NEIGHBORS {
        let sad = tip_refinemv_sad(pred0, pred1, dx, dy, bit_depth, best_sad);
        if sad < best_sad {
            best_sad = sad;
            best = (dx, dy);
        }
    }
    best
}

#[allow(clippy::inline_always, reason = "25 calls per refined TIP unit")]
#[inline(always)]
fn tip_refinemv_sad(
    pred0: &[u16; TIP_PREDICTION_AREA],
    pred1: &[u16; TIP_PREDICTION_AREA],
    dx: i32,
    dy: i32,
    bit_depth: splot_recon::BitDepth,
    limit: u32,
) -> u32 {
    const SPAN: usize = 10 * TIP_PREDICTION_SIZE + 12;
    let start = |dx: i32, dy: i32| ((2 + dy) * TIP_PREDICTION_SIZE as i32 + 2 + dx) as usize;
    let (Some(left), Some(right)) = (
        pred0
            .get(start(dx, dy)..)
            .and_then(<[u16]>::first_chunk::<SPAN>),
        pred1
            .get(start(-dx, -dy)..)
            .and_then(<[u16]>::first_chunk::<SPAN>),
    ) else {
        return u32::MAX;
    };
    // Four-lane widening abs-diff sums lower to one `uabal` each.
    let mut sums = [Simd::<u32, 4>::splat(0); 3];
    let shift = bit_depth.bits().saturating_sub(8);
    for row in (0..12).step_by(2) {
        for (column, sum) in sums.iter_mut().enumerate() {
            let at = row * TIP_PREDICTION_SIZE + 4 * column;
            *sum += Simd::<u16, 4>::from_slice(&left[at..])
                .abs_diff(Simd::from_slice(&right[at..]))
                .cast::<u32>();
        }
        // The remaining rows only add, so a partial sum at the limit already
        // loses the strict comparison.
        if row == 4 && (sums[0] + sums[1] + sums[2]).reduce_sum() >> shift >= limit {
            return limit;
        }
    }
    (sums[0] + sums[1] + sums[2]).reduce_sum() >> shift
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
fn refinemv_sad<'a>(
    pred0: &'a [u16],
    pred1: &'a [u16],
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
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    // One bounds check per predictor covers every compared row.
    let span = |pred: &'a [u16], start_y: usize, start_x: usize| {
        let last_row = height.saturating_sub(1) & !1;
        let start = start_y.checked_mul(stride)?.checked_add(start_x)?;
        let end = last_row.checked_mul(stride)?.checked_add(width)?;
        pred.get(start..start.checked_add(end)?)
    };
    let left = span(pred0, start0_y, start0_x).ok_or(ReconError::ArithmeticOverflow {
        context: "refine-MV SAD first lookup",
    })?;
    let right = span(pred1, start1_y, start1_x).ok_or(ReconError::ArithmeticOverflow {
        context: "refine-MV SAD second lookup",
    })?;
    let rows = left.chunks(2 * stride).zip(right.chunks(2 * stride));
    let sad = match width {
        12 => sad_rows::<12>(rows),
        20 => sad_rows::<20>(rows),
        _ => rows
            .map(
                |(left, right)| match (left.get(..width), right.get(..width)) {
                    (Some(left), Some(right)) => left
                        .iter()
                        .zip(right)
                        .map(|(&left, &right)| u32::from(left.abs_diff(right)))
                        .sum::<u32>(),
                    _ => u32::MAX,
                },
            )
            .fold(0, u32::saturating_add),
    };
    Ok(sad >> downshift)
}

/// `Σ |left - right|` over the first `W` samples of each row pair, in
/// four-lane widening steps that lower to one `uabal` each.
fn sad_rows<'a, const W: usize>(rows: impl Iterator<Item = (&'a [u16], &'a [u16])>) -> u32 {
    let mut sums = Simd::<u32, 4>::splat(0);
    for (left, right) in rows {
        let (Some(left), Some(right)) = (left.first_chunk::<W>(), right.first_chunk::<W>()) else {
            return u32::MAX;
        };
        for (left, right) in left.as_chunks::<4>().0.iter().zip(right.as_chunks::<4>().0) {
            sums += Simd::from_array(*left)
                .abs_diff(Simd::from_array(*right))
                .cast::<u32>();
        }
    }
    sums.reduce_sum()
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
