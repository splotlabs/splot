// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-cell tap lanes and source chunks of the PC-Wiener lane kernels, and
//! the 16-lane row kernel on `pmaddwd` tap pairs.

use std::simd::{Simd, ToBytes, cmp::SimdOrd, num::SimdInt, simd_swizzle};

use super::{
    LaneTaps, PC_WIENER_BLOCK_SIZE, PC_WIENER_CONFIG, PC_WIENER_FILTER_TAP_RADIUS,
    PC_WIENER_FULL_CLASSES, PC_WIENER_PREC_BITS, PC_WIENER_WINDOW_ROWS, PcWienerPaddedSample,
    TapWords,
};
use crate::wienerns_filter::{interleave_pairs, multiply_add_pairs};

/// Per-lane taps of the first `CELLS` 4x4 cells of `cells`: each `u64` tap
/// word already holds its cell's four lanes.
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn cell_taps<const CELLS: usize, const LANES: usize>(
    words: &[TapWords; PC_WIENER_FULL_CLASSES],
    cells: &[u8],
) -> LaneTaps<LANES>
where
    Simd<u64, CELLS>: ToBytes,
    Simd<i16, LANES>: ToBytes<Bytes = <Simd<u64, CELLS> as ToBytes>::Bytes>,
{
    let cells: [&TapWords; CELLS] = std::array::from_fn(|cell| &words[usize::from(cells[cell])]);
    std::array::from_fn(|tap| {
        Simd::from_ne_bytes(Simd::from_array(cells.map(|words| words[tap])).to_ne_bytes())
    })
}

/// The `REACH` columns from `col` of each of `ROWS` padded source rows.
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn chunk_rows<'a, T, const REACH: usize, const ROWS: usize>(
    rows: &[&'a [T]; ROWS],
    col: usize,
) -> Option<[&'a [T; REACH]; ROWS]> {
    let rows = rows.map(|row| row.get(col..).and_then(<[T]>::first_chunk::<REACH>));
    let first = rows.first().copied().flatten()?;
    rows.iter()
        .all(Option::is_some)
        .then(|| rows.map(|row| row.unwrap_or(first)))
}

/// `pmaddwd` weights of [`filter_pc_wiener_cells16`]: taps `2k` and `2k + 1`
/// interleaved as [`interleave_pairs`] leaves the tap sums, then tap 12 with
/// the rounding offset.
pub(super) type MaddWeights = [[Simd<i16, 16>; 2]; 7];

pub(super) fn pc_wiener_madd_weights(taps: &LaneTaps<16>) -> MaddWeights {
    core::array::from_fn(|pair| match pair {
        6 => interleave_pairs(taps[12], Simd::splat(1 << (PC_WIENER_PREC_BITS - 1))),
        _ => interleave_pairs(taps[2 * pair], taps[2 * pair + 1]),
    })
}

/// `filter_pc_wiener_group_simd` for 16 lanes on [`filter_pc_wiener_cells16`].
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn filter_pc_wiener_group16<T: PcWienerPaddedSample>(
    output: &mut [u16],
    (stride, rows): (usize, usize),
    lanes: &[&[T; 22]; PC_WIENER_BLOCK_SIZE + 2 * PC_WIENER_FILTER_TAP_RADIUS],
    weights: &MaddWeights,
    max_sample: u16,
) {
    let [r0, r1, r2, r3, r4, r5, r6, r7, r8, r9] = *lanes;
    let mut filter = |k: usize, rows| {
        let values = filter_pc_wiener_cells16::<T>(&rows, weights, max_sample);
        output[k * stride..][..16].copy_from_slice(&values.to_array()); // splot-copy-ok: publish PC-Wiener SIMD lanes
    };
    filter(0, [r0, r1, r2, r3, r4, r5, r6]);
    if rows > 1 {
        filter(1, [r1, r2, r3, r4, r5, r6, r7]);
    }
    if rows > 2 {
        filter(2, [r2, r3, r4, r5, r6, r7, r8]);
    }
    if rows > 3 {
        filter(3, [r3, r4, r5, r6, r7, r8, r9]);
    }
}

/// `filter_pc_wiener_padded_u16_simd` for 16 lanes with every tap pair, and
/// the center with the rounding offset, as one `pmaddwd`; the sums stay in
/// the unpack lane order, which the final saturating pack puts back in order.
#[allow(clippy::inline_always)]
#[inline(always)]
fn filter_pc_wiener_cells16<T: PcWienerPaddedSample>(
    window: &[&[T; 22]; PC_WIENER_WINDOW_ROWS],
    weights: &MaddWeights,
    max_sample: u16,
) -> Simd<u16, 16> {
    const R: usize = PC_WIENER_FILTER_TAP_RADIUS;
    let term = |j: usize| {
        let (dy, dx) = PC_WIENER_CONFIG[j];
        T::load_lanes::<16>(window[R.wrapping_add_signed(dy)], R.wrapping_add_signed(dx))
            + T::load_lanes::<16>(
                window[R.wrapping_add_signed(-dy)],
                R.wrapping_add_signed(-dx),
            )
    };
    let mut sums = [Simd::<i32, 8>::splat(0); 2];
    let mut add = |halves: [Simd<i16, 16>; 2], weights: &[Simd<i16, 16>; 2]| {
        for ((sum, half), weight) in sums.iter_mut().zip(halves).zip(weights) {
            *sum += multiply_add_pairs(half, *weight);
        }
    };
    macro_rules! pairs {
        ($($pair:literal)*) => {
            $(add(interleave_pairs(term(2 * $pair), term(2 * $pair + 1)), &weights[$pair]);)*
        };
    }
    pairs!(0 1 2 3 4 5);
    add(
        interleave_pairs(T::load_lanes::<16>(window[R], R), Simd::splat(1)),
        &weights[6],
    );
    let [low, high] = sums.map(|sum| sum >> Simd::splat(PC_WIENER_PREC_BITS as i32));
    simd_swizzle!(
        low,
        high,
        [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
    )
    .simd_clamp(Simd::splat(0), Simd::splat(i32::from(u16::MAX)))
    .cast::<u16>()
    .simd_min(Simd::splat(max_sample))
}
