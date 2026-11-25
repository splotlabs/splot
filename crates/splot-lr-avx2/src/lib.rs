// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Concrete AVX2 entry for word-to-word luma Wiener restoration runs.

#![cfg(target_arch = "x86_64")]

use core::arch::x86_64::*;

use fearless_simd::{Avx2, Simd, SimdBase, SimdInto, i16x16, u16x16};
use splot_lr_kernel::avx2::in_lane_weights;
use splot_lr_kernel::{Classes, LumaTaps, MaddWeights, TapGeometry, filter_run_groups};

splot_lr_kernel::madd_row_kernel!(word_row, u16, LumaTaps, 9, 24, |token, lanes| {
    u16x16::load_array_ref(token, lanes).into()
});

/// Filters the complete 16-column groups in `span = (x, end)`. A zero-row
/// or empty span succeeds without writing output.
///
/// For a nonempty run, `rows` is in `1..=8`. The first `rows + 8` source
/// rows contain the four-sample halo on each side of the run. Row zero
/// starts four rows above the output, and column zero starts four samples
/// left of output column zero. Output row `r` starts at `r * stride + x`,
/// and the written portions of successive rows must be disjoint.
///
/// Input words have been validated in `0..=max_sample`, and `max_sample`
/// is at most 1023. Each class contains the 16 luma tap-pair coefficients
/// and the precomputed center weight `128 - 2 * sum(coefficients)`.
/// A cell class map has one valid class-table index per four columns,
/// starting at `x`. The result is rounded by seven bits and clipped to
/// `0..=max_sample`.
///
/// Returns `false` when the run exceeds the supported geometry or a source,
/// output or class span is short. A short class span can leave earlier
/// groups written.
#[inline(never)]
pub fn filter_luma_run(
    token: Avx2,
    window: &[&[u16]],
    rows: usize,
    span: (usize, usize),
    classes: Classes<'_>,
    output: (&mut [u16], usize),
    max_sample: u16,
) -> bool {
    const ROWS: usize = 2 * LumaTaps::RADIUS + 1;
    const REACH: usize = 16 + 2 * LumaTaps::RADIUS;
    const PAIRS: usize = LumaTaps::TAPS.len() / 2;

    token.vectorize(
        #[inline(always)]
        || {
            filter_run_groups::<u16, u16, ROWS, REACH>(
                window,
                rows,
                span,
                classes,
                output,
                #[inline(always)]
                |cells| in_lane_weights(token, cells, PAIRS),
                #[inline(always)]
                |window, weights, output| {
                    *output = word_row(token, window, weights, max_sample);
                },
            )
        },
    )
}
