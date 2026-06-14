// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error types for reconstruction model construction.

use core::fmt;

use crate::{BitDepth, IntraDcEdge, PlaneId, PlaneRect, PlaneSize, ReferenceSlot};

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
    /// A decoded sample value cannot be represented by the requested storage type.
    SampleValueUnsupportedStorage {
        /// Rust sample storage type name.
        sample_type: &'static str,
        /// Observed sample value.
        value: u16,
        /// Maximum value representable by the storage type.
        max: u16,
    },
    /// A current-frame workspace backing allocation failed.
    WorkspaceAllocationFailed {
        /// Plane whose workspace storage was being allocated.
        plane: PlaneId,
        /// Short description of the failed allocation.
        context: &'static str,
    },
    /// A requested current-frame workspace plane is not present.
    MissingWorkspacePlane {
        /// Missing workspace plane.
        plane: PlaneId,
    },
    /// A current-frame workspace rectangle fell outside plane storage.
    WorkspaceRectOutOfBounds {
        /// Plane whose storage bounds were checked.
        plane: PlaneId,
        /// Storage dimensions used for the bounds check.
        storage: PlaneSize,
        /// Rectangle that exceeded `storage`.
        rect: PlaneRect,
    },
    /// A caller-provided workspace write stride was too small.
    WorkspaceWriteStrideTooSmall {
        /// Plane being written.
        plane: PlaneId,
        /// Supplied source stride in samples.
        stride_samples: usize,
        /// Required write width in samples.
        width: usize,
    },
    /// A caller-provided workspace write buffer was too small.
    WorkspaceWriteLengthMismatch {
        /// Plane being written.
        plane: PlaneId,
        /// Minimum required sample count.
        expected: usize,
        /// Actual supplied sample count.
        actual: usize,
    },
    /// A square intra prediction block size is outside the modeled range.
    InvalidIntraSquareBlockLog2 {
        /// Supplied base-2 logarithm of the square block size.
        log2_size: u8,
        /// Minimum supported base-2 logarithm.
        min: u8,
        /// Maximum supported base-2 logarithm.
        max: u8,
    },
    /// A supplied intra prediction edge did not match the block size.
    IntraPredictionEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraDcEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// An intra prediction edge sample exceeded the active bit depth.
    IntraPredictionSampleOutOfRange {
        /// Edge containing the out-of-range sample.
        edge: IntraDcEdge,
        /// Zero-based index within the edge samples.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// An intra prediction block backing allocation failed.
    IntraPredictionAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },
    /// A caller-provided intra prediction output stride was too small.
    IntraPredictionStrideTooSmall {
        /// Supplied output stride in samples.
        stride_samples: usize,
        /// Required prediction width in samples.
        width: usize,
    },
    /// A caller-provided intra prediction output buffer was too small.
    IntraPredictionOutputTooSmall {
        /// Minimum required sample count for the supplied block and stride.
        expected: usize,
        /// Actual output slice length.
        actual: usize,
    },
    /// A reference frame store capacity was outside the supported slot range.
    InvalidReferenceStoreCapacity {
        /// Requested store capacity.
        capacity: usize,
        /// Maximum supported store capacity.
        max_slots: usize,
    },
    /// A reference slot index was outside the source-backed slot ceiling.
    InvalidReferenceSlotIndex {
        /// Requested reference slot index.
        index: usize,
        /// Maximum supported slot count.
        max_slots: usize,
    },
    /// A valid reference slot was outside a particular store's capacity.
    ReferenceSlotOutOfBounds {
        /// Requested reference slot.
        slot: ReferenceSlot,
        /// Store capacity used for the bounds check.
        capacity: usize,
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
            Self::SampleValueUnsupportedStorage {
                sample_type,
                value,
                max,
            } => write!(
                f,
                "sample value {value} cannot be represented by {sample_type}; maximum is {max}"
            ),
            Self::WorkspaceAllocationFailed { plane, context } => write!(
                f,
                "failed to allocate current-frame workspace {} plane {context}",
                plane.name()
            ),
            Self::MissingWorkspacePlane { plane } => {
                write!(
                    f,
                    "current-frame workspace plane {} is not present",
                    plane.name()
                )
            }
            Self::WorkspaceRectOutOfBounds {
                plane,
                storage,
                rect,
            } => write!(
                f,
                "current-frame workspace {} rectangle x={} y={} width={} height={} is outside storage {}x{}",
                plane.name(),
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height(),
                storage.width(),
                storage.height()
            ),
            Self::WorkspaceWriteStrideTooSmall {
                plane,
                stride_samples,
                width,
            } => write!(
                f,
                "current-frame workspace {} write stride {stride_samples} samples is smaller than write width {width}",
                plane.name()
            ),
            Self::WorkspaceWriteLengthMismatch {
                plane,
                expected,
                actual,
            } => write!(
                f,
                "current-frame workspace {} write buffer is too small: expected at least {expected} samples, got {actual}",
                plane.name()
            ),
            Self::InvalidIntraSquareBlockLog2 {
                log2_size,
                min,
                max,
            } => write!(
                f,
                "unsupported square intra block log2 size {log2_size}; expected {min} through {max}"
            ),
            Self::IntraPredictionEdgeLengthMismatch {
                edge,
                expected,
                actual,
            } => write!(
                f,
                "intra prediction {} edge length mismatch: expected {expected} samples, got {actual}",
                edge.name()
            ),
            Self::IntraPredictionSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            } => write!(
                f,
                "intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}",
                edge.name()
            ),
            Self::IntraPredictionAllocationFailed { context } => {
                write!(f, "failed to allocate {context}")
            }
            Self::IntraPredictionStrideTooSmall {
                stride_samples,
                width,
            } => write!(
                f,
                "intra prediction output stride {stride_samples} samples is smaller than prediction width {width}"
            ),
            Self::IntraPredictionOutputTooSmall { expected, actual } => write!(
                f,
                "intra prediction output buffer is too small: expected at least {expected} samples, got {actual}"
            ),
            Self::InvalidReferenceStoreCapacity {
                capacity,
                max_slots,
            } => write!(
                f,
                "reference frame store capacity {capacity} is outside 1..={max_slots}"
            ),
            Self::InvalidReferenceSlotIndex { index, max_slots } => {
                write!(f, "reference slot index {index} is outside 0..{max_slots}")
            }
            Self::ReferenceSlotOutOfBounds { slot, capacity } => write!(
                f,
                "reference slot {} is outside store capacity {capacity}",
                slot.index()
            ),
        }
    }
}

impl std::error::Error for ReconError {}
