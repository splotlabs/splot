// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(target_arch = "aarch64")]
use fearless_simd::u16x8;
#[cfg(target_arch = "x86_64")]
use fearless_simd::{Avx2, Level, kernel};
use fearless_simd::{Simd, SimdIntWiden, i8x16, i16x8, i16x16, i32x8, prelude::*, u8x16, u16x16};

use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};
use splot_recon::BitDepth;
use splot_recon::math::round2_signed_i32;

use crate::Result;

use super::{
    GDF_BIAS, GDF_COORDS, GDF_INTRA_REF_DST, GDF_READ_RADIUS, GdfBlock, GdfClass, GdfSource,
    exact_slice, gdf_state_error,
};

pub(super) const TAP_REACH: usize = GDF_READ_RADIUS - 1;
/// Source rows a row pair reads: `TAP_REACH` above the pair to `TAP_REACH` below it.
pub(super) const WINDOW_ROWS: usize = 2 * TAP_REACH + 2;

/// The source rows a row pair reads, each `len` samples from column `origin.0 - TAP_REACH`.
/// Inlined so callers see that every row has length `len`.
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn source_rows<'a>(
    source: &GdfSource<'a>,
    origin: (usize, usize),
    len: usize,
) -> Result<[&'a [u16]; WINDOW_ROWS]> {
    let first_row = origin.1.checked_sub(TAP_REACH);
    let first_col = origin.0.checked_sub(TAP_REACH);
    let (Some(first_row), Some(first_col)) = (first_row, first_col) else {
        return Err(gdf_state_error());
    };
    let mut rows = [&source.samples[..0]; WINDOW_ROWS];
    for (index, row) in rows.iter_mut().enumerate() {
        let start = (first_row + index)
            .checked_mul(source.stride)
            .and_then(|start| start.checked_add(first_col))
            .ok_or_else(gdf_state_error)?;
        *row = exact_slice(source.samples, start, len).ok_or_else(gdf_state_error)?;
    }
    Ok(rows)
}

/// The `WIN` samples of every row in `rows` from column `x`.
#[inline]
pub(super) fn window_at<'a, const WIN: usize>(
    rows: &[&'a [u16]; WINDOW_ROWS],
    x: usize,
) -> Option<[&'a [u16; WIN]; WINDOW_ROWS]> {
    let end = x.checked_add(WIN)?;
    if rows.iter().any(|row| row.len() < end) {
        return None;
    }
    Some(rows.map(|row| {
        row.get(x..)
            .and_then(<[u16]>::first_chunk)
            .unwrap_or(const { &[0; WIN] })
    }))
}

/// The window of a 16-sample row pair: its rows with the tap reach on both sides.
type Window<'a> = [&'a [u16; 28]; WINDOW_ROWS];

/// The most columns of one column tile of a run, whose source rows sit
/// [`TILE_STRIDE`] apart: the tile's rows of a row pair window stay in L1, and
/// every window row is a constant offset from the window's first sample.
pub(super) const TILE_COLS: usize = 1024;
pub(super) const TILE_STRIDE: usize = TILE_COLS + 32;
const TILE_SPAN: usize = (WINDOW_ROWS - 1) * TILE_STRIDE + 28;
const _: () =
    assert!(TILE_STRIDE >= TILE_COLS + 2 * GDF_READ_RADIUS && TILE_COLS.is_multiple_of(16));

/// The rows of a row pair window.
trait WindowRows {
    fn row(&self, index: usize) -> &[u16; 28];
}

impl WindowRows for Window<'_> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row(&self, index: usize) -> &[u16; 28] {
        self.get(index).copied().unwrap_or(&[0; 28])
    }
}

/// A row pair window of a column tile, its rows [`TILE_STRIDE`] apart.
struct TiledWindow<'a>(&'a [u16; TILE_SPAN]);

impl WindowRows for TiledWindow<'_> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row(&self, index: usize) -> &[u16; 28] {
        self.0
            .get(index * TILE_STRIDE..)
            .and_then(<[u16]>::first_chunk)
            .unwrap_or(&[0; 28])
    }
}

/// Bit `index * 18 + k` is set when weight `index` of tap `k` is zero in every
/// GDF table for every class in the `classes` bit set.
const fn zero_weight_taps(classes: u8) -> u64 {
    let mut mask = 0;
    let mut bit = 0;
    while bit < 3 * GDF_COORDS.len() {
        let (index, k) = (bit / GDF_COORDS.len(), bit % GDF_COORDS.len());
        let mut zero = true;
        let mut table = 0;
        while table < GDF_WEIGHT.len() * GDF_WEIGHT[0].len() {
            let weights = &GDF_WEIGHT[table / GDF_WEIGHT[0].len()][table % GDF_WEIGHT[0].len()];
            let mut class = 0;
            while class < 4 {
                zero &= classes >> class & 1 == 0 || weights[index][k][class] == 0;
                class += 1;
            }
            table += 1;
        }
        if zero {
            mask |= 1 << bit;
        }
        bit += 1;
    }
    mask
}

/// The largest column distance of a tap at each row distance.
const TAP_COLUMN_REACH: [usize; TAP_REACH + 1] = {
    let mut reach = [0; TAP_REACH + 1];
    let mut k = 0;
    while k < GDF_COORDS.len() {
        let (dy, dx) = GDF_COORDS[k];
        if dx.unsigned_abs() > reach[dy as usize] {
            reach[dy as usize] = dx.unsigned_abs();
        }
        k += 1;
    }
    reach
};

/// The largest minus the smallest sample that the row pair's taps read, the
/// window's `tap_range`, or [`ROUGH`] once it exceeds [`LINEAR_RANGE`]. The
/// center rows go first: in textured windows they alone usually exceed it.
/// The rows are unrolled so that each row's tap reach is a constant.
#[inline]
fn window_range<S: Simd, W: WindowRows>(simd: S, window: &W) -> u16 {
    let (mut low, mut high) = (u16x16::splat(simd, u16::MAX), u16x16::splat(simd, 0));
    let range = |low: u16x16<S>, high: u16x16<S>| {
        simd.reduce_max_u16x16(high) - simd.reduce_min_u16x16(low)
    };
    macro_rules! rows_range {
        ($($row:literal)*) => {
            $({
                const ROW: usize = $row;
                const REACH: usize = {
                    let (above, below) = (ROW.abs_diff(TAP_REACH), ROW.abs_diff(TAP_REACH + 1));
                    TAP_COLUMN_REACH[if above < below { above } else { below }]
                };
                let row = window.row(ROW);
                for col in [TAP_REACH - REACH, TAP_REACH + REACH] {
                    let samples = u16x16::from_slice(simd, &row[col..col + 16]);
                    low = low.min(samples);
                    high = high.max(samples);
                }
            })*
        };
    }
    rows_range!(6 7);
    if range(low, high) > LINEAR_RANGE {
        return ROUGH;
    }
    rows_range!(0 1 2 3 4 5 8 9 10 11 12 13);
    match range(low, high) {
        range if range > LINEAR_RANGE => ROUGH,
        range => range,
    }
}

/// The widest `tap_range` whose combs may skip the § 7.20.5 comb clip:
/// two tap differences within it sum to at most 510.
const LINEAR_RANGE: u16 = 255;
/// [`window_range`] of a window wider than [`LINEAR_RANGE`].
const ROUGH: u16 = u16::MAX;

/// The taps of each [`row_pairs`] group, whose combs select among the same
/// diff pairs.
const TAP_GROUPS: [&[usize]; 8] = [
    &[0, 12],
    &[1, 13],
    &[2, 14],
    &[3, 15],
    &[4, 6, 7, 11],
    &[8, 10],
    &[9, 17],
    &[5, 16],
];

/// The tap that class `class` reads in place of class-0 tap `k`: class bit 0
/// transposes the tap and bit 1 mirrors it horizontally. A tap and its
/// negation are the same tap. `GDF_COORDS.len()` marks a missing tap.
const fn class_tap(k: usize, class: usize) -> usize {
    let (mut dy, mut dx) = GDF_COORDS[k];
    if class & 1 != 0 {
        (dy, dx) = (dx, dy);
    }
    if class & 2 != 0 {
        dx = -dx;
    }
    if dy < 0 || (dy == 0 && dx < 0) {
        (dy, dx) = (-dy, -dx);
    }
    let mut tap = 0;
    while tap < GDF_COORDS.len() && (GDF_COORDS[tap].0 != dy || GDF_COORDS[tap].1 != dx) {
        tap += 1;
    }
    tap
}

/// Every table's weights and clip bounds of class `c` are those of class 0
/// with the taps moved by `class_tap`, so one class-0 table serves all lanes.
const _: () = {
    let mut index = 0;
    while index < 6 * 6 * 4 * GDF_COORDS.len() {
        let (reference, qp) = (
            index / (6 * 4 * GDF_COORDS.len()),
            index / (4 * GDF_COORDS.len()) % 6,
        );
        let (class, k) = (index / GDF_COORDS.len() % 4, index % GDF_COORDS.len());
        let tap = class_tap(k, class);
        assert!(tap < GDF_COORDS.len());
        assert!(GDF_ALPHA[reference][qp][tap][class] == GDF_ALPHA[reference][qp][k][0]);
        let mut sum = 0;
        while sum < 3 {
            let weights = &GDF_WEIGHT[reference][qp][sum];
            assert!(weights[tap][class] == weights[k][0]);
            sum += 1;
        }
        index += 1;
    }
};

/// Class-0 tap pairs and the sum that each pair's multiply-add feeds.
const SYM_PAIRS: [(usize, usize, usize); 15] = [
    (0, 1, 0),
    (12, 13, 1),
    (2, 3, 0),
    (14, 15, 1),
    (3, 15, 2),
    (4, 6, 0),
    (7, 11, 1),
    (8, 10, 0),
    (8, 10, 1),
    (9, 17, 0),
    (9, 17, 1),
    (9, 17, 2),
    (5, 16, 0),
    (5, 16, 1),
    (5, 16, 2),
];
const CLASS0_ZERO_WEIGHTS: u64 = zero_weight_taps(0b0001);

/// The distinct tap pairs of `SYM_PAIRS`, in the order [`row_pairs`] interleaves them.
const COMB_PAIRS: [(usize, usize); 10] = [
    (0, 1),
    (12, 13),
    (2, 3),
    (14, 15),
    (3, 15),
    (4, 6),
    (7, 11),
    (8, 10),
    (9, 17),
    (5, 16),
];

/// The `COMB_PAIRS` slot of each `SYM_PAIRS` entry.
const PAIR_SLOT: [usize; SYM_PAIRS.len()] = {
    let mut slots = [0; SYM_PAIRS.len()];
    let mut pair = 0;
    while pair < SYM_PAIRS.len() {
        let mut slot = 0;
        while COMB_PAIRS[slot].0 != SYM_PAIRS[pair].0 || COMB_PAIRS[slot].1 != SYM_PAIRS[pair].1 {
            slot += 1;
        }
        slots[pair] = slot;
        pair += 1;
    }
    slots
};

/// `SYM_PAIRS` adds every class-0 weight that some table makes nonzero once.
const _: () = {
    let mut bit = 0;
    while bit < 3 * GDF_COORDS.len() {
        let (sum, k) = (bit / GDF_COORDS.len(), bit % GDF_COORDS.len());
        let mut uses = 0;
        let mut pair = 0;
        while pair < SYM_PAIRS.len() {
            let (a, b, target) = SYM_PAIRS[pair];
            if target == sum && (a == k || b == k) {
                uses += 1;
            }
            pair += 1;
        }
        assert!(uses == 1 || (uses == 0 && CLASS0_ZERO_WEIGHTS >> bit & 1 != 0));
        bit += 1;
    }
};

/// The error tables as `i8`, the inter ones padded to a power of two so
/// that a masked index needs no bounds check.
static GDF_INTRA_ERROR_I8: [[i8; 4096]; 6] = {
    let mut table = [[0; 4096]; 6];
    let mut index = 0;
    while index < 6 * 4096 {
        let value = GDF_INTRA_ERROR[index / 4096][index % 4096];
        assert!(value == value as i8 as i32);
        table[index / 4096][index % 4096] = value as i8;
        index += 1;
    }
    table
};
static GDF_INTER_ERROR_I8: [[[i8; 1024]; 6]; 5] = {
    let mut table = [[[0; 1024]; 6]; 5];
    let mut index = 0;
    while index < 5 * 6 * 1000 {
        let (reference, qp, entry) = (index / 6000, index / 1000 % 6, index % 1000);
        let value = GDF_INTER_ERROR[reference][qp][entry];
        assert!(value == value as i8 as i32);
        table[reference][qp][entry] = value as i8;
        index += 1;
    }
    table
};

/// The § 7.20.5 `err` of `block`'s table at each error index of `pos`, as
/// bytes: a byte lookup inserts straight from the table.
#[allow(clippy::inline_always)]
#[inline(always)]
fn errors(block: &GdfBlock, pos: &[u16; 16]) -> [i8; 16] {
    let table = error_table(block).unwrap_or_default();
    pos.map(|pos| {
        table
            .get(usize::from(pos) & (table.len() - 1))
            .copied()
            .unwrap_or(0)
    })
}

/// `block`'s § 7.20.5 error table, a power of two long.
fn error_table(block: &GdfBlock) -> Option<&'static [i8]> {
    if block.ref_dst_idx == GDF_INTRA_REF_DST {
        Some(GDF_INTRA_ERROR_I8.get(block.qp_idx)?)
    } else {
        Some(
            GDF_INTER_ERROR_I8
                .get(block.ref_dst_idx - 1)?
                .get(block.qp_idx)?,
        )
    }
}

/// Class-0 clip bounds and weights of one GDF table, with its bias and the
/// § 7.20.5 `scale`.
///
/// The kernels read `center - tap`, the negated tap differences. The clip
/// bounds are symmetric, so every comb comes out negated, and the weighted
/// combs are subtracted from the bias.
#[repr(align(32))]
pub(super) struct GdfSymParams {
    /// Each tap's clip bounds `[-alpha, alpha]` plus [`ABOVE_OFFSET`], then
    /// plus [`BELOW_OFFSET`], as whole vectors that the clamps read from memory.
    bounds: [[[u16; 16]; 4]; GDF_COORDS.len()],
    /// The weights of each `SYM_PAIRS` entry in alternating lanes.
    pair_weights: [[i16; 16]; SYM_PAIRS.len()],
    bias: [i32; 3],
    /// The widest `tap_range` at which each `TAP_GROUPS` entry skips the
    /// clips: its smallest clip bound, at most [`LINEAR_RANGE`].
    group_bounds: [u16; TAP_GROUPS.len()],
    /// The bias less [`COMB_BIAS`] times the weights of each sum, which the
    /// biased combs add back.
    tap_bias: [i32; 3],
    scale: i16,
    /// Whether a flat window with gradient bias 0 keeps its samples.
    keeps_flat: bool,
}

impl GdfSymParams {
    pub(super) fn new(block: &GdfBlock) -> Self {
        let alpha = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
        let table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
        let scale: i16 = if block.ref_dst_idx == GDF_INTRA_REF_DST {
            8
        } else {
            5
        };
        let bias = GDF_BIAS[block.ref_dst_idx][block.qp_idx];
        let flat_pos = bias.into_iter().fold(0, |pos, bias| {
            let scale = i32::from(scale);
            let digit = round2_signed_i32(bias * scale, 15).clamp(-scale, scale - 1);
            pos * 2 * scale + digit + scale
        });
        let flat_error = i16::from(errors(block, &[flat_pos as u16; 16])[0]);
        let rounding = 12 - u32::from(block.bit_depth.bits());
        Self {
            bounds: core::array::from_fn(|k| {
                let alpha = alpha[k][0];
                [
                    [ABOVE_OFFSET - alpha; 16],
                    [ABOVE_OFFSET + alpha; 16],
                    [BELOW_OFFSET - alpha; 16],
                    [BELOW_OFFSET + alpha; 16],
                ]
            }),
            pair_weights: SYM_PAIRS.map(|(a, b, sum)| {
                core::array::from_fn(|lane| table[sum][if lane & 1 == 0 { a } else { b }][0])
            }),
            bias,
            group_bounds: TAP_GROUPS.map(|taps| {
                taps.iter()
                    .fold(LINEAR_RANGE, |bound, &k| bound.min(alpha[k][0]))
            }),
            tap_bias: core::array::from_fn(|sum| {
                let weights: i32 = SYM_PAIRS
                    .iter()
                    .filter(|pair| pair.2 == sum)
                    .map(|&(a, b, _)| i32::from(table[sum][a][0]) + i32::from(table[sum][b][0]))
                    .sum();
                bias[sum] - COMB_BIAS * weights
            }),
            scale,
            keeps_flat: round2_signed_i32(i32::from(flat_error) * block.pix_scale, rounding) == 0,
        }
    }
}

/// How a kernel pairs comb lanes for its `i16` multiply-adds. The sums of
/// [`Lanes::madd`] and the halves of [`Lanes::spread`] hold the same
/// samples, and [`Lanes::digits`] reads its sums in that order.
trait Lanes<S: Simd>: Copy {
    /// The combs `a` and `b` as [`Lanes::madd`] reads them.
    fn pairs(self, simd: S, a: i16x16<S>, b: i16x16<S>) -> [i16x16<S>; 2];
    /// Subtracts from `sum` the combs of [`Lanes::pairs`] times their
    /// weights, which alternate in `weights`.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn madd(self, simd: S, sum: &mut [i32x8<S>; 2], pair: [i16x16<S>; 2], weights: &[i16; 16]) {
        let weights = i16x16::from_slice(simd, weights);
        for (sum, half) in sum.iter_mut().zip(pair) {
            *sum -= simd.pairwise_dot_product_i16x16(half, weights);
        }
    }
    /// Sums of the 8 two-sample class blocks, per sample in the order of
    /// the sums of [`Lanes::madd`].
    fn spread(self, simd: S, blocks: i32x8<S>) -> [i32x8<S>; 2];
    /// The § 7.20.5 digit `Clip3(-scale, scale - 1, Round2Signed(sum * scale, 15))`
    /// of every sample, in sample order.
    fn digits(self, simd: S, sums: [i32x8<S>; 2], scale: i16) -> i16x16<S>;
}

/// Keeps each comb whole and multiplies it by its weight in widening lanes:
/// the first half of a sum holds samples `0..8`.
#[derive(Clone, Copy)]
struct Widening;

impl<S: Simd> Lanes<S> for Widening {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn pairs(self, _simd: S, a: i16x16<S>, b: i16x16<S>) -> [i16x16<S>; 2] {
        [a, b]
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn madd(self, simd: S, sum: &mut [i32x8<S>; 2], pair: [i16x16<S>; 2], weights: &[i16; 16]) {
        let products = |comb: i16x16<S>, weight: i16| {
            let weight = i16x8::splat(simd, weight);
            let (low, high) = simd.split_i16x16(comb);
            let (p0, p1) = low.widening_mul(weight);
            let (p2, p3) = high.widening_mul(weight);
            [simd.combine_i32x4(p0, p1), simd.combine_i32x4(p2, p3)]
        };
        let ([a0, a1], [b0, b1]) = (products(pair[0], weights[0]), products(pair[1], weights[1]));
        sum[0] = sum[0] - a0 - b0;
        sum[1] = sum[1] - a1 - b1;
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn spread(self, simd: S, blocks: i32x8<S>) -> [i32x8<S>; 2] {
        let (low, high) = simd.interleave_i32x8(blocks, blocks);
        [low, high]
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn digits(self, simd: S, sums: [i32x8<S>; 2], scale: i16) -> i16x16<S> {
        let scale = i32::from(scale);
        let [low, high] = sums.map(|sum| {
            let scaled = sum * scale;
            ((scaled + (scaled >> 31) + (1 << 14)) >> 15)
                .clamp(i32x8::splat(simd, -scale), i32x8::splat(simd, scale - 1))
        });
        simd.narrow_i32x8(low, high)
    }
}

/// Pairs by AVX2 in-lane unpacks: the first half holds samples `0..4` and
/// `8..12`, which an in-lane pack of the two halves puts back in order.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
struct InLane;

#[cfg(target_arch = "x86_64")]
kernel!(
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn unpack_pairs(t: Avx2, a: i16x16<Avx2>, b: i16x16<Avx2>) -> [i16x16<Avx2>; 2] {
        use core::arch::x86_64::{_mm256_unpackhi_epi16, _mm256_unpacklo_epi16};
        [
            _mm256_unpacklo_epi16(a.into(), b.into()).simd_into(t),
            _mm256_unpackhi_epi16(a.into(), b.into()).simd_into(t),
        ]
    }
);

#[cfg(target_arch = "x86_64")]
kernel!(
    /// The sums clamp to `[-32768, 0x7000]`: the pack saturates the lower
    /// end, and past the positive cap every digit is `scale - 1`. `pmulhrsw`
    /// then rounds `|v| * scale` by 15 bits; -32768 gives `-scale`.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn packed_digits(t: Avx2, low: i32x8<Avx2>, high: i32x8<Avx2>, scale: i16) -> i16x16<Avx2> {
        use core::arch::x86_64::{
            __m256i, _mm256_min_epi32, _mm256_mulhrs_epi16, _mm256_packs_epi32, _mm256_set1_epi16,
            _mm256_set1_epi32, _mm256_sign_epi16,
        };
        let clamp = |sums: __m256i| _mm256_min_epi32(sums, _mm256_set1_epi32(0x7000));
        let value = _mm256_packs_epi32(clamp(low.into()), clamp(high.into()));
        let signed_scale = _mm256_sign_epi16(_mm256_set1_epi16(scale), value);
        let magnitude = _mm256_mulhrs_epi16(signed_scale, value);
        _mm256_sign_epi16(magnitude, value).simd_into(t)
    }
);

#[cfg(target_arch = "x86_64")]
kernel!(
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn unpack_blocks(t: Avx2, blocks: i32x8<Avx2>) -> [i32x8<Avx2>; 2] {
        use core::arch::x86_64::{_mm256_unpackhi_epi32, _mm256_unpacklo_epi32};
        let blocks = blocks.into();
        [
            _mm256_unpacklo_epi32(blocks, blocks).simd_into(t),
            _mm256_unpackhi_epi32(blocks, blocks).simd_into(t),
        ]
    }
);

#[cfg(target_arch = "x86_64")]
impl Lanes<Avx2> for InLane {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn pairs(self, simd: Avx2, a: i16x16<Avx2>, b: i16x16<Avx2>) -> [i16x16<Avx2>; 2] {
        unpack_pairs(simd, a, b)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn spread(self, simd: Avx2, blocks: i32x8<Avx2>) -> [i32x8<Avx2>; 2] {
        unpack_blocks(simd, blocks)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn digits(self, simd: Avx2, [low, high]: [i32x8<Avx2>; 2], scale: i16) -> i16x16<Avx2> {
        packed_digits(simd, low, high, scale)
    }
}

/// Offsets of the negated tap reads above and below a sample: each read
/// `center - tap` is taken modulo 2^16 with its offset added, which keeps it
/// in `u16`. The offsets sum to [`COMB_OFFSET`], so that a saturating `u16`
/// add of the two clipped reads clips their negated comb to at most 512.
const ABOVE_OFFSET: u16 = 32512;
const BELOW_OFFSET: u16 = COMB_OFFSET - ABOVE_OFFSET;
const COMB_OFFSET: u16 = u16::MAX - 512;
/// A negated comb plus [`COMB_OFFSET`], read as `i16`, is the comb less this.
const COMB_BIAS: i32 = 65536 - COMB_OFFSET as i32;

/// The centre samples of a row plus [`ABOVE_OFFSET`] and [`BELOW_OFFSET`].
type Centers<S> = (u16x16<S>, u16x16<S>);

/// The samples of `row` from column `col`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn samples<S: Simd>(simd: S, row: &[u16; 28], col: usize) -> u16x16<S> {
    u16x16::from_slice(simd, &row[col..col + 16])
}

/// The offset negated differences `center - tap` of tap `TAP` for row
/// `ROW`: the tap's two mirrored reads.
#[allow(clippy::inline_always)]
#[inline(always)]
fn tap_diffs<S: Simd, W: WindowRows, const TAP: usize, const ROW: usize>(
    simd: S,
    window: &W,
    (above, below): Centers<S>,
) -> (u16x16<S>, u16x16<S>) {
    let (dy, dx) = GDF_COORDS[TAP];
    let row = TAP_REACH + ROW;
    (
        above
            - samples(
                simd,
                window.row(row - dy as usize),
                (TAP_REACH as isize - dx) as usize,
            ),
        below
            - samples(
                simd,
                window.row(row + dy as usize),
                (TAP_REACH as isize + dx) as usize,
            ),
    )
}

/// The negated comb, less [`COMB_BIAS`], of the offset negated reads
/// `above` and `below`, each within its offset tap bounds: the saturating
/// add clips the comb's upper end at 512 and the max its lower end at -511.
/// Every lane of a bound holds one value, so NEON loads half of it.
#[allow(clippy::inline_always)]
#[inline(always)]
fn clipped_sum<S: Simd>(
    simd: S,
    above: u16x16<S>,
    below: u16x16<S>,
    bounds: &[[u16; 16]; 4],
) -> i16x16<S> {
    #[cfg(target_arch = "aarch64")]
    let bound = |index: usize| {
        let half = u16x8::from_slice(simd, &bounds[index][..8]);
        simd.combine_u16x8(half, half)
    };
    #[cfg(not(target_arch = "aarch64"))]
    let bound = |index: usize| u16x16::from_slice(simd, &bounds[index]);
    let above = above.max(bound(0)).min(bound(1));
    let below = below.max(bound(2)).min(bound(3));
    above
        .saturating_add(below)
        .max(u16x16::splat(simd, COMB_OFFSET - 511))
        .bitcast()
}

/// `a` and `b` with the lanes where `select` (all ones or zero per lane) is
/// set exchanged. NEON does it in two bit selects; elsewhere two xors of one
/// masked difference, four operations where two AVX2 blends are six micro-ops.
#[allow(clippy::inline_always)]
#[inline(always)]
fn swap<S: Simd>(a: u16x16<S>, b: u16x16<S>, select: u16x16<S>) -> (u16x16<S>, u16x16<S>) {
    #[cfg(target_arch = "aarch64")]
    let swapped = {
        let select = select.simd_ne(u16x16::splat(a.simd, 0));
        (select.select(b, a), select.select(a, b))
    };
    #[cfg(not(target_arch = "aarch64"))]
    let swapped = {
        let moved = (a ^ b) & select;
        (a ^ moved, b ^ moved)
    };
    swapped
}

/// The negated comb of tap `TAP` for row `ROW` less [`COMB_BIAS`], without
/// the clips: `center2` is the centre doubled less [`COMB_BIAS`]. It equals
/// the clipped comb when no tap read differs from the centre by more than the
/// tap's bound or by more than [`LINEAR_RANGE`].
#[allow(clippy::inline_always)]
#[inline(always)]
fn linear_comb<S: Simd, W: WindowRows, const TAP: usize, const ROW: usize>(
    simd: S,
    window: &W,
    center2: u16x16<S>,
) -> u16x16<S> {
    let (dy, dx) = GDF_COORDS[TAP];
    let row = TAP_REACH + ROW;
    center2
        - samples(
            simd,
            window.row(row - dy as usize),
            (TAP_REACH as isize - dx) as usize,
        )
        - samples(
            simd,
            window.row(row + dy as usize),
            (TAP_REACH as isize + dx) as usize,
        )
}

/// The class lane masks that transpose and mirror the taps.
type Masks<S> = (u16x16<S>, u16x16<S>);

/// The clipped sums of taps `K` and `T`, whose candidate samples coincide:
/// each lane of one reads the tap its class moves `K` to, so the two combs
/// select between the same two diff pairs. `select` is the class mask that
/// sends `K`'s lanes to `T` (`transposed`, or `mirrored` when `K`'s transpose
/// is itself and its mirror is `T`).
#[allow(clippy::inline_always)]
#[inline(always)]
fn paired_combs<S: Simd, W: WindowRows, const K: usize, const T: usize, const ROW: usize>(
    simd: S,
    window: &W,
    centers: Centers<S>,
    select: u16x16<S>,
    params: &GdfSymParams,
) -> [i16x16<S>; 2] {
    let (above, below) = tap_diffs::<S, W, K, ROW>(simd, window, centers);
    let (t_above, t_below) = tap_diffs::<S, W, T, ROW>(simd, window, centers);
    let (above, t_above) = swap(above, t_above, select);
    let (below, t_below) = swap(below, t_below, select);
    [
        clipped_sum(simd, above, below, &params.bounds[K]),
        clipped_sum(simd, t_above, t_below, &params.bounds[T]),
    ]
}

/// The clipped sums of the tap cycle `A`, `B`, `C`, `D`, whose four combs
/// select among the same four diff pairs: mirroring swaps `A`/`B` and
/// `C`/`D`, transposing swaps the pairs.
#[allow(clippy::inline_always)]
#[inline(always)]
fn cycle_combs<
    S: Simd,
    W: WindowRows,
    const A: usize,
    const B: usize,
    const C: usize,
    const D: usize,
    const ROW: usize,
>(
    simd: S,
    window: &W,
    centers: Centers<S>,
    (transposed, mirrored): Masks<S>,
    params: &GdfSymParams,
) -> [i16x16<S>; 4] {
    let (a_above, a_below) = tap_diffs::<S, W, A, ROW>(simd, window, centers);
    let (b_above, b_below) = tap_diffs::<S, W, B, ROW>(simd, window, centers);
    let (c_above, c_below) = tap_diffs::<S, W, C, ROW>(simd, window, centers);
    let (d_above, d_below) = tap_diffs::<S, W, D, ROW>(simd, window, centers);
    let select4 = |a: u16x16<S>, b: u16x16<S>, c: u16x16<S>, d: u16x16<S>| {
        let ((ab0, ab1), (cd0, cd1)) = (swap(a, b, mirrored), swap(c, d, mirrored));
        let ((first, third), (second, fourth)) =
            (swap(ab0, cd0, transposed), swap(ab1, cd1, transposed));
        [first, second, third, fourth]
    };
    let above = select4(a_above, b_above, c_above, d_above);
    let below = select4(a_below, b_below, c_below, d_below);
    [
        clipped_sum(simd, above[0], below[0], &params.bounds[A]),
        clipped_sum(simd, above[1], below[1], &params.bounds[B]),
        clipped_sum(simd, above[2], below[2], &params.bounds[C]),
        clipped_sum(simd, above[3], below[3], &params.bounds[D]),
    ]
}

/// The three § 7.20.5 `gdfIdx` sums of a row, in the halves of [`Lanes`].
type Sums<S> = [[i32x8<S>; 2]; 3];

/// The § 7.20.5 error index `pos` of the biased `gdfIdx` sums `sums`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn error_index<S: Simd, L: Lanes<S>>(
    simd: S,
    lanes: L,
    sums: Sums<S>,
    params: &GdfSymParams,
) -> [u16; 16] {
    let scale = params.scale;
    let [first, second, third] = sums.map(|sums| lanes.digits(simd, sums, scale));
    let radix = 2 * scale;
    let offset = (scale * radix + scale) * radix + scale;
    ((first * radix + second) * radix + third + offset)
        .bitcast::<u16x16<S>>()
        .to_array()
}

/// Adds to the samples of `output` the § 7.20.5 residual of the error
/// indices `pos`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn add_residual<S: Simd, O: GdfSample>(
    simd: S,
    pos: &[u16; 16],
    output: &mut [O; 16],
    block: &GdfBlock,
) {
    let (low, high) = simd.widen_i8x16(i8x16::from_slice(simd, &errors(block, pos)));
    let residual = residuals(simd.combine_i16x8(low, high), block);
    apply_residuals(simd, residual, output, block);
}

/// The § 7.20.5 residuals of the `err` lanes `error`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn residuals<S: Simd>(error: i16x16<S>, block: &GdfBlock) -> i16x16<S> {
    let scaled = error * block.pix_scale as i16;
    let rounding = 12 - u32::from(block.bit_depth.bits());
    if rounding == 0 {
        scaled
    } else {
        (scaled + (1 << (rounding - 1)) + (scaled >> 15)) >> rounding
    }
}

/// Adds `residual` to `output`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn apply_residuals<S: Simd, O: GdfSample>(
    simd: S,
    residual: i16x16<S>,
    output: &mut [O; 16],
    block: &GdfBlock,
) {
    let base = O::load(simd, output);
    let filtered = (base + residual)
        .max(i16x16::splat(simd, 0))
        .min(i16x16::splat(simd, block.max_sample as i16));
    O::store(simd, filtered, output);
}

/// A sample type of the luma rows GDF filters in place.
pub(super) trait GdfSample: Copy + Default {
    /// The 16 samples of `samples` as `i16` lanes.
    fn load<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S>;
    /// Stores 16 lanes, each within the sample range, into `samples`.
    fn store<S: Simd>(simd: S, lanes: i16x16<S>, samples: &mut [Self; 16]);
}

impl GdfSample for u16 {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn load<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S> {
        u16x16::from_slice(simd, samples).bitcast()
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn store<S: Simd>(_simd: S, lanes: i16x16<S>, samples: &mut [Self; 16]) {
        lanes.bitcast::<u16x16<S>>().store_slice(samples);
    }
}

impl GdfSample for u8 {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn load<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S> {
        let (low, high) = simd.widen_u8x16(u8x16::from_slice(simd, samples));
        simd.combine_u16x8(low, high).bitcast()
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn store<S: Simd>(simd: S, lanes: i16x16<S>, samples: &mut [Self; 16]) {
        let (low, high) = simd.split_u16x16(lanes.bitcast());
        simd.narrow_u16x8(low, high).store_slice(samples);
    }
}

/// The negated combs of one row for each `COMB_PAIRS` slot, as
/// [`Lanes::pairs`] gives them.
type RowPairs<S> = [[i16x16<S>; 2]; COMB_PAIRS.len()];

/// The negated combs of row `ROW` of 16 samples of any class mix, paired
/// for [`row_index`]. Taps are grouped so that each group's combs share
/// their candidate samples and selects: a tap and its transpose read
/// mirrored offsets, and taps `4`, `6`, `7`, `11` form a cycle. Kept apart
/// from the multiply-adds, whose six sums would otherwise force LLVM to
/// spill the combs.
///
/// Bit `g` of `linear` marks the `TAP_GROUPS` entries whose combs skip the
/// clips: the swaps then act on the combs instead of each diff.
#[inline(never)]
fn row_pairs<S: Simd, L: Lanes<S>, W: WindowRows, const ROW: usize>(
    simd: S,
    lanes: L,
    window: &W,
    (masks, linear): (Masks<S>, u8),
    params: &GdfSymParams,
) -> RowPairs<S> {
    let center = samples(simd, window.row(TAP_REACH + ROW), TAP_REACH);
    let centers = (center + ABOVE_OFFSET, center + BELOW_OFFSET);
    let center2 = centers.0 + centers.1;
    let (transposed, mirrored) = masks;
    macro_rules! pair {
        ($group:literal, $k:literal, $t:literal, $select:expr) => {
            if linear >> $group & 1 != 0 {
                let (k, t) = swap(
                    linear_comb::<S, W, $k, ROW>(simd, window, center2),
                    linear_comb::<S, W, $t, ROW>(simd, window, center2),
                    $select,
                );
                [k.bitcast(), t.bitcast()]
            } else {
                paired_combs::<S, W, $k, $t, ROW>(simd, window, centers, $select, params)
            }
        };
    }
    let mut out = [[i16x16::splat(simd, 0); 2]; COMB_PAIRS.len()];
    let [tap0, tap12] = pair!(0, 0, 12, transposed);
    let [tap1, tap13] = pair!(1, 1, 13, transposed);
    out[0] = lanes.pairs(simd, tap0, tap1);
    out[1] = lanes.pairs(simd, tap12, tap13);
    let [tap2, tap14] = pair!(2, 2, 14, transposed);
    let [tap3, tap15] = pair!(3, 3, 15, transposed);
    out[2] = lanes.pairs(simd, tap2, tap3);
    out[3] = lanes.pairs(simd, tap14, tap15);
    out[4] = lanes.pairs(simd, tap3, tap15);
    let [tap4, tap6, tap7, tap11] = if linear >> 4 & 1 != 0 {
        let comb = |tap: u16x16<S>| tap.bitcast::<i16x16<S>>();
        let (a, b) = swap(
            linear_comb::<S, W, 4, ROW>(simd, window, center2),
            linear_comb::<S, W, 6, ROW>(simd, window, center2),
            mirrored,
        );
        let (c, d) = swap(
            linear_comb::<S, W, 7, ROW>(simd, window, center2),
            linear_comb::<S, W, 11, ROW>(simd, window, center2),
            mirrored,
        );
        let ((first, third), (second, fourth)) = (swap(a, c, transposed), swap(b, d, transposed));
        [comb(first), comb(second), comb(third), comb(fourth)]
    } else {
        cycle_combs::<S, W, 4, 6, 7, 11, ROW>(simd, window, centers, masks, params)
    };
    out[5] = lanes.pairs(simd, tap4, tap6);
    out[6] = lanes.pairs(simd, tap7, tap11);
    let [tap8, tap10] = pair!(5, 8, 10, mirrored);
    out[7] = lanes.pairs(simd, tap8, tap10);
    let [tap9, tap17] = pair!(6, 9, 17, transposed);
    out[8] = lanes.pairs(simd, tap9, tap17);
    let [tap5, tap16] = pair!(7, 5, 16, transposed);
    out[9] = lanes.pairs(simd, tap5, tap16);
    out
}

/// The error indices of a row from its paired combs: each `SYM_PAIRS` entry
/// subtracts its weighted combs from its sum.
#[inline(never)]
fn row_index<S: Simd, L: Lanes<S>>(
    simd: S,
    lanes: L,
    pairs: &RowPairs<S>,
    init: &Sums<S>,
    params: &GdfSymParams,
) -> [u16; 16] {
    let mut sums = *init;
    macro_rules! add_pairs {
        ($($pair:literal)*) => {$({
            const PAIR: usize = $pair;
            let sum = &mut sums[SYM_PAIRS[PAIR].2];
            lanes.madd(simd, sum, pairs[PAIR_SLOT[PAIR]], &params.pair_weights[PAIR]);
        })*};
    }
    add_pairs!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14);
    error_index(simd, lanes, sums, params)
}

/// Filters in place a row pair of 16 samples whose class changes every two
/// samples, reading class-0 weights and moving each lane's taps by its
/// class. A window with `tap_range` 0 adds zero at every tap, so its sums
/// stay at the bias.
#[allow(clippy::inline_always)]
#[inline(always)]
fn sym_rows_with<S: Simd, L: Lanes<S>, W: WindowRows, R: RowSink>(
    simd: S,
    lanes: L,
    window: &W,
    mut sink: R,
    classes: i32x8<S>,
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    let range = window_range(simd, window);
    let flat = range == 0;
    if flat && params.keeps_flat && (classes >> 2).simd_eq(i32x8::splat(simd, 0)).all_true() {
        sink.keep(simd);
        return;
    }
    let biases = |bias: [i32; 3]| {
        [
            lanes.spread(simd, i32x8::splat(simd, bias[0])),
            lanes.spread(simd, i32x8::splat(simd, bias[1])),
            lanes.spread(simd, (classes >> 2) + bias[2]),
        ]
    };
    if flat {
        let pos = error_index(simd, lanes, biases(params.bias), params);
        sink.row(simd, 0, &pos, block);
        sink.row(simd, 1, &pos, block);
        return;
    }
    let init = biases(params.tap_bias);
    let class = classes & 3;
    let class = (class | (class << 16)).bitcast::<u16x16<S>>();
    let zero = u16x16::splat(simd, 0);
    let masks = (zero - (class & 1), zero - ((class >> 1) & 1));
    let linear = params
        .group_bounds
        .iter()
        .enumerate()
        .fold(0, |linear, (group, &bound)| {
            linear | u8::from(range <= bound) << group
        });
    let pairs = row_pairs::<S, L, W, 0>(simd, lanes, window, (masks, linear), params);
    let pos = row_index(simd, lanes, &pairs, &init, params);
    sink.row(simd, 0, &pos, block);
    let pairs = row_pairs::<S, L, W, 1>(simd, lanes, window, (masks, linear), params);
    let pos = row_index(simd, lanes, &pairs, &init, params);
    sink.row(simd, 1, &pos, block);
}

/// Where [`sym_rows_with`] puts a row pair's § 7.20.5 error indices.
trait RowSink {
    /// Takes the error indices `pos` of row `row` (0 or 1) of the pair.
    fn row<S: Simd>(&mut self, simd: S, row: usize, pos: &[u16; 16], block: &GdfBlock);
    /// Takes a pair whose samples all keep their values.
    fn keep<S: Simd>(&mut self, simd: S);
}

/// Adds the residuals to the pair's two output rows at once.
impl<O: GdfSample> RowSink for [&mut [O; 16]; 2] {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row<S: Simd>(&mut self, simd: S, row: usize, pos: &[u16; 16], block: &GdfBlock) {
        if let Some(output) = self.get_mut(row) {
            add_residual(simd, pos, output, block);
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn keep<S: Simd>(&mut self, _simd: S) {}
}

/// Stores the pair's residual bytes for [`add_residuals`]: each is a scalar
/// load and store, independent of the others, where inserting them into one
/// vector chains every lookup on the one before. The table holds the
/// residual of each entry of the block's error table from index 0, so that
/// any `u16` index reads it without a mask or bounds check; every residual
/// fits a byte, as `err` does and `GdfPixScale <= 1 << (12 - BitDepth)`.
struct ErrorRows<'a>([&'a mut [i8; 16]; 2], &'a [i8; 1 << 16]);

impl RowSink for ErrorRows<'_> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row<S: Simd>(&mut self, _simd: S, row: usize, pos: &[u16; 16], _block: &GdfBlock) {
        if let Some(errors) = self.0.get_mut(row) {
            for (error, &pos) in errors.iter_mut().zip(pos) {
                *error = self.1[usize::from(pos)];
            }
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn keep<S: Simd>(&mut self, simd: S) {
        for errors in &mut self.0 {
            i8x16::splat(simd, 0).store_slice(&mut errors[..]);
        }
    }
}

/// Adds to each sample of `output` its § 7.20.5 residual in `residuals`, 16
/// samples at a time.
fn add_residuals<S: Simd, O: GdfSample>(
    simd: S,
    residuals: &[i8],
    output: &mut [O],
    block: &GdfBlock,
) {
    let (residuals, _) = residuals.as_chunks::<16>();
    let (output, _) = output.as_chunks_mut::<16>();
    for (residuals, output) in residuals.iter().zip(output) {
        let (low, high) = simd.widen_i8x16(i8x16::from_slice(simd, residuals));
        apply_residuals(simd, simd.combine_i16x8(low, high), output, block);
    }
}

/// [`sym_rows_with`] through the target's widest kernel.
#[inline(never)]
pub(super) fn sym_rows<O: GdfSample>(
    window: &Window<'_>,
    output: [&mut [O; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        let classes = i32x8::from_slice(avx2, &classes.map(|class| class.0));
        return sym_rows_with(avx2, InLane, window, output, classes, block, params);
    }
    sym_rows_portable(window, output, classes, block, params);
}

/// [`sym_rows_with`] through the portable kernel.
pub(super) fn sym_rows_portable<O: GdfSample>(
    window: &Window<'_>,
    output: [&mut [O; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    crate::baseline_simd!(simd => sym_rows_with(
        simd,
        Widening,
        window,
        output,
        i32x8::from_slice(simd, &classes.map(|class| class.0)),
        block,
        params,
    ));
}

/// Filters in place `width` samples (at most 16) from column `x` of a row
/// pair through `sym_rows`, on copies padded to 16 lanes.
pub(super) fn sym_rows_narrow<O: GdfSample>(
    rows: &[&[u16]; WINDOW_ROWS],
    x: usize,
    width: usize,
    [top, bottom]: [&mut [O]; 2],
    classes: &[GdfClass],
    block: &GdfBlock,
    params: &GdfSymParams,
) -> Option<()> {
    let mut window = [[0; 28]; WINDOW_ROWS];
    for (padded, row) in window.iter_mut().zip(rows) {
        let samples = row.get(x..x.checked_add(width + 2 * TAP_REACH)?)?;
        padded.get_mut(..samples.len())?.copy_from_slice(samples);
    }
    let mut lane_classes = [GdfClass::default(); 8];
    let class_count = width.div_ceil(2);
    lane_classes
        .get_mut(..class_count)?
        .copy_from_slice(classes.get(..class_count)?);
    let (top, bottom) = (top.get_mut(..width)?, bottom.get_mut(..width)?);
    let mut output = [[O::default(); 16]; 2];
    output[0][..width].copy_from_slice(top);
    output[1][..width].copy_from_slice(bottom);
    let [first, second] = &mut output;
    sym_rows(
        &window.each_ref(),
        [first, second],
        &lane_classes,
        block,
        params,
    );
    top.copy_from_slice(&output[0][..width]);
    bottom.copy_from_slice(&output[1][..width]);
    Some(())
}

/// Gradient sums of 16 columns, one row per direction.
type GradChunk = [[u16; 16]; 4];

/// Scratch of [`band_classes`]: for the last two gradient row pairs, the
/// four direction gradients of each column summed over the pair's two rows,
/// in 16-column chunks.
#[derive(Default)]
pub(super) struct ClassScratch {
    pub(super) grads: Vec<GradChunk>,
    /// A row pair's residual bytes, two rows of [`ClassScratch::error_row`]
    /// each, for [`add_residuals`].
    pub(super) errors: Vec<i8>,
    /// [`ErrorRows`]' table, and the block facts it was built for.
    pub(super) table: Vec<i8>,
    pub(super) table_key: Option<(usize, usize, i32, BitDepth)>,
}

impl ClassScratch {
    /// The chunks of one gradient row pair for a run of `class_cols` class
    /// columns: those its `2 * class_cols + 2` columns cover, plus one that a
    /// class chunk reads past them.
    pub(super) fn chunks(class_cols: usize) -> usize {
        (2 * class_cols + 2).div_ceil(16) + 1
    }

    /// The `err` bytes of one row of a run `run` samples wide.
    pub(super) fn error_row(run: usize) -> usize {
        run.next_multiple_of(16)
    }
}

/// The four § 7.20.5 `grad` directions of the 16 columns from column 1 of
/// the row `rows[1]`, `rows[0]` above it and `rows[2]` below it.
#[allow(clippy::inline_always)]
#[inline(always)]
fn gradients<S: Simd>(simd: S, rows: [&[u16; 18]; 3]) -> [i16x16<S>; 4] {
    let load = |row: &[u16; 18], offset: usize| -> i16x16<S> {
        u16x16::from_slice(simd, &row[offset..offset + 16]).bitcast()
    };
    let [above, center, below] = rows;
    let mid = load(center, 1);
    let twice = mid + mid;
    let grad = |before: i16x16<S>, after: i16x16<S>| (twice - before - after).abs();
    [
        grad(load(above, 1), load(below, 1)),
        grad(load(center, 0), load(center, 2)),
        grad(load(above, 0), load(below, 2)),
        grad(load(below, 0), load(above, 2)),
    ]
}

/// Fills `out` with the gradients of the gradient row pair between the four
/// source rows, summed over its two rows, for the columns from column 1 of
/// each row; a chunk that reads past the rows reads zeros there.
fn gradient_row<S: Simd>(simd: S, rows: &[&[u16]; 4], out: &mut [GradChunk]) -> Option<()> {
    for (chunk, out) in out.iter_mut().enumerate() {
        let start = 16 * chunk;
        let chunks = rows.map(|row| row.get(start..).and_then(<[u16]>::first_chunk::<18>));
        if let [Some(a), Some(b), Some(c), Some(d)] = chunks {
            pair_gradients(simd, [a, b, c, d], out);
        } else {
            padded_gradients(simd, rows, start, out)?;
        }
    }
    Some(())
}

/// [`gradient_row`] for one chunk whose rows end before its reach, on
/// zero-padded copies; out of line, so that the padding is only built here.
#[inline(never)]
fn padded_gradients<S: Simd>(
    simd: S,
    rows: &[&[u16]; 4],
    start: usize,
    out: &mut GradChunk,
) -> Option<()> {
    let mut padded = [[0; 18]; 4];
    for (padded, row) in padded.iter_mut().zip(rows) {
        let tail = row.get(start..).unwrap_or_default();
        let tail = tail.get(..tail.len().min(18))?;
        padded.get_mut(..tail.len())?.copy_from_slice(tail);
    }
    pair_gradients(simd, padded.each_ref(), out);
    Some(())
}

/// The gradients of one chunk of a gradient row pair, summed over its rows.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pair_gradients<S: Simd>(simd: S, chunks: [&[u16; 18]; 4], out: &mut GradChunk) {
    let first = gradients(simd, [chunks[0], chunks[1], chunks[2]]);
    let second = gradients(simd, [chunks[1], chunks[2], chunks[3]]);
    for ((out, first), second) in out.iter_mut().zip(first).zip(second) {
        (first + second).bitcast::<u16x16<S>>().store_array(out);
    }
}

/// The direction gradients of a chunk summed over two gradient row pairs.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pair_sum<S: Simd>(simd: S, current: &GradChunk, previous: &GradChunk) -> [i16x16<S>; 4] {
    core::array::from_fn(|d| {
        (u16x16::from_slice(simd, &current[d]) + u16x16::from_slice(simd, &previous[d])).bitcast()
    })
}

/// The classes and gradient biases of the 8 class columns of a chunk from
/// the gradients `sums` of its columns and `next` of the next chunk over
/// both gradient row pairs: each direction strength sums four columns from
/// `2j`, at most `8 * 4092`, so it fits an `i16` multiply-add, which by 4096
/// also moves `strength >> 4` to the upper half. `tables` holds the clip
/// bounds and the weights of directions 0 and 1, then of 2 and 3, in
/// alternating lanes.
#[allow(clippy::inline_always)]
#[inline(always)]
fn classify_chunk<S: Simd>(
    simd: S,
    sums: [i16x16<S>; 4],
    next: [i16x16<S>; 4],
    tables: &[[i16; 16]; 4],
) -> i32x8<S> {
    let strengths: [i32x8<S>; 4] = core::array::from_fn(|d| {
        let columns = sums[d] + simd.slide_i16x16::<2>(sums[d], next[d]);
        simd.pairwise_dot_product_i16x16(columns, i16x16::splat(simd, 4096))
    });
    let bit = |a: i32x8<S>, b: i32x8<S>, value: i32| {
        a.simd_le(b)
            .select(i32x8::splat(simd, value), i32x8::splat(simd, 0))
    };
    let index = bit(strengths[0], strengths[1], 1) | bit(strengths[2], strengths[3], 2);
    let high = i32x8::splat(simd, -65536);
    let combs = |low: i32x8<S>, high_half: i32x8<S>, [alpha, weight]: [&[i16; 16]; 2]| {
        let packed: i16x16<S> = ((low >> 16) | (high_half & high)).bitcast();
        let alpha = i16x16::from_slice(simd, alpha);
        simd.pairwise_dot_product_i16x16(packed.min(alpha), i16x16::from_slice(simd, weight))
    };
    let bias = combs(strengths[0], strengths[1], [&tables[0], &tables[1]])
        + combs(strengths[2], strengths[3], [&tables[2], &tables[3]]);
    (bias << 2) | index
}

/// The four source rows of gradient row pair `row_pair` of the run of
/// `class_cols` class columns whose top-left sample sits at `origin`.
fn gradient_source_rows<'a>(
    source: &GdfSource<'a>,
    origin: (usize, usize),
    class_cols: usize,
    row_pair: usize,
) -> Option<[&'a [u16]; 4]> {
    let first_row = (origin.1 + 2 * row_pair).checked_sub(2)?;
    let first_col = origin.0.checked_sub(2)?;
    let row = |index: usize| {
        exact_slice(
            source.samples,
            (first_row + index) * source.stride + first_col,
            2 * class_cols + 4,
        )
    };
    Some([row(0)?, row(1)?, row(2)?, row(3)?])
}

/// The gradient-tap tables of [`classify_chunk`] for `block`'s GDF table.
fn class_tables(block: &GdfBlock) -> [[i16; 16]; 4] {
    let alpha = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx][GDF_COORDS.len()..];
    let weight = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx][2][GDF_COORDS.len()..];
    core::array::from_fn(|table| {
        core::array::from_fn(|lane| {
            let direction = (table & 2) + (lane & 1);
            if table & 1 == 0 {
                alpha[direction][0] as i16
            } else {
                weight[direction][0]
            }
        })
    })
}

/// Filters in place the run `block` of the stripe rows `output` (rows
/// `block.base_stride` apart, the run at column `block.x`) whose top-left
/// source sample sits at `origin`: per row pair it fills the next gradient
/// row pair, then classifies and filters each 16-column chunk in one pass.
#[inline(never)]
pub(super) fn filter_run<O: GdfSample>(
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    output: &mut [O],
    scratch: &mut ClassScratch,
) -> Option<()> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return filter_run_with(avx2, InLane, source, origin, block, output, scratch);
    }
    crate::baseline_simd!(simd => filter_run_with(
        simd, Widening, source, origin, block, output, scratch,
    ))
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn filter_run_with<S: Simd, L: Lanes<S>, O: GdfSample>(
    simd: S,
    lanes: L,
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    output: &mut [O],
    scratch: &mut ClassScratch,
) -> Option<()> {
    let (run, class_cols) = (block.width, block.width >> 1);
    if run == 0 || !run.is_multiple_of(2) || !block.height.is_multiple_of(2) {
        return None;
    }
    let chunks = ClassScratch::chunks(class_cols);
    let (first, second) = scratch.grads.get_mut(..2 * chunks)?.split_at_mut(chunks);
    let buffers = [first, second];
    let error_row = ClassScratch::error_row(run);
    let mut errors = scratch.errors.get_mut(..2 * error_row);
    let tables = class_tables(block);
    let params = GdfSymParams::new(block);
    let key = (
        block.ref_dst_idx,
        block.qp_idx,
        block.pix_scale,
        block.bit_depth,
    );
    if scratch.table_key != Some(key) {
        scratch.table.resize(1 << 16, 0);
        let (entries, _) = scratch.table.as_chunks_mut::<16>();
        for (entries, errors) in entries
            .iter_mut()
            .zip(error_table(block)?.as_chunks::<16>().0)
        {
            let (low, high) = simd.widen_i8x16(i8x16::from_slice(simd, errors));
            let (low, high) = simd.split_i16x16(residuals(simd.combine_i16x8(low, high), block));
            simd.saturating_narrow_i16x8(low, high).store_slice(entries);
        }
        scratch.table_key = Some(key);
    }
    let table = scratch.table.first_chunk()?;
    let grad_rows = |row_pair: usize| gradient_source_rows(source, origin, class_cols, row_pair);
    gradient_row(simd, &grad_rows(0)?, buffers[0])?;
    let mut chunk_classes = [[0; 8]; TILE_COLS / 16];
    let chunk_classes = chunk_classes.get_mut(..run.div_ceil(16))?;
    for row_pair in 0..block.height / 2 {
        let (current, previous) = ((row_pair + 1) & 1, row_pair & 1);
        gradient_row(simd, &grad_rows(row_pair + 1)?, buffers[current])?;
        let (current, previous) = (&*buffers[current], &*buffers[previous]);
        let window_rows = || {
            source_rows(
                source,
                (origin.0, origin.1 + 2 * row_pair),
                run + 2 * TAP_REACH,
            )
            .ok()
        };
        let tiled_base = (origin.1 + 2 * row_pair)
            .checked_sub(TAP_REACH)
            .zip(origin.0.checked_sub(TAP_REACH))
            .filter(|_| source.stride == TILE_STRIDE)
            .map(|(row, col)| row * TILE_STRIDE + col);
        let start = (2 * row_pair * block.base_stride).checked_add(block.x)?;
        let (top, bottom) = output
            .get_mut(start..)?
            .split_at_mut_checked(block.base_stride)?;
        let (top, bottom) = (top.get_mut(..run)?, bottom.get_mut(..run)?);
        let mut next = pair_sum(simd, current.first()?, previous.first()?);
        let grads = current.iter().zip(previous).skip(1);
        for (classes, (current, previous)) in chunk_classes.iter_mut().zip(grads) {
            let sums = next;
            next = pair_sum(simd, current, previous);
            *classes = classify_chunk(simd, sums, next, &tables).to_array();
        }
        let mut deferred = errors
            .as_deref_mut()
            .map(|errors| errors.split_at_mut(error_row));
        let mut done = 0;
        if let (Some(base), Some((top_errors, bottom_errors))) = (tiled_base, deferred.as_mut()) {
            let span = source.samples.get(base..).unwrap_or_default();
            let (top_errors, _) = top_errors.as_chunks_mut::<16>();
            let (bottom_errors, _) = bottom_errors.as_chunks_mut::<16>();
            let errors = top_errors.iter_mut().zip(bottom_errors);
            for (classes, (top, bottom)) in chunk_classes.iter().take(run / 16).zip(errors) {
                let Some(window) = span
                    .get(16 * done..)
                    .and_then(<[u16]>::first_chunk::<TILE_SPAN>)
                else {
                    break;
                };
                let (window, classes) = (TiledWindow(window), i32x8::from_slice(simd, classes));
                let sink = ErrorRows([top, bottom], table);
                sym_rows_with(simd, lanes, &window, sink, classes, block, &params);
                done += 1;
            }
        }
        for (x, classes) in (16 * done..run).step_by(16).zip(&chunk_classes[done..]) {
            let classes = i32x8::from_slice(simd, classes);
            let width = (run - x).min(16);
            let tiled = tiled_base
                .and_then(|base| source.samples.get(base + x..))
                .and_then(<[u16]>::first_chunk::<TILE_SPAN>);
            if width == 16
                && let Some(span) = tiled
            {
                let top = top.get_mut(x..)?.first_chunk_mut::<16>()?;
                let bottom = bottom.get_mut(x..)?.first_chunk_mut::<16>()?;
                let window = TiledWindow(span);
                sym_rows_with(simd, lanes, &window, [top, bottom], classes, block, &params);
            } else if width == 16
                && let Some(window) = window_at::<28>(&window_rows()?, x)
            {
                let top = top.get_mut(x..)?.first_chunk_mut::<16>()?;
                let bottom = bottom.get_mut(x..)?.first_chunk_mut::<16>()?;
                sym_rows_with(simd, lanes, &window, [top, bottom], classes, block, &params);
            } else {
                let classes = classes.to_array().map(GdfClass);
                let output = [&mut top[x..], &mut bottom[x..]];
                sym_rows_narrow(&window_rows()?, x, width, output, &classes, block, &params)?;
            }
        }
        if let Some((top_errors, bottom_errors)) = deferred {
            let end = 16 * done;
            add_residuals(simd, top_errors.get(..end)?, top.get_mut(..end)?, block);
            add_residuals(
                simd,
                bottom_errors.get(..end)?,
                bottom.get_mut(..end)?,
                block,
            );
        }
    }
    Some(())
}

/// The § 7.20.5 `gdfCls` and gradient bias of every 2x2 class block of
/// `block`, whose top-left sample sits at `origin` in `source`.
pub(super) fn band_classes(
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    classes: &mut [GdfClass],
    scratch: &mut ClassScratch,
) -> Option<()> {
    baseline_simd!(simd => band_classes_with(simd, source, origin, block, classes, scratch))
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn band_classes_with<S: Simd>(
    simd: S,
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    classes: &mut [GdfClass],
    scratch: &mut ClassScratch,
) -> Option<()> {
    let class_cols = block.width >> 1;
    let chunks = ClassScratch::chunks(class_cols);
    let (first, second) = scratch.grads.get_mut(..2 * chunks)?.split_at_mut(chunks);
    let buffers = [first, second];
    let rows = |row_pair: usize| gradient_source_rows(source, origin, class_cols, row_pair);
    let tables = class_tables(block);
    gradient_row(simd, &rows(0)?, buffers[0])?;
    for (row, classes) in classes.chunks_exact_mut(class_cols).enumerate() {
        let (current, previous) = ((row + 1) & 1, row & 1);
        gradient_row(simd, &rows(row + 1)?, buffers[current])?;
        let (current, previous) = (&*buffers[current], &*buffers[previous]);
        let mut next = pair_sum(simd, current.first()?, previous.first()?);
        let pairs = current.iter().zip(previous).skip(1);
        for (classes, (current, previous)) in classes.chunks_mut(8).zip(pairs) {
            let sums = next;
            next = pair_sum(simd, current, previous);
            let eight = classify_chunk(simd, sums, next, &tables)
                .to_array()
                .map(GdfClass);
            if let Ok(classes) = <&mut [GdfClass; 8]>::try_from(&mut *classes) {
                *classes = eight;
            } else {
                classes.copy_from_slice(eight.get(..classes.len())?);
            }
        }
    }
    Some(())
}
