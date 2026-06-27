// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error types for reconstruction model construction.

use thiserror::Error;

use crate::{
    BitDepth, IntraCardinalDirection, IntraCardinalEdge, IntraDcEdge, IntraDirectionalAngle,
    IntraDirectionalAngleEdge, IntraMiddleDirectionalAngle, IntraPaethEdge, IntraSmoothEdge,
    PlaneId, PlaneRect, PlaneSize, ReferenceSlot,
};

/// Result alias used by `splot-recon` constructors and helpers.
pub type Result<T> = core::result::Result<T, ReconError>;

/// Errors reported while constructing decoded frame and plane model values.
#[derive(Clone, Debug, Eq, PartialEq, Error)]
#[non_exhaustive]
pub enum ReconError {
    /// AV2 § 6.4.1 reserved or unsupported `bit_depth_idc` value.
    #[error("unsupported AV2 bit_depth_idc {idc}; expected 0 or 1")]
    UnsupportedBitDepthIdc {
        /// The rejected `bit_depth_idc` value.
        idc: u8,
    },
    /// AV2 § 6.4.1 reserved or unsupported `chroma_format_idc` value.
    #[error("unsupported AV2 chroma_format_idc {idc}; expected 0 through 3")]
    UnsupportedChromaFormatIdc {
        /// The rejected `chroma_format_idc` value.
        idc: u8,
    },
    /// A dimension that must be positive was zero.
    #[error("{field} must be greater than zero")]
    ZeroDimension {
        /// Name of the zero-valued field.
        field: &'static str,
    },
    /// Checked arithmetic overflowed while deriving a model value.
    #[error("arithmetic overflow while deriving {context}")]
    ArithmeticOverflow {
        /// Short description of the overflowed derivation.
        context: &'static str,
    },
    /// A plane stride was smaller than the storage width.
    #[error("plane stride {stride_samples} samples is smaller than storage width {storage_width}")]
    StrideTooSmall {
        /// Supplied stride in samples.
        stride_samples: usize,
        /// Required minimum stride in samples.
        storage_width: usize,
    },
    /// The supplied backing buffer length did not match the derived length.
    #[error("plane buffer length mismatch: expected {expected} samples, got {actual}")]
    BufferLengthMismatch {
        /// Expected sample count.
        expected: usize,
        /// Actual sample count.
        actual: usize,
    },
    /// A visible rectangle fell outside the storage rectangle.
    #[error("visible rectangle x={} y={} width={} height={} is outside storage {}x{}", .rect.x(), .rect.y(), .rect.width(), .rect.height(), .storage.width(), .storage.height())]
    VisibleRectOutOfBounds {
        /// Storage dimensions used for the bounds check.
        storage: PlaneSize,
        /// Visible rectangle that exceeded `storage`.
        rect: PlaneRect,
    },
    /// A luma crop origin was not aligned for the chroma subsampling format.
    #[error(
        "luma crop origin ({x}, {y}) is not aligned to subsampling ({subsampling_x}, {subsampling_y})"
    )]
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
    #[error("missing required chroma plane {}", .plane.name())]
    MissingChromaPlane {
        /// Missing chroma plane.
        plane: PlaneId,
    },
    /// A monochrome decoded frame unexpectedly included a chroma plane.
    #[error("unexpected chroma plane {} for monochrome output", .plane.name())]
    UnexpectedChromaPlane {
        /// Unexpected chroma plane.
        plane: PlaneId,
    },
    /// A plane's visible size did not match the expected decoded-frame size.
    #[error("plane {} visible size mismatch: expected {}x{}, got {}x{}", .plane.name(), .expected.width(), .expected.height(), .actual.width(), .actual.height())]
    PlaneSizeMismatch {
        /// Plane whose visible size was checked.
        plane: PlaneId,
        /// Expected visible size.
        expected: PlaneSize,
        /// Actual visible size.
        actual: PlaneSize,
    },
    /// The sample storage type cannot represent the requested bit depth.
    #[error("sample type {sample_type} cannot represent {}-bit decoded output", .bit_depth.bits())]
    SampleTypeUnsupportedBitDepth {
        /// Rust sample storage type name.
        sample_type: &'static str,
        /// Requested decoded-frame bit depth.
        bit_depth: BitDepth,
    },
    /// A stored sample exceeded the active decoded-frame bit depth range.
    #[error("plane {} sample {sample_index} value {value} exceeds maximum {max}", .plane.name())]
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
    #[error("sample value {value} cannot be represented by {sample_type}; maximum is {max}")]
    SampleValueUnsupportedStorage {
        /// Rust sample storage type name.
        sample_type: &'static str,
        /// Observed sample value.
        value: u16,
        /// Maximum value representable by the storage type.
        max: u16,
    },
    /// A current-frame workspace backing allocation failed.
    #[error("failed to allocate current-frame workspace {} plane {context}", .plane.name())]
    WorkspaceAllocationFailed {
        /// Plane whose workspace storage was being allocated.
        plane: PlaneId,
        /// Short description of the failed allocation.
        context: &'static str,
    },
    /// A requested current-frame workspace plane is not present.
    #[error("current-frame workspace plane {} is not present", .plane.name())]
    MissingWorkspacePlane {
        /// Missing workspace plane.
        plane: PlaneId,
    },
    /// A current-frame workspace rectangle fell outside plane storage.
    #[error("current-frame workspace {} rectangle x={} y={} width={} height={} is outside storage {}x{}", .plane.name(), .rect.x(), .rect.y(), .rect.width(), .rect.height(), .storage.width(), .storage.height())]
    WorkspaceRectOutOfBounds {
        /// Plane whose storage bounds were checked.
        plane: PlaneId,
        /// Storage dimensions used for the bounds check.
        storage: PlaneSize,
        /// Rectangle that exceeded `storage`.
        rect: PlaneRect,
    },
    /// A caller-provided workspace write stride was too small.
    #[error("current-frame workspace {} write stride {stride_samples} samples is smaller than write width {width}", .plane.name())]
    WorkspaceWriteStrideTooSmall {
        /// Plane being written.
        plane: PlaneId,
        /// Supplied source stride in samples.
        stride_samples: usize,
        /// Required write width in samples.
        width: usize,
    },
    /// A caller-provided workspace write buffer was too small.
    #[error("current-frame workspace {} write buffer is too small: expected at least {expected} samples, got {actual}", .plane.name())]
    WorkspaceWriteLengthMismatch {
        /// Plane being written.
        plane: PlaneId,
        /// Minimum required sample count.
        expected: usize,
        /// Actual supplied sample count.
        actual: usize,
    },
    /// A current-frame workspace copy source and target had different shapes.
    #[error("current-frame workspace {} copy source {}x{} does not match target {}x{}", .plane.name(), .source_rect.width(), .source_rect.height(), .target_rect.width(), .target_rect.height())]
    WorkspaceCopyShapeMismatch {
        /// Plane being copied.
        plane: PlaneId,
        /// Source rectangle. Named `source_rect` rather than `source` because
        /// `thiserror` treats a field named `source` as the error source.
        source_rect: PlaneRect,
        /// Target rectangle.
        target_rect: PlaneRect,
    },
    /// A square intra prediction block size is outside the modeled range.
    #[error("unsupported square intra block log2 size {log2_size}; expected {min} through {max}")]
    InvalidIntraSquareBlockLog2 {
        /// Supplied base-2 logarithm of the square block size.
        log2_size: u8,
        /// Minimum supported base-2 logarithm.
        min: u8,
        /// Maximum supported base-2 logarithm.
        max: u8,
    },
    /// A rectangular intra prediction block dimension is outside the modeled range.
    #[error(
        "unsupported rectangular intra block log2 size {log2_width}x{log2_height}; expected each dimension {min} through {max}"
    )]
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
    #[error("intra prediction {} edge length mismatch: expected {expected} samples, got {actual}", .edge.name())]
    IntraPredictionEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraDcEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// An intra prediction edge sample exceeded the active bit depth.
    #[error("intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}", .edge.name())]
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
    #[error("intra prediction output sample {sample_index} value {value} exceeds maximum {max}")]
    IntraPredictionOutputSampleOutOfRange {
        /// Zero-based sample index within the caller-owned strided output buffer.
        sample_index: usize,
        /// Observed sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A supplied PAETH intra prediction edge did not match the block size.
    #[error("PAETH intra prediction {} edge length mismatch: expected {expected} samples, got {actual}", .edge.name())]
    IntraPaethEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraPaethEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A PAETH intra prediction edge sample exceeded the active bit depth.
    #[error("PAETH intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}", .edge.name())]
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
    #[error("cardinal directional intra prediction {} mode requires {} edge", .direction.name(), .edge.name())]
    IntraCardinalEdgeUnavailable {
        /// Cardinal prediction direction being computed.
        direction: IntraCardinalDirection,
        /// Required edge that was absent.
        edge: IntraCardinalEdge,
    },
    /// A supplied cardinal directional intra prediction edge did not match the block size.
    #[error("cardinal directional intra prediction {} edge length mismatch: expected {expected} samples, got {actual}", .edge.name())]
    IntraCardinalEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraCardinalEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A cardinal directional intra prediction edge sample exceeded the active bit depth.
    #[error("cardinal directional intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}", .edge.name())]
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
    #[error(
        "unsupported one-sided directional intra prediction pAngle {p_angle}; expected 45, 67, or 203"
    )]
    UnsupportedIntraDirectionalAngle {
        /// Rejected AV2 pAngle value.
        p_angle: u16,
    },
    /// A required one-sided directional-angle prediction edge was absent.
    #[error("directional angle intra prediction pAngle {} requires {} edge", .angle.p_angle(), .edge.name())]
    IntraDirectionalAngleEdgeUnavailable {
        /// Directional pAngle being computed.
        angle: IntraDirectionalAngle,
        /// Required edge that was absent.
        edge: IntraDirectionalAngleEdge,
    },
    /// A supplied one-sided directional-angle edge did not match the block size.
    #[error("directional angle intra prediction {} edge length mismatch: expected {expected} samples, got {actual}", .edge.name())]
    IntraDirectionalAngleEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraDirectionalAngleEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A one-sided directional-angle edge sample exceeded the active bit depth.
    #[error("directional angle intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}", .edge.name())]
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
    #[error(
        "unsupported middle directional intra prediction pAngle {p_angle}; expected 113, 135, or 157"
    )]
    UnsupportedIntraMiddleDirectionalAngle {
        /// Rejected AV2 pAngle value.
        p_angle: u16,
    },
    /// A required middle directional-angle prediction edge was absent.
    #[error("middle directional angle intra prediction pAngle {} requires {} edge", .angle.p_angle(), .edge.name())]
    IntraMiddleDirectionalAngleEdgeUnavailable {
        /// Directional pAngle being computed.
        angle: IntraMiddleDirectionalAngle,
        /// Required edge that was absent.
        edge: IntraDirectionalAngleEdge,
    },
    /// A supplied middle directional-angle edge did not match the block size.
    #[error("middle directional angle intra prediction {} edge length mismatch: expected {expected} samples, got {actual}", .edge.name())]
    IntraMiddleDirectionalAngleEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraDirectionalAngleEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A middle directional-angle edge sample exceeded the active bit depth.
    #[error("middle directional angle intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}", .edge.name())]
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
    #[error("smooth intra prediction {} edge length mismatch: expected {expected} samples, got {actual}", .edge.name())]
    IntraSmoothEdgeLengthMismatch {
        /// Edge whose sample count was checked.
        edge: IntraSmoothEdge,
        /// Expected edge sample count.
        expected: usize,
        /// Actual edge sample count.
        actual: usize,
    },
    /// A smooth intra prediction edge sample exceeded the active bit depth.
    #[error("smooth intra prediction {} edge sample {sample_index} value {value} exceeds maximum {max}", .edge.name())]
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
    #[error(
        "smooth intra prediction sample at row {row} column {column} value {value} is outside {min}..={max}"
    )]
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
    #[error("failed to allocate {context}")]
    IntraPredictionAllocationFailed {
        /// Short description of the failed allocation.
        context: &'static str,
    },
    /// A caller-provided intra prediction output stride was too small.
    #[error(
        "intra prediction output stride {stride_samples} samples is smaller than prediction width {width}"
    )]
    IntraPredictionStrideTooSmall {
        /// Supplied output stride in samples.
        stride_samples: usize,
        /// Required prediction width in samples.
        width: usize,
    },
    /// A caller-provided intra prediction output buffer was too small.
    #[error(
        "intra prediction output buffer is too small: expected at least {expected} samples, got {actual}"
    )]
    IntraPredictionOutputTooSmall {
        /// Minimum required sample count for the supplied block and stride.
        expected: usize,
        /// Actual output slice length.
        actual: usize,
    },
    /// A workspace intra prediction helper could not read a required edge.
    #[error("current-frame workspace {} intra prediction requires {} edge for rectangle x={} y={} width={} height={}", .plane.name(), .edge.name(), .rect.x(), .rect.y(), .rect.width(), .rect.height())]
    WorkspaceIntraPredictionEdgeUnavailable {
        /// Plane whose workspace storage was checked.
        plane: PlaneId,
        /// Required edge that was outside workspace storage.
        edge: IntraPaethEdge,
        /// Prediction rectangle needing the edge.
        rect: PlaneRect,
    },
    /// A workspace smooth intra helper could not read a required prepared edge.
    #[error("current-frame workspace {} smooth intra prediction requires {} edge for rectangle x={} y={} width={} height={}", .plane.name(), .edge.name(), .rect.x(), .rect.y(), .rect.width(), .rect.height())]
    WorkspaceSmoothIntraPredictionEdgeUnavailable {
        /// Plane whose workspace storage was checked.
        plane: PlaneId,
        /// Required edge that was outside workspace storage.
        edge: IntraSmoothEdge,
        /// Prediction rectangle needing the edge.
        rect: PlaneRect,
    },
    /// A workspace cardinal directional intra helper could not read a required edge.
    #[error("current-frame workspace {} cardinal directional intra prediction requires {} edge for rectangle x={} y={} width={} height={}", .plane.name(), .edge.name(), .rect.x(), .rect.y(), .rect.width(), .rect.height())]
    WorkspaceCardinalIntraPredictionEdgeUnavailable {
        /// Plane whose workspace storage was checked.
        plane: PlaneId,
        /// Required edge that was outside workspace storage.
        edge: IntraCardinalEdge,
        /// Prediction rectangle needing the edge.
        rect: PlaneRect,
    },
    /// A workspace directional-angle helper could not read a required edge.
    #[error("current-frame workspace {} directional angle intra prediction pAngle {} requires {} edge for rectangle x={} y={} width={} height={}", .plane.name(), .p_angle, .edge.name(), .rect.x(), .rect.y(), .rect.width(), .rect.height())]
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
    #[error("current-frame workspace {} directional angle intra prediction pAngle {} requires luma IDIF for rectangle x={} y={} width={} height={}", .plane.name(), .p_angle, .rect.x(), .rect.y(), .rect.width(), .rect.height())]
    WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported {
        /// Luma plane whose workspace storage was rejected.
        plane: PlaneId,
        /// Directional pAngle being computed.
        p_angle: u16,
        /// Prediction rectangle needing luma IDIF.
        rect: PlaneRect,
    },
    /// A reference frame store capacity was outside the supported slot range.
    #[error("reference frame store capacity {capacity} is outside 1..={max_slots}")]
    InvalidReferenceStoreCapacity {
        /// Requested store capacity.
        capacity: usize,
        /// Maximum supported store capacity.
        max_slots: usize,
    },
    /// A reference slot index was outside the source-backed slot ceiling.
    #[error("reference slot index {index} is outside 0..{max_slots}")]
    InvalidReferenceSlotIndex {
        /// Requested reference slot index.
        index: usize,
        /// Maximum supported slot count.
        max_slots: usize,
    },
    /// A caller-derived reference refresh mask selected unsupported bits.
    #[error("reference refresh mask 0x{mask:08x} contains bits outside 0..{max_slots}")]
    InvalidReferenceRefreshMask {
        /// Requested refresh mask bits.
        mask: u32,
        /// Maximum supported slot count.
        max_slots: usize,
    },
    /// A valid reference slot was outside a particular store's capacity.
    #[error("reference slot {} is outside store capacity {capacity}", .slot.index())]
    ReferenceSlotOutOfBounds {
        /// Requested reference slot.
        slot: ReferenceSlot,
        /// Store capacity used for the bounds check.
        capacity: usize,
    },
    /// A valid refresh mask selected a slot outside a store's capacity.
    #[error("reference refresh mask 0x{mask:08x} selects a slot outside store capacity {capacity}")]
    ReferenceRefreshMaskOutOfBounds {
        /// Requested refresh mask bits.
        mask: u32,
        /// Store capacity used for the bounds check.
        capacity: usize,
    },
    /// An inverse transform was given an unsupported 1D length.
    #[error("unsupported inverse transform length {size}; expected 4, 8, 16, or 32")]
    InvalidInverseTransformSize {
        /// Supplied source length.
        size: usize,
    },
    /// An inverse transform output length did not match the source length.
    #[error("inverse transform output length {out_len} does not match source length {src_len}")]
    InverseTransformLengthMismatch {
        /// Source coefficient count.
        src_len: usize,
        /// Output buffer length.
        out_len: usize,
    },
    /// A reconstruct residual-addition call had mismatched buffer lengths.
    #[error(
        "reconstruct length mismatch: prediction {prediction_len}, residual {residual_len}, output {out_len}"
    )]
    ReconstructLengthMismatch {
        /// Prediction sample count.
        prediction_len: usize,
        /// Residual sample count.
        residual_len: usize,
        /// Output buffer length.
        out_len: usize,
    },
    /// A reconstruct prediction sample exceeded the active decoded bit depth.
    #[error("reconstruct prediction sample {sample_index} value {value} exceeds maximum {max}")]
    ReconstructPredictionOutOfRange {
        /// Zero-based index of the out-of-range prediction sample.
        sample_index: usize,
        /// Observed prediction sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A 2D inverse transform was given an unsupported original log2 shape.
    #[error(
        "unsupported 2D inverse transform log2 shape {log2_w}x{log2_h} (lossless={lossless}); expected each log2 in 2..=6, and 2x2 (4x4) when lossless"
    )]
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
    #[error(
        "no Transform_Shift entry for log2 shape {log2_width}x{log2_height}; expected one of the 25 AV2 TX_SIZES_ALL shapes"
    )]
    InvalidTransformShiftShape {
        /// Requested transform width base-2 logarithm.
        log2_width: u32,
        /// Requested transform height base-2 logarithm.
        log2_height: u32,
    },
    /// A § 7.15.4 `get_transform_1d_type` lookup was requested with a
    /// `PlaneTxType` outside the valid `TX_TYPES` range (`0..16`).
    #[error("invalid PlaneTxType {plane_tx_type}; expected a TX_TYPES index in 0..16")]
    InvalidPlaneTxType {
        /// Requested `PlaneTxType` index.
        plane_tx_type: usize,
    },
    /// A § 5.20.7.30 `get_scan` request had an unsupported transform shape
    /// (`w` / `h` must each be 4, 8, 16, or 32).
    #[error("unsupported get_scan shape {w}x{h}; expected each of w/h in 4/8/16/32")]
    InvalidScanShape {
        /// Requested operating width.
        w: usize,
        /// Requested operating height.
        h: usize,
    },
    /// A § 5.20.7.30 `get_scan` output buffer length did not match `w * h`.
    #[error("get_scan output length mismatch: expected {expected}, got {out_len}")]
    ScanLengthMismatch {
        /// Expected length (`w * h`).
        expected: usize,
        /// Supplied output buffer length.
        out_len: usize,
    },
    /// A 2D inverse transform input/output buffer length did not match `w * h`.
    #[error(
        "2D inverse transform buffer mismatch: expected {expected}, dequant {dequant_len}, residual {residual_len}"
    )]
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
    #[error(
        "2D outer inverse transform buffer mismatch: dequant expected {dequant_expected} (got {dequant_len}), residual expected {residual_expected} (got {residual_len})"
    )]
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
    #[error("unsupported secondary transform shape {w}x{h}; expected each side 4, 8, 16, or 32")]
    SecondaryTransformInvalidShape {
        /// Supplied operating width.
        w: usize,
        /// Supplied operating height.
        h: usize,
    },
    /// A § 7.15.3 secondary transform `dequant` buffer length did not match
    /// `w * h`.
    #[error("secondary transform buffer mismatch: expected {expected}, got {actual}")]
    SecondaryTransformBufferMismatch {
        /// Expected length (`w * h`).
        expected: usize,
        /// Supplied dequantized-coefficient buffer length.
        actual: usize,
    },
    /// A § 7.15.3 secondary transform parameter was out of range for the selected
    /// kernel set: `n` exceeds the kernel height, `kernel` exceeds the set size,
    /// or `sec_tx_type` is not in `1..=3`.
    #[error(
        "invalid secondary transform params: n {n}, kernel {kernel}, sec_tx_type {sec_tx_type}"
    )]
    SecondaryTransformInvalidParams {
        /// Supplied input coefficient count (`n`).
        n: usize,
        /// Supplied kernel-set index.
        kernel: usize,
        /// Supplied secondary transform type (`sec_tx_type`).
        sec_tx_type: usize,
    },
    /// A § 7.17.7.1 deblocking sample filter per-side width was not in `1..=8`.
    #[error(
        "invalid deblocking filter widths: neg {max_width_neg}, pos {max_width_pos}; expected each in 1..=8"
    )]
    DeblockFilterInvalidWidth {
        /// Supplied previous-side maximum width (`maxWidthNeg`).
        max_width_neg: usize,
        /// Supplied current-side maximum width (`maxWidthPos`).
        max_width_pos: usize,
    },
    /// A § 7.17.7.1 deblocking sample filter line did not contain the previous-
    /// and current-side samples the filter reads and writes around `boundary`.
    #[error(
        "deblocking filter line too short: boundary {boundary}, max_width_neg {max_width_neg}, width {width}, len {len}"
    )]
    DeblockFilterLineTooShort {
        /// Supplied current-side base index (`boundary`).
        boundary: usize,
        /// Supplied previous-side maximum width (`maxWidthNeg`).
        max_width_neg: usize,
        /// Derived filter width (`Max(maxWidthNeg, maxWidthPos)`).
        width: usize,
        /// Supplied sample-line length.
        len: usize,
    },
    /// A § 7.20.3 Wiener NS filter output stride was smaller than the block width.
    #[error(
        "Wiener NS filter output stride {stride_samples} samples is smaller than block width {width}"
    )]
    WienerNsFilterOutputStrideTooSmall {
        /// Supplied output stride in samples.
        stride_samples: usize,
        /// Required block width in samples.
        width: usize,
    },
    /// A § 7.20.3 Wiener NS filter output buffer did not contain the strided
    /// block area.
    #[error(
        "Wiener NS filter output buffer too small: expected at least {expected} samples, got {actual}"
    )]
    WienerNsFilterOutputTooSmall {
        /// Minimum required sample count.
        expected: usize,
        /// Supplied output buffer length.
        actual: usize,
    },
    /// A § 7.20.3 Wiener NS filter was supplied with no coefficient classes.
    #[error("Wiener NS filter requires at least one coefficient class")]
    WienerNsFilterMissingClasses,
    /// A § 7.20.3 Wiener NS luma subclass map did not cover every output sample.
    #[error("Wiener NS subclass map too short: expected at least {expected} entries, got {actual}")]
    WienerNsFilterSubclassMapTooShort {
        /// Required subclass count (`width * height`).
        expected: usize,
        /// Supplied subclass count.
        actual: usize,
    },
    /// A § 7.20.3 Wiener NS luma subclass index exceeded the supplied coefficient
    /// class count.
    #[error(
        "Wiener NS subclass {subclass} at sample {sample_index} is outside {classes} coefficient classes"
    )]
    WienerNsFilterSubclassOutOfRange {
        /// Zero-based output sample index whose subclass was invalid.
        sample_index: usize,
        /// Supplied subclass index.
        subclass: usize,
        /// Supplied coefficient class count.
        classes: usize,
    },
    /// A source sample supplied to a § 7.20.3 Wiener NS filter exceeded the active
    /// decoded bit-depth range.
    #[error(
        "Wiener NS source sample at ({x}, {y}) has value {value}, exceeding active bit-depth max {max}"
    )]
    WienerNsFilterSourceSampleOutOfRange {
        /// Block-relative source x coordinate requested by the tap.
        x: isize,
        /// Block-relative source y coordinate requested by the tap.
        y: isize,
        /// Observed source sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A § 7.20.3 Wiener NS chroma filter was supplied an invalid
    /// `cfl_ds_filter_index`.
    #[error("invalid Wiener NS chroma cfl_ds_filter_index {index}; expected 0..=3")]
    WienerNsFilterInvalidCflDsFilterIndex {
        /// Supplied index.
        index: u8,
    },
    /// Caller-resolved AV2 § 7.20.4 PC-Wiener classification bounds were
    /// internally inconsistent.
    #[error("invalid PC-Wiener classification {field}")]
    PcWienerInvalidBounds {
        /// Invalid field or derived range.
        field: &'static str,
    },
    /// A source sample supplied to AV2 § 7.20.4 PC-Wiener classification exceeded
    /// the active decoded bit-depth range.
    #[error(
        "PC-Wiener source sample at ({x}, {y}) has value {value}, exceeding active bit-depth max {max}"
    )]
    PcWienerSourceSampleOutOfRange {
        /// Source x coordinate requested by the classification feature.
        x: isize,
        /// Source y coordinate requested by the classification feature.
        y: isize,
        /// Observed source sample value.
        value: u16,
        /// Maximum sample value allowed by the active bit depth.
        max: u16,
    },
    /// A caller-resolved AV2 § 7.20.4 `LrTxSkip` value was outside the boolean
    /// domain.
    #[error(
        "invalid PC-Wiener LrTxSkip value {value} at sample ({x}, {y}) grid ({row}, {col}); expected 0 or 1"
    )]
    PcWienerInvalidTxSkip {
        /// Luma sample x coordinate after § 7.20.4 clipping.
        x: usize,
        /// Luma sample y coordinate after § 7.20.4 clipping.
        y: usize,
        /// Zero-based `LrTxSkip` row.
        row: usize,
        /// Zero-based `LrTxSkip` column.
        col: usize,
        /// Supplied value.
        value: i32,
    },
    /// Caller-resolved AV2 § 7.20.2 loop-restoration source-sample luma bounds
    /// were internally inconsistent.
    #[error("invalid loop-restoration source-sample {field}")]
    LoopRestorationSourceInvalidBounds {
        /// Invalid field or derived range.
        field: &'static str,
    },
    /// Caller-resolved AV2 § 7.20.2 chroma subsampling values were outside the
    /// AV2 `0..=1` domain.
    #[error(
        "invalid loop-restoration source-sample subsampling ({subsampling_x}, {subsampling_y}); expected each in 0..=1"
    )]
    LoopRestorationSourceInvalidSubsampling {
        /// Supplied `SubsamplingX`.
        subsampling_x: u8,
        /// Supplied `SubsamplingY`.
        subsampling_y: u8,
    },
    /// Caller-resolved AV2 § 7.20.2 chroma subsampling values did not match
    /// the selected source frame format.
    #[error("loop-restoration source-sample {} subsampling ({subsampling_x}, {subsampling_y}) does not match frame format ({expected_x}, {expected_y})", .plane.name())]
    LoopRestorationSourceSubsamplingMismatch {
        /// Chroma plane whose source sample was requested.
        plane: PlaneId,
        /// Supplied `SubsamplingX`.
        subsampling_x: u8,
        /// Supplied `SubsamplingY`.
        subsampling_y: u8,
        /// Expected `SubsamplingX` for the source frame format.
        expected_x: u8,
        /// Expected `SubsamplingY` for the source frame format.
        expected_y: u8,
    },
    /// The caller supplied different frame metadata for the § 7.20.2
    /// `CurrFrame` and `CdefFrame` source views.
    #[error("loop-restoration source-sample {field} mismatch between CurrFrame and CdefFrame")]
    LoopRestorationSourceFrameMismatch {
        /// Mismatched frame field.
        field: &'static str,
    },
    /// A caller-resolved AV2 § 7.20.2 loop-restoration source sample fell
    /// outside the selected frame view's coded plane storage.
    #[error("loop-restoration source-sample {} coordinate ({x}, {y}) is outside coded plane {width}x{height}", .plane.name())]
    LoopRestorationSourceSampleOutOfBounds {
        /// Plane whose source sample was requested.
        plane: PlaneId,
        /// Requested coded-plane x coordinate.
        x: usize,
        /// Requested coded-plane y coordinate.
        y: usize,
        /// Coded plane width.
        width: usize,
        /// Coded plane height.
        height: usize,
    },
    /// A § 7.14.4 dequantization block had an unsupported transform shape.
    #[error(
        "unsupported dequantization block shape {tx_width}x{tx_height}; expected each side 4, 8, 16, or 32"
    )]
    InvalidDequantBlockShape {
        /// Supplied dequantized transform-block width.
        tx_width: usize,
        /// Supplied dequantized transform-block height.
        tx_height: usize,
    },
    /// A § 7.14.4 dequantization `quant` / `out` buffer length did not match
    /// `tx_width * tx_height`.
    #[error(
        "dequantization block buffer mismatch: expected {expected}, quant {quant_len}, out {out_len}"
    )]
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
    #[error(
        "quantization-matrix index out of range: seg_level {seg_level}, qm_offset {qm_offset}, position {position}"
    )]
    InvalidQuantizerMatrixIndex {
        /// Requested `segLvl` segment level.
        seg_level: usize,
        /// Caller-resolved `Qm_Offset[txSz]` region start within the matrix row.
        qm_offset: usize,
        /// Derived flattened position within the matrix row.
        position: usize,
    },
    /// A § 7.13.3.18 sub-pel motion-compensation reference-plane sample buffer
    /// length did not equal `width * height`.
    #[error("subpel reference plane buffer mismatch: expected {expected} samples, got {actual}")]
    SubpelReferencePlaneMismatch {
        /// Expected length (`width * height`).
        expected: usize,
        /// Supplied reference-plane sample-buffer length.
        actual: usize,
    },
    /// A § 7.13.3.18 sub-pel motion-compensation block dimension exceeded the
    /// supported maximum (a 128-sample super-block side).
    #[error("unsupported subpel block dimension {w}x{h}; each side must be at most 128")]
    SubpelBlockDimensionUnsupported {
        /// Supplied block width (`w`).
        w: usize,
        /// Supplied block height (`h`).
        h: usize,
    },
    /// A § 7.13.3.18 sub-pel motion-compensation step was negative (the
    /// § 7.13.3.17 scaling steps are non-negative).
    #[error(
        "subpel motion-compensation step must be non-negative: step_x {step_x}, step_y {step_y}"
    )]
    SubpelNegativeStep {
        /// Supplied horizontal step (`stepX`).
        step_x: i64,
        /// Supplied vertical step (`stepY`).
        step_y: i64,
    },
    /// A § 7.13.3.18 sub-pel vertical-pass intermediate row index reached past
    /// the derived `intermediateHeight`.
    #[error(
        "subpel vertical-pass base row {base} exceeds intermediate height {intermediate_height}"
    )]
    SubpelIntermediateOutOfRange {
        /// The vertical-pass base row (`p >> SCALE_SUBPEL_BITS`).
        base: usize,
        /// The derived intermediate-array height.
        intermediate_height: usize,
    },
    /// Two § 7.13.3.18 compound predictors being blended did not cover the same
    /// number of output samples.
    #[error(
        "compound blend predictor length mismatch: left {left_len} samples, right {right_len} samples"
    )]
    CompoundBlendLengthMismatch {
        /// Left predictor sample count.
        left_len: usize,
        /// Right predictor sample count.
        right_len: usize,
    },
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "error_tests.rs"]
mod tests;
