// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Directional-angle intra prediction primitives.
//!
//! Feature tracking: `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`,
//! `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`.

use crate::intra_dc_math::{round2, validate_output_shape, validate_sample_type};
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

const ANGLE_D45: u16 = 45;
const ANGLE_D67: u16 = 67;
const ANGLE_D113: u16 = 113;
const ANGLE_D135: u16 = 135;
const ANGLE_D157: u16 = 157;
const ANGLE_D203: u16 = 203;
const INTERP_SCALE: u16 = 32;

// AV2 v1.0.0 §9.2 `Dr_Intra_Derivative[23]`.
const DR_INTRA_DERIVATIVE_23: u16 = 170;
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

/// Supported middle directional-angle pAngle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntraMiddleDirectionalAngle {
    p_angle: u16,
}

impl IntraMiddleDirectionalAngle {
    /// AV2 `D113_PRED`, `Mode_To_Angle == 113`.
    pub const D113: Self = Self {
        p_angle: ANGLE_D113,
    };

    /// AV2 `D135_PRED`, `Mode_To_Angle == 135`.
    pub const D135: Self = Self {
        p_angle: ANGLE_D135,
    };

    /// AV2 `D157_PRED`, `Mode_To_Angle == 157`.
    pub const D157: Self = Self {
        p_angle: ANGLE_D157,
    };

    /// Creates a supported middle directional angle from an AV2 pAngle.
    ///
    /// # Errors
    /// Returns [`ReconError::UnsupportedIntraMiddleDirectionalAngle`] for
    /// pAngles outside this primitive's narrow pAngle `113`, `135`, and `157`
    /// scope.
    pub const fn try_from_p_angle(p_angle: u16) -> Result<Self> {
        match p_angle {
            ANGLE_D113 => Ok(Self::D113),
            ANGLE_D135 => Ok(Self::D135),
            ANGLE_D157 => Ok(Self::D157),
            _ => Err(ReconError::UnsupportedIntraMiddleDirectionalAngle { p_angle }),
        }
    }

    /// Returns the AV2 pAngle value.
    pub const fn p_angle(self) -> u16 {
        self.p_angle
    }

    fn branch(self) -> Result<MiddleDirectionalAngleBranch> {
        match self.p_angle {
            ANGLE_D113 => Ok(MiddleDirectionalAngleBranch {
                dx: DR_INTRA_DERIVATIVE_67,
                dy: DR_INTRA_DERIVATIVE_23,
            }),
            ANGLE_D135 => Ok(MiddleDirectionalAngleBranch {
                dx: DR_INTRA_DERIVATIVE_45,
                dy: DR_INTRA_DERIVATIVE_45,
            }),
            ANGLE_D157 => Ok(MiddleDirectionalAngleBranch {
                dx: DR_INTRA_DERIVATIVE_23,
                dy: DR_INTRA_DERIVATIVE_67,
            }),
            _ => Err(ReconError::UnsupportedIntraMiddleDirectionalAngle {
                p_angle: self.p_angle,
            }),
        }
    }
}

/// Caller-provided prepared edge samples for middle AV2 §7.13.2.8 prediction.
///
/// Each supplied slice starts with the logical `-1` sample: `slice[0]` maps to
/// `AboveRow[-1]` or `LeftCol[-1]`, and `slice[index + 1]` maps to logical
/// index `index`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraMiddleDirectionalAngleEdges<'a, T: ReconSample> {
    left_with_minus_one: Option<&'a [T]>,
    above_with_minus_one: Option<&'a [T]>,
}

impl<'a, T: ReconSample> IntraMiddleDirectionalAngleEdges<'a, T> {
    /// Creates a prepared middle-edge set from optional left and above edges.
    ///
    /// Availability, fallback preparation, IDIF extension, and MRL remain
    /// outside this type and are owned by the broader AV2 §7.13.2.1 intra
    /// process.
    pub const fn new(
        left_with_minus_one: Option<&'a [T]>,
        above_with_minus_one: Option<&'a [T]>,
    ) -> Self {
        Self {
            left_with_minus_one,
            above_with_minus_one,
        }
    }

    /// Creates an edge set with both `LeftCol[-1..h)` and `AboveRow[-1..w)`.
    pub const fn both(left_with_minus_one: &'a [T], above_with_minus_one: &'a [T]) -> Self {
        Self::new(Some(left_with_minus_one), Some(above_with_minus_one))
    }

    /// Returns prepared left edge samples when available.
    pub const fn left_with_minus_one(self) -> Option<&'a [T]> {
        self.left_with_minus_one
    }

    /// Returns prepared above edge samples when available.
    pub const fn above_with_minus_one(self) -> Option<&'a [T]> {
        self.above_with_minus_one
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

/// Writes a supported middle AV2 §7.13.2.8 directional prediction into caller storage.
///
/// This primitive intentionally covers only chroma/no-IDIF/no-MRL middle
/// pAngles `113`, `135`, and `157` over already-prepared logical edges.
/// Callers provide `AboveRow[-1..w)` and `LeftCol[-1..h)`, where slice index
/// zero stores the logical `-1` sample. `output` points at the top-left
/// destination sample and `stride_samples` is the distance between adjacent
/// output rows.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// unsupported pAngles, missing or wrong-length prepared edges, out-of-range
/// edge samples, a too-small stride, a too-small output buffer, or checked
/// arithmetic overflow.
pub fn predict_intra_middle_directional_angle_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let context =
        validate_middle_inputs(bit_depth, size, angle, edges, output.len(), stride_samples)?;
    write_middle_prediction(
        size,
        angle,
        context.left,
        context.above,
        output,
        stride_samples,
    )
}

/// Writes a supported middle directional prediction from a raw AV2 pAngle.
///
/// # Errors
/// In addition to [`predict_intra_middle_directional_angle_rect_into`] errors,
/// returns [`ReconError::UnsupportedIntraMiddleDirectionalAngle`] before output
/// mutation when `p_angle` is not `113`, `135`, or `157`.
pub fn predict_intra_middle_directional_angle_rect_from_p_angle_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    p_angle: u16,
    edges: IntraMiddleDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;
    predict_intra_middle_directional_angle_rect_into(
        bit_depth,
        size,
        angle,
        edges,
        output,
        stride_samples,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectionalAngleBranch {
    Above { derivative: u16 },
    Left { derivative: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MiddleDirectionalAngleBranch {
    dx: u16,
    dy: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MiddleSampleReference {
    edge: IntraDirectionalAngleEdge,
    base: i64,
    shift: u16,
}

#[derive(Clone, Copy, Debug)]
struct ValidatedInputs<'a, T: ReconSample> {
    edge: &'a [T],
}

#[derive(Clone, Copy, Debug)]
struct ValidatedMiddleInputs<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
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

fn validate_middle_inputs<'a, T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleEdges<'a, T>,
    output_len: usize,
    stride_samples: usize,
) -> Result<ValidatedMiddleInputs<'a, T>> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(size, output_len, stride_samples)?;

    let left = edges.left_with_minus_one.ok_or(
        ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle,
            edge: IntraDirectionalAngleEdge::Left,
        },
    )?;
    let above = edges.above_with_minus_one.ok_or(
        ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle,
            edge: IntraDirectionalAngleEdge::Above,
        },
    )?;

    let left_len = required_middle_left_len(size)?;
    let above_len = required_middle_above_len(size)?;
    validate_middle_edge(IntraDirectionalAngleEdge::Left, left, left_len, bit_depth)?;
    validate_middle_edge(
        IntraDirectionalAngleEdge::Above,
        above,
        above_len,
        bit_depth,
    )?;

    validate_middle_index_bounds(size, angle, left.len(), above.len())?;

    Ok(ValidatedMiddleInputs { left, above })
}

fn required_edge_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(size.height())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "directional angle prepared edge length",
        })
}

fn required_middle_left_len(size: IntraRectBlockSize) -> Result<usize> {
    size.height()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle left edge length",
        })
}

fn required_middle_above_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle above edge length",
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

fn validate_middle_index_bounds(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    left_len: usize,
    above_len: usize,
) -> Result<()> {
    let branch = angle.branch()?;
    for row in 0..size.height() {
        for column in 0..size.width() {
            let reference = middle_sample_reference(row, column, branch)?;
            let len = match reference.edge {
                IntraDirectionalAngleEdge::Left => left_len,
                IntraDirectionalAngleEdge::Above => above_len,
            };
            logical_edge_offset(reference.base, len)?;
            let next_base =
                reference
                    .base
                    .checked_add(1)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "middle directional angle next edge base",
                    })?;
            logical_edge_offset(next_base, len)?;
        }
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

fn validate_middle_edge<T: ReconSample>(
    edge: IntraDirectionalAngleEdge,
    samples: &[T],
    expected_len: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    if samples.len() != expected_len {
        return Err(ReconError::IntraMiddleDirectionalAngleEdgeLengthMismatch {
            edge,
            expected: expected_len,
            actual: samples.len(),
        });
    }

    let max = bit_depth.max_sample();
    for (sample_index, sample) in samples.iter().copied().enumerate() {
        let value = sample.to_u16();
        if value > max {
            return Err(ReconError::IntraMiddleDirectionalAngleSampleOutOfRange {
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

fn write_middle_prediction<T: ReconSample>(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    left: &[T],
    above: &[T],
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let branch = angle.branch()?;
    for row in 0..size.height() {
        let row_start = row * stride_samples;
        for column in 0..size.width() {
            let reference = middle_sample_reference(row, column, branch)?;
            let edge = match reference.edge {
                IntraDirectionalAngleEdge::Left => left,
                IntraDirectionalAngleEdge::Above => above,
            };
            let next_base =
                reference
                    .base
                    .checked_add(1)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "middle directional angle next edge base",
                    })?;
            let value = bilinear(
                logical_edge_sample(edge, reference.base)?,
                logical_edge_sample(edge, next_base)?,
                reference.shift,
            );
            output[row_start + column] = T::try_from_u16(value)?;
        }
    }

    Ok(())
}

fn middle_sample_reference(
    row: usize,
    column: usize,
    branch: MiddleDirectionalAngleBranch,
) -> Result<MiddleSampleReference> {
    let column_scaled = checked_scaled_i64(column, "middle directional angle above index prefix")?;
    let row_plus_one = checked_usize_plus_one_i64(row, "middle directional angle above row index")?;
    let above_delta =
        row_plus_one
            .checked_mul(i64::from(branch.dx))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "middle directional angle above derivative product",
            })?;
    let above_idx =
        column_scaled
            .checked_sub(above_delta)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "middle directional angle above index",
            })?;
    let above_base = above_idx >> 6;
    if above_base >= -1 {
        return Ok(MiddleSampleReference {
            edge: IntraDirectionalAngleEdge::Above,
            base: above_base,
            shift: directional_shift(above_idx),
        });
    }

    let row_scaled = checked_scaled_i64(row, "middle directional angle left index prefix")?;
    let column_plus_one =
        checked_usize_plus_one_i64(column, "middle directional angle left column index")?;
    let left_delta = column_plus_one.checked_mul(i64::from(branch.dy)).ok_or(
        ReconError::ArithmeticOverflow {
            context: "middle directional angle left derivative product",
        },
    )?;
    let left_idx = row_scaled
        .checked_sub(left_delta)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle left index",
        })?;
    Ok(MiddleSampleReference {
        edge: IntraDirectionalAngleEdge::Left,
        base: left_idx >> 6,
        shift: directional_shift(left_idx),
    })
}

fn checked_scaled_i64(value: usize, context: &'static str) -> Result<i64> {
    let value = i64::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })?;
    value
        .checked_mul(64)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn checked_usize_plus_one_i64(value: usize, context: &'static str) -> Result<i64> {
    let value = i64::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })?;
    value
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn directional_shift(idx: i64) -> u16 {
    ((idx >> 1) & 0x1f) as u16
}

fn logical_edge_sample<T: ReconSample>(samples: &[T], logical_index: i64) -> Result<T> {
    let offset = logical_edge_offset(logical_index, samples.len())?;
    Ok(samples[offset])
}

fn logical_edge_offset(logical_index: i64, len: usize) -> Result<usize> {
    let shifted = logical_index
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle logical edge offset",
        })?;
    let offset = usize::try_from(shifted).map_err(|_| ReconError::ArithmeticOverflow {
        context: "middle directional angle logical edge coverage",
    })?;
    if offset >= len {
        return Err(ReconError::ArithmeticOverflow {
            context: "middle directional angle logical edge coverage",
        });
    }
    Ok(offset)
}

fn bilinear<T: ReconSample>(a: T, b: T, shift: u16) -> u16 {
    let weighted = u64::from(a.to_u16()) * u64::from(INTERP_SCALE - shift)
        + u64::from(b.to_u16()) * u64::from(shift);
    round2(weighted, 5)
}

#[cfg(test)]
#[path = "intra_directional_angle_tests.rs"]
mod tests;
