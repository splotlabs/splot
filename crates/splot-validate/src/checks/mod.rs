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
use splot_core::obu::finish_obu_payload;

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
        Box::new(obu::ReservedObuType),
        Box::new(obu::ReservedObuAllZeroPayload),
        Box::new(obu::TrailingBitsForEmptySyntaxObus),
        Box::new(sequence::SequenceHeaderSyntax),
        Box::new(hls::MsdoSyntax),
        Box::new(hls::MultiFrameHeaderSyntax),
        Box::new(hls::LayerConfigRecordSyntax),
        Box::new(hls::AtlasSegmentSyntax),
        Box::new(hls::OperatingPointSetSyntax),
        Box::new(hls::BufferRemovalTimingSyntax),
        Box::new(hls::QuantizerMatrixSyntax),
        Box::new(hls::FilmGrainSyntax),
        Box::new(hls::ContentInterpretationSyntax),
        Box::new(padding::PaddingSyntax),
        Box::new(metadata::MetadataSyntax),
        Box::new(layers::GlobalXLayerRequired),
        Box::new(layers::GlobalXLayerRequiresBaseLayers),
        Box::new(layers::GlobalXLayerAllowedTypes),
        Box::new(layers::BaseLayerOnlyTypes),
        Box::new(layers::TemporalLayerZeroOnlyTypes),
    ]
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
