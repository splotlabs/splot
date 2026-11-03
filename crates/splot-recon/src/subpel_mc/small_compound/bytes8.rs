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
//! their blend fit `i16`. A window that leaves the plane storage takes the
//! generic [`Cell8`] and repacks its rows.

use super::super::pairs::{Pair, horizontal, join, weigh, widen};
use super::*;

pub(super) struct Bytes8<'a, T> {
    cell: Cell8<'a, T>,
}

impl<T: ReconSample> Bytes8<'_, T> {
    /// The 16-byte windows of source rows `0..ROWS` from `cell.top`, or
    /// `None` when the samples are wider than 8 bits or a column is clamped.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn windows<const ROWS: usize>(&self) -> Option<[Simd<u8, 16>; ROWS]> {
        let samples = T::u8_slice(self.cell.samples)?;
        match self.cell.clamp {
            None => self.rows::<ROWS, false>(samples),
            Some(_) => self.rows::<ROWS, true>(samples),
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
        let clamp = if EDGES { cell.clamp } else { None };
        let mut windows = [Simd::splat(0); ROWS];
        for (row, window) in windows.iter_mut().enumerate() {
            let y = (cell.top + row as i32).max(cell.first_y).min(cell.last_y) as usize;
            let row = samples.get(y * cell.stride..)?;
            *window = match clamp {
                Some(clamp) => clamp.bytes(row)?,
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
                *pair = (lines[k], lines[k + 1], self.cell.v_pairs[j]);
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
        match cell.v_count {
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
        Some(Self {
            cell: Cell8::new(reference, params)?,
        })
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn rebind(&self, reference: &ReferencePlaneView<'a, T>) -> Self {
        Self {
            cell: self.cell.rebind(reference),
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
        let (Some(pred0), Some(pred1)) = (cells[0].predict(), cells[1].predict()) else {
            return false;
        };
        store_blended::<O, 4, 8>([pred0, pred1], params, cwp_weight, output, output_stride)
    }
}

/// Blends the predictor rows of both references, `16 / WIDTH` rows of
/// `WIDTH` lanes per [`Pair`], and stores them; `false` for an unsupported
/// output.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn store_blended<O: ReconSample, const P: usize, const WIDTH: usize>(
    pred: [[Pair; P]; 2],
    params: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [O],
    output_stride: usize,
) -> bool {
    let weights = Pair::from_array(core::array::from_fn(|lane| {
        if lane % 2 == 0 {
            cwp_weight
        } else {
            16 - cwp_weight
        }
    }));
    let max_sample = Pair::splat(params.bit_depth.max_sample().min(O::MAX_VALUE) as i16);
    let blend = |p: usize| {
        weigh::<{ 4 + compound_inter_post_round() }>(&[(pred[0][p], pred[1][p], weights)])
            .simd_max(Simd::splat(0))
            .simd_min(max_sample)
    };
    let rows = 16 / WIDTH;
    if let Some(output) = O::u8_slice_mut(output) {
        for p in 0..P {
            let lanes = blend(p).cast::<u8>().to_array();
            for (row, lanes) in lanes.chunks_exact(WIDTH).enumerate() {
                output[(rows * p + row) * output_stride..][..WIDTH].copy_from_slice(lanes); // splot-copy-ok: publish blended SIMD prediction lanes
            }
        }
        return true;
    }
    if let Some(output) = O::u16_slice_mut(output) {
        for p in 0..P {
            let lanes = blend(p).cast::<u16>().to_array();
            for (row, lanes) in lanes.chunks_exact(WIDTH).enumerate() {
                output[(rows * p + row) * output_stride..][..WIDTH].copy_from_slice(lanes); // splot-copy-ok: publish blended SIMD prediction lanes
            }
        }
        return true;
    }
    false
}
