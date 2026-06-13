// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot decode` — future reference-style decode / round-trip entry point.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;

/// Output artifact selected for future `splot decode` success.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum DecodeOutputFormat {
    /// Future Y4M decoded-video output.
    Y4m,
    /// Future deterministic decoded-frame hash output.
    Hash,
}

/// Arguments for `splot decode`.
#[derive(Args, Debug)]
pub struct DecodeArgs {
    /// Emit the unsupported decode diagnostic as JSON.
    #[arg(long)]
    pub json: bool,
    /// Select the future decode output artifact.
    #[arg(long = "output-format", value_enum, requires_if("y4m", "output"))]
    pub output_format: Option<DecodeOutputFormat>,
    /// Input AV2 bitstream.
    pub input: PathBuf,
    /// Output path for the selected artifact.
    #[arg(short = 'o', long, required_unless_present = "output_format")]
    pub output: Option<PathBuf>,
}

#[derive(Debug)]
enum DecodeOutputTarget<'a> {
    Y4m { path: &'a Path },
    Hash { path: Option<&'a Path> },
}

impl<'a> DecodeOutputTarget<'a> {
    fn path(&self) -> Option<&'a Path> {
        match self {
            Self::Y4m { path } => Some(path),
            Self::Hash { path } => *path,
        }
    }
}

impl DecodeArgs {
    fn output_target(&self) -> Option<DecodeOutputTarget<'_>> {
        match (
            self.output_format.unwrap_or(DecodeOutputFormat::Y4m),
            self.output.as_deref(),
        ) {
            (DecodeOutputFormat::Y4m, Some(path)) => Some(DecodeOutputTarget::Y4m { path }),
            (DecodeOutputFormat::Y4m, None) => None,
            (DecodeOutputFormat::Hash, path) => Some(DecodeOutputTarget::Hash { path }),
        }
    }
}

#[derive(Serialize)]
struct DecodeUnsupportedDiagnostic {
    rule_id: &'static str,
    severity: &'static str,
    spec_section: &'static str,
    matrix_row: &'static str,
    feature_id: &'static str,
    message: &'static str,
    remediation: &'static str,
}

const UNSUPPORTED_DIAGNOSTIC: DecodeUnsupportedDiagnostic = DecodeUnsupportedDiagnostic {
    rule_id: "decode/unsupported-feature",
    severity: "Error",
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
    let target = args.output_target();
    let _ = (
        &args.input,
        target.as_ref().and_then(DecodeOutputTarget::path),
    );

    if args.json {
        let json = serde_json::to_string_pretty(&UNSUPPORTED_DIAGNOSTIC)
            .context("failed to serialize decode unsupported diagnostic")?;
        println!("{json}");
    } else {
        eprintln!("rule_id: {}", UNSUPPORTED_DIAGNOSTIC.rule_id);
        eprintln!("severity: {}", UNSUPPORTED_DIAGNOSTIC.severity);
        eprintln!("spec_section: {}", UNSUPPORTED_DIAGNOSTIC.spec_section);
        eprintln!("matrix_row: {}", UNSUPPORTED_DIAGNOSTIC.matrix_row);
        eprintln!("feature_id: {}", UNSUPPORTED_DIAGNOSTIC.feature_id);
        eprintln!("message: {}", UNSUPPORTED_DIAGNOSTIC.message);
        eprintln!("remediation: {}", UNSUPPORTED_DIAGNOSTIC.remediation);
    }

    Ok(ExitCode::from(1))
}
