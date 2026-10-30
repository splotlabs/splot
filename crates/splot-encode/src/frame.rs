// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Owned source frames: three tightly packed `u16` planes for every bit depth;
//! the chroma planes of a monochrome frame are empty.

use splot_recon::{BitDepth, PlaneSize};

use crate::config::Config;
use crate::error::{Error, Result};

/// One input frame. Its plane sizes and bit depth come from the [`Config`] it was
/// built for, and every sample is in range for that bit depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFrame {
    pts: u64,
    bit_depth: BitDepth,
    sizes: Vec<PlaneSize>,
    planes: [Vec<u16>; 3],
}

impl SourceFrame {
    /// Takes ownership of tightly packed Y, U and V planes; U and V are empty
    /// for monochrome.
    ///
    /// # Errors
    /// Returns [`Error::PlaneSizeMismatch`] when a plane does not hold exactly
    /// its width times height samples, [`Error::SampleOutOfRange`] when a sample
    /// exceeds the bit depth, or a configuration error from
    /// [`Config::plane_sizes`].
    pub fn new(config: &Config, pts: u64, planes: [Vec<u16>; 3]) -> Result<Self> {
        let sizes = config.plane_sizes()?;
        let max = config.bit_depth.max_sample();
        for (plane, samples) in planes.iter().enumerate() {
            let expected = sizes
                .get(plane)
                .map_or(0, |size| size.width() * size.height());
            if samples.len() != expected {
                return Err(Error::PlaneSizeMismatch {
                    plane,
                    expected,
                    actual: samples.len(),
                });
            }
            if let Some(&sample) = samples.iter().find(|&&sample| sample > max) {
                return Err(Error::SampleOutOfRange { plane, sample, max });
            }
        }
        Ok(Self {
            pts,
            bit_depth: config.bit_depth,
            sizes,
            planes,
        })
    }

    /// Copies strided 8-bit planes given as `(samples, stride)`; needs an 8-bit
    /// `config`. Monochrome ignores the U and V planes.
    ///
    /// # Errors
    /// Returns [`Error::BitDepthMismatch`] for a non-8-bit `config`,
    /// [`Error::InvalidStride`] or [`Error::PlaneSizeMismatch`] for a bad
    /// stride or a short plane, or any error of [`Self::new`].
    pub fn from_u8(config: &Config, pts: u64, planes: [(&[u8], usize); 3]) -> Result<Self> {
        if config.bit_depth != BitDepth::Eight {
            return Err(Error::BitDepthMismatch {
                expected: config.bit_depth,
                actual: BitDepth::Eight,
            });
        }
        Self::from_strided(config, pts, planes)
    }

    /// Copies strided 16-bit planes given as `(samples, stride)`. Monochrome
    /// ignores the U and V planes.
    ///
    /// # Errors
    /// Returns [`Error::InvalidStride`] or [`Error::PlaneSizeMismatch`] for a
    /// bad stride or a short plane, or any error of [`Self::new`].
    pub fn from_u16(config: &Config, pts: u64, planes: [(&[u16], usize); 3]) -> Result<Self> {
        Self::from_strided(config, pts, planes)
    }

    fn from_strided<T: Copy + Into<u16>>(
        config: &Config,
        pts: u64,
        planes: [(&[T], usize); 3],
    ) -> Result<Self> {
        let sizes = config.plane_sizes()?;
        let mut owned: [Vec<u16>; 3] = Default::default();
        for (plane, ((samples, stride), size)) in planes.into_iter().zip(sizes.iter()).enumerate() {
            let (width, height) = (size.width(), size.height());
            if stride < width {
                return Err(Error::InvalidStride {
                    plane,
                    stride,
                    width,
                });
            }
            let needed = stride.saturating_mul(height - 1).saturating_add(width);
            if samples.len() < needed {
                return Err(Error::PlaneSizeMismatch {
                    plane,
                    expected: needed,
                    actual: samples.len(),
                });
            }
            owned[plane] = samples
                .chunks(stride)
                .take(height)
                .flat_map(|row| row[..width].iter().map(|&sample| sample.into()))
                .collect();
        }
        Self::new(config, pts, owned)
    }

    /// Presentation timestamp.
    #[must_use]
    pub fn pts(&self) -> u64 {
        self.pts
    }

    /// Sample bit depth.
    #[must_use]
    pub fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    /// Sizes of the Y, U and V planes; Y alone for monochrome.
    #[must_use]
    pub fn plane_sizes(&self) -> &[PlaneSize] {
        &self.sizes
    }

    /// The Y, U and V planes, each packed with stride equal to its width.
    #[must_use]
    pub fn planes(&self) -> &[Vec<u16>; 3] {
        &self.planes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strided_u8_input_is_packed() -> Result<()> {
        let config = Config::new(3, 2);
        let y = [1, 2, 3, 9, 4, 5, 6];
        let frame = SourceFrame::from_u8(&config, 7, [(&y, 4), (&[8, 8], 2), (&[9, 9], 2)])?;
        assert_eq!(frame.planes()[0], [1, 2, 3, 4, 5, 6]);
        assert_eq!(frame.planes()[1], [8, 8]);
        assert_eq!(frame.pts(), 7);
        Ok(())
    }

    #[test]
    fn rejects_short_planes_and_bad_strides() {
        let config = Config::new(3, 2);
        let short = SourceFrame::from_u8(&config, 0, [(&[0; 6], 4), (&[0; 2], 2), (&[0; 2], 2)]);
        assert!(matches!(
            short,
            Err(Error::PlaneSizeMismatch { plane: 0, .. })
        ));
        let narrow = SourceFrame::from_u8(&config, 0, [(&[0; 6], 2), (&[0; 2], 2), (&[0; 2], 2)]);
        assert!(matches!(narrow, Err(Error::InvalidStride { plane: 0, .. })));
    }

    #[test]
    fn rejects_samples_above_the_bit_depth() {
        let mut config = Config::new(2, 2);
        config.bit_depth = BitDepth::Ten;
        let planes = [vec![1023; 4], vec![1024], vec![0]];
        assert!(matches!(
            SourceFrame::new(&config, 0, planes),
            Err(Error::SampleOutOfRange {
                plane: 1,
                sample: 1024,
                max: 1023
            })
        ));
        assert!(matches!(
            SourceFrame::from_u8(&config, 0, [(&[0; 4], 2), (&[0], 1), (&[0], 1)]),
            Err(Error::BitDepthMismatch { .. })
        ));
    }
}
