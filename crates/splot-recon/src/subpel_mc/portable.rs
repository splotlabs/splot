// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The unscaled § 7.13.3.18 convolution on 128-bit `i16` lanes, for every
//! sample type and block width.
//!
//! A vector holds eight outputs of a row (or two four-column rows when
//! `w <= 4`). A zero phase skips its pass: `Round2(128 * s, 3) == s << 4`
//! and `Round2(Round2(x, 3) << 7, 11) == Round2(Round2(x, 3), 4)` keep the
//! two-pass value. § 6 Table 6.3 admits only 8- and 10-bit samples, so a
//! horizontal value fits `i16`.

use super::block::{Block, Sink, Taps};
use fearless_simd::{Simd, i16x8, i32x4, prelude::*, u8x16, u16x8};

/// Samples the kernels widen to `i16` lanes.
pub(super) trait Lanes: Copy {
    /// Eight samples from `at`, zero past the slice.
    fn lanes<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S>;

    /// Sixteen samples from `at` as two eight-lane vectors, zero past the slice.
    fn lanes16<S: Simd>(simd: S, samples: &[Self], at: usize) -> (i16x8<S>, i16x8<S>);
}

impl Lanes for u8 {
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    fn lanes<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S> {
        match samples.get(at..).and_then(<[u8]>::first_chunk::<16>) {
            Some(bytes) => u8x16::load_array_ref(simd, bytes).widen().0.bitcast(),
            None => i16x8::splat(simd, 0),
        }
    }

    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    fn lanes16<S: Simd>(simd: S, samples: &[Self], at: usize) -> (i16x8<S>, i16x8<S>) {
        match samples.get(at..).and_then(<[u8]>::first_chunk::<16>) {
            Some(bytes) => {
                let (low, high) = u8x16::load_array_ref(simd, bytes).widen();
                (low.bitcast(), high.bitcast())
            }
            None => (i16x8::splat(simd, 0), i16x8::splat(simd, 0)),
        }
    }
}

impl Lanes for u16 {
    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    fn lanes<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S> {
        match samples.get(at..).and_then(<[u16]>::first_chunk::<8>) {
            Some(lanes) => u16x8::load_array_ref(simd, lanes).bitcast(),
            None => i16x8::splat(simd, 0),
        }
    }

    #[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
    #[inline(always)]
    fn lanes16<S: Simd>(simd: S, samples: &[Self], at: usize) -> (i16x8<S>, i16x8<S>) {
        (
            Self::lanes(simd, samples, at),
            Self::lanes(simd, samples, at + 8),
        )
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

/// Predicts both blocks of an 8-bit compound cell (`w * h <= 64` or full-pel)
/// and stores their COMPOUND_AVERAGE into `output`.
pub(super) fn compound_u8(
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    stage: &mut [i16],
    output: &mut [u8],
    stride: usize,
) {
    if blocks
        .iter()
        .all(|b| b.horizontal.is_none() && b.vertical.is_none())
    {
        baseline_simd!(simd => copy_compound_u8(simd, blocks, forward, output, stride));
        return;
    }
    let w = blocks[0].w;
    let mut pred0 = [0i32; 64];
    baseline_simd!(simd => {
        run(
            simd,
            blocks[0],
            &mut Sink::Prep {
                output: &mut pred0,
                stride: w,
            },
            stage,
        );
        run(
            simd,
            blocks[1],
            &mut Sink::Average {
                pred0: &pred0,
                forward,
                output,
                stride,
            },
            stage,
        );
    });
}

/// Predicts both blocks of a 16-bit compound cell (`w * h <= 64` or full-pel)
/// and stores their COMPOUND_AVERAGE clipped to `max` into `output`.
pub(super) fn compound_u16(
    blocks: [&Block<'_, u16>; 2],
    forward: i32,
    max: u16,
    stage: &mut [i16],
    output: &mut [u16],
    stride: usize,
) {
    if blocks
        .iter()
        .all(|b| b.horizontal.is_none() && b.vertical.is_none())
    {
        baseline_simd!(simd => copy_compound_u16(simd, blocks, forward, max, output, stride));
        return;
    }
    let w = blocks[0].w;
    let mut pred0 = [0i32; 64];
    baseline_simd!(simd => {
        run(
            simd,
            blocks[0],
            &mut Sink::Prep {
                output: &mut pred0,
                stride: w,
            },
            stage,
        );
        run(
            simd,
            blocks[1],
            &mut Sink::AverageWide {
                pred0: &pred0,
                forward,
                output,
                stride,
                max,
            },
            stage,
        );
    });
}

#[inline(never)]
fn copy_compound_u8<S: Simd>(
    simd: S,
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    let (w, h) = (blocks[0].w, blocks[0].h);
    let (fwd, back, bias) = (
        i16x8::splat(simd, forward as i16),
        i16x8::splat(simd, (16 - forward) as i16),
        i16x8::splat(simd, 8),
    );
    let one = u16x8::splat(simd, 1);
    for row in 0..h {
        let (s0, s1, out_row) = (blocks[0].start(row), blocks[1].start(row), row * stride);
        for col in (0..w).step_by(8) {
            let a = u8::lanes(simd, blocks[0].source, s0 + col);
            let c = u8::lanes(simd, blocks[1].source, s1 + col);
            let avg: u16x8<S> = if forward == 8 {
                (a.bitcast::<u16x8<S>>() + c.bitcast::<u16x8<S>>() + one) >> 1
            } else {
                ((a * fwd + c * back + bias) >> 4).max(0).min(255).bitcast()
            };
            store(
                output,
                out_row + col,
                (w - col).min(8),
                avg.to_array().map(|v| v as u8),
            );
        }
    }
}

#[inline(never)]
fn copy_compound_u16<S: Simd>(
    simd: S,
    blocks: [&Block<'_, u16>; 2],
    forward: i32,
    max: u16,
    output: &mut [u16],
    stride: usize,
) {
    let (w, h) = (blocks[0].w, blocks[0].h);
    let max_v = u16x8::splat(simd, max);
    let one = u16x8::splat(simd, 1);
    if forward == 8 {
        if w == 4 {
            for row in 0..h {
                let a =
                    u16::lanes(simd, blocks[0].source, blocks[0].start(row)).bitcast::<u16x8<S>>();
                let b =
                    u16::lanes(simd, blocks[1].source, blocks[1].start(row)).bitcast::<u16x8<S>>();
                let lanes = (((a + b + one) >> 1).min(max_v)).to_array();
                if let Some(slot) = output
                    .get_mut(row * stride..)
                    .and_then(<[u16]>::first_chunk_mut::<4>)
                {
                    slot.copy_from_slice(&lanes[..4]); // splot-copy-ok: four blended u16 lanes
                }
            }
            return;
        }
        if w.is_multiple_of(8) {
            for row in 0..h {
                let (s0, s1, out_row) = (blocks[0].start(row), blocks[1].start(row), row * stride);
                let (Some(src0), Some(src1), Some(dst)) = (
                    blocks[0].source.get(s0..s0 + w),
                    blocks[1].source.get(s1..s1 + w),
                    output.get_mut(out_row..out_row + w),
                ) else {
                    continue;
                };
                for ((slot, a), b) in dst
                    .as_chunks_mut::<8>()
                    .0
                    .iter_mut()
                    .zip(src0.as_chunks::<8>().0)
                    .zip(src1.as_chunks::<8>().0)
                {
                    let a = u16x8::load_array_ref(simd, a);
                    let b = u16x8::load_array_ref(simd, b);
                    (((a + b + one) >> 1).min(max_v)).store_array(slot);
                }
            }
            return;
        }
        for row in 0..h {
            let (s0, s1, out_row) = (blocks[0].start(row), blocks[1].start(row), row * stride);
            for col in (0..w).step_by(8) {
                let a = u16::lanes(simd, blocks[0].source, s0 + col).bitcast::<u16x8<S>>();
                let b = u16::lanes(simd, blocks[1].source, s1 + col).bitcast::<u16x8<S>>();
                let avg = ((a + b + one) >> 1).min(max_v);
                store_wide(output, out_row + col, (w - col).min(8), avg);
            }
        }
        return;
    }
    let (f_v, b_v, max_i32) = (
        i16x8::splat(simd, forward as i16),
        i16x8::splat(simd, (16 - forward) as i16),
        i32::from(max),
    );
    if w == 4 {
        for row in 0..h {
            let a = u16::lanes(simd, blocks[0].source, blocks[0].start(row));
            let b = u16::lanes(simd, blocks[1].source, blocks[1].start(row));
            let l0 = ((a.widening_mul(f_v).0 + b.widening_mul(b_v).0 + 8) >> 4)
                .max(0)
                .min(max_i32);
            let lanes = l0.narrow(l0).bitcast::<u16x8<S>>().to_array();
            if let Some(slot) = output
                .get_mut(row * stride..)
                .and_then(<[u16]>::first_chunk_mut::<4>)
            {
                slot.copy_from_slice(&lanes[..4]); // splot-copy-ok: four blended u16 lanes
            }
        }
        return;
    }
    if w.is_multiple_of(8) {
        for row in 0..h {
            let (s0, s1, out_row) = (blocks[0].start(row), blocks[1].start(row), row * stride);
            let (Some(src0), Some(src1), Some(dst)) = (
                blocks[0].source.get(s0..s0 + w),
                blocks[1].source.get(s1..s1 + w),
                output.get_mut(out_row..out_row + w),
            ) else {
                continue;
            };
            for ((slot, a), b) in dst
                .as_chunks_mut::<8>()
                .0
                .iter_mut()
                .zip(src0.as_chunks::<8>().0)
                .zip(src1.as_chunks::<8>().0)
            {
                let a: i16x8<S> = u16x8::load_array_ref(simd, a).bitcast();
                let b: i16x8<S> = u16x8::load_array_ref(simd, b).bitcast();
                let (a0, a1) = a.widening_mul(f_v);
                let (b0, b1) = b.widening_mul(b_v);
                let l0 = ((a0 + b0 + 8) >> 4).max(0).min(max_i32);
                let l1 = ((a1 + b1 + 8) >> 4).max(0).min(max_i32);
                l0.narrow(l1).bitcast::<u16x8<S>>().store_array(slot);
            }
        }
        return;
    }
    for row in 0..h {
        let (s0, s1, out_row) = (blocks[0].start(row), blocks[1].start(row), row * stride);
        for col in (0..w).step_by(8) {
            let (a0, a1) = u16::lanes(simd, blocks[0].source, s0 + col).widening_mul(f_v);
            let (b0, b1) = u16::lanes(simd, blocks[1].source, s1 + col).widening_mul(b_v);
            let l0 = ((a0 + b0 + 8) >> 4).max(0).min(max_i32);
            let l1 = ((a1 + b1 + 8) >> 4).max(0).min(max_i32);
            store_wide(
                output,
                out_row + col,
                (w - col).min(8),
                l0.narrow(l1).bitcast(),
            );
        }
    }
}

type Remap<S> = [u8x16<S>; 2];

#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn remap_indices<S: Simd>(simd: S, remap: Option<[u8; 16]>) -> Option<Remap<S>> {
    let remap = u8x16::load_array_ref(simd, &remap?);
    let even = remap + remap;
    let odd = even + u8x16::splat(simd, 1);
    Some([even.zip_low(odd), even.zip_high(odd)])
}

#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn window16<S: Simd, T: Lanes>(
    simd: S,
    source: &[T],
    start: usize,
    remap: Option<&Remap<S>>,
) -> (i16x8<S>, i16x8<S>) {
    let (a, b) = T::lanes16(simd, source, start);
    match remap {
        None => (a, b),
        Some([idx0, idx1]) => {
            let (bytes_a, bytes_b) = (a.bitcast::<u8x16<S>>(), b.bitcast::<u8x16<S>>());
            (
                simd.concat_swizzle_dyn_u8x16(bytes_a, bytes_b, *idx0)
                    .bitcast(),
                simd.concat_swizzle_dyn_u8x16(bytes_a, bytes_b, *idx1)
                    .bitcast(),
            )
        }
    }
}

#[inline]
fn run<S: Simd, T: Lanes>(simd: S, block: &Block<'_, T>, sink: &mut Sink<'_>, stage: &mut [i16]) {
    match (block.horizontal, block.vertical) {
        (None, None) => copy(simd, block, sink),
        (Some(taps), None) => with_pairs!(taps.count, horizontal::<S, T>(simd, block, taps, sink)),
        (None, Some(taps)) => with_pairs!(taps.count, vertical::<S, T>(simd, block, taps, sink)),
        (Some(h_taps), Some(v_taps)) => {
            let rows = block.h + v_taps.span() - 1;
            with_pairs!(
                h_taps.count,
                stage_rows::<S, T>(simd, block, h_taps, rows, stage)
            );
            with_pairs!(
                v_taps.count,
                filter_stage::<S>(simd, block.w, block.h, v_taps, stage, sink)
            );
        }
    }
}

/// The active tap weights of `taps` splatted into eight-lane vectors.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn weights<S: Simd>(simd: S, taps: &Taps) -> [i16x8<S>; 8] {
    core::array::from_fn(|t| {
        let pair = taps.full[t / 2];
        let tap = if t % 2 == 0 {
            pair as i16
        } else {
            (pair >> 16) as i16
        };
        i16x8::splat(simd, tap)
    })
}

/// `Σ` over `2 * P` taps of `row(t) * weights[t]` into `i32` lanes `0..4` and
/// `4..8`.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn tap_sums<S: Simd, const P: usize>(
    weights: &[i16x8<S>; 8],
    mut row: impl FnMut(usize) -> i16x8<S>,
) -> (i32x4<S>, i32x4<S>) {
    let (mut low, mut high) = row(0).widening_mul(weights[0]);
    let (l1, h1) = row(1).widening_mul(weights[1]);
    low += l1;
    high += h1;
    if P >= 2 {
        let (l2, h2) = row(2).widening_mul(weights[2]);
        low += l2;
        high += h2;
        let (l3, h3) = row(3).widening_mul(weights[3]);
        low += l3;
        high += h3;
    }
    if P >= 3 {
        let (l4, h4) = row(4).widening_mul(weights[4]);
        low += l4;
        high += h4;
        let (l5, h5) = row(5).widening_mul(weights[5]);
        low += l5;
        high += h5;
    }
    if P >= 4 {
        let (l6, h6) = row(6).widening_mul(weights[6]);
        low += l6;
        high += h6;
        let (l7, h7) = row(7).widening_mul(weights[7]);
        low += l7;
        high += h7;
    }
    (low, high)
}

/// `Σ` over `2 * P` taps of the low four lanes of `row(t) * weights[t]`.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn tap_sums4<S: Simd, const P: usize>(
    weights: &[i16x8<S>; 8],
    mut row: impl FnMut(usize) -> i16x8<S>,
) -> i32x4<S> {
    let mul = |x: i16x8<S>, w: i16x8<S>| x.widening_mul(w).0;
    let mut sum = mul(row(0), weights[0]) + mul(row(1), weights[1]);
    if P >= 2 {
        sum += mul(row(2), weights[2]);
        sum += mul(row(3), weights[3]);
    }
    if P >= 3 {
        sum += mul(row(4), weights[4]);
        sum += mul(row(5), weights[5]);
    }
    if P >= 4 {
        sum += mul(row(6), weights[6]);
        sum += mul(row(7), weights[7]);
    }
    sum
}

/// The horizontal sums of an 8-column window `(a, b)`.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn horizontal_sums_ab<S: Simd, const P: usize>(
    a: i16x8<S>,
    b: i16x8<S>,
    weights: &[i16x8<S>; 8],
) -> (i32x4<S>, i32x4<S>) {
    tap_sums::<S, P>(weights, |t| match t {
        0 => a,
        1 => a.slide::<1>(b),
        2 => a.slide::<2>(b),
        3 => a.slide::<3>(b),
        4 => a.slide::<4>(b),
        5 => a.slide::<5>(b),
        6 => a.slide::<6>(b),
        _ => a.slide::<7>(b),
    })
}

/// The horizontal sums of an 8-column window starting at `start`.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn horizontal_sums<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    source: &[T],
    start: usize,
    weights: &[i16x8<S>; 8],
    remap: Option<&Remap<S>>,
) -> (i32x4<S>, i32x4<S>) {
    let (a, b) = window16(simd, source, start, remap);
    horizontal_sums_ab::<S, P>(a, b, weights)
}

/// The horizontal sums of a 4-column window starting at `start`.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn horizontal_sums4<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    source: &[T],
    start: usize,
    weights: &[i16x8<S>; 8],
    remap: Option<&Remap<S>>,
) -> i32x4<S> {
    let (a, b) = if P <= 2 && remap.is_none() {
        let a = T::lanes(simd, source, start);
        (a, a)
    } else {
        window16(simd, source, start, remap)
    };
    tap_sums4::<S, P>(weights, |t| match t {
        0 => a,
        1 => a.slide::<1>(b),
        2 => a.slide::<2>(b),
        3 => a.slide::<3>(b),
        4 => a.slide::<4>(b),
        5 => a.slide::<5>(b),
        6 => a.slide::<6>(b),
        _ => a.slide::<7>(b),
    })
}

#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn round<S: Simd>(
    (low, high): (i32x4<S>, i32x4<S>),
    bias: i32,
    shift: u32,
) -> (i32x4<S>, i32x4<S>) {
    ((low + bias) >> shift, (high + bias) >> shift)
}

#[inline(never)]
fn copy<S: Simd, T: Lanes>(simd: S, block: &Block<'_, T>, sink: &mut Sink<'_>) {
    let (w, h) = (block.w, block.h);
    match sink {
        Sink::PutWide {
            output,
            stride,
            max,
        } => {
            let (stride, max_v) = (*stride, u16x8::splat(simd, *max));
            if w == 4 {
                for row in 0..h {
                    let lanes = T::lanes(simd, block.source, block.start(row))
                        .bitcast::<u16x8<S>>()
                        .min(max_v)
                        .to_array();
                    if let Some(slot) = output
                        .get_mut(row * stride..)
                        .and_then(<[u16]>::first_chunk_mut::<4>)
                    {
                        slot.copy_from_slice(&lanes[..4]); // splot-copy-ok: four clamped u16 lanes
                    }
                }
                return;
            }
            if w.is_multiple_of(8) {
                for row in 0..h {
                    let (row_start, out_row) = (block.start(row), row * stride);
                    let Some(dst) = output.get_mut(out_row..out_row + w) else {
                        continue;
                    };
                    for (col, slot) in dst.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                        T::lanes(simd, block.source, row_start + col * 8)
                            .bitcast::<u16x8<S>>()
                            .min(max_v)
                            .store_array(slot);
                    }
                }
                return;
            }
            for row in 0..h {
                let (row_start, out_row) = (block.start(row), row * stride);
                for col in (0..w).step_by(8) {
                    let lanes = T::lanes(simd, block.source, row_start + col)
                        .bitcast::<u16x8<S>>()
                        .min(max_v);
                    store_wide(output, out_row + col, (w - col).min(8), lanes);
                }
            }
        }
        Sink::Put { output, stride } => {
            let (stride, max_v) = (*stride, u16x8::splat(simd, 255));
            for row in 0..h {
                let (row_start, out_row) = (block.start(row), row * stride);
                for col in (0..w).step_by(8) {
                    let lanes = T::lanes(simd, block.source, row_start + col)
                        .bitcast::<u16x8<S>>()
                        .min(max_v)
                        .to_array();
                    store(
                        output,
                        out_row + col,
                        (w - col).min(8),
                        lanes.map(|v| v as u8),
                    );
                }
            }
        }
        Sink::Prep { output, stride } => {
            let stride = *stride;
            if w <= 4 {
                for row in 0..h {
                    let low = T::lanes(simd, block.source, block.start(row)).widen().0 << 4;
                    let at = row * stride;
                    if w == 4
                        && let Some(slot) =
                            output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>)
                    {
                        low.store_array(slot);
                    } else {
                        store4(output, at, w, low.to_array());
                    }
                }
                return;
            }
            for row in 0..h {
                let (row_start, out_row) = (block.start(row), row * stride);
                for col in (0..w).step_by(8) {
                    let (low, high) = T::lanes(simd, block.source, row_start + col).widen();
                    let (low, high) = (low << 4, high << 4);
                    let count = (w - col).min(8);
                    let at = out_row + col;
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
                                core::array::from_fn(|i| if i < 4 { low[i] } else { high[i - 4] });
                            store(output, at, count, lanes);
                        }
                    }
                }
            }
        }
        _ => {
            if w <= 4 {
                for row in 0..h {
                    let low = T::lanes(simd, block.source, block.start(row)).widen().0 << 4;
                    emit4(simd, sink, w, row, low);
                }
                return;
            }
            for row in 0..h {
                let row_start = block.start(row);
                for col in (0..w).step_by(8) {
                    let (low, high) = T::lanes(simd, block.source, row_start + col).widen();
                    emit(simd, sink, w, row, col, (low << 4, high << 4));
                }
            }
        }
    }
}

#[inline(never)]
fn horizontal<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    block: &Block<'_, T>,
    taps: &Taps,
    sink: &mut Sink<'_>,
) {
    let weights = weights(simd, taps);
    let remap = remap_indices(simd, block.remap_lanes());
    let put = sink.is_put();
    if block.w <= 4 {
        for row in 0..block.h {
            let sum = horizontal_sums4::<S, T, P>(
                simd,
                block.source,
                block.start(row),
                &weights,
                remap.as_ref(),
            );
            let intermediate = (sum + 4) >> 3;
            let values = if put {
                (intermediate + 8) >> 4
            } else {
                intermediate
            };
            emit4(simd, sink, block.w, row, values);
        }
        return;
    }
    for row in 0..block.h {
        let row_start = block.start(row);
        if remap.is_none() {
            let mut a = T::lanes(simd, block.source, row_start);
            for col in (0..block.w).step_by(8) {
                let b = T::lanes(simd, block.source, row_start + col + 8);
                let sums = horizontal_sums_ab::<S, P>(a, b, &weights);
                a = b;
                let intermediate = round(sums, 4, 3);
                let values = if put {
                    round(intermediate, 8, 4)
                } else {
                    intermediate
                };
                emit(simd, sink, block.w, row, col, values);
            }
            continue;
        }
        for col in (0..block.w).step_by(8) {
            let sums = horizontal_sums::<S, T, P>(
                simd,
                block.source,
                row_start + col,
                &weights,
                remap.as_ref(),
            );
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

#[inline(never)]
fn vertical<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    block: &Block<'_, T>,
    taps: &Taps,
    sink: &mut Sink<'_>,
) {
    let weights = weights(simd, taps);
    let (bias, shift) = if sink.is_put() { (64, 7) } else { (4, 3) };
    if block.w <= 4 {
        let mut r: [i16x8<S>; 7] = core::array::from_fn(|t| {
            if t + 1 < 2 * P {
                T::lanes(simd, block.source, block.start(t))
            } else {
                i16x8::splat(simd, 0)
            }
        });
        for row in 0..block.h {
            let next = T::lanes(simd, block.source, block.start(row + 2 * P - 1));
            let sum = tap_sums4::<S, P>(&weights, |t| if t + 1 == 2 * P { next } else { r[t] });
            emit4(simd, sink, block.w, row, (sum + bias) >> shift);
            for t in 0..2 * P - 2 {
                r[t] = r[t + 1];
            }
            r[2 * P - 2] = next;
        }
        return;
    }
    for col in (0..block.w).step_by(8) {
        let mut r: [i16x8<S>; 7] = core::array::from_fn(|t| {
            if t + 1 < 2 * P {
                T::lanes(simd, block.source, block.start(t) + col)
            } else {
                i16x8::splat(simd, 0)
            }
        });
        for row in 0..block.h {
            let next = T::lanes(simd, block.source, block.start(row + 2 * P - 1) + col);
            let sums = tap_sums::<S, P>(&weights, |t| if t + 1 == 2 * P { next } else { r[t] });
            emit(simd, sink, block.w, row, col, round(sums, bias, shift));
            for t in 0..2 * P - 2 {
                r[t] = r[t + 1];
            }
            r[2 * P - 2] = next;
        }
    }
}

/// The horizontal pass: `rows` intermediate rows into `stage`.
#[inline(never)]
fn stage_rows<S: Simd, T: Lanes, const P: usize>(
    simd: S,
    block: &Block<'_, T>,
    taps: &Taps,
    rows: usize,
    stage: &mut [i16],
) {
    let weights = weights(simd, taps);
    let remap = remap_indices(simd, block.remap_lanes());
    if block.w <= 4 {
        for r in (0..rows).step_by(2) {
            let sum0 = (horizontal_sums4::<S, T, P>(
                simd,
                block.source,
                block.start(r),
                &weights,
                remap.as_ref(),
            ) + 4)
                >> 3;
            let sum1 = (horizontal_sums4::<S, T, P>(
                simd,
                block.source,
                block.start((r + 1).min(rows - 1)),
                &weights,
                remap.as_ref(),
            ) + 4)
                >> 3;
            if let Some(slot) = stage
                .get_mut(r * 4..)
                .and_then(<[i16]>::first_chunk_mut::<8>)
            {
                sum0.narrow(sum1).store_array(slot);
            }
        }
        return;
    }
    for row in 0..rows {
        let row_start = block.start(row);
        if remap.is_none() {
            let mut a = T::lanes(simd, block.source, row_start);
            for col in (0..block.w).step_by(8) {
                let b = T::lanes(simd, block.source, row_start + col + 8);
                let (low, high) = round(horizontal_sums_ab::<S, P>(a, b, &weights), 4, 3);
                a = b;
                let at = (col / 8 * rows + row) * 8;
                if let Some(slot) = stage.get_mut(at..).and_then(<[i16]>::first_chunk_mut::<8>) {
                    low.narrow(high).store_array(slot);
                }
            }
            continue;
        }
        for col in (0..block.w).step_by(8) {
            let (low, high) = round(
                horizontal_sums::<S, T, P>(
                    simd,
                    block.source,
                    row_start + col,
                    &weights,
                    remap.as_ref(),
                ),
                4,
                3,
            );
            let at = (col / 8 * rows + row) * 8;
            if let Some(slot) = stage.get_mut(at..).and_then(<[i16]>::first_chunk_mut::<8>) {
                low.narrow(high).store_array(slot);
            }
        }
    }
}

/// The vertical pass over [`stage_rows`].
#[inline(never)]
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
    let load_stage = |at: usize| match stage.get(at..).and_then(<[i16]>::first_chunk::<8>) {
        Some(lanes) => i16x8::load_array_ref(simd, lanes),
        None => i16x8::splat(simd, 0),
    };
    if w <= 4 {
        let mut e: [i16x8<S>; 4] = core::array::from_fn(|k| {
            if k < P {
                load_stage(k * 8)
            } else {
                i16x8::splat(simd, 0)
            }
        });
        for row in (0..h).step_by(2) {
            let next = load_stage((row + 2 * P) * 4);
            let adj = |k: usize| {
                let hi = if k + 1 == P { next } else { e[k + 1] };
                e[k].slide::<4>(hi)
            };
            let sums = tap_sums::<S, P>(&weights, |t| match t {
                0 => e[0],
                1 => adj(0),
                2 => e[1],
                3 => adj(1),
                4 => e[2],
                5 => adj(2),
                6 => e[3],
                _ => adj(3),
            });
            let (low, high) = round(sums, bias, shift);
            emit4(simd, sink, w, row, low);
            if row + 1 < h {
                emit4(simd, sink, w, row + 1, high);
            }
            for k in 0..P - 1 {
                e[k] = e[k + 1];
            }
            e[P - 1] = next;
        }
        return;
    }
    let rows = h + 2 * P - 1;
    for col in (0..w).step_by(8) {
        let strip_base = col / 8 * rows * 8;
        let mut r: [i16x8<S>; 7] = core::array::from_fn(|t| {
            if t + 1 < 2 * P {
                load_stage(strip_base + t * 8)
            } else {
                i16x8::splat(simd, 0)
            }
        });
        for row in 0..h {
            let next = load_stage(strip_base + (row + 2 * P - 1) * 8);
            let sums = tap_sums::<S, P>(&weights, |t| if t + 1 == 2 * P { next } else { r[t] });
            emit(simd, sink, w, row, col, round(sums, bias, shift));
            for t in 0..2 * P - 2 {
                r[t] = r[t + 1];
            }
            r[2 * P - 2] = next;
        }
    }
}

#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn load4_pred0<S: Simd, P: Copy + Into<i32>>(simd: S, pred0: &[P], at: usize) -> i32x4<S> {
    if let Some(lanes) = pred0.get(at..).and_then(<[P]>::first_chunk::<4>) {
        return i32x4::from_slice(simd, &lanes.map(Into::into));
    }
    let tail = pred0.get(at..).unwrap_or(&[]);
    let get = |i: usize| tail.get(i).map_or(0, |&value| value.into());
    i32x4::load_array_ref(simd, &[get(0), get(1), get(2), get(3)])
}

/// Stores up to four values of output row `row` of a `w <= 4` block.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn emit4<S: Simd>(simd: S, sink: &mut Sink<'_>, w: usize, row: usize, low: i32x4<S>) {
    let blended = |first: i32x4<S>, forward: i32| {
        if forward == 8 {
            (first + low + 16) >> 5
        } else {
            (first * forward + low * (16 - forward) + 128) >> 8
        }
    };
    match sink {
        Sink::Put { output, stride } => {
            let lanes = low.max(0).min(255).to_array().map(|v| v as u8);
            store4(output, row * *stride, w, lanes);
        }
        Sink::PutWide {
            output,
            stride,
            max,
        } => {
            let lanes = low.max(0).min(i32::from(*max)).to_array().map(|v| v as u16);
            store4(output, row * *stride, w, lanes);
        }
        Sink::Prep { output, stride } => {
            let at = row * *stride;
            if w == 4
                && let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>)
            {
                low.store_array(slot);
                return;
            }
            store4(output, at, w, low.to_array());
        }
        Sink::Average {
            pred0,
            forward,
            output,
            stride,
        } => {
            let lanes = blended(load4_pred0(simd, pred0, row * w), *forward)
                .max(0)
                .min(255)
                .to_array()
                .map(|v| v as u8);
            store4(output, row * *stride, w, lanes);
        }
        Sink::PrepNarrow { output, stride } => {
            store4(output, row * *stride, w, low.to_array().map(|v| v as i16));
        }
        Sink::AverageNarrow {
            pred0,
            forward,
            output,
            stride,
        } => {
            let lanes = blended(load4_pred0(simd, pred0, row * w), *forward)
                .max(0)
                .min(255)
                .to_array()
                .map(|v| v as u8);
            store4(output, row * *stride, w, lanes);
        }
        Sink::AverageWide {
            pred0,
            forward,
            output,
            stride,
            max,
        } => {
            let lanes = blended(load4_pred0(simd, pred0, row * w), *forward)
                .max(0)
                .min(i32::from(*max))
                .to_array()
                .map(|v| v as u16);
            store4(output, row * *stride, w, lanes);
        }
    }
}

#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn store4<T: Copy>(output: &mut [T], at: usize, count: usize, lanes: [T; 4]) {
    let slot = output.get_mut(at..).unwrap_or(&mut []);
    if count == 4 {
        if let Some(slot) = slot.first_chunk_mut::<4>() {
            *slot = lanes;
        }
        return;
    }
    if count >= 2
        && let (Some(slot), Some(pair)) = (slot.first_chunk_mut::<2>(), lanes.first_chunk::<2>())
    {
        *slot = *pair;
    }
    if count & 1 != 0
        && let Some(last) = slot.get_mut(count - 1)
    {
        *last = lanes[count - 1];
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
    let at = row * w + col;
    let blended = |(first, second): (i32x4<S>, i32x4<S>), forward: i32| {
        let blend = |first: i32x4<S>, second: i32x4<S>| {
            if forward == 8 {
                (first + second + 16) >> 5
            } else {
                (first * forward + second * (16 - forward) + 128) >> 8
            }
        };
        (blend(first, low), blend(second, high))
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
        Sink::PrepNarrow { output, stride } => {
            let (low, high) = (low.to_array(), high.to_array());
            let lanes = core::array::from_fn(|lane| {
                if lane < 4 {
                    low[lane] as i16
                } else {
                    high[lane - 4] as i16
                }
            });
            store(output, row * *stride + col, count, lanes);
        }
        Sink::AverageNarrow {
            pred0,
            forward,
            output,
            stride,
        } => {
            let (first, second) = blended(
                (
                    load4_pred0(simd, pred0, at),
                    load4_pred0(simd, pred0, at + 4),
                ),
                *forward,
            );
            let lanes = clipped(first, second, 255).to_array();
            store(
                output,
                row * *stride + col,
                count,
                lanes.map(|lane| lane as u8),
            );
        }
        Sink::Average {
            pred0,
            forward,
            output,
            stride,
        } => {
            let (first, second) = blended(
                (
                    load4_pred0(simd, pred0, at),
                    load4_pred0(simd, pred0, at + 4),
                ),
                *forward,
            );
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
            let (first, second) = blended(
                (
                    load4_pred0(simd, pred0, at),
                    load4_pred0(simd, pred0, at + 4),
                ),
                *forward,
            );
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
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn store_wide<S: Simd>(output: &mut [u16], at: usize, count: usize, lanes: u16x8<S>) {
    match output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>) {
        Some(slot) if count == 8 => lanes.store_array(slot),
        _ => store(output, at, count, lanes.to_array()),
    }
}

/// Stores the first `count` of `lanes` at `at`: a whole vector or half of
/// one in one store, else decomposed into 4/2/1 stores.
#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn store<T: Copy>(output: &mut [T], at: usize, count: usize, lanes: [T; 8]) {
    if count == 8 {
        if let Some(slot) = output.get_mut(at..).and_then(<[T]>::first_chunk_mut::<8>) {
            *slot = lanes;
        }
        return;
    }
    let [l0, l1, l2, l3, l4, l5, l6, l7] = lanes;
    if count >= 4 {
        store4(output, at, 4, [l0, l1, l2, l3]);
        if count > 4 {
            store4(output, at + 4, count - 4, [l4, l5, l6, l7]);
        }
    } else {
        store4(output, at, count, [l0, l1, l2, l3]);
    }
}
