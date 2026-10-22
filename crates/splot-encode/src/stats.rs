// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-frame statistics and quality metrics between source and reconstruction.

use splot_recon::BitDepth;

/// Statistics of one coded frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    /// Zero-based coding-order index.
    pub frame_index: u64,
    /// Presentation timestamp of the source frame.
    pub pts: u64,
    /// The frame's `base_q_idx`.
    pub qp: u8,
    /// Size of the frame's OBUs in bits, with the temporal delimiter (and the
    /// sequence header of a key frame) when the frame opens its temporal unit.
    pub bits: u64,
    /// Luma PSNR in dB; infinite for an exact reconstruction.
    pub psnr_y: f64,
    /// Cb PSNR in dB.
    pub psnr_u: f64,
    /// Cr PSNR in dB.
    pub psnr_v: f64,
    /// Luma SSIM, when computed.
    pub ssim_y: Option<f64>,
}

/// PSNR in dB of `recon` against `source` for samples of `bit_depth`.
/// Returns infinity when the planes are equal, and NaN when they are empty or
/// differ in length.
#[must_use]
pub fn psnr(source: &[u16], recon: &[u16], bit_depth: BitDepth) -> f64 {
    if source.len() != recon.len() {
        return f64::NAN;
    }
    let squared_error: u64 = source
        .iter()
        .zip(recon)
        .map(|(&a, &b)| u64::from(a.abs_diff(b)).pow(2))
        .sum();
    let peak = f64::from(bit_depth.max_sample());
    let mse = squared_error as f64 / source.len() as f64;
    10.0 * (peak * peak / mse).log10()
}

/// Mean SSIM of `recon` against `source`, a `width`-wide plane of `bit_depth`
/// samples, over non-overlapping 8x8 windows (edge windows are smaller).
/// Returns NaN when the planes are empty or differ in length.
#[must_use]
pub fn ssim(source: &[u16], recon: &[u16], width: usize, bit_depth: BitDepth) -> f64 {
    const WINDOW: usize = 8;
    if source.len() != recon.len() || width == 0 {
        return f64::NAN;
    }
    let peak = f64::from(bit_depth.max_sample());
    let c1 = (0.01 * peak).powi(2);
    let c2 = (0.03 * peak).powi(2);
    let height = source.len() / width;
    let mut total = 0.0;
    let mut windows = 0_u32;
    for y0 in (0..height).step_by(WINDOW) {
        for x0 in (0..width).step_by(WINDOW) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let mut n = 0.0;
            for y in y0..(y0 + WINDOW).min(height) {
                for x in x0..(x0 + WINDOW).min(width) {
                    let a = f64::from(source[y * width + x]);
                    let b = f64::from(recon[y * width + x]);
                    sa += a;
                    sb += b;
                    saa += a * a;
                    sbb += b * b;
                    sab += a * b;
                    n += 1.0;
                }
            }
            let (ma, mb) = (sa / n, sb / n);
            let (va, vb, cov) = (saa / n - ma * ma, sbb / n - mb * mb, sab / n - ma * mb);
            total += ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            windows += 1;
        }
    }
    total / f64::from(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psnr_of_known_error() {
        assert!(psnr(&[10, 20], &[10, 20], BitDepth::Eight).is_infinite());
        // MSE 1 at 8 bits: 10 * log10(255^2).
        let value = psnr(&[10, 20], &[11, 19], BitDepth::Eight);
        assert!((value - 48.130_803_608).abs() < 1e-6, "{value}");
    }

    #[test]
    fn ssim_is_one_for_equal_planes_and_lower_otherwise() {
        let source: Vec<u16> = (0..100).map(|i| (i * 7 % 256) as u16).collect();
        assert!((ssim(&source, &source, 10, BitDepth::Eight) - 1.0).abs() < 1e-12);
        let noisy: Vec<u16> = source.iter().map(|&s| s ^ 8).collect();
        assert!(ssim(&source, &noisy, 10, BitDepth::Eight) < 0.99);
    }
}
