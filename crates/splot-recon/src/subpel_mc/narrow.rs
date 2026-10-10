// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unscaled two-axis § 7.13.3.18 convolution for blocks 4, 8 or a multiple
//! of 16 samples wide.
//!
//! Each row is a whole number of vectors, so the filter state is fixed before
//! the row loops and each row costs one tap window and its vectors of taps.
//! The arithmetic and tap order are those of the general two-pass core.

use super::clipped_edges::{ClampedWindow, WINDOW_STORAGE};
use super::*;

/// Runs the convolution when `params.w` is 4, 8 or a multiple of 16; any
/// other width is left to the general core.
#[allow(clippy::too_many_arguments)]
pub(super) fn two_axis<T: ReconSample, O, F: SubpelOutput<O>>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    inter_round1: u32,
    intermediate: &mut [i16],
    output: &mut [O],
    output_stride: usize,
    finish: &mut F,
) {
    match params.w {
        4 => rows::<4, T, O, F>(
            reference,
            params,
            inter_round1,
            intermediate,
            output,
            output_stride,
            finish,
        ),
        8 => rows::<8, T, O, F>(
            reference,
            params,
            inter_round1,
            intermediate,
            output,
            output_stride,
            finish,
        ),
        w if w % 16 == 0 => rows::<16, T, O, F>(
            reference,
            params,
            inter_round1,
            intermediate,
            output,
            output_stride,
            finish,
        ),
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn rows<const LANES: usize, T: ReconSample, O, F: SubpelOutput<O>>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    inter_round1: u32,
    intermediate: &mut [i16],
    output: &mut [O],
    output_stride: usize,
    finish: &mut F,
) where
    Simd<i32, LANES>: SlideLanes<Intermediate = Simd<i16, LANES>>,
{
    let width = if LANES == 16 { params.w } else { LANES };
    let h_filter = params.interp.pass_index(width as u32) as usize;
    let h_taps = &SUBPEL_FILTERS[h_filter][((params.start_x >> 6) & SUBPEL_MASK) as usize];
    let packed_taps = slide::intermediate_taps::<T>(h_taps);
    let v_filter = params.interp.pass_index(params.h as u32) as usize;
    let v_phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
    let (v_start, v_end) = ACTIVE_TAP_SPANS[v_filter][v_phase];
    let v_count = match v_end - v_start {
        0..=2 => 2,
        3..=4 => 4,
        5..=6 => 6,
        _ => NUM_TAPS,
    };
    let v_start = v_start.min(NUM_TAPS - v_count);
    let v_taps = &SUBPEL_FILTERS[v_filter][v_phase][v_start..v_start + v_count];

    let window_x = subpel_horizontal_window_x(reference, params);
    let clamped = ClampedWindow::new(reference, params);
    let mut clamped_storage = None;
    let top = (params.start_y >> SCALE_SUBPEL_BITS) - 3 + v_start as i32;
    let row_count = params.h + v_count - 1;
    let intermediate = &mut intermediate[..row_count * width];
    let source = |row: usize| {
        ((top + row as i32).clamp(params.first_y, params.last_y) as usize)
            .min(reference.readable_rows - 1)
    };
    let filter = |window: &[T], column: usize| -> Simd<i16, LANES> {
        let span = window.get(column..column + <Simd<i32, LANES> as SlideLanes>::SPAN);
        if let Some(span) = span {
            return Simd::<i32, LANES>::slid_intermediate(span, 0, packed_taps);
        }
        let mut sum = Simd::splat(0);
        for (tap_index, &tap) in h_taps.iter().enumerate() {
            let lanes = reference_lanes::<LANES, T>(window, column + tap_index);
            sum = tap_mac(sum, lanes.cast(), tap);
        }
        round2_simd(sum, INTER_ROUND0).cast()
    };
    if LANES < 16 {
        for (row, lanes) in intermediate.chunks_exact_mut(LANES).enumerate() {
            let window = tap_window(
                reference,
                window_x,
                &clamped,
                &mut clamped_storage,
                source(row),
                width,
            );
            lanes.copy_from_slice(filter(window, 0).as_array()); // splot-copy-ok: publish one intermediate vector
        }
    } else {
        for row in 0..row_count {
            let window = tap_window(
                reference,
                window_x,
                &clamped,
                &mut clamped_storage,
                source(row),
                width,
            );
            for (strip, lanes) in intermediate.chunks_exact_mut(row_count * LANES).enumerate() {
                let filtered = filter(window, strip * LANES);
                lanes[row * LANES..][..LANES].copy_from_slice(filtered.as_array()); // splot-copy-ok: publish one intermediate vector
            }
        }
    }

    let vertical = match v_count {
        2 => vertical::<LANES, 2, O, F>,
        4 => vertical::<LANES, 4, O, F>,
        6 => vertical::<LANES, 6, O, F>,
        _ => vertical::<LANES, NUM_TAPS, O, F>,
    };
    vertical(
        v_taps,
        intermediate,
        width,
        params.h,
        inter_round1,
        output,
        output_stride,
        finish,
    );
}

/// The `width + 7`-sample tap window of reference row `row`, read in place
/// when `window_x` admits it, else built clamped in `storage`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn tap_window<'a, T: ReconSample>(
    reference: &'a ReferencePlaneView<'_, T>,
    window_x: Option<usize>,
    clamped: &ClampedWindow,
    storage: &'a mut Option<[T; WINDOW_STORAGE]>,
    row: usize,
    width: usize,
) -> &'a [T] {
    match window_x {
        Some(x) => {
            let base = row * reference.stride + x;
            let end = base + width + NUM_TAPS - 1;
            reference
                .samples
                .get(base..end + SLIDE_RESERVE)
                .unwrap_or(&reference.samples[base..end])
        }
        None => clamped.fill(
            reference.row(row),
            storage.get_or_insert([T::default(); WINDOW_STORAGE]),
        ),
    }
}

/// The vertical pass with a constant tap count, so the taps stay in
/// registers. Zero taps around the active span add exactly zero. The
/// intermediate holds each `LANES`-wide column strip as consecutive rows, so
/// the tap rows of an output vector sit at constant offsets.
#[allow(clippy::too_many_arguments)]
fn vertical<const LANES: usize, const TAPS: usize, O, F: SubpelOutput<O>>(
    taps: &[i32],
    intermediate: &[i16],
    width: usize,
    height: usize,
    inter_round1: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut F,
) {
    let width = if LANES == 16 { width } else { LANES };
    let taps = Simd::<i16, NUM_TAPS>::from_array(core::array::from_fn(|tap| {
        taps.get(tap).map_or(0, |&tap| tap as i16)
    }));
    let strip_len = (height + TAPS - 1) * LANES;
    for row in 0..height {
        let row_out = &mut output[row * output_stride..][..width];
        for (strip, lanes_out) in row_out.chunks_exact_mut(LANES).enumerate() {
            let window = &intermediate[strip * strip_len + row * LANES..][..TAPS * LANES];
            let mut sum = Simd::<i32, LANES>::splat(0);
            for tap in 0..TAPS {
                let lanes = Simd::from_slice(&window[tap * LANES..]);
                sum = tap_mac(sum, lanes, i32::from(taps[tap]));
            }
            finish.rounded(sum, 0, inter_round1, lanes_out);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The clipped output narrows its rounded sums to `i16`; extreme 10-bit
    /// contrast under the sharpest taps must still match the unnarrowed path.
    #[test]
    fn clipped_rounding_matches_the_unnarrowed_path_at_extreme_contrast() {
        let (width, height) = (48, 48);
        let widest = SUBPEL_FILTERS[EIGHTTAP_SHARP as usize][8];
        let peak = |i: usize| u16::from(widest[i % NUM_TAPS] > 0) * 1023;
        let planes: [Vec<u16>; 3] = [
            (0..width * height).map(|i| peak(i % width)).collect(),
            (0..width * height)
                .map(|i| peak(i % width).min(peak(i / width)))
                .collect(),
            (0..width * height)
                .map(|i| 1023 - peak(i % width).min(peak(i / width)))
                .collect(),
        ];
        for samples in &planes {
            let view = ReferencePlaneView::new(samples, width, height).unwrap();
            for (w, h) in [(4, 4), (8, 8), (16, 16), (32, 8), (32, 32)] {
                for phase in 0..256 {
                    let params = SubpelPredictParams {
                        interp: InterpolationFilter::EightTapSharp,
                        w,
                        h,
                        start_x: (4 << SCALE_SUBPEL_BITS) + ((phase % 16) << 6),
                        start_y: (4 << SCALE_SUBPEL_BITS) + ((phase / 16) << 6),
                        step_x: 1 << SCALE_SUBPEL_BITS,
                        step_y: 1 << SCALE_SUBPEL_BITS,
                        first_x: 0,
                        first_y: 0,
                        last_x: width as i32 - 1,
                        last_y: height as i32 - 1,
                        bit_depth: BitDepth::Ten,
                    };
                    let mut clipped = vec![0; w * h];
                    subpel_predict_block_into(&view, &params, &mut clipped).unwrap();
                    let unnarrowed = subpel_predict_block(&view, &params).unwrap();
                    assert_eq!(clipped, unnarrowed, "{w}x{h} phase {phase}");
                }
            }
        }
    }
}
