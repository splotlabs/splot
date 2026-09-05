// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Diagnostic conversion helpers for parser/container errors.

use splot_core::Error;
use splot_core::ivf::{IvfError, IvfWarning};

use crate::checks::{payload_parse_error_diagnostic, syntax_error_diagnostic};
use crate::diagnostic::{Diagnostic, Severity};

const IVF_DIAGNOSTIC_RULE_IDS: [&str; 5] = [
    "ivf/truncated-header",
    "ivf/invalid-signature",
    "ivf/invalid-header-length",
    "ivf/truncated-frame-header",
    "ivf/truncated-frame-payload",
];

const IVF_WARNING_DIAGNOSTIC_RULE_IDS: [&str; 1] = ["ivf/trailing-partial-frame-header"];

pub(super) fn parse_error_diagnostic(error: &Error) -> Diagnostic {
    if let Some(diagnostic) = syntax_error_diagnostic(error) {
        return diagnostic;
    }

    payload_parse_error_diagnostic(error, "Annex B")
}

pub(super) fn ivf_error_diagnostic(error: &IvfError) -> Diagnostic {
    debug_assert!(IVF_DIAGNOSTIC_RULE_IDS.contains(&error.rule_id()));
    Diagnostic::new(Severity::Error, error.rule_id(), error.to_string())
        .with_spec_section("IVF")
        .with_byte_offset(error.offset())
}

pub(super) fn ivf_warning_diagnostic(warning: &IvfWarning) -> Diagnostic {
    debug_assert!(IVF_WARNING_DIAGNOSTIC_RULE_IDS.contains(&warning.rule_id()));
    Diagnostic::new(Severity::Warning, warning.rule_id(), warning.to_string())
        .with_spec_section("IVF")
        .with_byte_offset(warning.offset())
}
