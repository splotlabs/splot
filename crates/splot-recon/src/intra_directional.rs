// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cardinal directional intra prediction primitive.
//!
//! Feature tracking: `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`.

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
    pub const fn required_edge(self) -> IntraCardinalEdge {
        match self {
            Self::Vertical => IntraCardinalEdge::Above,
            Self::Horizontal => IntraCardinalEdge::Left,
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

/// Edge identifier for cardinal directional intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraCardinalEdge {
    /// Left edge samples `LeftCol[0..h)`.
    Left,
    /// Above edge samples `AboveRow[0..w)`.
    Above,
}

impl IntraCardinalEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
        }
    }
}

/// Caller-provided prepared edge samples for AV2 §7.13.2.8 cardinal prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraCardinalEdges<'a, T: ReconSample> {
    left: Option<&'a [T]>,
    above: Option<&'a [T]>,
}

impl<'a, T: ReconSample> IntraCardinalEdges<'a, T> {
    /// Creates a prepared cardinal edge set from optional left and above edges.
    ///
    /// Availability and fallback preparation remain outside this type and are
    /// owned by the broader AV2 §7.13.2.1 intra process.
    pub const fn new(left: Option<&'a [T]>, above: Option<&'a [T]>) -> Self {
        Self { left, above }
    }

    /// Creates an edge set with only `LeftCol[0..h)` available.
    pub const fn left(left: &'a [T]) -> Self {
        Self::new(Some(left), None)
    }

    /// Creates an edge set with only `AboveRow[0..w)` available.
    pub const fn above(above: &'a [T]) -> Self {
        Self::new(None, Some(above))
    }

    /// Creates an edge set with both left and above samples available.
    pub const fn both(left: &'a [T], above: &'a [T]) -> Self {
        Self::new(Some(left), Some(above))
    }

    /// Returns prepared left edge samples when available.
    pub const fn left_samples(self) -> Option<&'a [T]> {
        self.left
    }

    /// Returns prepared above edge samples when available.
    pub const fn above_samples(self) -> Option<&'a [T]> {
        self.above
    }
}

/// Writes AV2 §7.13.2.8 pAngle 90/180 prediction into caller storage.
///
/// `Vertical` corresponds to `V_PRED` / pAngle 90 and copies `AboveRow[0..w)`
/// into every output row. `Horizontal` corresponds to `H_PRED` / pAngle 180 and
/// copies `LeftCol[0..h)` into every output column. `output` points at the
/// top-left destination sample and `stride_samples` is the distance between
/// adjacent output rows.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// missing or wrong-length prepared edges, out-of-range edge samples, a
/// too-small stride, or a too-small output buffer.
pub fn predict_intra_cardinal_directional_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    direction: IntraCardinalDirection,
    edges: IntraCardinalEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(size, output.len(), stride_samples)?;

    match direction {
        IntraCardinalDirection::Vertical => {
            let above = required_edge(direction, edges.above)?;
            validate_edge(IntraCardinalEdge::Above, above, size.width(), bit_depth)?;
            for row in 0..size.height() {
                let row_start =
                    row.checked_mul(stride_samples)
                        .ok_or(ReconError::ArithmeticOverflow {
                            context: "cardinal directional intra prediction output row offset",
                        })?;
                // splot-copy-ok: copy prepared AboveRow into the caller-owned prediction row
                output[row_start..row_start + size.width()].copy_from_slice(above);
            }
        }
        IntraCardinalDirection::Horizontal => {
            let left = required_edge(direction, edges.left)?;
            validate_edge(IntraCardinalEdge::Left, left, size.height(), bit_depth)?;
            for (row, left_sample) in left.iter().copied().enumerate() {
                let row_start =
                    row.checked_mul(stride_samples)
                        .ok_or(ReconError::ArithmeticOverflow {
                            context: "cardinal directional intra prediction output row offset",
                        })?;
                output[row_start..row_start + size.width()].fill(left_sample);
            }
        }
    }

    Ok(())
}

fn required_edge<T: ReconSample>(
    direction: IntraCardinalDirection,
    edge: Option<&[T]>,
) -> Result<&[T]> {
    edge.ok_or(ReconError::IntraCardinalEdgeUnavailable {
        direction,
        edge: direction.required_edge(),
    })
}

fn validate_sample_type<T: ReconSample>(bit_depth: BitDepth) -> Result<()> {
    if T::supports_bit_depth(bit_depth) {
        Ok(())
    } else {
        Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth,
        })
    }
}

fn validate_edge<T: ReconSample>(
    edge: IntraCardinalEdge,
    samples: &[T],
    expected_len: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    if samples.len() != expected_len {
        return Err(ReconError::IntraCardinalEdgeLengthMismatch {
            edge,
            expected: expected_len,
            actual: samples.len(),
        });
    }

    for (sample_index, sample) in samples.iter().copied().enumerate() {
        validate_sample(edge, sample_index, sample, bit_depth)?;
    }

    Ok(())
}

fn validate_sample<T: ReconSample>(
    edge: IntraCardinalEdge,
    sample_index: usize,
    sample: T,
    bit_depth: BitDepth,
) -> Result<()> {
    let value = sample.to_u16();
    let max = bit_depth.max_sample();
    if value > max {
        Err(ReconError::IntraCardinalSampleOutOfRange {
            edge,
            sample_index,
            value,
            max,
        })
    } else {
        Ok(())
    }
}

fn validate_output_shape(
    size: IntraRectBlockSize,
    output_len: usize,
    stride_samples: usize,
) -> Result<()> {
    let width = size.width();
    if stride_samples < width {
        return Err(ReconError::IntraPredictionStrideTooSmall {
            stride_samples,
            width,
        });
    }

    let required = (size.height() - 1)
        .checked_mul(stride_samples)
        .and_then(|prefix| prefix.checked_add(width))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "cardinal directional intra prediction output buffer length",
        })?;

    if output_len < required {
        return Err(ReconError::IntraPredictionOutputTooSmall {
            expected: required,
            actual: output_len,
        });
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    #[test]
    fn vertical_cardinal_prediction_copies_above_edge() {
        let above = [10, 20, 30, 40];
        let mut output = [0u8; 32];

        predict_intra_cardinal_directional_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraCardinalDirection::Vertical,
            IntraCardinalEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(
            output,
            [
                10, 20, 30, 40, 10, 20, 30, 40, 10, 20, 30, 40, 10, 20, 30, 40, 10, 20, 30, 40, 10,
                20, 30, 40, 10, 20, 30, 40, 10, 20, 30, 40
            ]
        );
    }

    #[test]
    fn horizontal_cardinal_prediction_copies_left_edge() {
        let left = [1, 3, 5, 7, 9, 11, 13, 15];
        let mut output = [0u8; 32];

        predict_intra_cardinal_directional_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraCardinalDirection::Horizontal,
            IntraCardinalEdges::left(&left),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(
            output,
            [
                1, 1, 1, 1, 3, 3, 3, 3, 5, 5, 5, 5, 7, 7, 7, 7, 9, 9, 9, 9, 11, 11, 11, 11, 13, 13,
                13, 13, 15, 15, 15, 15
            ]
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
            IntraCardinalEdges::above(&above),
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
                IntraCardinalEdges::<u8>::left(&[1; 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraCardinalEdgeUnavailable {
                direction: IntraCardinalDirection::Vertical,
                edge: IntraCardinalEdge::Above
            })
        ));
        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraCardinalDirection::Horizontal,
                IntraCardinalEdges::<u8>::above(&[1; 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraCardinalEdgeUnavailable {
                direction: IntraCardinalDirection::Horizontal,
                edge: IntraCardinalEdge::Left
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
                IntraCardinalEdges::above(&[1; 3]),
                &mut output,
                4
            ),
            Err(ReconError::IntraCardinalEdgeLengthMismatch {
                edge: IntraCardinalEdge::Above,
                expected: 4,
                actual: 3
            })
        ));
        assert!(matches!(
            predict_intra_cardinal_directional_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraCardinalDirection::Horizontal,
                IntraCardinalEdges::left(&[1; 7]),
                &mut output,
                4
            ),
            Err(ReconError::IntraCardinalEdgeLengthMismatch {
                edge: IntraCardinalEdge::Left,
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
                IntraCardinalEdges::above(&[1, 2, 256, 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraCardinalSampleOutOfRange {
                edge: IntraCardinalEdge::Above,
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
                IntraCardinalEdges::left(&[1, 2, 256, 4, 5, 6, 7, 8]),
                &mut output,
                4
            ),
            Err(ReconError::IntraCardinalSampleOutOfRange {
                edge: IntraCardinalEdge::Left,
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
                IntraCardinalEdges::left(&[1u8; 8]),
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
                IntraCardinalEdges::above(&above),
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
                IntraCardinalEdges::above(&above),
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
                IntraCardinalEdges::above(&[1u8; 4]),
                &mut output,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "cardinal directional intra prediction output buffer length"
            })
        ));
    }
}
