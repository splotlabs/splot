// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot decode` — future reference-style decode / round-trip entry point.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, ValueEnum};
use serde::Serialize;
use splot_decode::{DecodeDiagnostic, unsupported_feature_diagnostic};

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

fn render_text_diagnostic(diagnostic: &DecodeDiagnostic) {
    eprintln!("rule_id: {}", diagnostic.rule_id);
    eprintln!("severity: {}", diagnostic.severity);
    eprintln!("spec_section: {}", spec_section_text(diagnostic));
    eprintln!("matrix_row: {}", diagnostic.matrix_row);
    eprintln!("feature_id: {}", diagnostic.feature_id);
    eprintln!("message: {}", diagnostic.message);
    eprintln!("remediation: {}", diagnostic.remediation);
}

fn spec_section_text(diagnostic: &DecodeDiagnostic) -> &'static str {
    diagnostic.spec_section.unwrap_or_default()
}

#[derive(Serialize)]
struct DecodeDiagnosticJson<'a> {
    rule_id: &'a str,
    severity: &'a str,
    spec_section: &'a str,
    matrix_row: &'a str,
    feature_id: &'a str,
    message: &'a str,
    remediation: &'a str,
}

impl<'a> From<&'a DecodeDiagnostic> for DecodeDiagnosticJson<'a> {
    fn from(diagnostic: &'a DecodeDiagnostic) -> Self {
        Self {
            rule_id: diagnostic.rule_id,
            severity: diagnostic.severity.as_str(),
            spec_section: spec_section_text(diagnostic),
            matrix_row: diagnostic.matrix_row,
            feature_id: diagnostic.feature_id,
            message: diagnostic.message,
            remediation: diagnostic.remediation,
        }
    }
}

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

    let diagnostic = unsupported_feature_diagnostic();

    if args.json {
        let json = serde_json::to_string_pretty(&DecodeDiagnosticJson::from(&diagnostic))
            .context("failed to serialize decode unsupported diagnostic")?;
        println!("{json}");
    } else {
        render_text_diagnostic(&diagnostic);
    }

    Ok(ExitCode::from(1))
}
