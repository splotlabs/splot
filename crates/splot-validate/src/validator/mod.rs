// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 bitstream validator: parse, then run the check registry.

use crate::diagnostic::ValidationReport;
use crate::options::ValidationOptions;

mod diagnostics;
mod runner;

#[cfg(test)]
mod tests;

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
}
