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
pub(super) fn horizontal_only<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    ref_row: usize,
    taps: &[i32],
    tap_start: usize,
    tap_end: usize,
    inter_round1: u32,
    row_out: &mut [O],
    finish: &mut impl SubpelOutput<O>,
) -> bool {
    let Some(source) = T::u16_slice(reference.row(ref_row)) else {
        return false;
    };
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let interior = interior(
        params.w,
        x0,
        tap_start,
        tap_end,
        params.first_x,
        params.last_x,
        reference.width,
    );
    for (c, output) in row_out[..interior.start].iter_mut().enumerate() {
        let mut sum = 0i32;
        for (tap_offset, &tap) in taps.iter().enumerate() {
            let t = tap_start + tap_offset;
            let ref_col =
                (x0 + c as i32 + t as i32 - 3).clamp(params.first_x, params.last_x) as usize;
            sum += tap * reference.sample(ref_row, ref_col);
        }
        let horizontal = round2_i32(sum, INTER_ROUND0);
        *output = finish.one(round2_i32(horizontal << FILTER_BITS, inter_round1));
    }
    let vector_end8 = interior.start + interior.len() / 8 * 8;
    for c in (interior.start..vector_end8).step_by(8) {
        let mut sum = Simd::<i32, 8>::splat(0);
        let start = (x0 + c as i32 + tap_start as i32 - 3) as usize;
        for (tap_offset, &tap) in taps.iter().enumerate() {
            sum += Simd::<u16, 8>::from_slice(&source[start + tap_offset..]).cast::<i32>()
                * Simd::splat(tap);
        }
        let values = round2_simd(
            round2_simd(sum, INTER_ROUND0) << FILTER_BITS as i32,
            inter_round1,
        );
        finish.eight(values, &mut row_out[c..c + 8]);
    }
    let vector_end4 = interior.end - interior.end.saturating_sub(vector_end8) % 4;
    for c in (vector_end8..vector_end4).step_by(4) {
        let mut sum = Simd::<i32, 4>::splat(0);
        let start = (x0 + c as i32 + tap_start as i32 - 3) as usize;
        for (tap_offset, &tap) in taps.iter().enumerate() {
            sum += Simd::<u16, 4>::from_slice(&source[start + tap_offset..]).cast::<i32>()
                * Simd::splat(tap);
        }
        let values = round2_simd(
            round2_simd(sum, INTER_ROUND0) << FILTER_BITS as i32,
            inter_round1,
        );
        finish.four(values, &mut row_out[c..c + 4]);
    }
    for (offset, output) in row_out[vector_end4..interior.end].iter_mut().enumerate() {
        let c = vector_end4 + offset;
        let mut sum = 0i32;
        for (tap_offset, &tap) in taps.iter().enumerate() {
            let t = tap_start + tap_offset;
            sum += tap * i32::from(source[(x0 + c as i32 + t as i32 - 3) as usize]);
        }
        let horizontal = round2_i32(sum, INTER_ROUND0);
        *output = finish.one(round2_i32(horizontal << FILTER_BITS, inter_round1));
    }
    for (offset, output) in row_out[interior.end..].iter_mut().enumerate() {
        let c = interior.end + offset;
        let mut sum = 0i32;
        for (tap_offset, &tap) in taps.iter().enumerate() {
            let t = tap_start + tap_offset;
            let ref_col =
                (x0 + c as i32 + t as i32 - 3).clamp(params.first_x, params.last_x) as usize;
            sum += tap * reference.sample(ref_row, ref_col);
        }
        let horizontal = round2_i32(sum, INTER_ROUND0);
        *output = finish.one(round2_i32(horizontal << FILTER_BITS, inter_round1));
    }
    true
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
) -> bool {
    let Some(source) = T::u16_slice(reference.samples) else {
        return false;
    };
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
            let start =
                ref_row.min(reference.height - 1) * reference.stride + (x0 + c as i32) as usize;
            sum += Simd::<u16, 8>::from_slice(&source[start..]).cast::<i32>() * Simd::splat(tap);
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
            let start =
                ref_row.min(reference.height - 1) * reference.stride + (x0 + c as i32) as usize;
            sum += Simd::<u16, 4>::from_slice(&source[start..]).cast::<i32>() * Simd::splat(tap);
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
    true
}

#[allow(clippy::too_many_arguments)]
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

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn horizontal_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    ref_row: usize,
    h_filter: usize,
    row_out: &mut [i32],
) -> bool {
    if params.step_x != 1 << SCALE_SUBPEL_BITS {
        return false;
    }
    let Some(source) = T::u16_slice(reference.row(ref_row)) else {
        return false;
    };
    let phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
    let (tap_start, tap_end) = ACTIVE_TAP_SPANS[h_filter][phase];
    let taps = &SUBPEL_FILTERS[h_filter][phase][tap_start..tap_end];
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let interior = interior(
        params.w,
        x0,
        tap_start,
        tap_end,
        params.first_x,
        params.last_x,
        reference.width,
    );
    for (col, output) in row_out[..interior.start].iter_mut().enumerate() {
        *output = horizontal_scalar(reference, params, ref_row, col, taps, tap_start);
    }
    let vector_end8 = interior.start + interior.len() / 8 * 8;
    for c in (interior.start..vector_end8).step_by(8) {
        let mut sum = Simd::<i32, 8>::splat(0);
        let start = (x0 + c as i32 + tap_start as i32 - 3) as usize;
        for (tap_offset, &tap) in taps.iter().enumerate() {
            sum += Simd::<u16, 8>::from_slice(&source[start + tap_offset..]).cast::<i32>()
                * Simd::splat(tap);
        }
        row_out[c..c + 8].copy_from_slice(&round2_simd(sum, INTER_ROUND0).to_array()); // splot-copy-ok: publish clipped-edge SIMD convolution lanes
    }
    let vector_end4 = interior.end - interior.end.saturating_sub(vector_end8) % 4;
    for c in (vector_end8..vector_end4).step_by(4) {
        let mut sum = Simd::<i32, 4>::splat(0);
        let start = (x0 + c as i32 + tap_start as i32 - 3) as usize;
        for (tap_offset, &tap) in taps.iter().enumerate() {
            sum += Simd::<u16, 4>::from_slice(&source[start + tap_offset..]).cast::<i32>()
                * Simd::splat(tap);
        }
        row_out[c..c + 4].copy_from_slice(&round2_simd(sum, INTER_ROUND0).to_array()); // splot-copy-ok: publish clipped-edge SIMD convolution lanes
    }
    for (offset, output) in row_out[vector_end4..interior.end].iter_mut().enumerate() {
        *output = horizontal_scalar(
            reference,
            params,
            ref_row,
            vector_end4 + offset,
            taps,
            tap_start,
        );
    }
    for (offset, output) in row_out[interior.end..].iter_mut().enumerate() {
        *output = horizontal_scalar(
            reference,
            params,
            ref_row,
            interior.end + offset,
            taps,
            tap_start,
        );
    }
    true
}

fn horizontal_scalar<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    ref_row: usize,
    col: usize,
    taps: &[i32],
    tap_start: usize,
) -> i32 {
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let mut sum = 0i32;
    for (tap_offset, &tap) in taps.iter().enumerate() {
        let t = tap_start + tap_offset;
        let ref_col =
            (x0 + col as i32 + t as i32 - 3).clamp(params.first_x, params.last_x) as usize;
        sum += tap * reference.sample(ref_row, ref_col);
    }
    round2_i32(sum, INTER_ROUND0)
}
