// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unscaled § 7.13.3.18 interpolation of 16-bit storage of [`super`] in
//! AVX2 word lanes, on the validated rows of [`rows`].
//!
//! A group is eight `i32` outputs: one row of an 8-column strip, or two rows
//! of a 4-column block. Each tap pair is one `pmaddwd`; a horizontal row of
//! a strip holds its even outputs in the low lane and its odd ones in the
//! high lane. A vertical pass keeps each window row interleaved with the row
//! before it in the register ring of [`rows`], so each new row costs one
//! shift and one blend. A CPU without AVX2 declines to [`super::generic`].

#![allow(clippy::inline_always, reason = "measured word-lane row helpers")]

use super::super::block::{Block, Geometry, Sink, Taps, Window};
use super::super::cell::{Cell as ProgramCell, Plane, Written};
use super::super::rows::{self, Average, Out, Prep, Put, store_pair};
use super::super::{CellPlacement, ReferencePlaneView};
use super::generic;
use core::arch::x86_64::*;
use core::cell::Cell;
use fearless_simd::{Avx2, Simd, SimdBase, SimdFrom, i32x4, i32x8, u8x16, u8x32, u16x8};

fn token() -> Option<Avx2> {
    fearless_simd::Level::baseline().as_avx2()
}

/// Predicts a filtered 16-bit `block` into `sink`, as [`super::predict`]
/// admits.
pub(super) fn predict(block: &Block<'_, u16>, sink: &mut Sink<'_>) {
    match token() {
        Some(t) => predict_lanes(t, block, sink),
        None => generic::predict(block, sink),
    }
}

/// The COMPOUND_AVERAGE, clipped to `clip.1`, of a 4x4 or 8x8 cell pair
/// in registers.
pub(super) fn compound_cell(
    blocks: [&Block<'_, u16>; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) {
    match token() {
        Some(t) => cell_pair(t, blocks, clip, output, stride),
        None => generic::compound_cell(blocks, clip, output, stride),
    }
}

/// Calls `$f::<W, P, _>` for 4-column pairs or 8-column strips and the
/// pair count.
macro_rules! specialize {
    ($four:expr, $pairs:expr, $f:ident($($arg:expr),*)) => {
        match ($four, $pairs) {
            (true, 1) => $f::<4, 1, _>($($arg),*),
            (true, 2) => $f::<4, 2, _>($($arg),*),
            (true, 3) => $f::<4, 3, _>($($arg),*),
            (true, _) => $f::<4, 4, _>($($arg),*),
            (false, 1) => $f::<8, 1, _>($($arg),*),
            (false, 2) => $f::<8, 2, _>($($arg),*),
            (false, 3) => $f::<8, 3, _>($($arg),*),
            (false, _) => $f::<8, 4, _>($($arg),*),
        }
    };
}

/// Calls `$f::<W, H, V, _>` for the group width and the pair counts of
/// both passes.
macro_rules! specialize_pairs {
    ($four:expr, $h:expr, $v:expr, $f:ident $args:tt) => {
        specialize_pairs!(@ $four, $h, $v, $f $args,
            [1 1, 1 2, 1 3, 1 4, 2 1, 2 2, 2 3, 2 4, 3 1, 3 2, 3 3, 3 4, 4 1, 4 2, 4 3, 4 4])
    };
    (@ $four:expr, $h:expr, $v:expr, $f:ident $args:tt, [$($hh:literal $vv:literal),*]) => {
        match ($four, $h.min(4), $v.min(4)) {
            $((true, $hh, $vv) => $f::<4, $hh, $vv, _> $args,)*
            $((false, $hh, $vv) => $f::<8, $hh, $vv, _> $args,)*
            _ => {}
        }
    };
}

fearless_simd::kernel!(
    #[inline]
    fn predict_lanes(t: Avx2, block: &Block<'_, u16>, sink: &mut Sink<'_>) {
        let w = block.w;
        match sink {
            Sink::Put { output, stride } => {
                let stride = *stride;
                shapes(t, block, &mut Put { output, stride }, 255);
            }
            Sink::PutWide {
                output,
                stride,
                max,
            } => {
                let stride = *stride;
                shapes(t, block, &mut Put { output, stride }, *max);
            }
            Sink::Prep { output, stride } => {
                let stride = *stride;
                shapes(t, block, &mut Prep { output, stride }, 0);
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (forward, stride) = (*forward, *stride);
                let out = &mut Average {
                    pred0,
                    w,
                    forward,
                    output,
                    stride,
                };
                shapes(t, block, out, 255);
            }
            Sink::AverageWide {
                pred0,
                forward,
                output,
                stride,
                max,
            } => {
                let (forward, stride) = (*forward, *stride);
                let out = &mut Average {
                    pred0,
                    w,
                    forward,
                    output,
                    stride,
                };
                shapes(t, block, out, *max);
            }
            Sink::PrepNarrow { .. } | Sink::AverageNarrow { .. } => {}
        }
    }
);

/// Runs the kernel of the shape of `block` into `out`, its samples at
/// most `max`.
#[inline]
#[target_feature(enable = "avx2")]
fn shapes(t: Avx2, block: &Block<'_, u16>, out: &mut impl WordOut, max: u16) {
    let four = block.w == 4;
    match (block.horizontal, block.vertical) {
        (Some(taps), None) => specialize!(four, taps.count, horizontal(t, block, taps, out, max)),
        (None, Some(taps)) => specialize!(four, taps.count, vertical(t, block, taps, out, max)),
        (Some(h_taps), Some(v_taps)) => specialize_pairs!(
            four,
            h_taps.count,
            v_taps.count,
            two_pass(t, block, (h_taps, v_taps), out, max)
        ),
        (None, None) => copy(t, block, out, max),
    }
}

/// What a kernel stores from a group: eight outputs of row `a`, or four of
/// each of rows `a` and `b`, as `Clip1` samples ([`Self::words`]) or
/// compound predictors ([`Self::sums`]).
trait WordOut: Out {
    /// Whether the samples are bytes.
    const BYTES: bool = false;
    /// The first predictors of the outputs.
    fn first(t: Avx2, _: &Self::Row<'_>, _: Option<&Self::Row<'_>>) -> __m256i {
        i32x8::splat(t, 0).into()
    }
    /// Stores eight samples: words, or bytes in the low half.
    fn words(_: Avx2, _: Self::Row<'_>, _: Option<Self::Row<'_>>, _: __m128i) {}
    /// Stores eight compound predictors.
    fn sums(_: Avx2, _: Self::Row<'_>, _: Option<Self::Row<'_>>, _: __m256i) {}
}

/// Sample storage of word lanes whose values fit it.
trait Word: Copy {
    /// Whether the samples are bytes.
    const BYTES: bool;
    /// Stores eight samples into row `a`, or lanes `0..4` into `a` and
    /// `4..8` into `b`.
    fn store(t: Avx2, a: &mut [Self], b: Option<&mut [Self]>, values: __m128i);
}

impl Word for u8 {
    const BYTES: bool = true;
    #[inline(always)]
    fn store(t: Avx2, a: &mut [u8], b: Option<&mut [u8]>, values: __m128i) {
        let bytes = u8x16::<Avx2>::simd_from(t, values).to_array();
        let Some(&bytes) = bytes.first_chunk::<8>() else {
            return;
        };
        match (b, a.first_chunk_mut::<8>()) {
            (Some(b), _) => store_pair(a, b, bytes),
            (None, Some(slot)) => *slot = bytes,
            (None, None) => {}
        }
    }
}

impl Word for u16 {
    const BYTES: bool = false;
    #[inline(always)]
    fn store(t: Avx2, a: &mut [u16], b: Option<&mut [u16]>, values: __m128i) {
        let values = u16x8::<Avx2>::simd_from(t, values);
        match (b, a.first_chunk_mut::<8>()) {
            (Some(b), _) => store_pair(a, b, values.to_array()),
            (None, Some(slot)) => values.store_array(slot),
            (None, None) => {}
        }
    }
}

impl<T: Word> WordOut for Put<'_, T> {
    const BYTES: bool = T::BYTES;
    #[inline(always)]
    fn words(t: Avx2, a: &mut [T], b: Option<&mut [T]>, values: __m128i) {
        T::store(t, a, b, values);
    }
}

impl WordOut for Prep<'_, i32> {
    #[inline(always)]
    fn sums(t: Avx2, a: &mut [i32], b: Option<&mut [i32]>, sums: __m256i) {
        let sums = i32x8::<Avx2>::simd_from(t, sums);
        match (b, a.first_chunk_mut::<8>()) {
            (Some(b), _) => store_pair(a, b, sums.to_array()),
            (None, Some(slot)) => sums.store_array(slot),
            (None, None) => {}
        }
    }
}

impl<T: Word> WordOut for Average<'_, i32, T> {
    const BYTES: bool = T::BYTES;
    #[inline(always)]
    fn first(t: Avx2, (a, _): &(&[i32], &mut [T]), b: Option<&(&[i32], &mut [T])>) -> __m256i {
        let half = |lanes: Option<&[i32; 4]>| {
            lanes.map_or(i32x4::splat(t, 0), |lanes| i32x4::load_array_ref(t, lanes))
        };
        match b {
            Some((b, _)) => t
                .combine_i32x4(half(a.first_chunk()), half(b.first_chunk()))
                .into(),
            None => a
                .first_chunk::<8>()
                .map_or(i32x8::splat(t, 0), |lanes| i32x8::load_array_ref(t, lanes))
                .into(),
        }
    }
    #[inline(always)]
    fn words(t: Avx2, (_, a): (&[i32], &mut [T]), b: Option<(&[i32], &mut [T])>, values: __m128i) {
        T::store(t, a, b.map(|(_, b)| b), values);
    }
}

/// `Round2(x, S)` of each lane.
#[inline]
#[target_feature(enable = "avx2")]
fn round<const S: i32>(x: __m256i) -> __m256i {
    _mm256_srai_epi32::<S>(_mm256_add_epi32(x, _mm256_set1_epi32(1 << S >> 1)))
}

/// The COMPOUND_AVERAGE of the first predictors `first` and `sums`, as
/// `f * p0 + (16 - f) * p1 = 16 * p1 + f * (p0 - p1)`, before `Clip1`.
#[inline]
#[target_feature(enable = "avx2")]
fn average(first: __m256i, sums: __m256i, forward: i32) -> __m256i {
    if forward == 8 {
        return round::<5>(_mm256_add_epi32(first, sums));
    }
    let delta = _mm256_mullo_epi32(_mm256_sub_epi32(first, sums), _mm256_set1_epi32(forward));
    round::<8>(_mm256_add_epi32(_mm256_slli_epi32::<4>(sums), delta))
}

/// Stores a group of `O` into row `a`, or rows `a` and `b`: the put
/// `values` clipped to `max`, else the compound predictors `sums`, averaged
/// with the first ones when `O` averages.
#[inline]
#[target_feature(enable = "avx2")]
fn store<O: WordOut>(
    t: Avx2,
    (forward, max): (i32, __m128i),
    (a, b): (O::Row<'_>, Option<O::Row<'_>>),
    values: __m256i,
    sums: __m256i,
) {
    let clip = |x: __m256i| {
        let words = _mm_packus_epi32(_mm256_castsi256_si128(x), _mm256_extracti128_si256::<1>(x));
        let words = _mm_min_epu16(words, max);
        if O::BYTES {
            _mm_packus_epi16(words, words)
        } else {
            words
        }
    };
    if O::PUT {
        O::words(t, a, b, clip(values));
    } else if O::AVERAGE {
        let first = O::first(t, &a, b.as_ref());
        O::words(t, a, b, clip(average(first, sums, forward)));
    } else {
        O::sums(t, a, b, sums);
    }
}

/// The forward weight of `out` and the splat largest sample `max`.
#[inline]
#[target_feature(enable = "avx2")]
fn clip(out: &impl Out, max: u16) -> (i32, __m128i) {
    (out.forward(), _mm_set1_epi16(max as i16))
}

/// The tap pairs of `taps` as `pmaddwd` weights, zero past its pairs.
#[inline]
#[target_feature(enable = "avx2")]
fn weights(taps: &Taps) -> [__m256i; 4] {
    taps.full.map(|pair| _mm256_set1_epi32(pair))
}

/// The sum of `P` tap pairs `pair(p)` weighted by `weights`.
#[inline]
#[target_feature(enable = "avx2")]
fn madds<const P: usize>(weights: &[__m256i; 4], pair: impl Fn(usize) -> __m256i) -> __m256i {
    let mut sum = _mm256_madd_epi16(pair(0), weights[0]);
    unroll!(p < P, [1 2 3] => {
        sum = _mm256_add_epi32(sum, _mm256_madd_epi16(pair(p), weights[p]));
    });
    sum
}

/// Eight samples of `row` from column `k`.
#[inline]
#[target_feature(enable = "avx2")]
fn load8(t: Avx2, row: &[u16], k: usize) -> Option<__m128i> {
    Some(u16x8::load_array_ref(t, row.get(k..)?.first_chunk::<8>()?).into())
}

/// Eight samples of `row` from column `j` in the low lane and from `k` in
/// the high lane.
#[inline]
#[target_feature(enable = "avx2")]
fn load2x8(t: Avx2, row: &[u16], j: usize, k: usize) -> Option<__m256i> {
    Some(_mm256_set_m128i(load8(t, row, k)?, load8(t, row, j)?))
}

/// The horizontal sums of `P` tap pairs of the eight outputs of `row`:
/// even outputs in the low lane, odd ones in the high lane.
#[inline]
#[target_feature(enable = "avx2")]
fn row_sums<const P: usize>(t: Avx2, row: &[u16], weights: &[__m256i; 4]) -> Option<__m256i> {
    if P > 1 {
        let row = row.first_chunk::<16>()?;
        let (low, one, high) = (load8(t, row, 0)?, load8(t, row, 1)?, load8(t, row, 8)?);
        return Some(sums_of::<P>((low, one, high), weights));
    }
    let low = load2x8(t, row.first_chunk::<9>()?, 0, 1)?;
    Some(madds::<P>(weights, |_| low))
}

/// [`row_sums`] of the samples of a row from columns 0, 1 and 8.
#[inline]
#[target_feature(enable = "avx2")]
fn sums_of<const P: usize>(
    (at0, at1, at8): (__m128i, __m128i, __m128i),
    weights: &[__m256i; 4],
) -> __m256i {
    let low = _mm256_set_m128i(at1, at0);
    let high = _mm256_set_m128i(_mm_srli_si128::<2>(at8), at8);
    madds::<P>(weights, |p| match p {
        0 => low,
        1 => _mm256_alignr_epi8::<4>(high, low),
        2 => _mm256_alignr_epi8::<8>(high, low),
        _ => _mm256_alignr_epi8::<12>(high, low),
    })
}

/// `pshufb` patterns of 4-column rows: pair `p` of output `j` reads words
/// `j + 2p` and `j + 2p + 1` of its lane.
const WORD_PAIRS: [[u8; 32]; 2] = {
    let mut out = [[0; 32]; 2];
    let mut pair = 0;
    while pair < 2 {
        let mut byte = 0;
        while byte < 32 {
            let lane = byte % 16;
            let word = lane / 4 + 2 * pair + (lane / 2) % 2;
            out[pair][byte] = (2 * word + lane % 2) as u8;
            byte += 1;
        }
        pair += 1;
    }
    out
};

/// The horizontal sums of `P` tap pairs of four outputs of rows `a` (low
/// lane) and `b` (high lane).
#[inline]
#[target_feature(enable = "avx2")]
fn pair_sums<const P: usize>(
    t: Avx2,
    (a, b): (&[u16], &[u16]),
    weights: &[__m256i; 4],
) -> Option<__m256i> {
    let low = _mm256_set_m128i(load8(t, b, 0)?, load8(t, a, 0)?);
    let high = if P > 2 {
        _mm256_set_m128i(load8(t, b, 4)?, load8(t, a, 4)?)
    } else {
        low
    };
    Some(pairs_of::<P>(t, (low, high), weights))
}

/// [`pair_sums`] of the samples of both rows from columns 0 (`low`) and 4.
#[inline]
#[target_feature(enable = "avx2")]
fn pairs_of<const P: usize>(
    t: Avx2,
    (low, high): (__m256i, __m256i),
    weights: &[__m256i; 4],
) -> __m256i {
    let pattern = |p: usize| -> __m256i { u8x32::load_array_ref(t, &WORD_PAIRS[p % 2]).into() };
    madds::<P>(weights, |p| {
        _mm256_shuffle_epi8(if p < 2 { low } else { high }, pattern(p))
    })
}

/// The eight samples of `row` in `i32` lanes.
#[inline]
#[target_feature(enable = "avx2")]
fn widen8(t: Avx2, row: &[u16]) -> Option<__m256i> {
    Some(_mm256_cvtepu16_epi32(load8(t, row, 0)?))
}

/// Four samples of rows `a` (low lane) and `b` (high lane) in `i32` lanes.
#[inline]
#[target_feature(enable = "avx2")]
fn widen4(t: Avx2, a: &[u16], b: &[u16]) -> Option<__m256i> {
    let rows = _mm_unpacklo_epi64(load8(t, a, 0)?, load8(t, b, 0)?);
    Some(_mm256_cvtepu16_epi32(rows))
}

/// Each `i32` lane of `previous` (an `i16` value) paired with the same lane
/// of `next`, as the words of one `pmaddwd` tap pair.
#[inline]
#[target_feature(enable = "avx2")]
fn pair(previous: __m256i, next: __m256i) -> __m256i {
    _mm256_blend_epi16::<0b1010_1010>(previous, _mm256_slli_epi32::<16>(next))
}

/// The `line` of a vertical pass over 8-column strips: each row of `row`
/// paired with the row before it, which `last` keeps.
#[inline]
#[target_feature(enable = "avx2")]
fn strip_line(last: &Cell<__m256i>, row: Option<__m256i>) -> Option<__m256i> {
    let row = row?;
    let paired = pair(last.get(), row);
    last.set(row);
    Some(paired)
}

/// The `line` of a vertical pass over 4-column row pairs: rows `2m - 2`
/// and `2m - 1` (kept in `last`) paired with the rows after them, the
/// second with row `2m` of the new `rows`.
#[inline]
#[target_feature(enable = "avx2")]
fn pair_line(last: &Cell<__m256i>, rows: Option<__m256i>) -> Option<__m256i> {
    let rows = rows?;
    let previous = last.get();
    last.set(rows);
    Some(pair(
        previous,
        _mm256_permute2x128_si256::<0x21>(previous, rows),
    ))
}

/// Tap pair `p` of a strip's vertical pass of `V` pairs: window rows
/// `2p` and `2p + 1` from the register `ring` at `phase` and the `next` row.
#[inline]
#[target_feature(enable = "avx2")]
fn ring_pair<const V: usize>(
    phase: usize,
    ring: &[__m256i; 8],
    next: __m256i,
    p: usize,
) -> __m256i {
    if p + 1 < V {
        ring[(phase + 2 * p + 1) % (2 * V)]
    } else {
        next
    }
}

/// Tap pair `p` of a 4-column vertical pass of `V` pairs over the `ring` of
/// paired row pairs and the `next` one.
#[inline]
#[target_feature(enable = "avx2")]
fn pairs_pair<const V: usize>(ring: &[__m256i; 4], next: __m256i, p: usize) -> __m256i {
    if p + 1 < V { ring[p + 1] } else { next }
}

/// A strip's outputs with even ones in the low lane and odd ones in the
/// high lane, in output order.
#[inline]
#[target_feature(enable = "avx2")]
fn interleave(x: __m256i) -> __m256i {
    _mm256_permutevar8x32_epi32(x, _mm256_setr_epi32(0, 4, 1, 5, 2, 6, 3, 7))
}

/// The full-sample `block` of a compound cell: the samples, by 16 for a
/// compound predictor.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn copy<O: WordOut>(t: Avx2, block: &Block<'_, u16>, out: &mut O, max: u16) {
    let clip = clip(out, max);
    let zero = _mm256_setzero_si256();
    if block.w == 4 {
        let emit = |a: O::Row<'_>, b: O::Row<'_>, x: __m256i| {
            store::<O>(t, clip, (a, Some(b)), x, _mm256_slli_epi32::<4>(x));
        };
        let line = |a: &[u16], b: &[u16]| widen4(t, a, b);
        rows::pairs::<u16, _, _, O, 0>(block, out, 8, zero, line, |_, next| next, emit);
        return;
    }
    let emit = |r: O::Row<'_>, x: __m256i| {
        store::<O>(t, clip, (r, None), x, _mm256_slli_epi32::<4>(x));
    };
    let line = |row: &[u16]| widen8(t, row);
    rows::strips::<u16, _, _, O, 0>(block, out, (8, 8), zero, line, |_, _, next| next, emit);
}

/// The horizontal pass of `block`, `P` tap pairs, in 8-column strips or
/// 4-column row pairs: the compound predictor `Round2(sum, InterRound0)`,
/// a put `Round2` of that by 4, as one `Round2(sum + 4, 7)`.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn horizontal<const W: usize, const P: usize, O: WordOut>(
    t: Avx2,
    block: &Block<'_, u16>,
    taps: &Taps,
    out: &mut O,
    max: u16,
) {
    let (weights, clip) = (weights(taps), clip(out, max));
    let zero = _mm256_setzero_si256();
    let put = |x: __m256i| _mm256_srai_epi32::<7>(_mm256_add_epi32(x, _mm256_set1_epi32(68)));
    if W == 4 {
        let line = |a: &[u16], b: &[u16]| pair_sums::<P>(t, (a, b), &weights);
        let emit = |a: O::Row<'_>, b: O::Row<'_>, x: __m256i| {
            store::<O>(t, clip, (a, Some(b)), put(x), round::<3>(x));
        };
        let reach = if P > 2 { 12 } else { 8 };
        rows::pairs::<u16, _, _, O, 0>(block, out, reach, zero, line, |_, next| next, emit);
        return;
    }
    let line = |row: &[u16]| row_sums::<P>(t, row, &weights);
    let emit = |r: O::Row<'_>, x: __m256i| {
        let x = interleave(x);
        store::<O>(t, clip, (r, None), put(x), round::<3>(x));
    };
    let reach = if P > 1 { 16 } else { 9 };
    rows::strips::<u16, _, _, O, 0>(block, out, (8, reach), zero, line, |_, _, next| next, emit);
}

/// The vertical pass of `block`, `P` tap pairs, in 8-column strips or
/// 4-column row pairs, the paired window rows in a register ring: the
/// compound predictor `Round2(sum, 3)`, a put `Round2(sum, 7)`.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn vertical<const W: usize, const P: usize, O: WordOut>(
    t: Avx2,
    block: &Block<'_, u16>,
    taps: &Taps,
    out: &mut O,
    max: u16,
) {
    let (weights, clip) = (weights(taps), clip(out, max));
    let zero = _mm256_setzero_si256();
    let last = Cell::new(zero);
    if W == 4 {
        let line = |a: &[u16], b: &[u16]| pair_line(&last, widen4(t, a, b));
        let vertical =
            |ring: &[__m256i; 4], next| madds::<P>(&weights, |p| pairs_pair::<P>(ring, next, p));
        let emit = |a: O::Row<'_>, b: O::Row<'_>, y: __m256i| {
            store::<O>(t, clip, (a, Some(b)), round::<7>(y), round::<3>(y));
        };
        rows::pairs::<u16, _, _, O, P>(block, out, 8, zero, line, vertical, emit);
        return;
    }
    let line = |row: &[u16]| strip_line(&last, widen8(t, row));
    let vertical = |phase, ring: &[__m256i; 8], next| {
        madds::<P>(&weights, |p| ring_pair::<P>(phase, ring, next, p))
    };
    let emit = |r: O::Row<'_>, y: __m256i| {
        store::<O>(t, clip, (r, None), round::<7>(y), round::<3>(y));
    };
    rows::strips::<u16, _, _, O, P>(block, out, (8, 8), zero, line, vertical, emit);
}

/// Both passes of `block`, `H` and `V` tap pairs, in 8-column strips or
/// 4-column row pairs: the `Round2(sum, InterRound0)` horizontal rows,
/// paired, in a register ring, the vertical sum `Round2`d by 7 for a
/// compound predictor and by 11 for a put.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn two_pass<const W: usize, const H: usize, const V: usize, O: WordOut>(
    t: Avx2,
    block: &Block<'_, u16>,
    (h_taps, v_taps): (&Taps, &Taps),
    out: &mut O,
    max: u16,
) {
    let (h_weights, v_weights, clip) = (weights(h_taps), weights(v_taps), clip(out, max));
    let zero = _mm256_setzero_si256();
    let last = Cell::new(zero);
    if W == 4 {
        let line = |a: &[u16], b: &[u16]| {
            let sums = pair_sums::<H>(t, (a, b), &h_weights).map(|x| round::<3>(x));
            pair_line(&last, sums)
        };
        let vertical =
            |ring: &[__m256i; 4], next| madds::<V>(&v_weights, |p| pairs_pair::<V>(ring, next, p));
        let emit = |a: O::Row<'_>, b: O::Row<'_>, y: __m256i| {
            store::<O>(t, clip, (a, Some(b)), round::<11>(y), round::<7>(y));
        };
        let reach = if H > 2 { 12 } else { 8 };
        rows::pairs::<u16, _, _, O, V>(block, out, reach, zero, line, vertical, emit);
        return;
    }
    let line = |row: &[u16]| {
        strip_line(
            &last,
            row_sums::<H>(t, row, &h_weights).map(|x| round::<3>(x)),
        )
    };
    let vertical = |phase, ring: &[__m256i; 8], next| {
        madds::<V>(&v_weights, |p| ring_pair::<V>(phase, ring, next, p))
    };
    let emit = |r: O::Row<'_>, y: __m256i| {
        let y = interleave(y);
        store::<O>(t, clip, (r, None), round::<11>(y), round::<7>(y));
    };
    let reach = if H > 1 { 16 } else { 9 };
    rows::strips::<u16, _, _, O, V>(block, out, (8, reach), zero, line, vertical, emit);
}

/// Where a cell reads window row `r`: `base + r * stride`, or when clamped
/// `base + clamp(top + r, first, last) * stride`, window column `c` from
/// sample `clamp(start + c, 0, high)` of the row.
#[derive(Clone, Copy)]
struct Reads {
    base: usize,
    stride: usize,
    top: i32,
    first: i32,
    last: i32,
    start: i16,
    high: i16,
}

impl Reads {
    const fn in_place(base: usize, stride: usize) -> Self {
        Self {
            base,
            stride,
            top: 0,
            first: 0,
            last: 0,
            start: 0,
            high: 15,
        }
    }
}

/// A reference of a register cell: its [`Reads`], whether they are
/// clamped, and the taps of both passes.
type Placed = (Reads, bool, Option<&'static Taps>, Option<&'static Taps>);

/// The [`Placed`] reads of the `W`-square [`super::super::block::is_cell`]
/// `window` in a plane of `geometry` within `bounds`: in place when every
/// read is inside the clip bounds, else the § 7.13.3.18 `Clip3` reads;
/// `None` for an empty clip range or a clamped row that cannot load 16
/// samples.
#[inline]
fn place<const W: usize>(window: &Window, geometry: Geometry, bounds: [i32; 4]) -> Option<Placed> {
    let small = |taps: Option<&Taps>| W == 8 || taps.is_none_or(|taps| taps.count <= 2);
    if window.w != W || !small(window.horizontal) || !small(window.vertical) {
        return None;
    }
    let taps = (window.horizontal, window.vertical);
    let [stride, width, rows, len] = geometry;
    if let Some(base) = window.origin(geometry, bounds) {
        return Some((Reads::in_place(base, stride), false, taps.0, taps.1));
    }
    let bound = |value: i32, size: usize| value.max(0).min(size as i32 - 1);
    let (first_x, last_x) = (bound(bounds[0], width), bound(bounds[2], width));
    let (first, last) = (bound(bounds[1], rows), bound(bounds[3], rows));
    if first_x > last_x || first > last {
        return None;
    }
    let column = window.column.max(first_x).min(last_x);
    let end = (last as usize)
        .checked_mul(stride)?
        .checked_add(column as usize + 16)?;
    if end > len {
        return None;
    }
    let reads = Reads {
        base: column as usize,
        stride,
        top: window.row,
        first,
        last,
        start: (window.column.max(first_x - 16).min(last_x + 16) - column) as i16,
        high: (last_x - column).min(15) as i16,
    };
    Some((reads, true, taps.0, taps.1))
}

fearless_simd::kernel!(
    #[inline]
    fn cell_pair(
        t: Avx2,
        blocks: [&Block<'_, u16>; 2],
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) {
        let [a, b] = blocks.map(|block| {
            let reads = Reads::in_place(block.origin, block.stride);
            (reads, false, block.horizontal, block.vertical)
        });
        let sources = [blocks[0].source, blocks[1].source];
        if blocks[0].w == 4 {
            store_cell::<4>(t, sources, [&a, &b], clip, output, stride);
        } else {
            store_cell::<8>(t, sources, [&a, &b], clip, output, stride);
        }
    }
);

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of the 4x4 or 8x8
/// cell of `windows` within `bounds`, each read in place or by clamped rows
/// and columns of `views`; `false` without writing for a cell no kernel
/// takes.
pub(super) fn compound_windows(
    views: [&ReferencePlaneView<'_, u16>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    match token() {
        Some(t) => windows_cell(t, views, windows, bounds, clip, output, stride),
        None => generic::compound_windows(views, windows, bounds, clip, output, stride),
    }
}

fearless_simd::kernel!(
    #[inline]
    fn windows_cell(
        t: Avx2,
        views: [&ReferencePlaneView<'_, u16>; 2],
        windows: &[Window; 2],
        bounds: [[i32; 4]; 2],
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) -> bool {
        let sources = [views[0].samples, views[1].samples];
        macro_rules! cell {
            ($w:literal) => {{
                let (Some(a), Some(b)) = (
                    place::<$w>(&windows[0], views[0].geometry(), bounds[0]),
                    place::<$w>(&windows[1], views[1].geometry(), bounds[1]),
                ) else {
                    return false;
                };
                store_cell::<$w>(t, sources, [&a, &b], clip, output, stride)
            }};
        }
        if windows[0].w == 4 {
            cell!(4)
        } else {
            cell!(8)
        }
    }
);

/// Runs the cells of a program of `size`-square cells whose windows are
/// read in place or by clamped rows and columns into every plane, and
/// returns their bits.
pub(super) fn program(
    planes: &mut [Option<Plane<'_, '_, u16>>; 2],
    shape: (usize, usize),
    cells: &[ProgramCell],
    clip: (i32, u16),
) -> Written {
    match token() {
        Some(t) => run_cells(t, planes, shape, cells, clip),
        None => generic::program(planes, shape, cells, clip),
    }
}

fearless_simd::kernel!(
    #[inline]
    fn run_cells(
        t: Avx2,
        planes: &mut [Option<Plane<'_, '_, u16>>; 2],
        shape: (usize, usize),
        cells: &[ProgramCell],
        clip: (i32, u16),
    ) -> Written {
        let (filter, size) = shape;
        match size {
            4 => run_program::<4>(t, planes, filter, cells, clip),
            8 => run_program::<8>(t, planes, filter, cells, clip),
            _ => [[0; 4]; 2],
        }
    }
);

/// [`program`] of `W`-square cells of `filter`: each reference's reads
/// derived once per cell, every plane stored from them.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn run_program<const W: usize>(
    t: Avx2,
    planes: &mut [Option<Plane<'_, '_, u16>>; 2],
    filter: usize,
    cells: &[ProgramCell],
    clip: (i32, u16),
) -> Written {
    let mut written = [[0u64; 4]; 2];
    let Some((views, ..)) = &planes[0] else {
        return written;
    };
    let geometry = [views[0].geometry(), views[1].geometry()];
    for (index, &([row, col], placements)) in cells.iter().enumerate() {
        let window = |(start, bounds): CellPlacement, geometry: Geometry| {
            place::<W>(
                &Window::of_filters([filter; 2], W, W, start),
                geometry,
                bounds,
            )
        };
        let (Some(a), Some(b)) = (
            window(placements[0], geometry[0]),
            window(placements[1], geometry[1]),
        ) else {
            continue;
        };
        for (plane, written) in planes.iter_mut().zip(&mut written) {
            let Some((views, output, stride)) = plane else {
                continue;
            };
            let at = row as usize * *stride + col as usize;
            let Some(output) = output.get_mut(at..) else {
                continue;
            };
            let sources = [views[0].samples, views[1].samples];
            if store_cell::<W>(t, sources, [&a, &b], clip, output, *stride) {
                written[index / 64 % 4] |= 1 << (index % 64);
            }
        }
    }
    written
}

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of a `W`-square cell
/// of both references' [`Placed`] reads of `sources`; `false` without
/// writing when a read leaves its source or the output does not hold the
/// cell.
#[inline]
#[target_feature(enable = "avx2")]
fn store_cell<const W: usize>(
    t: Avx2,
    sources: [&[u16]; 2],
    [a, b]: [&Placed; 2],
    (forward, max): (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    let span = (W - 1).checked_mul(stride);
    if stride < W
        || span.is_none_or(|span| output.len().checked_sub(W).is_none_or(|room| span > room))
    {
        return false;
    }
    let still = |placed: &Placed| !placed.1 && placed.2.is_none() && placed.3.is_none();
    if still(a) && still(b) && (0..=16).contains(&forward) {
        let sources = [(sources[0], a.0.stride), (sources[1], b.0.stride)];
        let origins = [a.0.base, b.0.base];
        super::super::cell::fullpel_u16(sources, origins, W, (forward, max), output, stride);
        return true;
    }
    let (Some(first), Some(second)) = (
        cell_of::<W>(t, sources[0], a),
        cell_of::<W>(t, sources[1], b),
    ) else {
        return false;
    };
    let Some((mut rows, last)) = rows::rows_mut(output, 0, stride, W, W) else {
        return false;
    };
    let mut last = Some(last);
    let max = _mm_set1_epi16(max as i16);
    let average = |a, b| {
        let x = average(a, b, forward);
        _mm_min_epu16(
            _mm_packus_epi32(_mm256_castsi256_si128(x), _mm256_extracti128_si256::<1>(x)),
            max,
        )
    };
    for (&a, &b) in first.iter().zip(&second).take(if W == 4 { 2 } else { 8 }) {
        let values = average(a, b);
        let Some(row) = rows.next().or_else(|| last.take()) else {
            break;
        };
        let next = if W == 4 {
            let Some(next) = rows.next().or_else(|| last.take()) else {
                break;
            };
            Some(next)
        } else {
            None
        };
        u16::store(t, row, next, values);
    }
    true
}

/// Calls `cell::<$w, H, V, C>` for the listed tap-pair counts of both
/// passes and both read forms.
macro_rules! by_pairs {
    ($h:expr, $v:expr, $c:expr, $w:literal, $args:tt, [$($hh:literal $vv:literal),*]) => {
        match ($h, $v, $c) {
            $(($hh, $vv, false) => cell::<$w, $hh, $vv, false> $args,)*
            $(($hh, $vv, true) => cell::<$w, $hh, $vv, true> $args,)*
            _ => None,
        }
    };
}

/// The compound predictors of a `W`-square register cell of `source` read
/// by `placed`: one group per row of an 8x8 cell, per row pair of a 4x4 one
/// (the first two).
#[inline]
#[target_feature(enable = "avx2")]
fn cell_of<const W: usize>(t: Avx2, source: &[u16], placed: &Placed) -> Option<[__m256i; 8]> {
    let &(ref reads, clamped, horizontal, vertical) = placed;
    let pairs = |taps: Option<&Taps>| taps.map_or(0, |taps| taps.count);
    let (h, v) = (pairs(horizontal), pairs(vertical));
    if W == 4 {
        by_pairs!(h, v, clamped, 4, (t, source, reads, (horizontal, vertical)), [0 0, 0 1, 0 2, 1 0, 1 1, 1 2, 2 0, 2 1, 2 2])
    } else {
        by_pairs!(h, v, clamped, 8, (t, source, reads, (horizontal, vertical)),
            [0 0, 0 1, 0 2, 0 3, 0 4, 1 0, 1 1, 1 2, 1 3, 1 4, 2 0, 2 1, 2 2, 2 3, 2 4,
             3 0, 3 1, 3 2, 3 3, 3 4, 4 0, 4 1, 4 2, 4 3, 4 4])
    }
}

/// The `pshufb` pattern of a clamped row load and the lanes it takes from
/// the second eight samples: word `j` from sample `clamp(start + j, 0,
/// high)`.
#[inline]
#[target_feature(enable = "avx2")]
fn gather(reads: &Reads) -> (__m256i, __m256i) {
    let lanes = _mm256_setr_epi16(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
    let index = _mm256_add_epi16(lanes, _mm256_set1_epi16(reads.start));
    let index = _mm256_min_epi16(
        _mm256_max_epi16(index, _mm256_setzero_si256()),
        _mm256_set1_epi16(reads.high),
    );
    let low = _mm256_and_si256(index, _mm256_set1_epi16(7));
    let pattern = _mm256_add_epi16(
        _mm256_mullo_epi16(low, _mm256_set1_epi16(0x202)),
        _mm256_set1_epi16(0x100),
    );
    (pattern, _mm256_cmpgt_epi16(index, _mm256_set1_epi16(7)))
}

/// The predictors of [`cell_of`] of a cell whose passes have `H` and `V`
/// tap pairs, none for a skipped pass, its rows and columns clamped with
/// `C`: the window rows read once, each vertical tap pair one blend of two
/// of them.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn cell<const W: usize, const H: usize, const V: usize, const C: bool>(
    t: Avx2,
    source: &[u16],
    reads: &Reads,
    (horizontal, vertical): (Option<&Taps>, Option<&Taps>),
) -> Option<[__m256i; 8]> {
    let zero = _mm256_setzero_si256();
    let h_weights = horizontal.map_or([zero; 4], |taps| weights(taps));
    let v_weights = vertical.map_or([zero; 4], |taps| weights(taps));
    let rows = if V > 0 { W + 2 * V - 1 } else { W };
    let stride = reads.stride;
    let window = if C {
        source
    } else {
        source.get(reads.base..)?.get(..(rows - 1) * stride + 16)?
    };
    let at = |r: usize| {
        if C {
            let row = reads
                .top
                .saturating_add(r as i32)
                .max(reads.first)
                .min(reads.last);
            reads.base + row as usize * stride
        } else {
            r.min(rows - 1) * stride
        }
    };
    let (pattern, upper) = if C { gather(reads) } else { (zero, zero) };
    let narrow = |x: __m128i| {
        if C {
            _mm_shuffle_epi8(x, _mm256_castsi256_si128(pattern))
        } else {
            x
        }
    };
    let words8 = |r: usize| Some(narrow(load8(t, window.get(at(r)..)?, 0)?));
    let words16 = |r: usize| {
        let row = window.get(at(r)..)?.first_chunk::<16>()?;
        if !C {
            return Some((load8(t, row, 0)?, load8(t, row, 1)?, load8(t, row, 8)?));
        }
        let row = _mm256_set_m128i(load8(t, row, 8)?, load8(t, row, 0)?);
        let from = |lane| _mm256_shuffle_epi8(lane, pattern);
        let row = _mm256_blendv_epi8(
            from(_mm256_permute2x128_si256::<0x00>(row, row)),
            from(_mm256_permute2x128_si256::<0x11>(row, row)),
            upper,
        );
        let (low, high) = (
            _mm256_castsi256_si128(row),
            _mm256_extracti128_si256::<1>(row),
        );
        Some((low, _mm_alignr_epi8::<2>(high, low), high))
    };
    let finish = |x: __m256i| match (H, V) {
        (0, 0) => _mm256_slli_epi32::<4>(x),
        (_, 0) | (0, _) => round::<3>(x),
        _ => round::<7>(x),
    };
    let mut groups = [zero; 8];
    let mut lines = [zero; 16];
    if W == 8 {
        unroll!(r < rows, [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14] => {
            let line = if H > 0 {
                sums_of::<H>(words16(r)?, &h_weights)
            } else {
                _mm256_cvtepu16_epi32(words8(r)?)
            };
            if V == 0 {
                let line = if H > 0 { interleave(line) } else { line };
                groups[r] = finish(line);
            } else {
                let line = if H > 0 { round::<3>(line) } else { line };
                lines[r + 1] = line;
                lines[r] = pair(lines[r], line);
                if r + 1 >= 2 * V {
                    let y = r + 1 - 2 * V;
                    let sum = madds::<V>(&v_weights, |p| lines[y + 2 * p + 1]);
                    groups[y] = finish(if H > 0 { interleave(sum) } else { sum });
                }
            }
        });
        return Some(groups);
    }
    unroll!(m < if V == 0 { 2 } else { V + 2 }, [0 1 2 3] => {
        let rows = _mm256_set_m128i(words8(2 * m + 1)?, words8(2 * m)?);
        let line = if H > 0 {
            pairs_of::<H>(t, (rows, rows), &h_weights)
        } else {
            _mm256_cvtepu16_epi32(_mm_unpacklo_epi64(
                _mm256_castsi256_si128(rows),
                _mm256_extracti128_si256::<1>(rows),
            ))
        };
        if V == 0 {
            if m < 2 {
                groups[m] = finish(line);
            }
        } else {
            let line = if H > 0 { round::<3>(line) } else { line };
            if m > 0 {
                lines[m - 1] = pair(lines[m - 1], _mm256_permute2x128_si256::<0x21>(lines[m - 1], line));
            }
            lines[m] = line;
            if m >= V && m - V < 2 {
                let g = m - V;
                groups[g] = finish(madds::<V>(&v_weights, |p| lines[g + p]));
            }
        }
    });
    Some(groups)
}
