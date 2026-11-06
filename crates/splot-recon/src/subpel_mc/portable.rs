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
use fearless_simd::{Simd, i16x8, i32x4, prelude::*, u8x16, u16x8};

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

/// Predicts the block into `sink`; `stage` holds eight columns of every
/// intermediate row.
pub(super) fn predict<T: Lanes>(block: &Block<'_, T>, sink: &mut Sink<'_>, stage: &mut [i16]) {
    baseline_simd!(simd => run(simd, block, sink, stage));
}

#[inline]
fn run<S: Simd, T: Lanes>(simd: S, block: &Block<'_, T>, sink: &mut Sink<'_>, stage: &mut [i16]) {
    match (block.horizontal, block.vertical) {
        (None, None) => copy(simd, block, sink),
        (Some(taps), None) => with_pairs!(taps.count, horizontal::<S, T>(simd, block, &taps, sink)),
        (None, Some(taps)) => with_pairs!(taps.count, vertical::<S, T>(simd, block, &taps, sink)),
        (Some(h_taps), Some(v_taps)) => {
            let rows = block.h + v_taps.span() - 1;
            for col in (0..block.w).step_by(8) {
                with_pairs!(
                    h_taps.count,
                    stage_rows::<S, T>(simd, block, &h_taps, col, rows, stage)
                );
                with_pairs!(
                    v_taps.count,
                    filter_stage::<S>(simd, block.w, block.h, &v_taps, col, stage, sink)
                );
            }
        }
    }
}

/// The pair weights of `taps` as `i16` lane pairs.
#[inline]
fn weights<S: Simd>(simd: S, taps: &Taps) -> [i16x8<S>; 4] {
    taps.full.map(|pair| i32x4::splat(simd, pair).bitcast())
}

/// `Σ` over `P` pairs of the interleaved `rows(2j)`, `rows(2j + 1)` times the
/// pair weights, for lanes `0..4` and `4..8`.
#[inline]
fn pair_sums<S: Simd, const P: usize>(
    simd: S,
    weights: &[i16x8<S>; 4],
    rows: impl Fn(usize) -> i16x8<S>,
) -> (i32x4<S>, i32x4<S>) {
    let (mut low, mut high) = (i32x4::splat(simd, 0), i32x4::splat(simd, 0));
    for (pair, &weight) in weights.iter().enumerate().take(P) {
        let (a, b) = (rows(2 * pair), rows(2 * pair + 1));
        low += a.zip_low(b).pairwise_dot_product(weight);
        high += a.zip_high(b).pairwise_dot_product(weight);
    }
    (low, high)
}

/// The horizontal sums of the eight outputs whose windows start at `at`.
#[inline]
fn horizontal_sums<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    source: &[T],
    at: usize,
    weights: &[i16x8<S>; 4],
) -> (i32x4<S>, i32x4<S>) {
    let (a, b) = (T::lanes(simd, source, at), T::lanes(simd, source, at + 8));
    let windows = [
        a,
        a.slide::<1>(b),
        a.slide::<2>(b),
        a.slide::<3>(b),
        a.slide::<4>(b),
        a.slide::<5>(b),
        a.slide::<6>(b),
        a.slide::<7>(b),
    ];
    pair_sums::<S, P>(simd, weights, |window| windows[window])
}

#[inline]
fn round<S: Simd>(
    (low, high): (i32x4<S>, i32x4<S>),
    bias: i32,
    shift: u32,
) -> (i32x4<S>, i32x4<S>) {
    ((low + bias) >> shift, (high + bias) >> shift)
}

#[inline]
fn copy<S: Simd, T: Lanes>(simd: S, block: &Block<'_, T>, sink: &mut Sink<'_>) {
    let shift = if sink.is_put() { 0 } else { 4 };
    for row in 0..block.h {
        let start = block.origin + row * block.stride;
        for col in (0..block.w).step_by(8) {
            let (low, high) = T::lanes(simd, block.source, start + col).widen();
            emit(simd, sink, block.w, row, col, (low << shift, high << shift));
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
    for row in 0..block.h {
        let start = block.origin + row * block.stride;
        for col in (0..block.w).step_by(8) {
            let sums = horizontal_sums::<S, T, P>(simd, block.source, start + col, &weights);
            let intermediate = round(sums, 4, 3);
            let values = if put {
                round(intermediate, 8, 4)
            } else {
                intermediate
            };
            emit(simd, sink, block.w, row, col, values);
        }
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
    for row in 0..block.h {
        for col in (0..block.w).step_by(8) {
            let line = |k: usize| {
                T::lanes(
                    simd,
                    block.source,
                    block.origin + (row + k) * block.stride + col,
                )
            };
            let sums = pair_sums::<S, P>(simd, &weights, line);
            emit(simd, sink, block.w, row, col, round(sums, bias, shift));
        }
    }
}

/// The horizontal pass of columns `col..col + 8`: `rows` intermediate rows
/// of eight lanes into `stage`.
#[inline]
fn stage_rows<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    block: &Block<'_, T>,
    taps: &Taps,
    col: usize,
    rows: usize,
    stage: &mut [i16],
) {
    let weights = weights(simd, taps);
    for (row, slot) in stage.chunks_exact_mut(8).take(rows).enumerate() {
        let at = block.origin + row * block.stride + col;
        let (low, high) = round(
            horizontal_sums::<S, T, P>(simd, block.source, at, &weights),
            4,
            3,
        );
        if let Some(slot) = slot.first_chunk_mut::<8>() {
            low.narrow(high).store_array(slot);
        }
    }
}

/// The vertical pass of columns `col..col + 8` over [`stage_rows`].
#[inline]
fn filter_stage<S: Simd, const P: usize>(
    simd: S,
    w: usize,
    h: usize,
    taps: &Taps,
    col: usize,
    stage: &[i16],
    sink: &mut Sink<'_>,
) {
    let weights = weights(simd, taps);
    let (bias, shift) = if sink.is_put() {
        (1 << 10, 11)
    } else {
        (1 << 6, 7)
    };
    let zero = [0; 8];
    for row in 0..h {
        let line = |k: usize| {
            let lanes = stage
                .get((row + k) * 8..)
                .and_then(<[i16]>::first_chunk::<8>);
            i16x8::load_array_ref(simd, lanes.unwrap_or(&zero))
        };
        let sums = pair_sums::<S, P>(simd, &weights, line);
        emit(simd, sink, w, row, col, round(sums, bias, shift));
    }
}

/// Stores the eight values of output row `row`, columns `col..`, of a
/// `w`-wide block; columns past `w` are dropped.
#[inline]
fn emit<S: Simd>(
    simd: S,
    sink: &mut Sink<'_>,
    w: usize,
    row: usize,
    col: usize,
    (low, high): (i32x4<S>, i32x4<S>),
) {
    let count = (w - col).min(8);
    let clipped = |max: i32| {
        let (low, high) = (low.max(0).min(max), high.max(0).min(max));
        low.narrow(high)
    };
    let values = || {
        let (low, high) = (low.to_array(), high.to_array());
        core::array::from_fn(|lane| if lane < 4 { low[lane] } else { high[lane - 4] })
    };
    match sink {
        Sink::Put { output, stride } => {
            let lanes = clipped(255).to_array();
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
        } => {
            let lanes = clipped(i32::from(*max)).to_array();
            store(
                output,
                row * *stride + col,
                count,
                lanes.map(|lane| lane as u16),
            );
        }
        Sink::Prep { output, stride } => store(output, row * *stride + col, count, values()),
        Sink::Average {
            pred0,
            forward,
            output,
            stride,
        } => {
            let blended = average(simd, pred0, row * w + col, count, *forward, values(), 255);
            store(
                output,
                row * *stride + col,
                count,
                blended.map(|lane| lane as u8),
            );
        }
        Sink::AverageWide {
            pred0,
            forward,
            output,
            stride,
            max,
        } => {
            let max = i32::from(*max);
            let blended = average(simd, pred0, row * w + col, count, *forward, values(), max);
            store(
                output,
                row * *stride + col,
                count,
                blended.map(|lane| lane as u16),
            );
        }
    }
}

/// The COMPOUND_AVERAGE of `count` values with `pred0` from `at`, in
/// `i32` lanes.
#[inline]
fn average<S: Simd>(
    simd: S,
    pred0: &[i32],
    at: usize,
    count: usize,
    forward: i32,
    values: [i32; 8],
    max: i32,
) -> [i32; 8] {
    let mut first = [0; 8];
    for (slot, &value) in first
        .iter_mut()
        .zip(pred0.get(at..).unwrap_or(&[]))
        .take(count)
    {
        *slot = value;
    }
    let blend = |offset: usize| {
        let first = i32x4::from_fn(simd, |lane| first[offset + lane]);
        let second = i32x4::from_fn(simd, |lane| values[offset + lane]);
        let blended = if forward == 8 {
            (first + second + 16) >> 5
        } else {
            (first * forward + second * (16 - forward) + 128) >> 8
        };
        blended.max(0).min(max).to_array()
    };
    let (low, high) = (blend(0), blend(4));
    core::array::from_fn(|lane| if lane < 4 { low[lane] } else { high[lane - 4] })
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
