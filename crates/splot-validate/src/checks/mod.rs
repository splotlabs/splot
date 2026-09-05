// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Stateless conformance checks over individual OBUs.
//!
//! Stateful ordering and sequence/frame checks live in the internal `context` module.

mod hls;
mod layers;
mod metadata;
mod obu;
mod padding;
mod sequence;
mod syntax_error;

pub(crate) use syntax_error::{payload_parse_error_diagnostic, syntax_error_diagnostic};

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::error::Error;
use splot_core::obu::finish_obu_payload;
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

// Stateless unit structs need no per-validation allocation. A const slice avoids
// requiring Sync on the public Check trait.
const DEFAULT_CHECKS: &[&dyn Check] = &[
    &obu::ReservedObuType,
    &obu::ReservedObuAllZeroPayload,
    &obu::TrailingBitsForEmptySyntaxObus,
    &sequence::SequenceHeaderSyntax,
    &hls::MsdoSyntax,
    &hls::MultiFrameHeaderSyntax,
    &hls::LayerConfigRecordSyntax,
    &hls::AtlasSegmentSyntax,
    &hls::OperatingPointSetSyntax,
    &hls::BufferRemovalTimingSyntax,
    &hls::QuantizerMatrixSyntax,
    &hls::FilmGrainSyntax,
    &hls::ContentInterpretationSyntax,
    &padding::PaddingSyntax,
    &metadata::MetadataSyntax,
    &layers::GlobalXLayerRequired,
    &layers::GlobalXLayerRequiresBaseLayers,
    &layers::GlobalXLayerAllowedTypes,
    &layers::BaseLayerOnlyTypes,
    &layers::TemporalLayerZeroOnlyTypes,
];

/// Returns the default check registry, in execution order.
#[must_use]
pub fn default_checks() -> &'static [&'static dyn Check] {
    DEFAULT_CHECKS
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

/// Validates the § 5.2.1 OBU payload tail (`obu_extension_flag` / `trailing_bits`) and
/// pushes the mapped diagnostic if the tail is malformed.
///
/// `extensible` selects whether the OBU type carries an `obu_extension_flag` before its
/// `trailing_bits()`. Shared by every check that parses a payload and then validates the
/// remaining bits.
fn finish_payload_or_emit(
    reader: &mut BitReader<'_>,
    payload: &[u8],
    extensible: bool,
    report: &mut ValidationReport,
) {
    if let Err(error) = finish_obu_payload(reader, payload, extensible)
        && let Some(diagnostic) = syntax_error_diagnostic(&error)
    {
        report.push(diagnostic);
    }
}

/// Parses a matching OBU, runs its semantics, and validates the § 5.2.1 payload tail.
/// `extensible` selects whether `obu_extension_flag` precedes `trailing_bits()`.
fn run_payload_syntax_check<P>(
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
    obu_type: ObuType,
    spec_section: &'static str,
    extensible: bool,
    parse: impl FnOnce(&mut BitReader<'_>) -> Result<P, Error>,
    check: impl FnOnce(&P, &ObuEnvelope<'_>, &mut ValidationReport),
) {
    if obu.header.obu_type != obu_type {
        return;
    }

    let mut reader = BitReader::new(obu.payload, obu.payload_offset());
    match parse(&mut reader) {
        Ok(parsed) => {
            check(&parsed, obu, report);
            finish_payload_or_emit(&mut reader, obu.payload, extensible, report);
        }
        Err(error) => report.push(
            syntax_error_diagnostic(&error)
                .unwrap_or_else(|| payload_parse_error_diagnostic(&error, spec_section)),
        ),
    }
}
