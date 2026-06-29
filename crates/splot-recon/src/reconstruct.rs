// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.14.3 reconstruct residual-addition step.
//!
//! This module implements the scheduler-free final step of the AV2 § 7.14.3
//! reconstruct process
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-14-3`): adding the inverse-transform residual to the predicted samples
//! and clamping with § 4.8 `Clip1`,
//! `CurrFrame[plane][y + i][x + j] = Clip1(CurrFrame[plane][y + i][x + j] + Residual[i][j])`.
//!
//! Feature tracking: `RECON-RESIDUAL-ADDITION`.
//!
//! Scope: this is the residual-addition step only and is independent of how the
//! residual is produced. The § 7.14.3 secondary-transform invocation, the
//! § 7.15.4 2D inverse transform that fills the `Residual` array, the § 7.14.4
//! dequantization process, the DPCM adjustment, and the lossless-conformance
//! requirement are out of scope and tracked by their own future rows. The caller
//! supplies the prediction samples and the residual.

use crate::intra_dc_math::validate_sample_type;
use crate::{BitDepth, ReconError, ReconSample, Result};

/// Applies the AV2 § 7.14.3 residual-addition step to a block of predicted
/// samples, writing `out`.
///
/// Each output is `Clip1(prediction[i] + residual[i])`, where § 4.8
/// `Clip1(x) = Clip3(0, (1 << BitDepth) - 1, x)` clamps to the active decoded bit
/// depth. `prediction` holds the intra/inter predicted samples (the values
/// § 7.14.3 reads from `CurrFrame` before adding the residual) and `residual`
/// holds the signed inverse-transform output.
///
/// The sum uses `i64` intermediates, so it is total and never panics; because
/// `Clip1` bounds the result to `0..=max_sample` for the active bit depth (and
/// the sample type is validated to represent that depth), every written value
/// fits the storage type.
///
/// Prediction samples are validated up front against the active bit depth — like
/// the other `splot-recon` caller-buffer primitives — and a wider storage type
/// holding an out-of-range value (e.g. a `u16` above 255 at 8-bit) is rejected
/// rather than silently folded by `Clip1`. § 7.14.3 reads in-range prediction
/// samples from `CurrFrame`, so a conformant caller never trips this check.
///
/// # Errors
/// Returns [`ReconError::SampleTypeUnsupportedBitDepth`] if `T` cannot represent
/// `bit_depth`, [`ReconError::ReconstructLengthMismatch`] if `prediction`,
/// `residual`, and `out` do not all have the same length, and
/// [`ReconError::ReconstructPredictionOutOfRange`] if a prediction sample exceeds
/// the active bit depth. All inputs are validated before any output is written.
pub fn reconstruct_add_residual<T: ReconSample>(
    prediction: &[T],
    residual: &[i32],
    bit_depth: BitDepth,
    out: &mut [T],
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    if prediction.len() != residual.len() || out.len() != prediction.len() {
        return Err(ReconError::ReconstructLengthMismatch {
            prediction_len: prediction.len(),
            residual_len: residual.len(),
            out_len: out.len(),
        });
    }

    let max_sample = bit_depth.max_sample();
    for (sample_index, &pred) in prediction.iter().enumerate() {
        let value = pred.to_u16();
        if value > max_sample {
            return Err(ReconError::ReconstructPredictionOutOfRange {
                sample_index,
                value,
                max: max_sample,
            });
        }
    }

    let max = i64::from(max_sample);
    for ((slot, &pred), &res) in out.iter_mut().zip(prediction).zip(residual) {
        let reconstructed = (i64::from(pred.to_u16()) + i64::from(res)).clamp(0, max);
        *slot = T::try_from_u16(reconstructed as u16)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn adds_residual_to_prediction() {
        let mut out = [0u8; 4];
        reconstruct_add_residual(
            &[100u8, 100, 100, 100],
            &[10, -10, 0, 5],
            BitDepth::Eight,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [110, 90, 100, 105]);
    }

    #[test]
    fn clip1_clamps_below_zero_and_above_max() {
        let mut out = [0u8; 3];
        reconstruct_add_residual(&[10u8, 250, 0], &[-50, 50, 255], BitDepth::Eight, &mut out)
            .unwrap();
        assert_eq!(out, [0, 255, 255]);
    }

    #[test]
    fn ten_bit_uses_u16_and_clamps_to_1023() {
        let mut out = [0u16; 2];
        reconstruct_add_residual(&[1000u16, 5], &[100, -50], BitDepth::Ten, &mut out).unwrap();
        assert_eq!(out, [1023, 0]);
    }

    #[test]
    fn is_total_for_extreme_residual() {
        let mut out = [0u8; 2];
        reconstruct_add_residual(
            &[128u8, 128],
            &[i32::MAX, i32::MIN],
            BitDepth::Eight,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [255, 0]);
    }

    #[test]
    fn rejects_sample_type_too_narrow_for_bit_depth() {
        let mut out = [0u8; 1];
        assert!(matches!(
            reconstruct_add_residual(&[0u8], &[0], BitDepth::Ten, &mut out),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn rejects_length_mismatch() {
        let mut out = [0u8; 3];
        assert!(matches!(
            reconstruct_add_residual(&[0u8; 4], &[0; 4], BitDepth::Eight, &mut out),
            Err(ReconError::ReconstructLengthMismatch {
                prediction_len: 4,
                residual_len: 4,
                out_len: 3
            })
        ));
    }

    #[test]
    fn rejects_prediction_sample_above_bit_depth() {
        let mut out = [0u16; 2];
        assert!(matches!(
            reconstruct_add_residual(&[10u16, 300], &[0, -100], BitDepth::Eight, &mut out),
            Err(ReconError::ReconstructPredictionOutOfRange {
                sample_index: 1,
                value: 300,
                max: 255
            })
        ));
        assert_eq!(out, [0, 0]);
    }
}
