// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The decode instruction-count gate (`cargo xtask perf --base <rev>`).
//!
//! Builds the `splot` binary of the working tree (head) and of the base
//! revision (in a git worktree), decodes the pinned EwoutH sample [`STREAMS`]
//! with both under DynamoRIO, and compares the executed-instruction counts.
//! Both counts come from the same job, tool and CPU, so no baseline is
//! checked in. DynamoRIO decodes every instruction the release build emits
//! (AVX2 included), and the count is a stable proxy for decode CPU work,
//! unlike wall-clock time. The counting client (`xtask/perf-client`, built
//! with CMake against the release) counts only the `splot` binary, because
//! glibc picks its string functions per CPU, and adds each block's count with
//! one atomic 64-bit add, so threads cannot lose updates. The decode discards
//! its output, because the frame hash dispatches on CPU features. It runs
//! single-threaded with one frame in flight, pinned to one CPU (`taskset`), so
//! the idle pool worker does not spin on a second CPU while the main thread
//! works; spins only add to the count, and each binary keeps the smallest of
//! [`RUNS`]. The spread of the base runs is the A/A noise of the job.
//!
//! A commit trailer `Perf-Accept: <stream|all> +X% <reason>` in `<base>..HEAD`
//! raises the limit of that stream to X% (see [`parse_accept`]).
//!
//! The command needs Linux with DynamoRIO (`DYNAMORIO_HOME`) and CMake, and
//! fails without them. It is not part of `cargo xtask ci`; the `perf` CI job
//! installs a pinned DynamoRIO release.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::git_util::run_git;

/// Release streams of `tests/conformance/ewouth-av2.toml`: name, file, frames.
const STREAMS: [(&str, &str, u32); 5] = [
    ("DOTA2", "DOTA2_1920x1080_60_8bit_420_qp135_130f.ivf", 20),
    (
        "RitualDance",
        "RitualDance_1920x1080_60fps_10bit_420_qp135_600f.ivf",
        16,
    ),
    (
        "Slides1",
        "Slides1_1920x1080_30fps_8bit_420_screen_qp135_130f.ivf",
        16,
    ),
    (
        "Noise_Ocean",
        "Noise_Ocean_1920x1080_60fps_qp135_130f.ivf",
        16,
    ),
    (
        "old_town_cross",
        "old_town_cross_1080p50_tiles4x2_qp135_500f.ivf",
        10,
    ),
];
/// Runs per binary and stream; the smallest count is kept.
const RUNS: usize = 3;
/// A stream fails above max([`STREAM_FAIL_PCT`], [`NOISE_FACTOR`] x A/A spread).
const STREAM_FAIL_PCT: f64 = 0.10;
const NOISE_FACTOR: f64 = 5.0;
/// The total of all streams fails above this rise.
const TOTAL_FAIL_PCT: f64 = 0.05;
/// A stream or the total warns above this rise.
const WARN_PCT: f64 = 0.03;

/// Options of `cargo xtask perf`.
pub(crate) struct PerfOptions {
    /// Base revision.
    pub(crate) base: String,
    /// A `splot` binary of the base revision; used when the file exists.
    pub(crate) base_bin: Option<PathBuf>,
    /// Directory of the sample streams; missing streams are downloaded.
    pub(crate) streams: PathBuf,
    /// Print the table and never fail.
    pub(crate) report_only: bool,
}

/// The DynamoRIO release that `DYNAMORIO_HOME` names and its `drrun`, when
/// both exist.
pub(crate) fn dynamorio() -> Option<(PathBuf, PathBuf)> {
    let home = PathBuf::from(std::env::var_os("DYNAMORIO_HOME")?);
    let drrun = home.join("bin64").join("drrun");
    (drrun.is_file() && home.join("cmake").is_dir()).then_some((home, drrun))
}

/// Builds the counting client in `xtask/perf-client` against the DynamoRIO
/// release at `home` and returns the path of its shared library.
pub(crate) fn build_client(root: &Path, home: &Path) -> Result<PathBuf> {
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

/// Runs the gate: head against `options.base`.
///
/// # Errors
/// Returns an error if DynamoRIO is missing, a trailer is malformed, a stream
/// cannot be fetched or verified, a build or decode fails, or (without
/// `report_only`) a stream or the total rises above its limit.
pub(crate) fn run_perf(root: &Path, options: &PerfOptions) -> Result<()> {
    let Some((home, drrun)) = dynamorio() else {
        bail!(
            "perf needs Linux with DynamoRIO: set DYNAMORIO_HOME to a DynamoRIO release (the `perf` CI job installs a pinned one)"
        );
    };
    let base_sha = run_git(
        root,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", options.base),
        ],
    )?
    .trim()
    .to_owned();
    let log = run_git(
        root,
        &[
            "log",
            "--format=%(trailers:key=Perf-Accept,valueonly,unfold)",
            &format!("{base_sha}..HEAD"),
        ],
    )?;
    let accepts = log
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_accept)
        .collect::<Result<Vec<_>>>()?;

    let streams = root.join(&options.streams);
    fetch_streams(root, &streams)?;
    let head = crate::conformance::build_splot_binary(root, true)?;
    let base = match &options.base_bin {
        Some(path) if path.is_file() => path.clone(),
        _ => build_base(root, &base_sha)?,
    };
    if base == head {
        bail!(
            "head and base builds wrote the same binary {}; unset CARGO_TARGET_DIR",
            head.display()
        );
    }
    let client = build_client(root, &home)?;

    let mut rows = Vec::new();
    for (name, file, frames) in STREAMS {
        let mut args: Vec<OsString> = [
            "decode",
            "--quiet",
            "--threads",
            "1",
            "--frame-delay",
            "1",
            "--output-format",
            "null",
            "--limit",
        ]
        .map(Into::into)
        .into();
        args.push(frames.to_string().into());
        args.push(streams.join(file).into());
        let base_runs = measure_runs(&drrun, &client, &base, &args)?;
        let head_runs = measure_runs(&drrun, &client, &head, &args)?;
        rows.push(Row {
            name,
            frames,
            base: min(&base_runs)?,
            head: min(&head_runs)?,
            aa_pct: spread_pct(&base_runs)?,
        });
        eprintln!("perf: {name}: base {base_runs:?}, head {head_runs:?}");
    }

    let lines = judge(&rows, &accepts);
    let mut summary = format!(
        "### Decode instructions: head vs base `{}`\n\n| Stream | Frames | Base | Head | Change | A/A spread | Limit | Verdict |\n|---|---:|---:|---:|---:|---:|---:|---|\n",
        &base_sha[..base_sha.len().min(12)]
    );
    for line in &lines {
        let _ = writeln!(
            summary,
            "| {} | {} | {} | {} | {:+.3}% | {} | {:.2}% | {:?} |",
            line.name,
            line.frames.map_or(String::new(), |f| f.to_string()),
            line.base,
            line.head,
            delta_pct(line.base, line.head),
            line.aa_pct.map_or(String::new(), |a| format!("{a:.4}%")),
            line.limit,
            line.verdict
        );
        let annotation = match line.verdict {
            Verdict::Fail => "error",
            Verdict::Warn | Verdict::Accepted => "warning",
            Verdict::Ok => continue,
        };
        println!(
            "::{annotation}::perf {}: {:+.3}% instructions ({:?})",
            line.name,
            delta_pct(line.base, line.head),
            line.verdict
        );
    }
    for accept in &accepts {
        let _ = writeln!(
            summary,
            "\nPerf-Accept: {} +{}% {}",
            accept.stream, accept.pct, accept.reason
        );
    }
    println!("{summary}");
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        std::fs::write(&path, &summary).context("failed to write GITHUB_STEP_SUMMARY")?;
    }
    if !options.report_only && lines.iter().any(|line| line.verdict == Verdict::Fail) {
        bail!(
            "decode instruction count rose above the limit; see the table, or add a `Perf-Accept:` trailer"
        );
    }
    Ok(())
}

/// Downloads missing [`STREAMS`] and verifies every input SHA-256 with
/// `scripts/check-external-vectors.py`.
fn fetch_streams(root: &Path, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let mut command = Command::new("python3");
    command
        .arg(root.join("scripts").join("check-external-vectors.py"))
        .arg("ewouth-av2")
        .arg(dir);
    for (_, file, _) in STREAMS {
        command.arg("--fetch").arg(file);
    }
    let status = command
        .status()
        .context("failed to run python3 scripts/check-external-vectors.py")?;
    if !status.success() {
        bail!("fetching or verifying the perf streams failed ({status})");
    }
    Ok(())
}

/// Builds `splot` at `sha` in the git worktree `target/perf-base`.
///
/// shortcut: the nested cargo keeps the head toolchain, so a toolchain bump
/// shows only when the base binary comes from the cache; build with the base
/// `rust-toolchain.toml` if such a PR must be measured on a cache miss.
fn build_base(root: &Path, sha: &str) -> Result<PathBuf> {
    let tree = root.join("target").join("perf-base");
    if tree.exists() {
        run_git(&tree, &["checkout", "--quiet", "--force", "--detach", sha])?;
    } else {
        run_git(root, &["worktree", "prune"])?;
        let tree_arg = tree.to_string_lossy();
        run_git(
            root,
            &["worktree", "add", "--quiet", "--detach", &tree_arg, sha],
        )?;
    }
    crate::conformance::build_splot_binary(&tree, true)
}

/// Runs `splot` with `args` under the counting client [`RUNS`] times and
/// returns the smallest executed-instruction count.
pub(crate) fn measure_instructions(
    drrun: &Path,
    client: &Path,
    splot: &Path,
    args: &[OsString],
) -> Result<u64> {
    min(&measure_runs(drrun, client, splot, args)?)
}

/// Runs `splot` with `args` under the counting client [`RUNS`] times and
/// returns every executed-instruction count.
fn measure_runs(drrun: &Path, client: &Path, splot: &Path, args: &[OsString]) -> Result<Vec<u64>> {
    let mut counts = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let output = Command::new("taskset")
            .args(["--cpu-list", "0"])
            .arg(drrun)
            .arg("-c")
            .arg(client)
            .arg("--")
            .arg(splot)
            .args(args)
            .output()
            .context("failed to spawn taskset with drrun")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let lines: Vec<&str> = stderr.lines().collect();
            let tail = lines[lines.len().saturating_sub(40)..].join("\n");
            bail!(
                "DynamoRIO run failed for {args:?} ({}):\n{tail}",
                output.status
            );
        }
        let count = parse_inscount(&String::from_utf8_lossy(&output.stdout))
            .with_context(|| format!("inscount output for {args:?}"))?;
        if count == 0 {
            bail!("DynamoRIO counted 0 instructions for {args:?}");
        }
        counts.push(count);
    }
    Ok(counts)
}

fn min(counts: &[u64]) -> Result<u64> {
    counts
        .iter()
        .copied()
        .min()
        .ok_or_else(|| anyhow!("no DynamoRIO runs"))
}

/// The spread of `counts` in percent of their minimum.
fn spread_pct(counts: &[u64]) -> Result<f64> {
    let low = min(counts)?;
    let high = counts.iter().copied().max().unwrap_or(low);
    Ok(delta_pct(low, high))
}

fn delta_pct(base: u64, head: u64) -> f64 {
    (head as f64 - base as f64) / base as f64 * 100.0
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

/// One `Perf-Accept: <stream|all> +X% <reason>` trailer value.
#[derive(Debug, PartialEq)]
struct Accept {
    stream: String,
    pct: f64,
    reason: String,
}

/// Parses the value of a `Perf-Accept` trailer. The stream is a name of
/// [`STREAMS`] or `all`; X is a positive percent; the reason is not empty.
fn parse_accept(value: &str) -> Result<Accept> {
    let malformed = || {
        anyhow!(
            "malformed `Perf-Accept: {value}`; expected `Perf-Accept: <stream|all> +X% <reason>`"
        )
    };
    let mut words = value.split_whitespace();
    let stream = words.next().ok_or_else(malformed)?;
    let pct = words
        .next()
        .and_then(|word| {
            word.strip_prefix('+')?
                .strip_suffix('%')?
                .parse::<f64>()
                .ok()
        })
        .filter(|pct| pct.is_finite() && *pct > 0.0)
        .ok_or_else(malformed)?;
    let reason = words.collect::<Vec<_>>().join(" ");
    if reason.is_empty() {
        return Err(malformed());
    }
    if stream != "all" && !STREAMS.iter().any(|(name, ..)| *name == stream) {
        bail!("`Perf-Accept` names the unknown stream {stream}; use `all` or one of {STREAMS:?}");
    }
    Ok(Accept {
        stream: stream.to_owned(),
        pct,
        reason,
    })
}

/// The measured counts of one stream.
struct Row {
    name: &'static str,
    frames: u32,
    base: u64,
    head: u64,
    aa_pct: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Verdict {
    Ok,
    Warn,
    Accepted,
    Fail,
}

/// One judged table line: a stream, or the total (no frames, no A/A spread).
struct Line {
    name: &'static str,
    frames: Option<u32>,
    base: u64,
    head: u64,
    aa_pct: Option<f64>,
    limit: f64,
    verdict: Verdict,
}

/// Fails above `effective`; a rise above `limit` that `effective` allows is
/// accepted; a rise above [`WARN_PCT`] warns.
fn verdict(delta: f64, limit: f64, effective: f64) -> Verdict {
    if delta > effective {
        Verdict::Fail
    } else if delta > limit {
        Verdict::Accepted
    } else if delta > WARN_PCT {
        Verdict::Warn
    } else {
        Verdict::Ok
    }
}

/// Judges each stream and the total. An accepted stream may rise by its X%;
/// the total limit grows by the accepted share of the total base count.
fn judge(rows: &[Row], accepts: &[Accept]) -> Vec<Line> {
    let accepted = |name: &str| {
        accepts
            .iter()
            .filter(|accept| accept.stream == name || accept.stream == "all")
            .map(|accept| accept.pct)
            .reduce(f64::max)
    };
    let base_total: u64 = rows.iter().map(|row| row.base).sum();
    let head_total: u64 = rows.iter().map(|row| row.head).sum();
    let mut allowance = 0.0;
    let mut lines: Vec<Line> = rows
        .iter()
        .map(|row| {
            let limit = STREAM_FAIL_PCT.max(NOISE_FACTOR * row.aa_pct);
            let accept = accepted(row.name);
            allowance += accept.unwrap_or(0.0) * row.base as f64 / base_total as f64;
            let effective = accept.map_or(limit, |pct| limit.max(pct));
            Line {
                name: row.name,
                frames: Some(row.frames),
                base: row.base,
                head: row.head,
                aa_pct: Some(row.aa_pct),
                limit: effective,
                verdict: verdict(delta_pct(row.base, row.head), limit, effective),
            }
        })
        .collect();
    let effective = TOTAL_FAIL_PCT + allowance;
    lines.push(Line {
        name: "Total",
        frames: None,
        base: base_total,
        head: head_total,
        aa_pct: None,
        limit: effective,
        verdict: verdict(delta_pct(base_total, head_total), TOTAL_FAIL_PCT, effective),
    });
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{Accept, Row, Verdict, judge, parse_accept, parse_inscount, spread_pct};

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

    fn accept(stream: &str, pct: f64) -> Accept {
        Accept {
            stream: stream.to_owned(),
            pct,
            reason: "r".to_owned(),
        }
    }

    #[test]
    fn parses_stream_and_all_trailers() {
        assert_eq!(
            parse_accept("RitualDance +0.6% exact CDEF  rounding").unwrap(),
            Accept {
                stream: "RitualDance".to_owned(),
                pct: 0.6,
                reason: "exact CDEF rounding".to_owned()
            }
        );
        assert_eq!(parse_accept(" all +2% toolchain bump").unwrap().pct, 2.0);
    }

    #[test]
    fn rejects_malformed_trailers() {
        for value in [
            "",
            "all",
            "all +0.5%",
            "all 0.5% no plus",
            "all +0.5 no percent",
            "all +0% zero",
            "all +-1% negative",
            "all +nan% not a number",
            "all +inf% infinite",
            "Unknown +0.5% not a stream",
            "ritualdance +0.5% wrong case",
        ] {
            assert!(parse_accept(value).is_err(), "{value:?}");
        }
    }

    fn row(name: &'static str, base: u64, head: u64, aa_pct: f64) -> Row {
        Row {
            name,
            frames: 1,
            base,
            head,
            aa_pct,
        }
    }

    fn verdicts(rows: &[Row], accepts: &[Accept]) -> Vec<Verdict> {
        judge(rows, accepts)
            .iter()
            .map(|line| line.verdict)
            .collect()
    }

    #[test]
    fn stream_limit_is_the_larger_of_floor_and_noise() {
        use Verdict::{Fail, Ok, Warn};
        let rows = [
            row("DOTA2", 100_000, 100_005, 0.0),
            row("Slides1", 1_000_000, 1_000_000, 0.0),
        ];
        assert_eq!(verdicts(&rows, &[]), [Ok, Ok, Ok]);
        let rows = [
            row("DOTA2", 100_000, 100_050, 0.0),
            row("Slides1", 10_000_000, 10_000_000, 0.0),
        ];
        assert_eq!(verdicts(&rows, &[]), [Warn, Ok, Ok]);
        let rows = [
            row("DOTA2", 100_000, 100_150, 0.0),
            row("Slides1", 10_000_000, 10_000_000, 0.0),
        ];
        assert_eq!(verdicts(&rows, &[]), [Fail, Ok, Ok]);
        let rows = [
            row("DOTA2", 100_000, 100_150, 0.1),
            row("Slides1", 10_000_000, 10_000_000, 0.0),
        ];
        assert_eq!(verdicts(&rows, &[]), [Warn, Ok, Ok]);
    }

    #[test]
    fn total_fails_when_streams_rise_together() {
        let rows = [
            row("DOTA2", 100_000, 100_060, 0.0),
            row("Slides1", 100_000, 100_060, 0.0),
        ];
        assert_eq!(
            verdicts(&rows, &[]),
            [Verdict::Warn, Verdict::Warn, Verdict::Fail]
        );
    }

    #[test]
    fn accept_raises_only_its_stream_and_its_share_of_the_total() {
        use Verdict::{Accepted, Fail, Ok};
        let rows = [
            row("DOTA2", 100_000, 100_500, 0.0),
            row("Slides1", 100_000, 100_000, 0.0),
        ];
        assert_eq!(
            verdicts(&rows, &[accept("DOTA2", 0.6)]),
            [Accepted, Ok, Accepted]
        );
        assert_eq!(
            verdicts(&rows, &[accept("DOTA2", 0.4)]),
            [Fail, Ok, Accepted]
        );
        assert_eq!(verdicts(&rows, &[accept("Slides1", 0.6)])[0], Fail);
        assert_eq!(
            verdicts(&rows, &[accept("all", 0.6)]),
            [Accepted, Ok, Accepted]
        );
    }

    #[test]
    fn spread_is_relative_to_the_minimum() {
        assert!((spread_pct(&[1000, 1001, 1000]).unwrap() - 0.1).abs() < 1e-9);
        assert!(spread_pct(&[]).is_err());
    }
}
