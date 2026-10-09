// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

fn interior(
    width: usize,
    x0: i32,
    tap_start: usize,
    tap_end: usize,
    first_x: i32,
    last_x: i32,
    reference_width: usize,
) -> core::ops::Range<usize> {
    let sample_lo = i64::from(first_x.max(0));
    let sample_hi = i64::from(last_x.min(reference_width as i32 - 1));
    let first = sample_lo - i64::from(x0) - tap_start as i64 + 3;
    let end = sample_hi - i64::from(x0) - tap_end.saturating_sub(1) as i64 + 4;
    let end = end.clamp(0, width as i64) as usize;
    first.clamp(0, end as i64) as usize..end
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn vertical_only<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    row: usize,
    taps: &[i32],
    tap_start: usize,
    inter_round1: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) {
    let source = reference.samples;
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let interior = interior(
        params.w,
        x0,
        3,
        4,
        params.first_x,
        params.last_x,
        reference.width,
    );
    let row_out = &mut output[row * output_stride..][..params.w];
    for (col, output) in row_out[..interior.start].iter_mut().enumerate() {
        *output = finish.one(vertical_scalar_value(
            reference,
            params,
            row,
            col,
            taps,
            tap_start,
            inter_round1,
        ));
    }
    let vector_end8 = interior.start + interior.len() / 8 * 8;
    for c in (interior.start..vector_end8).step_by(8) {
        let mut sum = Simd::<i32, 8>::splat(0);
        for (tap_offset, &tap) in taps.iter().enumerate() {
            let t = tap_start + tap_offset;
            let ref_row =
                (y0 + row as i32 + t as i32 - 3).clamp(params.first_y, params.last_y) as usize;
            let start = ref_row.min(reference.readable_rows - 1) * reference.stride
                + (x0 + c as i32) as usize;
            sum = tap_mac(sum, reference_lanes::<8, T>(source, start).cast(), tap);
        }
        let values = round2_simd(sum << (FILTER_BITS - INTER_ROUND0) as i32, inter_round1);
        finish.eight(values, &mut row_out[c..c + 8]);
    }
    let vector_end4 = interior.end - interior.end.saturating_sub(vector_end8) % 4;
    for c in (vector_end8..vector_end4).step_by(4) {
        let mut sum = Simd::<i32, 4>::splat(0);
        for (tap_offset, &tap) in taps.iter().enumerate() {
            let t = tap_start + tap_offset;
            let ref_row =
                (y0 + row as i32 + t as i32 - 3).clamp(params.first_y, params.last_y) as usize;
            let start = ref_row.min(reference.readable_rows - 1) * reference.stride
                + (x0 + c as i32) as usize;
            sum = tap_mac(sum, reference_lanes::<4, T>(source, start).cast(), tap);
        }
        let values = round2_simd(sum << (FILTER_BITS - INTER_ROUND0) as i32, inter_round1);
        finish.four(values, &mut row_out[c..c + 4]);
    }
    for (offset, output) in row_out[vector_end4..].iter_mut().enumerate() {
        *output = finish.one(vertical_scalar_value(
            reference,
            params,
            row,
            vector_end4 + offset,
            taps,
            tap_start,
            inter_round1,
        ));
    }
}

fn vertical_scalar_value<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    row: usize,
    col: usize,
    taps: &[i32],
    tap_start: usize,
    inter_round1: u32,
) -> i32 {
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let mut sum = 0i32;
    for (tap_offset, &tap) in taps.iter().enumerate() {
        let t = tap_start + tap_offset;
        let ref_row =
            (y0 + row as i32 + t as i32 - 3).clamp(params.first_y, params.last_y) as usize;
        let ref_col = (x0 + col as i32).clamp(params.first_x, params.last_x) as usize;
        sum += tap * reference.sample(ref_row, ref_col);
    }
    round2_i32(sum << (FILTER_BITS - INTER_ROUND0), inter_round1)
}

/// Samples one clamped § 7.13.3.18 horizontal tap window holds, plus the
/// [`SLIDE_RESERVE`] the sliding load shape may read past it.
pub(super) const WINDOW_STORAGE: usize = MAX_BLOCK_DIM + NUM_TAPS - 1 + SLIDE_RESERVE;

/// Column layout of the `w + 7`-sample unscaled tap window starting at
/// `x0 - 3` when some taps fall outside `[firstX, lastX]` or the plane.
///
/// `Clip3(firstX, lastX, x)` followed by the plane clamp equals one clamp to
/// the plane-bounded range, so the window is a run of the first column, a
/// contiguous copy, and a run of the last column.
#[derive(Clone, Copy)]
pub(super) struct ClampedWindow {
    copy_start: usize,
    prefix: usize,
    middle_end: usize,
    len: usize,
    first: usize,
    last: usize,
}

impl ClampedWindow {
    pub(super) fn new<T: ReconSample>(
        reference: &ReferencePlaneView<'_, T>,
        params: &SubpelPredictParams,
    ) -> Self {
        let plane_last = reference.width as i32 - 1;
        let first = params.first_x.clamp(0, plane_last);
        let last = params.last_x.clamp(0, plane_last);
        let start = (params.start_x >> SCALE_SUBPEL_BITS) - 3;
        let len = params.w + NUM_TAPS - 1;
        let prefix = (i64::from(first) - i64::from(start)).clamp(0, len as i64) as usize;
        let middle_end =
            (i64::from(last) - i64::from(start) + 1).clamp(prefix as i64, len as i64) as usize;
        Self {
            copy_start: if prefix < middle_end {
                (start + prefix as i32) as usize
            } else {
                0
            },
            prefix,
            middle_end,
            len,
            first: first as usize,
            last: last as usize,
        }
    }

    /// Writes the window of one plane row into `storage` and returns it.
    pub(super) fn fill<'a, T: ReconSample>(
        &self,
        row: &[T],
        storage: &'a mut [T; WINDOW_STORAGE],
    ) -> &'a [T] {
        storage[..self.prefix].fill(row[self.first]);
        let middle = &row[self.copy_start..self.copy_start + self.middle_end - self.prefix];
        storage[self.prefix..self.middle_end].copy_from_slice(middle); // splot-copy-ok: materialize one clamped tap window
        storage[self.middle_end..self.len].fill(row[self.last]);
        storage
    }
}
