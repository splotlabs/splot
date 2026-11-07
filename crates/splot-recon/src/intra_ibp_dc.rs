// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Scalar IBP DC intra prediction modifier.
//!
//! Feature tracking: `RECON-INTRA-IBP-DC-PREDICTION`.

use crate::intra::{IntraDcEdge, IntraDcEdges, IntraRectBlockSize};
use crate::intra_dc_math::{
    IntraLaneSample, round2_u32, validate_dc_edge, validate_output_shape, validate_sample_type,
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

    if let Some(pred) = T::u16_slice_mut(pred) {
        let left = edges.left_samples().and_then(T::u16_slice);
        let above = edges.above_samples().and_then(T::u16_slice);
        if blend_ibp_dc_lanes(bit_depth, size, left, above, pred, stride_samples) {
            return Ok(());
        }
    } else if let Some(pred) = T::u8_slice_mut(pred) {
        let left = edges.left_samples().and_then(T::u8_slice);
        let above = edges.above_samples().and_then(T::u8_slice);
        if left.is_some() == have_left
            && above.is_some() == have_above
            && blend_ibp_dc_lanes(bit_depth, size, left, above, pred, stride_samples)
        {
            return Ok(());
        }
    }

    validate_pred_samples(bit_depth, size, have_left, have_above, pred, stride_samples)?;

    for (edge, samples, have_edge, have_other) in [
        (
            IntraDcEdge::Above,
            edges.above_samples(),
            have_above,
            have_left,
        ),
        (
            IntraDcEdge::Left,
            edges.left_samples(),
            have_left,
            have_above,
        ),
    ] {
        let Some(samples) = samples.filter(|_| have_edge) else {
            continue;
        };
        visit_ibp_zone(
            edge,
            size,
            have_other,
            stride_samples,
            |edge_index, log2_dimension, weight_index, pred_index| {
                let edge_sample = samples.get(edge_index).copied().ok_or_else(|| {
                    ReconError::IntraPredictionEdgeLengthMismatch {
                        edge,
                        expected: edge_len(edge, size),
                        actual: samples.len(),
                    }
                })?;
                let weight = ibp_weight(log2_dimension, weight_index)?;
                let sample = blend_sample(
                    bit_depth,
                    edge_sample,
                    pred_sample(pred, pred_index)?,
                    weight,
                )?;
                set_pred_sample(pred, pred_index, sample, required)
            },
        )?;
    }

    Ok(())
}

/// The lane form of the § 7.13.2.12 blend over in-range edges: each above-zone
/// row blends with one weight and each left-zone row with the `w / 4` column
/// weights. Returns `false` without writing when a zone sample is above
/// `bit_depth`, so the checked path reports it.
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
    let max = bit_depth.max_sample();
    let in_range = |rows: Range<usize>, columns: Range<usize>| {
        rows.into_iter().all(|row| {
            pred.get(row * stride + columns.start..row * stride + columns.end)
                .is_some_and(|run| run.iter().all(|&value| value.to_u16() <= max))
        })
    };
    if O::MAX_VALUE > max
        && ((above.is_some() && !in_range(0..height >> 2, above_start..width))
            || (left.is_some() && !in_range(left_start..height, 0..width >> 2)))
    {
        return false;
    }
    let wide = max > MAX_NARROW_BLEND_SAMPLE;
    let zones = IbpDcZones {
        above: above
            .and_then(|edge| edge.get(above_start..))
            .map(|edge| (edge, above_start, above_weights)),
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
        let narrow = |len: usize| start == 0 && width == len;
        if let Some(&edge) = edge.first_chunk::<4>().filter(|_| narrow(4)) {
            let edge = edge.map(E::to_u16);
            packed.blend::<O, 4>(rows, 0, pred, |_| edge, |row| [row_weight(row); 4]);
        } else if let Some(&edge) = edge.first_chunk::<8>().filter(|_| narrow(8)) {
            let edge = edge.map(E::to_u16);
            packed.blend::<O, 8>(rows, 0, pred, |_| edge, |row| [row_weight(row); 8]);
        } else {
            blend_above_rows(simd, wide, edge, start, width, rows, weights, pred, stride);
        }
    }
    let Some((edge, start, weights)) = zones.left else {
        return;
    };
    let rows = start..height.min(edge.len());
    let row_edge = |row: usize| edge[row].to_u16();
    if let Some(&run_weights) = weights.first_chunk::<4>().filter(|_| weights.len() == 4) {
        packed.blend::<O, 4>(rows, 0, pred, |row| [row_edge(row); 4], |_| run_weights);
    } else if let Some(&run_weights) = weights.first_chunk::<8>().filter(|_| weights.len() == 8) {
        packed.blend::<O, 8>(rows, 0, pred, |row| [row_edge(row); 8], |_| run_weights);
    } else {
        blend_left_rows(simd, wide, edge, width, rows, weights, pred, stride);
    }
}

/// Blends the § 7.13.2.12 runs of `C` samples (4 or 8) from column `start`
/// of each row, `16 / C` rows per lane group.
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
        start: usize,
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
                let at = (row + i) * self.stride + start;
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
                let at = (row + i) * self.stride + start;
                if let Some(run) = pred.get_mut(at..).and_then(<[O]>::first_chunk_mut::<C>) {
                    run.copy_from_slice(&lanes[i * C..(i + 1) * C]); // splot-copy-ok: publish one IBP run
                }
            }
            row += count;
        }
    }
}

/// The above zone of [`blend_ibp_dc_zones`] row by row.
#[allow(clippy::too_many_arguments)]
#[inline]
fn blend_above_rows<S: Simd, E: IntraLaneSample, O: IntraLaneSample>(
    simd: S,
    wide: bool,
    edge: &[E],
    start: usize,
    width: usize,
    rows: Range<usize>,
    weights: &[u16; 16],
    pred: &mut [O],
    stride: usize,
) {
    for (row, &weight) in weights.iter().enumerate().take(rows.end) {
        let Some(out) = pred.get_mut(row * stride + start..row * stride + width) else {
            continue;
        };
        let (runs, out) = out.as_chunks_mut::<16>();
        let (sources, edge) = edge.as_chunks::<16>();
        let weight_lanes = u16x16::splat(simd, weight);
        for (run, source) in runs.iter_mut().zip(sources) {
            let blended = blend_ibp_lanes(
                simd,
                wide,
                O::load16(simd, run),
                E::load16(simd, source),
                weight_lanes,
            );
            O::store16(simd, blended, run);
        }
        for (slot, &source) in out.iter_mut().zip(edge) {
            *slot = O::from_lane(blend_ibp_value(source.to_u16(), slot.to_u16(), weight));
        }
    }
}

/// The left zone of [`blend_ibp_dc_zones`] row by row.
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
    for (row, &edge) in edge.iter().enumerate().take(rows.end).skip(rows.start) {
        let edge = edge.to_u16();
        let Some(out) = pred.get_mut(row * stride..row * stride + (width >> 2)) else {
            continue;
        };
        let (runs, out) = out.as_chunks_mut::<16>();
        let (run_weights, weights) = weights.as_chunks::<16>();
        let edge_lanes = u16x16::splat(simd, edge);
        for (run, weight) in runs.iter_mut().zip(run_weights) {
            let blended = blend_ibp_lanes(
                simd,
                wide,
                O::load16(simd, run),
                edge_lanes,
                u16x16::from_slice(simd, weight),
            );
            O::store16(simd, blended, run);
        }
        for (slot, &weight) in out.iter_mut().zip(weights) {
            *slot = O::from_lane(blend_ibp_value(edge, slot.to_u16(), weight));
        }
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

fn blend_ibp_value(edge: u16, pred: u16, weight: u16) -> u16 {
    let sum =
        u32::from(edge) * u32::from(IBP_WEIGHT_MAX - weight) + u32::from(pred) * u32::from(weight);
    round2_u32(sum, IBP_WEIGHT_SHIFT) as u16
}

fn validate_pred_samples<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    have_left: bool,
    have_above: bool,
    pred: &[T],
    stride_samples: usize,
) -> Result<()> {
    for (edge, have_edge, have_other) in [
        (IntraDcEdge::Above, have_above, have_left),
        (IntraDcEdge::Left, have_left, have_above),
    ] {
        if have_edge {
            visit_ibp_zone(
                edge,
                size,
                have_other,
                stride_samples,
                |_, _, _, pred_index| validate_pred_sample(bit_depth, pred, pred_index),
            )?;
        }
    }

    Ok(())
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
    let edge_product = u32::from(edge.to_u16())
        .checked_mul(u32::from(inverse_weight))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC edge blend product",
        })?;
    let pred_product = u32::from(pred.to_u16())
        .checked_mul(u32::from(weight))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC prediction blend product",
        })?;
    let blended = edge_product
        .checked_add(pred_product)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "IBP DC blend sum",
        })?;
    let sample = round2_u32(blended, IBP_WEIGHT_SHIFT) as u16;
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

const fn edge_len(edge: IntraDcEdge, size: IntraRectBlockSize) -> usize {
    match edge {
        IntraDcEdge::Left => size.height(),
        IntraDcEdge::Above => size.width(),
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

    /// The `u16` lane blend must match the checked § 7.13.2.12 zone walk for
    /// every block shape and edge set, and fall back to its error unchanged.
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

                    let mut bad = pred.clone();
                    let at = if have_left {
                        (height - 1) * stride
                    } else {
                        width - 1
                    };
                    bad[at] = 1024;
                    let before = bad.clone();
                    assert_eq!(
                        apply_intra_ibp_dc_rect(BitDepth::Ten, size, edges, &mut bad, stride),
                        Err(ReconError::IntraPredictionOutputSampleOutOfRange {
                            sample_index: at,
                            value: 1024,
                            max: 1023
                        })
                    );
                    assert_eq!(bad, before);
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
