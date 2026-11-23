// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use fearless_simd::{i16x8, i16x16, i32x8, mask16x16, prelude::*, u8x16, u16x8, u16x16, u64x2};
use splot_core::simd::SimdBackend;

use crate::intra_dc_math::resolve_divisor_32;
use crate::math::round2_signed_i32;
use crate::{BitDepth, ReconError, Result, WindowSample};

#[cfg(target_arch = "x86_64")]
mod x86_sums;
#[cfg(target_arch = "x86_64")]
use x86_sums::{rows_8x8_sums, unit_8x8};

/// `OPFL_GRAD_UNIT` (§ 3): the gradients of § 7.13.3.11 clamp their taps to
/// units of this many samples in each direction.
const GRADIENT_UNIT: usize = 16;
const MAX_LS_BITS: u32 = 26;
const MV_REFINE_PREC_BITS: i32 = 4;
const MV_DELTA_LIMIT: i32 = 1 << MV_REFINE_PREC_BITS;

/// Reusable working storage for optical-flow motion-vector derivation.
#[derive(Debug, Default)]
pub struct OptflowScratch {
    deltas: Vec<[[i32; 2]; 2]>,
}

/// Derives the AV2 § 7.13.3.9 optical-flow motion-vector deltas for each
/// `unit_size` square in a pair of clipped, row-major luma predictors.
///
/// Each returned entry contains `[[row0, col0], [row1, col1]]` in 1/16-pel
/// units. Entries are ordered left-to-right, top-to-bottom. `distances` are the
/// unscaled relative order-hint distances; this function performs the § 7.13.3.9
/// ratio reduction before solving the least-squares system.
///
/// # Errors
///
/// Returns the errors of [`derive_optflow_mv_deltas_into`].
#[cfg(test)]
pub(crate) fn derive_optflow_mv_deltas(
    pred0: &[u16],
    pred1: &[u16],
    width: usize,
    height: usize,
    unit_size: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
) -> Result<Vec<[[i32; 2]; 2]>> {
    let mut scratch = OptflowScratch::default();
    derive_optflow_mv_deltas_into(
        [pred0, pred1],
        [0; 2],
        width,
        [width, height],
        unit_size,
        bit_depth,
        distances,
        &mut scratch,
    )?;
    Ok(core::mem::take(&mut scratch.deltas))
}

#[inline]
fn prepare_optflow_deltas<P: WindowSample>(
    preds: [&[P]; 2],
    starts: [usize; 2],
    (stride, [width, height], unit_size): (usize, [usize; 2], usize),
    bit_depth: BitDepth,
    distances: [i32; 2],
    scratch: &mut OptflowScratch,
) -> Result<Option<Weights>> {
    scratch.deltas.clear();
    if width == 0 {
        return Err(ReconError::ZeroDimension {
            field: "optical-flow predictor width",
        });
    }
    if height == 0 {
        return Err(ReconError::ZeroDimension {
            field: "optical-flow predictor height",
        });
    }
    if !width.is_multiple_of(8)
        || !height.is_multiple_of(8)
        || !(unit_size == 8 || unit_size == 4 && width == 8 && height == 8)
    {
        return Err(ReconError::InvalidOptflowUnitSize {
            unit_size,
            width,
            height,
        });
    }
    if stride < width {
        return Err(ReconError::BufferLengthMismatch {
            expected: width,
            actual: stride,
        });
    }
    let overflow = || ReconError::ArithmeticOverflow {
        context: "optical-flow predictor span",
    };
    let span = (height - 1)
        .checked_mul(stride)
        .and_then(|rows| rows.checked_add(width))
        .ok_or_else(overflow)?;
    for (pred, start) in preds.iter().zip(starts) {
        let end = start.checked_add(span).ok_or_else(overflow)?;
        if pred.len() < end {
            return Err(ReconError::BufferLengthMismatch {
                expected: end,
                actual: pred.len(),
            });
        }
    }
    scratch
        .deltas
        .resize((width / unit_size) * (height / unit_size), [[0; 2]; 2]);
    if distances.contains(&0) {
        return Ok(None);
    }
    let mut weights = Weights {
        distances: reduce_distances(distances),
        downshift: u32::from(bit_depth.bits().saturating_sub(8)),
        max: bit_depth.max_sample(),
    };
    if P::bytes(&[]).is_some() {
        weights.downshift = 0;
    }
    Ok(Some(weights))
}

/// Derives the AV2 § 7.13.3.9 optical-flow deltas of each `unit_size` square
/// of a `width` x `height` block into reusable buffers, from its two
/// predictions: prediction `i` starts at `starts[i]` in `preds[i]`, its rows
/// `stride` samples apart. Each § 7.13.3.11 gradient unit derives its
/// § 7.13.3.10 differences, gradients and § 7.13.3.12 sums in registers.
///
/// `scratch` retains its allocation between calls. The returned slice holds
/// one entry per unit in raster order; it is empty after an error. A unit
/// whose `suw` and `svw` are zero takes a zero delta without a solve.
///
/// # Errors
///
/// Returns [`ReconError::ZeroDimension`] for an empty block,
/// [`ReconError::InvalidOptflowUnitSize`] unless both dimensions are
/// multiples of 8 and the unit size is 8, or 4 in an 8x8 block,
/// [`ReconError::BufferLengthMismatch`] when `stride` is below `width` or a
/// prediction ends before the block's last sample, or the arithmetic errors
/// of the solve.
#[expect(clippy::too_many_arguments)]
pub fn derive_optflow_mv_deltas_into<'a, P: WindowSample>(
    preds: [&[P]; 2],
    starts: [usize; 2],
    stride: usize,
    [width, height]: [usize; 2],
    unit_size: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
    scratch: &'a mut OptflowScratch,
) -> Result<&'a [[[i32; 2]; 2]]> {
    let Some(weights) = prepare_optflow_deltas(
        preds,
        starts,
        (stride, [width, height], unit_size),
        bit_depth,
        distances,
        scratch,
    )?
    else {
        return Ok(&scratch.deltas);
    };
    let block = (preds, starts, stride, [width, height], unit_size);
    if let Err(error) =
        baseline_simd!(simd => block_deltas(simd, block, weights, &mut scratch.deltas))
    {
        scratch.deltas.clear();
        return Err(error);
    }
    Ok(&scratch.deltas)
}

/// Typed [`derive_optflow_mv_deltas_into`] using caller-selected `backend`.
///
/// # Errors
/// Returns the same validation and solve errors as [`derive_optflow_mv_deltas_into`].
#[expect(clippy::too_many_arguments)]
pub fn derive_optflow_mv_deltas_into_with<'a, B: SimdBackend, P: WindowSample>(
    backend: B,
    preds: [&[P]; 2],
    starts: [usize; 2],
    stride: usize,
    [width, height]: [usize; 2],
    unit_size: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
    scratch: &'a mut OptflowScratch,
) -> Result<&'a [[[i32; 2]; 2]]> {
    let Some(weights) = prepare_optflow_deltas(
        preds,
        starts,
        (stride, [width, height], unit_size),
        bit_depth,
        distances,
        scratch,
    )?
    else {
        return Ok(&scratch.deltas);
    };
    let block = (preds, starts, stride, [width, height], unit_size);
    let narrow = backend.narrow();
    if let Err(error) =
        narrow.vectorize(|| block_deltas(narrow.token(), block, weights, &mut scratch.deltas))
    {
        scratch.deltas.clear();
        return Err(error);
    }
    Ok(&scratch.deltas)
}

/// The reduced § 7.13.3.9 distances, the § 7.13.3.10 `BitDepth - 8` rounding
/// shift and the `Clip1` maximum of one block's optical flow.
#[derive(Clone, Copy)]
struct Weights {
    distances: [i32; 2],
    downshift: u32,
    max: u16,
}

/// A validated block of [`derive_optflow_mv_deltas_into`]: predictions,
/// starts, stride, `[width, height]` and unit size.
type Block<'a, P> = ([&'a [P]; 2], [usize; 2], usize, [usize; 2], usize);

/// One gradient unit of a [`Block`]: predictions, starts and stride.
type Tile<'a, P> = ([&'a [P]; 2], [usize; 2], usize);

/// The deltas of [`derive_optflow_mv_deltas_into`] over its validated block,
/// one gradient unit (`W` and `H` 16, or 8 at a block narrower or lower
/// than 16) at a time.
#[inline]
fn block_deltas<S: Simd, P: WindowSample>(
    simd: S,
    (preds, starts, stride, [width, height], unit_size): Block<'_, P>,
    weights: Weights,
    deltas: &mut [[[i32; 2]; 2]],
) -> Result<()> {
    let shift = unit_size.trailing_zeros();
    let columns = width >> shift;
    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let tile = (preds, starts.map(|start| start + y * stride + x), stride);
            let size = [width - x, height - y].map(|size| size.min(GRADIENT_UNIT));
            let sums = match size {
                [16, 16] => tile_sums::<S, P, 16, 16>(simd, tile, weights),
                [16, _] => tile_sums::<S, P, 16, 8>(simd, tile, weights),
                [_, 16] => tile_sums::<S, P, 8, 16>(simd, tile, weights),
                _ => tile_8x8(simd, tile, weights, unit_size == 4),
            };
            let [tile_columns, tile_rows] = size.map(|size| size >> shift);
            for row in 0..tile_rows {
                let first = ((y >> shift) + row) * columns + (x >> shift);
                for column in 0..tile_columns {
                    let sums = sums[row * tile_columns + column];
                    if let Some(delta) = deltas.get_mut(first + column)
                        && sums[3..] != [0; 2]
                    {
                        *delta = solve_sums(sums, weights.distances)?;
                    }
                }
            }
            x += GRADIENT_UNIT;
        }
        y += GRADIENT_UNIT;
    }
    Ok(())
}

/// Byte indices that repeat the first `i16` lane of a 128-bit block.
const FIRST_LANE: [u8; 16] = [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1];
/// Byte indices that repeat the last `i16` lane of a 128-bit block.
const LAST_LANE: [u8; 16] = [
    14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15, 14, 15,
];

/// The `[su2, sv2, suv, suw, svw]` sums of the 8x8 units of one `W` x `H`
/// gradient unit (8 or 16 each, not both 8) in raster order: row by row, the
/// § 7.13.3.10 `tmp` and `pDiff` lanes of both predictions, their § 7.13.3.11
/// gradients with the taps clamped to the unit, and the products, all in
/// registers. The `tmp` rows two above and two below the current row stay
/// in a five-row window; `pDiff` is derived again from the current row's
/// samples. An 8-wide unit fills the low eight lanes.
#[expect(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn tile_sums<S: Simd, P: WindowSample, const W: usize, const H: usize>(
    simd: S,
    (preds, at, stride): Tile<'_, P>,
    weights: Weights,
) -> [[i32; 5]; 4] {
    let row = |row: usize| {
        let [first, second] = [0, 1].map(|pred| {
            let at = at[pred] + row * stride;
            let lanes = if W == 16 {
                sample_row(simd, preds[pred], at, weights.max)
            } else {
                sample_pair(simd, preds[pred], [at; 2], weights.max)
            };
            lanes.bitcast::<i16x16<S>>()
        });
        weighted_difference_row(first, second, weights.distances, weights.downshift)
    };
    let [tmp0, tmp1, tmp2] = [0, 1, 2].map(|line| row(line).0);
    let mut tmp = [tmp0, tmp0, tmp0, tmp1, tmp2];
    let mut units = [[0; 5]; 4];
    for half in 0..H / 8 {
        let mut sums = [i32x8::splat(simd, 0); 5];
        for line in half * 8..half * 8 + 8 {
            let [prev2, prev, current, next, next2] = tmp;
            let u = horizontal_gradient::<S, W>(current);
            let edge = (((1u32 << (H - 1)) | 1) >> line) & 1;
            let v = gradient_16([next, prev], [next2, prev2], |magnitude| {
                (magnitude + (64 >> edge)) >> (7 - edge)
            });
            accumulate_unit_sums(&mut sums, u, v, row(line).1);
            let incoming = if line + 3 < H { row(line + 3).0 } else { next2 };
            tmp = [prev, current, next, next2, incoming];
        }
        for (sum, lanes) in sums.into_iter().enumerate() {
            let (left, right) = lanes.split();
            units[half * W / 8][sum] = left.reduce_sum();
            if W == 16 {
                units[half * 2 + 1][sum] = right.reduce_sum();
            }
        }
    }
    units.map(|sums| unit_sums(sums, 64))
}

/// The § 7.13.3.11 horizontal gradients of one row of a gradient unit `W`
/// samples wide: the taps past the unit's first and last samples repeat
/// them, and those two lanes double their taps (round by 6 bits, not 7).
#[expect(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn horizontal_gradient<S: Simd, const W: usize>(row: i16x16<S>) -> i16x16<S> {
    let simd = row.simd;
    let (left, right) = row.split();
    let repeat =
        |half: i16x8<S>, indices| half.swizzle_dyn_within_blocks(u8x16::load_array(simd, indices));
    let first = repeat(left, FIRST_LANE);
    let (before, after) = if W == 16 {
        (first.combine(left), right.combine(repeat(right, LAST_LANE)))
    } else {
        (first.combine(first), repeat(left, LAST_LANE).combine(right))
    };
    let next = row.slide_within_blocks::<1>(after);
    let next2 = row.slide_within_blocks::<2>(after);
    let prev = before.slide_within_blocks::<7>(row);
    let prev2 = before.slide_within_blocks::<6>(row);
    let edges = if W == 16 { 0x8001 } else { 0x0081 };
    gradient_16([next, prev], [next2, prev2], |magnitude| {
        edge_round(magnitude, edges)
    })
}

/// The sums of one 8x8 gradient unit as [`tile_sums`] derives them, with the
/// rows in pairs for [`rows_8x8_sums`]: the whole unit, or with `quarters`
/// its four 4x4 units in raster order.
#[expect(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn tile_8x8<S: Simd, P: WindowSample>(
    simd: S,
    (preds, at, stride): Tile<'_, P>,
    weights: Weights,
    quarters: bool,
) -> [[i32; 5]; 4] {
    let pairs: [(i16x16<S>, i16x16<S>); 4] = core::array::from_fn(|pair| {
        let [first, second] = [0, 1].map(|pred| {
            let at = at[pred] + 2 * pair * stride;
            sample_pair(simd, preds[pred], [at, at + stride], weights.max).bitcast::<i16x16<S>>()
        });
        weighted_difference_row(first, second, weights.distances, weights.downshift)
    });
    let rows = |pairs: [i16x16<S>; 4]| -> [i16x8<S>; 8] {
        let halves = pairs.map(SimdSplit::split);
        core::array::from_fn(|row| {
            let (low, high) = halves[row / 2];
            if row % 2 == 0 { low } else { high }
        })
    };
    let (weighted, difference) = (pairs.map(|pair| pair.0), pairs.map(|pair| pair.1));
    let halves = rows_8x8_sums(simd, &rows(weighted), &rows(difference));
    if !quarters {
        return [whole_8x8(halves), [0; 5], [0; 5], [0; 5]];
    }
    let mut units = [[0; 5]; 4];
    for (half, sums) in halves.iter().enumerate() {
        for (sum, &lanes) in sums.iter().enumerate() {
            [units[2 * half][sum], units[2 * half + 1][sum]] = column_halves(lanes);
        }
    }
    units.map(|sums| unit_sums(sums, 16))
}

/// Sixteen samples of `pred` from `at`, clipped to `max` (§ 7.13.3.10
/// `Clip1`). An `at` past the end, which validation rules out, reads the
/// last samples instead, so that the loads need no per-row bounds branch.
#[expect(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn sample_row<S: Simd, P: WindowSample>(simd: S, pred: &[P], at: usize, max: u16) -> u16x16<S> {
    let at = at.min(pred.len().saturating_sub(16));
    let Some(row) = pred.get(at..).and_then(<[P]>::first_chunk::<16>) else {
        return u16x16::splat(simd, 0);
    };
    if let Some(&bytes) = P::bytes(row).and_then(<[u8]>::first_chunk) {
        let row = u8x16::load_array(simd, bytes);
        return row.combine(row).widen().0;
    }
    let words = P::words(row).and_then(<[u16]>::first_chunk).copied();
    u16x16::load_array(simd, words.unwrap_or_default()).min(u16x16::splat(simd, max))
}

/// The eight samples of `pred` at each of `at` in the low and high eight
/// lanes, as [`sample_row`] reads them.
#[expect(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn sample_pair<S: Simd, P: WindowSample>(
    simd: S,
    pred: &[P],
    at: [usize; 2],
    max: u16,
) -> u16x16<S> {
    let last = pred.len().saturating_sub(8);
    let rows = at.map(|at| {
        let row = pred.get(at.min(last)..).and_then(<[P]>::first_chunk::<8>);
        row.map_or(&[][..], |row| &row[..])
    });
    if let [Some(first), Some(second)] = rows.map(P::bytes) {
        let row =
            |bytes: &[u8]| u64::from_le_bytes(bytes.first_chunk().copied().unwrap_or_default());
        let rows = u64x2::load_array(simd, [row(first), row(second)]).bitcast::<u8x16<S>>();
        return rows.combine(rows).widen().0;
    }
    let [first, second] = rows.map(|row| {
        let row = P::words(row).and_then(<[u16]>::first_chunk).copied();
        u16x8::load_array(simd, row.unwrap_or_default())
    });
    first.combine(second).min(u16x16::splat(simd, max))
}

struct ValidatedStrided8x8<'a> {
    pred0: &'a [u16],
    pred1: &'a [u16],
    distances: [i32; 2],
    downshift: u32,
    max_sample: u16,
}

#[inline]
fn validate_strided_8x8<'a>(
    (pred0, start0): (&'a [u16], usize),
    (pred1, start1): (&'a [u16], usize),
    stride: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
) -> Result<Option<ValidatedStrided8x8<'a>>> {
    if stride < 8 {
        return Err(ReconError::BufferLengthMismatch {
            expected: 8,
            actual: stride,
        });
    }
    let span_len = stride
        .checked_mul(7)
        .and_then(|offset| offset.checked_add(8))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided optical-flow predictor span",
        })?;
    let pred0 = pred0.get(start0..start0.saturating_add(span_len)).ok_or(
        ReconError::BufferLengthMismatch {
            expected: start0.saturating_add(span_len),
            actual: pred0.len(),
        },
    )?;
    let pred1 = pred1.get(start1..start1.saturating_add(span_len)).ok_or(
        ReconError::BufferLengthMismatch {
            expected: start1.saturating_add(span_len),
            actual: pred1.len(),
        },
    )?;
    if distances.contains(&0) {
        return Ok(None);
    }

    Ok(Some(ValidatedStrided8x8 {
        pred0,
        pred1,
        distances: reduce_distances(distances),
        downshift: u32::from(bit_depth.bits().saturating_sub(8)),
        max_sample: bit_depth.max_sample(),
    }))
}

/// Derives one 8x8 optical-flow delta from strided predictors.
///
/// # Errors
///
/// Returns [`ReconError::BufferLengthMismatch`] when either predictor does not
/// contain the requested 8x8 region, or the same arithmetic errors as
/// [`derive_optflow_mv_deltas_into`].
#[doc(hidden)]
pub fn derive_optflow_mv_delta_8x8_strided_into(
    pred0: &[u16],
    start0: usize,
    pred1: &[u16],
    start1: usize,
    stride: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
) -> Result<[[i32; 2]; 2]> {
    let Some(v) = validate_strided_8x8(
        (pred0, start0),
        (pred1, start1),
        stride,
        bit_depth,
        distances,
    )?
    else {
        return Ok([[0; 2]; 2]);
    };
    baseline_simd!(simd => strided_8x8_delta(
        simd, v.pred0, v.pred1, stride, v.distances, v.downshift, v.max_sample,
    ))
}

/// Typed [`derive_optflow_mv_delta_8x8_strided_into`] using caller-selected `backend`.
///
/// # Errors
/// Returns the same validation and solve errors as [`derive_optflow_mv_delta_8x8_strided_into`].
#[doc(hidden)]
#[expect(clippy::too_many_arguments)]
pub fn derive_optflow_mv_delta_8x8_strided_into_with<B: SimdBackend>(
    backend: B,
    pred0: &[u16],
    start0: usize,
    pred1: &[u16],
    start1: usize,
    stride: usize,
    bit_depth: BitDepth,
    distances: [i32; 2],
) -> Result<[[i32; 2]; 2]> {
    let Some(v) = validate_strided_8x8(
        (pred0, start0),
        (pred1, start1),
        stride,
        bit_depth,
        distances,
    )?
    else {
        return Ok([[0; 2]; 2]);
    };
    let narrow = backend.narrow();
    narrow.vectorize(|| {
        strided_8x8_delta(
            narrow.token(),
            v.pred0,
            v.pred1,
            stride,
            v.distances,
            v.downshift,
            v.max_sample,
        )
    })
}

/// [`derive_optflow_mv_delta_8x8_strided_into`] over its validated spans.
#[inline]
fn strided_8x8_delta<S: Simd>(
    simd: S,
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    distances: [i32; 2],
    downshift: u32,
    max_sample: u16,
) -> Result<[[i32; 2]; 2]> {
    let mut peak = u16x8::splat(simd, 0);
    let mut weighted = [i16x8::splat(simd, 0); 8];
    let mut difference = weighted;
    for row in 0..8 {
        let source = row * stride;
        let (Some(left), Some(right)) = (
            pred0.get(source..).and_then(<[u16]>::first_chunk),
            pred1.get(source..).and_then(<[u16]>::first_chunk),
        ) else {
            return Err(ReconError::BufferLengthMismatch {
                expected: source + 8,
                actual: pred0.len().min(pred1.len()),
            });
        };
        let left = u16x8::load_array_ref(simd, left);
        let right = u16x8::load_array_ref(simd, right);
        peak = peak.max(left).max(right);
        (weighted[row], difference[row]) =
            weighted_difference_row(left.bitcast(), right.bitcast(), distances, downshift);
    }
    if peak.reduce_max() > max_sample {
        return Err(strided_sample_out_of_range(
            pred0, pred1, stride, max_sample,
        ));
    }
    let changed = difference
        .iter()
        .fold(u16x8::splat(simd, 0), |changed, row| {
            changed | row.bitcast::<u16x8<S>>()
        });
    if changed.reduce_max() == 0 {
        return Ok([[0; 2]; 2]);
    }
    let sums = whole_8x8(rows_8x8_sums(simd, &weighted, &difference));
    solve_sums(sums, distances)
}

/// The error for the first strided 8x8 predictor sample above `max_sample`,
/// in logical sample order.
#[cold]
#[inline(never)]
fn strided_sample_out_of_range(
    pred0: &[u16],
    pred1: &[u16],
    stride: usize,
    max_sample: u16,
) -> ReconError {
    for row in 0..8 {
        for lane in 0..8 {
            let source = row * stride + lane;
            for (predictor, value) in [(0, pred0[source]), (1, pred1[source])] {
                if value > max_sample {
                    return ReconError::OptflowPredictorSampleOutOfRange {
                        predictor,
                        sample_index: row * 8 + lane,
                        value,
                        max: max_sample,
                    };
                }
            }
        }
    }
    ReconError::ArithmeticOverflow {
        context: "strided optical-flow predictor range",
    }
}

#[inline]
fn unit_8x8_u8_params(distances: [i32; 2], sad_threshold: Option<u32>) -> ([i32; 2], u32) {
    let threshold = sad_threshold.unwrap_or(0);
    let reduced = if distances.contains(&0) {
        [0; 2]
    } else {
        reduce_distances(distances)
    };
    (reduced, threshold)
}

/// The § 7.13.3.8 refinement of an eight-bit 8x8 optical-flow unit whose two
/// predictions are `rows`, eight little-endian samples per row: `None` when
/// their SAD is below `sad_threshold`, else the § 7.13.3.12 delta of
/// [`derive_optflow_mv_delta_8x8_strided_into`].
///
/// # Errors
///
/// Returns the arithmetic errors of the solve.
#[doc(hidden)]
pub fn optflow_unit_8x8_u8(
    rows: [[u64; 8]; 2],
    distances: [i32; 2],
    sad_threshold: Option<u32>,
) -> Result<Option<[[i32; 2]; 2]>> {
    let (reduced, threshold) = unit_8x8_u8_params(distances, sad_threshold);
    baseline_simd!(simd => unit_8x8(simd, &rows, reduced, threshold))
}

/// Typed [`optflow_unit_8x8_u8`] using caller-selected `backend`.
///
/// # Errors
/// Returns the same arithmetic errors as [`optflow_unit_8x8_u8`].
#[doc(hidden)]
pub fn optflow_unit_8x8_u8_with<B: SimdBackend>(
    backend: B,
    rows: [[u64; 8]; 2],
    distances: [i32; 2],
    sad_threshold: Option<u32>,
) -> Result<Option<[[i32; 2]; 2]>> {
    let (reduced, threshold) = unit_8x8_u8_params(distances, sad_threshold);
    let narrow = backend.narrow();
    narrow.vectorize(|| unit_8x8(narrow.token(), &rows, reduced, threshold))
}

/// [`optflow_unit_8x8_u8`] of its eight-byte rows and reduced `distances`:
/// the SAD from the `i16` differences, then the sums of [`rows_8x8_sums`].
#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn unit_8x8<S: Simd>(
    simd: S,
    rows: &[[u64; 8]; 2],
    distances: [i32; 2],
    sad_threshold: u32,
) -> Result<Option<[[i32; 2]; 2]>> {
    let widen = |rows: &[u64; 8]| -> [i16x8<S>; 8] {
        let mut lanes = [i16x8::splat(simd, 0); 8];
        for (pair, lanes) in rows
            .as_chunks::<2>()
            .0
            .iter()
            .zip(lanes.as_chunks_mut::<2>().0)
        {
            let (low, high) =
                simd.widen_u8x16(fearless_simd::u64x2::load_array_ref(simd, pair).bitcast());
            *lanes = [low.bitcast(), high.bitcast()];
        }
        lanes
    };
    let (first, second) = (widen(&rows[0]), widen(&rows[1]));
    let difference: [i16x8<S>; 8] = core::array::from_fn(|row| first[row] - second[row]);
    let sad = difference
        .iter()
        .fold(i16x8::splat(simd, 0), |sum, row| sum + row.abs());
    if u32::from(sad.reduce_sum().unsigned_abs()) < sad_threshold {
        return Ok(None);
    }
    if distances == [0; 2] {
        return Ok(Some([[0; 2]; 2]));
    }
    let [near, far] = distances.map(|distance| distance as i16);
    let weighted = core::array::from_fn(|row| first[row] * near - second[row] * far);
    solve_sums(
        whole_8x8(rows_8x8_sums(simd, &weighted, &difference)),
        distances,
    )
    .map(Some)
}

/// [`optflow_unit_8x8_u8`] of two 16-bit 8x8 predictions, each sample
/// clipped to `bit_depth` first; the SAD is compared at eight bits, and
/// differences that round to zero give a zero delta, as in
/// [`derive_optflow_mv_delta_8x8_strided_into`].
///
/// # Errors
///
/// Returns the arithmetic errors of the solve.
#[doc(hidden)]
pub fn optflow_unit_8x8_u16(
    rows: [[[u16; 8]; 8]; 2],
    bit_depth: BitDepth,
    distances: [i32; 2],
    sad_threshold: Option<u32>,
) -> Result<Option<[[i32; 2]; 2]>> {
    let threshold = sad_threshold.unwrap_or(0);
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    let max = bit_depth.max_sample();
    baseline_simd!(simd => unit_8x8_u16_simd(simd, &rows, downshift, max, distances, threshold))
}

/// Typed [`optflow_unit_8x8_u16`] using caller-selected `backend`.
///
/// # Errors
/// Returns the same arithmetic errors as [`optflow_unit_8x8_u16`].
#[doc(hidden)]
pub fn optflow_unit_8x8_u16_with<B: SimdBackend>(
    backend: B,
    rows: [[[u16; 8]; 8]; 2],
    bit_depth: BitDepth,
    distances: [i32; 2],
    sad_threshold: Option<u32>,
) -> Result<Option<[[i32; 2]; 2]>> {
    let threshold = sad_threshold.unwrap_or(0);
    let downshift = u32::from(bit_depth.bits().saturating_sub(8));
    let max = bit_depth.max_sample();
    let narrow = backend.narrow();
    narrow.vectorize(|| {
        unit_8x8_u16_simd(narrow.token(), &rows, downshift, max, distances, threshold)
    })
}

#[inline]
fn unit_8x8_u16_simd<S: Simd>(
    simd: S,
    rows: &[[[u16; 8]; 8]; 2],
    downshift: u32,
    max: u16,
    distances: [i32; 2],
    threshold: u32,
) -> Result<Option<[[i32; 2]; 2]>> {
    let load = |rows: &[[u16; 8]; 8]| {
        rows.map(|row| u16x8::load_array(simd, row).min(u16x8::splat(simd, max)))
    };
    let (first, second) = (load(&rows[0]), load(&rows[1]));
    let sad = first.iter().zip(&second).fold(0, |sum, (a, b)| {
        sum + u32::from((a.max(*b) - a.min(*b)).reduce_sum())
    });
    if sad >> downshift < threshold {
        return Ok(None);
    }
    if distances.contains(&0) {
        return Ok(Some([[0; 2]; 2]));
    }
    let distances = reduce_distances(distances);
    let rows: [(i16x8<_>, i16x8<_>); 8] = core::array::from_fn(|row| {
        weighted_difference_row(
            first[row].bitcast(),
            second[row].bitcast(),
            distances,
            downshift,
        )
    });
    let (weighted, difference) = (rows.map(|row| row.0), rows.map(|row| row.1));
    let changed = difference
        .iter()
        .fold(u16x8::splat(simd, 0), |changed, row| {
            changed | row.bitcast::<u16x8<_>>()
        });
    if changed.reduce_max() == 0 {
        return Ok(Some([[0; 2]; 2]));
    }
    solve_sums(
        whole_8x8(rows_8x8_sums(simd, &weighted, &difference)),
        distances,
    )
    .map(Some)
}

/// The § 7.13.3.9 `weighted` and `difference` rows of predictor sample pairs
/// in `i16` lanes.
///
/// The reduced distances are at most 2 in magnitude, so validated samples keep
/// `weighted` within `±4092` and `difference` within `±1023` before the
/// `Round2Signed`. Rows of samples past the bit depth are discarded by the
/// caller's range check or clipped first, so wrapped values never reach a solve.
#[expect(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn weighted_difference_row<S: Simd, V: SimdInt<S, Element = i16>>(
    left: V,
    right: V,
    distances: [i32; 2],
    downshift: u32,
) -> (V, V) {
    let round = |value: V| {
        if downshift == 0 {
            return value;
        }
        (value + (1 << (downshift - 1)) + (value >> 15)) >> downshift
    };
    let [near, far] = distances.map(|distance| distance as i16);
    (round(left * near - right * far), round(left - right))
}

fn reduce_distances(distances: [i32; 2]) -> [i32; 2] {
    let magnitudes = match distances[0]
        .unsigned_abs()
        .cmp(&distances[1].unsigned_abs())
    {
        core::cmp::Ordering::Equal => [1, 1],
        core::cmp::Ordering::Greater => [2, 1],
        core::cmp::Ordering::Less => [1, 2],
    };
    [
        if distances[0] < 0 {
            -magnitudes[0]
        } else {
            magnitudes[0]
        },
        if distances[1] < 0 {
            -magnitudes[1]
        } else {
            magnitudes[1]
        },
    ]
}

/// `Round2Signed(42 * (near[0] - near[1]) - 5 * (far[0] - far[1]), shift)`
/// in sixteen `i16` lanes, as [`gradient_i16`] derives it: `tmp` comes from
/// validated samples, `Round2Signed(d0 * s0 - d1 * s1, BitDepth - 8)` with
/// `|d| <= 2`, so any two of its values lie within 768 of each other and
/// both `42 * Δ` and `5 * Δ` fit `i16` lanes; only their difference needs
/// the separate sign.
#[expect(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient_16<S: Simd>(
    near: [i16x16<S>; 2],
    far: [i16x16<S>; 2],
    round: impl FnOnce(u16x16<S>) -> u16x16<S>,
) -> i16x16<S> {
    let near = (near[0] - near[1]) * 42;
    let far = (far[0] - far[1]) * 5;
    let magnitude: i16x16<S> = round((near.max(far) - near.min(far)).bitcast()).bitcast();
    let sign = far.simd_gt(near).to_vector();
    (magnitude ^ sign) - sign
}

/// `(magnitude + 64) >> 7` with the `edges` lanes, whose taps are doubled,
/// rounded by 6 bits instead.
#[expect(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn edge_round<S: Simd>(magnitude: u16x16<S>, edges: u64) -> u16x16<S> {
    mask16x16::from_bitmask(magnitude.simd, edges)
        .select((magnitude + 32) >> 6, (magnitude + 64) >> 7)
}

/// Solves the § 7.13.3.9 least-squares system from one unit's
/// `[su2, sv2, suv, suw, svw]` sums. Each covers at most 64 normalized
/// samples with gradient magnitude at most 564.
fn solve_sums(sums: [i32; 5], distances: [i32; 2]) -> Result<[[i32; 2]; 2]> {
    let [mut su2, mut sv2, mut suv, mut suw, mut svw] = sums;
    let max_product_bits = (1 + msb(su2 as u32)) + (1 + msb(sv2 as u32));
    let max_product_bits = max_product_bits
        .max((1 + msb(sv2 as u32)) + (1 + msb(suw.unsigned_abs())))
        .max((1 + msb(suv.unsigned_abs())) + (1 + msb(svw.unsigned_abs())))
        .max((1 + msb(su2 as u32)) + (1 + msb(svw.unsigned_abs())))
        .max((1 + msb(suv.unsigned_abs())) + (1 + msb(suw.unsigned_abs())));
    let reduction = max_product_bits.saturating_sub(MAX_LS_BITS - 3) >> 1;
    if reduction != 0 {
        let bias = 1 << (reduction - 1);
        [su2, sv2, suv, suw, svw] = sums.map(|value| (value + (value >> 31) + bias) >> reduction);
    }

    let determinant = su2 * sv2 - suv * suv;
    if determinant <= 0 {
        return Ok([[0; 2]; 2]);
    }
    let solution = divide_and_round(
        [sv2 * suw - suv * svw, su2 * svw - suv * suw],
        determinant,
        MV_REFINE_PREC_BITS - 1,
    )?;
    let col = -solution[0];
    let row = -solution[1];
    Ok([
        [
            (row * distances[0]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
            (col * distances[0]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
        ],
        [
            (row * distances[1]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
            (col * distances[1]).clamp(-MV_DELTA_LIMIT, MV_DELTA_LIMIT),
        ],
    ])
}

/// The lane sums of the top and bottom halves of an 8x8 unit held in lanes,
/// one row per call so that every row's clamped neighbours are constants,
/// each row's products widened into one lane per column (`smlal`).
#[cfg(not(target_arch = "x86_64"))]
#[allow(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn rows_8x8_sums<S: Simd>(
    simd: S,
    weighted: &[i16x8<S>; 8],
    difference: &[i16x8<S>; 8],
) -> [[i32x8<S>; 5]; 2] {
    let gradients = [
        row_8x8_gradients::<S, 0>(weighted),
        row_8x8_gradients::<S, 1>(weighted),
        row_8x8_gradients::<S, 2>(weighted),
        row_8x8_gradients::<S, 3>(weighted),
        row_8x8_gradients::<S, 4>(weighted),
        row_8x8_gradients::<S, 5>(weighted),
        row_8x8_gradients::<S, 6>(weighted),
        row_8x8_gradients::<S, 7>(weighted),
    ];
    let mut halves = [[i32x8::splat(simd, 0); 5]; 2];
    let accumulate =
        |sums: &mut [i32x8<S>; 5], rows: &[(i16x8<S>, i16x8<S>)], difference: &[i16x8<S>]| {
            for (&(u, v), &w) in rows.iter().zip(difference) {
                for (sum, (left, right)) in
                    sums.iter_mut()
                        .zip([(u, u), (v, v), (u, v), (u, w), (v, w)])
                {
                    let (low, high) = left.widening_mul(right);
                    *sum += low.combine(high);
                }
            }
        };
    let (top, bottom) = gradients.split_at(4);
    accumulate(&mut halves[0], top, &difference[..4]);
    accumulate(&mut halves[1], bottom, &difference[4..]);
    halves
}

/// The unit sums of a whole 8x8 unit from the lane sums of its halves.
#[expect(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn whole_8x8<S: Simd>([top, bottom]: [[i32x8<S>; 5]; 2]) -> [i32; 5] {
    let sums = core::array::from_fn(|sum| (top[sum] + bottom[sum]).reduce_sum());
    unit_sums(sums, 64)
}

/// Adds one row's `[u², v², uv, uw, vw]` products to `sums`: the low eight
/// lanes' into the low four `i32` lanes and the high eight lanes' into the
/// high four, the 8x8 units left and right in a gradient unit: adjacent
/// products summed (`pmaddwd`) on x86_64, each half widened into its own
/// lanes (`smlal`) on 128-bit targets.
#[expect(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn accumulate_unit_sums<S: Simd>(
    sums: &mut [i32x8<S>; 5],
    u: i16x16<S>,
    v: i16x16<S>,
    w: i16x16<S>,
) {
    for (sum, (left, right)) in sums
        .iter_mut()
        .zip([(u, u), (v, v), (u, v), (u, w), (v, w)])
    {
        #[cfg(target_arch = "x86_64")]
        {
            *sum += left.pairwise_dot_product(right);
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let products = [left.split(), right.split()];
            let half = |first: i16x8<S>, second: i16x8<S>| {
                let (low, high) = first.widening_mul(second);
                low + high
            };
            *sum += half(products[0].0, products[1].0).combine(half(products[0].1, products[1].1));
        }
    }
}

/// The sums of columns 0..4 and 4..8 in the lanes [`rows_8x8_sums`] fills.
#[expect(clippy::inline_always, reason = "per-unit optical-flow hot path")]
#[inline(always)]
fn column_halves<S: Simd>(lanes: i32x8<S>) -> [i32; 2] {
    let (first, second) = lanes.split();
    #[cfg(target_arch = "x86_64")]
    {
        let [a, b, c, d] = *(first + second);
        [a + b, c + d]
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        [first.reduce_sum(), second.reduce_sum()]
    }
}

/// `[next, prev, next2, prev2]` neighbours of every lane of a row of
/// `LAST + 1` samples, repeating the row's end samples past its edges: lane
/// `LAST` of `first` holds the first sample and lane 0 of `last` the last.
#[cfg(not(target_arch = "x86_64"))]
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn clamped_neighbours<S: Simd, V: SimdBase<S>, const LAST: usize>(
    samples: V,
    first: V,
    last: V,
) -> [V; 4] {
    debug_assert_eq!(LAST + 1, V::LEN);
    let next = samples.slide::<1>(last);
    let prev = first.slide::<LAST>(samples);
    [next, prev, next.slide::<1>(last), first.slide::<LAST>(prev)]
}

/// The horizontal and vertical gradients of one row of an 8x8 unit; a doubled
/// edge tap rounds by 6 bits instead of doubling before 7.
#[cfg(not(target_arch = "x86_64"))]
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn row_8x8_gradients<S: Simd, const ROW: usize>(rows: &[i16x8<S>; 8]) -> (i16x8<S>, i16x8<S>) {
    let reversed = rows[ROW].reverse();
    let [next, prev, next2, prev2] = clamped_neighbours::<S, _, 7>(rows[ROW], reversed, reversed);
    let u = gradient_i16([next, prev], [next2, prev2], |magnitude| {
        let edge = fearless_simd::mask16x8::from_bitmask(magnitude.simd, 0b1000_0001);
        edge.select((magnitude + 32) >> 6, (magnitude + 64) >> 7)
    });
    let shift = if ROW == 0 || ROW == 7 { 6 } else { 7 };
    let v = gradient_i16(
        [rows[(ROW + 1).min(7)], rows[ROW.saturating_sub(1)]],
        [rows[(ROW + 2).min(7)], rows[ROW.saturating_sub(2)]],
        |magnitude| (magnitude + (1 << (shift - 1))) >> shift,
    );
    (u, v)
}

/// [`gradient_16`] in eight lanes.
#[cfg(not(target_arch = "x86_64"))]
#[allow(clippy::inline_always, reason = "per-row optical-flow hot path")]
#[inline(always)]
fn gradient_i16<S: Simd>(
    near: [i16x8<S>; 2],
    far: [i16x8<S>; 2],
    round: impl FnOnce(u16x8<S>) -> u16x8<S>,
) -> i16x8<S> {
    let near = (near[0] - near[1]) * 42;
    let far = (far[0] - far[1]) * 5;
    let magnitude: i16x8<S> = round((near.max(far) - near.min(far)).bitcast()).bitcast();
    let sign = far.simd_gt(near).to_vector();
    (magnitude ^ sign) - sign
}

/// Adds the regularizing unit `area` to the `su2` and `sv2` diagonal of raw
/// `[su2, sv2, suv, suw, svw]` sums.
fn unit_sums(raw: [i32; 5], area: i32) -> [i32; 5] {
    [raw[0] + area, raw[1] + area, raw[2], raw[3], raw[4]]
}

fn divide_and_round(values: [i32; 2], denominator: i32, shift: i32) -> Result<[i32; 2]> {
    let (denominator_shift, inverse) = if denominator == 1 {
        (0i32, 1i32)
    } else {
        let (denominator_shift, inverse) = resolve_divisor_32(denominator as u32)?;
        (i32::from(denominator_shift), i32::from(inverse))
    };
    let inverse_msb = msb(inverse as u32);
    let mut result = [0i32; 2];
    for (output, value) in result.iter_mut().zip(values) {
        if value == 0 {
            continue;
        }
        let sign = value.signum();
        let mut magnitude = value.abs();
        let reduction = (msb(magnitude as u32) + inverse_msb + 4).saturating_sub(MAX_LS_BITS);
        magnitude = round2_signed_i32(magnitude, reduction);
        let increase = shift + reduction as i32 - denominator_shift;
        magnitude = if increase <= -31 {
            let reduced = round2_signed_i32(magnitude, (-increase - 30) as u32);
            round2_signed_i32(reduced * inverse, 30)
        } else if increase >= 0 {
            magnitude * inverse * (1i32 << increase)
        } else {
            round2_signed_i32(magnitude * inverse, (-increase) as u32)
        };
        *output = sign * magnitude;
    }
    Ok(result)
}

fn msb(value: u32) -> u32 {
    u32::BITS - 1 - value.max(1).leading_zeros()
}

#[cfg(test)]
mod tests {

    use super::*;

    /// § 7.13.3.9 to § 7.13.3.12 per sample, as the specification writes
    /// them: the deltas of every `n` x `n` unit of a `w` x `h` block whose
    /// predictions start at `starts` with rows `stride` apart.
    fn spec_deltas(
        preds: [&[u16]; 2],
        starts: [usize; 2],
        stride: usize,
        [width, height]: [usize; 2],
        n: usize,
        bit_depth: BitDepth,
        distances: [i32; 2],
    ) -> Vec<[[i32; 2]; 2]> {
        if distances.contains(&0) {
            return vec![[[0; 2]; 2]; (width / n) * (height / n)];
        }
        let dist = reduce_distances(distances);
        let shift = u32::from(bit_depth.bits() - 8);
        let max = i32::from(bit_depth.max_sample());
        let clip1 = |pred: usize, i: usize, j: usize| {
            i32::from(preds[pred][starts[pred] + i * stride + j]).min(max)
        };
        let tmp =
            |i, j| round2_signed_i32(dist[0] * clip1(0, i, j) - dist[1] * clip1(1, i, j), shift);
        let p_diff = |i, j| round2_signed_i32(clip1(0, i, j) - clip1(1, i, j), shift);
        let gradient = |i: usize, j: usize, vertical: bool| {
            let (at, len) = if vertical { (i, height) } else { (j, width) };
            let start = (at >> 4) << 4;
            let end = (start + GRADIENT_UNIT).min(len) - 1;
            let tap = |k: usize| if vertical { tmp(k, j) } else { tmp(i, k) };
            let prev = at.saturating_sub(1).max(start);
            let prev2 = at.saturating_sub(2).max(start);
            let mut temp = 42 * (tap((at + 1).min(end)) - tap(prev))
                - 5 * (tap((at + 2).min(end)) - tap(prev2));
            if at + 1 > end || at < start + 1 {
                temp <<= 1;
            }
            round2_signed_i32(temp, 7)
        };
        let mut deltas = Vec::new();
        for i_base in (0..height).step_by(n) {
            for j_base in (0..width).step_by(n) {
                let mut sums = [0; 5];
                for i in i_base..i_base + n {
                    for j in j_base..j_base + n {
                        let (u, v) = (gradient(i, j, false), gradient(i, j, true));
                        let diff = p_diff(i, j);
                        for (sum, product) in
                            sums.iter_mut()
                                .zip([u * u, v * v, u * v, u * diff, v * diff])
                        {
                            *sum += product;
                        }
                    }
                }
                deltas.push(solve_sums(unit_sums(sums, (n * n) as i32), dist).unwrap());
            }
        }
        deltas
    }

    /// The fused kernel matches the per-sample oracle for every block shape
    /// of the non-TIP path (gradient units 8 or 16 wide and high, blocks
    /// wider and higher than one unit), from byte and word predictions with
    /// a window stride and packed, at 8 and 10 bits, on random, all-max,
    /// 0 / max alternating, flat and out-of-range samples.
    #[test]
    fn fused_units_match_the_spec_oracle() {
        let shapes = [
            (8, 8, 4),
            (8, 8, 8),
            (8, 16, 8),
            (16, 8, 8),
            (16, 16, 8),
            (8, 32, 8),
            (32, 8, 8),
            (16, 32, 8),
            (32, 16, 8),
            (32, 32, 8),
            (64, 16, 8),
            (24, 8, 8),
            (8, 24, 8),
        ];
        let random = |seed: usize| {
            move |i: usize| ((i.wrapping_add(seed) as u32).wrapping_mul(2_654_435_761) >> 16) as u16
        };
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            let max = bit_depth.max_sample();
            let fills: [&dyn Fn(usize, usize) -> u16; 8] = [
                &|i, _| random(1)(i) % (max + 1),
                &|i, _| random(7)(i) % (max + 1),
                &|_, _| max,
                &|i, w| if (i / w + i % w) % 2 == 0 { max } else { 0 },
                &|i, _| if i % 2 == 0 { 0 } else { max },
                &|i, w| if (i / w) % 2 == 0 { max } else { 0 },
                &|_, _| 90,
                &|i, _| random(3)(i) % (max + 64),
            ];
            for (shape_w, shape_h, n) in shapes {
                for (first, second) in [
                    (0, 1),
                    (2, 3),
                    (3, 4),
                    (4, 3),
                    (5, 2),
                    (6, 6),
                    (7, 0),
                    (1, 5),
                ] {
                    for distances in [[1, -1], [2, -1], [-1, 2], [3, 1], [-4, -2], [1, 1], [0, 2]] {
                        for (stride, starts) in [
                            (shape_w, [0, 0]),
                            (shape_w + 8, [2 * (shape_w + 8) + 3, 4 * (shape_w + 8) + 1]),
                        ] {
                            let len = starts[1] + (shape_h - 1) * stride + shape_w;
                            let plane = |fill: &dyn Fn(usize, usize) -> u16| -> Vec<u16> {
                                (0..len).map(|i| fill(i, stride)).collect()
                            };
                            let words = [plane(fills[first]), plane(fills[second])];
                            let want = spec_deltas(
                                [&words[0], &words[1]],
                                starts,
                                stride,
                                [shape_w, shape_h],
                                n,
                                bit_depth,
                                distances,
                            );
                            let mut scratch = OptflowScratch::default();
                            let context = format!(
                                "{bit_depth:?} {shape_w}x{shape_h}/{n} {first}/{second} {distances:?} {stride}"
                            );
                            let got = derive_optflow_mv_deltas_into(
                                [&words[0][..], &words[1][..]],
                                starts,
                                stride,
                                [shape_w, shape_h],
                                n,
                                bit_depth,
                                distances,
                                &mut scratch,
                            );
                            assert_eq!(got.unwrap(), &want[..], "u16 {context}");
                            if bit_depth != BitDepth::Eight {
                                continue;
                            }
                            let bytes = words.each_ref().map(|plane| {
                                plane
                                    .iter()
                                    .map(|&sample| sample.min(255) as u8)
                                    .collect::<Vec<u8>>()
                            });
                            let got = derive_optflow_mv_deltas_into(
                                [&bytes[0][..], &bytes[1][..]],
                                starts,
                                stride,
                                [shape_w, shape_h],
                                n,
                                bit_depth,
                                distances,
                                &mut scratch,
                            );
                            assert_eq!(got.unwrap(), &want[..], "u8 {context}");
                        }
                    }
                }
            }
        }
    }

    /// The byte unit matches the `u16` strided derivation for every distance
    /// pair over textured, extreme-contrast and flat predictions, and skips
    /// the refinement exactly when the SAD is below the threshold.
    #[test]
    fn byte_8x8_units_match_the_strided_derivation() {
        let distance_pairs = [1i32, -1, 3, -4, 0]
            .into_iter()
            .flat_map(|near| [1, -1, 2, -3].map(|far| [near, far]));
        let stride = 16;
        let fills: [fn(usize) -> u8; 6] = [
            |i| ((i * 73 + i / 13 * 29 + (i * i) % 7) % 256) as u8,
            |i| if (i + i / 13) % 3 == 0 { 255 } else { 0 },
            |i| ((i * 7919) % 256) as u8,
            |_| 90,
            |i| ((i as u32).wrapping_mul(2_654_435_761) >> 24) as u8,
            |i| {
                if (i as u32).wrapping_mul(2_246_822_519) >> 31 == 0 {
                    255
                } else {
                    0
                }
            },
        ];
        for distances in distance_pairs {
            for (first, second) in [
                (0, 1),
                (1, 2),
                (2, 0),
                (3, 3),
                (0, 3),
                (4, 5),
                (5, 4),
                (4, 4),
            ] {
                let preds: [Vec<u8>; 2] = [
                    (0..stride * 9 + 2).map(fills[first]).collect(),
                    (0..stride * 9 + 2).map(|i| fills[second](i + 5)).collect(),
                ];
                let starts = [1, stride + 2];
                let widened = |pred: &[u8], start: usize| -> Vec<u16> {
                    (0..64)
                        .map(|i| u16::from(pred[start + i / 8 * stride + i % 8]))
                        .collect()
                };
                let (a, b) = (widened(&preds[0], starts[0]), widened(&preds[1], starts[1]));
                let sad: u32 = a
                    .iter()
                    .zip(&b)
                    .map(|(&x, &y)| u32::from(x.abs_diff(y)))
                    .sum();
                let delta = derive_optflow_mv_delta_8x8_strided_into(
                    &a,
                    0,
                    &b,
                    0,
                    8,
                    BitDepth::Eight,
                    distances,
                )
                .unwrap();
                let rows = [&a, &b].map(|pred| {
                    core::array::from_fn(|row| {
                        u64::from_le_bytes(core::array::from_fn(|col| pred[row * 8 + col] as u8))
                    })
                });
                for threshold in [None, Some(sad), Some(sad + 1)] {
                    let expected =
                        (threshold.is_none_or(|threshold| sad >= threshold)).then_some(delta);
                    let actual = optflow_unit_8x8_u8(rows, distances, threshold).unwrap();
                    assert_eq!(
                        actual, expected,
                        "{distances:?} {first}/{second} {threshold:?}"
                    );
                }
            }
        }
    }

    /// The 10-bit unit kernel matches the strided derivation, including
    /// differences that round to zero, and skips exactly when the eight-bit
    /// SAD is below the threshold.
    #[test]
    fn word_8x8_units_match_the_strided_derivation() {
        let fills: [fn(usize) -> u16; 5] = [
            |i| ((i * 73 + i / 13 * 29 + (i * i) % 7) % 1024) as u16,
            |i| if (i + i / 13) % 3 == 0 { 1023 } else { 0 },
            |i| ((i * 7919) % 1024) as u16,
            |i| (i % 2) as u16 + 500,
            |_| 700,
        ];
        for distances in [[1, -1], [3, 2], [-4, 1], [0, 2]] {
            for (first, second) in [(0, 1), (1, 2), (2, 0), (3, 3), (0, 0), (4, 4)] {
                let a: Vec<u16> = (0..64).map(fills[first]).collect();
                let b: Vec<u16> = (0..64).map(|i| fills[second](i + 5)).collect();
                let sad: u32 = a
                    .iter()
                    .zip(&b)
                    .map(|(&x, &y)| u32::from(x.abs_diff(y)))
                    .sum();
                let delta = derive_optflow_mv_delta_8x8_strided_into(
                    &a,
                    0,
                    &b,
                    0,
                    8,
                    BitDepth::Ten,
                    distances,
                )
                .unwrap();
                let rows = [&a, &b].map(|pred| {
                    core::array::from_fn(|row| core::array::from_fn(|col| pred[row * 8 + col]))
                });
                for threshold in [None, Some(sad >> 2), Some((sad >> 2) + 1)] {
                    let expected =
                        (threshold.is_none_or(|threshold| sad >> 2 >= threshold)).then_some(delta);
                    let actual =
                        optflow_unit_8x8_u16(rows, BitDepth::Ten, distances, threshold).unwrap();
                    assert_eq!(
                        actual, expected,
                        "{distances:?} {first}/{second} {threshold:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn i16_weighted_difference_rows_match_the_i32_rounding() {
        let left = [0, 1023, 1, 512, 1023, 7, 0, 333];
        let right = [1023, 0, 2, 511, 1023, 9, 0, 1000];
        for distances in [[1, 1], [2, -1], [-1, 2], [-2, -1], [1, -2]] {
            for downshift in [0, 2] {
                let (weighted, difference) = baseline_simd!(simd => {
                    let (weighted, difference) = weighted_difference_row(
                        i16x8::load_array(simd, left),
                        i16x8::load_array(simd, right),
                        distances,
                        downshift,
                    );
                    (weighted.to_array(), difference.to_array())
                });
                for lane in 0..8 {
                    let (l, r) = (i32::from(left[lane]), i32::from(right[lane]));
                    let want = |value: i32| round2_signed_i32(value, downshift) as i16;
                    assert_eq!(weighted[lane], want(distances[0] * l - distances[1] * r));
                    assert_eq!(difference[lane], want(l - r));
                }
            }
        }
    }

    #[test]
    fn equal_predictors_produce_zero_deltas() {
        let predictor = vec![128; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&predictor, &predictor, 8, 8, 8, BitDepth::Eight, [3, -3],)
                .unwrap(),
            vec![[[0; 2]; 2]],
        );
    }

    #[test]
    fn into_reuses_delta_allocations() {
        let predictor = [80u16; 8 * 8];
        let mut scratch = OptflowScratch::default();
        let derive = |scratch: &mut OptflowScratch| {
            derive_optflow_mv_deltas_into(
                [&predictor[..], &predictor[..]],
                [0; 2],
                8,
                [8, 8],
                4,
                BitDepth::Eight,
                [1, -2],
                scratch,
            )
            .map(<[_]>::len)
        };
        assert_eq!(derive(&mut scratch).unwrap(), 4);
        let deltas_ptr = scratch.deltas.as_ptr();
        assert_eq!(derive(&mut scratch).unwrap(), 4);
        assert_eq!(scratch.deltas.as_ptr(), deltas_ptr);
        assert_eq!(scratch.deltas, vec![[[0; 2]; 2]; 4]);
    }

    #[test]
    fn into_clears_deltas_when_validation_fails() {
        let predictor = vec![80u16; 16 * 16];
        let mut scratch = OptflowScratch {
            deltas: vec![[[1; 2]; 2]],
        };
        let mut derive = |pred0: &[u16], stride, size, unit_size| {
            let result = derive_optflow_mv_deltas_into(
                [pred0, &predictor[..]],
                [0; 2],
                stride,
                size,
                unit_size,
                BitDepth::Eight,
                [1, -1],
                &mut scratch,
            )
            .map(|_| ());
            assert!(scratch.deltas.is_empty());
            result
        };
        assert!(matches!(
            derive(&predictor[..63], 8, [8, 8], 8),
            Err(ReconError::BufferLengthMismatch {
                expected: 64,
                actual: 63,
            })
        ));
        assert!(matches!(
            derive(&predictor, 4, [8, 8], 8),
            Err(ReconError::BufferLengthMismatch {
                expected: 8,
                actual: 4,
            })
        ));
        for (size, unit_size) in [([16, 16], 4), ([12, 8], 8), ([8, 8], 2)] {
            assert!(matches!(
                derive(&predictor, 16, size, unit_size),
                Err(ReconError::InvalidOptflowUnitSize { .. })
            ));
        }
    }

    #[test]
    fn linear_horizontal_difference_solves_to_opposite_fractional_deltas() {
        let left: Vec<u16> = (0..8)
            .flat_map(|_| (0..8).map(|column| 100 + column))
            .collect();
        let right = vec![100; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1],).unwrap(),
            vec![[[0, -14], [0, 14]]],
        );
    }

    #[test]
    fn ten_bit_normalization_matches_eight_bit_deltas() {
        let left: Vec<u16> = (0..8)
            .flat_map(|_| (0..8).map(|column| 100 + column))
            .collect();
        let right = vec![100; 8 * 8];
        let left_10bit: Vec<u16> = left.iter().map(|sample| sample * 4).collect();
        let right_10bit: Vec<u16> = right.iter().map(|sample| sample * 4).collect();

        assert_eq!(
            derive_optflow_mv_deltas(&left_10bit, &right_10bit, 8, 8, 8, BitDepth::Ten, [1, -1],)
                .unwrap(),
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [1, -1]).unwrap(),
        );
    }

    #[test]
    fn zero_distance_disables_refinement() {
        let left = vec![0; 8 * 8];
        let right = vec![255; 8 * 8];
        assert_eq!(
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Eight, [0, -1],).unwrap(),
            vec![[[0; 2]; 2]],
        );
    }

    #[test]
    fn maximum_ten_bit_contrast_fits_i16_scratch_and_i32_solver() {
        let left: Vec<u16> = (0..8 * 8)
            .map(|index| if index & 1 == 0 { 1023 } else { 0 })
            .collect();
        let right: Vec<u16> = left.iter().map(|&value| 1023 - value).collect();

        let deltas =
            derive_optflow_mv_deltas(&left, &right, 8, 8, 8, BitDepth::Ten, [2, -1]).unwrap();
        assert!(deltas[0].iter().flatten().all(|&value| value.abs() <= 16));
    }

    #[test]
    fn strided_predictor_range_error_uses_logical_sample_order() {
        const STRIDE: usize = 12;
        let mut left = vec![0; STRIDE * 8];
        let mut right = vec![0; STRIDE * 8];
        right[STRIDE] = 256;
        left[STRIDE + 1] = 257;

        assert!(matches!(
            derive_optflow_mv_delta_8x8_strided_into(
                &left,
                0,
                &right,
                0,
                STRIDE,
                BitDepth::Eight,
                [1, -1],
            ),
            Err(ReconError::OptflowPredictorSampleOutOfRange {
                predictor: 1,
                sample_index: 8,
                value: 256,
                max: 255,
            })
        ));
    }

    #[test]
    fn typed_backend_matches_compatibility_routes() {
        use splot_core::simd::{SimdBackend, SimdPolicy, SimdScope, resolve_backend};

        struct Check;
        impl SimdScope<()> for Check {
            fn run<B: SimdBackend>(self, backend: B) {
                let stride = 20usize;
                let (w, h) = (16usize, 16usize);
                let pred0_8: Vec<u8> = (0..stride * h)
                    .map(|i| ((i * 17 + (i / stride) * 5 + 3) % 256) as u8)
                    .collect();
                let pred1_8: Vec<u8> = (0..stride * h)
                    .map(|i| ((i * 23 + (i / stride) * 11 + 7) % 256) as u8)
                    .collect();
                let mut compat_scratch = OptflowScratch::default();
                let mut typed_scratch = OptflowScratch::default();
                let compat_u8 = derive_optflow_mv_deltas_into(
                    [&pred0_8[..], &pred1_8[..]],
                    [0; 2],
                    stride,
                    [w, h],
                    8,
                    BitDepth::Eight,
                    [2, -1],
                    &mut compat_scratch,
                )
                .unwrap();
                let typed_u8 = derive_optflow_mv_deltas_into_with(
                    backend,
                    [&pred0_8[..], &pred1_8[..]],
                    [0; 2],
                    stride,
                    [w, h],
                    8,
                    BitDepth::Eight,
                    [2, -1],
                    &mut typed_scratch,
                )
                .unwrap();
                assert_eq!(typed_u8, compat_u8);

                let pred0_10: Vec<u16> = (0..stride * h)
                    .map(|i| ((i * 61 + (i / stride) * 13 + 9) % 1024) as u16)
                    .collect();
                let pred1_10: Vec<u16> = (0..stride * h)
                    .map(|i| ((i * 79 + (i / stride) * 19 + 15) % 1024) as u16)
                    .collect();
                let compat_u16 = derive_optflow_mv_deltas_into(
                    [&pred0_10[..], &pred1_10[..]],
                    [0; 2],
                    stride,
                    [w, h],
                    8,
                    BitDepth::Ten,
                    [1, -2],
                    &mut compat_scratch,
                )
                .unwrap();
                let typed_u16 = derive_optflow_mv_deltas_into_with(
                    backend,
                    [&pred0_10[..], &pred1_10[..]],
                    [0; 2],
                    stride,
                    [w, h],
                    8,
                    BitDepth::Ten,
                    [1, -2],
                    &mut typed_scratch,
                )
                .unwrap();
                assert_eq!(typed_u16, compat_u16);

                let strided_compat = derive_optflow_mv_delta_8x8_strided_into(
                    &pred0_10,
                    2,
                    &pred1_10,
                    3,
                    stride,
                    BitDepth::Ten,
                    [1, -1],
                )
                .unwrap();
                let strided_typed = derive_optflow_mv_delta_8x8_strided_into_with(
                    backend,
                    &pred0_10,
                    2,
                    &pred1_10,
                    3,
                    stride,
                    BitDepth::Ten,
                    [1, -1],
                )
                .unwrap();
                assert_eq!(strided_typed, strided_compat);

                let rows_u8: [[u64; 8]; 2] = core::array::from_fn(|p| {
                    core::array::from_fn(|r| {
                        let base = r * 8 + p * 31;
                        u64::from_le_bytes(core::array::from_fn(|c| ((base + c * 19) % 256) as u8))
                    })
                });
                assert_eq!(
                    optflow_unit_8x8_u8_with(backend, rows_u8, [2, -1], Some(16)).unwrap(),
                    optflow_unit_8x8_u8(rows_u8, [2, -1], Some(16)).unwrap(),
                );

                let rows_u16: [[[u16; 8]; 8]; 2] = core::array::from_fn(|p| {
                    core::array::from_fn(|r| {
                        core::array::from_fn(|c| ((r * 37 + c * 53 + p * 41) % 1024) as u16)
                    })
                });
                assert_eq!(
                    optflow_unit_8x8_u16_with(backend, rows_u16, BitDepth::Ten, [1, -2], Some(8))
                        .unwrap(),
                    optflow_unit_8x8_u16(rows_u16, BitDepth::Ten, [1, -2], Some(8)).unwrap(),
                );
            }
        }

        resolve_backend(SimdPolicy::Auto)
            .expect("auto backend")
            .enter(Check);
    }
}
