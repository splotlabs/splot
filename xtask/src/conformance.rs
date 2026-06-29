// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The committed conformance-corpus runner (`cargo xtask conformance`).
//!
//! Loads `tests/conformance/manifest.toml`, reads each committed vector's bytes,
//! validates them, and prints a per-vector pass/fail summary. This is the
//! ergonomic manual entry point; the CI gate is the `splot-cli` integration test
//! (`crates/splot-cli/tests/conformance.rs`), which shares this manifest format
//! and runs under `cargo test`.
//!
//! There is NO AVM dependency: the runner only validates already-committed vector
//! bytes against the manifest, and never invokes AVM or touches the network. AVM
//! is the LOCAL generator of the committed vectors only (see docs/CONFORMANCE.md).
//!
//! `xtask` is standalone (it depends on no `splot-*` crate), so it cannot call
//! the validator library directly. It shells out to the built `splot` binary's
//! `validate --json` command — a project binary, not AVM — to obtain the emitted
//! diagnostics, and compares them against the manifest just as the integration
//! test does in-process.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;

/// The manifest root: an array of `[[vector]]` entries.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    vector: Vec<VectorEntry>,
}

/// One conformance-corpus vector entry.
#[derive(Debug, Deserialize)]
struct VectorEntry {
    /// Path to the committed vector, relative to `tests/conformance/`.
    path: String,
    /// Human-readable note.
    description: String,
    /// Expected validation outcome.
    expect: Expect,
}

/// The expected validation outcome for a vector. `expect = "clean"` deserializes
/// from the bare string; `expect = { diagnostics = [...] }` from the table.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Expect {
    /// `expect = "clean"`: the validator must report zero errors.
    Clean(CleanTag),
    /// `expect = { diagnostics = [rule_id, ...] }`: the validator must emit
    /// exactly this set of error rule ids.
    Diagnostics { diagnostics: Vec<String> },
}

/// The literal `"clean"` string tag for [`Expect::Clean`].
#[derive(Debug, Deserialize)]
enum CleanTag {
    #[serde(rename = "clean")]
    Clean,
}

/// The minimal shape of `splot validate --json` output: a report with a list of
/// diagnostics, each carrying a `rule_id` and a `severity`.
#[derive(Debug, Deserialize)]
struct ValidationReportJson {
    #[serde(default)]
    diagnostics: Vec<DiagnosticJson>,
}

#[derive(Debug, Deserialize)]
struct DiagnosticJson {
    rule_id: String,
    severity: JsonSeverity,
}

/// The `splot validate --json` severity values. Deserializing into a typed enum
/// (rather than comparing a raw string) makes the cross-process coupling
/// explicit: if `splot` renames or adds a severity, the JSON parse fails loudly
/// here instead of silently miscounting which diagnostics are errors.
#[derive(Debug, PartialEq, Eq, Deserialize)]
enum JsonSeverity {
    Error,
    Warning,
    Info,
}

/// Runs the committed conformance corpus against its manifest and prints a
/// per-vector pass/fail summary. Builds the `splot` binary once, then validates
/// every committed vector with `splot validate --json`. NO AVM is invoked.
pub fn run_conformance(root: &Path) -> Result<()> {
    let conformance_root = root.join("tests").join("conformance");
    let manifest_path = conformance_root.join("manifest.toml");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: Manifest = toml::from_str(&manifest_text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;

    if manifest.vector.is_empty() {
        bail!(
            "manifest {} has no [[vector]] entries",
            manifest_path.display()
        );
    }

    let splot_bin = build_splot_binary(root)?;

    let mut failures: Vec<String> = Vec::new();
    eprintln!(
        "conformance: validating {} committed vector(s) (no AVM)",
        manifest.vector.len()
    );
    for entry in &manifest.vector {
        let vector_path = conformance_root.join(&entry.path);
        let got = validate_vector(&splot_bin, &vector_path)
            .with_context(|| format!("failed to validate committed vector {}", entry.path))?;
        let label = entry.description.lines().next().unwrap_or("").trim();
        match check_entry(entry, &got) {
            Ok(()) => eprintln!(
                "  PASS  {}  [{}]  — {label}",
                entry.path,
                summarize_expect(&entry.expect)
            ),
            Err(reason) => {
                eprintln!("  FAIL  {}  — {reason}", entry.path);
                failures.push(format!("{}: {reason}", entry.path));
            }
        }
    }

    if failures.is_empty() {
        eprintln!("conformance: ok ({} vector(s))", manifest.vector.len());
        Ok(())
    } else {
        bail!("conformance: {} vector(s) failed", failures.len())
    }
}

/// Compares the validator's emitted error rule ids for one vector against the
/// manifest's expectation. Returns `Err(reason)` describing the mismatch.
fn check_entry(entry: &VectorEntry, got: &BTreeSet<String>) -> Result<(), String> {
    match &entry.expect {
        Expect::Clean(CleanTag::Clean) => {
            if got.is_empty() {
                Ok(())
            } else {
                Err(format!("expected `clean` but got error(s): {got:?}"))
            }
        }
        Expect::Diagnostics { diagnostics } => {
            if diagnostics.is_empty() {
                return Err("empty `diagnostics` set; use `expect = \"clean\"` instead".to_owned());
            }
            let want: BTreeSet<String> = diagnostics.iter().cloned().collect();
            if *got == want {
                Ok(())
            } else {
                Err(format!("expected diagnostics {want:?}, got {got:?}"))
            }
        }
    }
}

/// A one-line description of the expected outcome for the summary.
fn summarize_expect(expect: &Expect) -> String {
    match expect {
        Expect::Clean(CleanTag::Clean) => "clean".to_owned(),
        Expect::Diagnostics { diagnostics } => format!("diagnostics {diagnostics:?}"),
    }
}

/// One `cargo build --message-format=json` line we care about: the
/// `compiler-artifact` message that carries the built binary's path.
#[derive(Debug, Deserialize)]
struct CargoArtifact {
    reason: String,
    target: CargoArtifactTarget,
    /// The built executable's absolute path (present for binary artifacts).
    executable: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CargoArtifactTarget {
    name: String,
}

/// Builds the `splot` binary (debug) and returns its path. Building it here means
/// `cargo xtask conformance` works from a clean checkout without a separate build
/// step; it is a project binary, not AVM.
///
/// The path is taken from Cargo's own `--message-format=json` artifact output
/// rather than reconstructed from a guessed `target/debug/` location, so it is
/// correct regardless of `CARGO_TARGET_DIR` / `CARGO_BUILD_TARGET_DIR` /
/// `[build] target-dir` configuration and the platform executable suffix.
fn build_splot_binary(root: &Path) -> Result<PathBuf> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .current_dir(root)
        .args([
            "build",
            "--locked",
            "-p",
            "splot-cli",
            "--bin",
            "splot",
            "--message-format=json",
        ])
        .output()
        .context("failed to spawn `cargo build -p splot-cli`")?;
    if !output.status.success() {
        bail!(
            "`cargo build -p splot-cli` failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout =
        String::from_utf8(output.stdout).context("cargo build JSON output was not UTF-8")?;
    for line in stdout.lines() {
        let Ok(artifact) = serde_json::from_str::<CargoArtifact>(line) else {
            continue; // non-artifact messages (build-script-executed, etc.)
        };
        if artifact.reason == "compiler-artifact"
            && artifact.target.name == "splot"
            && let Some(executable) = artifact.executable
        {
            return Ok(PathBuf::from(executable));
        }
    }
    Err(anyhow!(
        "cargo build did not report a `splot` executable artifact"
    ))
}

/// Runs `splot validate --json <vector>` and returns the set of emitted error
/// rule ids. A validation finding sets a non-zero exit; that is expected for
/// negative vectors, so the exit status is not treated as a runner error — only a
/// spawn failure or unparsable output is.
fn validate_vector(splot_bin: &Path, vector: &Path) -> Result<BTreeSet<String>> {
    let output = Command::new(splot_bin)
        .arg("validate")
        .arg("--json")
        .arg(vector)
        .output()
        .with_context(|| format!("failed to spawn {} validate", splot_bin.display()))?;
    let stdout = String::from_utf8(output.stdout)
        .context("splot validate --json produced non-UTF-8 output")?;
    let report: ValidationReportJson = serde_json::from_str(&stdout)
        .with_context(|| format!("failed to parse splot validate --json output: {stdout}"))?;
    Ok(report
        .diagnostics
        .into_iter()
        .filter(|d| d.severity == JsonSeverity::Error)
        .map(|d| d.rule_id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_entry() -> VectorEntry {
        VectorEntry {
            path: "vectors/valid/x.ivf".to_owned(),
            description: "d".to_owned(),
            expect: Expect::Clean(CleanTag::Clean),
        }
    }

    fn diag_entry(ids: &[&str]) -> VectorEntry {
        VectorEntry {
            path: "vectors/invalid/x.ivf".to_owned(),
            description: "d".to_owned(),
            expect: Expect::Diagnostics {
                diagnostics: ids.iter().map(|s| (*s).to_owned()).collect(),
            },
        }
    }

    fn ids(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn clean_passes_with_no_errors_and_fails_with_errors() {
        assert!(check_entry(&clean_entry(), &ids(&[])).is_ok());
        assert!(check_entry(&clean_entry(), &ids(&["ivf/truncated-frame-payload"])).is_err());
    }

    #[test]
    fn diagnostics_set_equality() {
        let entry = diag_entry(&["ivf/truncated-frame-payload"]);
        assert!(check_entry(&entry, &ids(&["ivf/truncated-frame-payload"])).is_ok());
        assert!(check_entry(&entry, &ids(&[])).is_err());
        assert!(
            check_entry(
                &entry,
                &ids(&["ivf/truncated-frame-payload", "bitstream/parse-error"])
            )
            .is_err()
        );
    }

    #[test]
    fn empty_diagnostics_set_is_rejected() {
        assert!(check_entry(&diag_entry(&[]), &ids(&[])).is_err());
    }

    #[test]
    fn manifest_parses_both_expect_arms() -> Result<()> {
        let text = r#"
[[vector]]
path = "vectors/valid/a.ivf"
description = "clean one"
expect = "clean"

[[vector]]
path = "vectors/invalid/b.ivf"
description = "negative one"
expect = { diagnostics = ["ivf/truncated-frame-payload"] }
"#;
        let manifest: Manifest = toml::from_str(text)?;
        assert_eq!(manifest.vector.len(), 2);
        assert!(matches!(
            manifest.vector[0].expect,
            Expect::Clean(CleanTag::Clean)
        ));
        match &manifest.vector[1].expect {
            Expect::Diagnostics { diagnostics } => {
                assert_eq!(diagnostics, &["ivf/truncated-frame-payload".to_owned()]);
            }
            Expect::Clean(_) => bail!("second entry should be a diagnostics arm"),
        }
        Ok(())
    }

    /// The bootstrap negative produces exactly `ivf/truncated-frame-payload`,
    /// so the committed manifest's diagnostics arm matches.
    #[test]
    fn committed_negative_id_matches_manifest() {
        let entry = diag_entry(&["ivf/truncated-frame-payload"]);
        assert!(check_entry(&entry, &ids(&["ivf/truncated-frame-payload"])).is_ok());
    }
}
