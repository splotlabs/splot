// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.6 MHCCP parameter derivation helpers.

use fearless_simd::{Bytes as _, Select as _, Simd, SimdBase, i16x8, i32x8, u16x8};
use splot_core::simd::SimdBackend;

use crate::{BitDepth, ReconSample};

/// The AV2 § 7.13.6 MHCCP fixed-point precision, in bits.
pub const MHCCP_BITS: u32 = 16;
pub(crate) const MHCCP_PARAM_COUNT: usize = 3;
const DIV_PREC_BITS: u32 = 14;
const DIV_PREC_BITS_POW2: u32 = 8;
const DIV_SLOT_BITS: u32 = 3;
const DIV_INTR_BITS: u32 = DIV_PREC_BITS - DIV_SLOT_BITS;
#[doc = "AV2 § 7.13.6 `get_division_scale_shift` lookup tables."]
const DIVISION_POW2_W: [i32; 8] = [214, 153, 113, 86, 67, 53, 43, 35];
const DIVISION_POW2_O: [i32; 8] = [4822, 5952, 6624, 6792, 6408, 5424, 3792, 1466];
const DIVISION_POW2_B: [i32; 8] = [12784, 12054, 11670, 11583, 11764, 12195, 12870, 13782];

/// One edge of the AV2 § 7.13.7 reference samples of an MHCCP block: the
/// `v[0]` and the centre luma `CflRef[0][i][j]` of each sample, in Q3 (eight
/// times the luma reference). Both planes of a block share it.
#[derive(Clone, Copy, Default)]
pub struct MhccpEdge<'a> {
    /// `v[0]` of each sample, in Q3.
    pub linear: &'a [i16],
    /// The centre luma of each sample, in Q3, as many as [`Self::linear`].
    pub centre: &'a [i16],
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

#[derive(Clone, Copy)]
struct BaselineMhccp;

#[derive(Clone, Copy)]
struct TypedMhccp(SimdBackend);

trait MhccpRunner: Copy {
    fn gather_sums<const N: usize>(
        self,
        edges: [MhccpEdge<'_>; 2],
        chroma: [[&[u16]; 2]; N],
        bits: u32,
    ) -> (LumaSums, [[i32; 3]; N]);

    #[expect(clippy::too_many_arguments)]
    fn predict<P: PredictedSample, const N: usize, const WIDE: bool>(
        self,
        bits: u32,
        predictors: [&MhccpPredictor; N],
        linear: &[i16],
        centre: &[i16],
        width: usize,
        outs: [&mut [P]; N],
        strides: [usize; N],
    );
}

impl MhccpRunner for BaselineMhccp {
    #[inline]
    fn gather_sums<const N: usize>(
        self,
        edges: [MhccpEdge<'_>; 2],
        chroma: [[&[u16]; 2]; N],
        bits: u32,
    ) -> (LumaSums, [[i32; 3]; N]) {
        baseline_simd!(simd => gather_mhccp_sums(simd, edges, chroma, bits))
    }

    #[inline]
    fn predict<P: PredictedSample, const N: usize, const WIDE: bool>(
        self,
        bits: u32,
        predictors: [&MhccpPredictor; N],
        linear: &[i16],
        centre: &[i16],
        width: usize,
        outs: [&mut [P]; N],
        strides: [usize; N],
    ) {
        baseline_simd!(
            simd => predict_rows::<_, _, N, WIDE>(simd, bits, predictors, linear, centre, width, outs, strides)
        );
    }
}

impl MhccpRunner for TypedMhccp {
    #[inline]
    fn gather_sums<const N: usize>(
        self,
        edges: [MhccpEdge<'_>; 2],
        chroma: [[&[u16]; 2]; N],
        bits: u32,
    ) -> (LumaSums, [[i32; 3]; N]) {
        splot_core::with_simd!(self.0, simd => gather_mhccp_sums(simd, edges, chroma, bits))
    }

    #[inline]
    fn predict<P: PredictedSample, const N: usize, const WIDE: bool>(
        self,
        bits: u32,
        predictors: [&MhccpPredictor; N],
        linear: &[i16],
        centre: &[i16],
        width: usize,
        outs: [&mut [P]; N],
        strides: [usize; N],
    ) {
        splot_core::with_simd!(self.0, simd => {
            predict_rows::<_, _, N, WIDE>(
                simd,
                bits,
                predictors,
                linear,
                centre,
                width,
                outs,
                strides,
            );
        });
    }
}

#[inline]
fn gather_mhccp_sums<S: Simd, const N: usize>(
    simd: S,
    edges: [MhccpEdge<'_>; 2],
    chroma: [[&[u16]; 2]; N],
    bits: u32,
) -> (LumaSums, [[i32; 3]; N]) {
    let mut luma = LumaSums::default();
    let mut cross = [[0i32; 3]; N];
    let mut lanes = LaneSums::new(simd);
    for (e, edge) in edges.iter().enumerate() {
        let len = edge.linear.len().min(edge.centre.len());
        for at in (0..len).step_by(8) {
            let n = (len - at).min(8);
            let targets = chroma.map(|plane| masked_lanes(simd, lanes8(plane[e], at), n));
            let [linear, centre] =
                [edge.linear, edge.centre].map(|luma| masked_lanes(simd, lanes8(luma, at), n) >> 3);
            lanes.add(linear, centre, targets, n, bits);
        }
    }
    lanes.reduce_into(simd, &mut luma, &mut cross);
    (luma, cross)
}

fn derive_mhccp_params_with_runner<R: MhccpRunner, const N: usize>(
    runner: R,
    edges: [MhccpEdge<'_>; 2],
    chroma: [[&[u16]; 2]; N],
    bit_depth: BitDepth,
) -> [[i32; MHCCP_PARAM_COUNT]; N] {
    let bits = u32::from(bit_depth.bits());
    let (luma, cross) = runner.gather_sums(edges, chroma, bits);
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

/// Derives the three MHCCP model parameters of each chroma plane in
/// `chroma` from the § 7.13.7 reference samples of `edges` (above, then
/// left), whose targets `CflRef[1]` are `chroma[plane][edge]`, at least as
/// long as the edge; the luma statistics are gathered once for all planes.
#[must_use]
pub fn derive_mhccp_params<const N: usize>(
    edges: [MhccpEdge<'_>; 2],
    chroma: [[&[u16]; 2]; N],
    bit_depth: BitDepth,
) -> [[i32; MHCCP_PARAM_COUNT]; N] {
    derive_mhccp_params_with_runner(BaselineMhccp, edges, chroma, bit_depth)
}

/// [`derive_mhccp_params`] using caller-selected `backend`.
#[must_use]
pub fn derive_mhccp_params_with<const N: usize>(
    backend: SimdBackend,
    edges: [MhccpEdge<'_>; 2],
    chroma: [[&[u16]; 2]; N],
    bit_depth: BitDepth,
) -> [[i32; MHCCP_PARAM_COUNT]; N] {
    derive_mhccp_params_with_runner(TypedMhccp(backend), edges, chroma, bit_depth)
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
    fn reduce_into(self, simd: S, luma: &mut LumaSums, cross: &mut [[i32; 3]; N]) {
        let reduce = |lanes: i32x8<S>| {
            let (low, high) = simd.split_i32x8(lanes);
            simd.reduce_sum_i32x4(low + high)
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

/// The first `count` (at most 8) of `lanes` as `i32` lanes, the rest zero.
#[inline]
fn masked_lanes<S: Simd, L: WideLanes>(simd: S, lanes: [L; 8], count: usize) -> i32x8<S> {
    let lanes = L::widen(simd, lanes);
    if count == 8 {
        return lanes;
    }
    let index = i32x8::from_slice(simd, &[0, 1, 2, 3, 4, 5, 6, 7]);
    index
        .simd_lt(i32x8::splat(simd, count as i32))
        .select(lanes, i32x8::splat(simd, 0))
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
#[expect(
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

    /// Predicts the block of centre luma `centre` (rows `width` wide: 4 or a
    /// multiple of 8) from its `v[0]` luma `linear`, both in Q3, into `out`,
    /// row `r` at `out[r * stride..]`; `false`, writing nothing, when `out`
    /// or `linear` is too short or the block shape is not one MHCCP codes.
    #[must_use]
    pub fn predict_block<T: ReconSample>(
        &self,
        linear: &[i16],
        centre: &[i16],
        width: usize,
        out: &mut [T],
        stride: usize,
    ) -> bool {
        Self::predict_planes([self], linear, centre, width, [(out, stride)])
    }

    /// [`Self::predict_block`] using caller-selected `backend`.
    #[must_use]
    pub fn predict_block_with<T: ReconSample>(
        &self,
        backend: SimdBackend,
        linear: &[i16],
        centre: &[i16],
        width: usize,
        out: &mut [T],
        stride: usize,
    ) -> bool {
        Self::predict_planes_with(backend, [self], linear, centre, width, [(out, stride)])
    }

    /// [`Self::predict_block`] of the `N` planes of `predictors`, which share
    /// the block's luma, in one pass over it.
    #[must_use]
    pub fn predict_planes<T: ReconSample, const N: usize>(
        predictors: [&Self; N],
        linear: &[i16],
        centre: &[i16],
        width: usize,
        outs: [(&mut [T], usize); N],
    ) -> bool {
        Self::predict_planes_with_runner(BaselineMhccp, predictors, linear, centre, width, outs)
    }

    /// [`Self::predict_planes`] using caller-selected `backend`.
    #[must_use]
    pub fn predict_planes_with<T: ReconSample, const N: usize>(
        backend: SimdBackend,
        predictors: [&Self; N],
        linear: &[i16],
        centre: &[i16],
        width: usize,
        outs: [(&mut [T], usize); N],
    ) -> bool {
        Self::predict_planes_with_runner(
            TypedMhccp(backend),
            predictors,
            linear,
            centre,
            width,
            outs,
        )
    }

    fn predict_planes_with_runner<R: MhccpRunner, T: ReconSample, const N: usize>(
        runner: R,
        predictors: [&Self; N],
        linear: &[i16],
        centre: &[i16],
        width: usize,
        outs: [(&mut [T], usize); N],
    ) -> bool {
        let height = centre.len() / width.max(1);
        if height == 0
            || height * width != centre.len()
            || !(width.is_multiple_of(8) || (width == 4 && height.is_multiple_of(2)))
            || linear.len() < centre.len()
            || outs
                .iter()
                .any(|(out, stride)| (height - 1) * stride + width > out.len())
        {
            return false;
        }
        let wide = predictors.iter().any(|predictor| !predictor.narrow);
        let strides = outs.each_ref().map(|out| out.1);
        let bits = predictors.first().map_or(8, |predictor| predictor.bits);
        if T::u16_slice(&[T::default()]).is_some() {
            let outs = outs.map(|out| T::u16_slice_mut(out.0).unwrap_or_default());
            if wide {
                runner
                    .predict::<_, N, true>(bits, predictors, linear, centre, width, outs, strides);
            } else {
                runner
                    .predict::<_, N, false>(bits, predictors, linear, centre, width, outs, strides);
            }
        } else {
            let outs = outs.map(|out| T::u8_slice_mut(out.0).unwrap_or_default());
            if wide {
                runner
                    .predict::<_, N, true>(bits, predictors, linear, centre, width, outs, strides);
            } else {
                runner
                    .predict::<_, N, false>(bits, predictors, linear, centre, width, outs, strides);
            }
        }
        true
    }

    /// [`Self::predict_block`] sample by sample: the oracle of the lanes.
    #[cfg(test)]
    fn predict_wide<T: ReconSample>(
        &self,
        linear: &[i16],
        centre: &[i16],
        width: usize,
        out: &mut [T],
        stride: usize,
    ) {
        let rows = linear.chunks_exact(width).zip(centre.chunks_exact(width));
        for ((linear, centre), out) in rows.zip(out.chunks_mut(stride)) {
            for ((out, &linear), &centre) in out.iter_mut().zip(linear).zip(centre) {
                let centre = i32::from(centre >> 3);
                let square = (centre * centre + (1 << (self.bits - 1))) >> self.bits;
                let t = mul_fixed32_adapt(self.params[0], i32::from(linear >> 3), MHCCP_BITS)
                    .saturating_add(mul_fixed32_adapt(self.params[1], square, MHCCP_BITS))
                    .saturating_add(self.offset);
                *out = T::try_from_u16(t.clamp(0, self.max) as u16).unwrap_or_default();
            }
        }
    }
}

/// The parameter lanes of one plane.
#[derive(Clone, Copy)]
struct PlaneLanes<S: Simd> {
    params: [i32x8<S>; 2],
    /// `GetMsb(Abs(param)) + 1` of each data parameter, when one of them is
    /// wide enough that `mul_fixed32_adapt` may down-shift.
    wide: Option<[i32; 2]>,
    offset: i32x8<S>,
    max: i32x8<S>,
}

impl<S: Simd> PlaneLanes<S> {
    /// Writes eight § 7.13.6 samples from their shared luma terms.
    #[expect(
        clippy::inline_always,
        reason = "keeps the parameter lanes in registers"
    )]
    #[inline(always)]
    fn store8<P: PredictedSample, const WIDE: bool>(
        self,
        simd: S,
        out: &mut [P],
        [linear, square]: [i32x8<S>; 2],
    ) {
        let t = match self.wide.filter(|_| WIDE) {
            None => {
                let round =
                    |x: i32x8<S>| (x + (1i32 << (MHCCP_BITS - 1)) + (x >> 31)) >> MHCCP_BITS;
                round(linear * self.params[0]) + round(square * self.params[1])
            }
            Some([bits0, bits1]) => {
                adapt_lanes(simd, self.params[0], bits0, linear)
                    + adapt_lanes(simd, self.params[1], bits1, square)
            }
        };
        P::store8(
            simd,
            out,
            (t + self.offset).max(i32x8::splat(simd, 0)).min(self.max),
        );
    }
}

/// `mul_fixed32_adapt(param, v, MHCCP_BITS)` of each lane `v` in
/// `0..1 << 10`, with `bits` the bit width of `param`: the bit width of `v`
/// is the exponent of `v` as `f32`, exact below `2^24`. Each term then has
/// at most 29 bits, so every product and the sum of the three § 7.13.6
/// terms fit in `i32`.
#[inline]
fn adapt_lanes<S: Simd>(simd: S, param: i32x8<S>, bits: i32, v: i32x8<S>) -> i32x8<S> {
    splot_core::in_scope!(simd => {
        let zero = i32x8::splat(simd, 0);
        let exponent = simd
            .cvt_f32_i32x8(v.max(i32x8::splat(simd, 1)))
            .bitcast::<i32x8<S>>()
            >> 23;
        let need = (exponent + (bits - 126 - 29)).max(zero);
        let s1 = need >> 1;
        let product = simd.shrv_i32x8(param, s1) * simd.shrv_i32x8(v, need - s1);
        let adjust = i32x8::splat(simd, MHCCP_BITS as i32) - need;
        let half = simd.shlv_i32x8(i32x8::splat(simd, 1), (adjust - 1).max(zero));
        let rounded = simd.shrv_i32x8(product.abs() + half, adjust.max(zero));
        let rounded = product.simd_lt(zero).select(zero - rounded, rounded);
        adjust.simd_gt(zero).select(rounded, product)
    })
}

/// [`MhccpPredictor::predict_planes`] in 8-lane groups, two rows each for
/// a 4-wide block: each group's `vec[0]` and `vec[1]` serve every plane.
#[expect(clippy::too_many_arguments)]
#[inline]
fn predict_rows<S: Simd, P: PredictedSample, const N: usize, const WIDE: bool>(
    simd: S,
    bits: u32,
    predictors: [&MhccpPredictor; N],
    linear: &[i16],
    centre: &[i16],
    width: usize,
    mut outs: [&mut [P]; N],
    strides: [usize; N],
) {
    let terms = |linear: &[i16; 8], centre: &[i16; 8]| {
        let centre = <i16 as WideLanes>::widen(simd, *centre) >> 3;
        let square = (centre * centre + (1i32 << (bits - 1))) >> bits;
        [<i16 as WideLanes>::widen(simd, *linear) >> 3, square]
    };
    let lanes = predictors.map(|predictor| PlaneLanes {
        params: [0, 1].map(|k| i32x8::splat(simd, predictor.params[k])),
        wide: (!predictor.narrow)
            .then(|| [0, 1].map(|k| bit_width_abs(predictor.params[k]) as i32)),
        offset: i32x8::splat(simd, predictor.offset),
        max: i32x8::splat(simd, predictor.max),
    });
    if width == 4 {
        let groups = linear
            .as_chunks::<8>()
            .0
            .iter()
            .zip(centre.as_chunks::<8>().0);
        for (pair, (linear, centre)) in groups.enumerate() {
            let terms = terms(linear, centre);
            for ((lanes, out), stride) in lanes.iter().zip(&mut outs).zip(strides) {
                let mut rows = [P::from_clipped(0); 8];
                lanes.store8::<P, WIDE>(simd, &mut rows, terms);
                let start = 2 * pair * stride;
                out[start..start + 4].copy_from_slice(&rows[..4]); // splot-copy-ok: publish a 4-wide MHCCP row
                out[start + stride..start + stride + 4].copy_from_slice(&rows[4..]); // splot-copy-ok: publish a 4-wide MHCCP row
            }
        }
        return;
    }
    let rows = linear.chunks_exact(width).zip(centre.chunks_exact(width));
    for (row, (linear, centre)) in rows.enumerate() {
        let groups = linear
            .as_chunks::<8>()
            .0
            .iter()
            .zip(centre.as_chunks::<8>().0);
        for (group, (linear, centre)) in groups.enumerate() {
            let terms = terms(linear, centre);
            for ((lanes, out), stride) in lanes.iter().zip(&mut outs).zip(strides) {
                let at = row * stride + 8 * group;
                lanes.store8::<P, WIDE>(simd, &mut out[at..at + 8], terms);
            }
        }
    }
}

/// A predicted sample type (`u8` or `u16`) that MHCCP and CfL lanes store into.
pub trait PredictedSample: ReconSample {
    /// Stores eight samples already clipped to the sample range.
    fn store8<S: Simd>(simd: S, out: &mut [Self], v: i32x8<S>);
    /// A value already clipped to the sample range.
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
fn lanes8<L: WideLanes>(samples: &[L], start: usize) -> [L; 8] {
    let rest = samples.get(start..).unwrap_or_default();
    rest.first_chunk::<8>().copied().unwrap_or_else(|| {
        let mut lanes = [L::default(); 8];
        for (lane, &sample) in lanes.iter_mut().zip(rest) {
            *lane = sample;
        }
        lanes
    })
}

/// A 16-bit lane type the MHCCP sums widen to `i32` lanes.
trait WideLanes: Copy + Default {
    fn widen<S: Simd>(simd: S, lanes: [Self; 8]) -> i32x8<S>;
}

impl WideLanes for u16 {
    #[inline]
    fn widen<S: Simd>(simd: S, lanes: [Self; 8]) -> i32x8<S> {
        let (low, high) = simd.widen_u16x8(u16x8::from_slice(simd, &lanes));
        simd.combine_u32x4(low, high).bitcast()
    }
}

impl WideLanes for i16 {
    #[inline]
    fn widen<S: Simd>(simd: S, lanes: [Self; 8]) -> i32x8<S> {
        let (low, high) = simd.widen_i16x8(i16x8::from_slice(simd, &lanes));
        simd.combine_i32x4(low, high)
    }
}

/// Multiplies fixed-point values with adaptive down-shifting to avoid
/// overflow: the shifted terms have at most 29 bits in all, so their
/// product is exact in `i32`.
#[must_use]
pub fn mul_fixed32_adapt(a: i32, b: i32, shift: u32) -> i32 {
    let (lhs, rhs, adjustment) = scaled_multiply_terms(a, b, shift);
    round_scaled_product(lhs.wrapping_mul(rhs), adjustment)
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
    use crate::math::round2_i32;

    fn lcg(state: &mut u64) -> u32 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (*state >> 33) as u32
    }

    /// Q3 luma of `CflRef[0]` value `v`, with random fraction bits.
    fn q3(v: u32, state: &mut u64) -> i16 {
        (8 * v + lcg(state) % 8) as i16
    }

    /// The § 7.13.7 statistics and normalization sample by sample, then the
    /// shared § 7.13.8 solve.
    fn spec_params(
        edges: [(&[i16], &[i16]); 2],
        chroma: &[[&[u16]; 2]],
        bit_depth: BitDepth,
    ) -> Vec<[i32; 3]> {
        let bits = u32::from(bit_depth.bits());
        let mid = 1i32 << (bits - 1);
        chroma
            .iter()
            .map(|plane| {
                let mut equations = NormalEquations::default();
                let mut count = 0usize;
                for (e, (linear, centre)) in edges.iter().enumerate() {
                    for (k, (&linear, &centre)) in linear.iter().zip(*centre).enumerate() {
                        let centre = i32::from(centre >> 3);
                        let v = [
                            i32::from(linear >> 3),
                            round2_i32(centre * centre, bits),
                            mid,
                        ];
                        let target = i32::from(plane[e][k]);
                        for i0 in 0..3 {
                            for i1 in i0..3 {
                                equations.ata[i0][i1] =
                                    equations.ata[i0][i1].wrapping_add(v[i0] * v[i1]);
                            }
                            equations.b[i0] = equations.b[i0].wrapping_add(v[i0] * target);
                        }
                        count += 1;
                    }
                }
                if count == 0 {
                    return None;
                }
                let shift = MHCCP_BITS as i32 + 6 - 2 * bits as i32 - ceil_log2_usize(count) as i32;
                equations.apply_matrix_shift(shift);
                Some(equations)
            })
            .map(|equations| match equations {
                Some(equations) => solve_mhccp(&[equations], bit_depth)[0],
                None => [0, 0, 1 << MHCCP_BITS],
            })
            .collect()
    }

    #[test]
    fn maximum_ten_bit_reference_set_uses_32_bit_equations() {
        let luma = [1023 * 8; 62];
        let chroma = [1023; 62];
        let edges = [
            MhccpEdge {
                linear: &luma,
                centre: &luma,
            },
            MhccpEdge {
                linear: &luma[..61],
                centre: &luma[..61],
            },
        ];

        let [params] = derive_mhccp_params(edges, [[&chroma, &chroma]], BitDepth::Ten);
        assert_eq!(params, [-162, 64_418, 2_689]);
    }

    #[test]
    fn lane_sums_match_spec_statistics_over_random_edges() {
        let mut state = 0x5eed;
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let max = u32::from(bit_depth.max_sample());
            for round in 0..400 {
                let lens = [lcg(&mut state) as usize % 70, lcg(&mut state) as usize % 70];
                let extreme = round % 4 == 0;
                let sample = |state: &mut u64| {
                    if extreme {
                        max * (lcg(state) % 2)
                    } else {
                        lcg(state) % (max + 1)
                    }
                };
                let luma: Vec<[Vec<i16>; 2]> = lens
                    .iter()
                    .map(|&len| {
                        core::array::from_fn(|_| {
                            (0..len)
                                .map(|_| q3(sample(&mut state), &mut state))
                                .collect()
                        })
                    })
                    .collect();
                let chroma: Vec<[Vec<u16>; 2]> = (0..2)
                    .map(|_| {
                        core::array::from_fn(|e| {
                            (0..lens[e] + 3)
                                .map(|_| sample(&mut state) as u16)
                                .collect()
                        })
                    })
                    .collect();
                let edges = [0, 1].map(|e| MhccpEdge {
                    linear: &luma[e][0],
                    centre: &luma[e][1],
                });
                let planes: Vec<[&[u16]; 2]> = chroma
                    .iter()
                    .map(|plane| [plane[0].as_slice(), plane[1].as_slice()])
                    .collect();
                let got = derive_mhccp_params(edges, [planes[0], planes[1]], bit_depth);
                let spec = spec_params(
                    [0, 1].map(|e| (luma[e][0].as_slice(), luma[e][1].as_slice())),
                    &planes,
                    bit_depth,
                );
                assert_eq!(got.as_slice(), spec, "{bit_depth:?} lengths {lens:?}");
            }
        }
    }

    #[test]
    fn predicted_blocks_match_spec_formula() {
        let mut state = 0xb10c;
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let bits = u32::from(bit_depth.bits());
            let max = u32::from(bit_depth.max_sample());
            for params in [
                [70_000, -3_000, 40_000],
                [-262_143, 262_143, -1],
                [1 << 22, -(1 << 21), 1 << 20],
                [-(1 << 18), 1 << 17, 1 << 16],
            ] {
                let predictor = MhccpPredictor::new(params, bit_depth);
                for (width, height) in [(4, 4), (4, 8), (8, 4), (16, 8), (32, 32), (8, 32)] {
                    let len = width * height;
                    let linear: Vec<i16> = (0..len)
                        .map(|_| q3(lcg(&mut state) % (max + 1), &mut state))
                        .collect();
                    let centre: Vec<i16> = (0..len)
                        .map(|_| q3(lcg(&mut state) % (max + 1), &mut state))
                        .collect();
                    let stride = width + 3;
                    let mut out = vec![0u16; (height - 1) * stride + width];
                    let mut out8 = vec![0u8; out.len()];
                    assert!(predictor.predict_block(&linear, &centre, width, &mut out, stride));
                    if bits == 8 {
                        assert!(
                            predictor.predict_block(&linear, &centre, width, &mut out8, stride)
                        );
                    }
                    let other =
                        MhccpPredictor::new([params[1], params[0], params[2] / 2], bit_depth);
                    let (mut first, mut second) =
                        (vec![0u16; out.len()], vec![0u16; 2 * width * height]);
                    let mut alone = second.clone();
                    let outs = [
                        (first.as_mut_slice(), stride),
                        (second.as_mut_slice(), 2 * width),
                    ];
                    assert!(MhccpPredictor::predict_planes(
                        [&predictor, &other],
                        &linear,
                        &centre,
                        width,
                        outs
                    ));
                    assert!(other.predict_block(&linear, &centre, width, &mut alone, 2 * width));
                    assert_eq!(
                        (&first, &second),
                        (&out, &alone),
                        "{params:?} {width}x{height} pair"
                    );
                    for i in 0..len {
                        let c = i32::from(centre[i] >> 3);
                        let v = [
                            i32::from(linear[i] >> 3),
                            round2_i32(c * c, bits),
                            1 << (bits - 1),
                        ];
                        let t = (0..3).fold(0i32, |t, k| {
                            t.saturating_add(mul_fixed32_adapt(params[k], v[k], MHCCP_BITS))
                        });
                        let at = (i / width) * stride + i % width;
                        let expected = t.clamp(0, max as i32);
                        assert_eq!(
                            i32::from(out[at]),
                            expected,
                            "{params:?} {width}x{height} {i}"
                        );
                        if bits == 8 {
                            assert_eq!(
                                i32::from(out8[at]),
                                expected,
                                "{params:?} {width}x{height} {i}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn wide_parameter_lanes_match_the_per_sample_oracle() {
        let mut state = 0x31de;
        let mut wide = 0;
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let max = u32::from(bit_depth.max_sample());
            for round in 0..300 {
                let param =
                    |state: &mut u64| {
                        let magnitude = lcg(state) >> (lcg(state) % 31);
                        let value = (magnitude as i32)
                            .wrapping_mul(if lcg(state).is_multiple_of(2) { 1 } else { -1 });
                        if round % 7 == 0 {
                            [i32::MIN, i32::MAX, 1 << 21, -(1 << 23)][lcg(state) as usize % 4]
                        } else {
                            value
                        }
                    };
                let predictor = MhccpPredictor::new(
                    [param(&mut state), param(&mut state), param(&mut state)],
                    bit_depth,
                );
                wide += usize::from(!predictor.narrow);
                for (width, height) in [
                    (4, 4),
                    (4, 8),
                    (8, 4),
                    (4, 16),
                    (16, 4),
                    (8, 8),
                    (16, 16),
                    (32, 8),
                    (8, 32),
                    (32, 32),
                ] {
                    let len = width * height;
                    let sample = |state: &mut u64| match round % 3 {
                        0 => q3(max * (lcg(state) % 2), state),
                        _ => q3(lcg(state) % (max + 1), state),
                    };
                    let linear: Vec<i16> = (0..len).map(|_| sample(&mut state)).collect();
                    let centre: Vec<i16> = (0..len).map(|_| sample(&mut state)).collect();
                    let stride = width + 5;
                    let mut lanes = vec![0u16; (height - 1) * stride + width];
                    let mut expected = lanes.clone();
                    assert!(predictor.predict_block(&linear, &centre, width, &mut lanes, stride));
                    predictor.predict_wide(&linear, &centre, width, &mut expected, stride);
                    assert_eq!(lanes, expected, "{:?} {width}x{height}", predictor.params);
                    if bit_depth == BitDepth::Eight {
                        let mut bytes = vec![0u8; lanes.len()];
                        assert!(
                            predictor.predict_block(&linear, &centre, width, &mut bytes, stride)
                        );
                        assert!(
                            bytes
                                .iter()
                                .zip(&expected)
                                .all(|(&b, &e)| u16::from(b) == e)
                        );
                    }
                }
            }
        }
        assert!(wide > 300, "{wide}");
    }

    #[test]
    fn predict_block_rejects_shapes_mhccp_does_not_code() {
        let predictor = MhccpPredictor::new([0, 0, 1 << MHCCP_BITS], BitDepth::Eight);
        let luma = [0i16; 64];
        let mut out = [0u8; 64];
        assert!(!predictor.predict_block(&luma[..12], &luma[..12], 4, &mut out, 4));
        assert!(!predictor.predict_block(&luma[..24], &luma[..24], 12, &mut out, 12));
        assert!(!predictor.predict_block(&luma[..15], &luma[..16], 4, &mut out, 4));
        assert!(!predictor.predict_block(&luma[..32], &luma[..32], 8, &mut out[..31], 8));
        assert_eq!(out, [0; 64]);
    }

    #[test]
    fn no_edge_references_use_identity_offset() {
        assert_eq!(
            derive_mhccp_params([MhccpEdge::default(); 2], [[&[], &[]]], BitDepth::Eight),
            [[0, 0, 1 << MHCCP_BITS]]
        );
    }

    #[test]
    fn typed_backend_matches_compatibility_routes() {
        use splot_core::simd::{SimdBackend, SimdPolicy, resolve_backend};

        fn check(backend: SimdBackend) {
            for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
                let max = bit_depth.max_sample() as i16;
                let lin_a: Vec<i16> = (0..18).map(|i| ((i * 5) % max) << 3).collect();
                let cen_a: Vec<i16> = (0..18).map(|i| ((i * 7) % max) << 3).collect();
                let lin_l: Vec<i16> = (0..12).map(|i| ((i * 9) % max) << 3).collect();
                let cen_l: Vec<i16> = (0..12).map(|i| ((i * 11) % max) << 3).collect();
                let edges = [
                    MhccpEdge {
                        linear: &lin_a,
                        centre: &cen_a,
                    },
                    MhccpEdge {
                        linear: &lin_l,
                        centre: &cen_l,
                    },
                ];
                let cu0: Vec<u16> = (0..18)
                    .map(|i| (i as u16 * 13 + 7) % (max as u16 + 1))
                    .collect();
                let cu1: Vec<u16> = (0..12)
                    .map(|i| (i as u16 * 17 + 11) % (max as u16 + 1))
                    .collect();
                let cv0: Vec<u16> = (0..18)
                    .map(|i| (i as u16 * 19 + 3) % (max as u16 + 1))
                    .collect();
                let cv1: Vec<u16> = (0..12)
                    .map(|i| (i as u16 * 23 + 5) % (max as u16 + 1))
                    .collect();
                let compat_p = derive_mhccp_params(edges, [[&cu0, &cu1], [&cv0, &cv1]], bit_depth);
                let typed_p = derive_mhccp_params_with(
                    backend,
                    edges,
                    [[&cu0, &cu1], [&cv0, &cv1]],
                    bit_depth,
                );
                assert_eq!(typed_p, compat_p);

                let narrow_pred = MhccpPredictor::new([512, -256, 65536], bit_depth);
                let wide_pred = MhccpPredictor::new([32_768, 16_384, 65536], bit_depth);
                let (w, h, stride) = (16, 4, 20);
                let lin_blk: Vec<i16> = (0..w * h)
                    .map(|i| ((i as i16 * 13 + 3) % max) << 3)
                    .collect();
                let cen_blk: Vec<i16> = (0..w * h)
                    .map(|i| ((i as i16 * 17 + 9) % max) << 3)
                    .collect();
                if bit_depth == BitDepth::Eight {
                    let (mut compat_a, mut typed_a) =
                        (vec![0u8; stride * h], vec![0u8; stride * h]);
                    assert!(narrow_pred.predict_block(
                        &lin_blk,
                        &cen_blk,
                        w,
                        &mut compat_a,
                        stride
                    ));
                    assert!(narrow_pred.predict_block_with(
                        backend,
                        &lin_blk,
                        &cen_blk,
                        w,
                        &mut typed_a,
                        stride
                    ));
                    assert_eq!(typed_a, compat_a);
                    let (mut cu, mut cv) = (vec![0u8; stride * h], vec![0u8; stride * h]);
                    let (mut tu, mut tv) = (vec![0u8; stride * h], vec![0u8; stride * h]);
                    assert!(MhccpPredictor::predict_planes(
                        [&narrow_pred, &wide_pred],
                        &lin_blk,
                        &cen_blk,
                        w,
                        [(&mut cu, stride), (&mut cv, stride)],
                    ));
                    assert!(MhccpPredictor::predict_planes_with(
                        backend,
                        [&narrow_pred, &wide_pred],
                        &lin_blk,
                        &cen_blk,
                        w,
                        [(&mut tu, stride), (&mut tv, stride)],
                    ));
                    assert_eq!(tu, cu);
                    assert_eq!(tv, cv);
                } else {
                    let (mut compat_a, mut typed_a) =
                        (vec![0u16; stride * h], vec![0u16; stride * h]);
                    assert!(wide_pred.predict_block(&lin_blk, &cen_blk, w, &mut compat_a, stride));
                    assert!(wide_pred.predict_block_with(
                        backend,
                        &lin_blk,
                        &cen_blk,
                        w,
                        &mut typed_a,
                        stride
                    ));
                    assert_eq!(typed_a, compat_a);
                    let (mut cu, mut cv) = (vec![0u16; stride * h], vec![0u16; stride * h]);
                    let (mut tu, mut tv) = (vec![0u16; stride * h], vec![0u16; stride * h]);
                    assert!(MhccpPredictor::predict_planes(
                        [&narrow_pred, &wide_pred],
                        &lin_blk,
                        &cen_blk,
                        w,
                        [(&mut cu, stride), (&mut cv, stride)],
                    ));
                    assert!(MhccpPredictor::predict_planes_with(
                        backend,
                        [&narrow_pred, &wide_pred],
                        &lin_blk,
                        &cen_blk,
                        w,
                        [(&mut tu, stride), (&mut tv, stride)],
                    ));
                    assert_eq!(tu, cu);
                    assert_eq!(tv, cv);
                }
            }
        }

        check(
            resolve_backend(SimdPolicy::Auto)
                .expect("auto backend")
                .backend(),
        );
    }
}
