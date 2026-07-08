// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-decode` - scaffold for the future AV2 decode driver.
//!
//! This crate will coordinate parsed AV2 bitstream facts from `splot-core` with
//! reconstruction and output state from `splot-recon`. It owns the current
//! structured `decode/unsupported-feature` diagnostic API plus local
//! resource-limit policy types, plus a bounded, plan-only raw byte stream
//! planner. It also exposes a [`DecodeContext`]/[`DecodeRuntimeConfig`]
//! worker-pool scaffold that owns a [`splot_parallel::WorkerPool`]. The stream
//! planners consume either bounded raw bytes or already parsed `splot-core`
//! stream facts, while the runtime adapters expose only the documented minimal
//! hash, raw, and Y4M byte-output tier.
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
    DecodeOutputErrorDetails, DecodePlanSummary, DecodeResourceLimitDetails,
    DecodeUnsupportedStructureDetails, MALFORMED_SOURCE_RULE_ID, OUTPUT_ERROR_RULE_ID,
    RESOURCE_LIMIT_RULE_ID,
};
pub use error::{DecodeError, Result};
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

/// Current unsupported diagnostic descriptor for `splot decode`.
///
/// The descriptor cites AV2 §7.1 as context for the unimplemented decoding
/// process, while keeping `cli-decode-entrypoint` intentionally unsupported.
pub const UNSUPPORTED_FEATURE_DIAGNOSTIC: DecodeDiagnostic = DecodeDiagnostic {
    rule_id: UNSUPPORTED_FEATURE_RULE_ID,
    severity: DecodeSeverity::Error,
    spec_section: Some("7.1"),
    message: "Byte stream planning succeeded, but `splot decode` runtime output is not implemented yet.",
};

/// Returns the current unsupported diagnostic for `splot decode`.
///
/// This function is intentionally metadata-only: it does not allocate decoded
/// frames, write output paths, or invoke external decoders.
#[must_use]
pub const fn unsupported_feature_diagnostic() -> DecodeDiagnostic {
    UNSUPPORTED_FEATURE_DIAGNOSTIC
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
