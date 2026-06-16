// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One-sided directional-angle intra prediction primitive.
//!
//! Feature tracking: `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`.

use crate::intra_dc_math::{round2, validate_output_shape, validate_sample_type};
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

const ANGLE_D45: u16 = 45;
const ANGLE_D67: u16 = 67;
const ANGLE_D203: u16 = 203;
const INTERP_SCALE: u16 = 32;

// AV2 v1.0.0 §9.2 `Dr_Intra_Derivative[45]`.
const DR_INTRA_DERIVATIVE_45: u16 = 64;
// AV2 v1.0.0 §9.2 `Dr_Intra_Derivative[67]`.
const DR_INTRA_DERIVATIVE_67: u16 = 24;

/// Supported one-sided directional-angle pAngle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntraDirectionalAngle {
    p_angle: u16,
}

impl IntraDirectionalAngle {
    /// AV2 `D45_PRED`, `Mode_To_Angle == 45`.
    pub const D45: Self = Self { p_angle: ANGLE_D45 };

    /// AV2 `D67_PRED`, `Mode_To_Angle == 67`.
    pub const D67: Self = Self { p_angle: ANGLE_D67 };

    /// AV2 `D203_PRED`, `Mode_To_Angle == 203`.
    pub const D203: Self = Self {
        p_angle: ANGLE_D203,
    };

    /// Creates a supported one-sided directional angle from an AV2 pAngle.
    ///
    /// # Errors
    /// Returns [`ReconError::UnsupportedIntraDirectionalAngle`] for pAngles
    /// outside this primitive's narrow pAngle `45`, `67`, and `203` scope.
    pub const fn try_from_p_angle(p_angle: u16) -> Result<Self> {
        match p_angle {
            ANGLE_D45 => Ok(Self::D45),
            ANGLE_D67 => Ok(Self::D67),
            ANGLE_D203 => Ok(Self::D203),
            _ => Err(ReconError::UnsupportedIntraDirectionalAngle { p_angle }),
        }
    }

    /// Returns the AV2 pAngle value.
    pub const fn p_angle(self) -> u16 {
        self.p_angle
    }

    /// Returns the required prepared edge for this pAngle.
    pub const fn required_edge(self) -> IntraDirectionalAngleEdge {
        match self.p_angle {
            ANGLE_D45 | ANGLE_D67 => IntraDirectionalAngleEdge::Above,
            ANGLE_D203 => IntraDirectionalAngleEdge::Left,
            _ => IntraDirectionalAngleEdge::Above,
        }
    }

    const fn branch(self) -> DirectionalAngleBranch {
        match self.p_angle {
            ANGLE_D45 => DirectionalAngleBranch::Above {
                derivative: DR_INTRA_DERIVATIVE_45,
            },
            ANGLE_D67 => DirectionalAngleBranch::Above {
                derivative: DR_INTRA_DERIVATIVE_67,
            },
            ANGLE_D203 => DirectionalAngleBranch::Left {
                derivative: DR_INTRA_DERIVATIVE_67,
            },
            _ => DirectionalAngleBranch::Above { derivative: 0 },
        }
    }
}

/// Edge identifier for one-sided directional-angle prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraDirectionalAngleEdge {
    /// Left edge samples `LeftCol[0..w+h)`.
    Left,
    /// Above edge samples `AboveRow[0..w+h)`.
    Above,
}

impl IntraDirectionalAngleEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
        }
    }
}

/// Caller-provided prepared edge samples for one-sided AV2 §7.13.2.8 prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraDirectionalAngleEdges<'a, T: ReconSample> {
    left: Option<&'a [T]>,
    above: Option<&'a [T]>,
}

impl<'a, T: ReconSample> IntraDirectionalAngleEdges<'a, T> {
    /// Creates a prepared edge set from optional left and above edges.
    ///
    /// Availability and fallback preparation remain outside this type and are
    /// owned by the broader AV2 §7.13.2.1 intra process.
    pub const fn new(left: Option<&'a [T]>, above: Option<&'a [T]>) -> Self {
        Self { left, above }
    }

    /// Creates an edge set with only `LeftCol[0..w+h)` available.
    pub const fn left(left: &'a [T]) -> Self {
        Self::new(Some(left), None)
    }

    /// Creates an edge set with only `AboveRow[0..w+h)` available.
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

/// Writes a supported one-sided AV2 §7.13.2.8 directional prediction into caller storage.
///
/// This primitive intentionally covers only chroma/no-IDIF/no-MRL one-sided
/// pAngles `45`, `67`, and `203` over already-prepared edges. For pAngles `45`
/// and `67`, callers provide `AboveRow[0..w+h)`; for pAngle `203`, callers
/// provide `LeftCol[0..w+h)`. `output` points at the top-left destination sample
/// and `stride_samples` is the distance between adjacent output rows.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// missing or wrong-length prepared edges, out-of-range edge samples, a
/// too-small stride, a too-small output buffer, or checked arithmetic overflow.
pub fn predict_intra_directional_angle_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let context = validate_inputs(bit_depth, size, angle, edges, output.len(), stride_samples)?;
    write_prediction(bit_depth, size, angle, context.edge, output, stride_samples)
}

/// Writes a supported one-sided directional prediction from a raw AV2 pAngle.
///
/// # Errors
/// In addition to [`predict_intra_directional_angle_rect_into`] errors, returns
/// [`ReconError::UnsupportedIntraDirectionalAngle`] before output mutation when
/// `p_angle` is not `45`, `67`, or `203`.
pub fn predict_intra_directional_angle_rect_from_p_angle_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    p_angle: u16,
    edges: IntraDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
    predict_intra_directional_angle_rect_into(bit_depth, size, angle, edges, output, stride_samples)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectionalAngleBranch {
    Above { derivative: u16 },
    Left { derivative: u16 },
}

#[derive(Clone, Copy, Debug)]
struct ValidatedInputs<'a, T: ReconSample> {
    edge: &'a [T],
}

fn validate_inputs<'a, T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleEdges<'a, T>,
    output_len: usize,
    stride_samples: usize,
) -> Result<ValidatedInputs<'a, T>> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(size, output_len, stride_samples)?;
    let expected_len = required_edge_len(size)?;
    validate_index_bounds(size, angle, expected_len)?;

    let edge_kind = angle.required_edge();
    let edge = match edge_kind {
        IntraDirectionalAngleEdge::Left => {
            edges
                .left
                .ok_or(ReconError::IntraDirectionalAngleEdgeUnavailable {
                    angle,
                    edge: edge_kind,
                })?
        }
        IntraDirectionalAngleEdge::Above => {
            edges
                .above
                .ok_or(ReconError::IntraDirectionalAngleEdgeUnavailable {
                    angle,
                    edge: edge_kind,
                })?
        }
    };
    validate_edge(edge_kind, edge, expected_len, bit_depth)?;

    Ok(ValidatedInputs { edge })
}

fn required_edge_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(size.height())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "directional angle prepared edge length",
        })
}

fn validate_index_bounds(
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edge_len: usize,
) -> Result<()> {
    let max_base = edge_len
        .checked_sub(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "directional angle maximum base index",
        })?;
    let branch = angle.branch();
    let (outer, inner) = match branch {
        DirectionalAngleBranch::Above { .. } => (size.height(), size.width()),
        DirectionalAngleBranch::Left { .. } => (size.width(), size.height()),
    };
    let derivative = match branch {
        DirectionalAngleBranch::Above { derivative }
        | DirectionalAngleBranch::Left { derivative } => usize::from(derivative),
    };
    let max_idx = outer
        .checked_mul(derivative)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "directional angle derivative product",
        })?;
    let max_base_prefix = max_idx >> 6;
    let max_inner = inner.checked_sub(1).ok_or(ReconError::ArithmeticOverflow {
        context: "directional angle inner dimension",
    })?;
    let max_referenced_base =
        max_base_prefix
            .checked_add(max_inner)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "directional angle referenced base index",
            })?;
    if max_referenced_base > max_base {
        return Err(ReconError::ArithmeticOverflow {
            context: "directional angle prepared edge coverage",
        });
    }
    Ok(())
}

fn validate_edge<T: ReconSample>(
    edge: IntraDirectionalAngleEdge,
    samples: &[T],
    expected_len: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    if samples.len() != expected_len {
        return Err(ReconError::IntraDirectionalAngleEdgeLengthMismatch {
            edge,
            expected: expected_len,
            actual: samples.len(),
        });
    }

    let max = bit_depth.max_sample();
    for (sample_index, sample) in samples.iter().copied().enumerate() {
        let value = sample.to_u16();
        if value > max {
            return Err(ReconError::IntraDirectionalAngleSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            });
        }
    }

    Ok(())
}

fn write_prediction<T: ReconSample>(
    _bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edge: &[T],
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let max_base = required_edge_len(size)? - 1;
    match angle.branch() {
        DirectionalAngleBranch::Above { derivative } => {
            for row in 0..size.height() {
                let idx = (row + 1) * usize::from(derivative);
                let base_prefix = idx >> 6;
                let shift = ((idx >> 1) & 0x1f) as u16;
                let row_start = row * stride_samples;
                for column in 0..size.width() {
                    let base = base_prefix + column;
                    let value = if base < max_base {
                        bilinear(edge[base], edge[base + 1], shift)
                    } else {
                        edge[max_base].to_u16()
                    };
                    output[row_start + column] = T::try_from_u16(value)?;
                }
            }
        }
        DirectionalAngleBranch::Left { derivative } => {
            for row in 0..size.height() {
                let row_start = row * stride_samples;
                for column in 0..size.width() {
                    let idx = (column + 1) * usize::from(derivative);
                    let base = (idx >> 6) + row;
                    let shift = ((idx >> 1) & 0x1f) as u16;
                    let value = if base < max_base {
                        bilinear(edge[base], edge[base + 1], shift)
                    } else {
                        edge[max_base].to_u16()
                    };
                    output[row_start + column] = T::try_from_u16(value)?;
                }
            }
        }
    }

    Ok(())
}

fn bilinear<T: ReconSample>(a: T, b: T, shift: u16) -> u16 {
    let weighted = u64::from(a.to_u16()) * u64::from(INTERP_SCALE - shift)
        + u64::from(b.to_u16()) * u64::from(shift);
    round2(weighted, 5)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    #[test]
    fn d45_prediction_uses_above_edge_and_edge_end_fallback() {
        let above = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut output = [0u8; 16];

        predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDirectionalAngle::D45,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(
            output,
            [
                20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 80
            ]
        );
    }

    #[test]
    fn d67_prediction_matches_non_idif_bilinear_formula() {
        let above = [0, 32, 64, 96, 128, 160, 192, 224];
        let mut output = [0u8; 16];

        predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDirectionalAngle::D67,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(
            output,
            [
                12, 44, 76, 108, 24, 56, 88, 120, 36, 68, 100, 132, 48, 80, 112, 144
            ]
        );
    }

    #[test]
    fn d203_prediction_matches_non_idif_bilinear_formula() {
        let left = [0, 32, 64, 96, 128, 160, 192, 224];
        let mut output = [0u8; 16];

        predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDirectionalAngle::D203,
            IntraDirectionalAngleEdges::left(&left),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(
            output,
            [
                12, 24, 36, 48, 44, 56, 68, 80, 76, 88, 100, 112, 108, 120, 132, 144
            ]
        );
    }

    #[test]
    fn directional_angle_prediction_accepts_10_bit_u16_samples() {
        let above = [0u16, 64, 128, 192, 256, 320, 384, 1023];
        let mut output = [0u16; 16];

        predict_intra_directional_angle_rect_into(
            BitDepth::Ten,
            rect_size(2, 2),
            IntraDirectionalAngle::D67,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(output[0], 24);
        assert_eq!(output[15], 288);
    }

    #[test]
    fn directional_angle_prediction_rejects_unsupported_pangles_without_mutation() {
        let above = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut output = [9u8; 16];

        let err = predict_intra_directional_angle_rect_from_p_angle_into(
            BitDepth::Eight,
            rect_size(2, 2),
            90,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::UnsupportedIntraDirectionalAngle { p_angle: 90 }
        );
        assert_eq!(output, [9u8; 16]);
    }

    #[test]
    fn directional_angle_prediction_rejects_all_excluded_pangles() {
        for p_angle in [0, 90, 113, 135, 157, 180, 270] {
            assert_eq!(
                IntraDirectionalAngle::try_from_p_angle(p_angle),
                Err(ReconError::UnsupportedIntraDirectionalAngle { p_angle })
            );
        }
    }

    #[test]
    fn directional_angle_prediction_validates_required_edge_presence() {
        let mut output = [9u8; 16];

        let err = predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDirectionalAngle::D203,
            IntraDirectionalAngleEdges::new(None, None),
            &mut output,
            4,
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::IntraDirectionalAngleEdgeUnavailable {
                angle: IntraDirectionalAngle::D203,
                edge: IntraDirectionalAngleEdge::Left
            }
        );
        assert_eq!(output, [9u8; 16]);
    }

    #[test]
    fn directional_angle_prediction_validates_edge_lengths() {
        let above = [10, 20, 30, 40, 50, 60, 70];
        let mut output = [9u8; 16];

        let err = predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDirectionalAngle::D45,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::IntraDirectionalAngleEdgeLengthMismatch {
                edge: IntraDirectionalAngleEdge::Above,
                expected: 8,
                actual: 7
            }
        );
        assert_eq!(output, [9u8; 16]);
    }

    #[test]
    fn directional_angle_prediction_validates_edge_sample_ranges() {
        let above = [0u16, 1, 2, 3, 4, 5, 6, 1024];
        let mut output = [9u16; 16];

        let err = predict_intra_directional_angle_rect_into(
            BitDepth::Ten,
            rect_size(2, 2),
            IntraDirectionalAngle::D67,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::IntraDirectionalAngleSampleOutOfRange {
                edge: IntraDirectionalAngleEdge::Above,
                sample_index: 7,
                value: 1024,
                max: 1023
            }
        );
        assert_eq!(output, [9u16; 16]);
    }

    #[test]
    fn directional_angle_prediction_validates_sample_type_against_bit_depth() {
        let above = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut output = [9u8; 16];

        let err = predict_intra_directional_angle_rect_into(
            BitDepth::Ten,
            rect_size(2, 2),
            IntraDirectionalAngle::D45,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            4,
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            }
        );
        assert_eq!(output, [9u8; 16]);
    }

    #[test]
    fn directional_angle_prediction_validates_output_shape() {
        let above = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut output = [9u8; 16];

        let err = predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDirectionalAngle::D45,
            IntraDirectionalAngleEdges::above(&above),
            &mut output,
            3,
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            }
        );
        assert_eq!(output, [9u8; 16]);

        let mut short_output = [9u8; 15];
        let err = predict_intra_directional_angle_rect_into(
            BitDepth::Eight,
            rect_size(2, 2),
            IntraDirectionalAngle::D45,
            IntraDirectionalAngleEdges::above(&above),
            &mut short_output,
            4,
        )
        .unwrap_err();

        assert_eq!(
            err,
            ReconError::IntraPredictionOutputTooSmall {
                expected: 16,
                actual: 15
            }
        );
        assert_eq!(short_output, [9u8; 15]);
    }
}
