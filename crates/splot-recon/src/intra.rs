// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Scalar intra prediction primitives.
//!
//! Feature tracking: `RECON-INTRA-DC-SQUARE-PREDICTION`,
//! `RECON-INTRA-DC-RECTANGULAR-PREDICTION`.

use crate::intra_dc_math::{
    fill_validated_output_shape, predict_intra_dc_rect_value_from_sums, validate_dc_edge,
    validate_output_shape, validate_sample_type,
};
use crate::{BitDepth, ReconError, ReconSample, Result};

const MIN_SQUARE_BLOCK_LOG2: u8 = 2;
const MAX_SQUARE_BLOCK_LOG2: u8 = 6;
const MIN_RECT_BLOCK_LOG2: u8 = 2;
const MAX_RECT_BLOCK_LOG2: u8 = 6;

/// Square transform-block size supported by the first DC intra predictor.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntraSquareBlockSize {
    log2_size: u8,
    side_len: usize,
    sample_count: usize,
}

impl IntraSquareBlockSize {
    /// Creates a square block size from AV2 transform-size log2 dimensions.
    ///
    /// # Errors
    /// Returns [`ReconError::InvalidIntraSquareBlockLog2`] for values outside
    /// the source-backed square transform-size range 4x4 through 64x64.
    pub const fn new(log2_size: u8) -> Result<Self> {
        if log2_size < MIN_SQUARE_BLOCK_LOG2 || log2_size > MAX_SQUARE_BLOCK_LOG2 {
            return Err(ReconError::InvalidIntraSquareBlockLog2 {
                log2_size,
                min: MIN_SQUARE_BLOCK_LOG2,
                max: MAX_SQUARE_BLOCK_LOG2,
            });
        }

        let side_len = 1usize << log2_size;
        let sample_count = side_len * side_len;
        Ok(Self {
            log2_size,
            side_len,
            sample_count,
        })
    }

    /// Returns `log2(width) == log2(height)`.
    pub const fn log2_size(self) -> u8 {
        self.log2_size
    }

    /// Returns the square block width and height in samples.
    pub const fn side_len(self) -> usize {
        self.side_len
    }

    /// Returns the number of samples in the square block.
    pub const fn sample_count(self) -> usize {
        self.sample_count
    }
}

/// Rectangular transform-block size supported by DC intra prediction.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntraRectBlockSize {
    log2_width: u8,
    log2_height: u8,
    width: usize,
    height: usize,
    sample_count: usize,
}

impl IntraRectBlockSize {
    /// Creates a rectangular block size from AV2 transform-size log2 dimensions.
    ///
    /// # Errors
    /// Returns [`ReconError::InvalidIntraRectBlockLog2`] for values outside the
    /// source-backed transform-size dimension range 4 through 64 samples.
    pub const fn new(log2_width: u8, log2_height: u8) -> Result<Self> {
        if log2_width < MIN_RECT_BLOCK_LOG2
            || log2_width > MAX_RECT_BLOCK_LOG2
            || log2_height < MIN_RECT_BLOCK_LOG2
            || log2_height > MAX_RECT_BLOCK_LOG2
        {
            return Err(ReconError::InvalidIntraRectBlockLog2 {
                log2_width,
                log2_height,
                min: MIN_RECT_BLOCK_LOG2,
                max: MAX_RECT_BLOCK_LOG2,
            });
        }

        let width = 1usize << log2_width;
        let height = 1usize << log2_height;
        let sample_count = width * height;
        Ok(Self {
            log2_width,
            log2_height,
            width,
            height,
            sample_count,
        })
    }

    /// Returns `log2(width)`.
    pub const fn log2_width(self) -> u8 {
        self.log2_width
    }

    /// Returns `log2(height)`.
    pub const fn log2_height(self) -> u8 {
        self.log2_height
    }

    /// Returns the block width in samples.
    pub const fn width(self) -> usize {
        self.width
    }

    /// Returns the block height in samples.
    pub const fn height(self) -> usize {
        self.height
    }

    /// Returns the number of samples in the rectangular block.
    pub const fn sample_count(self) -> usize {
        self.sample_count
    }
}

impl From<IntraSquareBlockSize> for IntraRectBlockSize {
    fn from(size: IntraSquareBlockSize) -> Self {
        Self {
            log2_width: size.log2_size,
            log2_height: size.log2_size,
            width: size.side_len,
            height: size.side_len,
            sample_count: size.sample_count,
        }
    }
}

/// Edge identifier for DC intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraDcEdge {
    /// Left edge samples.
    Left,
    /// Above edge samples.
    Above,
}

impl IntraDcEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
        }
    }
}

/// Caller-provided edge samples for DC intra prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraDcEdges<'a, T: ReconSample> {
    left: Option<&'a [T]>,
    above: Option<&'a [T]>,
}

impl<'a, T: ReconSample> IntraDcEdges<'a, T> {
    /// Creates an edge set from optional left and above edge samples.
    pub const fn new(left: Option<&'a [T]>, above: Option<&'a [T]>) -> Self {
        Self { left, above }
    }

    /// Creates an edge set with no available neighboring samples.
    pub const fn none() -> Self {
        Self::new(None, None)
    }

    /// Creates an edge set with only left samples available.
    pub const fn left(left: &'a [T]) -> Self {
        Self::new(Some(left), None)
    }

    /// Creates an edge set with only above samples available.
    pub const fn above(above: &'a [T]) -> Self {
        Self::new(None, Some(above))
    }

    /// Creates an edge set with both left and above samples available.
    pub const fn both(left: &'a [T], above: &'a [T]) -> Self {
        Self::new(Some(left), Some(above))
    }

    /// Returns left edge samples when available.
    pub const fn left_samples(self) -> Option<&'a [T]> {
        self.left
    }

    /// Returns above edge samples when available.
    pub const fn above_samples(self) -> Option<&'a [T]> {
        self.above
    }
}

/// Owned square prediction block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SquareIntraPredictionBlock<T: ReconSample> {
    size: IntraSquareBlockSize,
    samples: Vec<T>,
}

impl<T: ReconSample> SquareIntraPredictionBlock<T> {
    /// Creates a block filled with one predicted sample value.
    ///
    /// # Errors
    /// Returns [`ReconError::IntraPredictionAllocationFailed`] if the backing
    /// sample allocation cannot be reserved.
    pub fn filled(size: IntraSquareBlockSize, sample: T) -> Result<Self> {
        Self::filled_with_sample_count(size, sample, size.sample_count)
    }

    fn filled_with_sample_count(
        size: IntraSquareBlockSize,
        sample: T,
        sample_count: usize,
    ) -> Result<Self> {
        let mut samples = Vec::new();
        samples.try_reserve_exact(sample_count).map_err(|_| {
            ReconError::IntraPredictionAllocationFailed {
                context: "square DC prediction block samples",
            }
        })?;
        samples.resize(sample_count, sample);
        Ok(Self { size, samples })
    }

    /// Returns the block size.
    pub const fn size(&self) -> IntraSquareBlockSize {
        self.size
    }

    /// Returns the block width in samples.
    pub const fn width(&self) -> usize {
        self.size.side_len()
    }

    /// Returns the block height in samples.
    pub const fn height(&self) -> usize {
        self.size.side_len()
    }

    /// Returns the backing samples in row-major order.
    pub fn samples(&self) -> &[T] {
        &self.samples
    }

    /// Consumes the block and returns row-major samples.
    pub fn into_samples(self) -> Vec<T> {
        self.samples
    }

    /// Iterates over prediction rows.
    pub const fn rows(&self) -> SquareIntraPredictionRows<'_, T> {
        SquareIntraPredictionRows {
            block: self,
            next_row: 0,
        }
    }
}

/// Iterator over square intra prediction rows.
#[derive(Clone, Debug)]
pub struct SquareIntraPredictionRows<'a, T: ReconSample> {
    block: &'a SquareIntraPredictionBlock<T>,
    next_row: usize,
}

impl<'a, T: ReconSample> Iterator for SquareIntraPredictionRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_row >= self.block.height() {
            return None;
        }

        let width = self.block.width();
        let start = self.next_row * width;
        let end = start + width;
        self.next_row += 1;
        Some(&self.block.samples[start..end])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.block.height() - self.next_row;
        (remaining, Some(remaining))
    }
}

impl<T: ReconSample> ExactSizeIterator for SquareIntraPredictionRows<'_, T> {}

/// Computes the constant sample value for square AV2 §7.13.2.10 DC prediction.
///
/// Computes the scalar DC value without allocating an output block.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples, or storage conversion
/// failure.
pub fn predict_intra_dc_square_value<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraSquareBlockSize,
    edges: IntraDcEdges<'_, T>,
) -> Result<T> {
    predict_intra_dc_rect_value(bit_depth, size.into(), edges)
}

/// Computes the constant sample value for rectangular AV2 §7.13.2.10 DC prediction.
///
/// Computes the scalar DC value without allocating an output block. For rectangular
/// both-edge prediction, the average uses the AV2 §7.13.3.22
/// `resolve_divisor` / `approx_divide` path rather than ordinary division.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples, arithmetic overflow, or
/// storage conversion failure.
pub fn predict_intra_dc_rect_value<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDcEdges<'_, T>,
) -> Result<T> {
    validate_sample_type::<T>(bit_depth)?;
    let left_sum = validate_dc_edge(IntraDcEdge::Left, edges.left, size.height(), bit_depth)?;
    let above_sum = validate_dc_edge(IntraDcEdge::Above, edges.above, size.width(), bit_depth)?;

    predict_intra_dc_rect_value_from_sums(bit_depth, size, left_sum, above_sum)
}

/// Writes square AV2 §7.13.2.10 DC prediction into caller-owned storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// square are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for invalid prediction inputs, a too-small stride, a
/// too-small output buffer, or storage conversion failure.
pub fn predict_intra_dc_square_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraSquareBlockSize,
    edges: IntraDcEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    predict_intra_dc_rect_into(bit_depth, size.into(), edges, output, stride_samples)
}

/// Writes rectangular AV2 §7.13.2.10 DC prediction into caller-owned storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for invalid prediction inputs, a too-small stride, a
/// too-small output buffer, arithmetic overflow, or storage conversion failure.
pub fn predict_intra_dc_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDcEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let required = validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "intra prediction output buffer length",
    )?;
    let sample = predict_intra_dc_rect_value(bit_depth, size, edges)?;
    fill_validated_output_shape(size, output, stride_samples, required, sample);
    Ok(())
}

/// Predicts a square block with AV2 §7.13.2.10 DC intra prediction.
///
/// This models only the square subset where `log2W == log2H`. For the both-edge
/// case, `w + h` is therefore a power of two, so the §7.13.3.22
/// `resolve_divisor` path used by `approx_divide(sum, w + h)` specializes to
/// `Round2(sum, log2_size + 1)`.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples, or allocation failure.
pub fn predict_intra_dc_square<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraSquareBlockSize,
    edges: IntraDcEdges<'_, T>,
) -> Result<SquareIntraPredictionBlock<T>> {
    let sample = predict_intra_dc_square_value(bit_depth, size, edges)?;
    SquareIntraPredictionBlock::filled(size, sample)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn size(log2_size: u8) -> IntraSquareBlockSize {
        IntraSquareBlockSize::new(log2_size).unwrap()
    }

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    #[test]
    fn square_block_size_accepts_av2_square_transform_range() {
        let four = size(2);
        assert_eq!(four.log2_size(), 2);
        assert_eq!(four.side_len(), 4);
        assert_eq!(four.sample_count(), 16);
        let sixty_four = size(6);
        assert_eq!(sixty_four.side_len(), 64);
        assert_eq!(sixty_four.sample_count(), 4096);
    }

    #[test]
    fn square_block_size_rejects_out_of_range_log2_values() {
        assert!(matches!(
            IntraSquareBlockSize::new(1),
            Err(ReconError::InvalidIntraSquareBlockLog2 {
                log2_size: 1,
                min: 2,
                max: 6
            })
        ));
        assert!(IntraSquareBlockSize::new(7).is_err());
    }

    #[test]
    fn rect_block_size_accepts_av2_transform_dimension_range() {
        let four_by_eight = rect_size(2, 3);
        assert_eq!(four_by_eight.log2_width(), 2);
        assert_eq!(four_by_eight.log2_height(), 3);
        assert_eq!(four_by_eight.width(), 4);
        assert_eq!(four_by_eight.height(), 8);
        assert_eq!(four_by_eight.sample_count(), 32);

        let sixty_four_by_four = rect_size(6, 2);
        assert_eq!(sixty_four_by_four.width(), 64);
        assert_eq!(sixty_four_by_four.height(), 4);
        assert_eq!(sixty_four_by_four.sample_count(), 256);
    }

    #[test]
    fn rect_block_size_rejects_out_of_range_log2_values() {
        assert!(matches!(
            IntraRectBlockSize::new(1, 2),
            Err(ReconError::InvalidIntraRectBlockLog2 {
                log2_width: 1,
                log2_height: 2,
                min: 2,
                max: 6
            })
        ));
        assert!(IntraRectBlockSize::new(2, 7).is_err());
    }

    #[test]
    fn dc_prediction_with_no_edges_uses_midpoint_sample() {
        let block =
            predict_intra_dc_square::<u8>(BitDepth::Eight, size(2), IntraDcEdges::none()).unwrap();

        assert_eq!(block.width(), 4);
        assert_eq!(block.height(), 4);
        assert_eq!(block.samples(), &[128u8; 16]);
        assert_eq!(block.rows().collect::<Vec<_>>(), vec![&[128u8; 4][..]; 4]);
    }

    #[test]
    fn dc_prediction_value_avoids_output_allocation() {
        let left = [10u8, 20, 30, 40];
        let above = [50u8, 60, 70, 80];

        let sample = predict_intra_dc_square_value(
            BitDepth::Eight,
            size(2),
            IntraDcEdges::both(&left, &above),
        )
        .unwrap();

        assert_eq!(sample, 45);
    }

    #[test]
    fn rect_dc_prediction_no_edges_uses_midpoint_sample() {
        let sample = predict_intra_dc_rect_value::<u16>(
            BitDepth::Ten,
            rect_size(2, 3),
            IntraDcEdges::none(),
        )
        .unwrap();

        assert_eq!(sample, 512);
    }

    #[test]
    fn rect_dc_prediction_with_left_edge_uses_height_average() {
        let left = [1u8, 2, 3, 4, 5, 6, 7, 8];

        let sample = predict_intra_dc_rect_value(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraDcEdges::left(&left),
        )
        .unwrap();

        assert_eq!(sample, 5);
    }

    #[test]
    fn rect_dc_prediction_with_above_edge_uses_width_average() {
        let above = [100u16, 101, 102, 103];

        let sample = predict_intra_dc_rect_value(
            BitDepth::Ten,
            rect_size(2, 3),
            IntraDcEdges::above(&above),
        )
        .unwrap();

        assert_eq!(sample, 102);
    }

    #[test]
    fn rect_dc_prediction_with_both_edges_uses_av2_approximate_divide() {
        let left = [1u8, 1, 1, 1, 1, 1, 1, 0];
        let above = [0u8, 0, 0, 0];

        let sample = predict_intra_dc_rect_value(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraDcEdges::both(&left, &above),
        )
        .unwrap();

        let truncating_integer_division = 7u8 / 12;
        assert_eq!(sample, 1);
        assert_ne!(sample, truncating_integer_division);
    }

    #[test]
    fn rect_dc_prediction_into_fills_rectangle_with_stride() {
        let left = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut output = [99u8; 48];

        predict_intra_dc_rect_into(
            BitDepth::Eight,
            rect_size(2, 3),
            IntraDcEdges::left(&left),
            &mut output,
            6,
        )
        .unwrap();

        for row in 0..8 {
            let start = row * 6;
            assert_eq!(&output[start..start + 6], &[5, 5, 5, 5, 99, 99]);
        }
    }

    #[test]
    fn dc_prediction_into_fills_square_region_with_stride() {
        let left = [1u8, 2, 3, 4];
        let mut output = [99u8; 24];

        predict_intra_dc_square_into(
            BitDepth::Eight,
            size(2),
            IntraDcEdges::left(&left),
            &mut output,
            6,
        )
        .unwrap();

        assert_eq!(&output[0..6], &[3, 3, 3, 3, 99, 99]);
        assert_eq!(&output[6..12], &[3, 3, 3, 3, 99, 99]);
        assert_eq!(&output[12..18], &[3, 3, 3, 3, 99, 99]);
        assert_eq!(&output[18..24], &[3, 3, 3, 3, 99, 99]);
    }

    #[test]
    fn dc_prediction_with_left_edge_rounds_left_average() {
        let left = [1u8, 2, 3, 4];
        let block =
            predict_intra_dc_square(BitDepth::Eight, size(2), IntraDcEdges::left(&left)).unwrap();

        assert_eq!(block.samples(), &[3u8; 16]);
    }

    #[test]
    fn dc_prediction_with_above_edge_rounds_above_average() {
        let above = [100u16, 101, 102, 103];
        let block =
            predict_intra_dc_square(BitDepth::Ten, size(2), IntraDcEdges::above(&above)).unwrap();

        assert_eq!(block.samples(), &[102u16; 16]);
    }

    #[test]
    fn dc_prediction_with_both_edges_uses_square_power_of_two_average() {
        let left = [0u8, 1, 2, 3, 4, 5, 6, 7];
        let above = [8u8, 9, 10, 11, 12, 13, 14, 15];
        let block =
            predict_intra_dc_square(BitDepth::Eight, size(3), IntraDcEdges::both(&left, &above))
                .unwrap();

        assert_eq!(block.samples(), &[8u8; 64]);
    }

    #[test]
    fn square_dc_prediction_matches_rectangular_compatibility_path() {
        let left = [0u8, 1, 2, 3, 4, 5, 6, 7];
        let above = [8u8, 9, 10, 11, 12, 13, 14, 15];

        let square = predict_intra_dc_square_value(
            BitDepth::Eight,
            size(3),
            IntraDcEdges::both(&left, &above),
        )
        .unwrap();
        let rect = predict_intra_dc_rect_value(
            BitDepth::Eight,
            IntraRectBlockSize::from(size(3)),
            IntraDcEdges::both(&left, &above),
        )
        .unwrap();

        assert_eq!(square, rect);
        assert_eq!(rect, 8);
    }

    #[test]
    fn dc_prediction_validates_edge_lengths() {
        let left = [1u8, 2, 3];
        assert!(matches!(
            predict_intra_dc_square(BitDepth::Eight, size(2), IntraDcEdges::left(&left)),
            Err(ReconError::IntraPredictionEdgeLengthMismatch {
                edge: IntraDcEdge::Left,
                expected: 4,
                actual: 3
            })
        ));
    }

    #[test]
    fn rect_dc_prediction_validates_edge_lengths_by_dimension() {
        let left = [1u8, 2, 3, 4];
        let above = [1u8, 2, 3, 4, 5, 6, 7, 8];

        assert!(matches!(
            predict_intra_dc_rect_value(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraDcEdges::left(&left)
            ),
            Err(ReconError::IntraPredictionEdgeLengthMismatch {
                edge: IntraDcEdge::Left,
                expected: 8,
                actual: 4
            })
        ));
        assert!(matches!(
            predict_intra_dc_rect_value(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraDcEdges::above(&above)
            ),
            Err(ReconError::IntraPredictionEdgeLengthMismatch {
                edge: IntraDcEdge::Above,
                expected: 4,
                actual: 8
            })
        ));
    }

    #[test]
    fn dc_prediction_validates_sample_type_against_bit_depth() {
        let left = [1u8, 2, 3, 4];
        assert!(matches!(
            predict_intra_dc_square(BitDepth::Ten, size(2), IntraDcEdges::left(&left)),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
    }

    #[test]
    fn dc_prediction_validates_edge_samples_against_bit_depth() {
        let left = [256u16, 2, 3, 4];
        assert!(matches!(
            predict_intra_dc_square(BitDepth::Eight, size(2), IntraDcEdges::left(&left)),
            Err(ReconError::IntraPredictionSampleOutOfRange {
                edge: IntraDcEdge::Left,
                sample_index: 0,
                value: 256,
                max: 255
            })
        ));
    }

    #[test]
    fn rect_dc_prediction_validates_edge_samples_against_bit_depth() {
        let above = [1u16, 2, 256, 4];
        assert!(matches!(
            predict_intra_dc_rect_value(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraDcEdges::above(&above)
            ),
            Err(ReconError::IntraPredictionSampleOutOfRange {
                edge: IntraDcEdge::Above,
                sample_index: 2,
                value: 256,
                max: 255
            })
        ));
    }

    #[test]
    fn dc_prediction_into_validates_output_stride_and_length() {
        let left = [1u8, 2, 3, 4];
        let mut output = [0u8; 15];

        assert!(matches!(
            predict_intra_dc_square_into(
                BitDepth::Eight,
                size(2),
                IntraDcEdges::left(&left),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));

        assert!(matches!(
            predict_intra_dc_square_into(
                BitDepth::Eight,
                size(2),
                IntraDcEdges::left(&left),
                &mut output,
                4
            ),
            Err(ReconError::IntraPredictionOutputTooSmall {
                expected: 16,
                actual: 15
            })
        ));
    }

    #[test]
    fn rect_dc_prediction_into_validates_output_stride_and_length() {
        let left = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut output = [0u8; 31];

        assert!(matches!(
            predict_intra_dc_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraDcEdges::left(&left),
                &mut output,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));

        assert!(matches!(
            predict_intra_dc_rect_into(
                BitDepth::Eight,
                rect_size(2, 3),
                IntraDcEdges::left(&left),
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
    fn dc_prediction_into_rejects_overflowing_output_shape() {
        let left = [1u8, 2, 3, 4];
        let mut output = [];

        assert!(matches!(
            predict_intra_dc_square_into(
                BitDepth::Eight,
                size(2),
                IntraDcEdges::left(&left),
                &mut output,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "intra prediction output buffer length"
            })
        ));
    }

    #[test]
    fn prediction_block_allocation_failure_is_typed() {
        assert!(matches!(
            SquareIntraPredictionBlock::filled_with_sample_count(size(2), 0u8, usize::MAX),
            Err(ReconError::IntraPredictionAllocationFailed {
                context: "square DC prediction block samples"
            })
        ));
    }
}
