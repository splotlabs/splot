// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Scalar subsampled DC intra prediction primitive.
//!
//! Feature tracking: `RECON-INTRA-DC-SUBSAMPLED-PREDICTION`.

use crate::intra::{IntraDcEdge, IntraDcEdges, IntraRectBlockSize};
use crate::intra_dc_math::{
    DcEdgeSum, approx_divide, clip1, dc_midpoint, fill_validated_output_shape,
    validate_dc_edge_sampled_sum, validate_output_shape, validate_sample_type,
};
use crate::{BitDepth, ReconError, ReconSample, Result};

/// Computes the constant sample value for AV2 §7.13.2.11 subsampled DC prediction.
///
/// The caller provides prepared `LeftCol[0..h]` and `AboveRow[0..w]` samples.
/// Edges are fully validated against the requested bit depth, then sampled with
/// the AV2 subsampled-DC step: every sample for dimensions up to 32 and every
/// second sample for dimensions greater than 32. Any nonzero sampled count uses
/// the §7.13.2.11 / §7.13.3.22 `Clip1(approx_divide(sum, count))` path.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples, arithmetic overflow, or
/// storage conversion failure.
pub fn predict_intra_dc_subsampled_rect_value<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDcEdges<'_, T>,
) -> Result<T> {
    validate_sample_type::<T>(bit_depth)?;
    let left = validate_dc_edge_sampled_sum(
        IntraDcEdge::Left,
        edges.left_samples(),
        size.height(),
        subsampled_step(size.height()),
        bit_depth,
    )?;
    let above = validate_dc_edge_sampled_sum(
        IntraDcEdge::Above,
        edges.above_samples(),
        size.width(),
        subsampled_step(size.width()),
        bit_depth,
    )?;

    predict_intra_dc_subsampled_rect_value_from_sums(bit_depth, left, above)
}

pub(crate) fn predict_intra_dc_subsampled_rect_value_from_sums<T: ReconSample>(
    bit_depth: BitDepth,
    left: Option<DcEdgeSum>,
    above: Option<DcEdgeSum>,
) -> Result<T> {
    validate_sample_type::<T>(bit_depth)?;
    let (sum, count) = sampled_sum_and_count(left, above)?;
    let predicted = if count == 0 {
        dc_midpoint(bit_depth)
    } else {
        clip1(approx_divide(sum, count)?, bit_depth)
    };

    T::try_from_u16(predicted)
}

/// Writes AV2 §7.13.2.11 subsampled DC prediction into caller-owned storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for invalid prediction inputs, a too-small stride, a
/// too-small output buffer, arithmetic overflow, or storage conversion failure.
pub fn predict_intra_dc_subsampled_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDcEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let required = validate_output_shape(size, output.len(), stride_samples)?;
    let sample = predict_intra_dc_subsampled_rect_value(bit_depth, size, edges)?;

    fill_validated_output_shape(size, output, stride_samples, required, sample);
    Ok(())
}

pub(crate) const fn subsampled_step(dimension: usize) -> usize {
    if dimension > 32 { 2 } else { 1 }
}

fn sampled_sum_and_count(left: Option<DcEdgeSum>, above: Option<DcEdgeSum>) -> Result<(u64, u64)> {
    let mut sum = 0u64;
    let mut count = 0u64;
    for edge in [left, above].into_iter().flatten() {
        sum = sum
            .checked_add(edge.sum)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "subsampled intra DC sample sum",
            })?;
        count = count
            .checked_add(edge.count)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "subsampled intra DC sample count",
            })?;
    }
    Ok((sum, count))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    #[test]
    fn intra_dc_subsampled_no_edges_use_midpoint() {
        let size = rect_size(2, 3);
        let eight_bit = predict_intra_dc_subsampled_rect_value::<u8>(
            BitDepth::Eight,
            size,
            IntraDcEdges::none(),
        )
        .unwrap();
        let ten_bit = predict_intra_dc_subsampled_rect_value::<u16>(
            BitDepth::Ten,
            size,
            IntraDcEdges::none(),
        )
        .unwrap();

        assert_eq!(eight_bit, 128);
        assert_eq!(ten_bit, 512);
    }

    #[test]
    fn intra_dc_subsampled_large_edges_use_every_second_sample() {
        let size = rect_size(6, 6);
        let mut left = [200u8; 64];
        let mut above = [200u8; 64];
        for index in (0..64).step_by(2) {
            left[index] = 10;
            above[index] = 30;
        }

        let sample = predict_intra_dc_subsampled_rect_value(
            BitDepth::Eight,
            size,
            IntraDcEdges::both(&left, &above),
        )
        .unwrap();

        assert_eq!(sample, 20);
    }

    #[test]
    fn intra_dc_subsampled_validates_skipped_large_edge_samples() {
        let size = rect_size(2, 6);
        let mut left = [10u16; 64];
        left[1] = 300;

        assert!(matches!(
            predict_intra_dc_subsampled_rect_value(
                BitDepth::Eight,
                size,
                IntraDcEdges::left(&left)
            ),
            Err(ReconError::IntraPredictionSampleOutOfRange {
                edge: IntraDcEdge::Left,
                sample_index: 1,
                value: 300,
                max: 255
            })
        ));
    }

    #[test]
    fn intra_dc_subsampled_uses_approximate_divide_for_nonzero_count() {
        let left = [1u8, 1, 1, 1, 1, 1, 1, 0];
        let above = [0u8, 0, 0, 0];

        let sample = predict_intra_dc_subsampled_rect_value(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraDcEdges::both(&left, &above),
        )
        .unwrap();

        let truncating_integer_division = 7u8 / 12;
        assert_eq!(sample, 1);
        assert_ne!(sample, truncating_integer_division);
    }

    #[test]
    fn intra_dc_subsampled_accepts_10_bit_edge_range() {
        let left = [1023u16; 4];

        let sample = predict_intra_dc_subsampled_rect_value(
            BitDepth::Ten,
            rect_size(2, 2),
            IntraDcEdges::left(&left),
        )
        .unwrap();

        assert_eq!(sample, 1023);
    }

    #[test]
    fn intra_dc_subsampled_rejects_unsupported_sample_type() {
        let left = [1u8; 4];

        assert!(matches!(
            predict_intra_dc_subsampled_rect_value(
                BitDepth::Ten,
                rect_size(2, 2),
                IntraDcEdges::left(&left)
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn intra_dc_subsampled_rejects_edge_length_mismatch() {
        let left = [1u8, 2, 3];

        assert!(matches!(
            predict_intra_dc_subsampled_rect_value(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraDcEdges::left(&left)
            ),
            Err(ReconError::IntraPredictionEdgeLengthMismatch {
                edge: IntraDcEdge::Left,
                expected: 4,
                actual: 3
            })
        ));
    }

    #[test]
    fn intra_dc_subsampled_rejects_edge_samples_out_of_range() {
        let above = [1u16, 2, 256, 4];

        assert!(matches!(
            predict_intra_dc_subsampled_rect_value(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraDcEdges::above(&above)
            ),
            Err(ReconError::IntraPredictionSampleOutOfRange {
                edge: IntraDcEdge::Above,
                sample_index: 2,
                value: 256,
                max: 255
            })
        ));
    }

    #[test]
    fn intra_dc_subsampled_into_fills_rectangle_with_stride() {
        let left = [10u8, 10, 10, 10];
        let mut output = [99u8; 24];

        predict_intra_dc_subsampled_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDcEdges::left(&left),
            &mut output,
            6,
        )
        .unwrap();

        for row in 0..4 {
            let start = row * 6;
            assert_eq!(&output[start..start + 6], &[10, 10, 10, 10, 99, 99]);
        }
    }

    #[test]
    fn intra_dc_subsampled_into_validates_output_stride_and_length() {
        let left = [1u8, 2, 3, 4];
        let mut output = [0u8; 15];

        assert!(matches!(
            predict_intra_dc_subsampled_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraDcEdges::left(&left),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));

        assert!(matches!(
            predict_intra_dc_subsampled_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraDcEdges::left(&left),
                &mut output,
                4
            ),
            Err(ReconError::IntraPredictionOutputTooSmall {
                expected: 16,
                actual: 15
            })
        ));
    }
}
