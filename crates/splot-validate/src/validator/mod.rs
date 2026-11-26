// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 bitstream validator: parse, then run the check registry.

use std::io::Read;

use splot_core::span::ByteOffset;

use crate::diagnostic::ValidationReport;
use crate::options::ValidationOptions;

mod diagnostics;
mod runner;
mod streaming;

pub use streaming::StreamValidateError;

#[cfg(test)]
mod tests;

/// Limits for a streaming validation run with execution evidence.
#[derive(Debug, Clone, Copy)]
pub struct ValidationLimits {
    /// Maximum declared Annex B OBU or IVF frame size, in bytes.
    pub max_unit_bytes: usize,
    /// Maximum number of findings retained in the returned report.
    pub max_diagnostics: usize,
}

/// Counts of all findings emitted during the run, including omitted findings.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticCounts {
    /// Number of errors.
    pub errors: usize,
    /// Number of warnings.
    pub warnings: usize,
    /// Number of informational findings.
    pub info: usize,
}

/// Why streaming validation stopped.
#[derive(Debug, Default)]
#[non_exhaustive]
pub enum ValidationCompletion {
    /// The reader reached its end. Individual IVF records may have parse failures.
    #[default]
    EndOfInput,
    /// Invalid Annex B or IVF framing prevented further reads.
    MalformedInput,
    /// The cancellation callback requested a stop.
    Cancelled,
    /// An I/O failure or declared size over the configured limit prevented further reads.
    InputError(StreamValidateError),
}

/// Observed execution of the existing OBU checks and stream finalization.
///
/// This coverage does not identify which conditional rules ran on each OBU.
/// Some AV2 requirements are outside the validator's implemented check set.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ValidationCoverage {
    /// Number of parsed envelopes passed through the validator's OBU checks.
    pub checked_obus: u64,
    /// End of the last checked OBU in absolute file bytes. Earlier IVF records may contain gaps.
    pub last_checked_obu_end: Option<ByteOffset>,
    /// Number of framing failures, including malformed Annex B in IVF records.
    pub malformed_units: u64,
    /// Whether the validator performed stream-finalization checks.
    pub finalized: bool,
}

/// Streaming findings and execution evidence. A report can represent partial input.
///
/// Apply severity policy to [`Self::counts`], which includes findings omitted from
/// [`Self::report`]. Use [`Self::completion`] and [`Self::coverage`] to describe scope.
#[derive(Debug, Default)]
pub struct ValidationExecution {
    /// The first findings in the validator's emission order, within the requested cap.
    pub report: ValidationReport,
    /// Counts of every finding emitted before the run stopped.
    pub counts: DiagnosticCounts,
    /// Why the run stopped.
    pub completion: ValidationCompletion,
    /// OBU processing and stream-finalization evidence.
    pub coverage: ValidationCoverage,
}

/// Validates AV2 length-delimited bitstreams and produces a [`ValidationReport`].
#[derive(Debug, Clone, Copy)]
pub struct Validator {
    /// When `true`, [`Validator::is_acceptable`] treats a report with warnings
    /// (not just errors) as a conformance failure. The set of diagnostics produced
    /// by [`Validator::validate_bytes`] is unaffected.
    pub strict: bool,
}

impl Validator {
    /// Creates a validator.
    #[must_use]
    pub fn new(strict: bool) -> Self {
        Self { strict }
    }

    /// Returns `true` if `report` passes under this validator's strictness.
    ///
    /// A report always fails if it contains any [`crate::Severity::Error`]; in
    /// [`Validator::strict`] mode it additionally fails if it contains any warning.
    /// This is the single source of truth for pass/fail (the CLI's exit status
    /// uses it).
    #[must_use]
    pub fn is_acceptable(&self, report: &ValidationReport) -> bool {
        report.is_conformant() && !(self.strict && report.warnings().next().is_some())
    }

    /// Validates `data` as a raw AV2 Annex B bitstream or an IVF-wrapped Annex B
    /// bitstream with the default
    /// [`ValidationOptions`] (no external HLS).
    ///
    /// A malformed bitstream is reported as one or more [`crate::Severity::Error`]
    /// diagnostics, never as a panic or an `Err`.
    #[must_use]
    pub fn validate_bytes(&self, data: &[u8]) -> ValidationReport {
        self.validate_bytes_with_options(data, &ValidationOptions::default())
    }

    /// Validates `data` as a raw AV2 Annex B bitstream or an IVF-wrapped Annex B
    /// bitstream using `options`.
    ///
    /// `options` supplies caller-provided external HLS availability (AV2 § 7.3.8);
    /// the default ([`Validator::validate_bytes`]) assumes none. A malformed
    /// bitstream is reported as one or more [`crate::Severity::Error`] diagnostics, never as
    /// a panic or an `Err`.
    #[must_use]
    pub fn validate_bytes_with_options(
        &self,
        data: &[u8],
        options: &ValidationOptions,
    ) -> ValidationReport {
        runner::validate_bytes_with_options(data, options)
    }

    /// Validates a forward-only `Read` stream (raw Annex B or IVF-wrapped Annex
    /// B), bounding peak input memory to a single temporal unit instead of the
    /// whole file.
    ///
    /// The report is byte-identical to [`Validator::validate_bytes`] on the same
    /// bitstream. Truncated or malformed input is reported as diagnostics, never
    /// an `Err`; the `Err` path is reserved for genuine reader I/O failures and
    /// over-cap units (see [`StreamValidateError`]).
    ///
    /// # Errors
    /// Returns [`StreamValidateError`] for a reader I/O failure or a temporal unit
    /// exceeding the per-unit byte cap.
    pub fn validate_reader<R: Read>(
        &self,
        reader: R,
    ) -> Result<ValidationReport, StreamValidateError> {
        self.validate_reader_with_options(reader, &ValidationOptions::default())
    }

    /// Validates a forward-only `Read` stream using `options` (external HLS
    /// availability, AV2 § 7.3.8).
    ///
    /// # Errors
    /// Returns [`StreamValidateError`] for a reader I/O failure or an over-cap
    /// temporal unit.
    pub fn validate_reader_with_options<R: Read>(
        &self,
        reader: R,
        options: &ValidationOptions,
    ) -> Result<ValidationReport, StreamValidateError> {
        streaming::validate_reader_with_options(reader, options)
    }

    /// Runs the existing OBU checks on Annex B or IVF and reports their execution.
    ///
    /// The callback runs before each unit read and before each OBU check. A read
    /// or check in progress runs until it returns. On cancellation, terminal
    /// malformation, or input failure, this retains emitted findings without
    /// running end-of-stream checks on the incomplete input.
    ///
    /// The diagnostic cap limits the retained report. Temporary memory during
    /// individual checks and validator state follow the existing check logic.
    pub fn validate_reader_with_evidence<R: Read>(
        &self,
        reader: R,
        options: &ValidationOptions,
        limits: ValidationLimits,
        should_cancel: impl FnMut() -> bool,
    ) -> ValidationExecution {
        streaming::validate_reader_with_evidence(reader, options, limits, should_cancel)
    }
}
