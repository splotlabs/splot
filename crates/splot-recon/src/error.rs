// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error types for reconstruction model construction.

use core::fmt;

use crate::{BitDepth, PlaneId, PlaneRect, PlaneSize};

/// Result alias used by `splot-recon` constructors and helpers.
pub type Result<T> = core::result::Result<T, ReconError>;

/// Errors reported while constructing decoded frame and plane model values.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ReconError {
    /// AV2 § 6.4.1 reserved or unsupported `bit_depth_idc` value.
    UnsupportedBitDepthIdc {
        /// The rejected `bit_depth_idc` value.
        idc: u8,
    },
    /// AV2 § 6.4.1 reserved or unsupported `chroma_format_idc` value.
    UnsupportedChromaFormatIdc {
        /// The rejected `chroma_format_idc` value.
        idc: u8,
    },
    /// A dimension that must be positive was zero.
    ZeroDimension {
        /// Name of the zero-valued field.
        field: &'static str,
    },
    /// Checked arithmetic overflowed while deriving a model value.
    ArithmeticOverflow {
        /// Short description of the overflowed derivation.
        context: &'static str,
    },
    /// A plane stride was smaller than the storage width.
    StrideTooSmall {
        /// Supplied stride in samples.
        stride_samples: usize,
        /// Required minimum stride in samples.
        storage_width: usize,
    },
    /// The supplied backing buffer length did not match the derived length.
    BufferLengthMismatch {
        /// Expected sample count.
        expected: usize,
        /// Actual sample count.
        actual: usize,
    },
    /// A visible rectangle fell outside the storage rectangle.
    VisibleRectOutOfBounds {
        /// Storage dimensions used for the bounds check.
        storage: PlaneSize,
        /// Visible rectangle that exceeded `storage`.
        rect: PlaneRect,
    },
    /// A luma crop origin was not aligned for the chroma subsampling format.
    CropOriginNotAligned {
        /// Luma crop x origin in samples.
        x: usize,
        /// Luma crop y origin in samples.
        y: usize,
        /// AV2 `SubsamplingX` value for the pixel format.
        subsampling_x: u8,
        /// AV2 `SubsamplingY` value for the pixel format.
        subsampling_y: u8,
    },
    /// A non-monochrome decoded frame was missing a chroma plane.
    MissingChromaPlane {
        /// Missing chroma plane.
        plane: PlaneId,
    },
    /// A monochrome decoded frame unexpectedly included a chroma plane.
    UnexpectedChromaPlane {
        /// Unexpected chroma plane.
        plane: PlaneId,
    },
    /// A plane's visible size did not match the expected decoded-frame size.
    PlaneSizeMismatch {
        /// Plane whose visible size was checked.
        plane: PlaneId,
        /// Expected visible size.
        expected: PlaneSize,
        /// Actual visible size.
        actual: PlaneSize,
    },
    /// The sample storage type cannot represent the requested bit depth.
    SampleTypeUnsupportedBitDepth {
        /// Rust sample storage type name.
        sample_type: &'static str,
        /// Requested decoded-frame bit depth.
        bit_depth: BitDepth,
    },
    /// A stored sample exceeded the active decoded-frame bit depth range.
    SampleOutOfRange {
        /// Plane containing the out-of-range sample.
        plane: PlaneId,
        /// Zero-based index within the plane backing buffer.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
}

impl fmt::Display for ReconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBitDepthIdc { idc } => {
                write!(f, "unsupported AV2 bit_depth_idc {idc}; expected 0 or 1")
            }
            Self::UnsupportedChromaFormatIdc { idc } => {
                write!(
                    f,
                    "unsupported AV2 chroma_format_idc {idc}; expected 0 through 3"
                )
            }
            Self::ZeroDimension { field } => {
                write!(f, "{field} must be greater than zero")
            }
            Self::ArithmeticOverflow { context } => {
                write!(f, "arithmetic overflow while deriving {context}")
            }
            Self::StrideTooSmall {
                stride_samples,
                storage_width,
            } => write!(
                f,
                "plane stride {stride_samples} samples is smaller than storage width {storage_width}"
            ),
            Self::BufferLengthMismatch { expected, actual } => write!(
                f,
                "plane buffer length mismatch: expected {expected} samples, got {actual}"
            ),
            Self::VisibleRectOutOfBounds { storage, rect } => write!(
                f,
                "visible rectangle x={} y={} width={} height={} is outside storage {}x{}",
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height(),
                storage.width(),
                storage.height()
            ),
            Self::CropOriginNotAligned {
                x,
                y,
                subsampling_x,
                subsampling_y,
            } => write!(
                f,
                "luma crop origin ({x}, {y}) is not aligned to subsampling ({subsampling_x}, {subsampling_y})"
            ),
            Self::MissingChromaPlane { plane } => {
                write!(f, "missing required chroma plane {}", plane.name())
            }
            Self::UnexpectedChromaPlane { plane } => {
                write!(
                    f,
                    "unexpected chroma plane {} for monochrome output",
                    plane.name()
                )
            }
            Self::PlaneSizeMismatch {
                plane,
                expected,
                actual,
            } => write!(
                f,
                "plane {} visible size mismatch: expected {}x{}, got {}x{}",
                plane.name(),
                expected.width(),
                expected.height(),
                actual.width(),
                actual.height()
            ),
            Self::SampleTypeUnsupportedBitDepth {
                sample_type,
                bit_depth,
            } => write!(
                f,
                "sample type {sample_type} cannot represent {}-bit decoded output",
                bit_depth.bits()
            ),
            Self::SampleOutOfRange {
                plane,
                sample_index,
                value,
                max,
            } => write!(
                f,
                "plane {} sample {sample_index} value {value} exceeds maximum {max}",
                plane.name()
            ),
        }
    }
}

impl std::error::Error for ReconError {}
