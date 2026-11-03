// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Data-driven intra prediction primitive.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use crate::intra_dc_math::{
    IntraEdgeError, IntraLaneSample, validate_intra_edge_samples, validate_output_shape,
    validate_sample_type,
};
use core::simd::cmp::SimdOrd;
use core::simd::num::SimdInt;
use core::simd::{Simd, Swizzle, simd_swizzle};

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
    /// `left` must contain `height + height / 4` samples and `above` must
    /// contain `width + width / 4` samples. Availability and fallback
    /// preparation remain outside this type and are owned by the broader AV2
    /// §7.13.2.1 intra process.
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
    predict_coarse_grid(
        bit_depth,
        size,
        dip_mode,
        dip_transpose,
        features,
        output,
        stride_samples,
    )?;
    interpolate_grid(size, edges, output, stride_samples)?;
    Ok(())
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
    validate_intra_edge_samples(
        IntraDipEdge::Left,
        edges.left,
        size.height() + (size.height() >> 2),
        bit_depth,
    )?;
    validate_intra_edge_samples(
        IntraDipEdge::Above,
        edges.above,
        size.width() + (size.width() >> 2),
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
    let group_len = block_len >> 2;
    let group_log2 = log2_len - 2;
    let round = block_len >> 3;
    let mut averages = [0i32; 5];
    for (group, average) in averages.iter_mut().enumerate() {
        let start = group * group_len;
        let end = start + group_len;
        let sum = samples[start..end]
            .iter()
            .fold(0i32, |acc, sample| acc + i32::from(sample.to_u16()));
        *average = (sum + round as i32) >> group_log2;
    }
    averages
}

/// `DIP_WEIGHTS[mode][index]` of one grid row, interleaved for a pairwise
/// multiply-add: lane `2 * gx + j` of entry `k` holds the weight of feature
/// `2 * k + j` at grid column `gx` (feature 11 is zero). Indexed
/// `[transpose][mode][iy][k]`.
static DIP_ROW_WEIGHTS: [[[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE]; DIP_MODE_COUNT]; 2] =
    dip_row_weights();

const DIP_FEATURE_PAIRS: usize = DIP_FEATURE_COUNT.div_ceil(2);

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

/// Writes the § 7.13.2.3 coarse grid samples: every grid row is eight dot
/// products of the features with that row's weights, one pairwise
/// multiply-add per feature pair.
fn predict_coarse_grid<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    dip_mode: usize,
    dip_transpose: bool,
    features: [i32; DIP_FEATURE_COUNT],
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let grid = dip_grid(size);
    let Some(rows) = DIP_ROW_WEIGHTS[usize::from(dip_transpose)].get(dip_mode) else {
        return Err(ReconError::UnsupportedIntraDipMode {
            mode: dip_mode,
            max: DIP_MODE_COUNT - 1,
        });
    };
    let pairs: [Simd<i16, 16>; DIP_FEATURE_PAIRS] = core::array::from_fn(|k| {
        let low = features[2 * k] as i16;
        let high = features.get(2 * k + 1).map_or(0, |&f| f as i16);
        Simd::from_array(core::array::from_fn(
            |lane| if lane % 2 == 0 { low } else { high },
        ))
    });
    let max = Simd::splat(i32::from(bit_depth.max_sample()));
    let mut y = grid.step_y - 1;
    for gy in 0..grid.height {
        let weights = &rows[gy * grid.down_y];
        let mut sum = Simd::<i32, 8>::splat(1 << 9);
        for (pair, weight) in pairs.iter().zip(weights) {
            let product = pair.cast::<i32>() * Simd::<i16, 16>::from_array(*weight).cast::<i32>();
            sum += simd_swizzle!(product, [0, 2, 4, 6, 8, 10, 12, 14])
                + simd_swizzle!(product, [1, 3, 5, 7, 9, 11, 13, 15]);
        }
        let values = (sum >> 10)
            .simd_max(Simd::splat(0))
            .simd_min(max)
            .to_array();
        let row = output
            .get_mut(y * stride_samples..)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "DIP intra prediction coarse grid row",
            })?;
        let mut x = grid.step_x - 1;
        for gx in 0..grid.width {
            if let Some(slot) = row.get_mut(x) {
                *slot = T::try_from_u16(values[gx * grid.down_x] as u16)?;
            }
            x += grid.step_x;
        }
        y += grid.step_y;
    }
    Ok(())
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

fn interpolate_grid<T: ReconSample>(
    size: IntraRectBlockSize,
    edges: IntraDipEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let grid = dip_grid(size);
    let shape_error = || ReconError::ArithmeticOverflow {
        context: "DIP intra prediction interpolation shape",
    };
    macro_rules! interpolate {
        ($view:ident, $view_mut:ident, $sample:ty) => {
            if let (Some(left), Some(above), Some(output)) = (
                T::$view(edges.left),
                T::$view(edges.above),
                T::$view_mut(output),
            ) {
                return interpolate_samples::<$sample>(
                    size,
                    grid,
                    left,
                    above,
                    output,
                    stride_samples,
                )
                .ok_or_else(shape_error);
            }
        };
    }
    interpolate!(u8_slice, u8_slice_mut, u8);
    interpolate!(u16_slice, u16_slice_mut, u16);
    Err(shape_error())
}

/// Fills the samples between the coarse grid: each sampled row horizontally
/// from the left edge, then the rows between sampled rows from the row or
/// above edge before them. Every result is a convex combination of two
/// samples with power-of-two weights, so it needs no clipping.
fn interpolate_samples<P: IntraLaneSample>(
    size: IntraRectBlockSize,
    grid: DipGrid,
    left: &[P],
    above: &[P],
    output: &mut [P],
    stride: usize,
) -> Option<()> {
    let width = size.width();
    if grid.step_x > 1 {
        for band in 0..grid.height {
            let y = band * grid.step_y + grid.step_y - 1;
            let row = output.get_mut(y * stride..)?.get_mut(..width)?;
            let mut points = [0u16; 16];
            points[0] = left.get(y)?.to_u16();
            for (gx, point) in points[1..=DIP_GRID_SIDE].iter_mut().enumerate() {
                *point = row.get(gx * grid.step_x + grid.step_x - 1)?.to_u16();
            }
            let points = Simd::from_array(points);
            match grid.up_log2_x {
                1 => expand_grid_row::<1, 0, P>(points, row),
                2 => {
                    expand_grid_row::<2, 0, P>(points, row);
                    expand_grid_row::<2, 1, P>(points, row);
                }
                3 => {
                    expand_grid_row::<3, 0, P>(points, row);
                    expand_grid_row::<3, 1, P>(points, row);
                    expand_grid_row::<3, 2, P>(points, row);
                    expand_grid_row::<3, 3, P>(points, row);
                }
                _ => return None,
            }
        }
    }
    if grid.step_y > 1 {
        let shift = Simd::splat(u16::from(grid.up_log2_y));
        for band in 0..grid.height {
            let top = band * grid.step_y;
            let (before, rest) = output.split_at_mut_checked(top * stride)?;
            let p0 = if band == 0 {
                above.get(..width)?
            } else {
                before.get((top - 1) * stride..)?.get(..width)?
            };
            let (between, last) = rest.split_at_mut_checked((grid.step_y - 1) * stride)?;
            let p1 = last.get(..width)?;
            for z in 0..grid.step_y - 1 {
                let w0 = Simd::splat((grid.step_y - z - 1) as u16);
                let w1 = Simd::splat(z as u16 + 1);
                let row = between.get_mut(z * stride..)?.get_mut(..width)?;
                lerp_row(p0, p1, w0, w1, shift, row);
            }
        }
    }
    Some(())
}

/// Lane `c` of [`expand_grid_row`] chunk `K` reads grid point
/// `((16 * K + c) >> UP) + B` (point 0 is the left edge sample).
struct GridPoint<const UP: u32, const K: usize, const B: usize>;

impl<const UP: u32, const K: usize, const B: usize> Swizzle<16> for GridPoint<UP, K, B> {
    const INDEX: [usize; 16] = {
        let mut index = [0; 16];
        let mut lane = 0;
        while lane < 16 {
            index[lane] = ((16 * K + lane) >> UP) + B;
            lane += 1;
        }
        index
    };
}

/// Interpolates samples `16 * K..16 * K + 16` of a sampled row from its grid
/// `points`, `1 << UP` samples per grid step.
#[allow(clippy::inline_always, reason = "keeps the swizzles constant")]
#[inline(always)]
fn expand_grid_row<const UP: u32, const K: usize, P: IntraLaneSample>(
    points: Simd<u16, 16>,
    row: &mut [P],
) {
    let step = 1u16 << UP;
    let w1 = Simd::from_array(core::array::from_fn(|lane| (lane as u16 & (step - 1)) + 1));
    let w0 = Simd::splat(step) - w1;
    let value = (GridPoint::<UP, K, 0>::swizzle(points) * w0
        + GridPoint::<UP, K, 1>::swizzle(points) * w1)
        >> Simd::splat(UP as u16);
    if let Some(out) = row.get_mut(16 * K..).and_then(<[P]>::first_chunk_mut) {
        P::store_lanes(value, out);
    }
}

/// `(p0 * w0 + p1 * w1) >> shift` over a row.
fn lerp_row<P: IntraLaneSample>(
    p0: &[P],
    p1: &[P],
    w0: Simd<u16, 16>,
    w1: Simd<u16, 16>,
    shift: Simd<u16, 16>,
    row: &mut [P],
) {
    let mut out = row.chunks_exact_mut(16);
    let mut a = p0.chunks_exact(16);
    let mut b = p1.chunks_exact(16);
    for ((out, a), b) in out.by_ref().zip(a.by_ref()).zip(b.by_ref()) {
        if let (Some(out), Some(a), Some(b)) =
            (out.first_chunk_mut(), a.first_chunk(), b.first_chunk())
        {
            P::store_lanes((P::load(a) * w0 + P::load(b) * w1) >> shift, out);
        }
    }
    let narrow = |v: Simd<u16, 16>| simd_swizzle!(v, [0, 1, 2, 3]);
    for ((out, a), b) in out
        .into_remainder()
        .chunks_exact_mut(4)
        .zip(a.remainder().chunks_exact(4))
        .zip(b.remainder().chunks_exact(4))
    {
        if let (Some(out), Some(a), Some(b)) =
            (out.first_chunk_mut(), a.first_chunk(), b.first_chunk())
        {
            let value = (P::load::<4>(a) * narrow(w0) + P::load(b) * narrow(w1)) >> narrow(shift);
            P::store_lanes(value, out);
        }
    }
}

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
            IntraDipEdges::new(&[128u8; 9], &[128u8; 10], 128),
            &mut [0u8; 64],
            8,
        )
        .unwrap_err();
        assert_eq!(
            err,
            ReconError::IntraDipEdgeLengthMismatch {
                edge: IntraDipEdge::Left,
                expected: 10,
                actual: 9,
            }
        );
    }
}
