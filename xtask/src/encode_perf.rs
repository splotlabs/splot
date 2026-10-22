// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The encoder speed harness (`cargo xtask encode-perf`).
//!
//! Generates fixed synthetic Y4M clips (moving gradients plus a hashed
//! texture; no third-party content), encodes each at its presets (0, 6, 10)
//! with `--threads 1`, and prints the wall frames per second, the IVF sha256 and an
//! instruction count. With `DYNAMORIO_HOME` set (Linux CI), the count comes
//! from the `xtask perf` DynamoRIO client and is compared against
//! `tests/perf/encode-baseline.toml` with the same tolerance as the decode
//! gate; unrecorded rows (`ir_stable = 0`) only print. On macOS the count is
//! the `/usr/bin/time -l` "instructions retired" figure, which is
//! informational and never gates.
//!
//! `--coverage` samples a longer release encode (`sample` on macOS, `perf` on
//! Linux) and prints the share of kernel samples that land in `std::simd`
//! kernels, over the allow-listed kernel symbols in [`KERNELS`].

use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::perf;

/// Baseline file, relative to the workspace root.
const BASELINE_REL: &str = "tests/perf/encode-baseline.toml";
/// Comment header prepended on `--bless`.
const BASELINE_HEADER: &str = "\
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
# SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#
# Encode instruction-count baselines (DynamoRIO inscount, smallest of three
# runs) for `cargo xtask encode-perf`. Rows with ir_stable = 0 are unrecorded
# and only print. Re-bless on a Linux runner with DynamoRIO after a toolchain
# bump or an intended encoder change: `cargo xtask encode-perf --bless`.
";
/// Fail when a recorded row rises more than this percentage.
const TOLERANCE_PCT: f64 = 0.5;
/// One synthetic clip and the presets it is encoded at.
struct Clip {
    name: &'static str,
    width: usize,
    height: usize,
    frames: usize,
    presets: &'static [u8],
}

/// The measured clips, sized so one DynamoRIO pass of all rows stays short:
/// preset 0 costs about 10M instructions per sample.
const CLIPS: [Clip; 3] = [
    Clip {
        name: "inter-128x96",
        width: 128,
        height: 96,
        frames: 4,
        presets: &[6, 10],
    },
    Clip {
        name: "intra-176x144",
        width: 176,
        height: 144,
        frames: 1,
        presets: &[6, 10],
    },
    Clip {
        name: "intra-64x64",
        width: 64,
        height: 64,
        frames: 1,
        presets: &[0],
    },
];

/// The longer clip `--coverage` samples, at preset 6.
const COVERAGE_CLIP: Clip = Clip {
    name: "coverage-352x288",
    width: 352,
    height: 288,
    frames: 8,
    presets: &[6],
};

/// Whether a kernel symbol runs in vector lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Simd,
    Scalar,
}

/// Allow-listed kernel symbols and whether they run in vector lanes
/// (`std::simd`, or an inner loop the compiler vectorizes, checked in the
/// disassembly), first match wins. A key is a `::` path suffix (a trailing `::` names a module); it
/// matches the demangled name or the length-prefixed identifiers that legacy
/// and v0 mangling both write (`17forward_transform12forward_simd`). The set
/// is the sample kernels a preset-6 encode spends time in: encoder distortion,
/// transform and quantizer kernels, and the `splot-recon` / prediction kernels
/// the encoder reconstructs with. RD logic (RDOQ walk, rate models) is not a
/// kernel.
const KERNELS: &[(&str, Kind)] = &[
    ("distortion::sad_simd", Kind::Simd),
    ("distortion::sad_lanes", Kind::Simd),
    ("distortion::sse_simd", Kind::Simd),
    ("distortion::sse_lanes", Kind::Simd),
    ("distortion::satd_simd", Kind::Simd),
    ("distortion::residual_simd", Kind::Simd),
    ("distortion::residual_lanes", Kind::Simd),
    ("distortion::", Kind::Scalar),
    ("forward_transform::forward_simd", Kind::Simd),
    ("forward_transform::forward_lanes", Kind::Simd),
    ("forward_transform::", Kind::Scalar),
    ("ForwardTransform2d::input", Kind::Scalar),
    ("quantize::quantize_simd", Kind::Simd),
    ("quantize::", Kind::Scalar),
    ("inverse_transform::kernel_row_quad", Kind::Simd),
    (
        "inverse_transform::inverse_transform_1d_row_quad",
        Kind::Simd,
    ),
    (
        "inverse_transform::inverse_transform_1d_columns",
        Kind::Simd,
    ),
    ("inverse_transform::inverse_transform_1d", Kind::Simd),
    ("inverse_transform::", Kind::Scalar),
    ("subpel_mc::", Kind::Simd),
    ("dequant_process::dequantize_block", Kind::Simd),
    ("cfl::fill_cfl_luma_ac_420_u16", Kind::Simd),
    ("sample_range::u16_samples_exceed", Kind::Simd),
    ("cfl::", Kind::Scalar),
    (
        "intra_directional_angle::predict_intra_directional_angle_rect_one_sided_idif_mrl_into",
        Kind::Simd,
    ),
    (
        "intra_directional_angle::predict_intra_directional_angle_rect_one_sided_idif_into",
        Kind::Simd,
    ),
    ("intra_directional_angle::", Kind::Scalar),
    ("intra_directional::", Kind::Scalar),
    ("intra_ibp_angular::", Kind::Scalar),
    ("intra_ibp_dc::", Kind::Scalar),
    (
        "smooth::predict_intra_smooth_over_available_edges_into",
        Kind::Scalar,
    ),
    ("reconstruct::add_residual_u16", Kind::Simd),
];

/// Whether `symbol` (demangled or mangled) names the allow-listed `key`.
fn kernel_matches(symbol: &str, key: &str) -> bool {
    let mut mangled = String::new();
    for segment in key.split("::").filter(|segment| !segment.is_empty()) {
        let _ = write!(mangled, "{}{segment}", segment.len());
    }
    symbol.contains(key) || symbol.contains(&mangled)
}

/// The `tests/perf/encode-baseline.toml` document.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    row: Vec<Row>,
}

/// One clip and preset with its recorded instruction count.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Row {
    clip: String,
    preset: u8,
    ir_stable: u64,
}

/// Writes a deterministic 8-bit 4:2:0 Y4M clip.
///
/// Luma is a triangle-wave gradient plus a 5-bit hashed texture that both move
/// by (2, 1) samples per frame; chroma is a slower gradient.
pub(crate) fn synthetic_y4m(width: usize, height: usize, frames: usize) -> Vec<u8> {
    let tri = |v: usize| {
        let v = v % 512;
        u8::try_from(if v < 256 { v } else { 511 - v }).unwrap_or(u8::MAX)
    };
    let texture = |x: usize, y: usize| {
        let h = ((x as u32).wrapping_mul(7919) ^ (y as u32).wrapping_mul(104_729))
            .wrapping_mul(2_654_435_761);
        (h >> 13) as usize & 31
    };
    let mut out = format!("YUV4MPEG2 W{width} H{height} F30:1 Ip A1:1 C420jpeg\n").into_bytes();
    for t in 0..frames {
        out.extend_from_slice(b"FRAME\n");
        for y in 0..height {
            for x in 0..width {
                let (mx, my) = (x + 2 * t, y + t);
                out.push(tri((3 * mx + 2 * my) / 2 + texture(mx, my)).clamp(16, 235));
            }
        }
        for plane in 0..2 {
            for y in 0..height / 2 {
                for x in 0..width / 2 {
                    out.push(tri(2 * x + y + 64 * plane + 3 * t) / 2 + 64);
                }
            }
        }
    }
    out
}

/// The `splot encode` arguments at `preset`.
fn encode_args(preset: u8, input: &Path, output: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "encode".into(),
        "--threads".into(),
        "1".into(),
        "--preset".into(),
        preset.to_string().into(),
    ];
    args.extend([input.into(), "-o".into(), output.into()]);
    args
}

/// Runs `splot args` once (under `/usr/bin/time -l` on macOS) and returns the
/// wall time and the macOS instructions-retired count.
fn timed_run(splot: &Path, args: &[OsString]) -> Result<(Duration, Option<u64>)> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/time");
        command.arg("-l").arg(splot);
        command
    } else {
        Command::new(splot)
    };
    command.args(args);
    let start = Instant::now();
    let output = command.output().context("failed to spawn splot encode")?;
    let wall = start.elapsed();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!("splot {args:?} failed ({}):\n{stderr}", output.status);
    }
    let retired = stderr.lines().find_map(|line| {
        let line = line.trim();
        line.strip_suffix("instructions retired")
            .and_then(|count| count.trim().parse().ok())
    });
    Ok((wall, retired))
}

/// Options of `cargo xtask encode-perf`.
pub(crate) struct EncodePerfOptions {
    /// Rewrite the baseline from DynamoRIO counts.
    pub(crate) bless: bool,
    /// Print the SIMD sample share instead of the speed table.
    pub(crate) coverage: bool,
    /// Wall-time runs per row; the fastest is kept.
    pub(crate) runs: usize,
}

/// Runs the encoder speed harness.
///
/// # Errors
/// Returns an error if the build, an encode, the baseline I/O, or (with
/// DynamoRIO) the regression check fails.
pub(crate) fn run_encode_perf(root: &Path, options: &EncodePerfOptions) -> Result<()> {
    let work = root.join("target").join("encode-perf");
    std::fs::create_dir_all(&work).with_context(|| format!("create {}", work.display()))?;
    let splot = crate::conformance::build_splot_binary(root, true)?;
    if options.coverage {
        return run_coverage(&splot, &work);
    }
    let baseline_path = root.join(BASELINE_REL);
    let mut baseline: Baseline = crate::util::load_toml(&baseline_path)?;
    let dynamorio = perf::dynamorio();
    if options.bless && dynamorio.is_none() {
        bail!("encode-perf --bless requires DynamoRIO; set DYNAMORIO_HOME to a release");
    }
    let client = match &dynamorio {
        Some((home, drrun)) => Some((drrun.clone(), perf::build_client(root, home)?)),
        None => None,
    };

    let mut regressions = Vec::new();
    println!(
        "{:<15} {:>6} {:>9} {:>15} {:>15} {:>8}  sha256",
        "clip", "preset", "fps", "instructions", "baseline", "delta%"
    );
    for clip in &CLIPS {
        let input = work.join(format!("{}.y4m", clip.name));
        std::fs::write(&input, synthetic_y4m(clip.width, clip.height, clip.frames))
            .with_context(|| format!("write {}", input.display()))?;
        for &preset in clip.presets {
            let output = work.join(format!("{}-p{preset}.ivf", clip.name));
            let args = encode_args(preset, &input, &output);
            let mut best = Duration::MAX;
            let mut retired = None;
            for _ in 0..options.runs.max(1) {
                let (wall, count) = timed_run(&splot, &args)?;
                best = best.min(wall);
                retired = match (retired, count) {
                    (Some(a), Some(b)) => Some(u64::min(a, b)),
                    (a, b) => a.or(b),
                };
            }
            let fps = clip.frames as f64 / best.as_secs_f64();
            let ivf =
                std::fs::read(&output).with_context(|| format!("read {}", output.display()))?;
            let sha = crate::git_util::sha256_hex(&ivf);
            let row = baseline
                .row
                .iter_mut()
                .find(|row| row.clip == clip.name && row.preset == preset);
            let base = row.as_ref().map_or(0, |row| row.ir_stable);
            let (count, delta) = match &client {
                Some((drrun, lib)) => {
                    let measured = perf::measure_instructions(drrun, lib, &splot, &args)?;
                    let delta =
                        (base != 0).then(|| (measured as f64 - base as f64) / base as f64 * 100.0);
                    (Some(measured), delta)
                }
                None => (retired, None),
            };
            let shown = count.map_or_else(|| "-".to_owned(), |c| c.to_string());
            let delta_shown = delta.map_or_else(|| "-".to_owned(), |d| format!("{d:+.3}"));
            println!(
                "{:<15} {preset:>6} {fps:>9.3} {shown:>15} {base:>15} {delta_shown:>8}  {sha}",
                clip.name
            );
            match (options.bless, row, count) {
                (true, Some(row), Some(measured)) => row.ir_stable = measured,
                (true, None, Some(measured)) => baseline.row.push(Row {
                    clip: clip.name.to_owned(),
                    preset,
                    ir_stable: measured,
                }),
                _ => {}
            }
            if let Some(delta) = delta
                && delta > TOLERANCE_PCT
            {
                regressions.push(format!("{} p{preset}: {delta:+.3}%", clip.name));
            }
        }
    }
    if client.is_none() {
        println!("encode-perf: instruction counts are informational (no DynamoRIO)");
    }
    if options.bless {
        let body = toml::to_string(&baseline).context("serialize encode baseline")?;
        std::fs::write(&baseline_path, format!("{BASELINE_HEADER}\n{body}"))
            .with_context(|| format!("write {}", baseline_path.display()))?;
        println!("encode-perf: blessed {BASELINE_REL}");
        return Ok(());
    }
    if !regressions.is_empty() {
        bail!(
            "encode instruction count regressed beyond {TOLERANCE_PCT}%: {}",
            regressions.join(", ")
        );
    }
    Ok(())
}

/// Samples a preset-6 encode of [`COVERAGE_CLIP`] and prints the SIMD share.
fn run_coverage(splot: &Path, work: &Path) -> Result<()> {
    let clip = &COVERAGE_CLIP;
    let input = work.join(format!("{}.y4m", clip.name));
    std::fs::write(&input, synthetic_y4m(clip.width, clip.height, clip.frames))
        .with_context(|| format!("write {}", input.display()))?;
    let output = work.join(format!("{}.ivf", clip.name));
    let args = encode_args(6, &input, &output);
    let profile = if cfg!(target_os = "macos") {
        sample_macos(splot, &args, &work.join("coverage.sample.txt"))?
    } else {
        sample_perf(splot, &args, &work.join("coverage.perf.data"))?
    };
    let total: f64 = profile.iter().map(|(_, weight)| weight).sum();
    let mut simd = 0.0;
    let mut scalar = 0.0;
    let mut rows: Vec<(String, f64, Kind)> = Vec::new();
    for (symbol, weight) in &profile {
        if let Some(&(_, kind)) = KERNELS.iter().find(|(key, _)| kernel_matches(symbol, key)) {
            match kind {
                Kind::Simd => simd += weight,
                Kind::Scalar => scalar += weight,
            }
            rows.push((symbol.clone(), *weight, kind));
        }
    }
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (symbol, weight, kind) in rows.iter().take(25) {
        println!("{:>7.2}%  {kind:?}  {symbol}", weight / total * 100.0);
    }
    let kernel = simd + scalar;
    if kernel == 0.0 {
        bail!("no allow-listed kernel symbol in the profile");
    }
    println!(
        "kernel samples {:.1}% of all; SIMD share {:.1}% of kernel samples",
        kernel / total * 100.0,
        simd / kernel * 100.0
    );
    Ok(())
}

/// Profiles `splot args` with macOS `sample` and returns self-time samples per
/// symbol from its "Sort by top of stack" section.
fn sample_macos(splot: &Path, args: &[OsString], report: &Path) -> Result<Vec<(String, f64)>> {
    let mut child = Command::new(splot)
        .args(args)
        .spawn()
        .context("spawn splot encode")?;
    let status = Command::new("/usr/bin/sample")
        .arg(child.id().to_string())
        .args(["120", "1", "-mayDie", "-file"])
        .arg(report)
        .output()
        .context("spawn sample")?;
    let encoded = child.wait().context("wait for splot encode")?;
    if !encoded.success() || !status.status.success() {
        bail!("coverage encode or sample failed");
    }
    let text = std::fs::read_to_string(report)?;
    let section = text
        .split_once("Sort by top of stack")
        .map(|(_, rest)| rest)
        .context("sample report has no top-of-stack section")?;
    Ok(section
        .lines()
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let (symbol, count) = line.trim().rsplit_once(char::is_whitespace)?;
            Some((symbol.trim().to_owned(), count.parse().ok()?))
        })
        .collect())
}

/// Profiles `splot args` with Linux `perf` and returns the self-time share per
/// symbol.
fn sample_perf(splot: &Path, args: &[OsString], data: &Path) -> Result<Vec<(String, f64)>> {
    let record = Command::new("perf")
        .arg("record")
        .arg("-o")
        .arg(data)
        .arg("--")
        .arg(splot)
        .args(args)
        .output()
        .context("spawn perf record")?;
    if !record.status.success() {
        bail!(
            "perf record failed: {}",
            String::from_utf8_lossy(&record.stderr)
        );
    }
    let report = Command::new("perf")
        .args([
            "report",
            "--stdio",
            "--no-children",
            "--sort",
            "symbol",
            "-i",
        ])
        .arg(data)
        .output()
        .context("spawn perf report")?;
    Ok(String::from_utf8_lossy(&report.stdout)
        .lines()
        .filter_map(|line| {
            let (percent, symbol) = line.trim().split_once('%')?;
            let symbol = symbol.trim().strip_prefix("[.]")?.trim();
            Some((symbol.to_owned(), percent.trim().parse().ok()?))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{kernel_matches, synthetic_y4m};

    #[test]
    fn kernel_keys_match_demangled_and_mangled_names() {
        let v0 = "_RNvNtNtCs1_12splot_encode7kernels17forward_transform12forward_simd";
        assert!(kernel_matches(v0, "forward_transform::forward_simd"));
        assert!(kernel_matches(v0, "forward_transform::"));
        assert!(!kernel_matches(v0, "forward_transform::forward_lanes"));
        let demangled = "splot_recon::intra_directional_angle::write_middle";
        assert!(kernel_matches(demangled, "intra_directional_angle::"));
        assert!(!kernel_matches(demangled, "intra_directional::"));
        assert!(!kernel_matches(
            "_RNvCs1_11splot_recon23intra_directional_angle",
            "intra_directional::"
        ));
    }

    #[test]
    fn synthetic_clip_has_header_and_full_frames() {
        let clip = synthetic_y4m(16, 8, 2);
        let header = b"YUV4MPEG2 W16 H8 F30:1 Ip A1:1 C420jpeg\n";
        assert!(clip.starts_with(header));
        assert_eq!(clip.len(), header.len() + 2 * (6 + 16 * 8 * 3 / 2));
    }
}
