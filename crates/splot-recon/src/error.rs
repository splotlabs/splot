// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error types for reconstruction model construction.

use core::fmt;

use crate::{
    BitDepth, IntraCardinalDirection, IntraCardinalEdge, IntraDcEdge, IntraDirectionalAngle,
    IntraDirectionalAngleEdge, IntraMiddleDirectionalAngle, IntraPaethEdge, IntraSmoothEdge,
    PlaneId, PlaneRect, PlaneSize, ReferenceSlot,
};

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
    /// A rectangular intra prediction block dimension is outside the modeled range.
    InvalidIntraRectBlockLog2 {
        /// Supplied base-2 logarithm of the block width.
        log2_width: u8,
        /// Supplied base-2 logarithm of the block height.
        log2_height: u8,
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
    /// A caller-owned intra prediction output sample exceeded the active bit depth.
    IntraPredictionOutputSampleOutOfRange {
        /// Zero-based sample index within the caller-owned strided output buffer.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A supplied PAETH intra prediction edge did not match the block size.
    IntraPaethEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraPaethEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A PAETH intra prediction edge sample exceeded the active bit depth.
    IntraPaethSampleOutOfRange {
        /// Edge containing the out-of-range sample.
        edge: IntraPaethEdge,
        /// Zero-based index within the edge samples.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A required cardinal directional intra prediction edge was absent.
    IntraCardinalEdgeUnavailable {
        /// Cardinal prediction direction being computed.
        direction: IntraCardinalDirection,
        /// Required edge that was absent.
        edge: IntraCardinalEdge,
    },
    /// A supplied cardinal directional intra prediction edge did not match the block size.
    IntraCardinalEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraCardinalEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A cardinal directional intra prediction edge sample exceeded the active bit depth.
    IntraCardinalSampleOutOfRange {
        /// Edge containing the out-of-range sample.
        edge: IntraCardinalEdge,
        /// Zero-based index within the edge samples.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A directional-angle pAngle is outside the currently source-backed subset.
    UnsupportedIntraDirectionalAngle {
        /// Rejected AV2 pAngle value.
        p_angle: u16,
    },
    /// A required one-sided directional-angle prediction edge was absent.
    IntraDirectionalAngleEdgeUnavailable {
        /// Directional pAngle being computed.
        angle: IntraDirectionalAngle,
        /// Required edge that was absent.
        edge: IntraDirectionalAngleEdge,
    },
    /// A supplied one-sided directional-angle edge did not match the block size.
    IntraDirectionalAngleEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraDirectionalAngleEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A one-sided directional-angle edge sample exceeded the active bit depth.
    IntraDirectionalAngleSampleOutOfRange {
        /// Edge containing the out-of-range sample.
        edge: IntraDirectionalAngleEdge,
        /// Zero-based index within the edge samples.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A middle directional-angle pAngle is outside the currently source-backed subset.
    UnsupportedIntraMiddleDirectionalAngle {
        /// Rejected AV2 pAngle value.
        p_angle: u16,
    },
    /// A required middle directional-angle prediction edge was absent.
    IntraMiddleDirectionalAngleEdgeUnavailable {
        /// Directional pAngle being computed.
        angle: IntraMiddleDirectionalAngle,
        /// Required edge that was absent.
        edge: IntraDirectionalAngleEdge,
    },
    /// A supplied middle directional-angle edge did not match the block size.
    IntraMiddleDirectionalAngleEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraDirectionalAngleEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A middle directional-angle edge sample exceeded the active bit depth.
    IntraMiddleDirectionalAngleSampleOutOfRange {
        /// Edge containing the out-of-range sample.
        edge: IntraDirectionalAngleEdge,
        /// Zero-based index within the edge samples.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A supplied smooth intra prediction edge did not match the block size.
    IntraSmoothEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraSmoothEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A smooth intra prediction edge sample exceeded the active bit depth.
    IntraSmoothSampleOutOfRange {
        /// Edge containing the out-of-range sample.
        edge: IntraSmoothEdge,
        /// Zero-based index within the edge samples.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A smooth intra prediction sample was outside the active bit depth.
    IntraSmoothPredictionOutOfRange {
        /// Row of the predicted sample.
        row: usize,
        /// Column of the predicted sample.
        column: usize,
        /// Predicted sample value.
        value: i64,
        /// Minimum allowed sample value.
        min: i64,
        /// Maximum sample value allowed by the active bit depth.
        max: i64,
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
    /// A workspace intra prediction helper could not read a required edge.
    WorkspaceIntraPredictionEdgeUnavailable {
        /// Plane whose workspace storage was checked.
        plane: PlaneId,
        /// Required edge that was outside workspace storage.
        edge: IntraPaethEdge,
        /// Prediction rectangle needing the edge.
        rect: PlaneRect,
    },
    /// A workspace smooth intra helper could not read a required prepared edge.
    WorkspaceSmoothIntraPredictionEdgeUnavailable {
        /// Plane whose workspace storage was checked.
        plane: PlaneId,
        /// Required edge that was outside workspace storage.
        edge: IntraSmoothEdge,
        /// Prediction rectangle needing the edge.
        rect: PlaneRect,
    },
    /// A workspace cardinal directional intra helper could not read a required edge.
    WorkspaceCardinalIntraPredictionEdgeUnavailable {
        /// Plane whose workspace storage was checked.
        plane: PlaneId,
        /// Required edge that was outside workspace storage.
        edge: IntraCardinalEdge,
        /// Prediction rectangle needing the edge.
        rect: PlaneRect,
    },
    /// A workspace directional-angle helper could not read a required edge.
    WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
        /// Plane whose workspace storage was checked.
        plane: PlaneId,
        /// Directional pAngle being computed.
        p_angle: u16,
        /// Required edge that was outside workspace storage.
        edge: IntraDirectionalAngleEdge,
        /// Prediction rectangle needing the edge.
        rect: PlaneRect,
    },
    /// A workspace directional-angle helper would need luma IDIF.
    WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported {
        /// Luma plane whose workspace storage was rejected.
        plane: PlaneId,
        /// Directional pAngle being computed.
        p_angle: u16,
        /// Prediction rectangle needing luma IDIF.
        rect: PlaneRect,
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
    /// A caller-derived reference refresh mask selected unsupported bits.
    InvalidReferenceRefreshMask {
        /// Requested refresh mask bits.
        mask: u32,
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
    /// A valid refresh mask selected a slot outside a store's capacity.
    ReferenceRefreshMaskOutOfBounds {
        /// Requested refresh mask bits.
        mask: u32,
        /// Store capacity used for the bounds check.
        capacity: usize,
    },
    /// An inverse transform was given an unsupported 1D length.
    InvalidInverseTransformSize {
        /// Supplied source length.
        size: usize,
    },
    /// An inverse transform output length did not match the source length.
    InverseTransformLengthMismatch {
        /// Source coefficient count.
        src_len: usize,
        /// Output buffer length.
        out_len: usize,
    },
    /// A reconstruct residual-addition call had mismatched buffer lengths.
    ReconstructLengthMismatch {
        /// Prediction sample count.
        prediction_len: usize,
        /// Residual sample count.
        residual_len: usize,
        /// Output buffer length.
        out_len: usize,
    },
    /// A reconstruct prediction sample exceeded the active decoded bit depth.
    ReconstructPredictionOutOfRange {
        /// Zero-based index of the out-of-range prediction sample.
        sample_index: usize,
        /// Observed prediction sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A 2D inverse transform was given an unsupported original log2 shape.
    InvalidInverseTransform2dShape {
        /// Supplied original (unadjusted) transform width base-2 logarithm.
        log2_w: u32,
        /// Supplied original (unadjusted) transform height base-2 logarithm.
        log2_h: u32,
        /// Whether the lossless flag was set (which requires a 4x4 block).
        lossless: bool,
    },
    /// A § 7.15.4 `Transform_Shift` lookup was requested for a `(log2W, log2H)`
    /// pair that is not one of the 25 AV2 `TX_SIZES_ALL` transform shapes.
    InvalidTransformShiftShape {
        /// Requested transform width base-2 logarithm.
        log2_width: u32,
        /// Requested transform height base-2 logarithm.
        log2_height: u32,
    },
    /// A § 7.15.4 `get_transform_1d_type` lookup was requested with a
    /// `PlaneTxType` outside the valid `TX_TYPES` range (`0..16`).
    InvalidPlaneTxType {
        /// Requested `PlaneTxType` index.
        plane_tx_type: usize,
    },
    /// A § 5.20.7.30 `get_scan` request had an unsupported transform shape
    /// (`w` / `h` must each be 4, 8, 16, or 32).
    InvalidScanShape {
        /// Requested operating width.
        w: usize,
        /// Requested operating height.
        h: usize,
    },
    /// A § 5.20.7.30 `get_scan` output buffer length did not match `w * h`.
    ScanLengthMismatch {
        /// Expected length (`w * h`).
        expected: usize,
        /// Supplied output buffer length.
        out_len: usize,
    },
    /// A 2D inverse transform input/output buffer length did not match `w * h`.
    InverseTransform2dBufferMismatch {
        /// Expected length (`w * h`).
        expected: usize,
        /// Supplied dequantized-coefficient buffer length.
        dequant_len: usize,
        /// Supplied residual buffer length.
        residual_len: usize,
    },
    /// A § 7.15.4 outer 2D inverse transform buffer length did not match its
    /// expected size: the dequantized block is the adjusted `adjW * adjH`, while
    /// the residual block is the original `w * h`.
    InverseTransform2dOuterBufferMismatch {
        /// Expected dequantized-coefficient length (adjusted `adjW * adjH`).
        dequant_expected: usize,
        /// Expected residual length (original `w * h`).
        residual_expected: usize,
        /// Supplied dequantized-coefficient buffer length.
        dequant_len: usize,
        /// Supplied residual buffer length.
        residual_len: usize,
    },
    /// A § 7.15.3 secondary transform operating side was not a power of two in
    /// `4..=32`.
    SecondaryTransformInvalidShape {
        /// Supplied operating width.
        w: usize,
        /// Supplied operating height.
        h: usize,
    },
    /// A § 7.15.3 secondary transform `dequant` buffer length did not match
    /// `w * h`.
    SecondaryTransformBufferMismatch {
        /// Expected length (`w * h`).
        expected: usize,
        /// Supplied dequantized-coefficient buffer length.
        actual: usize,
    },
    /// A § 7.15.3 secondary transform parameter was out of range for the selected
    /// kernel set: `n` exceeds the kernel height, `kernel` exceeds the set size,
    /// or `sec_tx_type` is not in `1..=3`.
    SecondaryTransformInvalidParams {
        /// Supplied input coefficient count (`n`).
        n: usize,
        /// Supplied kernel-set index.
        kernel: usize,
        /// Supplied secondary transform type (`sec_tx_type`).
        sec_tx_type: usize,
    },
    /// A § 7.14.4 dequantization block had an unsupported transform shape.
    InvalidDequantBlockShape {
        /// Supplied dequantized transform-block width.
        tx_width: usize,
        /// Supplied dequantized transform-block height.
        tx_height: usize,
    },
    /// A § 7.14.4 dequantization `quant` / `out` buffer length did not match
    /// `tx_width * tx_height`.
    DequantBlockLengthMismatch {
        /// Expected length (`tx_width * tx_height`).
        expected: usize,
        /// Supplied coded-coefficient buffer length.
        quant_len: usize,
        /// Supplied dequantized-output buffer length.
        out_len: usize,
    },
    /// A § 7.14.4 quantization-matrix weight lookup index was out of range for
    /// the generated `Quantizer_Matrix`.
    InvalidQuantizerMatrixIndex {
        /// Requested `segLvl` segment level.
        seg_level: usize,
        /// Caller-resolved `Qm_Offset[txSz]` region start within the matrix row.
        qm_offset: usize,
        /// Derived flattened position within the matrix row.
        position: usize,
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
            Self::InvalidIntraRectBlockLog2 {
                log2_width,
                log2_height,
                min,
                max,
            } => write!(
                f,
                "unsupported rectangular intra block log2 size {log2_width}x{log2_height}; expected each dimension {min} through {max}"
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
            Self::IntraPredictionOutputSampleOutOfRange {
                sample_index,
                value,
                max,
            } => write!(
                f,
                "intra prediction output sample {sample_index} value {value} exceeds maximum {max}"
            ),
            Self::IntraPaethEdgeLengthMismatch {
                edge,
                expected,
                actual,
            } => write!(
                f,
                "PAETH intra prediction {} edge length mismatch: expected {expected} samples, got {actual}",
                edge.name()
            ),
            Self::IntraPaethSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            } => write!(
                f,
                "PAETH intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}",
                edge.name()
            ),
            Self::IntraCardinalEdgeUnavailable { direction, edge } => write!(
                f,
                "cardinal directional intra prediction {} mode requires {} edge",
                direction.name(),
                edge.name()
            ),
            Self::IntraCardinalEdgeLengthMismatch {
                edge,
                expected,
                actual,
            } => write!(
                f,
                "cardinal directional intra prediction {} edge length mismatch: expected {expected} samples, got {actual}",
                edge.name()
            ),
            Self::IntraCardinalSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            } => write!(
                f,
                "cardinal directional intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}",
                edge.name()
            ),
            Self::UnsupportedIntraDirectionalAngle { p_angle } => write!(
                f,
                "unsupported one-sided directional intra prediction pAngle {p_angle}; expected 45, 67, or 203"
            ),
            Self::IntraDirectionalAngleEdgeUnavailable { angle, edge } => write!(
                f,
                "directional angle intra prediction pAngle {} requires {} edge",
                angle.p_angle(),
                edge.name()
            ),
            Self::IntraDirectionalAngleEdgeLengthMismatch {
                edge,
                expected,
                actual,
            } => write!(
                f,
                "directional angle intra prediction {} edge length mismatch: expected {expected} samples, got {actual}",
                edge.name()
            ),
            Self::IntraDirectionalAngleSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            } => write!(
                f,
                "directional angle intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}",
                edge.name()
            ),
            Self::UnsupportedIntraMiddleDirectionalAngle { p_angle } => write!(
                f,
                "unsupported middle directional intra prediction pAngle {p_angle}; expected 113, 135, or 157"
            ),
            Self::IntraMiddleDirectionalAngleEdgeUnavailable { angle, edge } => write!(
                f,
                "middle directional angle intra prediction pAngle {} requires {} edge",
                angle.p_angle(),
                edge.name()
            ),
            Self::IntraMiddleDirectionalAngleEdgeLengthMismatch {
                edge,
                expected,
                actual,
            } => write!(
                f,
                "middle directional angle intra prediction {} edge length mismatch: expected {expected} samples, got {actual}",
                edge.name()
            ),
            Self::IntraMiddleDirectionalAngleSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            } => write!(
                f,
                "middle directional angle intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}",
                edge.name()
            ),
            Self::IntraSmoothEdgeLengthMismatch {
                edge,
                expected,
                actual,
            } => write!(
                f,
                "smooth intra prediction {} edge length mismatch: expected {expected} samples, got {actual}",
                edge.name()
            ),
            Self::IntraSmoothSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            } => write!(
                f,
                "smooth intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}",
                edge.name()
            ),
            Self::IntraSmoothPredictionOutOfRange {
                row,
                column,
                value,
                min,
                max,
            } => write!(
                f,
                "smooth intra prediction sample at row {row} column {column} value {value} is outside {min}..={max}"
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
            Self::WorkspaceIntraPredictionEdgeUnavailable { plane, edge, rect } => write!(
                f,
                "current-frame workspace {} intra prediction requires {} edge for rectangle x={} y={} width={} height={}",
                plane.name(),
                edge.name(),
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height()
            ),
            Self::WorkspaceSmoothIntraPredictionEdgeUnavailable { plane, edge, rect } => write!(
                f,
                "current-frame workspace {} smooth intra prediction requires {} edge for rectangle x={} y={} width={} height={}",
                plane.name(),
                edge.name(),
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height()
            ),
            Self::WorkspaceCardinalIntraPredictionEdgeUnavailable { plane, edge, rect } => write!(
                f,
                "current-frame workspace {} cardinal directional intra prediction requires {} edge for rectangle x={} y={} width={} height={}",
                plane.name(),
                edge.name(),
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height()
            ),
            Self::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
                plane,
                p_angle,
                edge,
                rect,
            } => write!(
                f,
                "current-frame workspace {} directional angle intra prediction pAngle {} requires {} edge for rectangle x={} y={} width={} height={}",
                plane.name(),
                p_angle,
                edge.name(),
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height()
            ),
            Self::WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported {
                plane,
                p_angle,
                rect,
            } => write!(
                f,
                "current-frame workspace {} directional angle intra prediction pAngle {} requires luma IDIF for rectangle x={} y={} width={} height={}",
                plane.name(),
                p_angle,
                rect.x(),
                rect.y(),
                rect.width(),
                rect.height()
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
            Self::InvalidReferenceRefreshMask { mask, max_slots } => write!(
                f,
                "reference refresh mask 0x{mask:08x} contains bits outside 0..{max_slots}"
            ),
            Self::ReferenceSlotOutOfBounds { slot, capacity } => write!(
                f,
                "reference slot {} is outside store capacity {capacity}",
                slot.index()
            ),
            Self::ReferenceRefreshMaskOutOfBounds { mask, capacity } => write!(
                f,
                "reference refresh mask 0x{mask:08x} selects a slot outside store capacity {capacity}"
            ),
            Self::InvalidInverseTransformSize { size } => write!(
                f,
                "unsupported inverse transform length {size}; expected 4, 8, 16, or 32"
            ),
            Self::InverseTransformLengthMismatch { src_len, out_len } => write!(
                f,
                "inverse transform output length {out_len} does not match source length {src_len}"
            ),
            Self::ReconstructLengthMismatch {
                prediction_len,
                residual_len,
                out_len,
            } => write!(
                f,
                "reconstruct length mismatch: prediction {prediction_len}, residual {residual_len}, output {out_len}"
            ),
            Self::ReconstructPredictionOutOfRange {
                sample_index,
                value,
                max,
            } => write!(
                f,
                "reconstruct prediction sample {sample_index} value {value} exceeds maximum {max}"
            ),
            Self::InvalidInverseTransform2dShape {
                log2_w,
                log2_h,
                lossless,
            } => write!(
                f,
                "unsupported 2D inverse transform log2 shape {log2_w}x{log2_h} (lossless={lossless}); expected each log2 in 2..=6, and 2x2 (4x4) when lossless"
            ),
            Self::InvalidTransformShiftShape {
                log2_width,
                log2_height,
            } => write!(
                f,
                "no Transform_Shift entry for log2 shape {log2_width}x{log2_height}; expected one of the 25 AV2 TX_SIZES_ALL shapes"
            ),
            Self::InvalidPlaneTxType { plane_tx_type } => write!(
                f,
                "invalid PlaneTxType {plane_tx_type}; expected a TX_TYPES index in 0..16"
            ),
            Self::InvalidScanShape { w, h } => write!(
                f,
                "unsupported get_scan shape {w}x{h}; expected each of w/h in 4/8/16/32"
            ),
            Self::ScanLengthMismatch { expected, out_len } => write!(
                f,
                "get_scan output length mismatch: expected {expected}, got {out_len}"
            ),
            Self::InverseTransform2dBufferMismatch {
                expected,
                dequant_len,
                residual_len,
            } => write!(
                f,
                "2D inverse transform buffer mismatch: expected {expected}, dequant {dequant_len}, residual {residual_len}"
            ),
            Self::InverseTransform2dOuterBufferMismatch {
                dequant_expected,
                residual_expected,
                dequant_len,
                residual_len,
            } => write!(
                f,
                "2D outer inverse transform buffer mismatch: dequant expected {dequant_expected} (got {dequant_len}), residual expected {residual_expected} (got {residual_len})"
            ),
            Self::SecondaryTransformInvalidShape { w, h } => write!(
                f,
                "unsupported secondary transform shape {w}x{h}; expected each side 4, 8, 16, or 32"
            ),
            Self::SecondaryTransformBufferMismatch { expected, actual } => write!(
                f,
                "secondary transform buffer mismatch: expected {expected}, got {actual}"
            ),
            Self::SecondaryTransformInvalidParams {
                n,
                kernel,
                sec_tx_type,
            } => write!(
                f,
                "invalid secondary transform params: n {n}, kernel {kernel}, sec_tx_type {sec_tx_type}"
            ),
            Self::InvalidDequantBlockShape {
                tx_width,
                tx_height,
            } => write!(
                f,
                "unsupported dequantization block shape {tx_width}x{tx_height}; expected each side 4, 8, 16, or 32"
            ),
            Self::DequantBlockLengthMismatch {
                expected,
                quant_len,
                out_len,
            } => write!(
                f,
                "dequantization block buffer mismatch: expected {expected}, quant {quant_len}, out {out_len}"
            ),
            Self::InvalidQuantizerMatrixIndex {
                seg_level,
                qm_offset,
                position,
            } => write!(
                f,
                "quantization-matrix index out of range: seg_level {seg_level}, qm_offset {qm_offset}, position {position}"
            ),
        }
    }
}

impl std::error::Error for ReconError {}
