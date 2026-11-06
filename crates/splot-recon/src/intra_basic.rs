// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Basic/PAETH intra prediction primitive.
//!
//! Feature tracking: `RECON-INTRA-BASIC-PAETH-PREDICTION`.

use fearless_simd::{Bytes as _, Level, Select as _, Simd, SimdBase, dispatch, i16x16};

use crate::intra_dc_math::{
    IntraEdgeError, IntraLaneSample, validate_intra_edge_samples, validate_output_shape,
    validate_sample_type,
};
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

/// Edge identifier for PAETH intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraPaethEdge {
    /// Left edge samples.
    Left,
    /// Above edge samples.
    Above,
    /// Top-left corner sample.
    TopLeft,
}

impl IntraPaethEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
            Self::TopLeft => "top-left",
        }
    }
}

impl IntraEdgeError for IntraPaethEdge {
    fn length_mismatch(self, expected: usize, actual: usize) -> ReconError {
        ReconError::IntraPaethEdgeLengthMismatch {
            edge: self,
            expected,
            actual,
        }
    }

    fn sample_out_of_range(self, sample_index: usize, value: u16, max: u16) -> ReconError {
        ReconError::IntraPaethSampleOutOfRange {
            edge: self,
            sample_index,
            value,
            max,
        }
    }
}

/// Caller-provided prepared edge samples for AV2 §7.13.2.2 basic prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraPaethEdges<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
    top_left: T,
}

impl<'a, T: ReconSample> IntraPaethEdges<'a, T> {
    /// Creates a prepared PAETH edge set.
    ///
    /// `top_left` is the sample identified by AV2 §7.13.2.2 as
    /// `AboveRow[-1]`. Availability and fallback preparation remain outside
    /// this type and are owned by the broader §7.13.2.1 intra process.
    pub const fn new(left: &'a [T], above: &'a [T], top_left: T) -> Self {
        Self {
            left,
            above,
            top_left,
        }
    }

    /// Returns prepared left edge samples.
    pub const fn left_samples(self) -> &'a [T] {
        self.left
    }

    /// Returns prepared above edge samples.
    pub const fn above_samples(self) -> &'a [T] {
        self.above
    }
}

/// Writes rectangular AV2 §7.13.2.2 basic/PAETH prediction into caller storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples, a too-small stride, or a
/// too-small output buffer.
pub fn predict_intra_paeth_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraPaethEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_intra_edge_samples(IntraPaethEdge::Left, edges.left, size.height(), bit_depth)?;
    validate_intra_edge_samples(IntraPaethEdge::Above, edges.above, size.width(), bit_depth)?;
    validate_intra_edge_samples(
        IntraPaethEdge::TopLeft,
        core::slice::from_ref(&edges.top_left),
        1,
        bit_depth,
    )?;
    validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "PAETH intra prediction output buffer length",
    )?;
    if let (Some(left), Some(above), Some(output)) = (
        T::u16_slice(edges.left),
        T::u16_slice(edges.above),
        T::u16_slice_mut(output),
    ) {
        predict_paeth_lanes(
            size,
            left,
            above,
            edges.top_left.to_u16(),
            output,
            stride_samples,
        );
        return Ok(());
    }
    if let (Some(left), Some(above), Some(output)) = (
        T::u8_slice(edges.left),
        T::u8_slice(edges.above),
        T::u8_slice_mut(output),
    ) {
        predict_paeth_lanes(
            size,
            left,
            above,
            edges.top_left.to_u16(),
            output,
            stride_samples,
        );
        return Ok(());
    }

    for row_index in 0..size.height() {
        let row_start =
            row_index
                .checked_mul(stride_samples)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "PAETH intra prediction output row offset",
                })?;
        let left = edges.left[row_index];
        for column in 0..size.width() {
            output[row_start + column] =
                predict_paeth_sample(left, edges.above[column], edges.top_left);
        }
    }

    Ok(())
}

/// Writes the § 7.13.2.2 PAETH prediction in 16 `i16` lanes: 16 columns of
/// a row, or `16 / w` rows of a narrower block.
///
/// With `base = above + left - topLeft`, `pLeft = |above - topLeft|`,
/// `pTop = |left - topLeft|` and `pTopLeft = |above + left - 2 * topLeft|`.
/// The validated samples are at most 10 bits, so every term fits `i16`.
fn predict_paeth_lanes<O: IntraLaneSample>(
    size: IntraRectBlockSize,
    left: &[O],
    above: &[O],
    top_left: u16,
    output: &mut [O],
    stride_samples: usize,
) {
    dispatch!(Level::baseline(), simd => paeth_rows(simd, size, left, above, top_left, output, stride_samples));
}

#[inline]
fn paeth_rows<S: Simd, O: IntraLaneSample>(
    simd: S,
    size: IntraRectBlockSize,
    left: &[O],
    above: &[O],
    top_left: u16,
    output: &mut [O],
    stride_samples: usize,
) {
    let (width, height) = (size.width(), size.height());
    let top_left = i16x16::splat(simd, top_left as i16);
    let lane = |value: O| value.to_u16() as i16;
    if width < 16 {
        let above = i16x16::from_fn(simd, |j| lane(above[j % width]));
        let rows = 16 / width;
        for row in (0..height).step_by(rows) {
            let left = i16x16::from_fn(simd, |j| lane(left[row + j / width]));
            let mut values = [O::default(); 16];
            O::store16(
                simd,
                paeth_lanes(above, left, top_left).bitcast(),
                &mut values,
            );
            for (r, samples) in values.chunks_exact(width).enumerate() {
                let start = (row + r) * stride_samples;
                output[start..start + width].copy_from_slice(samples); // splot-copy-ok: publish one PAETH row
            }
        }
        return;
    }
    for (row, &left) in left.iter().take(height).enumerate() {
        let left = i16x16::splat(simd, lane(left));
        let target = &mut output[row * stride_samples..row * stride_samples + width];
        for (target, above) in target
            .as_chunks_mut::<16>()
            .0
            .iter_mut()
            .zip(above.as_chunks::<16>().0)
        {
            let above = O::load16(simd, above).bitcast::<i16x16<S>>();
            O::store16(simd, paeth_lanes(above, left, top_left).bitcast(), target);
        }
    }
}

/// One lane group of [`predict_paeth_lanes`].
#[inline]
fn paeth_lanes<S: Simd>(above: i16x16<S>, left: i16x16<S>, top_left: i16x16<S>) -> i16x16<S> {
    let p_left = (above - top_left).abs();
    let p_top = (left - top_left).abs();
    let p_top_left = (above + left - top_left - top_left).abs();
    let take_left = p_left.simd_le(p_top) & p_left.simd_le(p_top_left);
    let take_top = p_top.simd_le(p_top_left);
    take_left.select(left, take_top.select(above, top_left))
}

pub(crate) fn predict_paeth_sample<T: ReconSample>(left: T, above: T, top_left: T) -> T {
    let left_value = i32::from(left.to_u16());
    let above_value = i32::from(above.to_u16());
    let top_left_value = i32::from(top_left.to_u16());
    let base = above_value + left_value - top_left_value;

    let p_left = base.abs_diff(left_value);
    let p_top = base.abs_diff(above_value);
    let p_top_left = base.abs_diff(top_left_value);

    if p_left <= p_top && p_left <= p_top_left {
        left
    } else if p_top <= p_top_left {
        above
    } else {
        top_left
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn assert_paeth_fills(left: [u8; 8], above: [u8; 4], top_left: u8, expected: u8) {
        let mut output = [0u8; 32];

        predict_intra_paeth_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraPaethEdges::new(&left, &above, top_left),
            &mut output,
            4,
        )
        .unwrap();

        assert_eq!(output, [expected; 32]);
    }

    #[test]
    fn paeth_prediction_selects_left_candidate() {
        assert_paeth_fills([30; 8], [12; 4], 10, 30);
    }

    #[test]
    fn paeth_prediction_selects_above_candidate() {
        assert_paeth_fills([12; 8], [30; 4], 10, 30);
    }

    #[test]
    fn paeth_prediction_selects_top_left_candidate() {
        assert_paeth_fills([0; 8], [20; 4], 10, 10);
    }

    #[test]
    fn paeth_prediction_tie_order_is_left_then_above() {
        assert_eq!(predict_paeth_sample(10u8, 25, 20), 10);
        assert_eq!(predict_paeth_sample(25u8, 10, 20), 10);
    }

    #[test]
    fn paeth_prediction_fills_rectangle_with_stride() {
        let left = [30u8; 8];
        let above = [12u8; 4];
        let mut output = [99u8; 48];

        predict_intra_paeth_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraPaethEdges::new(&left, &above, 10),
            &mut output,
            6,
        )
        .unwrap();

        for row in 0..8 {
            let start = row * 6;
            assert_eq!(&output[start..start + 6], &[30, 30, 30, 30, 99, 99]);
        }
    }

    #[test]
    fn paeth_prediction_validates_edge_lengths_by_dimension() {
        let left = [1u8; 4];
        let above = [1u8; 8];
        let mut output = [0u8; 32];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&left, &[1; 4], 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethEdgeLengthMismatch {
                edge: IntraPaethEdge::Left,
                expected: 8,
                actual: 4
            })
        ));
        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1; 8], &above, 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethEdgeLengthMismatch {
                edge: IntraPaethEdge::Above,
                expected: 4,
                actual: 8
            })
        ));
    }

    #[test]
    fn paeth_prediction_validates_edge_samples_against_bit_depth() {
        let mut output = [7u16; 32];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1; 8], &[1, 2, 256, 4], 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethSampleOutOfRange {
                edge: IntraPaethEdge::Above,
                sample_index: 2,
                value: 256,
                max: 255
            })
        ));
        assert_eq!(output, [7u16; 32]);

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1; 8], &[1; 4], 256),
                &mut output,
                4
            ),
            Err(ReconError::IntraPaethSampleOutOfRange {
                edge: IntraPaethEdge::TopLeft,
                sample_index: 0,
                value: 256,
                max: 255
            })
        ));
    }

    #[test]
    fn paeth_prediction_validates_sample_type_against_bit_depth() {
        let mut output = [0u8; 32];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Ten,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1u8; 8], &[1u8; 4], 1),
                &mut output,
                4
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn paeth_prediction_validates_output_stride_and_length() {
        let left = [1u8; 8];
        let above = [1u8; 4];
        let mut output = [0u8; 31];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&left, &above, 1),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&left, &above, 1),
                &mut output,
                4
            ),
            Err(ReconError::IntraPredictionOutputTooSmall {
                expected: 32,
                actual: 31
            })
        ));
    }

    #[test]
    fn paeth_prediction_rejects_overflowing_output_shape() {
        let mut output = [];

        assert!(matches!(
            predict_intra_paeth_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraPaethEdges::new(&[1u8; 8], &[1u8; 4], 1),
                &mut output,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "PAETH intra prediction output buffer length"
            })
        ));
    }

    /// The lane path must write `predict_paeth_sample` for every block shape,
    /// both storage types and the sample extremes, and leave the stride gap.
    #[test]
    fn paeth_lanes_match_the_per_sample_choice() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let max = u64::from(bit_depth.max_sample());
            let mut sample = || match next() % 4 {
                0 => 0,
                1 => max as u16,
                _ => (next() % (max + 1)) as u16,
            };
            for log2_width in 2..=6u8 {
                for log2_height in 2..=6u8 {
                    let size = rect_size(log2_width, log2_height);
                    let left: Vec<u16> = (0..size.height()).map(|_| sample()).collect();
                    let above: Vec<u16> = (0..size.width()).map(|_| sample()).collect();
                    let top_left = sample();
                    let stride = size.width() + 3;
                    let mut wide = vec![7u16; stride * size.height()];
                    let edges = IntraPaethEdges::new(&left[..], &above[..], top_left);
                    predict_intra_paeth_rect_into(bit_depth, size, edges, &mut wide, stride)
                        .unwrap();
                    for (index, &got) in wide.iter().enumerate() {
                        let (row, column) = (index / stride, index % stride);
                        let expected = if column < size.width() {
                            predict_paeth_sample(left[row], above[column], top_left)
                        } else {
                            7
                        };
                        assert_eq!(got, expected, "{bit_depth:?} at ({row}, {column})");
                    }
                    if bit_depth == BitDepth::Eight {
                        let narrow = |v: &[u16]| v.iter().map(|&s| s as u8).collect::<Vec<_>>();
                        let mut out = vec![7u8; wide.len()];
                        let (left, above) = (narrow(&left), narrow(&above));
                        let edges = IntraPaethEdges::new(&left[..], &above[..], top_left as u8);
                        predict_intra_paeth_rect_into(bit_depth, size, edges, &mut out, stride)
                            .unwrap();
                        assert_eq!(out, narrow(&wide));
                    }
                }
            }
        }
    }
}
