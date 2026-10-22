// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Local luma BD-rate (`cargo xtask bd-rate`).
//!
//! Encodes one 8-bit 4:2:0 Y4M clip (default: the synthetic `encode-perf`
//! clip) at four or more QPs with the release `splot`, and the same QPs with
//! the anchor: `avmenc` (path in `AVMENC`) or, with `--baseline`, another
//! `splot` binary. Every stream is decoded by `splot decode`, its luma PSNR is
//! measured against the source, and the BD-rate of `splot` against the anchor
//! is printed (negative = fewer bits at equal PSNR). Local only; never in CI.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// Options of `cargo xtask bd-rate`.
pub(crate) struct BdRateOptions {
    /// Source clip; `None` writes the synthetic clip.
    pub(crate) clip: Option<PathBuf>,
    /// Frames to encode.
    pub(crate) frames: usize,
    /// `splot` preset.
    pub(crate) preset: u8,
    /// `avmenc --cpu-used`.
    pub(crate) cpu_used: u8,
    /// Quantizer indices, at least four.
    pub(crate) qps: Vec<u16>,
    /// Code every frame as a key frame.
    pub(crate) intra: bool,
    /// Anchor `splot` binary instead of `avmenc`.
    pub(crate) baseline: Option<PathBuf>,
}

/// The encoder that produces one curve.
enum Encoder<'a> {
    Splot(&'a Path),
    Avm(&'a Path),
}

/// Runs the BD-rate comparison.
///
/// # Errors
/// Returns an error for fewer than four QPs, a clip that is not 8-bit 4:2:0,
/// a missing anchor, or a failed encode or decode.
pub(crate) fn run_bd_rate(root: &Path, options: &BdRateOptions) -> Result<()> {
    if options.qps.len() < 4 {
        bail!("bd-rate needs at least four --qp values");
    }
    let work = root.join("target").join("bd-rate");
    std::fs::create_dir_all(&work).with_context(|| format!("create {}", work.display()))?;
    let clip = if let Some(clip) = &options.clip {
        clip.clone()
    } else {
        let path = work.join("synthetic-352x288.y4m");
        let clip = crate::encode_perf::synthetic_y4m(352, 288, options.frames);
        std::fs::write(&path, clip).with_context(|| format!("write {}", path.display()))?;
        path
    };
    let source = std::fs::read(&clip).with_context(|| format!("read {}", clip.display()))?;
    let (width, height, luma) = y4m_luma(&source, options.frames)?;
    let splot = crate::conformance::build_splot_binary(root, true)?;
    let avmenc = std::env::var_os("AVMENC").map(PathBuf::from);
    let anchor = match (&options.baseline, &avmenc) {
        (Some(baseline), _) => Encoder::Splot(baseline),
        (None, Some(avmenc)) => Encoder::Avm(avmenc),
        (None, None) => bail!("set AVMENC to an avmenc binary, or pass --baseline <splot>"),
    };
    let mut curves = Vec::new();
    for (name, encoder) in [("splot", Encoder::Splot(&splot)), ("anchor", anchor)] {
        let mut points = Vec::new();
        for &qp in &options.qps {
            let ivf = work.join(format!("{name}-q{qp}.ivf"));
            let status = Command::new(match encoder {
                Encoder::Splot(binary) | Encoder::Avm(binary) => binary,
            })
            .args(encode_args(&encoder, options, qp, &clip, &ivf))
            .output()
            .context("spawn encoder")?;
            if !status.status.success() {
                bail!(
                    "{name} qp {qp} failed:\n{}",
                    String::from_utf8_lossy(&status.stderr)
                );
            }
            let bytes = std::fs::metadata(&ivf)?.len();
            let decoded = Command::new(&splot)
                .args(["decode", "--quiet", "--output-format", "raw", "-o", "-"])
                .arg(&ivf)
                .output()
                .context("spawn splot decode")?;
            if !decoded.status.success() {
                bail!("decode of {} failed", ivf.display());
            }
            let psnr = luma_psnr(&luma, &decoded.stdout, width, height)?;
            let kbits = bytes as f64 * 8.0 / 1000.0 / luma.len() as f64;
            println!("{name:<6} qp {qp:>3}: {kbits:>9.2} kbit/frame  Y-PSNR {psnr:>6.2}");
            points.push((kbits, psnr));
        }
        curves.push(points);
    }
    let rate = bd_rate(&curves[1], &curves[0])
        .context("the two curves share no PSNR range; widen the QPs")?;
    println!("BD-rate splot vs anchor (luma): {rate:+.2}%");
    Ok(())
}

/// The encode command line for `encoder` at `qp`.
fn encode_args(
    encoder: &Encoder<'_>,
    options: &BdRateOptions,
    qp: u16,
    clip: &Path,
    ivf: &Path,
) -> Vec<OsString> {
    let frames = options.frames.to_string();
    let mut args: Vec<OsString> = Vec::new();
    match encoder {
        Encoder::Splot(_) => {
            let keyint = if options.intra { "1" } else { "30" };
            args.extend(
                [
                    "encode",
                    "--quiet",
                    "--threads",
                    "1",
                    "--limit",
                    &frames,
                    "--keyint",
                    keyint,
                    "--preset",
                    &options.preset.to_string(),
                    "--qp",
                    &qp.to_string(),
                    "-o",
                ]
                .map(OsString::from),
            );
            args.push(ivf.into());
            args.push(clip.into());
        }
        Encoder::Avm(_) => {
            args.push(clip.into());
            args.push("-o".into());
            args.push(ivf.into());
            args.extend(
                [
                    "--ivf".to_owned(),
                    "-q".to_owned(),
                    format!("--limit={frames}"),
                    "--end-usage=q".to_owned(),
                    format!("--qp={qp}"),
                    format!("--cpu-used={}", options.cpu_used),
                    format!("--kf-max-dist={}", if options.intra { 0 } else { 30 }),
                    format!("--lag-in-frames={}", if options.intra { 0 } else { 16 }),
                ]
                .map(OsString::from),
            );
        }
    }
    args
}

/// The size and the first `limit` luma planes of an 8-bit 4:2:0 Y4M clip.
fn y4m_luma(data: &[u8], limit: usize) -> Result<(usize, usize, Vec<&[u8]>)> {
    let header_end = data
        .iter()
        .position(|&b| b == b'\n')
        .context("Y4M header has no newline")?;
    let header = std::str::from_utf8(&data[..header_end]).context("Y4M header is not UTF-8")?;
    let field = |tag: char| {
        header
            .split_whitespace()
            .find_map(|field| field.strip_prefix(tag))
    };
    let width: usize = field('W').context("Y4M W")?.parse()?;
    let height: usize = field('H').context("Y4M H")?.parse()?;
    if !field('C').is_none_or(|c| c.starts_with("420") && !c.starts_with("420p1")) {
        bail!("bd-rate measures 8-bit 4:2:0 clips only");
    }
    let frame = width * height * 3 / 2;
    let mut pos = header_end + 1;
    let mut planes = Vec::new();
    while planes.len() < limit && pos < data.len() {
        let line = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .context("Y4M frame header has no newline")?;
        pos += line + 1;
        let luma = data
            .get(pos..pos + width * height)
            .context("Y4M frame is truncated")?;
        planes.push(luma);
        pos += frame;
    }
    if planes.is_empty() {
        bail!("Y4M clip has no frames");
    }
    Ok((width, height, planes))
}

/// The mean per-frame luma PSNR of raw 8-bit 4:2:0 `decoded` against `source`.
fn luma_psnr(source: &[&[u8]], decoded: &[u8], width: usize, height: usize) -> Result<f64> {
    let frame = width * height * 3 / 2;
    let mut sum = 0.0;
    for (index, src) in source.iter().enumerate() {
        let out = decoded
            .get(index * frame..index * frame + width * height)
            .context("decoded stream has fewer frames than the source")?;
        let sse: u64 = src
            .iter()
            .zip(out)
            .map(|(&a, &b)| u64::from(a.abs_diff(b)).pow(2))
            .sum();
        let mse = sse as f64 / (width * height) as f64;
        sum += if mse == 0.0 {
            100.0
        } else {
            10.0 * (255.0 * 255.0 / mse).log10()
        };
    }
    Ok(sum / source.len() as f64)
}

/// The Bjontegaard rate difference of `test` against `anchor` in percent:
/// a least-squares cubic of log rate over PSNR per curve, averaged over the
/// shared PSNR range. Points are `(rate, psnr)`; `None` without overlap.
fn bd_rate(anchor: &[(f64, f64)], test: &[(f64, f64)]) -> Option<f64> {
    let range = |points: &[(f64, f64)]| {
        points
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), &(_, p)| {
                (lo.min(p), hi.max(p))
            })
    };
    let ((lo_a, hi_a), (lo_t, hi_t)) = (range(anchor), range(test));
    let (lo, hi) = (lo_a.max(lo_t), hi_a.min(hi_t));
    if hi <= lo {
        return None;
    }
    let integral = |points: &[(f64, f64)]| -> Option<f64> {
        let c = cubic_fit(points)?;
        let antiderivative = |x: f64| {
            (0..4)
                .map(|k| c[k] * x.powi(k as i32 + 1) / (k + 1) as f64)
                .sum::<f64>()
        };
        Some(antiderivative(hi) - antiderivative(lo))
    };
    let diff = (integral(test)? - integral(anchor)?) / (hi - lo);
    Some((diff.exp() - 1.0) * 100.0)
}

/// Least-squares `ln(rate) = c0 + c1 p + c2 p^2 + c3 p^3` over `(rate, p)`.
fn cubic_fit(points: &[(f64, f64)]) -> Option<[f64; 4]> {
    let mut m = [[0.0f64; 5]; 4];
    for &(rate, p) in points {
        let y = rate.ln();
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().take(4).enumerate() {
                *cell += p.powi((i + j) as i32);
            }
            row[4] += y * p.powi(i as i32);
        }
    }
    for col in 0..4 {
        let pivot = (col..4).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
        m.swap(col, pivot);
        if m[col][col].abs() < 1e-12 {
            return None;
        }
        for row in 0..4 {
            if row != col {
                let factor = m[row][col] / m[col][col];
                let pivot_row = m[col];
                for (cell, &p) in m[row].iter_mut().zip(&pivot_row) {
                    *cell -= factor * p;
                }
            }
        }
    }
    Some([0, 1, 2, 3].map(|i| m[i][4] / m[i][i]))
}

#[cfg(test)]
mod tests {
    use super::bd_rate;

    #[test]
    fn bd_rate_of_a_scaled_curve_is_the_scale() {
        let anchor = [(100.0, 30.0), (200.0, 33.0), (400.0, 36.0), (800.0, 39.0)];
        let test = anchor.map(|(rate, psnr)| (rate * 0.9, psnr));
        let rate = bd_rate(&anchor, &test).unwrap_or(f64::NAN);
        assert!((rate + 10.0).abs() < 1e-6, "{rate}");
        assert_eq!(bd_rate(&anchor, &[(1.0, 50.0), (2.0, 51.0)]), None);
    }
}
