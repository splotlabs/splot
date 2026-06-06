// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Structured validation diagnostics and reports.

use core::fmt;

use serde::Serialize;
use splot_core::span::{BitOffset, ByteOffset};

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    /// A bitstream conformance violation.
    Error,
    /// A non-fatal concern worth surfacing.
    Warning,
    /// Informational only.
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Error => "ERROR",
            Self::Warning => "WARNING",
            Self::Info => "INFO",
        };
        f.write_str(text)
    }
}

/// A single validator finding with a stable rule id and location.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    /// Stable, machine-readable rule identifier (e.g. `"obu-header/global-xlayer-required"`).
    pub rule_id: String,
    /// AV2 spec section the rule derives from (e.g. `"6.2.2"`), if any.
    pub spec_section: Option<String>,
    /// Severity of the finding.
    pub severity: Severity,
    /// Byte offset the finding applies to, if known.
    pub byte_offset: Option<ByteOffset>,
    /// Bit offset within [`Diagnostic::byte_offset`], if known.
    pub bit_offset: Option<BitOffset>,
    /// Human-readable message.
    pub message: String,
}

impl Diagnostic {
    /// Creates a diagnostic with the given severity, rule id, and message.
    #[must_use]
    pub fn new(severity: Severity, rule_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.into(),
            spec_section: None,
            severity,
            byte_offset: None,
            bit_offset: None,
            message: message.into(),
        }
    }

    /// Creates an [`Severity::Error`] diagnostic.
    #[must_use]
    pub fn error(rule_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, rule_id, message)
    }

    /// Creates a [`Severity::Warning`] diagnostic.
    #[must_use]
    pub fn warning(rule_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, rule_id, message)
    }

    /// Creates a [`Severity::Info`] diagnostic.
    #[must_use]
    pub fn info(rule_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, rule_id, message)
    }

    /// Sets the spec section.
    #[must_use]
    pub fn with_spec_section(mut self, section: impl Into<String>) -> Self {
        self.spec_section = Some(section.into());
        self
    }

    /// Sets the byte offset.
    #[must_use]
    pub fn with_byte_offset(mut self, offset: ByteOffset) -> Self {
        self.byte_offset = Some(offset);
        self
    }

    /// Sets the bit offset.
    #[must_use]
    pub fn with_bit_offset(mut self, offset: BitOffset) -> Self {
        self.bit_offset = Some(offset);
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.severity, self.rule_id)?;
        if let Some(section) = &self.spec_section {
            write!(f, " (§{section})")?;
        }
        if let Some(offset) = self.byte_offset {
            write!(f, " @byte {offset}")?;
            if let Some(bit) = self.bit_offset {
                write!(f, ".{bit}")?;
            }
        }
        write!(f, ": {}", self.message)
    }
}

/// A collection of [`Diagnostic`]s produced by validating one bitstream.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ValidationReport {
    /// All diagnostics, in the order they were produced.
    pub diagnostics: Vec<Diagnostic>,
}

impl ValidationReport {
    /// Creates an empty report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    /// Returns `true` if there are no [`Severity::Error`] diagnostics.
    #[must_use]
    pub fn is_conformant(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Iterates over the error diagnostics.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
    }

    /// Iterates over the warning diagnostics.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
    }
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for diagnostic in &self.diagnostics {
            writeln!(f, "{diagnostic}")?;
        }
        let errors = self.errors().count();
        let warnings = self.warnings().count();
        let infos = self.diagnostics.len().saturating_sub(errors + warnings);
        writeln!(f, "{errors} error(s), {warnings} warning(s), {infos} info")?;
        if self.is_conformant() {
            writeln!(f, "conformant: no errors found")
        } else {
            writeln!(f, "NOT conformant")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_is_conformant() {
        let report = ValidationReport::new();
        assert!(report.is_conformant());
        assert_eq!(report.errors().count(), 0);
    }

    #[test]
    fn errors_make_report_non_conformant() {
        let mut report = ValidationReport::new();
        report.push(Diagnostic::warning("w", "a warning"));
        assert!(report.is_conformant());
        report.push(
            Diagnostic::error("e", "an error")
                .with_spec_section("6.2.2")
                .with_byte_offset(ByteOffset::new(3)),
        );
        assert!(!report.is_conformant());
        assert_eq!(report.errors().count(), 1);
        assert_eq!(report.warnings().count(), 1);
    }

    #[test]
    fn diagnostic_display_includes_location() {
        let diagnostic = Diagnostic::error("obu-header/x", "bad")
            .with_spec_section("6.2.2")
            .with_byte_offset(ByteOffset::new(5));
        assert_eq!(
            diagnostic.to_string(),
            "[ERROR] obu-header/x (§6.2.2) @byte 5: bad"
        );
    }
}
