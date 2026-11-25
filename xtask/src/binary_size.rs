// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Release size budgets (`cargo xtask check-binary-size`).
//!
//! Builds the production `splot` CLI binary and `libsplot_capi` shared library and
//! fails when either artifact exceeds the host target budget in
//! `tests/perf/binary-size.toml`.
//! When `readelf` is available, also prints `.text` and `.rodata` section sizes
//! for quick size-growth attribution.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// Budget file, relative to the workspace root.
const BUDGET_REL: &str = "tests/perf/binary-size.toml";
/// An artifact this far below its budget asks for a lower budget.
const SLACK_BYTES: u64 = 100_000;

#[derive(Deserialize)]
struct Budgets {
    /// Largest allowed size in bytes of the `splot` CLI binary, by `<arch>-<os>`.
    budget: BTreeMap<String, u64>,
    /// Largest allowed size in bytes of the `libsplot_capi` shared library, by `<arch>-<os>`.
    capi_budget: BTreeMap<String, u64>,
}

/// Builds the release CLI and C ABI artifacts and checks their sizes against the host budgets.
pub(crate) fn check_binary_size(root: &Path) -> Result<()> {
    let cli_path = crate::conformance::build_splot_binary(root, true)?;
    let status = Command::new(env!("CARGO"))
        .current_dir(root)
        .args(["build", "--release", "-p", "splot-capi", "--locked"])
        .status()
        .context("failed to run cargo build")?;
    if !status.success() {
        bail!("cargo build --release -p splot-capi failed");
    }
    let release_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("release");
    let text = std::fs::read_to_string(root.join(BUDGET_REL))
        .with_context(|| format!("failed to read {BUDGET_REL}"))?;
    let budgets: Budgets =
        toml::from_str(&text).with_context(|| format!("failed to parse {BUDGET_REL}"))?;
    let target = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);

    let capi_filename = format!(
        "{}splot_capi{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let capi_path = release_dir.join(capi_filename);

    check_artifact(
        "splot",
        &cli_path,
        budgets.budget.get(&target).copied(),
        &target,
    )?;
    check_artifact(
        "libsplot_capi",
        &capi_path,
        budgets.capi_budget.get(&target).copied(),
        &target,
    )?;
    Ok(())
}

fn check_artifact(artifact: &str, path: &Path, budget: Option<u64>, target: &str) -> Result<()> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .len();
    let mut line = verdict(artifact, size, budget, target)?;
    if let Some(sections) = section_summary(path) {
        line.push_str(&sections);
    }
    println!("{line}");
    Ok(())
}

fn verdict(artifact: &str, size: u64, budget: Option<u64>, target: &str) -> Result<String> {
    let Some(budget) = budget else {
        bail!(
            "{target}: {artifact} is {size} bytes and {BUDGET_REL} has no budget for this target"
        );
    };
    if size > budget {
        bail!(
            "{target}: {artifact} is {size} bytes, {} over the budget of {budget}; \
             remove the growth, or raise the budget in {BUDGET_REL} and give the reason in the PR",
            size - budget
        );
    }
    let mut line = format!("{target}: {artifact} is {size} bytes (budget {budget})");
    if budget - size > SLACK_BYTES {
        line.push_str("; lower the budget to keep the saving");
    }
    Ok(line)
}

fn section_summary(path: &Path) -> Option<String> {
    let output = Command::new("readelf")
        .args(["-SW"])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_elf_sections(&stdout)
}

fn parse_elf_sections(readelf_output: &str) -> Option<String> {
    let mut text_bytes = None;
    let mut rodata_bytes = None;
    for line in readelf_output.lines() {
        let Some((_, after_nr)) = line.split_once(']') else {
            continue;
        };
        let mut cols = after_nr.split_whitespace();
        let Some(name) = cols.next() else {
            continue;
        };
        if !matches!(name, ".text" | ".rodata") {
            continue;
        }
        let Some(size_hex) = cols.nth(3) else {
            continue;
        };
        let Ok(size) = u64::from_str_radix(size_hex, 16) else {
            continue;
        };
        match name {
            ".text" => text_bytes = Some(size),
            ".rodata" => rodata_bytes = Some(size),
            _ => {}
        }
    }
    match (text_bytes, rodata_bytes) {
        (Some(text), Some(rodata)) => Some(format!(" (.text {text}, .rodata {rodata})")),
        (Some(text), None) => Some(format!(" (.text {text})")),
        (None, Some(rodata)) => Some(format!(" (.rodata {rodata})")),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_elf_sections, verdict};

    #[test]
    fn size_over_budget_or_without_budget_fails() {
        assert!(verdict("splot", 101, Some(100), "x86_64-linux").is_err());
        assert!(verdict("libsplot_capi", 101, Some(100), "x86_64-linux").is_err());
        assert!(verdict("splot", 100, None, "riscv64-linux").is_err());
    }

    #[test]
    fn size_within_budget_passes_and_reports_slack() {
        let exact = verdict("splot", 100, Some(100), "x86_64-linux").unwrap_or_default();
        assert!(exact.contains("splot is 100 bytes") && !exact.contains("lower"));
        let capi_slack =
            verdict("libsplot_capi", 100, Some(200_000), "x86_64-linux").unwrap_or_default();
        assert!(
            capi_slack.contains("libsplot_capi is 100 bytes")
                && capi_slack.contains("lower the budget")
        );
    }

    #[test]
    fn parses_text_and_rodata_from_readelf_output() {
        let sample = "\
  [Nr] Name              Type            Address          Off    Size   ES Flg Lk Inf Al\n\
  [ 0]                   NULL            0000000000000000 000000 000000 00      0   0  0\n\
  [10] .rodata           PROGBITS        0000000000001e40 001e40 1378cc 00 AMS  0   0 32\n\
  [16] .text             PROGBITS        000000000018bc50 18ac50 64f863 00  AX  0   0 16\n";
        assert_eq!(
            parse_elf_sections(sample).as_deref(),
            Some(" (.text 6617187, .rodata 1276108)")
        );
    }
}
