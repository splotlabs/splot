// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.3 secondary (inverse) transform process.
//!
//! This module implements the scheduler-free matrix-based secondary transform
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-15-3`) that runs over the dequantized coefficients before the § 7.15.4
//! 2D inverse transform when `sec_tx_type != 0`. It gathers the first `n`
//! coefficients in the caller-resolved primary scan order, multiplies them by
//! the § 9.7 IST kernel, and scatters the `Round2Signed` / `Clip3` results back
//! into the top-left scan sub-block of `Dequant`.
//!
//! Feature tracking: `RECON-SECONDARY-INVERSE-TRANSFORM`.

use splot_tables::tables::secondary_transform::{IST_4X4_KERNEL, IST_8X8_KERNEL, STX_SCAN_MAP};

use crate::coefficient_scan::TransformClass;
use crate::math::round2_signed_i32;
use crate::{BitDepth, ReconError, Result};

/// AV2 § 3 `IST_4X4_HEIGHT`: rows in the 4x4 secondary-transform matrix.
const IST_4X4_HEIGHT: usize = 8;
/// AV2 § 3 `IST_4X4_WIDTH`: columns in the 4x4 secondary-transform matrix.
const IST_4X4_WIDTH: usize = 16;
/// AV2 § 3 `IST_8X8_HEIGHT`: rows in the 8x8 secondary-transform matrix.
const IST_8X8_HEIGHT: usize = 32;
/// AV2 § 3 `IST_8X8_WIDTH`: columns in the 8x8 secondary-transform matrix.
const IST_8X8_WIDTH: usize = 48;
/// AV2 § 3 `IST_SET_SIZE_4X4`: number of 4x4 secondary-transform kernel sets.
const IST_SET_SIZE_4X4: usize = 14;
/// AV2 § 3 `IST_SET_SIZE_8X8`: number of 8x8 secondary-transform kernel sets.
const IST_SET_SIZE_8X8: usize = 11;
/// AV2 § 3 `STX_TYPES`: number of secondary transform types (`sec_tx_type`
/// `0..=3`; `0` is "no secondary transform", and types `1..=3` index the kernel
/// at `sec_tx_type - 1`).
const STX_TYPES: usize = 4;

/// AV2 § 7.15.3 `Stx_Scan_Order_4x4[IST_4X4_WIDTH]`, transcribed verbatim from the
/// spec process body (`07-decoding-process.md#s-7-15-3`). It is a § 7.15.3
/// process-body constant absent from the generated `all_tables.h`, so it is a
/// hand-written, spec-cited constant.
const STX_SCAN_ORDER_4X4: [usize; IST_4X4_WIDTH] =
    [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// AV2 § 7.15.3 `Stx_Scan_Order_8x8[64]`, transcribed verbatim from the spec
/// process body (`07-decoding-process.md#s-7-15-3`). Like `Stx_Scan_Order_4x4`,
/// it is a hand-written, spec-cited process-body constant.
const STX_SCAN_ORDER_8X8: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Caller-resolved parameters for the AV2 § 7.15.3 secondary transform.
///
/// `w` / `h` are the operating dimensions `Min(32, Tx_Width[txSz])` /
/// `Min(32, Tx_Height[txSz])` (each a power of two in `4..=32`); `large` (whether
/// `w >= 8 && h >= 8`), `bwl` (`Min(5, Tx_Width_Log2[txSz])`, i.e. `log2(w)`), and
/// the output width are derived internally. `n` is the § 7.15.3 input
/// coefficient count (`IST_4X4_HEIGHT`, `IST_8X8_HEIGHT`, or the reduced
/// `IST_8X8_HEIGHT_RED`); `kernel` and `sec_tx_type` (`1..=3`) select the § 9.7
/// IST kernel; `primary_scan_class` aligns the IST input support with the
/// primary transform scan; `transpose` chooses the § 7.15.3 output layout;
/// `bit_depth` bounds the `Clip3`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecondaryInverseTransform {
    /// Operating width `Min(32, Tx_Width[txSz])` (a power of two in `4..=32`).
    pub w: usize,
    /// Operating height `Min(32, Tx_Height[txSz])` (a power of two in `4..=32`).
    pub h: usize,
    /// § 7.15.3 input coefficient count (`n`), at most the kernel height.
    pub n: usize,
    /// § 9.7 IST kernel-set index (`kernel`).
    pub kernel: usize,
    /// § 7.15.3 secondary transform type (`sec_tx_type`, `1..=3`).
    pub sec_tx_type: usize,
    /// AVM-compatible input scan class for the primary transform.
    pub primary_scan_class: TransformClass,
    /// § 7.15.3 output transpose flag.
    pub transpose: bool,
    /// Active decoded bit depth (bounds the `Clip3`).
    pub bit_depth: BitDepth,
}

/// Applies the AV2 § 7.15.3 secondary inverse transform to the dequantized block
/// `dequant` (a `w * h` row-major array), modifying it in place.
///
/// The process gathers the first `params.n` coefficients of `dequant` in
/// § 5.20.7.30 2D scan order (zeroing those positions), multiplies the gathered
/// vector by the § 9.7 IST kernel (`Ist_4x4_Kernel` or `Ist_8x8_Kernel` selected
/// by `large = w >= 8 && h >= 8`), clears the dequantized coefficient block as
/// AVM does before writeback, applies `Round2Signed(t, 7)` and the
/// `Clip3(±(1 << (BitDepth + 7)))` bound, and scatters the results into the
/// top-left scan sub-block via `Stx_Scan_Order_4x4` / `Stx_Scan_Order_8x8` (and,
/// for the large case, `Stx_Scan_Map`), honoring `transpose`.
///
/// The computation is total and panic-free for valid inputs: the accumulation
/// uses `i32` (at most 32 products of § 7.14.4-bounded coefficients and kernels
/// below 128, fitting below 2^30), every table index is validated before use,
/// and the scan scratch is a fixed 32x32 stack buffer.
///
/// # Errors
/// Returns [`ReconError::SecondaryTransformInvalidShape`] if `w` / `h` are not
/// powers of two in `4..=32`, [`ReconError::SecondaryTransformBufferMismatch`] if
/// `dequant` is not `w * h`, and [`ReconError::SecondaryTransformInvalidParams`]
/// if `n`, `kernel`, or `sec_tx_type` are out of range for the selected kernel
/// set. All inputs are validated before any coefficient is modified.
pub fn secondary_inverse_transform(
    dequant: &mut [i32],
    params: &SecondaryInverseTransform,
) -> Result<()> {
    let SecondaryInverseTransform {
        w,
        h,
        n,
        kernel,
        sec_tx_type,
        primary_scan_class,
        transpose,
        bit_depth,
    } = *params;

    if !is_valid_side(w) || !is_valid_side(h) {
        return Err(ReconError::SecondaryTransformInvalidShape { w, h });
    }
    let expected = w * h;
    if dequant.len() != expected {
        return Err(ReconError::SecondaryTransformBufferMismatch {
            expected,
            actual: dequant.len(),
        });
    }

    let large = w >= 8 && h >= 8;
    let (set_size, kernel_height, kernel_width) = if large {
        (IST_SET_SIZE_8X8, IST_8X8_HEIGHT, IST_8X8_WIDTH)
    } else {
        (IST_SET_SIZE_4X4, IST_4X4_HEIGHT, IST_4X4_WIDTH)
    };
    if n == 0 || n > kernel_height || kernel >= set_size || !(1..STX_TYPES).contains(&sec_tx_type) {
        return Err(ReconError::SecondaryTransformInvalidParams {
            n,
            kernel,
            sec_tx_type,
        });
    }
    let stx = sec_tx_type - 1;

    let scan = crate::coefficient_scan_slice(w, h, primary_scan_class)?;
    let bound = 1i32 << (u32::from(bit_depth.bits()) + 7);
    let mut coefs = [0i32; IST_8X8_HEIGHT];
    for (slot, &pos) in coefs[..n].iter_mut().zip(scan.iter()) {
        let index = pos as usize;
        *slot = dequant[index].clamp(-bound, bound - 1);
    }
    dequant.fill(0);

    let (scan_bwl, scan_w) = if large {
        (3u32, 8usize)
    } else {
        (2u32, 4usize)
    };
    for i in 0..kernel_width {
        let mut t = 0i32;
        for (j, &coef) in coefs[..n].iter().enumerate() {
            let weight = if large {
                IST_8X8_KERNEL[kernel][stx][j][i]
            } else {
                IST_4X4_KERNEL[kernel][stx][j][i]
            };
            t += coef * weight;
        }
        let v = round2_signed_i32(t, 7).clamp(-bound, bound - 1);

        let pos = if large {
            let mapped = usize::try_from(STX_SCAN_MAP[kernel][stx][i]).map_err(|_| {
                ReconError::SecondaryTransformInvalidParams {
                    n,
                    kernel,
                    sec_tx_type,
                }
            })?;
            STX_SCAN_ORDER_8X8[mapped]
        } else {
            STX_SCAN_ORDER_4X4[i]
        };
        let x = pos & (scan_w - 1);
        let y = pos >> scan_bwl;
        let out_index = if transpose { x * w + y } else { y * w + x };
        dequant[out_index] = v;
    }
    Ok(())
}

/// Whether `side` is a valid § 7.15.3 operating side: a power of two in `4..=32`.
const fn is_valid_side(side: usize) -> bool {
    matches!(side, 4 | 8 | 16 | 32)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::coefficient_scan_order;

    fn round2_signed_reference(value: i64, shift: u32) -> i64 {
        let magnitude = value.unsigned_abs();
        let rounded = if shift == 0 {
            magnitude
        } else {
            (magnitude + (1u64 << (shift - 1))) >> shift
        };
        if value < 0 {
            -(rounded as i64)
        } else {
            rounded as i64
        }
    }

    fn params(
        w: usize,
        h: usize,
        n: usize,
        kernel: usize,
        sec: usize,
        transpose: bool,
    ) -> SecondaryInverseTransform {
        SecondaryInverseTransform {
            w,
            h,
            n,
            kernel,
            sec_tx_type: sec,
            primary_scan_class: TransformClass::TwoD,
            transpose,
            bit_depth: BitDepth::Eight,
        }
    }

    #[allow(clippy::many_single_char_names)]
    fn reference(dequant: &mut [i32], p: &SecondaryInverseTransform) {
        let large = p.w >= 8 && p.h >= 8;
        let (kernel_width, scan_bwl, scan_w) = if large {
            (IST_8X8_WIDTH, 3u32, 8usize)
        } else {
            (IST_4X4_WIDTH, 2u32, 4usize)
        };
        let mut scan = [0u16; 32 * 32];
        let scan = &mut scan[..p.w * p.h];
        coefficient_scan_order(p.w, p.h, p.primary_scan_class, scan).unwrap();
        let coefs: Vec<i64> = scan[..p.n]
            .iter()
            .map(|&pos| i64::from(dequant[pos as usize]))
            .collect();
        dequant.fill(0);
        let stx = p.sec_tx_type - 1;
        let bound = 1i64 << (u32::from(p.bit_depth.bits()) + 7);
        for i in 0..kernel_width {
            let mut t = 0i64;
            for (j, &c) in coefs.iter().enumerate() {
                let weight = if large {
                    IST_8X8_KERNEL[p.kernel][stx][j][i]
                } else {
                    IST_4X4_KERNEL[p.kernel][stx][j][i]
                };
                t += c * i64::from(weight);
            }
            let v = round2_signed_reference(t, 7).clamp(-bound, bound - 1) as i32;
            let pos = if large {
                STX_SCAN_ORDER_8X8[STX_SCAN_MAP[p.kernel][stx][i] as usize]
            } else {
                STX_SCAN_ORDER_4X4[i]
            };
            let x = pos & (scan_w - 1);
            let y = pos >> scan_bwl;
            dequant[if p.transpose {
                x * p.w + y
            } else {
                y * p.w + x
            }] = v;
        }
    }

    fn assert_matches_reference<const N: usize>(base: [i32; N], p: &SecondaryInverseTransform) {
        let mut produced = base;
        secondary_inverse_transform(&mut produced, p).unwrap();
        let mut expected = base;
        reference(&mut expected, p);
        assert_eq!(produced, expected);
    }

    #[test]
    fn small_4x4_matches_independent_reference() {
        let base: [i32; 16] = core::array::from_fn(|i| (i as i32 - 7) * 11);
        assert_matches_reference(base, &params(4, 4, IST_4X4_HEIGHT, 3, 2, false));
    }

    #[test]
    fn small_4x4_transpose_swaps_output_axes() {
        let base: [i32; 16] = core::array::from_fn(|i| (i as i32) * 5 - 30);
        let p_plain = params(4, 4, IST_4X4_HEIGHT, 1, 1, false);
        let p_transposed = params(4, 4, IST_4X4_HEIGHT, 1, 1, true);
        assert_matches_reference(base, &p_plain);
        assert_matches_reference(base, &p_transposed);

        let mut plain = base;
        secondary_inverse_transform(&mut plain, &p_plain).unwrap();
        let mut transposed = base;
        secondary_inverse_transform(&mut transposed, &p_transposed).unwrap();
        assert_ne!(plain, transposed);
    }

    #[test]
    fn input_scan_class_selects_gather_order() {
        let base: [i32; 16] = core::array::from_fn(|i| (i as i32 + 1) * 3);
        let two_d = params(4, 4, IST_4X4_HEIGHT, 2, 1, false);
        let mut horizontal = two_d;
        horizontal.primary_scan_class = TransformClass::Horizontal;

        let mut produced_two_d = base;
        secondary_inverse_transform(&mut produced_two_d, &two_d).unwrap();
        let mut produced_horizontal = base;
        secondary_inverse_transform(&mut produced_horizontal, &horizontal).unwrap();

        assert_ne!(produced_two_d, produced_horizontal);
        assert_matches_reference(base, &horizontal);
    }

    #[test]
    fn secondary_transform_clears_coefficients_outside_the_support_region() {
        let mut dequant = [0i32; 16];
        dequant[15] = 4096;
        secondary_inverse_transform(&mut dequant, &params(4, 4, IST_4X4_HEIGHT, 0, 1, false))
            .unwrap();
        assert_eq!(dequant[15], 0);
    }

    #[test]
    fn large_8x8_matches_independent_reference() {
        let base: [i32; 64] = core::array::from_fn(|i| (i as i32 % 9 - 4) * 13);
        assert_matches_reference(base, &params(8, 8, IST_8X8_HEIGHT, 7, 3, false));
    }

    #[test]
    fn large_8x8_reduced_height_uses_only_n_inputs() {
        let base: [i32; 64] = core::array::from_fn(|i| (i as i32 % 5 - 2) * 7);
        assert_matches_reference(base, &params(8, 8, 20, 2, 1, false));
    }

    #[test]
    fn rejects_invalid_shape_buffer_and_params() {
        let mut nine = [0i32; 9];
        assert!(matches!(
            secondary_inverse_transform(&mut nine, &params(3, 3, 8, 0, 1, false)),
            Err(ReconError::SecondaryTransformInvalidShape { w: 3, h: 3 })
        ));
        let mut short = [0i32; 15];
        assert!(matches!(
            secondary_inverse_transform(&mut short, &params(4, 4, 8, 0, 1, false)),
            Err(ReconError::SecondaryTransformBufferMismatch {
                expected: 16,
                actual: 15
            })
        ));
        let mut block = [0i32; 16];
        assert!(matches!(
            secondary_inverse_transform(&mut block, &params(4, 4, 9, 0, 1, false)),
            Err(ReconError::SecondaryTransformInvalidParams { .. })
        ));
        assert!(matches!(
            secondary_inverse_transform(&mut block, &params(4, 4, 8, 0, 0, false)),
            Err(ReconError::SecondaryTransformInvalidParams { .. })
        ));
        assert!(matches!(
            secondary_inverse_transform(&mut block, &params(4, 4, 8, 14, 1, false)),
            Err(ReconError::SecondaryTransformInvalidParams { .. })
        ));
        assert_eq!(block, [0i32; 16]);
    }

    #[test]
    fn is_total_for_extreme_coefficients() {
        for &(w, h, n, kernel) in &[
            (4usize, 4usize, IST_4X4_HEIGHT, 13usize),
            (32, 32, IST_8X8_HEIGHT, 0),
        ] {
            let mut max_block = vec![i32::MAX; w * h];
            secondary_inverse_transform(&mut max_block, &params(w, h, n, kernel, 3, true)).unwrap();
            let mut min_block = vec![i32::MIN; w * h];
            secondary_inverse_transform(&mut min_block, &params(w, h, n, kernel, 3, false))
                .unwrap();
        }
    }

    #[test]
    fn small_4x4_dc_only_matches_hand_computed_kernel_values() {
        let mut dequant = [0i32; 16];
        dequant[0] = 128; // scan position 0 is the DC for a 4x4 2D scan
        secondary_inverse_transform(&mut dequant, &params(4, 4, IST_4X4_HEIGHT, 0, 1, false))
            .unwrap();
        assert_eq!(dequant[0], 102); // (128*102 + 64) >> 7 = 102
        assert_eq!(dequant[1], -45); // -((128*45 + 64) >> 7) = -45
        assert_eq!(dequant[4], -53); // -((128*53 + 64) >> 7) = -53
    }
}
