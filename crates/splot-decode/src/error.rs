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
    /// Parsed header state was internally inconsistent.
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

/// Runtime parsed-header state consistency failure.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeHeaderStateError {
    /// A successfully parsed inter frame did not carry complete frame-header state.
    #[error("inter-frame header state is incomplete")]
    IncompleteInterFrame,
    /// An inter frame required its parsed control region, but it was absent.
    #[error("inter-frame control region is missing")]
    MissingInterControlRegion,
    /// An inter frame required its parsed coding-mode tail, but it was absent.
    #[error("inter-frame coding-mode tail is missing")]
    MissingInterTail,
    /// An inter frame required its parsed interpolation filter, but it was absent.
    #[error("inter-frame interpolation filter is missing")]
    MissingInterpolationFilter,
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
    /// A frame's parsed width or height was zero.
    #[error("frame dimensions must be nonzero")]
    ZeroFrameSize,
    /// An inter frame's reference count was invalid or did not match its map length.
    #[error("inter-frame reference count and map are inconsistent")]
    InvalidInterReferenceMap,
    /// A frame required sequence-level quantizer configuration that was absent.
    #[error("sequence transform, quantizer, and entropy configuration is missing")]
    MissingSequenceTransformQuantEntropy,
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

    /// Returns true when this operation belongs to the minimal raw output path.
    #[must_use]
    pub const fn is_raw(self) -> bool {
        matches!(
            self,
            Self::SerializeRaw
                | Self::WriteRawStream
                | Self::ResolveRawOutputPath
                | Self::CreateRawTempFile
                | Self::WriteRawTempFile
                | Self::FlushRawTempFile
                | Self::SyncRawTempFile
                | Self::RenameRawOutput
                | Self::SyncRawOutputDirectory
                | Self::CleanupRawTempFile
        )
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
