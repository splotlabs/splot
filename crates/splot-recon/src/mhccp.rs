// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.6 MHCCP parameter derivation helpers.

use crate::BitDepth;
use crate::math::{clip3, round2, round2_signed};

#[doc = "AV2 § 3 symbols and § 7.13.6 MHCCP process constants."]
pub const MHCCP_BITS: u32 = 16;
pub const MHCCP_PARAM_COUNT: usize = 3;
const DIV_PREC_BITS: u32 = 14;
const DIV_PREC_BITS_POW2: u32 = 8;
const DIV_SLOT_BITS: u32 = 3;
const DIV_INTR_BITS: u32 = DIV_PREC_BITS - DIV_SLOT_BITS;
#[doc = "AV2 § 7.13.6 `get_division_scale_shift` lookup tables."]
const DIVISION_POW2_W: [i64; 8] = [214, 153, 113, 86, 67, 53, 43, 35];
const DIVISION_POW2_O: [i64; 8] = [4822, 5952, 6624, 6792, 6408, 5424, 3792, 1466];
const DIVISION_POW2_B: [i64; 8] = [12784, 12054, 11670, 11583, 11764, 12195, 12870, 13782];

/// Reference samples used to derive AV2 § 7.13.6 MHCCP parameters.
pub struct MhccpRefs {
    /// Reference grid width.
    pub width: usize,
    /// Reference grid height.
    pub height: usize,
    /// Number of above-reference rows.
    pub above: usize,
    /// Number of left-reference columns.
    pub left: usize,
    /// Luma reference samples.
    pub luma: Vec<i64>,
    /// Chroma reference samples.
    pub chroma: Vec<i64>,
}

impl MhccpRefs {
    fn has_edge_refs(&self) -> bool {
        self.above != 0 || self.left != 0
    }

    fn is_edge_ref_sample(&self, row: usize, col: usize) -> bool {
        row < self.above || col < self.left
    }

    fn luma_at(&self, row: usize, col: usize) -> i64 {
        self.luma[row * self.width + col]
    }

    fn chroma_at(&self, row: usize, col: usize) -> i64 {
        self.chroma[row * self.width + col]
    }
}

const UPPER_TRIANGLE: [(usize, usize); 6] = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];

#[derive(Clone, Copy, Debug, Default)]
struct NormalEquations {
    ata: [[i64; MHCCP_PARAM_COUNT]; MHCCP_PARAM_COUNT],
    b: [i64; MHCCP_PARAM_COUNT],
    samples: usize,
}

impl NormalEquations {
    fn add(&mut self, basis: [i64; MHCCP_PARAM_COUNT], target: i64) {
        for (row, col) in UPPER_TRIANGLE {
            self.ata[row][col] = self.ata[row][col].saturating_add(basis[row] * basis[col]);
        }
        for (dst, value) in self.b.iter_mut().zip(basis) {
            *dst = dst.saturating_add(value * target);
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
                self.ata[row][col] <<= shift;
            } else {
                self.ata[row][col] >>= shift;
            }
        }
        for value in &mut self.b {
            if matrix_shift.is_positive() {
                *value <<= shift;
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
) -> [i64; MHCCP_PARAM_COUNT] {
    let mut equations = NormalEquations::default();
    if refs.has_edge_refs() {
        let square_shift = u32::from(bit_depth.bits());
        let midpoint = 1i64 << (square_shift - 1);
        for (row, col) in mhccp_interior_positions(refs.width, refs.height) {
            if refs.is_edge_ref_sample(row, col) {
                let center = refs.luma_at(row, col);
                let basis = mhccp_basis(refs, row, col, mh_dir, center, square_shift, midpoint);
                equations.add(basis, refs.chroma_at(row, col));
            }
        }
    }
    if equations.samples == 0 {
        return [0, 0, 1i64 << MHCCP_BITS];
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
    center: i64,
    square_shift: u32,
    midpoint: i64,
) -> [i64; MHCCP_PARAM_COUNT] {
    [
        mhccp_linear_ref(refs, row, col, mh_dir, center),
        round2(center.saturating_mul(center), square_shift),
        midpoint,
    ]
}

fn mhccp_linear_ref(refs: &MhccpRefs, row: usize, col: usize, mh_dir: u8, center: i64) -> i64 {
    match mh_dir {
        0 => center,
        1 => refs.luma_at(row - 1, col),
        _ => refs.luma_at(row, col - 1),
    }
}

fn solve_mhccp(equations: NormalEquations, bit_depth: BitDepth) -> [i64; MHCCP_PARAM_COUNT] {
    let mut rows = augmented_rows(&equations, bit_depth);
    forward_eliminate(&mut rows);
    back_substitute(&rows)
}

fn augmented_rows(
    equations: &NormalEquations,
    bit_depth: BitDepth,
) -> [[i64; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT] {
    let mut rows = [[0i64; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT];
    for (row_index, row) in rows.iter_mut().enumerate() {
        for (col_index, value) in row.iter_mut().take(MHCCP_PARAM_COUNT).enumerate() {
            *value = symmetric_equation_value(equations, row_index, col_index);
        }
        row[row_index] = row[row_index].saturating_add(2i64 << (u32::from(bit_depth.bits()) - 8));
        row[MHCCP_PARAM_COUNT] = equations.b[row_index];
    }
    rows
}

fn symmetric_equation_value(equations: &NormalEquations, row: usize, col: usize) -> i64 {
    if col >= row {
        equations.ata[row][col]
    } else {
        equations.ata[col][row]
    }
}

fn forward_eliminate(rows: &mut [[i64; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT]) {
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
    target: &mut [i64; MHCCP_PARAM_COUNT + 1],
    pivot: [i64; MHCCP_PARAM_COUNT + 1],
    pivot_col: usize,
) {
    let factor = target[pivot_col];
    for col in pivot_col + 1..=MHCCP_PARAM_COUNT {
        let delta = mul_fixed32_adapt(factor, pivot[col], MHCCP_BITS);
        target[col] = target[col].saturating_sub(delta);
    }
}

fn back_substitute(rows: &[[i64; MHCCP_PARAM_COUNT + 1]; MHCCP_PARAM_COUNT]) -> [i64; 3] {
    let mut params = [0i64; MHCCP_PARAM_COUNT];
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

fn mhccp_division_scale_shift(denom: u64) -> (i64, u32) {
    let shift = denom.checked_ilog2().unwrap_or(0);
    let norm_diff_clip = clip3(
        1,
        (1i64 << (DIV_PREC_BITS + 1)) - 1,
        normalized_divisor(denom, shift),
    );
    let norm_diff = norm_diff_clip & ((1i64 << DIV_PREC_BITS) - 1);
    (division_scale(norm_diff), shift)
}

fn normalized_divisor(denom: u64, shift: u32) -> i64 {
    let delta = shift as i32 - DIV_PREC_BITS as i32;
    if delta >= 0 {
        let right = delta as u32;
        let bias = right.checked_sub(1).map_or(0, |bits| 1u64 << bits);
        return ((denom.saturating_add(bias)) >> right) as i64;
    }
    let left = (-delta) as u32;
    if left >= i64::BITS {
        return i64::MAX;
    }
    let max = (i64::MAX as u64) >> left;
    (denom.min(max) << left) as i64
}

fn division_scale(norm_diff: i64) -> i64 {
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
pub fn mul_fixed32_adapt(a: i64, b: i64, shift: u32) -> i64 {
    let (lhs, rhs, adjustment) = scaled_multiply_terms(a, b, shift);
    round_scaled_product(lhs.saturating_mul(rhs), adjustment)
}

fn round_scaled_product(product: i64, adjustment: i32) -> i64 {
    if adjustment <= 0 {
        product
    } else {
        round_positive_scaled_product(product, adjustment as u32)
    }
}

fn round_positive_scaled_product(product: i64, adjustment: u32) -> i64 {
    if adjustment > 29 {
        0
    } else {
        round2_signed(product, adjustment)
    }
}

fn scaled_multiply_terms(a: i64, b: i64, shift: u32) -> (i64, i64, i32) {
    let overflow_bits = bit_width_abs(a)
        .saturating_add(bit_width_abs(b))
        .saturating_sub(29);
    let left_shift = overflow_bits / 2;
    let right_shift = overflow_bits - left_shift;
    (
        a >> left_shift,
        b >> right_shift,
        shift as i32 - overflow_bits as i32,
    )
}

fn bit_width_abs(value: i64) -> u32 {
    let bits = u64::BITS - value.unsigned_abs().leading_zeros();
    bits.max(1)
}

fn ceil_log2_usize(value: usize) -> u32 {
    value
        .saturating_sub(1)
        .checked_ilog2()
        .map_or(0, |log| log + 1)
}
