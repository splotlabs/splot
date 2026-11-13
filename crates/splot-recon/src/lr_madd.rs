// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! 16-lane § 7.20.3 and § 7.20.4 row kernels on `pmaddwd` tap pairs.
//!
//! Both filters sum symmetric taps first, so every tap sum, the center and
//! each weight fit `i16`, and a pair of tap sums times a pair of weights is
//! one `pmaddwd`. The last pair weighs the center and a lane of ones by the
//! center weight and the rounding offset. Four 4-sample cells share a
//! vector; each cell's weights are a [`MaddPattern`] of four lanes.
//!
//! On x86 the pairs interleave in-lane, so the weights of cells 0 and 2 and
//! of cells 1 and 3 share a vector, and the final pack restores the order;
//! the portable kernel interleaves in sample order.

#[cfg(not(target_arch = "x86_64"))]
use fearless_simd::i32x4;
use fearless_simd::{Simd, SimdIntWiden, i16x16, i32x8, prelude::*, u16x16};

/// Runs `$body` with the baseline SIMD token bound to `$simd`; on x86 the
/// AVX2 token binds directly, since a `dispatch!` closure does not inline
/// into per-row callers.
macro_rules! lr_simd {
    ($simd:ident => $body:expr) => {
        baseline_simd!($simd => $body)
    };
}
pub(crate) use lr_simd;

use crate::wienerns_filter::{LumaSimdOutput, LumaSimdSource};

/// One 4-sample cell's weights for one pair: `[w0, w1]` per lane.
pub(crate) type MaddPattern = [i16; 8];

/// Both `pmaddwd` weight vectors of one pair over four cells.
pub(crate) type MaddWeights = [[i16; 16]; 2];

/// Whether [`MaddWeights`] over `T` rows take the in-lane arrangement of the
/// x86 kernel.
#[inline]
fn in_lane<T: LumaSimdSource>() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        T::WIDE && fearless_simd::Level::baseline().as_avx2().is_some()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// Writes the weights of one pair for cells `0..4` in the kernel's
/// arrangement.
#[inline]
pub(crate) fn arrange_into<T: LumaSimdSource>(weights: &mut MaddWeights, cells: [&MaddPattern; 4]) {
    let [a, b, c, d] = if in_lane::<T>() {
        [cells[0], cells[2], cells[1], cells[3]]
    } else {
        cells
    };
    #[cfg(not(target_arch = "x86_64"))]
    {
        weights[0] = [
            a[0], a[0], a[0], a[0], b[0], b[0], b[0], b[0], c[0], c[0], c[0], c[0], d[0], d[0],
            d[0], d[0],
        ];
        weights[1] = [
            a[1], a[1], a[1], a[1], b[1], b[1], b[1], b[1], c[1], c[1], c[1], c[1], d[1], d[1],
            d[1], d[1],
        ];
    }
    #[cfg(target_arch = "x86_64")]
    for (half, [low, high]) in weights.iter_mut().zip([[a, b], [c, d]]) {
        let (first, second) = half.split_at_mut(8);
        first.copy_from_slice(low); // splot-copy-ok: one cell's weights into its vector half
        second.copy_from_slice(high); // splot-copy-ok: one cell's weights into its vector half
    }
}

/// One class of a [`TapPairs`] filter: the weight of each pair, then the
/// center weight.
pub(crate) type LaneClass = ([i16; 16], i16);

/// The classes of the 4-column cells of a run.
#[derive(Clone, Copy)]
pub(crate) enum Classes<'a> {
    /// One class for every cell.
    Uniform(&'a LaneClass),
    /// Cell `k` of the run takes `table[cells[k] % table.len()]`.
    Cells {
        cells: &'a [u8],
        table: &'a [LaneClass],
    },
}

impl<'a> Classes<'a> {
    /// The classes of the four cells of group `group` of the run.
    #[inline]
    pub(crate) fn group(self, group: usize) -> Option<[&'a LaneClass; 4]> {
        match self {
            Self::Uniform(class) => Some([class; 4]),
            Self::Cells { cells, table } => {
                let ids = cells.get(4 * group..)?.first_chunk::<4>()?;
                let class = |id: u8| table.get(usize::from(id) % table.len().max(1));
                Some([
                    class(ids[0])?,
                    class(ids[1])?,
                    class(ids[2])?,
                    class(ids[3])?,
                ])
            }
        }
    }
}

/// The `pmaddwd` pattern of pair `pair` of `class` over the `pairs` tap
/// pairs of a filter; pair `pairs` weighs the center and the `Round2` by 7
/// offset.
#[inline]
pub(crate) fn pattern(class: &LaneClass, pairs: usize, pair: usize) -> MaddPattern {
    let [low, high] = match class.0.get(2 * pair..2 * pair + 2) {
        Some(&[low, high]) if pair < pairs => [low, high],
        _ => [class.1, 1 << 6],
    };
    [low, high, low, high, low, high, low, high]
}

/// The [`filter16`] weights of `F` for cells `0..4` of `classes`.
#[inline]
pub(crate) fn run_weights<F: TapPairs, T: LumaSimdSource>(
    classes: [&LaneClass; 4],
) -> [MaddWeights; 9] {
    #[cfg(target_arch = "x86_64")]
    if in_lane::<T>()
        && let Some(t) = fearless_simd::Level::baseline().as_avx2()
    {
        return in_lane_weights(t, classes, F::TAPS.len() / 2);
    }
    pattern_weights::<F, T>(classes)
}

/// [`run_weights`] from each cell's [`pattern`] of every pair.
#[inline]
fn pattern_weights<F: TapPairs, T: LumaSimdSource>(classes: [&LaneClass; 4]) -> [MaddWeights; 9] {
    let pairs = F::TAPS.len() / 2;
    let mut weights = [[[0; 16]; 2]; 9];
    for (pair, weights) in weights.iter_mut().enumerate().take(pairs + 1) {
        let cells: [MaddPattern; 4] =
            core::array::from_fn(|cell| pattern(classes[cell], pairs, pair));
        arrange_into::<T>(weights, cells.each_ref());
    }
    weights
}

/// [`run_weights`] in the in-lane arrangement: each 128-bit half of a pair's
/// weights broadcasts one 32-bit weight pair of its cell.
#[cfg(target_arch = "x86_64")]
#[inline]
fn in_lane_weights(
    t: fearless_simd::Avx2,
    classes: [&LaneClass; 4],
    pairs: usize,
) -> [MaddWeights; 9] {
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdInto};

    fearless_simd::kernel!(
        #[inline(always)]
        fn kernel(t: Avx2, classes: [&LaneClass; 4], pairs: usize) -> [MaddWeights; 9] {
            let load =
                |class: &LaneClass| -> __m256i { i16x16::load_array_ref(t, &class.0).into() };
            let center = |class: &LaneClass| i32::from(class.1 as u16) | (1 << 22);
            let store = |weights: &mut [i16; 16], lanes: __m256i| {
                let lanes: i16x16<Avx2> = lanes.simd_into(t);
                lanes.store_array(weights);
            };
            let mut weights = [[[0; 16]; 2]; 9];
            for (half, (low, high)) in [(classes[0], classes[2]), (classes[1], classes[3])]
                .into_iter()
                .enumerate()
            {
                let (x, y) = (load(low), load(high));
                let first = _mm256_permute2x128_si256::<0x20>(x, y);
                let last = _mm256_permute2x128_si256::<0x31>(x, y);
                macro_rules! pair {
                    ($($pair:literal $lanes:ident $imm:literal),*) => {$(
                        if $pair < pairs {
                            store(&mut weights[$pair][half], _mm256_shuffle_epi32::<$imm>($lanes));
                        }
                    )*};
                }
                pair!(0 first 0x00, 1 first 0x55, 2 first 0xaa, 3 first 0xff);
                pair!(4 last 0x00, 5 last 0x55, 6 last 0xaa, 7 last 0xff);
                if let Some(weights) = weights.get_mut(pairs) {
                    let (a, b) = (center(low), center(high));
                    store(
                        &mut weights[half],
                        _mm256_setr_epi32(a, a, a, a, b, b, b, b),
                    );
                }
            }
            weights
        }
    );
    kernel(t, classes, pairs)
}

/// Filters the 16-column groups from column `x` to `end` of `rows` output
/// rows, row `r` stored at `r * stride + x` of `output`, with the cell
/// `classes`. Window row `k` of output row `r` is `window[r + k]`, from the
/// group's column on. The [`by_lane`] kernel runs it on AArch64, the
/// [`filter16`] group kernel otherwise; returns `false` when a row, a class
/// or the output is short.
#[allow(clippy::too_many_arguments)]
#[inline]
pub(crate) fn filter_run<
    F: TapPairs,
    T: LumaSimdSource,
    O: LumaSimdOutput,
    const ROWS: usize,
    const REACH: usize,
>(
    window: &[&[T]],
    rows: usize,
    span: (usize, usize),
    classes: Classes<'_>,
    (output, stride): (&mut [O], usize),
    max_sample: u16,
) -> bool {
    #[cfg(target_arch = "aarch64")]
    if let Some(t) = fearless_simd::Level::baseline().as_neon() {
        let (window, out) = (T::lane_rows(window), O::lanes_mut(output));
        return F::by_lane(t, window, rows, span, classes, (out, stride), max_sample);
    }
    filter_run_groups::<F, T, O, ROWS, REACH>(
        window,
        rows,
        span,
        classes,
        (output, stride),
        max_sample,
    )
}

/// [`filter_run`] on the [`filter16`] group kernel, groups outer and rows
/// inner: the window and output rows are cut to the run once, so a group
/// checks one length per plane, and cell weights are built once per group.
#[inline]
fn filter_run_groups<
    F: TapPairs,
    T: LumaSimdSource,
    O: LumaSimdOutput,
    const ROWS: usize,
    const REACH: usize,
>(
    window: &[&[T]],
    rows: usize,
    (x, end): (usize, usize),
    classes: Classes<'_>,
    (output, stride): (&mut [O], usize),
    max_sample: u16,
) -> bool {
    const MAX_ROWS: usize = 16;
    let pairs = F::TAPS.len() / 2;
    let span = end.saturating_sub(x) & !15;
    if span == 0 || rows == 0 {
        return true;
    }
    let need = rows + ROWS - 1;
    let reach = span + REACH - 16;
    let Some(first) = window.first().and_then(|row| row.get(x..x + reach)) else {
        return false;
    };
    if need > MAX_ROWS || window.len() < need {
        return false;
    }
    let mut source = [first; MAX_ROWS];
    for (slot, row) in source.iter_mut().zip(window).take(need) {
        match row.get(x..x + reach) {
            Some(row) => *slot = row,
            None => return false,
        }
    }
    let mut targets: [&mut [O]; MAX_ROWS] = Default::default();
    let mut rest = output;
    for (r, slot) in targets.iter_mut().enumerate().take(rows) {
        let skip = if r == 0 {
            Some(x)
        } else {
            stride.checked_sub(span)
        };
        let Some(after) = skip
            .and_then(|skip| rest.get_mut(skip..))
            .filter(|after| after.len() >= span)
        else {
            return false;
        };
        (*slot, rest) = after.split_at_mut(span);
    }
    let uniform = match classes {
        Classes::Uniform(class) => Some(run_weights::<F, T>([class; 4])),
        Classes::Cells { .. } => None,
    };
    for (index, col) in (0..span).step_by(16).enumerate() {
        let cell_weights;
        let weights = match &uniform {
            Some(weights) => weights,
            None => match classes.group(index) {
                Some(cells) => {
                    cell_weights = run_weights::<F, T>(cells);
                    &cell_weights
                }
                None => return false,
            },
        };
        let Some(head) = first.get(col..).and_then(<[T]>::first_chunk::<REACH>) else {
            return false;
        };
        let mut group = [head; MAX_ROWS];
        for (slot, row) in group.iter_mut().zip(&source).take(need) {
            match row.get(col..).and_then(<[T]>::first_chunk::<REACH>) {
                Some(row) => *slot = row,
                None => return false,
            }
        }
        for (r, target) in targets.iter_mut().enumerate().take(rows) {
            let (Some(rows_of), Some(target)) = (
                group.get(r..).and_then(<[_]>::first_chunk::<ROWS>),
                target.get_mut(col..).and_then(<[O]>::first_chunk_mut::<16>),
            ) else {
                return false;
            };
            O::store16(
                filter16::<F, T, ROWS, REACH>(rows_of, &weights[..=pairs], max_sample),
                target,
            );
        }
    }
    true
}

/// The symmetric taps `(dy, dx)` paired with `(-dy, -dx)` of one filter.
pub(crate) trait TapPairs {
    /// Tap reach in rows and columns.
    const RADIUS: usize;
    /// Pair `j` of the symmetric taps.
    const TAPS: &'static [(isize, isize)];
    /// The x86 kernel over `u16` rows.
    #[cfg(target_arch = "x86_64")]
    fn avx2<const ROWS: usize, const REACH: usize>(
        t: fearless_simd::Avx2,
        window: &[&[u16; REACH]; ROWS],
        weights: &[MaddWeights],
        max_sample: u16,
    ) -> Option<[u16; 16]>;
    /// The [`by_lane`] run kernel.
    #[cfg(target_arch = "aarch64")]
    #[allow(clippy::too_many_arguments)]
    fn by_lane(
        t: fearless_simd::Neon,
        window: by_lane::Rows<'_>,
        rows: usize,
        span: (usize, usize),
        classes: Classes<'_>,
        output: (by_lane::Out<'_>, usize),
        max_sample: u16,
    ) -> bool;
}

/// Implements [`TapPairs::by_lane`] for a tap table whose windows are
/// `ROWS` rows of `REACH` columns.
#[cfg(target_arch = "aarch64")]
macro_rules! by_lane_kernel {
    ($taps:ty, $rows:literal, $reach:literal) => {
        fn by_lane(
            t: fearless_simd::Neon,
            window: $crate::lr_madd::by_lane::Rows<'_>,
            rows: usize,
            span: (usize, usize),
            classes: $crate::lr_madd::Classes<'_>,
            output: ($crate::lr_madd::by_lane::Out<'_>, usize),
            max_sample: u16,
        ) -> bool {
            use $crate::lr_madd::Classes;
            use $crate::lr_madd::by_lane::{Out, Rows, filter_run};

            fearless_simd::kernel!(
                #[inline]
                fn kernel(
                    t: Neon,
                    window: Rows<'_>,
                    rows: usize,
                    span: (usize, usize),
                    classes: Classes<'_>,
                    output: (Out<'_>, usize),
                    max_sample: u16,
                ) -> bool {
                    let (output, stride) = output;
                    macro_rules! run {
                        ($window:expr, $output:expr) => {
                            filter_run::<$taps, _, _, $rows, $reach>(
                                t,
                                $window,
                                rows,
                                span,
                                classes,
                                ($output, stride),
                                max_sample,
                            )
                        };
                    }
                    match (window, output) {
                        (Rows::Words(window), Out::Words(output)) => run!(window, output),
                        (Rows::Words(window), Out::Bytes(output)) => run!(window, output),
                        (Rows::Bytes(window), Out::Words(output)) => run!(window, output),
                        (Rows::Bytes(window), Out::Bytes(output)) => run!(window, output),
                    }
                }
            );
            kernel(t, window, rows, span, classes, output, max_sample)
        }
    };
}

#[cfg(target_arch = "aarch64")]
pub(crate) use by_lane_kernel;

/// The 16 filtered samples of one row from padded `window` rows, each slice
/// starting `RADIUS` columns left of the first sample.
#[allow(
    clippy::inline_always,
    reason = "measured: the row kernel must inline into its group loop"
)]
#[inline(always)]
pub(crate) fn filter16<F: TapPairs, T: LumaSimdSource, const ROWS: usize, const REACH: usize>(
    window: &[&[T; REACH]; ROWS],
    weights: &[MaddWeights],
    max_sample: u16,
) -> [u16; 16] {
    #[cfg(target_arch = "x86_64")]
    if let (Some(token), Some(rows)) = (
        fearless_simd::Level::baseline().as_avx2(),
        T::u16_rows(window),
    ) && let Some(values) = F::avx2(token, &rows, weights, max_sample)
    {
        return values;
    }
    lr_simd!(simd => portable::<_, F, T, ROWS, REACH>(simd, window, weights, max_sample))
}

#[allow(
    clippy::inline_always,
    reason = "measured: the row kernel must inline into its group loop"
)]
#[inline(always)]
fn portable<S: Simd, F: TapPairs, T: LumaSimdSource, const ROWS: usize, const REACH: usize>(
    simd: S,
    window: &[&[T; REACH]; ROWS],
    weights: &[MaddWeights],
    max_sample: u16,
) -> [u16; 16] {
    let r = F::RADIUS;
    let load = |row: usize, col: usize| -> i16x16<S> {
        window
            .get(row)
            .and_then(|row| row.get(col..))
            .and_then(<[T]>::first_chunk::<16>)
            .map_or_else(
                || i16x16::splat(simd, 0),
                |lanes| T::widen16(simd, lanes).bitcast(),
            )
    };
    let term = |j: usize| {
        let (dy, dx) = F::TAPS[j];
        load(r.wrapping_add_signed(dy), r.wrapping_add_signed(dx))
            + load(r.wrapping_add_signed(-dy), r.wrapping_add_signed(-dx))
    };
    #[cfg(not(target_arch = "x86_64"))]
    let mut sums = [i32x4::splat(simd, 0); 4];
    #[cfg(not(target_arch = "x86_64"))]
    let mut add_term = |j: usize, w: &[i16; 16]| {
        let (a_lo, a_hi) = simd.split_i16x16(term(j));
        let w_lo = fearless_simd::i16x8::from_slice(simd, &w[..8]);
        let w_hi = fearless_simd::i16x8::from_slice(simd, &w[8..]);
        let (p0, p1) = a_lo.widening_mul(w_lo);
        let (p2, p3) = a_hi.widening_mul(w_hi);
        sums[0] += p0;
        sums[1] += p1;
        sums[2] += p2;
        sums[3] += p3;
    };
    #[cfg(target_arch = "x86_64")]
    let mut sums = [i32x8::splat(simd, 0); 2];
    #[cfg(target_arch = "x86_64")]
    let mut add = |(low, high): (i16x16<S>, i16x16<S>), weights: &MaddWeights| {
        sums[0] += low.pairwise_dot_product(i16x16::load_array_ref(simd, &weights[0]));
        sums[1] += high.pairwise_dot_product(i16x16::load_array_ref(simd, &weights[1]));
    };
    let pairs = F::TAPS.len() / 2;
    macro_rules! pair_step {
        ($pair:literal) => {
            if $pair < pairs
                && let Some(weights) = weights.get($pair)
            {
                #[cfg(not(target_arch = "x86_64"))]
                {
                    add_term(2 * $pair, &weights[0]);
                    add_term(2 * $pair + 1, &weights[1]);
                }
                #[cfg(target_arch = "x86_64")]
                add(term(2 * $pair).interleave(term(2 * $pair + 1)), weights);
            }
        };
    }
    pair_step!(0);
    pair_step!(1);
    pair_step!(2);
    pair_step!(3);
    pair_step!(4);
    pair_step!(5);
    pair_step!(6);
    pair_step!(7);
    if let Some(weights) = weights.get(pairs) {
        #[cfg(not(target_arch = "x86_64"))]
        {
            let (a_lo, a_hi) = simd.split_i16x16(load(r, r));
            let w_lo = fearless_simd::i16x8::from_slice(simd, &weights[0][..8]);
            let w_hi = fearless_simd::i16x8::from_slice(simd, &weights[0][8..]);
            let (p0, p1) = a_lo.widening_mul(w_lo);
            let (p2, p3) = a_hi.widening_mul(w_hi);
            let round = i32x4::splat(simd, i32::from(weights[1][0]));
            sums[0] += p0 + round;
            sums[1] += p1 + round;
            sums[2] += p2 + round;
            sums[3] += p3 + round;
        }
        #[cfg(target_arch = "x86_64")]
        add(load(r, r).interleave(i16x16::splat(simd, 1)), weights);
    }
    #[cfg(not(target_arch = "x86_64"))]
    let sums = [
        simd.combine_i32x4(sums[0], sums[1]),
        simd.combine_i32x4(sums[2], sums[3]),
    ];
    let [low, high] =
        sums.map(|sum| (sum >> 7).clamp(i32x8::splat(simd, 0), i32x8::splat(simd, 65535)));
    let values: u16x16<S> = low.narrow(high).bitcast();
    values.min(u16x16::splat(simd, max_sample)).into()
}

/// Implements [`TapPairs::avx2`] for a tap table with one x86 kernel over
/// `ROWS` rows of `REACH` columns.
#[cfg(target_arch = "x86_64")]
macro_rules! madd_avx2 {
    ($taps:ty, $rows:literal, $reach:literal) => {
        #[allow(
            clippy::inline_always,
            reason = "measured: the row kernel must inline into its group loop"
        )]
        #[inline(always)]
        fn avx2<const ROWS: usize, const REACH: usize>(
            t: fearless_simd::Avx2,
            window: &[&[u16; REACH]; ROWS],
            weights: &[$crate::lr_madd::MaddWeights],
            max_sample: u16,
        ) -> Option<[u16; 16]> {
            use core::arch::x86_64::*;
            use fearless_simd::{Avx2, SimdBase, SimdInto, i16x16, u16x16};
            use $crate::lr_madd::MaddWeights;

            fearless_simd::kernel!(
                #[inline(always)]
                fn kernel(
                    t: Avx2,
                    window: &[&[u16; $reach]; $rows],
                    weights: &[MaddWeights],
                    max_sample: u16,
                ) -> [u16; 16] {
                    let r = <$taps as $crate::lr_madd::TapPairs>::RADIUS;
                    let taps = <$taps as $crate::lr_madd::TapPairs>::TAPS;
                    let load = |row: usize, col: usize| -> __m256i {
                        match window
                            .get(row)
                            .and_then(|row| row.get(col..))
                            .and_then(<[u16]>::first_chunk::<16>)
                        {
                            Some(lanes) => u16x16::load_array_ref(t, lanes).into(),
                            None => _mm256_setzero_si256(),
                        }
                    };
                    let term = |j: usize| {
                        let (dy, dx) = taps[j];
                        _mm256_add_epi16(
                            load(r.wrapping_add_signed(dy), r.wrapping_add_signed(dx)),
                            load(r.wrapping_add_signed(-dy), r.wrapping_add_signed(-dx)),
                        )
                    };
                    let half = |weights: &MaddWeights, half: usize| -> __m256i {
                        i16x16::load_array_ref(t, &weights[half]).into()
                    };
                    let mut sums = [_mm256_setzero_si256(); 2];
                    let mut add = |a: __m256i, b: __m256i, weights: &MaddWeights| {
                        sums[0] = _mm256_add_epi32(
                            sums[0],
                            _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), half(weights, 0)),
                        );
                        sums[1] = _mm256_add_epi32(
                            sums[1],
                            _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), half(weights, 1)),
                        );
                    };
                    let pairs = taps.len() / 2;
                    for (pair, weights) in weights.iter().enumerate().take(pairs) {
                        add(term(2 * pair), term(2 * pair + 1), weights);
                    }
                    if let Some(weights) = weights.get(pairs) {
                        add(load(r, r), _mm256_set1_epi16(1), weights);
                    }
                    let values = _mm256_min_epu16(
                        _mm256_packus_epi32(
                            _mm256_srai_epi32::<7>(sums[0]),
                            _mm256_srai_epi32::<7>(sums[1]),
                        ),
                        _mm256_set1_epi16(max_sample as i16),
                    );
                    let values: u16x16<Avx2> = values.simd_into(t);
                    values.into()
                }
            );

            if ROWS != $rows {
                return None;
            }
            let mut rows: [&[u16; $reach]; $rows] = [&[0; $reach]; $rows];
            for (slot, row) in rows.iter_mut().zip(window) {
                *slot = row.as_slice().first_chunk::<$reach>()?;
            }
            Some(kernel(t, &rows, weights, max_sample))
        }
    };
}

#[cfg(target_arch = "x86_64")]
pub(crate) use madd_avx2;

/// The AArch64 kernels for runs of 16-column groups that share one class,
/// and for runs of groups whose 4-column cells take classes of their own.
///
/// The weights stay in registers for the whole run, or for every row of a
/// group of cells; each tap pair is its symmetric sum times a weight lane
/// (one `smlal` per 4-output cell), and a uniform run whose last pairs weigh
/// zero takes a variant that stops before them: the far taps that end both
/// tap tables are the ones classes zero most. Tap sums of samples validated
/// against a `BitDepth` of at most 10 (§ 6 Table 6.3) stay below `2^11`, so
/// they are `i16` lanes; `sqrshrun` by 7 is `Round2` and the clip to `0`,
/// `umin` the clip to the maximum sample.
#[cfg(target_arch = "aarch64")]
pub(crate) mod by_lane {
    use core::arch::aarch64::*;
    use fearless_simd::{Neon, Simd, SimdBase, SimdFrom, i16x8, u16x8};

    use super::{Classes, LaneClass as Class, LumaSimdSource, TapPairs};

    /// Filters a [`super::filter_run`] run.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    #[target_feature(enable = "neon")]
    pub(crate) fn filter_run<
        F: TapPairs,
        T: LumaSimdSource,
        O: Store,
        const ROWS: usize,
        const REACH: usize,
    >(
        t: Neon,
        window: &[&[T]],
        rows: usize,
        span: (usize, usize),
        classes: Classes<'_>,
        output: (&mut [O], usize),
        max_sample: u16,
    ) -> bool {
        let class = match classes {
            Classes::Uniform(class) => class,
            Classes::Cells { .. } => {
                return cells::<F, T, O, ROWS, REACH>(
                    t, window, rows, span, classes, output, max_sample,
                );
            }
        };
        let pairs = F::TAPS.len().min(16);
        let used = class.0[..pairs]
            .iter()
            .rposition(|&weight| weight != 0)
            .map_or(0, |last| last + 1);
        macro_rules! variant {
            ($used:literal) => {
                run::<F, T, O, $used, ROWS, REACH>(t, window, rows, span, class, output, max_sample)
            };
        }
        match used {
            0..=6 => variant!(6),
            7..=10 => variant!(10),
            11 | 12 => variant!(12),
            13 | 14 => variant!(14),
            _ => variant!(16),
        }
    }

    /// Adds pair `$j` of `$F` when it is below `$used` to the cell sums.
    macro_rules! pair_term {
        ($F:ty, $used:expr, $j:literal, $load:ident, $hide:ident, $tap:ident, $low:ident, $high:ident, $acc:ident) => {
            if $j < <$F>::TAPS.len() && $j < $used {
                const L: i32 = $j % 8;
                let (dy, dx) = <$F>::TAPS[$j];
                let (p, q) = (($tap(dy), $tap(dx)), ($tap(-dy), $tap(-dx)));
                let (a, b) = ($load(p.0, p.1), $load(q.0, q.1));
                let [s0, s1] = $hide([vaddq_s16(a[0], b[0]), vaddq_s16(a[1], b[1])]);
                let w = if $j < 8 { $low } else { $high };
                $acc[0] = vmlal_laneq_s16::<L>($acc[0], vget_low_s16(s0), w[0]);
                $acc[1] = vmlal_high_laneq_s16::<L>($acc[1], s0, w[1]);
                $acc[2] = vmlal_laneq_s16::<L>($acc[2], vget_low_s16(s1), w[2]);
                $acc[3] = vmlal_high_laneq_s16::<L>($acc[3], s1, w[3]);
            }
        };
    }

    /// The 16 filtered samples of one row of a group: `$load(row, col)`
    /// reads 16 window samples, `$low` and `$high` hold pairs `0..8` and
    /// `8..16` of each cell, lane `k` of `$center` its center weight.
    macro_rules! group16 {
        ($F:ty, $used:expr, ($load:expr, $hide:expr), $low:expr, $high:expr, $center:expr, $max:expr) => {{
            let (load, hide) = ($load, $hide);
            let (low, high, center): (&[int16x8_t; 4], &[int16x8_t; 4], int16x8_t) =
                ($low, $high, $center);
            let tap = |offset: isize| <$F>::RADIUS.wrapping_add_signed(offset);
            let [m0, m1] = hide(load(tap(0), tap(0)));
            let mut acc = [
                vmull_laneq_s16::<0>(vget_low_s16(m0), center),
                vmull_high_laneq_s16::<1>(m0, center),
                vmull_laneq_s16::<2>(vget_low_s16(m1), center),
                vmull_high_laneq_s16::<3>(m1, center),
            ];
            pair_term!($F, $used, 0, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 1, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 2, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 3, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 4, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 5, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 6, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 7, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 8, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 9, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 10, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 11, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 12, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 13, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 14, load, hide, tap, low, high, acc);
            pair_term!($F, $used, 15, load, hide, tap, low, high, acc);
            let narrow =
                |a, b| vminq_u16(vqrshrun_high_n_s32::<7>(vqrshrun_n_s32::<7>(a), b), $max);
            [narrow(acc[0], acc[1]), narrow(acc[2], acc[3])]
        }};
    }

    /// The weights of the four cells of a group: pairs `0..8`, pairs
    /// `8..16`, and lane `k` the center weight of cell `k`.
    #[inline]
    #[target_feature(enable = "neon")]
    fn weights(t: Neon, cells: [&Class; 4]) -> ([int16x8_t; 4], [int16x8_t; 4], int16x8_t) {
        let half = |class: &Class, at: usize| -> int16x8_t {
            match class.0.get(at..).and_then(<[i16]>::first_chunk::<8>) {
                Some(lanes) => i16x8::load_array_ref(t, lanes).into(),
                None => vdupq_n_s16(0),
            }
        };
        (
            cells.map(|class| half(class, 0)),
            cells.map(|class| half(class, 8)),
            i16x8::load_array(t, core::array::from_fn(|k| cells[k % 4].1)).into(),
        )
    }

    /// 16 samples from `col` of `row` of `group`, zero past the window.
    #[allow(clippy::inline_always, reason = "one load per tap")]
    #[inline(always)]
    fn lanes<T: LumaSimdSource, const ROWS: usize, const REACH: usize>(
        t: Neon,
        group: &[&[T; REACH]; ROWS],
        (row, col): (usize, usize),
    ) -> [int16x8_t; 2] {
        match group
            .get(row)
            .and_then(|row| row.get(col..))
            .and_then(<[T]>::first_chunk::<16>)
        {
            Some(lanes) => T::lanes16(t, lanes),
            None => [i16x8::splat(t, 0).into(); 2],
        }
    }

    /// Zero lanes LLVM cannot see as zero: 8-bit tap sums it can prove
    /// non-negative it widens to `i32` multiplies instead of `smlal`.
    #[inline]
    fn opaque_zero(t: Neon) -> i16x8<Neon> {
        i16x8::splat(t, core::hint::black_box(0))
    }

    /// A uniform run: rows outer, groups inner.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    #[target_feature(enable = "neon")]
    fn run<
        F: TapPairs,
        T: LumaSimdSource,
        O: Store,
        const USED: usize,
        const ROWS: usize,
        const REACH: usize,
    >(
        t: Neon,
        window: &[&[T]],
        rows: usize,
        (x, end): (usize, usize),
        class: &Class,
        (output, stride): (&mut [O], usize),
        max_sample: u16,
    ) -> bool {
        let Some(first) = window.first() else {
            return false;
        };
        let (low, high, center) = weights(t, [class; 4]);
        let (max, pad) = (vdupq_n_u16(max_sample), opaque_zero(t));
        let reach = (end - x + REACH).saturating_sub(16);
        for r in 0..rows {
            let (Some(window_rows), Some(target)) = (
                window.get(r..).and_then(<[_]>::first_chunk::<ROWS>),
                output.get_mut(r * stride + x..r * stride + end),
            ) else {
                return false;
            };
            let mut source = [*first; ROWS];
            for (slot, row) in source.iter_mut().zip(window_rows) {
                match row.get(x..x + reach) {
                    Some(row) => *slot = row,
                    None => return false,
                }
            }
            for target in target.chunks_exact_mut(16) {
                let Some(row) = source.first().and_then(|row| row.first_chunk::<REACH>()) else {
                    return false;
                };
                let mut group = [row; ROWS];
                for (slot, row) in group.iter_mut().zip(&mut source) {
                    match (row.first_chunk::<REACH>(), row.get(16..)) {
                        (Some(window), Some(rest)) => (*slot, *row) = (window, rest),
                        _ => return false,
                    }
                }
                let Some(target) = target.first_chunk_mut::<16>() else {
                    return false;
                };
                let load = |row: usize, col: usize| lanes(t, &group, (row, col));
                let hide = |lanes| T::hide(t, lanes, pad);
                let values = group16!(F, USED, (load, hide), &low, &high, center, max);
                O::store16(t, values.map(|half| u16x8::simd_from(t, half)), target);
            }
        }
        true
    }

    /// A run of groups whose cells take classes of their own: groups outer,
    /// rows inner, each group's weights loaded once.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    #[target_feature(enable = "neon")]
    fn cells<F: TapPairs, T: LumaSimdSource, O: Store, const ROWS: usize, const REACH: usize>(
        t: Neon,
        window: &[&[T]],
        rows: usize,
        (x, end): (usize, usize),
        classes: Classes<'_>,
        (output, stride): (&mut [O], usize),
        max_sample: u16,
    ) -> bool {
        let (max, pad) = (vdupq_n_u16(max_sample), opaque_zero(t));
        let reach = rows + ROWS - 1;
        let (mut col, mut index) = (x, 0);
        while col + 16 <= end {
            let (Some(cells), Some(first)) = (
                classes.group(index),
                window
                    .first()
                    .and_then(|row| row.get(col..))
                    .and_then(<[T]>::first_chunk::<REACH>),
            ) else {
                return false;
            };
            let (low, high, center) = weights(t, cells);
            let mut rows_of_group = [first; 16];
            if window.len() < reach || reach > rows_of_group.len() {
                return false;
            }
            for (slot, row) in rows_of_group.iter_mut().zip(window).take(reach) {
                match row.get(col..).and_then(<[T]>::first_chunk::<REACH>) {
                    Some(row) => *slot = row,
                    None => return false,
                }
            }
            for r in 0..rows {
                let (Some(group), Some(target)) = (
                    rows_of_group.get(r..).and_then(<[_]>::first_chunk::<ROWS>),
                    output
                        .get_mut(r * stride + col..)
                        .and_then(<[O]>::first_chunk_mut::<16>),
                ) else {
                    return false;
                };
                let load = |row: usize, at: usize| lanes(t, group, (row, at));
                let hide = |lanes| T::hide(t, lanes, pad);
                let values = group16!(F, 16, (load, hide), &low, &high, center, max);
                O::store16(t, values.map(|half| u16x8::simd_from(t, half)), target);
            }
            col += 16;
            index += 1;
        }
        true
    }

    /// Output samples of the run kernels.
    pub(crate) trait Store: Copy {
        /// Stores 16 values within the sample range.
        fn store16(t: Neon, values: [u16x8<Neon>; 2], output: &mut [Self; 16]);
    }

    impl Store for u16 {
        #[allow(clippy::inline_always, reason = "one store per kernel group")]
        #[inline(always)]
        fn store16(_t: Neon, [low, high]: [u16x8<Neon>; 2], output: &mut [Self; 16]) {
            let (first, second) = output.split_at_mut(8);
            if let (Some(first), Some(second)) =
                (first.first_chunk_mut::<8>(), second.first_chunk_mut::<8>())
            {
                low.store_array(first);
                high.store_array(second);
            }
        }
    }

    impl Store for u8 {
        #[allow(clippy::inline_always, reason = "one store per kernel group")]
        #[inline(always)]
        fn store16(t: Neon, [low, high]: [u16x8<Neon>; 2], output: &mut [Self; 16]) {
            t.narrow_u16x8(low, high).store_array(output);
        }
    }

    /// The output of a run: `u16` or `u8` samples.
    pub(crate) enum Out<'a> {
        Words(&'a mut [u16]),
        Bytes(&'a mut [u8]),
    }

    /// The source rows of a run: `u16` or `u8` samples.
    #[derive(Clone, Copy)]
    pub(crate) enum Rows<'a> {
        Words(&'a [&'a [u16]]),
        Bytes(&'a [&'a [u8]]),
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::{LaneClass, TapPairs, in_lane_weights, pattern_weights};

    struct Twelve;
    impl TapPairs for Twelve {
        super::madd_avx2!(Twelve, 7, 22);
        const RADIUS: usize = 3;
        const TAPS: &'static [(isize, isize)] = &[(0, 1); 12];
    }

    struct Sixteen;
    impl TapPairs for Sixteen {
        super::madd_avx2!(Sixteen, 9, 24);
        const RADIUS: usize = 4;
        const TAPS: &'static [(isize, isize)] = &[(0, 1); 16];
    }

    fn check<F: TapPairs>(next: &mut impl FnMut() -> i16) {
        let Some(t) = fearless_simd::Level::baseline().as_avx2() else {
            return;
        };
        for _ in 0..200 {
            let classes: [LaneClass; 4] =
                core::array::from_fn(|_| (core::array::from_fn(|_| next()), next()));
            let cells = classes.each_ref();
            assert_eq!(
                in_lane_weights(t, cells, F::TAPS.len() / 2),
                pattern_weights::<F, u16>(cells)
            );
        }
    }

    /// The in-lane weights match the per-pair patterns for random and
    /// extreme classes.
    #[test]
    fn in_lane_weights_match_the_patterns() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            match state % 4 {
                0 => i16::MIN,
                1 => i16::MAX,
                _ => (state >> 16) as i16,
            }
        };
        check::<Twelve>(&mut next);
        check::<Sixteen>(&mut next);
    }
}
