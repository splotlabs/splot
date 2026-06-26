// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Scalar IBP DC intra prediction modifier.
//!
//! Feature tracking: `RECON-INTRA-IBP-DC-PREDICTION`.

use crate::intra::{IntraDcEdge, IntraDcEdges, IntraRectBlockSize};
use crate::intra_dc_math::{round2, validate_dc_edge, validate_output_shape, validate_sample_type};
use crate::{BitDepth, ReconError, ReconSample, Result};

const IBP_WEIGHT_MAX: u16 = 128;
const IBP_WEIGHT_SHIFT: u8 = 7;

#[rustfmt::skip]
const IBP_WEIGHTS: [[u16; 16]; 5] = [
    [96, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [86, 107, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [77, 90, 102, 115, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [71, 78, 86, 92, 100, 107, 114, 121, 0, 0, 0, 0, 0, 0, 0, 0],
    [68, 72, 76, 79, 83, 87, 90, 94, 98, 102, 106, 109, 113, 117, 121, 124],
];

/// Applies AV2 §7.13.2.12 IBP DC prediction to existing DC samples.
///
/// The caller supplies a strided rectangular `pred` buffer that already holds
/// AV2 §7.13.2.10 DC prediction. This modifier validates the sample type,
/// prepared `LeftCol[0..h)` / `AboveRow[0..w)` edge lengths and ranges, output
/// shape, and every existing `pred` sample it will blend before mutating
/// storage. If both edges are absent, the call is a validated no-op.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edges or prediction samples, a too-small
/// stride, a too-small output buffer, arithmetic overflow, or storage
/// conversion failure.
pub fn apply_intra_ibp_dc_rect<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDcEdges<'_, T>,
    pred: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    let required = validate_output_shape(
        size,
        pred.len(),
        stride_samples,
        "intra prediction output buffer length",
    )?;
    let have_left = validate_dc_edge(
        IntraDcEdge::Left,
        edges.left_samples(),
        size.height(),
        bit_depth,
    )?
    .is_some();
    let have_above = validate_dc_edge(
        IntraDcEdge::Above,
        edges.above_samples(),
        size.width(),
        bit_depth,
    )?
    .is_some();

    validate_pred_samples(bit_depth, size, have_left, have_above, pred, stride_samples)?;

    if let Some(above) = edges.above_samples()
        && have_above
    {
        for row in 0..top_row_count(size) {
            let weight = ibp_weight(size.log2_height(), row)?;
            for (column, above_sample) in above
                .iter()
                .copied()
                .enumerate()
                .take(size.width())
                .skip(above_start_column(size, have_left))
            {
                let index = pred_index(row, column, stride_samples)?;
                let sample =
                    blend_sample(bit_depth, above_sample, pred_sample(pred, index)?, weight)?;
                set_pred_sample(pred, index, sample, required)?;
            }
        }
    }

    if let Some(left) = edges.left_samples()
        && have_left
    {
        for (row, left_sample) in left
            .iter()
            .copied()
            .enumerate()
            .skip(left_start_row(size, have_above))
        {
            for column in 0..left_column_count(size) {
                let weight = ibp_weight(size.log2_width(), column)?;
                let index = pred_index(row, column, stride_samples)?;
                let sample =
                    blend_sample(bit_depth, left_sample, pred_sample(pred, index)?, weight)?;
                set_pred_sample(pred, index, sample, required)?;
            }
        }
    }

    Ok(())
}

fn validate_pred_samples<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    have_left: bool,
    have_above: bool,
    pred: &[T],
    stride_samples: usize,
) -> Result<()> {
    if have_above {
        for row in 0..top_row_count(size) {
            for column in above_start_column(size, have_left)..size.width() {
                validate_pred_sample(bit_depth, pred, pred_index(row, column, stride_samples)?)?;
            }
        }
    }

    if have_left {
        for row in left_start_row(size, have_above)..size.height() {
            for column in 0..left_column_count(size) {
                validate_pred_sample(bit_depth, pred, pred_index(row, column, stride_samples)?)?;
            }
        }
    }

    Ok(())
}

fn validate_pred_sample<T: ReconSample>(
    bit_depth: BitDepth,
    pred: &[T],
    sample_index: usize,
) -> Result<()> {
    let value = pred_sample(pred, sample_index)?.to_u16();
    let max = bit_depth.max_sample();
    if value > max {
        Err(ReconError::IntraPredictionOutputSampleOutOfRange {
            sample_index,
            value,
            max,
        })
    } else {
        Ok(())
    }
}

fn blend_sample<T: ReconSample>(bit_depth: BitDepth, edge: T, pred: T, weight: u16) -> Result<T> {
    let inverse_weight =
        IBP_WEIGHT_MAX
            .checked_sub(weight)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "IBP DC inverse weight",
            })?;
    let edge_product = u64::from(edge.to_u16())
        .checked_mul(u64::from(inverse_weight))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC edge blend product",
        })?;
    let pred_product = u64::from(pred.to_u16())
        .checked_mul(u64::from(weight))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC prediction blend product",
        })?;
    let blended = edge_product
        .checked_add(pred_product)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC blend sum",
        })?;
    let sample = round2(blended, IBP_WEIGHT_SHIFT);
    if sample > bit_depth.max_sample() {
        return Err(ReconError::IntraPredictionOutputSampleOutOfRange {
            sample_index: 0,
            value: sample,
            max: bit_depth.max_sample(),
        });
    }

    T::try_from_u16(sample)
}

fn ibp_weight(log2_dimension: u8, index: usize) -> Result<u16> {
    let row = usize::from(
        log2_dimension
            .checked_sub(2)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "IBP DC weight row",
            })?,
    );
    IBP_WEIGHTS
        .get(row)
        .and_then(|weights| weights.get(index))
        .copied()
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC weight lookup",
        })
}

fn pred_index(row: usize, column: usize, stride_samples: usize) -> Result<usize> {
    row.checked_mul(stride_samples)
        .and_then(|row_start| row_start.checked_add(column))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC prediction sample index",
        })
}

fn pred_sample<T: ReconSample>(pred: &[T], index: usize) -> Result<T> {
    pred.get(index)
        .copied()
        .ok_or(ReconError::IntraPredictionOutputTooSmall {
            expected: index.saturating_add(1),
            actual: pred.len(),
        })
}

fn set_pred_sample<T: ReconSample>(
    pred: &mut [T],
    index: usize,
    sample: T,
    required: usize,
) -> Result<()> {
    let Some(slot) = pred.get_mut(index) else {
        return Err(ReconError::IntraPredictionOutputTooSmall {
            expected: required,
            actual: pred.len(),
        });
    };
    *slot = sample;
    Ok(())
}

const fn top_row_count(size: IntraRectBlockSize) -> usize {
    size.height() >> 2
}

const fn left_column_count(size: IntraRectBlockSize) -> usize {
    size.width() >> 2
}

const fn above_start_column(size: IntraRectBlockSize, have_left: bool) -> usize {
    if size.width() < size.height() && have_left {
        size.width() >> 2
    } else {
        0
    }
}

const fn left_start_row(size: IntraRectBlockSize, have_above: bool) -> usize {
    if size.width() >= size.height() && have_above {
        size.height() >> 2
    } else {
        0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn expected_blend(edge: u16, pred: u16, weight: u16) -> u16 {
        round2(
            u64::from(edge) * u64::from(IBP_WEIGHT_MAX - weight)
                + u64::from(pred) * u64::from(weight),
            IBP_WEIGHT_SHIFT,
        )
    }

    #[test]
    fn ibp_dc_above_only_modifies_top_rows() {
        let size = rect_size(3, 3);
        let above = [100u8, 110, 120, 130, 140, 150, 160, 170];
        let mut pred = [50u8; 64];

        apply_intra_ibp_dc_rect(
            BitDepth::Eight,
            size,
            IntraDcEdges::above(&above),
            &mut pred,
            8,
        )
        .unwrap();

        for column in 0..size.width() {
            assert_eq!(
                pred[column],
                expected_blend(u16::from(above[column]), 50, 86) as u8
            );
            assert_eq!(
                pred[8 + column],
                expected_blend(u16::from(above[column]), 50, 107) as u8
            );
        }
        assert_eq!(&pred[16..], &[50u8; 48]);
    }

    #[test]
    fn ibp_dc_left_only_modifies_left_columns() {
        let size = rect_size(3, 3);
        let left = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let mut pred = [100u8; 64];

        apply_intra_ibp_dc_rect(
            BitDepth::Eight,
            size,
            IntraDcEdges::left(&left),
            &mut pred,
            8,
        )
        .unwrap();

        for row in 0..size.height() {
            assert_eq!(
                pred[row * 8],
                expected_blend(u16::from(left[row]), 100, 86) as u8
            );
            assert_eq!(
                pred[row * 8 + 1],
                expected_blend(u16::from(left[row]), 100, 107) as u8
            );
            assert_eq!(&pred[row * 8 + 2..row * 8 + 8], &[100u8; 6]);
        }
    }

    #[test]
    fn ibp_dc_both_edges_square_skips_left_top_overlap() {
        let size = rect_size(3, 3);
        let left = [20u8; 8];
        let above = [200u8; 8];
        let mut pred = [80u8; 64];

        apply_intra_ibp_dc_rect(
            BitDepth::Eight,
            size,
            IntraDcEdges::both(&left, &above),
            &mut pred,
            8,
        )
        .unwrap();

        let top_row0 = expected_blend(200, 80, 86) as u8;
        let top_row1 = expected_blend(200, 80, 107) as u8;
        let left_col0 = expected_blend(20, 80, 86) as u8;
        let left_col1 = expected_blend(20, 80, 107) as u8;
        assert_eq!(&pred[0..8], &[top_row0; 8]);
        assert_eq!(&pred[8..16], &[top_row1; 8]);
        for row in 2..8 {
            assert_eq!(pred[row * 8], left_col0);
            assert_eq!(pred[row * 8 + 1], left_col1);
            assert_eq!(&pred[row * 8 + 2..row * 8 + 8], &[80u8; 6]);
        }
    }

    #[test]
    fn ibp_dc_both_edges_wide_skips_left_top_overlap() {
        let size = rect_size(4, 3);
        let left = [20u8; 8];
        let above = [200u8; 16];
        let mut pred = [80u8; 128];

        apply_intra_ibp_dc_rect(
            BitDepth::Eight,
            size,
            IntraDcEdges::both(&left, &above),
            &mut pred,
            16,
        )
        .unwrap();

        assert!(pred[0..32].iter().all(|sample| *sample > 80));
        for row in 2..8 {
            assert!(
                pred[row * 16..row * 16 + 4]
                    .iter()
                    .all(|sample| *sample < 80)
            );
            assert_eq!(&pred[row * 16 + 4..row * 16 + 16], &[80u8; 12]);
        }
    }

    #[test]
    fn ibp_dc_both_edges_tall_skips_above_left_overlap() {
        let size = rect_size(3, 4);
        let left = [20u8; 16];
        let above = [200u8; 8];
        let mut pred = [80u8; 128];

        apply_intra_ibp_dc_rect(
            BitDepth::Eight,
            size,
            IntraDcEdges::both(&left, &above),
            &mut pred,
            8,
        )
        .unwrap();

        for row in 0..size.height() {
            assert!(pred[row * 8] < 80);
            assert!(pred[row * 8 + 1] < 80);
        }
        for row in 0..4 {
            assert!(
                pred[row * 8 + 2..row * 8 + 8]
                    .iter()
                    .all(|sample| *sample > 80)
            );
        }
        for row in 4..size.height() {
            assert_eq!(&pred[row * 8 + 2..row * 8 + 8], &[80u8; 6]);
        }
    }

    #[test]
    fn ibp_dc_both_edges_wide_uses_exact_weights_and_skip_boundary() {
        let size = rect_size(4, 3);
        let left = [20u8; 8];
        let above = [200u8; 16];
        let mut pred = [80u8; 128];

        apply_intra_ibp_dc_rect(
            BitDepth::Eight,
            size,
            IntraDcEdges::both(&left, &above),
            &mut pred,
            16,
        )
        .unwrap();

        for column in 0..size.width() {
            assert_eq!(pred[column], expected_blend(200, 80, 86) as u8);
            assert_eq!(pred[16 + column], expected_blend(200, 80, 107) as u8);
        }
        for row in 2..size.height() {
            for column in 0..4 {
                let weight = [77, 90, 102, 115][column];
                assert_eq!(
                    pred[row * 16 + column],
                    expected_blend(20, 80, weight) as u8
                );
            }
            assert_eq!(&pred[row * 16 + 4..row * 16 + 16], &[80u8; 12]);
        }
    }

    #[test]
    fn ibp_dc_both_edges_tall_uses_exact_weights_and_skip_boundary() {
        let size = rect_size(3, 4);
        let left = [20u8; 16];
        let above = [200u8; 8];
        let mut pred = [80u8; 128];

        apply_intra_ibp_dc_rect(
            BitDepth::Eight,
            size,
            IntraDcEdges::both(&left, &above),
            &mut pred,
            8,
        )
        .unwrap();

        for row in 0..size.height() {
            assert_eq!(pred[row * 8], expected_blend(20, 80, 86) as u8);
            assert_eq!(pred[row * 8 + 1], expected_blend(20, 80, 107) as u8);
        }
        for row in 0..4 {
            let weight = [77, 90, 102, 115][row];
            for column in 2..size.width() {
                assert_eq!(
                    pred[row * 8 + column],
                    expected_blend(200, 80, weight) as u8
                );
            }
        }
        for row in 4..size.height() {
            assert_eq!(&pred[row * 8 + 2..row * 8 + 8], &[80u8; 6]);
        }
    }

    #[test]
    fn ibp_dc_max_size_10_bit_uses_last_weight_row() {
        let size = rect_size(6, 6);
        let above = [1023u16; 64];
        let mut pred = [512u16; 4096];

        apply_intra_ibp_dc_rect(
            BitDepth::Ten,
            size,
            IntraDcEdges::above(&above),
            &mut pred,
            64,
        )
        .unwrap();

        for row in 0..16 {
            let weight = IBP_WEIGHTS[4][row];
            assert_eq!(pred[row * 64], expected_blend(1023, 512, weight));
        }
        assert_eq!(pred[16 * 64], 512);
    }

    #[test]
    fn ibp_dc_no_edges_is_validated_no_op() {
        let size = rect_size(2, 2);
        let mut pred = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let before = pred;

        apply_intra_ibp_dc_rect(BitDepth::Eight, size, IntraDcEdges::none(), &mut pred, 4).unwrap();

        assert_eq!(pred, before);
    }

    #[test]
    fn ibp_dc_accepts_10_bit_samples() {
        let size = rect_size(2, 2);
        let above = [1023u16; 4];
        let mut pred = [512u16; 16];

        apply_intra_ibp_dc_rect(
            BitDepth::Ten,
            size,
            IntraDcEdges::above(&above),
            &mut pred,
            4,
        )
        .unwrap();

        assert_eq!(pred[0], expected_blend(1023, 512, 96));
    }

    #[test]
    fn ibp_dc_rejects_typed_invalid_inputs_without_mutation() {
        let size = rect_size(2, 2);
        let left = [1u8, 2, 3];
        let mut pred = [9u8; 16];
        assert!(matches!(
            apply_intra_ibp_dc_rect(
                BitDepth::Eight,
                size,
                IntraDcEdges::left(&left),
                &mut pred,
                4
            ),
            Err(ReconError::IntraPredictionEdgeLengthMismatch {
                edge: IntraDcEdge::Left,
                expected: 4,
                actual: 3
            })
        ));
        assert_eq!(pred, [9u8; 16]);

        let above = [1u16, 2, 300, 4];
        let mut pred = [9u16; 16];
        assert!(matches!(
            apply_intra_ibp_dc_rect(
                BitDepth::Eight,
                size,
                IntraDcEdges::above(&above),
                &mut pred,
                4
            ),
            Err(ReconError::IntraPredictionSampleOutOfRange {
                edge: IntraDcEdge::Above,
                sample_index: 2,
                value: 300,
                max: 255
            })
        ));
        assert_eq!(pred, [9u16; 16]);

        let above = [1u8; 4];
        let mut pred = [9u8; 16];
        assert!(matches!(
            apply_intra_ibp_dc_rect(
                BitDepth::Ten,
                size,
                IntraDcEdges::above(&above),
                &mut pred,
                4
            ),
            Err(ReconError::SampleTypeUnsupportedBitDepth {
                sample_type: "u8",
                bit_depth: BitDepth::Ten
            })
        ));
        assert_eq!(pred, [9u8; 16]);
    }

    #[test]
    fn ibp_dc_rejects_invalid_prediction_output_without_mutation() {
        let size = rect_size(2, 2);
        let above = [1u16; 4];
        let mut pred = [9u16; 16];
        pred[0] = 300;
        let before = pred;

        assert!(matches!(
            apply_intra_ibp_dc_rect(
                BitDepth::Eight,
                size,
                IntraDcEdges::above(&above),
                &mut pred,
                4
            ),
            Err(ReconError::IntraPredictionOutputSampleOutOfRange {
                sample_index: 0,
                value: 300,
                max: 255
            })
        ));
        assert_eq!(pred, before);
    }

    #[test]
    fn ibp_dc_rejects_invalid_output_shape() {
        let size = rect_size(2, 2);
        let above = [1u8; 4];
        let mut pred = [9u8; 15];
        let before = pred;

        assert!(matches!(
            apply_intra_ibp_dc_rect(
                BitDepth::Eight,
                size,
                IntraDcEdges::above(&above),
                &mut pred,
                3
            ),
            Err(ReconError::IntraPredictionStrideTooSmall {
                stride_samples: 3,
                width: 4
            })
        ));
        assert_eq!(pred, before);
        assert!(matches!(
            apply_intra_ibp_dc_rect(
                BitDepth::Eight,
                size,
                IntraDcEdges::above(&above),
                &mut pred,
                4
            ),
            Err(ReconError::IntraPredictionOutputTooSmall {
                expected: 16,
                actual: 15
            })
        ));
        assert_eq!(pred, before);

        let mut pred = [9u8; 16];
        let before = pred;
        assert!(matches!(
            apply_intra_ibp_dc_rect(
                BitDepth::Eight,
                size,
                IntraDcEdges::above(&above),
                &mut pred,
                usize::MAX
            ),
            Err(ReconError::ArithmeticOverflow {
                context: "intra prediction output buffer length"
            })
        ));
        assert_eq!(pred, before);
    }
}
