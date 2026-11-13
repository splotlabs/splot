// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IBP DC intra prediction modifier.
//!
//! Feature tracking: `RECON-INTRA-IBP-DC-PREDICTION`.

use crate::intra::{IntraDcEdge, IntraDcEdges, IntraRectBlockSize};
use crate::intra_dc_math::{
    IntraLaneSample, load_lanes, predict_intra_dc_rect_value_from_sums, store_lanes,
    validate_dc_edge, validate_output_shape, validate_sample_type,
};
use crate::{BitDepth, ReconError, ReconSample, Result};
use fearless_simd::{Simd, SimdBase, u16x16};

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

/// `IBP_INVERSE[log2(n) - 2][i]` is `128 - weight` of § 7.13.2.12 IBP weight
/// `i` for an `n`-sample dimension, zero past its `n / 4` weights.
const IBP_INVERSE: [[u16; 16]; 5] = {
    let mut table = [[0u16; 16]; 5];
    let mut row = 0;
    while row < 5 {
        let mut lane = 0;
        while lane < 1 << row {
            table[row][lane] = IBP_WEIGHT_MAX - IBP_WEIGHTS[row][lane];
            lane += 1;
        }
        row += 1;
    }
    table
};

/// Writes rectangular AV2 §7.13.2.10 DC prediction with the §7.13.2.12 IBP
/// blend into caller storage.
///
/// `pred` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong edge lengths, out-of-range edge samples, a too-small stride, or a
/// too-small output buffer.
pub fn predict_intra_ibp_dc_rect_into<T: ReconSample>(
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
    let (left, above) = (edges.left_samples(), edges.above_samples());
    let left_sum = validate_dc_edge(IntraDcEdge::Left, left, size.height(), bit_depth)?;
    let above_sum = validate_dc_edge(IntraDcEdge::Above, above, size.width(), bit_depth)?;
    let dc = predict_intra_dc_rect_value_from_sums::<T>(bit_depth, size, left_sum, above_sum)?;
    let wide = bit_depth.max_sample() > MAX_NARROW_BLEND_SAMPLE;
    macro_rules! predict {
        ($view:ident, $view_mut:ident) => {
            if let Some(pred) = T::$view_mut(pred) {
                let edges = (left.and_then(T::$view), above.and_then(T::$view));
                let block = IbpDcBlock {
                    wide,
                    size,
                    dc: dc.to_u16(),
                    stride: stride_samples,
                };
                return baseline_simd!(simd => ibp_dc_block(simd, block, edges, pred)).ok_or(
                    ReconError::ArithmeticOverflow {
                        context: "IBP DC prediction output shape",
                    },
                );
            }
        };
    }
    predict!(u8_slice, u8_slice_mut);
    predict!(u16_slice, u16_slice_mut);
    Err(ReconError::SampleTypeUnsupportedBitDepth {
        sample_type: T::TYPE_NAME,
        bit_depth,
    })
}

/// Largest sample whose § 7.13.2.12 blend sum `128 * sample + 64` fits `u16`.
const MAX_NARROW_BLEND_SAMPLE: u16 = 511;

#[derive(Clone, Copy)]
struct IbpDcBlock {
    wide: bool,
    size: IntraRectBlockSize,
    dc: u16,
    stride: usize,
}

type IbpDcEdges<'a, P> = (Option<&'a [P]>, Option<&'a [P]>);

#[inline]
fn ibp_dc_block<S: Simd, P: IntraLaneSample>(
    simd: S,
    block: IbpDcBlock,
    edges: IbpDcEdges<'_, P>,
    pred: &mut [P],
) -> Option<()> {
    match block.size.width() {
        4 => ibp_dc_rows::<S, P, 4, 1>(simd, block, edges, pred),
        8 => ibp_dc_rows::<S, P, 8, 1>(simd, block, edges, pred),
        16 => ibp_dc_rows::<S, P, 16, 1>(simd, block, edges, pred),
        32 => ibp_dc_rows::<S, P, 32, 2>(simd, block, edges, pred),
        _ => ibp_dc_rows::<S, P, 64, 4>(simd, block, edges, pred),
    }
}

/// Writes the block row by row: rows outside both § 7.13.2.12 zones are the
/// DC value, above-zone rows blend with the above edge at the row's weight
/// from the zone's first column, and left-zone rows blend their first `W / 4`
/// columns with the row's left sample at the column weights.
#[inline]
fn ibp_dc_rows<S: Simd, P: IntraLaneSample, const W: usize, const C: usize>(
    simd: S,
    IbpDcBlock {
        wide,
        size,
        dc,
        stride,
    }: IbpDcBlock,
    (left, above): IbpDcEdges<'_, P>,
    pred: &mut [P],
) -> Option<()> {
    let height = size.height();
    let row_inverse = IBP_INVERSE.get(usize::from(size.log2_height()) - 2)?;
    let column_inverse =
        u16x16::from_slice(simd, IBP_INVERSE.get(usize::from(size.log2_width()) - 2)?);
    let above_start = if W < height && left.is_some() {
        W / 4
    } else {
        0
    };
    let left_start = if W >= height && above.is_some() {
        height / 4
    } else {
        0
    };
    let lane = u16x16::from_slice(
        simd,
        &core::array::from_fn::<u16, 16, _>(|lane| lane as u16),
    );
    let above_lanes = simd.simd_ge_u16x16(lane, u16x16::splat(simd, above_start as u16));
    let left_lanes = simd.simd_lt_u16x16(lane, u16x16::splat(simd, (W / 4) as u16));
    let dc_lanes = u16x16::splat(simd, dc);
    let above_edge = match above {
        Some(above) => load_lanes::<S, P, W, C>(simd, above.first_chunk::<W>()?)?,
        None => [dc_lanes; C],
    };
    let zero = u16x16::splat(simd, 0);
    let above_rows = if above.is_some() { height / 4 } else { 0 };
    let left_edge = left.unwrap_or(&[]);
    let left_row = |sample: &P| {
        let edge = u16x16::splat(simd, sample.to_u16());
        ibp_dc_lanes(simd, wide, dc_lanes, edge, column_inverse)
    };
    let mut rows = pred.chunks_mut(stride);
    for (row, &inverse) in row_inverse.iter().enumerate().take(above_rows) {
        let inverse = u16x16::splat(simd, inverse);
        let mut lanes = [dc_lanes; C];
        for (k, (lanes, edge)) in lanes.iter_mut().zip(above_edge).enumerate() {
            let inverse = if k == 0 {
                simd.select_u16x16(above_lanes, inverse, zero)
            } else {
                inverse
            };
            *lanes = ibp_dc_lanes(simd, wide, dc_lanes, edge, inverse);
        }
        if let Some(sample) = left_edge.get(row).filter(|_| left_start == 0) {
            lanes[0] = simd.select_u16x16(left_lanes, left_row(sample), lanes[0]);
        }
        store_lanes::<S, P, W, C>(simd, lanes, rows.next()?.first_chunk_mut::<W>()?)?;
    }
    let rows = rows.take(height - above_rows);
    if left_edge.is_empty() {
        let fill = [P::from_lane(dc); W];
        for out in rows {
            *out.first_chunk_mut::<W>()? = fill;
        }
        return Some(());
    }
    for (out, sample) in rows.zip(left_edge.get(above_rows..)?) {
        let mut lanes = [dc_lanes; C];
        lanes[0] = left_row(sample);
        store_lanes::<S, P, W, C>(simd, lanes, out.first_chunk_mut::<W>()?)?;
    }
    Some(())
}

/// `Round2(dc * (128 - inverse) + edge * inverse, 7)`: in wrapping `u16`
/// lanes as `128 * dc + 64 + (edge - dc) * inverse`, exact since the sum fits,
/// unless `wide` samples need `u32`.
#[inline]
fn ibp_dc_lanes<S: Simd>(
    simd: S,
    wide: bool,
    dc: u16x16<S>,
    edge: u16x16<S>,
    inverse: u16x16<S>,
) -> u16x16<S> {
    if wide {
        return blend_ibp_lanes(
            simd,
            true,
            dc,
            edge,
            u16x16::splat(simd, IBP_WEIGHT_MAX) - inverse,
        );
    }
    let base = (dc << u32::from(IBP_WEIGHT_SHIFT)) + (1 << (IBP_WEIGHT_SHIFT - 1));
    (base + (edge - dc) * inverse) >> u32::from(IBP_WEIGHT_SHIFT)
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
    use crate::intra::predict_intra_dc_rect_value;
    use crate::intra_dc_math::round2_u32;

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

    /// The § 7.13.2.10 DC value, then the § 7.13.2.12 above and left zones
    /// sample by sample.
    #[allow(clippy::needless_range_loop, reason = "the § 7.13.2.12 loops")]
    fn spec_ibp_dc(
        bit_depth: BitDepth,
        size: IntraRectBlockSize,
        left: Option<&[u16]>,
        above: Option<&[u16]>,
    ) -> Vec<u16> {
        let (w, h) = (size.width(), size.height());
        let dc =
            predict_intra_dc_rect_value(bit_depth, size, IntraDcEdges::new(left, above)).unwrap();
        let mut pred = vec![dc; w * h];
        let weight = |log2: u8, index: usize| IBP_WEIGHTS[usize::from(log2) - 2][index];
        if let Some(above) = above {
            let start = if w < h && left.is_some() { w >> 2 } else { 0 };
            for i in 0..h >> 2 {
                for j in start..w {
                    let at = i * w + j;
                    pred[at] = expected_blend(above[j], pred[at], weight(size.log2_height(), i));
                }
            }
        }
        if let Some(left) = left {
            let start = if w >= h && above.is_some() { h >> 2 } else { 0 };
            for i in start..h {
                for j in 0..w >> 2 {
                    let at = i * w + j;
                    pred[at] = expected_blend(left[i], pred[at], weight(size.log2_width(), j));
                }
            }
        }
        pred
    }

    fn predict<T: ReconSample>(
        bit_depth: BitDepth,
        size: IntraRectBlockSize,
        edges: IntraDcEdges<'_, T>,
        stride: usize,
    ) -> Vec<u16> {
        let sentinel = T::try_from_u16(7).unwrap();
        let mut pred = vec![sentinel; stride * size.height()];
        predict_intra_ibp_dc_rect_into(bit_depth, size, edges, &mut pred, stride).unwrap();
        let mut samples = Vec::new();
        for row in pred.chunks(stride) {
            samples.extend(row[..size.width()].iter().map(|sample| sample.to_u16()));
            assert!(
                row[size.width()..]
                    .iter()
                    .all(|sample| sample.to_u16() == 7)
            );
        }
        samples
    }

    #[test]
    fn ibp_dc_matches_the_spec_zone_walk_at_8_and_10_bit() {
        let mut state = 0x0bad_5eed_u64;
        let mut next = |max: u16| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as u32 % (u32::from(max) + 1)) as u16
        };
        for (log2_width, log2_height) in (2..=6u8).flat_map(|w| (2..=6u8).map(move |h| (w, h))) {
            let size = rect_size(log2_width, log2_height);
            let (width, height) = (size.width(), size.height());
            for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
                let max = bit_depth.max_sample();
                for pattern in 0..5 {
                    let mut sample = |i: usize| match pattern {
                        0 => max,
                        1 => 0,
                        2 => [max, 0][i % 2],
                        _ => next(max),
                    };
                    let left: Vec<u16> = (0..height).map(&mut sample).collect();
                    let above: Vec<u16> = (0..width).map(&mut sample).collect();
                    for (have_left, have_above) in
                        [(false, false), (false, true), (true, false), (true, true)]
                    {
                        let left = have_left.then_some(&left[..]);
                        let above = have_above.then_some(&above[..]);
                        let expected = spec_ibp_dc(bit_depth, size, left, above);
                        let edges = IntraDcEdges::new(left, above);
                        let got = predict(bit_depth, size, edges, width + 3);
                        let case = format!(
                            "{width}x{height} {bit_depth:?} p{pattern} {have_left} {have_above}"
                        );
                        assert_eq!(got, expected, "u16 {case}");
                        if bit_depth == BitDepth::Eight {
                            let narrow =
                                |edge: &[u16]| edge.iter().map(|&v| v as u8).collect::<Vec<_>>();
                            let (left, above) = (left.map(narrow), above.map(narrow));
                            let edges = IntraDcEdges::new(left.as_deref(), above.as_deref());
                            assert_eq!(
                                predict(bit_depth, size, edges, width + 5),
                                expected,
                                "u8 {case}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ibp_dc_both_edges_wide_uses_exact_weights_and_skip_boundary() {
        let size = rect_size(4, 3);
        let left = [20u8; 8];
        let above = [200u8; 16];
        let got = predict(BitDepth::Eight, size, IntraDcEdges::both(&left, &above), 16);
        let dc =
            predict_intra_dc_rect_value(BitDepth::Eight, size, IntraDcEdges::both(&left, &above))
                .unwrap()
                .to_u16();
        for column in 0..16 {
            assert_eq!(got[column], expected_blend(200, dc, 86));
            assert_eq!(got[16 + column], expected_blend(200, dc, 107));
        }
        for row in 2..8 {
            for column in 0..4 {
                let weight = [77, 90, 102, 115][column];
                assert_eq!(got[row * 16 + column], expected_blend(20, dc, weight));
            }
            assert!(got[row * 16 + 4..row * 16 + 16].iter().all(|&v| v == dc));
        }
    }

    #[test]
    fn ibp_dc_max_size_10_bit_uses_last_weight_row() {
        let size = rect_size(6, 6);
        let above = [1023u16; 64];
        let got = predict(BitDepth::Ten, size, IntraDcEdges::above(&above), 64);
        for row in 0..16 {
            assert_eq!(
                got[row * 64],
                expected_blend(1023, 1023, IBP_WEIGHTS[4][row])
            );
        }
        let left = [0u16; 64];
        let got = predict(BitDepth::Ten, size, IntraDcEdges::both(&left, &above), 64);
        for row in 0..16 {
            assert_eq!(
                got[row * 64 + 5],
                expected_blend(1023, 512, IBP_WEIGHTS[4][row])
            );
        }
        assert_eq!(got[16 * 64 + 20], 512);
    }

    #[test]
    fn ibp_dc_rejects_typed_invalid_inputs_without_mutation() {
        let size = rect_size(2, 2);
        let left = [1u8, 2, 3];
        let mut pred = [9u8; 16];
        assert!(matches!(
            predict_intra_ibp_dc_rect_into(
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
            predict_intra_ibp_dc_rect_into(
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
            predict_intra_ibp_dc_rect_into(
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
    fn ibp_dc_rejects_invalid_output_shape() {
        let size = rect_size(2, 2);
        let above = [1u8; 4];
        let mut pred = [9u8; 15];
        let before = pred;
        assert!(matches!(
            predict_intra_ibp_dc_rect_into(
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
            predict_intra_ibp_dc_rect_into(
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
            predict_intra_ibp_dc_rect_into(
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
