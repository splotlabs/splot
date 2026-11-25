// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 Annex D.5 alpha blending within the intersection of texture and alpha rows.

use fearless_simd::{Simd, prelude::*, u8x16, u16x8, u32x4};
use splot_core::simd::SimdBackend;

use crate::{ReconError, ReconSample, Result, intra_dc_math::IntraLaneSample};

/// Samples from the visible part of an alpha plane row.
#[derive(Clone, Copy, Debug)]
pub enum AtlasAlphaRow<'a> {
    /// Eight-bit samples.
    Eight(&'a [u8]),
    /// Ten-bit samples in the low ten bits of each word.
    Ten(&'a [u16]),
}

/// Decoded samples that support atlas composition.
pub trait AtlasSample: ReconSample {
    /// Blends aligned texture and alpha rows into the destination with Annex D.5 rounding.
    ///
    /// # Errors
    /// Returns a length error if the required texture or alpha samples are missing.
    fn blend_atlas_row<B: SimdBackend>(
        backend: B,
        canvas: &mut [Self],
        texture: &[Self],
        alpha: AtlasAlphaRow<'_>,
    ) -> Result<()>;

    /// Resamples one clipped run by selecting the sample at floor(x * source width / target width).
    ///
    /// The first output sample has coordinate `first` in the complete target row.
    /// Horizontal chroma subsampling selects every second target coordinate for alpha.
    ///
    /// # Errors
    /// Returns an error for an empty source, invalid target coordinates, or an overflowing index.
    fn nearest_atlas_row<B: SimdBackend>(
        backend: B,
        out: &mut [Self],
        source: &[Self],
        width: u32,
        first: u32,
        subsampled_x: bool,
    ) -> Result<()>;
}

impl<T: IntraLaneSample> AtlasSample for T {
    fn blend_atlas_row<B: SimdBackend>(
        backend: B,
        canvas: &mut [T],
        texture: &[T],
        alpha: AtlasAlphaRow<'_>,
    ) -> Result<()> {
        for len in [
            texture.len(),
            match alpha {
                AtlasAlphaRow::Eight(row) => row.len(),
                AtlasAlphaRow::Ten(row) => row.len(),
            },
        ] {
            if len != canvas.len() {
                return Err(ReconError::BufferLengthMismatch {
                    expected: canvas.len(),
                    actual: len,
                });
            }
        }
        let narrow = backend.narrow();
        narrow.vectorize(|| match alpha {
            AtlasAlphaRow::Eight(row) => blend(narrow.token(), canvas, texture, row, 8),
            AtlasAlphaRow::Ten(row) => blend(narrow.token(), canvas, texture, row, 10),
        });
        Ok(())
    }

    fn nearest_atlas_row<B: SimdBackend>(
        backend: B,
        out: &mut [T],
        source: &[T],
        width: u32,
        first: u32,
        subsampled_x: bool,
    ) -> Result<()> {
        if out.is_empty() {
            return Ok(());
        }
        if source.is_empty() {
            return Err(ReconError::ZeroDimension {
                field: "atlas source width",
            });
        }
        let source_width =
            u32::try_from(source.len()).map_err(|_| ReconError::ArithmeticOverflow {
                context: "atlas source width",
            })?;
        let step = 1 + u32::from(subsampled_x);
        let available = width.saturating_sub(first).div_ceil(step) as usize;
        if out.len() > available {
            return Err(ReconError::BufferLengthMismatch {
                expected: out.len(),
                actual: available,
            });
        }
        if width == source_width && step == 1 {
            // splot-copy-ok: place a native row in the independent presentation canvas
            out.copy_from_slice(&source[first as usize..][..out.len()]);
            return Ok(());
        }
        let narrow = backend.narrow();
        narrow.vectorize(|| {
            let simd = narrow.token();
            for (group, out) in out.chunks_mut(8).enumerate() {
                let mut indices = [0; 8];
                for (lane, index) in indices.iter_mut().take(out.len()).enumerate() {
                    let x = first + (group * 8 + lane) as u32 * step;
                    *index = (u64::from(x) * u64::from(source_width) / u64::from(width)) as usize;
                }
                let mut value = u8x16::splat(simd, 0);
                let mut lane = 0;
                while lane < out.len() {
                    let start = indices[lane];
                    let mut select = [0x80; 16];
                    while lane < out.len() && indices[lane] - start < 8 {
                        let byte = 2 * (indices[lane] - start) as u8;
                        select[2 * lane] = byte;
                        select[2 * lane + 1] = byte + 1;
                        lane += 1;
                    }
                    let samples = load(simd, &source[start..]).bitcast::<u8x16<_>>();
                    value |= samples.swizzle_dyn(u8x16::load_array(simd, select));
                }
                store(simd, value.bitcast(), out);
            }
        });
        Ok(())
    }
}

fn load<S: Simd, T: IntraLaneSample>(simd: S, source: &[T]) -> u16x8<S> {
    if let Some(full) = source.first_chunk::<8>() {
        return T::load8(simd, full);
    }
    let mut padded = [T::default(); 8];
    padded[..source.len()].copy_from_slice(source); // splot-copy-ok: pad a partial SIMD load
    T::load8(simd, &padded)
}

fn blend<S: Simd, T: IntraLaneSample, A: IntraLaneSample>(
    simd: S,
    canvas: &mut [T],
    texture: &[T],
    alpha: &[A],
    depth: u32,
) {
    let alpha_max = 1 << depth;
    let bias = u32x4::splat(simd, alpha_max / 2);
    for ((out, foreground), mask) in canvas
        .chunks_mut(8)
        .zip(texture.chunks(8))
        .zip(alpha.chunks(8))
    {
        let weights = load(simd, mask);
        let inverse = u16x8::splat(simd, alpha_max as u16) - weights;
        let (back_lo, back_hi) = load(simd, out).widening_mul(inverse);
        let (front_lo, front_hi) = load(simd, foreground).widening_mul(weights);
        let low = (back_lo + front_lo + bias) >> depth;
        let high = (back_hi + front_hi + bias) >> depth;
        let result = low.narrow(high);
        store(simd, result, out);
    }
}

fn store<S: Simd, T: IntraLaneSample>(simd: S, value: u16x8<S>, out: &mut [T]) {
    if let Some(chunk) = out.first_chunk_mut::<8>() {
        T::store8(simd, value, chunk);
    } else {
        let mut padded = [T::default(); 8];
        T::store8(simd, value, &mut padded);
        out.copy_from_slice(&padded[..out.len()]); // splot-copy-ok: store only visible SIMD lanes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_rounding_for_eight_and_ten_bit_textures() {
        baseline_simd!(simd => {
            let mut luma = [20u8; 3];
            blend(simd, &mut luma, &[220; 3], &[0u8, 128, 255], 8);
            assert_eq!(luma, [20, 120, 219]);
            let mut chroma = [20u16; 3];
            blend(simd, &mut chroma, &[220; 3], &[0u8, 128, 255], 8);
            assert_eq!(chroma, [20, 120, 219]);
        });
    }

    #[test]
    fn blend_matches_integer_oracle_for_all_tail_lengths() {
        let alpha: Vec<u16> = (0..65).map(|x| (x * 157) % 1024).collect();
        let texture: Vec<u16> = (0..33).map(|x| (x * 259 + 97) % 1024).collect();
        baseline_simd!(simd => {
                for width in 0..=texture.len() {
                    let mut out = vec![1023; width];
                    blend(simd, &mut out, &texture[..width], &alpha, 10);
                    for (i, &sample) in out.iter().enumerate() {
                        let a = u32::from(alpha[i]);
                        let expected = ((1024 - a) * 1023 + a * u32::from(texture[i]) + 512) >> 10;
                        assert_eq!(u32::from(sample), expected);
                    }
                }
        });
    }
}
