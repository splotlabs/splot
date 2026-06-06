// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Pluggable conformance checks run against parsed OBUs.
//!
//! Each [`Check`] enforces one constraint and emits structured [`Diagnostic`]s.
//! The checks here are the straightforward, header-only constraints from AV2
//! v1.0.0 § 6.2.2 (they do not require an activated sequence header). OBU
//! ordering and sequence/frame-level conformance are future work.
//
// TODO(spec: AV2-7.3-OBU-ORDERING): add OBU-ordering and sequence-header-activated checks.

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::error::{ByteAlignmentErrorKind, Error, TrailingBitsErrorKind};
use splot_core::obu::parse_trailing_bits;
use splot_core::types::ObuType;

use crate::diagnostic::{Diagnostic, Severity, ValidationReport};

/// A single conformance check over one OBU envelope.
pub trait Check {
    /// Stable rule id reported in diagnostics.
    fn id(&self) -> &'static str;
    /// Spec section this check enforces, if any.
    fn spec_section(&self) -> Option<&'static str>;
    /// Runs the check, pushing any findings into `report`.
    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport);
}

/// Returns the default check registry, in execution order.
#[must_use]
pub fn default_checks() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(ReservedObuType),
        Box::new(ReservedObuAllZeroPayload),
        Box::new(TrailingBitsForEmptySyntaxObus),
        Box::new(GlobalXLayerRequired),
        Box::new(GlobalXLayerRequiresBaseLayers),
        Box::new(GlobalXLayerAllowedTypes),
        Box::new(BaseLayerOnlyTypes),
        Box::new(TemporalLayerZeroOnlyTypes),
    ]
}

/// Converts core payload-boundary syntax errors into stable validator diagnostics.
#[must_use]
pub(crate) fn syntax_error_diagnostic(error: &Error) -> Option<Diagnostic> {
    match error {
        Error::InvalidTrailingBits {
            offset,
            bit_offset,
            kind,
        } => {
            let rule_id = match kind {
                TrailingBitsErrorKind::Empty => "trailing-bits/empty",
                TrailingBitsErrorKind::MissingOneBit => "trailing-bits/missing-one-bit",
                TrailingBitsErrorKind::ZeroBitNotZero => "trailing-bits/zero-bit-not-zero",
            };
            Some(
                Diagnostic::error(rule_id, kind.to_string())
                    .with_spec_section("6.2.3")
                    .with_byte_offset(*offset)
                    .with_bit_offset(*bit_offset),
            )
        }
        Error::InvalidByteAlignment {
            offset,
            bit_offset,
            kind,
        } => {
            let rule_id = match kind {
                ByteAlignmentErrorKind::ZeroBitNotZero => "byte-alignment/zero-bit-not-zero",
            };
            Some(
                Diagnostic::error(rule_id, kind.to_string())
                    .with_spec_section("6.2.4")
                    .with_byte_offset(*offset)
                    .with_bit_offset(*bit_offset),
            )
        }
        _ => None,
    }
}

/// Builds and pushes a diagnostic located at `obu`, tagged with `check`'s id and section.
fn emit(
    report: &mut ValidationReport,
    check: &dyn Check,
    severity: Severity,
    obu: &ObuEnvelope<'_>,
    message: String,
) {
    let mut diagnostic =
        Diagnostic::new(severity, check.id(), message).with_byte_offset(obu.offset);
    if let Some(section) = check.spec_section() {
        diagnostic = diagnostic.with_spec_section(section);
    }
    report.push(diagnostic);
}

/// OBUs with empty payload syntax still carry `trailing_bits` when their declared
/// payload is non-empty. Until full payload dispatch exists, only these OBU types
/// can be checked without guessing where payload syntax ends.
struct TrailingBitsForEmptySyntaxObus;

impl Check for TrailingBitsForEmptySyntaxObus {
    fn id(&self) -> &'static str {
        // Registry identifier only; emitted diagnostics use syntax_error_diagnostic() rule ids.
        "trailing-bits/empty-syntax-obu-payload"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.2.3")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.payload.is_empty() || !has_empty_payload_syntax(obu.header.obu_type) {
            return;
        }

        let payload_offset = obu
            .offset
            .saturating_add(u64::from(obu.header.header_size_bytes));
        let mut reader = BitReader::new(obu.payload, payload_offset);
        let nb_bits = (obu.payload.len() as u64).saturating_mul(8);
        if let Err(error) = parse_trailing_bits(&mut reader, nb_bits)
            && let Some(diagnostic) = syntax_error_diagnostic(&error)
        {
            report.push(diagnostic);
        }
    }
}

fn has_empty_payload_syntax(obu_type: ObuType) -> bool {
    matches!(
        obu_type,
        ObuType::Reserved0 | ObuType::Reserved(_) | ObuType::TemporalDelimiter
    )
}

/// Informational: reserved OBU types are ignored by conformant decoders (AV2 Table 6.1).
struct ReservedObuType;

impl Check for ReservedObuType {
    fn id(&self) -> &'static str {
        "obu-header/reserved-obu-type"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type.is_reserved() {
            emit(
                report,
                self,
                Severity::Info,
                obu,
                format!(
                    "reserved obu_type {} is ignored by conformant decoders",
                    obu.header.obu_type.raw()
                ),
            );
        }
    }
}

/// A reserved OBU that carries payload must have at least one non-zero payload byte
/// (AV2 § 5.3 / § 6.2.3: `trailing_one_bit` shall be 1).
struct ReservedObuAllZeroPayload;

impl Check for ReservedObuAllZeroPayload {
    fn id(&self) -> &'static str {
        "obu-reserved/all-zero-payload"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("5.3")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        if obu.header.obu_type.is_reserved()
            && !obu.payload.is_empty()
            && obu.payload.iter().all(|&byte| byte == 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                "reserved OBU payload is entirely zero; AV2 § 5.3 requires at least one non-zero \
                 payload byte (including the trailing bit)"
                    .to_owned(),
            );
        }
    }
}

/// `OBU_MSDO` / `OBU_TEMPORAL_DELIMITER` must use `obu_xlayer_id == GLOBAL_XLAYER_ID` (§ 6.2.2).
struct GlobalXLayerRequired;

impl Check for GlobalXLayerRequired {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-required"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_global_xlayer() && !header.extended_layer_id.is_global() {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_xlayer_id == GLOBAL_XLAYER_ID (31), found {}",
                    header.obu_type.spec_name(),
                    header.extended_layer_id.get()
                ),
            );
        }
    }
}

/// `obu_xlayer_id == GLOBAL_XLAYER_ID` requires base embedded and temporal layers (§ 6.2.2).
struct GlobalXLayerRequiresBaseLayers;

impl Check for GlobalXLayerRequiresBaseLayers {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-requires-base-layers"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.extended_layer_id.is_global()
            && (header.embedded_layer_id.get() != 0 || header.temporal_layer_id.get() != 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "obu_xlayer_id == GLOBAL_XLAYER_ID requires obu_mlayer_id and obu_tlayer_id == 0 \
                     (found mlayer={}, tlayer={})",
                    header.embedded_layer_id.get(),
                    header.temporal_layer_id.get()
                ),
            );
        }
    }
}

/// `obu_xlayer_id == GLOBAL_XLAYER_ID` is only allowed for certain OBU types (§ 6.2.2).
struct GlobalXLayerAllowedTypes;

impl Check for GlobalXLayerAllowedTypes {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-allowed-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.extended_layer_id.is_global() && !header.obu_type.permits_global_xlayer() {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} is not permitted to use obu_xlayer_id == GLOBAL_XLAYER_ID",
                    header.obu_type.spec_name()
                ),
            );
        }
    }
}

/// Sequence header, temporal delimiter, LCR, OPS, and atlas segment must be base-layer (§ 6.2.2).
struct BaseLayerOnlyTypes;

impl Check for BaseLayerOnlyTypes {
    fn id(&self) -> &'static str {
        "obu-header/base-layer-only-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_base_temporal_and_embedded_layer()
            && (header.temporal_layer_id.get() != 0 || header.embedded_layer_id.get() != 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_tlayer_id and obu_mlayer_id == 0 (found tlayer={}, mlayer={})",
                    header.obu_type.spec_name(),
                    header.temporal_layer_id.get(),
                    header.embedded_layer_id.get()
                ),
            );
        }
    }
}

/// Closed/open-loop key, switch, and RAS frames must have `obu_tlayer_id == 0` (§ 6.2.2).
struct TemporalLayerZeroOnlyTypes;

impl Check for TemporalLayerZeroOnlyTypes {
    fn id(&self) -> &'static str {
        "obu-header/temporal-layer-zero-only-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_base_temporal_layer() && header.temporal_layer_id.get() != 0 {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_tlayer_id == 0 (found {})",
                    header.obu_type.spec_name(),
                    header.temporal_layer_id.get()
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_core::span::{BitOffset, ByteOffset};

    #[test]
    fn syntax_error_diagnostic_maps_trailing_bits_errors() {
        let diagnostic = syntax_error_diagnostic(&Error::InvalidTrailingBits {
            offset: ByteOffset::new(3),
            bit_offset: BitOffset::from_bits(1),
            kind: TrailingBitsErrorKind::ZeroBitNotZero,
        });
        assert!(
            diagnostic.is_some(),
            "trailing-bit error should map to a diagnostic"
        );
        let diagnostic =
            diagnostic.unwrap_or_else(|| Diagnostic::error("trailing-bits/test", "missing"));
        assert_eq!(diagnostic.rule_id, "trailing-bits/zero-bit-not-zero");
        assert_eq!(diagnostic.spec_section.as_deref(), Some("6.2.3"));
        assert_eq!(diagnostic.byte_offset, Some(ByteOffset::new(3)));
        assert_eq!(diagnostic.bit_offset, Some(BitOffset::from_bits(1)));
    }

    #[test]
    fn syntax_error_diagnostic_maps_byte_alignment_errors() {
        let diagnostic = syntax_error_diagnostic(&Error::InvalidByteAlignment {
            offset: ByteOffset::new(7),
            bit_offset: BitOffset::from_bits(5),
            kind: ByteAlignmentErrorKind::ZeroBitNotZero,
        });
        assert!(
            diagnostic.is_some(),
            "byte-alignment error should map to a diagnostic"
        );
        let diagnostic =
            diagnostic.unwrap_or_else(|| Diagnostic::error("byte-alignment/test", "missing"));
        assert_eq!(diagnostic.rule_id, "byte-alignment/zero-bit-not-zero");
        assert_eq!(diagnostic.spec_section.as_deref(), Some("6.2.4"));
        assert_eq!(diagnostic.byte_offset, Some(ByteOffset::new(7)));
        assert_eq!(diagnostic.bit_offset, Some(BitOffset::from_bits(5)));
    }
}
