// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot validate` — validate raw AV2 Annex B or IVF-wrapped Annex B bitstreams.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Args;
use splot_validate::Validator;

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
}

/// Runs `splot validate`.
///
/// Exit codes: `0` if conformant, `1` if validation errors exist (or, with
/// `--strict`, any warnings), and `2` (via an `Err`) for I/O failures.
///
/// # Errors
/// Returns an error if the input file cannot be read or the report cannot be
/// serialized.
pub fn run(args: &ValidateArgs) -> Result<ExitCode> {
    let data = read_input(&args.input)?;
    let validator = Validator::new(args.strict);
    let report = validator.validate_bytes(&data);

    if args.json {
        let json = serde_json::to_string_pretty(&report).context("failed to serialize report")?;
        println!("{json}");
    } else {
        print!("{report}");
    }

    Ok(if validator.is_acceptable(&report) {
        ExitCode::from(0)
    } else {
        ExitCode::from(1)
    })
}
