// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fixed-ratio resize of source frames before coding. The resampler is the
//! § 7.13.3.18 stepped `EIGHTTAP_SMOOTH` convolution of scaled references.

use splot_recon::{
    BitDepth, InterpolationFilter, PlaneSize, ReferencePlaneView, SubpelPredictParams,
    subpel_predict_block_strided_into,
};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::frame::SourceFrame;

/// Output tile side of one convolution call; at most the 256-sample block limit.
const TILE: usize = 64;
/// `1 << SCALE_SUBPEL_BITS`: one sample in § 7.13.3.18 position units.
const ONE: i64 = 1 << 10;

impl SourceFrame {
    /// The frame resampled to the plane sizes of `config`, at most 2x smaller
    /// per axis.
    pub(crate) fn resized(&self, config: &Config) -> Result<Self> {
        let sizes = config.plane_sizes()?;
        let mut planes: [Vec<u16>; 3] = Default::default();
        for (plane, (out, &size)) in planes.iter_mut().zip(&sizes).enumerate() {
            let (Some(samples), Some(&from)) =
                (self.planes().get(plane), self.plane_sizes().get(plane))
            else {
                continue;
            };
            *out = resample(samples, from, size, self.bit_depth())?;
        }
        Self::new(config, self.pts(), planes)
    }
}

/// The `from`-sized `samples` resampled to `to`, sample centres aligned.
fn resample(
    samples: &[u16],
    from: PlaneSize,
    to: PlaneSize,
    bit_depth: BitDepth,
) -> Result<Vec<u16>> {
    let failed = |source: splot_recon::ReconError| Error::Coding {
        stage: "resize",
        source: Box::new(source),
    };
    let reference =
        ReferencePlaneView::new(samples, from.width(), from.height()).map_err(failed)?;
    let axis = |from: usize, to: usize| {
        let step = ((from as i64 * ONE) + to as i64 / 2) / to as i64;
        (step, (step - ONE) / 2)
    };
    let (step_x, offset_x) = axis(from.width(), to.width());
    let (step_y, offset_y) = axis(from.height(), to.height());
    let narrow = |value: i64| i32::try_from(value).unwrap_or(i32::MAX);
    let mut out = vec![0; to.width() * to.height()];
    for y in (0..to.height()).step_by(TILE) {
        for x in (0..to.width()).step_by(TILE) {
            let params = SubpelPredictParams {
                interp: InterpolationFilter::EightTapSmooth,
                w: TILE.min(to.width() - x),
                h: TILE.min(to.height() - y),
                start_x: narrow(x as i64 * step_x + offset_x),
                start_y: narrow(y as i64 * step_y + offset_y),
                step_x: narrow(step_x),
                step_y: narrow(step_y),
                first_x: 0,
                first_y: 0,
                last_x: narrow(from.width() as i64 - 1),
                last_y: narrow(from.height() as i64 - 1),
                bit_depth,
            };
            let at = y * to.width() + x;
            subpel_predict_block_strided_into(&reference, &params, &mut out[at..], to.width())
                .map_err(failed)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: usize, height: usize) -> PlaneSize {
        PlaneSize::new(width, height).unwrap_or_else(|_| unreachable!("positive size"))
    }

    #[test]
    fn same_size_is_a_copy() -> Result<()> {
        let samples: Vec<u16> = (0..35u16).map(|i| i * 29 % 251).collect();
        let out = resample(&samples, size(7, 5), size(7, 5), BitDepth::Eight)?;
        assert_eq!(out, samples);
        Ok(())
    }

    #[test]
    fn flat_planes_stay_flat_at_every_ratio() -> Result<()> {
        for (sample, bit_depth) in [(37, BitDepth::Eight), (1023, BitDepth::Ten)] {
            let samples = vec![sample; 131 * 70];
            for denominator in 9..=16 {
                let scale = |value: usize| (value * 8 + denominator / 2) / denominator;
                let to = size(scale(131), scale(70));
                let out = resample(&samples, size(131, 70), to, bit_depth)?;
                assert!(out.iter().all(|&s| s == sample), "{denominator}");
            }
        }
        Ok(())
    }

    /// A half-size ramp follows the source ramp at the mapped sample centres,
    /// including the edges, where the source is clamped.
    #[test]
    fn ramps_land_on_the_mapped_centres() -> Result<()> {
        let (width, height) = (200, 6);
        let samples: Vec<u16> = (0..height)
            .flat_map(|_| (0..width).map(|x| 3 * x as u16 + 17))
            .collect();
        let out = resample(&samples, size(width, height), size(100, 3), BitDepth::Ten)?;
        for (x, &sample) in out[..100].iter().enumerate() {
            let centre = 2.0 * x as f64 + 0.5;
            let expected = 3.0 * centre + 17.0;
            assert!(
                (f64::from(sample) - expected).abs() <= 2.0,
                "{x}: {sample} vs {expected}"
            );
        }
        Ok(())
    }
}
