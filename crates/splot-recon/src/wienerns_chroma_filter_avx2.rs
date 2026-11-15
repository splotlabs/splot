// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The x86 group kernels of the padded § 7.20.3 chroma filter.

use core::arch::x86_64::*;

use fearless_simd::{Avx2, SimdBase, SimdInto, u8x16, u16x8, u16x16};

use super::{
    WIENER_NS_CHROMA_COEFF_SLOTS, WIENER_NS_CHROMA_COEFFS, WIENER_NS_CHROMA_TAP_RADIUS,
    WIENER_NS_CONFIG_UV, WIENER_NS_CONFIG_UV_PAIRS, WIENER_NS_PREC_BITS,
};

const R: usize = WIENER_NS_CHROMA_TAP_RADIUS;
const REACH: usize = 16 + 2 * R;

/// One plane's samples and stride.
type Plane<'a, S> = (&'a [S], usize);

/// The padded rows of one 16-sample group: row `R + dy` from column
/// `R + dx` holds the taps at offset `(dy, dx)`.
pub(super) type Window<'a, S> = [&'a [S; REACH]; 2 * R + 1];

/// The `2 * R + 1` rows of `plane` from row `r`, each `width + 2 * R`
/// long, so one length check covers a group in all of them.
#[inline]
fn rows<S>((samples, stride): Plane<'_, S>, r: usize, width: usize) -> Option<[&[S]; 5]> {
    let mut rows = [&[][..]; 2 * R + 1];
    for (k, row) in rows.iter_mut().enumerate() {
        let start = (r + k) * stride;
        *row = samples.get(start..start + width + 2 * R)?;
    }
    Some(rows)
}

/// The luma [`Window`] of a filter without luma taps.
const NO_LUMA: Window<'static, u16> = [&[0; REACH]; 2 * R + 1];

/// The [`Window`] from column `c` of `rows`.
#[inline]
fn group<'a, S>(rows: &[&'a [S]; 5], c: usize) -> Option<Window<'a, S>> {
    let row = |k: usize| rows[k].get(c..)?.first_chunk::<REACH>();
    Some([row(0)?, row(1)?, row(2)?, row(3)?, row(4)?])
}

/// `filter_chroma_rows` over a block at least 16 samples wide, the last
/// group of a row overlapping the one before it; returns `false` when a
/// window or the output is short.
pub(super) fn filter_block<O: super::ChromaOutputSample, S: Sample>(
    t: Avx2,
    (output, output_stride): (&mut [O], usize),
    (width, height): (usize, usize),
    (chroma, luma): (Plane<'_, S>, Plane<'_, u16>),
    (pairs, reads_luma): (&PairVectors, bool),
    max_sample: u16,
) -> bool {
    if width < 16 {
        return false;
    }
    for r in 0..height {
        let start = r * output_stride;
        let (Some(target), Some(chroma_rows)) =
            (output.get_mut(start..start + width), rows(chroma, r, width))
        else {
            return false;
        };
        let Some(luma_rows) = (if reads_luma {
            rows(luma, r, width)
        } else {
            Some([&[][..]; 5])
        }) else {
            return false;
        };
        let mut c = 0;
        while c < width {
            c = c.min(width - 16);
            let luma = if reads_luma {
                group(&luma_rows, c)
            } else {
                Some(NO_LUMA)
            };
            let (Some(chroma), Some(luma), Some(slot)) = (
                group(&chroma_rows, c),
                luma,
                target.get_mut(c..).and_then(<[O]>::first_chunk_mut::<16>),
            ) else {
                return false;
            };
            let values = S::filter16(t, &chroma, &luma, pairs, reads_luma, max_sample);
            O::write_lanes::<16>(slot, values);
            c += 16;
        }
    }
    true
}

#[inline]
fn at<'a, S>(window: &Window<'a, S>, dy: isize, dx: isize) -> Option<&'a [S; 16]> {
    window[R.wrapping_add_signed(dy)][R.wrapping_add_signed(dx)..].first_chunk::<16>()
}

/// The three chroma and six luma coefficient pairs as `pmaddwd` weights.
pub(super) struct PairVectors {
    weights: [__m256i; 9],
}

fearless_simd::kernel!(
    #[inline]
    pub(super) fn pair_vectors(t: Avx2, coeffs: &[i16; WIENER_NS_CHROMA_COEFFS]) -> PairVectors {
        let _ = t;
        let words: [i32; 9] = core::array::from_fn(|pair| {
            let [low, high] = match pair {
                0..3 => [
                    coeffs[WIENER_NS_CONFIG_UV_PAIRS[2 * pair].2],
                    coeffs[WIENER_NS_CONFIG_UV_PAIRS[2 * pair + 1].2],
                ],
                _ => [
                    coeffs[2 * (pair - 3) + WIENER_NS_CHROMA_COEFF_SLOTS],
                    coeffs[2 * (pair - 3) + 1 + WIENER_NS_CHROMA_COEFF_SLOTS],
                ],
            };
            i32::from(low as u16) | (i32::from(high) << 16)
        });
        PairVectors {
            weights: words.map(|word| _mm256_set1_epi32(word)),
        }
    }
);

/// The § 7.20.3 chroma sum over tap loaders `chroma(dy, dx)` and
/// `luma(dy, dx)` that return 16 lanes, with the in-lane unpack the final
/// saturating pack restores.
macro_rules! chroma_lanes {
    ($chroma:expr, $luma:expr, $pairs:expr, $reads_luma:expr, $max_sample:expr) => {{
        let (chroma, luma, pairs) = ($chroma, $luma, $pairs);
        let mut sums = [_mm256_setzero_si256(); 2];
        let mut add = |a: __m256i, b: __m256i, weights: __m256i| {
            sums[0] = _mm256_add_epi32(
                sums[0],
                _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights),
            );
            sums[1] = _mm256_add_epi32(
                sums[1],
                _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights),
            );
        };
        let center = chroma(0, 0);
        add(
            center,
            _mm256_set1_epi16(1),
            _mm256_set1_epi32((1 << WIENER_NS_PREC_BITS) | (1 << (WIENER_NS_PREC_BITS - 1 + 16))),
        );
        let twice_center = _mm256_add_epi16(center, center);
        let diff = |j: usize| {
            let (dy, dx, _) = WIENER_NS_CONFIG_UV_PAIRS[j];
            _mm256_sub_epi16(
                _mm256_add_epi16(chroma(dy, dx), chroma(-dy, -dx)),
                twice_center,
            )
        };
        for (pair, &weights) in pairs.weights[..3].iter().enumerate() {
            add(diff(2 * pair), diff(2 * pair + 1), weights);
        }
        if $reads_luma {
            let luma_center = luma(0, 0);
            let diff = |j: usize| {
                let (dy, dx, _) = WIENER_NS_CONFIG_UV[j];
                _mm256_sub_epi16(luma(dy, dx), luma_center)
            };
            for (pair, &weights) in pairs.weights[3..].iter().enumerate() {
                add(diff(2 * pair), diff(2 * pair + 1), weights);
            }
        }
        _mm256_min_epu16(
            _mm256_packus_epi32(
                _mm256_srai_epi32::<{ WIENER_NS_PREC_BITS as i32 }>(sums[0]),
                _mm256_srai_epi32::<{ WIENER_NS_PREC_BITS as i32 }>(sums[1]),
            ),
            _mm256_set1_epi16($max_sample as i16),
        )
    }};
}

/// The padded rows of one 8-sample group, as [`Window`] for 8 lanes.
pub(super) type HalfWindow<'a, S> = [&'a [S; HALF_REACH]; 2 * R + 1];
const HALF_REACH: usize = 8 + 2 * R;

/// The luma [`HalfWindow`] of a filter without luma taps.
pub(super) const NO_HALF_LUMA: HalfWindow<'static, u16> = [&[0; HALF_REACH]; 2 * R + 1];

/// The [`HalfWindow`] whose first sample is `samples[base]`.
#[inline]
pub(super) fn half_window<S>(
    samples: &[S],
    base: usize,
    stride: usize,
) -> Option<HalfWindow<'_, S>> {
    let row = |dy: usize| {
        samples
            .get(base + dy * stride..)?
            .first_chunk::<HALF_REACH>()
    };
    Some([row(0)?, row(1)?, row(2)?, row(3)?, row(4)?])
}

#[inline]
fn half_at<'a, S>(window: &HalfWindow<'a, S>, dy: isize, dx: isize) -> Option<&'a [S; 8]> {
    window[R.wrapping_add_signed(dy)][R.wrapping_add_signed(dx)..].first_chunk::<8>()
}

/// The chroma sample types of the group kernels.
pub(super) trait Sample: Copy {
    /// 16 samples as word lanes.
    fn words16(t: Avx2, lanes: &[Self; 16]) -> __m256i;
    /// 8 samples of each of two rows as word lanes, `low` first.
    fn words8x2(t: Avx2, low: &[Self; 8], high: &[Self; 8]) -> __m256i;
    /// One 16-sample group of one row of `filter_chroma_rows`.
    fn filter16(
        t: Avx2,
        chroma: &Window<'_, Self>,
        luma: &Window<'_, u16>,
        pairs: &PairVectors,
        reads_luma: bool,
        max_sample: u16,
    ) -> [u16; 16];
    /// One 8-sample group of two rows of `filter_chroma_rows`, the first
    /// row in the low lane halves and the second in the high ones.
    fn filter8x2(
        t: Avx2,
        chroma: [&HalfWindow<'_, Self>; 2],
        luma: [&HalfWindow<'_, u16>; 2],
        pairs: &PairVectors,
        reads_luma: bool,
        max_sample: u16,
    ) -> [u16; 16];
}

/// Implements [`Sample`] for `$sample` with its word loads.
macro_rules! sample_kernels {
    ($sample:ty, |$t:ident, $lanes:ident| $words16:block, |$u:ident, $low:ident, $high:ident| $words8x2:block) => {
        impl Sample for $sample {
            #[allow(clippy::inline_always, reason = "one load per tap")]
            #[inline(always)]
            fn words16(t: Avx2, lanes: &[Self; 16]) -> __m256i {
                fearless_simd::kernel!(
                    #[inline(always)]
                    fn kernel($t: Avx2, $lanes: &[$sample; 16]) -> __m256i {
                        $words16
                    }
                );
                kernel(t, lanes)
            }

            #[allow(clippy::inline_always, reason = "one load per tap")]
            #[inline(always)]
            fn words8x2(t: Avx2, low: &[Self; 8], high: &[Self; 8]) -> __m256i {
                fearless_simd::kernel!(
                    #[inline(always)]
                    fn kernel($u: Avx2, $low: &[$sample; 8], $high: &[$sample; 8]) -> __m256i {
                        $words8x2
                    }
                );
                kernel(t, low, high)
            }

            #[allow(
                clippy::inline_always,
                reason = "the group kernel inlines into its row loop"
            )]
            #[inline(always)]
            fn filter16(
                t: Avx2,
                chroma: &Window<'_, Self>,
                luma: &Window<'_, u16>,
                pairs: &PairVectors,
                reads_luma: bool,
                max_sample: u16,
            ) -> [u16; 16] {
                fearless_simd::kernel!(
                    #[inline]
                    fn kernel(
                        t: Avx2,
                        chroma: &Window<'_, $sample>,
                        luma: &Window<'_, u16>,
                        pairs: &PairVectors,
                        reads_luma: bool,
                        max_sample: u16,
                    ) -> [u16; 16] {
                        let zero = _mm256_setzero_si256();
                        let chroma = |dy, dx| {
                            at(chroma, dy, dx).map_or(zero, |lanes| <$sample>::words16(t, lanes))
                        };
                        let luma =
                            |dy, dx| at(luma, dy, dx).map_or(zero, |lanes| u16::words16(t, lanes));
                        let values = chroma_lanes!(chroma, luma, pairs, reads_luma, max_sample);
                        let values: u16x16<Avx2> = values.simd_into(t);
                        values.into()
                    }
                );
                kernel(t, chroma, luma, pairs, reads_luma, max_sample)
            }

            #[allow(
                clippy::inline_always,
                reason = "the group kernel inlines into its row loop"
            )]
            #[inline(always)]
            fn filter8x2(
                t: Avx2,
                chroma: [&HalfWindow<'_, Self>; 2],
                luma: [&HalfWindow<'_, u16>; 2],
                pairs: &PairVectors,
                reads_luma: bool,
                max_sample: u16,
            ) -> [u16; 16] {
                fearless_simd::kernel!(
                    #[inline]
                    fn kernel(
                        t: Avx2,
                        chroma: [&HalfWindow<'_, $sample>; 2],
                        luma: [&HalfWindow<'_, u16>; 2],
                        pairs: &PairVectors,
                        reads_luma: bool,
                        max_sample: u16,
                    ) -> [u16; 16] {
                        let chroma =
                            |dy, dx| match (half_at(chroma[0], dy, dx), half_at(chroma[1], dy, dx))
                            {
                                (Some(low), Some(high)) => <$sample>::words8x2(t, low, high),
                                _ => _mm256_setzero_si256(),
                            };
                        let luma =
                            |dy, dx| match (half_at(luma[0], dy, dx), half_at(luma[1], dy, dx)) {
                                (Some(low), Some(high)) => u16::words8x2(t, low, high),
                                _ => _mm256_setzero_si256(),
                            };
                        let values = chroma_lanes!(chroma, luma, pairs, reads_luma, max_sample);
                        let values: u16x16<Avx2> = values.simd_into(t);
                        values.into()
                    }
                );
                kernel(t, chroma, luma, pairs, reads_luma, max_sample)
            }
        }
    };
}

sample_kernels!(
    u16,
    |t, lanes| { u16x16::load_array_ref(t, lanes).into() },
    |t, low, high| {
        _mm256_set_m128i(
            u16x8::load_array_ref(t, high).into(),
            u16x8::load_array_ref(t, low).into(),
        )
    }
);
sample_kernels!(
    u8,
    |t, lanes| { _mm256_cvtepu8_epi16(u8x16::load_array_ref(t, lanes).into()) },
    |t, low, high| {
        let _ = t;
        _mm256_cvtepu8_epi16(_mm_set_epi64x(
            i64::from_le_bytes(*high),
            i64::from_le_bytes(*low),
        ))
    }
);
