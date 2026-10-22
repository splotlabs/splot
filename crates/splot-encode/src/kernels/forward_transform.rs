// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Forward 2D transform, the encoder inverse of the § 7.15.4 decoder transform.
//!
//! AV2 specifies only the inverse
//! ([`07-decoding-process.md`](../../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-15-4-1`). The decoder computes
//! `Residual = K_h^T * Dequant * K_w * r / 2^(rowShift + colShift)`, where `K_n`
//! is the § 7.15.2.1 kernel (row `j` is basis `j`), the § 7.15.2.3 identity
//! `get_identity_scale(log2n) * I`, and `r = 2896 / 4096` when
//! `|log2W - log2H|` is odd. The forward transform is that equation solved for
//! `Dequant`, so its output feeds the decoder's dequantized-coefficient scale:
//!
//! `Dequant = 2^(rowShift + colShift) / r * (K_h^-1)^T * X * K_w^-1`.
//!
//! Each `K_n` is read from `splot-recon`'s public inverse (one unit vector per
//! basis), so the forward kernels follow the decoder's own § 7.15.2.1 dispatch:
//! FDST and FDDT need no separate flip, every type at
//! length 4 other than DCT and ADST is FDST, and every type at length 32 is DCT.
//! The exact inverse `K_n^-1` (not `K_n^T`) is used because the integer kernels
//! are not exactly orthogonal; it is stored as
//! `round(K_n^-1 * 2^(12 + EXTRA_BITS) * n)`, about `K_n^T * 2^EXTRA_BITS`.
//!
//! A 64-sample side is transformed at 32: § 7.15.4 transforms at
//! `Adjusted_Tx_Size` and duplicates each residual sample, so the forward
//! transform sums each sample pair first and keeps only the 32 coefficients
//! the decoder reads (the AV2 form of the 64-point zero-out).
//!
//! Arithmetic is wrapping `i32` in both forms. With column L1 norms at most
//! `1.25 * 2^(6 + EXTRA_BITS + log2n)` (times `4096 / 2896` for the rescaled row
//! matrix, both checked by a test), residuals below `2^BitDepth`, and the
//! intermediate shift
//! `T1 = BitDepth + p + log2w + log2h + 2 * EXTRA_BITS + odd - 18` (`p` counts
//! the pair-summed sides), no sum leaves `i32`.

use std::simd::Simd;
use std::sync::LazyLock;

use splot_recon::{
    BitDepth, InverseTransform1dType, InverseTransform2dDim, inverse_transform_1d, transform_shift,
};

use super::KernelError;

/// Extra fractional bits kept in the forward matrices.
const EXTRA_BITS: u32 = 6;

/// The largest adjusted transform side.
const MAX_DIM: usize = 32;

/// Matrix kinds: identity, then the five kernel types.
const KINDS: [InverseTransform2dDim; 6] = [
    InverseTransform2dDim::Identity,
    InverseTransform2dDim::Kernel(InverseTransform1dType::Dct),
    InverseTransform2dDim::Kernel(InverseTransform1dType::Adst),
    InverseTransform2dDim::Kernel(InverseTransform1dType::Fdst),
    InverseTransform2dDim::Kernel(InverseTransform1dType::Ddtx),
    InverseTransform2dDim::Kernel(InverseTransform1dType::Fddt),
];

/// Forward matrices, sample-major (`m[x * n + k]` weights sample `x` into
/// coefficient `k`), indexed by [`matrix_index`].
static MATRICES: LazyLock<Vec<Box<[i32]>>> = LazyLock::new(|| {
    let mut out = Vec::with_capacity(KINDS.len() * 8);
    for kind in KINDS {
        for log2n in 2..=5 {
            for rescaled in [false, true] {
                out.push(forward_matrix(kind, log2n, rescaled));
            }
        }
    }
    out
});

fn matrix_index(kind: InverseTransform2dDim, log2n: u32, rescaled: bool) -> usize {
    let kind = KINDS.iter().position(|&k| k == kind).unwrap_or(0);
    kind * 8 + (log2n as usize - 2) * 2 + usize::from(rescaled)
}

/// § 7.15.4.1 `get_identity_scale( log2Sz )`.
fn identity_scale(log2n: u32) -> f64 {
    match log2n {
        2 => 128.0,
        3 => 181.0,
        4 => 256.0,
        _ => 362.0,
    }
}

/// The decoder matrix `K_n` (row `j` is the inverse of a unit coefficient `j`).
fn inverse_matrix(kind: InverseTransform2dDim, log2n: u32) -> Vec<f64> {
    let n = 1usize << log2n;
    let mut k = vec![0.0; n * n];
    for (j, row) in k.chunks_exact_mut(n).enumerate() {
        match kind {
            InverseTransform2dDim::Identity => row[j] = identity_scale(log2n),
            InverseTransform2dDim::Kernel(tx_type) => {
                let mut unit = [0i32; MAX_DIM];
                unit[j] = 1;
                let mut basis = [0i32; MAX_DIM];
                if inverse_transform_1d(
                    &unit[..n],
                    tx_type,
                    0,
                    false,
                    BitDepth::Eight,
                    &mut basis[..n],
                )
                .is_ok()
                {
                    for (slot, &value) in row.iter_mut().zip(&basis) {
                        *slot = f64::from(value);
                    }
                }
            }
        }
    }
    k
}

/// Gauss-Jordan inverse of the `n x n` row-major matrix `a`, or zeros if `a` is
/// singular (no AV2 kernel is).
fn invert(mut a: Vec<f64>, n: usize) -> Vec<f64> {
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    for col in 0..n {
        let pivot = (col..n).max_by(|&p, &q| a[p * n + col].abs().total_cmp(&a[q * n + col].abs()));
        let Some(pivot) = pivot.filter(|&p| a[p * n + col] != 0.0) else {
            return vec![0.0; n * n];
        };
        for j in 0..n {
            a.swap(col * n + j, pivot * n + j);
            inv.swap(col * n + j, pivot * n + j);
        }
        let scale = a[col * n + col];
        for j in 0..n {
            a[col * n + j] /= scale;
            inv[col * n + j] /= scale;
        }
        for row in (0..n).filter(|&row| row != col) {
            let factor = a[row * n + col];
            for j in 0..n {
                a[row * n + j] -= factor * a[col * n + j];
                inv[row * n + j] -= factor * inv[col * n + j];
            }
        }
    }
    inv
}

fn forward_matrix(kind: InverseTransform2dDim, log2n: u32, rescaled: bool) -> Box<[i32]> {
    let n = 1usize << log2n;
    let rescale = if rescaled { 4096.0 / 2896.0 } else { 1.0 };
    let scale = f64::from(1u32 << (12 + EXTRA_BITS)) * n as f64 * rescale;
    invert(inverse_matrix(kind, log2n), n)
        .iter()
        .map(|v| (v * scale).round() as i32)
        .collect()
}

/// A validated forward 2D transform: the encoder inverse of one § 7.15.4.1
/// inverse transform with the same shape and 1D types.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ForwardTransform2d {
    log2_width: u32,
    log2_height: u32,
    pair_width: bool,
    pair_height: bool,
    row: &'static [i32],
    col: &'static [i32],
    row_shift: u32,
    col_shift: u32,
}

impl ForwardTransform2d {
    /// The forward transform for an original `2^log2_width x 2^log2_height`
    /// block whose decoder pass types are `row_type` / `col_type` (the
    /// § 7.15.4.1 `get_transform_1d_type` results) at `bit_depth`.
    ///
    /// # Errors
    /// Returns [`KernelError::InvalidTransformShape`] if the shape is not one of
    /// the AV2 `TX_SIZES_ALL` shapes.
    pub(crate) fn new(
        log2_width: u32,
        log2_height: u32,
        row_type: InverseTransform2dDim,
        col_type: InverseTransform2dDim,
        bit_depth: BitDepth,
    ) -> Result<Self, KernelError> {
        let (row_shift, col_shift) = transform_shift(log2_width, log2_height).map_err(|_| {
            KernelError::InvalidTransformShape {
                log2_width,
                log2_height,
            }
        })?;
        let (lw, lh) = (log2_width.min(5), log2_height.min(5));
        let pairs = u32::from(log2_width > 5) + u32::from(log2_height > 5);
        let odd = log2_width.abs_diff(log2_height) % 2 == 1;
        let total =
            24 + lw + lh + pairs + 2 * EXTRA_BITS - u32::from(row_shift) - u32::from(col_shift);
        let bits = u32::from(bit_depth.bits()) + pairs + lw + lh + 2 * EXTRA_BITS + u32::from(odd);
        let row_shift = bits.saturating_sub(18);
        Ok(Self {
            log2_width: lw,
            log2_height: lh,
            pair_width: log2_width > 5,
            pair_height: log2_height > 5,
            row: &MATRICES[matrix_index(row_type, lw, odd)],
            col: &MATRICES[matrix_index(col_type, lh, false)],
            row_shift,
            col_shift: total - row_shift,
        })
    }

    /// The coefficient block width (`Min(32, Tx_Width)`).
    pub(crate) fn coeff_width(&self) -> usize {
        1 << self.log2_width
    }

    /// The coefficient block height (`Min(32, Tx_Height)`).
    pub(crate) fn coeff_height(&self) -> usize {
        1 << self.log2_height
    }

    /// The residual block length (`Tx_Width * Tx_Height`).
    pub(crate) fn residual_len(&self) -> usize {
        (self.coeff_width() * self.coeff_height())
            << (u32::from(self.pair_width) + u32::from(self.pair_height))
    }

    /// Validates the buffers and returns the transform input: `residual`, or
    /// its pair sums in `scratch` (filled only then) when a side is 64.
    fn input<'a>(
        &self,
        residual: &'a [i32],
        coeffs: &[i32],
        scratch: &'a mut Option<[i32; MAX_DIM * MAX_DIM]>,
    ) -> Result<&'a [i32], KernelError> {
        let (w, h) = (self.coeff_width(), self.coeff_height());
        if residual.len() != self.residual_len() {
            return Err(KernelError::BufferLength {
                buffer: "residual",
                expected: self.residual_len(),
                actual: residual.len(),
            });
        }
        if coeffs.len() != w * h {
            return Err(KernelError::BufferLength {
                buffer: "coeffs",
                expected: w * h,
                actual: coeffs.len(),
            });
        }
        if !self.pair_width && !self.pair_height {
            return Ok(residual);
        }
        let px = usize::from(self.pair_width);
        let py = usize::from(self.pair_height);
        let stride = w << px;
        let scratch = scratch.insert([0; MAX_DIM * MAX_DIM]);
        for (y, row) in scratch.chunks_exact_mut(w).take(h).enumerate() {
            for (x, slot) in row.iter_mut().enumerate() {
                let mut sum = 0i32;
                for dy in 0..=py {
                    for dx in 0..=px {
                        sum =
                            sum.wrapping_add(residual[((y << py) + dy) * stride + (x << px) + dx]);
                    }
                }
                *slot = sum;
            }
        }
        Ok(&scratch[..w * h])
    }
}

fn round_shift(value: i32, shift: u32) -> i32 {
    if shift == 0 {
        value
    } else {
        value.wrapping_add(1 << (shift - 1)) >> shift
    }
}

pub(super) fn forward_scalar(
    tx: &ForwardTransform2d,
    residual: &[i32],
    coeffs: &mut [i32],
) -> Result<(), KernelError> {
    let mut scratch = None;
    let input = tx.input(residual, coeffs, &mut scratch)?;
    let (w, h) = (tx.coeff_width(), tx.coeff_height());
    let mut tmp = [0i32; MAX_DIM * MAX_DIM];
    for (x_row, t_row) in input.chunks_exact(w).zip(tmp.chunks_exact_mut(w)) {
        for (k, slot) in t_row.iter_mut().enumerate() {
            let mut sum = 0i32;
            for (x, &sample) in x_row.iter().enumerate() {
                sum = sum.wrapping_add(sample.wrapping_mul(tx.row[x * w + k]));
            }
            *slot = round_shift(sum, tx.row_shift);
        }
    }
    for (i, c_row) in coeffs.chunks_exact_mut(w).enumerate() {
        for (k, slot) in c_row.iter_mut().enumerate() {
            let mut sum = 0i32;
            for y in 0..h {
                sum = sum.wrapping_add(tx.col[y * h + i].wrapping_mul(tmp[y * w + k]));
            }
            *slot = round_shift(sum, tx.col_shift);
        }
    }
    Ok(())
}

pub(super) fn forward_simd(
    tx: &ForwardTransform2d,
    residual: &[i32],
    coeffs: &mut [i32],
) -> Result<(), KernelError> {
    let mut scratch = None;
    let input = tx.input(residual, coeffs, &mut scratch)?;
    match tx.coeff_width() {
        4 => forward_lanes::<4>(tx, input, coeffs),
        8 => forward_lanes::<8>(tx, input, coeffs),
        16 => forward_lanes::<16>(tx, input, coeffs),
        _ => forward_lanes::<32>(tx, input, coeffs),
    }
    Ok(())
}

/// Both passes with one `W`-lane vector per row (`W` is the coefficient
/// width): the row pass accumulates `sample * matrix row`, the column pass
/// `matrix weight * intermediate row`.
fn forward_lanes<const W: usize>(tx: &ForwardTransform2d, input: &[i32], coeffs: &mut [i32]) {
    let h = tx.coeff_height();
    let rows = tx.row.as_chunks::<W>().0;
    let mut tmp = [Simd::<i32, W>::splat(0); MAX_DIM];
    let row_round = Simd::splat(if tx.row_shift == 0 {
        0
    } else {
        1 << (tx.row_shift - 1)
    });
    for (slot, x_row) in tmp.iter_mut().zip(input.as_chunks::<W>().0) {
        let mut acc = Simd::splat(0);
        for (&sample, m_row) in x_row.iter().zip(rows) {
            acc += Simd::splat(sample) * Simd::from_array(*m_row);
        }
        *slot = (acc + row_round) >> Simd::splat(tx.row_shift as i32);
    }
    let col_round = Simd::splat(if tx.col_shift == 0 {
        0
    } else {
        1 << (tx.col_shift - 1)
    });
    for (i, out) in coeffs.as_chunks_mut::<W>().0.iter_mut().enumerate() {
        let mut acc = Simd::splat(0);
        for (y, &row) in tmp.iter().take(h).enumerate() {
            acc += Simd::splat(tx.col[y * h + i]) * row;
        }
        *out = ((acc + col_round) >> Simd::splat(tx.col_shift as i32)).to_array();
    }
}

#[cfg(test)]
#[path = "forward_transform_tests.rs"]
mod tests;
