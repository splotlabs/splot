// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![feature(portable_simd)]

//! `splot-decode` - the AV2 decode driver.
//!
//! This crate coordinates parsed AV2 bitstream facts from `splot-core` with
//! reconstruction and output state from `splot-recon`. It owns the structured
//! `decode/unsupported-feature` diagnostic API, local resource-limit policy
//! types, and a bounded raw byte stream planner. It also exposes a
//! [`DecodeContext`]/[`DecodeRuntimeConfig`] pair that owns a
//! [`splot_parallel::WorkerPool`]. The stream planners consume either bounded
//! raw bytes or already parsed `splot-core` stream facts; the runtime decodes
//! planned streams to hash, raw, and Y4M byte output.
//!
//! Feature tracking: `INFRA-DECODER-CRATE-SCAFFOLDING`,
//! `DECODE-UNSUPPORTED-DIAGNOSTIC-API`, `DECODE-LIMITS-RUNTIME-API`,
//! `DECODE-STREAM-STATE-PLANNER`, `DECODE-BYTE-STREAM-PLANNER`,
//! `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`,
//! `DECODE-GENERAL-INTRA-FRAME-FRONTIER`,
//! `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`,
//! `DECODE-Y4M-RUNTIME-OUTPUT`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod bitstream;
pub mod context;
pub mod diagnostic;
pub mod error;
mod filters;
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
#[path = "../../../fuzz/decode_tile_payload_harness.rs"]
pub mod fuzzing;
pub mod hash_report;
mod output;
mod pipeline;
mod prediction;
mod reference;
mod residual;
pub mod runtime;
mod support;
#[cfg(test)]
#[path = "test_support_tests.rs"]
mod test_support;
mod tile;
mod timing;

pub use bitstream::stream_plan;
pub use context::DecodeContext;
pub use diagnostic::{
    DecodeDiagnosticDetails, DecodeDiagnosticReport, DecodeMalformedSourceDetails,
    DecodeOutputErrorDetails, DecodeResourceLimitDetails, DecodeUnsupportedStructureDetails,
    MALFORMED_SOURCE_RULE_ID, OUTPUT_ERROR_RULE_ID, RESOURCE_LIMIT_RULE_ID,
};
pub use error::{DecodeError, DecodeReferenceStateError, Result};
pub use error::{DecodeOutputError, DecodeOutputOperation, DecodeUnsupportedFeature};
pub use hash_report::{
    DECODE_HASH_REPORT_BYTE_STREAM_ID, DECODE_HASH_REPORT_CONTRACT_ID,
    DECODE_HASH_REPORT_CONTRACT_VERSION, DECODE_HASH_REPORT_HASH_ALGORITHM_ID,
    DECODE_HASH_REPORT_RAW_INTERMEDIATE_OUTPUT_VARIANT, DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN,
    DecodeHashEntry, DecodeHashFrame, DecodeHashPixelFormat, DecodeHashReport, DecodeOutputVariant,
};
pub use runtime::DecodeRuntimeConfig;
pub use stream_plan::{
    DecodeIvfFrameContext, DecodeLayerSelection, DecodeObuSourceKind, DecodePlannedObu,
    DecodePlannedObuRole, DecodeSourceIssue, DecodeSourceIssueKind, DecodeStreamInput,
    DecodeStreamPlan, DecodeUnsupportedReason, DecodeUnsupportedStructure,
};

use core::fmt;

mod limits;

pub use limits::{
    DecodeLimitCheck, DecodeLimitError, DecodeLimitName, DecodeLimitOp, DecodeLimitResult,
    DecodeLimitThreshold, DecodeLimitUnit, DecodeLimits, DecodeOptions,
};

/// Stable rule id for the current unsupported decode diagnostic.
pub const UNSUPPORTED_FEATURE_RULE_ID: &str = "decode/unsupported-feature";

/// Severity for a [`DecodeDiagnostic`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeSeverity {
    /// A fatal diagnostic that exits the requested decode operation.
    Error,
}

impl DecodeSeverity {
    /// Returns the stable text representation used by CLI rendering and JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "Error",
        }
    }
}

impl fmt::Display for DecodeSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured diagnostic emitted by the decoder-facing API and rendered by the
/// CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DecodeDiagnostic {
    /// Stable, machine-readable diagnostic rule id.
    pub rule_id: &'static str,
    /// Diagnostic severity.
    pub severity: DecodeSeverity,
    /// Optional AV2 spec section associated with the diagnostic.
    pub spec_section: Option<&'static str>,
    /// Human-readable diagnostic message.
    pub message: &'static str,
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
