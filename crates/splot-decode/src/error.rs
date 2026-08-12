// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot decode driver.

use core::fmt;
use std::io;

use splot_core::span::ByteOffset;

use crate::bitstream::stream_plan::{DecodeSourceIssue, DecodeUnsupportedStructure};
use crate::limits::DecodeLimitError;

/// An error from the splot decode driver.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The decode worker pool could not be constructed.
    #[error("failed to construct the decode worker pool: {source}")]
    Pool {
        /// The underlying parallel-runtime error.
        #[from]
        source: splot_parallel::ParallelError,
    },
    /// A decode source failed a local decode resource policy.
    #[error("decode stream plan rejected by resource limit: {source}")]
    Limit {
        /// The underlying limit failure.
        #[from]
        source: DecodeLimitError,
    },
    /// The supplied decode source recorded a fatal container or bitstream parse issue.
    #[error("malformed decode source: {issue}")]
    MalformedSource {
        /// Source issue that prevented planning or runtime syntax decode.
        issue: DecodeSourceIssue,
    },
    /// The supplied source uses AV2 structures outside the supported planner tier.
    #[error("unsupported decode stream structure: {unsupported}")]
    UnsupportedStructure {
        /// Unsupported structure metadata.
        unsupported: DecodeUnsupportedStructure,
    },
    /// The supplied source is valid enough to plan, but outside the current
    /// runtime hash tier.
    #[error("unsupported decode runtime feature: {unsupported}")]
    UnsupportedFeature {
        /// Unsupported runtime feature metadata.
        unsupported: Box<DecodeUnsupportedFeature>,
    },
    /// A validated decode pipeline invariant failed at runtime.
    #[error("internal decode state `{reason}` failed at byte {byte_offset}")]
    InternalState {
        /// Stable internal-state reason id.
        reason: &'static str,
        /// Byte offset associated with the failed invariant.
        byte_offset: ByteOffset,
    },
    /// Parsed header or derived runtime decode state was internally inconsistent.
    #[error("decode header state failed: {source}")]
    HeaderState {
        /// Underlying header-state consistency failure.
        #[from]
        source: DecodeHeaderStateError,
    },
    /// Runtime reconstruction model construction failed after tier validation.
    #[error("decode reconstruction failed: {source}")]
    Reconstruction {
        /// Underlying reconstruction model error.
        #[from]
        source: splot_recon::ReconError,
    },
    /// Runtime reference-frame state was internally inconsistent.
    #[error("decode reference state failed: {source}")]
    ReferenceState {
        /// Underlying reference-state consistency failure.
        #[from]
        source: DecodeReferenceStateError,
    },
    /// Decode output serialization or caller-writer I/O failed.
    #[error("decode output failed: {source}")]
    Output {
        /// Output serialization or write failure.
        #[from]
        source: DecodeOutputError,
    },
}

/// Runtime parsed-header or derived decode-state consistency failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeHeaderStateError {
    /// A decoded general-intra mode escaped its typed AV2 domain.
    #[error("general-intra mode state is inconsistent")]
    InvalidGeneralIntraModeState,
    /// A canonical three-symbol MHCCP direction decoded outside its typed domain.
    #[error("general-intra MHCCP direction state is inconsistent")]
    InvalidGeneralIntraMhccpDirection,
    /// A complete intra header contradicted its tile-group carrier or frame-kind facts.
    #[error("intra-only tile-group header state is inconsistent")]
    InvalidIntraOnlyTileGroupState,
    /// A successfully parsed inter frame did not carry complete frame-header state.
    #[error("inter-frame header state is incomplete")]
    IncompleteInterFrame,
    /// An inter tile's parser was reused after its traversal had finished.
    #[error("inter-tile traversal state is inconsistent")]
    InvalidInterTileTraversalState,
    /// A complete inter header was missing mandatory frame-tool state.
    #[error("inter-frame tool state is incomplete")]
    IncompleteInterFrameTools,
    /// An inter frame required its parsed control region, but it was absent.
    #[error("inter-frame control region is missing")]
    MissingInterControlRegion,
    /// An inter frame required its parsed coding-mode tail, but it was absent.
    #[error("inter-frame coding-mode tail is missing")]
    MissingInterTail,
    /// An inter frame required its parsed interpolation filter, but it was absent.
    #[error("inter-frame interpolation filter is missing")]
    MissingInterpolationFilter,
    /// A complete ordinary inter header was missing its derived frame MV precision.
    #[error("inter-frame motion-vector precision is missing")]
    MissingInterMotionVectorPrecision,
    /// An inter frame carried an interpolation-filter variant unknown to this decoder.
    #[error("inter-frame interpolation filter is invalid")]
    InvalidInterpolationFilter,
    /// An inter frame required its derived display order hint, but it was absent.
    #[error("inter-frame display order hint is missing")]
    MissingDisplayOrderHint,
    /// A successfully parsed show-existing frame was missing a mandatory derived field.
    #[error("show-existing-frame header state is incomplete")]
    IncompleteShowExistingFrame,
    /// A successfully parsed TIP-output frame was missing mandatory derived state.
    #[error("TIP-output frame header state is incomplete")]
    IncompleteTipOutput,
    /// A frame required its parsed dimensions, but they were absent.
    #[error("frame size is missing")]
    MissingFrameSize,
    /// A frame reached output-effect validation without both output flags.
    #[error("frame output classification is missing")]
    MissingFrameOutputClassification,
    /// Multiple pending BRTs did not retain the second OBU's source offset.
    #[error("buffer-removal-timing source offset is missing")]
    MissingBufferRemovalTimingOffset,
    /// A frame's parsed width or height was zero.
    #[error("frame dimensions must be nonzero")]
    ZeroFrameSize,
    /// A split inter walk received a tile plan inconsistent with its validated header.
    #[error("split inter walk requires exactly one tile, got {actual}")]
    InvalidSplitTileCount {
        /// Number of tile work units materialized by the validated payload plan.
        actual: usize,
    },
    /// A validated block-size value could not produce its table-defined geometry.
    #[error("block geometry is inconsistent with the decoded block-size domain")]
    InvalidBlockGeometry,
    /// A decoded frame could not materialize its § 7.23 segmentation map.
    #[error("frame segmentation map is unavailable")]
    MissingSegmentIdMap,
    /// Validated frame geometry produced an empty segmentation-map dimension.
    #[error("frame segmentation map dimensions must be nonzero, got {mi_rows}x{mi_cols}")]
    InvalidSegmentIdMapDimensions {
        /// Frame height in 4x4 luma units.
        mi_rows: usize,
        /// Frame width in 4x4 luma units.
        mi_cols: usize,
    },
    /// Validated frame geometry overflowed while sizing its segmentation map.
    #[error("frame segmentation map arithmetic overflow in {operation}: {left} * {right}")]
    SegmentIdMapSizeOverflow {
        /// Sizing operation that overflowed.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// An inter frame's reference count was invalid or did not match its map length.
    #[error("inter-frame reference count and map are inconsistent")]
    InvalidInterReferenceMap,
    /// The CDF selector or row used by single-reference syntax was internally inconsistent.
    #[error("single-reference entropy CDF state is inconsistent")]
    InvalidSingleReferenceCdfState,
    /// A frame required sequence-level quantizer configuration that was absent.
    #[error("sequence transform, quantizer, and entropy configuration is missing")]
    MissingSequenceTransformQuantEntropy,
    /// Decoded inter residual records were inconsistent at reconstruction time.
    #[error("inter residual reconstruction state is inconsistent")]
    InvalidInterResidualReconstruction,
    /// A decoded inter CCTX record did not have its paired chroma block.
    #[error("inter residual CCTX pair state is inconsistent")]
    MissingInterResidualCctxPair,
    /// Derived selectable transform-record state was internally inconsistent.
    #[error("selectable transform-record derivation state is inconsistent")]
    InvalidSelectableTransformRecords,
    /// A palette symbol read encountered invalid CDF or arithmetic-decoder state.
    #[error("general-intra palette entropy state is inconsistent")]
    InvalidGeneralIntraPaletteEntropyState,
    /// A decoded palette index escaped its CDF-derived palette-size domain.
    #[error("general-intra palette color-index state is inconsistent")]
    InvalidGeneralIntraPaletteColorState,
    /// A general-intra coefficient read encountered invalid CDF or arithmetic-decoder state.
    #[error("general-intra coefficient entropy state is inconsistent")]
    InvalidGeneralIntraCoefficientEntropyState,
    /// Derived general-intra coefficient geometry, table, scan, or context state was inconsistent.
    #[error("general-intra coefficient state is inconsistent")]
    InvalidGeneralIntraCoefficientState,
    /// General-intra tile coefficient neighbour state was internally inconsistent.
    #[error("general-intra coefficient context state is inconsistent")]
    InvalidGeneralIntraCoefficientContextState,
    /// General-intra prediction, dequantization, or transform state was inconsistent.
    #[error("general-intra reconstruction state is inconsistent")]
    InvalidGeneralIntraReconstructionState,
    /// General-intra directional edge geometry or derived neighbour state was inconsistent.
    #[error("general-intra directional edge state is inconsistent")]
    InvalidGeneralIntraDirectionalEdgeState,
    /// Derived inter warp geometry, divisor, delta, or translation state was inconsistent.
    #[error("inter warp model state is inconsistent")]
    InvalidInterWarpModelState,
    /// Derived inter-intra wedge angle or table state was inconsistent.
    #[error("inter-intra wedge state is inconsistent")]
    InvalidInterWedgeState,
    /// An admitted EXTENDWARP neighbour was absent from resolved motion state.
    #[error("EXTENDWARP neighbour motion state is inconsistent")]
    InvalidExtendWarpNeighbourState,
    /// Derived GDF filter state was internally inconsistent.
    #[error("GDF filter derivation state is inconsistent")]
    InvalidGdfFilterState,
    /// The loop-restoration filter pipeline state was internally inconsistent.
    #[error("loop-restoration filter pipeline state is inconsistent")]
    InvalidLoopRestorationFilterState,
    /// An SDP chroma leaf was reached before its collocated luma mode was published.
    #[error("SDP chroma block at ({mi_row}, {mi_col}) is missing collocated luma mode state")]
    MissingSdpLumaModeState {
        /// Block row in 4x4 luma units.
        mi_row: usize,
        /// Block column in 4x4 luma units.
        mi_col: usize,
    },
}

/// Runtime reference-frame state consistency failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeReferenceStateError {
    /// An inter block selected an entry outside its frame-header reference map.
    #[error("reference-list index {index} is outside the active {list_len}-entry reference map")]
    ReferenceListIndexOutOfRange {
        /// Zero-based reference-list index selected by block syntax.
        index: i8,
        /// Number of entries in the frame-header reference map.
        list_len: usize,
    },
    /// A frame header named a slot outside the active reference buffer.
    #[error("reference slot {slot} is outside the active {slot_count}-slot buffer")]
    SlotOutOfRange {
        /// Zero-based slot index from the frame header.
        slot: usize,
        /// Active reference-slot count.
        slot_count: usize,
    },
    /// A valid reference slot was missing quantizer metadata needed by TIP output.
    #[error("reference slot {slot} has no stored quantizer metadata")]
    MissingQuantizerMetadata {
        /// Zero-based reference slot selected for TIP output.
        slot: usize,
    },
    /// A selected reference slot had no published entropy context.
    #[error("reference slot {slot} has no published CDF context")]
    MissingCdfContext {
        /// Zero-based reference slot selected for cross-frame CDF initialization.
        slot: usize,
    },
    /// A selected reference slot had no saved CCSO parameters.
    #[error("reference slot {slot} has no saved CCSO parameters")]
    MissingCcsoParams {
        /// Zero-based reference slot selected for CCSO parameter reuse.
        slot: usize,
    },
    /// A selected reference had not published the motion field required by TIP output.
    #[error("a selected TIP-output reference has no published motion field")]
    MissingMotionFieldPublication,
    /// A selected or valid reference slot had no decoded frame attached.
    #[error("reference slot {slot} has no stored decoded frame")]
    MissingFrame {
        /// Zero-based reference slot index.
        slot: usize,
    },
    /// A selected reference slot had no readable samples.
    #[error("reference slot {slot} has no readable decoded-frame samples")]
    ReferenceSamplesUnavailable {
        /// Zero-based reference slot index.
        slot: usize,
    },
    /// A slot pointed past the decoded-frame buffer.
    #[error(
        "reference slot {slot} points to decoded-frame index {frame_index}, but only {frame_count} frames are available"
    )]
    FrameIndexOutOfRange {
        /// Zero-based reference slot index.
        slot: usize,
        /// Stored decoded-frame index.
        frame_index: usize,
        /// Number of decoded frames available to the builder.
        frame_count: usize,
    },
    /// A valid slot's frame-size metadata disagreed with its retained frame.
    #[error(
        "reference slot {slot} metadata size {expected_width}x{expected_height} does not match decoded-frame index {frame_index} size {actual_width}x{actual_height}"
    )]
    FrameSizeMismatch {
        /// Zero-based reference slot index.
        slot: usize,
        /// Stored decoded-frame index.
        frame_index: usize,
        /// Slot metadata width in luma samples.
        expected_width: u32,
        /// Slot metadata height in luma samples.
        expected_height: u32,
        /// Retained decoded-frame coded luma width in samples.
        actual_width: usize,
        /// Retained decoded-frame coded luma height in samples.
        actual_height: usize,
    },
    /// A derive-order-hint SEF selected a slot that § 6.17.2 does not permit showing.
    #[error("reference slot {slot} is not eligible for derive-order-hint show-existing output")]
    ShowExistingFrameIneligible {
        /// Zero-based reference slot index.
        slot: usize,
    },
}

/// A specialized [`Result`](core::result::Result) for decode context operations.
pub type Result<T> = core::result::Result<T, DecodeError>;

/// Unsupported runtime feature metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeUnsupportedFeature {
    reason: &'static str,
    spec_section: &'static str,
    message: &'static str,
    byte_offset: Option<ByteOffset>,
}

impl DecodeUnsupportedFeature {
    /// Creates unsupported runtime feature metadata.
    #[must_use]
    pub const fn new(
        reason: &'static str,
        spec_section: &'static str,
        message: &'static str,
        byte_offset: Option<ByteOffset>,
    ) -> Self {
        Self {
            reason,
            spec_section,
            message,
            byte_offset,
        }
    }

    /// Stable unsupported reason.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
    }

    /// AV2 spec section associated with the rejection.
    #[must_use]
    pub const fn spec_section(&self) -> &'static str {
        self.spec_section
    }

    /// Human-readable unsupported message.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// Byte offset associated with the unsupported feature, when known.
    #[must_use]
    pub const fn byte_offset(&self) -> Option<ByteOffset> {
        self.byte_offset
    }
}

impl core::fmt::Display for DecodeUnsupportedFeature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.byte_offset {
            Some(offset) => write!(f, "{} at byte {}: {}", self.reason, offset, self.message),
            None => write!(f, "{}: {}", self.reason, self.message),
        }
    }
}

/// Stable decode output operation that failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeOutputOperation {
    /// Serializing a runtime Y4M stream with the reconstruction writer failed.
    SerializeY4m,
    /// Writing the complete Y4M stream bytes to the caller-provided writer failed.
    WriteY4mStream,
    /// Serializing a runtime raw sample byte stream failed.
    SerializeRaw,
    /// Writing the complete raw sample bytes to the caller-provided writer failed.
    WriteRawStream,
    /// Resolving the requested Y4M output path failed before publication.
    ResolveY4mOutputPath,
    /// Resolving the requested raw output path failed before publication.
    ResolveRawOutputPath,
    /// Creating the same-directory temporary Y4M output file failed.
    CreateY4mTempFile,
    /// Creating the same-directory temporary raw output file failed.
    CreateRawTempFile,
    /// Writing bytes into the temporary Y4M output file failed.
    WriteY4mTempFile,
    /// Writing bytes into the temporary raw output file failed.
    WriteRawTempFile,
    /// Flushing the temporary Y4M output file failed.
    FlushY4mTempFile,
    /// Flushing the temporary raw output file failed.
    FlushRawTempFile,
    /// Syncing the temporary Y4M output file failed.
    SyncY4mTempFile,
    /// Syncing the temporary raw output file failed.
    SyncRawTempFile,
    /// Renaming the temporary file into the requested Y4M output path failed.
    RenameY4mOutput,
    /// Renaming the temporary file into the requested raw output path failed.
    RenameRawOutput,
    /// Reserved identifier for parent-directory durability sync reporting.
    ///
    /// The current CLI attempts that post-rename sync as best-effort, so this
    /// operation is retained for diagnostic identifier stability but is not
    /// emitted by current publication code.
    SyncY4mOutputDirectory,
    /// Reserved identifier for parent-directory durability sync reporting for raw output.
    ///
    /// The current CLI attempts that post-rename sync as best-effort, so this
    /// operation is retained for diagnostic identifier stability but is not
    /// emitted by current publication code.
    SyncRawOutputDirectory,
    /// Removing a failed temporary Y4M output file failed.
    CleanupY4mTempFile,
    /// Removing a failed temporary raw output file failed.
    CleanupRawTempFile,
}

impl DecodeOutputOperation {
    /// Returns the stable snake_case operation identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SerializeY4m => "serialize_y4m",
            Self::WriteY4mStream => "write_y4m_stream",
            Self::SerializeRaw => "serialize_raw",
            Self::WriteRawStream => "write_raw_stream",
            Self::ResolveY4mOutputPath => "resolve_y4m_output_path",
            Self::ResolveRawOutputPath => "resolve_raw_output_path",
            Self::CreateY4mTempFile => "create_y4m_temp_file",
            Self::CreateRawTempFile => "create_raw_temp_file",
            Self::WriteY4mTempFile => "write_y4m_temp_file",
            Self::WriteRawTempFile => "write_raw_temp_file",
            Self::FlushY4mTempFile => "flush_y4m_temp_file",
            Self::FlushRawTempFile => "flush_raw_temp_file",
            Self::SyncY4mTempFile => "sync_y4m_temp_file",
            Self::SyncRawTempFile => "sync_raw_temp_file",
            Self::RenameY4mOutput => "rename_y4m_output",
            Self::RenameRawOutput => "rename_raw_output",
            Self::SyncY4mOutputDirectory => "sync_y4m_output_directory",
            Self::SyncRawOutputDirectory => "sync_raw_output_directory",
            Self::CleanupY4mTempFile => "cleanup_y4m_temp_file",
            Self::CleanupRawTempFile => "cleanup_raw_temp_file",
        }
    }
}

impl fmt::Display for DecodeOutputOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Output serialization or caller-writer failure from a decode API.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeOutputError {
    /// `splot-recon` rejected or failed Y4M serialization.
    #[error("{operation} failed: {source}")]
    Y4m {
        /// Stable operation identifier.
        operation: DecodeOutputOperation,
        /// Underlying Y4M serialization error.
        #[source]
        source: splot_recon::Y4mError,
    },
    /// The decoded frame set cannot be serialized by the requested output format.
    #[error("{operation} failed: {reason}")]
    InvalidFrameSet {
        /// Stable operation identifier.
        operation: DecodeOutputOperation,
        /// Stable reason for the frame-set rejection.
        reason: &'static str,
    },
    /// The caller-provided output writer returned an I/O error.
    #[error("{operation} failed: {source}")]
    Io {
        /// Stable operation identifier.
        operation: DecodeOutputOperation,
        /// Underlying writer error.
        #[source]
        source: io::Error,
    },
}

impl DecodeOutputError {
    /// Creates a Y4M serialization output error.
    #[must_use]
    pub const fn y4m(operation: DecodeOutputOperation, source: splot_recon::Y4mError) -> Self {
        Self::Y4m { operation, source }
    }

    /// Creates an output error for a decoded frame-set shape that cannot be serialized.
    #[must_use]
    pub const fn invalid_frame_set(operation: DecodeOutputOperation, reason: &'static str) -> Self {
        Self::InvalidFrameSet { operation, reason }
    }

    /// Creates a caller-writer output error.
    #[must_use]
    pub const fn io(operation: DecodeOutputOperation, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    /// Returns the stable operation identifier.
    #[must_use]
    pub const fn operation(&self) -> DecodeOutputOperation {
        match self {
            Self::Y4m { operation, .. }
            | Self::InvalidFrameSet { operation, .. }
            | Self::Io { operation, .. } => *operation,
        }
    }

    /// Returns the stable source category.
    #[must_use]
    pub const fn source_kind(&self) -> &'static str {
        match self {
            Self::Y4m { .. } => "y4m",
            Self::InvalidFrameSet { .. } => "frame_set",
            Self::Io { .. } => "io",
        }
    }

    /// Returns the underlying source error message.
    #[must_use]
    pub fn source_message(&self) -> String {
        match self {
            Self::Y4m { source, .. } => source.to_string(),
            Self::InvalidFrameSet { reason, .. } => reason.to_string(),
            Self::Io { source, .. } => source.to_string(),
        }
    }
}
