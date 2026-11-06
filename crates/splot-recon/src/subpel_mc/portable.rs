// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The unscaled § 7.13.3.18 convolution on 128-bit `i16` lanes, for every
//! sample type and block width.
//!
//! A vector holds eight outputs of a row. Each tap pair interleaves two
//! windows of the row, or two rows, into one multiply-add, so every sum is
//! exact in `i32`. A zero phase skips its pass: `Round2(128 * s, 3) == s << 4`
//! and `Round2(Round2(x, 3) << 7, 11) == Round2(Round2(x, 3), 4)` keep the
//! two-pass value. § 6 Table 6.3 admits only 8- and 10-bit samples, so a
//! horizontal value fits `i16`.

use super::block::{Block, Sink, Taps};
use fearless_simd::{Simd, i16x8, i16x16, i32x4, i32x8, prelude::*, u8x16, u16x8};

/// Samples the kernels widen to `i16` lanes.
pub(super) trait Lanes: Copy {
    /// Eight samples from `at`, zero past the slice.
    fn lanes<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S>;
}

impl Lanes for u8 {
    #[inline]
    fn lanes<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S> {
        match samples.get(at..).and_then(<[u8]>::first_chunk::<16>) {
            Some(bytes) => u8x16::load_array_ref(simd, bytes).widen().0.bitcast(),
            None => i16x8::from_fn(simd, |lane| {
                samples
                    .get(at + lane)
                    .map_or(0, |&sample| i16::from(sample))
            }),
        }
    }
}

impl Lanes for u16 {
    #[inline]
    fn lanes<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S> {
        match samples.get(at..).and_then(<[u16]>::first_chunk::<8>) {
            Some(lanes) => u16x8::load_array_ref(simd, lanes).bitcast(),
            None => i16x8::from_fn(simd, |lane| {
                samples.get(at + lane).map_or(0, |&sample| sample as i16)
            }),
        }
    }
}

/// Calls `$f::<P>` for the run-time pair count.
macro_rules! with_pairs {
    ($pairs:expr, $f:ident::<$($ty:ty),*>($($arg:expr),*)) => {
        match $pairs {
            1 => $f::<$($ty,)* 1>($($arg),*),
            2 => $f::<$($ty,)* 2>($($arg),*),
            3 => $f::<$($ty,)* 3>($($arg),*),
            _ => $f::<$($ty,)* 4>($($arg),*),
        }
    };
}

/// Predicts the block into `sink`; `stage` holds every intermediate row.
pub(super) fn predict<T: Lanes>(block: &Block<'_, T>, sink: &mut Sink<'_>, stage: &mut [i16]) {
    baseline_simd!(simd => run(simd, block, sink, stage));
}

/// Eight output columns of one row: a job. A vector runs two jobs, one per
/// 128-bit half, so every lane operation stays inside its half.
#[derive(Clone, Copy)]
struct Jobs {
    chunks: usize,
    count: usize,
}

impl Jobs {
    const fn new(w: usize, rows: usize) -> Self {
        let chunks = w.div_ceil(8);
        Self {
            chunks,
            count: rows * chunks,
        }
    }

    /// The job pairs as job indices and coordinates; the second is `None`
    /// past the last job.
    fn pairs(self) -> impl Iterator<Item = (usize, usize, (usize, usize), Option<(usize, usize)>)> {
        let mut cursor = (0, 0);
        let step = move |(row, col): (usize, usize)| {
            if col + 8 < self.chunks * 8 {
                (row, col + 8)
            } else {
                (row + 1, 0)
            }
        };
        (0..self.count).step_by(2).map(move |job| {
            let first = cursor;
            let second = step(first);
            cursor = step(second);
            let second_job = if job + 1 < self.count { job + 1 } else { job };
            (
                job,
                second_job,
                first,
                (job + 1 < self.count).then_some(second),
            )
        })
    }
}

#[inline]
fn run<S: Simd, T: Lanes>(simd: S, block: &Block<'_, T>, sink: &mut Sink<'_>, stage: &mut [i16]) {
    match (block.horizontal, block.vertical) {
        (None, None) => copy(simd, block, sink),
        (Some(taps), None) => with_pairs!(taps.count, horizontal::<S, T>(simd, block, &taps, sink)),
        (None, Some(taps)) => with_pairs!(taps.count, vertical::<S, T>(simd, block, &taps, sink)),
        (Some(h_taps), Some(v_taps)) => {
            let rows = block.h + v_taps.span() - 1;
            with_pairs!(
                h_taps.count,
                stage_rows::<S, T>(simd, block, &h_taps, rows, stage)
            );
            with_pairs!(
                v_taps.count,
                filter_stage::<S>(simd, block.w, block.h, &v_taps, stage, sink)
            );
        }
    }
}

/// The pair weights of `taps` as `i16` lane pairs.
#[inline]
fn weights<S: Simd>(simd: S, taps: &Taps) -> [i16x16<S>; 4] {
    taps.full.map(|pair| i32x8::splat(simd, pair).bitcast())
}

/// The two jobs' eight lanes from `at`, one per half.
#[inline]
fn job_lanes<S: Simd, T: Lanes>(simd: S, source: &[T], first: usize, second: usize) -> i16x16<S> {
    T::lanes(simd, source, first).combine(T::lanes(simd, source, second))
}

/// Interleaves the low (`HIGH == false`) or high four lanes of each half of
/// `a` and `b`.
#[inline]
fn zip_halves<S: Simd, const HIGH: bool>(a: i16x16<S>, b: i16x16<S>) -> i16x16<S> {
    let ((a0, a1), (b0, b1)) = (a.split(), b.split());
    if HIGH {
        a0.zip_high(b0).combine(a1.zip_high(b1))
    } else {
        a0.zip_low(b0).combine(a1.zip_low(b1))
    }
}

/// `Σ` over `P` pairs of the interleaved `rows(2j)`, `rows(2j + 1)` times the
/// pair weights: each half's lanes `0..4` and `4..8`.
#[inline]
fn pair_sums<S: Simd, const P: usize>(
    simd: S,
    weights: &[i16x16<S>; 4],
    rows: impl Fn(usize) -> i16x16<S>,
) -> (i32x8<S>, i32x8<S>) {
    let (mut low, mut high) = (i32x8::splat(simd, 0), i32x8::splat(simd, 0));
    for (pair, &weight) in weights.iter().enumerate().take(P) {
        let (a, b) = (rows(2 * pair), rows(2 * pair + 1));
        low += zip_halves::<S, false>(a, b).pairwise_dot_product(weight);
        high += zip_halves::<S, true>(a, b).pairwise_dot_product(weight);
    }
    (low, high)
}

/// The horizontal sums of two jobs whose windows start at `first` and
/// `second`.
#[inline]
fn horizontal_sums<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    source: &[T],
    (first, second): (usize, usize),
    weights: &[i16x16<S>; 4],
) -> (i32x8<S>, i32x8<S>) {
    let a = job_lanes(simd, source, first, second);
    let b = job_lanes(simd, source, first + 8, second + 8);
    let windows = [
        a,
        a.slide_within_blocks::<1>(b),
        a.slide_within_blocks::<2>(b),
        a.slide_within_blocks::<3>(b),
        a.slide_within_blocks::<4>(b),
        a.slide_within_blocks::<5>(b),
        a.slide_within_blocks::<6>(b),
        a.slide_within_blocks::<7>(b),
    ];
    pair_sums::<S, P>(simd, weights, |window| windows[window])
}

#[inline]
fn round<S: Simd>(
    (low, high): (i32x8<S>, i32x8<S>),
    bias: i32,
    shift: u32,
) -> (i32x8<S>, i32x8<S>) {
    ((low + bias) >> shift, (high + bias) >> shift)
}

/// Stores the two jobs' values: each half of `low` and `high` is one job's
/// lanes `0..4` and `4..8`.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn emit_pair<S: Simd>(
    simd: S,
    sink: &mut Sink<'_>,
    w: usize,
    jobs: [Option<(usize, usize)>; 2],
    (low, high): (i32x8<S>, i32x8<S>),
) {
    let ((low0, low1), (high0, high1)) = (low.split(), high.split());
    if let Some((row, col)) = jobs[0] {
        emit(simd, sink, w, row, col, (low0, high0));
    }
    if let Some((row, col)) = jobs[1] {
        emit(simd, sink, w, row, col, (low1, high1));
    }
}

#[inline]
fn copy<S: Simd, T: Lanes>(simd: S, block: &Block<'_, T>, sink: &mut Sink<'_>) {
    let shift = if sink.is_put() { 0 } else { 4 };
    let jobs = Jobs::new(block.w, block.h);
    let start = |(row, col): (usize, usize)| block.origin + row * block.stride + col;
    for (_, _, a, second) in jobs.pairs() {
        let b = second.unwrap_or(a);
        let (low, high) = job_lanes(simd, block.source, start(a), start(b)).widen();
        let (first_lanes, second_lanes) = ((low << shift).split(), (high << shift).split());
        emit(simd, sink, block.w, a.0, a.1, first_lanes);
        if second.is_some() {
            emit(simd, sink, block.w, b.0, b.1, second_lanes);
        }
    }
}

#[inline]
fn horizontal<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    block: &Block<'_, T>,
    taps: &Taps,
    sink: &mut Sink<'_>,
) {
    let weights = weights(simd, taps);
    let put = sink.is_put();
    let jobs = Jobs::new(block.w, block.h);
    let start = |(row, col): (usize, usize)| block.origin + row * block.stride + col;
    for (_, _, a, second) in jobs.pairs() {
        let b = second.unwrap_or(a);
        let sums = horizontal_sums::<S, T, P>(simd, block.source, (start(a), start(b)), &weights);
        let intermediate = round(sums, 4, 3);
        let values = if put {
            round(intermediate, 8, 4)
        } else {
            intermediate
        };
        emit_pair(simd, sink, block.w, [Some(a), second], values);
    }
}

#[inline]
fn vertical<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    block: &Block<'_, T>,
    taps: &Taps,
    sink: &mut Sink<'_>,
) {
    let weights = weights(simd, taps);
    let (bias, shift) = if sink.is_put() { (64, 7) } else { (4, 3) };
    let jobs = Jobs::new(block.w, block.h);
    let start = |(row, col): (usize, usize)| block.origin + row * block.stride + col;
    for (_, _, a, second) in jobs.pairs() {
        let b = second.unwrap_or(a);
        let line = |k: usize| {
            let step = k * block.stride;
            job_lanes(simd, block.source, start(a) + step, start(b) + step)
        };
        let sums = pair_sums::<S, P>(simd, &weights, line);
        emit_pair(
            simd,
            sink,
            block.w,
            [Some(a), second],
            round(sums, bias, shift),
        );
    }
}

/// The horizontal pass: `rows` intermediate rows, eight lanes per job, into
/// `stage` in job order.
#[inline]
fn stage_rows<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    block: &Block<'_, T>,
    taps: &Taps,
    rows: usize,
    stage: &mut [i16],
) {
    let weights = weights(simd, taps);
    let jobs = Jobs::new(block.w, rows);
    let start = |(row, col): (usize, usize)| block.origin + row * block.stride + col;
    for ((_, _, a, second), slot) in jobs.pairs().zip(stage.chunks_exact_mut(16)) {
        let b = second.unwrap_or(a);
        let (low, high) = round(
            horizontal_sums::<S, T, P>(simd, block.source, (start(a), start(b)), &weights),
            4,
            3,
        );
        let ((low0, low1), (high0, high1)) = (low.split(), high.split());
        if let Some(slot) = slot.first_chunk_mut::<16>() {
            low0.narrow(high0)
                .combine(low1.narrow(high1))
                .store_array(slot);
        }
    }
}

/// The vertical pass over [`stage_rows`].
#[inline]
fn filter_stage<S: Simd, const P: usize>(
    simd: S,
    w: usize,
    h: usize,
    taps: &Taps,
    stage: &[i16],
    sink: &mut Sink<'_>,
) {
    let weights = weights(simd, taps);
    let (bias, shift) = if sink.is_put() {
        (1 << 10, 11)
    } else {
        (1 << 6, 7)
    };
    let jobs = Jobs::new(w, h);
    let zero = [0; 8];
    let lanes = |job: usize| {
        let lanes = stage.get(job * 8..).and_then(<[i16]>::first_chunk::<8>);
        i16x8::load_array_ref(simd, lanes.unwrap_or(&zero))
    };
    for (first, second_job, a, second) in jobs.pairs() {
        let line = |k: usize| {
            let step = k * jobs.chunks;
            lanes(first + step).combine(lanes(second_job + step))
        };
        let sums = pair_sums::<S, P>(simd, &weights, line);
        let pair = [Some(a), second];
        emit_pair(simd, sink, w, pair, round(sums, bias, shift));
    }
}

/// Stores the eight values of output row `row`, columns `col..`, of a
/// `w`-wide block; columns past `w` are dropped.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn emit<S: Simd>(
    simd: S,
    sink: &mut Sink<'_>,
    w: usize,
    row: usize,
    col: usize,
    (low, high): (i32x4<S>, i32x4<S>),
) {
    let count = (w - col).min(8);
    let clipped = |low: i32x4<S>, high: i32x4<S>, max: i32| {
        low.max(0)
            .min(max)
            .narrow(high.max(0).min(max))
            .bitcast::<u16x8<S>>()
    };
    let blended = |pred0: &[i32], forward: i32| {
        let at = row * w + col;
        let first =
            |offset: usize| match pred0.get(at + offset..).and_then(<[i32]>::first_chunk::<4>) {
                Some(lanes) if count == 8 => i32x4::load_array_ref(simd, lanes),
                _ => i32x4::from_fn(simd, |lane| {
                    if offset + lane < count {
                        pred0.get(at + offset + lane).copied().unwrap_or(0)
                    } else {
                        0
                    }
                }),
            };
        let blend = |first: i32x4<S>, second: i32x4<S>| {
            if forward == 8 {
                (first + second + 16) >> 5
            } else {
                (first * forward + second * (16 - forward) + 128) >> 8
            }
        };
        (blend(first(0), low), blend(first(4), high))
    };
    match sink {
        Sink::Put { output, stride } => {
            let lanes = clipped(low, high, 255).to_array();
            store(
                output,
                row * *stride + col,
                count,
                lanes.map(|lane| lane as u8),
            );
        }
        Sink::PutWide {
            output,
            stride,
            max,
        } => store_wide(
            output,
            row * *stride + col,
            count,
            clipped(low, high, i32::from(*max)),
        ),
        Sink::Prep { output, stride } => {
            let at = row * *stride + col;
            match output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<8>) {
                Some(slot) if count == 8 => {
                    let (first, second) = slot.split_at_mut(4);
                    if let (Some(first), Some(second)) =
                        (first.first_chunk_mut::<4>(), second.first_chunk_mut::<4>())
                    {
                        low.store_array(first);
                        high.store_array(second);
                    }
                }
                _ => {
                    let (low, high) = (low.to_array(), high.to_array());
                    let lanes =
                        core::array::from_fn(
                            |lane| {
                                if lane < 4 { low[lane] } else { high[lane - 4] }
                            },
                        );
                    store(output, at, count, lanes);
                }
            }
        }
        Sink::Average {
            pred0,
            forward,
            output,
            stride,
        } => {
            let (first, second) = blended(pred0, *forward);
            let lanes = clipped(first, second, 255).to_array();
            store(
                output,
                row * *stride + col,
                count,
                lanes.map(|lane| lane as u8),
            );
        }
        Sink::AverageWide {
            pred0,
            forward,
            output,
            stride,
            max,
        } => {
            let (first, second) = blended(pred0, *forward);
            store_wide(
                output,
                row * *stride + col,
                count,
                clipped(first, second, i32::from(*max)),
            );
        }
    }
}

/// Stores the first `count` of `lanes` at `at`.
#[inline]
fn store_wide<S: Simd>(output: &mut [u16], at: usize, count: usize, lanes: u16x8<S>) {
    match output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>) {
        Some(slot) if count == 8 => lanes.store_array(slot),
        _ => store(output, at, count, lanes.to_array()),
    }
}

/// Stores the first `count` of `lanes` at `at`.
#[inline]
fn store<T: Copy>(output: &mut [T], at: usize, count: usize, lanes: [T; 8]) {
    if count == 8
        && let Some(slot) = output.get_mut(at..).and_then(<[T]>::first_chunk_mut::<8>)
    {
        *slot = lanes;
        return;
    }
    for (slot, lane) in output.iter_mut().skip(at).zip(lanes).take(count) {
        *slot = lane;
    }
}
