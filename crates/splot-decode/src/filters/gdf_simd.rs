// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(target_arch = "x86_64")]
use fearless_simd::{Avx2, kernel};
use fearless_simd::{Level, Simd, dispatch, i16x16, i32x8, prelude::*, u16x16, u32x8};

use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};
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

/// Whether every sample that the row pair's taps read equals the center
/// sample, i.e. the window's `tap_range` is zero. The center rows go first:
/// in textured windows they alone usually prove the window uneven. The rows
/// are unrolled so that each row's tap reach is a constant.
#[inline]
fn window_is_flat<S: Simd>(simd: S, window: &Window<'_>) -> bool {
    let expected = u16x16::splat(simd, window[TAP_REACH][TAP_REACH]);
    let even = |row: &[u16; 28], col: usize| {
        u16x16::from_slice(simd, &row[col..col + 16])
            .simd_eq(expected)
            .all_true()
    };
    macro_rules! rows_even {
        ($($row:literal)*) => {
            $({
                const ROW: usize = $row;
                const REACH: usize = {
                    let (above, below) = (ROW.abs_diff(TAP_REACH), ROW.abs_diff(TAP_REACH + 1));
                    TAP_COLUMN_REACH[if above < below { above } else { below }]
                };
                even(window[ROW], TAP_REACH - REACH) && even(window[ROW], TAP_REACH + REACH)
            })&&*
        };
    }
    rows_even!(6 7 0 1 2 3 4 5 8 9 10 11 12 13)
}

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

/// The § 7.20.5 `err` of `block`'s table at each error index of `pos`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn errors(block: &GdfBlock, pos: &[u16; 16]) -> [i16; 16] {
    fn lookup<const LEN: usize>(table: &[i8; LEN], pos: &[u16; 16]) -> [i16; 16] {
        core::array::from_fn(|lane| i16::from(table[usize::from(pos[lane]) & (LEN - 1)]))
    }
    if block.ref_dst_idx == GDF_INTRA_REF_DST {
        lookup(&GDF_INTRA_ERROR_I8[block.qp_idx], pos)
    } else {
        lookup(
            &GDF_INTER_ERROR_I8[block.ref_dst_idx - 1][block.qp_idx],
            pos,
        )
    }
}

/// Class-0 clip bounds and weights of one GDF table, with its bias and the
/// § 7.20.5 `scale`.
///
/// The kernels read `center - tap`, the negated tap differences. The clip
/// bounds are symmetric, so every comb comes out negated, and the weighted
/// combs are subtracted from the bias.
pub(super) struct GdfSymParams {
    /// Each tap's clip bounds `[-alpha, alpha]` plus [`ABOVE_OFFSET`], then
    /// plus [`BELOW_OFFSET`], as whole vectors that the clamps read from memory.
    bounds: [[[u16; 16]; 4]; GDF_COORDS.len()],
    /// The weights of each `SYM_PAIRS` entry in alternating lanes.
    pair_weights: [[i16; 16]; SYM_PAIRS.len()],
    bias: [i32; 3],
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
        let flat_error = errors(block, &[flat_pos as u16; 16])[0];
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

/// How a kernel pairs comb lanes for its `i16` multiply-adds. The halves of
/// [`Lanes::pairs`] and of [`Lanes::spread`] hold the same samples, and
/// [`Lanes::digits`] reads its sums in that order.
trait Lanes<S: Simd>: Copy {
    /// `a` and `b` interleaved, in two halves.
    fn pairs(self, simd: S, a: i16x16<S>, b: i16x16<S>) -> [i16x16<S>; 2];
    /// Sums of the 8 two-sample class blocks, per sample in the order of
    /// [`Lanes::pairs`].
    fn spread(self, simd: S, blocks: i32x8<S>) -> [i32x8<S>; 2];
    /// The § 7.20.5 digit `Clip3(-scale, scale - 1, Round2Signed(sum * scale, 15))`
    /// of every sample, in sample order.
    fn digits(self, simd: S, sums: [i32x8<S>; 2], scale: i16) -> i16x16<S>;
}

/// Pairs by whole-vector interleaves: the first half holds samples `0..8`.
#[derive(Clone, Copy)]
struct Interleaved;

impl<S: Simd> Lanes<S> for Interleaved {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn pairs(self, simd: S, a: i16x16<S>, b: i16x16<S>) -> [i16x16<S>; 2] {
        let (low, high) = simd.interleave_i16x16(a, b);
        [low, high]
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
    /// The sums clamp to `[-32768, 0x7000]`, the pack's own saturation range
    /// with a positive cap past which every digit is `scale - 1`. `pmulhrsw` then
    /// rounds `|v| * scale` by 15 bits; -32768 gives `-scale`.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn packed_digits(t: Avx2, low: i32x8<Avx2>, high: i32x8<Avx2>, scale: i16) -> i16x16<Avx2> {
        use core::arch::x86_64::{
            __m256i, _mm256_max_epi32, _mm256_min_epi32, _mm256_mulhrs_epi16, _mm256_packs_epi32,
            _mm256_set1_epi16, _mm256_set1_epi32, _mm256_sign_epi16,
        };
        let clamp = |sums: __m256i| {
            _mm256_min_epi32(
                _mm256_max_epi32(sums, _mm256_set1_epi32(-32768)),
                _mm256_set1_epi32(0x7000),
            )
        };
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
fn tap_diffs<S: Simd, const TAP: usize, const ROW: usize>(
    simd: S,
    window: &Window<'_>,
    (above, below): Centers<S>,
) -> (u16x16<S>, u16x16<S>) {
    let (dy, dx) = GDF_COORDS[TAP];
    let row = TAP_REACH + ROW;
    (
        above
            - samples(
                simd,
                window[row - dy as usize],
                (TAP_REACH as isize - dx) as usize,
            ),
        below
            - samples(
                simd,
                window[row + dy as usize],
                (TAP_REACH as isize + dx) as usize,
            ),
    )
}

/// The negated comb, less [`COMB_BIAS`], of the offset negated reads
/// `above` and `below`, each within its offset tap bounds: the saturating
/// add clips the comb's upper end at 512 and the max its lower end at -511.
#[allow(clippy::inline_always)]
#[inline(always)]
fn clipped_sum<S: Simd>(
    simd: S,
    above: u16x16<S>,
    below: u16x16<S>,
    bounds: &[[u16; 16]; 4],
) -> i16x16<S> {
    let bound = |index: usize| u16x16::from_slice(simd, &bounds[index]);
    let above = above.max(bound(0)).min(bound(1));
    let below = below.max(bound(2)).min(bound(3));
    above
        .saturating_add(below)
        .max(u16x16::splat(simd, COMB_OFFSET - 511))
        .bitcast()
}

/// `a` in the lanes where `select` is clear, `b` where it is all ones.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pick<S: Simd>(a: u16x16<S>, b: u16x16<S>, select: u16x16<S>) -> u16x16<S> {
    a ^ ((a ^ b) & select)
}

/// The clipped sums of taps `K` and `T`, whose candidate samples coincide:
/// each lane of one reads the tap its class moves `K` to, so the two combs
/// select between the same two diff pairs. `select` is the class mask that
/// sends `K`'s lanes to `T` (`transposed`, or `mirrored` when `K`'s transpose
/// is itself and its mirror is `T`).
#[allow(clippy::inline_always)]
#[inline(always)]
fn paired_combs<S: Simd, const K: usize, const T: usize, const ROW: usize>(
    simd: S,
    window: &Window<'_>,
    centers: Centers<S>,
    select: u16x16<S>,
    params: &GdfSymParams,
) -> [i16x16<S>; 2] {
    let (above, below) = tap_diffs::<S, K, ROW>(simd, window, centers);
    let (t_above, t_below) = tap_diffs::<S, T, ROW>(simd, window, centers);
    [
        clipped_sum(
            simd,
            pick(above, t_above, select),
            pick(below, t_below, select),
            &params.bounds[K],
        ),
        clipped_sum(
            simd,
            pick(t_above, above, select),
            pick(t_below, below, select),
            &params.bounds[T],
        ),
    ]
}

/// The clipped sums of the tap cycle `A`, `B`, `C`, `D`, whose four combs
/// select among the same four diff pairs: mirroring swaps `A`/`B` and
/// `C`/`D`, transposing swaps the pairs.
#[allow(clippy::inline_always)]
#[inline(always)]
fn cycle_combs<
    S: Simd,
    const A: usize,
    const B: usize,
    const C: usize,
    const D: usize,
    const ROW: usize,
>(
    simd: S,
    window: &Window<'_>,
    centers: Centers<S>,
    (transposed, mirrored): (u16x16<S>, u16x16<S>),
    params: &GdfSymParams,
) -> [i16x16<S>; 4] {
    let (a_above, a_below) = tap_diffs::<S, A, ROW>(simd, window, centers);
    let (b_above, b_below) = tap_diffs::<S, B, ROW>(simd, window, centers);
    let (c_above, c_below) = tap_diffs::<S, C, ROW>(simd, window, centers);
    let (d_above, d_below) = tap_diffs::<S, D, ROW>(simd, window, centers);
    let select4 = |a: u16x16<S>, b: u16x16<S>, c: u16x16<S>, d: u16x16<S>| {
        let (ab, ba) = (pick(a, b, mirrored), pick(b, a, mirrored));
        let (cd, dc) = (pick(c, d, mirrored), pick(d, c, mirrored));
        [
            pick(ab, cd, transposed),
            pick(ba, dc, transposed),
            pick(cd, ab, transposed),
            pick(dc, ba, transposed),
        ]
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

/// Subtracts the negated combs `a`/`b` of `SYM_PAIRS` entry `P`, weighted,
/// from its sum.
#[allow(clippy::inline_always)]
#[inline(always)]
fn add_pair<S: Simd, L: Lanes<S>, const P: usize>(
    simd: S,
    lanes: L,
    sums: &mut Sums<S>,
    a: i16x16<S>,
    b: i16x16<S>,
    params: &GdfSymParams,
) {
    let weights = i16x16::from_slice(simd, &params.pair_weights[P]);
    for (sum, half) in sums[SYM_PAIRS[P].2].iter_mut().zip(lanes.pairs(simd, a, b)) {
        *sum -= simd.pairwise_dot_product_i16x16(half, weights);
    }
}

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
fn add_residual<S: Simd>(simd: S, pos: &[u16; 16], output: &mut [u16; 16], block: &GdfBlock) {
    let error = i16x16::from_slice(simd, &errors(block, pos));
    let scaled = error * block.pix_scale as i16;
    let rounding = 12 - u32::from(block.bit_depth.bits());
    let residual = if rounding == 0 {
        scaled
    } else {
        (scaled + (1 << (rounding - 1)) + (scaled >> 15)) >> rounding
    };
    let base = u16x16::from_slice(simd, output).bitcast::<i16x16<S>>();
    (base + residual)
        .max(i16x16::splat(simd, 0))
        .min(i16x16::splat(simd, block.max_sample as i16))
        .bitcast::<u16x16<S>>()
        .store_slice(output);
}

/// The error indices of row `ROW` of 16 samples of any class mix. Not inlined:
/// across both rows LLVM would hoist the weights out of the multiply-adds
/// and spill them. Taps are grouped so that each group's combs share their
/// candidate samples and selects: a tap and its transpose read mirrored
/// offsets, and taps `4`, `6`, `7`, `11` form a cycle.
#[inline(never)]
fn sym_row<S: Simd, L: Lanes<S>, const ROW: usize>(
    simd: S,
    lanes: L,
    window: &Window<'_>,
    masks: (u16x16<S>, u16x16<S>),
    init: &Sums<S>,
    params: &GdfSymParams,
) -> [u16; 16] {
    let center = samples(simd, window[TAP_REACH + ROW], TAP_REACH);
    let centers = (center + ABOVE_OFFSET, center + BELOW_OFFSET);
    let mut sums = *init;
    let sums = &mut sums;
    let (transposed, mirrored) = masks;
    let [tap0, tap12] = paired_combs::<S, 0, 12, ROW>(simd, window, centers, transposed, params);
    let [tap1, tap13] = paired_combs::<S, 1, 13, ROW>(simd, window, centers, transposed, params);
    add_pair::<S, L, 0>(simd, lanes, sums, tap0, tap1, params);
    add_pair::<S, L, 1>(simd, lanes, sums, tap12, tap13, params);
    let [tap2, tap14] = paired_combs::<S, 2, 14, ROW>(simd, window, centers, transposed, params);
    let [tap3, tap15] = paired_combs::<S, 3, 15, ROW>(simd, window, centers, transposed, params);
    add_pair::<S, L, 2>(simd, lanes, sums, tap2, tap3, params);
    add_pair::<S, L, 3>(simd, lanes, sums, tap14, tap15, params);
    add_pair::<S, L, 4>(simd, lanes, sums, tap3, tap15, params);
    let [tap4, tap6, tap7, tap11] =
        cycle_combs::<S, 4, 6, 7, 11, ROW>(simd, window, centers, masks, params);
    add_pair::<S, L, 5>(simd, lanes, sums, tap4, tap6, params);
    add_pair::<S, L, 6>(simd, lanes, sums, tap7, tap11, params);
    let [tap8, tap10] = paired_combs::<S, 8, 10, ROW>(simd, window, centers, mirrored, params);
    add_pair::<S, L, 7>(simd, lanes, sums, tap8, tap10, params);
    add_pair::<S, L, 8>(simd, lanes, sums, tap8, tap10, params);
    let [tap9, tap17] = paired_combs::<S, 9, 17, ROW>(simd, window, centers, transposed, params);
    add_pair::<S, L, 9>(simd, lanes, sums, tap9, tap17, params);
    add_pair::<S, L, 10>(simd, lanes, sums, tap9, tap17, params);
    add_pair::<S, L, 11>(simd, lanes, sums, tap9, tap17, params);
    let [tap5, tap16] = paired_combs::<S, 5, 16, ROW>(simd, window, centers, transposed, params);
    add_pair::<S, L, 12>(simd, lanes, sums, tap5, tap16, params);
    add_pair::<S, L, 13>(simd, lanes, sums, tap5, tap16, params);
    add_pair::<S, L, 14>(simd, lanes, sums, tap5, tap16, params);
    error_index(simd, lanes, *sums, params)
}

/// Filters in place a row pair of 16 samples whose class changes every two
/// samples, reading class-0 weights and moving each lane's taps by its
/// class. A window with `tap_range` 0 adds zero at every tap, so its sums
/// stay at the bias.
#[allow(clippy::inline_always)]
#[inline(always)]
fn sym_rows_with<S: Simd, L: Lanes<S>>(
    simd: S,
    lanes: L,
    window: &Window<'_>,
    [top, bottom]: [&mut [u16; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    let flat = window_is_flat(simd, window);
    if flat && params.keeps_flat && classes.iter().all(|class| class.gradient_bias() == 0) {
        return;
    }
    let classes = i32x8::from_slice(simd, &classes.map(|class| class.0));
    let biases = |bias: [i32; 3]| {
        [
            lanes.spread(simd, i32x8::splat(simd, bias[0])),
            lanes.spread(simd, i32x8::splat(simd, bias[1])),
            lanes.spread(simd, (classes >> 2) + bias[2]),
        ]
    };
    if flat {
        let pos = error_index(simd, lanes, biases(params.bias), params);
        add_residual(simd, &pos, top, block);
        add_residual(simd, &pos, bottom, block);
        return;
    }
    let init = biases(params.tap_bias);
    let class = classes & 3;
    let class = (class | (class << 16)).bitcast::<u16x16<S>>();
    let zero = u16x16::splat(simd, 0);
    let masks = (zero - (class & 1), zero - ((class >> 1) & 1));
    let pos = sym_row::<S, L, 0>(simd, lanes, window, masks, &init, params);
    add_residual(simd, &pos, top, block);
    let pos = sym_row::<S, L, 1>(simd, lanes, window, masks, &init, params);
    add_residual(simd, &pos, bottom, block);
}

/// [`sym_rows_with`] through the target's widest kernel.
#[inline(never)]
pub(super) fn sym_rows(
    window: &Window<'_>,
    output: [&mut [u16; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return sym_rows_with(avx2, InLane, window, output, classes, block, params);
    }
    sym_rows_interleaved(window, output, classes, block, params);
}

/// [`sym_rows_with`] through the portable kernel.
pub(super) fn sym_rows_interleaved(
    window: &Window<'_>,
    output: [&mut [u16; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    dispatch!(Level::baseline(), simd => sym_rows_with(
        simd,
        Interleaved,
        window,
        output,
        classes,
        block,
        params,
    ));
}

/// Filters in place `width` samples (at most 16) from column `x` of a row
/// pair through `sym_rows`, on copies padded to 16 lanes.
pub(super) fn sym_rows_narrow(
    rows: &[&[u16]; WINDOW_ROWS],
    x: usize,
    width: usize,
    [top, bottom]: [&mut [u16]; 2],
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
    let mut output = [[0; 16]; 2];
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

/// Scratch of [`band_classes`]: the direction strengths of the last class
/// row's gradient row pair, direction-major.
#[derive(Default)]
pub(super) struct ClassScratch {
    pub(super) strengths: Vec<u32>,
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

/// The sums of horizontally adjacent gradient pairs over the two gradient
/// rows between the four source rows, for the 16 gradient columns from
/// column 1 of each row.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pair_sums<S: Simd>(simd: S, rows: [&[u16; 18]; 4]) -> [u32x8<S>; 4] {
    let first = gradients(simd, [rows[0], rows[1], rows[2]]);
    let second = gradients(simd, [rows[1], rows[2], rows[3]]);
    core::array::from_fn(|d| simd.pairwise_widening_add_u16x16((first[d] + second[d]).bitcast()))
}

/// The pair sums of chunk `chunk` (gradient columns from `16 * chunk`) of
/// the four source rows; a chunk that reads past the rows reads zeros there.
#[allow(clippy::inline_always)]
#[inline(always)]
fn chunk_pair_sums<S: Simd>(simd: S, rows: &[&[u16]; 4], chunk: usize) -> Option<[u32x8<S>; 4]> {
    let start = 16 * chunk;
    let mut chunks = [&[0; 18]; 4];
    let mut padded = [[0; 18]; 4];
    if rows.iter().all(|row| row.len() >= start + 18) {
        for (chunk, row) in chunks.iter_mut().zip(rows) {
            *chunk = row.get(start..)?.first_chunk::<18>()?;
        }
        return Some(pair_sums(simd, chunks));
    }
    for (padded, row) in padded.iter_mut().zip(rows) {
        let tail = row.get(start..)?;
        padded.get_mut(..tail.len())?.copy_from_slice(tail);
    }
    Some(pair_sums(simd, padded.each_ref()))
}

/// The direction strengths of the `class_cols` class columns of one
/// gradient row pair from its four source rows: each sums the gradients of
/// four columns from `2j` over both rows. With `classes`, classifies the
/// columns from these and the previous row pair's strengths in `strengths`,
/// which then take the new ones.
fn class_row<S: Simd>(
    simd: S,
    rows: [&[u16]; 4],
    class_cols: usize,
    strengths: &mut [u32],
    mut classes: Option<&mut [GdfClass]>,
    tables: &[(i16, i16); 4],
) -> Option<()> {
    let stride = strengths.len() / 4;
    let pair_chunks = (class_cols + 1).div_ceil(8);
    let mut current = chunk_pair_sums(simd, &rows, 0)?;
    for chunk in 0..class_cols.div_ceil(8) {
        let next = if chunk + 1 < pair_chunks {
            chunk_pair_sums(simd, &rows, chunk + 1)?
        } else {
            [u32x8::splat(simd, 0); 4]
        };
        let sums: [u32x8<S>; 4] =
            core::array::from_fn(|d| current[d] + simd.slide_u32x8::<1>(current[d], next[d]));
        let mut previous = [u32x8::splat(simd, 0); 4];
        for (direction, (previous, sums)) in previous.iter_mut().zip(sums).enumerate() {
            let slot = strengths
                .get_mut(direction * stride + 8 * chunk..)?
                .first_chunk_mut::<8>()?;
            *previous = u32x8::from_slice(simd, slot);
            sums.store_array(slot);
        }
        if let Some(classes) = classes.as_deref_mut() {
            let classes = classes.get_mut(8 * chunk..)?;
            let count = classes.len().min(8);
            let eight = classify_eight(simd, previous, sums, tables);
            classes
                .get_mut(..count)?
                .copy_from_slice(eight.get(..count)?);
        }
        current = next;
    }
    Some(())
}

/// The classes and gradient biases of 8 class columns from their direction
/// strengths over two gradient row pairs; `tables` holds each direction's
/// clip bound and weight.
#[allow(clippy::inline_always)]
#[inline(always)]
fn classify_eight<S: Simd>(
    simd: S,
    previous: [u32x8<S>; 4],
    current: [u32x8<S>; 4],
    tables: &[(i16, i16); 4],
) -> [GdfClass; 8] {
    let strengths: [u32x8<S>; 4] = core::array::from_fn(|d| previous[d] + current[d]);
    let bit = |a: u32x8<S>, b: u32x8<S>, value: i32| {
        a.simd_le(b)
            .select(i32x8::splat(simd, value), i32x8::splat(simd, 0))
    };
    let index = bit(strengths[0], strengths[1], 1) | bit(strengths[2], strengths[3], 2);
    let mut bias = i32x8::splat(simd, 0);
    for (strength, &(alpha, weight)) in strengths.into_iter().zip(tables) {
        let comb = (strength >> 4)
            .min(u32x8::splat(simd, alpha as u32))
            .bitcast::<i32x8<S>>();
        bias += comb * i32::from(weight);
    }
    ((bias << 2) | index).to_array().map(GdfClass)
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
    dispatch!(Level::baseline(), simd => band_classes_with(simd, source, origin, block, classes, scratch))
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
    let strengths = scratch
        .strengths
        .get_mut(..4 * class_cols.div_ceil(8) * 8)?;
    let row_len = 2 * class_cols + 4;
    let first_col = origin.0.checked_sub(2)?;
    let rows = |row_pair: usize| -> Option<[&[u16]; 4]> {
        let first_row = (origin.1 + 2 * row_pair).checked_sub(2)?;
        let row = |index: usize| {
            exact_slice(
                source.samples,
                (first_row + index) * source.stride + first_col,
                row_len,
            )
        };
        Some([row(0)?, row(1)?, row(2)?, row(3)?])
    };
    let alpha_table = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx][GDF_COORDS.len()..];
    let weight_table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx][2][GDF_COORDS.len()..];
    let tables: [(i16, i16); 4] =
        core::array::from_fn(|d| (alpha_table[d][0] as i16, weight_table[d][0]));
    class_row(simd, rows(0)?, class_cols, strengths, None, &tables)?;
    for (row, classes) in classes.chunks_exact_mut(class_cols).enumerate() {
        class_row(
            simd,
            rows(row + 1)?,
            class_cols,
            strengths,
            Some(classes),
            &tables,
        )?;
    }
    Some(())
}
