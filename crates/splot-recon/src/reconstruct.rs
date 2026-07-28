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

use std::simd::num::{SimdInt as _, SimdUint as _};
use std::simd::{Simd, cmp::SimdOrd as _};

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
/// The sum uses saturating `i32` addition, so it is total and never panics; because
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
    validate_prediction_range(prediction, max_sample)?;
    add_residual_row(prediction, residual, i32::from(max_sample), out)
}

/// Applies the AV2 § 7.14.3 residual-addition step of one contiguous prediction
/// block directly to strided destination rows.
///
/// `out` starts at the destination rectangle's first sample and spans through
/// its final row, advancing `out_stride` samples per row; `prediction` and
/// `residual` hold `width * height` samples in block raster order. Callers own
/// the `out_stride >= width` geometry invariant, which the current-frame
/// rectangle view establishes before building `out`.
///
/// This is [`reconstruct_add_residual`] without the block staging buffer: it
/// writes `Clip1(prediction + residual)` where the copy-based path would first
/// fill a block, range-scan it, and copy it row by row into the destination.
/// Every input is validated before the first destination sample changes, and
/// `Clip1` bounds each written value to the active bit depth, so no destination
/// write can fail once those checks pass.
///
/// # Errors
/// Returns [`ReconError::SampleTypeUnsupportedBitDepth`] if `T` cannot represent
/// `bit_depth`, [`ReconError::ZeroDimension`] for an empty rectangle,
/// [`ReconError::ReconstructLengthMismatch`] if `prediction` or `residual` is
/// not `width * height` samples, [`ReconError::ArithmeticOverflow`] or
/// [`ReconError::BufferLengthMismatch`] if the destination rows do not fit
/// `out`, and [`ReconError::ReconstructPredictionOutOfRange`] if a prediction
/// sample exceeds the active bit depth.
pub(crate) fn add_block_residual_into_rows<T: ReconSample>(
    prediction: &[T],
    residual: &[i32],
    bit_depth: BitDepth,
    out: &mut [T],
    out_stride: usize,
    width: usize,
    height: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    if width == 0 || height == 0 {
        return Err(ReconError::ZeroDimension {
            field: "residual destination rectangle",
        });
    }
    debug_assert!(out_stride >= width);
    let samples = width
        .checked_mul(height)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "residual destination sample count",
        })?;
    if prediction.len() != samples || residual.len() != samples {
        return Err(ReconError::ReconstructLengthMismatch {
            prediction_len: prediction.len(),
            residual_len: residual.len(),
            out_len: samples,
        });
    }
    let span = (height - 1)
        .checked_mul(out_stride)
        .and_then(|offset| offset.checked_add(width))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "residual destination row span",
        })?;
    if out.len() < span {
        return Err(ReconError::BufferLengthMismatch {
            expected: span,
            actual: out.len(),
        });
    }

    let max_sample = bit_depth.max_sample();
    validate_prediction_range(prediction, max_sample)?;
    let max = i32::from(max_sample);
    for row in 0..height {
        let source = row * width;
        let target = row * out_stride;
        add_residual_row(
            &prediction[source..source + width],
            &residual[source..source + width],
            max,
            &mut out[target..target + width],
        )?;
    }
    Ok(())
}

/// Rejects a prediction sample the active bit depth cannot represent, scanning
/// `u16` storage a lane group at a time.
fn validate_prediction_range<T: ReconSample>(prediction: &[T], max_sample: u16) -> Result<()> {
    if let Some(samples) = T::u16_slice(prediction)
        && !crate::workspace::u16_samples_exceed(samples, max_sample)
    {
        return Ok(());
    }
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
    Ok(())
}

/// Writes `Clip1(prediction + residual)` over one destination row.
fn add_residual_row<T: ReconSample>(
    prediction: &[T],
    residual: &[i32],
    max: i32,
    out: &mut [T],
) -> Result<()> {
    if let Some(prediction) = T::u16_slice(prediction)
        && let Some(out) = T::u16_slice_mut(out)
    {
        add_residual_u16(prediction, residual, max, out);
        return Ok(());
    }
    for ((slot, &pred), &res) in out.iter_mut().zip(prediction).zip(residual) {
        let reconstructed = i32::from(pred.to_u16()).saturating_add(res).clamp(0, max);
        *slot = T::try_from_u16(reconstructed as u16)?;
    }
    Ok(())
}

/// Adds the § 7.14.3 residual over `u16` storage, widest lane group first.
///
/// Each lane repeats the scalar `Clip1(prediction + residual)` step with the
/// same saturating `i32` addition, so every output sample is bit-identical;
/// `Clip1` bounds the result to `0..=max`, which the `u16` storage represents
/// exactly.
fn add_residual_u16(prediction: &[u16], residual: &[i32], max: i32, out: &mut [u16]) {
    let len = out.len();
    let mut index = 0usize;
    macro_rules! add_lane_group {
        ($lanes:literal) => {
            while index + $lanes <= len {
                let pred = Simd::<u16, $lanes>::from_slice(&prediction[index..]).cast::<i32>();
                let res = Simd::<i32, $lanes>::from_slice(&residual[index..]);
                let values = pred
                    .saturating_add(res)
                    .simd_clamp(Simd::splat(0), Simd::splat(max))
                    .cast::<u16>()
                    .to_array();
                out[index..index + $lanes].copy_from_slice(&values); // splot-copy-ok: publish a § 7.14.3 residual-addition lane group
                index += $lanes;
            }
        };
    }
    add_lane_group!(16);
    add_lane_group!(8);
    add_lane_group!(4);
    for ((slot, &pred), &res) in out[index..]
        .iter_mut()
        .zip(&prediction[index..])
        .zip(&residual[index..])
    {
        *slot = i32::from(pred).saturating_add(res).clamp(0, max) as u16;
    }
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

    /// The `u16` lane groups must reproduce the scalar `Clip1` step exactly for
    /// every block length, over randomized predictions and residuals including
    /// the `i32` extremes.
    #[test]
    fn u16_lane_groups_match_the_scalar_reference() {
        let mut state = 0x1234_5678_9abc_def1u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for len in 1..=64usize {
            for &bit_depth in &[BitDepth::Eight, BitDepth::Ten] {
                let max = i32::from(bit_depth.max_sample());
                let prediction: Vec<u16> = (0..len)
                    .map(|_| (next() % (max as u64 + 1)) as u16)
                    .collect();
                let residual: Vec<i32> = (0..len)
                    .map(|index| match index % 8 {
                        0 => i32::MAX,
                        1 => i32::MIN,
                        _ => (next() as i32) >> (next() % 20) as i32,
                    })
                    .collect();
                let expected: Vec<u16> = prediction
                    .iter()
                    .zip(&residual)
                    .map(|(&pred, &res)| i32::from(pred).saturating_add(res).clamp(0, max) as u16)
                    .collect();
                let mut actual = vec![0u16; len];
                add_residual_u16(&prediction, &residual, max, &mut actual);
                assert_eq!(actual, expected, "len {len} {bit_depth:?}");
            }
        }
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
