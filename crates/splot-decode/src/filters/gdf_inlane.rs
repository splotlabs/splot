// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The x86 AVX2 GDF row-pair kernel: class-0 slots, whose lanes swap their
//! taps by class, and `pmaddwd` on slot pairs. Kept until a measurement on
//! x86 shows [`super::rough_half`] at least as fast.

use fearless_simd::{Avx2, Simd, i16x16, i32x8, kernel, prelude::*, u16x16};

use splot_core::tables::loop_restoration::{GDF_ALPHA, GDF_WEIGHT};

use super::{
    GDF_BIAS, GDF_COORDS, GdfBlock, GdfSymParams, LINEAR_RANGE, ROUGH, RowPairs, RowSink,
    TAP_REACH, WindowRows, window_range,
};

/// The class-0 clip bounds and weights of one GDF table for [`InLane`].
pub(super) struct SlotParams {
    /// Each tap's clip bounds `[-alpha, alpha]` plus [`ABOVE_OFFSET`], then
    /// plus [`BELOW_OFFSET`], as whole vectors that the clamps read from memory.
    bounds: [[[u16; 16]; 4]; GDF_COORDS.len()],
    /// The widest `tap_range` at which every comb skips the clips: the
    /// smallest clip bound, at most [`LINEAR_RANGE`].
    linear_bound: u16,
    /// The weights of each `SYM_PAIRS` entry in alternating lanes.
    pair_weights: [[i16; 16]; SYM_PAIRS.len()],
    /// The bias less [`COMB_BIAS`] times the weights of each sum, which the
    /// biased combs add back.
    tap_bias: [i32; 3],
}

impl SlotParams {
    pub(super) fn new(block: &GdfBlock) -> Self {
        let alpha = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
        let table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
        let bias = GDF_BIAS[block.ref_dst_idx][block.qp_idx];
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
            linear_bound: alpha[..GDF_COORDS.len()]
                .iter()
                .fold(LINEAR_RANGE, |bound, alpha| bound.min(alpha[0])),
            pair_weights: SYM_PAIRS.map(|(a, b, sum)| {
                core::array::from_fn(|lane| table[sum][if lane & 1 == 0 { a } else { b }][0])
            }),
            tap_bias: core::array::from_fn(|sum| {
                let weights: i32 = SYM_PAIRS
                    .iter()
                    .filter(|pair| pair.2 == sum)
                    .map(|&(a, b, _)| i32::from(table[sum][a][0]) + i32::from(table[sum][b][0]))
                    .sum();
                bias[sum] - COMB_BIAS * weights
            }),
        }
    }
}

impl RowPairs<Avx2> for InLane {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn rows<W: WindowRows, R: RowSink>(
        self,
        simd: Avx2,
        window: &W,
        sink: R,
        classes: i32x8<Avx2>,
        (block, params): (&GdfBlock, &GdfSymParams),
    ) {
        slot_rows_with(simd, self, window, sink, classes, block, params);
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

/// The distinct tap pairs of `SYM_PAIRS`, in the order [`row_index`] interleaves them.
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

/// How a kernel pairs comb lanes for its `i16` multiply-adds. The sums of
/// [`Lanes::accumulate`] and the halves of [`Lanes::spread`] hold the same
/// samples, and [`Lanes::digits`] reads its sums in that order.
trait Lanes<S: Simd>: Copy {
    /// The combs `a` and `b` as [`Lanes::accumulate`] reads them.
    fn pairs(self, simd: S, a: i16x16<S>, b: i16x16<S>) -> [i16x16<S>; 2];
    /// The factor of the sums that [`Lanes::accumulate`] and
    /// [`Lanes::digits`] use.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn sum_scale(self, _params: &GdfSymParams) -> i32 {
        1
    }
    /// Subtracts from `sums` the combs `pair` of `COMB_PAIRS` slot `slot`
    /// times their weights in each `SYM_PAIRS` entry of the slot.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn accumulate(
        self,
        simd: S,
        sums: &mut Sums<S>,
        (slot, pair): (usize, [i16x16<S>; 2]),
        params: &GdfSymParams,
    ) {
        macro_rules! add_pairs {
            ($($pair:literal)*) => {$(if PAIR_SLOT[$pair] == slot {
                const PAIR: usize = $pair;
                let weights = i16x16::from_slice(simd, &params.slot.pair_weights[PAIR]);
                let sum = &mut sums[SYM_PAIRS[PAIR].2];
                for (sum, half) in sum.iter_mut().zip(pair) {
                    *sum -= simd.pairwise_dot_product_i16x16(half, weights);
                }
            })*};
        }
        add_pairs!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14);
    }
    /// Sums of the 8 two-sample class blocks, per sample in the order of
    /// the sums of [`Lanes::accumulate`].
    fn spread(self, simd: S, blocks: i32x8<S>) -> [i32x8<S>; 2];
    /// The § 7.20.5 digit `Clip3(-scale, scale - 1, Round2Signed(sum * scale, 15))`
    /// of every sample, in sample order, from the sums times
    /// [`Lanes::sum_scale`].
    fn digits(self, simd: S, sums: [i32x8<S>; 2], scale: i16) -> i16x16<S>;
    fn range(self, simd: S, low: u16x16<S>, high: u16x16<S>) -> u16;
}

/// Pairs by AVX2 in-lane unpacks: the first half holds samples `0..4` and
/// `8..12`, which an in-lane pack of the two halves puts back in order.
#[derive(Clone, Copy)]
pub(super) struct InLane;

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

kernel!(
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn lane_range(_t: Avx2, low: u16x16<Avx2>, high: u16x16<Avx2>) -> u16 {
        use core::arch::x86_64::{
            _mm_cvtsi128_si32, _mm_max_epu16, _mm_min_epu16, _mm_minpos_epu16, _mm_set1_epi8,
            _mm_xor_si128, _mm256_castsi256_si128, _mm256_extracti128_si256,
        };
        let (low, high) = (low.into(), high.into());
        let low = _mm_min_epu16(
            _mm256_castsi256_si128(low),
            _mm256_extracti128_si256::<1>(low),
        );
        let high = _mm_max_epu16(
            _mm256_castsi256_si128(high),
            _mm256_extracti128_si256::<1>(high),
        );
        let inverted_max = _mm_minpos_epu16(_mm_xor_si128(high, _mm_set1_epi8(-1)));
        let min = _mm_cvtsi128_si32(_mm_minpos_epu16(low)) as u16;
        !(_mm_cvtsi128_si32(inverted_max) as u16) - min
    }
);

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

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn range(self, simd: Avx2, low: u16x16<Avx2>, high: u16x16<Avx2>) -> u16 {
        lane_range(simd, low, high)
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
/// `above` and `below`, each clipped to its offset tap bounds: the
/// saturating add clips the comb's upper end at 512 and the max its lower
/// end at -511.
#[allow(clippy::inline_always)]
#[inline(always)]
fn clipped_sum<S: Simd>(
    simd: S,
    above: u16x16<S>,
    below: u16x16<S>,
    bounds: &[[u16; 16]; 4],
) -> i16x16<S> {
    let bound = |index: usize| u16x16::from_slice(simd, &bounds[index]);
    let (above, below) = (
        above.max(bound(0)).min(bound(1)),
        below.max(bound(2)).min(bound(3)),
    );
    above
        .saturating_add(below)
        .max(u16x16::splat(simd, COMB_OFFSET - 511))
        .bitcast()
}

/// `a` and `b` with the lanes where `select` (all ones or zero per lane) is
/// set exchanged: two xors of one masked difference, four operations where
/// two AVX2 blends are six micro-ops.
#[allow(clippy::inline_always)]
#[inline(always)]
fn swap<S: Simd>(a: u16x16<S>, b: u16x16<S>, select: u16x16<S>) -> (u16x16<S>, u16x16<S>) {
    let moved = (a ^ b) & select;
    (a ^ moved, b ^ moved)
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
        clipped_sum(simd, above, below, &params.slot.bounds[K]),
        clipped_sum(simd, t_above, t_below, &params.slot.bounds[T]),
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
        clipped_sum(simd, above[0], below[0], &params.slot.bounds[A]),
        clipped_sum(simd, above[1], below[1], &params.slot.bounds[B]),
        clipped_sum(simd, above[2], below[2], &params.slot.bounds[C]),
        clipped_sum(simd, above[3], below[3], &params.slot.bounds[D]),
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

/// The § 7.20.5 error indices of row `ROW` of 16 samples of any class mix:
/// each `COMB_PAIRS` slot's negated combs, paired by [`Lanes::pairs`], go
/// into the sums from `init` as soon as they exist. Taps are grouped so that
/// each group's combs share their candidate samples and selects: a tap and
/// its transpose read mirrored offsets, and taps `4`, `6`, `7`, `11` form a
/// cycle. With `LINEAR` every comb skips the clips: the swaps then act on
/// the combs instead of each diff. Out of line, so that the rows share no
/// loads that would spill.
#[inline(never)]
fn row_index<S: Simd, L: Lanes<S>, W: WindowRows, const ROW: usize, const LINEAR: bool>(
    simd: S,
    lanes: L,
    window: &W,
    masks: Masks<S>,
    init: &Sums<S>,
    params: &GdfSymParams,
) -> [u16; 16] {
    let center = samples(simd, window.row(TAP_REACH + ROW), TAP_REACH);
    let centers = (center + ABOVE_OFFSET, center + BELOW_OFFSET);
    let center2 = centers.0 + centers.1;
    let (transposed, mirrored) = masks;
    macro_rules! pair {
        ($k:literal, $t:literal, $select:expr) => {
            if LINEAR {
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
    let mut sums = *init;
    let mut add = |slot: usize, a: i16x16<S>, b: i16x16<S>| {
        lanes.accumulate(simd, &mut sums, (slot, lanes.pairs(simd, a, b)), params);
    };
    let [tap0, tap12] = pair!(0, 12, transposed);
    let [tap1, tap13] = pair!(1, 13, transposed);
    add(0, tap0, tap1);
    add(1, tap12, tap13);
    let [tap2, tap14] = pair!(2, 14, transposed);
    let [tap3, tap15] = pair!(3, 15, transposed);
    add(2, tap2, tap3);
    add(3, tap14, tap15);
    add(4, tap3, tap15);
    let [tap4, tap6, tap7, tap11] = if LINEAR {
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
    add(5, tap4, tap6);
    add(6, tap7, tap11);
    let [tap8, tap10] = pair!(8, 10, mirrored);
    add(7, tap8, tap10);
    let [tap9, tap17] = pair!(9, 17, transposed);
    add(8, tap9, tap17);
    let [tap5, tap16] = pair!(5, 16, transposed);
    add(9, tap5, tap16);
    error_index(simd, lanes, sums, params)
}

/// Filters in place a row pair of 16 samples whose class changes every two
/// samples, reading class-0 weights and moving each lane's taps by its
/// class. A window with `tap_range` 0 adds zero at every tap, so its sums
/// stay at the bias.
#[allow(clippy::inline_always)]
#[inline(always)]
fn slot_rows_with<S: Simd, L: Lanes<S>, W: WindowRows, R: RowSink>(
    simd: S,
    lanes: L,
    window: &W,
    mut sink: R,
    classes: i32x8<S>,
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    let range = window_range(simd, window, params.slot.linear_bound, |low, high| {
        lanes.range(simd, low, high)
    });
    let flat = range == 0;
    if flat && params.keeps_flat && (classes >> 2).simd_eq(i32x8::splat(simd, 0)).all_true() {
        sink.keep(simd);
        return;
    }
    let factor = lanes.sum_scale(params);
    let bias = if flat {
        &params.bias
    } else {
        &params.slot.tap_bias
    };
    let init = [
        lanes.spread(simd, i32x8::splat(simd, bias[0] * factor)),
        lanes.spread(simd, i32x8::splat(simd, bias[1] * factor)),
        lanes.spread(simd, ((classes >> 2) + bias[2]) * factor),
    ];
    if flat {
        let pos = error_index(simd, lanes, init, params);
        sink.row(simd, 0, &pos, block);
        sink.row(simd, 1, &pos, block);
        return;
    }
    let class = classes & 3;
    let class = (class | (class << 16)).bitcast::<u16x16<S>>();
    let zero = u16x16::splat(simd, 0);
    let masks = (zero - (class & 1), zero - ((class >> 1) & 1));
    let rows = if range != ROUGH {
        [row_index::<S, L, W, 0, true>, row_index::<S, L, W, 1, true>]
    } else {
        [
            row_index::<S, L, W, 0, false>,
            row_index::<S, L, W, 1, false>,
        ]
    };
    for (row, row_index) in rows.into_iter().enumerate() {
        let pos = row_index(simd, lanes, window, masks, &init, params);
        sink.row(simd, row, &pos, block);
    }
}

#[cfg(test)]
#[test]
fn lane_range_matches_lane_extremes() {
    let Some(simd) = fearless_simd::Level::baseline().as_avx2() else {
        return;
    };
    let mut seed = 0x9e37_79b9_u32;
    for case in 0..256 {
        let mut sample = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            match (seed >> 8) % 8 {
                0 if case % 2 == 0 => 0,
                1 if case % 3 == 0 => u16::MAX,
                _ => (seed >> 16) as u16,
            }
        };
        let pairs: [(u16, u16); 16] = core::array::from_fn(|_| {
            let (a, b) = (sample(), sample());
            (a.min(b), a.max(b))
        });
        let (low, high) = (pairs.map(|pair| pair.0), pairs.map(|pair| pair.1));
        let expected =
            high.iter().max().copied().unwrap_or(0) - low.iter().min().copied().unwrap_or(0);
        let lanes = (
            u16x16::from_slice(simd, &low),
            u16x16::from_slice(simd, &high),
        );
        assert_eq!(lane_range(simd, lanes.0, lanes.1), expected, "case {case}");
    }
}
