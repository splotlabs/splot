// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IBP DC intra prediction modifier.
//!
//! Feature tracking: `RECON-INTRA-IBP-DC-PREDICTION`.

use crate::intra::{IntraDcEdge, IntraDcEdges, IntraRectBlockSize};
use crate::intra_dc_math::{
    IntraLaneSample, validate_dc_edge, validate_output_shape, validate_sample_type,
};
use crate::{BitDepth, ReconError, ReconSample, Result};
use fearless_simd::{Simd, SimdBase, u16x16};
use std::ops::Range;

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

/// Applies AV2 §7.13.2.12 IBP DC prediction to validated DC samples.
///
/// # Errors
/// Returns [`ReconError`] for invalid inputs, arithmetic overflow, or storage
/// conversion failure.
pub fn apply_intra_ibp_dc_rect<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDcEdges<'_, T>,
    pred: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(
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

    let blended = if let Some(pred) = T::u16_slice_mut(pred) {
        let left = edges.left_samples().and_then(T::u16_slice);
        let above = edges.above_samples().and_then(T::u16_slice);
        blend_ibp_dc_lanes(bit_depth, size, left, above, pred, stride_samples)
    } else if let Some(pred) = T::u8_slice_mut(pred) {
        let left = edges.left_samples().and_then(T::u8_slice);
        let above = edges.above_samples().and_then(T::u8_slice);
        left.is_some() == have_left
            && above.is_some() == have_above
            && blend_ibp_dc_lanes(bit_depth, size, left, above, pred, stride_samples)
    } else {
        false
    };
    if blended {
        Ok(())
    } else {
        Err(ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: T::TYPE_NAME,
            bit_depth,
        })
    }
}

/// The lane form of the § 7.13.2.12 blend over in-range edges and DC
/// samples: each above-zone row blends with one weight and each left-zone row
/// with the `w / 4` column weights. Returns `false` without writing for a block
/// size without IBP weights.
fn blend_ibp_dc_lanes<E: IntraLaneSample, O: IntraLaneSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    left: Option<&[E]>,
    above: Option<&[E]>,
    pred: &mut [O],
    stride: usize,
) -> bool {
    let (width, height) = (size.width(), size.height());
    let weights = |log2: u8| {
        usize::from(log2)
            .checked_sub(2)
            .and_then(|row| IBP_WEIGHTS.get(row))
    };
    let (Some(above_weights), Some(left_weights)) = (
        weights(size.log2_height()),
        weights(size.log2_width()).and_then(|row| row.get(..width >> 2)),
    ) else {
        return false;
    };
    let above_start = if width < height && left.is_some() {
        width >> 2
    } else {
        0
    };
    let left_start = if width >= height && above.is_some() {
        height >> 2
    } else {
        0
    };
    let wide = bit_depth.max_sample() > MAX_NARROW_BLEND_SAMPLE;
    let zones = IbpDcZones {
        above: above.map(|edge| (edge, above_start, above_weights)),
        left: left.map(|edge| (edge, left_start, left_weights)),
    };
    baseline_simd!(simd => blend_ibp_dc_zones(simd, wide, size, zones, pred, stride));
    true
}

/// Largest sample whose § 7.13.2.12 blend sum `128 * sample + 64` fits `u16`.
const MAX_NARROW_BLEND_SAMPLE: u16 = 511;

/// The § 7.13.2.12 above and left zones: edge samples from the zone's first
/// sample, the first row or column of the zone, and the weights.
#[derive(Clone, Copy)]
struct IbpDcZones<'a, E> {
    above: Option<(&'a [E], usize, &'a [u16; 16])>,
    left: Option<(&'a [E], usize, &'a [u16])>,
}

/// Blends both zones in whole lane groups. An above-zone row runs from column
/// zero, its columns before the zone at weight 128, which keeps the sample
/// exactly, so no run is ever partial.
#[inline]
fn blend_ibp_dc_zones<S: Simd, E: IntraLaneSample, O: IntraLaneSample>(
    simd: S,
    wide: bool,
    size: IntraRectBlockSize,
    zones: IbpDcZones<'_, E>,
    pred: &mut [O],
    stride: usize,
) {
    let (width, height) = (size.width(), size.height());
    let packed = PackedRuns { simd, wide, stride };
    if let Some((edge, start, weights)) = zones.above {
        let rows = 0..height >> 2;
        let row_weight = |row: usize| weights.get(row).copied().unwrap_or(0);
        let lane_weights = |row: usize| -> [u16; 16] {
            core::array::from_fn(|lane| {
                if lane < start {
                    IBP_WEIGHT_MAX
                } else {
                    row_weight(row)
                }
            })
        };
        let run: [u16; 16] =
            core::array::from_fn(|lane| edge.get(lane).map_or(0, |&sample| sample.to_u16()));
        match width {
            4 => packed.blend::<O, 4>(rows, pred, |_| head(run), |row| head(lane_weights(row))),
            8 => packed.blend::<O, 8>(rows, pred, |_| head(run), |row| head(lane_weights(row))),
            _ => blend_above_rows(simd, wide, edge, width, rows, &lane_weights, pred, stride),
        }
    }
    let Some((edge, start, weights)) = zones.left else {
        return;
    };
    let rows = start..height.min(edge.len());
    let row_edge = |row: usize| edge[row].to_u16();
    let columns: [u16; 16] = core::array::from_fn(|lane| weights.get(lane).copied().unwrap_or(0));
    match weights.len() {
        1 => packed.blend::<O, 1>(rows, pred, |row| [row_edge(row)], |_| head(columns)),
        2 => packed.blend::<O, 2>(rows, pred, |row| [row_edge(row); 2], |_| head(columns)),
        4 => packed.blend::<O, 4>(rows, pred, |row| [row_edge(row); 4], |_| head(columns)),
        8 => packed.blend::<O, 8>(rows, pred, |row| [row_edge(row); 8], |_| head(columns)),
        _ => blend_left_rows(simd, wide, edge, width, rows, weights, pred, stride),
    }
}

/// The first `C` of sixteen lanes.
#[inline]
fn head<const C: usize>(lanes: [u16; 16]) -> [u16; C] {
    core::array::from_fn(|lane| lanes[lane])
}

/// Blends the § 7.13.2.12 runs of `C` samples (1 to 8) from column zero of
/// each row, `16 / C` rows per lane group.
#[derive(Clone, Copy)]
struct PackedRuns<S> {
    simd: S,
    wide: bool,
    stride: usize,
}

impl<S: Simd> PackedRuns<S> {
    #[inline]
    fn blend<O: IntraLaneSample, const C: usize>(
        self,
        rows: Range<usize>,
        pred: &mut [O],
        edge: impl Fn(usize) -> [u16; C],
        weight: impl Fn(usize) -> [u16; C],
    ) {
        let per_group = 16 / C;
        let mut row = rows.start;
        while row < rows.end {
            let count = per_group.min(rows.end - row);
            let mut lanes = [O::from_lane(0); 16];
            let (mut edges, mut weights) = ([0u16; 16], [0u16; 16]);
            for i in 0..count {
                let at = (row + i) * self.stride;
                let Some(run) = pred.get(at..).and_then(<[O]>::first_chunk::<C>) else {
                    return;
                };
                lanes[i * C..(i + 1) * C].copy_from_slice(run); // splot-copy-ok: gather one IBP run
                edges[i * C..(i + 1) * C].copy_from_slice(&edge(row + i)); // splot-copy-ok: gather one IBP run edge
                weights[i * C..(i + 1) * C].copy_from_slice(&weight(row + i)); // splot-copy-ok: gather one IBP run weight
            }
            let blended = blend_ibp_lanes(
                self.simd,
                self.wide,
                O::load16(self.simd, &lanes),
                u16x16::from_slice(self.simd, &edges),
                u16x16::from_slice(self.simd, &weights),
            );
            O::store16(self.simd, blended, &mut lanes);
            for i in 0..count {
                let at = (row + i) * self.stride;
                if let Some(run) = pred.get_mut(at..).and_then(<[O]>::first_chunk_mut::<C>) {
                    run.copy_from_slice(&lanes[i * C..(i + 1) * C]); // splot-copy-ok: publish one IBP run
                }
            }
            row += count;
        }
    }
}

/// The above zone of [`blend_ibp_dc_zones`] row by row for widths of 16 and
/// more, `lane_weights(row)` the weights of the first lane group.
#[allow(clippy::too_many_arguments)]
#[inline]
fn blend_above_rows<S: Simd, E: IntraLaneSample, O: IntraLaneSample>(
    simd: S,
    wide: bool,
    edge: &[E],
    width: usize,
    rows: Range<usize>,
    lane_weights: &impl Fn(usize) -> [u16; 16],
    pred: &mut [O],
    stride: usize,
) {
    for row in rows {
        let Some(out) = pred.get_mut(row * stride..row * stride + width) else {
            continue;
        };
        let first = u16x16::from_slice(simd, &lane_weights(row));
        let rest = u16x16::splat(simd, lane_weights(row)[15]);
        for (index, (run, source)) in out
            .as_chunks_mut::<16>()
            .0
            .iter_mut()
            .zip(edge.as_chunks::<16>().0)
            .enumerate()
        {
            let weight = if index == 0 { first } else { rest };
            let blended = blend_ibp_lanes(
                simd,
                wide,
                O::load16(simd, run),
                E::load16(simd, source),
                weight,
            );
            O::store16(simd, blended, run);
        }
    }
}

/// The left zone of [`blend_ibp_dc_zones`] row by row for 16 columns.
#[allow(clippy::too_many_arguments)]
#[inline]
fn blend_left_rows<S: Simd, E: IntraLaneSample, O: IntraLaneSample>(
    simd: S,
    wide: bool,
    edge: &[E],
    width: usize,
    rows: Range<usize>,
    weights: &[u16],
    pred: &mut [O],
    stride: usize,
) {
    let Some(weights) = weights.first_chunk::<16>() else {
        return;
    };
    let weights = u16x16::from_slice(simd, weights);
    for (row, &edge) in edge.iter().enumerate().take(rows.end).skip(rows.start) {
        let Some(run) = pred
            .get_mut(row * stride..row * stride + (width >> 2))
            .and_then(<[O]>::first_chunk_mut::<16>)
        else {
            continue;
        };
        let blended = blend_ibp_lanes(
            simd,
            wide,
            O::load16(simd, run),
            u16x16::splat(simd, edge.to_u16()),
            weights,
        );
        O::store16(simd, blended, run);
    }
}

/// `Round2(pred * weight + edge * (128 - weight), 7)` over 16 lanes, in `u16`
/// lanes unless `wide` samples need `u32`.
#[inline]
pub(crate) fn blend_ibp_lanes<S: Simd>(
    simd: S,
    wide: bool,
    pred: u16x16<S>,
    edge: u16x16<S>,
    weight: u16x16<S>,
) -> u16x16<S> {
    let inverse = u16x16::splat(simd, IBP_WEIGHT_MAX) - weight;
    let round = 1 << (IBP_WEIGHT_SHIFT - 1);
    if !wide {
        return (pred * weight + edge * inverse + round) >> u32::from(IBP_WEIGHT_SHIFT);
    }
    let [
        (pred_low, pred_high),
        (edge_low, edge_high),
        (weight_low, weight_high),
        (inverse_low, inverse_high),
    ] = [pred, edge, weight, inverse].map(|lanes| simd.widen_u16x16(lanes));
    let low = (pred_low * weight_low + edge_low * inverse_low + u32::from(round))
        >> u32::from(IBP_WEIGHT_SHIFT);
    let high = (pred_high * weight_high + edge_high * inverse_high + u32::from(round))
        >> u32::from(IBP_WEIGHT_SHIFT);
    simd.relaxed_narrow_u32x8(low, high)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::intra_dc_math::round2_u32;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn visit_ibp_zone(
        edge: IntraDcEdge,
        size: IntraRectBlockSize,
        have_other: bool,
        stride_samples: usize,
        mut visit: impl FnMut(usize, u8, usize, usize) -> Result<()>,
    ) -> Result<()> {
        let (rows, columns, log2_dimension) = match edge {
            IntraDcEdge::Above => {
                let start_column = if size.width() < size.height() && have_other {
                    size.width() >> 2
                } else {
                    0
                };
                (
                    0..(size.height() >> 2),
                    start_column..size.width(),
                    size.log2_height(),
                )
            }
            IntraDcEdge::Left => {
                let start_row = if size.width() >= size.height() && have_other {
                    size.height() >> 2
                } else {
                    0
                };
                (
                    start_row..size.height(),
                    0..(size.width() >> 2),
                    size.log2_width(),
                )
            }
        };

        for row in rows {
            for column in columns.clone() {
                let (edge_index, weight_index) = match edge {
                    IntraDcEdge::Above => (column, row),
                    IntraDcEdge::Left => (row, column),
                };
                visit(
                    edge_index,
                    log2_dimension,
                    weight_index,
                    pred_index(row, column, stride_samples)?,
                )?;
            }
        }

        Ok(())
    }

    fn ibp_weight(log2_dimension: u8, index: usize) -> Result<u16> {
        let row = usize::from(log2_dimension.checked_sub(2).ok_or(
            ReconError::ArithmeticOverflow {
                context: "IBP DC weight row",
            },
        )?);
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

    fn expected_blend(edge: u16, pred: u16, weight: u16) -> u16 {
        round2_u32(
            u32::from(edge) * u32::from(IBP_WEIGHT_MAX - weight)
                + u32::from(pred) * u32::from(weight),
            IBP_WEIGHT_SHIFT,
        ) as u16
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

    /// The `u16` lane blend must match the checked § 7.13.2.12 zone walk for
    /// every block shape and edge set.
    /// At 8 bits `u8` storage must write the same samples as `u16` storage.
    #[test]
    fn ibp_dc_u16_lanes_match_the_scalar_zone_walk() {
        let sample = |index: usize, seed: usize| match index % 5 {
            0 => 1023,
            1 => 0,
            _ => ((index * 7919 + seed) % 1024) as u16,
        };
        for log2_width in 2..=6u8 {
            for log2_height in 2..=6u8 {
                let size = rect_size(log2_width, log2_height);
                let (width, height) = (size.width(), size.height());
                let stride = width + 3;
                let left: Vec<u16> = (0..height).map(|i| sample(i, 5)).collect();
                let above: Vec<u16> = (0..width).map(|i| sample(i + 1, 9)).collect();
                let pred: Vec<u16> = (0..stride * height).map(|i| sample(i + 3, 13)).collect();
                for (have_left, have_above) in [(false, true), (true, false), (true, true)] {
                    let edges = IntraDcEdges::new(
                        have_left.then_some(&left[..]),
                        have_above.then_some(&above[..]),
                    );
                    let mut expected = pred.clone();
                    for (edge, samples, have_edge, have_other) in [
                        (IntraDcEdge::Above, &above, have_above, have_left),
                        (IntraDcEdge::Left, &left, have_left, have_above),
                    ] {
                        if have_edge {
                            visit_ibp_zone(edge, size, have_other, stride, |e, log2, w, at| {
                                let weight = ibp_weight(log2, w).unwrap();
                                expected[at] = expected_blend(samples[e], pred[at], weight);
                                Ok(())
                            })
                            .unwrap();
                        }
                    }
                    let mut got = pred.clone();
                    apply_intra_ibp_dc_rect(BitDepth::Ten, size, edges, &mut got, stride).unwrap();
                    assert_eq!(got, expected, "{width}x{height} {have_left} {have_above}");

                    let wide =
                        |samples: &[u16]| samples.iter().map(|&v| v >> 2).collect::<Vec<_>>();
                    let narrow =
                        |samples: &[u16]| samples.iter().map(|&v| v as u8).collect::<Vec<_>>();
                    let (wide_left, wide_above) = (wide(&left), wide(&above));
                    let (narrow_left, narrow_above) = (narrow(&wide_left), narrow(&wide_above));
                    let mut wide_pred = wide(&pred);
                    let mut narrow_pred = narrow(&wide_pred);
                    let wide_edges = IntraDcEdges::new(
                        have_left.then_some(&wide_left[..]),
                        have_above.then_some(&wide_above[..]),
                    );
                    apply_intra_ibp_dc_rect(
                        BitDepth::Eight,
                        size,
                        wide_edges,
                        &mut wide_pred,
                        stride,
                    )
                    .unwrap();
                    let narrow_edges = IntraDcEdges::new(
                        have_left.then_some(&narrow_left[..]),
                        have_above.then_some(&narrow_above[..]),
                    );
                    apply_intra_ibp_dc_rect(
                        BitDepth::Eight,
                        size,
                        narrow_edges,
                        &mut narrow_pred,
                        stride,
                    )
                    .unwrap();
                    assert_eq!(narrow(&wide_pred), narrow_pred);
                }
            }
        }
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
