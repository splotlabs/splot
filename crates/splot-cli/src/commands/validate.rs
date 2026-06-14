// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot validate` — validate raw AV2 Annex B or IVF-wrapped Annex B bitstreams.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Args;
use splot_validate::{RenderOptions, Validator};

use crate::commands::read_input;

/// Arguments for `splot validate`.
#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Path to a raw AV2 Annex B bitstream or IVF-wrapped Annex B stream.
    pub input: PathBuf,
    /// Emit the validation report as JSON.
    #[arg(long)]
    pub json: bool,
    /// Treat warnings as conformance failures.
    #[arg(long)]
    pub strict: bool,
    /// Show at most N diagnostics, with a truncation notice for the rest. Does not
    /// change which diagnostics are computed, the summary counts, or the exit code.
    #[arg(long, value_name = "N")]
    pub max_diagnostics: Option<usize>,
    /// Print only the summary counts and the conformance line (no per-diagnostic
    /// lines). Exit code is unchanged. Distinct from the global `--quiet` (logging).
    #[arg(long)]
    pub summary_only: bool,
}

/// Runs `splot validate`.
///
/// Exit codes: `0` if conformant, `1` if validation errors exist (or, with
/// `--strict`, any warnings), and `2` (via an `Err`) for I/O failures. The
/// `--max-diagnostics` / `--summary-only` flags affect only presentation; the exit
/// code always derives from the full report.
///
/// # Errors
/// Returns an error if the input file cannot be read or the report cannot be
/// serialized.
pub fn run(args: &ValidateArgs) -> Result<ExitCode> {
    let data = read_input(&args.input)?;
    let validator = Validator::new(args.strict);
    let report = validator.validate_bytes(&data);

    // The single pass/fail decision (honors --strict) drives both the exit code and
    // the reported conformance, so the summary never contradicts the exit code.
    let acceptable = validator.is_acceptable(&report);
    let render = RenderOptions {
        max_diagnostics: args.max_diagnostics,
        summary_only: args.summary_only,
        acceptable: Some(acceptable),
    };
    if args.json {
        let json = serde_json::to_string_pretty(&report.rendered(&render))
            .context("failed to serialize report")?;
        println!("{json}");
    } else {
        print!("{}", report.render_text(&render));
    }

    Ok(if acceptable {
        ExitCode::from(0)
    } else {
        ExitCode::from(1)
    })
}
