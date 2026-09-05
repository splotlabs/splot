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

#[cfg(test)]
use crate::inverse_transform_2d::inverse_transform_2d;
use crate::inverse_transform_2d::{
    InverseTransform2d, InverseTransform2dDim, inverse_transform_2d_with_scratch,
};
use crate::transform_params::{TransformPass, get_transform_1d_type, transform_shift};
use crate::{BitDepth, ReconError, Result};

/// Maximum adjusted 1D transform length (§ 7.15.4 caps each adjusted side at 32).
const MAX_ADJ_DIM: usize = 32;

/// Minimum original transform-dimension base-2 logarithm (a 4-sample side).
const MIN_LOG2_DIM: u32 = 2;

/// Maximum original transform-dimension base-2 logarithm (a 64-sample side).
const MAX_LOG2_DIM: u32 = 6;

/// Maximum adjusted transform-dimension base-2 logarithm (§ 7.15.4 caps each
/// adjusted side at 32 samples, i.e. `Adjusted_Tx_Size` caps `log2` at 5).
const MAX_ADJ_LOG2_DIM: u32 = 5;

/// AV2 `IDTX` transform-type value. `PlaneTxType == IDTX` selects the § 7.15.4
/// lossless bit-shift shortcut. `docs/spec/av2/1.0.0/03-symbols.md` lists
/// `IDTX = 9`.
const IDTX_TX_TYPE: usize = 9;

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
/// `colType` via `get_transform_1d_type`). When `lossless`
/// without IDTX the Walsh-Hadamard core requires a 4x4 block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InverseTransform2dOuter {
    /// Original (unadjusted) transform width base-2 logarithm (`log2W`), 2..=6.
    pub log2_width: u32,
    /// Original (unadjusted) transform height base-2 logarithm (`log2H`), 2..=6.
    pub log2_height: u32,
    /// Whether the block is lossless.
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

impl InverseTransform2dOuter {
    /// Resolves the AV2 § 7.15.4 outer parameters from a single transform-block
    /// fact set, so the per-pass transform selections, the per-pass shifts, the
    /// `PlaneTxType == IDTX` flag, and the stored log2 dimensions are mutually
    /// consistent by construction
    /// (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-15-4`).
    ///
    /// This is the combined § 7.15.4 transform-parameter resolve helper that the
    /// [`transform_shift`] (`RECON-TRANSFORM-SHIFT-LOOKUP`) and
    /// [`get_transform_1d_type`] (`RECON-GET-TRANSFORM-1D-TYPE`) rows left for a
    /// caller to compose. Deriving `row_shift` / `col_shift` and `row_type` /
    /// `col_type` from the *same* `(log2_width, log2_height)` and `plane_tx_type`
    /// the result stores avoids the dual-source hazard of a caller computing the
    /// shifts and types from one `txSz` and the dimensions from another.
    ///
    /// `plane_tx_type` is `PlaneTxType` (`0..TX_TYPES`); `log2_width` /
    /// `log2_height` are the original (unadjusted) `txSz` base-2 log dimensions
    /// (`Tx_Width_Log2[txSz]` / `Tx_Height_Log2[txSz]`, each `2..=6`); `use_ddt`
    /// is the caller-resolved `enable_inter_ddt && !use_intrabc && is_inter`.
    /// `lossless`, `bit_depth`, and `dpcm` are independent caller facts the
    /// transform size and type cannot supply.
    ///
    /// The shifts come from `Transform_Shift[txSz]` via [`transform_shift`]; the
    /// per-pass types from [`get_transform_1d_type`] applied to the *adjusted*
    /// sample sizes `w = 1 << Min(log2_width, 5)` and `h = 1 << Min(log2_height,
    /// 5)`, exactly as § 7.15.4.1 sets `rowType = get_transform_1d_type(0, w)` and
    /// `colType = get_transform_1d_type(1, h)`. (When `lossless`, the apply step
    /// takes either the IDTX shortcut or Walsh-Hadamard path and ignores
    /// `row_type` / `col_type`; they are still resolved here for a uniform, total
    /// constructor.)
    ///
    /// This is a `const fn` so a fixed transform shape resolves at compile time.
    ///
    /// # Errors
    /// Returns [`ReconError::InvalidTransformShiftShape`] if `(log2_width,
    /// log2_height)` is not one of the AV2 `TX_SIZES_ALL` transform shapes, and
    /// [`ReconError::InvalidPlaneTxType`] if `plane_tx_type` is not a `TX_TYPES`
    /// index (`0..16`). Neither the shape nor the type is consumed before both
    /// are validated, so a rejected call resolves no partial parameters.
    pub const fn resolve(
        plane_tx_type: usize,
        log2_width: u32,
        log2_height: u32,
        use_ddt: bool,
        lossless: bool,
        bit_depth: BitDepth,
        dpcm: Option<DpcmDirection>,
    ) -> Result<Self> {
        let (row_shift, col_shift) = match transform_shift(log2_width, log2_height) {
            Ok(shifts) => shifts,
            Err(error) => return Err(error),
        };
        let adj_w = 1usize << cap_adjusted_log2(log2_width);
        let adj_h = 1usize << cap_adjusted_log2(log2_height);
        let row_type =
            match get_transform_1d_type(plane_tx_type, TransformPass::Row, adj_w, use_ddt) {
                Ok(kind) => kind,
                Err(error) => return Err(error),
            };
        let col_type =
            match get_transform_1d_type(plane_tx_type, TransformPass::Col, adj_h, use_ddt) {
                Ok(kind) => kind,
                Err(error) => return Err(error),
            };
        Ok(Self {
            log2_width,
            log2_height,
            lossless,
            plane_tx_type_is_idtx: plane_tx_type == IDTX_TX_TYPE,
            row_type,
            col_type,
            row_shift,
            col_shift,
            bit_depth,
            dpcm,
        })
    }
}

/// Caps a transform-dimension base-2 logarithm at the § 7.15.4 adjusted size (a
/// `const fn` form of `Min(log2, 5)`, the `Adjusted_Tx_Size` per-side cap).
const fn cap_adjusted_log2(log2_dim: u32) -> u32 {
    if log2_dim < MAX_ADJ_LOG2_DIM {
        log2_dim
    } else {
        MAX_ADJ_LOG2_DIM
    }
}

const _RESOLVE_CONST_EVAL_CHECK: () = assert!(matches!(
    InverseTransform2dOuter::resolve(0, 2, 2, false, false, BitDepth::Eight, None),
    Ok(InverseTransform2dOuter {
        row_shift: 7,
        col_shift: 10,
        plane_tx_type_is_idtx: false,
        row_type: InverseTransform2dDim::Kernel(_),
        col_type: InverseTransform2dDim::Kernel(_),
        ..
    })
));

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
/// `log2_height` are not each in `2..=6` (or not both `2` for lossless non-IDTX),
/// and [`ReconError::InverseTransform2dOuterBufferMismatch`] if `dequant` is not
/// the adjusted `adjW * adjH` or `residual` is not the original `w * h`.
pub fn inverse_transform_2d_outer(
    params: &InverseTransform2dOuter,
    dequant: &[i32],
    residual: &mut [i32],
) -> Result<()> {
    let (adj_w, adj_h, orig_w, orig_h) = transform_dimensions(params)?;
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

    let mut transform_scratch = [0i32; MAX_ADJ_DIM * MAX_ADJ_DIM];
    if orig_pels == adj_pels {
        return inverse_transform_2d_outer_adjusted_inner(
            params,
            dequant,
            residual,
            &mut transform_scratch,
            adj_w,
            adj_h,
        );
    }

    let mut adjusted = [0i32; MAX_ADJ_DIM * MAX_ADJ_DIM];
    let adj = &mut adjusted[..adj_pels];
    inverse_transform_2d_outer_adjusted_inner(
        params,
        dequant,
        adj,
        &mut transform_scratch,
        adj_w,
        adj_h,
    )?;

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

/// Applies AV2 § 7.15.4 through the adjusted-size residual, before the final
/// sample-duplication expansion for a 64-sample transform side.
///
/// `dequant` and `residual` must each contain the adjusted `adjW * adjH`
/// samples. The caller supplies one reusable 32x32 matrix-transform scratch
/// block and applies the § 7.15.4 duplication when writing an original-size
/// output.
///
/// # Errors
/// Returns the same shape and transform errors as [`inverse_transform_2d_outer`],
/// [`ReconError::InverseTransform2dBufferMismatch`] when `dequant` or `residual`
/// does not match the adjusted size.
#[inline]
pub fn inverse_transform_2d_outer_adjusted(
    params: &InverseTransform2dOuter,
    dequant: &[i32],
    residual: &mut [i32],
    transform_scratch: &mut [i32; 32 * 32],
) -> Result<()> {
    let (adj_w, adj_h, _, _) = transform_dimensions(params)?;
    let adj_pels = adj_w * adj_h;
    if dequant.len() != adj_pels || residual.len() != adj_pels {
        return Err(ReconError::InverseTransform2dBufferMismatch {
            expected: adj_pels,
            dequant_len: dequant.len(),
            residual_len: residual.len(),
        });
    }
    inverse_transform_2d_outer_adjusted_inner(
        params,
        dequant,
        residual,
        transform_scratch,
        adj_w,
        adj_h,
    )
}

fn inverse_transform_2d_outer_adjusted_inner(
    params: &InverseTransform2dOuter,
    dequant: &[i32],
    residual: &mut [i32],
    transform_scratch: &mut [i32; MAX_ADJ_DIM * MAX_ADJ_DIM],
    adj_w: usize,
    adj_h: usize,
) -> Result<()> {
    let (log2_w, log2_h) = (params.log2_width, params.log2_height);
    let adj_pels = adj_w * adj_h;

    if params.lossless && params.plane_tx_type_is_idtx {
        let shift = u32::from(adj_pels > 256) + u32::from(adj_pels > 1024);
        let down = 3 - shift; // shift is 0..=2, so down is 1..=3.
        for (out, &coeff) in residual.iter_mut().zip(dequant.iter()) {
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
        inverse_transform_2d_with_scratch(&inner, dequant, residual, transform_scratch)?;
    }

    if let Some(direction) = params.dpcm {
        apply_dpcm(residual, adj_w, adj_h, direction);
    }
    Ok(())
}

fn transform_dimensions(params: &InverseTransform2dOuter) -> Result<(usize, usize, usize, usize)> {
    let (log2_w, log2_h) = (params.log2_width, params.log2_height);
    if !(MIN_LOG2_DIM..=MAX_LOG2_DIM).contains(&log2_w)
        || !(MIN_LOG2_DIM..=MAX_LOG2_DIM).contains(&log2_h)
        || (params.lossless
            && !params.plane_tx_type_is_idtx
            && (log2_w != MIN_LOG2_DIM || log2_h != MIN_LOG2_DIM))
    {
        return Err(ReconError::InvalidInverseTransform2dShape {
            log2_w,
            log2_h,
            lossless: params.lossless,
        });
    }
    Ok((
        1usize << log2_w.min(MAX_ADJ_LOG2_DIM),
        1usize << log2_h.min(MAX_ADJ_LOG2_DIM),
        1usize << log2_w,
        1usize << log2_h,
    ))
}

/// AV2 § 7.15.4 DPCM cumulative sum over the `w * h` row-major `res` block. Uses
/// `wrapping_add` so the primitive is total; conformant `Clip3`-bounded residuals
/// never overflow.
fn apply_dpcm(res: &mut [i32], w: usize, h: usize, direction: DpcmDirection) {
    match direction {
        DpcmDirection::Vertical => {
            for i in 1..h {
                for j in 0..w {
                    res[i * w + j] = res[i * w + j].wrapping_add(res[(i - 1) * w + j]);
                }
            }
        }
        DpcmDirection::Horizontal => {
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
    fn lossless_idtx_shortcut_admits_rectangular_transform_sizes() {
        let mut dequant = [0i32; 32];
        dequant[0] = 64;
        dequant[7] = -32;
        dequant[31] = 15;
        let p = params(3, 2, true, true, dct(), dct(), 0, 0, None);

        let mut residual = [0i32; 32];
        inverse_transform_2d_outer(&p, &dequant, &mut residual).unwrap();

        let mut expected = [0i32; 32];
        for (e, &d) in expected.iter_mut().zip(dequant.iter()) {
            *e = d >> 3;
        }
        assert_eq!(residual, expected);
        assert_eq!(residual[0], 8);
        assert_eq!(residual[7], -4);
        assert_eq!(residual[31], 1);
    }

    #[test]
    fn vertical_dpcm_accumulates_down_columns() {
        let id = InverseTransform2dDim::Identity;
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
        const ADJ_W: usize = 32;
        const ADJ_H: usize = 8;
        const ORIG_W: usize = 64;
        let mut dequant = [0i32; ADJ_W * ADJ_H];
        dequant[0] = 256;
        dequant[ADJ_W + 1] = -40;
        dequant[3 * ADJ_W + 7] = 11;

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
        assert_eq!(
            [residual[0], residual[4], residual[8], residual[12]],
            [8, 10, 13, 12]
        );
    }

    #[test]
    fn dpcm_is_total_for_extreme_values() {
        let id = InverseTransform2dDim::Identity;
        let mut dequant = [0i32; 16];
        dequant.fill(i32::MAX);
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
    fn rejects_non_4x4_lossless_walsh_hadamard() {
        let mut residual = [0i32; 32];
        assert!(matches!(
            inverse_transform_2d_outer(
                &params(3, 2, true, false, dct(), dct(), 0, 0, None),
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

    const RESOLVE_SHAPES: [(u32, u32); 8] = [
        (2, 2),
        (3, 3),
        (4, 4),
        (5, 5),
        (6, 6),
        (2, 3),
        (3, 2),
        (6, 5),
    ];

    #[test]
    fn resolve_wires_the_shift_and_type_helpers_with_the_right_arguments() {
        for &(log2_w, log2_h) in &RESOLVE_SHAPES {
            for plane_tx_type in [0usize, 3, 9, 13, 15] {
                for use_ddt in [false, true] {
                    let resolved = InverseTransform2dOuter::resolve(
                        plane_tx_type,
                        log2_w,
                        log2_h,
                        use_ddt,
                        false,
                        BitDepth::Eight,
                        Some(DpcmDirection::Vertical),
                    )
                    .unwrap();

                    let (row_shift, col_shift) = transform_shift(log2_w, log2_h).unwrap();
                    let adj_w = 1usize << log2_w.min(5);
                    let adj_h = 1usize << log2_h.min(5);
                    let row_type =
                        get_transform_1d_type(plane_tx_type, TransformPass::Row, adj_w, use_ddt)
                            .unwrap();
                    let col_type =
                        get_transform_1d_type(plane_tx_type, TransformPass::Col, adj_h, use_ddt)
                            .unwrap();

                    assert_eq!(
                        resolved,
                        InverseTransform2dOuter {
                            log2_width: log2_w,
                            log2_height: log2_h,
                            lossless: false,
                            plane_tx_type_is_idtx: plane_tx_type == 9,
                            row_type,
                            col_type,
                            row_shift,
                            col_shift,
                            bit_depth: BitDepth::Eight,
                            dpcm: Some(DpcmDirection::Vertical),
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn resolve_applies_ddt_substitution_per_pass_on_the_adjusted_size() {
        let resolved =
            InverseTransform2dOuter::resolve(3, 3, 2, true, false, BitDepth::Eight, None).unwrap();
        assert_eq!(
            resolved.row_type,
            InverseTransform2dDim::Kernel(InverseTransform1dType::Ddtx)
        );
        assert_eq!(
            resolved.col_type,
            InverseTransform2dDim::Kernel(InverseTransform1dType::Adst)
        );

        let no_ddt =
            InverseTransform2dOuter::resolve(3, 3, 2, false, false, BitDepth::Eight, None).unwrap();
        assert_eq!(
            no_ddt.row_type,
            InverseTransform2dDim::Kernel(InverseTransform1dType::Adst)
        );
        assert_eq!(
            no_ddt.col_type,
            InverseTransform2dDim::Kernel(InverseTransform1dType::Adst)
        );
    }

    #[test]
    fn resolve_produces_params_that_drive_the_outer_transform() {
        let resolved =
            InverseTransform2dOuter::resolve(0, 6, 5, false, false, BitDepth::Eight, None).unwrap();
        let (row_shift, col_shift) = transform_shift(6, 5).unwrap();
        let manual = params(6, 5, false, false, dct(), dct(), row_shift, col_shift, None);
        assert_eq!(resolved, manual);

        let mut dequant = [0i32; 32 * 32];
        dequant[0] = 200;
        dequant[33] = -50;
        let mut from_resolved = [0i32; 64 * 32];
        let mut from_manual = [0i32; 64 * 32];
        inverse_transform_2d_outer(&resolved, &dequant, &mut from_resolved).unwrap();
        inverse_transform_2d_outer(&manual, &dequant, &mut from_manual).unwrap();
        assert_eq!(from_resolved, from_manual);
    }

    #[test]
    fn resolve_rejects_invalid_shape_and_plane_tx_type_without_partial_state() {
        assert!(matches!(
            InverseTransform2dOuter::resolve(0, 7, 7, false, false, BitDepth::Eight, None),
            Err(ReconError::InvalidTransformShiftShape {
                log2_width: 7,
                log2_height: 7
            })
        ));
        assert!(matches!(
            InverseTransform2dOuter::resolve(16, 2, 2, false, false, BitDepth::Eight, None),
            Err(ReconError::InvalidPlaneTxType { plane_tx_type: 16 })
        ));
    }

    #[test]
    fn resolve_is_total_for_every_valid_shape_and_tx_type() {
        for log2_w in 0..=8u32 {
            for log2_h in 0..=8u32 {
                for plane_tx_type in 0..18usize {
                    for use_ddt in [false, true] {
                        match InverseTransform2dOuter::resolve(
                            plane_tx_type,
                            log2_w,
                            log2_h,
                            use_ddt,
                            false,
                            BitDepth::Eight,
                            None,
                        ) {
                            Ok(resolved) => {
                                assert_eq!(resolved.log2_width, log2_w);
                                assert_eq!(resolved.log2_height, log2_h);
                                assert_eq!(resolved.plane_tx_type_is_idtx, plane_tx_type == 9);
                            }
                            Err(error) => assert!(
                                matches!(
                                    error,
                                    ReconError::InvalidTransformShiftShape { .. }
                                        | ReconError::InvalidPlaneTxType { .. }
                                ),
                                "unexpected resolve error: {error:?}"
                            ),
                        }
                    }
                }
            }
        }
    }
}
