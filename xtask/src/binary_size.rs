// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The release binary size budget (`cargo xtask check-binary-size`).
//!
//! Builds the production `splot` binary (`cargo build --release -p splot-cli
//! --locked`: stripped, the full command set, no optional features) and fails
//! when its file size exceeds the budget of the host target in
//! `tests/perf/binary-size.toml`. A size change does not show in tests or in
//! the instruction-count gates, so this is the only gate that sees it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// Budget file, relative to the workspace root.
const BUDGET_REL: &str = "tests/perf/binary-size.toml";
/// A binary this far below its budget asks for a lower budget.
const SLACK_BYTES: u64 = 100_000;

#[derive(Deserialize)]
struct Budgets {
    /// Largest allowed size in bytes, by `<arch>-<os>`.
    budget: BTreeMap<String, u64>,
}

/// Builds the release binary and checks its size against the host budget.
pub(crate) fn check_binary_size(root: &Path) -> Result<()> {
    let status = Command::new(env!("CARGO"))
        .current_dir(root)
        .args(["build", "--release", "-p", "splot-cli", "--locked"])
        .status()
        .context("failed to run cargo build")?;
    if !status.success() {
        bail!("cargo build --release -p splot-cli failed");
    }
    let binary = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("release/splot")
        .with_extension(std::env::consts::EXE_EXTENSION);
    let size = std::fs::metadata(&binary)
        .with_context(|| format!("failed to read {}", binary.display()))?
        .len();
    let text = std::fs::read_to_string(root.join(BUDGET_REL))
        .with_context(|| format!("failed to read {BUDGET_REL}"))?;
    let budgets: Budgets =
        toml::from_str(&text).with_context(|| format!("failed to parse {BUDGET_REL}"))?;
    let target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    println!(
        "{}",
        verdict(size, budgets.budget.get(&target).copied(), &target)?
    );
    Ok(())
}

fn verdict(size: u64, budget: Option<u64>, target: &str) -> Result<String> {
    let Some(budget) = budget else {
        bail!("{target}: splot is {size} bytes and {BUDGET_REL} has no budget for this target");
    };
    if size > budget {
        bail!(
            "{target}: splot is {size} bytes, {} over the budget of {budget}; \
             remove the growth, or raise the budget in {BUDGET_REL} and give the reason in the PR",
            size - budget
        );
    }
    let mut line = format!("{target}: splot is {size} bytes (budget {budget})");
    if budget - size > SLACK_BYTES {
        line.push_str("; lower the budget to keep the saving");
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::verdict;

    #[test]
    fn size_over_budget_or_without_budget_fails() {
        assert!(verdict(101, Some(100), "x86_64-linux").is_err());
        assert!(verdict(100, None, "riscv64-linux").is_err());
    }

    #[test]
    fn size_within_budget_passes_and_reports_slack() {
        let exact = verdict(100, Some(100), "x86_64-linux").unwrap_or_default();
        assert!(exact.contains("100 bytes") && !exact.contains("lower"));
        let slack = verdict(100, Some(200_000), "x86_64-linux").unwrap_or_default();
        assert!(slack.contains("lower the budget"));
    }
}
