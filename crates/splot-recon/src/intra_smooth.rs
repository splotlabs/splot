// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Smooth intra prediction primitive.
//!
//! Feature tracking: `RECON-INTRA-SMOOTH-PREDICTION`.

use crate::intra_dc_math::{validate_output_shape, validate_sample_type};
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

const BLEND_WEIGHT_MAX: i64 = 32;

/// Smooth intra prediction mode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraSmoothMode {
    /// AV2 `SMOOTH_PRED`.
    Smooth,
    /// AV2 `SMOOTH_V_PRED`.
    SmoothVertical,
    /// AV2 `SMOOTH_H_PRED`.
    SmoothHorizontal,
}

impl IntraSmoothMode {
    /// Returns a stable human-readable mode name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Smooth => "smooth",
            Self::SmoothVertical => "smooth_v",
            Self::SmoothHorizontal => "smooth_h",
        }
    }
}

/// Edge identifier for smooth intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraSmoothEdge {
    /// Left edge samples `LeftCol[0..h)`.
    Left,
    /// Above edge samples `AboveRow[0..w)`.
    Above,
    /// Bottom-left sentinel sample `LeftCol[h]`.
    BottomLeft,
    /// Top-right sentinel sample `AboveRow[w]`.
    TopRight,
}

impl IntraSmoothEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
            Self::BottomLeft => "bottom-left",
            Self::TopRight => "top-right",
        }
    }
}

/// Caller-provided prepared edge samples for AV2 §7.13.2.13 smooth prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraSmoothEdges<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
}

impl<'a, T: ReconSample> IntraSmoothEdges<'a, T> {
    /// Creates a prepared smooth edge set.
    ///
    /// `left` must contain `LeftCol[0..h]`, including the bottom-left
    /// `LeftCol[h]` sentinel. `above` must contain `AboveRow[0..w]`,
    /// including the top-right `AboveRow[w]` sentinel. Availability and
    /// fallback preparation remain outside this type and are owned by the
    /// broader AV2 §7.13.2.1 intra process.
    pub const fn new(left: &'a [T], above: &'a [T]) -> Self {
        Self { left, above }
    }

    /// Returns prepared left edge samples, including `LeftCol[h]`.
    pub const fn left_samples(self) -> &'a [T] {
        self.left
    }

    /// Returns prepared above edge samples, including `AboveRow[w]`.
    pub const fn above_samples(self) -> &'a [T] {
        self.above
    }
}

/// Writes rectangular AV2 §7.13.2.13 smooth prediction into caller storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples or predictions, a too-small
/// stride, or a too-small output buffer.
pub fn predict_intra_smooth_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    edges: IntraSmoothEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_edges(bit_depth, size, edges)?;
    validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "smooth intra prediction output buffer length",
    )?;

    for row in 0..size.height() {
        let row_start = row
            .checked_mul(stride_samples)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "smooth intra prediction output row offset",
            })?;
        for column in 0..size.width() {
            output[row_start + column] =
                predict_smooth_sample(bit_depth, size, mode, edges, row, column)?;
        }
    }

    Ok(())
}

pub(crate) fn predict_smooth_sample<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    edges: IntraSmoothEdges<'_, T>,
    row: usize,
    column: usize,
) -> Result<T> {
    predict_smooth_sample_values(
        bit_depth,
        size,
        mode,
        SmoothSampleEdges {
            left: edges.left[row],
            top: edges.above[column],
            bottom_left: edges.left[size.height()],
            top_right: edges.above[size.width()],
        },
        SmoothSamplePosition { row, column },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SmoothSampleEdges<T: ReconSample> {
    pub(crate) left: T,
    pub(crate) top: T,
    pub(crate) bottom_left: T,
    pub(crate) top_right: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SmoothSamplePosition {
    pub(crate) row: usize,
    pub(crate) column: usize,
}

pub(crate) fn predict_smooth_sample_values<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    mode: IntraSmoothMode,
    samples: SmoothSampleEdges<T>,
    position: SmoothSamplePosition,
) -> Result<T> {
    let top = i64::from(samples.top.to_u16());
    let left = i64::from(samples.left.to_u16());
    let top_right = i64::from(samples.top_right.to_u16());
    let bottom_left = i64::from(samples.bottom_left.to_u16());
    let scale = round2_i64(
        i64::from(size.log2_width()) + i64::from(size.log2_height()) - 4,
        2,
    )?;
    let scale = u8::try_from(scale).map_err(|_| ReconError::ArithmeticOverflow {
        context: "smooth intra prediction scale",
    })?;

    let top_weight_shift = position
        .row
        .checked_shl(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction top weight row shift",
        })?
        >> scale;
    let left_weight_shift =
        position
            .column
            .checked_shl(1)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "smooth intra prediction left weight column shift",
            })?
            >> scale;
    let s_top = BLEND_WEIGHT_MAX >> core::cmp::min(6usize, top_weight_shift);
    let s_left = BLEND_WEIGHT_MAX >> core::cmp::min(6usize, left_weight_shift);
    let h_factor = i64::try_from(size.width() - 1 - position.column).map_err(|_| {
        ReconError::ArithmeticOverflow {
            context: "smooth intra prediction horizontal factor",
        }
    })?;
    let v_factor = i64::try_from(size.height() - 1 - position.row).map_err(|_| {
        ReconError::ArithmeticOverflow {
            context: "smooth intra prediction vertical factor",
        }
    })?;

    let pred_h = top_right
        + round2_i64(
            (left - top_right)
                .checked_mul(h_factor)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction horizontal product",
                })?,
            size.log2_width(),
        )?;
    let pred_v = bottom_left
        + round2_i64(
            (top - bottom_left)
                .checked_mul(v_factor)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction vertical product",
                })?,
            size.log2_height(),
        )?;
    let pred_h2 = pred_h
        + round2_i64(
            (left - pred_h)
                .checked_mul(s_left)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction horizontal blend product",
                })?,
            6,
        )?;
    let pred_v2 = pred_v
        + round2_i64(
            (top - pred_v)
                .checked_mul(s_top)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction vertical blend product",
                })?,
            6,
        )?;
    let predicted = match mode {
        IntraSmoothMode::SmoothHorizontal => pred_h2,
        IntraSmoothMode::SmoothVertical => pred_v2,
        IntraSmoothMode::Smooth => round2_i64(
            pred_v2
                .checked_add(pred_h2)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "smooth intra prediction combined sample",
                })?,
            1,
        )?,
    };

    let max = i64::from(bit_depth.max_sample());
    if predicted < 0 || predicted > max {
        return Err(ReconError::IntraSmoothPredictionOutOfRange {
            row: position.row,
            column: position.column,
            value: predicted,
            min: 0,
            max,
        });
    }

    T::try_from_u16(predicted as u16)
}

fn validate_edges<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraSmoothEdges<'_, T>,
) -> Result<()> {
    let expected_left = size
        .height()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction left edge length",
        })?;
    let expected_above = size
        .width()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction above edge length",
        })?;
    if edges.left.len() != expected_left {
        return Err(ReconError::IntraSmoothEdgeLengthMismatch {
            edge: IntraSmoothEdge::Left,
            expected: expected_left,
            actual: edges.left.len(),
        });
    }
    if edges.above.len() != expected_above {
        return Err(ReconError::IntraSmoothEdgeLengthMismatch {
            edge: IntraSmoothEdge::Above,
            expected: expected_above,
            actual: edges.above.len(),
        });
    }

    for (sample_index, sample) in edges.left[..size.height()].iter().copied().enumerate() {
        validate_sample(IntraSmoothEdge::Left, sample_index, sample, bit_depth)?;
    }
    validate_sample(
        IntraSmoothEdge::BottomLeft,
        size.height(),
        edges.left[size.height()],
        bit_depth,
    )?;
    for (sample_index, sample) in edges.above[..size.width()].iter().copied().enumerate() {
        validate_sample(IntraSmoothEdge::Above, sample_index, sample, bit_depth)?;
    }
    validate_sample(
        IntraSmoothEdge::TopRight,
        size.width(),
        edges.above[size.width()],
        bit_depth,
    )
}

fn validate_sample<T: ReconSample>(
    edge: IntraSmoothEdge,
    sample_index: usize,
    sample: T,
    bit_depth: BitDepth,
) -> Result<()> {
    let value = sample.to_u16();
    let max = bit_depth.max_sample();
    if value > max {
        Err(ReconError::IntraSmoothSampleOutOfRange {
            edge,
            sample_index,
            value,
            max,
        })
    } else {
        Ok(())
    }
}

fn round2_i64(value: i64, shift: u8) -> Result<i64> {
    if shift == 0 {
        return Ok(value);
    }
    let rounding =
        1i64.checked_shl(u32::from(shift - 1))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "smooth intra prediction Round2 offset",
            })?;
    let divisor = 1i64
        .checked_shl(u32::from(shift))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction Round2 divisor",
        })?;
    value
        .checked_add(rounding)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "smooth intra prediction Round2 input",
        })
        .map(|rounded| rounded.div_euclid(divisor))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn non_uniform_edges() -> ([u8; 5], [u8; 5]) {
        ([20, 40, 60, 80, 100], [10, 30, 50, 70, 90])
    }

    fn assert_smooth_prediction(mode: IntraSmoothMode, expected: [u8; 16]) {
        let (left, above) = non_uniform_edges();
        let mut output = [0u8; 16];

        predict_intra_smooth_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            mode,
            IntraSmoothEdges::new(&left, &above),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(output, expected);
    }

    #[test]
    fn smooth_horizontal_prediction_matches_spec_formula() {
        assert_smooth_prediction(
            IntraSmoothMode::SmoothHorizontal,
            [
                29, 51, 71, 90, 47, 62, 77, 90, 64, 73, 82, 90, 82, 84, 88, 90,
            ],
        );
    }

    #[test]
    fn smooth_vertical_prediction_matches_spec_formula() {
        assert_smooth_prediction(
            IntraSmoothMode::SmoothVertical,
            [
                22, 39, 57, 74, 49, 61, 72, 83, 76, 81, 87, 92, 100, 100, 100, 100,
            ],
        );
    }

    #[test]
    fn smooth_prediction_averages_horizontal_and_vertical_paths() {
        assert_smooth_prediction(
            IntraSmoothMode::Smooth,
            [
                26, 45, 64, 82, 48, 62, 75, 87, 70, 77, 85, 91, 91, 92, 94, 95,
            ],
        );
    }

    #[test]
    fn smooth_prediction_fills_rectangle_with_stride() {
        let (left, above) = non_uniform_edges();
        let mut output = [99u8; 24];

        predict_intra_smooth_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraSmoothMode::SmoothHorizontal,
            IntraSmoothEdges::new(&left, &above),
            &mut output,
            6,
        )
        .unwrap();

        assert_eq!(&output[0..6], &[29, 51, 71, 90, 99, 99]);
        assert_eq!(&output[6..12], &[47, 62, 77, 90, 99, 99]);
        assert_eq!(&output[12..18], &[64, 73, 82, 90, 99, 99]);
        assert_eq!(&output[18..24], &[82, 84, 88, 90, 99, 99]);
    }

    #[test]
    fn smooth_prediction_validates_edge_lengths() {
        let mut output = [9u8; 16];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 4], &[1; 5]),
                &mut output,
                4
            ),
            Err(ReconError::IntraSmoothEdgeLengthMismatch {
                edge: IntraSmoothEdge::Left,
                expected: 5,
                actual: 4
            })
        ));
        assert_eq!(output, [9u8; 16]);

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 5], &[1; 4]),
                &mut output,
                4
            ),
            Err(ReconError::IntraSmoothEdgeLengthMismatch {
                edge: IntraSmoothEdge::Above,
                expected: 5,
                actual: 4
            })
        ));
        assert_eq!(output, [9u8; 16]);
    }

    #[test]
    fn smooth_prediction_validates_edge_samples_against_bit_depth() {
        let mut output = [0u16; 16];

        for (left, above, expected_edge, expected_index) in [
            ([1, 300, 1, 1, 1], [1; 5], IntraSmoothEdge::Left, 1),
            ([1; 5], [1, 300, 1, 1, 1], IntraSmoothEdge::Above, 1),
            ([1, 1, 1, 1, 300], [1; 5], IntraSmoothEdge::BottomLeft, 4),
            ([1; 5], [1, 1, 1, 1, 300], IntraSmoothEdge::TopRight, 4),
        ] {
            assert!(matches!(
                predict_intra_smooth_rect_into(
                    BitDepth::Eight,
                    rect_size(2, 2),
                    IntraSmoothMode::Smooth,
                    IntraSmoothEdges::new(&left, &above),
                    &mut output,
                    4
                ),
                Err(ReconError::IntraSmoothSampleOutOfRange {
                    edge,
                    sample_index,
                    value: 300,
                    max: 255
                }) if edge == expected_edge && sample_index == expected_index
            ));
        }
    }

    #[test]
    fn smooth_prediction_validates_computed_sample_range() {
        assert!(matches!(
            predict_smooth_sample_values(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::SmoothHorizontal,
                SmoothSampleEdges {
                    left: 65535u16,
                    top: 0u16,
                    bottom_left: 0u16,
                    top_right: 65535u16,
                },
                SmoothSamplePosition { row: 0, column: 0 }
            ),
            Err(ReconError::IntraSmoothPredictionOutOfRange {
                row: 0,
                column: 0,
                value,
                min: 0,
                max: 255
            }) if value > 255
        ));
    }

    #[test]
    fn smooth_prediction_validates_sample_type_against_bit_depth() {
        let mut output = [0u8; 16];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Ten,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 5], &[1; 5]),
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
    fn smooth_prediction_validates_output_stride_and_length() {
        let (left, above) = non_uniform_edges();
        let mut output = [9u8; 15];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&left, &above),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));
        assert_eq!(output, [9u8; 15]);

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(2, 2),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&left, &above),
                &mut output,
                4
            ),
            Err(ReconError::IntraPredictionOutputTooSmall {
                expected: 16,
                actual: 15
            })
        ));
        assert_eq!(output, [9u8; 15]);
    }

    #[test]
    fn smooth_prediction_rejects_overflowing_output_shape() {
        let mut output = [0u8; 0];

        assert!(matches!(
            predict_intra_smooth_rect_into(
                BitDepth::Eight,
                rect_size(6, 6),
                IntraSmoothMode::Smooth,
                IntraSmoothEdges::new(&[1; 65], &[1; 65]),
                &mut output,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "smooth intra prediction output buffer length"
            })
        ));
    }
}
