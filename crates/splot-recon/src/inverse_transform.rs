// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.2.1 1D inverse transform process.
//!
//! This module implements the scheduler-free AV2 § 7.15.2.1 kernel-based 1D
//! inverse transform
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-15-2-1`): a matrix multiplication of the input coefficients by the
//! size-and-type transform kernel, followed by § 4.8 `Round2` and the § 7.15.2.1
//! `colTx`-dependent `Clip3`. The kernels come from the dependency-free
//! `splot-tables` crate (§ 9.6, generated verbatim from the spec attachment).
//!
//! Feature tracking: `RECON-INVERSE-TRANSFORM-1D`.
//!
//! Scope: this is the § 7.15.2.1 kernel transform only. The § 7.15.2.2 inverse
//! Walsh-Hadamard transform, the § 7.15.2.3 inverse identity transform (the
//! Table 7.1 `IDT` type), the § 7.15.3 secondary transform, and the § 7.15.4 2D
//! inverse transform that orchestrates row/column passes (including the
//! `Transform_Shift` / `get_transform_1d_type` derivations) are out of scope and
//! tracked by their own future rows. The caller supplies the already-derived
//! 1D transform type, shift, and `colTx` flag.

use splot_tables::tables::transform_1d::{
    ADST_KERNEL4, ADST_KERNEL8, ADST_KERNEL16, DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16,
    DCT_KERNEL32, DDTX_KERNEL8, DDTX_KERNEL16, FDST_KERNEL4, FDST_KERNEL8, FDST_KERNEL16,
};

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
/// 1D kernel). The accumulation uses `i64` intermediates, so the matrix multiply
/// cannot overflow for in-range dequantized inputs, and the final `Clip3` bound
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
    if !matches!(sz, 4 | 8 | 16 | 32) {
        return Err(ReconError::InvalidInverseTransformSize { size: sz });
    }
    if out.len() != sz {
        return Err(ReconError::InverseTransformLengthMismatch {
            src_len: sz,
            out_len: out.len(),
        });
    }

    let bound: i64 = 1 << (u32::from(bit_depth.bits()) + if col_tx { 0 } else { 7 });
    let (lo, hi) = (-bound, bound - 1);

    for (i, slot) in out.iter_mut().enumerate() {
        let s = kernel_sum(src, tx_type, sz, i);
        *slot = round2(s, shift).clamp(lo, hi) as i32;
    }
    Ok(())
}

/// Computes `sum over j of kernel[j][i] * src[j]` for the § 7.15.2.1 dispatch.
///
/// `sz` is guaranteed to be 4, 8, 16, or 32 and `i` is in `0..sz` by the caller.
fn kernel_sum(src: &[i32], tx_type: InverseTransform1dType, sz: usize, i: usize) -> i64 {
    use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
    let mut s: i64 = 0;
    match sz {
        4 => {
            for (j, &coeff) in src.iter().enumerate() {
                let k = match tx_type {
                    Dct => DCT_KERNEL4[j][i],
                    Adst => ADST_KERNEL4[j][i],
                    // Spec § 7.15.2.1 length-4 `else`: FDST/DDTX/FDDT use Fdst4.
                    Fdst | Ddtx | Fddt => FDST_KERNEL4[j][i],
                };
                s += i64::from(k) * i64::from(coeff);
            }
        }
        8 => {
            for (j, &coeff) in src.iter().enumerate() {
                let k = match tx_type {
                    Dct => DCT_KERNEL8[j][i],
                    Adst => ADST_KERNEL8[j][i],
                    Fdst => FDST_KERNEL8[j][i],
                    Ddtx => DDTX_KERNEL8[j][i],
                    Fddt => DDTX_KERNEL8[j][7 - i],
                };
                s += i64::from(k) * i64::from(coeff);
            }
        }
        16 => {
            for (j, &coeff) in src.iter().enumerate() {
                let k = match tx_type {
                    Dct => DCT_KERNEL16[j][i],
                    Adst => ADST_KERNEL16[j][i],
                    Fdst => FDST_KERNEL16[j][i],
                    Ddtx => DDTX_KERNEL16[j][i],
                    Fddt => DDTX_KERNEL16[j][15 - i],
                };
                s += i64::from(k) * i64::from(coeff);
            }
        }
        // Spec § 7.15.2.1 length-32: the DCT kernel is used for every type.
        _ => {
            for (j, &coeff) in src.iter().enumerate() {
                s += i64::from(DCT_KERNEL32[j][i]) * i64::from(coeff);
            }
        }
    }
    s
}

/// AV2 § 4.8 `Round2(x, n)`: `n == 0` returns `x`, else `(x + (1 << (n - 1))) >> n`
/// with arithmetic (sign-extending) shift over `i64`.
///
/// Total and panic-free for every `shift`: a `shift` at or above the `i64` width
/// can only come from an out-of-contract caller (the spec's `Transform_Shift`
/// values are small), so it saturates to the arithmetic-shift limit instead of
/// overflowing.
fn round2(value: i64, shift: u8) -> i64 {
    if shift == 0 {
        return value;
    }
    let shift = u32::from(shift);
    if shift >= i64::BITS {
        return value >> (i64::BITS - 1);
    }
    (value + (1i64 << (shift - 1))) >> shift
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
        // DCT_KERNEL4 row 0 (the DC basis) is [64, 64, 64, 64], so a DC-only
        // input yields a constant field: Round2(64 * 2, 0) = 128 everywhere.
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
        // Only j = 1 is non-zero, so out[i] = DCT_KERNEL4[1][i] * 2; row 1 is
        // [83, 35, -35, -83], all within the col_tx 8-bit range [-256, 255].
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
        // Same dot products [166, 70, -70, -166] with shift 1:
        // Round2(166,1)=83, Round2(70,1)=35, Round2(-70,1)=-35, Round2(-166,1)=-83.
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
        // DCT_KERNEL4[1] * 100 = [8300, 3500, -3500, -8300].
        // col_tx = true (8-bit) clamps to [-256, 255]; col_tx = false uses the
        // wider [-32768, 32767] range and leaves the values unclamped.
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
        // Per the spec, FDDT indexes the DDTX kernel column in reverse, so
        // FDDT output[i] == DDTX output[sz - 1 - i].
        let src = [3, -7, 11, -2, 9, 4, -5, 1];
        let ddtx = run(&src, InverseTransform1dType::Ddtx, 2, false, BitDepth::Ten);
        let fddt = run(&src, InverseTransform1dType::Fddt, 2, false, BitDepth::Ten);
        let reversed: Vec<i32> = ddtx.iter().rev().copied().collect();
        assert_eq!(fddt, reversed);
    }

    #[test]
    fn length_4_maps_fdst_ddtx_and_fddt_to_the_fdst_kernel() {
        // At length 4 the spec `else` routes FDST, DDTX, and FDDT to Fdst4.
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
        // At length 32 the spec uses the DCT kernel regardless of type.
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

    #[test]
    fn round2_matches_spec_and_is_total_for_large_shift() {
        // In-contract: Round2(x, n) = (x + (1 << (n - 1))) >> n.
        assert_eq!(round2(0, 0), 0);
        assert_eq!(round2(7, 0), 7);
        assert_eq!(round2(6, 2), 2); // (6 + 2) >> 2
        assert_eq!(round2(-6, 2), -1); // (-6 + 2) >> 2 = -4 >> 2
        // Out-of-contract huge shift saturates to the arithmetic limit, no panic.
        assert_eq!(round2(1_000, 64), 0);
        assert_eq!(round2(-1_000, 200), -1);
    }
}
