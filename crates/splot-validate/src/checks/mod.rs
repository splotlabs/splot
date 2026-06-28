// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Pluggable conformance checks run against parsed OBUs.
//!
//! Each [`Check`] enforces one constraint and emits structured [`Diagnostic`]s.
//! This module owns the [`Check`] trait, the [`default_checks`] registry, and the
//! shared `emit` / `finish_payload_or_emit` helpers; the checks themselves live in
//! namespace submodules:
//!
//! - `obu` — generic OBU-header and reserved-OBU-type checks.
//! - `sequence` — `OBU_SEQUENCE_HEADER` syntax and tile-params constraints.
//! - `hls` — high-level-syntax OBUs (MSDO, MFH, LCR, atlas, OPS, BRT, QM, film
//!   grain, content interpretation).
//! - `metadata` — metadata OBU syntax and per-unit semantics.
//! - `padding` — `OBU_PADDING` syntax.
//! - `layers` — § 6.2.2 OBU-header layer-id constraints.
//!
//! Error-kind → diagnostic mapping lives in `syntax_error`. OBU ordering and
//! sequence/frame-level conformance are future work.
//
// TODO(spec: AV2-7.3-OBU-ORDERING): add OBU-ordering checks.

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

/// The default check registry, in execution order.
///
/// Every check is a zero-sized stateless unit struct, so the registry is a
/// `'static` slice of trait-object references with no per-validation heap
/// allocation; the (zero-sized) check values and their vtables live in
/// read-only static memory. A `const` (rather than `static`) avoids a `Sync`
/// bound on the `Check` trait — the slice is a compile-time value, not shared
/// mutable state.
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

/// Runs a single-OBU payload syntax check with the shared parse-then-validate scaffold.
///
/// When `obu` is of `obu_type`, this reads its payload, parses it with `parse`, runs the
/// locally decidable `check` semantics on a successful parse, and validates the § 5.2.1
/// payload tail (`extensible` selects whether an `obu_extension_flag` precedes
/// `trailing_bits()`). A parse error is mapped through [`syntax_error_diagnostic`],
/// falling back to the generic [`payload_parse_error_diagnostic`] tagged with
/// `spec_section`.
///
/// This is the shared scaffold for the payload checks whose only per-OBU variation is the
/// parser, the extensibility flag, and the post-parse semantics. Checks that diverge from
/// this shape — a pre-parse header check (`OBU_MSDO`), a conditional tail (sequence
/// header), multiple OBU types (metadata), or no payload reader (`OBU_PADDING`) — stay
/// hand-written.
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
