// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The decode instruction-count regression gate (`cargo xtask perf`).
//!
//! Decodes a curated subset of committed conformance vectors under
//! `valgrind --tool=callgrind` and compares the total retired-instruction count
//! (Ir) of each against a checked-in baseline (`tests/perf/baseline.toml`).
//! Callgrind Ir with cache/branch simulation disabled is deterministic to the
//! instruction for a fixed binary, so — run single-threaded — it is a stable,
//! runner-independent proxy for decode CPU work, unlike wall-clock time.
//!
//! This is a heavy gate (valgrind is ~20-30x slowdown) and is **not** part of
//! `cargo xtask ci`; it runs as its own CI job and on demand. `--bless` re-runs
//! every fixture and rewrites the baseline (the only write it performs). When
//! `valgrind` is absent it skips with a hint rather than failing, mirroring the
//! run-if-present style of the fuzz/coverage tooling, so a fresh checkout can
//! still run the rest of `xtask` — CI installs valgrind and therefore enforces.
//!
//! `xtask` depends on no `splot-*` crate, so it shells out to the built `splot`
//! binary's `decode` command, exactly as `xtask conformance` shells out to
//! `validate`.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// Baseline file, relative to the workspace root.
const BASELINE_REL: &str = "tests/perf/baseline.toml";
/// Comment header prepended on `--bless` (TOML serialization drops comments).
const BASELINE_HEADER: &str = "\
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#
# Deterministic decode instruction-count (callgrind Ir) baselines for the
# `cargo xtask perf` regression gate. Each fixture is decoded single-threaded so
# the Ir count is reproducible to the instruction on a fixed binary. Ir is only
# comparable within one binary/toolchain: a compiler or RUSTFLAGS bump shifts
# every count wholesale and requires a re-bless (`cargo xtask perf --bless`), not
# a regression. Under callgrind the environment also spans glibc and the valgrind
# build (which redirects malloc/memcpy/...), so these numbers are pinned to the
# `perf` CI job's ubuntu-latest image; that job is the enforced check. If a
# valgrind/glibc bump skews a fixture past the tolerance, re-bless on-runner.
# Generated file: edit fixtures here, but let `--bless` set the numbers.
";
/// Fail the gate when a fixture's Ir rises more than this percentage above its
/// baseline. Callgrind Ir is deterministic for a fixed binary (sub-0.1%
/// run-to-run), so this budget only absorbs incidental codegen drift within one
/// pinned toolchain; a real regression moves far past it.
const TOLERANCE_PCT: f64 = 0.5;

/// The whole `tests/perf/baseline.toml` document. `deny_unknown_fields` turns a
/// mistyped table (e.g. `[[fixtures]]`) into a load error instead of silently
/// dropping every fixture and greening the gate over zero coverage.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    meta: Meta,
    fixture: Vec<FixtureRow>,
}

/// Free-form provenance for the recorded numbers.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    note: String,
}

/// One measured fixture: its path, the hot kernel(s) it exercises, and the
/// recorded stable-toolchain Ir baseline.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FixtureRow {
    path: String,
    targets: String,
    ir_stable: u64,
}

/// Removes a scratch directory when dropped, so a mid-loop `?` cannot leak it.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs the perf gate. With `bless`, rewrites the baseline from the measured
/// numbers instead of comparing against it.
///
/// # Errors
/// Returns an error if the baseline is unreadable, a fixture is missing,
/// valgrind fails, or (without `bless`) any fixture regresses beyond
/// [`TOLERANCE_PCT`].
pub(crate) fn run_perf(root: &Path, bless: bool) -> Result<()> {
    let baseline_path = root.join(BASELINE_REL);
    let mut baseline: Baseline = crate::util::load_toml(&baseline_path)?;
    if baseline.fixture.is_empty() {
        bail!("perf baseline {BASELINE_REL} lists no fixtures; the gate would cover nothing");
    }

    if !crate::tool_available("valgrind") {
        if bless {
            bail!("perf --bless requires valgrind; install it to record the baseline");
        }
        eprintln!("perf: skipped (valgrind not found; install valgrind to run the decode Ir gate)");
        return Ok(());
    }

    let splot = crate::conformance::build_splot_binary(root, true)?;
    let scratch =
        ScratchDir(std::env::temp_dir().join(format!("splot-perf-{}", std::process::id())));
    std::fs::create_dir_all(&scratch.0)
        .with_context(|| format!("failed to create scratch dir {}", scratch.0.display()))?;
    let out_file = scratch.0.join("callgrind.out");

    let mut regressions = Vec::new();
    eprintln!(
        "perf: {:>14}  {:>14}  {:>8}  target / fixture",
        "baseline", "measured", "delta%"
    );
    for row in &mut baseline.fixture {
        let fixture = root.join(&row.path);
        if !fixture.exists() {
            bail!("perf fixture missing: {}", row.path);
        }
        let measured = measure_ir(&splot, &fixture, &out_file)?;
        let base = row.ir_stable;
        let delta = if base == 0 {
            0.0
        } else {
            (measured as f64 - base as f64) / base as f64 * 100.0
        };
        eprintln!(
            "perf: {base:>14}  {measured:>14}  {delta:>+8.3}  {} / {}",
            row.targets, row.path
        );
        if bless {
            row.ir_stable = measured;
        } else if base != 0 && delta > TOLERANCE_PCT {
            regressions.push((row.path.clone(), base, measured, delta));
        }
    }

    if bless {
        let body = toml::to_string(&baseline).context("failed to serialize perf baseline")?;
        let rendered = format!("{BASELINE_HEADER}\n{body}");
        std::fs::write(&baseline_path, rendered)
            .with_context(|| format!("failed to write {}", baseline_path.display()))?;
        eprintln!(
            "perf: blessed {} fixture(s) into {BASELINE_REL}",
            baseline.fixture.len()
        );
        return Ok(());
    }

    if regressions.is_empty() {
        eprintln!(
            "perf: ok ({} fixture(s) within {TOLERANCE_PCT}% of baseline)",
            baseline.fixture.len()
        );
        return Ok(());
    }
    for (path, base, measured, delta) in &regressions {
        eprintln!("perf: REGRESSION {path}: {base} -> {measured} ({delta:+.3}%)");
    }
    bail!(
        "{} fixture(s) regressed beyond {TOLERANCE_PCT}% (re-bless with `cargo xtask perf --bless` if intentional)",
        regressions.len()
    )
}

/// Decodes one fixture under callgrind and returns the total Ir from the
/// output's `summary:` line.
fn measure_ir(splot: &Path, fixture: &Path, out_file: &Path) -> Result<u64> {
    let _ = std::fs::remove_file(out_file);
    let status = Command::new("valgrind")
        .args(["--tool=callgrind", "--cache-sim=no", "--branch-sim=no"])
        .arg(format!("--callgrind-out-file={}", out_file.display()))
        .arg(splot)
        .args([
            "decode",
            "--quiet",
            "--threads",
            "1",
            "--output-format",
            "hash",
        ])
        .arg(fixture)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to spawn valgrind")?;
    if !status.success() {
        bail!("valgrind decode failed for {}", fixture.display());
    }
    let text = std::fs::read_to_string(out_file)
        .with_context(|| format!("failed to read {}", out_file.display()))?;
    parse_summary_ir(&text).with_context(|| format!("callgrind output for {}", fixture.display()))
}

/// Extracts the total Ir from a callgrind output body's `summary:` line (the
/// first whitespace-separated integer after the `summary:` prefix).
fn parse_summary_ir(text: &str) -> Result<u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("summary:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|token| token.parse::<u64>().ok())
                .ok_or_else(|| anyhow!("could not parse callgrind summary line: {line}"));
        }
    }
    bail!("no `summary:` line in callgrind output")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::parse_summary_ir;

    #[test]
    fn parses_single_token_summary() {
        assert_eq!(
            parse_summary_ir("events: Ir\nsummary: 21219322\n").unwrap(),
            21_219_322
        );
    }

    #[test]
    fn parses_first_token_of_multi_column_summary() {
        assert_eq!(parse_summary_ir("summary: 42 7 3\n").unwrap(), 42);
    }

    #[test]
    fn missing_summary_line_is_an_error() {
        assert!(parse_summary_ir("events: Ir\ntotals: 5\n").is_err());
    }

    #[test]
    fn non_numeric_summary_is_an_error() {
        assert!(parse_summary_ir("summary: not-a-number\n").is_err());
    }
}
