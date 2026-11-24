// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Data-driven intra prediction primitive.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use crate::intra_dc_math::{
    IntraEdgeError, IntraLaneSample, load_lanes, store_lanes, validate_intra_edge_samples,
    validate_output_shape, validate_sample_type,
};
use fearless_simd::{
    Bytes as _, Simd, SimdBase, i16x16, i32x8, u8x16, u8x32, u16x8, u16x16, u32x8,
};

use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};
use splot_core::simd::SimdBackend;

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

#[derive(Clone, Copy)]
struct BaselineDip;

#[derive(Clone, Copy)]
struct TypedDip<B: SimdBackend>(B);

trait DipRunner: Copy {
    fn predict<P: IntraLaneSample>(
        self,
        block: DipBlock,
        features: &[u32; DIP_FEATURE_PAIRS],
        rows: &[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE],
        left: &[P],
        above: &[P],
        output: &mut [P],
    ) -> Option<()>;
}

impl DipRunner for BaselineDip {
    #[inline]
    fn predict<P: IntraLaneSample>(
        self,
        block: DipBlock,
        features: &[u32; DIP_FEATURE_PAIRS],
        rows: &[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE],
        left: &[P],
        above: &[P],
        output: &mut [P],
    ) -> Option<()> {
        baseline_simd!(simd => {
            predict_dip_samples(simd, block, features, rows, left, above, output)
        })
    }
}

impl<B: SimdBackend> DipRunner for TypedDip<B> {
    #[inline]
    fn predict<P: IntraLaneSample>(
        self,
        block: DipBlock,
        features: &[u32; DIP_FEATURE_PAIRS],
        rows: &[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE],
        left: &[P],
        above: &[P],
        output: &mut [P],
    ) -> Option<()> {
        let narrow = self.0.narrow();
        splot_core::with_simd!(narrow, simd => {
            predict_dip_samples(simd, block, features, rows, left, above, output)
        })
    }
}

type DipRowWeights = [[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE];

struct PreparedDipInputs {
    block: DipBlock,
    features: [u32; DIP_FEATURE_PAIRS],
    rows: &'static DipRowWeights,
}

fn prepare_dip_inputs<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    dip_mode: usize,
    dip_transpose: bool,
    edges: IntraDipEdges<'_, T>,
    output_len: usize,
    stride_samples: usize,
) -> Result<PreparedDipInputs> {
    validate_sample_type::<T>(bit_depth)?;
    let Some(rows) = DIP_ROW_WEIGHTS[usize::from(dip_transpose)].get(dip_mode) else {
        return Err(ReconError::UnsupportedIntraDipMode {
            mode: dip_mode,
            max: DIP_MODE_COUNT - 1,
        });
    };
    validate_dip_edges(bit_depth, size, edges)?;
    validate_output_shape(
        size,
        output_len,
        stride_samples,
        "DIP intra prediction output buffer length",
    )?;
    let features = dip_features(size, edges, dip_transpose);
    let block = DipBlock {
        size,
        max: i32::from(bit_depth.max_sample()),
        stride: stride_samples,
    };
    Ok(PreparedDipInputs {
        block,
        features,
        rows,
    })
}

fn predict_intra_dip_rect_into_with_runner<R: DipRunner, T: ReconSample>(
    runner: R,
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    dip_mode: usize,
    dip_transpose: bool,
    edges: IntraDipEdges<'_, T>,
    (output, stride_samples): (&mut [T], usize),
) -> Result<()> {
    let PreparedDipInputs {
        block,
        features,
        rows,
    } = prepare_dip_inputs(
        bit_depth,
        size,
        dip_mode,
        dip_transpose,
        edges,
        output.len(),
        stride_samples,
    )?;
    let shape_error = || ReconError::ArithmeticOverflow {
        context: "DIP intra prediction output shape",
    };
    macro_rules! predict {
        ($view:ident, $view_mut:ident) => {
            if let (Some(left), Some(above), Some(output)) = (
                T::$view(edges.left),
                T::$view(edges.above),
                T::$view_mut(output),
            ) {
                return runner
                    .predict(block, &features, rows, left, above, output)
                    .ok_or_else(shape_error);
            }
        };
    }
    predict!(u8_slice, u8_slice_mut);
    predict!(u16_slice, u16_slice_mut);
    Err(shape_error())
}

/// Writes rectangular DIP intra prediction into caller storage.
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
    predict_intra_dip_rect_into_with_runner(
        BaselineDip,
        bit_depth,
        size,
        dip_mode,
        dip_transpose,
        edges,
        (output, stride_samples),
    )
}

/// Typed [`predict_intra_dip_rect_into`] using caller-selected `backend`.
///
/// # Errors
/// Returns the same validation errors as [`predict_intra_dip_rect_into`].
#[expect(clippy::too_many_arguments)]
pub fn predict_intra_dip_rect_into_with<B: SimdBackend, T: ReconSample>(
    backend: B,
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    dip_mode: usize,
    dip_transpose: bool,
    edges: IntraDipEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    predict_intra_dip_rect_into_with_runner(
        TypedDip(backend),
        bit_depth,
        size,
        dip_mode,
        dip_transpose,
        edges,
        (output, stride_samples),
    )
}

/// The geometry of one DIP block.
#[derive(Clone, Copy)]
struct DipBlock {
    size: IntraRectBlockSize,
    max: i32,
    stride: usize,
}

#[inline]
fn predict_dip_samples<S: Simd, P: IntraLaneSample>(
    simd: S,
    block: DipBlock,
    features: &[u32; DIP_FEATURE_PAIRS],
    rows: &[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE],
    left: &[P],
    above: &[P],
    output: &mut [P],
) -> Option<()> {
    let inputs = (features, rows, left, above);
    match block.size.log2_width() {
        2 => dip_bands::<S, P, 4, 1>(simd, block, inputs, output),
        3 => dip_bands::<S, P, 8, 1>(simd, block, inputs, output),
        4 => dip_bands::<S, P, 16, 1>(simd, block, inputs, output),
        5 => dip_bands::<S, P, 32, 2>(simd, block, inputs, output),
        _ => dip_bands::<S, P, 64, 4>(simd, block, inputs, output),
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
) -> [u32; DIP_FEATURE_PAIRS] {
    let above = dip_averages(edges.above, size.width(), size.log2_width());
    let left = dip_averages(edges.left, size.height(), size.log2_height());
    let (x, y) = if dip_transpose {
        (left, above)
    } else {
        (above, left)
    };
    let pack = |low, high| u32::from(low) | (u32::from(high) << 16);
    [
        pack(edges.top_left.to_u16(), x[0]),
        pack(x[1], x[2]),
        pack(x[3], y[0]),
        pack(y[1], y[2]),
        pack(y[3], x[4]),
        u32::from(y[4]),
    ]
}

fn dip_averages<T: ReconSample>(samples: &[T], block_len: usize, log2_len: u8) -> [u16; 5] {
    /// The sums of the first five `N`-sample groups of `samples`, which
    /// holds four to five groups; the samples it lacks repeat its last.
    #[inline]
    fn group_sums<T: ReconSample, const N: usize>(samples: &[T]) -> [i32; 5] {
        let value = |sample: &T| i32::from(sample.to_u16());
        let sum = |group: &[T; N]| group.iter().map(value).sum::<i32>();
        let (groups, rest) = samples.as_chunks::<N>();
        let Some(head) = groups.first_chunk::<4>() else {
            return [0; 5];
        };
        let last = if let Some(group) = groups.get(4) {
            sum(group)
        } else {
            let repeat = samples.last().map_or(0, value);
            rest.iter().map(value).sum::<i32>() + (N - rest.len()) as i32 * repeat
        };
        let [a, b, c, d] = head.each_ref().map(sum);
        [a, b, c, d, last]
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
    sums.map(|sum| ((sum + round) >> group_log2) as u16)
}

/// `DIP_WEIGHTS[mode][index]` of one grid row, interleaved for a pairwise
/// multiply-add: lane `2 * gx + j` of entry `k` holds the weight of feature
/// `2 * k + j` at grid column `gx` (feature 11 is zero). Indexed
/// `[transpose][mode][iy][k]`.
static DIP_ROW_WEIGHTS: [[[[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE]; DIP_MODE_COUNT]; 2] =
    dip_row_weights();

const DIP_FEATURE_PAIRS: usize = DIP_FEATURE_COUNT.div_ceil(2);

#[expect(
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

type DipInputs<'a, P> = (
    &'a [u32; DIP_FEATURE_PAIRS],
    &'a [[[i16; 16]; DIP_FEATURE_PAIRS]; DIP_GRID_SIDE],
    &'a [P],
    &'a [P],
);

/// Writes the block band by band in § 7.13.2.6 order. Each sampled row is the
/// `Clip1(Round2(Σ features * weights, 10))` grid row, horizontally
/// interpolated from the left edge sample; the rows before it are vertically
/// interpolated from the row (or above edge) before them. Every result is a
/// convex combination of two samples with power-of-two weights, so it needs
/// no clipping, and a between row's `a * step + z * (b - a)` is a wrapping
/// running sum that stays exact.
#[inline]
fn dip_bands<S: Simd, P: IntraLaneSample, const W: usize, const C: usize>(
    simd: S,
    DipBlock { size, max, stride }: DipBlock,
    (features, rows, left, above): DipInputs<'_, P>,
    output: &mut [P],
) -> Option<()> {
    let height = size.height();
    let up_x = W.trailing_zeros().saturating_sub(3);
    let up_y = u32::from(size.log2_height().max(3) - 3);
    let step = 1usize << up_y;
    let pairs: [i16x16<S>; DIP_FEATURE_PAIRS] =
        features.map(|pair| u32x8::splat(simd, pair).bitcast());
    let [w0, w1] = DIP_LERP_WEIGHTS[up_x as usize].map(|w| u16x16::from_slice(simd, &w));
    let mut row_start = 0;
    let mut before = if step > 1 {
        load_lanes::<S, P, W, C>(simd, above.first_chunk::<W>()?)?
    } else {
        [u16x16::splat(simd, 0); C]
    };
    let down_y = usize::from(height < DIP_GRID_SIDE) + 1;
    for band in 0..height.min(DIP_GRID_SIDE) {
        let weights = rows.get(band * down_y)?;
        let mut sum = i32x8::splat(simd, 1 << 9);
        for (pair, weight) in pairs.iter().zip(weights) {
            sum += simd.pairwise_dot_product_i16x16(*pair, i16x16::from_slice(simd, weight));
        }
        let (low, high) = simd.split_u32x8((sum >> 10).max(0).min(max).bitcast());
        let grid = simd.relaxed_narrow_u32x4(low, high);
        let sampled = if W <= 8 {
            let grid = if W == 4 {
                let even =
                    u8x16::load_array(simd, [0, 1, 4, 5, 8, 9, 12, 13, 0, 1, 4, 5, 8, 9, 12, 13]);
                simd.swizzle_dyn_u8x16(grid.bitcast(), even).bitcast()
            } else {
                grid
            };
            [simd.combine_u16x8(grid, grid); C]
        } else {
            let edge = u16x8::splat(simd, left.get((band + 1) * step - 1)?.to_u16());
            let near = simd.slide_u16x8::<7>(edge, grid);
            if W == 16 {
                let middle = (near + grid) >> 1;
                let (low, high) = simd.interleave_u16x8(middle, grid);
                [simd.combine_u16x8(low, high); C]
            } else {
                let near = simd.combine_u16x8(near, near);
                let far = simd.combine_u16x8(grid, grid);
                let indices = &DIP_POINT_INDEX[up_x as usize - 2];
                core::array::from_fn(|k| {
                    let index = u8x32::from_slice(simd, &indices[k]);
                    let near = simd.swizzle_dyn_within_blocks_u16x16(near, index);
                    let far = simd.swizzle_dyn_within_blocks_u16x16(far, index);
                    (near * w0 + far * w1) >> up_x
                })
            }
        };
        if step > 1 {
            let delta: [u16x16<S>; C] = core::array::from_fn(|k| sampled[k] - before[k]);
            let mut lerp = before.map(|a| a << up_y);
            for _ in 1..step {
                for (lerp, delta) in lerp.iter_mut().zip(delta) {
                    *lerp += delta;
                }
                let row = output.get_mut(row_start..)?.first_chunk_mut::<W>()?;
                row_start += stride;
                store_lanes::<S, P, W, C>(simd, lerp.map(|v| v >> up_y), row)?;
            }
        }
        let row = output.get_mut(row_start..)?.first_chunk_mut::<W>()?;
        row_start += stride;
        store_lanes::<S, P, W, C>(simd, sampled, row)?;
        before = sampled;
    }
    Some(())
}

/// `DIP_LERP_WEIGHTS[up]` holds the weights `[upx - w1, w1]` of the points
/// before and after each lane, `w1 = lane mod upx + 1` with `upx = 1 << up`.
static DIP_LERP_WEIGHTS: [[[u16; 16]; 2]; 4] = {
    let mut table = [[[0u16; 16]; 2]; 4];
    let mut up = 0;
    while up < 4 {
        let mut lane = 0;
        while lane < 16 {
            let w1 = (lane & ((1 << up) - 1)) + 1;
            table[up][0][lane] = ((1 << up) - w1) as u16;
            table[up][1][lane] = w1 as u16;
            lane += 1;
        }
        up += 1;
    }
    table
};

/// `DIP_POINT_INDEX[log2(w) - 5][k]` holds, for samples `16 * k..16 * k + 16`
/// of a sampled row, the byte indices of the point before each sample in
/// `[left, grid[0..7]]`, which are those of the point after it in `grid`.
static DIP_POINT_INDEX: [[[u8; 32]; 4]; 2] = {
    let mut table = [[[0u8; 32]; 4]; 2];
    let mut class = 0;
    while class < 2 {
        let mut k = 0;
        while k < 4 {
            let mut byte = 0;
            while byte < 32 {
                let x = 16 * k + (byte % 16) / 2 + 8 * (byte / 16);
                let point = (x >> (class + 2)) % 8;
                table[class][k][byte] = (2 * point + byte % 2) as u8;
                byte += 1;
            }
            k += 1;
        }
        class += 1;
    }
    table
};
#[cfg(test)]
mod tests {
    use super::*;

    fn rect_size(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    #[expect(clippy::many_single_char_names, reason = "the § 7.13.2.4-6 names")]
    fn spec_dip(
        (w, h, max): (usize, usize, i32),
        (mode, transpose): (usize, bool),
        left: &[u16],
        above: &[u16],
        top_left: u16,
    ) -> Vec<i32> {
        let avg = |edge: &[u16], n: usize| -> [i32; 5] {
            let down = n >> 2;
            core::array::from_fn(|i| {
                let t: i32 = (0..down).map(|j| i32::from(edge[i * down + j])).sum();
                (t + (down as i32 >> 1)) / down as i32
            })
        };
        let (fa, fl) = (avg(above, w), avg(left, h));
        let (first, second) = if transpose { (fl, fa) } else { (fa, fl) };
        let mut f = [i32::from(top_left); 11];
        for i in 0..4 {
            (f[i + 1], f[i + 5]) = (first[i], second[i]);
        }
        (f[9], f[10]) = (first[4], second[4]);
        let mut grid = [[0i32; 8]; 8];
        for i in 0..8 {
            for j in 0..8 {
                let weights = &splot_tables::tables::loop_restoration::DIP_WEIGHTS[mode][i * 8 + j];
                let c: i32 = (0..11).map(|k| weights[k] * f[k]).sum();
                let v = ((c + 512) >> 10).clamp(0, max);
                if transpose {
                    grid[j][i] = v;
                } else {
                    grid[i][j] = v;
                }
            }
        }
        let (upx, upy, downx, downy) = (
            (w / 8).max(1),
            (h / 8).max(1),
            (8 / w).max(1),
            (8 / h).max(1),
        );
        let mut horz = vec![vec![0i32; w]; 8];
        for i in 0..h.min(8) {
            let y = (i + 1) * upy - 1;
            for j in 0..w.min(8) {
                let p0 = if j == 0 {
                    i32::from(left[y])
                } else {
                    grid[i * downy][(j - 1) * downx]
                };
                let p1 = grid[i * downy][j * downx];
                for k in 0..upx {
                    let w1 = (k + 1) as i32;
                    horz[i][j * upx + k] = ((upx as i32 - w1) * p0 + w1 * p1) / upx as i32;
                }
            }
        }
        let mut pred = vec![0i32; w * h];
        for x in 0..w {
            for i in 0..h.min(8) {
                let p0 = if i == 0 {
                    i32::from(above[x])
                } else {
                    horz[i - 1][x]
                };
                let p1 = horz[i][x];
                for k in 0..upy {
                    let w1 = (k + 1) as i32;
                    pred[(i * upy + k) * w + x] = ((upy as i32 - w1) * p0 + w1 * p1) / upy as i32;
                }
            }
        }
        pred
    }

    fn check_against_spec<T: ReconSample + TryFrom<u16>>(bit_depth: BitDepth) {
        let max = bit_depth.max_sample();
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = |bound: u16| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as u32 % (u32::from(bound) + 1)) as u16
        };
        for (log2_width, log2_height) in (2..=6).flat_map(|w| (2..=6).map(move |h| (w, h))) {
            let size = rect_size(log2_width, log2_height);
            let (w, h) = (size.width(), size.height());
            for pattern in 0..6 {
                let mut sample = |i: usize| match pattern {
                    0 => max,
                    1 => 0,
                    2 => [max, 0][i % 2],
                    _ => next(max),
                };
                let left: Vec<u16> = (0..h + h / 4).map(&mut sample).collect();
                let above: Vec<u16> = (0..w + w / 4).map(&mut sample).collect();
                let top_left = sample(1);
                let cast = |edge: &[u16]| -> Vec<T> {
                    edge.iter().map(|&v| T::try_from(v).ok().unwrap()).collect()
                };
                let (left_t, above_t) = (cast(&left), cast(&above));
                let corner = T::try_from(top_left).ok().unwrap();
                for mode in 0..DIP_MODE_COUNT {
                    for transpose in [false, true] {
                        let stride = w + 3;
                        let sentinel = T::try_from(max / 3).ok().unwrap();
                        let mut output = vec![sentinel; stride * h];
                        predict_intra_dip_rect_into(
                            bit_depth,
                            size,
                            mode,
                            transpose,
                            IntraDipEdges::new(&left_t, &above_t, corner),
                            &mut output,
                            stride,
                        )
                        .unwrap();
                        let want = spec_dip(
                            (w, h, i32::from(max)),
                            (mode, transpose),
                            &left,
                            &above,
                            top_left,
                        );
                        for y in 0..h {
                            for x in 0..stride {
                                let got = i32::from(output[y * stride + x].to_u16());
                                let expected = if x < w {
                                    want[y * w + x]
                                } else {
                                    i32::from(max / 3)
                                };
                                assert_eq!(
                                    got, expected,
                                    "{w}x{h} p{pattern} m{mode} t{transpose} ({x},{y})"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn matches_the_spec_process_at_8_and_10_bit() {
        check_against_spec::<u8>(BitDepth::Eight);
        check_against_spec::<u16>(BitDepth::Ten);
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

    #[test]
    fn typed_backend_matches_compatibility_route() {
        use splot_core::simd::{SimdBackend, SimdPolicy, SimdScope, resolve_backend};

        struct Check;
        impl SimdScope<()> for Check {
            fn run<B: SimdBackend>(self, backend: B) {
                for (log2_w, log2_h) in [(2, 2), (3, 4), (4, 3)] {
                    let size = rect_size(log2_w, log2_h);
                    let stride = size.width() + 4;
                    let left8: Vec<u8> = (0..=size.height()).map(|i| (i * 31) as u8).collect();
                    let above8: Vec<u8> = (0..=size.width()).map(|i| (i * 43) as u8).collect();
                    let edges8 = IntraDipEdges::new(&left8[..], &above8[..], 80);
                    let mut compat8 = vec![5u8; stride * size.height()];
                    let mut typed8 = Vec::from(compat8.as_slice());
                    predict_intra_dip_rect_into(
                        BitDepth::Eight,
                        size,
                        2,
                        false,
                        edges8,
                        &mut compat8,
                        stride,
                    )
                    .unwrap();
                    predict_intra_dip_rect_into_with(
                        backend,
                        BitDepth::Eight,
                        size,
                        2,
                        false,
                        edges8,
                        &mut typed8,
                        stride,
                    )
                    .unwrap();
                    assert_eq!(typed8, compat8);

                    let left10: Vec<u16> = (0..=size.height())
                        .map(|i| ((i * 67) % 1024) as u16)
                        .collect();
                    let above10: Vec<u16> = (0..=size.width())
                        .map(|i| ((i * 89) % 1024) as u16)
                        .collect();
                    let edges10 = IntraDipEdges::new(&left10[..], &above10[..], 512);
                    let mut compat10 = vec![9u16; stride * size.height()];
                    let mut typed10 = Vec::from(compat10.as_slice());
                    predict_intra_dip_rect_into(
                        BitDepth::Ten,
                        size,
                        4,
                        true,
                        edges10,
                        &mut compat10,
                        stride,
                    )
                    .unwrap();
                    predict_intra_dip_rect_into_with(
                        backend,
                        BitDepth::Ten,
                        size,
                        4,
                        true,
                        edges10,
                        &mut typed10,
                        stride,
                    )
                    .unwrap();
                    assert_eq!(typed10, compat10);
                }
            }
        }

        resolve_backend(SimdPolicy::Auto)
            .expect("auto backend")
            .enter(Check);
    }
}
