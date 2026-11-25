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

/// Filters the complete 16-column groups of a padded luma Wiener run.
/// `window` contains `rows + 8` input rows, starting four rows above the output.
/// Source row slices start four samples left of output column zero; output
/// row `r` starts at `r * stride` of the supplied buffer.
/// Samples must fit the active bit depth of at most 10 bits. Returns
/// `false` if a source, output or class span is short or the run exceeds
/// eight output rows.
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
