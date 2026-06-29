// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Validator stream orchestration and check execution.

use splot_core::annexb::ObuEnvelope;
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};

use crate::checks::{Check, default_checks};
use crate::context::ValidatorContext;
use crate::diagnostic::ValidationReport;
use crate::options::ValidationOptions;

use super::diagnostics::{ivf_error_diagnostic, ivf_warning_diagnostic, parse_error_diagnostic};

pub(super) fn validate_bytes_with_options(
    data: &[u8],
    options: &ValidationOptions,
) -> ValidationReport {
    let mut report = ValidationReport::new();
    let parsed = parse_bitstream_partial(data);
    let checks = default_checks();
    let mut context = ValidatorContext::default();

    match parsed {
        ParsedBitstream::AnnexB(parsed) => {
            for obu in &parsed.obus {
                process_obu(&mut context, checks, obu, options, &mut report);
            }
            context.finish(options, &mut report);
            if let Some(error) = parsed.error {
                report.push(parse_error_diagnostic(&error));
            }
        }
        ParsedBitstream::Ivf(parsed) => {
            for frame in &parsed.frames {
                for obu in &frame.obus {
                    process_obu(&mut context, checks, obu, options, &mut report);
                }
                if let Some(error) = &frame.error {
                    report.push(parse_error_diagnostic(error));
                }
            }
            context.finish(options, &mut report);
            for warning in &parsed.warnings {
                report.push(ivf_warning_diagnostic(warning));
            }
            if let Some(error) = &parsed.error {
                report.push(ivf_error_diagnostic(error));
            }
        }
    }

    report
}

/// Observes one OBU into the validator context and runs the check registry over
/// it. Shared by the in-memory ([`validate_bytes_with_options`]) and streaming
/// ([`super::streaming`]) paths so both apply identical per-OBU semantics.
pub(super) fn process_obu(
    context: &mut ValidatorContext,
    checks: &[&dyn Check],
    obu: &ObuEnvelope<'_>,
    options: &ValidationOptions,
    report: &mut ValidationReport,
) {
    context.observe_obu(obu, options, report);
    run_checks(checks, obu, report);
}

fn run_checks(checks: &[&dyn Check], obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
    for check in checks {
        check.run(obu, report);
    }
}
