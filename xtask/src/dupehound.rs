// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Duplicate-code budget gate (`cargo xtask check-duplication`).
//!
//! Enforces the committed *production*-duplication ceiling in
//! `tools/dupehound/budget.toml` against the deletable-line count reported by
//! [`dupehound`](https://github.com/Rafaelpta/dupehound). The gate fails only
//! when the measured count *exceeds* the ceiling, so production duplication can
//! never silently regress; the ceiling is ratcheted down whenever a duplicate
//! cluster is removed. The complementary per-PR ratchet
//! (`dupehound check --diff <base>`) lives in `.github/workflows/ci.yml`.
//!
//! This is a **ratchet, not a zero mandate**. Real codebases carry legitimate
//! duplication — deliberately-explicit per-scenario tests, intentional
//! parser/writer decoupling, `&self`/`&mut self` accessor pairs. The goal is to
//! *prevent new* duplication (the PR ratchet) and *reduce* the production
//! duplication that hurts maintainability (this budget), never to force a zero
//! that would require deleting intentional code. The scope is dupehound's
//! default, which excludes `#[test]` bodies — see `check_duplication`.
//!
//! Like the other external-tool checks (`typos`, `cargo-machete`, `cargo-deny`),
//! this follows the run-if-present policy: it is mandatory in CI (the workflow
//! installs `dupehound`) and skipped locally with an install hint when the
//! binary is absent, so a fresh checkout can still run `cargo xtask ci`.

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::tool_available;

/// Committed budget file, relative to the workspace root.
const BUDGET_PATH: &str = "tools/dupehound/budget.toml";

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

    let budget_path = root.join(BUDGET_PATH);
    let budget_text = std::fs::read_to_string(&budget_path)
        .with_context(|| format!("failed to read {}", budget_path.display()))?;
    let budget: Budget = toml::from_str(&budget_text)
        .with_context(|| format!("failed to parse {}", budget_path.display()))?;

    let display = "dupehound scan <root> --json";
    eprintln!("> {display}");
    let output = Command::new("dupehound")
        .arg("scan")
        .arg(root)
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

    let report: ScanReport = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("failed to parse JSON from `{display}`"))?;

    let message = enforce_budget(report.score.deletable_lines, budget.max_deletable_lines)?;
    eprintln!("{message}");
    Ok(())
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
