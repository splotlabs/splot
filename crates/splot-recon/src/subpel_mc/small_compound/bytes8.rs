// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The 8-bit 8x8 compound cell, two rows per 16-lane vector.
//!
//! A [`Pair`] holds rows `2p` and `2p + 1`, one per 128-bit half, so every
//! slide, interleave and pack stays inside a half. The horizontal pass runs on
//! pairs of source rows; the vertical pass reads the pairs `k, k + 1` of
//! consecutive source rows and multiplies adjacent rows with one tap pair, so
//! the low half yields output row `i` and the high half row `i + 1` with the
//! same taps. 8-bit predictor rows stay below 9300 in magnitude, so they and
//! their blend fit `i16`. Clamped columns and wider samples take the generic
//! [`Cell8`] and repack its rows.

use super::*;
use std::simd::{Mask, Select, Swizzle, cmp::SimdPartialOrd};

type Pair = Simd<i16, 16>;
type Sums = Simd<i32, 8>;

pub(super) struct Bytes8<'a, T> {
    cell: Cell8<'a, T>,
    /// The tap pairs of `cell.v_taps` for [`paired_tap_sum`]-style sums.
    v_pairs: [Simd<i16, 16>; NUM_TAPS / 2],
    /// Clamped columns of a window that starts inside its row.
    edges: Option<Edges>,
}

/// A window whose columns `first + i` are clamped to `[left, right]`: it
/// loads from `first` and replaces the lanes past either bound with that
/// bound's sample, which is the `Clip3` read.
#[derive(Clone, Copy)]
struct Edges {
    first: usize,
    left: usize,
    right: usize,
    before: Mask<i8, 16>,
    after: Mask<i8, 16>,
}

impl Edges {
    fn new(params: &SubpelPredictParams) -> Option<Self> {
        let horizontal = (params.start_x >> 6) & SUBPEL_MASK != 0;
        let first = (params.start_x >> SCALE_SUBPEL_BITS) - if horizontal { 3 } else { 0 };
        let columns = Simd::splat(first) + Simd::from_array(LANE_INDEX);
        Some(Self {
            first: usize::try_from(first).ok()?,
            left: usize::try_from(params.first_x).ok()?,
            right: usize::try_from(params.last_x).ok()?,
            before: columns.simd_lt(Simd::splat(params.first_x)).cast(),
            after: columns.simd_gt(Simd::splat(params.last_x)).cast(),
        })
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn window(&self, row: &[u8]) -> Option<Simd<u8, 16>> {
        let lanes = Simd::from_array(*row.get(self.first..)?.first_chunk::<16>()?);
        let (left, right) = (*row.get(self.left)?, *row.get(self.right)?);
        Some(self.after.select(
            Simd::splat(right),
            self.before.select(Simd::splat(left), lanes),
        ))
    }
}

/// Lane `i` of each half reads lane `i + K` of the half's concatenated
/// `(low, high)`: one in-lane `palignr`.
struct Slide<const K: usize>;

impl<const K: usize> Swizzle<16> for Slide<K> {
    const INDEX: [usize; 16] = {
        let mut index = [0; 16];
        let mut lane = 0;
        while lane < 16 {
            let (half, i) = (lane / 8, lane % 8 + K);
            index[lane] = if i < 8 {
                half * 8 + i
            } else {
                16 + half * 8 + i - 8
            };
            lane += 1;
        }
        index
    };
}

/// In-lane interleave of the low (`HIGH == 0`) or high four lanes of each
/// half of two vectors: `punpcklwd` / `punpckhwd`.
struct Interleave<const HIGH: usize>;

impl<const HIGH: usize> Swizzle<16> for Interleave<HIGH> {
    const INDEX: [usize; 16] = {
        let mut index = [0; 16];
        let mut lane = 0;
        while lane < 16 {
            let source = (lane / 8) * 8 + HIGH * 4 + (lane % 8) / 2;
            index[lane] = source + (lane % 2) * 16;
            lane += 1;
        }
        index
    };
}

/// `packssdw`: each half's four `low` lanes, then its four `high` lanes.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn pack(low: Sums, high: Sums) -> Pair {
    let lanes: Simd<i32, 16> = simd_swizzle!(
        low,
        high,
        [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
    );
    lanes
        .simd_max(Simd::splat(i32::from(i16::MIN)))
        .simd_min(Simd::splat(i32::from(i16::MAX)))
        .cast()
}

/// `Round2(Σ weights * (a, b), shift)` of interleaved lanes, packed back to
/// the lane order of `a`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn weigh<const SHIFT: u32>(pairs: &[(Pair, Pair, Pair)]) -> Pair {
    let bias = Sums::splat(1 << (SHIFT - 1));
    let (mut low, mut high) = (bias, bias);
    for &(a, b, weights) in pairs {
        low += multiply_add_pairs(Interleave::<0>::concat_swizzle(a, b), weights);
        high += multiply_add_pairs(Interleave::<1>::concat_swizzle(a, b), weights);
    }
    pack(low >> SHIFT as i32, high >> SHIFT as i32)
}

/// The two 16-byte source windows as one vector, one row per half.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn join(first: Simd<u8, 16>, second: Simd<u8, 16>) -> Simd<u8, 32> {
    simd_swizzle!(
        first,
        second,
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31
        ]
    )
}

/// Each half's samples `0..8` and `8..16` as `i16` lanes.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn widen(bytes: Simd<u8, 32>) -> (Pair, Pair) {
    let zero = Simd::splat(0);
    let (bytes, zero) = if cfg!(target_endian = "big") {
        (zero, bytes)
    } else {
        (bytes, zero)
    };
    let low: Simd<u8, 32> = simd_swizzle!(
        bytes,
        zero,
        [
            0, 32, 1, 33, 2, 34, 3, 35, 4, 36, 5, 37, 6, 38, 7, 39, 16, 48, 17, 49, 18, 50, 19, 51,
            20, 52, 21, 53, 22, 54, 23, 55
        ]
    );
    let high: Simd<u8, 32> = simd_swizzle!(
        bytes,
        zero,
        [
            8, 40, 9, 41, 10, 42, 11, 43, 12, 44, 13, 45, 14, 46, 15, 47, 24, 56, 25, 57, 26, 58,
            27, 59, 28, 60, 29, 61, 30, 62, 31, 63
        ]
    );
    let lanes = |bytes| Simd::<u16, 16>::from_ne_bytes(bytes).cast::<i16>();
    (lanes(low), lanes(high))
}

/// The halved-tap horizontal pass of a pair of rows, as in [`eight_bit_row`].
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn horizontal(bytes: Simd<u8, 32>, taps: Simd<i16, NUM_TAPS>) -> Pair {
    let (low, high) = widen(bytes);
    macro_rules! tap {
        ($k:literal) => {
            Slide::<$k>::concat_swizzle(low, high) * Simd::splat(taps[$k])
        };
    }
    let half = Pair::splat(1 << (INTER_ROUND0 - 2))
        + low * Simd::splat(taps[0])
        + tap!(1)
        + tap!(2)
        + tap!(3)
        + tap!(4)
        + tap!(5)
        + tap!(6)
        + tap!(7);
    half >> (INTER_ROUND0 - 1) as i16
}

impl<T: ReconSample> Bytes8<'_, T> {
    /// The 16-byte windows of source rows `0..ROWS` from `cell.top`, or
    /// `None` when the samples are wider than 8 bits or a column is clamped.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn windows<const ROWS: usize>(&self) -> Option<[Simd<u8, 16>; ROWS]> {
        let samples = T::u8_slice(self.cell.samples)?;
        match (self.cell.gather, self.edges) {
            (None, _) => self.rows::<ROWS, false>(samples),
            (Some(_), Some(_)) => self.rows::<ROWS, true>(samples),
            (Some(_), None) => None,
        }
    }

    /// Source row `row`'s window, every row clamped to `[firstY, lastY]`,
    /// which is the identity for an unclamped cell.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn rows<const ROWS: usize, const EDGES: bool>(
        &self,
        samples: &[u8],
    ) -> Option<[Simd<u8, 16>; ROWS]> {
        let cell = &self.cell;
        let edges = if EDGES { self.edges } else { None };
        let mut windows = [Simd::splat(0); ROWS];
        for (row, window) in windows.iter_mut().enumerate() {
            let y = (cell.top + row as i32).max(cell.first_y).min(cell.last_y) as usize;
            let row = samples.get(y * cell.stride..)?;
            *window = match edges {
                Some(edges) => edges.window(row)?,
                None => Simd::from_array(*row.get(cell.column..)?.first_chunk()?),
            };
        }
        Some(windows)
    }

    /// Pair `k` of the vertical pass: source rows `k` and `k + 1`.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn vertical<const TAPS: usize, const SHIFT: u32>(&self, lines: &[Pair]) -> [Pair; 4] {
        let mut out = [Pair::splat(0); 4];
        for (p, out) in out.iter_mut().enumerate() {
            let mut pairs = [(Pair::splat(0), Pair::splat(0), Pair::splat(0)); NUM_TAPS / 2];
            for (j, pair) in pairs.iter_mut().enumerate().take(TAPS / 2) {
                let k = (2 * p + 2 * j).min(lines.len() - 2);
                *pair = (lines[k], lines[k + 1], self.v_pairs[j]);
            }
            *out = weigh::<SHIFT>(&pairs[..TAPS / 2]);
        }
        out
    }

    /// The overlapping pairs `k, k + 1` of the `2 * PAIRS` source rows whose
    /// pairs `2m, 2m + 1` are `rows[m]`.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn overlap<const PAIRS: usize>(rows: &[Pair; PAIRS]) -> [Pair; 2 * NUM_TAPS] {
        let mut lines = [Pair::splat(0); 2 * NUM_TAPS];
        for (m, pair) in rows.iter().enumerate() {
            lines[2 * m] = *pair;
            if let Some(next) = rows.get(m + 1) {
                lines[2 * m + 1] = simd_swizzle!(
                    *pair,
                    *next,
                    [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23]
                );
            }
        }
        lines
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn two_pass<const TAPS: usize, const PAIRS: usize, const ROWS: usize>(
        &self,
    ) -> Option<[Pair; 4]> {
        if self.cell.horizontal {
            self.two_pass_with::<TAPS, PAIRS, ROWS, true>()
        } else {
            self.two_pass_with::<TAPS, PAIRS, ROWS, false>()
        }
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn two_pass_with<const TAPS: usize, const PAIRS: usize, const ROWS: usize, const H: bool>(
        &self,
    ) -> Option<[Pair; 4]> {
        let windows = self.windows::<ROWS>()?;
        let taps = self.cell.h_taps;
        let mut rows = [Pair::splat(0); PAIRS];
        for (m, pair) in rows.iter_mut().enumerate() {
            let (first, second) = (
                windows[(2 * m).min(ROWS - 1)],
                windows[(2 * m + 1).min(ROWS - 1)],
            );
            let bytes = join(first, second);
            *pair = if H {
                horizontal(bytes, taps)
            } else {
                widen(bytes).0
            };
        }
        let lines = Self::overlap(&rows);
        let lines = &lines[..2 * PAIRS - 1];
        Some(if H {
            self.vertical::<TAPS, INTER_ROUND1_COMPOUND>(lines)
        } else {
            self.vertical::<TAPS, INTER_ROUND0>(lines)
        })
    }

    /// The fast 8-bit rows, `None` when [`Self::windows`] declines.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn bytes(&self) -> Option<[Pair; 4]> {
        let cell = &self.cell;
        if !cell.vertical {
            let windows = self.windows::<8>()?;
            let mut out = [Pair::splat(0); 4];
            for (p, out) in out.iter_mut().enumerate() {
                let bytes = join(windows[2 * p], windows[2 * p + 1]);
                *out = if cell.horizontal {
                    horizontal(bytes, cell.h_taps)
                } else {
                    widen(bytes).0 << (FILTER_BITS - INTER_ROUND0) as i16
                };
            }
            return Some(out);
        }
        match cell.v_taps.len() {
            2 => self.two_pass::<2, 5, 9>(),
            4 => self.two_pass::<4, 6, 11>(),
            6 => self.two_pass::<6, 7, 13>(),
            _ => self.two_pass::<NUM_TAPS, 8, 15>(),
        }
    }
}

impl<'a, T: ReconSample> CellReference<'a, T> for Bytes8<'a, T> {
    type Rows = [Pair; 4];

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn new(reference: &ReferencePlaneView<'a, T>, params: &SubpelPredictParams) -> Option<Self> {
        let cell = Cell8::new(reference, params)?;
        let v_pairs = paired_taps(cell.v_taps);
        let edges = cell.gather.and_then(|_| Edges::new(params));
        Some(Self {
            cell,
            v_pairs,
            edges,
        })
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn rebind(&self, reference: &ReferencePlaneView<'a, T>) -> Self {
        Self {
            cell: self.cell.rebind(reference),
            v_pairs: self.v_pairs,
            edges: self.edges,
        }
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn predict(&self) -> Option<[Pair; 4]> {
        if let Some(rows) = self.bytes() {
            return Some(rows);
        }
        let rows = self.cell.predict()?;
        let mut out = [Pair::splat(0); 4];
        for (p, out) in out.iter_mut().enumerate() {
            let pair: Simd<i32, 16> = simd_swizzle!(
                rows[2 * p],
                rows[2 * p + 1],
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
            );
            *out = pair.cast();
        }
        Some(out)
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn store<O: ReconSample>(
        cells: [&Self; 2],
        params: &SubpelPredictParams,
        cwp_weight: i16,
        output: &mut [O],
        output_stride: usize,
    ) -> bool {
        store_pairs(cells, params, cwp_weight, output, output_stride)
    }
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn store_pairs<T: ReconSample, O: ReconSample>(
    cells: [&Bytes8<'_, T>; 2],
    params: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [O],
    output_stride: usize,
) -> bool {
    let (Some(pred0), Some(pred1)) = (cells[0].predict(), cells[1].predict()) else {
        return false;
    };
    let weights = Pair::from_array(core::array::from_fn(|lane| {
        if lane % 2 == 0 {
            cwp_weight
        } else {
            16 - cwp_weight
        }
    }));
    let max_sample = Pair::splat(params.bit_depth.max_sample().min(O::MAX_VALUE) as i16);
    let blend = |p: usize| {
        weigh::<{ 4 + compound_inter_post_round() }>(&[(pred0[p], pred1[p], weights)])
            .simd_max(Simd::splat(0))
            .simd_min(max_sample)
    };
    if let Some(output) = O::u8_slice_mut(output) {
        for p in 0..4 {
            let lanes = blend(p).cast::<u8>().to_array();
            for (half, row) in lanes.chunks_exact(8).enumerate() {
                output[(2 * p + half) * output_stride..][..8].copy_from_slice(row); // splot-copy-ok: publish blended SIMD prediction lanes
            }
        }
        return true;
    }
    if let Some(output) = O::u16_slice_mut(output) {
        for p in 0..4 {
            let lanes = blend(p).cast::<u16>().to_array();
            for (half, row) in lanes.chunks_exact(8).enumerate() {
                output[(2 * p + half) * output_stride..][..8].copy_from_slice(row); // splot-copy-ok: publish blended SIMD prediction lanes
            }
        }
        return true;
    }
    false
}
