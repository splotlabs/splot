// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 transform-block reconstruction: the residual chain composition.
//!
//! This module composes the three scheduler-free `splot-recon` residual
//! primitives into the AV2 transform-block reconstruction sequence a decoder (or
//! an encoder closed loop) runs to turn decoded quantized coefficients into
//! reconstructed samples:
//!
//! 1. § 7.14.4 dequantization
//!    ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//!    `#s-7-14-4`) via [`dequantize_block`],
//! 2. § 7.15.4 inverse transform (`#s-7-15-4`) via [`inverse_transform_2d_outer`], and
//! 3. § 7.14.3 reconstruction / residual addition with § 4.8 `Clip1` (`#s-7-14-3`)
//!    via [`reconstruct_add_residual`].
//!
//! Feature tracking: `RECON-RECONSTRUCT-TRANSFORM-BLOCK`.
//!
//! Scope: this is the residual-chain composition over caller-resolved transform
//! and dequantization parameters. It produces no coefficients (the caller decodes
//! `Quant[]`), reads no frame, segment, or tile state, performs no output
//! scheduling, and is not wired into the runtime decode path; a decoder consumes
//! it once its coefficient loop hands a transform block's `Quant[]` to recon.

use crate::dequant_process::{DequantBlockParams, dequantize_block};
use crate::inverse_transform_2d_outer::{InverseTransform2dOuter, inverse_transform_2d_outer};
use crate::reconstruct::reconstruct_add_residual;
use crate::{ReconSample, Result};

/// Reconstructs one transform block from its decoded quantized coefficients,
/// composing the AV2 § 7.14.4 → § 7.15.4 → § 7.14.3 residual chain over a
/// prediction: `out = Clip1(prediction + inverse_transform(dequant(quant)))`.
///
/// `quant` holds the adjusted `adjW * adjH` decoded quantized coefficients,
/// matching `dequant_params.tx_width * dequant_params.tx_height`; `prediction`
/// and `out` are the original `origW * origH` samples (the original size is
/// `1 << log2` per side of `transform`, the adjusted size `1 << Min(log2, 5)`).
/// `dequant_scratch` (`adjW * adjH`) and `residual_scratch` (`origW * origH`) are
/// caller-owned working buffers so the composition allocates nothing. The Clip1
/// bound is `transform.bit_depth`; resolve `transform` with
/// [`InverseTransform2dOuter::resolve`] so its shifts, per-pass types, and
/// dimensions are mutually consistent.
///
/// The composition is total and panic-free for consistent inputs: each step is
/// total, and every buffer-length or geometry inconsistency is rejected by the
/// underlying primitive before it mutates `out`.
///
/// # Errors
/// Propagates the typed [`crate::ReconError`] of the first failing step:
/// [`dequantize_block`] for a bad dequant shape or `quant` / `dequant_scratch`
/// length, [`inverse_transform_2d_outer`] for a bad transform shape or
/// `dequant_scratch` / `residual_scratch` length, and [`reconstruct_add_residual`]
/// for an unsupported sample type, a `prediction` / `residual_scratch` / `out`
/// length mismatch, or an out-of-range prediction sample.
pub fn reconstruct_transform_block_residual<T: ReconSample>(
    prediction: &[T],
    quant: &[i32],
    dequant_params: &DequantBlockParams,
    transform: &InverseTransform2dOuter,
    dequant_scratch: &mut [i32],
    residual_scratch: &mut [i32],
    out: &mut [T],
) -> Result<()> {
    dequantize_block(dequant_params, quant, dequant_scratch)?;
    inverse_transform_2d_outer(transform, dequant_scratch, residual_scratch)?;
    reconstruct_add_residual(prediction, residual_scratch, transform.bit_depth, out)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{BitDepth, PlaneId, QuantizerDeltas, ReconError, ac_quantizer, dc_quantizer};

    // DCT_DCT (PlaneTxType 0) luma dequant params for an 8-bit transform block of
    // the given adjusted dimensions, mirroring the encoder closed-loop reference
    // (`dq_denom == 1`, zero quantizer deltas).
    fn dct_dequant_params(tx_width: usize, tx_height: usize, qindex: u32) -> DequantBlockParams {
        let deltas = QuantizerDeltas {
            y_dc: 0,
            u_dc: 0,
            v_dc: 0,
            u_ac: 0,
            v_ac: 0,
        };
        DequantBlockParams {
            dc_quant: dc_quantizer(PlaneId::Y, qindex, deltas, BitDepth::Eight),
            ac_quant: ac_quantizer(PlaneId::Y, qindex, deltas, BitDepth::Eight),
            tx_width,
            tx_height,
            dq_denom: 1,
            bit_depth: BitDepth::Eight,
        }
    }

    fn dct_transform(log2: u32) -> InverseTransform2dOuter {
        InverseTransform2dOuter::resolve(0, log2, log2, false, false, BitDepth::Eight, None)
            .unwrap()
    }

    #[test]
    fn all_zero_quant_preserves_prediction_4x4() {
        // An all-zero quantized block dequantizes to zero, inverse-transforms to a
        // zero residual, and adds nothing: the reconstruction equals the
        // prediction exactly.
        let transform = dct_transform(2);
        let params = dct_dequant_params(4, 4, 100);
        let prediction: [u8; 16] = core::array::from_fn(|i| 120 + (i as u8 % 7));
        let (mut dq, mut res, mut out) = ([0i32; 16], [0i32; 16], [0u8; 16]);

        reconstruct_transform_block_residual(
            &prediction,
            &[0i32; 16],
            &params,
            &transform,
            &mut dq,
            &mut res,
            &mut out,
        )
        .unwrap();

        assert_eq!(out, prediction);
    }

    #[test]
    fn nonzero_dc_produces_uniform_signed_residual_4x4() {
        // A single nonzero DC coefficient inverse-transforms to a *flat* residual:
        // a positive DC raises every sample above the flat prediction and a
        // negative DC lowers every sample, with no per-sample variation.
        let transform = dct_transform(2);
        let params = dct_dequant_params(4, 4, 100);
        let prediction = [128u8; 16];
        let (mut dq, mut res) = ([0i32; 16], [0i32; 16]);

        let mut positive = [0u8; 16];
        let mut pos_quant = [0i32; 16];
        pos_quant[0] = 200;
        reconstruct_transform_block_residual(
            &prediction,
            &pos_quant,
            &params,
            &transform,
            &mut dq,
            &mut res,
            &mut positive,
        )
        .unwrap();
        assert!(
            positive.iter().all(|&s| s == positive[0]),
            "DC residual must be flat"
        );
        assert!(positive[0] > 128, "positive DC must raise the samples");

        let mut negative = [0u8; 16];
        let mut neg_quant = [0i32; 16];
        neg_quant[0] = -200;
        reconstruct_transform_block_residual(
            &prediction,
            &neg_quant,
            &params,
            &transform,
            &mut dq,
            &mut res,
            &mut negative,
        )
        .unwrap();
        assert!(
            negative.iter().all(|&s| s == negative[0]),
            "DC residual must be flat"
        );
        assert!(negative[0] < 128, "negative DC must lower the samples");
    }

    #[test]
    fn all_zero_quant_preserves_prediction_tx_64x64() {
        // TX_64X64: the adjusted 32x32 dequant block expands to a 64x64 residual.
        // An all-zero block reproduces the original-size prediction exactly,
        // exercising the adjusted-to-original sample-duplication geometry.
        let transform = dct_transform(6);
        let params = dct_dequant_params(32, 32, 100);
        let prediction: [u8; 64 * 64] = core::array::from_fn(|i| 100 + (i as u8 % 11));
        let mut dq = [0i32; 32 * 32];
        let mut res = [0i32; 64 * 64];
        let mut out = [0u8; 64 * 64];

        reconstruct_transform_block_residual(
            &prediction,
            &[0i32; 32 * 32],
            &params,
            &transform,
            &mut dq,
            &mut res,
            &mut out,
        )
        .unwrap();

        assert_eq!(out, prediction);
    }

    #[test]
    fn nonzero_dc_is_uniform_after_expansion_tx_64x64() {
        // A single DC coefficient on a TX_64X64 block yields a flat 64x64
        // reconstruction (uniform after the inverse transform and sample
        // duplication), strictly above a flat prediction for a positive DC.
        let transform = dct_transform(6);
        let params = dct_dequant_params(32, 32, 100);
        let prediction = [128u8; 64 * 64];
        let mut quant = [0i32; 32 * 32];
        quant[0] = 200;
        let mut dq = [0i32; 32 * 32];
        let mut res = [0i32; 64 * 64];
        let mut out = [0u8; 64 * 64];

        reconstruct_transform_block_residual(
            &prediction,
            &quant,
            &params,
            &transform,
            &mut dq,
            &mut res,
            &mut out,
        )
        .unwrap();

        assert!(
            out.iter().all(|&s| s == out[0]),
            "DC residual must be flat after expansion"
        );
        assert!(out[0] > 128, "positive DC must raise the samples");
    }

    #[test]
    fn rejects_inconsistent_buffers_before_writing_output() {
        // A dequant-scratch length that disagrees with the dequant params is
        // rejected by the first step; `out` is left untouched.
        let transform = dct_transform(2);
        let params = dct_dequant_params(4, 4, 100);
        let prediction = [50u8; 16];
        let mut dq = [0i32; 8]; // wrong: should be 16
        let mut res = [0i32; 16];
        let mut out = [7u8; 16];

        assert!(matches!(
            reconstruct_transform_block_residual(
                &prediction,
                &[0i32; 16],
                &params,
                &transform,
                &mut dq,
                &mut res,
                &mut out,
            ),
            Err(ReconError::DequantBlockLengthMismatch { .. })
        ));
        assert_eq!(out, [7u8; 16], "output is untouched on a rejected input");
    }
}
