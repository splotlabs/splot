// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.4 2D inverse transform process (outer orchestration).
//!
//! This module wraps the § 7.15.4.1 2D matrix transform core
//! ([`inverse_transform_2d`](crate::inverse_transform_2d)) with the surrounding
//! § 7.15.4 process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-15-4`): the adjusted-size derivation, the `Lossless && IDTX` bit-shift
//! shortcut, the DPCM cumulative sum, and the adjusted-size sample duplication,
//! over a caller-supplied dequantized coefficient block.
//!
//! Feature tracking: `RECON-INVERSE-TRANSFORM-2D-OUTER`.
//!
//! Scope: this is the § 7.15.4 outer process parameterized by the *original*
//! (unadjusted) `txSz` log2 dimensions, the caller-resolved per-dimension
//! transform selection and shifts (the same inputs the § 7.15.4.1 core takes),
//! plus the `Lossless` / `PlaneTxType == IDTX` flags and the DPCM direction. The
//! adjusted operating size is derived as `1 << Min(log2, 5)` (the
//! `Adjusted_Tx_Size` cap) and the original size as `1 << log2` (the
//! `Tx_Width` / `Tx_Height` of `txSz`), so no conversion tables are needed.
//!
//! Out of scope (caller's responsibility or a future row): the
//! `get_transform_1d_type` derivation of `rowType` / `colType` (which needs
//! `PlaneTxType`, the `Transform_1d_Type` table, and the `enable_inter_ddt` /
//! `use_intrabc` / `is_inter` flags), the `Transform_Shift` lookup, the § 7.15.3
//! secondary transform, the § 7.14.4 dequantization process, residual addition,
//! and runtime decode wiring.

use crate::inverse_transform_2d::{
    InverseTransform2d, InverseTransform2dDim, inverse_transform_2d,
};
use crate::{BitDepth, ReconError, Result};

/// Maximum adjusted 1D transform length (§ 7.15.4 caps each adjusted side at 32).
const MAX_ADJ_DIM: usize = 32;

/// Minimum original transform-dimension base-2 logarithm (a 4-sample side).
const MIN_LOG2_DIM: u32 = 2;

/// Maximum original transform-dimension base-2 logarithm (a 64-sample side).
const MAX_LOG2_DIM: u32 = 6;

/// AV2 § 7.15.4 DPCM cumulative-sum direction. Selected by the prediction mode:
/// `V_PRED` sums down each column, every other DPCM mode sums across each row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DpcmDirection {
    /// `V_PRED`: cumulative sum down each column (`Residual[i][j] += Residual[i-1][j]`).
    Vertical,
    /// Any other DPCM mode: cumulative sum across each row (`Residual[i][j] += Residual[i][j-1]`).
    Horizontal,
}

/// Caller-resolved parameters for the AV2 § 7.15.4 outer 2D inverse transform.
///
/// `log2_width` / `log2_height` are the *original* (unadjusted) `txSz` log2
/// dimensions (`Tx_Width_Log2[txSz]` / `Tx_Height_Log2[txSz]`, each `2..=6`).
/// The adjusted operating size is `1 << Min(log2, 5)` and the original residual
/// size is `1 << log2`. `row_type` / `col_type` / `row_shift` / `col_shift` are
/// the caller-resolved § 7.15.4.1 selections (the caller derives `rowType` /
/// `colType` via `get_transform_1d_type`, out of scope here). When `lossless`
/// the block must be 4x4 (`log2_width == log2_height == 2`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InverseTransform2dOuter {
    /// Original (unadjusted) transform width base-2 logarithm (`log2W`), 2..=6.
    pub log2_width: u32,
    /// Original (unadjusted) transform height base-2 logarithm (`log2H`), 2..=6.
    pub log2_height: u32,
    /// Whether the block is lossless (forces the Walsh-Hadamard transform path).
    pub lossless: bool,
    /// Whether `PlaneTxType` is `IDTX` (selects the lossless bit-shift shortcut).
    pub plane_tx_type_is_idtx: bool,
    /// Row (first pass) transform selection when not lossless.
    pub row_type: InverseTransform2dDim,
    /// Column (second pass) transform selection when not lossless.
    pub col_type: InverseTransform2dDim,
    /// Row-pass down-shift (`rowShift`).
    pub row_shift: u8,
    /// Column-pass down-shift (`colShift`).
    pub col_shift: u8,
    /// Active decoded bit depth.
    pub bit_depth: BitDepth,
    /// DPCM cumulative-sum direction, or `None` when `useDpcm` is 0.
    pub dpcm: Option<DpcmDirection>,
}

/// Applies the AV2 § 7.15.4 outer 2D inverse transform.
///
/// `dequant` is the adjusted-size `adjW * adjH` row-major dequantized block (the
/// coefficients beyond the adjusted size are not coded), and `residual` is the
/// original-size `w * h` row-major output, where `adjW = 1 << Min(log2_width, 5)`,
/// `w = 1 << log2_width` (and likewise for height).
///
/// The process is: (1) if `lossless && plane_tx_type_is_idtx`, take the
/// § 7.15.4 shortcut `Residual = Dequant >> (3 - shift)` with
/// `shift = (pels > 256) + (pels > 1024)` (`pels = adjW * adjH`); otherwise
/// invoke the § 7.15.4.1 matrix transform into the adjusted block; (2) apply the
/// DPCM cumulative sum to the adjusted block when `dpcm` is set; (3) expand the
/// adjusted block into the original-size residual by sample duplication
/// (nearest-neighbour 2x along any dimension whose original size exceeds 32).
///
/// The computation is total and panic-free for valid shapes: the matrix
/// transform it calls is total, the adjusted scratch is a fixed 32x32 stack
/// buffer, the shortcut shift is in `1..=3`, and the DPCM sum uses
/// `wrapping_add` (conformant residuals are `Clip3`-bounded and never overflow).
///
/// # Errors
/// Returns [`ReconError::InvalidInverseTransform2dShape`] if `log2_width` /
/// `log2_height` are not each in `2..=6` (or not both `2` when lossless), and
/// [`ReconError::InverseTransform2dOuterBufferMismatch`] if `dequant` is not the
/// adjusted `adjW * adjH` or `residual` is not the original `w * h`.
pub fn inverse_transform_2d_outer(
    params: &InverseTransform2dOuter,
    dequant: &[i32],
    residual: &mut [i32],
) -> Result<()> {
    let (log2_w, log2_h) = (params.log2_width, params.log2_height);
    if !(MIN_LOG2_DIM..=MAX_LOG2_DIM).contains(&log2_w)
        || !(MIN_LOG2_DIM..=MAX_LOG2_DIM).contains(&log2_h)
        || (params.lossless && (log2_w != MIN_LOG2_DIM || log2_h != MIN_LOG2_DIM))
    {
        return Err(ReconError::InvalidInverseTransform2dShape {
            log2_w,
            log2_h,
            lossless: params.lossless,
        });
    }

    // Adjusted operating size (capped at 32) and original size.
    let adj_w = 1usize << log2_w.min(5);
    let adj_h = 1usize << log2_h.min(5);
    let orig_w = 1usize << log2_w;
    let orig_h = 1usize << log2_h;
    let adj_pels = adj_w * adj_h;
    let orig_pels = orig_w * orig_h;
    if dequant.len() != adj_pels || residual.len() != orig_pels {
        return Err(ReconError::InverseTransform2dOuterBufferMismatch {
            dequant_expected: adj_pels,
            residual_expected: orig_pels,
            dequant_len: dequant.len(),
            residual_len: residual.len(),
        });
    }

    // Adjusted-size residual scratch; sample duplication expands it into the
    // caller's original-size `residual`.
    let mut scratch = [0i32; MAX_ADJ_DIM * MAX_ADJ_DIM];
    let adj = &mut scratch[..adj_pels];

    if params.lossless && params.plane_tx_type_is_idtx {
        // § 7.15.4 lossless IDTX shortcut: Residual = Dequant >> (3 - shift).
        let shift = u32::from(adj_pels > 256) + u32::from(adj_pels > 1024);
        let down = 3 - shift; // shift is 0..=2, so down is 1..=3.
        for (out, &coeff) in adj.iter_mut().zip(dequant.iter()) {
            *out = coeff >> down;
        }
    } else {
        let inner = InverseTransform2d {
            log2_width: log2_w,
            log2_height: log2_h,
            lossless: params.lossless,
            row_type: params.row_type,
            col_type: params.col_type,
            row_shift: params.row_shift,
            col_shift: params.col_shift,
            bit_depth: params.bit_depth,
        };
        inverse_transform_2d(&inner, dequant, adj)?;
    }

    if let Some(direction) = params.dpcm {
        apply_dpcm(adj, adj_w, adj_h, direction);
    }

    // Sample duplication (§ 7.15.4): nearest-neighbour 2x along any dimension
    // whose original size exceeds the adjusted size. `w_factor`/`h_factor` are
    // 1 or 2; when both are 1 this is a straight copy.
    let w_factor = orig_w / adj_w;
    let h_factor = orig_h / adj_h;
    for oi in 0..orig_h {
        let src_row = (oi / h_factor) * adj_w;
        let dst_row = oi * orig_w;
        for oj in 0..orig_w {
            residual[dst_row + oj] = adj[src_row + oj / w_factor];
        }
    }
    Ok(())
}

/// AV2 § 7.15.4 DPCM cumulative sum over the `w * h` row-major `res` block. Uses
/// `wrapping_add` so the primitive is total; conformant `Clip3`-bounded residuals
/// never overflow.
fn apply_dpcm(res: &mut [i32], w: usize, h: usize, direction: DpcmDirection) {
    match direction {
        DpcmDirection::Vertical => {
            // V_PRED: accumulate down each column.
            for i in 1..h {
                for j in 0..w {
                    res[i * w + j] = res[i * w + j].wrapping_add(res[(i - 1) * w + j]);
                }
            }
        }
        DpcmDirection::Horizontal => {
            // Otherwise: accumulate across each row.
            for i in 0..h {
                for j in 1..w {
                    res[i * w + j] = res[i * w + j].wrapping_add(res[i * w + j - 1]);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::inverse_transform::InverseTransform1dType;

    fn dct() -> InverseTransform2dDim {
        InverseTransform2dDim::Kernel(InverseTransform1dType::Dct)
    }

    #[allow(clippy::too_many_arguments)]
    fn params(
        log2_width: u32,
        log2_height: u32,
        lossless: bool,
        plane_tx_type_is_idtx: bool,
        row_type: InverseTransform2dDim,
        col_type: InverseTransform2dDim,
        row_shift: u8,
        col_shift: u8,
        dpcm: Option<DpcmDirection>,
    ) -> InverseTransform2dOuter {
        InverseTransform2dOuter {
            log2_width,
            log2_height,
            lossless,
            plane_tx_type_is_idtx,
            row_type,
            col_type,
            row_shift,
            col_shift,
            bit_depth: BitDepth::Eight,
            dpcm,
        }
    }

    #[test]
    fn non_adjusted_block_matches_core_transform() {
        // For a non-64 shape with no shortcut/DPCM, the outer process is exactly
        // the § 7.15.4.1 core (no adjustment, no duplication).
        let mut dequant = [0i32; 16];
        dequant[0] = 128;
        dequant[5] = 20;
        let p = params(2, 2, false, false, dct(), dct(), 7, 10, None);

        let mut got = [0i32; 16];
        inverse_transform_2d_outer(&p, &dequant, &mut got).unwrap();

        let inner = InverseTransform2d {
            log2_width: 2,
            log2_height: 2,
            lossless: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 7,
            col_shift: 10,
            bit_depth: BitDepth::Eight,
        };
        let mut expected = [0i32; 16];
        inverse_transform_2d(&inner, &dequant, &mut expected).unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn lossless_idtx_shortcut_is_dequant_shifted() {
        // 4x4 lossless IDTX: shift = (16>256)+(16>1024) = 0, so Residual =
        // Dequant >> 3, bypassing the matrix transform entirely.
        let mut dequant = [0i32; 16];
        dequant[0] = 64; // >> 3 = 8
        dequant[7] = -32; // >> 3 = -4
        dequant[15] = 9; // 9 >> 3 = 1 (arithmetic floor)
        let p = params(2, 2, true, true, dct(), dct(), 0, 0, None);

        let mut residual = [0i32; 16];
        inverse_transform_2d_outer(&p, &dequant, &mut residual).unwrap();

        let mut expected = [0i32; 16];
        for (e, &d) in expected.iter_mut().zip(dequant.iter()) {
            *e = d >> 3;
        }
        assert_eq!(residual, expected);
        assert_eq!(residual[0], 8);
        assert_eq!(residual[7], -4);
        assert_eq!(residual[15], 1);
    }

    #[test]
    fn vertical_dpcm_accumulates_down_columns() {
        // With the identity transform (so the residual mirrors the input) and a
        // single-column impulse, V_PRED DPCM turns a unit step into a ramp down
        // the column. Use identity row+col, zero shifts: residual == dequant,
        // then cumulative sum.
        let id = InverseTransform2dDim::Identity;
        // Column 0 holds [a, b, c, d]; after vertical DPCM: [a, a+b, a+b+c, ...].
        // First run WITHOUT dpcm to learn the post-identity column values, then
        // assert the dpcm run is their running sum.
        let mut dequant = [0i32; 16];
        dequant[0] = 4; // (0,0)
        dequant[4] = 1; // (1,0)
        dequant[8] = 2; // (2,0)
        dequant[12] = 3; // (3,0)

        let base = params(2, 2, false, false, id, id, 0, 0, None);
        let mut plain = [0i32; 16];
        inverse_transform_2d_outer(&base, &dequant, &mut plain).unwrap();

        let vp = params(
            2,
            2,
            false,
            false,
            id,
            id,
            0,
            0,
            Some(DpcmDirection::Vertical),
        );
        let mut summed = [0i32; 16];
        inverse_transform_2d_outer(&vp, &dequant, &mut summed).unwrap();

        // Each column of `summed` is the running top-to-bottom sum of `plain`.
        for j in 0..4 {
            let mut acc = 0i32;
            for i in 0..4 {
                acc += plain[i * 4 + j];
                assert_eq!(summed[i * 4 + j], acc, "column {j} row {i}");
            }
        }
    }

    #[test]
    fn horizontal_dpcm_accumulates_across_rows() {
        let id = InverseTransform2dDim::Identity;
        let mut dequant = [0i32; 16];
        dequant[0] = 5;
        dequant[1] = 1;
        dequant[2] = 2;
        dequant[3] = 3;

        let base = params(2, 2, false, false, id, id, 0, 0, None);
        let mut plain = [0i32; 16];
        inverse_transform_2d_outer(&base, &dequant, &mut plain).unwrap();

        let hp = params(
            2,
            2,
            false,
            false,
            id,
            id,
            0,
            0,
            Some(DpcmDirection::Horizontal),
        );
        let mut summed = [0i32; 16];
        inverse_transform_2d_outer(&hp, &dequant, &mut summed).unwrap();

        for i in 0..4 {
            let mut acc = 0i32;
            for j in 0..4 {
                acc += plain[i * 4 + j];
                assert_eq!(summed[i * 4 + j], acc, "row {i} col {j}");
            }
        }
    }

    #[test]
    fn sample_duplication_expands_64_wide_block() {
        // TX_64X8 (log2 6x3): adjusted is 32x8, original is 64x8, so each adjusted
        // column is duplicated horizontally (orig[i][2j] == orig[i][2j+1] ==
        // adj[i][j]). Compare against the adjusted core result.
        const ADJ_W: usize = 32;
        const ADJ_H: usize = 8;
        const ORIG_W: usize = 64;
        let mut dequant = [0i32; ADJ_W * ADJ_H];
        dequant[0] = 256;
        dequant[ADJ_W + 1] = -40;
        dequant[3 * ADJ_W + 7] = 11;

        // Adjusted-core reference (log2 derives adjusted 32x8 internally).
        let inner = InverseTransform2d {
            log2_width: 6,
            log2_height: 3,
            lossless: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 6,
            col_shift: 12,
            bit_depth: BitDepth::Eight,
        };
        let mut adj = [0i32; ADJ_W * ADJ_H];
        inverse_transform_2d(&inner, &dequant, &mut adj).unwrap();

        let p = params(6, 3, false, false, dct(), dct(), 6, 12, None);
        let mut residual = [0i32; ORIG_W * ADJ_H];
        inverse_transform_2d_outer(&p, &dequant, &mut residual).unwrap();

        for i in 0..ADJ_H {
            for j in 0..ADJ_W {
                let v = adj[i * ADJ_W + j];
                assert_eq!(residual[i * ORIG_W + 2 * j], v, "dup left at ({i},{j})");
                assert_eq!(
                    residual[i * ORIG_W + 2 * j + 1],
                    v,
                    "dup right at ({i},{j})"
                );
            }
        }
    }

    #[test]
    fn sample_duplication_expands_both_dimensions_for_64x64() {
        // TX_64X64 (log2 6x6): adjusted 32x32, original 64x64; every adjusted
        // sample maps to a 2x2 original block: orig[2i+di][2j+dj] == adj[i][j].
        const ADJ: usize = 32;
        const ORIG: usize = 64;
        let mut dequant = [0i32; ADJ * ADJ];
        dequant[0] = 512;
        dequant[ADJ * 5 + 9] = -17;

        let inner = InverseTransform2d {
            log2_width: 6,
            log2_height: 6,
            lossless: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 6,
            col_shift: 13,
            bit_depth: BitDepth::Eight,
        };
        let mut adj = [0i32; ADJ * ADJ];
        inverse_transform_2d(&inner, &dequant, &mut adj).unwrap();

        let p = params(6, 6, false, false, dct(), dct(), 6, 13, None);
        let mut residual = [0i32; ORIG * ORIG];
        inverse_transform_2d_outer(&p, &dequant, &mut residual).unwrap();

        for i in 0..ADJ {
            for j in 0..ADJ {
                let v = adj[i * ADJ + j];
                for di in 0..2 {
                    for dj in 0..2 {
                        assert_eq!(
                            residual[(2 * i + di) * ORIG + (2 * j + dj)],
                            v,
                            "2x2 dup at ({i},{j}) offset ({di},{dj})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn sample_duplication_expands_64_tall_block() {
        // TX_8X64 (log2 3x6): adjusted is 8x32, original is 8x64, so each adjusted
        // row is duplicated vertically (orig[2i][j] == orig[2i+1][j] == adj[i][j]).
        const ADJ_W: usize = 8;
        const ADJ_H: usize = 32;
        const ORIG_H: usize = 64;
        let mut dequant = [0i32; ADJ_W * ADJ_H];
        dequant[0] = 256;
        dequant[ADJ_W + 1] = -40;
        dequant[7 * ADJ_W + 3] = 11;

        let inner = InverseTransform2d {
            log2_width: 3,
            log2_height: 6,
            lossless: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 6,
            col_shift: 12,
            bit_depth: BitDepth::Eight,
        };
        let mut adj = [0i32; ADJ_W * ADJ_H];
        inverse_transform_2d(&inner, &dequant, &mut adj).unwrap();

        let p = params(3, 6, false, false, dct(), dct(), 6, 12, None);
        let mut residual = [0i32; ADJ_W * ORIG_H];
        inverse_transform_2d_outer(&p, &dequant, &mut residual).unwrap();

        for i in 0..ADJ_H {
            for j in 0..ADJ_W {
                let v = adj[i * ADJ_W + j];
                assert_eq!(residual[(2 * i) * ADJ_W + j], v, "dup top at ({i},{j})");
                assert_eq!(
                    residual[(2 * i + 1) * ADJ_W + j],
                    v,
                    "dup bottom at ({i},{j})"
                );
            }
        }
    }

    #[test]
    fn dpcm_applies_to_adjusted_block_before_duplication() {
        // TX_64X8 (log2 6x3) with vertical DPCM: the cumulative sum runs on the
        // adjusted 32x8 block, and the duplicated original-size output mirrors
        // the post-DPCM adjusted samples (orig[i][2j] == orig[i][2j+1] ==
        // dpcm(adj)[i][j]). Proves the DPCM-then-duplicate order and interaction.
        const ADJ_W: usize = 32;
        const ADJ_H: usize = 8;
        const ORIG_W: usize = 64;
        let mut dequant = [0i32; ADJ_W * ADJ_H];
        dequant[0] = 256;
        dequant[ADJ_W + 1] = -40;
        dequant[5 * ADJ_W + 6] = 11;

        let inner = InverseTransform2d {
            log2_width: 6,
            log2_height: 3,
            lossless: false,
            row_type: dct(),
            col_type: dct(),
            row_shift: 6,
            col_shift: 12,
            bit_depth: BitDepth::Eight,
        };
        let mut adj = [0i32; ADJ_W * ADJ_H];
        inverse_transform_2d(&inner, &dequant, &mut adj).unwrap();
        // Manual vertical DPCM down each adjusted column.
        for j in 0..ADJ_W {
            for i in 1..ADJ_H {
                adj[i * ADJ_W + j] += adj[(i - 1) * ADJ_W + j];
            }
        }

        let p = params(
            6,
            3,
            false,
            false,
            dct(),
            dct(),
            6,
            12,
            Some(DpcmDirection::Vertical),
        );
        let mut residual = [0i32; ORIG_W * ADJ_H];
        inverse_transform_2d_outer(&p, &dequant, &mut residual).unwrap();

        for i in 0..ADJ_H {
            for j in 0..ADJ_W {
                let v = adj[i * ADJ_W + j];
                assert_eq!(residual[i * ORIG_W + 2 * j], v, "dup left at ({i},{j})");
                assert_eq!(
                    residual[i * ORIG_W + 2 * j + 1],
                    v,
                    "dup right at ({i},{j})"
                );
            }
        }
    }

    #[test]
    fn dpcm_applies_after_lossless_idtx_shortcut() {
        // Lossless IDTX shortcut feeds the DPCM cumulative sum: the 4x4 result is
        // the vertical running sum of (Dequant >> 3), bypassing the matrix
        // transform. Proves the shortcut-then-DPCM order.
        let mut dequant = [0i32; 16];
        dequant[0] = 64; // >> 3 = 8
        dequant[4] = 16; // >> 3 = 2
        dequant[8] = 24; // >> 3 = 3
        dequant[12] = -8; // >> 3 = -1
        dequant[1] = 32; // >> 3 = 4 (separate column, stays put under vertical DPCM)

        let p = params(
            2,
            2,
            true,
            true,
            dct(),
            dct(),
            0,
            0,
            Some(DpcmDirection::Vertical),
        );
        let mut residual = [0i32; 16];
        inverse_transform_2d_outer(&p, &dequant, &mut residual).unwrap();

        // Expected: shifted, then vertical cumulative sum down each column.
        let mut expected = [0i32; 16];
        for (e, &d) in expected.iter_mut().zip(dequant.iter()) {
            *e = d >> 3;
        }
        for j in 0..4 {
            for i in 1..4 {
                expected[i * 4 + j] += expected[(i - 1) * 4 + j];
            }
        }
        assert_eq!(residual, expected);
        // Column 0 spot check: [8, 2, 3, -1] -> [8, 10, 13, 12].
        assert_eq!(
            [residual[0], residual[4], residual[8], residual[12]],
            [8, 10, 13, 12]
        );
    }

    #[test]
    fn dpcm_is_total_for_extreme_values() {
        // Vertical DPCM over i32::MAX-heavy identity output must not panic
        // (wrapping_add keeps it total even past i32 range).
        let id = InverseTransform2dDim::Identity;
        let mut dequant = [0i32; 16];
        // Identity at zero shift with a large coefficient; the identity clamp in
        // the 1D primitive bounds it, but exercise the DPCM totality path.
        for d in dequant.iter_mut() {
            *d = i32::MAX;
        }
        let vp = params(
            2,
            2,
            false,
            false,
            id,
            id,
            0,
            0,
            Some(DpcmDirection::Vertical),
        );
        let mut residual = [0i32; 16];
        assert!(inverse_transform_2d_outer(&vp, &dequant, &mut residual).is_ok());
    }

    #[test]
    fn rejects_unsupported_shape() {
        let mut residual = [0i32; 16];
        assert!(matches!(
            inverse_transform_2d_outer(
                &params(7, 2, false, false, dct(), dct(), 0, 0, None),
                &[0i32; 16],
                &mut residual
            ),
            Err(ReconError::InvalidInverseTransform2dShape {
                log2_w: 7,
                log2_h: 2,
                lossless: false
            })
        ));
    }

    #[test]
    fn rejects_non_4x4_lossless() {
        let mut residual = [0i32; 32];
        assert!(matches!(
            inverse_transform_2d_outer(
                &params(3, 2, true, true, dct(), dct(), 0, 0, None),
                &[0i32; 32],
                &mut residual
            ),
            Err(ReconError::InvalidInverseTransform2dShape {
                log2_w: 3,
                log2_h: 2,
                lossless: true
            })
        ));
    }

    #[test]
    fn rejects_buffer_length_mismatch() {
        // TX_64X8: dequant must be adjusted 32*8=256, residual original 64*8=512.
        let mut residual = [0i32; 512];
        assert!(matches!(
            inverse_transform_2d_outer(
                &params(6, 3, false, false, dct(), dct(), 6, 12, None),
                &[0i32; 255],
                &mut residual
            ),
            Err(ReconError::InverseTransform2dOuterBufferMismatch {
                dequant_expected: 256,
                residual_expected: 512,
                dequant_len: 255,
                residual_len: 512
            })
        ));
    }
}
