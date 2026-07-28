// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.2 1D inverse transform processes.
//!
//! This module implements the scheduler-free AV2 § 7.15.2 1D inverse transforms
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)):
//!
//! - § 7.15.2.1 ([`inverse_transform_1d`]): the kernel-based transform — a matrix
//!   multiplication of the input coefficients by the size-and-type § 9.6 kernel
//!   (from the dependency-free `splot-tables` crate), followed by § 4.8 `Round2`
//!   and the `colTx`-dependent `Clip3`.
//! - § 7.15.2.2 ([`inverse_walsh_hadamard`]): the 4-element inverse
//!   Walsh-Hadamard butterfly used by lossless blocks (no kernel, no `Clip3`).
//! - § 7.15.2.3 ([`inverse_identity_transform`]): the inverse identity transform
//!   (the Table 7.1 `IDT` type) — a per-sample `Round2(src * scale, shift)`
//!   followed by the same `colTx`-dependent `Clip3`.
//!
//! Feature tracking: `RECON-INVERSE-TRANSFORM-1D`,
//! `RECON-INVERSE-TRANSFORM-MATRIX-FREE`.
//!
//! Scope: the § 7.15.3 secondary transform and the § 7.15.4 2D inverse transform
//! that orchestrates row/column passes (including the `Transform_Shift`,
//! `get_transform_1d_type`, and `get_identity_scale` derivations, the DPCM
//! cumulative sum, and adjusted-size sample duplication) are out of scope and
//! tracked by their own future rows. The caller supplies the already-derived 1D
//! transform type, scale, shift, and `colTx` flag.

use std::simd::{Simd, cmp::SimdOrd as _};

use splot_tables::tables::transform_1d::{
    ADST_KERNEL4, ADST_KERNEL8, ADST_KERNEL16, DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16,
    DCT_KERNEL32, DDTX_KERNEL8, DDTX_KERNEL16, FDST_KERNEL4, FDST_KERNEL8, FDST_KERNEL16,
};

use crate::math::round2_i32;
use crate::{BitDepth, ReconError, Result};

/// AV2 § 7.15.4.1 Table 7.1 kernel-based 1D inverse transform type.
///
/// Table 7.1 also defines `IDT` (value 1), the inverse identity transform; it is
/// handled by the separate § 7.15.2.3 process and is therefore not represented
/// here. The variants below are the kernel-multiply types reachable by
/// § 7.15.2.1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InverseTransform1dType {
    /// `DCT` (Table 7.1 value 0).
    Dct,
    /// `ADST` (Table 7.1 value 2).
    Adst,
    /// `FDST` (Table 7.1 value 3).
    Fdst,
    /// `DDTX` (Table 7.1 value 4).
    Ddtx,
    /// `FDDT` (Table 7.1 value 5) — the flipped `DDTX` (the § 7.15.2.1 `else`
    /// branch, indexing the `DDTX` kernel column in reverse).
    Fddt,
}

/// Applies the AV2 § 7.15.2.1 1D inverse transform to `src`, writing `out`.
///
/// `src.len()` must be one of 4, 8, 16, or 32 (the spec's supported 1D transform
/// lengths) and must equal `out.len()`. `shift` is the § 7.15.2.1 down-shift,
/// `col_tx` selects the clamp range (`1 << (BitDepth + (col_tx ? 0 : 7))`), and
/// `bit_depth` is the active decoded bit depth.
///
/// Following the spec dispatch exactly: at length 4 only `DCT` and `ADST` have
/// their own kernel and every other type falls to the `FDST` kernel; at length
/// 32 the `DCT` kernel is used regardless of type (AV2 defines no other length-32
/// 1D kernel). The accumulation uses `i32`, matching AVM; inputs are clamped to
/// the § 7.14.4 dequant range before multiplication, and the final `Clip3` bound
/// keeps every written value inside `i32`.
///
/// # Errors
/// Returns [`ReconError::InvalidInverseTransformSize`] if `src.len()` is not 4,
/// 8, 16, or 32, and [`ReconError::InverseTransformLengthMismatch`] if `out.len()`
/// does not equal `src.len()`.
pub fn inverse_transform_1d(
    src: &[i32],
    tx_type: InverseTransform1dType,
    shift: u8,
    col_tx: bool,
    bit_depth: BitDepth,
    out: &mut [i32],
) -> Result<()> {
    let sz = src.len();
    let Some(kernel) = select_kernel(sz, tx_type) else {
        return Err(ReconError::InvalidInverseTransformSize { size: sz });
    };
    if out.len() != sz {
        return Err(ReconError::InverseTransformLengthMismatch {
            src_len: sz,
            out_len: out.len(),
        });
    }

    match kernel {
        Kernel1d::N4(k) => kernel_transform(src, k, false, shift, col_tx, bit_depth, out),
        Kernel1d::N8(k, rev) => kernel_transform(src, k, rev, shift, col_tx, bit_depth, out),
        Kernel1d::N16(k, rev) => kernel_transform(src, k, rev, shift, col_tx, bit_depth, out),
        Kernel1d::N32(k) => kernel_transform(src, k, false, shift, col_tx, bit_depth, out),
    }
    Ok(())
}

/// The § 7.15.2.1 kernel selected for one 1D length, paired with the `FDDT`
/// reverse flag (the § 7.15.2.1 `else` branch, which indexes the kernel column
/// in reverse).
#[derive(Clone, Copy)]
enum Kernel1d {
    /// The length-4 kernel; no type reaches the reversed branch at this length.
    N4(&'static [[i32; 4]; 4]),
    /// The length-8 kernel.
    N8(&'static [[i32; 8]; 8], bool),
    /// The length-16 kernel.
    N16(&'static [[i32; 16]; 16], bool),
    /// The length-32 kernel, the only one AV2 defines at this length.
    N32(&'static [[i32; 32]; 32]),
}

/// Resolves the § 7.15.2.1 kernel dispatch, or `None` for a length the spec does
/// not define.
///
/// At length 4 only `DCT` and `ADST` have their own kernel and every other type
/// falls to the `FDST` kernel; at length 32 the `DCT` kernel is used regardless
/// of type.
fn select_kernel(sz: usize, tx_type: InverseTransform1dType) -> Option<Kernel1d> {
    use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
    Some(match sz {
        4 => Kernel1d::N4(match tx_type {
            Dct => &DCT_KERNEL4,
            Adst => &ADST_KERNEL4,
            Fdst | Ddtx | Fddt => &FDST_KERNEL4,
        }),
        8 => match tx_type {
            Dct => Kernel1d::N8(&DCT_KERNEL8, false),
            Adst => Kernel1d::N8(&ADST_KERNEL8, false),
            Fdst => Kernel1d::N8(&FDST_KERNEL8, false),
            Ddtx => Kernel1d::N8(&DDTX_KERNEL8, false),
            Fddt => Kernel1d::N8(&DDTX_KERNEL8, true),
        },
        16 => match tx_type {
            Dct => Kernel1d::N16(&DCT_KERNEL16, false),
            Adst => Kernel1d::N16(&ADST_KERNEL16, false),
            Fdst => Kernel1d::N16(&FDST_KERNEL16, false),
            Ddtx => Kernel1d::N16(&DDTX_KERNEL16, false),
            Fddt => Kernel1d::N16(&DDTX_KERNEL16, true),
        },
        32 => Kernel1d::N32(&DCT_KERNEL32),
        _ => return None,
    })
}

/// One § 7.15.4.1 column pass over a row-major block of `stride`-sample rows.
///
/// `len` is the column length (the § 7.15.4 adjusted transform height) and
/// `nonzero_rows` marks the source rows the row pass wrote: a clear bit means
/// the row is all zero and contributes only zero terms to every column sum, so
/// the pass never reads it.
#[derive(Clone, Copy)]
pub(crate) struct ColumnPass {
    /// Row stride of both blocks, in samples.
    pub stride: usize,
    /// Column length (`1 << Min(log2H, 5)`).
    pub len: usize,
    /// Bit `i` marks source row `i` as written and possibly non-zero.
    pub nonzero_rows: u32,
    /// Column-pass down-shift (`colShift`).
    pub shift: u8,
    /// Active decoded bit depth.
    pub bit_depth: BitDepth,
}

/// Applies the AV2 § 7.15.2.1 kernel transform down `LANES` adjacent columns of
/// `src` at once, writing the same columns of `out`.
///
/// Both slices start at the first sample of the column group, so lane `l` of row
/// `i` is at offset `i * pass.stride + l`. Because each column's § 7.15.2.1 sum
/// is independent of every other column, running `LANES` of them in lanes
/// re-associates nothing: every lane performs exactly the scalar
/// [`kernel_transform`] sequence of `i32` products, adds, `Round2`, and `Clip3`
/// on its own column. Returns `false` when `pass.len` is not a spec 1D length,
/// which leaves the block to the caller's per-column path.
pub(crate) fn inverse_transform_1d_columns<const LANES: usize>(
    src: &[i32],
    out: &mut [i32],
    tx_type: InverseTransform1dType,
    pass: ColumnPass,
) -> bool {
    match select_kernel(pass.len, tx_type) {
        Some(Kernel1d::N4(k)) => kernel_columns::<4, LANES>(src, out, k, false, pass),
        Some(Kernel1d::N8(k, rev)) => kernel_columns::<8, LANES>(src, out, k, rev, pass),
        Some(Kernel1d::N16(k, rev)) => kernel_columns::<16, LANES>(src, out, k, rev, pass),
        Some(Kernel1d::N32(k)) => kernel_columns::<32, LANES>(src, out, k, false, pass),
        None => return false,
    }
    true
}

/// One `LANES`-wide column group of [`inverse_transform_1d_columns`].
///
/// Each lane repeats [`kernel_transform`] literally: the § 7.14.4 input clamp,
/// the kernel-row-major `i32` accumulation over the rows `nonzero_rows` marks,
/// the `FDDT` output reversal, and the § 4.8 `Round2` expanded as
/// `(v >> n) + ((v >> (n - 1)) & 1)` — the same value the scalar
/// [`round2_i32`] computes for `1 <= n < 32` — followed by the § 7.15.2.1
/// `Clip3`.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 column-pass hot path"
)]
#[inline(always)]
fn kernel_columns<const N: usize, const LANES: usize>(
    src: &[i32],
    out: &mut [i32],
    kernel: &[[i32; N]; N],
    reversed: bool,
    pass: ColumnPass,
) {
    let input_bound = transform_input_bound(pass.bit_depth);
    let (lo, hi) = transform_clip_bounds(true, pass.bit_depth);
    let zero = Simd::<i32, LANES>::splat(0);
    let mut acc = [zero; N];
    for (row, kernel_row) in kernel.iter().enumerate() {
        if row >= u32::BITS as usize || pass.nonzero_rows & (1u32 << row) == 0 {
            continue;
        }
        let start = row * pass.stride;
        let Some(chunk) = src.get(start..start + LANES) else {
            break;
        };
        let coeff = Simd::<i32, LANES>::from_slice(chunk)
            .simd_clamp(Simd::splat(-input_bound), Simd::splat(input_bound - 1));
        for (slot, &k) in acc.iter_mut().zip(kernel_row.iter()) {
            *slot += Simd::splat(k) * coeff;
        }
    }

    let shift = u32::from(pass.shift);
    let (round_shift, carry_shift) = if shift == 0 || shift >= i32::BITS {
        (Simd::splat(0), Simd::splat(0))
    } else {
        (Simd::splat(shift as i32), Simd::splat(shift as i32 - 1))
    };
    let carry_mask = Simd::splat(i32::from(shift != 0 && shift < i32::BITS));
    let saturate = shift >= i32::BITS;
    for index in 0..N {
        let value = acc[if reversed { N - 1 - index } else { index }];
        let rounded = if saturate {
            zero
        } else {
            (value >> round_shift) + ((value >> carry_shift) & carry_mask)
        };
        let clamped = rounded.simd_clamp(Simd::splat(lo), Simd::splat(hi));
        let start = index * pass.stride;
        if let Some(dst) = out.get_mut(start..start + LANES) {
            dst.copy_from_slice(&clamped.to_array()); // splot-copy-ok: a column lane group
        }
    }
}

/// AV2 § 7.15.2.2 inverse Walsh-Hadamard transform: the 4-element lossless
/// butterfly over `src` with a pre-scaling `shift`.
///
/// This is the lossless-block 1D transform. It applies no `Clip3` (the spec
/// produces the butterfly result directly) and is total: inputs are clamped to
/// the maximum AV2 dequant range and the pre-scale `shift` is clamped below the `i32`
/// width, so an out-of-contract `shift` saturates to the arithmetic-shift limit
/// instead of panicking (spec-conformant callers use shift 0 or 3). The result
/// is returned as `i32` (lossless residuals are bounded well within `i32`).
#[allow(clippy::many_single_char_names)]
#[must_use]
pub fn inverse_walsh_hadamard(src: [i32; 4], shift: u8) -> [i32; 4] {
    let shift = u32::from(shift).min(i32::BITS - 1);
    let bound = 1 << 17;
    let mut a = src[0].clamp(-bound, bound - 1) >> shift;
    let mut c = src[1].clamp(-bound, bound - 1) >> shift;
    let mut d = src[2].clamp(-bound, bound - 1) >> shift;
    let mut b = src[3].clamp(-bound, bound - 1) >> shift;
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;
    [a, b, c, d]
}

/// Applies the AV2 § 7.15.2.3 inverse identity transform to `src`, writing `out`.
///
/// Each output is `Clip3(bound, Round2(src[i] * scale, shift))` with the same
/// `colTx`-dependent bound as [`inverse_transform_1d`]. `scale` is the
/// caller-supplied § 7.15.4.1 `get_identity_scale` value, `shift` the down-shift,
/// `col_tx` the column-pass flag, and `bit_depth` the active decoded bit depth.
/// Inputs and the caller-derived scale are clamped to their AV2 ranges before
/// the `i32` product, so the computation is total and never panics.
///
/// # Errors
/// Returns [`ReconError::InverseTransformLengthMismatch`] if `out.len()` does not
/// equal `src.len()`.
pub fn inverse_identity_transform(
    src: &[i32],
    scale: i32,
    shift: u8,
    col_tx: bool,
    bit_depth: BitDepth,
    out: &mut [i32],
) -> Result<()> {
    if out.len() != src.len() {
        return Err(ReconError::InverseTransformLengthMismatch {
            src_len: src.len(),
            out_len: out.len(),
        });
    }
    let (lo, hi) = transform_clip_bounds(col_tx, bit_depth);
    let input_bound = transform_input_bound(bit_depth);
    let scale = scale.clamp(0, 362);
    for (slot, &coeff) in out.iter_mut().zip(src) {
        let scaled = coeff.clamp(-input_bound, input_bound - 1) * scale;
        *slot = round2_i32(scaled, u32::from(shift)).clamp(lo, hi);
    }
    Ok(())
}

/// The AV2 § 7.15.2.1 / § 7.15.2.3 `Clip3` bounds
/// `[-(1 << (BitDepth + (colTx ? 0 : 7))), (1 << (BitDepth + (colTx ? 0 : 7))) - 1]`.
fn transform_clip_bounds(col_tx: bool, bit_depth: BitDepth) -> (i32, i32) {
    let bound = 1i32 << (u32::from(bit_depth.bits()) + if col_tx { 0 } else { 7 });
    (-bound, bound - 1)
}

fn transform_input_bound(bit_depth: BitDepth) -> i32 {
    1i32 << (u32::from(bit_depth.bits()) + 7)
}

/// Applies one § 7.15.2.1 kernel matrix multiply: `out[i] = Clip3(lo, hi,
/// Round2(sum over j of kernel[j][i] * src[j], shift))`, indexing the kernel
/// column in reverse (`kernel[j][N - 1 - i]`) when `reversed` (the `FDDT`
/// dispatch branch).
///
/// The accumulation runs kernel-row-major and skips zero coefficients (adding
/// zero terms is an identity over the same `i32` sums). `src` and `out` have
/// length `N` by the caller's dispatch; the `zip`s make the loops total either
/// way. Every kernel entry has magnitude below `2^7`; after the AV2 input clamp,
/// `|acc| <= 32 * 2^7 * 2^17 < 2^30`.
fn kernel_transform<const N: usize>(
    src: &[i32],
    kernel: &[[i32; N]; N],
    reversed: bool,
    shift: u8,
    col_tx: bool,
    bit_depth: BitDepth,
    out: &mut [i32],
) {
    let input_bound = transform_input_bound(bit_depth);
    let (lo, hi) = transform_clip_bounds(col_tx, bit_depth);
    let mut acc = [0i32; N];
    for (&coeff, kernel_row) in src.iter().zip(kernel.iter()) {
        if coeff == 0 {
            continue;
        }
        let c = coeff.clamp(-input_bound, input_bound - 1);
        for (slot, &k) in acc.iter_mut().zip(kernel_row.iter()) {
            *slot += k * c;
        }
    }
    if reversed {
        acc.reverse();
    }
    for (slot, &value) in out.iter_mut().zip(acc.iter()) {
        *slot = round2_i32(value, u32::from(shift)).clamp(lo, hi);
    }
}

/// AV2 § 4.8 `Round2(x, n)`: `n == 0` returns `x`, else `(x + (1 << (n - 1))) >> n`
/// with arithmetic (sign-extending) shift.
///
/// For `1 <= n < 64` this uses the floor-shift identity
/// `Round2(x, n) == ((x >> (n - 1)) + 1) >> 1`: writing `x = q * 2^(n-1) + r`
/// with `0 <= r < 2^(n-1)`, both sides equal `floor((q + 1) / 2)`. The `+ 1`
/// cannot overflow because every caller bounds `|value|` at `2^62` (kernel
/// accumulators stay below `2^43`; the identity transform's product is at most
/// `|i32::MIN| * |i32::MIN| = 2^62`), so `(value >> (n - 1)) + 1 <= 2^62 + 1`.
/// A `shift` at or above the `i64` width saturates to the arithmetic-shift
/// limit instead of shifting out of range.
#[cfg(test)]
fn round2(value: i64, shift: u8) -> i64 {
    if shift == 0 {
        return value;
    }
    let shift = u32::from(shift);
    if shift >= i64::BITS {
        return value >> (i64::BITS - 1);
    }
    ((value >> (shift - 1)) + 1) >> 1
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(
        src: &[i32],
        tx_type: InverseTransform1dType,
        shift: u8,
        col_tx: bool,
        bit_depth: BitDepth,
    ) -> Vec<i32> {
        let mut out = vec![0; src.len()];
        inverse_transform_1d(src, tx_type, shift, col_tx, bit_depth, &mut out).unwrap();
        out
    }

    #[test]
    fn dc_only_dct4_is_flat() {
        assert_eq!(
            run(
                &[2, 0, 0, 0],
                InverseTransform1dType::Dct,
                0,
                true,
                BitDepth::Eight
            ),
            [128, 128, 128, 128]
        );
    }

    #[test]
    fn dct4_single_coefficient_matches_kernel_row() {
        assert_eq!(
            run(
                &[0, 2, 0, 0],
                InverseTransform1dType::Dct,
                0,
                true,
                BitDepth::Eight
            ),
            [166, 70, -70, -166]
        );
    }

    #[test]
    fn round2_applies_arithmetic_downshift() {
        assert_eq!(
            run(
                &[0, 2, 0, 0],
                InverseTransform1dType::Dct,
                1,
                true,
                BitDepth::Eight
            ),
            [83, 35, -35, -83]
        );
    }

    #[test]
    fn col_tx_flag_selects_clamp_range() {
        assert_eq!(
            run(
                &[0, 100, 0, 0],
                InverseTransform1dType::Dct,
                0,
                true,
                BitDepth::Eight
            ),
            [255, 255, -256, -256]
        );
        assert_eq!(
            run(
                &[0, 100, 0, 0],
                InverseTransform1dType::Dct,
                0,
                false,
                BitDepth::Eight
            ),
            [8300, 3500, -3500, -8300]
        );
    }

    #[test]
    fn fddt_is_the_column_reverse_of_ddtx() {
        let src = [3, -7, 11, -2, 9, 4, -5, 1];
        let ddtx = run(&src, InverseTransform1dType::Ddtx, 2, false, BitDepth::Ten);
        let fddt = run(&src, InverseTransform1dType::Fddt, 2, false, BitDepth::Ten);
        let reversed: Vec<i32> = ddtx.iter().rev().copied().collect();
        assert_eq!(fddt, reversed);
    }

    #[test]
    fn length_4_maps_fdst_ddtx_and_fddt_to_the_fdst_kernel() {
        let src = [5, -3, 8, 2];
        let fdst = run(
            &src,
            InverseTransform1dType::Fdst,
            1,
            false,
            BitDepth::Eight,
        );
        let ddtx = run(
            &src,
            InverseTransform1dType::Ddtx,
            1,
            false,
            BitDepth::Eight,
        );
        let fddt = run(
            &src,
            InverseTransform1dType::Fddt,
            1,
            false,
            BitDepth::Eight,
        );
        assert_eq!(fdst, ddtx);
        assert_eq!(fdst, fddt);
    }

    #[test]
    fn length_32_uses_dct_kernel_for_every_type() {
        let src: Vec<i32> = (0..32).map(|j| j - 16).collect();
        let dct = run(&src, InverseTransform1dType::Dct, 2, false, BitDepth::Ten);
        let adst = run(&src, InverseTransform1dType::Adst, 2, false, BitDepth::Ten);
        let fddt = run(&src, InverseTransform1dType::Fddt, 2, false, BitDepth::Ten);
        assert_eq!(dct, adst);
        assert_eq!(dct, fddt);
    }

    #[test]
    fn rejects_unsupported_size() {
        let mut out = vec![0; 5];
        assert!(matches!(
            inverse_transform_1d(
                &[0; 5],
                InverseTransform1dType::Dct,
                0,
                true,
                BitDepth::Eight,
                &mut out,
            ),
            Err(ReconError::InvalidInverseTransformSize { size: 5 })
        ));
    }

    #[test]
    fn rejects_output_length_mismatch() {
        let mut out = vec![0; 3];
        assert!(matches!(
            inverse_transform_1d(
                &[0; 4],
                InverseTransform1dType::Dct,
                0,
                true,
                BitDepth::Eight,
                &mut out,
            ),
            Err(ReconError::InverseTransformLengthMismatch {
                src_len: 4,
                out_len: 3
            })
        ));
    }

    fn round2_reference(value: i64, shift: u8) -> i64 {
        if shift == 0 {
            return value;
        }
        let shift = u32::from(shift);
        if shift >= i64::BITS {
            return value >> (i64::BITS - 1);
        }
        ((i128::from(value) + (1i128 << (shift - 1))) >> shift) as i64
    }

    #[test]
    fn round2_shift_identity_matches_wide_reference() {
        let values = [
            0i64,
            1,
            -1,
            2,
            -2,
            3,
            -3,
            7,
            -7,
            1017,
            -1017,
            123_456_789,
            -123_456_789,
            i64::from(i32::MAX),
            i64::from(i32::MIN),
            (1i64 << 43) - 1,
            -(1i64 << 43),
            (1i64 << 62) - 12_345,
            1i64 << 62,
            -(1i64 << 62),
        ];
        for &value in &values {
            for shift in 1..=63u8 {
                assert_eq!(
                    round2(value, shift),
                    round2_reference(value, shift),
                    "round2({value}, {shift})"
                );
            }
        }
    }

    fn reference_kernel_sum(
        src: &[i32],
        tx_type: InverseTransform1dType,
        sz: usize,
        i: usize,
    ) -> i64 {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        let mut s: i64 = 0;
        for (j, &coeff) in src.iter().enumerate() {
            let k = match sz {
                4 => match tx_type {
                    Dct => DCT_KERNEL4[j][i],
                    Adst => ADST_KERNEL4[j][i],
                    Fdst | Ddtx | Fddt => FDST_KERNEL4[j][i],
                },
                8 => match tx_type {
                    Dct => DCT_KERNEL8[j][i],
                    Adst => ADST_KERNEL8[j][i],
                    Fdst => FDST_KERNEL8[j][i],
                    Ddtx => DDTX_KERNEL8[j][i],
                    Fddt => DDTX_KERNEL8[j][7 - i],
                },
                16 => match tx_type {
                    Dct => DCT_KERNEL16[j][i],
                    Adst => ADST_KERNEL16[j][i],
                    Fdst => FDST_KERNEL16[j][i],
                    Ddtx => DDTX_KERNEL16[j][i],
                    Fddt => DDTX_KERNEL16[j][15 - i],
                },
                _ => DCT_KERNEL32[j][i],
            };
            s += i64::from(k) * i64::from(coeff);
        }
        s
    }

    #[test]
    fn kernel_transform_matches_per_element_reference() {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        let mut lcg: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move || {
            lcg = lcg.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((lcg >> 33) as i32 % 40_000) - 17_331
        };
        for sz in [4usize, 8, 16, 32] {
            for tx_type in [Dct, Adst, Fdst, Ddtx, Fddt] {
                for (shift, col_tx, bit_depth) in [
                    (0u8, true, BitDepth::Eight),
                    (2, false, BitDepth::Ten),
                    (7, true, BitDepth::Ten),
                    (12, false, BitDepth::Eight),
                ] {
                    let mut src: Vec<i32> = (0..sz).map(|_| next()).collect();
                    for slot in src.iter_mut().step_by(3) {
                        *slot = 0;
                    }
                    let got = run(&src, tx_type, shift, col_tx, bit_depth);
                    let (lo, hi) = transform_clip_bounds(col_tx, bit_depth);
                    let input_bound = transform_input_bound(bit_depth);
                    let clamped: Vec<i32> = src
                        .iter()
                        .map(|&value| value.clamp(-input_bound, input_bound - 1))
                        .collect();
                    let expected: Vec<i32> = (0..sz)
                        .map(|i| {
                            let s = reference_kernel_sum(&clamped, tx_type, sz, i);
                            round2_reference(s, shift).clamp(i64::from(lo), i64::from(hi)) as i32
                        })
                        .collect();
                    assert_eq!(got, expected, "sz={sz} tx={tx_type:?} shift={shift}");
                }
            }
        }
    }

    #[test]
    fn all_zero_input_produces_all_zero_output_for_every_kernel() {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        for sz in [4usize, 8, 16, 32] {
            for tx_type in [Dct, Adst, Fdst, Ddtx, Fddt] {
                for col_tx in [false, true] {
                    let src = vec![0i32; sz];
                    let out = run(&src, tx_type, 3, col_tx, BitDepth::Ten);
                    assert_eq!(out, vec![0i32; sz], "sz={sz} tx={tx_type:?}");
                }
            }
        }
    }

    #[test]
    fn round2_matches_spec_and_is_total_for_large_shift() {
        assert_eq!(round2(0, 0), 0);
        assert_eq!(round2(7, 0), 7);
        assert_eq!(round2(6, 2), 2); // (6 + 2) >> 2
        assert_eq!(round2(-6, 2), -1); // (-6 + 2) >> 2 = -4 >> 2
        assert_eq!(round2(1_000, 64), 0);
        assert_eq!(round2(-1_000, 200), -1);
    }

    #[test]
    fn walsh_hadamard_matches_spec_butterfly() {
        assert_eq!(inverse_walsh_hadamard([4, 0, 0, 0], 0), [2, 2, 2, 2]);
        assert_eq!(inverse_walsh_hadamard([0, 0, 0, 4], 0), [2, -2, 2, -2]);
    }

    #[test]
    fn walsh_hadamard_applies_pre_shift() {
        assert_eq!(inverse_walsh_hadamard([8, 0, 0, 0], 1), [2, 2, 2, 2]);
    }

    #[test]
    fn walsh_hadamard_is_total_for_large_shift() {
        assert_eq!(inverse_walsh_hadamard([4, 0, 0, 0], 64), [0, 0, 0, 0]);
        assert_eq!(inverse_walsh_hadamard([7, 3, 1, 5], 200), [0, 0, 0, 0]);
    }

    fn identity(src: &[i32], scale: i32, shift: u8, col_tx: bool, bit_depth: BitDepth) -> Vec<i32> {
        let mut out = vec![0; src.len()];
        inverse_identity_transform(src, scale, shift, col_tx, bit_depth, &mut out).unwrap();
        out
    }

    #[test]
    fn identity_scales_rounds_and_clamps() {
        assert_eq!(identity(&[10, -4], 181, 8, false, BitDepth::Eight), [7, -3]);
    }

    #[test]
    fn identity_clamp_uses_col_tx_range() {
        assert_eq!(identity(&[1000], 256, 0, true, BitDepth::Eight), [255]);
        assert_eq!(identity(&[1000], 256, 0, false, BitDepth::Eight), [32767]);
    }

    #[test]
    fn identity_rejects_output_length_mismatch() {
        let mut out = vec![0; 2];
        assert!(matches!(
            inverse_identity_transform(&[0; 4], 64, 0, true, BitDepth::Eight, &mut out),
            Err(ReconError::InverseTransformLengthMismatch {
                src_len: 4,
                out_len: 2
            })
        ));
    }

    #[test]
    fn identity_is_total_for_extreme_inputs() {
        assert_eq!(
            identity(&[i32::MIN], i32::MIN, 63, false, BitDepth::Ten),
            [0]
        );
    }
}
