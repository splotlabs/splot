// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::prediction::inter::mv_scaling::PlaneScaling;
use crate::prediction::inter::read_mv::{MV_LOW, MV_UPP};
use fearless_simd::{
    Bytes, Simd, SimdBase, SimdIntWiden, SimdNarrow, SimdWiden, u8x16, u16x8, u32x4, u64x2,
};

const REFINEMV_UNIT_SIZE: usize = 16;
pub(super) const TIP_PREDICTION_SIZE: usize = 16;
pub(super) const TIP_PREDICTION_AREA: usize = TIP_PREDICTION_SIZE * TIP_PREDICTION_SIZE;

/// The 16x16 TIP refine-MV search areas of both references, kept by the
/// caller so that no unit zeroes them: each unit writes what it reads.
#[derive(Debug)]
pub(crate) struct TipSearchScratch {
    predictions: [[u16; TIP_PREDICTION_AREA]; 2],
    windows: [[u8; TIP_PREDICTION_AREA]; 2],
}

impl Default for TipSearchScratch {
    fn default() -> Self {
        Self {
            predictions: [[0; TIP_PREDICTION_AREA]; 2],
            windows: [[0; TIP_PREDICTION_AREA]; 2],
        }
    }
}
pub(super) const SEARCH_PADDING: i32 = 4 * 8;
use splot_recon::REFINEMV_NEIGHBORS as SEARCH_NEIGHBORS;

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
    if !scaling.is_scaled() {
        return unscaled_area_bounds(
            [plane_x << 4, plane_y << 4],
            [width, height],
            candidate,
            [sub_x, sub_y],
            scaling,
        );
    }
    let x_padding = if width == 4 { (1, 2) } else { (3, 4) };
    let y_padding = if height == 4 { (1, 2) } else { (3, 4) };
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

/// [`reference_area_bounds`] of an unscaled reference, from the area's
/// 1/16-sample origin: the unscaled start is `((pos << 4) + mv_offset) << 6
/// | 32`, so its integer sample is the 1/16 position shifted down by four.
#[allow(clippy::inline_always, reason = "called per unit and grid cell")]
#[inline(always)]
pub(super) fn unscaled_area_bounds(
    [x16, y16]: [i32; 2],
    [width, height]: [usize; 2],
    candidate: Mv,
    [sub_x, sub_y]: [u32; 2],
    scaling: &PlaneScaling,
) -> ReferenceAreaBounds {
    let padding = |size: usize| if size == 4 { (1, 2) } else { (3, 4) };
    let (x_padding, y_padding) = (padding(width), padding(height));
    let x = (x16 + ((2 * candidate.col) >> sub_x)) >> 4;
    let y = (y16 + ((2 * candidate.row) >> sub_y)) >> 4;
    let clamp = |value: i32, last: i32| value.max(0).min(last);
    ReferenceAreaBounds {
        first_x: clamp(x - x_padding.0, scaling.last_x),
        first_y: clamp(y - y_padding.0, scaling.last_y),
        last_x: clamp(x + width as i32 - 1 + x_padding.1, scaling.last_x),
        last_y: clamp(y + height as i32 - 1 + y_padding.1, scaling.last_y),
    }
}

/// A refine-MV bilinear prediction that a one-pass kernel covers, written
/// at stride `params.w`, or `false` without writing for any other block: a
/// block of whole eight-column groups with no clamped tap (eight-bit, or
/// 8x8), or the `N`x`N` § 7.13.3.6 search window
/// (`N` 16 or 24) whose clip bounds are its inner `N - 1` samples on each
/// axis, where the clamped outer column and row read their inner neighbour
/// twice. Every source row is filtered horizontally once and serves two
/// output rows.
pub(super) fn bilinear_search_window<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> bool {
    let window = match params.w {
        16 => search_window::<T, 16, 2, 15>(view, params, output),
        24 => search_window::<T, 24, 3, 23>(view, params, output),
        _ => false,
    };
    window
        || splot_recon::subpel_predict_bilinear_u8(view, params, output, params.w)
        || (params.w == 8 && unclamped_8x8(view, params, output))
}

/// An 8x8 bilinear block whose taps all lie inside its clip bounds and the
/// published plane: one horizontal pass per source row, shared by two
/// output rows.
#[inline(never)]
fn unclamped_8x8<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> bool {
    let (x0, y0) = (params.start_x >> 10, params.start_y >> 10);
    if params.interp != InterpolationFilter::Bilinear
        || (params.step_x, params.step_y, params.h) != (1 << 10, 1 << 10, 8)
        || x0 < params.first_x.max(0)
        || y0 < params.first_y.max(0)
        || x0 + 8 > params.last_x
        || y0 + 8 > params.last_y
    {
        return false;
    }
    let (Some(output), Some(spans)) = (
        output.first_chunk_mut::<64>(),
        view.readable_spans::<9>(
            core::array::from_fn(|row| y0 as usize + row),
            x0 as usize,
            9,
        ),
    ) else {
        return false;
    };
    let h = ((params.start_x >> 6) & 15) as u16;
    let v = ((params.start_y >> 6) & 15) as u16;
    let max_sample = params.bit_depth.max_sample();
    baseline_simd!(simd => {
        let horizontal = |span: &[T]| {
            let lanes = |at: usize| sample_lanes(simd, span, at);
            lanes(0) * (16 - h) + lanes(1) * h
        };
        let mut top = horizontal(spans[0]);
        for (row, span) in spans.iter().skip(1).enumerate() {
            let bottom = horizontal(span);
            if let Some(slot) = output.get_mut(row * 8..).and_then(<[u16]>::first_chunk_mut::<8>) {
                vertical_blend(simd, top, bottom, v, max_sample).store_array(slot);
            }
            top = bottom;
        }
    });
    true
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
    let h = ((params.start_x >> 6) & 15) as u16;
    let v = ((params.start_y >> 6) & 15) as u16;
    let max_sample = params.bit_depth.max_sample();
    baseline_simd!(simd => {
        let horizontal = |span: &[T; M]| -> [u16x8<_>; G] {
            let mut lanes = [u16x8::splat(simd, 0); G];
            for (group, lanes) in lanes.iter_mut().enumerate() {
                let (left, right) = if group == 0 {
                    let samples = sample_lanes(simd, span, 0);
                    (u16x8::splat(simd, samples[0]).slide::<7>(samples), samples)
                } else if group == G - 1 {
                    let samples = sample_lanes(simd, span, 8 * group - 1);
                    (samples, samples.slide::<1>(u16x8::splat(simd, samples[7])))
                } else {
                    (sample_lanes(simd, span, 8 * group - 1), sample_lanes(simd, span, 8 * group))
                };
                *lanes = left * (16 - h) + right * h;
            }
            lanes
        };
        let mut write = |row: usize, top: [u16x8<_>; G], bottom: [u16x8<_>; G]| {
            for group in 0..G {
                if let Some(slot) =
                    output.get_mut(row * N + 8 * group..).and_then(<[u16]>::first_chunk_mut::<8>)
                {
                    vertical_blend(simd, top[group], bottom[group], v, max_sample).store_array(slot);
                }
            }
        };
        let mut upper = None;
        for row in 0..N {
            let lower = match spans.get(row) {
                Some(span) => match span.first_chunk::<M>() {
                    Some(span) => horizontal(span),
                    None => return false,
                },
                None => match upper {
                    Some(upper) => upper,
                    None => return false,
                },
            };
            write(row, upper.unwrap_or(lower), lower);
            upper = Some(lower);
        }
        true
    })
}

/// Eight samples from `at` widened to `u16`, zero past the span.
#[inline]
fn sample_lanes<S: Simd, T: ReconSample>(simd: S, span: &[T], at: usize) -> u16x8<S> {
    if let Some(samples) = T::u16_slice(span).and_then(|span| span.get(at..)?.first_chunk::<8>()) {
        return u16x8::load_array_ref(simd, samples);
    }
    if let Some(samples) = T::u8_slice(span).and_then(|span| span.get(at..)?.first_chunk::<8>()) {
        return u16x8::load_array(simd, samples.map(u16::from));
    }
    u16x8::from_fn(simd, |lane| {
        span.get(at + lane).map_or(0, |sample| sample.to_u16())
    })
}

/// `Min(Round2(top * (16 - v) + bottom * v, 8), max)` of two horizontal rows.
#[inline]
fn vertical_blend<S: Simd>(simd: S, top: u16x8<S>, bottom: u16x8<S>, v: u16, max: u16) -> u16x8<S> {
    let (top_low, top_high) = top.widening_mul(u16x8::splat(simd, 16 - v));
    let (bottom_low, bottom_high) = bottom.widening_mul(u16x8::splat(simd, v));
    let blend = |top: u32x4<S>, bottom: u32x4<S>| (top + bottom + 128) >> 8;
    blend(top_low, bottom_low)
        .narrow(blend(top_high, bottom_high))
        .min(u16x8::splat(simd, max))
}

/// The bilinear `(w + 8) x (h + 8)` § 7.13.3.6 search areas of one refine-MV
/// unit of at most 16x16, for both references, as bytes or as samples.
pub(super) type SearchBytes = [[u8; SEARCH_AREA]; 2];
pub(super) type SearchSamples = [[u16; SEARCH_AREA]; 2];
pub(super) const SEARCH_AREA: usize = (REFINEMV_UNIT_SIZE + 8) * (REFINEMV_UNIT_SIZE + 8);

/// Which search area of [`search_unit`] holds the bilinear predictions of
/// the unit's `(w + 8) x (h + 8)` area, at least its centre `w x h` samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SearchArea {
    None,
    Bytes,
    Samples,
}

#[inline(never)]
pub(super) fn search_refinemv<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    rect: McBlockRect,
    offset: ByteOffset,
) -> Result<[Mv; 2]> {
    let candidates = [block.mv0, block.mv1];
    if !search_range_allowed(candidates) {
        return Ok(candidates);
    }
    let mut luma = super::optflow::LumaReference::pair(sink.info(), block, offset);
    let prediction_rect = search_rect(rect);
    // AV2 § 7.13.3.6: `allowCentre = tipPred || !is_switchable_refinemv()`.
    let allow_center = !block.refinemv_switchable;
    if T::u8_slice(&[]).is_some() {
        let mut windows = [[0u8; SEARCH_AREA]; 2];
        let search = (prediction_rect, candidates, allow_center);
        if let Some(mvs) = search_bytes(&mut luma, search, &mut windows)?.0 {
            return Ok(mvs);
        }
    }
    let (width, height) = (prediction_rect.luma_w, prediction_rect.luma_h);
    let (dx, dy) = super::with_initial_luma_predictions(width, height, |pred0, pred1| {
        search_samples(&mut luma, rect, candidates, allow_center, [pred0, pred1])
    })?;
    Ok(refined_mvs(candidates, dx, dy))
}

/// The § 7.13.3.6 search of the refine-MV unit `rect` (at most 16x16) from
/// `candidates`: the refined motion vectors and which of `bytes` and
/// `samples` holds the search area they were chosen from.
pub(super) fn search_unit<T: ReconSample>(
    luma: &mut [super::optflow::LumaReference<'_, T>; 2],
    (rect, candidates, allow_center): (McBlockRect, [Mv; 2], bool),
    bytes: &mut SearchBytes,
    samples: &mut SearchSamples,
) -> Result<([Mv; 2], SearchArea)> {
    if !search_range_allowed(candidates) {
        return Ok((candidates, SearchArea::None));
    }
    let prediction_rect = search_rect(rect);
    if T::u8_slice(&[]).is_some()
        && let (Some(mvs), area) =
            search_bytes(luma, (prediction_rect, candidates, allow_center), bytes)?
    {
        return Ok((mvs, area));
    }
    let len = prediction_rect.luma_w * prediction_rect.luma_h;
    let [first, second] = samples;
    let (Some(pred0), Some(pred1)) = (first.get_mut(..len), second.get_mut(..len)) else {
        return Err(ReconError::SubpelBlockDimensionUnsupported {
            w: rect.luma_w,
            h: rect.luma_h,
        }
        .into());
    };
    let (dx, dy) = search_samples(luma, rect, candidates, allow_center, [pred0, pred1])?;
    Ok((refined_mvs(candidates, dx, dy), SearchArea::Samples))
}

/// The `(w + 8) x (h + 8)` search area of the refine-MV unit `rect`.
const fn search_rect(rect: McBlockRect) -> McBlockRect {
    let mut prediction_rect = rect;
    prediction_rect.luma_w = rect.luma_w + 8;
    prediction_rect.luma_h = rect.luma_h + 8;
    prediction_rect
}

/// The eight-bit search: the candidates when both are full-pel and the
/// centre SAD read from the references keeps them, else the search of the
/// byte windows; `None` when the byte kernels do not cover the unit.
fn search_bytes<T: ReconSample>(
    luma: &mut [super::optflow::LumaReference<'_, T>; 2],
    (prediction_rect, candidates, allow_center): (McBlockRect, [Mv; 2], bool),
    windows: &mut SearchBytes,
) -> Result<(Option<[Mv; 2]>, SearchArea)> {
    if allow_center
        && super::optflow::fullpel_center_ends_search(luma, prediction_rect, candidates)?
            == Some(true)
    {
        return Ok((Some(candidates), SearchArea::None));
    }
    if super::optflow::search_windows_u8(luma, prediction_rect, candidates, windows)?
        && let Some((dx, dy)) = splot_recon::refinemv_search_u8(
            [&windows[0], &windows[1]],
            prediction_rect.luma_w,
            prediction_rect.luma_h - 4,
            allow_center,
        )
    {
        return Ok((Some(refined_mvs(candidates, dx, dy)), SearchArea::Bytes));
    }
    Ok((None, SearchArea::None))
}

/// The search offset from sample predictions of the search area: the centre
/// alone first when it may keep the candidates, else the whole area.
fn search_samples<T: ReconSample>(
    luma: &mut [super::optflow::LumaReference<'_, T>; 2],
    rect: McBlockRect,
    candidates: [Mv; 2],
    allow_center: bool,
    [pred0, pred1]: [&mut [u16]; 2],
) -> Result<(i32, i32)> {
    let prediction_rect = search_rect(rect);
    let bit_depth = luma[0].bit_depth();
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
    let stride = prediction_rect.luma_w;
    if allow_center {
        predict(true, pred0, pred1)?;
        if refinemv_center_sad(pred0, pred1, stride, rect.luma_w, rect.luma_h, bit_depth)?.is_none()
        {
            return Ok((0, 0));
        }
    }
    predict(false, pred0, pred1)?;
    let sad_h = prediction_rect.luma_h - 4;
    splot_recon::refinemv_search_u16([pred0, pred1], stride, sad_h, allow_center, bit_depth)
        .ok_or_else(|| {
            ReconError::SubpelBlockDimensionUnsupported {
                w: rect.luma_w + 4,
                h: rect.luma_h + 4,
            }
            .into()
        })
}

/// The candidates moved by the full-sample search offset `(dx, dy)`, in
/// opposite directions.
fn refined_mvs(candidates: [Mv; 2], dx: i32, dy: i32) -> [Mv; 2] {
    [
        Mv {
            row: candidates[0].row + dy * 8,
            col: candidates[0].col + dx * 8,
        },
        Mv {
            row: candidates[1].row - dy * 8,
            col: candidates[1].col - dx * 8,
        },
    ]
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
            let view = references[reference]
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
        let rows = spans.map(|spans| {
            let mut rows = [0u64; 8];
            for (row, span) in rows.iter_mut().zip(spans) {
                *row = u64::from_le_bytes(*T::u8_slice(span)?.first_chunk::<8>()?);
            }
            Some(rows)
        });
        let threshold = batch.optflow_sad_threshold;
        let delta = if let [Some(first), Some(second)] = rows {
            splot_recon::optflow_unit_8x8_u8([first, second], distances, threshold)?
        } else {
            let rows = spans.map(|spans| {
                let mut rows = [[0u16; 8]; 8];
                for (row, span) in rows.iter_mut().zip(spans) {
                    *row = *T::u16_slice(span)?.first_chunk::<8>()?;
                }
                Some(rows)
            });
            let [Some(first), Some(second)] = rows else {
                return Ok(None);
            };
            let bit_depth = sink.info().bit_depth();
            splot_recon::optflow_unit_8x8_u16([first, second], bit_depth, distances, threshold)?
        };
        Ok(Some(delta.map_or_else(
            || MotionCell::from_refinemv(mvs),
            |delta| MotionCell::from_optflow(mvs, delta),
        )))
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
        let mut sad = 0;
        for (&left, &right) in spans[0].iter().zip(&spans[1]) {
            let width = if left.len() > 8 { 12 } else { 8 };
            if let (Some(left), Some(right)) = (T::u8_slice(left), T::u8_slice(right)) {
                sad += if width == 8 {
                    byte_sad::<8>(left, right)?
                } else {
                    byte_sad::<12>(left, right)?
                };
                continue;
            }
            let (left, right) = (T::u16_slice(left)?, T::u16_slice(right)?);
            let rows = core::iter::once((left, right));
            sad += if width == 8 {
                sad_rows::<8>(rows, self.max_sample)
            } else {
                sad_rows::<12>(rows, self.max_sample)
            };
        }
        Some(sad)
    }
}

/// `Σ |left - right|` over the first `W` eight-bit samples, 8 or 12, which
/// need no clip to the maximum.
fn byte_sad<const W: usize>(left: &[u8], right: &[u8]) -> Option<u32> {
    let words = |samples: &[u8]| -> Option<[u64; 2]> {
        let low = u64::from_le_bytes(*samples.first_chunk::<8>()?);
        if W == 8 {
            return Some([low, 0]);
        }
        let high = samples.get(8..)?.first_chunk::<4>()?;
        Some([low, u64::from(u32::from_le_bytes(*high))])
    };
    let (left, right) = (words(left)?, words(right)?);
    Some(baseline_simd!(simd => byte_sad_lanes(simd, left, right)))
}

/// `Σ |left - right|` over the bytes of two pairs of words.
#[inline]
fn byte_sad_lanes<S: Simd>(simd: S, left: [u64; 2], right: [u64; 2]) -> u32 {
    let (left, right): (u8x16<S>, u8x16<S>) = (
        u64x2::from_slice(simd, &left).bitcast(),
        u64x2::from_slice(simd, &right).bitcast(),
    );
    let (low, high) =
        simd.widening_add_u8x16(left.max(right) - left.min(right), u8x16::splat(simd, 0));
    let (low, high) = (low + high).widen();
    (low + high).reduce_sum()
}

/// Refines one 8x8 TIP unit. The centre SAD and the centre optical flow read
/// only the 12x12 centre of the 16x16 bilinear search area, so the centre is
/// predicted first and the whole area only when the search goes on.
pub(super) fn tip_refinemv_optflow_motion_cell<'a, T: ReconSample>(
    block: &CompoundMcBlock<'_, T>,
    (rect, candidates): (McBlockRect, [Mv; 2]),
    search: &mut TipSearchScratch,
    luma: &mut [super::optflow::LumaReference<'a, T>; 2],
    carries: &mut [splot_recon::BilinearOverlapCarry<'a, T>; 2],
) -> Result<Option<MotionCell>> {
    const PREDICTION_SIZE: usize = TIP_PREDICTION_SIZE;
    let TipSearchScratch {
        predictions,
        windows,
    } = search;

    let Some(distances) = block.optflow_distances else {
        return Ok(None);
    };
    if !block.search_refinemv
        || rect.luma_w != 8
        || rect.luma_h != 8
        || !search_range_allowed(candidates)
    {
        return Ok(None);
    }
    let mut prediction_rect = rect;
    prediction_rect.luma_w = PREDICTION_SIZE;
    prediction_rect.luma_h = PREDICTION_SIZE;
    if T::u8_slice(&[]).is_some()
        && super::optflow::search_windows_u8(luma, prediction_rect, candidates, windows)?
        && let Some((dx, dy)) =
            splot_recon::refinemv_search_u8([&windows[0], &windows[1]], PREDICTION_SIZE, 12, true)
    {
        let base_mvs = refined_mvs(candidates, dx, dy);
        let starts = [unit_start(dx, dy), unit_start(-dx, -dy)];
        let rows = [0, 1].map(|reference| {
            let unit = windows[reference]
                .get(starts[reference]..)
                .and_then(<[u8]>::first_chunk::<{ 7 * PREDICTION_SIZE + 8 }>);
            core::array::from_fn(|row| {
                unit.and_then(|unit| unit.get(row * PREDICTION_SIZE..)?.first_chunk::<8>())
                    .map_or(0, |bytes| u64::from_le_bytes(*bytes))
            })
        });
        let delta = splot_recon::optflow_unit_8x8_u8(rows, distances, block.optflow_sad_threshold)?;
        return Ok(Some(delta.map_or_else(
            || MotionCell::from_refinemv(base_mvs),
            |delta| MotionCell::from_optflow(base_mvs, delta),
        )));
    }
    let (dx, dy) = tip_search(luma, prediction_rect, candidates, predictions, carries)?;
    let [pred0, pred1] = predictions;
    let bit_depth = luma[0].bit_depth();
    let base_mvs = refined_mvs(candidates, dx, dy);
    super::optflow::tip_optflow_motion_cell_strided(
        pred0,
        unit_start(dx, dy),
        pred1,
        unit_start(-dx, -dy),
        PREDICTION_SIZE,
        bit_depth,
        distances,
        block.optflow_sad_threshold,
        base_mvs,
    )
    .map(Some)
}

/// The start of the 8x8 unit that the search offset `(dx, dy)`, each in
/// `-2..=2`, moves to in a 16x16 TIP prediction.
const fn unit_start(dx: i32, dy: i32) -> usize {
    ((4 + dy) * TIP_PREDICTION_SIZE as i32 + 4 + dx) as usize
}

/// The search offset of a TIP unit from u16 predictions: the 12x12 centre
/// first, the whole 16x16 area only when the search goes on.
fn tip_search<'a, T: ReconSample>(
    luma: &mut [super::optflow::LumaReference<'a, T>; 2],
    prediction_rect: McBlockRect,
    candidates: [Mv; 2],
    predictions: &mut [[u16; TIP_PREDICTION_AREA]; 2],
    carries: &mut [splot_recon::BilinearOverlapCarry<'a, T>; 2],
) -> Result<(i32, i32)> {
    const CENTER_SIZE: usize = 8;
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
    Ok((dx, dy))
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

/// Returns `limit` early once a partial sum reaches it: later rows only add, so
/// such a sum already loses the strict comparison.
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
    let shift = bit_depth.bits().saturating_sub(8);
    let rows = |range: core::ops::Range<usize>| {
        let pairs = left[range.clone()].chunks(2 * TIP_PREDICTION_SIZE);
        sad_rows::<12>(
            pairs.zip(right[range].chunks(2 * TIP_PREDICTION_SIZE)),
            u16::MAX,
        )
    };
    let half = 6 * TIP_PREDICTION_SIZE;
    let sad = rows(0..half);
    if sad >> shift >= limit {
        return limit;
    }
    (sad + rows(half..SPAN)) >> shift
}

fn search_range_allowed(candidates: [Mv; 2]) -> bool {
    candidates.into_iter().all(|mv| {
        [mv.row, mv.col].into_iter().all(|component| {
            (MV_LOW + 1 + SEARCH_PADDING..=MV_UPP - 1 - 2 * 8).contains(&component)
        })
    })
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
fn refinemv_center_sad<'a>(
    pred0: &'a [u16],
    pred1: &'a [u16],
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
    let span = |pred: &'a [u16]| {
        let start = stride.checked_mul(2)?.checked_add(2)?;
        let last_row = sad_height.saturating_sub(1) & !1;
        let len = last_row.checked_mul(stride)?.checked_add(sad_width)?;
        pred.get(start..start.checked_add(len)?)
    };
    let (Some(left), Some(right)) = (span(pred0), span(pred1)) else {
        return Err(ReconError::ArithmeticOverflow {
            context: "refine-MV SAD lookup",
        });
    };
    let rows = left.chunks(2 * stride).zip(right.chunks(2 * stride));
    let center = match sad_width {
        12 => sad_rows::<12>(rows, u16::MAX),
        20 => sad_rows::<20>(rows, u16::MAX),
        _ => {
            return Err(ReconError::SubpelBlockDimensionUnsupported {
                w: sad_width,
                h: sad_height,
            });
        }
    } >> bit_depth.bits().saturating_sub(8);
    let biased_center = center - (center >> 3);
    Ok((biased_center >= threshold).then_some(biased_center))
}

/// Lanes 4..8 of a `u16x8`: the last four of the final eight samples of a
/// row, past the samples of the full vectors.
const HIGH_HALF: [u16; 8] = [0, 0, 0, 0, u16::MAX, u16::MAX, u16::MAX, u16::MAX];

/// `Σ |Min(left, max) - Min(right, max)|` over the first `W` samples, 8 or
/// `W % 8 == 4`, of each row pair: each column's differences add up in `u16`
/// lanes for up to 16 rows at a time, which 12-bit samples keep below `2^16`.
pub(super) fn sad_rows<'a, const W: usize>(
    rows: impl Iterator<Item = (&'a [u16], &'a [u16])>,
    max: u16,
) -> u32 {
    baseline_simd!(simd => sad_row_lanes::<_, W>(simd, rows, max))
}

#[inline]
fn sad_row_lanes<'a, S: Simd, const W: usize>(
    simd: S,
    rows: impl Iterator<Item = (&'a [u16], &'a [u16])>,
    max: u16,
) -> u32 {
    let (zero, max) = (u16x8::splat(simd, 0), u16x8::splat(simd, max));
    let mut lanes = [zero; 3];
    let mut sad = 0;
    let flush = |lanes: &mut [u16x8<S>; 3]| -> u32 {
        let total = lanes
            .iter()
            .take(W.div_ceil(8))
            .fold(u32x4::splat(simd, 0), |total, lanes| {
                let (low, high) = lanes.widen();
                total + low + high
            });
        *lanes = [zero; 3];
        total.reduce_sum()
    };
    for (row, (left, right)) in rows.enumerate() {
        let (Some(left), Some(right)) = (left.first_chunk::<W>(), right.first_chunk::<W>()) else {
            return u32::MAX;
        };
        let load = |samples: &[u16; W], at: usize| match samples
            .get(at..)
            .and_then(<[u16]>::first_chunk::<8>)
        {
            Some(lanes) => u16x8::load_array_ref(simd, lanes),
            None => samples.last_chunk::<8>().map_or(zero, |lanes| {
                u16x8::load_array_ref(simd, lanes) & u16x8::load_array_ref(simd, &HIGH_HALF)
            }),
        };
        for (vector, lanes) in lanes.iter_mut().enumerate().take(W.div_ceil(8)) {
            let (left, right) = (
                load(left, 8 * vector).min(max),
                load(right, 8 * vector).min(max),
            );
            *lanes += left.max(right) - left.min(right);
        }
        if row % 16 == 15 {
            sad += flush(&mut lanes);
        }
    }
    sad + flush(&mut lanes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn default_search_keeps_a_low_sad_centre() {
        let prediction = vec![80u16; 24 * 24];
        assert_eq!(
            splot_recon::refinemv_search_u16(
                [&prediction[..], &prediction[..]],
                24,
                20,
                true,
                splot_recon::BitDepth::Eight,
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
            splot_recon::refinemv_search_u16(
                [&pred0[..], &pred1[..]],
                24,
                20,
                true,
                splot_recon::BitDepth::Eight,
            )
            .expect("offset search"),
            (1, -1)
        );
    }

    #[test]
    fn switchable_search_rejects_low_sad_center_and_starts_with_first_neighbor() {
        let prediction = vec![80u16; 24 * 24];
        assert_eq!(
            splot_recon::refinemv_search_u16(
                [&prediction[..], &prediction[..]],
                24,
                20,
                false,
                splot_recon::BitDepth::Eight,
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
