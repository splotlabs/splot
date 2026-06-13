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
    // Parse the whole stream, keeping OBUs parsed before any later structural
    // error so their conformance diagnostics are not lost.
    let parsed = parse_bitstream_partial(data);
    let checks = default_checks();
    let mut context = ValidatorContext::default();

    match parsed {
        ParsedBitstream::AnnexB(parsed) => {
            for obu in &parsed.obus {
                context.observe_obu(obu, options, &mut report);
                run_checks(&checks, obu, &mut report);
            }
            // The end of the bitstream completes the final temporal unit, flushing
            // the deferred coded-video-sequence-scoped diagnostics (AV2 § 7.3.6;
            // see ValidatorContext::finish).
            context.finish(options, &mut report);
            if let Some(error) = parsed.error {
                report.push(parse_error_diagnostic(&error));
            }
        }
        ParsedBitstream::Ivf(parsed) => {
            for frame in &parsed.frames {
                for obu in &frame.obus {
                    context.observe_obu(obu, options, &mut report);
                    run_checks(&checks, obu, &mut report);
                }
                if let Some(error) = &frame.error {
                    report.push(parse_error_diagnostic(error));
                }
            }
            // The end of the IVF input completes the final temporal unit just like
            // the end of a raw Annex B bitstream.
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

fn run_checks(checks: &[Box<dyn Check>], obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
    for check in checks {
        check.run(obu, report);
    }
}
