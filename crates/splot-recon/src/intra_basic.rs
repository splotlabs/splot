// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Basic/PAETH intra prediction primitive.
//!
//! Feature tracking: `RECON-INTRA-BASIC-PAETH-PREDICTION`.

use crate::intra_dc_math::{
    IntraEdgeError, validate_intra_edge_samples, validate_output_shape, validate_sample_type,
};
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

/// Edge identifier for PAETH intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraPaethEdge {
    /// Left edge samples.
    Left,
    /// Above edge samples.
    Above,
    /// Top-left corner sample.
    TopLeft,
}

impl IntraPaethEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
            Self::TopLeft => "top-left",
        }
    }
}

impl IntraEdgeError for IntraPaethEdge {
    fn length_mismatch(self, expected: usize, actual: usize) -> ReconError {
        ReconError::IntraPaethEdgeLengthMismatch {
            edge: self,
            expected,
            actual,
        }
    }

    fn sample_out_of_range(self, sample_index: usize, value: u16, max: u16) -> ReconError {
        ReconError::IntraPaethSampleOutOfRange {
            edge: self,
            sample_index,
            value,
            max,
        }
    }
}

/// Caller-provided prepared edge samples for AV2 §7.13.2.2 basic prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraPaethEdges<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
    top_left: T,
}

impl<'a, T: ReconSample> IntraPaethEdges<'a, T> {
    /// Creates a prepared PAETH edge set.
    ///
    /// `top_left` is the sample identified by AV2 §7.13.2.2 as
    /// `AboveRow[-1]`. Availability and fallback preparation remain outside
    /// this type and are owned by the broader §7.13.2.1 intra process.
    pub const fn new(left: &'a [T], above: &'a [T], top_left: T) -> Self {
        Self {
            left,
            above,
            top_left,
        }
    }

    /// Returns prepared left edge samples.
    pub const fn left_samples(self) -> &'a [T] {
        self.left
    }

    /// Returns prepared above edge samples.
    pub const fn above_samples(self) -> &'a [T] {
        self.above
    }
}

/// Writes rectangular AV2 §7.13.2.2 basic/PAETH prediction into caller storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples, a too-small stride, or a
/// too-small output buffer.
pub fn predict_intra_paeth_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraPaethEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_intra_edge_samples(IntraPaethEdge::Left, edges.left, size.height(), bit_depth)?;
    validate_intra_edge_samples(IntraPaethEdge::Above, edges.above, size.width(), bit_depth)?;
    validate_intra_edge_samples(
        IntraPaethEdge::TopLeft,
        core::slice::from_ref(&edges.top_left),
        1,
        bit_depth,
    )?;
    validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "PAETH intra prediction output buffer length",
    )?;

    for row_index in 0..size.height() {
        let row_start =
            row_index
                .checked_mul(stride_samples)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PAETH intra prediction output row offset",
                })?;
        let left = edges.left[row_index];
        for column in 0..size.width() {
            output[row_start + column] =
                predict_paeth_sample(left, edges.above[column], edges.top_left);
        }
    }

    Ok(())
}

pub(crate) fn predict_paeth_sample<T: ReconSample>(left: T, above: T, top_left: T) -> T {
    let left_value = i32::from(left.to_u16());
    let above_value = i32::from(above.to_u16());
    let top_left_value = i32::from(top_left.to_u16());
    let base = above_value + left_value - top_left_value;

    let p_left = base.abs_diff(left_value);
    let p_top = base.abs_diff(above_value);
    let p_top_left = base.abs_diff(top_left_value);

    if p_left <= p_top && p_left <= p_top_left {
        left
    } else if p_top <= p_top_left {
        above
    } else {
        top_left
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn assert_paeth_fills(left: [u8; 8], above: [u8; 4], top_left: u8, expected: u8) {
        let mut output = [0u8; 32];

        predict_intra_paeth_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraPaethEdges::new(&left, &above, top_left),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(output, [expected; 32]);
    }

    #[test]
    fn paeth_prediction_selects_left_candidate() {
        assert_paeth_fills([30; 8], [12; 4], 10, 30);
    }

    #[test]
    fn paeth_prediction_selects_above_candidate() {
        assert_paeth_fills([12; 8], [30; 4], 10, 30);
    }

    #[test]
    fn paeth_prediction_selects_top_left_candidate() {
        assert_paeth_fills([0; 8], [20; 4], 10, 10);
    }

    #[test]
    fn paeth_prediction_tie_order_is_left_then_above() {
        assert_eq!(predict_paeth_sample(10u8, 25, 20), 10);
        assert_eq!(predict_paeth_sample(25u8, 10, 20), 10);
    }

    #[test]
    fn paeth_prediction_fills_rectangle_with_stride() {
        let left = [30u8; 8];
        let above = [12u8; 4];
        let mut output = [99u8; 48];

        predict_intra_paeth_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraPaethEdges::new(&left, &above, 10),
            &mut output,
            6,
        )
        .unwrap();

        for row in 0..8 {
            let start = row * 6;
            assert_eq!(&output[start..start + 6], &[30, 30, 30, 30, 99, 99]);
        }
    }

    #[test]
    fn paeth_prediction_validates_edge_lengths_by_dimension() {
        let left = [1u8; 4];
        let above = [1u8; 8];
        let mut output = [0u8; 32];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&left, &[1; 4], 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethEdgeLengthMismatch {
                edge: IntraPaethEdge::Left,
                expected: 8,
                actual: 4
            })
        ));
        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1; 8], &above, 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethEdgeLengthMismatch {
                edge: IntraPaethEdge::Above,
                expected: 4,
                actual: 8
            })
        ));
    }

    #[test]
    fn paeth_prediction_validates_edge_samples_against_bit_depth() {
        let mut output = [7u16; 32];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1; 8], &[1, 2, 256, 4], 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethSampleOutOfRange {
                edge: IntraPaethEdge::Above,
                sample_index: 2,
                value: 256,
                max: 255
            })
        ));
        assert_eq!(output, [7u16; 32]);

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1; 8], &[1; 4], 256),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethSampleOutOfRange {
                edge: IntraPaethEdge::TopLeft,
                sample_index: 0,
                value: 256,
                max: 255
            })
        ));
    }

    #[test]
    fn paeth_prediction_validates_sample_type_against_bit_depth() {
        let mut output = [0u8; 32];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Ten,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1u8; 8], &[1u8; 4], 1),
                &mut output,
                4
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn paeth_prediction_validates_output_stride_and_length() {
        let left = [1u8; 8];
        let above = [1u8; 4];
        let mut output = [0u8; 31];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&left, &above, 1),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&left, &above, 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPredictionOutputTooSmall {
                expected: 32,
                actual: 31
            })
        ));
    }

    #[test]
    fn paeth_prediction_rejects_overflowing_output_shape() {
        let mut output = [];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1u8; 8], &[1u8; 4], 1),
                &mut output,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "PAETH intra prediction output buffer length"
            })
        ));
    }
}
