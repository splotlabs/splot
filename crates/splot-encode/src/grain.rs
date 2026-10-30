// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Film grain estimation: the noise of a key frame source as the § 5.18.10.2
//! scaling functions of a model that § 7.21.7 synthesizes at the decoder.

use splot_core::headers::frame::GrainScaling;
use splot_core::tables::conversion::GAUSSIAN_SEQUENCE;

use crate::frame::SourceFrame;

/// Block side of the noise measurement.
const BLOCK: usize = 8;
/// Intensity bins of each scaling function, over the 8-bit sample range.
const BINS: usize = 8;
/// Blocks a bin needs before it gives a scaling point.
const MIN_BLOCKS: usize = 8;
/// The noise standard deviation, in 8-bit samples, below which a plane gets
/// no grain.
const MIN_SIGMA: f64 = 1.0;
/// `2^(ScalingShift + 4)` of `grain_scaling_minus_8 = 0` and
/// `grain_scale_shift = 0`: a scaling `s` gives noise of standard deviation
/// `s * std(Gaussian_Sequence) / 4096` 8-bit samples (§ 7.21.7.3, § 7.21.7.5).
const NOISE_UNIT: f64 = 4096.0;

/// The `[Y, Cb, Cr]` scaling functions of the noise in `frame`; a plane whose
/// noise stays under [`MIN_SIGMA`] in every bin gets no points.
pub(crate) fn estimate(frame: &SourceFrame) -> [GrainScaling; 3] {
    let shift = u32::from(frame.bit_depth().bits()) - 8;
    let gaussian = gaussian_sigma();
    let mut scalings: [GrainScaling; 3] = Default::default();
    for ((scaling, samples), size) in scalings
        .iter_mut()
        .zip(frame.planes())
        .zip(frame.plane_sizes())
    {
        let sigmas = bin_sigmas(samples, size.width(), size.height(), shift);
        if sigmas.iter().flatten().all(|&sigma| sigma < MIN_SIGMA) {
            continue;
        }
        *scaling = sigmas
            .iter()
            .enumerate()
            .filter_map(|(bin, &sigma)| {
                let value = (bin * 256 + 128) / BINS;
                let scaling = (sigma? * NOISE_UNIT / gaussian).round().clamp(0.0, 255.0);
                Some((value as u8, scaling as u8))
            })
            .collect();
    }
    scalings
}

/// The standard deviation of `Gaussian_Sequence` (§ 7.21.7.3).
fn gaussian_sigma() -> f64 {
    let sum: f64 = GAUSSIAN_SEQUENCE
        .iter()
        .map(|&g| f64::from(g).powi(2))
        .sum();
    (sum / GAUSSIAN_SEQUENCE.len() as f64).sqrt()
}

/// The noise standard deviation, in 8-bit samples, of each intensity bin of a
/// `width x height` plane: the lower quartile over its whole 8x8 blocks of
/// the Laplacian estimate `sqrt(pi / 2) / 6 * mean |L|`, which flat blocks set.
fn bin_sigmas(samples: &[u16], width: usize, height: usize, shift: u32) -> [Option<f64>; BINS] {
    let mut bins: [Vec<f64>; BINS] = Default::default();
    for top in (0..height / BLOCK * BLOCK).step_by(BLOCK) {
        for left in (0..width / BLOCK * BLOCK).step_by(BLOCK) {
            let at = |x: usize, y: usize| i64::from(samples[(top + y) * width + left + x]);
            let (mut sum, mut laplacian) = (0, 0);
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    sum += at(x, y);
                    if (1..BLOCK - 1).contains(&x) && (1..BLOCK - 1).contains(&y) {
                        let cross = at(x - 1, y) + at(x + 1, y) + at(x, y - 1) + at(x, y + 1);
                        let diagonal = at(x - 1, y - 1)
                            + at(x + 1, y - 1)
                            + at(x - 1, y + 1)
                            + at(x + 1, y + 1);
                        laplacian += (4 * at(x, y) - 2 * cross + diagonal).abs();
                    }
                }
            }
            let mean = (sum >> shift) as usize / (BLOCK * BLOCK);
            let interior = ((BLOCK - 2) * (BLOCK - 2)) as f64;
            let sigma = (core::f64::consts::FRAC_PI_2).sqrt() * laplacian as f64
                / (6.0 * interior * f64::from(1u32 << shift));
            bins[(mean * BINS / 256).min(BINS - 1)].push(sigma);
        }
    }
    bins.map(|mut sigmas| {
        if sigmas.len() < MIN_BLOCKS {
            return None;
        }
        sigmas.sort_by(f64::total_cmp);
        sigmas.get(sigmas.len() / 4).copied()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use splot_recon::BitDepth;

    /// A plane of flat halves at 60 and 180 (8-bit scale) with Gaussian noise
    /// of `sigma` 8-bit samples, from a seeded Box-Muller generator.
    fn noisy_frame(bit_depth: BitDepth, sigma: f64) -> SourceFrame {
        let mut config = Config::new(128, 96);
        config.bit_depth = bit_depth;
        let scale = f64::from(1u32 << (bit_depth.bits() - 8));
        let mut state = 0x2545_f491_u32;
        let mut uniform = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (f64::from(state) + 1.0) / 4_294_967_297.0
        };
        let sizes = config.plane_sizes().unwrap_or_default();
        let planes = core::array::from_fn(|plane| {
            let width = sizes.get(plane).map_or(0, |size| size.width());
            let count = sizes
                .get(plane)
                .map_or(0, |size| size.width() * size.height());
            (0..count)
                .map(|i| {
                    let base = if i % width < width / 2 { 60.0 } else { 180.0 };
                    let gauss = (-2.0 * uniform().ln()).sqrt()
                        * (2.0 * core::f64::consts::PI * uniform()).cos();
                    ((base + sigma * gauss) * scale).round() as u16
                })
                .collect()
        });
        SourceFrame::new(&config, 0, planes).unwrap_or_else(|_| unreachable!("valid planes"))
    }

    /// The scaling of each point maps back to the noise within 30%, at both
    /// bit depths, in the two bins the halves fill.
    #[test]
    fn scaling_tracks_the_source_noise() {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            for sigma in [3.0, 8.0] {
                let [y, cb, cr] = estimate(&noisy_frame(bit_depth, sigma));
                for scaling in [&y, &cb, &cr] {
                    let values: Vec<u8> = scaling.iter().map(|&(value, _)| value).collect();
                    assert_eq!(values, [48, 176], "{bit_depth:?} {sigma}");
                    for &(_, scale) in scaling {
                        let measured = f64::from(scale) * gaussian_sigma() / NOISE_UNIT;
                        assert!(
                            (measured / sigma - 1.0).abs() < 0.3,
                            "{bit_depth:?}: {measured} vs {sigma}"
                        );
                    }
                }
            }
        }
    }

    /// Clean content, and noise under the threshold, give no grain.
    #[test]
    fn quiet_sources_get_no_grain() {
        for sigma in [0.0, 0.4] {
            let scalings = estimate(&noisy_frame(BitDepth::Eight, sigma));
            assert!(scalings.iter().all(Vec::is_empty), "{sigma}");
        }
    }
}
