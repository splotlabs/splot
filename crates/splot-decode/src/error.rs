// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot decode driver.

use core::fmt;
use std::io;

use splot_core::span::ByteOffset;

use crate::limits::DecodeLimitError;
use crate::stream_plan::{DecodeSourceIssue, DecodeUnsupportedStructure};

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
    tier_id: &'static str,
    matrix_row: &'static str,
    feature_id: &'static str,
    spec_section: &'static str,
    message: &'static str,
    remediation: &'static str,
    byte_offset: Option<ByteOffset>,
}

impl DecodeUnsupportedFeature {
    /// Creates unsupported runtime feature metadata.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        reason: &'static str,
        tier_id: &'static str,
        matrix_row: &'static str,
        feature_id: &'static str,
        spec_section: &'static str,
        message: &'static str,
        remediation: &'static str,
        byte_offset: Option<ByteOffset>,
    ) -> Self {
        Self {
            reason,
            tier_id,
            matrix_row,
            feature_id,
            spec_section,
            message,
            remediation,
            byte_offset,
        }
    }

    /// Stable unsupported reason.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
    }

    /// Runtime tier identifier that rejected the source.
    #[must_use]
    pub const fn tier_id(&self) -> &'static str {
        self.tier_id
    }

    /// Decoder support matrix row that owns the rejection.
    #[must_use]
    pub const fn matrix_row(&self) -> &'static str {
        self.matrix_row
    }

    /// Feature ID that owns the rejection.
    #[must_use]
    pub const fn feature_id(&self) -> &'static str {
        self.feature_id
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

    /// Suggested remediation for users.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        self.remediation
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
            Some(offset) => write!(
                f,
                "{} in tier {} at byte {}: {}",
                self.reason, self.tier_id, offset, self.message
            ),
            None => write!(
                f,
                "{} in tier {}: {}",
                self.reason, self.tier_id, self.message
            ),
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
    /// Resolving the requested Y4M output path failed before publication.
    ResolveY4mOutputPath,
    /// Creating the same-directory temporary Y4M output file failed.
    CreateY4mTempFile,
    /// Writing bytes into the temporary Y4M output file failed.
    WriteY4mTempFile,
    /// Flushing the temporary Y4M output file failed.
    FlushY4mTempFile,
    /// Syncing the temporary Y4M output file failed.
    SyncY4mTempFile,
    /// Renaming the temporary file into the requested Y4M output path failed.
    RenameY4mOutput,
    /// Attempting the parent directory durability sync after Y4M publication.
    SyncY4mOutputDirectory,
    /// Removing a failed temporary Y4M output file failed.
    CleanupY4mTempFile,
}

impl DecodeOutputOperation {
    /// Returns the stable snake_case operation identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SerializeY4m => "serialize_y4m",
            Self::WriteY4mStream => "write_y4m_stream",
            Self::ResolveY4mOutputPath => "resolve_y4m_output_path",
            Self::CreateY4mTempFile => "create_y4m_temp_file",
            Self::WriteY4mTempFile => "write_y4m_temp_file",
            Self::FlushY4mTempFile => "flush_y4m_temp_file",
            Self::SyncY4mTempFile => "sync_y4m_temp_file",
            Self::RenameY4mOutput => "rename_y4m_output",
            Self::SyncY4mOutputDirectory => "sync_y4m_output_directory",
            Self::CleanupY4mTempFile => "cleanup_y4m_temp_file",
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
