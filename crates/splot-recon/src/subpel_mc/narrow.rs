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
use std::simd::simd_swizzle;

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
    let (v_start, v_count) = even_tap_span(v_filter, v_phase);
    let v_taps = paired_taps(v_filter, v_phase);

    let window_x = subpel_horizontal_window_x(reference, params);
    let prefetch_x = window_x.filter(|_| prefetches(reference.samples));
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
        let mut row = 0;
        while row < row_count {
            macro_rules! bytes {
                ($row:expr) => {
                    window_bytes(tap_window(
                        reference,
                        window_x,
                        &clamped,
                        &mut clamped_storage,
                        source($row),
                        width,
                    ))
                };
            }
            if row + 1 < row_count
                && let Some(first) = bytes!(row)
                && let Some(second) = bytes!(row + 1)
            {
                let pair = pairs::horizontal(pairs::join(first, second), packed_taps);
                let lanes = &mut intermediate[row * LANES..][..2 * LANES];
                if LANES == 8 {
                    lanes.copy_from_slice(&pair.as_array()[..2 * LANES]); // splot-copy-ok: publish two intermediate rows
                } else {
                    let rows: Simd<i16, 8> = simd_swizzle!(pair, [0, 1, 2, 3, 8, 9, 10, 11]);
                    lanes.copy_from_slice(&rows.as_array()[..2 * LANES]); // splot-copy-ok: publish two intermediate rows
                }
                row += 2;
                continue;
            }
            let window = tap_window(
                reference,
                window_x,
                &clamped,
                &mut clamped_storage,
                source(row),
                width,
            );
            intermediate[row * LANES..][..LANES].copy_from_slice(filter(window, 0).as_array()); // splot-copy-ok: publish one intermediate vector
            row += 1;
        }
    } else {
        for row in 0..row_count {
            if let Some(x) = prefetch_x {
                prefetch_reference_row(
                    reference,
                    top + (row + PREFETCH_ROWS) as i32,
                    x,
                    width + NUM_TAPS - 1,
                );
            }
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

    if F::INTER_ROUND1 == Some(INTER_ROUND1_NON_COMPOUND) && params.h.is_multiple_of(16 / LANES) {
        let vertical = match v_count {
            2 => paired_vertical::<LANES, 2, O, F>,
            4 => paired_vertical::<LANES, 4, O, F>,
            6 => paired_vertical::<LANES, 6, O, F>,
            _ => paired_vertical::<LANES, NUM_TAPS, O, F>,
        };
        vertical(
            &v_taps,
            intermediate,
            width,
            params.h,
            output,
            output_stride,
            finish,
        );
        return;
    }
    let vertical = match v_count {
        2 => vertical::<LANES, 2, O, F>,
        4 => vertical::<LANES, 4, O, F>,
        6 => vertical::<LANES, 6, O, F>,
        _ => vertical::<LANES, NUM_TAPS, O, F>,
    };
    vertical(
        &v_taps,
        intermediate,
        width,
        params.h,
        inter_round1,
        output,
        output_stride,
        finish,
    );
}

/// The first 16 samples of an 8-bit tap window.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn window_bytes<T: ReconSample>(window: &[T]) -> Option<Simd<u8, 16>> {
    Some(Simd::from_array(*T::u8_slice(window)?.first_chunk::<16>()?))
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
    pairs: &[Simd<i16, 16>; NUM_TAPS / 2],
    intermediate: &[i16],
    width: usize,
    height: usize,
    inter_round1: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut F,
) {
    let width = if LANES == 16 { width } else { LANES };
    let strip_len = (height + TAPS - 1) * LANES;
    for row in 0..height {
        let row_out = &mut output[row * output_stride..][..width];
        for (strip, lanes_out) in row_out.chunks_exact_mut(LANES).enumerate() {
            let window = &intermediate[strip * strip_len + row * LANES..][..TAPS * LANES];
            let sum = paired_tap_sum::<LANES, TAPS>(pairs, |tap| {
                Simd::from_slice(&window[tap * LANES..])
            });
            finish.rounded(sum, 0, inter_round1, lanes_out);
        }
    }
}

/// The non-compound vertical pass, `16 / LANES` output rows per
/// [`pairs::weigh`]. Each `LANES`-wide strip keeps its rows contiguous, so a
/// 16-lane load at row `k` is rows `k..k + 16 / LANES`, and the in-lane
/// interleave of the loads at `k` and `k + 1` pairs every row with its
/// successor. A non-compound value fits `i16`.
#[allow(clippy::too_many_arguments)]
fn paired_vertical<const LANES: usize, const TAPS: usize, O, F: SubpelOutput<O>>(
    pairs: &[Simd<i16, 16>; NUM_TAPS / 2],
    intermediate: &[i16],
    width: usize,
    height: usize,
    output: &mut [O],
    output_stride: usize,
    finish: &mut F,
) {
    let weights = pairs;
    let strip_len = (height + TAPS - 1) * LANES;
    for row in (0..height).step_by(16 / LANES) {
        for strip in 0..width / LANES {
            let window =
                &intermediate[strip * strip_len + row * LANES..][..(TAPS - 1) * LANES + 16];
            let line = |k: usize| pairs::Pair::from_slice(&window[k * LANES..]);
            let mut terms = [(line(0), line(1), weights[0]); NUM_TAPS / 2];
            for (pair, term) in terms.iter_mut().enumerate().take(TAPS / 2) {
                *term = (line(2 * pair), line(2 * pair + 1), weights[pair]);
            }
            let values = pairs::weigh::<INTER_ROUND1_NON_COMPOUND>(&terms[..TAPS / 2]);
            let output = &mut output[row * output_stride + strip * LANES..];
            finish.rounded_rows(values, LANES.min(16), output, output_stride);
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
