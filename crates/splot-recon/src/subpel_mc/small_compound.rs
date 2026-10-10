// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of both references of a 4x4 or 8x8 compound
//! cell and their § 7.13.3.16 COMPOUND_AVERAGE blend.
//!
//! A 4-sample axis uses a 4-tap small-block filter whose taps sit at indices
//! 2..=5, so a 4x4 row is one 8-sample window; an 8x8 row is one sliding
//! 16-sample window. Every intermediate stays in registers. A zero phase
//! skips its pass: `Round2(128 * s, 3) == s << 4` and `Round2(h << 7, 7) == h`
//! hold exactly, so each phase class keeps the two-pass arithmetic; with a
//! zero horizontal phase the vertical pass is `Round2(sum, 3)`, which equals
//! `Round2(sum << 4, 7)`. Clamped source rows use the clamped row, and a
//! window with clamped columns gathers them with a byte shuffle; both are the
//! spec read.

use super::*;
use std::simd::{ToBytes, simd_swizzle};

type Row = Simd<i32, 4>;
type Taps = Simd<i16, 4>;

/// Taps 2..=5 of every `Subpel_Filters` row; only the 4-tap rows are read.
const SMALL_TAPS: [[[i16; 4]; NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut taps = [[[0; 4]; NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 0;
        while phase < NUM_PHASES {
            let mut tap = 0;
            while tap < 4 {
                taps[filter][phase][tap] = SUBPEL_FILTERS[filter][phase][tap + 2] as i16;
                tap += 1;
            }
            phase += 1;
        }
        filter += 1;
    }
    taps
};

/// One reference of a 4x4 cell.
struct Cell<'a, T> {
    samples: &'a [T],
    /// Offsets of the clamped source rows `y0 - 1 ..= y0 + 5` at the first
    /// column the cell loads.
    rows: [usize; 7],
    /// The [`clamped_window`] shuffle when a window column is clamped.
    gather: Option<Simd<u8, 16>>,
    h_taps: Taps,
    v_taps: Taps,
    horizontal: bool,
    vertical: bool,
}

/// Halves the even taps of an 8-bit row, which keeps its sums in `i16` as in
/// `slide::intermediate_taps`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn horizontal_taps<T: ReconSample>(taps: Taps) -> Taps {
    if T::MAX_VALUE > u16::from(u8::MAX) {
        taps
    } else {
        taps >> 1
    }
}

const LANE_INDEX: [i32; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// Returns the first plane column of a window whose columns from `first` on
/// are clamped to `[firstX, lastX]`, and, for each byte of the window's `u16`
/// lanes, the byte of the samples loaded from that column that it copies.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn clamped_window(first: i32, params: &SubpelPredictParams) -> (usize, [Simd<u8, 16>; 2]) {
    let columns = (Simd::splat(first) + Simd::from_array(LANE_INDEX))
        .simd_max(Simd::splat(params.first_x))
        .simd_min(Simd::splat(params.last_x));
    let bytes = ((columns - Simd::splat(columns[0])) * Simd::splat(2)).cast::<u8>();
    let (low, high) = bytes.interleave(bytes + Simd::splat(1));
    (columns[0] as usize, [low, high])
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn gather8(lanes: Simd<u16, 8>, index: Simd<u8, 16>) -> Simd<u16, 8> {
    Simd::from_ne_bytes(lanes.to_ne_bytes().swizzle_dyn(index))
}

/// The 16-lane [`gather8`]. A byte index past one 16-byte half selects zero
/// from it, so the two half lookups combine with an OR.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn gather16(lanes: Simd<u16, 16>, index: [Simd<u8, 16>; 2]) -> Simd<u16, 16> {
    let bytes = lanes.to_ne_bytes();
    let low: Simd<u8, 16> = Simd::from_slice(&bytes.as_array()[..16]);
    let high: Simd<u8, 16> = Simd::from_slice(&bytes.as_array()[16..]);
    let half = |index| low.swizzle_dyn(index) | high.swizzle_dyn(index - Simd::splat(16));
    let (first, second) = (half(index[0]), half(index[1]));
    Simd::from_ne_bytes(Simd::from_array(core::array::from_fn(|byte| {
        if byte < 16 {
            first[byte]
        } else {
            second[byte - 16]
        }
    })))
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn horizontal<T: ReconSample>(lanes: Simd<u16, 8>, taps: Taps) -> Simd<i16, 4> {
    let samples = lanes.cast::<i16>();
    let windows = [
        simd_swizzle!(samples, [0, 1, 2, 3]),
        simd_swizzle!(samples, [1, 2, 3, 4]),
        simd_swizzle!(samples, [2, 3, 4, 5]),
        simd_swizzle!(samples, [3, 4, 5, 6]),
    ];
    if T::MAX_VALUE > u16::from(u8::MAX) {
        let mut sum = Row::splat(1 << (INTER_ROUND0 - 1));
        for (tap, window) in windows.into_iter().enumerate() {
            sum = tap_mac(sum, window, i32::from(taps[tap]));
        }
        return (sum >> INTER_ROUND0 as i32).cast();
    }
    let mut half = Simd::splat(1 << (INTER_ROUND0 - 2));
    for (tap, window) in windows.into_iter().enumerate() {
        half += window * Simd::splat(taps[tap]);
    }
    half >> (INTER_ROUND0 - 1) as i16
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn vertical(rows: &[Simd<i16, 4>], taps: Taps, shift: u32) -> Row {
    let mut sum = Row::splat(1 << (shift - 1));
    for (tap, &row) in rows[..4].iter().enumerate() {
        sum = tap_mac(sum, row, i32::from(taps[tap]));
    }
    sum >> shift as i32
}

/// One reference of a fused cell: `new` checks it and `predict` returns its
/// compound predictor rows, or `None` to decline the cell.
trait CellReference<'a, T: ReconSample>: Sized {
    type Rows;

    fn new(reference: &ReferencePlaneView<'a, T>, params: &SubpelPredictParams) -> Option<Self>;

    fn predict(&self) -> Option<Self::Rows>;
}

impl<'a, T: ReconSample> CellReference<'a, T> for Cell<'a, T> {
    type Rows = [Row; 4];

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn new(reference: &ReferencePlaneView<'a, T>, params: &SubpelPredictParams) -> Option<Self> {
        let filter = params.interp.pass_index(SMALL_BLOCK_DIM) as usize;
        let h_phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
        let v_phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
        let (tap_start, tap_end) = ACTIVE_TAP_SPANS[filter][h_phase];
        let x0 = params.start_x >> SCALE_SUBPEL_BITS;
        let first = x0 - i32::from(h_phase != 0);
        let (column, gather) = if first >= 0
            && x0 + tap_start as i32 - 3 >= params.first_x
            && x0 + tap_end as i32 - 1 <= params.last_x
        {
            (first as usize, None)
        } else {
            let (column, index) = clamped_window(first, params);
            (column, Some(index[0]))
        };
        let rows = (Simd::<i32, 8>::splat((params.start_y >> SCALE_SUBPEL_BITS) - 1)
            + Simd::from_array([0, 1, 2, 3, 4, 5, 6, 7]))
        .simd_max(Simd::splat(params.first_y))
        .simd_min(Simd::splat(params.last_y));
        Some(Cell {
            samples: reference.samples,
            rows: core::array::from_fn(|row| rows[row] as usize * reference.stride + column),
            gather,
            h_taps: horizontal_taps::<T>(Simd::from_array(SMALL_TAPS[filter][h_phase])),
            v_taps: Simd::from_array(SMALL_TAPS[filter][v_phase]),
            horizontal: h_phase != 0,
            vertical: v_phase != 0,
        })
    }

    /// The cell's compound predictor rows, `Round2(.., InterRound1)` of the
    /// two-pass convolution.
    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn predict(&self) -> Option<[Row; 4]> {
        let lanes = |row: usize| {
            let offset = self.rows[row];
            let lanes = reference_lanes::<8, T>(self.samples.get(offset..offset + 8)?, 0);
            Some(self.gather.map_or(lanes, |index| gather8(lanes, index)))
        };
        let mut rows = [Simd::splat(0); 7];
        let mut out = [Row::splat(0); 4];
        match (self.horizontal, self.vertical) {
            (true, true) => {
                for (row, value) in rows.iter_mut().enumerate() {
                    *value = horizontal::<T>(lanes(row)?, self.h_taps);
                }
                for (i, out) in out.iter_mut().enumerate() {
                    *out = vertical(&rows[i..], self.v_taps, INTER_ROUND1_COMPOUND);
                }
            }
            (true, false) => {
                for (i, out) in out.iter_mut().enumerate() {
                    *out = horizontal::<T>(lanes(i + 1)?, self.h_taps).cast();
                }
            }
            (false, true) => {
                for (row, value) in rows.iter_mut().enumerate() {
                    *value = simd_swizzle!(lanes(row)?, [0, 1, 2, 3]).cast();
                }
                for (i, out) in out.iter_mut().enumerate() {
                    *out = vertical(&rows[i..], self.v_taps, INTER_ROUND0);
                }
            }
            (false, false) => {
                for (i, out) in out.iter_mut().enumerate() {
                    *out = simd_swizzle!(lanes(i + 1)?, [0, 1, 2, 3]).cast::<i32>()
                        << (FILTER_BITS - INTER_ROUND0) as i32;
                }
            }
        }
        Some(out)
    }
}

/// Predicts both references of a cell, then blends and stores it; returns
/// `false` without writing when either reference declines. With
/// `cwp_weight` in `0..=16` a blended value is a weighted mean of predictor
/// values over 16, so it fits `i16` before its clamp.
#[inline(never)]
fn fused<'a, T: ReconSample, O: ReconSample, C, const LANES: usize, const ROWS: usize>(
    references: [(&ReferencePlaneView<'a, T>, &SubpelPredictParams); 2],
    cwp_weight: i16,
    output: &mut [O],
    output_stride: usize,
) -> bool
where
    C: CellReference<'a, T, Rows = [Simd<i32, LANES>; ROWS]>,
{
    let [(reference0, params0), (reference1, params1)] = references;
    let (Some(cell0), Some(cell1)) = (C::new(reference0, params0), C::new(reference1, params1))
    else {
        return false;
    };
    let (Some(pred0), Some(pred1)) = (cell0.predict(), cell1.predict()) else {
        return false;
    };
    let pred = [pred0, pred1];
    let bit_depth = params0.bit_depth;
    let forward = Simd::splat(i32::from(cwp_weight));
    let backward = Simd::splat(16 - i32::from(cwp_weight));
    let max_sample = Simd::splat(bit_depth.max_sample().min(O::MAX_VALUE) as i16);
    let shift = 4 + compound_inter_post_round();
    let blend = |i: usize| {
        ((pred[0][i] * forward + pred[1][i] * backward + Simd::splat(1 << (shift - 1)))
            >> shift as i32)
            .cast::<i16>()
            .simd_max(Simd::splat(0))
            .simd_min(max_sample)
    };
    if let Some(output) = O::u16_slice_mut(output) {
        for i in 0..ROWS {
            output[i * output_stride..][..LANES]
                .copy_from_slice(&blend(i).cast::<u16>().to_array()); // splot-copy-ok: publish blended SIMD prediction lanes
        }
        return true;
    }
    if let Some(output) = O::u8_slice_mut(output) {
        for i in 0..ROWS {
            output[i * output_stride..][..LANES].copy_from_slice(&blend(i).cast::<u8>().to_array()); // splot-copy-ok: publish blended SIMD prediction lanes
        }
        return true;
    }
    false
}

type Row8 = Simd<i32, 8>;
type Line8 = Simd<i16, 8>;

/// One reference of an 8x8 cell. `top` is the first source row the vertical
/// taps read, and `column` the first column the cell loads.
struct Cell8<'a, T> {
    samples: &'a [T],
    stride: usize,
    column: usize,
    /// The [`clamped_window`] shuffle when a window column is clamped.
    gather: Option<[Simd<u8, 16>; 2]>,
    top: i32,
    /// The offset of row `top` when no row the cell reads is clamped.
    unclamped: Option<usize>,
    first_y: i32,
    last_y: i32,
    h_taps: Simd<i16, NUM_TAPS>,
    /// `h_taps` packed for the gathered `u16` lanes.
    gather_taps: Simd<i16, NUM_TAPS>,
    v_taps: &'static [i32],
    horizontal: bool,
    vertical: bool,
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn window8<'a, T>(cell: &Cell8<'a, T>, row: usize, len: usize) -> Option<&'a [T]> {
    let offset = cell.unclamped.map_or_else(
        || {
            let y = (cell.top + row as i32).max(cell.first_y).min(cell.last_y) as usize;
            y * cell.stride + cell.column
        },
        |top| top + row * cell.stride,
    );
    cell.samples.get(offset..offset + len)
}

/// One 8-bit horizontal-pass row of an 8x8 cell from its 16-sample window,
/// with the halved taps of `slide::intermediate_taps`. Widening by
/// interleaving zero bytes stays a zip, so each window is one 16-byte slide;
/// LLVM turns a plain cast into a slide and a widen per window.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn eight_bit_row(bytes: Simd<u8, 16>, taps: Simd<i16, NUM_TAPS>) -> Line8 {
    let zero = Simd::splat(0);
    let (low, high) = if cfg!(target_endian = "big") {
        zero.interleave(bytes)
    } else {
        bytes.interleave(zero)
    };
    let low = Simd::<u16, 8>::from_ne_bytes(low).cast::<i16>();
    let high = Simd::<u16, 8>::from_ne_bytes(high).cast::<i16>();
    let windows = [
        low,
        simd_swizzle!(low, high, [1, 2, 3, 4, 5, 6, 7, 8]),
        simd_swizzle!(low, high, [2, 3, 4, 5, 6, 7, 8, 9]),
        simd_swizzle!(low, high, [3, 4, 5, 6, 7, 8, 9, 10]),
        simd_swizzle!(low, high, [4, 5, 6, 7, 8, 9, 10, 11]),
        simd_swizzle!(low, high, [5, 6, 7, 8, 9, 10, 11, 12]),
        simd_swizzle!(low, high, [6, 7, 8, 9, 10, 11, 12, 13]),
        simd_swizzle!(low, high, [7, 8, 9, 10, 11, 12, 13, 14]),
    ];
    let mut half = Line8::splat(1 << (INTER_ROUND0 - 2));
    for (tap, window) in windows.into_iter().enumerate() {
        half += window * Simd::splat(taps[tap]);
    }
    half >> (INTER_ROUND0 - 1) as i16
}

/// Source row `row` of the cell: the horizontal-pass intermediate when
/// `HORIZONTAL`, else the samples themselves.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn line8<T: ReconSample, const HORIZONTAL: bool>(cell: &Cell8<'_, T>, row: usize) -> Option<Line8> {
    if HORIZONTAL {
        let window = window8(cell, row, 2 * NUM_TAPS)?;
        let Some(index) = cell.gather else {
            if let Some(bytes) = T::u8_slice(window) {
                return Some(eight_bit_row(Simd::from_slice(bytes), cell.h_taps));
            }
            return Some(Row8::slid_intermediate(window, 0, cell.h_taps));
        };
        let lanes = gather16(reference_lanes::<16, T>(window, 0), index).to_array();
        return Some(Row8::slid_intermediate(&lanes[..], 0, cell.gather_taps));
    }
    let lanes = reference_lanes::<8, T>(window8(cell, row, 8)?, 0);
    Some(
        cell.gather
            .map_or(lanes, |index| gather8(lanes, index[0]))
            .cast(),
    )
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn two_pass8<T: ReconSample, const TAPS: usize, const HORIZONTAL: bool>(
    cell: &Cell8<'_, T>,
) -> Option<[Row8; 8]> {
    let mut lines = [Line8::splat(0); 8 + NUM_TAPS - 1];
    for (row, line) in lines[..8 + TAPS - 1].iter_mut().enumerate() {
        *line = line8::<T, HORIZONTAL>(cell, row)?;
    }
    let taps: &[i32; TAPS] = cell.v_taps.try_into().ok()?;
    let shift = if HORIZONTAL {
        INTER_ROUND1_COMPOUND
    } else {
        INTER_ROUND0
    };
    let mut out = [Row8::splat(0); 8];
    for (i, out) in out.iter_mut().enumerate() {
        let mut sum = Row8::splat(1 << (shift - 1));
        for (&tap, &line) in taps.iter().zip(&lines[i..]) {
            sum = tap_mac(sum, line, tap);
        }
        *out = sum >> shift as i32;
    }
    Some(out)
}

impl<'a, T: ReconSample> CellReference<'a, T> for Cell8<'a, T> {
    type Rows = [Row8; 8];

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn new(reference: &ReferencePlaneView<'a, T>, params: &SubpelPredictParams) -> Option<Self> {
        let filter = params.interp.pass_index(8) as usize;
        let h_phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
        let v_phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
        let (tap_start, tap_end) = ACTIVE_TAP_SPANS[filter][h_phase];
        let x0 = params.start_x >> SCALE_SUBPEL_BITS;
        let first = x0 - if h_phase != 0 { 3 } else { 0 };
        let (column, gather) = if first >= 0
            && x0 + tap_start as i32 - 3 >= params.first_x
            && x0 + tap_end as i32 + 3 <= params.last_x
        {
            (first as usize, None)
        } else {
            let (column, index) = clamped_window(first, params);
            (column, Some(index))
        };
        let (v_start, v_end) = ACTIVE_TAP_SPANS[filter][v_phase];
        let v_count = match v_end - v_start {
            0..=2 => 2,
            3..=4 => 4,
            5..=6 => 6,
            _ => NUM_TAPS,
        };
        let v_start = v_start.min(NUM_TAPS - v_count);
        let top = (params.start_y >> SCALE_SUBPEL_BITS) - 3 + v_start as i32;
        let unclamped = (top >= params.first_y && top + 6 + NUM_TAPS as i32 <= params.last_y)
            .then(|| top as usize * reference.stride + column);
        Some(Cell8 {
            samples: reference.samples,
            stride: reference.stride,
            column,
            gather,
            top,
            unclamped,
            first_y: params.first_y,
            last_y: params.last_y,
            h_taps: slide::intermediate_taps::<T>(&SUBPEL_FILTERS[filter][h_phase]),
            gather_taps: slide::intermediate_taps::<u16>(&SUBPEL_FILTERS[filter][h_phase]),
            v_taps: &SUBPEL_FILTERS[filter][v_phase][v_start..v_start + v_count],
            horizontal: h_phase != 0,
            vertical: v_phase != 0,
        })
    }

    #[allow(clippy::inline_always, reason = "measured subpel hot path")]
    #[inline(always)]
    fn predict(&self) -> Option<[Row8; 8]> {
        match (self.horizontal, self.vertical, self.v_taps.len()) {
            (true, true, 2) => two_pass8::<T, 2, true>(self),
            (true, true, 4) => two_pass8::<T, 4, true>(self),
            (true, true, 6) => two_pass8::<T, 6, true>(self),
            (true, true, _) => two_pass8::<T, NUM_TAPS, true>(self),
            (false, true, 2) => two_pass8::<T, 2, false>(self),
            (false, true, 4) => two_pass8::<T, 4, false>(self),
            (false, true, 6) => two_pass8::<T, 6, false>(self),
            (false, true, _) => two_pass8::<T, NUM_TAPS, false>(self),
            (true, false, _) => {
                let mut out = [Row8::splat(0); 8];
                for (row, out) in out.iter_mut().enumerate() {
                    *out = line8::<T, true>(self, row)?.cast();
                }
                Some(out)
            }
            (false, false, _) => {
                let mut out = [Row8::splat(0); 8];
                for (row, out) in out.iter_mut().enumerate() {
                    *out =
                        line8::<T, false>(self, row)?.cast() << (FILTER_BITS - INTER_ROUND0) as i32;
                }
                Some(out)
            }
        }
    }
}

/// Writes a blended 4x4 or 8x8 cell and returns `true`, or returns `false`
/// without writing for other sizes, for a `cwp_weight` outside `0..=16`, or
/// when a window of either reference leaves the plane storage.
///
/// The parameters are plane-bounded and validated by the caller, and `output`
/// holds the strided cell rectangle.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
pub(super) fn predict<T: ReconSample, O: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [O],
    output_stride: usize,
) -> bool {
    if !(0..=16).contains(&cwp_weight) {
        return false;
    }
    let references = [(reference0, params0), (reference1, params1)];
    match (params0.w, params0.h) {
        (4, 4) => fused::<T, O, Cell<'_, T>, 4, 4>(references, cwp_weight, output, output_stride),
        (8, 8) => fused::<T, O, Cell8<'_, T>, 8, 8>(references, cwp_weight, output, output_stride),
        _ => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as usize % bound
        }
    }

    const FILTERS: [InterpolationFilter; 4] = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];

    fn random_params(
        rng: &mut Lcg,
        interp: InterpolationFilter,
        size: usize,
        (width, height): (usize, usize),
        bit_depth: BitDepth,
    ) -> SubpelPredictParams {
        let mut phase = || {
            if rng.below(3) == 0 {
                0
            } else {
                rng.below(16) as i32
            }
        };
        let (phase_x, phase_y) = (phase(), phase());
        let x = rng.below(width + 12) as i32 - 6;
        let y = rng.below(height + 12) as i32 - 6;
        let mut window = |extent: usize| {
            if rng.below(3) == 0 {
                let first = rng.below(extent / 2);
                (first as i32, (first + 1 + rng.below(extent / 2)) as i32)
            } else {
                (0, extent as i32 - 1)
            }
        };
        let ((first_x, last_x), (first_y, last_y)) = (window(width), window(height));
        SubpelPredictParams {
            interp,
            w: size,
            h: size,
            start_x: (x << SCALE_SUBPEL_BITS) + (phase_x << 6),
            start_y: (y << SCALE_SUBPEL_BITS) + (phase_y << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x,
            first_y,
            last_x,
            last_y,
            bit_depth,
        }
    }

    /// Compares random `size x size` compound cells of every phase class with
    /// the two-call path, through the fast dispatch and the fused kernel.
    fn check_cells<T: ReconSample>(bit_depth: BitDepth, size: usize) {
        let (width, height) = (24 + 4 * size, 20 + 4 * size);
        let max = bit_depth.max_sample();
        let mut rng = Lcg(u64::from(max) * 31 + size as u64);
        let mut plane = || {
            (0..width * height)
                .map(|_| T::try_from_u16(rng.below(usize::from(max) + 1) as u16).unwrap())
                .collect::<Vec<T>>()
        };
        let planes = [plane(), plane()];
        let views =
            [0, 1].map(|index| ReferencePlaneView::new(&planes[index], width, height).unwrap());
        let stride = size + 3;
        let sentinel = T::try_from_u16(max / 3).unwrap();
        let (mut fused, mut declined) = (0, 0);
        for case in 0..3000 {
            let interp = FILTERS[case % FILTERS.len()];
            let params =
                [(); 2].map(|()| random_params(&mut rng, interp, size, (width, height), bit_depth));
            let weight = [8, 12, 4, 10, 1000][case % 5];
            let pred = [0, 1].map(|index| {
                subpel_predict_block_compound_intermediate(&views[index], &params[index]).unwrap()
            });
            let expected =
                blend_compound_average_weighted(&pred[0], &pred[1], bit_depth, weight).unwrap();
            let mut output = vec![sentinel; stride * size];
            assert!(
                subpel_predict_block_compound_average_fast_validated_strided_into(
                    &views[0],
                    &params[0],
                    &views[1],
                    &params[1],
                    weight,
                    &mut [],
                    &mut output,
                    stride,
                )
                .unwrap()
            );
            let bounded = [0, 1].map(|index| plane_bounded(&views[index], &params[index]));
            let mut direct = vec![sentinel; stride * size];
            let accepted = predict(
                &views[0],
                &bounded[0],
                &views[1],
                &bounded[1],
                weight,
                &mut direct,
                stride,
            );
            assert!(!accepted || weight <= 16);
            if accepted {
                fused += 1;
            } else {
                declined += 1;
                assert!(
                    direct
                        .iter()
                        .all(|&sample| sample.to_u16() == sentinel.to_u16())
                );
            }
            for row in 0..size {
                let lanes = row * stride..row * stride + size;
                let expected = &expected[row * size..(row + 1) * size];
                for (results, check) in [(&output, true), (&direct, accepted)] {
                    let actual = results[lanes.clone()].iter().map(|sample| sample.to_u16());
                    assert!(
                        !check || actual.eq(expected.iter().copied()),
                        "{case} {params:?} {weight}"
                    );
                }
                assert!(
                    output[row * stride + size..(row + 1) * stride]
                        .iter()
                        .all(|&sample| sample.to_u16() == sentinel.to_u16())
                );
            }
        }
        assert!(
            fused > 2000 && declined > 600,
            "fused {fused} declined {declined}"
        );
    }

    #[test]
    fn fused_cells_match_the_two_call_path() {
        for size in [4, 8] {
            check_cells::<u8>(BitDepth::Eight, size);
            check_cells::<u16>(BitDepth::Ten, size);
            check_cells::<u16>(BitDepth::Eight, size);
        }
    }
}
