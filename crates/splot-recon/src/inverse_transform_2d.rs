// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.4.1 2D inverse transform (matrix transform) core.
//!
//! This module wires the § 7.15.2 1D inverse transforms (the § 7.15.2.1 kernel
//! transform, the § 7.15.2.2 Walsh-Hadamard transform, and the § 7.15.2.3
//! identity transform) into the § 7.15.4.1 row-then-column 2D matrix transform
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-15-4-1`) over a caller-supplied dequantized coefficient block, producing
//! the residual block.
//!
//! Feature tracking: `RECON-INVERSE-TRANSFORM-2D`.
//!
//! Scope: this is the § 7.15.4.1 matrix transform parameterized by the
//! *original* (unadjusted) `txSz` log2 dimensions, the per-dimension transform
//! selection, the row/column shifts, and the lossless flag. The § 7.15.4.1
//! process derives `log2W`/`log2H` from `txSz` and the adjusted `adjLog2W` /
//! `adjLog2H` from `adjTxSz`; because `Adjusted_Tx_Size` caps each dimension's
//! log2 at 5 (`adjLog2 = Min(log2, 5)`, the
//! [`Adjusted_Tx_Size`](../../../docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md)
//! table — see also § 8 `bwl = Min(Tx_Width_Log2[txSz], 5)`), this module takes
//! the original log2 dimensions and derives the adjusted operating size
//! internally. The rescale parity and identity scales use the *original* log2
//! dimensions, exactly as the spec specifies, so transforms with a 64-sample
//! dimension (whose adjusted parity differs) rescale correctly.

use fearless_simd::{Bytes as _, Simd, SimdBase as _, i32x4, i64x2};

use crate::inverse_transform::InverseTransform1dType;
use crate::inverse_transform_2d_8bit::{Transform8, inverse_transform_2d_8bit};
use crate::{BitDepth, ReconError, Result};

/// Maximum 1D transform length: the § 7.15.4 adjusted transform size caps each
/// dimension at 32.
const MAX_DIM: usize = 32;

/// Minimum original transform-dimension base-2 logarithm (a 4-sample side).
const MIN_LOG2_DIM: u32 = 2;

/// Maximum original transform-dimension base-2 logarithm (a 64-sample side).
const MAX_LOG2_DIM: u32 = 6;

/// AV2 § 7.15.4.1 per-dimension 1D transform selection for the non-lossless
/// passes. When the block is lossless, both passes instead use the § 7.15.2.2
/// Walsh-Hadamard transform and these selections are ignored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InverseTransform2dDim {
    /// The § 7.15.2.3 inverse identity transform (the Table 7.1 `IDT` type).
    Identity,
    /// A § 7.15.2.1 kernel transform of the given type.
    Kernel(InverseTransform1dType),
}

/// Caller-resolved parameters for the AV2 § 7.15.4.1 2D matrix transform.
///
/// `log2_width` and `log2_height` are the *original* (unadjusted) transform
/// dimensions `log2W = Tx_Width_Log2[txSz]` / `log2H = Tx_Height_Log2[txSz]`,
/// each in `2..=6` (a 4-, 8-, 16-, 32-, or 64-sample side). The operating
/// (adjusted) size is derived internally as `Min(log2, 5)` per the
/// `Adjusted_Tx_Size` table, so the block actually transformed is at most
/// 32x32. `row_shift` and `col_shift` are the § 7.15.4.1 `rowShift` / `colShift`
/// (the caller's `Transform_Shift[txSz]` lookup). When `lossless` is true the
/// block must be 4x4 (`log2_width == log2_height == 2`) and both passes use the
/// Walsh-Hadamard transform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InverseTransform2d {
    /// Original (unadjusted) transform width base-2 logarithm (`log2W`), 2..=6.
    pub log2_width: u32,
    /// Original (unadjusted) transform height base-2 logarithm (`log2H`), 2..=6.
    pub log2_height: u32,
    /// Whether the block is lossless (forces the Walsh-Hadamard transform).
    pub lossless: bool,
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
}

/// Applies the AV2 § 7.15.4.1 2D matrix transform to the `w * h` row-major
/// `dequant` block, writing the `w * h` row-major `residual`, where `w` and `h`
/// are the adjusted dimensions `1 << Min(log2_width, 5)` and
/// `1 << Min(log2_height, 5)`. `dequant` may hold only the block's leading whole
/// rows; the rows it omits are zero.
///
/// The row pass transforms each row (with the § 7.15.4.1 `Round2(x * 2896, 12)`
/// rescale when `|log2_width - log2_height|` is odd — computed from the
/// *original* log2 dimensions) into an intermediate buffer, and the column pass
/// transforms each intermediate column into the residual. Each pass dispatches
/// to the Walsh-Hadamard transform (lossless), the identity transform
/// (`Identity`), or the kernel transform (`Kernel`); the identity scale is the
/// § 7.15.4.1 `get_identity_scale` of the *original* pass log2 dimension.
///
/// The whole computation is total and panic-free for valid shapes: the 1D
/// transforms it calls are total, and the intermediate buffers are fixed-size
/// stack arrays sized to the 32-element maximum adjusted dimension. Rows whose
/// dequantized coefficients are all zero skip the row pass and fill their
/// intermediate row with zeros — a bit-exact shortcut, since every 1D pass
/// maps an all-zero input row to all zeros.
///
/// # Errors
/// Returns [`ReconError::InvalidInverseTransform2dShape`] if `log2_width` /
/// `log2_height` are not each in `2..=6` (or not both `2` when lossless), and
/// [`ReconError::InverseTransform2dBufferMismatch`] if `dequant` is not a
/// whole-row prefix of `w * h` or `residual` is not exactly `w * h` long.
pub fn inverse_transform_2d(
    params: &InverseTransform2d,
    dequant: &[i32],
    residual: &mut [i32],
) -> Result<()> {
    let mut intermediate = [0i32; MAX_DIM * MAX_DIM];
    inverse_transform_2d_with_scratch(params, dequant, residual, &mut intermediate)
}

pub(super) fn inverse_transform_2d_with_scratch(
    params: &InverseTransform2d,
    dequant: &[i32],
    residual: &mut [i32],
    intermediate: &mut [i32; MAX_DIM * MAX_DIM],
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

    let w = 1usize << log2_w.min(5);
    let h = 1usize << log2_h.min(5);
    let expected = w * h;
    if dequant.len() > expected || !dequant.len().is_multiple_of(w) || residual.len() != expected {
        return Err(ReconError::InverseTransform2dBufferMismatch {
            expected,
            dequant_len: dequant.len(),
            residual_len: residual.len(),
        });
    }

    if params.lossless {
        baseline_simd!(simd => walsh_hadamard_4x4(simd, dequant, residual));
        return Ok(());
    }
    let transform = Transform8 {
        bit_depth: params.bit_depth,
        row: params.row_type,
        col: params.col_type,
        w,
        h,
        rescale: log2_w.abs_diff(log2_h) % 2 == 1,
        row_shift: params.row_shift,
        col_shift: params.col_shift,
        dequant: None,
    };
    if inverse_transform_2d_8bit(transform, dequant, residual, intermediate) {
        return Ok(());
    }
    Err(ReconError::InverseTransform2dBufferMismatch {
        expected,
        dequant_len: dequant.len(),
        residual_len: residual.len(),
    })
}

/// The lossless § 7.15.2.2 Walsh-Hadamard row pass (`shift` 3) and column
/// pass (`shift` 0) of the 4x4 whole-row prefix `dequant` into `residual`:
/// the rows load as vectors and transpose, so one butterfly runs all four
/// rows, and the row outputs transpose back for the column butterfly.
#[inline]
fn walsh_hadamard_4x4<S: Simd>(simd: S, dequant: &[i32], residual: &mut [i32]) {
    let row = |r: usize| {
        dequant
            .get(4 * r..4 * r + 4)
            .map_or(i32x4::splat(simd, 0), |v| i32x4::from_slice(simd, v))
    };
    let columns = transpose_4x4(simd, [row(0), row(1), row(2), row(3)]);
    let rows = walsh_hadamard_lanes(simd, columns, 3);
    let out = walsh_hadamard_lanes(simd, transpose_4x4(simd, rows), 0);
    for (dst, v) in residual.chunks_exact_mut(4).zip(out) {
        v.store_slice(dst);
    }
}

/// The § 7.15.2.2 butterfly with input `[src[0], src[1], src[2], src[3]]`
/// one vector each, each input clamped to the § 7.14.4 dequant range
/// `±2^17` and shifted down by `shift` first.
#[allow(clippy::many_single_char_names, reason = "the § 7.15.2.2 names")]
#[inline]
fn walsh_hadamard_lanes<S: Simd>(simd: S, src: [i32x4<S>; 4], shift: u32) -> [i32x4<S>; 4] {
    let bound = 1 << 17;
    let [mut a, mut c, mut d, mut b] = src.map(|v| {
        v.max(i32x4::splat(simd, -bound))
            .min(i32x4::splat(simd, bound - 1))
            >> shift
    });
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;
    [a, b, c, d]
}

/// The transpose of the 4x4 block whose rows are `rows`.
#[inline]
fn transpose_4x4<S: Simd>(simd: S, [r0, r1, r2, r3]: [i32x4<S>; 4]) -> [i32x4<S>; 4] {
    let pairs = |a: i32x4<S>, b: i32x4<S>| -> (i64x2<S>, i64x2<S>) {
        (
            simd.zip_low_i32x4(a, b).bitcast(),
            simd.zip_high_i32x4(a, b).bitcast(),
        )
    };
    let ((low01, high01), (low23, high23)) = (pairs(r0, r1), pairs(r2, r3));
    [
        simd.zip_low_i64x2(low01, low23).bitcast(),
        simd.zip_high_i64x2(low01, low23).bitcast(),
        simd.zip_low_i64x2(high01, high23).bitcast(),
        simd.zip_high_i64x2(high01, high23).bitcast(),
    ]
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::inverse_transform::{
        inverse_identity_transform, inverse_transform_1d, inverse_walsh_hadamard,
    };

    /// Resolved parameters for one § 7.15.4.1 1D pass of the scalar oracle.
    #[derive(Clone, Copy)]
    struct Pass {
        lossless: bool,
        dim: InverseTransform2dDim,
        shift: u8,
        col_tx: bool,
        log2_size: u32,
        bit_depth: BitDepth,
    }

    /// The scalar oracle of one § 7.15.4.1 1D pass over `src` into `out`.
    fn run_1d(src: &[i32], out: &mut [i32], pass: Pass) -> Result<()> {
        if pass.lossless {
            let walsh_shift = if pass.col_tx { 0 } else { 3 };
            let input = [src[0], src[1], src[2], src[3]];
            out.copy_from_slice(&inverse_walsh_hadamard(input, walsh_shift)); // splot-copy-ok: test oracle output row
            return Ok(());
        }
        match pass.dim {
            InverseTransform2dDim::Identity => inverse_identity_transform(
                src,
                identity_scale(pass.log2_size),
                pass.shift,
                pass.col_tx,
                pass.bit_depth,
                out,
            ),
            InverseTransform2dDim::Kernel(tx_type) => {
                inverse_transform_1d(src, tx_type, pass.shift, pass.col_tx, pass.bit_depth, out)
            }
        }
    }

    /// AV2 § 7.15.4.1 `get_identity_scale( log2Sz )`.
    fn identity_scale(log2_size: u32) -> i32 {
        match log2_size {
            2 => 128,
            3 => 181,
            4 => 256,
            _ => 362,
        }
    }

    /// AV2 § 7.15.4.1 rectangular rescale `Round2( x * 2896, 12 )`.
    fn round2_2896(x: i32) -> i32 {
        (x * 2896 + (1 << 11)) >> 12
    }

    fn params(
        log2_width: u32,
        log2_height: u32,
        lossless: bool,
        row_type: InverseTransform2dDim,
        col_type: InverseTransform2dDim,
        row_shift: u8,
        col_shift: u8,
    ) -> InverseTransform2d {
        InverseTransform2d {
            log2_width,
            log2_height,
            lossless,
            row_type,
            col_type,
            row_shift,
            col_shift,
            bit_depth: BitDepth::Eight,
        }
    }

    fn dct() -> InverseTransform2dDim {
        InverseTransform2dDim::Kernel(InverseTransform1dType::Dct)
    }

    fn adst() -> InverseTransform2dDim {
        InverseTransform2dDim::Kernel(InverseTransform1dType::Adst)
    }

    #[test]
    fn dc_only_dct_4x4_is_flat() {
        let mut dequant = [0i32; 16];
        dequant[0] = 128;
        let mut residual = [0i32; 16];
        inverse_transform_2d(
            &params(2, 2, false, dct(), dct(), 7, 10),
            &dequant,
            &mut residual,
        )
        .unwrap();
        assert_eq!(residual, [4i32; 16]);
    }

    #[test]
    fn lossless_4x4_walsh_hadamard_dc() {
        let mut dequant = [0i32; 16];
        dequant[0] = 64;
        let mut residual = [0i32; 16];
        inverse_transform_2d(
            &params(2, 2, true, dct(), dct(), 0, 0),
            &dequant,
            &mut residual,
        )
        .unwrap();
        assert_eq!(residual, [2i32; 16]);
    }

    /// The vector Walsh-Hadamard matches the per-row and per-column oracle
    /// on random and extreme coefficients, for every coded-row prefix.
    #[test]
    fn lossless_walsh_hadamard_matches_the_reference() {
        let mut state = 0x005e_ed0f_ab1e_u64;
        for trial in 0..200 {
            let mut dequant = [0i32; 16];
            for slot in &mut dequant {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *slot = match trial % 4 {
                    0 => [i32::MIN, i32::MAX, 1 << 17, -(1 << 17) - 1][(state % 4) as usize],
                    1 => (state % (1 << 18)) as i32 - (1 << 17),
                    _ => (state % 512) as i32 - 256,
                };
            }
            let p = params(2, 2, true, dct(), dct(), 0, 0);
            for coded in [16, 12, 8, 4, 0] {
                let mut full = [0i32; 16];
                full[..coded].copy_from_slice(&dequant[..coded]); // splot-copy-ok: test coded-row prefix
                let mut expected = [0i32; 16];
                let scratch = &mut [0; MAX_DIM * MAX_DIM];
                reference_transform_2d(&p, &full, &mut expected, scratch);
                let mut got = [i32::MIN; 16];
                inverse_transform_2d(&p, &dequant[..coded], &mut got).unwrap();
                assert_eq!(got, expected, "trial {trial} coded {coded}");
            }
        }
    }

    #[test]
    fn identity_2d_preserves_coefficient_position() {
        const POS: usize = 5;
        let mut dequant = [0i32; 16];
        dequant[POS] = 10;
        let mut residual = [0i32; 16];
        let p = params(
            2,
            2,
            false,
            InverseTransform2dDim::Identity,
            InverseTransform2dDim::Identity,
            0,
            0,
        );
        inverse_transform_2d(&p, &dequant, &mut residual).unwrap();
        assert_ne!(residual[POS], 0);
        for (idx, &v) in residual.iter().enumerate() {
            if idx != POS {
                assert_eq!(v, 0, "energy leaked to position {idx}");
            }
        }
    }

    #[test]
    fn round2_2896_applies_sqrt2_rescale() {
        assert_eq!(round2_2896(4096), 2896);
        assert_eq!(round2_2896(0), 0);
        assert!(round2_2896(262_144) > 0);
    }

    #[test]
    fn rectangular_4x8_applies_rescale_path() {
        let mut dequant = [0i32; 32];
        dequant[0] = 1024;
        let mut residual = [0i32; 32];
        inverse_transform_2d(
            &params(2, 3, false, dct(), dct(), 6, 13),
            &dequant,
            &mut residual,
        )
        .unwrap();
        let first = residual[0];
        assert_ne!(first, 0);
        assert!(
            residual.iter().all(|&v| v == first),
            "DC-only rectangular block must be flat, got {residual:?}"
        );
    }

    #[test]
    fn rescale_parity_uses_original_not_adjusted_dimensions() {
        const N: usize = 32 * 32;
        let mut dequant_64x32 = [0i32; N];
        dequant_64x32[0] = 4096;
        let mut out_64x32 = [0i32; N];
        inverse_transform_2d(
            &params(6, 5, false, dct(), dct(), 6, 12),
            &dequant_64x32,
            &mut out_64x32,
        )
        .unwrap();

        let mut dequant_pre = [0i32; N];
        dequant_pre[0] = round2_2896(4096); // 2896
        let mut out_pre = [0i32; N];
        inverse_transform_2d(
            &params(5, 5, false, dct(), dct(), 6, 12),
            &dequant_pre,
            &mut out_pre,
        )
        .unwrap();
        assert_eq!(
            out_64x32, out_pre,
            "TX_64X32 must rescale by √2 like a pre-rescaled 32x32"
        );

        let mut dequant_raw = [0i32; N];
        dequant_raw[0] = 4096;
        let mut out_raw = [0i32; N];
        inverse_transform_2d(
            &params(5, 5, false, dct(), dct(), 6, 12),
            &dequant_raw,
            &mut out_raw,
        )
        .unwrap();
        assert_ne!(out_64x32, out_raw, "rescale must change the result");
    }

    #[test]
    fn dc_only_dct_8x8_is_flat() {
        let mut dequant = [0i32; 64];
        dequant[0] = 256;
        let mut residual = [0i32; 64];
        inverse_transform_2d(
            &params(3, 3, false, dct(), dct(), 7, 11),
            &dequant,
            &mut residual,
        )
        .unwrap();
        let first = residual[0];
        assert_ne!(first, 0);
        assert!(
            residual.iter().all(|&v| v == first),
            "DC-only 8x8 block must be flat, got {residual:?}"
        );
    }

    #[test]
    fn mixed_row_dct_col_identity_confines_energy_to_row_zero() {
        const W: usize = 8;
        const H: usize = 8;
        let mut dequant = [0i32; W * H];
        dequant[0] = 256;
        let mut residual = [0i32; W * H];
        inverse_transform_2d(
            &params(3, 3, false, dct(), InverseTransform2dDim::Identity, 7, 0),
            &dequant,
            &mut residual,
        )
        .unwrap();
        assert!(
            residual[..W].iter().any(|&v| v != 0),
            "row 0 must hold energy"
        );
        for (idx, &v) in residual.iter().enumerate().skip(W) {
            assert_eq!(v, 0, "energy leaked below row 0 at index {idx}");
        }
    }

    #[test]
    fn orchestration_matches_manual_row_then_column_for_non_square_kernels() {
        const W: usize = 8;
        const H: usize = 16;
        const ROW_SHIFT: u8 = 6;
        const COL_SHIFT: u8 = 12;
        let bd = BitDepth::Eight;

        let mut dequant = [0i32; W * H];
        dequant[0] = 100;
        dequant[1] = 50;
        dequant[W] = -30;
        dequant[2 * W + 3] = 17;
        dequant[5 * W + 6] = -9;

        let mut got = [0i32; W * H];
        inverse_transform_2d(
            &params(3, 4, false, adst(), dct(), ROW_SHIFT, COL_SHIFT),
            &dequant,
            &mut got,
        )
        .unwrap();

        let mut intermediate = [0i32; W * H];
        for i in 0..H {
            let mut row_in = [0i32; W];
            for j in 0..W {
                row_in[j] = round2_2896(dequant[i * W + j]);
            }
            let mut row_out = [0i32; W];
            inverse_transform_1d(
                &row_in,
                InverseTransform1dType::Adst,
                ROW_SHIFT,
                false,
                bd,
                &mut row_out,
            )
            .unwrap();
            for j in 0..W {
                intermediate[i * W + j] = row_out[j];
            }
        }
        let mut expected = [0i32; W * H];
        for j in 0..W {
            let mut col_in = [0i32; H];
            for i in 0..H {
                col_in[i] = intermediate[i * W + j];
            }
            let mut col_out = [0i32; H];
            inverse_transform_1d(
                &col_in,
                InverseTransform1dType::Dct,
                COL_SHIFT,
                true,
                bd,
                &mut col_out,
            )
            .unwrap();
            for i in 0..H {
                expected[i * W + j] = col_out[i];
            }
        }

        assert_eq!(
            got, expected,
            "2D orchestration must match the manual row-then-column reference"
        );
        assert!(
            got.iter().any(|&v| v != 0),
            "reference produced a trivial all-zero block"
        );
    }

    #[test]
    fn sparse_blocks_match_manual_row_then_column_reference() {
        let bd = BitDepth::Eight;
        let mut lcg: u64 = 0xdead_beef_1234_5678;
        let mut next = move || {
            lcg = lcg.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            ((lcg >> 33) as i32 % 4_001) - 1_777
        };
        for (log2_w, log2_h, row_shift, col_shift) in [
            (2u32, 2u32, 7u8, 10u8),
            (3, 4, 6, 12),
            (5, 5, 6, 12),
            (3, 2, 6, 13),
        ] {
            let w = 1usize << log2_w;
            let h = 1usize << log2_h;
            let mut dequant = vec![0i32; w * h];
            dequant[0] = next();
            dequant[1] = next();
            if h > 2 {
                dequant[2 * w] = next();
            }
            let odd_ratio = log2_w.abs_diff(log2_h) % 2 == 1;

            let mut got = vec![0i32; w * h];
            inverse_transform_2d(
                &params(log2_w, log2_h, false, adst(), dct(), row_shift, col_shift),
                &dequant,
                &mut got,
            )
            .unwrap();

            let mut intermediate = vec![0i32; w * h];
            for i in 0..h {
                let row_in: Vec<i32> = (0..w)
                    .map(|j| {
                        let coeff = dequant[i * w + j];
                        if odd_ratio { round2_2896(coeff) } else { coeff }
                    })
                    .collect();
                let mut row_out = vec![0i32; w];
                inverse_transform_1d(
                    &row_in,
                    InverseTransform1dType::Adst,
                    row_shift,
                    false,
                    bd,
                    &mut row_out,
                )
                .unwrap();
                // splot-copy-ok: test reference stages 1-D rows into its own 2-D scratch
                intermediate[i * w..(i + 1) * w].copy_from_slice(&row_out);
            }
            let mut expected = vec![0i32; w * h];
            for j in 0..w {
                let col_in: Vec<i32> = (0..h).map(|i| intermediate[i * w + j]).collect();
                let mut col_out = vec![0i32; h];
                inverse_transform_1d(
                    &col_in,
                    InverseTransform1dType::Dct,
                    col_shift,
                    true,
                    bd,
                    &mut col_out,
                )
                .unwrap();
                for i in 0..h {
                    expected[i * w + j] = col_out[i];
                }
            }
            assert_eq!(got, expected, "shape {w}x{h}");
        }
    }

    /// The row quads and the column-pass lane groups must reproduce the
    /// per-row and per-column [`run_1d`] reference exactly, for every adjusted
    /// shape, every row kernel type, both column-transform families, every
    /// kernel type, both bit depths, whole zero rows, and a coefficient field
    /// spanning the § 7.14.4 dequant range at the widest bit depth, so the
    /// § 7.15.2.1 input clamp is live at 8 bits.
    #[test]
    fn lane_groups_match_the_per_row_and_column_reference() {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        let mut state = 0x1234_5678_9abc_def1u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for log2_w in 2..=5u32 {
            for log2_h in 2..=5u32 {
                let types = [
                    InverseTransform2dDim::Identity,
                    InverseTransform2dDim::Kernel(Dct),
                    InverseTransform2dDim::Kernel(Adst),
                    InverseTransform2dDim::Kernel(Fdst),
                    InverseTransform2dDim::Kernel(Ddtx),
                    InverseTransform2dDim::Kernel(Fddt),
                ];
                for (row_type, col_type) in types
                    .into_iter()
                    .flat_map(|row| types.map(|col| (row, col)))
                {
                    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
                        for col_shift in [0u8, 1, 4, 12, 31, 255] {
                            let (w, h) = (1usize << log2_w, 1usize << log2_h);
                            let mut dequant = vec![0i32; w * h];
                            let range = 1i32 << 17;
                            for (index, slot) in dequant.iter_mut().enumerate() {
                                *slot = match index % 11 {
                                    _ if (index / w) % 3 == 2 => 0,
                                    0 => 0,
                                    1 => range - 1,
                                    2 => -range,
                                    _ => (next() as i32 % range) >> (next() % 6) as i32,
                                };
                            }
                            let mut p =
                                params(log2_w, log2_h, false, row_type, col_type, 5, col_shift);
                            p.bit_depth = bit_depth;

                            let mut got = vec![0i32; w * h];
                            inverse_transform_2d(&p, &dequant, &mut got).unwrap();

                            let mut scratch = [0i32; MAX_DIM * MAX_DIM];
                            let mut expected = vec![0i32; w * h];
                            reference_transform_2d(&p, &dequant, &mut expected, &mut scratch);

                            assert_eq!(
                                got, expected,
                                "{w}x{h} {row_type:?} {col_type:?} {bit_depth:?} shift {col_shift}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The lane form must reproduce the per-row and per-column reference at
    /// both bit depths for every adjusted shape and pass pair, row and column
    /// shifts at both ends of `1..=20` and past them, and dense extreme, sparse,
    /// single-coefficient, DC-only and first-column inputs, so that the
    /// input clamp, the row `Clip3`, the zero-group and zero-row skips and the
    /// DC-only path all run. The coded-row prefix goes into stale output and
    /// scratch, so every sample must be written.
    #[test]
    fn lane_form_matches_the_reference_at_every_shift_and_sparsity() {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        let types = [
            InverseTransform2dDim::Identity,
            InverseTransform2dDim::Kernel(Dct),
            InverseTransform2dDim::Kernel(Adst),
            InverseTransform2dDim::Kernel(Fdst),
            InverseTransform2dDim::Kernel(Ddtx),
            InverseTransform2dDim::Kernel(Fddt),
        ];
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |range: u32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % u64::from(range)) as i32
        };
        for (log2_w, log2_h) in (2..=5u32).flat_map(|w| (2..=5u32).map(move |h| (w, h))) {
            let (w, h) = (1usize << log2_w, 1usize << log2_h);
            for (row_type, col_type) in types.into_iter().flat_map(|r| types.map(|c| (r, c))) {
                let shifts = [(1u8, 20u8), (20, 1), (5, 12), (12, 4), (0, 31), (40, 0)];
                let cases = shifts.into_iter().flat_map(|s| [(s, false), (s, true)]);
                for ((row_shift, col_shift), ten) in cases {
                    for pattern in 0..5 {
                        let mut dequant = vec![0i32; w * h];
                        let spot = next((w * h) as u32) as usize;
                        for (index, slot) in dequant.iter_mut().enumerate() {
                            *slot = match pattern {
                                0 => [(1 << 17) - 1, -(1 << 17), 32767, -32768][index % 4],
                                1 if next(9) == 0 => next(1 << 15) - (1 << 14),
                                2 if index == spot => next(1 << 16) - (1 << 15),
                                3 if index == 0 => next(1 << 16) - (1 << 15),
                                4 if index % w == 0 => next(1 << 12) - (1 << 11),
                                _ => 0,
                            };
                        }
                        let mut p = params(
                            log2_w, log2_h, false, row_type, col_type, row_shift, col_shift,
                        );
                        if ten {
                            p.bit_depth = BitDepth::Ten;
                        }
                        let rows = dequant
                            .iter()
                            .rposition(|&c| c != 0)
                            .map_or(0, |i| i / w + 1);
                        let mut got = vec![i32::MIN; w * h];
                        let mut scratch = [i32::MAX; MAX_DIM * MAX_DIM];
                        let coded = &dequant[..rows * w];
                        inverse_transform_2d_with_scratch(&p, coded, &mut got, &mut scratch)
                            .unwrap();
                        let mut expected = vec![0i32; w * h];
                        reference_transform_2d(&p, &dequant, &mut expected, &mut scratch);
                        assert_eq!(
                            got, expected,
                            "{w}x{h} {row_type:?} {col_type:?} shifts {row_shift}/{col_shift} pattern {pattern}"
                        );
                    }
                }
            }
        }
    }

    /// A whole-row prefix of the coefficients transforms like the block with
    /// its omitted rows zero, for every kernel pair at 8 bits.
    #[test]
    fn coded_row_prefix_matches_the_zero_padded_block() {
        use InverseTransform1dType::{Adst, Dct, Fddt};
        let mut state = 0x0bad_5eed_1234_5678u64;
        for (log2_w, log2_h) in [(2u32, 2u32), (2, 4), (3, 3), (4, 3), (4, 5), (5, 4), (5, 5)] {
            for (row, col) in [(Dct, Dct), (Adst, Dct), (Dct, Adst), (Fddt, Dct)] {
                let (w, h) = (1usize << log2_w, 1usize << log2_h);
                let p = params(
                    log2_w,
                    log2_h,
                    false,
                    InverseTransform2dDim::Kernel(row),
                    InverseTransform2dDim::Kernel(col),
                    6,
                    12,
                );
                for coded in 1..=h {
                    let mut full = vec![0i32; w * h];
                    for slot in &mut full[..coded * w] {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        *slot = (state % 4001) as i32 - 2000;
                    }
                    let mut expected = vec![0i32; w * h];
                    inverse_transform_2d(&p, &full, &mut expected).unwrap();
                    let mut got = vec![0i32; w * h];
                    inverse_transform_2d(&p, &full[..coded * w], &mut got).unwrap();
                    assert_eq!(got, expected, "{w}x{h} {row:?} {col:?} coded {coded}");
                }
            }
        }
    }

    /// A block whose only non-zero coefficient is the DC one matches the
    /// per-row and per-column reference for every 8-bit shape and type pair.
    #[test]
    fn dc_only_blocks_match_the_reference() {
        use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
        let types = [
            InverseTransform2dDim::Identity,
            InverseTransform2dDim::Kernel(Dct),
            InverseTransform2dDim::Kernel(Adst),
            InverseTransform2dDim::Kernel(Fdst),
            InverseTransform2dDim::Kernel(Ddtx),
            InverseTransform2dDim::Kernel(Fddt),
        ];
        for (log2_w, log2_h) in (2..=5u32).flat_map(|w| (2..=5u32).map(move |h| (w, h))) {
            let (w, h) = (1usize << log2_w, 1usize << log2_h);
            for (row, col) in types
                .into_iter()
                .flat_map(|row| types.map(|col| (row, col)))
            {
                for (dc, shifts) in [(40_000, (1, 20)), (-517, (6, 12)), (3, (3, 1))] {
                    let p = params(log2_w, log2_h, false, row, col, shifts.0, shifts.1);
                    let mut dequant = vec![0i32; w];
                    dequant[0] = dc;
                    let mut got = vec![0i32; w * h];
                    inverse_transform_2d(&p, &dequant, &mut got).unwrap();
                    let mut scratch = [0i32; MAX_DIM * MAX_DIM];
                    let mut expected = vec![0i32; w * h];
                    reference_transform_2d(&p, &dequant, &mut expected, &mut scratch);
                    assert_eq!(got, expected, "{w}x{h} {row:?} {col:?} dc {dc}");
                }
            }
        }
    }

    /// The pre-lane-group § 7.15.4.1 orchestration: a per-column gather, one
    /// [`run_1d`] call, then a scatter.
    fn reference_transform_2d(
        params: &InverseTransform2d,
        dequant: &[i32],
        residual: &mut [i32],
        intermediate: &mut [i32; MAX_DIM * MAX_DIM],
    ) {
        let (w, h) = (1usize << params.log2_width, 1usize << params.log2_height);
        let odd_ratio = params.log2_width.abs_diff(params.log2_height) % 2 == 1;
        let row_pass = Pass {
            lossless: params.lossless,
            dim: params.row_type,
            shift: params.row_shift,
            col_tx: false,
            log2_size: params.log2_width,
            bit_depth: params.bit_depth,
        };
        let mut buf_in = [0i32; MAX_DIM];
        let mut buf_out = [0i32; MAX_DIM];
        for (dequant_row, intermediate_row) in dequant
            .chunks_exact(w)
            .zip(intermediate.chunks_exact_mut(w))
            .take(h)
        {
            for (slot, &coeff) in buf_in.iter_mut().zip(dequant_row.iter()) {
                *slot = if odd_ratio { round2_2896(coeff) } else { coeff };
            }
            run_1d(&buf_in[..w], intermediate_row, row_pass).unwrap();
        }
        let col_pass = Pass {
            lossless: params.lossless,
            dim: params.col_type,
            shift: params.col_shift,
            col_tx: true,
            log2_size: params.log2_height,
            bit_depth: params.bit_depth,
        };
        for j in 0..w {
            for i in 0..h {
                buf_in[i] = intermediate[i * w + j];
            }
            run_1d(&buf_in[..h], &mut buf_out[..h], col_pass).unwrap();
            for i in 0..h {
                residual[i * w + j] = buf_out[i];
            }
        }
    }

    #[test]
    fn rejects_unsupported_shape() {
        let mut residual = [0i32; 16];
        assert!(matches!(
            inverse_transform_2d(
                &params(7, 2, false, dct(), dct(), 0, 0),
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
            inverse_transform_2d(
                &params(3, 2, true, dct(), dct(), 0, 0),
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
        let mut residual = [0i32; 16];
        assert!(matches!(
            inverse_transform_2d(
                &params(2, 2, false, dct(), dct(), 0, 0),
                &[0i32; 15],
                &mut residual
            ),
            Err(ReconError::InverseTransform2dBufferMismatch {
                expected: 16,
                dequant_len: 15,
                residual_len: 16
            })
        ));
    }
}
