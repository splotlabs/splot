// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Data-driven intra prediction primitive.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use crate::intra_dc_math::{
    IntraEdgeError, IntraLaneSample, validate_intra_edge_samples, validate_output_shape,
    validate_sample_type,
};
use fearless_simd::{Bytes as _, Simd, SimdBase, i16x16, i32x8, u8x32, u16x8, u16x16, u32x8};

use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

const DIP_FEATURE_COUNT: usize = 11;
const DIP_GRID_SIDE: usize = 8;
const DIP_MODE_COUNT: usize = 6;

/// Edge identifier for DIP intra prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraDipEdge {
    /// Left edge samples, including the bottom-left DIP extension.
    Left,
    /// Above edge samples, including the top-right DIP extension.
    Above,
    /// Top-left corner sample `AboveRow[-1]`.
    TopLeft,
}

impl IntraDipEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
            Self::TopLeft => "top-left",
        }
    }
}

impl IntraEdgeError for IntraDipEdge {
    fn length_mismatch(self, expected: usize, actual: usize) -> ReconError {
        ReconError::IntraDipEdgeLengthMismatch {
            edge: self,
            expected,
            actual,
        }
    }

    fn sample_out_of_range(self, sample_index: usize, value: u16, max: u16) -> ReconError {
        ReconError::IntraDipSampleOutOfRange {
            edge: self,
            sample_index,
            value,
            max,
        }
    }
}

/// Caller-provided prepared edge samples for AV2 §7.13.2.3 DIP prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraDipEdges<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
    top_left: T,
}

impl<'a, T: ReconSample> IntraDipEdges<'a, T> {
    /// Creates a prepared DIP edge set.
    ///
    /// `left` holds `height` to `height + height / 4` samples and `above`
    /// `width` to `width + width / 4`; the samples a shorter edge lacks repeat
    /// its last, as the AV2 §7.13.2.1 edge preparation extends an edge past
    /// its available samples. Availability and fallback preparation remain
    /// outside this type and are owned by that broader intra process.
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

/// Writes rectangular AV2 §7.13.2.3 data-driven intra prediction into caller storage.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. Samples outside the predicted
/// rectangle are left unchanged.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// wrong DIP mode, wrong edge lengths, out-of-range edge samples, a too-small
/// stride, or a too-small output buffer.
pub fn predict_intra_dip_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    dip_mode: usize,
    dip_transpose: bool,
    edges: IntraDipEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    validate_dip_mode(dip_mode)?;
    validate_dip_edges(bit_depth, size, edges)?;
    validate_output_shape(
        size,
        output.len(),
        stride_samples,
        "DIP intra prediction output buffer length",
    )?;

    let features = dip_features(size, edges, dip_transpose);
    let Some(rows) = DIP_ROW_WEIGHTS[usize::from(dip_transpose)].get(dip_mode) else {
        return Err(ReconError::UnsupportedIntraDipMode {
            mode: dip_mode,
            max: DIP_MODE_COUNT - 1,
        });
    };
    let shape_error = || ReconError::ArithmeticOverflow {
        context: "DIP intra prediction output shape",
    };
    let block = DipBlock {
        size,
        grid: dip_grid(size),
        max: i32::from(bit_depth.max_sample()),
        stride: stride_samples,
    };
    macro_rules! predict {
        ($view:ident, $view_mut:ident) => {
            if let (Some(left), Some(above), Some(output)) = (
                T::$view(edges.left),
                T::$view(edges.above),
                T::$view_mut(output),
            ) {
                return baseline_simd!(simd => {
                    predict_dip_samples(simd, block, &features, rows, left, above, output)
                })
                .ok_or_else(shape_error);
            }
        };
    }
    predict!(u8_slice, u8_slice_mut);
    predict!(u16_slice, u16_slice_mut);
    Err(shape_error())
}

/// The geometry of one DIP block.
#[derive(Clone, Copy)]
struct DipBlock {
    size: IntraRectBlockSize,
    grid: DipGrid,
    max: i32,
    stride: usize,
}

/// Writes the § 7.13.2.3 coarse grid samples, then interpolates the samples
/// between them.
#[inline]
fn predict_dip_samples<S: Simd, P: IntraLaneSample>(
    simd: S,
    block: DipBlock,
    features: &[i32; DIP_FEATURE_COUNT],
    rows: &[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE],
    left: &[P],
    above: &[P],
    output: &mut [P],
) -> Option<()> {
    let DipBlock {
        size,
        grid,
        max,
        stride,
    } = block;
    let mut values = [[0i32; 8]; DIP_GRID_SIDE];
    coarse_grid_rows(simd, features, rows, grid, max, &mut values);
    interpolate_lanes(simd, size, grid, &values, left, above, output, stride)
}

fn validate_dip_mode(dip_mode: usize) -> Result<()> {
    if dip_mode < DIP_MODE_COUNT {
        Ok(())
    } else {
        Err(ReconError::UnsupportedIntraDipMode {
            mode: dip_mode,
            max: DIP_MODE_COUNT - 1,
        })
    }
}

fn validate_dip_edges<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    edges: IntraDipEdges<'_, T>,
) -> Result<()> {
    let extended = |edge: &[T], len: usize| edge.len().clamp(len, len + (len >> 2));
    validate_intra_edge_samples(
        IntraDipEdge::Left,
        edges.left,
        extended(edges.left, size.height()),
        bit_depth,
    )?;
    validate_intra_edge_samples(
        IntraDipEdge::Above,
        edges.above,
        extended(edges.above, size.width()),
        bit_depth,
    )?;
    validate_intra_edge_samples(
        IntraDipEdge::TopLeft,
        core::slice::from_ref(&edges.top_left),
        1,
        bit_depth,
    )
}

fn dip_features<T: ReconSample>(
    size: IntraRectBlockSize,
    edges: IntraDipEdges<'_, T>,
    dip_transpose: bool,
) -> [i32; DIP_FEATURE_COUNT] {
    let mut features = [0i32; DIP_FEATURE_COUNT];
    features[0] = i32::from(edges.top_left.to_u16());

    let above = dip_averages(edges.above, size.width(), size.log2_width());
    let left = dip_averages(edges.left, size.height(), size.log2_height());
    for index in 0..4 {
        features[index + 1] = if dip_transpose {
            left[index]
        } else {
            above[index]
        };
        features[index + 5] = if dip_transpose {
            above[index]
        } else {
            left[index]
        };
    }
    features[9] = if dip_transpose { left[4] } else { above[4] };
    features[10] = if dip_transpose { above[4] } else { left[4] };
    features
}

fn dip_averages<T: ReconSample>(samples: &[T], block_len: usize, log2_len: u8) -> [i32; 5] {
    /// The sums of the first five `N`-sample groups of `samples`, which
    /// holds four to five groups; the samples it lacks repeat its last.
    #[inline]
    fn group_sums<T: ReconSample, const N: usize>(samples: &[T]) -> [i32; 5] {
        let value = |sample: &T| i32::from(sample.to_u16());
        let mut sums = [0i32; 5];
        let (groups, rest) = samples.as_chunks::<N>();
        for (sum, group) in sums.iter_mut().zip(groups) {
            *sum = group.iter().map(value).sum();
        }
        if groups.len() < 5
            && let Some(last) = samples.last()
        {
            sums[4] = rest.iter().map(value).sum::<i32>() + (N - rest.len()) as i32 * value(last);
        }
        sums
    }
    let sums = match block_len >> 2 {
        1 => group_sums::<T, 1>(samples),
        2 => group_sums::<T, 2>(samples),
        4 => group_sums::<T, 4>(samples),
        8 => group_sums::<T, 8>(samples),
        _ => group_sums::<T, 16>(samples),
    };
    let group_log2 = log2_len - 2;
    let round = (block_len >> 3) as i32;
    sums.map(|sum| (sum + round) >> group_log2)
}

/// `DIP_WEIGHTS[mode][index]` of one grid row, interleaved for a pairwise
/// multiply-add: lane `2 * gx + j` of entry `k` holds the weight of feature
/// `2 * k + j` at grid column `gx` (feature 11 is zero). Indexed
/// `[transpose][mode][iy][k]`.
static DIP_ROW_WEIGHTS: [[[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE]; DIP_MODE_COUNT]; 2] =
    dip_row_weights();

const DIP_FEATURE_PAIRS: usize = DIP_FEATURE_COUNT.div_ceil(2);

#[allow(
    clippy::large_stack_arrays,
    reason = "evaluated only at compile time for a static"
)]
const fn dip_row_weights() -> [[[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE]; DIP_MODE_COUNT]; 2]
{
    let mut table = [[[[[0i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE]; DIP_MODE_COUNT]; 2];
    let mut transpose = 0;
    while transpose < 2 {
        let mut mode = 0;
        while mode < DIP_MODE_COUNT {
            let mut iy = 0;
            while iy < DIP_GRID_SIDE {
                let mut gx = 0;
                while gx < DIP_GRID_SIDE {
                    let index = if transpose == 1 {
                        gx * DIP_GRID_SIDE + iy
                    } else {
                        iy * DIP_GRID_SIDE + gx
                    };
                    let weights = &splot_tables::tables::loop_restoration::DIP_WEIGHTS[mode][index];
                    let mut feature = 0;
                    while feature < DIP_FEATURE_COUNT {
                        table[transpose][mode][iy][feature / 2][2 * gx + feature % 2] =
                            weights[feature] as i16;
                        feature += 1;
                    }
                    gx += 1;
                }
                iy += 1;
            }
            mode += 1;
        }
        transpose += 1;
    }
    table
}

/// The coarse grid rows of a DIP block, each `Clip1(Round2(Σ
/// features * weights, 10))` over its eight columns.
#[inline]
fn coarse_grid_rows<S: Simd>(
    simd: S,
    features: &[i32; DIP_FEATURE_COUNT],
    rows: &[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE],
    grid: DipGrid,
    max: i32,
    values: &mut [[i32; 8]; DIP_GRID_SIDE],
) {
    let pairs: [i16x16<S>; DIP_FEATURE_PAIRS] = core::array::from_fn(|k| {
        let low = u32::from(features[2 * k] as u16);
        let high = features.get(2 * k + 1).map_or(0, |&f| u32::from(f as u16));
        u32x8::splat(simd, low | high << 16).bitcast()
    });
    for (gy, out) in values.iter_mut().take(grid.height).enumerate() {
        let weights = &rows[gy * grid.down_y];
        let mut sum = i32x8::splat(simd, 1 << 9);
        for (pair, weight) in pairs.iter().zip(weights) {
            sum += simd.pairwise_dot_product_i16x16(*pair, i16x16::from_slice(simd, weight));
        }
        (sum >> 10).max(0).min(max).store_slice(out);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DipGrid {
    step_x: usize,
    step_y: usize,
    down_x: usize,
    down_y: usize,
    width: usize,
    height: usize,
    up_log2_x: u8,
    up_log2_y: u8,
}

fn dip_grid(size: IntraRectBlockSize) -> DipGrid {
    let width_log2 = size.log2_width() - 2;
    let height_log2 = size.log2_height() - 2;
    let (up_log2_x, down_log2_x) = split_dip_scale_log2(width_log2);
    let (up_log2_y, down_log2_y) = split_dip_scale_log2(height_log2);
    DipGrid {
        step_x: 1usize << up_log2_x,
        step_y: 1usize << up_log2_y,
        down_x: 1usize << down_log2_x,
        down_y: 1usize << down_log2_y,
        width: DIP_GRID_SIDE >> down_log2_x,
        height: DIP_GRID_SIDE >> down_log2_y,
        up_log2_x,
        up_log2_y,
    }
}

fn split_dip_scale_log2(log2_quarter_len: u8) -> (u8, u8) {
    if log2_quarter_len == 0 {
        (0, 1)
    } else {
        (log2_quarter_len - 1, 0)
    }
}

/// Writes each sampled row from its coarse grid `values`, horizontally
/// interpolated from the left edge when the grid is coarser than the block,
/// then fills the rows between sampled rows from the row or above edge before
/// them. Every interpolated result is a convex combination of two samples with
/// power-of-two weights, so it needs no clipping, and a between row's
/// `a * step + z * (b - a)` is a wrapping running sum that stays exact.
#[allow(clippy::too_many_arguments)]
#[inline]
fn interpolate_lanes<S: Simd, P: IntraLaneSample>(
    simd: S,
    size: IntraRectBlockSize,
    grid: DipGrid,
    values: &[[i32; 8]; DIP_GRID_SIDE],
    left: &[P],
    above: &[P],
    output: &mut [P],
    stride: usize,
) -> Option<()> {
    let width = size.width();
    let up = u32::from(grid.up_log2_x);
    let step = 1u16 << up;
    let w1 = u16x16::from_fn(simd, |lane| (lane as u16 & (step - 1)) + 1);
    let w0 = u16x16::splat(simd, step) - w1;
    for (band, row_values) in values.iter().enumerate().take(grid.height) {
        let y = band * grid.step_y + grid.step_y - 1;
        let row = output.get_mut(y * stride..)?.get_mut(..width)?;
        let grid_row = || {
            let (low, high) = simd.split_u32x8(i32x8::from_slice(simd, row_values).bitcast());
            simd.relaxed_narrow_u32x4(low, high)
        };
        if grid.down_x == 1
            && let Some(row) = row.first_chunk_mut::<8>()
            && width == 8
        {
            P::store8(simd, grid_row(), row);
            continue;
        }
        if grid.up_log2_x == 1
            && let Some(row) = row.first_chunk_mut::<16>()
            && width == 16
        {
            let right = grid_row();
            let before = simd.slide_u16x8::<7>(u16x8::splat(simd, left.get(y)?.to_u16()), right);
            let between = (before + right) >> 1;
            let samples = simd.combine_u16x8(
                simd.zip_low_u16x8(between, right),
                simd.zip_high_u16x8(between, right),
            );
            P::store16(simd, samples, row);
            continue;
        }
        if grid.step_x == 1 {
            let values = grid_row();
            let mut lanes = [P::from_lane(0); 8];
            P::store8(simd, simd.unzip_low_u16x8(values, values), &mut lanes);
            *row.first_chunk_mut::<4>()? = *lanes.first_chunk::<4>()?;
            continue;
        }
        let mut points = [0u16; 16];
        points[0] = left.get(y)?.to_u16();
        for (point, &value) in points[1..=DIP_GRID_SIDE].iter_mut().zip(row_values) {
            *point = value as u16;
        }
        let points = u16x16::from_slice(simd, &points);
        let index = GRID_POINT_INDEX.get(usize::from(grid.up_log2_x) - 1)?;
        for (out, index) in row.as_chunks_mut::<16>().0.iter_mut().zip(index) {
            let near = points.swizzle_dyn(u8x32::from_slice(simd, &index[0]));
            let far = points.swizzle_dyn(u8x32::from_slice(simd, &index[1]));
            P::store16(simd, (near * w0 + far * w1) >> up, out);
        }
    }
    if grid.step_y > 1 {
        let shift = u32::from(grid.up_log2_y);
        let step = grid.step_y;
        macro_rules! lerp_columns {
            ($load:ident, $store:ident, $columns:expr) => {
                for column in $columns {
                    let mut a = P::$load(simd, above.get(column..)?.first_chunk()?);
                    let mut at = column;
                    for _ in 0..grid.height {
                        let last = output.get(at + (step - 1) * stride..)?;
                        let b = P::$load(simd, last.first_chunk()?);
                        let (mut lerp, delta) = (a << shift, b - a);
                        for _ in 1..step {
                            lerp += delta;
                            P::$store(
                                simd,
                                lerp >> shift,
                                output.get_mut(at..)?.first_chunk_mut()?,
                            );
                            at += stride;
                        }
                        at += stride;
                        a = b;
                    }
                }
            };
        }
        if width >= 16 {
            lerp_columns!(load16, store16, (0..width).step_by(16));
        } else if width == 8 {
            lerp_columns!(load8, store8, [0]);
        } else {
            for band in 0..grid.height {
                let top = band * step;
                let last = (top + step - 1) * stride;
                let a: [P; 4] = *match top.checked_sub(1) {
                    Some(row) => output.get(row * stride..),
                    None => above.get(..),
                }?
                .first_chunk()?;
                let b: [P; 4] = *output.get(last..)?.first_chunk()?;
                let lanes = |run: [P; 4]| {
                    let mut lanes = [P::from_lane(0); 8];
                    lanes[..4].copy_from_slice(&run); // splot-copy-ok: widen one 4-sample run
                    P::load8(simd, &lanes)
                };
                let (a, b) = (lanes(a), lanes(b));
                for z in 1..step {
                    let (w0, w1) = ((step - z) as u16, z as u16);
                    let mut values = [P::from_lane(0); 8];
                    P::store8(simd, (a * w0 + b * w1) >> shift, &mut values);
                    let row = output.get_mut((top + z - 1) * stride..)?;
                    *row.first_chunk_mut::<4>()? = *values.first_chunk::<4>()?;
                }
            }
        }
    }
    Some(())
}

/// `GRID_POINT_INDEX[up - 1][k]` holds the byte indices of the grid points
/// around samples `16 * k..16 * k + 16` of a sampled row with `1 << up` samples
/// per grid step: sample `c` lies between points `c >> up` and `(c >> up) + 1`
/// (point 0 is the left edge sample).
static GRID_POINT_INDEX: [[[[u8; 32]; 2]; 4]; 3] = {
    let mut table = [[[[0u8; 32]; 2]; 4]; 3];
    let mut up = 1;
    while up <= 3 {
        let mut k = 0;
        while k < 4 {
            let mut byte = 0;
            while byte < 32 {
                let point = ((16 * k + byte / 2) >> up) % 8;
                table[up - 1][k][0][byte] = (2 * point + byte % 2) as u8;
                table[up - 1][k][1][byte] = (2 * (point + 1) + byte % 2) as u8;
                byte += 1;
            }
            k += 1;
        }
        up += 1;
    }
    table
};

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    #[test]
    fn flat_edges_stay_near_flat() {
        let size = rect_size(3, 4);
        let left = [128u8; 20];
        let above = [128u8; 10];
        for mode in 0..DIP_MODE_COUNT {
            for transpose in [false, true] {
                let mut output = [0u8; 128];
                predict_intra_dip_rect_into(
                    BitDepth::Eight,
                    size,
                    mode,
                    transpose,
                    IntraDipEdges::new(&left, &above, 128),
                    &mut output,
                    8,
                )
                .unwrap();
                assert!(output.iter().all(|sample| (127..=129).contains(sample)));
            }
        }
    }

    #[test]
    fn accepts_quarter_grid_blocks() {
        let size = rect_size(2, 2);
        let left = [80u8, 84, 88, 92, 96];
        let above = [180u8, 176, 172, 168, 164];
        for mode in 0..DIP_MODE_COUNT {
            for transpose in [false, true] {
                let mut output = [0u8; 16];
                predict_intra_dip_rect_into(
                    BitDepth::Eight,
                    size,
                    mode,
                    transpose,
                    IntraDipEdges::new(&left, &above, 128),
                    &mut output,
                    4,
                )
                .unwrap();
                assert!(output.iter().any(|&sample| sample > 0));
            }
        }
    }

    #[test]
    fn rejects_bad_mode() {
        let err = predict_intra_dip_rect_into(
            BitDepth::Eight,
            rect_size(3, 3),
            DIP_MODE_COUNT,
            false,
            IntraDipEdges::new(&[128u8; 10], &[128u8; 10], 128),
            &mut [0u8; 64],
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ReconError::UnsupportedIntraDipMode {
                mode: DIP_MODE_COUNT,
                max: DIP_MODE_COUNT - 1,
            }
        );
    }

    #[test]
    fn validates_extended_edges() {
        let err = predict_intra_dip_rect_into(
            BitDepth::Eight,
            rect_size(3, 3),
            0,
            false,
            IntraDipEdges::new(&[128u8; 7], &[128u8; 10], 128),
            &mut [0u8; 64],
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ReconError::IntraDipEdgeLengthMismatch {
                edge: IntraDipEdge::Left,
                expected: 8,
                actual: 7,
            }
        );
    }

    #[test]
    fn short_edges_repeat_their_last_sample() {
        for (log2_width, log2_height) in [(2, 2), (3, 4), (4, 3), (5, 5), (6, 4)] {
            let size = rect_size(log2_width, log2_height);
            let (width, height) = (size.width(), size.height());
            let left: Vec<u8> = (0..height + height / 4)
                .map(|i| (37 * i % 251) as u8)
                .collect();
            let above: Vec<u8> = (0..width + width / 4)
                .map(|i| (53 * i % 241) as u8)
                .collect();
            let partial = (height + (height / 4).min(1), width + (width / 4).min(3));
            for (left_len, above_len) in [(height, width), partial] {
                let extend = |edge: &[u8], len: usize| -> Vec<u8> {
                    let last = edge[len - 1];
                    (0..edge.len())
                        .map(|i| if i < len { edge[i] } else { last })
                        .collect()
                };
                let (full_left, full_above) = (extend(&left, left_len), extend(&above, above_len));
                for mode in [0, 3, 5] {
                    let mut short = vec![0u8; width * height];
                    let mut full = vec![0u8; width * height];
                    let edges = IntraDipEdges::new(&left[..left_len], &above[..above_len], 90);
                    predict_intra_dip_rect_into(
                        BitDepth::Eight,
                        size,
                        mode,
                        true,
                        edges,
                        &mut short,
                        width,
                    )
                    .unwrap();
                    let edges = IntraDipEdges::new(&full_left, &full_above, 90);
                    predict_intra_dip_rect_into(
                        BitDepth::Eight,
                        size,
                        mode,
                        true,
                        edges,
                        &mut full,
                        width,
                    )
                    .unwrap();
                    assert_eq!(
                        short, full,
                        "{width}x{height} {left_len}/{above_len} mode {mode}"
                    );
                }
            }
        }
    }
}
