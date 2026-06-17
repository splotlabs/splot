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

    let (lo, hi) = transform_clip_bounds(col_tx, bit_depth);

    for (i, slot) in out.iter_mut().enumerate() {
        let s = kernel_sum(src, tx_type, sz, i);
        *slot = round2(s, shift).clamp(lo, hi) as i32;
    }
    Ok(())
}

/// AV2 § 7.15.2.2 inverse Walsh-Hadamard transform: the 4-element lossless
/// butterfly over `src` with a pre-scaling `shift`.
///
/// This is the lossless-block 1D transform. It applies no `Clip3` (the spec
/// produces the butterfly result directly) and is total: the additions use `i64`
/// intermediates and the result is returned as `i32` (lossless residuals are
/// bounded well within `i32`).
#[must_use]
pub fn inverse_walsh_hadamard(src: [i32; 4], shift: u8) -> [i32; 4] {
    let shift = u32::from(shift);
    let mut a = i64::from(src[0]) >> shift;
    let mut c = i64::from(src[1]) >> shift;
    let mut d = i64::from(src[2]) >> shift;
    let mut b = i64::from(src[3]) >> shift;
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;
    [a as i32, b as i32, c as i32, d as i32]
}

/// Applies the AV2 § 7.15.2.3 inverse identity transform to `src`, writing `out`.
///
/// Each output is `Clip3(bound, Round2(src[i] * scale, shift))` with the same
/// `colTx`-dependent bound as [`inverse_transform_1d`]. `scale` is the
/// caller-supplied § 7.15.4.1 `get_identity_scale` value, `shift` the down-shift,
/// `col_tx` the column-pass flag, and `bit_depth` the active decoded bit depth.
/// The computation uses `i64` intermediates, so it is total and never panics.
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
    for (slot, &coeff) in out.iter_mut().zip(src) {
        let scaled = i64::from(coeff) * i64::from(scale);
        *slot = round2(scaled, shift).clamp(lo, hi) as i32;
    }
    Ok(())
}

/// The AV2 § 7.15.2.1 / § 7.15.2.3 `Clip3` bounds
/// `[-(1 << (BitDepth + (colTx ? 0 : 7))), (1 << (BitDepth + (colTx ? 0 : 7))) - 1]`.
fn transform_clip_bounds(col_tx: bool, bit_depth: BitDepth) -> (i64, i64) {
    let bound: i64 = 1 << (u32::from(bit_depth.bits()) + if col_tx { 0 } else { 7 });
    (-bound, bound - 1)
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

    #[test]
    fn walsh_hadamard_matches_spec_butterfly() {
        // src = [4,0,0,0], shift 0: a=4,c=d=b=0 -> a=4, d=0, e=2, b=2, c=2,
        // a=2, d=2 => [2, 2, 2, 2].
        assert_eq!(inverse_walsh_hadamard([4, 0, 0, 0], 0), [2, 2, 2, 2]);
        // src = [0,0,0,4], shift 0: a=0,c=0,d=0,b=4 -> d=-4, e=2, b=-2, c=2,
        // a=2, d=-2 => [2, -2, 2, -2].
        assert_eq!(inverse_walsh_hadamard([0, 0, 0, 4], 0), [2, -2, 2, -2]);
    }

    #[test]
    fn walsh_hadamard_applies_pre_shift() {
        // src = [8,0,0,0], shift 1 pre-scales src[0] to 4, then the [4,0,0,0]
        // butterfly gives [2, 2, 2, 2].
        assert_eq!(inverse_walsh_hadamard([8, 0, 0, 0], 1), [2, 2, 2, 2]);
    }

    fn identity(src: &[i32], scale: i32, shift: u8, col_tx: bool, bit_depth: BitDepth) -> Vec<i32> {
        let mut out = vec![0; src.len()];
        inverse_identity_transform(src, scale, shift, col_tx, bit_depth, &mut out).unwrap();
        out
    }

    #[test]
    fn identity_scales_rounds_and_clamps() {
        // Round2(10*181, 8) = (1810 + 128) >> 8 = 7; Round2(-4*181, 8) =
        // (-724 + 128) >> 8 = -596 >> 8 = -3; both within the col_tx=false range.
        assert_eq!(identity(&[10, -4], 181, 8, false, BitDepth::Eight), [7, -3]);
    }

    #[test]
    fn identity_clamp_uses_col_tx_range() {
        // 1000 * 256 = 256000. col_tx=true (8-bit) clamps to 255; col_tx=false
        // clamps to the wider 32767.
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
}
