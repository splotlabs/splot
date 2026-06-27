// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.14.4 dequantization process.
//!
//! This module dequantizes coded transform coefficients (`Quant`) into the
//! `Dequant` array consumed by the § 7.15.4 inverse transform
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-14-4`). It provides the per-coefficient core (steps 3-8) and a
//! transform-block helper that selects the DC quantizer for the `(0, 0)`
//! coefficient and the AC quantizer otherwise.
//!
//! Feature tracking: `RECON-DEQUANT-PROCESS`.
//!
//! Scope: the dequantization arithmetic over a caller-resolved per-coefficient
//! quantizer `q2` and the dequant denominator `dq_denom = 1 << shift`, plus the
//! § 7.14.4 step-2 quantization-matrix weighting for the built-in
//! `Quantizer_Matrix` (the `quantization_matrix_weight` lookup and the
//! `qm_weighted_quantizer` `Round2(q * m, 5)`). The caller resolves `q2` (the
//! § 7.14.2 [`dc_quantizer`](crate::dc_quantizer) /
//! [`ac_quantizer`](crate::ac_quantizer) value, optionally passed through
//! `qm_weighted_quantizer`) and `dq_denom` (the § 7.14.4 `shift` derivation,
//! including the `allow_tcq` adjustment). The `useQm` / `useUserQm` / `segLvl`
//! gating, the user-defined `UserQm` matrices, the `shift` / `useFsc`
//! derivation, and the adjusted-size handling beyond the `Min(32, ·)` block are
//! out of scope (caller-resolved or future rows); the block helper covers the
//! path where every AC coefficient shares one quantizer.

use splot_tables::tables::quantizer::QUANTIZER_MATRIX;

use crate::math::round2;
use crate::{BitDepth, ReconError, Result};

/// Maximum dequantized transform-block side (§ 7.14.4 `Min(32, Tx_Width/Height)`).
const MAX_DEQUANT_DIM: usize = 32;

/// AV2 § 3 `QUANT_TABLE_BITS`: the number of low bits discarded from the
/// quantizer product before the denominator divide.
const QUANT_TABLE_BITS: u32 = 3;

/// AV2 § 7.14.4 quantization-matrix weight shift: `q2 = Round2(q * m, 5)`.
const QM_WEIGHT_SHIFT: u32 = 5;

/// AV2 § 7.14.4 per-coefficient dequantization (steps 3-8).
///
/// `quant_coeff` is the coded coefficient `Quant[i][j]`, `q2` is the resolved
/// per-coefficient quantizer (the § 7.14.2 DC/AC quantizer, optionally
/// quantization-matrix-weighted `Round2(q * m, 5)`), and `dq_denom` is
/// `1 << shift`. The result is
/// `Clip3(-(1 << (7 + BitDepth)), (1 << (7 + BitDepth)) - 1, sign * (Round2(Abs(qc) * q2 & 0xFFFFFF, QUANT_TABLE_BITS) / dq_denom))`.
///
/// The computation is total and panic-free: the product and rounding use `i64`,
/// `unsigned_abs` handles `i32::MIN`, and a zero `dq_denom` is treated as 1.
#[must_use]
pub fn dequant_coefficient(quant_coeff: i32, q2: u32, dq_denom: u32, bit_depth: BitDepth) -> i32 {
    let sign: i64 = if quant_coeff < 0 { -1 } else { 1 };
    let dq_high = i64::from(quant_coeff.unsigned_abs()) * i64::from(q2);
    let dq = round2(dq_high & 0xFF_FFFF, QUANT_TABLE_BITS);
    let denom = i64::from(dq_denom.max(1));
    let dq2 = sign * (dq / denom);
    let bound = 1i64 << (7 + u32::from(bit_depth.bits()));
    dq2.clamp(-bound, bound - 1) as i32
}

/// Caller-resolved parameters for the AV2 § 7.14.4 transform-block
/// dequantization (the non-quantization-matrix path).
///
/// `tx_width` / `tx_height` are the dequantized block dimensions
/// `Min(32, Tx_Width[txSz])` / `Min(32, Tx_Height[txSz])`, each 4, 8, 16, or 32.
/// `dc_quant` / `ac_quant` are the § 7.14.2 DC/AC quantizers and `dq_denom` is
/// `1 << shift`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DequantBlockParams {
    /// DC quantizer for the `(0, 0)` coefficient (`get_dc_quant`).
    pub dc_quant: u32,
    /// AC quantizer for every non-DC coefficient (`get_ac_quant`).
    pub ac_quant: u32,
    /// Dequantized block width `Min(32, Tx_Width[txSz])` (4, 8, 16, or 32).
    pub tx_width: usize,
    /// Dequantized block height `Min(32, Tx_Height[txSz])` (4, 8, 16, or 32).
    pub tx_height: usize,
    /// Dequant denominator `1 << shift`.
    pub dq_denom: u32,
    /// Active decoded bit depth.
    pub bit_depth: BitDepth,
}

/// AV2 § 7.14.4 dequantization over a `tx_width * tx_height` row-major transform
/// block, selecting `dc_quant` for the DC coefficient `(0, 0)` and `ac_quant`
/// for every other coefficient (the non-quantization-matrix path), writing the
/// `Dequant` block into `out`.
///
/// # Errors
/// Returns [`ReconError::InvalidDequantBlockShape`] if `tx_width` / `tx_height`
/// are not each 4/8/16/32, and [`ReconError::DequantBlockLengthMismatch`] if
/// `quant` or `out` is not exactly `tx_width * tx_height` long.
pub fn dequantize_block(params: &DequantBlockParams, quant: &[i32], out: &mut [i32]) -> Result<()> {
    let (tx_width, tx_height) = (params.tx_width, params.tx_height);
    if !matches!(tx_width, 4 | 8 | 16 | 32) || !matches!(tx_height, 4 | 8 | 16 | 32) {
        return Err(ReconError::InvalidDequantBlockShape {
            tx_width,
            tx_height,
        });
    }
    debug_assert!(tx_width <= MAX_DEQUANT_DIM && tx_height <= MAX_DEQUANT_DIM);
    let expected = tx_width * tx_height;
    if quant.len() != expected || out.len() != expected {
        return Err(ReconError::DequantBlockLengthMismatch {
            expected,
            quant_len: quant.len(),
            out_len: out.len(),
        });
    }
    for i in 0..tx_height {
        for j in 0..tx_width {
            let idx = i * tx_width + j;
            let q2 = if i == 0 && j == 0 {
                params.dc_quant
            } else {
                params.ac_quant
            };
            out[idx] = dequant_coefficient(quant[idx], q2, params.dq_denom, params.bit_depth);
        }
    }
    Ok(())
}

/// Caller-resolved indices for the § 7.14.4 built-in quantization-matrix weight
/// lookup `Quantizer_Matrix[seg_level][plane > 0][qm_offset + row * tx_width + col]`.
///
/// Every field is resolved by the caller from a single `txSz`, matching the
/// crate-wide "caller resolves the spec dimensions" contract used by the § 7.15
/// inverse-transform primitives (which take `Tx_Width_Log2` / `Tx_Height_Log2`
/// rather than `txSz`, since `splot-recon` does not depend on the § 9.2
/// conversion tables in `splot-core`). `qm_offset`, `tx_width`, and `tx_height`
/// are therefore derived together — `Qm_Offset[txSz]`,
/// `Min(32, Tx_Width[txSz])`, and `Min(32, Tx_Height[txSz])` — so the region
/// offset and the row stride cannot disagree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QmWeightIndex {
    /// `segLvl` quantization-matrix segment level (`0..NUM_CUSTOM_QMS`).
    pub seg_level: usize,
    /// `plane > 0` (chroma selects the second `Quantizer_Matrix` plane row).
    pub plane_is_chroma: bool,
    /// Caller-resolved `Qm_Offset[txSz]`: the start of `txSz`'s region in the
    /// flattened `Quantizer_Matrix` row (from `splot_tables` `QM_OFFSET`).
    pub qm_offset: usize,
    /// Coefficient row `i` (must be `< tx_height`).
    pub row: usize,
    /// Coefficient column `j` (must be `< tx_width`).
    pub col: usize,
    /// Dequantized block width `tw = Min(32, Tx_Width[txSz])`, the row stride.
    pub tx_width: usize,
    /// Dequantized block height `th = Min(32, Tx_Height[txSz])`.
    pub tx_height: usize,
}

/// AV2 § 7.14.4 built-in quantization-matrix weight `m`
/// (`Quantizer_Matrix[segLvl][plane > 0][Qm_Offset[txSz] + i * tw + j]`), the
/// non-`UserQm` path.
///
/// The coefficient `(row, col)` must lie inside the selected transform's
/// `tx_width * tx_height` sub-block, so a coordinate outside the transform is
/// rejected rather than silently reading a neighbouring transform's weight.
/// Because `qm_offset` (the region start) and `tx_width` (the row stride) are
/// resolved from the same `txSz` by the caller, the flattened index stays inside
/// the intended region; the final matrix-row bound is a defensive backstop.
///
/// # Errors
/// Returns [`ReconError::InvalidQuantizerMatrixIndex`] if `row` / `col` are
/// outside the `tx_width * tx_height` sub-block, or if `seg_level` or the derived
/// position is out of range for the generated `Quantizer_Matrix`.
pub fn quantization_matrix_weight(index: &QmWeightIndex) -> Result<i32> {
    // The coefficient must lie inside the selected transform's sub-block; a
    // larger row/col would otherwise index a later transform's region of the
    // flattened matrix row.
    if index.row >= index.tx_height || index.col >= index.tx_width {
        return Err(ReconError::InvalidQuantizerMatrixIndex {
            seg_level: index.seg_level,
            qm_offset: index.qm_offset,
            position: index
                .qm_offset
                .saturating_add(index.row.saturating_mul(index.tx_width))
                .saturating_add(index.col),
        });
    }
    let level =
        QUANTIZER_MATRIX
            .get(index.seg_level)
            .ok_or(ReconError::InvalidQuantizerMatrixIndex {
                seg_level: index.seg_level,
                qm_offset: index.qm_offset,
                position: 0,
            })?;
    let plane = &level[usize::from(index.plane_is_chroma)];
    // `qm_offset` is the caller-resolved region start; widen the in-region offset
    // and bounds-check the flattened position against the matrix row.
    let position = index
        .qm_offset
        .saturating_add(index.row.saturating_mul(index.tx_width))
        .saturating_add(index.col);
    plane
        .get(position)
        .copied()
        .ok_or(ReconError::InvalidQuantizerMatrixIndex {
            seg_level: index.seg_level,
            qm_offset: index.qm_offset,
            position,
        })
}

/// AV2 § 7.14.4 step 2 quantization-matrix-weighted quantizer
/// `q2 = Round2(q * m, 5)`, computed in `i64` and clamped into `u32` so it is
/// total (the built-in `Quantizer_Matrix` weights are non-negative).
#[must_use]
pub fn qm_weighted_quantizer(q: u32, m: i32) -> u32 {
    let product = i64::from(q) * i64::from(m);
    let rounded = (product + (1 << (QM_WEIGHT_SHIFT - 1))) >> QM_WEIGHT_SHIFT;
    rounded.clamp(0, i64::from(u32::MAX)) as u32
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use splot_tables::tables::quantizer::QM_OFFSET;

    use super::*;

    #[test]
    fn dequant_coefficient_applies_round2_mask_and_denom() {
        // dq_denom = 1 (shift 0): Dequant = sign * Round2(Abs(qc)*q2 & 0xFFFFFF, 3).
        // qc = 4, q2 = 16 -> dqHigh = 64 -> Round2(64,3) = (64+4)>>3 = 8.
        assert_eq!(dequant_coefficient(4, 16, 1, BitDepth::Eight), 8);
        // Negative coefficient keeps the sign.
        assert_eq!(dequant_coefficient(-4, 16, 1, BitDepth::Eight), -8);
        // Zero coefficient -> 0.
        assert_eq!(dequant_coefficient(0, 16, 1, BitDepth::Eight), 0);
    }

    #[test]
    fn dequant_coefficient_divides_by_dq_denom() {
        // dq_denom = 4 (shift 2): the rounded product is divided by 4.
        // qc = 8, q2 = 16 -> dqHigh = 128 -> Round2(128,3) = (128+4)>>3 = 16;
        // 16 / 4 = 4.
        assert_eq!(dequant_coefficient(8, 16, 4, BitDepth::Eight), 4);
    }

    #[test]
    fn dequant_coefficient_masks_to_24_bits() {
        // The product is masked with 0xFFFFFF before rounding. A product whose
        // low 24 bits are 0 rounds to 0 regardless of the high bits.
        // qc such that Abs(qc)*q2 == 0x1000000 (16777216): low 24 bits are 0.
        // 0x1000000 = 4096 * 4096.
        assert_eq!(dequant_coefficient(4096, 4096, 1, BitDepth::Eight), 0);
        // One more (0x1000008) keeps only the low 24 bits (8) -> Round2(8,3)=1.
        assert_eq!(dequant_coefficient(0x100_0008, 1, 1, BitDepth::Eight), 1);
    }

    #[test]
    fn dequant_coefficient_clips_to_bit_depth_bound() {
        // The result is clamped to [-(1<<(7+BitDepth)), (1<<(7+BitDepth))-1].
        // 8-bit bound is 1<<15 = 32768. A huge product clamps to 32767 / -32768.
        let max = dequant_coefficient(0xFF_FFFF, 1, 1, BitDepth::Eight);
        assert_eq!(max, 32767);
        let min = dequant_coefficient(-0xFF_FFFF, 1, 1, BitDepth::Eight);
        assert_eq!(min, -32768);
        // 10-bit widens the bound to 1<<17 = 131072, so the same large product
        // clamps to 131071 instead of 32767.
        let wide = dequant_coefficient(0xFF_FFFF, 1, 1, BitDepth::Ten);
        assert_eq!(wide, 131_071);
    }

    #[test]
    fn dequant_coefficient_is_total_for_extreme_inputs() {
        // i32::MIN coefficient and the maximum quantizer must not panic/overflow.
        let _ = dequant_coefficient(i32::MIN, u32::MAX, 0, BitDepth::Eight);
        let _ = dequant_coefficient(i32::MAX, u32::MAX, 8, BitDepth::Ten);
    }

    #[test]
    fn dequantize_block_uses_dc_quant_for_origin_and_ac_elsewhere() {
        // 4x4 block: DC (0,0) uses dc_quant, all others ac_quant. Use distinct
        // quantizers and a unit coefficient everywhere to observe the selection.
        let quant = [1i32; 16];
        let mut out = [0i32; 16];
        // dc_quant = 16 -> Round2(16,3)=2; ac_quant = 32 -> Round2(32,3)=4.
        let params = DequantBlockParams {
            dc_quant: 16,
            ac_quant: 32,
            tx_width: 4,
            tx_height: 4,
            dq_denom: 1,
            bit_depth: BitDepth::Eight,
        };
        dequantize_block(&params, &quant, &mut out).unwrap();
        assert_eq!(out[0], 2, "DC coefficient uses dc_quant");
        for (idx, &v) in out.iter().enumerate().skip(1) {
            assert_eq!(v, 4, "AC coefficient {idx} uses ac_quant");
        }
    }

    fn params(tx_width: usize, tx_height: usize) -> DequantBlockParams {
        DequantBlockParams {
            dc_quant: 1,
            ac_quant: 1,
            tx_width,
            tx_height,
            dq_denom: 1,
            bit_depth: BitDepth::Eight,
        }
    }

    #[test]
    fn dequantize_block_rejects_unsupported_shape() {
        let mut out = [0i32; 20];
        assert!(matches!(
            dequantize_block(&params(5, 4), &[0i32; 20], &mut out),
            Err(ReconError::InvalidDequantBlockShape {
                tx_width: 5,
                tx_height: 4
            })
        ));
    }

    #[test]
    fn dequantize_block_rejects_length_mismatch() {
        let mut out = [0i32; 16];
        assert!(matches!(
            dequantize_block(&params(4, 4), &[0i32; 15], &mut out),
            Err(ReconError::DequantBlockLengthMismatch {
                expected: 16,
                quant_len: 15,
                out_len: 16
            })
        ));
    }

    #[test]
    fn qm_weighted_quantizer_applies_round2_shift5() {
        // q2 = Round2(q*m, 5). q=8, m=64 -> (512+16)>>5 = 16.
        assert_eq!(qm_weighted_quantizer(8, 64), 16);
        // A neutral weight of 32 (= 1<<5) round-trips q within rounding:
        // Round2(q*32, 5) = q.
        assert_eq!(qm_weighted_quantizer(40, 32), 40);
        // Zero weight -> 0.
        assert_eq!(qm_weighted_quantizer(100, 0), 0);
    }

    #[test]
    fn qm_weighted_quantizer_is_total_for_extreme_inputs() {
        // u32::MAX * a large weight must not panic/overflow (i64 + clamp).
        let _ = qm_weighted_quantizer(u32::MAX, i32::MAX);
    }

    #[test]
    fn quantization_matrix_weight_matches_the_generated_table() {
        // The (0,0) coefficient of seg_level 0, luma, tx_size 0 is
        // Quantizer_Matrix[0][0][Qm_Offset[0] + 0] = Quantizer_Matrix[0][0][0].
        let index = QmWeightIndex {
            seg_level: 0,
            plane_is_chroma: false,
            qm_offset: QM_OFFSET[0] as usize,
            row: 0,
            col: 0,
            tx_width: 4,
            tx_height: 4,
        };
        let m = quantization_matrix_weight(&index).unwrap();
        assert_eq!(m, QUANTIZER_MATRIX[0][0][0]);
        // Chroma selects the second plane row.
        let chroma = QmWeightIndex {
            plane_is_chroma: true,
            ..index
        };
        assert_eq!(
            quantization_matrix_weight(&chroma).unwrap(),
            QUANTIZER_MATRIX[0][1][0]
        );
        // A non-origin coefficient uses Qm_Offset[txSz] + row*tw + col, with the
        // offset and the stride both resolved from tx_size 1 (an 8x8 transform).
        let off = QM_OFFSET[1] as usize;
        let pos = QmWeightIndex {
            qm_offset: off,
            row: 1,
            col: 2,
            tx_width: 8,
            tx_height: 8,
            ..index
        };
        assert_eq!(
            quantization_matrix_weight(&pos).unwrap(),
            QUANTIZER_MATRIX[0][0][off + pos.row * pos.tx_width + pos.col]
        );
    }

    #[test]
    fn quantization_matrix_weight_rejects_out_of_range_indices() {
        // seg_level beyond the matrix (row/col inside the sub-block).
        let bad_level = QmWeightIndex {
            seg_level: 99,
            plane_is_chroma: false,
            qm_offset: QM_OFFSET[0] as usize,
            row: 0,
            col: 0,
            tx_width: 4,
            tx_height: 4,
        };
        assert!(matches!(
            quantization_matrix_weight(&bad_level),
            Err(ReconError::InvalidQuantizerMatrixIndex { seg_level: 99, .. })
        ));
    }

    #[test]
    fn quantization_matrix_weight_rejects_coords_outside_the_transform_sub_block() {
        // A coordinate outside the selected transform's sub-block must be
        // rejected, not silently read from a neighbouring transform's region.
        // tx_size 0 is 4x4: row == 4 is outside, even though Qm_Offset[0] + 4*4
        // = 16 still lands inside the 3600-entry matrix row (the start of the
        // 8x8 region).
        let outside_row = QmWeightIndex {
            seg_level: 0,
            plane_is_chroma: false,
            qm_offset: QM_OFFSET[0] as usize,
            row: 4,
            col: 0,
            tx_width: 4,
            tx_height: 4,
        };
        assert!(matches!(
            quantization_matrix_weight(&outside_row),
            Err(ReconError::InvalidQuantizerMatrixIndex { qm_offset: 0, .. })
        ));
        // A column outside the width is rejected too.
        let outside_col = QmWeightIndex {
            col: 4,
            row: 0,
            ..outside_row
        };
        assert!(matches!(
            quantization_matrix_weight(&outside_col),
            Err(ReconError::InvalidQuantizerMatrixIndex { qm_offset: 0, .. })
        ));
    }
}
