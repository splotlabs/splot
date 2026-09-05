// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Duplicate-code budget gate (`cargo xtask check-duplication`).
//!
//! Checks production duplicate lines against `tools/dupehound/budget.toml`.
//! Test bodies and source test-module files are excluded.
//!
//! CI installs the pinned `dupehound` version. Local runs skip the check with
//! an installation hint when the binary is absent.

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::tool_available;

/// Committed budget file, relative to the workspace root.
const BUDGET_PATH: &str = "tools/dupehound/budget.toml";
const TEST_MODULE_EXCLUDES: &[&str] = &["*_tests.rs", "*/tests.rs"];

/// The `[budget]`-less top-level of `tools/dupehound/budget.toml`.
#[derive(Debug, Deserialize)]
struct Budget {
    /// Maximum number of deletable duplicate lines tolerated workspace-wide.
    max_deletable_lines: u64,
}

/// The subset of `dupehound scan --json` this gate reads.
#[derive(Debug, Deserialize)]
struct ScanReport {
    score: ScanScore,
}

/// The `score` object of a dupehound scan report.
#[derive(Debug, Deserialize)]
struct ScanScore {
    /// Lines deletable if every duplicate cluster kept a single copy.
    deletable_lines: u64,
}

/// Runs the duplicate-code budget gate. Skips (returning `Ok`) when `dupehound`
/// is not installed; CI installs it, so CI always enforces the ceiling.
pub(crate) fn check_duplication(root: &Path) -> Result<()> {
    if !tool_available("dupehound") {
        eprintln!(
            "ci: `dupehound` not installed; skipping `dupehound scan` budget gate.\n     \
             install: `cargo install dupehound` (https://github.com/Rafaelpta/dupehound)"
        );
        return Ok(());
    }

    let budget = read_budget(root)?;
    let report = scan(root)?;
    let message = enforce_budget(report.score.deletable_lines, budget.max_deletable_lines)?;
    eprintln!("{message}");
    Ok(())
}

fn read_budget(root: &Path) -> Result<Budget> {
    crate::util::load_toml(&root.join(BUDGET_PATH))
}

fn scan(root: &Path) -> Result<ScanReport> {
    let display = "dupehound scan <root> --exclude-tests --exclude '*_tests.rs' --exclude '*/tests.rs' --json";
    eprintln!("> {display}");
    let mut command = Command::new("dupehound");
    command.arg("scan").arg(root).arg("--exclude-tests");
    for pattern in TEST_MODULE_EXCLUDES {
        command.arg("--exclude").arg(pattern);
    }
    let output = command
        .arg("--json")
        .output()
        .with_context(|| format!("failed to spawn `{display}`"))?;
    if !output.status.success() {
        bail!(
            "`{display}` failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("failed to parse JSON from `{display}`"))
}

/// Compares the measured deletable-line count against the committed ceiling.
///
/// Fails when `actual` exceeds `ceiling` (an aggregate regression); otherwise
/// returns an `ok` message that, when under budget, nudges the ratchet down.
/// Kept pure (no I/O) so the threshold logic is unit-tested without invoking
/// `dupehound`.
fn enforce_budget(actual: u64, ceiling: u64) -> Result<String> {
    if actual > ceiling {
        bail!(
            "check-duplication: {actual} deletable duplicate lines exceed the budget of {ceiling} \
             (+{} over).\n     New code duplicates existing code. Reuse it instead of \
             reimplementing, or run `dupehound scan .` (and `--explain <N>`) to find the \
             original. Raising the budget in {BUDGET_PATH} is not allowed.",
            actual - ceiling
        );
    }
    if actual < ceiling {
        Ok(format!(
            "check-duplication: ok ({actual} deletable duplicate lines, {} under the {ceiling} \
             budget).\n     Ratchet down: lower max_deletable_lines in {BUDGET_PATH} to {actual}.",
            ceiling - actual
        ))
    } else {
        Ok(format!(
            "check-duplication: ok ({actual} deletable duplicate lines, at budget)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::enforce_budget;

    #[test]
    fn over_budget_is_rejected() {
        let result = enforce_budget(101, 100);
        assert!(result.is_err(), "over budget must fail");
        let msg = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(msg.contains("exceed the budget of 100"), "{msg}");
        assert!(msg.contains("+1 over"), "{msg}");
    }

    #[test]
    fn at_budget_passes() {
        for (actual, ceiling) in [(100_u64, 100_u64), (0, 0)] {
            let result = enforce_budget(actual, ceiling);
            assert!(result.is_ok(), "at budget ({actual}/{ceiling}) must pass");
            let msg = result.ok().unwrap_or_default();
            assert!(msg.contains("at budget"), "{msg}");
        }
    }

    #[test]
    fn under_budget_passes_and_nudges_ratchet() {
        let result = enforce_budget(90, 100);
        assert!(result.is_ok(), "under budget must pass");
        let msg = result.ok().unwrap_or_default();
        assert!(msg.contains("10 under"), "{msg}");
        assert!(msg.contains("lower max_deletable_lines"), "{msg}");
    }
}
