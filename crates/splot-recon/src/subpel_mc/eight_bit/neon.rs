// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit unscaled § 7.13.3.18 interpolation of [`super`] in NEON
//! lanes.
//!
//! The arithmetic is that of the AVX2 byte-lane kernels: with halved taps
//! one pass over eight-bit samples sums exactly in `i16`, one `mla` by lane
//! per tap, and the vertical pass of a two-pass block weighs its rounded
//! horizontal rows into `i32`, one `smlal` pair per tap. A group is eight
//! final-pass values: one row of an 8-column strip, or two rows of a
//! 4-column block. The rows of a vertical pass stay in a register ring, and
//! a compound cell keeps its first predictors in a group array for the
//! average, so no pass stages through memory.

use super::super::block::{Block, Sink, Taps};
use core::arch::aarch64::*;
use fearless_simd::{Neon, SimdBase, SimdFrom, i32x4, u8x16, u16x8};

/// Whether the kernels cover `block`: strips of 8 columns, or 4 columns in
/// pairs of rows with at most two horizontal tap pairs.
pub(in crate::subpel_mc) fn fits<T>(block: &Block<'_, T>) -> bool {
    (block.w > 0 && block.w.is_multiple_of(8))
        || (block.w == 4
            && block.h.is_multiple_of(2)
            && block.horizontal.is_none_or(|taps| taps.count <= 2))
}

/// Calls `$f::<PUT, CELL, W, H, V>` for the run-time group width and the
/// tap-pair counts of both passes, zero for a skipped pass; with `CELL` a
/// 4x4 block, whose rows unroll.
macro_rules! specialize {
    ($four:expr, $h:expr, $v:expr, $f:ident::<$put:ident, $cell:ident> $args:tt) => {
        specialize!(@ $four, $h, $v, $f $put $cell $args,
            [0 0, 0 1, 0 2, 0 3, 0 4, 1 0, 1 1, 1 2, 1 3, 1 4, 2 0, 2 1, 2 2, 2 3, 2 4],
            [3 0, 3 1, 3 2, 3 3, 3 4, 4 0, 4 1, 4 2, 4 3, 4 4])
    };
    (@ $four:expr, $h:expr, $v:expr, $f:ident $put:ident $cell:ident $args:tt,
        [$($h4:literal $v4:literal),*], [$($h8:literal $v8:literal),*]) => {
        if $cell {
            match ($h, $v) {
                $(($h4, $v4) => $f::<$put, true, 4, $h4, $v4> $args,)*
                _ => {}
            }
        } else {
            match ($four, $h, $v) {
                $((true, $h4, $v4) => $f::<$put, false, 4, $h4, $v4> $args,)*
                $((false, $h4, $v4) => $f::<$put, false, 8, $h4, $v4> $args,)*
                $((false, $h8, $v8) => $f::<$put, false, 8, $h8, $v8> $args,)*
                _ => {}
            }
        }
    };
}

fearless_simd::kernel!(
    /// Predicts `block`, which [`fits`], into an eight-bit `sink`.
    #[inline]
    pub(in crate::subpel_mc) fn predict(t: Neon, block: &Block<'_, u8>, sink: &mut Sink<'_>) {
        let w = block.w;
        match sink {
            Sink::Put { output, stride } => {
                let (output, stride) = (&mut **output, *stride);
                passes::<true, false>(t, block, &mut |_, row, col, values| {
                    store_bytes(output, w, row * stride + col, stride, vqmovun_s16(values));
                });
            }
            Sink::PutWide { output, stride, .. } => {
                let (output, stride) = (&mut **output, *stride);
                let (zero, max) = (vdupq_n_s16(0), vdupq_n_s16(255));
                passes::<true, false>(t, block, &mut |_, row, col, values| {
                    let values = vreinterpretq_u16_s16(vminq_s16(vmaxq_s16(values, zero), max));
                    store_words(t, output, w, row * stride + col, stride, values);
                });
            }
            Sink::Prep { output, stride } => {
                let (output, stride) = (&mut **output, *stride);
                passes::<false, false>(t, block, &mut |_, row, col, values| {
                    let at = row * stride + col;
                    let next = if w == 4 { at + stride } else { at + 4 };
                    store_i32(t, output, at, vmovl_s16(vget_low_s16(values)));
                    store_i32(t, output, next, vmovl_high_s16(values));
                });
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (pred0, forward, output, stride) = (&**pred0, *forward, &mut **output, *stride);
                passes::<false, false>(t, block, &mut |_, row, col, values| {
                    let at = row * w + col;
                    let first = vmovn_high_s32(
                        vmovn_s32(load_i32(t, pred0, at)),
                        load_i32(t, pred0, at + 4),
                    );
                    let values = vqmovun_s16(blend(first, values, forward));
                    store_bytes(output, w, row * stride + col, stride, values);
                });
            }
            Sink::AverageWide { .. } => {}
        }
    }
);

fearless_simd::kernel!(
    /// Predicts both blocks of a compound cell of at most 64 samples (or of
    /// two full-sample blocks of any size), which [`fits`], and stores their
    /// COMPOUND_AVERAGE into `output`.
    #[inline]
    pub(in crate::subpel_mc) fn compound(
        t: Neon,
        blocks: [&Block<'_, u8>; 2],
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) {
        if blocks
            .iter()
            .all(|block| block.horizontal.is_none() && block.vertical.is_none())
        {
            fullpel(t, blocks, forward, output, stride);
            return;
        }
        let w = blocks[0].w;
        let mut groups = [vdupq_n_s16(0); 8];
        let mut first = |group: usize, _, _, values| {
            if let Some(slot) = groups.get_mut(group) {
                *slot = values;
            }
        };
        let cell = w == 4 && blocks[0].h == 4;
        if cell {
            passes::<false, true>(t, blocks[0], &mut first);
        } else {
            passes::<false, false>(t, blocks[0], &mut first);
        }
        let mut second = |group: usize, row: usize, col: usize, values| {
            let first = groups.get(group).copied().unwrap_or(values);
            let values = vqmovun_s16(blend(first, values, forward));
            store_bytes(output, w, row * stride + col, stride, values);
        };
        if cell {
            passes::<false, true>(t, blocks[1], &mut second);
        } else {
            passes::<false, false>(t, blocks[1], &mut second);
        }
    }
);

/// [`compound`] of two full-sample blocks: the COMPOUND_AVERAGE of the
/// `<< 4` samples is `(a * f + b * (16 - f) + 8) >> 4`, and for `f == 8` the
/// rounded halving add, 16 samples per vector.
#[inline(never)]
#[target_feature(enable = "neon")]
fn fullpel(t: Neon, blocks: [&Block<'_, u8>; 2], forward: i32, output: &mut [u8], stride: usize) {
    let [first, second] = blocks;
    let remaps = blocks.map(|block| columns(t, block));
    let weights = (vdupq_n_u8(forward as u8), vdupq_n_u8((16 - forward) as u8));
    let starts = [starts(first, first.h), starts(second, first.h)];
    for row in 0..first.h {
        let at = [starts[0](row), starts[1](row)];
        for col in (0..first.w).step_by(16) {
            let p0 = window(t, first.source, at[0] + col, remaps[0]);
            let p1 = window(t, second.source, at[1] + col, remaps[1]);
            let bytes = if forward == 8 {
                vrhaddq_u8(p0, p1)
            } else if (0..=16).contains(&forward) {
                let (front, back) = weights;
                let low = vmull_u8(vget_low_u8(p0), vget_low_u8(front));
                let low = vmlal_u8(low, vget_low_u8(p1), vget_low_u8(back));
                let high = vmlal_high_u8(vmull_high_u8(p0, front), p1, back);
                vrshrn_high_n_u16::<4>(vrshrn_n_u16::<4>(low), high)
            } else {
                let weigh = |p0: uint8x8_t, p1: uint8x8_t| {
                    let sum = vmulq_n_s16(widen(p0), forward as i16);
                    let sum = vmlaq_n_s16(sum, widen(p1), (16 - forward) as i16);
                    vqmovun_s16(vshrq_n_s16::<4>(vaddq_s16(sum, vdupq_n_s16(8))))
                };
                vcombine_u8(
                    weigh(vget_low_u8(p0), vget_low_u8(p1)),
                    weigh(vget_high_u8(p0), vget_high_u8(p1)),
                )
            };
            let Some(slot) = output.get_mut(row * stride + col..) else {
                return;
            };
            if first.w - col >= 16 {
                if let Some(slot) = slot.first_chunk_mut::<16>() {
                    u8x16::<Neon>::simd_from(t, bytes).store_array(slot);
                }
            } else {
                let bytes = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(bytes)).to_le_bytes();
                if first.w - col == 8 {
                    if let Some(slot) = slot.first_chunk_mut::<8>() {
                        *slot = bytes;
                    }
                } else if let (Some(slot), Some(bytes)) =
                    (slot.first_chunk_mut::<4>(), bytes.first_chunk::<4>())
                {
                    *slot = *bytes;
                }
            }
        }
    }
}

/// Runs the passes of `block`, a 4x4 one with `CELL`, handing `out` each
/// group as `(group, row, col, values)`, strip by strip: with `PUT` the
/// single-reference predictions before `Clip1`, else the compound
/// predictors.
#[inline]
#[target_feature(enable = "neon")]
fn passes<const PUT: bool, const CELL: bool>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    let (h, v) = (
        block.horizontal.map_or(0, |taps| taps.count),
        block.vertical.map_or(0, |taps| taps.count),
    );
    specialize!(block.w == 4, h, v, run::<PUT, CELL>(t, block, out));
}

/// Dispatches one pass shape of `block`.
#[inline]
#[target_feature(enable = "neon")]
fn run<const PUT: bool, const CELL: bool, const W: usize, const H: usize, const V: usize>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    match (H, V) {
        (0, 0) => copy::<PUT, CELL, W>(t, block, out),
        (_, 0) => horizontal::<PUT, CELL, W, H>(t, block, out),
        (0, _) => vertical::<PUT, CELL, W, V>(t, block, out),
        _ => two_pass::<PUT, CELL, W, H, V>(t, block, out),
    }
}

/// Sixteen bytes from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load16(t: Neon, samples: &[u8], at: usize) -> uint8x16_t {
    match samples.get(at..).and_then(<[u8]>::first_chunk::<16>) {
        Some(bytes) => u8x16::load_array_ref(t, bytes).into(),
        None => vdupq_n_u8(0),
    }
}

/// Eight bytes from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load8(samples: &[u8], at: usize) -> uint8x8_t {
    let word = samples
        .get(at..)
        .and_then(<[u8]>::first_chunk::<8>)
        .map_or(0, |bytes| u64::from_le_bytes(*bytes));
    vcreate_u8(word)
}

/// Four `i32` values from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load_i32(t: Neon, values: &[i32], at: usize) -> int32x4_t {
    match values.get(at..).and_then(<[i32]>::first_chunk::<4>) {
        Some(lanes) => i32x4::load_array_ref(t, lanes).into(),
        None => vdupq_n_s32(0),
    }
}

/// Widens eight bytes to `i16` lanes.
#[inline]
#[target_feature(enable = "neon")]
fn widen(bytes: uint8x8_t) -> int16x8_t {
    vreinterpretq_s16_u16(vmovl_u8(bytes))
}

/// The halved taps of a pass in `i16` lanes, zero past its pairs.
#[inline]
#[target_feature(enable = "neon")]
fn weights(t: Neon, taps: Option<&Taps>) -> int16x8_t {
    taps.map_or(vdupq_n_s16(0), |taps| {
        vreinterpretq_s16_s32(i32x4::load_array_ref(t, &taps.words).into())
    })
}

/// The `tbl` indices of a block's column remap.
#[inline]
#[target_feature(enable = "neon")]
fn columns(t: Neon, block: &Block<'_, u8>) -> Option<uint8x16_t> {
    block.remap.map(|remap| load16(t, &remap, 0))
}

/// The source index of each of the first `rows` window rows of `block`:
/// stepped by the stride while those rows are inside the clip bounds.
#[inline]
fn starts<'a>(block: &'a Block<'_, u8>, rows: usize) -> impl Fn(usize) -> usize + 'a {
    let inside = block.top >= block.rows[0] && block.top + rows as isize - 1 <= block.rows[1];
    let base = block.start(0);
    move |row| {
        if inside {
            base + row * block.stride
        } else {
            block.start(row)
        }
    }
}

/// Window columns `col..col + 16` of the window row from `start`.
#[inline]
#[target_feature(enable = "neon")]
fn window(t: Neon, source: &[u8], start: usize, remap: Option<uint8x16_t>) -> uint8x16_t {
    let bytes = load16(t, source, start);
    remap.map_or(bytes, |remap| vqtbl1q_u8(bytes, remap))
}

/// Byte indices of window columns `k..k + 4` (lanes `0..4`) and `k + 1..k +
/// 5` (lanes `8..12`) of a row in bytes `0..8`, and the same of a row in
/// bytes `8..16` in lanes `4..8` and `12..16`, for `k` 0 and 2.
const PAIR_TAPS: [[u8; 16]; 2] = {
    let mut out = [[0u8; 16]; 2];
    let mut pair = 0;
    while pair < 2 {
        let mut lane = 0;
        while lane < 16 {
            out[pair][lane] = (2 * pair + lane / 8 + lane % 4 + lane / 4 % 2 * 8) as u8;
            lane += 1;
        }
        pair += 1;
    }
    out
};

/// [`PAIR_TAPS`] read through the column remap of a 4-column block.
#[inline]
#[target_feature(enable = "neon")]
fn pair_taps(t: Neon, block: &Block<'_, u8>) -> [uint8x16_t; 2] {
    let taps = PAIR_TAPS.map(|taps| load16(t, &taps, 0));
    let Some(remap) = columns(t, block) else {
        return taps;
    };
    let (rows, cols) = (vdupq_n_u8(8), vdupq_n_u8(7));
    taps.map(|taps| {
        let column = vqtbl1q_u8(remap, vandq_u8(taps, cols));
        vorrq_u8(vandq_u8(taps, rows), column)
    })
}

#[inline]
#[target_feature(enable = "neon")]
fn mla<const L: i32>(sum: int16x8_t, x: int16x8_t, weights: int16x8_t) -> int16x8_t {
    vmlaq_laneq_s16::<L>(sum, x, weights)
}

/// `Σ` over `2 * P` taps of `tap(k) * weights[k]` in `i16` lanes.
#[inline]
#[target_feature(enable = "neon")]
fn sums16<const P: usize>(
    weights: int16x8_t,
    mut tap: impl FnMut(usize) -> int16x8_t,
) -> int16x8_t {
    let mut sum = vmulq_laneq_s16::<0>(tap(0), weights);
    sum = mla::<1>(sum, tap(1), weights);
    if P >= 2 {
        sum = mla::<2>(sum, tap(2), weights);
        sum = mla::<3>(sum, tap(3), weights);
    }
    if P >= 3 {
        sum = mla::<4>(sum, tap(4), weights);
        sum = mla::<5>(sum, tap(5), weights);
    }
    if P >= 4 {
        sum = mla::<6>(sum, tap(6), weights);
        sum = mla::<7>(sum, tap(7), weights);
    }
    sum
}

/// The `i32` sums of a group: outputs `0..4` and `4..8`.
type Sums = (int32x4_t, int32x4_t);

#[inline]
#[target_feature(enable = "neon")]
fn mac<const L: i32>((low, high): Sums, x: int16x8_t, weights: int16x8_t) -> Sums {
    (
        vmlal_laneq_s16::<L>(low, vget_low_s16(x), weights),
        vmlal_high_laneq_s16::<L>(high, x, weights),
    )
}

/// `Σ` over `2 * P` taps of `tap(k) * weights[k]` in `i32` lanes.
#[inline]
#[target_feature(enable = "neon")]
fn sums32<const P: usize>(weights: int16x8_t, mut tap: impl FnMut(usize) -> int16x8_t) -> Sums {
    let x = tap(0);
    let mut sums = (
        vmull_laneq_s16::<0>(vget_low_s16(x), weights),
        vmull_high_laneq_s16::<0>(x, weights),
    );
    sums = mac::<1>(sums, tap(1), weights);
    if P >= 2 {
        sums = mac::<2>(sums, tap(2), weights);
        sums = mac::<3>(sums, tap(3), weights);
    }
    if P >= 3 {
        sums = mac::<4>(sums, tap(4), weights);
        sums = mac::<5>(sums, tap(5), weights);
    }
    if P >= 4 {
        sums = mac::<6>(sums, tap(6), weights);
        sums = mac::<7>(sums, tap(7), weights);
    }
    sums
}

/// The halved-tap horizontal sums of eight outputs of the window row from
/// `start`: zipped with zero bytes, its halves stay `i16` vectors that the
/// taps slide, one `ext` each.
#[inline]
#[target_feature(enable = "neon")]
fn row_sums<const P: usize>(
    t: Neon,
    source: &[u8],
    start: usize,
    weights: int16x8_t,
    remap: Option<uint8x16_t>,
) -> int16x8_t {
    let bytes = window(t, source, start, remap);
    let zero = vdupq_n_u8(0);
    let (a, b) = (
        vreinterpretq_s16_u8(vzip1q_u8(bytes, zero)),
        vreinterpretq_s16_u8(vzip2q_u8(bytes, zero)),
    );
    sums16::<P>(weights, |k| match k {
        0 => a,
        1 => vextq_s16::<1>(a, b),
        2 => vextq_s16::<2>(a, b),
        3 => vextq_s16::<3>(a, b),
        4 => vextq_s16::<4>(a, b),
        5 => vextq_s16::<5>(a, b),
        6 => vextq_s16::<6>(a, b),
        _ => vextq_s16::<7>(a, b),
    })
}

/// The halved-tap horizontal sums of four outputs of the window rows from
/// `starts`, the first row in lanes `0..4`.
#[inline]
#[target_feature(enable = "neon")]
fn pair_sums<const P: usize>(
    source: &[u8],
    starts: [usize; 2],
    weights: int16x8_t,
    columns: &[uint8x16_t; 2],
) -> int16x8_t {
    let bytes = vcombine_u8(load8(source, starts[0]), load8(source, starts[1]));
    let zero = vdupq_n_u8(0);
    let pair = |pair: usize| {
        let lanes = vqtbl1q_u8(bytes, columns[pair]);
        (
            vreinterpretq_s16_u8(vzip1q_u8(lanes, zero)),
            vreinterpretq_s16_u8(vzip2q_u8(lanes, zero)),
        )
    };
    let first = pair(0);
    let second = if P > 1 { pair(1) } else { first };
    sums16::<P>(weights, |k| match k {
        0 => first.0,
        1 => first.1,
        2 => second.0,
        _ => second.1,
    })
}

/// A block without passes: its samples, `<< 4` for a compound predictor.
#[inline(never)]
#[target_feature(enable = "neon")]
fn copy<const PUT: bool, const CELL: bool, const W: usize>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    let rows = if CELL { 4 } else { block.h };
    let remap = columns(t, block);
    let scale = |x: int16x8_t| if PUT { x } else { vshlq_n_s16::<4>(x) };
    let start = starts(block, rows);
    if W == 4 {
        for row in (0..rows).step_by(2) {
            let rows = [row, row + 1].map(|row| window(t, block.source, start(row), remap));
            let lanes = vreinterpretq_u8_u32(vzip1q_u32(
                vreinterpretq_u32_u8(rows[0]),
                vreinterpretq_u32_u8(rows[1]),
            ));
            out(row / 2, row, 0, scale(widen(vget_low_u8(lanes))));
        }
        return;
    }
    for col in (0..block.w).step_by(8) {
        for row in 0..rows {
            let bytes = window(t, block.source, start(row) + col, remap);
            out(
                col / 8 * rows + row,
                row,
                col,
                scale(widen(vget_low_u8(bytes))),
            );
        }
    }
}

/// A block with only a horizontal pass of `H` tap pairs.
#[inline(never)]
#[target_feature(enable = "neon")]
fn horizontal<const PUT: bool, const CELL: bool, const W: usize, const H: usize>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    let rows = if CELL { 4 } else { block.h };
    let weights = weights(t, block.horizontal);
    let finish = |sums: int16x8_t| {
        if PUT {
            vshrq_n_s16::<6>(vaddq_s16(sums, vdupq_n_s16(34)))
        } else {
            vrshrq_n_s16::<2>(sums)
        }
    };
    let start = starts(block, rows);
    if W == 4 {
        let columns = pair_taps(t, block);
        for row in (0..rows).step_by(2) {
            let starts = [start(row), start(row + 1)];
            out(
                row / 2,
                row,
                0,
                finish(pair_sums::<H>(block.source, starts, weights, &columns)),
            );
        }
        return;
    }
    let remap = columns(t, block);
    for col in (0..block.w).step_by(8) {
        for row in 0..rows {
            let sums = row_sums::<H>(t, block.source, start(row) + col, weights, remap);
            out(col / 8 * rows + row, row, col, finish(sums));
        }
    }
}

/// A block with only a vertical pass of `V` tap pairs over its samples; the
/// last row pair of a 4-column block reads a row past the window that no
/// tap weighs.
#[inline(never)]
#[target_feature(enable = "neon")]
fn vertical<const PUT: bool, const CELL: bool, const W: usize, const V: usize>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    let rows = if CELL { 4 } else { block.h };
    let weights = weights(t, block.vertical);
    let remap = columns(t, block);
    let finish = |sums: int16x8_t| {
        if PUT {
            vrshrq_n_s16::<6>(sums)
        } else {
            vrshrq_n_s16::<2>(sums)
        }
    };
    let start = starts(block, rows + 2 * V - 1);
    if W == 4 {
        let line = |row: usize| {
            let rows = [row, row + 1].map(|row| window(t, block.source, start(row), remap));
            let lanes = vzip1q_u32(vreinterpretq_u32_u8(rows[0]), vreinterpretq_u32_u8(rows[1]));
            widen(vget_low_u8(vreinterpretq_u8_u32(lanes)))
        };
        let mut pairs: [int16x8_t; 4] =
            core::array::from_fn(|k| if k < V { line(2 * k) } else { vdupq_n_s16(0) });
        for row in (0..rows).step_by(2) {
            let next = line(row + 2 * V);
            let pair = |k: usize| if k < V { pairs[k] } else { next };
            let sums = sums16::<V>(weights, |k| {
                if k % 2 == 0 {
                    pair(k / 2)
                } else {
                    vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1))
                }
            });
            out(row / 2, row, 0, finish(sums));
            pairs = core::array::from_fn(|k| pair(k + 1));
        }
        return;
    }
    for col in (0..block.w).step_by(8) {
        let line = |row: usize| {
            widen(vget_low_u8(window(
                t,
                block.source,
                start(row) + col,
                remap,
            )))
        };
        let mut ring: [int16x8_t; 7] = core::array::from_fn(|k| {
            if k + 1 < 2 * V {
                line(k)
            } else {
                vdupq_n_s16(0)
            }
        });
        for row in 0..rows {
            let next = line(row + 2 * V - 1);
            let tap = |k: usize| if k + 1 < 2 * V { ring[k] } else { next };
            out(
                col / 8 * rows + row,
                row,
                col,
                finish(sums16::<V>(weights, tap)),
            );
            ring = core::array::from_fn(|k| tap(k + 1));
        }
    }
}

/// Both passes of `block`, `H` and `V` tap pairs: the `Round2(sum,
/// InterRound0)` horizontal rows of each strip in a register ring, two rows
/// per vector for a 4-column block, whose last pair reads a row past the
/// window that no tap weighs.
#[inline(never)]
#[target_feature(enable = "neon")]
fn two_pass<const PUT: bool, const CELL: bool, const W: usize, const H: usize, const V: usize>(
    t: Neon,
    block: &Block<'_, u8>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    let rows = if CELL { 4 } else { block.h };
    let (h_weights, v_weights) = (weights(t, block.horizontal), weights(t, block.vertical));
    let finish = |(low, high): Sums| {
        if PUT {
            vrshrn_high_n_s32::<10>(vrshrn_n_s32::<10>(low), high)
        } else {
            vrshrn_high_n_s32::<6>(vrshrn_n_s32::<6>(low), high)
        }
    };
    let start = starts(block, rows + 2 * V - 1);
    if W == 4 {
        let columns = pair_taps(t, block);
        let line = |row: usize| {
            let starts = [start(row), start(row + 1)];
            vrshrq_n_s16::<2>(pair_sums::<H>(block.source, starts, h_weights, &columns))
        };
        let mut pairs: [int16x8_t; 4] =
            core::array::from_fn(|k| if k < V { line(2 * k) } else { vdupq_n_s16(0) });
        for row in (0..rows).step_by(2) {
            let next = line(row + 2 * V);
            let pair = |k: usize| if k < V { pairs[k] } else { next };
            let sums = sums32::<V>(v_weights, |k| {
                if k % 2 == 0 {
                    pair(k / 2)
                } else {
                    vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1))
                }
            });
            out(row / 2, row, 0, finish(sums));
            pairs = core::array::from_fn(|k| pair(k + 1));
        }
        return;
    }
    let remap = columns(t, block);
    for col in (0..block.w).step_by(8) {
        let line = |row: usize| {
            let start = start(row) + col;
            vrshrq_n_s16::<2>(row_sums::<H>(t, block.source, start, h_weights, remap))
        };
        let mut ring: [int16x8_t; 7] = core::array::from_fn(|k| {
            if k + 1 < 2 * V {
                line(k)
            } else {
                vdupq_n_s16(0)
            }
        });
        for row in 0..rows {
            let next = line(row + 2 * V - 1);
            let tap = |k: usize| if k + 1 < 2 * V { ring[k] } else { next };
            out(
                col / 8 * rows + row,
                row,
                col,
                finish(sums32::<V>(v_weights, tap)),
            );
            ring = core::array::from_fn(|k| tap(k + 1));
        }
    }
}

/// The COMPOUND_AVERAGE of two groups of compound predictors, before
/// `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn blend(first: int16x8_t, second: int16x8_t, forward: i32) -> int16x8_t {
    if forward == 8 {
        return vrshrq_n_s16::<5>(vaddq_s16(first, second));
    }
    let (f, b) = (forward as i16, (16 - forward) as i16);
    let low = vmlal_n_s16(vmull_n_s16(vget_low_s16(first), f), vget_low_s16(second), b);
    let high = vmlal_high_n_s16(vmull_high_n_s16(first, f), second, b);
    vrshrn_high_n_s32::<8>(vrshrn_n_s32::<8>(low), high)
}

/// Four `i32` values into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_i32(t: Neon, output: &mut [i32], at: usize, values: int32x4_t) {
    if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>) {
        i32x4::<Neon>::simd_from(t, values).store_array(slot);
    }
}

/// The eight bytes of a group into `output` from `at`: one row of a
/// `w`-wide strip, or two rows `stride` apart of a 4-wide block.
#[inline]
#[target_feature(enable = "neon")]
fn store_bytes(output: &mut [u8], w: usize, at: usize, stride: usize, values: uint8x8_t) {
    let bytes = vget_lane_u64::<0>(vreinterpret_u64_u8(values)).to_le_bytes();
    if w != 4 {
        if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
            *slot = bytes;
        }
        return;
    }
    for (at, lanes) in [at, at + stride].into_iter().zip(bytes.as_chunks::<4>().0) {
        if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
            *slot = *lanes;
        }
    }
}

/// The eight words of a group into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_words(
    t: Neon,
    output: &mut [u16],
    w: usize,
    at: usize,
    stride: usize,
    values: uint16x8_t,
) {
    let values = u16x8::<Neon>::simd_from(t, values);
    if w != 4 {
        if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>) {
            values.store_array(slot);
        }
        return;
    }
    let lanes = values.to_array();
    for (at, lanes) in [at, at + stride].into_iter().zip(lanes.as_chunks::<4>().0) {
        if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<4>) {
            *slot = *lanes;
        }
    }
}
