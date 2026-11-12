// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use fearless_simd::{
    Simd, SimdIntWiden, i8x16, i16x8, i16x16, i32x8, mask16x8, prelude::*, u8x16, u8x32, u16x8,
    u16x16,
};

use splot_core::tables::loop_restoration::{
    GDF_ALPHA, GDF_INTER_ERROR, GDF_INTRA_ERROR, GDF_WEIGHT,
};
use splot_recon::BitDepth;
use splot_recon::math::round2_signed_i32;

use crate::Result;

#[cfg(target_arch = "x86_64")]
#[path = "gdf_inlane.rs"]
mod inlane;

use super::{
    GDF_BIAS, GDF_COORDS, GDF_INTRA_REF_DST, GDF_READ_RADIUS, GdfBlock, GdfClass, GdfSource,
    exact_slice, gdf_state_error,
};

pub(super) const TAP_REACH: usize = GDF_READ_RADIUS - 1;
/// Source rows a row pair reads: `TAP_REACH` above the pair to `TAP_REACH` below it.
pub(super) const WINDOW_ROWS: usize = 2 * TAP_REACH + 2;

/// The source rows a row pair reads, each `len` samples from column `origin.0 - TAP_REACH`.
/// Inlined so callers see that every row has length `len`.
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn source_rows<'a>(
    source: &GdfSource<'a>,
    origin: (usize, usize),
    len: usize,
) -> Result<[&'a [u16]; WINDOW_ROWS]> {
    let first_row = origin.1.checked_sub(TAP_REACH);
    let first_col = origin.0.checked_sub(TAP_REACH);
    let (Some(first_row), Some(first_col)) = (first_row, first_col) else {
        return Err(gdf_state_error());
    };
    let mut rows = [&source.samples[..0]; WINDOW_ROWS];
    for (index, row) in rows.iter_mut().enumerate() {
        let start = (first_row + index)
            .checked_mul(source.stride)
            .and_then(|start| start.checked_add(first_col))
            .ok_or_else(gdf_state_error)?;
        *row = exact_slice(source.samples, start, len).ok_or_else(gdf_state_error)?;
    }
    Ok(rows)
}

/// The `WIN` samples of every row in `rows` from column `x`.
#[inline]
pub(super) fn window_at<'a, const WIN: usize>(
    rows: &[&'a [u16]; WINDOW_ROWS],
    x: usize,
) -> Option<[&'a [u16; WIN]; WINDOW_ROWS]> {
    let end = x.checked_add(WIN)?;
    if rows.iter().any(|row| row.len() < end) {
        return None;
    }
    Some(rows.map(|row| {
        row.get(x..)
            .and_then(<[u16]>::first_chunk)
            .unwrap_or(const { &[0; WIN] })
    }))
}

/// The window of a 16-sample row pair: its rows with the tap reach on both sides.
type Window<'a> = [&'a [u16; 28]; WINDOW_ROWS];

/// The most columns of one column tile of a run, whose source rows sit
/// [`TILE_STRIDE`] apart: the tile's rows of a row pair window stay in L1, and
/// every window row is a constant offset from the window's first sample.
pub(super) const TILE_COLS: usize = 1024;
pub(super) const TILE_STRIDE: usize = TILE_COLS + 32;
const TILE_SPAN: usize = (WINDOW_ROWS - 1) * TILE_STRIDE + 28;
const _: () =
    assert!(TILE_STRIDE >= TILE_COLS + 2 * GDF_READ_RADIUS && TILE_COLS.is_multiple_of(16));

/// The rows of a row pair window.
trait WindowRows {
    fn row(&self, index: usize) -> &[u16; 28];
}

impl WindowRows for Window<'_> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row(&self, index: usize) -> &[u16; 28] {
        self.get(index).copied().unwrap_or(&[0; 28])
    }
}

/// A row pair window of a column tile, its rows [`TILE_STRIDE`] apart.
struct TiledWindow<'a>(&'a [u16; TILE_SPAN]);

impl WindowRows for TiledWindow<'_> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row(&self, index: usize) -> &[u16; 28] {
        self.0
            .get(index * TILE_STRIDE..)
            .and_then(<[u16]>::first_chunk)
            .unwrap_or(&[0; 28])
    }
}

/// The largest column distance of a tap at each row distance.
const TAP_COLUMN_REACH: [usize; TAP_REACH + 1] = {
    let mut reach = [0; TAP_REACH + 1];
    let mut k = 0;
    while k < GDF_COORDS.len() {
        let (dy, dx) = GDF_COORDS[k];
        if dx.unsigned_abs() > reach[dy as usize] {
            reach[dy as usize] = dx.unsigned_abs();
        }
        k += 1;
    }
    reach
};

/// The largest minus the smallest sample that the row pair's taps read, the
/// window's `tap_range`, or [`ROUGH`] once it exceeds `limit`. The
/// center rows go first: in textured windows they alone usually exceed it.
/// The rows are unrolled so that each row's tap reach is a constant.
#[inline]
fn window_range<S: Simd, W: WindowRows>(simd: S, window: &W, limit: u16) -> u16 {
    let (mut low, mut high) = (u16x16::splat(simd, u16::MAX), u16x16::splat(simd, 0));
    let range = |low: u16x16<S>, high: u16x16<S>| {
        simd.reduce_max_u16x16(high) - simd.reduce_min_u16x16(low)
    };
    macro_rules! rows_range {
        ($($row:literal)*) => {
            $({
                const ROW: usize = $row;
                const REACH: usize = {
                    let (above, below) = (ROW.abs_diff(TAP_REACH), ROW.abs_diff(TAP_REACH + 1));
                    TAP_COLUMN_REACH[if above < below { above } else { below }]
                };
                let row = window.row(ROW);
                for col in [TAP_REACH - REACH, TAP_REACH + REACH] {
                    let samples = u16x16::from_slice(simd, &row[col..col + 16]);
                    low = low.min(samples);
                    high = high.max(samples);
                }
            })*
        };
    }
    rows_range!(6 7);
    if range(low, high) > limit {
        return ROUGH;
    }
    rows_range!(0 1 2 3 4 5 8 9 10 11 12 13);
    match range(low, high) {
        range if range > limit => ROUGH,
        range => range,
    }
}

/// Bit `lane` in each lane.
const LANE_BITS: [u16; 16] = {
    let mut bits = [0; 16];
    let mut lane = 0;
    while lane < 16 {
        bits[lane] = 1 << lane;
        lane += 1;
    }
    bits
};

/// The widest `tap_range` whose combs may skip the § 7.20.5 comb clip:
/// two tap differences within it sum to at most 510.
const LINEAR_RANGE: u16 = 255;
/// The widest `tap_range` at which some tap's differences may skip the
/// § 7.20.5 clip: the largest clip bound.
const CLIP_RANGE: u16 = 511;
/// [`window_range`] of a window wider than [`CLIP_RANGE`].
const ROUGH: u16 = u16::MAX;

/// The taps of each group that a class's transpose or mirror permutes: a
/// group clips as one.
const TAP_GROUPS: [&[usize]; 8] = [
    &[0, 12],
    &[1, 13],
    &[2, 14],
    &[3, 15],
    &[4, 6, 7, 11],
    &[8, 10],
    &[9, 17],
    &[5, 16],
];

/// The error tables as `i8`, the inter ones padded to a power of two so
/// that a masked index needs no bounds check.
static GDF_INTRA_ERROR_I8: [[i8; 4096]; 6] = {
    let mut table = [[0; 4096]; 6];
    let mut index = 0;
    while index < 6 * 4096 {
        let value = GDF_INTRA_ERROR[index / 4096][index % 4096];
        assert!(value == value as i8 as i32);
        table[index / 4096][index % 4096] = value as i8;
        index += 1;
    }
    table
};
static GDF_INTER_ERROR_I8: [[[i8; 1024]; 6]; 5] = {
    let mut table = [[[0; 1024]; 6]; 5];
    let mut index = 0;
    while index < 5 * 6 * 1000 {
        let (reference, qp, entry) = (index / 6000, index / 1000 % 6, index % 1000);
        let value = GDF_INTER_ERROR[reference][qp][entry];
        assert!(value == value as i8 as i32);
        table[reference][qp][entry] = value as i8;
        index += 1;
    }
    table
};

/// The § 7.20.5 `err` of `block`'s table at each error index of `pos`, as
/// bytes: a byte lookup inserts straight from the table.
#[allow(clippy::inline_always)]
#[inline(always)]
fn errors(block: &GdfBlock, pos: &[u16; 16]) -> [i8; 16] {
    let table = error_table(block).unwrap_or_default();
    pos.map(|pos| {
        table
            .get(usize::from(pos) & (table.len() - 1))
            .copied()
            .unwrap_or(0)
    })
}

/// `block`'s § 7.20.5 error table, a power of two long.
fn error_table(block: &GdfBlock) -> Option<&'static [i8]> {
    if block.ref_dst_idx == GDF_INTRA_REF_DST {
        Some(GDF_INTRA_ERROR_I8.get(block.qp_idx)?)
    } else {
        Some(
            GDF_INTER_ERROR_I8
                .get(block.ref_dst_idx - 1)?
                .get(block.qp_idx)?,
        )
    }
}

/// The by-class clip bounds and weights of one GDF table, with its bias and
/// the § 7.20.5 `scale`.
pub(super) struct GdfSymParams {
    /// Each `TAP_GROUPS` entry's widest `tap_range` at which its combs skip
    /// the clips (its smallest clip bound, at most [`LINEAR_RANGE`]), then its
    /// smallest clip bound: a window marks the bounds below its `tap_range`.
    group_bounds: [u16; 16],
    /// The clip bounds of taps `2j` and `2j + 1` by class, for [`rough_half`].
    lane_alpha: [[u16; 8]; GDF_COORDS.len() / 2],
    /// Each sum's weights of taps `2j` and `2j + 1` by class times
    /// `2 * scale`, with sums 0 and 1 exchanged for odd classes.
    lane_weights: [[[i16; 8]; GDF_COORDS.len() / 2]; 3],
    bias: [i32; 3],
    scale: i16,
    /// The tables of the x86 slot kernel.
    #[cfg(target_arch = "x86_64")]
    slot: inlane::SlotParams,
    /// Whether a flat window with gradient bias 0 keeps its samples.
    keeps_flat: bool,
}

impl GdfSymParams {
    pub(super) fn new(block: &GdfBlock) -> Self {
        let alpha = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx];
        let table = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx];
        let scale: i16 = if block.ref_dst_idx == GDF_INTRA_REF_DST {
            8
        } else {
            5
        };
        let bias = GDF_BIAS[block.ref_dst_idx][block.qp_idx];
        let flat_pos = bias.into_iter().fold(0, |pos, bias| {
            let scale = i32::from(scale);
            let digit = round2_signed_i32(bias * scale, 15).clamp(-scale, scale - 1);
            pos * 2 * scale + digit + scale
        });
        let flat_error = i16::from(errors(block, &[flat_pos as u16; 16])[0]);
        let rounding = 12 - u32::from(block.bit_depth.bits());
        Self {
            group_bounds: core::array::from_fn(|lane| {
                let taps = TAP_GROUPS[lane % TAP_GROUPS.len()];
                let bound = if lane < TAP_GROUPS.len() {
                    LINEAR_RANGE
                } else {
                    u16::MAX
                };
                taps.iter().fold(bound, |bound, &k| bound.min(alpha[k][0]))
            }),
            lane_alpha: core::array::from_fn(|pair| {
                core::array::from_fn(|lane| alpha[2 * pair + lane / 4][lane % 4])
            }),
            lane_weights: core::array::from_fn(|sum| {
                core::array::from_fn(|pair| {
                    core::array::from_fn(|lane| {
                        let class = lane % 4;
                        let sum = if sum < 2 { sum ^ (class & 1) } else { sum };
                        table[sum][2 * pair + lane / 4][class] * 2 * scale
                    })
                })
            }),
            bias,
            scale,
            #[cfg(target_arch = "x86_64")]
            slot: inlane::SlotParams::new(block),
            keeps_flat: round2_signed_i32(i32::from(flat_error) * block.pix_scale, rounding) == 0,
        }
    }
}

/// Bit `sum * 18 + k` is set when weight `sum` of tap `k` is zero in every
/// GDF table for every class, with sums 0 and 1 exchanged for odd classes
/// as [`rough_half`] adds them.
const LANE_ZERO_WEIGHTS: u64 = {
    let mut mask = 0;
    let mut bit = 0;
    while bit < 3 * GDF_COORDS.len() {
        let (sum, k) = (bit / GDF_COORDS.len(), bit % GDF_COORDS.len());
        let mut zero = true;
        let mut table = 0;
        while table < GDF_WEIGHT.len() * GDF_WEIGHT[0].len() {
            let weights = &GDF_WEIGHT[table / GDF_WEIGHT[0].len()][table % GDF_WEIGHT[0].len()];
            let mut class = 0;
            while class < 4 {
                let row = if sum < 2 { sum ^ (class & 1) } else { sum };
                zero &= weights[row][k][class] == 0;
                class += 1;
            }
            table += 1;
        }
        if zero {
            mask |= 1 << bit;
        }
        bit += 1;
    }
    mask
};

/// How [`filter_run_with`] filters a row pair of 16 samples into a sink.
trait RowPairs<S: Simd>: Copy {
    fn rows<W: WindowRows, R: RowSink>(
        self,
        simd: S,
        window: &W,
        sink: R,
        classes: i32x8<S>,
        block_params: (&GdfBlock, &GdfSymParams),
    );
}

/// [`sym_rows_with`]: [`rough_half`] for every window.
#[derive(Clone, Copy)]
struct Rough;

impl<S: Simd> RowPairs<S> for Rough {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn rows<W: WindowRows, R: RowSink>(
        self,
        simd: S,
        window: &W,
        sink: R,
        classes: i32x8<S>,
        (block, params): (&GdfBlock, &GdfSymParams),
    ) {
        sym_rows_with(simd, window, sink, classes, block, params);
    }
}

/// The three sums of 8 samples, each lane in sample order.
type HalfSums<S> = [i32x8<S>; 3];

/// Writes into `pos` the § 7.20.5 error indices of both rows of a row pair
/// for the 8 samples from column `8 * HALF`: it adds to the sums from `init`
/// every tap's comb times its weight, each lane
/// with its own class's clip bounds and weights: `index` holds each lane's
/// byte offsets into a by-class table of two taps. With `CLIP` every tap
/// clips its differences, with `COMB` its combs.
#[inline(never)]
fn rough_half<S: Simd, W: WindowRows, const HALF: usize, const CLIP: bool, const COMB: bool>(
    simd: S,
    window: &W,
    (index, odd): (u8x16<S>, mask16x8<S>),
    init: &HalfSums<S>,
    (pos, params): (&mut [[u16; 16]; 2], &GdfSymParams),
) {
    let mut sums = [*init, *init];
    let load = |row: usize, col: usize| -> i16x8<S> {
        let row = window.row(row);
        u16x8::from_slice(simd, &row[col + 8 * HALF..col + 8 * HALF + 8]).bitcast()
    };
    let lookup = |table: &[u16; 8], odd: bool| -> i16x8<S> {
        let table = u16x8::from_slice(simd, table).bitcast::<u8x16<S>>();
        let index = if odd { index + 8 } else { index };
        simd.swizzle_dyn_within_blocks_u8x16(table, index).bitcast()
    };
    let lookup_weight = |table: &[i16; 8], odd: bool| -> i16x8<S> {
        let table = i16x8::from_slice(simd, table).bitcast::<u8x16<S>>();
        let index = if odd { index + 8 } else { index };
        simd.swizzle_dyn_within_blocks_u8x16(table, index).bitcast()
    };
    let centers = [load(TAP_REACH, TAP_REACH), load(TAP_REACH + 1, TAP_REACH)];
    let zero = i16x8::splat(simd, 0);
    macro_rules! taps {
        ($($k:literal)*) => {$({
            const K: usize = $k;
            let (dy, dx) = GDF_COORDS[K];
            let alpha = lookup(&params.lane_alpha[K / 2], K & 1 != 0);
            let low = zero - alpha;
            let weights: [i16x8<S>; 3] = core::array::from_fn(|sum| {
                if LANE_ZERO_WEIGHTS >> (sum * GDF_COORDS.len() + K) & 1 == 0 {
                    lookup_weight(&params.lane_weights[sum][K / 2], K & 1 != 0)
                } else {
                    zero
                }
            });
            for (row, (center, sums)) in centers.iter().zip(&mut sums).enumerate() {
                let at = TAP_REACH + row;
                let above = load(at - dy as usize, (TAP_REACH as isize - dx) as usize) - *center;
                let below = load(at + dy as usize, (TAP_REACH as isize + dx) as usize) - *center;
                let (above, below) = if CLIP {
                    (above.max(low).min(alpha), below.max(low).min(alpha))
                } else {
                    (above, below)
                };
                let comb = above + below;
                let comb = if COMB {
                    comb.max(i16x8::splat(simd, -512)).min(i16x8::splat(simd, 511))
                } else {
                    comb
                };
                for (sum, (total, weight)) in sums.iter_mut().zip(weights).enumerate() {
                    if LANE_ZERO_WEIGHTS >> (sum * GDF_COORDS.len() + K) & 1 == 0 {
                        let (first, second) = comb.widening_mul(weight);
                        *total += simd.combine_i32x4(first, second);
                    }
                }
            }
        })*};
    }
    taps!(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17);
    for (sums, pos) in sums.into_iter().zip(pos) {
        rough_index(simd, sums, odd, params).store_slice(&mut pos[8 * HALF..8 * HALF + 8]);
    }
}

/// The § 7.20.5 error indices of 8 samples from their [`rough_half`] sums,
/// which hold sums 0 and 1 exchanged where `odd` is set.
#[allow(clippy::inline_always)]
#[inline(always)]
fn rough_index<S: Simd>(
    simd: S,
    sums: HalfSums<S>,
    odd: mask16x8<S>,
    params: &GdfSymParams,
) -> u16x8<S> {
    let scale = params.scale;
    let [first, second, third] = sums.map(|sum| {
        let rounded = (sum + (sum >> 31) + (1 << 15)).bitcast::<i16x16<S>>();
        let (low, high) = simd.split_i16x16(rounded);
        simd.unzip_high_i16x8(low, high)
            .clamp(i16x8::splat(simd, -scale), i16x8::splat(simd, scale - 1))
    });
    let (first, second) = (odd.select(second, first), odd.select(first, second));
    let radix = 2 * scale;
    let offset = (scale * radix + scale) * radix + scale;
    ((first * radix + second) * radix + third + offset).bitcast()
}

/// Byte `b` of each half's lookup index: the low byte of lane `b / 2`'s
/// class block, then its high byte.
const HALF_INDEX: [u8; 16] = [0, 1, 0, 1, 4, 5, 4, 5, 8, 9, 8, 9, 12, 13, 12, 13];

/// Filters a row pair through [`rough_half`] into `sink`: `classes` holds
/// each class block's class and gradient bias.
#[inline(never)]
fn rough_rows<S: Simd, W: WindowRows, R: RowSink>(
    simd: S,
    window: &W,
    mut sink: R,
    classes: i32x8<S>,
    (clipped, range): (u8, u16),
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    let factor = 2 * i32::from(params.scale);
    let odd = (classes & 1).simd_ne(i32x8::splat(simd, 0));
    let [b0, b1, b2] = params.bias.map(|bias| i32x8::splat(simd, bias * factor));
    let spread = |blocks: i32x8<S>| simd.interleave_i32x8(blocks, blocks);
    let (first, second, third) = (
        spread(odd.select(b1, b0)),
        spread(odd.select(b0, b1)),
        spread((classes >> 2) * factor + b2),
    );
    let class = (classes & 3) * 0x202 + 0x100;
    let (low, high) = simd.split_u8x32(class.bitcast::<u8x32<S>>());
    let pattern = u8x16::from_slice(simd, &HALF_INDEX);
    let index = [low, high].map(|blocks| simd.swizzle_dyn_within_blocks_u8x16(blocks, pattern));
    let parity =
        index.map(|index| (index.bitcast::<u16x8<S>>() & 2).simd_ne(u16x8::splat(simd, 0)));
    let init = |half: usize| -> HalfSums<S> {
        let half = |pair: (i32x8<S>, i32x8<S>)| if half == 0 { pair.0 } else { pair.1 };
        [half(first), half(second), half(third)]
    };
    let mut pos = [[0; 16]; 2];
    macro_rules! halves {
        ($clip:literal, $comb:literal) => {{
            let state = (&mut pos, params);
            rough_half::<S, W, 0, $clip, $comb>(
                simd,
                window,
                (index[0], parity[0]),
                &init(0),
                state,
            );
            let state = (&mut pos, params);
            rough_half::<S, W, 1, $clip, $comb>(
                simd,
                window,
                (index[1], parity[1]),
                &init(1),
                state,
            );
        }};
    }
    match (clipped != 0, range > LINEAR_RANGE) {
        _ if range == 0 => {
            for half in 0..2 {
                let first = rough_index(simd, init(half), parity[half], params);
                first.store_slice(&mut pos[0][8 * half..8 * half + 8]);
            }
            pos[1] = pos[0];
        }
        (true, true) => halves!(true, true),
        (true, false) => halves!(true, false),
        (false, true) => halves!(false, true),
        (false, false) => halves!(false, false),
    }
    sink.row(simd, 0, &pos[0], block);
    sink.row(simd, 1, &pos[1], block);
}

/// Adds to the samples of `output` the § 7.20.5 residual of the error
/// indices `pos`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn add_residual<S: Simd, O: GdfSample>(
    simd: S,
    pos: &[u16; 16],
    output: &mut [O; 16],
    block: &GdfBlock,
) {
    let (low, high) = simd.widen_i8x16(i8x16::from_slice(simd, &errors(block, pos)));
    let residual = residuals(simd.combine_i16x8(low, high), block);
    apply_residuals(simd, residual, output, block);
}

/// The § 7.20.5 residuals of the `err` lanes `error`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn residuals<S: Simd>(error: i16x16<S>, block: &GdfBlock) -> i16x16<S> {
    let scaled = error * block.pix_scale as i16;
    let rounding = 12 - u32::from(block.bit_depth.bits());
    if rounding == 0 {
        scaled
    } else {
        (scaled + (1 << (rounding - 1)) + (scaled >> 15)) >> rounding
    }
}

/// Adds `residual` to `output`.
#[allow(clippy::inline_always)]
#[inline(always)]
fn apply_residuals<S: Simd, O: GdfSample>(
    simd: S,
    residual: i16x16<S>,
    output: &mut [O; 16],
    block: &GdfBlock,
) {
    let base = O::load(simd, output);
    let filtered = (base + residual)
        .max(i16x16::splat(simd, 0))
        .min(i16x16::splat(simd, block.max_sample as i16));
    O::store(simd, filtered, output);
}

/// A sample type of the luma rows GDF filters in place.
pub(super) trait GdfSample: Copy + Default {
    /// The 16 samples of `samples` as `i16` lanes.
    fn load<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S>;
    /// Stores 16 lanes, each within the sample range, into `samples`.
    fn store<S: Simd>(simd: S, lanes: i16x16<S>, samples: &mut [Self; 16]);
}

impl GdfSample for u16 {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn load<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S> {
        u16x16::from_slice(simd, samples).bitcast()
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn store<S: Simd>(_simd: S, lanes: i16x16<S>, samples: &mut [Self; 16]) {
        lanes.bitcast::<u16x16<S>>().store_slice(samples);
    }
}

impl GdfSample for u8 {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn load<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S> {
        let (low, high) = simd.widen_u8x16(u8x16::from_slice(simd, samples));
        simd.combine_u16x8(low, high).bitcast()
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn store<S: Simd>(simd: S, lanes: i16x16<S>, samples: &mut [Self; 16]) {
        let (low, high) = simd.split_u16x16(lanes.bitcast());
        simd.narrow_u16x8(low, high).store_slice(samples);
    }
}

/// Filters in place a row pair of 16 samples whose class changes every two
/// samples. A window with `tap_range` 0 adds zero at every tap, so its sums
/// stay at the bias.
#[allow(clippy::inline_always)]
#[inline(always)]
fn sym_rows_with<S: Simd, W: WindowRows, R: RowSink>(
    simd: S,
    window: &W,
    sink: R,
    classes: i32x8<S>,
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    let range = window_range(simd, window, CLIP_RANGE);
    if range == 0 && params.keeps_flat && (classes >> 2).simd_eq(i32x8::splat(simd, 0)).all_true() {
        let mut sink = sink;
        sink.keep(simd);
        return;
    }
    let below = u16x16::from_slice(simd, &params.group_bounds).simd_lt(u16x16::splat(simd, range));
    let bits = u16x16::from_slice(simd, &LANE_BITS);
    let groups = simd.reduce_sum_u16x16(below.select(bits, u16x16::splat(simd, 0)));
    rough_rows(
        simd,
        window,
        sink,
        classes,
        ((groups >> 8) as u8, range),
        block,
        params,
    );
}

/// Where [`sym_rows_with`] puts a row pair's § 7.20.5 error indices.
trait RowSink {
    /// Takes the error indices `pos` of row `row` (0 or 1) of the pair.
    fn row<S: Simd>(&mut self, simd: S, row: usize, pos: &[u16; 16], block: &GdfBlock);
    /// Takes a pair whose samples all keep their values.
    fn keep<S: Simd>(&mut self, simd: S);
}

/// Adds the residuals to the pair's two output rows at once.
impl<O: GdfSample> RowSink for [&mut [O; 16]; 2] {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row<S: Simd>(&mut self, simd: S, row: usize, pos: &[u16; 16], block: &GdfBlock) {
        if let Some(output) = self.get_mut(row) {
            add_residual(simd, pos, output, block);
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn keep<S: Simd>(&mut self, _simd: S) {}
}

/// Stores the pair's residual bytes for [`add_residuals`]: each is a scalar
/// load and store, independent of the others, where inserting them into one
/// vector chains every lookup on the one before. The table holds the
/// residual of each entry of the block's error table from index 0, so that
/// any `u16` index reads it without a mask or bounds check; every residual
/// fits a byte, as `err` does and `GdfPixScale <= 1 << (12 - BitDepth)`.
struct ErrorRows<'a>([&'a mut [i8; 16]; 2], &'a [i8; 1 << 16]);

impl RowSink for ErrorRows<'_> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn row<S: Simd>(&mut self, _simd: S, row: usize, pos: &[u16; 16], _block: &GdfBlock) {
        if let Some(errors) = self.0.get_mut(row) {
            for (error, &pos) in errors.iter_mut().zip(pos) {
                *error = self.1[usize::from(pos)];
            }
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn keep<S: Simd>(&mut self, simd: S) {
        for errors in &mut self.0 {
            i8x16::splat(simd, 0).store_slice(&mut errors[..]);
        }
    }
}

/// Adds to each sample of `output` its § 7.20.5 residual in `residuals`, 16
/// samples at a time.
fn add_residuals<S: Simd, O: GdfSample>(
    simd: S,
    residuals: &[i8],
    output: &mut [O],
    block: &GdfBlock,
) {
    let (residuals, _) = residuals.as_chunks::<16>();
    let (output, _) = output.as_chunks_mut::<16>();
    for (residuals, output) in residuals.iter().zip(output) {
        let (low, high) = simd.widen_i8x16(i8x16::from_slice(simd, residuals));
        apply_residuals(simd, simd.combine_i16x8(low, high), output, block);
    }
}

/// [`sym_rows_with`] through the target's widest kernel.
#[inline(never)]
pub(super) fn sym_rows<O: GdfSample>(
    window: &Window<'_>,
    output: [&mut [O; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = fearless_simd::Level::baseline().as_avx2() {
        let classes = i32x8::from_slice(avx2, &classes.map(|class| class.0));
        return inlane::InLane.rows(avx2, window, output, classes, (block, params));
    }
    sym_rows_portable(window, output, classes, block, params);
}

/// [`sym_rows_with`] through the portable kernel.
pub(super) fn sym_rows_portable<O: GdfSample>(
    window: &Window<'_>,
    output: [&mut [O; 16]; 2],
    classes: &[GdfClass; 8],
    block: &GdfBlock,
    params: &GdfSymParams,
) {
    crate::baseline_simd!(simd => sym_rows_with(
        simd,
        window,
        output,
        i32x8::from_slice(simd, &classes.map(|class| class.0)),
        block,
        params,
    ));
}

/// Filters in place `width` samples (at most 16) from column `x` of a row
/// pair through `sym_rows`, on copies padded to 16 lanes.
pub(super) fn sym_rows_narrow<O: GdfSample>(
    rows: &[&[u16]; WINDOW_ROWS],
    x: usize,
    width: usize,
    [top, bottom]: [&mut [O]; 2],
    classes: &[GdfClass],
    block: &GdfBlock,
    params: &GdfSymParams,
) -> Option<()> {
    let mut window = [[0; 28]; WINDOW_ROWS];
    for (padded, row) in window.iter_mut().zip(rows) {
        let samples = row.get(x..x.checked_add(width + 2 * TAP_REACH)?)?;
        padded.get_mut(..samples.len())?.copy_from_slice(samples);
    }
    let mut lane_classes = [GdfClass::default(); 8];
    let class_count = width.div_ceil(2);
    lane_classes
        .get_mut(..class_count)?
        .copy_from_slice(classes.get(..class_count)?);
    let (top, bottom) = (top.get_mut(..width)?, bottom.get_mut(..width)?);
    let mut output = [[O::default(); 16]; 2];
    output[0][..width].copy_from_slice(top);
    output[1][..width].copy_from_slice(bottom);
    let [first, second] = &mut output;
    sym_rows(
        &window.each_ref(),
        [first, second],
        &lane_classes,
        block,
        params,
    );
    top.copy_from_slice(&output[0][..width]);
    bottom.copy_from_slice(&output[1][..width]);
    Some(())
}

/// Gradient sums of 16 columns, one row per direction.
type GradChunk = [[u16; 16]; 4];

/// Scratch of [`band_classes`]: for the last two gradient row pairs, the
/// four direction gradients of each column summed over the pair's two rows,
/// in 16-column chunks.
#[derive(Default)]
pub(super) struct ClassScratch {
    pub(super) grads: Vec<GradChunk>,
    /// A row pair's residual bytes, two rows of [`ClassScratch::error_row`]
    /// each, for [`add_residuals`].
    pub(super) errors: Vec<i8>,
    /// [`ErrorRows`]' table, and the block facts it was built for.
    pub(super) table: Vec<i8>,
    pub(super) table_key: Option<(usize, usize, i32, BitDepth)>,
}

impl ClassScratch {
    /// The chunks of one gradient row pair for a run of `class_cols` class
    /// columns: those its `2 * class_cols + 2` columns cover, plus one that a
    /// class chunk reads past them.
    pub(super) fn chunks(class_cols: usize) -> usize {
        (2 * class_cols + 2).div_ceil(16) + 1
    }

    /// The `err` bytes of one row of a run `run` samples wide.
    pub(super) fn error_row(run: usize) -> usize {
        run.next_multiple_of(16)
    }
}

/// The four § 7.20.5 `grad` directions of the 16 columns from column 1 of
/// the row `rows[1]`, `rows[0]` above it and `rows[2]` below it.
#[allow(clippy::inline_always)]
#[inline(always)]
fn gradients<S: Simd>(simd: S, rows: [&[u16; 18]; 3]) -> [i16x16<S>; 4] {
    let load = |row: &[u16; 18], offset: usize| -> i16x16<S> {
        u16x16::from_slice(simd, &row[offset..offset + 16]).bitcast()
    };
    let [above, center, below] = rows;
    let mid = load(center, 1);
    let twice = mid + mid;
    let grad = |before: i16x16<S>, after: i16x16<S>| (twice - before - after).abs();
    [
        grad(load(above, 1), load(below, 1)),
        grad(load(center, 0), load(center, 2)),
        grad(load(above, 0), load(below, 2)),
        grad(load(below, 0), load(above, 2)),
    ]
}

/// Fills `out` with the gradients of the gradient row pair between the four
/// source rows, summed over its two rows, for the columns from column 1 of
/// each row; a chunk that reads past the rows reads zeros there.
fn gradient_row<S: Simd>(simd: S, rows: &[&[u16]; 4], out: &mut [GradChunk]) -> Option<()> {
    for (chunk, out) in out.iter_mut().enumerate() {
        let start = 16 * chunk;
        let chunks = rows.map(|row| row.get(start..).and_then(<[u16]>::first_chunk::<18>));
        if let [Some(a), Some(b), Some(c), Some(d)] = chunks {
            pair_gradients(simd, [a, b, c, d], out);
        } else {
            padded_gradients(simd, rows, start, out)?;
        }
    }
    Some(())
}

/// [`gradient_row`] for one chunk whose rows end before its reach, on
/// zero-padded copies; out of line, so that the padding is only built here.
#[inline(never)]
fn padded_gradients<S: Simd>(
    simd: S,
    rows: &[&[u16]; 4],
    start: usize,
    out: &mut GradChunk,
) -> Option<()> {
    let mut padded = [[0; 18]; 4];
    for (padded, row) in padded.iter_mut().zip(rows) {
        let tail = row.get(start..).unwrap_or_default();
        let tail = tail.get(..tail.len().min(18))?;
        padded.get_mut(..tail.len())?.copy_from_slice(tail);
    }
    pair_gradients(simd, padded.each_ref(), out);
    Some(())
}

/// The gradients of one chunk of a gradient row pair, summed over its rows.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pair_gradients<S: Simd>(simd: S, chunks: [&[u16; 18]; 4], out: &mut GradChunk) {
    let first = gradients(simd, [chunks[0], chunks[1], chunks[2]]);
    let second = gradients(simd, [chunks[1], chunks[2], chunks[3]]);
    for ((out, first), second) in out.iter_mut().zip(first).zip(second) {
        (first + second).bitcast::<u16x16<S>>().store_array(out);
    }
}

/// The direction gradients of a chunk summed over two gradient row pairs.
#[allow(clippy::inline_always)]
#[inline(always)]
fn pair_sum<S: Simd>(simd: S, current: &GradChunk, previous: &GradChunk) -> [i16x16<S>; 4] {
    core::array::from_fn(|d| {
        (u16x16::from_slice(simd, &current[d]) + u16x16::from_slice(simd, &previous[d])).bitcast()
    })
}

/// The classes and gradient biases of the 8 class columns of a chunk from
/// the gradients `sums` of its columns and `next` of the next chunk over
/// both gradient row pairs: each direction strength sums four columns from
/// `2j`, at most `8 * 4092`, so it fits an `i16` multiply-add, which by 4096
/// also moves `strength >> 4` to the upper half. `tables` holds the clip
/// bounds and the weights of directions 0 and 1, then of 2 and 3, in
/// alternating lanes.
#[allow(clippy::inline_always)]
#[inline(always)]
fn classify_chunk<S: Simd>(
    simd: S,
    sums: [i16x16<S>; 4],
    next: [i16x16<S>; 4],
    tables: &[[i16; 16]; 4],
) -> i32x8<S> {
    let strengths: [i32x8<S>; 4] = core::array::from_fn(|d| {
        let columns = sums[d] + simd.slide_i16x16::<2>(sums[d], next[d]);
        simd.pairwise_dot_product_i16x16(columns, i16x16::splat(simd, 4096))
    });
    let bit = |a: i32x8<S>, b: i32x8<S>, value: i32| {
        a.simd_le(b)
            .select(i32x8::splat(simd, value), i32x8::splat(simd, 0))
    };
    let index = bit(strengths[0], strengths[1], 1) | bit(strengths[2], strengths[3], 2);
    let high = i32x8::splat(simd, -65536);
    let combs = |low: i32x8<S>, high_half: i32x8<S>, [alpha, weight]: [&[i16; 16]; 2]| {
        let packed: i16x16<S> = ((low >> 16) | (high_half & high)).bitcast();
        let alpha = i16x16::from_slice(simd, alpha);
        simd.pairwise_dot_product_i16x16(packed.min(alpha), i16x16::from_slice(simd, weight))
    };
    let bias = combs(strengths[0], strengths[1], [&tables[0], &tables[1]])
        + combs(strengths[2], strengths[3], [&tables[2], &tables[3]]);
    (bias << 2) | index
}

/// The four source rows of gradient row pair `row_pair` of the run of
/// `class_cols` class columns whose top-left sample sits at `origin`.
fn gradient_source_rows<'a>(
    source: &GdfSource<'a>,
    origin: (usize, usize),
    class_cols: usize,
    row_pair: usize,
) -> Option<[&'a [u16]; 4]> {
    let first_row = (origin.1 + 2 * row_pair).checked_sub(2)?;
    let first_col = origin.0.checked_sub(2)?;
    let row = |index: usize| {
        exact_slice(
            source.samples,
            (first_row + index) * source.stride + first_col,
            2 * class_cols + 4,
        )
    };
    Some([row(0)?, row(1)?, row(2)?, row(3)?])
}

/// The gradient-tap tables of [`classify_chunk`] for `block`'s GDF table.
fn class_tables(block: &GdfBlock) -> [[i16; 16]; 4] {
    let alpha = &GDF_ALPHA[block.ref_dst_idx][block.qp_idx][GDF_COORDS.len()..];
    let weight = &GDF_WEIGHT[block.ref_dst_idx][block.qp_idx][2][GDF_COORDS.len()..];
    core::array::from_fn(|table| {
        core::array::from_fn(|lane| {
            let direction = (table & 2) + (lane & 1);
            if table & 1 == 0 {
                alpha[direction][0] as i16
            } else {
                weight[direction][0]
            }
        })
    })
}

/// Filters in place the run `block` of the stripe rows `output` (rows
/// `block.base_stride` apart, the run at column `block.x`) whose top-left
/// source sample sits at `origin`: per row pair it fills the next gradient
/// row pair, then classifies and filters each 16-column chunk in one pass.
#[inline(never)]
pub(super) fn filter_run<O: GdfSample>(
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    output: &mut [O],
    scratch: &mut ClassScratch,
) -> Option<()> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = fearless_simd::Level::baseline().as_avx2() {
        return filter_run_with(avx2, inlane::InLane, source, origin, block, output, scratch);
    }
    crate::baseline_simd!(simd => filter_run_with(simd, Rough, source, origin, block, output, scratch))
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn filter_run_with<S: Simd, P: RowPairs<S>, O: GdfSample>(
    simd: S,
    pairs: P,
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    output: &mut [O],
    scratch: &mut ClassScratch,
) -> Option<()> {
    let (run, class_cols) = (block.width, block.width >> 1);
    if run == 0 || !run.is_multiple_of(2) || !block.height.is_multiple_of(2) {
        return None;
    }
    let chunks = ClassScratch::chunks(class_cols);
    let (first, second) = scratch.grads.get_mut(..2 * chunks)?.split_at_mut(chunks);
    let buffers = [first, second];
    let error_row = ClassScratch::error_row(run);
    let mut errors = scratch.errors.get_mut(..2 * error_row);
    let tables = class_tables(block);
    let params = GdfSymParams::new(block);
    let key = (
        block.ref_dst_idx,
        block.qp_idx,
        block.pix_scale,
        block.bit_depth,
    );
    if scratch.table_key != Some(key) {
        scratch.table.resize(1 << 16, 0);
        let (entries, _) = scratch.table.as_chunks_mut::<16>();
        for (entries, errors) in entries
            .iter_mut()
            .zip(error_table(block)?.as_chunks::<16>().0)
        {
            let (low, high) = simd.widen_i8x16(i8x16::from_slice(simd, errors));
            let (low, high) = simd.split_i16x16(residuals(simd.combine_i16x8(low, high), block));
            simd.saturating_narrow_i16x8(low, high).store_slice(entries);
        }
        scratch.table_key = Some(key);
    }
    let table = scratch.table.first_chunk()?;
    let grad_rows = |row_pair: usize| gradient_source_rows(source, origin, class_cols, row_pair);
    gradient_row(simd, &grad_rows(0)?, buffers[0])?;
    let mut chunk_classes = [[0; 8]; TILE_COLS / 16];
    let chunk_classes = chunk_classes.get_mut(..run.div_ceil(16))?;
    for row_pair in 0..block.height / 2 {
        let (current, previous) = ((row_pair + 1) & 1, row_pair & 1);
        gradient_row(simd, &grad_rows(row_pair + 1)?, buffers[current])?;
        let (current, previous) = (&*buffers[current], &*buffers[previous]);
        let window_rows = || {
            source_rows(
                source,
                (origin.0, origin.1 + 2 * row_pair),
                run + 2 * TAP_REACH,
            )
            .ok()
        };
        let tiled_base = (origin.1 + 2 * row_pair)
            .checked_sub(TAP_REACH)
            .zip(origin.0.checked_sub(TAP_REACH))
            .filter(|_| source.stride == TILE_STRIDE)
            .map(|(row, col)| row * TILE_STRIDE + col);
        let start = (2 * row_pair * block.base_stride).checked_add(block.x)?;
        let (top, bottom) = output
            .get_mut(start..)?
            .split_at_mut_checked(block.base_stride)?;
        let (top, bottom) = (top.get_mut(..run)?, bottom.get_mut(..run)?);
        let mut next = pair_sum(simd, current.first()?, previous.first()?);
        let grads = current.iter().zip(previous).skip(1);
        for (classes, (current, previous)) in chunk_classes.iter_mut().zip(grads) {
            let sums = next;
            next = pair_sum(simd, current, previous);
            *classes = classify_chunk(simd, sums, next, &tables).to_array();
        }
        let mut deferred = errors
            .as_deref_mut()
            .map(|errors| errors.split_at_mut(error_row));
        let mut done = 0;
        if let (Some(base), Some((top_errors, bottom_errors))) = (tiled_base, deferred.as_mut()) {
            let span = source.samples.get(base..).unwrap_or_default();
            let (top_errors, _) = top_errors.as_chunks_mut::<16>();
            let (bottom_errors, _) = bottom_errors.as_chunks_mut::<16>();
            let errors = top_errors.iter_mut().zip(bottom_errors);
            for (classes, (top, bottom)) in chunk_classes.iter().take(run / 16).zip(errors) {
                let Some(window) = span
                    .get(16 * done..)
                    .and_then(<[u16]>::first_chunk::<TILE_SPAN>)
                else {
                    break;
                };
                let (window, classes) = (TiledWindow(window), i32x8::from_slice(simd, classes));
                let sink = ErrorRows([top, bottom], table);
                pairs.rows(simd, &window, sink, classes, (block, &params));
                done += 1;
            }
        }
        for (x, classes) in (16 * done..run).step_by(16).zip(&chunk_classes[done..]) {
            let classes = i32x8::from_slice(simd, classes);
            let width = (run - x).min(16);
            let tiled = tiled_base
                .and_then(|base| source.samples.get(base + x..))
                .and_then(<[u16]>::first_chunk::<TILE_SPAN>);
            if width == 16
                && let Some(span) = tiled
            {
                let top = top.get_mut(x..)?.first_chunk_mut::<16>()?;
                let bottom = bottom.get_mut(x..)?.first_chunk_mut::<16>()?;
                let window = TiledWindow(span);
                pairs.rows(simd, &window, [top, bottom], classes, (block, &params));
            } else if width == 16
                && let Some(window) = window_at::<28>(&window_rows()?, x)
            {
                let top = top.get_mut(x..)?.first_chunk_mut::<16>()?;
                let bottom = bottom.get_mut(x..)?.first_chunk_mut::<16>()?;
                pairs.rows(simd, &window, [top, bottom], classes, (block, &params));
            } else {
                let classes = classes.to_array().map(GdfClass);
                let output = [&mut top[x..], &mut bottom[x..]];
                sym_rows_narrow(&window_rows()?, x, width, output, &classes, block, &params)?;
            }
        }
        if let Some((top_errors, bottom_errors)) = deferred {
            let end = 16 * done;
            add_residuals(simd, top_errors.get(..end)?, top.get_mut(..end)?, block);
            add_residuals(
                simd,
                bottom_errors.get(..end)?,
                bottom.get_mut(..end)?,
                block,
            );
        }
    }
    Some(())
}

/// The § 7.20.5 `gdfCls` and gradient bias of every 2x2 class block of
/// `block`, whose top-left sample sits at `origin` in `source`.
pub(super) fn band_classes(
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    classes: &mut [GdfClass],
    scratch: &mut ClassScratch,
) -> Option<()> {
    baseline_simd!(simd => band_classes_with(simd, source, origin, block, classes, scratch))
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn band_classes_with<S: Simd>(
    simd: S,
    source: &GdfSource<'_>,
    origin: (usize, usize),
    block: &GdfBlock,
    classes: &mut [GdfClass],
    scratch: &mut ClassScratch,
) -> Option<()> {
    let class_cols = block.width >> 1;
    let chunks = ClassScratch::chunks(class_cols);
    let (first, second) = scratch.grads.get_mut(..2 * chunks)?.split_at_mut(chunks);
    let buffers = [first, second];
    let rows = |row_pair: usize| gradient_source_rows(source, origin, class_cols, row_pair);
    let tables = class_tables(block);
    gradient_row(simd, &rows(0)?, buffers[0])?;
    for (row, classes) in classes.chunks_exact_mut(class_cols).enumerate() {
        let (current, previous) = ((row + 1) & 1, row & 1);
        gradient_row(simd, &rows(row + 1)?, buffers[current])?;
        let (current, previous) = (&*buffers[current], &*buffers[previous]);
        let mut next = pair_sum(simd, current.first()?, previous.first()?);
        let pairs = current.iter().zip(previous).skip(1);
        for (classes, (current, previous)) in classes.chunks_mut(8).zip(pairs) {
            let sums = next;
            next = pair_sum(simd, current, previous);
            let eight = classify_chunk(simd, sums, next, &tables)
                .to_array()
                .map(GdfClass);
            if let Ok(classes) = <&mut [GdfClass; 8]>::try_from(&mut *classes) {
                *classes = eight;
            } else {
                classes.copy_from_slice(eight.get(..classes.len())?);
            }
        }
    }
    Some(())
}
