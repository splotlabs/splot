// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Error and result types for the splot decode driver.

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
