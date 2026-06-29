// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Render-time presentation options for a [`ValidationReport`].
//!
//! These options control only how a finished report is *displayed* — they never
//! affect which diagnostics are computed (that is the validator's job) or the
//! pass/fail decision (that is [`crate::Validator::is_acceptable`] over the full
//! report). The summary counts and the truncation arithmetic are always derived
//! from the full report, so a capped or summary-only view stays truthful.
//!
//! [`ValidationReport::render_text`] reproduces the default [`core::fmt::Display`] output
//! when given [`RenderOptions::default`] (pinned by a parity test); the CLI uses it
//! for text output and [`ValidationReport::rendered`] for JSON, so all
//! presentation logic lives in this crate rather than the thin CLI.

use core::fmt::Write as _;

use serde::Serialize;

use crate::diagnostic::{Diagnostic, ValidationReport};

/// How to present a [`ValidationReport`]. The default is the full, uncapped output.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Show at most this many diagnostics, with a truncation notice for the rest.
    /// `None` (the default) shows every diagnostic.
    pub max_diagnostics: Option<usize>,
    /// Show only the summary counts and the conformance line, no per-diagnostic
    /// lines. Takes precedence over [`RenderOptions::max_diagnostics`].
    pub summary_only: bool,
    /// The authoritative pass/fail decision (e.g. `Validator::is_acceptable`,
    /// which honors `--strict`) to report as "conformant". `None` (the default)
    /// falls back to `ValidationReport::is_conformant` (no errors), which keeps the
    /// default render byte-identical to the `Display` output. Set it so the printed
    /// conformance line and the JSON `conformant` field match the exit code under
    /// `--strict` (where a warning-only report is not acceptable).
    pub acceptable: Option<bool>,
}

/// The summary counts for a report, derived from the full diagnostic list.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ReportSummary {
    /// Number of [`crate::Severity::Error`] diagnostics.
    pub errors: usize,
    /// Number of [`crate::Severity::Warning`] diagnostics.
    pub warnings: usize,
    /// Number of [`crate::Severity::Info`] diagnostics.
    pub info: usize,
    /// The pass/fail decision: [`RenderOptions::acceptable`] when set (so it tracks
    /// the exit code under `--strict`), otherwise whether the report has no errors.
    pub conformant: bool,
}

/// Truncation metadata, present only when `--max-diagnostics` actually capped the
/// diagnostic list. Counts are computed from the full report.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Truncation {
    /// How many diagnostics are shown.
    pub shown: usize,
    /// How many diagnostics the report contains in total.
    pub total: usize,
    /// How many diagnostics were omitted (`total - shown`).
    pub omitted: usize,
}

/// A machine-readable, presentation-shaped view of a report for `--json`.
///
/// With [`RenderOptions::default`] this serializes to exactly `{"diagnostics": […]}`
/// — byte-compatible with the historical `serde_json` output of
/// [`ValidationReport`]. `summary` is present under [`RenderOptions::summary_only`]
/// and whenever `--max-diagnostics` truncated the list (so a consumer counting the
/// capped array still has the true counts); `truncation` only when capping omitted
/// some diagnostics. The `diagnostics` key is always present (empty under
/// summary-only) so naive consumers never break.
#[derive(Debug, Serialize)]
pub struct RenderedReport<'a> {
    /// Summary counts; present under summary-only or when capping truncated the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReportSummary>,
    /// The (possibly capped) diagnostics; empty under summary-only.
    pub diagnostics: Vec<&'a Diagnostic>,
    /// Truncation metadata; present only when capping omitted diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<Truncation>,
}

impl ValidationReport {
    /// The `(errors, warnings, info)` counts over the full report, matching the
    /// [`core::fmt::Display`] tally.
    fn counts(&self) -> (usize, usize, usize) {
        let errors = self.errors().count();
        let warnings = self.warnings().count();
        let info = self.diagnostics.len().saturating_sub(errors + warnings);
        (errors, warnings, info)
    }

    /// The error-based summary counts derived from the full report (`conformant`
    /// reflects [`ValidationReport::is_conformant`], i.e. no errors).
    #[must_use]
    pub fn summary(&self) -> ReportSummary {
        self.build_summary(self.is_conformant())
    }

    /// Builds a [`ReportSummary`] over the full counts with the given conformance.
    fn build_summary(&self, conformant: bool) -> ReportSummary {
        let (errors, warnings, info) = self.counts();
        ReportSummary {
            errors,
            warnings,
            info,
            conformant,
        }
    }

    /// The conformance to report: [`RenderOptions::acceptable`] when set, else
    /// [`ValidationReport::is_conformant`] (so the default render matches `Display`).
    fn conformant_for(&self, options: &RenderOptions) -> bool {
        options.acceptable.unwrap_or_else(|| self.is_conformant())
    }

    /// Renders the report as text under `options`. With
    /// [`RenderOptions::default`] this equals the [`core::fmt::Display`] output. The
    /// summary line counts are always computed from the full report.
    #[must_use]
    pub fn render_text(&self, options: &RenderOptions) -> String {
        let mut out = String::new();
        if !options.summary_only {
            let cap = options.max_diagnostics.unwrap_or(self.diagnostics.len());
            for diagnostic in self.diagnostics.iter().take(cap) {
                let _ = writeln!(out, "{diagnostic}");
            }
            let omitted = self.diagnostics.len().saturating_sub(cap);
            if omitted > 0 {
                let _ = writeln!(
                    out,
                    "... {omitted} more diagnostic(s) not shown (--max-diagnostics {cap})"
                );
            }
        }
        let (errors, warnings, info) = self.counts();
        let _ = writeln!(out, "{errors} error(s), {warnings} warning(s), {info} info");
        if self.conformant_for(options) {
            let _ = writeln!(out, "conformant: no errors found");
        } else {
            let _ = writeln!(out, "NOT conformant");
        }
        out
    }

    /// Builds the machine-readable view of the report under `options`.
    #[must_use]
    pub fn rendered(&self, options: &RenderOptions) -> RenderedReport<'_> {
        let conformant = self.conformant_for(options);
        if options.summary_only {
            return RenderedReport {
                summary: Some(self.build_summary(conformant)),
                diagnostics: Vec::new(),
                truncation: None,
            };
        }
        let total = self.diagnostics.len();
        let cap = options.max_diagnostics.unwrap_or(total);
        let diagnostics: Vec<&Diagnostic> = self.diagnostics.iter().take(cap).collect();
        let shown = diagnostics.len();
        let truncation = (total > shown).then_some(Truncation {
            shown,
            total,
            omitted: total - shown,
        });
        let summary = truncation.is_some().then(|| self.build_summary(conformant));
        RenderedReport {
            summary,
            diagnostics,
            truncation,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::diagnostic::Diagnostic;
    use splot_core::span::ByteOffset;

    fn report() -> ValidationReport {
        let mut report = ValidationReport::new();
        report.push(Diagnostic::warning(
            "obu-header/test-warning",
            "first warning",
        ));
        report.push(
            Diagnostic::error("obu-header/test-error-1", "first error")
                .with_byte_offset(ByteOffset::new(3)),
        );
        report.push(Diagnostic::error("obu-header/test-error-2", "second error"));
        report
    }

    #[test]
    fn default_render_text_equals_display() {
        let report = report();
        assert_eq!(
            report.render_text(&RenderOptions::default()),
            format!("{report}")
        );
    }

    #[test]
    fn max_diagnostics_caps_and_notes_omitted() {
        let report = report();
        let opts = RenderOptions {
            max_diagnostics: Some(1),
            summary_only: false,
            acceptable: None,
        };
        let text = report.render_text(&opts);
        assert!(text.contains("obu-header/test-warning"), "{text}");
        assert!(!text.contains("obu-header/test-error-2"), "{text}");
        assert!(
            text.contains("... 2 more diagnostic(s) not shown (--max-diagnostics 1)"),
            "{text}"
        );
        assert!(text.contains("2 error(s), 1 warning(s), 0 info"), "{text}");
        assert!(text.contains("NOT conformant"), "{text}");
    }

    #[test]
    fn summary_only_omits_diagnostic_lines() {
        let report = report();
        let opts = RenderOptions {
            max_diagnostics: None,
            summary_only: true,
            acceptable: None,
        };
        let text = report.render_text(&opts);
        assert!(!text.contains("obu-header/test-error-1"), "{text}");
        assert!(!text.contains("not shown"), "{text}");
        assert!(text.contains("2 error(s), 1 warning(s), 0 info"), "{text}");
        assert!(text.contains("NOT conformant"), "{text}");
    }

    #[test]
    fn rendered_default_has_all_diagnostics_no_extras() {
        let report = report();
        let rendered = report.rendered(&RenderOptions::default());
        assert_eq!(rendered.diagnostics.len(), 3);
        assert!(rendered.summary.is_none());
        assert!(rendered.truncation.is_none());
    }

    #[test]
    fn rendered_cap_sets_truncation_from_full_report() {
        let report = report();
        let opts = RenderOptions {
            max_diagnostics: Some(2),
            summary_only: false,
            acceptable: None,
        };
        let rendered = report.rendered(&opts);
        assert_eq!(rendered.diagnostics.len(), 2);
        let truncation = rendered.truncation.expect("capped report has truncation");
        assert_eq!(truncation.shown, 2);
        assert_eq!(truncation.total, 3);
        assert_eq!(truncation.omitted, 1);
    }

    #[test]
    fn rendered_summary_only_omits_diagnostics_keeps_key() {
        let report = report();
        let opts = RenderOptions {
            max_diagnostics: Some(1),
            summary_only: true,
            acceptable: None,
        };
        let rendered = report.rendered(&opts);
        assert!(rendered.diagnostics.is_empty());
        assert!(rendered.truncation.is_none());
        let summary = rendered.summary.expect("summary-only sets summary");
        assert_eq!(summary.errors, 2);
        assert_eq!(summary.warnings, 1);
        assert!(!summary.conformant);
    }

    #[test]
    fn max_zero_omits_all_without_panicking() {
        let report = report();
        let opts = RenderOptions {
            max_diagnostics: Some(0),
            summary_only: false,
            acceptable: None,
        };
        let text = report.render_text(&opts);
        assert!(
            text.contains("... 3 more diagnostic(s) not shown (--max-diagnostics 0)"),
            "{text}"
        );
        let rendered = report.rendered(&opts);
        assert!(rendered.diagnostics.is_empty());
        assert_eq!(rendered.truncation.map(|t| t.omitted), Some(3));
    }

    #[test]
    fn acceptable_override_drives_conformance() {
        let mut report = ValidationReport::new();
        report.push(Diagnostic::warning("obu-header/test-warning", "a warning"));
        assert!(report.is_conformant());

        let strict = RenderOptions {
            max_diagnostics: None,
            summary_only: true,
            acceptable: Some(false),
        };
        let text = report.render_text(&strict);
        assert!(text.contains("NOT conformant"), "{text}");
        assert!(!report.rendered(&strict).summary.unwrap().conformant);

        let default = RenderOptions {
            summary_only: true,
            ..RenderOptions::default()
        };
        assert!(
            report
                .render_text(&default)
                .contains("conformant: no errors found")
        );
        assert!(report.rendered(&default).summary.unwrap().conformant);
    }

    #[test]
    fn capped_json_includes_full_summary() {
        let report = report();
        let opts = RenderOptions {
            max_diagnostics: Some(1),
            summary_only: false,
            acceptable: Some(false),
        };
        let rendered = report.rendered(&opts);
        assert_eq!(rendered.diagnostics.len(), 1);
        assert!(rendered.truncation.is_some());
        let summary = rendered
            .summary
            .expect("capped output carries the full summary");
        assert_eq!(summary.errors, 2);
        assert_eq!(summary.warnings, 1);
        assert!(!summary.conformant);
    }
}
