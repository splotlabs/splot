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
//!
//! The § 7.15.4 outer process — the `Adjusted_Tx_Size` lookup itself, the
//! `Transform_Shift` / `get_transform_1d_type` derivations, the `Lossless &&
//! IDTX` bit-shift shortcut, the DPCM cumulative sum, and the adjusted-size
//! sample duplication — is out of scope and tracked by its own future row, as
//! are dequantization, the secondary transform, and residual addition.

use crate::inverse_transform::{
    InverseTransform1dType, inverse_identity_transform, inverse_transform_1d,
    inverse_walsh_hadamard,
};
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
/// `1 << Min(log2_height, 5)`.
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
/// stack arrays sized to the 32-element maximum adjusted dimension.
///
/// # Errors
/// Returns [`ReconError::InvalidInverseTransform2dShape`] if `log2_width` /
/// `log2_height` are not each in `2..=6` (or not both `2` when lossless), and
/// [`ReconError::InverseTransform2dBufferMismatch`] if `dequant` or `residual`
/// is not exactly `w * h` long.
pub fn inverse_transform_2d(
    params: &InverseTransform2d,
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

    // Adjusted operating dimensions: `Adjusted_Tx_Size` caps each side's log2 at
    // 5 (§ 7.15.4 / the conversion table; `adjLog2 = Min(log2, 5)`), so `w`/`h`
    // are each in 4..=32.
    let w = 1usize << log2_w.min(5);
    let h = 1usize << log2_h.min(5);
    let expected = w * h;
    if dequant.len() != expected || residual.len() != expected {
        return Err(ReconError::InverseTransform2dBufferMismatch {
            expected,
            dequant_len: dequant.len(),
            residual_len: residual.len(),
        });
    }

    // The √2 rescale parity is taken from the *original* log2 dimensions, not the
    // adjusted ones (§ 7.15.4.1 line `If Abs( log2W - log2H ) is odd`).
    let odd_ratio = log2_w.abs_diff(log2_h) % 2 == 1;

    // `intermediate[i * w + j]` holds the row-transformed coefficients.
    let mut intermediate = [0i32; MAX_DIM * MAX_DIM];
    let mut buf_in = [0i32; MAX_DIM];
    let mut buf_out = [0i32; MAX_DIM];

    // Row pass: transform each row into `intermediate`. The identity scale uses
    // the original row log2 dimension.
    let row_pass = Pass {
        lossless: params.lossless,
        dim: params.row_type,
        shift: params.row_shift,
        col_tx: false,
        log2_size: log2_w,
        bit_depth: params.bit_depth,
    };
    for i in 0..h {
        for j in 0..w {
            let coeff = dequant[i * w + j];
            buf_in[j] = if odd_ratio { round2_2896(coeff) } else { coeff };
        }
        // Write the row transform straight into the intermediate row (no scratch
        // round-trip), then the column pass reads it back column by column.
        run_1d(&buf_in[..w], &mut intermediate[i * w..i * w + w], row_pass)?;
    }

    // Column pass: transform each intermediate column into `residual`. The
    // identity scale uses the original column log2 dimension.
    let col_pass = Pass {
        lossless: params.lossless,
        dim: params.col_type,
        shift: params.col_shift,
        col_tx: true,
        log2_size: log2_h,
        bit_depth: params.bit_depth,
    };
    for j in 0..w {
        for i in 0..h {
            buf_in[i] = intermediate[i * w + j];
        }
        run_1d(&buf_in[..h], &mut buf_out[..h], col_pass)?;
        for i in 0..h {
            residual[i * w + j] = buf_out[i];
        }
    }
    Ok(())
}

/// Resolved parameters for one § 7.15.4.1 1D pass.
#[derive(Clone, Copy)]
struct Pass {
    lossless: bool,
    dim: InverseTransform2dDim,
    shift: u8,
    col_tx: bool,
    log2_size: u32,
    bit_depth: BitDepth,
}

/// Runs one § 7.15.4.1 1D pass over `src`, writing `out` (same length).
fn run_1d(src: &[i32], out: &mut [i32], pass: Pass) -> Result<()> {
    if pass.lossless {
        // Lossless implies a 4x4 block, so each pass is a 4-element
        // Walsh-Hadamard transform (§ 7.15.4.1: row shift 3, column shift 0).
        let walsh_shift = if pass.col_tx { 0 } else { 3 };
        let input = [src[0], src[1], src[2], src[3]];
        for (dst, value) in out
            .iter_mut()
            .zip(inverse_walsh_hadamard(input, walsh_shift))
        {
            *dst = value;
        }
        Ok(())
    } else {
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

/// AV2 § 7.15.4.1 rectangular rescale `Round2( x * 2896, 12 )` (the √2 factor for
/// transforms whose width/height log2 ratio is odd). Computed in `i64` so the
/// multiply cannot overflow for in-range dequantized coefficients.
fn round2_2896(x: i32) -> i32 {
    ((i64::from(x) * 2896 + (1 << 11)) >> 12) as i32
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
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
        // A DC-only DCT block reconstructs a flat field. With DC = 128 and the
        // 4x4 Transform_Shift (row 7, col 10): row DCT -> 64 everywhere, col DCT
        // -> Round2(64*64, 10) = 4 everywhere.
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
        // Lossless DC-only: row Walsh-Hadamard (shift 3) of [64,0,0,0] ->
        // [4,4,4,4]; column Walsh-Hadamard (shift 0) of [4,0,0,0] -> [2,2,2,2];
        // so the block is flat 2.
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

    #[test]
    fn identity_2d_preserves_coefficient_position() {
        // The identity transform is diagonal, so a single coefficient at (1, 1)
        // — index 5 in a 4-wide block — stays at (1, 1) and spreads nowhere else.
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
        // 4096 * 2896 / 4096 = 2896 (the √2 ~ 2896/2048 factor at 12 bits).
        assert_eq!(round2_2896(4096), 2896);
        assert_eq!(round2_2896(0), 0);
        // Panic-free for the dequant magnitude extreme (~2^18).
        assert!(round2_2896(262_144) > 0);
    }

    #[test]
    fn rectangular_4x8_applies_rescale_path() {
        // A 4x8 block (log2 2x3) has odd |log2W - log2H|, so the row inputs are
        // √2-rescaled. A DC-only block stays flat; the test confirms the
        // rectangular path runs end-to-end and produces a uniform field.
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
        // Regression for the original-vs-adjusted parity blocker: TX_64X32 has
        // original log2 (6, 5) -> |6 - 5| = 1 (odd) so the √2 rescale MUST fire,
        // even though both adjusted dimensions are 32 (adjusted parity would be
        // even). Operating size is 32x32 either way.
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

        // Equivalent 32x32 (log2 5,5; even parity, no rescale) fed the manually
        // pre-rescaled DC: identical, proving (6,5) applied Round2(x*2896,12).
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

        // And the same 32x32 fed the raw DC (no rescale) must differ, proving the
        // rescale actually fired rather than both paths skipping it.
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
        // Larger square (log2 3x3): a DC-only DCT block is still a flat field.
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
        // Row DCT spreads a DC-only input across row 0 (and leaves rows 1..h at
        // zero); the column identity transform then keeps every column's single
        // non-zero sample at i = 0. So energy stays in row 0 only.
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
        // Pins the row-then-column wiring (intermediate layout, the per-pass
        // transpose, and the per-pass type/shift) for a non-square, non-DCT,
        // rescaled case that symmetric flat-field tests cannot catch. An 8x16
        // block (log2 3x4, odd ratio -> the √2 rescale fires) with an ADST row
        // pass and a DCT column pass, against an asymmetric coefficient pattern,
        // must equal a manual reference built from the same trusted 1D primitive.
        const W: usize = 8;
        const H: usize = 16;
        const ROW_SHIFT: u8 = 6;
        const COL_SHIFT: u8 = 12;
        let bd = BitDepth::Eight;

        let mut dequant = [0i32; W * H];
        // Asymmetric, off-diagonal coefficients so a transpose/index swap shows up.
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

        // Manual reference: row ADST (with the odd-ratio √2 rescale) into an
        // intermediate, then column DCT, both via inverse_transform_1d.
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
        // Guard against a vacuous all-zero comparison.
        assert!(
            got.iter().any(|&v| v != 0),
            "reference produced a trivial all-zero block"
        );
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
