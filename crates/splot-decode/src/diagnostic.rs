// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Library-owned diagnostic adaptation for decode planner results.
//!
//! Feature tracking: `CLI-DECODE`, `DECODE-BYTE-STREAM-PLANNER`,
//! `DECODE-STREAM-STATE-PLANNER`, `DECODE-GENERAL-INTRA-BLOCK-MODES`,
//! `DOC-DECODE-LIMITS-CONTRACT`.

use crate::{
    DecodeDiagnostic, DecodeError, DecodeLimitError, DecodeLimitName, DecodeOutputError,
    DecodeSeverity, DecodeSourceIssue, DecodeUnsupportedFeature, DecodeUnsupportedStructure,
    UNSUPPORTED_FEATURE_RULE_ID,
};

/// Stable rule id for malformed decode-source diagnostics.
pub const MALFORMED_SOURCE_RULE_ID: &str = "decode/malformed-source";

/// Stable rule id for decode resource-limit diagnostics.
pub const RESOURCE_LIMIT_RULE_ID: &str = "decode/resource-limit";

/// Stable rule id for decode output serialization/publication diagnostics.
pub const OUTPUT_ERROR_RULE_ID: &str = "decode/output-error";

/// Diagnostic plus typed details for one `splot decode` result.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DecodeDiagnosticReport {
    /// Stable base diagnostic fields.
    pub diagnostic: DecodeDiagnostic,
    /// Case-specific structured details.
    pub details: DecodeDiagnosticDetails,
}

impl DecodeDiagnosticReport {
    /// Converts a decode error into a user-facing diagnostic report when the
    /// error represents input/planner state rather than an operational runtime
    /// failure.
    #[must_use]
    pub fn from_decode_error(error: &DecodeError) -> Option<Self> {
        match error {
            DecodeError::Pool { .. }
            | DecodeError::HeaderState { .. }
            | DecodeError::Reconstruction { .. }
            | DecodeError::ReferenceState { .. } => None,
            DecodeError::Limit { source } => Some(Self::resource_limit(*source)),
            DecodeError::MalformedSource { issue } => Some(Self::malformed_source(issue)),
            DecodeError::UnsupportedStructure { unsupported } => {
                Some(Self::unsupported_structure(unsupported))
            }
            DecodeError::UnsupportedFeature { unsupported } => {
                Some(Self::unsupported_feature(unsupported.as_ref()))
            }
            DecodeError::Output { source } => Some(Self::output_error(source)),
        }
    }

    fn malformed_source(issue: &DecodeSourceIssue) -> Self {
        Self {
            diagnostic: error_diagnostic(
                MALFORMED_SOURCE_RULE_ID,
                issue.spec_section(),
                "decode source is malformed and could not be parsed.",
            ),
            details: DecodeDiagnosticDetails::MalformedSource(DecodeMalformedSourceDetails {
                source_issue_kind: issue.kind().as_str(),
                parser_rule_id: issue.rule_id(),
                byte_offset: issue.offset().map(splot_core::span::ByteOffset::get),
                frame_index: issue.frame_index(),
                parser_message: issue.message().to_owned(),
            }),
        }
    }

    fn resource_limit(source: DecodeLimitError) -> Self {
        let limit_name = source.name();
        let check = source.check();
        Self {
            diagnostic: error_diagnostic(
                RESOURCE_LIMIT_RULE_ID,
                resource_limit_spec_section(limit_name),
                "decode planning stopped because a configured resource limit was exceeded.",
            ),
            details: DecodeDiagnosticDetails::ResourceLimit(DecodeResourceLimitDetails {
                limit_name: limit_name.as_str(),
                limit: check.and_then(|check| check.threshold().max_value()),
                actual: source.actual(),
                unit: check
                    .map_or_else(|| limit_name.unit().as_str(), |check| check.unit().as_str()),
                byte_offset: None,
                bit_offset: None,
            }),
        }
    }

    fn unsupported_structure(unsupported: &DecodeUnsupportedStructure) -> Self {
        Self {
            diagnostic: error_diagnostic(
                UNSUPPORTED_FEATURE_RULE_ID,
                Some(unsupported.spec_section()),
                unsupported.message(),
            ),
            details: DecodeDiagnosticDetails::UnsupportedStructure(
                DecodeUnsupportedStructureDetails {
                    unsupported_reason: unsupported.reason().as_str(),
                    obu_type: unsupported.obu_type().spec_name(),
                    byte_offset: unsupported.offset().get(),
                },
            ),
        }
    }

    fn unsupported_feature(unsupported: &DecodeUnsupportedFeature) -> Self {
        Self {
            diagnostic: error_diagnostic(
                UNSUPPORTED_FEATURE_RULE_ID,
                Some(unsupported.spec_section()),
                unsupported.message(),
            ),
            details: DecodeDiagnosticDetails::UnsupportedFeature(DecodeUnsupportedFeatureDetails {
                unsupported_reason: unsupported.reason(),
                byte_offset: unsupported
                    .byte_offset()
                    .map(splot_core::span::ByteOffset::get),
            }),
        }
    }

    fn output_error(source: &DecodeOutputError) -> Self {
        Self {
            diagnostic: error_diagnostic(
                OUTPUT_ERROR_RULE_ID,
                None,
                "decode output could not be serialized or written.",
            ),
            details: DecodeDiagnosticDetails::OutputError(DecodeOutputErrorDetails {
                operation: source.operation().as_str(),
                source_kind: source.source_kind(),
                source_message: source.source_message(),
            }),
        }
    }
}

/// Builds an error-severity [`DecodeDiagnostic`] from its rule id, optional AV2
/// spec section, and message — the shape shared by every diagnostic constructor.
fn error_diagnostic(
    rule_id: &'static str,
    spec_section: Option<&'static str>,
    message: &'static str,
) -> DecodeDiagnostic {
    DecodeDiagnostic {
        rule_id,
        severity: DecodeSeverity::Error,
        spec_section,
        message,
    }
}

/// Case-specific structured diagnostic fields.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeDiagnosticDetails {
    /// Malformed source/container details.
    MalformedSource(DecodeMalformedSourceDetails),
    /// Resource-limit details.
    ResourceLimit(DecodeResourceLimitDetails),
    /// Unsupported parsed structure details.
    UnsupportedStructure(DecodeUnsupportedStructureDetails),
    /// Unsupported runtime feature details.
    UnsupportedFeature(DecodeUnsupportedFeatureDetails),
    /// Decode output serialization or caller-writer failure details.
    OutputError(DecodeOutputErrorDetails),
}

/// Details for `decode/malformed-source`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeMalformedSourceDetails {
    /// Stable source issue category.
    pub source_issue_kind: &'static str,
    /// Parser rule id, when the source parser exposes one.
    pub parser_rule_id: Option<&'static str>,
    /// Byte offset associated with the source issue, when known.
    pub byte_offset: Option<u64>,
    /// IVF frame index associated with the issue, when known.
    pub frame_index: Option<usize>,
    /// Parser/container message from the source issue.
    pub parser_message: String,
}

/// Details for `decode/resource-limit`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeResourceLimitDetails {
    /// Stable decode limit name.
    pub limit_name: &'static str,
    /// Configured inclusive limit, when finite and available.
    pub limit: Option<u64>,
    /// Measured actual value, when available.
    pub actual: Option<u64>,
    /// Unit for `limit` and `actual`.
    pub unit: &'static str,
    /// Byte offset associated with the failed limit, when known.
    pub byte_offset: Option<u64>,
    /// Bit offset associated with the failed limit, when known.
    pub bit_offset: Option<u64>,
}

/// Details for planner-level `decode/unsupported-feature`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeUnsupportedStructureDetails {
    /// Stable unsupported-structure reason.
    pub unsupported_reason: &'static str,
    /// AV2 OBU type that triggered the unsupported result.
    pub obu_type: &'static str,
    /// OBU byte offset that triggered the unsupported result.
    pub byte_offset: u64,
}

/// Details for runtime-tier `decode/unsupported-feature`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeUnsupportedFeatureDetails {
    /// Stable unsupported reason.
    pub unsupported_reason: &'static str,
    /// Byte offset associated with the unsupported feature, when known.
    pub byte_offset: Option<u64>,
}

/// Details for `decode/output-error`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeOutputErrorDetails {
    /// Stable output operation identifier.
    pub operation: &'static str,
    /// Stable source category for the underlying output error.
    pub source_kind: &'static str,
    /// Human-readable underlying output error message.
    pub source_message: String,
}

const fn resource_limit_spec_section(name: DecodeLimitName) -> Option<&'static str> {
    match name {
        DecodeLimitName::MaxInputBytes | DecodeLimitName::MaxIvfFrameRecords => None,
        DecodeLimitName::MaxObus => Some("5.2.1"),
        DecodeLimitName::MaxFramesToDecode
        | DecodeLimitName::MaxOutputFrames
        | DecodeLimitName::MaxFrameWidth
        | DecodeLimitName::MaxFrameHeight
        | DecodeLimitName::MaxLumaSamplesPerFrame
        | DecodeLimitName::MaxDecodedFrameBytes
        | DecodeLimitName::MaxReferenceSlots
        | DecodeLimitName::MaxReferenceStoreBytes
        | DecodeLimitName::MaxTileCount
        | DecodeLimitName::MaxTilePartitionSteps
        | DecodeLimitName::MaxTilePayloadBytes => Some("7.1"),
        DecodeLimitName::MaxLoopRestorationSourceReads => Some("7.20.2"),
    }
}

#[cfg(test)]
mod tests;
