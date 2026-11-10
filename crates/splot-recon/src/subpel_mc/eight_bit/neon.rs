// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit unscaled § 7.13.3.18 interpolation of [`super`] on 128-bit
//! `i16` lanes, run on AArch64.
//!
//! The arithmetic is that of the AVX2 byte-lane kernels: with halved taps the
//! sums of one pass over eight-bit samples stay in `i16`, and the vertical
//! pass of a two-pass block weighs its rounded horizontal rows in `i32`. A
//! vector holds eight outputs of one row; a 4-wide block uses its low four
//! lanes. The horizontal rows of a two-pass block stay in registers, so the
//! kernels read no stage. Only generic `fearless_simd` operations are used.

use super::super::block::{Block, Sink, Taps};
use fearless_simd::{Simd, i16x8, i32x4, prelude::*, u8x16, u16x8, u32x4, u64x2};

/// Whether the kernels cover `block`: a width of 4 or a multiple of 8, and at
/// most one pass. A block with both passes is faster in the staged portable
/// kernel (measured +3.9% decode time on an 8-bit 4:2:2 stream here).
pub(in crate::subpel_mc) const fn fits<T>(block: &Block<'_, T>) -> bool {
    (block.w == 4 || block.w.is_multiple_of(8))
        && (block.horizontal.is_none() || block.vertical.is_none())
}

/// Where the eight final-pass values of a strip row go.
enum Out<'a, 'b, S: Simd> {
    Sink(&'a mut Sink<'b>),
    /// The compound predictors of a first block, kept for [`Self::Second`].
    First(&'a mut [i16x8<S>; 16]),
    /// The COMPOUND_AVERAGE with the [`Self::First`] predictors into `u8`.
    Second {
        first: &'a [i16x8<S>; 16],
        forward: i32,
        output: &'a mut [u8],
        stride: usize,
    },
}

/// Predicts `block`, whose width [`fits`], into an eight-bit `sink`.
#[inline]
pub(in crate::subpel_mc) fn predict<S: Simd>(simd: S, block: &Block<'_, u8>, sink: &mut Sink<'_>) {
    let put = sink.is_put();
    passes(simd, block, put, &mut Out::Sink(sink));
}

/// Predicts both blocks of a compound cell of at most 64 samples (or of
/// two full-sample blocks of any size), whose widths [`fits`], and stores
/// their COMPOUND_AVERAGE into `output`.
#[inline]
pub(in crate::subpel_mc) fn compound<S: Simd>(
    simd: S,
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    if blocks
        .iter()
        .all(|block| block.horizontal.is_none() && block.vertical.is_none())
    {
        fullpel_compound(simd, blocks, forward, output, stride);
        return;
    }
    let mut first = [i16x8::splat(simd, 0); 16];
    passes(simd, blocks[0], false, &mut Out::First(&mut first));
    passes(
        simd,
        blocks[1],
        false,
        &mut Out::Second {
            first: &first,
            forward,
            output,
            stride,
        },
    );
}

/// [`compound`] of two full-sample blocks: the [`average`] of the `<< 4`
/// samples reduced to `(a * f + b * (16 - f) + 8) >> 4`, with no taps or
/// predictor groups.
#[allow(clippy::inline_always, reason = "full-sample compound cell hot path")]
#[inline(always)]
pub(in crate::subpel_mc) fn fullpel_compound<S: Simd>(
    simd: S,
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    let [first, second] = blocks;
    if first.remap.is_none() && second.remap.is_none() && (0..=16).contains(&forward) {
        let weights = (
            u8x16::splat(simd, forward as u8),
            u8x16::splat(simd, (16 - forward) as u8),
        );
        let load = |block: &Block<'_, u8>, at: usize| {
            block
                .source
                .get(at..)
                .and_then(<[u8]>::first_chunk::<16>)
                .map_or(u8x16::splat(simd, 0), |bytes| {
                    u8x16::load_array_ref(simd, bytes)
                })
        };
        for row in 0..first.h {
            let starts = [first.start(row), second.start(row)];
            for col in (0..first.w).step_by(16) {
                let (a, b) = (load(first, starts[0] + col), load(second, starts[1] + col));
                let (low, high) = if forward == 8 {
                    let (low, high) = simd.widening_add_u8x16(a, b);
                    ((low + 1) >> 1, (high + 1) >> 1)
                } else {
                    let (a0, a1) = simd.widening_mul_u8x16(a, weights.0);
                    let (b0, b1) = simd.widening_mul_u8x16(b, weights.1);
                    ((a0 + b0 + 8) >> 4, (a1 + b1 + 8) >> 4)
                };
                let bytes = simd.narrow_u16x8(low, high);
                let Some(slot) = output.get_mut(row * stride + col..) else {
                    return;
                };
                match first.w - col {
                    4 => {
                        if let Some(slot) = slot.first_chunk_mut::<4>() {
                            *slot = bytes.bitcast::<u32x4<S>>()[0].to_le_bytes();
                        }
                    }
                    8 => {
                        if let Some(slot) = slot.first_chunk_mut::<8>() {
                            *slot = bytes.bitcast::<u64x2<S>>()[0].to_le_bytes();
                        }
                    }
                    _ => {
                        if let Some(slot) = slot.first_chunk_mut::<16>() {
                            bytes.store_array(slot);
                        }
                    }
                }
            }
        }
        return;
    }
    let remaps = blocks.map(|block| block.remap.map(|remap| u8x16::load_array(simd, remap)));
    let weights = (
        i16x8::splat(simd, forward as i16),
        i16x8::splat(simd, (16 - forward) as i16),
    );
    for row in 0..first.h {
        for col in (0..first.w).step_by(8) {
            let (a, _) = samples(simd, first, remaps[0], row, col);
            let (b, _) = samples(simd, second, remaps[1], row, col);
            let values = if forward == 8 {
                (a + b + 1) >> 1
            } else {
                (a * weights.0 + b * weights.1 + 8) >> 4
            };
            store_bytes(
                simd,
                output,
                row * stride + col,
                (first.w - col).min(8),
                values,
            );
        }
    }
}

/// The COMPOUND_AVERAGE `(a * f + b * (16 - f) + 8) >> 4` of two
/// full-sample `N`-square windows, `N` 4 or 8, read from `origins` of their
/// `(samples, stride)` sources, into `output`; `forward` lies in `0..=16`.
#[allow(clippy::inline_always, reason = "full-sample compound cell hot path")]
#[inline(always)]
pub(in crate::subpel_mc) fn fullpel_cell<S: Simd, const N: usize>(
    simd: S,
    sources: [(&[u8], usize); 2],
    origins: [usize; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    let rows = |(samples, step): (&[u8], usize), origin: usize, row: usize| -> u8x16<S> {
        let at = |lane: usize| origin + (row + lane) * step;
        let word = |at: usize| {
            samples
                .get(at..)
                .and_then(<[u8]>::first_chunk::<8>)
                .map_or(0, |bytes| u64::from_le_bytes(*bytes))
        };
        if N == 8 {
            u64x2::from_slice(simd, &[word(at(0)), word(at(1))]).bitcast()
        } else {
            let lanes: [u32; 4] = core::array::from_fn(|lane| {
                samples
                    .get(at(lane)..)
                    .and_then(<[u8]>::first_chunk::<4>)
                    .map_or(0, |bytes| u32::from_le_bytes(*bytes))
            });
            u32x4::from_slice(simd, &lanes).bitcast()
        }
    };
    for row in (0..N).step_by(16 / N) {
        let (a, b) = (
            rows(sources[0], origins[0], row),
            rows(sources[1], origins[1], row),
        );
        let (low, high) = if forward == 8 {
            let (low, high) = simd.widening_add_u8x16(a, b);
            ((low + 1) >> 1, (high + 1) >> 1)
        } else {
            let weights = (
                u8x16::splat(simd, forward as u8),
                u8x16::splat(simd, (16 - forward) as u8),
            );
            let (a0, a1) = simd.widening_mul_u8x16(a, weights.0);
            let (b0, b1) = simd.widening_mul_u8x16(b, weights.1);
            ((a0 + b0 + 8) >> 4, (a1 + b1 + 8) >> 4)
        };
        let bytes = simd.narrow_u16x8(low, high);
        if N == 8 {
            let words = bytes.bitcast::<u64x2<S>>();
            for lane in 0..2 {
                let at = (row + lane) * stride;
                if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
                    *slot = words[lane].to_le_bytes();
                }
            }
        } else {
            let words = bytes.bitcast::<u32x4<S>>();
            for lane in 0..4 {
                let at = (row + lane) * stride;
                if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
                    *slot = words[lane].to_le_bytes();
                }
            }
        }
    }
}

/// Calls `$f::<_, H>` or `$f::<_, H, V>` for the tap-pair counts of a
/// block's passes, zero for a skipped pass.
macro_rules! by_pairs {
    ($pairs:expr, $one:ident, $two:ident $args:tt) => {
        by_pairs!(@ $pairs, $one, $two $args,
            [0, 1, 2, 3, 4],
            [0 1, 0 2, 0 3, 0 4, 1 1, 1 2, 1 3, 1 4, 2 1, 2 2, 2 3, 2 4,
             3 1, 3 2, 3 3, 3 4, 4 1, 4 2, 4 3, 4 4])
    };
    (@ $pairs:expr, $one:ident, $two:ident $args:tt,
        [$($h:literal),*], [$($hh:literal $v:literal),*]) => {
        match $pairs {
            $(($h, 0) => $one::<_, $h> $args,)*
            $(($hh, $v) => $two::<_, $hh, $v> $args,)*
            _ => {}
        }
    };
}

#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn passes<S: Simd>(simd: S, block: &Block<'_, u8>, put: bool, out: &mut Out<'_, '_, S>) {
    let pairs = (
        block.horizontal.map_or(0, |taps| taps.count),
        block.vertical.map_or(0, |taps| taps.count),
    );
    if block.w == 4 && block.h.is_multiple_of(2) && block.remap.is_none() {
        match pairs {
            (0, 1) => return paired_passes::<_, 0, 1>(simd, block, put, out),
            (0, 2) => return paired_passes::<_, 0, 2>(simd, block, put, out),
            (0, 4) => return paired_passes::<_, 0, 4>(simd, block, put, out),
            (1, 1) => return paired_passes::<_, 1, 1>(simd, block, put, out),
            (1, 2) => return paired_passes::<_, 1, 2>(simd, block, put, out),
            (1, 4) => return paired_passes::<_, 1, 4>(simd, block, put, out),
            (2, 1) => return paired_passes::<_, 2, 1>(simd, block, put, out),
            (2, 2) => return paired_passes::<_, 2, 2>(simd, block, put, out),
            (2, 4) => return paired_passes::<_, 2, 4>(simd, block, put, out),
            _ => {}
        }
    }
    by_pairs!(pairs, one_pass, two_passes(simd, block, put, out));
}

/// Byte indices of the two-row pair of window columns `tap..tap + 4` and
/// `tap + 1..tap + 5` of rows held in bytes `0..8` and `8..16`.
const fn pair_taps(tap: usize) -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut lane = 0;
    while lane < 16 {
        out[lane] = (lane / 8 + tap + lane % 4 + lane / 4 % 2 * 8) as u8;
        lane += 1;
    }
    out
}

const PAIR_TAPS: [[u8; 16]; 2] = [pair_taps(0), pair_taps(2)];

/// [`two_passes`] of a 4-wide block of even height with unclamped columns,
/// rows `2k` and `2k + 1` of each pass in the halves of one vector. A 4-wide
/// block's horizontal filter has at most two tap pairs, so a pair's window
/// rows fit eight bytes each.
#[inline(never)]
fn paired_passes<S: Simd, const H: usize, const V: usize>(
    simd: S,
    block: &Block<'_, u8>,
    put: bool,
    out: &mut Out<'_, '_, S>,
) {
    let (h_weights, v_weights) = (halved(simd, block.horizontal), halved(simd, block.vertical));
    let taps = PAIR_TAPS.map(|taps| u8x16::load_array(simd, taps));
    let (bias, shift) = match (H > 0, put) {
        (true, true) => (1 << 9, 10),
        (true, false) | (false, true) => (1 << 5, 6),
        (false, false) => (2, 2),
    };
    let word = |row: usize| {
        block
            .source
            .get(block.start(row)..)
            .and_then(<[u8]>::first_chunk::<8>)
            .map_or(0, |bytes| u64::from_le_bytes(*bytes))
    };
    let pair = |row: usize| -> i16x8<S> {
        let bytes = u64x2::from_slice(simd, &[word(row), word(row + 1)]).bitcast::<u8x16<S>>();
        let columns = |pair: usize| {
            let (low, high) = simd
                .swizzle_dyn_within_blocks_u8x16(bytes, taps[pair])
                .widen();
            (low.bitcast::<i16x8<S>>(), high.bitcast::<i16x8<S>>())
        };
        let (first, second) = columns(0);
        if H == 0 {
            return first;
        }
        let mut sum = first * h_weights[0] + second * h_weights[1];
        if H > 1 {
            let (third, fourth) = columns(1);
            sum += third * h_weights[2] + fourth * h_weights[3];
        }
        (sum + 2) >> 2
    };
    let mut ring = [i16x8::splat(simd, 0); 5];
    for (index, slot) in ring.iter_mut().enumerate().take(V) {
        *slot = pair(2 * index);
    }
    for row in (0..block.h).step_by(2) {
        ring[V] = pair(row + 2 * V);
        let tap = |k: usize| {
            if k.is_multiple_of(2) {
                ring[k / 2]
            } else {
                ring[k / 2].slide::<4>(ring[k / 2 + 1])
            }
        };
        let values = if H > 0 {
            let (mut low, mut high) = (i32x4::splat(simd, bias), i32x4::splat(simd, bias));
            let mut add = |k: usize| {
                let (l, h) = tap(k).widening_mul(v_weights[k]);
                low += l;
                high += h;
            };
            add(0);
            add(1);
            if V > 1 {
                add(2);
                add(3);
            }
            if V > 2 {
                add(4);
                add(5);
            }
            if V > 3 {
                add(6);
                add(7);
            }
            (low >> shift).narrow(high >> shift)
        } else {
            let mut sum = i16x8::splat(simd, bias as i16) + tap(0) * v_weights[0];
            sum += tap(1) * v_weights[1];
            if V > 1 {
                sum += tap(2) * v_weights[2] + tap(3) * v_weights[3];
            }
            if V > 2 {
                sum += tap(4) * v_weights[4] + tap(5) * v_weights[5];
            }
            if V > 3 {
                sum += tap(6) * v_weights[6] + tap(7) * v_weights[7];
            }
            sum >> shift
        };
        emit(simd, out, 4, row, 0, values);
        emit(simd, out, 4, row + 1, 0, values.slide::<4>(values));
        for index in 0..V {
            ring[index] = ring[index + 1];
        }
    }
}

/// The halved taps of a pass splatted into eight-lane vectors.
#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn halved<S: Simd>(simd: S, taps: Option<&Taps>) -> [i16x8<S>; 8] {
    core::array::from_fn(|tap| {
        let pair = taps.map_or(0, |taps| taps.full[tap / 2]);
        let full = if tap % 2 == 0 {
            pair as i16
        } else {
            (pair >> 16) as i16
        };
        i16x8::splat(simd, full / 2)
    })
}

/// The samples of window row `row` from column `col`, widened: columns
/// `col..col + 8` and `col + 8..col + 16`, through the remap of a
/// column-clamped window.
#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn samples<S: Simd>(
    simd: S,
    block: &Block<'_, u8>,
    remap: Option<u8x16<S>>,
    row: usize,
    col: usize,
) -> (i16x8<S>, i16x8<S>) {
    let bytes = block
        .source
        .get(block.start(row) + col..)
        .and_then(<[u8]>::first_chunk::<16>)
        .map_or(u8x16::splat(simd, 0), |bytes| {
            u8x16::load_array_ref(simd, bytes)
        });
    let bytes = match remap {
        Some(remap) => simd.swizzle_dyn_within_blocks_u8x16(bytes, remap),
        None => bytes,
    };
    let (low, high) = bytes.widen();
    (low.bitcast(), high.bitcast())
}

/// The halved-tap sums of the `2 * H` taps of a horizontal pass over the
/// columns `(a, b)`, or the samples `a` without a horizontal pass.
#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn horizontal<S: Simd, const H: usize>(
    (a, b): (i16x8<S>, i16x8<S>),
    weights: &[i16x8<S>; 8],
) -> i16x8<S> {
    if H == 0 {
        return a;
    }
    // Spelled out per pair: as a loop over `slide` arms it compiled to a
    // branchy runtime tap loop.
    let mut sum = a * weights[0] + a.slide::<1>(b) * weights[1];
    if H > 1 {
        sum += a.slide::<2>(b) * weights[2] + a.slide::<3>(b) * weights[3];
    }
    if H > 2 {
        sum += a.slide::<4>(b) * weights[4] + a.slide::<5>(b) * weights[5];
    }
    if H > 3 {
        sum += a.slide::<6>(b) * weights[6] + a.slide::<7>(b) * weights[7];
    }
    sum
}

/// A block whose vertical pass is skipped: the horizontal pass, or a copy.
#[inline(never)]
fn one_pass<S: Simd, const H: usize>(
    simd: S,
    block: &Block<'_, u8>,
    put: bool,
    out: &mut Out<'_, '_, S>,
) {
    let weights = halved(simd, block.horizontal);
    let remap = block.remap.map(|remap| u8x16::load_array(simd, remap));
    for col in (0..block.w).step_by(8) {
        for row in 0..block.h {
            let sums = horizontal::<S, H>(samples(simd, block, remap, row, col), &weights);
            let values = match (H > 0, put) {
                (false, true) => sums,
                (false, false) => sums << 4,
                (true, true) => (sums + 34) >> 6,
                (true, false) => (sums + 2) >> 2,
            };
            emit(simd, out, block.w, row, col, values);
        }
    }
}

/// A block with a vertical pass of `V` tap pairs over its window rows: the
/// samples, or the `Round2(sum, InterRound0)` horizontal rows of a two-pass
/// block, kept in a register ring.
#[inline(never)]
fn two_passes<S: Simd, const H: usize, const V: usize>(
    simd: S,
    block: &Block<'_, u8>,
    put: bool,
    out: &mut Out<'_, '_, S>,
) {
    let (h_weights, v_weights) = (halved(simd, block.horizontal), halved(simd, block.vertical));
    let remap = block.remap.map(|remap| u8x16::load_array(simd, remap));
    let (bias, shift) = match (H > 0, put) {
        (true, true) => (1 << 9, 10),
        (true, false) | (false, true) => (1 << 5, 6),
        (false, false) => (2, 2),
    };
    for col in (0..block.w).step_by(8) {
        let line = |row: usize| {
            let sums = horizontal::<S, H>(samples(simd, block, remap, row, col), &h_weights);
            if H > 0 { (sums + 2) >> 2 } else { sums }
        };
        let mut ring = [i16x8::splat(simd, 0); 8];
        for (row, slot) in ring.iter_mut().enumerate().take(2 * V - 1) {
            *slot = line(row);
        }
        for row in 0..block.h {
            ring[2 * V - 1] = line(row + 2 * V - 1);
            let values = if H > 0 {
                let (mut low, mut high) = (i32x4::splat(simd, bias), i32x4::splat(simd, bias));
                for (line, &weights) in ring.iter().zip(&v_weights).take(2 * V) {
                    let (l, h) = line.widening_mul(weights);
                    low += l;
                    high += h;
                }
                (low >> shift).narrow(high >> shift)
            } else {
                let mut sum = i16x8::splat(simd, bias as i16);
                for (line, &weights) in ring.iter().zip(&v_weights).take(2 * V) {
                    sum += *line * weights;
                }
                sum >> shift
            };
            emit(simd, out, block.w, row, col, values);
            for tap in 1..2 * V {
                ring[tap - 1] = ring[tap];
            }
        }
    }
}

/// The COMPOUND_AVERAGE of two groups of compound predictors, before
/// `Clip1`.
#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn average<S: Simd>(simd: S, first: i16x8<S>, second: i16x8<S>, forward: i32) -> i16x8<S> {
    if forward == 8 {
        return (first + second + 16) >> 5;
    }
    let (f0, f1) = first.widening_mul(i16x8::splat(simd, forward as i16));
    let (s0, s1) = second.widening_mul(i16x8::splat(simd, (16 - forward) as i16));
    ((f0 + s0 + 128) >> 8).narrow((f1 + s1 + 128) >> 8)
}

/// Stores the values of output row `row`, columns `col..col + count`, of a
/// `w`-wide block, `count` 8 or (for a 4-wide block) 4.
#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn emit<S: Simd>(
    simd: S,
    out: &mut Out<'_, '_, S>,
    w: usize,
    row: usize,
    col: usize,
    values: i16x8<S>,
) {
    let count = (w - col).min(8);
    let group = row * w.div_ceil(8) + col / 8;
    match out {
        Out::Sink(sink) => match sink {
            Sink::Put { output, stride } => {
                store_bytes(simd, output, row * *stride + col, count, values);
            }
            Sink::PutWide { output, stride, .. } => {
                let lanes = values.max(0).min(255).bitcast::<u16x8<S>>();
                store(output, row * *stride + col, count, *lanes);
            }
            Sink::Prep { output, stride } => {
                let (low, high) = values.widen();
                let at = row * *stride + col;
                store(output, at, count.min(4), *low);
                if count > 4 {
                    store(output, at + 4, 4, *high);
                }
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let at = row * w + col;
                let load = |at: usize| {
                    pred0
                        .get(at..)
                        .and_then(<[i32]>::first_chunk::<4>)
                        .map_or(i32x4::splat(simd, 0), |lanes| {
                            i32x4::load_array_ref(simd, lanes)
                        })
                };
                let high = if count > 4 {
                    load(at + 4)
                } else {
                    i32x4::splat(simd, 0)
                };
                let first = load(at).narrow(high);
                let blended = average(simd, first, values, *forward);
                store_bytes(simd, output, row * *stride + col, count, blended);
            }
            Sink::AverageWide { .. } => {}
        },
        Out::First(first) => {
            if let Some(slot) = first.get_mut(group) {
                *slot = values;
            }
        }
        Out::Second {
            first,
            forward,
            output,
            stride,
        } => {
            if let Some(&first) = first.get(group) {
                let blended = average(simd, first, values, *forward);
                store_bytes(simd, output, row * *stride + col, count, blended);
            }
        }
    }
}

/// `Clip1` of the first `count` values into `output` from `at`.
#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn store_bytes<S: Simd>(simd: S, output: &mut [u8], at: usize, count: usize, values: i16x8<S>) {
    let unsigned = values.max(0).bitcast::<u16x8<S>>();
    let bytes = simd.saturating_narrow_u16x8(unsigned, unsigned);
    if count == 8 {
        if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
            *slot = bytes.bitcast::<u64x2<S>>()[0].to_le_bytes();
        }
    } else if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
        *slot = bytes.bitcast::<u32x4<S>>()[0].to_le_bytes();
    }
}

/// Stores the first `count` of `lanes`, 8 or 4, into `output` from `at`.
#[allow(clippy::inline_always, reason = "sub-pel hot path")]
#[inline(always)]
fn store<T: Copy, const N: usize>(output: &mut [T], at: usize, count: usize, lanes: [T; N]) {
    let Some(slot) = output.get_mut(at..) else {
        return;
    };
    if count == 8
        && let (Some(slot), Some(lanes)) = (slot.first_chunk_mut::<8>(), lanes.first_chunk::<8>())
    {
        *slot = *lanes;
    } else if let (Some(slot), Some(lanes)) =
        (slot.first_chunk_mut::<4>(), lanes.first_chunk::<4>())
    {
        *slot = *lanes;
    }
}
