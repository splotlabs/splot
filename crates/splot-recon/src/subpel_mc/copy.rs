// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

/// Blends two § 7.13.3.18 compound intermediate predictors with § 7.13.3.16
/// COMPOUND_AVERAGE and the supplied `cwpWeight`, then applies the final § 4.8
/// `Clip1`.
///
/// # Errors
///
/// Returns [`ReconError::CompoundBlendLengthMismatch`] when `pred0` and `pred1`
/// have different lengths.
pub fn blend_compound_average_weighted(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: BitDepth,
    cwp_weight: i16,
) -> Result<Vec<u16>> {
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }

    Ok(pred0
        .iter()
        .zip(pred1.iter())
        .map(|(&left, &right)| {
            blend_compound_average_weighted_sample(left, right, bit_depth, cwp_weight)
        })
        .collect())
}

/// Blends one pair of § 7.13.3.18 compound intermediate samples with the
/// supplied § 7.13.3.16 `cwpWeight`, then applies the final § 4.8 `Clip1`.
#[inline]
pub fn blend_compound_average_weighted_sample(
    pred0: i32,
    pred1: i32,
    bit_depth: BitDepth,
    cwp_weight: i16,
) -> u16 {
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    let blended = round2_i32(
        forward * pred0 + backward * pred1,
        4 + compound_inter_post_round(),
    );
    blended.clamp(0, i32::from(bit_depth.max_sample())) as u16
}

/// Blends two § 7.13.3.18 compound intermediate predictors with § 7.13.3.16
/// COMPOUND_AVERAGE and `CWP_EQUAL` (`cwpWeight == 8`), then applies the final
/// § 4.8 `Clip1`.
///
/// With `InterRound0 == 3`, compound `InterRound1 == 7`, and `FILTER_BITS == 7`,
/// `InterPostRound == 4`; the equal-weight formula
/// `Round2(8 * p0 + 8 * p1, 4 + InterPostRound)` simplifies exactly to
/// `Round2(p0 + p1, 1 + InterPostRound)`.
///
/// # Errors
///
/// Returns [`ReconError::CompoundBlendLengthMismatch`] when `pred0` and `pred1`
/// have different lengths.
pub fn blend_compound_average_equal(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: BitDepth,
) -> Result<Vec<u16>> {
    blend_compound_average_weighted(pred0, pred1, bit_depth, 8)
}

pub(super) const fn compound_inter_post_round() -> u32 {
    2 * FILTER_BITS - (INTER_ROUND0 + INTER_ROUND1_COMPOUND)
}

impl<T: ReconSample> ReferencePlaneView<'_, T> {
    /// Returns the `width()` samples of row `row`, or `None` when the row is
    /// past the published prefix a § 7.13.3.18 read may use.
    #[must_use]
    pub fn readable_row(&self, row: usize) -> Option<&[T]> {
        if row >= self.readable_rows {
            return None;
        }
        self.samples.get(row * self.stride..)?.get(..self.width)
    }

    /// The `len` samples from column `x` of each of `rows`, or `None` when a
    /// row is past the published prefix or the span leaves the row.
    #[must_use]
    pub fn readable_spans<const N: usize>(
        &self,
        rows: [usize; N],
        x: usize,
        len: usize,
    ) -> Option<[&[T]; N]> {
        if x.checked_add(len)? > self.width {
            return None;
        }
        let mut spans = [&self.samples[..0]; N];
        for (span, row) in spans.iter_mut().zip(rows) {
            if row >= self.readable_rows {
                return None;
            }
            let start = row * self.stride + x;
            *span = self.samples.get(start..start + len)?;
        }
        Some(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subpel_mc::tests::full_pel_params;

    #[test]
    fn readable_row_stops_at_the_published_prefix() -> Result<()> {
        let samples: Vec<u16> = (0..40).collect();
        let view = ReferencePlaneView::from_published_strided(&samples, 10, 8, 4, 3)?;
        assert_eq!(view.readable_row(0), Some(&samples[..8]));
        assert_eq!(view.readable_row(2), Some(&samples[20..28]));
        assert_eq!(view.readable_row(3), None);
        assert_eq!(view.readable_row(usize::MAX), None);
        let spans = view.readable_spans([2, 0, 2], 3, 5);
        assert_eq!(
            spans,
            Some([&samples[23..28], &samples[3..8], &samples[23..28]])
        );
        assert_eq!(view.readable_spans([0, 3], 3, 5), None);
        assert_eq!(view.readable_spans([0, 1], 4, 5), None);
        assert_eq!(view.readable_spans([0], usize::MAX, 2), None);
        Ok(())
    }

    /// Fullpel copies and the `BILINEAR` kernels of clipped blocks against the
    /// general convolution, which clips every read on its own.
    fn check_clipped_blocks<T: ReconSample>(bit_depth: BitDepth) -> Result<()> {
        let (width, height) = (40usize, 12usize);
        let max = u32::from(bit_depth.max_sample());
        let samples = (0..width * height)
            .map(|i| T::try_from_u16(((i as u32 * 7919 + 13) % (max + 1)) as u16))
            .collect::<Result<Vec<T>>>()?;
        let view = ReferencePlaneView::new(&samples, width, height)?;
        for w in [4, 6, 8, 12, 16, 24, 32] {
            for x0 in -3..(width as i32 - w as i32 + 3) {
                for (first, last) in [(x0 + 1, x0 + w as i32 - 1), (x0 + 5, x0 + 6), (0, 99)] {
                    for (phase_x, phase_y) in [(0, 0), (5, 0), (0, 11), (7, 9)] {
                        let mut params = full_pel_params(
                            InterpolationFilter::Bilinear,
                            w,
                            3,
                            x0,
                            2,
                            width as i32,
                            height as i32,
                        );
                        params.bit_depth = bit_depth;
                        params.start_x += phase_x << 6;
                        params.start_y += phase_y << 6;
                        (params.first_x, params.last_x) = (first.max(0), last.max(0));
                        let mut predicted = vec![0; w * 3];
                        subpel_predict_block_into(&view, &params, &mut predicted)?;
                        let expected = subpel_predict_block(&view, &params)?;
                        assert_eq!(predicted, expected, "{params:?}");
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn inverted_clip_ranges_are_errors() -> Result<()> {
        let samples = vec![100u8; 32 * 32];
        let view = ReferencePlaneView::new(&samples, 32, 32)?;
        for (vertical, phase_x, phase_y) in [(true, 0, 5), (false, 5, 0), (true, 5, 9)] {
            let mut params =
                full_pel_params(InterpolationFilter::EightTapSharp, 8, 8, 8, 8, 32, 32);
            params.start_x += phase_x << 6;
            params.start_y += phase_y << 6;
            if vertical {
                (params.first_y, params.last_y) = (20, 10);
            } else {
                (params.first_x, params.last_x) = (20, 10);
            }
            let mut predicted = vec![0u16; 64];
            assert!(matches!(
                subpel_predict_block_into(&view, &params, &mut predicted),
                Err(ReconError::SubpelReferenceBoundsInvalid { .. })
            ));
            assert!(matches!(
                subpel_predict_block(&view, &params),
                Err(ReconError::SubpelReferenceBoundsInvalid { .. })
            ));
        }
        Ok(())
    }

    #[test]
    fn clipped_fullpel_and_bilinear_blocks_match_the_general_convolution() -> Result<()> {
        check_clipped_blocks::<u16>(BitDepth::Ten)?;
        check_clipped_blocks::<u8>(BitDepth::Eight)
    }
}
