// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The decode instruction-count regression gate (`cargo xtask perf`).
//!
//! Decodes a curated subset of committed conformance vectors under DynamoRIO
//! and compares the executed-instruction count of each against a checked-in
//! baseline (`tests/perf/baseline.toml`). DynamoRIO decodes every instruction
//! the release build emits (AVX2 included), and the count is a stable proxy
//! for decode CPU work, unlike wall-clock time. The counting client
//! (`xtask/perf-client`, built with CMake against the release) counts only the
//! `splot` binary, because glibc picks its string functions per CPU, and adds
//! each block's count with one atomic 64-bit add, so threads cannot lose
//! updates. The decode discards its output, because the frame hash dispatches
//! on CPU features. Idle worker spins only add to the count, so each fixture
//! keeps the smallest count of [`RUNS`] runs.
//!
//! This is a heavy gate and is **not** part of `cargo xtask ci`; it runs as its
//! own CI job and on demand. `--bless` re-runs every fixture and rewrites the
//! baseline (the only write it performs). When `DYNAMORIO_HOME` does not name a
//! DynamoRIO release it skips with a hint rather than failing, mirroring the
//! run-if-present style of the fuzz/coverage tooling, so a fresh checkout can
//! still run the rest of `xtask` — CI installs a pinned release and therefore
//! enforces.
//!
//! `xtask` depends on no `splot-*` crate, so it shells out to the built `splot`
//! binary's `decode` command, exactly as `xtask conformance` shells out to
//! `validate`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// Baseline file, relative to the workspace root.
const BASELINE_REL: &str = "tests/perf/baseline.toml";
/// Comment header prepended on `--bless` (TOML serialization drops comments).
const BASELINE_HEADER: &str = "\
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#
# Decode instruction-count baselines (DynamoRIO inscount) for the
# `cargo xtask perf` regression gate. Each fixture is decoded single-threaded
# and keeps the smallest count of three runs. Counts are only comparable within
# one binary/toolchain: a compiler or RUSTFLAGS bump shifts every count
# wholesale and requires a re-bless (`cargo xtask perf --bless`), not a
# regression. The counts cover only the `splot` binary (the atomic counter in
# xtask/perf-client) and a decode with null output, so the runner's CPU does
# not change them. The
# `perf` CI job and its DynamoRIO release are the enforced check; after a
# toolchain or DynamoRIO bump, re-bless on-runner.
# Generated file: edit fixtures here, but let `--bless` set the numbers.
";
/// Fail the gate when a fixture's count rises more than this percentage above
/// its baseline. This budget only absorbs incidental codegen drift within one
/// pinned toolchain; a real regression moves far past it.
const TOLERANCE_PCT: f64 = 0.5;
/// Runs per fixture; the smallest count is kept.
const RUNS: usize = 3;

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
/// recorded instruction-count baseline.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FixtureRow {
    path: String,
    targets: String,
    ir_stable: u64,
}

/// The DynamoRIO release that `DYNAMORIO_HOME` names and its `drrun`, when
/// both exist.
fn dynamorio() -> Option<(PathBuf, PathBuf)> {
    let home = PathBuf::from(std::env::var_os("DYNAMORIO_HOME")?);
    let drrun = home.join("bin64").join("drrun");
    (drrun.is_file() && home.join("cmake").is_dir()).then_some((home, drrun))
}

/// Builds the counting client in `xtask/perf-client` against the DynamoRIO
/// release at `home` and returns the path of its shared library.
fn build_client(root: &Path, home: &Path) -> Result<PathBuf> {
    let build = root.join("target").join("perf-client");
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(root.join("xtask").join("perf-client"))
        .arg("-B")
        .arg(&build)
        .arg(format!("-DDynamoRIO_DIR={}", home.join("cmake").display()))
        .arg("-DCMAKE_BUILD_TYPE=Release");
    let mut compile = Command::new("cmake");
    compile.arg("--build").arg(&build);
    for command in [&mut configure, &mut compile] {
        let output = command
            .output()
            .context("failed to run cmake for the perf client")?;
        if !output.status.success() {
            bail!(
                "building the perf client failed ({}):
{}{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(build.join("libsplotinscount.so"))
}

/// Runs the perf gate. With `bless`, rewrites the baseline from the measured
/// numbers instead of comparing against it.
///
/// # Errors
/// Returns an error if the baseline is unreadable, a fixture is missing, a
/// decode under DynamoRIO fails, or (without `bless`) any fixture regresses
/// beyond [`TOLERANCE_PCT`].
pub(crate) fn run_perf(root: &Path, bless: bool) -> Result<()> {
    let baseline_path = root.join(BASELINE_REL);
    let mut baseline: Baseline = crate::util::load_toml(&baseline_path)?;
    if baseline.fixture.is_empty() {
        bail!("perf baseline {BASELINE_REL} lists no fixtures; the gate would cover nothing");
    }
    if !bless && let Some(row) = baseline.fixture.iter().find(|row| row.ir_stable == 0) {
        bail!(
            "perf baseline {BASELINE_REL}: fixture {} has ir_stable = 0 (unrecorded); run `cargo xtask perf --bless` to record it",
            row.path
        );
    }

    let Some((home, drrun)) = dynamorio() else {
        if bless {
            bail!(
                "perf --bless requires DynamoRIO; set DYNAMORIO_HOME to a release to record the baseline"
            );
        }
        eprintln!(
            "perf: skipped (DYNAMORIO_HOME does not name a DynamoRIO release; set it to run the decode instruction gate)"
        );
        return Ok(());
    };

    let splot = crate::conformance::build_splot_binary(root, true)?;
    let client = build_client(root, &home)?;
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
        let measured = measure_instructions(&drrun, &client, &splot, &fixture)?;
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

/// Decodes one fixture under the counting client [`RUNS`] times and returns
/// the smallest executed-instruction count.
fn measure_instructions(drrun: &Path, client: &Path, splot: &Path, fixture: &Path) -> Result<u64> {
    let mut best = u64::MAX;
    for _ in 0..RUNS {
        let output = Command::new(drrun)
            .arg("-c")
            .arg(client)
            .arg("--")
            .arg(splot)
            .args([
                "decode",
                "--quiet",
                "--threads",
                "1",
                "--frame-delay",
                "1",
                "--output-format",
                "null",
            ])
            .arg(fixture)
            .output()
            .context("failed to spawn drrun")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let lines: Vec<&str> = stderr.lines().collect();
            let tail = lines[lines.len().saturating_sub(40)..].join("\n");
            bail!(
                "DynamoRIO decode failed for {} ({}):\n{tail}",
                fixture.display(),
                output.status
            );
        }
        let count = parse_inscount(&String::from_utf8_lossy(&output.stdout))
            .with_context(|| format!("inscount output for {}", fixture.display()))?;
        best = best.min(count);
    }
    Ok(best)
}

/// Extracts the count from the counting client's
/// `Instrumentation results: N instructions executed` line.
fn parse_inscount(text: &str) -> Result<u64> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("Instrumentation results:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|token| token.parse::<u64>().ok())
                .ok_or_else(|| anyhow!("could not parse inscount line: {line}"));
        }
    }
    bail!("no `Instrumentation results:` line in inscount output")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::parse_inscount;

    #[test]
    fn parses_the_results_line_among_decode_output() {
        let text = "frame.hash: abc\nInstrumentation results: 21219322 instructions executed\n";
        assert_eq!(parse_inscount(text).unwrap(), 21_219_322);
    }

    #[test]
    fn missing_results_line_is_an_error() {
        assert!(parse_inscount("frame.hash: abc\n").is_err());
    }

    #[test]
    fn non_numeric_count_is_an_error() {
        assert!(parse_inscount("Instrumentation results: many instructions executed\n").is_err());
    }
}
