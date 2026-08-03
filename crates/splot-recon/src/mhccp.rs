// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.6 MHCCP parameter derivation helpers.

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

    fn is_edge_ref_sample(&self, row: usize, col: usize) -> bool {
        row < self.above || col < self.left
    }

    fn luma_at(&self, row: usize, col: usize) -> u16 {
        self.luma[row * self.width + col]
    }

    fn chroma_at(&self, row: usize, col: usize) -> u16 {
        self.chroma[row * self.width + col]
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
    fn add(&mut self, basis: [i16; MHCCP_PARAM_COUNT], target: u16) {
        for (row, col) in UPPER_TRIANGLE {
            self.ata[row][col] = self.ata[row][col]
                .saturating_add(i32::from(basis[row]).saturating_mul(i32::from(basis[col])));
        }
        for (dst, value) in self.b.iter_mut().zip(basis) {
            *dst = dst.saturating_add(i32::from(value).saturating_mul(i32::from(target)));
        }
        self.samples = self.samples.saturating_add(1);
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
    let mut equations = NormalEquations::default();
    if refs.has_edge_refs() {
        let square_shift = u32::from(bit_depth.bits());
        let midpoint = 1i16 << (square_shift - 1);
        let reference_width = refs.reference_width.min(refs.width);
        let reference_height = refs.reference_height.min(refs.height);
        for (row, col) in mhccp_interior_positions(reference_width, reference_height) {
            if refs.is_edge_ref_sample(row, col) {
                let center = refs.luma_at(row, col);
                let basis = mhccp_basis(refs, row, col, mh_dir, center, square_shift, midpoint);
                equations.add(basis, refs.chroma_at(row, col));
            }
        }
    }
    if equations.samples == 0 {
        return [0, 0, 1i32 << MHCCP_BITS];
    }
    let bit_depth_bits = i32::from(bit_depth.bits());
    let matrix_shift =
        MHCCP_BITS as i32 + 6 - 2 * bit_depth_bits - ceil_log2_usize(equations.samples) as i32;
    equations.apply_matrix_shift(matrix_shift);
    solve_mhccp(equations, bit_depth)
}

fn mhccp_interior_positions(width: usize, height: usize) -> impl Iterator<Item = (usize, usize)> {
    (1..height.saturating_sub(1))
        .flat_map(move |row| (1..width.saturating_sub(1)).map(move |col| (row, col)))
}

fn mhccp_basis(
    refs: &MhccpRefs,
    row: usize,
    col: usize,
    mh_dir: u8,
    center: u16,
    square_shift: u32,
    midpoint: i16,
) -> [i16; MHCCP_PARAM_COUNT] {
    let center_i32 = i32::from(center);
    [
        mhccp_linear_ref(refs, row, col, mh_dir, center) as i16,
        round2_i32(center_i32.saturating_mul(center_i32), square_shift) as i16,
        midpoint,
    ]
}

fn mhccp_linear_ref(refs: &MhccpRefs, row: usize, col: usize, mh_dir: u8, center: u16) -> u16 {
    match mh_dir {
        0 => center,
        1 => refs.luma_at(row - 1, col),
        _ => refs.luma_at(row, col - 1),
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

/// Multiplies fixed-point values with adaptive down-shifting to avoid overflow.
#[must_use]
pub fn mul_fixed32_adapt(a: i32, b: i32, shift: u32) -> i32 {
    let (lhs, rhs, adjustment) = scaled_multiply_terms(a, b, shift);
    round_scaled_product(lhs.saturating_mul(rhs), adjustment)
}

fn round_scaled_product(product: i32, adjustment: i32) -> i32 {
    if adjustment <= 0 {
        product
    } else {
        round_positive_scaled_product(product, adjustment as u32)
    }
}

fn round_positive_scaled_product(product: i32, adjustment: u32) -> i32 {
    if adjustment > 29 {
        0
    } else {
        round2_signed_i32(product, adjustment)
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
