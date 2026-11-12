// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit unscaled § 7.13.3.18 interpolation of [`super`] in AVX2
//! lanes.
//!
//! One `pmaddubsw` weighs a pair of source bytes by a pair of halved taps;
//! each byte pair stays below its saturation bound. A vector covers 16
//! outputs: one row of a 16-column strip, or one row of each of two rows of
//! an 8- or 4-column block. The rows of a vertical pass stay in a register
//! ring, so no pass stages through memory. The [`cell`] kernels keep a 4x4
//! or 8x8 cell in registers. Each entry takes the blocks [`super`] sends
//! it; a shape these kernels do not cover, or a CPU without AVX2, declines
//! to the portable kernels of [`super::generic`].

use super::super::ReferencePlaneView;
use super::super::block::{Block, Sink, Taps, Window};
use super::super::cell::{Cell, Plane, Written};
use super::super::rows::{self, Average, Out, Prep, Put};
use super::{generic, pairs};
use core::arch::x86_64::*;
use fearless_simd::{Avx2, SimdBase, SimdFrom, i16x8, i16x16, i32x4, i32x8, u8x16, u8x32, u16x16};

mod cell;

/// The AVX2 token, when the compiled baseline has AVX2.
#[allow(clippy::inline_always, reason = "one check per kernel call")]
#[inline(always)]
fn token() -> Option<Avx2> {
    fearless_simd::Level::baseline().as_avx2()
}

/// Whether the byte-lane kernels cover the shape of `block`, which
/// [`super::fits`]: strips of 16 columns, or 8 or 4 columns in pairs of
/// rows.
fn fits<T>(block: &Block<'_, T>) -> bool {
    block.w >= 16 || (block.h.is_multiple_of(2) && (block.w == 4 || block.w >= 8))
}

/// The stage the 16-bit strip kernels fill for `block`: the group rows of
/// its horizontal pass or the packed rows of its vertical pass, none
/// without a vertical pass.
pub(in crate::subpel_mc) fn stage_len<T>(block: &Block<'_, T>) -> usize {
    let Some(taps) = block.vertical else {
        return 0;
    };
    let rows = block.h + taps.span() - 1;
    let groups = rows.next_multiple_of(4) * 16;
    let strips = rows * block.w.div_ceil(8) * 8 + 64;
    groups.max(strips)
}

/// Predicts `block` into an eight-bit `sink`, as [`super::predict`] admits.
pub(super) fn predict(block: &Block<'_, u8>, sink: &mut Sink<'_>) {
    match token() {
        Some(t) if fits(block) => predict_on(t, block, sink),
        _ => generic::predict(block, sink),
    }
}

/// Stores the `Clip1` single-reference prediction of `block`: a filtered
/// 4x4 or 8x8 block in the cell kernels, else in the strip kernels.
pub(super) fn put(block: &Block<'_, u8>, output: &mut [u8], stride: usize) {
    let size = block.w;
    let filtered = block.horizontal.is_some() || block.vertical.is_some();
    let narrow = size == 4 && block.horizontal.is_some_and(|taps| taps.count > 2);
    let cell = block.h == size && (size == 4 || size == 8) && filtered && !narrow;
    if cell
        && let Some(t) = token()
        && cell::put(t, block, size, output, stride)
    {
        return;
    }
    match token() {
        Some(t) if fits(block) => put_on(t, block, output, stride),
        _ => generic::put(block, output, stride),
    }
}

/// [`put`] of two blocks of one window into `outputs`.
pub(super) fn put_pair(blocks: [&Block<'_, u8>; 2], outputs: [(&mut [u8], usize); 2]) {
    for (block, (output, stride)) in blocks.into_iter().zip(outputs) {
        put(block, output, stride);
    }
}

/// The COMPOUND_AVERAGE of each of `P` 4x4 or 8x8 cell pairs into its
/// output, in the cell kernels, else through [`compound_fullpel`].
pub(super) fn compound_cell<const P: usize>(
    pairs: [[&Block<'_, u8>; 2]; P],
    forward: i32,
    outputs: [(&mut [u8], usize); P],
) {
    for (blocks, (output, stride)) in pairs.into_iter().zip(outputs) {
        let size = blocks[0].w;
        let taken =
            token().is_some_and(|t| cell::compound(t, blocks, size, forward, &mut *output, stride));
        if !taken {
            compound_fullpel(blocks, forward, output, stride);
        }
    }
}

/// The COMPOUND_AVERAGE of a pair in one call of the strip kernels, the
/// `i16` first predictor of at most 64 samples on the stack, else in
/// retained storage.
pub(super) fn compound_fullpel(
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    let len = blocks[0].w * blocks[0].h;
    match token() {
        Some(t) if fits(blocks[0]) && fits(blocks[1]) && len <= 64 => {
            compound_on(t, blocks, forward, &mut [0; 64], output, stride);
        }
        Some(t) if fits(blocks[0]) && fits(blocks[1]) => {
            super::super::with_compound_pred0_narrow(len, |pred0| {
                compound_on(t, blocks, forward, pred0, output, stride);
            });
        }
        _ => super::staged(blocks, forward, output, stride),
    }
}

/// [`compound_fullpel`] of each of `P` compound pairs into its output.
pub(super) fn compound_strips<const P: usize>(
    pairs: [[&Block<'_, u8>; 2]; P],
    forward: i32,
    outputs: [(&mut [u8], usize); P],
) {
    for (blocks, (output, stride)) in pairs.into_iter().zip(outputs) {
        compound_fullpel(blocks, forward, output, stride);
    }
}

/// Stores the COMPOUND_AVERAGE of the 4x4 or 8x8 cell of `windows`
/// within `bounds`, read in place or by clamped rows and columns of
/// `views`, or returns `false` without writing.
pub(super) fn compound_windows(
    views: [&ReferencePlaneView<'_, u8>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    token()
        .is_some_and(|t| cell::compound_windows(t, views, windows, bounds, forward, output, stride))
}

/// Runs the cells of a program that the register cells cover into
/// `planes`, and returns their bits.
pub(super) fn program(
    planes: &mut [Option<Plane<'_, '_, u8>>; 2],
    shape: (usize, usize),
    cells: &[Cell],
    clip: (i32, u16),
) -> Written {
    token().map_or([[0; 4]; 2], |t| {
        cell::program(t, planes, shape, cells, clip)
    })
}

/// `pshufb` patterns gathering source byte pairs `(2j + k, 2j + k + 1)`
/// for output `k` of each lane, or of each 8-byte half for 4-wide rows.
const fn pair_shuffles(half_rows: bool) -> [[u8; 32]; 4] {
    let mut out = [[0; 32]; 4];
    let mut pair = 0;
    while pair < 4 {
        let mut lane = 0;
        while lane < 32 {
            let byte = lane % 16;
            let (base, output) = if half_rows {
                ((byte / 8) * 8, (byte % 8) / 2)
            } else {
                (0, byte / 2)
            };
            out[pair][lane] = (base + 2 * pair + output + byte % 2) as u8;
            lane += 1;
        }
        pair += 1;
    }
    out
}

const LANE_SHUFFLES: [[u8; 32]; 4] = pair_shuffles(false);
const HALF_SHUFFLES: [[u8; 32]; 4] = pair_shuffles(true);

/// An output row start of [`store_rows`].
pub(in crate::subpel_mc) trait RowStart:
    Copy + core::ops::Add<Output = Self> + core::ops::Mul<Output = Self> + From<u8>
{
    fn row_mut<const N: usize>(self, output: &mut [u8]) -> Option<&mut [u8; N]>;
}

impl RowStart for usize {
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn row_mut<const N: usize>(self, output: &mut [u8]) -> Option<&mut [u8; N]> {
        output.get_mut(self..)?.first_chunk_mut::<N>()
    }
}

impl RowStart for u32 {
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn row_mut<const N: usize>(self, output: &mut [u8]) -> Option<&mut [u8; N]> {
        let at = self as usize;
        if at + N > output.len() {
            return None;
        }
        output.get_mut(at..)?.first_chunk_mut::<N>()
    }
}

/// Calls `$call!(H, V)` for the shape key `5 * H + V` of a kernel whose
/// passes have `H` and `V` tap pairs; `$narrow` keys have `H` at most 2.
macro_rules! shapes {
    ($key:expr, $narrow:literal, $call:ident) => {
        if $narrow {
            shapes!(@ $key, $call, [
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
                10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4
            ])
        } else {
            shapes!(@ $key, $call, [
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
                10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4, 15 3 0, 16 3 1, 17 3 2, 18 3 3,
                19 3 4, 20 4 0, 21 4 1, 22 4 2, 23 4 3, 24 4 4
            ])
        }
    };
    (@ $key:expr, $call:ident, [$($k:literal $h:literal $v:literal),*]) => {
        match $key {
            $($k => $call!($h, $v),)*
            _ => {}
        }
    };
}

fearless_simd::kernel!(
    #[inline]
    fn predict_on(t: Avx2, block: &Block<'_, u8>, sink: &mut Sink<'_>) {
        let w = block.w;
        match sink {
            Sink::Put { output, stride } => {
                strips(
                    t,
                    block,
                    &mut Put {
                        output,
                        stride: *stride,
                    },
                );
            }
            Sink::PutWide { output, stride, .. } => {
                strips(
                    t,
                    block,
                    &mut Put {
                        output,
                        stride: *stride,
                    },
                );
            }
            Sink::Prep { output, stride } => {
                strips(
                    t,
                    block,
                    &mut Prep {
                        output,
                        stride: *stride,
                    },
                );
            }
            Sink::PrepNarrow { output, stride } => {
                strips(
                    t,
                    block,
                    &mut Prep {
                        output,
                        stride: *stride,
                    },
                );
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (forward, stride) = (*forward, *stride);
                strips(
                    t,
                    block,
                    &mut Average {
                        pred0,
                        w,
                        forward,
                        output,
                        stride,
                    },
                );
            }
            Sink::AverageNarrow {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (forward, stride) = (*forward, *stride);
                strips(
                    t,
                    block,
                    &mut Average {
                        pred0,
                        w,
                        forward,
                        output,
                        stride,
                    },
                );
            }
            Sink::AverageWide { .. } => {}
        }
    }
);

fearless_simd::kernel!(
    #[inline]
    fn put_on(t: Avx2, block: &Block<'_, u8>, output: &mut [u8], stride: usize) {
        strips(t, block, &mut Put { output, stride });
    }
);

fearless_simd::kernel!(
    /// The COMPOUND_AVERAGE of a pair through the `i16` first predictor
    /// `pred0` of at least `w * h` values, or one staged for the second.
    #[inline]
    fn compound_on(
        t: Avx2,
        blocks: [&Block<'_, u8>; 2],
        forward: i32,
        pred0: &mut [i16],
        output: &mut [u8],
        stride: usize,
    ) {
        let w = blocks[0].w;
        let Some(pred0) = pred0.get_mut(..w * blocks[0].h) else {
            return;
        };
        strips(
            t,
            blocks[0],
            &mut Prep {
                output: &mut *pred0,
                stride: w,
            },
        );
        strips(
            t,
            blocks[1],
            &mut Average {
                pred0,
                w,
                forward,
                output,
                stride,
            },
        );
    }
);

/// Predicts `block`, which [`fits`], into `out`: in strips of 16 columns,
/// else in pairs of rows of 4 columns or of 8 columns, the last 8 overlapping
/// the first.
#[inline]
#[target_feature(enable = "avx2")]
fn strips<O: LaneOut>(t: Avx2, block: &Block<'_, u8>, out: &mut O) {
    let key = 5 * pairs(block.horizontal) + pairs(block.vertical);
    macro_rules! narrow {
        ($h:literal, $v:literal) => {
            pair_block::<$h, $v, 4, O>(t, block, out)
        };
    }
    macro_rules! strip {
        ($h:literal, $v:literal) => {
            strip_block::<$h, $v, O>(t, block, out)
        };
    }
    match block.w {
        4 => shapes!(key, true, narrow),
        w if w < 16 => {
            let last = w - 8;
            for col in (0..=last).step_by(last.max(1)) {
                let part = Block {
                    origin: block.origin + col,
                    w: 8,
                    ..*block
                };
                macro_rules! eight {
                    ($h:literal, $v:literal) => {
                        pair_block::<$h, $v, 8, Cols<'_, O>>(t, &part, &mut Cols(out, col))
                    };
                }
                shapes!(key, false, eight);
            }
        }
        _ => shapes!(key, false, strip),
    }
}

/// An [`Out`] of 8 columns from column `.1` of the [`Out`] `.0`.
struct Cols<'o, O>(&'o mut O, usize);

impl<O: Out> Out for Cols<'_, O> {
    const PUT: bool = O::PUT;
    const AVERAGE: bool = O::AVERAGE;
    type Row<'r>
        = O::Row<'r>
    where
        Self: 'r;
    type Rows<'r>
        = O::Rows<'r>
    where
        Self: 'r;
    #[allow(clippy::inline_always, reason = "measured strip-kernel row helper")]
    #[inline(always)]
    fn forward(&self) -> i32 {
        self.0.forward()
    }
    #[allow(clippy::inline_always, reason = "measured strip-kernel row helper")]
    #[inline(always)]
    fn rows(&mut self, _: usize, _: usize, n: usize) -> Option<(Self::Rows<'_>, Self::Row<'_>)> {
        self.0.rows(self.1, 8, n)
    }
}

impl<O: LaneOut> LaneOut for Cols<'_, O> {
    #[allow(clippy::inline_always, reason = "measured strip-kernel row helper")]
    #[inline(always)]
    fn row<'r>(row: Self::Row<'r>) -> Row<'r>
    where
        Self: 'r,
    {
        O::row(row)
    }
}

/// The halved taps of a pass as `pmaddubsw` byte pairs and `pmaddwd` word
/// pairs, zero for a skipped pass.
#[inline]
#[target_feature(enable = "avx2")]
fn weights<const P: usize>(taps: Option<&Taps>) -> ([__m256i; 4], [__m256i; 4]) {
    let (dot, words) = taps.map_or((0, [0; 4]), |taps| (taps.dot, taps.words));
    let bytes = core::array::from_fn(|pair| _mm256_set1_epi16((dot >> (16 * pair)) as i16));
    let words = core::array::from_fn(|pair| _mm256_set1_epi32(words[pair.min(P.max(1) - 1)]));
    (bytes, words)
}

/// `Round2` of the `i32` vertical sums `low` and `high` by `S1 + 1`, as
/// `i16`: shifted by `S1`, packed, then halved with rounding.
#[inline]
#[target_feature(enable = "avx2")]
fn round_pack<const S1: i32>(low: __m256i, high: __m256i) -> __m256i {
    let packed = _mm256_packs_epi32(_mm256_srai_epi32::<S1>(low), _mm256_srai_epi32::<S1>(high));
    _mm256_mulhrs_epi16(packed, _mm256_set1_epi16(1 << 14))
}

/// The final value of a vertical pass of `V` tap pairs over the `i16`
/// rows `tap(k)`: horizontal intermediates with `H`, samples without.
#[inline]
#[target_feature(enable = "avx2")]
fn word_value<const H: usize, const V: usize>(
    put: bool,
    words: &[__m256i; 4],
    tap: impl Fn(usize) -> __m256i,
) -> __m256i {
    let (mut low, mut high) = (_mm256_setzero_si256(), _mm256_setzero_si256());
    for (pair, &weights) in words.iter().enumerate().take(V) {
        let (a, b) = (tap(2 * pair), tap(2 * pair + 1));
        let (l, h) = (
            _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights),
            _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights),
        );
        (low, high) = if pair == 0 {
            (l, h)
        } else {
            (_mm256_add_epi32(low, l), _mm256_add_epi32(high, h))
        };
    }
    match (H > 0, put) {
        (true, true) => round_pack::<9>(low, high),
        (true, false) | (false, true) => round_pack::<5>(low, high),
        (false, false) => round_pack::<1>(low, high),
    }
}

/// The final value of a vertical pass of `V` tap pairs over the byte rows
/// `tap(k)`.
#[inline]
#[target_feature(enable = "avx2")]
fn byte_value<const V: usize>(
    put: bool,
    bytes: &[__m256i; 4],
    tap: impl Fn(usize) -> __m256i,
) -> __m256i {
    let mut sums = _mm256_setzero_si256();
    for (pair, &weights) in bytes.iter().enumerate().take(V) {
        let pairs = _mm256_unpacklo_epi8(tap(2 * pair), tap(2 * pair + 1));
        let product = _mm256_maddubs_epi16(pairs, weights);
        sums = if pair == 0 {
            product
        } else {
            _mm256_add_epi16(sums, product)
        };
    }
    _mm256_mulhrs_epi16(sums, _mm256_set1_epi16(if put { 1 << 9 } else { 1 << 13 }))
}

/// The horizontal-pass value of the halved-tap `sums`: before `Clip1` for
/// a put without a vertical pass, else `Round2(sum, InterRound0)`.
#[inline]
#[target_feature(enable = "avx2")]
fn horizontal_value<const V: usize>(put: bool, sums: __m256i) -> __m256i {
    if V == 0 && put {
        _mm256_srai_epi16::<6>(_mm256_add_epi16(sums, _mm256_set1_epi16(34)))
    } else {
        _mm256_mulhrs_epi16(sums, _mm256_set1_epi16(1 << 13))
    }
}

/// The 16-column strips of `block` whose passes have `H` and `V` tap
/// pairs: each window row of a strip read once, its horizontal sums or
/// samples kept in a register ring for the vertical pass.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn strip_block<const H: usize, const V: usize, O: LaneOut>(
    t: Avx2,
    block: &Block<'_, u8>,
    out: &mut O,
) {
    let reach = if H > 0 { 24 } else { 16 };
    let filter = Horizontal::new::<16, H>(t, block.horizontal.unwrap_or(&SKIPPED));
    let (bytes, words) = weights::<V>(block.vertical);
    let (put, forward) = (O::PUT, out.forward());
    let line = |row: &[u8]| -> Option<__m256i> {
        let low = load16(t, row.first_chunk::<16>()?);
        if H == 0 {
            return Some(if V == 0 {
                _mm256_cvtepu8_epi16(low)
            } else {
                _mm256_permute4x64_epi64::<0b01_01_00_00>(_mm256_castsi128_si256(low))
            });
        }
        let high = load16(t, row.get(8..)?.first_chunk::<16>()?);
        let sums = filter.sums::<H>(_mm256_set_m128i(high, low));
        Some(horizontal_value::<V>(put, sums))
    };
    let vertical = |phase: usize, ring: &[__m256i; 8], next: __m256i| -> __m256i {
        let tap = |k: usize| {
            if k + 1 < 2 * V {
                ring[(phase + k) % (2 * V)]
            } else {
                next
            }
        };
        match (H, V) {
            (_, 0) if H > 0 || put => next,
            (0, 0) => _mm256_slli_epi16::<4>(next),
            (0, _) => byte_value::<V>(put, &bytes, tap),
            _ => word_value::<H, V>(put, &words, tap),
        }
    };
    let emit = |row: O::Row<'_>, values: __m256i| store16(t, O::row(row), values, forward);
    let zero = _mm256_setzero_si256();
    rows::strips::<u8, _, _, O, V>(block, out, (16, reach), zero, line, vertical, emit);
}

/// Stores the `$c` final-pass values of row `a` (lanes `0..$c`) and of row
/// `b` (lanes `8..16` for 8 columns, `4..8` for 4), inside each kernel so
/// that only the arm of its sink remains.
macro_rules! store_pair {
    ($t:expr, $c:ident, $a:expr, $b:expr, $values:expr, $forward:expr) => {{
        let (t, values) = ($t, $values);
        let low = _mm256_castsi256_si128(values);
        let high = _mm256_extracti128_si256::<1>(values);
        let halves = |a: &mut [i16], b: &mut [i16], (low, high): (__m128i, __m128i)| {
            if $c == 8 {
                for (row, lanes) in [(a, low), (b, high)] {
                    if let Some(slot) = row.first_chunk_mut::<8>() {
                        i16x8::<Avx2>::simd_from(t, lanes).store_array(slot);
                    }
                }
            } else {
                copy_pair::<4, _>(a, b, &i16x8::<Avx2>::simd_from(t, low).to_array());
            }
        };
        match ($a, $b) {
            (Row::Bytes(a), Row::Bytes(b)) => bytes_pair::<$c>(a, b, values),
            (Row::Words(a), Row::Words(b)) => {
                let lanes = u16x16::<Avx2>::simd_from(t, clip_words(values)).to_array();
                copy_pair::<$c, _>(a, b, &lanes);
            }
            (Row::Narrow(a), Row::Narrow(b)) => halves(a, b, (low, high)),
            (Row::Wide(a), Row::Wide(b)) => {
                let low = _mm256_cvtepi16_epi32(low);
                if $c == 8 {
                    let high = _mm256_cvtepi16_epi32(high);
                    for (row, lanes) in [(a, low), (b, high)] {
                        if let Some(slot) = row.first_chunk_mut::<8>() {
                            i32x8::<Avx2>::simd_from(t, lanes).store_array(slot);
                        }
                    }
                } else {
                    copy_pair::<4, _>(a, b, &i32x8::<Avx2>::simd_from(t, low).to_array());
                }
            }
            (Row::AverageNarrow(first_a, a), Row::AverageNarrow(first_b, b)) => {
                let first = if $c == 8 {
                    let lanes = |row: &[i16]| -> __m128i {
                        row.first_chunk::<8>().map_or(_mm_setzero_si128(), |lanes| {
                            i16x8::load_array_ref(t, lanes).into()
                        })
                    };
                    _mm256_set_m128i(lanes(first_b), lanes(first_a))
                } else {
                    let word = |row: &[i16]| {
                        row.first_chunk::<4>().map_or(0, |lanes| {
                            lanes
                                .iter()
                                .rev()
                                .fold(0i64, |word, &lane| word << 16 | i64::from(lane as u16))
                        })
                    };
                    _mm256_castsi128_si256(_mm_set_epi64x(word(first_b), word(first_a)))
                };
                bytes_pair::<$c>(a, b, average(first, values, $forward));
            }
            (Row::AverageWide(first_a, a), Row::AverageWide(first_b, b)) => {
                let first = if $c == 8 {
                    let packed = _mm256_packs_epi32(wide8(t, first_a), wide8(t, first_b));
                    _mm256_permute4x64_epi64::<0b11_01_10_00>(packed)
                } else {
                    let lanes = |row: &[i32]| -> __m128i {
                        row.first_chunk::<4>().map_or(_mm_setzero_si128(), |lanes| {
                            i32x4::load_array_ref(t, lanes).into()
                        })
                    };
                    _mm256_castsi128_si256(_mm_packs_epi32(lanes(first_a), lanes(first_b)))
                };
                bytes_pair::<$c>(a, b, average(first, values, $forward));
            }
            _ => {}
        }
    }};
}

/// A `C`-column `block`, 8 or 4, whose passes have `H` and `V` tap pairs,
/// two rows per vector (in the two lanes, or the two halves of the low
/// lane): the horizontal sums or samples of each window row pair in a
/// register ring, whose last pair repeats the last window row, which no
/// tap weighs.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn pair_block<const H: usize, const V: usize, const C: usize, O: LaneOut>(
    t: Avx2,
    block: &Block<'_, u8>,
    out: &mut O,
) {
    let filter = Horizontal::new::<C, H>(t, block.horizontal.unwrap_or(&SKIPPED));
    let (bytes, words) = weights::<V>(block.vertical);
    let forward = out.forward();
    let word = |row: &[u8]| Some(i64::from_le_bytes(*row.first_chunk::<8>()?));
    let line = |a: &[u8], b: &[u8]| -> Option<__m256i> {
        let rows = if C == 8 {
            if H == 0 && V == 0 {
                return Some(_mm256_cvtepu8_epi16(_mm_set_epi64x(word(b)?, word(a)?)));
            }
            _mm256_set_m128i(
                load16(t, b.first_chunk::<16>()?),
                load16(t, a.first_chunk::<16>()?),
            )
        } else {
            if H == 0 {
                let word = |row: &[u8]| Some(i32::from_le_bytes(*row.first_chunk::<4>()?));
                let bytes = _mm_set_epi32(0, 0, word(b)?, word(a)?);
                return Some(_mm256_castsi128_si256(_mm_cvtepu8_epi16(bytes)));
            }
            _mm256_castsi128_si256(_mm_set_epi64x(word(b)?, word(a)?))
        };
        if H == 0 {
            return Some(rows);
        }
        let sums = filter.sums::<H>(rows);
        Some(horizontal_value::<V>(O::PUT, sums))
    };
    let vertical = |ring: &[__m256i; 4], next: __m256i| -> __m256i {
        let pair = |k: usize| if k < V { ring[k] } else { next };
        let tap = |k: usize| {
            if k.is_multiple_of(2) {
                pair(k / 2)
            } else if C == 8 {
                _mm256_permute2x128_si256::<0x21>(pair(k / 2), pair(k / 2 + 1))
            } else {
                _mm256_alignr_epi8::<8>(pair(k / 2 + 1), pair(k / 2))
            }
        };
        match (H, V) {
            (_, 0) if H > 0 || O::PUT => next,
            (0, 0) => _mm256_slli_epi16::<4>(next),
            (0, _) if C == 8 => byte_value::<V>(O::PUT, &bytes, tap),
            _ => word_value::<H, V>(O::PUT, &words, tap),
        }
    };
    let emit = |a: O::Row<'_>, b: O::Row<'_>, values: __m256i| {
        store_pair!(t, C, O::row(a), O::row(b), values, forward);
    };
    let zero = _mm256_setzero_si256();
    rows::pairs::<u8, _, _, O, V>(block, out, 2 * C, zero, line, vertical, emit);
}

/// One output row of a kernel, by sink: `Clip1` samples, compound
/// predictors, or the first predictors and samples of an average.
enum Row<'r> {
    Bytes(&'r mut [u8]),
    Words(&'r mut [u16]),
    Narrow(&'r mut [i16]),
    Wide(&'r mut [i32]),
    AverageNarrow(&'r [i16], &'r mut [u8]),
    AverageWide(&'r [i32], &'r mut [u8]),
}

/// An [`Out`] whose rows are [`Row`]s.
trait LaneOut: Out {
    /// The [`Row`] of an output row.
    fn row<'r>(row: Self::Row<'r>) -> Row<'r>
    where
        Self: 'r;
}

macro_rules! lane_outs {
    ($($out:ty => |$row:pat_param| $value:expr;)*) => {
        $(
            impl LaneOut for $out {
                #[allow(clippy::inline_always, reason = "measured strip-kernel row helper")]
                #[inline(always)]
                fn row<'r>($row: Self::Row<'r>) -> Row<'r>
                where
                    Self: 'r,
                {
                    $value
                }
            }
        )*
    };
}

lane_outs! {
    Put<'_, u8> => |row| Row::Bytes(row);
    Put<'_, u16> => |row| Row::Words(row);
    Prep<'_, i16> => |row| Row::Narrow(row);
    Prep<'_, i32> => |row| Row::Wide(row);
    Average<'_, i16, u8> => |(first, row)| Row::AverageNarrow(first, row);
    Average<'_, i32, u8> => |(first, row)| Row::AverageWide(first, row);
}

/// Stores the 16 final-pass values of a strip row.
#[inline]
#[target_feature(enable = "avx2")]
fn store16(t: Avx2, row: Row<'_>, values: __m256i, forward: i32) {
    let halves = [
        _mm256_castsi256_si128(values),
        _mm256_extracti128_si256::<1>(values),
    ];
    match row {
        Row::Bytes(row) => bytes16(t, row, values),
        Row::Words(row) => {
            if let Some(slot) = row.first_chunk_mut::<16>() {
                u16x16::<Avx2>::simd_from(t, clip_words(values)).store_array(slot);
            }
        }
        Row::Narrow(row) => {
            if let Some(slot) = row.first_chunk_mut::<16>() {
                i16x16::<Avx2>::simd_from(t, values).store_array(slot);
            }
        }
        Row::Wide(row) => {
            for (slot, half) in row.as_chunks_mut::<8>().0.iter_mut().zip(halves) {
                i32x8::<Avx2>::simd_from(t, _mm256_cvtepi16_epi32(half)).store_array(slot);
            }
        }
        Row::AverageNarrow(first, row) => {
            let first = first
                .first_chunk::<16>()
                .map_or(_mm256_setzero_si256(), |lanes| {
                    i16x16::load_array_ref(t, lanes).into()
                });
            bytes16(t, row, average(first, values, forward));
        }
        Row::AverageWide(first, row) => {
            let packed =
                _mm256_packs_epi32(wide8(t, first), wide8(t, first.get(8..).unwrap_or(&[])));
            let first = _mm256_permute4x64_epi64::<0b11_01_10_00>(packed);
            bytes16(t, row, average(first, values, forward));
        }
    }
}

/// The first eight `i32` predictors of `row`, zero past it.
#[inline]
#[target_feature(enable = "avx2")]
fn wide8(t: Avx2, row: &[i32]) -> __m256i {
    row.first_chunk::<8>()
        .map_or(_mm256_setzero_si256(), |lanes| {
            i32x8::load_array_ref(t, lanes).into()
        })
}

/// Copies lanes `0..C` of `lanes` into row `a` and the `C` lanes of the
/// second row (from 8 for 8 columns, else from 4) into row `b`.
#[allow(clippy::inline_always, reason = "measured strip-kernel row helper")]
#[inline(always)]
fn copy_pair<const C: usize, T: Copy>(a: &mut [T], b: &mut [T], lanes: &[T]) {
    let offset = if C == 8 { 8 } else { 4 };
    for (row, at) in [(a, 0), (b, offset)] {
        let lanes = lanes.get(at..).and_then(<[T]>::first_chunk::<C>);
        if let (Some(slot), Some(lanes)) = (row.first_chunk_mut::<C>(), lanes) {
            *slot = *lanes;
        }
    }
}

/// `Clip1` of 16 values into a byte row.
#[inline]
#[target_feature(enable = "avx2")]
fn bytes16(t: Avx2, row: &mut [u8], values: __m256i) {
    let packed = _mm_packus_epi16(
        _mm256_castsi256_si128(values),
        _mm256_extracti128_si256::<1>(values),
    );
    if let Some(slot) = row.first_chunk_mut::<16>() {
        u8x16::<Avx2>::simd_from(t, packed).store_array(slot);
    }
}

/// `Clip1` of the `C` values of each of two rows into byte rows `a` and
/// `b`.
#[inline]
#[target_feature(enable = "avx2")]
fn bytes_pair<const C: usize>(a: &mut [u8], b: &mut [u8], values: __m256i) {
    let low = _mm256_castsi256_si128(values);
    if C == 8 {
        let packed = _mm_packus_epi16(low, _mm256_extracti128_si256::<1>(values));
        if let (Some(a), Some(b)) = (a.first_chunk_mut::<8>(), b.first_chunk_mut::<8>()) {
            *a = _mm_cvtsi128_si64(packed).to_le_bytes();
            *b = _mm_extract_epi64::<1>(packed).to_le_bytes();
        }
    } else {
        let packed = _mm_packus_epi16(low, low);
        if let (Some(a), Some(b)) = (a.first_chunk_mut::<4>(), b.first_chunk_mut::<4>()) {
            *a = _mm_cvtsi128_si32(packed).to_le_bytes();
            *b = _mm_extract_epi32::<1>(packed).to_le_bytes();
        }
    }
}

/// `Clip1` of 16 eight-bit values in their `u16` lanes.
#[inline]
#[target_feature(enable = "avx2")]
fn clip_words(values: __m256i) -> __m256i {
    _mm256_min_epi16(
        _mm256_max_epi16(values, _mm256_setzero_si256()),
        _mm256_set1_epi16(255),
    )
}

/// The COMPOUND_AVERAGE of two groups of compound predictors, before
/// `Clip1`.
#[inline]
#[target_feature(enable = "avx2")]
pub(in crate::subpel_mc) fn average(first: __m256i, second: __m256i, forward: i32) -> __m256i {
    if forward == 8 {
        return _mm256_mulhrs_epi16(_mm256_add_epi16(first, second), _mm256_set1_epi16(1 << 10));
    }
    let weights = _mm256_set1_epi32((forward & 0xffff) | ((16 - forward) << 16));
    let bias = _mm256_set1_epi32(1 << 7);
    let low = _mm256_madd_epi16(_mm256_unpacklo_epi16(first, second), weights);
    let high = _mm256_madd_epi16(_mm256_unpackhi_epi16(first, second), weights);
    _mm256_packs_epi32(
        _mm256_srai_epi32::<8>(_mm256_add_epi32(low, bias)),
        _mm256_srai_epi32::<8>(_mm256_add_epi32(high, bias)),
    )
}

#[inline]
#[target_feature(enable = "avx2")]
fn load16(t: Avx2, bytes: &[u8; 16]) -> __m128i {
    u8x16::load_array_ref(t, bytes).into()
}

/// The taps of a skipped pass, which no kernel reads.
static SKIPPED: Taps = Taps {
    start: 0,
    count: 0,
    dot: 0,
    words: [0; 4],
    full: [0; 4],
};

pub(in crate::subpel_mc) struct Horizontal {
    shuffles: [__m256i; 4],
    weights: [__m256i; 4],
}

impl Horizontal {
    #[inline]
    #[target_feature(enable = "avx2")]
    pub(in crate::subpel_mc) fn new<const W: usize, const P: usize>(t: Avx2, taps: &Taps) -> Self {
        let patterns = if W == 4 {
            &HALF_SHUFFLES
        } else {
            &LANE_SHUFFLES
        };
        Self {
            shuffles: core::array::from_fn(|pair| {
                u8x32::load_array_ref(t, &patterns[pair.min(P.max(1) - 1)]).into()
            }),
            weights: core::array::from_fn(|pair| {
                _mm256_set1_epi16((taps.dot >> (16 * pair.min(P.max(1) - 1))) as i16)
            }),
        }
    }

    /// The halved-tap sums of the 16 outputs of `bytes`.
    #[inline]
    #[target_feature(enable = "avx2")]
    pub(in crate::subpel_mc) fn sums<const P: usize>(&self, bytes: __m256i) -> __m256i {
        let mut sums = _mm256_maddubs_epi16(
            _mm256_shuffle_epi8(bytes, self.shuffles[0]),
            self.weights[0],
        );
        for pair in 1..P {
            sums = _mm256_add_epi16(
                sums,
                _mm256_maddubs_epi16(
                    _mm256_shuffle_epi8(bytes, self.shuffles[pair]),
                    self.weights[pair],
                ),
            );
        }
        sums
    }
}

/// [`store_bytes`] from rows `at + r * stride`.
#[inline]
#[target_feature(enable = "avx2")]
pub(in crate::subpel_mc) fn store_rows<const W: usize, S: RowStart>(
    t: Avx2,
    output: &mut [u8],
    at: S,
    stride: S,
    values: __m256i,
) {
    let row = |row: u8| at + S::from(row) * stride;
    let packed = _mm256_packus_epi16(values, values);
    let lanes = _mm256_castsi256_si128(_mm256_permute4x64_epi64::<0b1000>(packed));
    match W {
        4 => {
            let words = [
                _mm_cvtsi128_si32(lanes),
                _mm_extract_epi32::<1>(lanes),
                _mm_extract_epi32::<2>(lanes),
                _mm_extract_epi32::<3>(lanes),
            ];
            for (index, word) in (0..).zip(words) {
                if let Some(slot) = row(index).row_mut::<4>(output) {
                    *slot = word.to_le_bytes();
                }
            }
        }
        8 => {
            let words = [_mm_cvtsi128_si64(lanes), _mm_extract_epi64::<1>(lanes)];
            for (index, word) in (0..).zip(words) {
                if let Some(slot) = row(index).row_mut::<8>(output) {
                    *slot = word.to_le_bytes();
                }
            }
        }
        _ => {
            if let Some(slot) = at.row_mut::<16>(output) {
                u8x16::<Avx2>::simd_from(t, lanes).store_array(slot);
            }
        }
    }
}
