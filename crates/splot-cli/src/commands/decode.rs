// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot decode` — future reference-style decode / round-trip entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Args;
use serde::Serialize;

/// Arguments for `splot decode`.
#[derive(Args, Debug)]
pub struct DecodeArgs {
    /// Emit the unsupported decode diagnostic as JSON.
    #[arg(long)]
    pub json: bool,
    /// Input AV2 bitstream.
    pub input: PathBuf,
    /// Output decoded video file (Y4M).
    #[arg(short = 'o', long)]
    pub output: PathBuf,
}

#[derive(Serialize)]
struct DecodeUnsupportedDiagnostic {
    code: &'static str,
    severity: &'static str,
    spec_section: &'static str,
    matrix_row: &'static str,
    feature_id: &'static str,
    message: &'static str,
    remediation: &'static str,
}

const UNSUPPORTED_DIAGNOSTIC: DecodeUnsupportedDiagnostic = DecodeUnsupportedDiagnostic {
    code: "decode/unsupported-feature",
    severity: "error",
    spec_section: "7.1",
    matrix_row: "cli-decode-entrypoint",
    feature_id: "CLI-DECODE",
    message: "`splot decode` is not implemented for AV2 bitstreams yet.",
    remediation: "Use `splot validate` or `splot inspect` for bitstream analysis until CLI-DECODE is implemented.",
};

/// Runs `splot decode`. Reference-style decode / round-trip testing is a future
/// milestone, so this exits non-zero.
///
/// # Errors
/// Returns an error if the JSON diagnostic cannot be serialized.
pub fn run(args: &DecodeArgs) -> Result<ExitCode> {
    let _ = (&args.input, &args.output);

    if args.json {
        let json = serde_json::to_string_pretty(&UNSUPPORTED_DIAGNOSTIC)
            .context("failed to serialize decode unsupported diagnostic")?;
        println!("{json}");
    } else {
        eprintln!("code: {}", UNSUPPORTED_DIAGNOSTIC.code);
        eprintln!("severity: {}", UNSUPPORTED_DIAGNOSTIC.severity);
        eprintln!("spec_section: {}", UNSUPPORTED_DIAGNOSTIC.spec_section);
        eprintln!("matrix_row: {}", UNSUPPORTED_DIAGNOSTIC.matrix_row);
        eprintln!("feature_id: {}", UNSUPPORTED_DIAGNOSTIC.feature_id);
        eprintln!("message: {}", UNSUPPORTED_DIAGNOSTIC.message);
        eprintln!("remediation: {}", UNSUPPORTED_DIAGNOSTIC.remediation);
    }

    Ok(ExitCode::from(1))
}
