// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Library-owned diagnostic adaptation for decode planner results.
//!
//! Feature tracking: `CLI-DECODE`, `DECODE-BYTE-STREAM-PLANNER`,
//! `DECODE-STREAM-STATE-PLANNER`, `DOC-DECODE-LIMITS-CONTRACT`.

use splot_core::stream::BitstreamFormat;

use crate::{
    DecodeDiagnostic, DecodeError, DecodeLimitError, DecodeLimitName, DecodeSeverity,
    DecodeSourceIssue, DecodeSourceIssueKind, DecodeStreamPlan, DecodeUnsupportedFeature,
    DecodeUnsupportedStructure, UNSUPPORTED_FEATURE_RULE_ID,
};

/// Stable rule id for malformed decode-source diagnostics.
pub const MALFORMED_SOURCE_RULE_ID: &str = "decode/malformed-source";

/// Stable rule id for decode resource-limit diagnostics.
pub const RESOURCE_LIMIT_RULE_ID: &str = "decode/resource-limit";

const DECODE_BYTE_STREAM_PLANNER_MATRIX_ROW: &str = "decode-byte-stream-planner";
const DECODE_BYTE_STREAM_PLANNER_FEATURE_ID: &str = "DECODE-BYTE-STREAM-PLANNER";
const DECODE_LIMITS_BUDGET_MATRIX_ROW: &str = "decode-limits-budget";
const DOC_DECODE_LIMITS_CONTRACT_FEATURE_ID: &str = "DOC-DECODE-LIMITS-CONTRACT";

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
            DecodeError::Pool { .. } => None,
            DecodeError::Limit { source } => Some(Self::resource_limit(*source)),
            DecodeError::MalformedSource { issue } => Some(Self::malformed_source(issue)),
            DecodeError::UnsupportedStructure { unsupported } => {
                Some(Self::unsupported_structure(unsupported))
            }
            DecodeError::UnsupportedFeature { unsupported } => {
                Some(Self::unsupported_feature(unsupported.as_ref()))
            }
            DecodeError::Reconstruction { .. } => None,
        }
    }

    /// Builds the diagnostic emitted after byte planning succeeds but runtime
    /// decode/output remains unsupported.
    #[must_use]
    pub fn runtime_unsupported(plan: &DecodeStreamPlan) -> Self {
        Self {
            diagnostic: crate::UNSUPPORTED_FEATURE_DIAGNOSTIC,
            details: DecodeDiagnosticDetails::RuntimeUnsupported(DecodePlanSummary::from(plan)),
        }
    }

    fn malformed_source(issue: &DecodeSourceIssue) -> Self {
        Self {
            diagnostic: DecodeDiagnostic {
                rule_id: MALFORMED_SOURCE_RULE_ID,
                severity: DecodeSeverity::Error,
                spec_section: malformed_source_spec_section(issue.kind()),
                matrix_row: DECODE_BYTE_STREAM_PLANNER_MATRIX_ROW,
                feature_id: DECODE_BYTE_STREAM_PLANNER_FEATURE_ID,
                message: "decode source is malformed and could not be planned.",
                remediation: "Check the AV2 Annex B or IVF source bytes before retrying `splot decode`.",
            },
            details: DecodeDiagnosticDetails::MalformedSource(DecodeMalformedSourceDetails {
                source_issue_kind: issue.kind().as_str(),
                parser_rule_id: issue.rule_id(),
                byte_offset: issue.offset().map(|offset| offset.get()),
                frame_index: issue.frame_index(),
                parser_message: issue.message().to_owned(),
            }),
        }
    }

    fn resource_limit(source: DecodeLimitError) -> Self {
        let limit_name = source.name();
        let check = source.check();
        Self {
            diagnostic: DecodeDiagnostic {
                rule_id: RESOURCE_LIMIT_RULE_ID,
                severity: DecodeSeverity::Error,
                spec_section: resource_limit_spec_section(limit_name),
                matrix_row: DECODE_LIMITS_BUDGET_MATRIX_ROW,
                feature_id: DOC_DECODE_LIMITS_CONTRACT_FEATURE_ID,
                message: "decode planning stopped because a configured resource limit was exceeded.",
                remediation: "Use a smaller input or raise the decode limit policy before retrying.",
            },
            details: DecodeDiagnosticDetails::ResourceLimit(DecodeResourceLimitDetails {
                limit_name: limit_name.as_str(),
                limit: check.and_then(|check| check.threshold().max_value()),
                actual: source.actual(),
                unit: check
                    .map(|check| check.unit().as_str())
                    .unwrap_or_else(|| limit_name.unit().as_str()),
                byte_offset: None,
                bit_offset: None,
            }),
        }
    }

    fn unsupported_structure(unsupported: &DecodeUnsupportedStructure) -> Self {
        Self {
            diagnostic: DecodeDiagnostic {
                rule_id: UNSUPPORTED_FEATURE_RULE_ID,
                severity: DecodeSeverity::Error,
                spec_section: Some(unsupported.spec_section()),
                matrix_row: unsupported.matrix_row(),
                feature_id: unsupported.feature_id(),
                message: unsupported.message(),
                remediation: "Use a stream within the initial planner tier or track the referenced decoder support row before decoding this structure.",
            },
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
            diagnostic: DecodeDiagnostic {
                rule_id: UNSUPPORTED_FEATURE_RULE_ID,
                severity: DecodeSeverity::Error,
                spec_section: Some(unsupported.spec_section()),
                matrix_row: unsupported.matrix_row(),
                feature_id: unsupported.feature_id(),
                message: unsupported.message(),
                remediation: unsupported.remediation(),
            },
            details: DecodeDiagnosticDetails::UnsupportedFeature(DecodeUnsupportedFeatureDetails {
                unsupported_reason: unsupported.reason(),
                tier_id: unsupported.tier_id(),
                byte_offset: unsupported.byte_offset().map(|offset| offset.get()),
            }),
        }
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
    /// Byte-plan summary for runtime decode/output deferral.
    RuntimeUnsupported(DecodePlanSummary),
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
    /// Runtime tier id that rejected the input.
    pub tier_id: &'static str,
    /// Byte offset associated with the unsupported feature, when known.
    pub byte_offset: Option<u64>,
}

/// Byte-plan summary attached to runtime unsupported diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodePlanSummary {
    /// Detected input bitstream/container format.
    pub bitstream_format: &'static str,
    /// Caller-provided input length in bytes.
    pub input_len_bytes: u64,
    /// Number of accepted planned OBUs.
    pub obu_count: u64,
    /// Number of accepted frame candidates.
    pub frame_candidate_count: u64,
    /// Number of non-fatal source warnings carried into the plan.
    pub source_warning_count: u64,
    /// Selected temporal layer id.
    pub selected_temporal_layer_id: u8,
    /// Selected embedded layer id.
    pub selected_embedded_layer_id: u8,
    /// Selected extended layer id.
    pub selected_extended_layer_id: u8,
}

impl From<&DecodeStreamPlan> for DecodePlanSummary {
    fn from(plan: &DecodeStreamPlan) -> Self {
        let selected_layer = plan.selected_layer();
        Self {
            bitstream_format: bitstream_format_as_str(plan.format()),
            input_len_bytes: plan.input_len_bytes(),
            obu_count: plan.obu_count(),
            frame_candidate_count: plan.frame_candidate_count(),
            source_warning_count: plan.source_warnings().len() as u64,
            selected_temporal_layer_id: selected_layer.temporal_layer_id().get(),
            selected_embedded_layer_id: selected_layer.embedded_layer_id().get(),
            selected_extended_layer_id: selected_layer.extended_layer_id().get(),
        }
    }
}

const fn malformed_source_spec_section(kind: DecodeSourceIssueKind) -> Option<&'static str> {
    match kind {
        DecodeSourceIssueKind::AnnexBParseError
        | DecodeSourceIssueKind::IvfFramePayloadError
        | DecodeSourceIssueKind::IvfContainerError
        | DecodeSourceIssueKind::IvfWarning => None,
    }
}

const fn resource_limit_spec_section(name: DecodeLimitName) -> Option<&'static str> {
    match name {
        DecodeLimitName::MaxInputBytes => None,
        DecodeLimitName::MaxObus => Some("5.2.1"),
        DecodeLimitName::MaxIvfFrameRecords => None,
        DecodeLimitName::MaxFramesToDecode
        | DecodeLimitName::MaxOutputFrames
        | DecodeLimitName::MaxFrameWidth
        | DecodeLimitName::MaxFrameHeight
        | DecodeLimitName::MaxLumaSamplesPerFrame
        | DecodeLimitName::MaxDecodedFrameBytes
        | DecodeLimitName::MaxReferenceSlots
        | DecodeLimitName::MaxReferenceStoreBytes
        | DecodeLimitName::MaxTileCount
        | DecodeLimitName::MaxTilePayloadBytes
        | DecodeLimitName::MaxOutputBytes => Some("7.1"),
    }
}

const fn bitstream_format_as_str(format: BitstreamFormat) -> &'static str {
    match format {
        BitstreamFormat::AnnexB => "annex_b",
        BitstreamFormat::Ivf => "ivf",
    }
}

#[cfg(test)]
mod tests;
