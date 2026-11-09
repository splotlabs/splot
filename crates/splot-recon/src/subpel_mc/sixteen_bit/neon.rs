// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unscaled § 7.13.3.18 interpolation of 16-bit storage in NEON lanes.
//!
//! A group is eight outputs: one row of an 8-column strip, or two rows of a
//! 4-column block. Each tap is one `smlal` by lane into `i32`, each rounding
//! one `srshl` or `rshrn`, and each clip one `sqxtun` and one `umin`. The
//! stage has the layout of the portable kernels, whose values these match.

use super::super::block::{Block, Sink, Taps};
use core::arch::aarch64::*;
use fearless_simd::{Neon, SimdBase, SimdFrom, i16x8, i32x4, u8x16, u16x8};

/// The `i32` sums of a group: outputs `0..4` and `4..8`.
type Sums = (int32x4_t, int32x4_t);

/// Calls `$f::<W, P>` for the run-time group width and pair count.
macro_rules! specialize {
    ($width:expr, $pairs:expr, $f:ident($($arg:expr),*)) => {
        match ($width, $pairs) {
            (4, 1) => $f::<4, 1>($($arg),*),
            (4, 2) => $f::<4, 2>($($arg),*),
            (4, 3) => $f::<4, 3>($($arg),*),
            (4, _) => $f::<4, 4>($($arg),*),
            (_, 1) => $f::<8, 1>($($arg),*),
            (_, 2) => $f::<8, 2>($($arg),*),
            (_, 3) => $f::<8, 3>($($arg),*),
            (_, _) => $f::<8, 4>($($arg),*),
        }
    };
}

fearless_simd::kernel!(
    #[inline]
    pub(super) fn predict(t: Neon, block: &Block<'_, u16>, sink: &mut Sink<'_>, stage: &mut [i16]) {
        let w = block.w;
        match sink {
            Sink::Put { output, stride } => {
                let (output, stride, max) = (&mut **output, *stride, vdupq_n_u16(255));
                put(t, block, stage, &mut move |width, row, col, values| {
                    let values = vminq_u16(values, max);
                    store_bytes(output, width, row * stride + col, stride, values);
                });
            }
            Sink::PutWide {
                output,
                stride,
                max,
            } => {
                let (output, stride, max) = (&mut **output, *stride, vdupq_n_u16(*max));
                put(t, block, stage, &mut move |width, row, col, values| {
                    let values = vminq_u16(values, max);
                    store_words(t, output, width, row * stride + col, stride, values);
                });
            }
            Sink::Prep { output, stride } => {
                let (output, stride) = (&mut **output, *stride);
                passes(t, block, false, stage, &mut move |width, row, col, sums| {
                    store_prep(t, output, width, row * stride + col, stride, sums);
                });
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (pred0, forward, output, stride) = (&**pred0, *forward, &mut **output, *stride);
                let max = vdupq_n_u16(255);
                passes(t, block, false, stage, &mut move |width, row, col, sums| {
                    let values = vminq_u16(average(t, pred0, row * w + col, sums, forward), max);
                    store_bytes(output, width, row * stride + col, stride, values);
                });
            }
            Sink::AverageWide {
                pred0,
                forward,
                output,
                stride,
                max,
            } => {
                let (pred0, forward, output, stride) = (&**pred0, *forward, &mut **output, *stride);
                let max = vdupq_n_u16(*max);
                passes(t, block, false, stage, &mut move |width, row, col, sums| {
                    let values = vminq_u16(average(t, pred0, row * w + col, sums, forward), max);
                    store_words(t, output, width, row * stride + col, stride, values);
                });
            }
        }
    }
);

fearless_simd::kernel!(
    #[inline]
    pub(super) fn compound(
        t: Neon,
        blocks: [&Block<'_, u16>; 2],
        forward: i32,
        max: u16,
        stage: &mut [i16],
        output: &mut [u16],
        stride: usize,
    ) {
        let w = blocks[0].w;
        let mut pred0 = [0i32; 64];
        let prep = &mut Sink::Prep {
            output: &mut pred0,
            stride: w,
        };
        predict(t, blocks[0], prep, stage);
        let average = &mut Sink::AverageWide {
            pred0: &pred0,
            forward,
            output,
            stride,
            max,
        };
        predict(t, blocks[1], average, stage);
    }
);

/// Runs the passes of a single-reference `block`, handing `out` each group
/// as `(W, row, col, values)` saturated to `u16` before `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn put(
    t: Neon,
    block: &Block<'_, u16>,
    stage: &mut [i16],
    out: &mut impl FnMut(usize, usize, usize, uint16x8_t),
) {
    if block.horizontal.is_some() || block.vertical.is_some() {
        passes(
            t,
            block,
            true,
            stage,
            &mut |width, row, col, (low, high)| {
                out(width, row, col, vqmovun_high_s32(vqmovun_s32(low), high));
            },
        );
    } else if block.w == 4 {
        copy::<4>(t, block, &mut |w, row, col, x| {
            out(w, row, col, vreinterpretq_u16_s16(x));
        });
    } else {
        copy::<8>(t, block, &mut |w, row, col, x| {
            out(w, row, col, vreinterpretq_u16_s16(x));
        });
    }
}

/// Runs the passes of `block`, handing `out` each group as
/// `(W, row, col, sums)`: compound predictors, or with `put` single-reference
/// predictions before `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn passes(
    t: Neon,
    block: &Block<'_, u16>,
    put: bool,
    stage: &mut [i16],
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let width = if block.w == 4 { 4 } else { 8 };
    match (block.horizontal, block.vertical) {
        (None, None) => {
            let mut widen = |w, row, col, x: int16x8_t| {
                let sums = if put {
                    let x = vreinterpretq_u16_s16(x);
                    (
                        vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(x))),
                        vreinterpretq_s32_u32(vmovl_high_u16(x)),
                    )
                } else {
                    (vshll_n_s16::<4>(vget_low_s16(x)), vshll_high_n_s16::<4>(x))
                };
                out(w, row, col, sums);
            };
            if width == 4 {
                copy::<4>(t, block, &mut widen);
            } else {
                copy::<8>(t, block, &mut widen);
            }
        }
        (Some(taps), None) => {
            specialize!(width, taps.count, horizontal(t, block, taps, put, out));
        }
        (None, Some(taps)) => {
            specialize!(width, taps.count, vertical(t, block, taps, put, out));
        }
        (Some(h_taps), Some(v_taps)) => {
            let rows = block.h + v_taps.span() - 1;
            specialize!(
                width,
                h_taps.count,
                stage_rows(t, block, h_taps, rows, stage)
            );
            specialize!(
                width,
                v_taps.count,
                filter_stage(t, block, v_taps, rows, put, stage, out)
            );
        }
    }
}

/// Eight samples from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load(t: Neon, samples: &[u16], at: usize) -> int16x8_t {
    match samples.get(at..).and_then(<[u16]>::first_chunk::<8>) {
        Some(lanes) => vreinterpretq_s16_u16(u16x8::load_array_ref(t, lanes).into()),
        None => vdupq_n_s16(0),
    }
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

/// The eight samples of the group at window row `row`, column `col`.
#[inline]
#[target_feature(enable = "neon")]
fn group<const W: usize>(t: Neon, block: &Block<'_, u16>, row: usize, col: usize) -> int16x8_t {
    let first = load(t, block.source, block.start(row) + col);
    if W == 8 {
        return first;
    }
    let second = load(t, block.source, block.start(row + 1) + col);
    vcombine_s16(vget_low_s16(first), vget_low_s16(second))
}

/// The eight taps of `taps`, zero past its pairs.
#[inline]
#[target_feature(enable = "neon")]
fn weights(t: Neon, taps: &Taps) -> int16x8_t {
    let lanes = core::array::from_fn(|tap| (taps.full[tap / 2] >> (16 * (tap % 2))) as i16);
    i16x8::load_array_ref(t, &lanes).into()
}

/// `tbl` indices of a column-clamped window: the bytes of window columns
/// `0..8` and `8..16` in the 32 bytes of the 16 samples from its origin.
#[inline]
#[target_feature(enable = "neon")]
fn remap(t: Neon, block: &Block<'_, u16>) -> Option<[uint8x16_t; 2]> {
    let columns: uint8x16_t = u8x16::load_array(t, block.remap?).into();
    let even = vaddq_u8(columns, columns);
    let odd = vaddq_u8(even, vdupq_n_u8(1));
    Some([vzip1q_u8(even, odd), vzip2q_u8(even, odd)])
}

/// Window columns `0..8` and `8..16` of the 16 samples `(a, b)`.
#[inline]
#[target_feature(enable = "neon")]
fn gather(a: int16x8_t, b: int16x8_t, remap: Option<&[uint8x16_t; 2]>) -> (int16x8_t, int16x8_t) {
    match remap {
        None => (a, b),
        Some([low, high]) => {
            let table = uint8x16x2_t(vreinterpretq_u8_s16(a), vreinterpretq_u8_s16(b));
            (
                vreinterpretq_s16_u8(vqtbl2q_u8(table, *low)),
                vreinterpretq_s16_u8(vqtbl2q_u8(table, *high)),
            )
        }
    }
}

/// Window columns `k..k + 8` of the 16 samples `(a, b)`.
#[inline]
#[target_feature(enable = "neon")]
fn slide(a: int16x8_t, b: int16x8_t, k: usize) -> int16x8_t {
    match k {
        0 => a,
        1 => vextq_s16::<1>(a, b),
        2 => vextq_s16::<2>(a, b),
        3 => vextq_s16::<3>(a, b),
        4 => vextq_s16::<4>(a, b),
        5 => vextq_s16::<5>(a, b),
        6 => vextq_s16::<6>(a, b),
        _ => vextq_s16::<7>(a, b),
    }
}

#[inline]
#[target_feature(enable = "neon")]
fn mac<const L: i32>((low, high): Sums, x: int16x8_t, weights: int16x8_t) -> Sums {
    (
        vmlal_laneq_s16::<L>(low, vget_low_s16(x), weights),
        vmlal_high_laneq_s16::<L>(high, x, weights),
    )
}

/// `Σ` over `2 * P` taps of `row(k) * weights[k]`.
#[inline]
#[target_feature(enable = "neon")]
fn sums<const P: usize>(weights: int16x8_t, mut row: impl FnMut(usize) -> int16x8_t) -> Sums {
    let x = row(0);
    let mut sums = (
        vmull_laneq_s16::<0>(vget_low_s16(x), weights),
        vmull_high_laneq_s16::<0>(x, weights),
    );
    sums = mac::<1>(sums, row(1), weights);
    if P >= 2 {
        sums = mac::<2>(sums, row(2), weights);
        sums = mac::<3>(sums, row(3), weights);
    }
    if P >= 3 {
        sums = mac::<4>(sums, row(4), weights);
        sums = mac::<5>(sums, row(5), weights);
    }
    if P >= 4 {
        sums = mac::<6>(sums, row(6), weights);
        sums = mac::<7>(sums, row(7), weights);
    }
    sums
}

/// [`sums`] of four lanes.
#[inline]
#[target_feature(enable = "neon")]
fn sums4<const P: usize>(weights: int16x8_t, mut row: impl FnMut(usize) -> int16x4_t) -> int32x4_t {
    let mut sum = vmull_laneq_s16::<0>(row(0), weights);
    sum = vmlal_laneq_s16::<1>(sum, row(1), weights);
    if P >= 2 {
        sum = vmlal_laneq_s16::<2>(sum, row(2), weights);
        sum = vmlal_laneq_s16::<3>(sum, row(3), weights);
    }
    if P >= 3 {
        sum = vmlal_laneq_s16::<4>(sum, row(4), weights);
        sum = vmlal_laneq_s16::<5>(sum, row(5), weights);
    }
    if P >= 4 {
        sum = vmlal_laneq_s16::<6>(sum, row(6), weights);
        sum = vmlal_laneq_s16::<7>(sum, row(7), weights);
    }
    sum
}

/// The horizontal sums of four outputs of the window row from `start`.
#[inline]
#[target_feature(enable = "neon")]
fn horizontal_sums4<const P: usize>(
    t: Neon,
    source: &[u16],
    start: usize,
    weights: int16x8_t,
    remap: Option<&[uint8x16_t; 2]>,
) -> int32x4_t {
    let a = load(t, source, start);
    let b = if P <= 2 && remap.is_none() {
        a
    } else {
        load(t, source, start + 8)
    };
    let (a, b) = gather(a, b, remap);
    sums4::<P>(weights, |k| vget_low_s16(slide(a, b, k)))
}

#[inline]
#[target_feature(enable = "neon")]
fn round((low, high): Sums, shift: int32x4_t) -> Sums {
    (vrshlq_s32(low, shift), vrshlq_s32(high, shift))
}

#[inline(never)]
#[target_feature(enable = "neon")]
fn copy<const W: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    for row in (0..block.h).step_by(8 / W) {
        for col in (0..block.w).step_by(8) {
            out(W, row, col, group::<W>(t, block, row, col));
        }
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
fn horizontal<const W: usize, const P: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    put: bool,
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let weights = weights(t, taps);
    let remap = remap(t, block);
    let (round0, round1) = (vdupq_n_s32(-3), vdupq_n_s32(-4));
    let finish = |sums: Sums| {
        let sums = round(sums, round0);
        if put { round(sums, round1) } else { sums }
    };
    if W == 4 {
        for row in (0..block.h).step_by(2) {
            let sums = |row: usize| {
                horizontal_sums4::<P>(t, block.source, block.start(row), weights, remap.as_ref())
            };
            out(4, row, 0, finish((sums(row), sums(row + 1))));
        }
        return;
    }
    for row in 0..block.h {
        let start = block.start(row);
        let mut a = load(t, block.source, start);
        for col in (0..block.w).step_by(8) {
            let b = load(t, block.source, start + col + 8);
            let window = gather(a, b, remap.as_ref());
            a = b;
            out(
                8,
                row,
                col,
                finish(sums::<P>(weights, |k| slide(window.0, window.1, k))),
            );
        }
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
fn vertical<const W: usize, const P: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    put: bool,
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let weights = weights(t, taps);
    let shift = vdupq_n_s32(if put { -7 } else { -3 });
    let line = |row: usize, col: usize| load(t, block.source, block.start(row) + col);
    if W == 4 {
        let mut r: [int16x4_t; 7] = core::array::from_fn(|k| {
            if k + 1 < 2 * P {
                vget_low_s16(line(k, 0))
            } else {
                vdup_n_s16(0)
            }
        });
        for row in (0..block.h).step_by(2) {
            let next = [
                vget_low_s16(line(row + 2 * P - 1, 0)),
                vget_low_s16(line(row + 2 * P, 0)),
            ];
            let tap = |k: usize| {
                if k + 1 < 2 * P {
                    r[k]
                } else {
                    next[k + 1 - 2 * P]
                }
            };
            let low = sums4::<P>(weights, tap);
            let high = sums4::<P>(weights, |k| tap(k + 1));
            out(4, row, 0, round((low, high), shift));
            r = core::array::from_fn(|k| if k + 1 < 2 * P { tap(k + 2) } else { r[k] });
        }
        return;
    }
    for col in (0..block.w).step_by(8) {
        let mut r: [int16x8_t; 7] = core::array::from_fn(|k| {
            if k + 1 < 2 * P {
                line(k, col)
            } else {
                vdupq_n_s16(0)
            }
        });
        for row in 0..block.h {
            let next = line(row + 2 * P - 1, col);
            let tap = |k: usize| if k + 1 < 2 * P { r[k] } else { next };
            out(8, row, col, round(sums::<P>(weights, tap), shift));
            r = core::array::from_fn(|k| tap(k + 1));
        }
    }
}

/// The horizontal pass: `rows` intermediate rows, `Round2(sum, InterRound0)`,
/// into `stage`; a ragged last pair of 4-column rows repeats the last row.
#[inline(never)]
#[target_feature(enable = "neon")]
fn stage_rows<const W: usize, const P: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    rows: usize,
    stage: &mut [i16],
) {
    let weights = weights(t, taps);
    let remap = remap(t, block);
    let mut store = |at: usize, low: int32x4_t, high: int32x4_t| {
        if let Some(slot) = stage.get_mut(at..).and_then(<[i16]>::first_chunk_mut::<8>) {
            let values = vrshrn_high_n_s32::<3>(vrshrn_n_s32::<3>(low), high);
            i16x8::<Neon>::simd_from(t, values).store_array(slot);
        }
    };
    if W == 4 {
        for row in (0..rows).step_by(2) {
            let sums = |row: usize| {
                horizontal_sums4::<P>(t, block.source, block.start(row), weights, remap.as_ref())
            };
            store(row * 4, sums(row), sums((row + 1).min(rows - 1)));
        }
        return;
    }
    for row in 0..rows {
        let start = block.start(row);
        let mut a = load(t, block.source, start);
        for col in (0..block.w).step_by(8) {
            let b = load(t, block.source, start + col + 8);
            let window = gather(a, b, remap.as_ref());
            a = b;
            let (low, high) = sums::<P>(weights, |k| slide(window.0, window.1, k));
            store((col / 8 * rows + row) * 8, low, high);
        }
    }
}

/// The vertical pass over [`stage_rows`].
#[inline(never)]
#[target_feature(enable = "neon")]
fn filter_stage<const W: usize, const P: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    rows: usize,
    put: bool,
    stage: &[i16],
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let weights = weights(t, taps);
    let shift = vdupq_n_s32(if put { -11 } else { -7 });
    let line = |at: usize| match stage.get(at..).and_then(<[i16]>::first_chunk::<8>) {
        Some(lanes) => i16x8::load_array_ref(t, lanes).into(),
        None => vdupq_n_s16(0),
    };
    if W == 4 {
        let mut pairs: [int16x8_t; 4] =
            core::array::from_fn(|k| if k < P { line(k * 8) } else { vdupq_n_s16(0) });
        for row in (0..block.h).step_by(2) {
            let next = line((row + 2 * P) * 4);
            let pair = |k: usize| if k < P { pairs[k] } else { next };
            let sums = sums::<P>(weights, |k| {
                if k % 2 == 0 {
                    pair(k / 2)
                } else {
                    vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1))
                }
            });
            out(4, row, 0, round(sums, shift));
            pairs = core::array::from_fn(|k| pair(k + 1));
        }
        return;
    }
    for col in (0..block.w).step_by(8) {
        let base = col / 8 * rows * 8;
        let mut r: [int16x8_t; 7] = core::array::from_fn(|k| {
            if k + 1 < 2 * P {
                line(base + k * 8)
            } else {
                vdupq_n_s16(0)
            }
        });
        for row in 0..block.h {
            let next = line(base + (row + 2 * P - 1) * 8);
            let tap = |k: usize| if k + 1 < 2 * P { r[k] } else { next };
            out(8, row, col, round(sums::<P>(weights, tap), shift));
            r = core::array::from_fn(|k| tap(k + 1));
        }
    }
}

/// The COMPOUND_AVERAGE of the eight first predictors at `at` of `pred0`
/// and `sums`, saturated to `u16` before `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn average(t: Neon, pred0: &[i32], at: usize, (low, high): Sums, forward: i32) -> uint16x8_t {
    let (first, second) = (load_i32(t, pred0, at), load_i32(t, pred0, at + 4));
    if forward == 8 {
        let low = vqrshrun_n_s32::<5>(vaddq_s32(first, low));
        return vqrshrun_high_n_s32::<5>(low, vaddq_s32(second, high));
    }
    let blend = |pred, value| vmlaq_n_s32(vmulq_n_s32(pred, forward), value, 16 - forward);
    vqrshrun_high_n_s32::<8>(vqrshrun_n_s32::<8>(blend(first, low)), blend(second, high))
}

/// The compound predictors of a group into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_prep(
    t: Neon,
    output: &mut [i32],
    width: usize,
    at: usize,
    stride: usize,
    (low, high): Sums,
) {
    let mut put = |at: usize, values: int32x4_t| {
        if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>) {
            i32x4::<Neon>::simd_from(t, values).store_array(slot);
        }
    };
    put(at, low);
    put(if width == 4 { at + stride } else { at + 4 }, high);
}

/// The eight words `values` of a group into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_words(
    t: Neon,
    output: &mut [u16],
    width: usize,
    at: usize,
    stride: usize,
    values: uint16x8_t,
) {
    let values = u16x8::<Neon>::simd_from(t, values);
    if width == 8 {
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

/// The eight words `values`, at most 255, of a group as bytes into `output`.
#[inline]
#[target_feature(enable = "neon")]
fn store_bytes(output: &mut [u8], width: usize, at: usize, stride: usize, values: uint16x8_t) {
    let bytes = vget_lane_u64::<0>(vreinterpret_u64_u8(vmovn_u16(values))).to_le_bytes();
    if width == 8 {
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
