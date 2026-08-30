// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot explain` — describe a validator diagnostic rule id (or list them all).
//!
//! Reads the doc-generated registry in `splot-validate` (`explain` module); it
//! changes no validator behavior and only reports what the validator already
//! documents. An unknown rule id is a clean operational error (exit 2), never a
//! panic.

use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Args;
use splot_validate::explain;

/// Arguments for `splot explain`.
#[derive(Args, Debug)]
pub(crate) struct ExplainArgs {
    /// The diagnostic rule id to describe (e.g. `obu-header/global-xlayer-required`).
    /// Omit it with `--list` to enumerate every rule id instead.
    #[arg(value_name = "RULE_ID")]
    rule_id: Option<String>,
    /// List every known rule id instead of describing one.
    #[arg(long)]
    list: bool,
    /// Emit the output as JSON.
    #[arg(long)]
    json: bool,
}

/// Runs `splot explain`.
///
/// Exit codes: `0` on a successful describe/list; `2` (via an `Err`) for an unknown
/// rule id or a missing argument.
///
/// # Errors
/// Returns an error for an unknown rule id, a missing rule id without `--list`, or
/// a JSON serialization failure.
pub(crate) fn run(args: &ExplainArgs) -> Result<ExitCode> {
    if args.list {
        list(args.json)?;
        return Ok(ExitCode::SUCCESS);
    }

    let Some(rule_id) = args.rule_id.as_deref() else {
        bail!("provide a rule id to explain, or `--list` to see them all");
    };

    let Some(info) = explain::explain(rule_id) else {
        let hints = explain::did_you_mean(rule_id);
        let suggestion = if hints.is_empty() {
            "run `splot explain --list` to see every rule id".to_owned()
        } else {
            format!(
                "did you mean: {}? (run `splot explain --list` to see all)",
                hints.join(", ")
            )
        };
        bail!("unknown rule id `{rule_id}`; {suggestion}");
    };

    if args.json {
        let json = serde_json::to_string_pretty(info)?;
        println!("{json}");
    } else {
        print_info(info);
    }
    Ok(ExitCode::SUCCESS)
}

/// Lists every known rule id (text: one id per line; JSON: the full catalog).
fn list(json: bool) -> Result<()> {
    if json {
        let json = serde_json::to_string_pretty(explain::all())?;
        println!("{json}");
    } else {
        for info in explain::all() {
            println!("{}", info.rule_id);
        }
    }
    Ok(())
}

/// Prints the human-readable description of one diagnostic.
fn print_info(info: &splot_validate::DiagnosticInfo) {
    println!("{}", info.rule_id);
    println!("  severity: {}", info.severity);
    match info.section {
        Some(section) => println!("  section:  {section}"),
        None => println!("  section:  (none recorded)"),
    }
    println!("  summary:  {}", info.summary);
    println!("\nFull registry: docs/DIAGNOSTICS.md");
}
