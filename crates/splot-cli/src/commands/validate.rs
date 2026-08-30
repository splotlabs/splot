// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot validate` — validate raw AV2 Annex B or IVF-wrapped Annex B bitstreams.

use std::fs::File;
use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Args;
use splot_validate::{RenderOptions, Validator};

/// Arguments for `splot validate`.
#[derive(Args, Debug)]
pub(crate) struct ValidateArgs {
    /// Path to a raw AV2 Annex B bitstream or IVF-wrapped Annex B stream, or `-`
    /// for standard input.
    input: PathBuf,
    /// Emit the validation report as JSON.
    #[arg(long)]
    json: bool,
    /// Treat warnings as conformance failures.
    #[arg(long)]
    strict: bool,
    /// Show at most N diagnostics, with a truncation notice for the rest. Does not
    /// change which diagnostics are computed, the summary counts, or the exit code.
    #[arg(long, value_name = "N")]
    max_diagnostics: Option<usize>,
    /// Print only the summary counts and the conformance line (no per-diagnostic
    /// lines). Exit code is unchanged. Distinct from the global `--quiet` (logging).
    #[arg(long)]
    summary_only: bool,
}

/// Runs `splot validate`.
///
/// The input is streamed one temporal unit at a time (forward-only), so peak
/// memory is bounded by the largest unit rather than the whole file; `-` reads
/// from standard input.
///
/// Exit codes: `0` if conformant, `1` if validation errors exist (or, with
/// `--strict`, any warnings), and `2` (via an `Err`) for I/O failures. The
/// `--max-diagnostics` / `--summary-only` flags affect only presentation; the exit
/// code always derives from the full report.
///
/// # Errors
/// Returns an error if the input cannot be opened or read (including a temporal
/// unit exceeding the reader's per-unit cap) or the report cannot be serialized.
pub(crate) fn run(args: &ValidateArgs) -> Result<ExitCode> {
    let validator = Validator::new(args.strict);
    let report = if args.input.as_os_str() == "-" {
        validator
            .validate_reader(BufReader::new(io::stdin().lock()))
            .context("failed to validate standard input")?
    } else {
        let file = File::open(&args.input)
            .with_context(|| format!("failed to open input file: {}", args.input.display()))?;
        validator
            .validate_reader(BufReader::new(file))
            .with_context(|| format!("failed to validate input file: {}", args.input.display()))?
    };

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
