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
    /// The supplied decode source recorded a fatal source/container parse issue.
    #[error("malformed decode source: {issue}")]
    MalformedSource {
        /// Source issue that prevented transactional planning.
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
    /// Runtime reconstruction model construction failed after tier validation.
    #[error("decode reconstruction failed: {source}")]
    Reconstruction {
        /// Underlying reconstruction model error.
        #[from]
        source: splot_recon::ReconError,
    },
    /// Decode output serialization or caller-writer I/O failed.
    #[error("decode output failed: {source}")]
    Output {
        /// Output serialization or write failure.
        #[from]
        source: DecodeOutputError,
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

    /// Creates a caller-writer output error.
    #[must_use]
    pub const fn io(operation: DecodeOutputOperation, source: io::Error) -> Self {
        Self::Io { operation, source }
    }

    /// Returns the stable operation identifier.
    #[must_use]
    pub const fn operation(&self) -> DecodeOutputOperation {
        match self {
            Self::Y4m { operation, .. } | Self::Io { operation, .. } => *operation,
        }
    }

    /// Returns the stable source category.
    #[must_use]
    pub const fn source_kind(&self) -> &'static str {
        match self {
            Self::Y4m { .. } => "y4m",
            Self::Io { .. } => "io",
        }
    }

    /// Returns the underlying source error message.
    #[must_use]
    pub fn source_message(&self) -> String {
        match self {
            Self::Y4m { source, .. } => source.to_string(),
            Self::Io { source, .. } => source.to_string(),
        }
    }
}
