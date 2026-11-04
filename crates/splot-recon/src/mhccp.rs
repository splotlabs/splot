// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.6 MHCCP parameter derivation helpers.

use std::simd::cmp::{SimdOrd, SimdPartialOrd};
use std::simd::num::{SimdInt, SimdUint};
use std::simd::{Select, Simd};

use crate::BitDepth;
use crate::math::{round2_i32, round2_signed_i32};

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

/// Reference samples used to derive AV2 § 7.13.6 MHCCP parameters.
pub struct MhccpRefs {
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
    pub luma: Vec<u16>,
    /// Chroma reference samples.
    pub chroma: Vec<u16>,
}

impl MhccpRefs {
    fn has_edge_refs(&self) -> bool {
        self.above != 0 || self.left != 0
    }
}

const UPPER_TRIANGLE: [(usize, usize); 6] = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];

#[derive(Clone, Copy, Debug, Default)]
struct NormalEquations {
    ata: [[i32; MHCCP_PARAM_COUNT]; MHCCP_PARAM_COUNT],
    b: [i32; MHCCP_PARAM_COUNT],
    samples: usize,
}

impl NormalEquations {
    /// The § 7.13.7 statistics with `v[2]` fixed at `mid`. Each product of
    /// two `BitDepth`-bit samples is below `2^(2 * BitDepth)`, so the sums of
    /// at most `2^(31 - 2 * BitDepth)` samples are exact in `i32`.
    fn from_sums(sums: &EdgeSums, mid: i32) -> Self {
        let count = sums.count as i32;
        Self {
            ata: [
                [sums.s00, sums.s01, sums.s0.wrapping_mul(mid)],
                [0, sums.s11, sums.s1.wrapping_mul(mid)],
                [0, 0, count.wrapping_mul(mid).wrapping_mul(mid)],
            ],
            b: [sums.s0t, sums.s1t, sums.st.wrapping_mul(mid)],
            samples: sums.count,
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

/// Derives the three MHCCP model parameters for a reference sample set.
#[must_use]
pub fn derive_mhccp_params(
    refs: &MhccpRefs,
    mh_dir: u8,
    bit_depth: BitDepth,
) -> [i32; MHCCP_PARAM_COUNT] {
    let bits = u32::from(bit_depth.bits());
    let mut sums = EdgeSums::default();
    if refs.has_edge_refs() {
        let stride = refs.width;
        let reference_width = refs.reference_width.min(refs.width);
        let reference_height = refs.reference_height.min(refs.height);
        for row in 1..reference_height.saturating_sub(1) {
            let cols = if row < refs.above {
                reference_width.saturating_sub(1)
            } else {
                refs.left.min(reference_width.saturating_sub(1))
            };
            if cols <= 1 {
                continue;
            }
            let start = row * stride;
            let linear_start = match mh_dir {
                0 => start + 1,
                1 => start - stride + 1,
                _ => start,
            };
            let linear = &refs.luma[linear_start..linear_start + cols - 1];
            let center = &refs.luma[start + 1..start + cols];
            let target = &refs.chroma[start + 1..start + cols];
            if cols > 8 {
                sums.add_run(linear, center, target, bits);
            } else {
                for ((&linear, &center), &target) in linear.iter().zip(center).zip(target) {
                    sums.add(linear, center, target, bits);
                }
            }
        }
    }
    let mut equations = NormalEquations::from_sums(&sums, 1i32 << (bits - 1));
    if equations.samples == 0 {
        return [0, 0, 1i32 << MHCCP_BITS];
    }
    let bit_depth_bits = i32::from(bit_depth.bits());
    let matrix_shift =
        MHCCP_BITS as i32 + 6 - 2 * bit_depth_bits - ceil_log2_usize(equations.samples) as i32;
    equations.apply_matrix_shift(matrix_shift);
    solve_mhccp(equations, bit_depth)
}

/// Wrapping sums of the § 7.13.7 products over the edge samples.
#[derive(Debug, Default)]
struct EdgeSums {
    s00: i32,
    s01: i32,
    s11: i32,
    s0: i32,
    s1: i32,
    s0t: i32,
    s1t: i32,
    st: i32,
    count: usize,
}

impl EdgeSums {
    /// Adds one row run of `v[0]` (`linear`), centre luma and target chroma.
    fn add_run(&mut self, linear: &[u16], center: &[u16], target: &[u16], bits: u32) {
        let half16 = Simd::<i32, 16>::splat(1 << (bits - 1));
        let shift16 = Simd::<i32, 16>::splat(bits as i32);
        let mut acc16 = [Simd::<i32, 16>::splat(0); 8];
        let mut done = 0;
        while linear.len() - done >= 16 {
            let v0 = Simd::<u16, 16>::from_slice(&linear[done..done + 16]).cast::<i32>();
            let c = Simd::<u16, 16>::from_slice(&center[done..done + 16]).cast::<i32>();
            let t = Simd::<u16, 16>::from_slice(&target[done..done + 16]).cast::<i32>();
            let v1 = (c * c + half16) >> shift16;
            for (acc, term) in
                acc16
                    .iter_mut()
                    .zip([v0 * v0, v0 * v1, v1 * v1, v0, v1, v0 * t, v1 * t, t])
            {
                *acc += term;
            }
            done += 16;
        }
        let half8 = Simd::<i32, 8>::splat(1 << (bits - 1));
        let shift8 = Simd::<i32, 8>::splat(bits as i32);
        let mut acc8 = [Simd::<i32, 8>::splat(0); 8];
        while linear.len() - done >= 8 {
            let v0 = Simd::<u16, 8>::from_slice(&linear[done..done + 8]).cast::<i32>();
            let c = Simd::<u16, 8>::from_slice(&center[done..done + 8]).cast::<i32>();
            let t = Simd::<u16, 8>::from_slice(&target[done..done + 8]).cast::<i32>();
            let v1 = (c * c + half8) >> shift8;
            for (acc, term) in
                acc8.iter_mut()
                    .zip([v0 * v0, v0 * v1, v1 * v1, v0, v1, v0 * t, v1 * t, t])
            {
                *acc += term;
            }
            done += 8;
        }
        let [s00_16, s01_16, s11_16, s0_16, s1_16, s0t_16, s1t_16, st_16] =
            acc16.map(SimdInt::reduce_sum);
        let [s00_8, s01_8, s11_8, s0_8, s1_8, s0t_8, s1t_8, st_8] = acc8.map(SimdInt::reduce_sum);
        self.count += done;
        self.s00 = self.s00.wrapping_add(s00_16).wrapping_add(s00_8);
        self.s01 = self.s01.wrapping_add(s01_16).wrapping_add(s01_8);
        self.s11 = self.s11.wrapping_add(s11_16).wrapping_add(s11_8);
        self.s0 = self.s0.wrapping_add(s0_16).wrapping_add(s0_8);
        self.s1 = self.s1.wrapping_add(s1_16).wrapping_add(s1_8);
        self.s0t = self.s0t.wrapping_add(s0t_16).wrapping_add(s0t_8);
        self.s1t = self.s1t.wrapping_add(s1t_16).wrapping_add(s1t_8);
        self.st = self.st.wrapping_add(st_16).wrapping_add(st_8);
        for ((&linear, &center), &target) in linear[done..]
            .iter()
            .zip(&center[done..])
            .zip(&target[done..])
        {
            self.add(linear, center, target, bits);
        }
    }

    /// Adds one sample's `v[0]` (`linear`), centre luma and target chroma.
    fn add(&mut self, linear: u16, center: u16, target: u16, bits: u32) {
        let (v0, t) = (i32::from(linear), i32::from(target));
        let center = i32::from(center);
        let v1 = round2_i32(center.wrapping_mul(center), bits);
        self.s00 = self.s00.wrapping_add(v0.wrapping_mul(v0));
        self.s01 = self.s01.wrapping_add(v0.wrapping_mul(v1));
        self.s11 = self.s11.wrapping_add(v1.wrapping_mul(v1));
        self.s0 = self.s0.wrapping_add(v0);
        self.s1 = self.s1.wrapping_add(v1);
        self.s0t = self.s0t.wrapping_add(v0.wrapping_mul(t));
        self.s1t = self.s1t.wrapping_add(v1.wrapping_mul(t));
        self.st = self.st.wrapping_add(t);
        self.count += 1;
    }
}

fn solve_mhccp(equations: NormalEquations, bit_depth: BitDepth) -> [i32; MHCCP_PARAM_COUNT] {
    let mut rows = augmented_rows(&equations, bit_depth);
    forward_eliminate(&mut rows);
    back_substitute(&rows)
}

fn augmented_rows(
    equations: &NormalEquations,
    bit_depth: BitDepth,
) -> [[i32; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT] {
    let mut rows = [[0i32; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT];
    for (row_index, row) in rows.iter_mut().enumerate() {
        for (col_index, value) in row.iter_mut().take(MHCCP_PARAM_COUNT).enumerate() {
            *value = symmetric_equation_value(equations, row_index, col_index);
        }
        row[row_index] = row[row_index].saturating_add(2i32 << (u32::from(bit_depth.bits()) - 8));
        row[MHCCP_PARAM_COUNT] = equations.b[row_index];
    }
    rows
}

fn symmetric_equation_value(equations: &NormalEquations, row: usize, col: usize) -> i32 {
    if col >= row {
        equations.ata[row][col]
    } else {
        equations.ata[col][row]
    }
}

fn forward_eliminate(rows: &mut [[i32; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT]) {
    for i in 0..MHCCP_PARAM_COUNT {
        let diag = rows[i][i].unsigned_abs().max(1);
        let (scale, shift) = mhccp_division_scale_shift(diag);
        for value in rows[i].iter_mut().skip(i + 1) {
            *value = mul_fixed32_adapt(*value, scale, shift);
        }
        let pivot = rows[i];
        for target in rows.iter_mut().skip(i + 1) {
            eliminate_with_pivot(target, pivot, i);
        }
    }
}

fn eliminate_with_pivot(
    target: &mut [i32; MHCCP_PARAM_COUNT + 1],
    pivot: [i32; MHCCP_PARAM_COUNT + 1],
    pivot_col: usize,
) {
    let factor = target[pivot_col];
    for col in pivot_col + 1..=MHCCP_PARAM_COUNT {
        let delta = mul_fixed32_adapt(factor, pivot[col], MHCCP_BITS);
        target[col] = target[col].saturating_sub(delta);
    }
}

fn back_substitute(rows: &[[i32; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT]) -> [i32; 3] {
    let mut params = [0i32; MHCCP_PARAM_COUNT];
    for row in (0..MHCCP_PARAM_COUNT).rev() {
        let mut value = rows[row][MHCCP_PARAM_COUNT];
        for col in row + 1..MHCCP_PARAM_COUNT {
            value =
                value.saturating_sub(mul_fixed32_adapt(rows[row][col], params[col], MHCCP_BITS));
        }
        params[row] = value;
    }
    params
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

    /// Predicts `out.len()` samples; `linear` and `center` must be as long.
    pub fn row(&self, linear: &[u16], center: &[u16], out: &mut [u16]) {
        let mut done = 0;
        if self.narrow {
            let p0_16 = Simd::<i32, 16>::splat(self.params[0]);
            let p1_16 = Simd::<i32, 16>::splat(self.params[1]);
            let half_16 = Simd::<i32, 16>::splat(1i32 << (MHCCP_BITS - 1));
            let square_half_16 = Simd::<i32, 16>::splat(1i32 << (self.bits - 1));
            let bits_16 = Simd::<i32, 16>::splat(self.bits as i32);
            let shift_16 = Simd::<i32, 16>::splat(MHCCP_BITS as i32);
            let offset_16 = Simd::<i32, 16>::splat(self.offset);
            let zero_16 = Simd::<i32, 16>::splat(0);
            let max_16 = Simd::<i32, 16>::splat(self.max);
            let round_16 = |x: Simd<i32, 16>| {
                let r = (x.abs() + half_16) >> shift_16;
                x.simd_lt(zero_16).select(-r, r)
            };
            while out.len() - done >= 16 {
                let linear_v = Simd::<u16, 16>::from_slice(&linear[done..done + 16]).cast::<i32>();
                let center_v = Simd::<u16, 16>::from_slice(&center[done..done + 16]).cast::<i32>();
                let square = (center_v * center_v + square_half_16) >> bits_16;
                let t = round_16(p0_16 * linear_v) + round_16(p1_16 * square) + offset_16;
                let t = t.simd_max(zero_16).simd_min(max_16);
                t.cast::<u16>().copy_to_slice(&mut out[done..done + 16]);
                done += 16;
            }
            let p0_8 = Simd::<i32, 8>::splat(self.params[0]);
            let p1_8 = Simd::<i32, 8>::splat(self.params[1]);
            let half_8 = Simd::<i32, 8>::splat(1i32 << (MHCCP_BITS - 1));
            let square_half_8 = Simd::<i32, 8>::splat(1i32 << (self.bits - 1));
            let bits_8 = Simd::<i32, 8>::splat(self.bits as i32);
            let shift_8 = Simd::<i32, 8>::splat(MHCCP_BITS as i32);
            let offset_8 = Simd::<i32, 8>::splat(self.offset);
            let zero_8 = Simd::<i32, 8>::splat(0);
            let max_8 = Simd::<i32, 8>::splat(self.max);
            let round_8 = |x: Simd<i32, 8>| {
                let r = (x.abs() + half_8) >> shift_8;
                x.simd_lt(zero_8).select(-r, r)
            };
            while out.len() - done >= 8 {
                let linear_v = Simd::<u16, 8>::from_slice(&linear[done..done + 8]).cast::<i32>();
                let center_v = Simd::<u16, 8>::from_slice(&center[done..done + 8]).cast::<i32>();
                let square = (center_v * center_v + square_half_8) >> bits_8;
                let t = round_8(p0_8 * linear_v) + round_8(p1_8 * square) + offset_8;
                let t = t.simd_max(zero_8).simd_min(max_8);
                t.cast::<u16>().copy_to_slice(&mut out[done..done + 8]);
                done += 8;
            }
        }
        for ((out, &linear), &center) in out[done..]
            .iter_mut()
            .zip(&linear[done..])
            .zip(&center[done..])
        {
            *out = self.sample(linear, center);
        }
    }
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
        round2_signed_i32(product, adjustment as u32)
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
            luma: vec![1023; 64 * 64],
            chroma: vec![1023; 64 * 64],
        };

        let params = derive_mhccp_params(&refs, 0, BitDepth::Ten);
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
            let mut out = vec![0; 36];
            predictor.row(&luma[1..], &luma[..36], &mut out);
            for (i, &got) in out.iter().enumerate() {
                let center = i32::from(luma[i]);
                let vector = [i32::from(luma[i + 1]), round2_i32(center * center, 10), 512];
                let t = (0..3).fold(0i32, |t, k| {
                    t.saturating_add(mul_fixed32_adapt(params[k], vector[k], MHCCP_BITS))
                });
                assert_eq!(
                    i32::from(got),
                    t.clamp(0, 1023),
                    "params {params:?} sample {i}"
                );
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
            luma: vec![0; 9],
            chroma: vec![0; 9],
        };

        assert_eq!(
            derive_mhccp_params(&refs, 0, BitDepth::Eight),
            [0, 0, 1 << MHCCP_BITS]
        );
    }

    #[test]
    fn parameter_derivation_ignores_uncaptured_prediction_padding() {
        let refs = |padding| {
            let mut luma = vec![64; 16];
            let mut chroma = vec![96; 16];
            luma[12..].fill(padding);
            chroma[12..].fill(padding);
            MhccpRefs {
                width: 4,
                height: 4,
                reference_width: 4,
                reference_height: 3,
                above: 2,
                left: 2,
                luma,
                chroma,
            }
        };

        let expected = derive_mhccp_params(&refs(0), 0, BitDepth::Eight);
        assert_ne!(expected, [0, 0, 1 << MHCCP_BITS]);
        assert_eq!(
            derive_mhccp_params(&refs(u8::MAX.into()), 0, BitDepth::Eight),
            expected
        );
    }
}
