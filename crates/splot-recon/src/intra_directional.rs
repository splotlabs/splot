// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cardinal directional intra prediction over the directional edge model.

use crate::intra_dc_math::{validate_output_shape, validate_sample_type};
use crate::intra_directional_angle::{
    IntraDirectionalAngleEdge, IntraDirectionalAngleEdges, validate_directional_edge,
};
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

/// Cardinal AV2 directional intra prediction mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraCardinalDirection {
    /// AV2 `V_PRED`, `Mode_To_Angle == 90`.
    Vertical,
    /// AV2 `H_PRED`, `Mode_To_Angle == 180`.
    Horizontal,
}

impl IntraCardinalDirection {
    /// Returns the required prepared edge for this cardinal direction.
    pub const fn required_edge(self) -> IntraDirectionalAngleEdge {
        match self {
            Self::Vertical => IntraDirectionalAngleEdge::Above,
            Self::Horizontal => IntraDirectionalAngleEdge::Left,
        }
    }

    /// Returns the AV2 pAngle value.
    pub const fn p_angle(self) -> u16 {
        match self {
            Self::Vertical => 90,
            Self::Horizontal => 180,
        }
    }

    /// Returns a stable human-readable mode name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
        }
    }
}

/// Writes AV2 §7.13.2.8 pAngle 90/180 prediction into caller storage.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// missing or wrong-length prepared edges, out-of-range edge samples, a
/// too-small stride, or a too-small output buffer.
pub fn predict_intra_cardinal_directional_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    direction: IntraCardinalDirection,
    edges: IntraDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "cardinal directional intra prediction output buffer length",
    )?;

    let edge_kind = direction.required_edge();
    let edge = required_edge(
        direction,
        match edge_kind {
            IntraDirectionalAngleEdge::Left => edges.left_samples(),
            IntraDirectionalAngleEdge::Above => edges.above_samples(),
        },
    )?;
    let edge_len = match edge_kind {
        IntraDirectionalAngleEdge::Left => size.height(),
        IntraDirectionalAngleEdge::Above => size.width(),
    };
    validate_directional_edge(edge_kind, edge, edge_len, bit_depth)?;

    for row in 0..size.height() {
        let row_start = row
            .checked_mul(stride_samples)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "cardinal directional intra prediction output row offset",
            })?;
        for column in 0..size.width() {
            let sample_index = match edge_kind {
                IntraDirectionalAngleEdge::Left => row,
                IntraDirectionalAngleEdge::Above => column,
            };
            output[row_start + column] = edge[sample_index];
        }
    }

    Ok(())
}

fn required_edge<T: ReconSample>(
    direction: IntraCardinalDirection,
    edge: Option<&[T]>,
) -> Result<&[T]> {
    edge.ok_or(ReconError::IntraDirectionalAngleEdgeUnavailable {
        p_angle: direction.p_angle(),
        edge: direction.required_edge(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn assert_cardinal_prediction(
        direction: IntraCardinalDirection,
        edges: IntraDirectionalAngleEdges<'_, u8>,
        expected_sample: impl Fn(usize, usize) -> u8,
    ) {
        let mut output = [0u8; 32];
        predict_intra_cardinal_directional_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            direction,
            edges,
            &mut output,
            4,
        )
        .unwrap();
        for row in 0..8 {
            for column in 0..4 {
                assert_eq!(output[row * 4 + column], expected_sample(row, column));
            }
        }
    }

    #[test]
    fn vertical_cardinal_prediction_copies_above_edge() {
        let above = [10, 20, 30, 40];
        assert_cardinal_prediction(
            IntraCardinalDirection::Vertical,
            IntraDirectionalAngleEdges::above(&above),
            |_, column| above[column],
        );
    }

    #[test]
    fn horizontal_cardinal_prediction_copies_left_edge() {
        let left = [1, 3, 5, 7, 9, 11, 13, 15];
        assert_cardinal_prediction(
            IntraCardinalDirection::Horizontal,
            IntraDirectionalAngleEdges::left(&left),
            |row, _| left[row],
        );
    }

    #[test]
    fn cardinal_prediction_fills_rectangle_with_stride() {
        let above = [10, 20, 30, 40];
        let mut output = [99u8; 48];

        predict_intra_cardinal_directional_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraCardinalDirection::Vertical,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            6,
        )
        .unwrap();

        for row in 0..8 {
            let start = row * 6;
            assert_eq!(&output[start..start + 6], &[10, 20, 30, 40, 99, 99]);
        }
    }

    #[test]
    fn cardinal_prediction_validates_required_edge_presence() {
        let mut output = [0u8; 16];

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraCardinalDirection::Vertical,
                IntraDirectionalAngleEdges::<u8>::left(&[1; 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraDirectionalAngleEdgeUnavailable {
                p_angle: 90,
                edge: IntraDirectionalAngleEdge::Above
            })
        ));
        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraCardinalDirection::Horizontal,
                IntraDirectionalAngleEdges::<u8>::above(&[1; 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraDirectionalAngleEdgeUnavailable {
                p_angle: 180,
                edge: IntraDirectionalAngleEdge::Left
            })
        ));
    }

    #[test]
    fn cardinal_prediction_validates_edge_lengths_by_direction() {
        let mut output = [0u8; 32];

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Vertical,
                IntraDirectionalAngleEdges::above(&[1; 3]),
                &mut output,
                4
            ),
            Err(ReconError::IntraDirectionalAngleEdgeLengthMismatch {
                edge: IntraDirectionalAngleEdge::Above,
                expected: 4,
                actual: 3
            })
        ));
        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Horizontal,
                IntraDirectionalAngleEdges::left(&[1; 7]),
                &mut output,
                4
            ),
            Err(ReconError::IntraDirectionalAngleEdgeLengthMismatch {
                edge: IntraDirectionalAngleEdge::Left,
                expected: 8,
                actual: 7
            })
        ));
    }

    #[test]
    fn cardinal_prediction_validates_edge_samples_against_bit_depth() {
        let mut output = [7u16; 32];

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Vertical,
                IntraDirectionalAngleEdges::above(&[1, 2, 256, 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraDirectionalAngleSampleOutOfRange {
                edge: IntraDirectionalAngleEdge::Above,
                sample_index: 2,
                value: 256,
                max: 255
            })
        ));
        assert_eq!(output, [7u16; 32]);

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Horizontal,
                IntraDirectionalAngleEdges::left(&[1, 2, 256, 4, 5, 6, 7, 8]),
                &mut output,
                4
            ),
            Err(ReconError::IntraDirectionalAngleSampleOutOfRange {
                edge: IntraDirectionalAngleEdge::Left,
                sample_index: 2,
                value: 256,
                max: 255
            })
        ));
    }

    #[test]
    fn cardinal_prediction_validates_sample_type_against_bit_depth() {
        let mut output = [0u8; 32];

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Ten,
                rect_size(2, 3),
                IntraCardinalDirection::Horizontal,
                IntraDirectionalAngleEdges::left(&[1u8; 8]),
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
    fn cardinal_prediction_validates_output_stride_and_length() {
        let above = [1u8; 4];
        let mut output = [0u8; 31];

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Vertical,
                IntraDirectionalAngleEdges::above(&above),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Vertical,
                IntraDirectionalAngleEdges::above(&above),
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
    fn cardinal_prediction_rejects_overflowing_output_shape() {
        let mut output = [];

        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Vertical,
                IntraDirectionalAngleEdges::above(&[1u8; 4]),
                &mut output,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "cardinal directional intra prediction output buffer length"
            })
        ));
    }
}
