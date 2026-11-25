// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared SIMD operations for the four-block CDEF chroma row segment.

#[cfg(target_arch = "x86_64")]
use fearless_simd::{Avx2, Sse2};
use fearless_simd::{Simd, SimdBase, u8x16, u16x8, u16x16};

/// Chroma samples per plane across the four blocks, including tap reach.
pub const SPAN: usize = 20;
/// Rows gathered per plane, including tap reach.
pub const ROWS: usize = 8;
/// Interleaved U/V bytes per block in the packed layout.
pub const PACKED_BLOCK: usize = ROWS * 16;
/// Interleaved U/V bytes for all four blocks.
pub const PACKED_BYTES: usize = 4 * PACKED_BLOCK;

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    #[inline(always)]
    /// Reduces eight unsigned lanes with the AVX2 backend's minimum-position instruction.
    pub fn minpos_bounds(t: Avx2, min: u16x8<Avx2>, max: u16x8<Avx2>) -> [u16; 2] {
        use core::arch::x86_64::{
            _mm_cvtsi128_si32, _mm_minpos_epu16, _mm_set1_epi16, _mm_xor_si128,
        };
        let _ = t;
        let least = _mm_cvtsi128_si32(_mm_minpos_epu16(min.into())) as u16;
        let flipped = _mm_xor_si128(max.into(), _mm_set1_epi16(-1));
        let greatest = !(_mm_cvtsi128_si32(_mm_minpos_epu16(flipped)) as u16);
        [least, greatest]
    }
);

/// Widens sixteen unsigned bytes through the low half of a doubled vector.
#[expect(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
pub fn widen_u8<S: Simd>(simd: S, lanes: u8x16<S>) -> u16x16<S> {
    simd.widen_u8x32(simd.combine_u8x16(lanes, lanes)).0
}

/// The larger plane window range of each block from the column-wise least and greatest
/// of both planes at columns 0, 8 and 12. Each minimum must be no larger
/// than its corresponding maximum. `bounds` returns the least lane of its
/// first argument and the greatest lane of its second.
#[expect(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
pub fn pair_ranges<S: Simd>(
    simd: S,
    bounds: impl Fn(u16x8<S>, u16x8<S>) -> [u16; 2],
    min: [[u16x8<S>; 3]; 2],
    max: [[u16x8<S>; 3]; 2],
) -> [u16; 4] {
    let range = |plane: usize, block: usize| {
        let pick = |[first, second, third]: [u16x8<S>; 3]| match block {
            0 => first,
            1 => simd.slide_u16x8::<4>(first, second),
            2 => second,
            _ => third,
        };
        let [least, greatest] = bounds(pick(min[plane]), pick(max[plane]));
        greatest - least
    };
    let block = |block: usize| range(0, block).max(range(1, block));
    [block(0), block(1), block(2), block(3)]
}

/// Gathers two planes whose slices begin at the upper-left tap, reduces
/// the four window ranges and passes interleaved byte vectors to `store`.
/// Row starts are `stride` bytes apart. Values below [`SPAN`] overlap the
/// windows, including zero, which repeats the first row. Returns `None` if
/// either plane lacks a complete row or a row offset overflows. Earlier
/// rows may already have been stored. `bounds` follows [`pair_ranges`].
#[expect(
    clippy::inline_always,
    reason = "instantiate the row loop in the caller's ISA scope"
)]
#[inline(always)]
pub fn gather_with<S: Simd>(
    simd: S,
    planes: [&[u8]; 2],
    stride: usize,
    bounds: impl Fn(u16x8<S>, u16x8<S>) -> [u16; 2],
    mut store: impl FnMut(usize, usize, usize, u8x16<S>),
) -> Option<[u16; 4]> {
    let mut min = [[u8x16::splat(simd, u8::MAX); 2]; 2];
    let mut max = [[u8x16::splat(simd, 0); 2]; 2];
    for row in 0usize..ROWS {
        let offset = row.checked_mul(stride)?;
        let rows = planes.map(|rows| rows.get(offset..)?.get(..SPAN));
        let [Some(u_row), Some(v_row)] = rows else {
            return None;
        };
        for (half, start) in [0, SPAN - 16].into_iter().enumerate() {
            let planes = [u_row, v_row].map(|row| u8x16::from_slice(simd, &row[start..start + 16]));
            for plane in 0..2 {
                min[plane][half] = min[plane][half].min(planes[plane]);
                max[plane][half] = max[plane][half].max(planes[plane]);
            }
            let pairs = [
                simd.zip_low_u8x16(planes[0], planes[1]),
                simd.zip_high_u8x16(planes[0], planes[1]),
            ];
            for (index, pair) in pairs.into_iter().enumerate() {
                store(row, half, index, pair);
            }
        }
    }
    let chunks = |[head, tail]: [u8x16<S>; 2]| {
        let (first, second) = simd.split_u16x16(widen_u8(simd, head));
        [first, second, simd.split_u16x16(widen_u8(simd, tail)).1]
    };
    Some(pair_ranges(simd, bounds, min.map(chunks), max.map(chunks)))
}

/// Gathers two byte planes into block-major interleaved, biased scratch.
/// Row addressing, partial writes on failure and `bounds` follow [`gather_with`].
#[expect(
    clippy::inline_always,
    reason = "instantiate the packed gather in the caller's ISA scope"
)]
#[inline(always)]
pub fn gather_packed_with<S: Simd>(
    simd: S,
    planes: [&[u8]; 2],
    stride: usize,
    bytes: &mut [u8; PACKED_BYTES],
    bounds: impl Fn(u16x8<S>, u16x8<S>) -> [u16; 2],
) -> Option<[u16; 4]> {
    let bias = u8x16::splat(simd, 0x80);
    gather_with(simd, planes, stride, bounds, |row, half, index, pair| {
        let block = 2 * index + half;
        let at = block * PACKED_BLOCK + row * 16;
        (pair ^ bias).store_slice(&mut bytes[at..at + 16]);
    })
}

/// SSE2 entry for [`gather_packed_with`], including overlapping rows and
/// partial scratch writes if a row is truncated.
#[cfg(target_arch = "x86_64")]
#[inline(never)]
pub fn gather_packed(
    simd: Sse2,
    planes: [&[u8]; 2],
    stride: usize,
    bytes: &mut [u8; PACKED_BYTES],
) -> Option<[u16; 4]> {
    gather_packed_with(simd, planes, stride, bytes, |min, max| {
        [min.reduce_min(), max.reduce_max()]
    })
}
