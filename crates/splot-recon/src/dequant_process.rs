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
//! gating, the `shift` / `useFsc` derivation, and the adjusted-size handling
//! beyond the `Min(32, ·)` block are caller-resolved. User-defined `UserQm`
//! planes are carried by [`QmDequant`] when `useUserQm` is true.

use std::simd::num::{SimdInt as _, SimdUint as _};
use std::simd::{Simd, cmp::SimdOrd as _};
use std::sync::Arc;

use splot_tables::tables::quantizer::QUANTIZER_MATRIX;

use crate::intra_dc_math::round2_u32;
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
/// The computation is total and panic-free: only the product's low 24 bits are
/// normative, so wrapping `u32` multiplication computes them exactly;
/// `unsigned_abs` handles `i32::MIN`, and a zero `dq_denom` is treated as 1.
#[must_use]
pub fn dequant_coefficient(quant_coeff: i32, q2: u32, dq_denom: u32, bit_depth: BitDepth) -> i32 {
    let sign = if quant_coeff < 0 { -1 } else { 1 };
    let dq_high = quant_coeff.unsigned_abs().wrapping_mul(q2);
    let dq = round2_u32(dq_high & 0xFF_FFFF, QUANT_TABLE_BITS as u8);
    let dq2 = sign * (dq / dq_denom.max(1)) as i32;
    let bound = 1i32 << (7 + u32::from(bit_depth.bits()));
    dq2.clamp(-bound, bound - 1)
}

/// Caller-resolved parameters for the AV2 § 7.14.4 transform-block
/// dequantization (the non-quantization-matrix path).
///
/// `tx_width` / `tx_height` are the dequantized block dimensions
/// `Min(32, Tx_Width[txSz])` / `Min(32, Tx_Height[txSz])`, each 4, 8, 16, or 32.
/// `dc_quant` / `ac_quant` are the § 7.14.2 DC/AC quantizers and `dq_denom` is
/// `1 << shift`.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    /// § 7.14.4 built-in quantization-matrix weighting (`useQm`). When `Some`, each
    /// coefficient's quantizer `q` is replaced by `q2 = Round2(q * m, 5)`, where `m`
    /// is the `Quantizer_Matrix` weight at the coefficient's position. `None` is the
    /// flat path (`useQm == 0`).
    pub qm: Option<QmDequant>,
}

/// Frame-level § 7.14.4 built-in quantization-matrix levels, carried on the decode
/// workspace when `using_qmatrix == 1`. `levels_gt8[plane]` is `qm_y/u/v[0]` (the
/// `segLvl` used when `tw > 8 || th > 8`); `levels_le8[segment_id][plane]` is
/// `SegQMLevel[plane][segment_id]` (used otherwise). The per-block `segLvl` /
/// `useQm` / `Qm_Offset` are resolved from these by the transform-block dequant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QmFrameLevels {
    /// `qm_y[0]`, `qm_u[0]`, `qm_v[0]` (Y/U/V), used when `tw > 8 || th > 8`.
    pub levels_gt8: [u8; 3],
    /// `SegQMLevel[Y/U/V][segment_id]`, indexed as `[segment_id][plane]`.
    pub levels_le8: [[u8; 3]; 16],
}

/// § 7.14.4 built-in quantization-matrix selection for a transform block, resolved
/// by the caller from `segLvl`, `plane`, and `txSz`. Applied per coefficient by
/// [`dequantize_block`] via [`quantization_matrix_weight`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QmDequant {
    /// `segLvl` quantization-matrix level (`< NUM_CUSTOM_QMS`).
    pub seg_level: usize,
    /// `plane > 0` (chroma selects the second `Quantizer_Matrix` plane row).
    pub plane_is_chroma: bool,
    /// Caller-resolved `Qm_Offset[txSz]` (from `splot_tables` `QM_OFFSET`).
    pub qm_offset: usize,
    /// § 7.14.4 `UserQm[segLvl][t][plane]`, selected by the caller for transform
    /// shape `t`. `None` selects the generated default quantizer matrix.
    pub user: Option<QmUserPlane>,
}

/// One immutable user-defined quantizer-matrix plane used by § 7.14.4.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QmUserPlane {
    /// Fundamental matrix width (8 for every AV2 fundamental QM transform).
    pub width: usize,
    /// Fundamental matrix height (8 for square, 4 for wide, 8 for tall).
    pub height: usize,
    /// Row-major non-zero weights.
    pub values: Arc<[u8]>,
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
    if params.qm.is_none() && dequantize_flat_block(params, quant, out) {
        return Ok(());
    }
    for i in 0..tx_height {
        for j in 0..tx_width {
            let idx = i * tx_width + j;
            let base_q = if i == 0 && j == 0 {
                params.dc_quant
            } else {
                params.ac_quant
            };
            let q2 = match params.qm.as_ref() {
                Some(qm) => {
                    let m = if let Some(user) = &qm.user {
                        user_quantization_matrix_weight(user, i, j, tx_width, tx_height)?
                    } else {
                        quantization_matrix_weight(&QmWeightIndex {
                            seg_level: qm.seg_level,
                            plane_is_chroma: qm.plane_is_chroma,
                            qm_offset: qm.qm_offset,
                            row: i,
                            col: j,
                            tx_width,
                            tx_height,
                        })?
                    };
                    qm_weighted_quantizer(base_q, m)
                }
                None => base_q,
            };
            out[idx] = dequant_coefficient(quant[idx], q2, params.dq_denom, params.bit_depth);
        }
    }
    Ok(())
}

/// Dequantizes a whole `useQm == 0` block with the § 7.14.4 quantizer carried in
/// registers, widest lane group first.
///
/// Every coefficient but `(0, 0)` takes `ac_quant`, so the lane groups run the
/// AC quantizer over the whole block and the DC coefficient is rewritten
/// afterwards — the same values the per-coefficient loop writes, in the same
/// slots. Returns `false` when `dq_denom` is not a power of two, which leaves
/// the § 7.14.4 divide to the caller's per-coefficient loop.
fn dequantize_flat_block(params: &DequantBlockParams, quant: &[i32], out: &mut [i32]) -> bool {
    let denom = params.dq_denom.max(1);
    if !denom.is_power_of_two() {
        return false;
    }
    let denom_shift = denom.trailing_zeros();
    let bound = 1i32 << (7 + u32::from(params.bit_depth.bits()));
    let len = quant.len();
    let mut index = 0usize;
    macro_rules! dequant_lane_group {
        ($lanes:literal) => {
            while index + $lanes <= len {
                dequant_flat_lanes::<$lanes>(
                    &mut out[index..],
                    &quant[index..],
                    params.ac_quant,
                    denom_shift,
                    bound,
                );
                index += $lanes;
            }
        };
    }
    dequant_lane_group!(16);
    dequant_lane_group!(8);
    dequant_lane_group!(4);
    for (slot, &coeff) in out[index..].iter_mut().zip(&quant[index..]) {
        *slot = dequant_coefficient(coeff, params.ac_quant, denom, params.bit_depth);
    }
    if let (Some(slot), Some(&coeff)) = (out.first_mut(), quant.first()) {
        *slot = dequant_coefficient(coeff, params.dc_quant, denom, params.bit_depth);
    }
    true
}

/// One `LANES`-wide group of [`dequantize_flat_block`].
///
/// Each lane repeats [`dequant_coefficient`] literally: `sign`/`unsigned_abs`
/// as the two's-complement `(c ^ s) - s` pair (exact for `i32::MIN`), the
/// wrapping product masked to its normative low 24 bits,
/// `Round2(·, QUANT_TABLE_BITS)` expanded as `(v >> n) + ((v >> (n - 1)) & 1)`,
/// the power-of-two `dq_denom` divide as a shift, and the § 7.14.4 clamp. The
/// masked `Round2` result never exceeds `1 << 21`, so restoring the sign and
/// narrowing to `i32` are both exact.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.14.4 dequantization hot path"
)]
#[inline(always)]
fn dequant_flat_lanes<const LANES: usize>(
    out: &mut [i32],
    quant: &[i32],
    q2: u32,
    denom_shift: u32,
    bound: i32,
) {
    let coeff = Simd::<i32, LANES>::from_slice(quant);
    let sign = coeff >> Simd::splat(i32::BITS as i32 - 1);
    let magnitude = (coeff ^ sign) - sign;
    let product = (magnitude.cast::<u32>() * Simd::splat(q2)) & Simd::splat(0x00FF_FFFF);
    let shift = QUANT_TABLE_BITS;
    let rounded =
        (product >> Simd::splat(shift)) + ((product >> Simd::splat(shift - 1)) & Simd::splat(1));
    let scaled = (rounded >> Simd::splat(denom_shift)).cast::<i32>();
    let signed = (scaled ^ sign) - sign;
    let clamped = signed.simd_clamp(Simd::splat(-bound), Simd::splat(bound - 1));
    out[..LANES].copy_from_slice(&clamped.to_array()); // splot-copy-ok: publish a § 7.14.4 dequant lane group
}

fn user_quantization_matrix_weight(
    user: &QmUserPlane,
    row: usize,
    col: usize,
    tx_width: usize,
    tx_height: usize,
) -> Result<i32> {
    let (user_row, user_col) = if tx_width == tx_height {
        (
            row.saturating_mul(8) / tx_height,
            col.saturating_mul(8) / tx_width,
        )
    } else {
        (row, col)
    };
    let position = user_row
        .checked_mul(user.width)
        .and_then(|base| base.checked_add(user_col))
        .ok_or(ReconError::InvalidQuantizerMatrixIndex {
            seg_level: 0,
            qm_offset: 0,
            position: usize::MAX,
        })?;
    if user_row >= user.height || user_col >= user.width {
        return Err(ReconError::InvalidQuantizerMatrixIndex {
            seg_level: 0,
            qm_offset: 0,
            position,
        });
    }
    user.values.get(position).copied().map(i32::from).ok_or(
        ReconError::InvalidQuantizerMatrixIndex {
            seg_level: 0,
            qm_offset: 0,
            position,
        },
    )
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
        assert_eq!(dequant_coefficient(4, 16, 1, BitDepth::Eight), 8);
        assert_eq!(dequant_coefficient(-4, 16, 1, BitDepth::Eight), -8);
        assert_eq!(dequant_coefficient(0, 16, 1, BitDepth::Eight), 0);
    }

    #[test]
    fn dequant_coefficient_divides_by_dq_denom() {
        assert_eq!(dequant_coefficient(8, 16, 4, BitDepth::Eight), 4);
    }

    #[test]
    fn dequant_coefficient_masks_to_24_bits() {
        assert_eq!(dequant_coefficient(4096, 4096, 1, BitDepth::Eight), 0);
        assert_eq!(dequant_coefficient(0x100_0008, 1, 1, BitDepth::Eight), 1);
    }

    #[test]
    fn dequant_coefficient_clips_to_bit_depth_bound() {
        let max = dequant_coefficient(0xFF_FFFF, 1, 1, BitDepth::Eight);
        assert_eq!(max, 32767);
        let min = dequant_coefficient(-0xFF_FFFF, 1, 1, BitDepth::Eight);
        assert_eq!(min, -32768);
        let wide = dequant_coefficient(0xFF_FFFF, 1, 1, BitDepth::Ten);
        assert_eq!(wide, 131_071);
    }

    #[test]
    fn dequant_coefficient_is_total_for_extreme_inputs() {
        let _ = dequant_coefficient(i32::MIN, u32::MAX, 0, BitDepth::Eight);
        let _ = dequant_coefficient(i32::MAX, u32::MAX, 8, BitDepth::Ten);
    }

    #[test]
    fn dequantize_block_uses_dc_quant_for_origin_and_ac_elsewhere() {
        let quant = [1i32; 16];
        let mut out = [0i32; 16];
        let params = DequantBlockParams {
            dc_quant: 16,
            ac_quant: 32,
            tx_width: 4,
            tx_height: 4,
            dq_denom: 1,
            bit_depth: BitDepth::Eight,
            qm: None,
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
            qm: None,
        }
    }

    /// A 4x4 luma block (`TX_4X4`, `Qm_Offset == 0`) at qm level 0: each output must
    /// equal the § 7.14.4 `Round2(q * m, 5)` weighted quantizer for that position's
    /// weight, not the flat dc/ac quantizer.
    #[test]
    fn dequantize_block_applies_qm_weight_per_coefficient() {
        let quant = [1i32; 16];
        let mut out = [0i32; 16];
        let params = DequantBlockParams {
            dc_quant: 40,
            ac_quant: 40,
            tx_width: 4,
            tx_height: 4,
            dq_denom: 1,
            bit_depth: BitDepth::Eight,
            qm: Some(QmDequant {
                seg_level: 0,
                plane_is_chroma: false,
                qm_offset: QM_OFFSET[0] as usize,
                user: None,
            }),
        };
        dequantize_block(&params, &quant, &mut out).unwrap();
        for i in 0..4 {
            for j in 0..4 {
                let m = quantization_matrix_weight(&QmWeightIndex {
                    seg_level: 0,
                    plane_is_chroma: false,
                    qm_offset: QM_OFFSET[0] as usize,
                    row: i,
                    col: j,
                    tx_width: 4,
                    tx_height: 4,
                })
                .unwrap();
                let q2 = qm_weighted_quantizer(40, m);
                let expected = dequant_coefficient(1, q2, 1, BitDepth::Eight);
                assert_eq!(out[i * 4 + j], expected, "qm weight at ({i},{j})");
            }
        }
        assert_eq!(
            out[0],
            dequant_coefficient(1, 40, 1, BitDepth::Eight),
            "top-left weight 32 is identity"
        );
        assert_ne!(
            out[15], out[0],
            "a non-identity qm weight changes the dequant"
        );
    }

    /// The `useQm == 0` lane groups must reproduce [`dequant_coefficient`]
    /// exactly for every block shape, bit depth, and `dq_denom` shift, over
    /// randomized coefficients including the `i32` extremes.
    #[test]
    fn flat_lane_groups_match_the_per_coefficient_reference() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
            for &tx_width in &[4usize, 8, 16, 32] {
                for &tx_height in &[4usize, 8, 16, 32] {
                    for dq_shift in 0..4u32 {
                        let params = DequantBlockParams {
                            dc_quant: (next() % 8192) as u32 + 1,
                            ac_quant: (next() % 8192) as u32 + 1,
                            tx_width,
                            tx_height,
                            dq_denom: 1 << dq_shift,
                            bit_depth,
                            qm: None,
                        };
                        let samples = tx_width * tx_height;
                        let mut quant = vec![0i32; samples];
                        for (index, slot) in quant.iter_mut().enumerate() {
                            *slot = match index % 16 {
                                0 => i32::MIN,
                                1 => i32::MAX,
                                2 => 0,
                                _ => (next() as i32) >> (next() % 24) as i32,
                            };
                        }
                        let mut expected = vec![0i32; samples];
                        for (index, slot) in expected.iter_mut().enumerate() {
                            let base_q = if index == 0 {
                                params.dc_quant
                            } else {
                                params.ac_quant
                            };
                            *slot = dequant_coefficient(
                                quant[index],
                                base_q,
                                params.dq_denom,
                                bit_depth,
                            );
                        }
                        let mut actual = vec![0i32; samples];
                        assert!(dequantize_flat_block(&params, &quant, &mut actual));
                        assert_eq!(
                            actual, expected,
                            "{tx_width}x{tx_height} {bit_depth:?} shift {dq_shift}"
                        );
                    }
                }
            }
        }
    }

    /// A non-power-of-two `dq_denom` has no shift identity, so the fast path
    /// must decline and leave the § 7.14.4 divide to the per-coefficient loop.
    #[test]
    fn flat_block_declines_a_non_power_of_two_denominator() {
        let params = DequantBlockParams {
            dq_denom: 3,
            ..params(4, 4)
        };
        let mut out = [0i32; 16];
        assert!(!dequantize_flat_block(&params, &[7i32; 16], &mut out));
        dequantize_block(&params, &[7i32; 16], &mut out).unwrap();
        assert_eq!(
            out[1],
            dequant_coefficient(7, params.ac_quant, 3, params.bit_depth)
        );
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
        assert_eq!(qm_weighted_quantizer(8, 64), 16);
        assert_eq!(qm_weighted_quantizer(40, 32), 40);
        assert_eq!(qm_weighted_quantizer(100, 0), 0);
    }

    #[test]
    fn qm_weighted_quantizer_is_total_for_extreme_inputs() {
        let _ = qm_weighted_quantizer(u32::MAX, i32::MAX);
    }

    #[test]
    fn quantization_matrix_weight_matches_the_generated_table() {
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
        let chroma = QmWeightIndex {
            plane_is_chroma: true,
            ..index
        };
        assert_eq!(
            quantization_matrix_weight(&chroma).unwrap(),
            QUANTIZER_MATRIX[0][1][0]
        );
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
