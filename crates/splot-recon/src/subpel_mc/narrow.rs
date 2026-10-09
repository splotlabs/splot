// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unscaled two-axis § 7.13.3.18 convolution for 4- and 8-wide blocks.
//!
//! These widths are one vector per row, so the filter state is fixed before
//! the row loops and each row costs one tap window and one vector of taps.
//! The arithmetic and tap order are those of the general two-pass core.

use super::clipped_edges::{ClampedWindow, WINDOW_STORAGE};
use super::*;

/// Runs the convolution when `params.w` is 4 or 8; any other width is left
/// to the general core.
#[allow(clippy::too_many_arguments)]
pub(super) fn two_axis<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    inter_round1: u32,
    intermediate: &mut [i16],
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) {
    match params.w {
        4 => rows::<4, T, O>(
            reference,
            params,
            inter_round1,
            intermediate,
            output,
            output_stride,
            finish,
        ),
        8 => rows::<8, T, O>(
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
fn rows<const LANES: usize, T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    inter_round1: u32,
    intermediate: &mut [i16],
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) where
    Simd<i32, LANES>: SlideLanes<Intermediate = Simd<i16, LANES>>,
{
    let h_filter = params.interp.pass_index(LANES as u32) as usize;
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
    for (row, lanes) in intermediate[..row_count * LANES]
        .chunks_exact_mut(LANES)
        .enumerate()
    {
        let ref_row = ((top + row as i32).clamp(params.first_y, params.last_y) as usize)
            .min(reference.readable_rows - 1);
        let window = match window_x {
            Some(x) => {
                let base = ref_row * reference.stride + x;
                let end = base + LANES + NUM_TAPS - 1;
                reference
                    .samples
                    .get(base..end + SLIDE_RESERVE)
                    .unwrap_or(&reference.samples[base..end])
            }
            None => clamped.fill(
                reference.row(ref_row),
                clamped_storage.get_or_insert([T::default(); WINDOW_STORAGE]),
            ),
        };
        let filtered = if Simd::<i32, LANES>::admits(window.len(), 0) {
            Simd::<i32, LANES>::slid_intermediate(window, 0, packed_taps)
        } else {
            let mut sum = Simd::splat(0);
            for (tap_index, &tap) in h_taps.iter().enumerate() {
                sum = tap_mac(
                    sum,
                    reference_lanes::<LANES, T>(window, tap_index).cast(),
                    tap,
                );
            }
            round2_simd(sum, INTER_ROUND0).cast()
        };
        lanes.copy_from_slice(filtered.as_array()); // splot-copy-ok: publish one intermediate row
    }

    let intermediate = &intermediate[..row_count * LANES];
    let vertical = match v_count {
        2 => vertical::<LANES, 2, O>,
        4 => vertical::<LANES, 4, O>,
        6 => vertical::<LANES, 6, O>,
        _ => vertical::<LANES, NUM_TAPS, O>,
    };
    vertical(
        v_taps,
        intermediate,
        params.h,
        inter_round1,
        output,
        output_stride,
        finish,
    );
}

/// The vertical pass with a constant tap count, so the taps stay in
/// registers. Zero taps around the active span add exactly zero.
#[allow(clippy::too_many_arguments)]
fn vertical<const LANES: usize, const TAPS: usize, O>(
    taps: &[i32],
    intermediate: &[i16],
    height: usize,
    inter_round1: u32,
    output: &mut [O],
    output_stride: usize,
    finish: &mut impl SubpelOutput<O>,
) {
    let taps: [i32; TAPS] = core::array::from_fn(|tap| taps[tap]);
    for row in 0..height {
        let rows = &intermediate[row * LANES..][..TAPS * LANES];
        let mut sum = Simd::<i32, LANES>::splat(0);
        for (tap, samples) in taps.iter().zip(rows.chunks_exact(LANES)) {
            sum = tap_mac(sum, Simd::from_slice(samples), *tap);
        }
        let values = round2_simd(sum, inter_round1);
        let row_out = &mut output[row * output_stride..][..LANES];
        if LANES == 4 {
            finish.four(Simd::from_slice(values.as_array()), row_out);
        } else {
            finish.eight(Simd::from_slice(values.as_array()), row_out);
        }
    }
}
