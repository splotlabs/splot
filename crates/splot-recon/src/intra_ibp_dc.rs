// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! IBP DC intra prediction modifier.
//!
//! Feature tracking: `RECON-INTRA-IBP-DC-PREDICTION`.

use crate::intra::{IntraDcEdge, IntraDcEdges, IntraRectBlockSize};
use crate::intra_dc_math::{
    IntraLaneSample, dc_midpoint, load_lanes, predict_intra_dc_rect_value_from_sums, round2_u32,
    store_lanes, validate_dc_edge, validate_output_shape, validate_sample_type,
};
use crate::{BitDepth, ReconError, ReconSample, Result};
use fearless_simd::{Simd, SimdBase, u16x16};
use splot_core::simd::SimdBackend;

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

#[derive(Clone, Copy)]
struct BaselineIbpDc;

#[derive(Clone, Copy)]
struct TypedIbpDc<B: SimdBackend>(B);

trait IbpDcRunner: Copy {
    fn ibp_dc<P: IntraLaneSample>(
        self,
        block: IbpDcBlock,
        edges: (Option<&[P]>, Option<&[P]>),
        pred: &mut [P],
    ) -> Option<()>;
    fn dc_frame<P: IntraLaneSample>(self, block: DcFrameBlock, samples: &mut [P]) -> Option<()>;
}

impl IbpDcRunner for BaselineIbpDc {
    #[inline]
    fn ibp_dc<P: IntraLaneSample>(
        self,
        block: IbpDcBlock,
        edges: (Option<&[P]>, Option<&[P]>),
        pred: &mut [P],
    ) -> Option<()> {
        baseline_simd!(simd => ibp_dc_block(simd, block, edges, pred))
    }

    #[inline]
    fn dc_frame<P: IntraLaneSample>(self, block: DcFrameBlock, samples: &mut [P]) -> Option<()> {
        baseline_simd!(simd => dc_frame_block(simd, block, samples))
    }
}

impl<B: SimdBackend> IbpDcRunner for TypedIbpDc<B> {
    #[inline]
    fn ibp_dc<P: IntraLaneSample>(
        self,
        block: IbpDcBlock,
        edges: (Option<&[P]>, Option<&[P]>),
        pred: &mut [P],
    ) -> Option<()> {
        let narrow = self.0.narrow();
        narrow.vectorize(|| ibp_dc_block(narrow.token(), block, edges, pred))
    }

    #[inline]
    fn dc_frame<P: IntraLaneSample>(self, block: DcFrameBlock, samples: &mut [P]) -> Option<()> {
        let narrow = self.0.narrow();
        narrow.vectorize(|| dc_frame_block(narrow.token(), block, samples))
    }
}

fn predict_intra_ibp_dc_rect_into_with_runner<R: IbpDcRunner, T: ReconSample>(
    runner: R,
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
                return runner
                    .ibp_dc(block, edges, pred)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "IBP DC prediction output shape",
                    });
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
    predict_intra_ibp_dc_rect_into_with_runner(
        BaselineIbpDc,
        bit_depth,
        size,
        edges,
        pred,
        stride_samples,
    )
}

/// Typed [`predict_intra_ibp_dc_rect_into`] using caller-selected `backend`.
///
/// # Errors
/// Returns the same validation errors as [`predict_intra_ibp_dc_rect_into`].
pub fn predict_intra_ibp_dc_rect_into_with<B: SimdBackend, T: ReconSample>(
    backend: B,
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDcEdges<'_, T>,
    pred: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    predict_intra_ibp_dc_rect_into_with_runner(
        TypedIbpDc(backend),
        bit_depth,
        size,
        edges,
        pred,
        stride_samples,
    )
}

/// One block predicted in place by [`predict_dc_in_frame`]: its first sample
/// at `origin` of the plane samples, rows `stride` apart, with the edges that
/// `left` and `above` make available stored beside it.
#[derive(Clone, Copy)]
pub(crate) struct DcFrameBlock {
    pub(crate) size: IntraRectBlockSize,
    pub(crate) bit_depth: BitDepth,
    pub(crate) origin: usize,
    pub(crate) stride: usize,
    pub(crate) left: bool,
    pub(crate) above: bool,
    pub(crate) ibp: bool,
}

fn predict_dc_in_frame_with_runner<R: IbpDcRunner, T: ReconSample>(
    runner: R,
    samples: &mut [T],
    block: DcFrameBlock,
) -> Result<()> {
    validate_sample_type::<T>(block.bit_depth)?;
    let written = if let Some(samples) = T::u8_slice_mut(samples) {
        runner.dc_frame(block, samples)
    } else if let Some(samples) = T::u16_slice_mut(samples) {
        runner.dc_frame(block, samples)
    } else {
        None
    };
    written.ok_or(ReconError::ArithmeticOverflow {
        context: "DC prediction frame block",
    })
}

/// Writes the § 7.13.2.10 DC prediction of `block`, with the § 7.13.2.12
/// IBP blend when `block.ibp`, straight from and into the plane `samples`,
/// which hold samples of the active bit depth.
pub(crate) fn predict_dc_in_frame<T: ReconSample>(
    samples: &mut [T],
    block: DcFrameBlock,
) -> Result<()> {
    predict_dc_in_frame_with_runner(BaselineIbpDc, samples, block)
}

/// Typed [`predict_dc_in_frame`] using caller-selected `backend`.
pub(crate) fn predict_dc_in_frame_with<B: SimdBackend, T: ReconSample>(
    backend: B,
    samples: &mut [T],
    block: DcFrameBlock,
) -> Result<()> {
    predict_dc_in_frame_with_runner(TypedIbpDc(backend), samples, block)
}

#[inline]
fn dc_frame_block<S: Simd, P: IntraLaneSample>(
    simd: S,
    block: DcFrameBlock,
    samples: &mut [P],
) -> Option<()> {
    match block.size.width() {
        4 => dc_frame_rows::<S, P, 4, 1>(simd, block, samples),
        8 => dc_frame_rows::<S, P, 8, 1>(simd, block, samples),
        16 => dc_frame_rows::<S, P, 16, 1>(simd, block, samples),
        32 => dc_frame_rows::<S, P, 32, 2>(simd, block, samples),
        _ => dc_frame_rows::<S, P, 64, 4>(simd, block, samples),
    }
}

#[inline]
fn dc_frame_rows<S: Simd, P: IntraLaneSample, const W: usize, const C: usize>(
    simd: S,
    DcFrameBlock {
        size,
        bit_depth,
        origin,
        stride,
        left,
        above,
        ibp,
    }: DcFrameBlock,
    samples: &mut [P],
) -> Option<()> {
    let height = size.height();
    let mut column = [P::default(); 64];
    let left = if left {
        let rows = samples.get(origin.checked_sub(1)?..)?;
        for (slot, row) in column
            .get_mut(..height)?
            .iter_mut()
            .zip(rows.chunks(stride))
        {
            *slot = *row.first()?;
        }
        Some(column.get(..height)?)
    } else {
        None
    };
    let (before, target) = samples.split_at_mut_checked(origin)?;
    let above = if above {
        Some(
            before
                .get(origin.checked_sub(stride)?..)?
                .first_chunk::<W>()?,
        )
    } else {
        None
    };
    let max = bit_depth.max_sample();
    let sum = |edge: &[P]| crate::sample_range::sum_lanes(simd, edge, false);
    let dc = match (left, above) {
        (Some(left), Some(above)) => {
            let (shift, scale) =
                crate::intra_dc_math::resolve_divisor_32((W + height) as u32).ok()?;
            round2_u32(
                (sum(left) + sum(above)).wrapping_mul(u32::from(scale)),
                shift,
            )
        }
        (Some(left), None) => round2_u32(sum(left), size.log2_height()),
        (None, Some(above)) => round2_u32(sum(above), size.log2_width()),
        (None, None) => u32::from(dc_midpoint(bit_depth)),
    }
    .min(u32::from(max)) as u16;
    if ibp {
        let block = IbpDcBlock {
            wide: max > MAX_NARROW_BLEND_SAMPLE,
            size,
            dc,
            stride,
        };
        return ibp_dc_rows::<S, P, W, C>(simd, block, (left, above.map(|a| &a[..])), target);
    }
    let fill = [P::from_lane(dc); W];
    for row in target.chunks_mut(stride).take(height) {
        *row.first_chunk_mut::<W>()? = fill;
    }
    Some(())
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
#[expect(
    clippy::inline_always,
    reason = "measured: an outlined call costs more than the rows of a small block"
)]
#[inline(always)]
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

    #[test]
    fn typed_backend_matches_compatibility_routes() {
        use splot_core::simd::{SimdBackend, SimdPolicy, SimdScope, resolve_backend};

        struct Check;
        impl SimdScope<()> for Check {
            fn run<B: SimdBackend>(self, backend: B) {
                for (log2_w, log2_h) in [(2, 3), (4, 4), (5, 4)] {
                    let size = rect_size(log2_w, log2_h);
                    let stride = size.width() + 4;
                    let left8: Vec<u8> = (0..size.height()).map(|i| (i * 29 + 3) as u8).collect();
                    let above8: Vec<u8> = (0..size.width()).map(|i| (i * 41 + 7) as u8).collect();
                    let edges8 = IntraDcEdges::new(Some(&left8[..]), Some(&above8[..]));
                    let mut compat8 = vec![3u8; stride * size.height()];
                    let mut typed8 = Vec::from(compat8.as_slice());
                    predict_intra_ibp_dc_rect_into(
                        BitDepth::Eight,
                        size,
                        edges8,
                        &mut compat8,
                        stride,
                    )
                    .unwrap();
                    predict_intra_ibp_dc_rect_into_with(
                        backend,
                        BitDepth::Eight,
                        size,
                        edges8,
                        &mut typed8,
                        stride,
                    )
                    .unwrap();
                    assert_eq!(typed8, compat8);

                    let left10: Vec<u16> = (0..size.height())
                        .map(|i| (i as u16 * 71 + 9) % 1024)
                        .collect();
                    let above10: Vec<u16> = (0..size.width())
                        .map(|i| (i as u16 * 97 + 13) % 1024)
                        .collect();
                    let edges10 = IntraDcEdges::new(Some(&left10[..]), Some(&above10[..]));
                    let mut compat10 = vec![5u16; stride * size.height()];
                    let mut typed10 = Vec::from(compat10.as_slice());
                    predict_intra_ibp_dc_rect_into(
                        BitDepth::Ten,
                        size,
                        edges10,
                        &mut compat10,
                        stride,
                    )
                    .unwrap();
                    predict_intra_ibp_dc_rect_into_with(
                        backend,
                        BitDepth::Ten,
                        size,
                        edges10,
                        &mut typed10,
                        stride,
                    )
                    .unwrap();
                    assert_eq!(typed10, compat10);

                    let frame_stride = size.width() + 8;
                    let origin = frame_stride + 1;
                    let len = (size.height() + 2) * frame_stride;
                    for ibp in [false, true] {
                        let mut frame8_a: Vec<u8> =
                            (0..len).map(|i| ((i * 19 + 5) % 256) as u8).collect();
                        let mut frame8_b = Vec::from(frame8_a.as_slice());
                        let block8 = DcFrameBlock {
                            size,
                            bit_depth: BitDepth::Eight,
                            origin,
                            stride: frame_stride,
                            left: true,
                            above: true,
                            ibp,
                        };
                        predict_dc_in_frame(&mut frame8_a, block8).unwrap();
                        predict_dc_in_frame_with(backend, &mut frame8_b, block8).unwrap();
                        assert_eq!(frame8_b, frame8_a);

                        let mut frame10_a: Vec<u16> =
                            (0..len).map(|i| ((i * 109 + 11) % 1024) as u16).collect();
                        let mut frame10_b = Vec::from(frame10_a.as_slice());
                        let block10 = DcFrameBlock {
                            bit_depth: BitDepth::Ten,
                            ..block8
                        };
                        predict_dc_in_frame(&mut frame10_a, block10).unwrap();
                        predict_dc_in_frame_with(backend, &mut frame10_b, block10).unwrap();
                        assert_eq!(frame10_b, frame10_a);
                    }
                }
            }
        }

        resolve_backend(SimdPolicy::Auto)
            .expect("auto backend")
            .enter(Check);
    }
}
