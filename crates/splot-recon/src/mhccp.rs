// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.6 MHCCP parameter derivation helpers.

use fearless_simd::{Bytes as _, Select as _, Simd, SimdBase, i32x8, u16x8};

use crate::math::{round2_i32, round2_signed_i32};
use crate::{BitDepth, ReconSample};

#[doc = "AV2 § 3 symbols and § 7.13.6 MHCCP process constants."]
pub const MHCCP_BITS: u32 = 16;
pub const MHCCP_PARAM_COUNT: usize = 3;
const DIV_PREC_BITS: u32 = 14;
const DIV_PREC_BITS_POW2: u32 = 8;
const DIV_SLOT_BITS: u32 = 3;
const DIV_INTR_BITS: u32 = DIV_PREC_BITS - DIV_SLOT_BITS;
#[doc = "AV2 § 7.13.6 `get_division_scale_shift` lookup tables."]
const DIVISION_POW2_W: [i32; 8] = [214, 153, 113, 86, 67, 53, 43, 35];
const DIVISION_POW2_O: [i32; 8] = [4822, 5952, 6624, 6792, 6408, 5424, 3792, 1466];
const DIVISION_POW2_B: [i32; 8] = [12784, 12054, 11670, 11583, 11764, 12195, 12870, 13782];

/// The luma references of an AV2 § 7.13.6 MHCCP block (`CflRef[0]`), shared
/// by both chroma planes, whose `CflRef[1]` buffers use the same layout.
#[derive(Clone, Copy)]
pub struct MhccpRefs<'a> {
    /// Reference-buffer row stride.
    pub width: usize,
    /// Reference-buffer row count.
    pub height: usize,
    /// Width of the captured reference region used for parameter derivation.
    pub reference_width: usize,
    /// Height of the captured reference region used for parameter derivation.
    pub reference_height: usize,
    /// Number of above-reference rows.
    pub above: usize,
    /// Number of left-reference columns.
    pub left: usize,
    /// Luma reference samples.
    pub luma: &'a [u16],
}

impl MhccpRefs<'_> {
    fn has_edge_refs(&self) -> bool {
        self.above != 0 || self.left != 0
    }
}

const UPPER_TRIANGLE: [(usize, usize); 6] = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];

#[derive(Clone, Copy, Debug, Default)]
struct NormalEquations {
    ata: [[i32; MHCCP_PARAM_COUNT]; MHCCP_PARAM_COUNT],
    b: [i32; MHCCP_PARAM_COUNT],
}

impl NormalEquations {
    /// The § 7.13.7 statistics with `v[2]` fixed at `mid`. Each product of
    /// two `BitDepth`-bit samples is below `2^(2 * BitDepth)`, so the sums of
    /// at most `2^(31 - 2 * BitDepth)` samples are exact in `i32`.
    fn from_sums(luma: &LumaSums, cross: [i32; 3], mid: i32) -> Self {
        let [s00, s01, s11, s0, s1] = luma.sums;
        let count = luma.count as i32;
        Self {
            ata: [
                [s00, s01, s0.wrapping_mul(mid)],
                [0, s11, s1.wrapping_mul(mid)],
                [0, 0, count.wrapping_mul(mid).wrapping_mul(mid)],
            ],
            b: [cross[0], cross[1], cross[2].wrapping_mul(mid)],
        }
    }

    fn apply_matrix_shift(&mut self, matrix_shift: i32) {
        if matrix_shift == 0 {
            return;
        }
        let shift = matrix_shift.unsigned_abs();
        for (row, col) in UPPER_TRIANGLE {
            if matrix_shift.is_positive() {
                self.ata[row][col] = self.ata[row][col].saturating_mul(1i32 << shift);
            } else {
                self.ata[row][col] >>= shift;
            }
        }
        for value in &mut self.b {
            if matrix_shift.is_positive() {
                *value = value.saturating_mul(1i32 << shift);
            } else {
                *value >>= shift;
            }
        }
    }
}

/// Derives the three MHCCP model parameters of each chroma plane in
/// `chroma`; the luma statistics are gathered once for all planes.
#[must_use]
pub fn derive_mhccp_params<const N: usize>(
    refs: &MhccpRefs<'_>,
    chroma: [&[u16]; N],
    mh_dir: u8,
    bit_depth: BitDepth,
) -> [[i32; MHCCP_PARAM_COUNT]; N] {
    let bits = u32::from(bit_depth.bits());
    let mut luma = LumaSums::default();
    let mut cross = [[0i32; 3]; N];
    if refs.has_edge_refs() {
        let stride = refs.width;
        let reference_width = refs.reference_width.min(refs.width);
        let reference_height = refs.reference_height.min(refs.height);
        let rows = 1..reference_height.saturating_sub(1);
        let linear_offset = |start: usize| match mh_dir {
            0 => start + 1,
            1 => start - stride + 1,
            _ => start,
        };
        let cols = reference_width.saturating_sub(1);
        let left_rows = rows.start.max(refs.above)..rows.end;
        baseline_simd!(simd => {
            let mut lanes = LaneSums::new(simd);
            if cols > 1 {
                for row in rows.start..rows.end.min(refs.above) {
                    let start = row * stride;
                    let linear_start = linear_offset(start);
                    for at in (0..cols - 1).step_by(8) {
                        let n = (cols - 1 - at).min(8);
                        let run = |samples: &[u16], start: usize| {
                            masked_lanes(simd, samples, start + at, n)
                        };
                        let targets = chroma.map(|plane| run(plane, start + 1));
                        let linear = run(refs.luma, linear_start);
                        lanes.add(linear, run(refs.luma, start + 1), targets, n, bits);
                    }
                }
            }
            if refs.left.min(cols) > 1 {
                for first in left_rows.clone().step_by(8) {
                    let n = (left_rows.end - first).min(8);
                    let at = |k: usize| (first + k) * stride;
                    let gather = |samples: &[u16], linear: bool| {
                        i32x8::from_fn(simd, |k| {
                            let at = if linear { linear_offset(at(k)) } else { at(k) + 1 };
                            match samples.get(at) {
                                Some(&sample) if k < n => i32::from(sample),
                                _ => 0,
                            }
                        })
                    };
                    let targets = chroma.map(|plane| gather(plane, false));
                    lanes.add(gather(refs.luma, true), gather(refs.luma, false), targets, n, bits);
                }
            }
            lanes.reduce_into(&mut luma, &mut cross);
        });
    }
    let mid = 1i32 << (bits - 1);
    if luma.count == 0 {
        return [[0, 0, 1i32 << MHCCP_BITS]; N];
    }
    let matrix_shift = MHCCP_BITS as i32 + 6
        - 2 * i32::from(bit_depth.bits())
        - ceil_log2_usize(luma.count) as i32;
    let equations = cross.map(|cross| {
        let mut equations = NormalEquations::from_sums(&luma, cross, mid);
        equations.apply_matrix_shift(matrix_shift);
        equations
    });
    solve_mhccp(&equations, bit_depth)
}

/// Wrapping sums of the § 7.13.7 luma products `v0 * v0`, `v0 * v1`,
/// `v1 * v1`, `v0` and `v1` over the edge samples.
#[derive(Debug, Default)]
struct LumaSums {
    sums: [i32; 5],
    count: usize,
}

/// The § 7.13.7 sums of [`LumaSums`] and the per-plane `v0 * t`, `v1 * t`,
/// `t` sums, eight samples per lane, wrapping like the scalar sums.
struct LaneSums<S: Simd, const N: usize> {
    luma: [i32x8<S>; 5],
    cross: [[i32x8<S>; 3]; N],
    count: usize,
}

impl<S: Simd, const N: usize> LaneSums<S, N> {
    #[inline]
    fn new(simd: S) -> Self {
        let zero = i32x8::splat(simd, 0);
        Self {
            luma: [zero; 5],
            cross: [[zero; 3]; N],
            count: 0,
        }
    }

    /// Adds `count` samples of `v[0]`, centre luma and each plane's target;
    /// the lanes past `count` are zero, so every term they add is zero.
    #[inline]
    fn add(
        &mut self,
        v0: i32x8<S>,
        center: i32x8<S>,
        targets: [i32x8<S>; N],
        count: usize,
        bits: u32,
    ) {
        let v1 = (center * center + (1i32 << (bits - 1))) >> bits;
        for (acc, term) in self
            .luma
            .iter_mut()
            .zip([v0 * v0, v0 * v1, v1 * v1, v0, v1])
        {
            *acc += term;
        }
        for (acc, t) in self.cross.iter_mut().zip(targets) {
            for (acc, term) in acc.iter_mut().zip([v0 * t, v1 * t, t]) {
                *acc += term;
            }
        }
        self.count += count;
    }

    #[inline]
    fn reduce_into(self, luma: &mut LumaSums, cross: &mut [[i32; 3]; N]) {
        let reduce = |lanes: i32x8<S>| {
            lanes
                .as_slice()
                .iter()
                .fold(0i32, |sum, &lane| sum.wrapping_add(lane))
        };
        for (sum, acc) in luma.sums.iter_mut().zip(self.luma) {
            *sum = sum.wrapping_add(reduce(acc));
        }
        for (sums, acc) in cross.iter_mut().zip(self.cross) {
            for (sum, acc) in sums.iter_mut().zip(acc) {
                *sum = sum.wrapping_add(reduce(acc));
            }
        }
        luma.count += self.count;
    }
}

/// The first `count` (at most 8) samples of `samples` from `start` as `i32`
/// lanes, the rest zero.
#[inline]
fn masked_lanes<S: Simd>(simd: S, samples: &[u16], start: usize, count: usize) -> i32x8<S> {
    let lanes = lanes8(samples, start);
    let index = i32x8::from_slice(simd, &[0, 1, 2, 3, 4, 5, 6, 7]);
    let count = i32x8::splat(simd, count as i32);
    index
        .simd_lt(count)
        .select(widen_lanes(simd, &lanes), i32x8::splat(simd, 0))
}

/// The § 7.13.8 Gaussian elimination of each plane's `equations`, which
/// share their matrix: its eliminated rows are worked out once, and each
/// plane's right-hand side follows the same steps.
fn solve_mhccp<const N: usize>(
    equations: &[NormalEquations; N],
    bit_depth: BitDepth,
) -> [[i32; MHCCP_PARAM_COUNT]; N] {
    let Some(first) = equations.first() else {
        return [[0; MHCCP_PARAM_COUNT]; N];
    };
    let mut c = [[0i32; MHCCP_PARAM_COUNT]; MHCCP_PARAM_COUNT];
    for (row, values) in c.iter_mut().enumerate() {
        for (col, value) in values.iter_mut().enumerate() {
            *value = symmetric_equation_value(first, row, col);
        }
        values[row] = values[row].saturating_add(2i32 << (u32::from(bit_depth.bits()) - 8));
    }
    let mut b = equations.map(|equations| equations.b);
    eliminate::<0, N>(&mut c, &mut b);
    eliminate::<1, N>(&mut c, &mut b);
    eliminate::<2, N>(&mut c, &mut b);
    b.map(|b| {
        let mut params = [0i32; MHCCP_PARAM_COUNT];
        for row in (0..MHCCP_PARAM_COUNT).rev() {
            let mut value = b[row];
            for col in row + 1..MHCCP_PARAM_COUNT {
                value =
                    value.saturating_sub(mul_fixed32_adapt(c[row][col], params[col], MHCCP_BITS));
            }
            params[row] = value;
        }
        params
    })
}

/// Step `I` of [`solve_mhccp`]: scales pivot row `I` and eliminates it from
/// the rows below.
#[allow(
    clippy::inline_always,
    reason = "constant loop bounds unroll each step"
)]
#[inline(always)]
fn eliminate<const I: usize, const N: usize>(
    c: &mut [[i32; MHCCP_PARAM_COUNT]; MHCCP_PARAM_COUNT],
    b: &mut [[i32; MHCCP_PARAM_COUNT]; N],
) {
    let diag = c[I][I].unsigned_abs().max(1);
    let (scale, shift) = mhccp_division_scale_shift(diag);
    for value in c[I].iter_mut().skip(I + 1) {
        *value = mul_fixed32_adapt(*value, scale, shift);
    }
    for b in &mut *b {
        b[I] = mul_fixed32_adapt(b[I], scale, shift);
    }
    let pivot = c[I];
    for j in I + 1..MHCCP_PARAM_COUNT {
        let factor = c[j][I];
        for col in I + 1..MHCCP_PARAM_COUNT {
            c[j][col] = c[j][col].saturating_sub(mul_fixed32_adapt(factor, pivot[col], MHCCP_BITS));
        }
        for b in &mut *b {
            b[j] = b[j].saturating_sub(mul_fixed32_adapt(factor, b[I], MHCCP_BITS));
        }
    }
}

fn symmetric_equation_value(equations: &NormalEquations, row: usize, col: usize) -> i32 {
    if col >= row {
        equations.ata[row][col]
    } else {
        equations.ata[col][row]
    }
}

fn mhccp_division_scale_shift(denom: u32) -> (i32, u32) {
    let shift = denom.checked_ilog2().unwrap_or(0);
    let norm_diff_clip =
        normalized_divisor(denom, shift).clamp(1, (1i32 << (DIV_PREC_BITS + 1)) - 1);
    let norm_diff = norm_diff_clip & ((1i32 << DIV_PREC_BITS) - 1);
    (division_scale(norm_diff), shift)
}

fn normalized_divisor(denom: u32, shift: u32) -> i32 {
    let delta = shift as i32 - DIV_PREC_BITS as i32;
    if delta >= 0 {
        let right = delta as u32;
        let bias = right.checked_sub(1).map_or(0, |bits| 1u32 << bits);
        return ((denom.saturating_add(bias)) >> right) as i32;
    }
    let left = (-delta) as u32;
    (denom << left) as i32
}

fn division_scale(norm_diff: i32) -> i32 {
    let index = ((norm_diff >> DIV_INTR_BITS) as usize).min(DIVISION_POW2_W.len() - 1);
    let norm_diff2 = norm_diff - DIVISION_POW2_O[index];
    let squared = (norm_diff2.saturating_mul(norm_diff2)) >> DIV_PREC_BITS;
    let mut scale = ((DIVISION_POW2_W[index].saturating_mul(squared)) >> DIV_PREC_BITS_POW2)
        - (norm_diff2 >> 1)
        + DIVISION_POW2_B[index];
    scale <<= MHCCP_BITS - DIV_PREC_BITS;
    scale
}

/// The § 7.13.6 per-sample MHCCP prediction for one block's parameters.
#[derive(Clone, Copy, Debug)]
pub struct MhccpPredictor {
    params: [i32; MHCCP_PARAM_COUNT],
    offset: i32,
    bits: u32,
    max: i32,
    /// `vec[0]` and `vec[1]` are at most `bits` wide, so `mul_fixed32_adapt`
    /// never down-shifts when each data parameter fits in `29 - bits` bits.
    narrow: bool,
}

impl MhccpPredictor {
    /// Resolves the constant `vec[2]` term of § 7.13.6 once per block.
    #[must_use]
    pub fn new(params: [i32; MHCCP_PARAM_COUNT], bit_depth: BitDepth) -> Self {
        let bits = u32::from(bit_depth.bits());
        let mid = 1i32 << (bits - 1);
        let narrow = params[..2].iter().all(|&p| bit_width_abs(p) + bits <= 29);
        Self {
            params,
            offset: mul_fixed32_adapt(params[2], mid, MHCCP_BITS),
            bits,
            max: i32::from(bit_depth.max_sample()),
            narrow,
        }
    }

    /// Predicts one sample from its linear and centre luma references.
    #[must_use]
    pub fn sample(&self, linear: u16, center: u16) -> u16 {
        let center = i32::from(center);
        let square = round2_i32(center * center, self.bits);
        mul_fixed32_adapt(self.params[0], i32::from(linear), MHCCP_BITS)
            .saturating_add(mul_fixed32_adapt(self.params[1], square, MHCCP_BITS))
            .saturating_add(self.offset)
            .clamp(0, self.max) as u16
    }

    /// Predicts the `width` x `height` block of `refs` in direction `mh_dir`
    /// into `out`, row `r` at `out[r * stride..]`; `false` when `out` or the
    /// references are too short for the block.
    #[must_use]
    pub fn predict_block<T: ReconSample>(
        &self,
        refs: &MhccpRefs<'_>,
        mh_dir: u8,
        width: usize,
        height: usize,
        out: &mut [T],
        stride: usize,
    ) -> bool {
        if width == 0
            || height == 0
            || (height - 1) * stride + width > out.len()
            || (refs.above + height) * refs.width > refs.luma.len()
            || refs.left + width > refs.width
        {
            return false;
        }
        if let Some(out) = T::u16_slice_mut(out) {
            baseline_simd!(simd => self.predict_rows(simd, refs, mh_dir, width, height, out, stride));
        } else if let Some(out) = T::u8_slice_mut(out) {
            baseline_simd!(simd => self.predict_rows(simd, refs, mh_dir, width, height, out, stride));
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn predict_rows<S: Simd, P: PredictedSample>(
        &self,
        simd: S,
        refs: &MhccpRefs<'_>,
        mh_dir: u8,
        width: usize,
        height: usize,
        out: &mut [P],
        stride: usize,
    ) {
        let luma = refs.luma;
        let lanes = width.next_multiple_of(8);
        let run = |start: usize| &luma[start..(start + lanes).min(luma.len())];
        let taps = |row: usize| {
            let center = (refs.above + row) * refs.width + refs.left;
            match mh_dir {
                0 => center,
                1 => (refs.above + row).saturating_sub(1) * refs.width + refs.left,
                _ => center - 1,
            }
        };
        let mut row = 0;
        let centre = |row: usize| (refs.above + row) * refs.width + refs.left;
        if width.is_multiple_of(8) && self.narrow && (mh_dir != 2 || refs.left > 0) {
            for row in 0..height {
                let (linear, center) = (taps(row), centre(row));
                let out = &mut out[row * stride..row * stride + width];
                for (j, out) in out.chunks_exact_mut(8).enumerate() {
                    let t = self.group(
                        simd,
                        lanes8(luma, linear + 8 * j),
                        lanes8(luma, center + 8 * j),
                    );
                    P::store8(simd, out, t);
                }
            }
            return;
        }
        if width == 4 && self.narrow && (mh_dir != 2 || refs.left > 0) {
            while row + 1 < height {
                let mut linear = [0u16; 8];
                let mut center = [0u16; 8];
                for (half, row) in [row, row + 1].into_iter().enumerate() {
                    let at = centre(row);
                    linear[4 * half..4 * half + 4].copy_from_slice(&luma[taps(row)..taps(row) + 4]); // splot-copy-ok: pack two 4-wide MHCCP rows
                    center[4 * half..4 * half + 4].copy_from_slice(&luma[at..at + 4]); // splot-copy-ok: pack two 4-wide MHCCP rows
                }
                let mut pair = [P::from_clipped(0); 8];
                P::store8(simd, &mut pair, self.group(simd, linear, center));
                out[row * stride..row * stride + 4].copy_from_slice(&pair[..4]); // splot-copy-ok: publish a 4-wide MHCCP row
                out[(row + 1) * stride..(row + 1) * stride + 4].copy_from_slice(&pair[4..]); // splot-copy-ok: publish a 4-wide MHCCP row
                row += 2;
            }
        }
        for row in row..height {
            let out = &mut out[row * stride..row * stride + width];
            let center_start = (refs.above + row) * refs.width + refs.left;
            let center = run(center_start);
            match mh_dir {
                0 => self.row(simd, center, center, out),
                1 => {
                    let top = (refs.above + row).saturating_sub(1) * refs.width + refs.left;
                    self.row(simd, run(top), center, out);
                }
                _ if refs.left == 0 => {
                    out[0] = P::from_clipped(i32::from(self.sample(center[0], center[0])));
                    self.row(simd, center, run(center_start + 1), &mut out[1..]);
                }
                _ => self.row(simd, run(center_start - 1), center, out),
            }
        }
    }

    /// Predicts `out.len()` samples; `linear` and `center` must be at least
    /// as long, and their samples past `out.len()` let the last 8-lane group
    /// run whole.
    #[inline]
    fn row<S: Simd, P: PredictedSample>(
        &self,
        simd: S,
        linear: &[u16],
        center: &[u16],
        out: &mut [P],
    ) {
        if !self.narrow {
            for ((out, &linear), &center) in out.iter_mut().zip(linear).zip(center) {
                *out = P::from_clipped(i32::from(self.sample(linear, center)));
            }
            return;
        }
        let square_half = 1i32 << (self.bits - 1);
        let lanes = linear.len().min(center.len());
        let mut at = 0;
        while at < out.len() && at + 8 <= lanes {
            let t = self.group(simd, lanes8(linear, at), lanes8(center, at));
            if let Some(out) = out.get_mut(at..at + 8) {
                P::store8(simd, out, t);
            } else {
                let mut group = [P::from_clipped(0); 8];
                P::store8(simd, &mut group, t);
                let out = &mut out[at..];
                out.copy_from_slice(&group[..out.len()]); // splot-copy-ok: publish a short MHCCP row
            }
            at += 8;
        }
        for ((out, &linear), &center) in out.iter_mut().zip(linear).zip(center).skip(at) {
            let center = i32::from(center);
            let square = (center * center + square_half) >> self.bits;
            let t = round2_signed_i32(i32::from(linear) * self.params[0], MHCCP_BITS)
                + round2_signed_i32(square * self.params[1], MHCCP_BITS)
                + self.offset;
            *out = P::from_clipped(t.clamp(0, self.max));
        }
    }

    /// Eight § 7.13.6 samples from their linear and centre luma references,
    /// for a [`Self::narrow`] predictor.
    #[allow(
        clippy::inline_always,
        reason = "keeps the parameter lanes in registers"
    )]
    #[inline(always)]
    fn group<S: Simd>(&self, simd: S, linear: [u16; 8], center: [u16; 8]) -> i32x8<S> {
        let round = |x: i32x8<S>| (x + (1i32 << (MHCCP_BITS - 1)) + (x >> 31)) >> MHCCP_BITS;
        let center = widen_lanes(simd, &center);
        let square = (center * center + (1i32 << (self.bits - 1))) >> self.bits;
        let t = round(widen_lanes(simd, &linear) * self.params[0])
            + round(square * self.params[1])
            + self.offset;
        t.max(i32x8::splat(simd, 0)).min(self.max)
    }
}

/// A predicted sample type the MHCCP lanes store into.
trait PredictedSample: Copy {
    /// Stores eight samples already clipped to the sample range.
    fn store8<S: Simd>(simd: S, out: &mut [Self], v: i32x8<S>);
    fn from_clipped(v: i32) -> Self;
}

impl PredictedSample for u8 {
    #[inline]
    fn store8<S: Simd>(simd: S, out: &mut [Self], v: i32x8<S>) {
        let (low, high) = simd.split_u32x8(v.bitcast());
        let words = simd.relaxed_narrow_u32x4(low, high);
        let bytes: [u8; 16] = simd.relaxed_narrow_u16x8(words, words).into();
        out[..8].copy_from_slice(&bytes[..8]); // splot-copy-ok: publish eight MHCCP samples
    }

    #[inline]
    fn from_clipped(v: i32) -> Self {
        v as u8
    }
}

impl PredictedSample for u16 {
    #[inline]
    fn store8<S: Simd>(simd: S, out: &mut [Self], v: i32x8<S>) {
        let (low, high) = simd.split_u32x8(v.bitcast());
        simd.relaxed_narrow_u32x4(low, high)
            .store_slice(&mut out[..8]);
    }

    #[inline]
    fn from_clipped(v: i32) -> Self {
        v as u16
    }
}

/// The eight samples of `samples` from `start`, zero past its end.
#[inline]
fn lanes8(samples: &[u16], start: usize) -> [u16; 8] {
    let rest = samples.get(start..).unwrap_or_default();
    rest.first_chunk::<8>().copied().unwrap_or_else(|| {
        let mut lanes = [0u16; 8];
        for (lane, &sample) in lanes.iter_mut().zip(rest) {
            *lane = sample;
        }
        lanes
    })
}

/// The first eight samples of `samples` as `i32` lanes.
#[inline]
fn widen_lanes<S: Simd>(simd: S, samples: &[u16]) -> i32x8<S> {
    let (low, high) = simd.widen_u16x8(u16x8::from_slice(simd, &samples[..8]));
    simd.combine_u32x4(low, high).bitcast()
}

/// Multiplies fixed-point values with adaptive down-shifting to avoid overflow.
#[must_use]
pub fn mul_fixed32_adapt(a: i32, b: i32, shift: u32) -> i32 {
    let (lhs, rhs, adjustment) = scaled_multiply_terms(a, b, shift);
    round_scaled_product(lhs.saturating_mul(rhs), adjustment)
}

fn round_scaled_product(product: i32, adjustment: i32) -> i32 {
    if adjustment <= 0 {
        product
    } else if adjustment > 29 {
        0
    } else {
        let n = adjustment as u32;
        let rounded = ((product.unsigned_abs() + (1 << (n - 1))) >> n) as i32;
        if product < 0 { -rounded } else { rounded }
    }
}

fn scaled_multiply_terms(a: i32, b: i32, shift: u32) -> (i32, i32, i32) {
    let overflow_bits = bit_width_abs(a)
        .saturating_add(bit_width_abs(b))
        .saturating_sub(29);
    let left_shift = overflow_bits / 2;
    let right_shift = overflow_bits - left_shift;
    (
        a >> left_shift,
        b >> right_shift,
        shift.min(i32::MAX as u32) as i32 - overflow_bits as i32,
    )
}

fn bit_width_abs(value: i32) -> u32 {
    let bits = u32::BITS - value.unsigned_abs().leading_zeros();
    bits.max(1)
}

fn ceil_log2_usize(value: usize) -> u32 {
    value
        .saturating_sub(1)
        .checked_ilog2()
        .map_or(0, |log| log + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_ten_bit_reference_set_uses_32_bit_equations() {
        let refs = MhccpRefs {
            width: 64,
            height: 64,
            reference_width: 64,
            reference_height: 64,
            above: 2,
            left: 2,
            luma: &[1023; 64 * 64],
        };

        let [params] = derive_mhccp_params(&refs, [&[1023; 64 * 64]], 0, BitDepth::Ten);
        assert_eq!(params, [-162, 64_418, 2_689]);
    }

    #[test]
    fn vector_row_matches_spec_formula() {
        let bit_depth = BitDepth::Ten;
        let luma: Vec<u16> = (0..37u16).map(|i| (i * 211 + 7) % 1024).collect();
        for params in [
            [70_000, -3_000, 40_000],
            [-262_143, 262_143, -1],
            [1 << 22, -(1 << 21), 1 << 20],
        ] {
            let predictor = MhccpPredictor::new(params, bit_depth);
            let rows: Vec<u16> = luma[1..].iter().chain(&luma[..36]).copied().collect();
            let refs = MhccpRefs {
                width: 36,
                height: 2,
                reference_width: 36,
                reference_height: 2,
                above: 1,
                left: 0,
                luma: &rows,
            };
            for width in [32, 36] {
                let mut out = vec![0u16; width];
                assert!(predictor.predict_block(&refs, 1, width, 1, &mut out, 36));
                for (i, &got) in out.iter().enumerate() {
                    let center = i32::from(luma[i]);
                    let vector = [i32::from(luma[i + 1]), round2_i32(center * center, 10), 512];
                    let t = (0..3).fold(0i32, |t, k| {
                        t.saturating_add(mul_fixed32_adapt(params[k], vector[k], MHCCP_BITS))
                    });
                    assert_eq!(
                        i32::from(got),
                        t.clamp(0, 1023),
                        "params {params:?} width {width} sample {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn no_edge_references_use_identity_offset() {
        let refs = MhccpRefs {
            width: 3,
            height: 3,
            reference_width: 3,
            reference_height: 3,
            above: 0,
            left: 0,
            luma: &[0; 9],
        };

        assert_eq!(
            derive_mhccp_params(&refs, [&[0; 9]], 0, BitDepth::Eight),
            [[0, 0, 1 << MHCCP_BITS]]
        );
    }

    #[test]
    fn parameter_derivation_ignores_uncaptured_prediction_padding() {
        let params = |padding| {
            let mut luma = vec![64; 16];
            let mut chroma = vec![96; 16];
            luma[12..].fill(padding);
            chroma[12..].fill(padding);
            let refs = MhccpRefs {
                width: 4,
                height: 4,
                reference_width: 4,
                reference_height: 3,
                above: 2,
                left: 2,
                luma: &luma,
            };
            derive_mhccp_params(&refs, [&chroma, &chroma], 0, BitDepth::Eight)
        };

        let [expected, v] = params(0);
        assert_ne!(expected, [0, 0, 1 << MHCCP_BITS]);
        assert_eq!(v, expected);
        assert_eq!(params(u8::MAX.into()), [expected; 2]);
    }
}
