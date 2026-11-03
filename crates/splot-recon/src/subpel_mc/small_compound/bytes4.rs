// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The 8-bit 4x4 compound cell, four rows per 16-lane vector.
//!
//! Each 128-bit half holds the 8-byte windows of two rows, so one byte
//! shuffle per tap widens every row's tap window at once. Four consecutive
//! horizontal rows form a vector, and the vectors starting at rows `k` and
//! `k + 1` interleave into the tap pairs of output rows `0, 2` (low lanes)
//! and `1, 3` (high lanes), the [`weigh`] layout of a [`Pair`].

use super::super::pairs::{Pair, join, weigh};
use super::bytes8::store_blended;
use super::*;
use std::simd::Swizzle;

/// Taps `2, 3` and `4, 5` of every `Subpel_Filters` row, each pair repeated
/// across 8 lanes; a table keeps the weights opaque to the multiply-add.
const SMALL_PAIRS: [[[i16; 8]; 2]; NUM_PHASES * NUM_FILTER_TYPES] = {
    let mut pairs = [[[0; 8]; 2]; NUM_PHASES * NUM_FILTER_TYPES];
    let mut row = 0;
    while row < pairs.len() {
        let taps = &SUBPEL_FILTERS[row / NUM_PHASES][row % NUM_PHASES];
        let mut lane = 0;
        while lane < 16 {
            pairs[row][lane / 8][lane % 8] = taps[2 + 2 * (lane / 8) + lane % 2] as i16;
            lane += 1;
        }
        row += 1;
    }
    pairs
};

/// Lane `4r + c` of each half widens byte `8r + c + K` of the half: tap `K`
/// of output column `c` for the half's two rows.
struct Window<const K: usize>;

impl<const K: usize> Swizzle<32> for Window<K> {
    const INDEX: [usize; 32] = {
        let mut index = [0; 32];
        let mut byte = 0;
        while byte < 32 {
            let lane = byte / 2;
            let source = (lane / 8) * 16 + (lane % 8) / 4 * 8 + lane % 4 + K;
            let low = (byte % 2 == 0) == cfg!(target_endian = "little");
            index[byte] = if low { source } else { 32 };
            byte += 1;
        }
        index
    };
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn window<const K: usize>(bytes: Simd<u8, 32>) -> Pair {
    let lanes = Window::<K>::concat_swizzle(bytes, Simd::splat(0));
    Simd::<u16, 16>::from_ne_bytes(lanes).cast()
}

/// The 8-byte windows of four rows, two per half.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn join4(rows: [Simd<u8, 8>; 4]) -> Simd<u8, 32> {
    let low = simd_swizzle!(
        rows[0],
        rows[1],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    let high = simd_swizzle!(
        rows[2],
        rows[3],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
    join(low, high)
}

/// The halved-tap horizontal pass of four rows, as in `horizontal`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn horizontal4(bytes: Simd<u8, 32>, taps: Taps) -> Pair {
    let half = Pair::splat(1 << (INTER_ROUND0 - 2))
        + window::<0>(bytes) * Simd::splat(taps[0])
        + window::<1>(bytes) * Simd::splat(taps[1])
        + window::<2>(bytes) * Simd::splat(taps[2])
        + window::<3>(bytes) * Simd::splat(taps[3]);
    half >> (INTER_ROUND0 - 1) as i16
}

pub(super) struct Bytes4<'a, T> {
    cell: Cell<'a, T>,
    v_pairs: [Pair; 2],
}

impl<T: ReconSample> Bytes4<'_, T> {
    /// Source rows `FIRST..FIRST + N` from `cell.top`, clamped to
    /// `[firstY, lastY]`; `None` for wider samples or a window that leaves
    /// the plane storage.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn windows<const FIRST: usize, const N: usize>(&self) -> Option<[Simd<u8, 8>; N]> {
        let cell = &self.cell;
        let samples = T::u8_slice(cell.samples)?;
        let mut windows = [Simd::splat(0); N];
        for (row, window) in windows.iter_mut().enumerate() {
            let y = (cell.top + (FIRST + row) as i32)
                .max(cell.first_y)
                .min(cell.last_y) as usize;
            let row = samples.get(y * cell.stride..)?;
            *window = match cell.clamp {
                Some(clamp) => clamp.bytes(row)?,
                None => Simd::from_array(*row.get(cell.column..)?.first_chunk()?),
            };
        }
        Some(windows)
    }

    /// The vertical pass of the seven rows in `first` (rows `0..4`) and
    /// `second` (rows `4..7`).
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn vertical<const SHIFT: u32>(&self, first: Pair, second: Pair) -> Pair {
        let one: Pair = simd_swizzle!(
            first,
            second,
            [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]
        );
        let two: Pair = simd_swizzle!(
            first,
            second,
            [8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23]
        );
        let three: Pair = simd_swizzle!(
            first,
            second,
            [
                12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27
            ]
        );
        weigh::<SHIFT>(&[(first, one, self.v_pairs[0]), (two, three, self.v_pairs[1])])
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn bytes(&self) -> Option<Pair> {
        let cell = &self.cell;
        if !cell.vertical {
            let bytes = join4(self.windows::<1, 4>()?);
            return Some(if cell.horizontal {
                horizontal4(bytes, cell.h_taps)
            } else {
                window::<0>(bytes) << (FILTER_BITS - INTER_ROUND0) as i16
            });
        }
        let rows = self.windows::<0, 7>()?;
        let first = join4([rows[0], rows[1], rows[2], rows[3]]);
        let second = join4([rows[4], rows[5], rows[6], rows[6]]);
        Some(if cell.horizontal {
            self.vertical::<INTER_ROUND1_COMPOUND>(
                horizontal4(first, cell.h_taps),
                horizontal4(second, cell.h_taps),
            )
        } else {
            self.vertical::<INTER_ROUND0>(window::<0>(first), window::<0>(second))
        })
    }
}

impl<'a, T: ReconSample> CellReference<'a, T> for Bytes4<'a, T> {
    type Rows = [Pair; 1];

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn new(reference: &ReferencePlaneView<'a, T>, params: &SubpelPredictParams) -> Option<Self> {
        let filter = params.interp.pass_index(SMALL_BLOCK_DIM) as usize;
        let v_phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
        let v_pairs = SMALL_PAIRS[filter * NUM_PHASES + v_phase].map(|pair| {
            let pair = Simd::from_array(pair);
            simd_swizzle!(pair, [0, 1, 2, 3, 4, 5, 6, 7, 0, 1, 2, 3, 4, 5, 6, 7])
        });
        Some(Self {
            cell: Cell::new(reference, params)?,
            v_pairs,
        })
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn rebind(&self, reference: &ReferencePlaneView<'a, T>) -> Self {
        Self {
            cell: self.cell.rebind(reference),
            v_pairs: self.v_pairs,
        }
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn predict(&self) -> Option<[Pair; 1]> {
        if let Some(rows) = self.bytes() {
            return Some([rows]);
        }
        let [a, b, c, d] = self.cell.predict()?;
        let low: Simd<i32, 8> = simd_swizzle!(a, b, [0, 1, 2, 3, 4, 5, 6, 7]);
        let high: Simd<i32, 8> = simd_swizzle!(c, d, [0, 1, 2, 3, 4, 5, 6, 7]);
        let rows: Simd<i32, 16> = simd_swizzle!(
            low,
            high,
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );
        Some([rows.cast()])
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
        let (Some(pred0), Some(pred1)) = (cells[0].predict(), cells[1].predict()) else {
            return false;
        };
        store_blended::<O, 1, 4>([pred0, pred1], params, cwp_weight, output, output_stride)
    }
}
