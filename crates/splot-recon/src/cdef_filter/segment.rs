// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Row segments of [`CDEF_SEGMENT_BLOCKS`] interior CDEF blocks, gathered once
//! per plane, and the § 7.18.3 filters that read them.
//!
//! A segment's scratch and the sample range of each block's window are
//! private and only its gather writes them, so a range always bounds the
//! samples that it describes. The filters select their kernel from that
//! range.

use super::*;
#[cfg(target_arch = "x86_64")]
use fearless_simd::Avx2;
use fearless_simd::{i8x16, i8x32, u8x32, u64x2};

const TAP_REACH: usize = 2;
const LUMA_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_SEGMENT_BLOCK_AREA;
const PAIR_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_PAIR_SEGMENT_BLOCK_AREA;
const PAIR_ROWS: usize = 4 + 2 * TAP_REACH;
const PAIR_SPAN: usize = CDEF_PAIR_SEGMENT_STRIDE / 2;
/// The byte scratch of an 8-bit segment, past which an 8-byte row read may
/// load 16 bytes.
const LUMA_BYTES_AREA: usize = LUMA_SEGMENT_AREA + 8;
const LUMA_BLOCK_BYTES: usize = CDEF_SEGMENT_BLOCK_AREA + 8;
const PAIR_BYTES_AREA: usize = PAIR_SEGMENT_AREA + 8;
const PAIR_BLOCK_BYTES: usize = CDEF_PAIR_SEGMENT_BLOCK_AREA + 8;
/// No byte scratch, for the kernels of samples wider than 8 bits.
static NO_BYTES: [u8; LUMA_BLOCK_BYTES] = [0; LUMA_BLOCK_BYTES];

/// The tap rows of one luma row segment of [`CDEF_SEGMENT_BLOCKS`] 8x8
/// blocks at [`CDEF_SEGMENT_STRIDE`] lanes per row, and the least and
/// greatest sample of each block's 12x12 window.
pub struct CdefLumaSegment {
    pad: [u16; LUMA_SEGMENT_AREA],
    /// The scratch of an 8-bit segment as bytes offset by 128, `has_bytes`.
    bytes: [u8; LUMA_BYTES_AREA],
    has_bytes: bool,
    /// Whether `pad` holds the segment, which a bytes-only gather skips.
    has_pad: bool,
    ranges: [[u16; 2]; CDEF_SEGMENT_BLOCKS],
}

impl Default for CdefLumaSegment {
    fn default() -> Self {
        Self {
            pad: [0; LUMA_SEGMENT_AREA],
            bytes: [0; LUMA_BYTES_AREA],
            has_bytes: false,
            has_pad: true,
            ranges: [[0; 2]; CDEF_SEGMENT_BLOCKS],
        }
    }
}

impl CdefLumaSegment {
    /// Copies the tap rows of the segment whose first block starts at column
    /// `origin.0`, row `origin.1` of `samples` (rows `stride` apart, `width`
    /// samples wide) and records each block's window range. Returns `false`,
    /// with every range set to `[0, u16::MAX]`, when the tap reach leaves
    /// the rows.
    pub fn gather<T: crate::ReconSample>(
        &mut self,
        samples: &[T],
        width: usize,
        stride: usize,
        origin: (usize, usize),
    ) -> bool {
        let ranges = gather_luma(
            samples,
            width,
            stride,
            (&mut self.pad, &mut self.bytes),
            origin,
        );
        self.ranges = ranges.unwrap_or([[0, u16::MAX]; CDEF_SEGMENT_BLOCKS]);
        self.has_bytes = ranges.is_some() && T::u8_slice(samples).is_some();
        self.has_pad = ranges.is_some();
        ranges.is_some()
    }

    /// [`CdefLumaSegment::gather`] of 8-bit samples into the byte scratch
    /// alone: [`CdefLumaSegment::block`] then holds nothing, and
    /// [`cdef_filter_segment`] filters the segment's luma only through the
    /// 32-lane kernel, which [`CdefStrengths::luma_bytes_suffice`] checks.
    #[inline]
    pub fn gather_bytes(
        &mut self,
        samples: &[u8],
        width: usize,
        stride: usize,
        origin: (usize, usize),
    ) -> bool {
        let ranges = gather_luma_bytes(samples, width, stride, &mut self.bytes, origin);
        self.ranges = ranges.unwrap_or([[0, u16::MAX]; CDEF_SEGMENT_BLOCKS]);
        self.has_bytes = ranges.is_some();
        self.has_pad = false;
        ranges.is_some()
    }

    /// [`CdefLumaSegment::block`] as offset bytes, for an 8-bit segment.
    #[inline]
    fn block_bytes(&self, block: usize) -> Option<&[u8; LUMA_BLOCK_BYTES]> {
        if !self.has_bytes || block >= CDEF_SEGMENT_BLOCKS {
            return None;
        }
        self.bytes.get(8 * block..)?.first_chunk()
    }

    /// The scratch from the first tap-reach lane of block `block` on, or
    /// `None` when `block` is not below [`CDEF_SEGMENT_BLOCKS`].
    #[inline]
    pub fn block(&self, block: usize) -> Option<&[u16; CDEF_SEGMENT_BLOCK_AREA]> {
        if !self.has_pad {
            return None;
        }
        self.scratch(block)
    }

    /// The `u16` scratch of block `block`, whether or not it holds the
    /// segment.
    #[inline]
    fn scratch(&self, block: usize) -> Option<&[u16; CDEF_SEGMENT_BLOCK_AREA]> {
        if block >= CDEF_SEGMENT_BLOCKS {
            return None;
        }
        self.pad.get(8 * block..)?.first_chunk()
    }

    /// The least and greatest sample of block `block`'s window.
    #[inline]
    pub fn range(&self, block: usize) -> Option<[u16; 2]> {
        self.ranges.get(block).copied()
    }
}

/// The tap rows of one row segment of both chroma planes, interleaved as
/// for [`cdef_filter_block_chroma_pair`] at [`CDEF_PAIR_SEGMENT_STRIDE`]
/// lanes per row, and for each 4x4 block the larger of its two planes'
/// window ranges (greatest minus least sample).
pub struct CdefPairSegment {
    pad: [u16; PAIR_SEGMENT_AREA],
    /// The scratch of an 8-bit segment as bytes offset by 128, `has_bytes`.
    bytes: [u8; PAIR_BYTES_AREA],
    has_bytes: bool,
    ranges: [u16; CDEF_SEGMENT_BLOCKS],
}

impl Default for CdefPairSegment {
    fn default() -> Self {
        Self {
            pad: [0; PAIR_SEGMENT_AREA],
            bytes: [0; PAIR_BYTES_AREA],
            has_bytes: false,
            ranges: [0; CDEF_SEGMENT_BLOCKS],
        }
    }
}

impl CdefPairSegment {
    /// [`CdefLumaSegment::gather`] for the two chroma planes `u` and `v`,
    /// which share `width` and `stride`. A failed gather sets every range to
    /// `u16::MAX`.
    pub fn gather<T: crate::ReconSample>(
        &mut self,
        u: &[T],
        v: &[T],
        width: usize,
        stride: usize,
        origin: (usize, usize),
    ) -> bool {
        let ranges = gather_pair(
            u,
            v,
            width,
            stride,
            (&mut self.pad, &mut self.bytes),
            origin,
        );
        self.ranges = ranges.unwrap_or([u16::MAX; CDEF_SEGMENT_BLOCKS]);
        self.has_bytes = ranges.is_some() && T::u8_slice(u).is_some();
        ranges.is_some()
    }

    /// [`CdefPairSegment::block`] as offset bytes, for an 8-bit segment.
    #[inline]
    fn block_bytes(&self, block: usize) -> Option<&[u8; PAIR_BLOCK_BYTES]> {
        if !self.has_bytes || block >= CDEF_SEGMENT_BLOCKS {
            return None;
        }
        self.bytes.get(8 * block..)?.first_chunk()
    }

    /// [`CdefLumaSegment::block`] for the interleaved chroma pair.
    #[inline]
    pub fn block(&self, block: usize) -> Option<&[u16; CDEF_PAIR_SEGMENT_BLOCK_AREA]> {
        if block >= CDEF_SEGMENT_BLOCKS {
            return None;
        }
        self.pad.get(8 * block..)?.first_chunk()
    }

    /// The larger of block `block`'s two plane window ranges.
    #[inline]
    pub fn range(&self, block: usize) -> Option<u16> {
        self.ranges.get(block).copied()
    }
}

/// The `rows` rows of `span` samples from column `x0 - TAP_REACH`, row
/// `y0 - TAP_REACH` on, as one slice whose row `r` starts at `r * stride`,
/// provided they lie inside `width` and `samples` and rows `stride` apart do
/// not overlap (`stride` is at least `width`).
fn reach_rows<T>(
    samples: &[T],
    width: usize,
    stride: usize,
    (x0, y0): (usize, usize),
    (span, rows): (usize, usize),
) -> Option<&[T]> {
    let left = x0.checked_sub(TAP_REACH)?;
    if stride < width || left.checked_add(span)? > width {
        return None;
    }
    let first = y0
        .checked_sub(TAP_REACH)?
        .checked_mul(stride)?
        .checked_add(left)?;
    let len = (rows - 1).checked_mul(stride)?.checked_add(span)?;
    samples.get(first..)?.get(..len)
}

/// The first 16 samples as `u16` lanes.
#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn widen<S: SimdToken, T: crate::ReconSample>(simd: S, samples: &[T]) -> Option<u16x16<S>> {
    if let Some(samples) = T::u8_slice(samples) {
        return Some(widen_u8(simd, u8x16::from_slice(simd, samples.get(..16)?)));
    }
    Some(u16x16::from_slice(simd, T::u16_slice(samples)?.get(..16)?))
}

/// How a gather reduces eight `u16` lanes to their least and greatest.
trait LaneBounds<S: SimdToken>: Copy {
    fn bounds(self, simd: S, min: u16x8<S>, max: u16x8<S>) -> [u16; 2];
}

/// The generic lane reductions.
#[derive(Clone, Copy)]
struct Reduced;

impl<S: SimdToken> LaneBounds<S> for Reduced {
    #[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
    #[inline(always)]
    fn bounds(self, _simd: S, min: u16x8<S>, max: u16x8<S>) -> [u16; 2] {
        [min.reduce_min(), max.reduce_max()]
    }
}

/// `phminposuw` reductions, the greatest lane as the complement of the
/// least complemented one.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
struct MinPos;

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    #[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
    #[inline(always)]
    fn minpos_bounds(t: Avx2, min: u16x8<Avx2>, max: u16x8<Avx2>) -> [u16; 2] {
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

#[cfg(target_arch = "x86_64")]
impl LaneBounds<Avx2> for MinPos {
    #[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
    #[inline(always)]
    fn bounds(self, simd: Avx2, min: u16x8<Avx2>, max: u16x8<Avx2>) -> [u16; 2] {
        minpos_bounds(simd, min, max)
    }
}

/// The least and greatest lanes of `min` and `max` over lanes `0..8` and
/// lanes `0..4` of the next eighth `next_min`/`next_max`.
#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn window_bounds<S: SimdToken, R: LaneBounds<S>>(
    simd: S,
    reduce: R,
    (min, max): (u16x8<S>, u16x8<S>),
    (next_min, next_max): (u16x8<S>, u16x8<S>),
) -> [u16; 2] {
    let high = u16x8::from_slice(simd, &[0, 0, 0, 0, u16::MAX, u16::MAX, u16::MAX, u16::MAX]);
    reduce.bounds(simd, min.min(next_min | high), max.max(next_max & !high))
}

/// Copies the luma segment's tap rows into `pad` and returns the least and
/// greatest sample of each block's 12x12 window.
fn gather_luma<T: crate::ReconSample>(
    samples: &[T],
    width: usize,
    stride: usize,
    pad: (&mut [u16; LUMA_SEGMENT_AREA], &mut [u8; LUMA_BYTES_AREA]),
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return gather_luma_with(avx2, MinPos, samples, width, stride, pad, origin);
    }
    baseline_simd!(simd => gather_luma_with(simd, Reduced, samples, width, stride, pad, origin))
}

#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn gather_luma_with<S: SimdToken, R: LaneBounds<S>, T: crate::ReconSample>(
    simd: S,
    reduce: R,
    samples: &[T],
    width: usize,
    stride: usize,
    (pad, bytes): (&mut [u16; LUMA_SEGMENT_AREA], &mut [u8; LUMA_BYTES_AREA]),
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    const STARTS: [usize; 3] = [0, 16, CDEF_SEGMENT_STRIDE - 16];
    const { assert!(CDEF_SEGMENT_BLOCKS == 4 && CDEF_SEGMENT_STRIDE == 36) };
    let rows = reach_rows(samples, width, stride, origin, (CDEF_SEGMENT_STRIDE, 12))?;
    let mut wide_min = [u16x16::splat(simd, u16::MAX); 3];
    let mut wide_max = [u16x16::splat(simd, 0); 3];
    for row in 0..12 {
        let src = rows
            .get(row * stride..)?
            .first_chunk::<CDEF_SEGMENT_STRIDE>()?;
        let dst = pad
            .get_mut(row * CDEF_SEGMENT_STRIDE..)?
            .first_chunk_mut::<CDEF_SEGMENT_STRIDE>()?;
        let chunks = if let Some(src) = T::u8_slice(src) {
            let (head, tail) = (
                u8x32::from_slice(simd, &src[..32]),
                u8x32::from_slice(simd, &src[4..36]),
            );
            let bias = u8x32::splat(simd, 0x80);
            let row_bytes = bytes
                .get_mut(row * CDEF_SEGMENT_STRIDE..)?
                .first_chunk_mut::<CDEF_SEGMENT_STRIDE>()?;
            (head ^ bias).store_slice(&mut row_bytes[..32]);
            (tail ^ bias).store_slice(&mut row_bytes[4..]);
            let (first, second) = simd.widen_u8x32(head);
            let (_, third) = simd.widen_u8x32(tail);
            [first, second, third]
        } else {
            let src = T::u16_slice(src)?;
            STARTS.map(|start| u16x16::from_slice(simd, &src[start..start + 16]))
        };
        for (chunk, (start, lanes)) in STARTS.into_iter().zip(chunks).enumerate() {
            lanes.store_slice(&mut dst[start..start + 16]);
            wide_min[chunk] = wide_min[chunk].min(lanes);
            wide_max[chunk] = wide_max[chunk].max(lanes);
        }
    }
    Some(segment_bounds(simd, reduce, wide_min, wide_max))
}

/// The least and greatest sample of each block's 12-column window from the
/// column-wise least and greatest of a segment's rows at the chunk starts
/// `0`, `16` and `CDEF_SEGMENT_STRIDE - 16`.
#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn segment_bounds<S: SimdToken, R: LaneBounds<S>>(
    simd: S,
    reduce: R,
    wide_min: [u16x16<S>; 3],
    wide_max: [u16x16<S>; 3],
) -> [[u16; 2]; CDEF_SEGMENT_BLOCKS] {
    let eighths = |[a, b, c]: [u16x16<S>; 3]| {
        let (a, b, c) = (
            simd.split_u16x16(a),
            simd.split_u16x16(b),
            simd.split_u16x16(c),
        );
        [a.0, a.1, b.0, b.1, c.1]
    };
    let (min, max) = (eighths(wide_min), eighths(wide_max));
    core::array::from_fn(|block| {
        let next = if block + 1 < CDEF_SEGMENT_BLOCKS {
            (min[block + 1], max[block + 1])
        } else {
            let (min, max) = (min[4], max[4]);
            (
                simd.slide_u16x8::<4>(min, min),
                simd.slide_u16x8::<4>(max, max),
            )
        };
        window_bounds(simd, reduce, (min[block], max[block]), next)
    })
}

/// [`gather_luma`] into the byte scratch alone.
fn gather_luma_bytes(
    samples: &[u8],
    width: usize,
    stride: usize,
    bytes: &mut [u8; LUMA_BYTES_AREA],
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return gather_luma_bytes_with(avx2, MinPos, samples, width, stride, bytes, origin);
    }
    baseline_simd!(simd => gather_luma_bytes_with(simd, Reduced, samples, width, stride, bytes, origin))
}

#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn gather_luma_bytes_with<S: SimdToken, R: LaneBounds<S>>(
    simd: S,
    reduce: R,
    samples: &[u8],
    width: usize,
    stride: usize,
    bytes: &mut [u8; LUMA_BYTES_AREA],
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    const { assert!(CDEF_SEGMENT_BLOCKS == 4 && CDEF_SEGMENT_STRIDE == 36) };
    let rows = reach_rows(samples, width, stride, origin, (CDEF_SEGMENT_STRIDE, 12))?;
    let bias = u8x32::splat(simd, 0x80);
    let mut min = [u8x32::splat(simd, u8::MAX); 2];
    let mut max = [u8x32::splat(simd, 0); 2];
    for row in 0..12 {
        let src = rows
            .get(row * stride..)?
            .first_chunk::<CDEF_SEGMENT_STRIDE>()?;
        let row_bytes = bytes
            .get_mut(row * CDEF_SEGMENT_STRIDE..)?
            .first_chunk_mut::<CDEF_SEGMENT_STRIDE>()?;
        let lanes = [
            u8x32::from_slice(simd, &src[..32]),
            u8x32::from_slice(simd, &src[4..36]),
        ];
        (lanes[0] ^ bias).store_slice(&mut row_bytes[..32]);
        (lanes[1] ^ bias).store_slice(&mut row_bytes[4..]);
        for half in 0..2 {
            min[half] = min[half].min(lanes[half]);
            max[half] = max[half].max(lanes[half]);
        }
    }
    let chunks = |[head, tail]: [u8x32<S>; 2]| {
        let (first, second) = simd.widen_u8x32(head);
        [first, second, simd.widen_u8x32(tail).1]
    };
    Some(segment_bounds(simd, reduce, chunks(min), chunks(max)))
}

/// [`cdef_direction_segment`] of a block of an 8-bit segment from its byte
/// scratch, whose bytes are already the § 7.18.2 samples minus 128.
#[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
#[inline(always)]
fn direction_from_bytes(bytes: &[u8; LUMA_BLOCK_BYTES]) -> (usize, i32) {
    #[allow(clippy::inline_always, reason = "measured CDEF direction hot path")]
    #[inline(always)]
    fn rows<S: SimdToken>(simd: S, bytes: &[u8; LUMA_BLOCK_BYTES]) -> [i16x8<S>; 8] {
        core::array::from_fn(|i| {
            let start = (i + 2) * CDEF_SEGMENT_STRIDE + 2;
            let lanes = u8x16::from_slice(simd, &bytes[start..start + 16]).bitcast::<i8x16<S>>();
            simd.widen_i8x16(lanes).0
        })
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return cdef_best_direction(cdef_direction_costs_avx2(avx2, rows(avx2, bytes)));
    }
    baseline_simd!(simd => cdef_direction_rows(simd, rows(simd, bytes)))
}

/// Interleaves the chroma segment's tap rows, `PAIR_SPAN` samples of each
/// plane, into `pad` and returns for each block the larger of its two
/// planes' window ranges. Block `b` covers plane columns `4b..4b + 8`: of
/// the 16 columns from 0 and the 16 from 4 in each row, the halves hold
/// blocks 0, 2 and blocks 1, 3.
fn gather_pair<T: crate::ReconSample>(
    u_samples: &[T],
    v_samples: &[T],
    width: usize,
    stride: usize,
    pad: (&mut [u16; PAIR_SEGMENT_AREA], &mut [u8; PAIR_BYTES_AREA]),
    origin: (usize, usize),
) -> Option<[u16; CDEF_SEGMENT_BLOCKS]> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return gather_pair_with(
            avx2, MinPos, u_samples, v_samples, width, stride, pad, origin,
        );
    }
    baseline_simd!(simd => gather_pair_with(
        simd, Reduced, u_samples, v_samples, width, stride, pad, origin,
    ))
}

#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn gather_pair_with<S: SimdToken, R: LaneBounds<S>, T: crate::ReconSample>(
    simd: S,
    reduce: R,
    u_samples: &[T],
    v_samples: &[T],
    width: usize,
    stride: usize,
    (pad, bytes): (&mut [u16; PAIR_SEGMENT_AREA], &mut [u8; PAIR_BYTES_AREA]),
    origin: (usize, usize),
) -> Option<[u16; CDEF_SEGMENT_BLOCKS]> {
    const { assert!(CDEF_SEGMENT_BLOCKS == 4 && PAIR_SPAN == 20) };
    let u_rows = reach_rows(u_samples, width, stride, origin, (PAIR_SPAN, PAIR_ROWS))?;
    let v_rows = reach_rows(v_samples, width, stride, origin, (PAIR_SPAN, PAIR_ROWS))?;
    let mut min = [[u16x16::splat(simd, u16::MAX); 2]; 2];
    let mut max = [[u16x16::splat(simd, 0); 2]; 2];
    for row in 0..PAIR_ROWS {
        let u_row = u_rows.get(row * stride..)?.first_chunk::<PAIR_SPAN>()?;
        let v_row = v_rows.get(row * stride..)?.first_chunk::<PAIR_SPAN>()?;
        let lanes = pad
            .get_mut(row * CDEF_PAIR_SEGMENT_STRIDE..)?
            .first_chunk_mut::<CDEF_PAIR_SEGMENT_STRIDE>()?;
        if let (Some(u_row), Some(v_row)) = (T::u8_slice(u_row), T::u8_slice(v_row)) {
            let row_bytes = bytes
                .get_mut(row * CDEF_PAIR_SEGMENT_STRIDE..)?
                .first_chunk_mut::<CDEF_PAIR_SEGMENT_STRIDE>()?;
            let bias = u8x16::splat(simd, 0x80);
            for start in [0, PAIR_SPAN - 16] {
                let u = u8x16::from_slice(simd, u_row.get(start..start + 16)?) ^ bias;
                let v = u8x16::from_slice(simd, v_row.get(start..start + 16)?) ^ bias;
                simd.zip_low_u8x16(u, v)
                    .store_slice(&mut row_bytes[2 * start..2 * start + 16]);
                simd.zip_high_u8x16(u, v)
                    .store_slice(&mut row_bytes[2 * start + 16..2 * start + 32]);
            }
        }
        for (chunk, start) in [0, PAIR_SPAN - 16].into_iter().enumerate() {
            let planes = [widen(simd, &u_row[start..])?, widen(simd, &v_row[start..])?];
            for plane in 0..2 {
                min[plane][chunk] = min[plane][chunk].min(planes[plane]);
                max[plane][chunk] = max[plane][chunk].max(planes[plane]);
            }
            let (low, high) = simd.interleave_u16x16(planes[0], planes[1]);
            low.store_slice(&mut lanes[2 * start..2 * start + 16]);
            high.store_slice(&mut lanes[2 * start + 16..2 * start + 32]);
        }
    }
    let range = |plane: usize, chunk: usize, half: usize| {
        let (min, max) = (
            simd.split_u16x16(min[plane][chunk]),
            simd.split_u16x16(max[plane][chunk]),
        );
        let (min, max) = if half == 0 {
            (min.0, max.0)
        } else {
            (min.1, max.1)
        };
        let [least, greatest] = reduce.bounds(simd, min, max);
        greatest - least
    };
    let block = |chunk: usize, half: usize| range(0, chunk, half).max(range(1, chunk, half));
    Some([block(0, 0), block(1, 0), block(0, 1), block(1, 1)])
}

/// [`cdef_filter_block_chroma_pair`] for block `block` of `segment`. The
/// block's window range selects the kernel: the linear kernel, which skips
/// `constrain`, when `cdef_window_linear` admits the range; its `CLAMP`
/// mode, which clips each tap to the centre -/+ strength, when
/// `cdef_window_clamped` admits it; else the 32-lane kernel when
/// `cdef_wide8_fits` admits the filter, or the general kernel. The taps of
/// one lane all come from one plane, so the larger plane range bounds every
/// tap difference. Returns `false` when `block` is not below
/// [`CDEF_SEGMENT_BLOCKS`].
pub fn cdef_filter_block_chroma_pair_segment(
    segment: &CdefPairSegment,
    block: usize,
    filter: &CdefBlockFilter,
    out: &mut [u16; CDEF_PAIR_OUTPUT],
) -> bool {
    let (Some(pad), Some(range)) = (segment.block(block), segment.range(block)) else {
        return false;
    };
    let bytes = segment.block_bytes(block);
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return pair_segment_with(avx2, InLane, (pad, bytes), range, filter, out);
    }
    baseline_simd!(simd => pair_segment_with(
        simd,
        Ordered,
        (pad, bytes),
        range,
        filter,
        out,
    ))
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn pair_segment_with<S: SimdToken, Q: QuadLanes<S>>(
    simd: S,
    quads: Q,
    (pad, bytes): (
        &[u16; CDEF_PAIR_SEGMENT_BLOCK_AREA],
        Option<&[u8; PAIR_BLOCK_BYTES]>,
    ),
    range: u16,
    filter: &CdefBlockFilter,
    out: &mut [u16; CDEF_PAIR_OUTPUT],
) -> bool {
    const SPAN: usize = 3 * CDEF_PAIR_SEGMENT_STRIDE + 8;
    const AREA: usize = CDEF_PAIR_SEGMENT_BLOCK_AREA;
    const SPAN8: usize = SPAN + 8;
    const AREA8: usize = PAIR_BLOCK_BYTES;
    const STRIDE: usize = CDEF_PAIR_SEGMENT_STRIDE;
    let starts = &CDEF_PAIR_SEGMENT_TAP_STARTS;
    let args = (pad, 4, filter, starts, &mut out[..], 8);
    macro_rules! copy {
        () => {
            cdef_copy_centre_rows::<8, STRIDE, 4, 4, AREA>(pad, 4, out, 8)
        };
    }
    macro_rules! clamped {
        ($clamp:literal) => {
            cdef_by_families!(
                filter,
                cdef_filter_rows_clamped::<S, 8, STRIDE, 4, 4, SPAN, AREA, $clamp>(simd, args),
                copy!()
            )
            .is_some()
        };
    }
    let range = i32::from(range);
    if cdef_window_linear(filter, range) {
        return clamped!(false);
    }
    if cdef_window_clamped(filter, range) {
        return clamped!(true);
    }
    if cdef_wide8_fits(filter) {
        let Some(no_bytes) = NO_BYTES.first_chunk::<PAIR_BLOCK_BYTES>() else {
            return false;
        };
        return match bytes {
            Some(bytes) => cdef_by_families!(
                filter,
                cdef_filter_rows_wide8::<S, Q, STRIDE, 4, 4, SPAN, AREA, true, SPAN8, AREA8>(
                    simd, quads, bytes, args
                ),
                copy!()
            ),
            None => cdef_by_families!(
                filter,
                cdef_filter_rows_wide8::<S, Q, STRIDE, 4, 4, SPAN, AREA, false, SPAN8, AREA8>(
                    simd, quads, no_bytes, args
                ),
                copy!()
            ),
        }
        .is_some();
    }
    cdef_filter_block_rows::<S, 8, false, STRIDE, 4, 4, SPAN, AREA>(simd, args).is_some()
}

/// Whether [`cdef_filter_rows_wide8`] computes `filter` exactly: besides
/// [`cdef_i8_diffs_fit`], the primary sum `2 * pair0 + pair1`, at most
/// `6 * pri_str`, and the secondary sum `2 * family0 + family1`, at most
/// `12 * sec_str`, fit `i8`.
#[inline]
fn cdef_wide8_fits(filter: &CdefBlockFilter) -> bool {
    cdef_i8_diffs_fit(filter) && filter.pri_str <= 21 && filter.sec_str <= 10
}

/// How [`cdef_filter_rows_wide8`] packs four 8-lane rows, two `i16` row
/// pairs, into one `i8` vector and back.
trait QuadLanes<S: SimdToken>: Copy {
    /// The block rows, from the first of four, that the lanes hold in
    /// order, eight each.
    const ROWS: [usize; 4];
    /// The row pairs saturated to `i8`.
    fn narrow(self, simd: S, pairs: [i16x16<S>; 2]) -> i8x32<S>;
    /// The inverse of [`QuadLanes::narrow`] for lanes that fit `i8`.
    fn widen(self, simd: S, lanes: i8x32<S>) -> [i16x16<S>; 2];
    /// `diff`, whose absolute value is `magnitude`, clamped to `-clip..=clip`
    /// for a `clip` below 128.
    fn clamp(self, simd: S, diff: i8x32<S>, magnitude: u8x32<S>, clip: u8x32<S>) -> i8x32<S>;
}

/// Rows in order: the first pair fills lanes `0..16`.
#[derive(Clone, Copy)]
struct Ordered;

impl<S: SimdToken> QuadLanes<S> for Ordered {
    const ROWS: [usize; 4] = [0, 1, 2, 3];

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn narrow(self, simd: S, [low, high]: [i16x16<S>; 2]) -> i8x32<S> {
        simd.saturating_narrow_i16x16(low, high)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn widen(self, simd: S, lanes: i8x32<S>) -> [i16x16<S>; 2] {
        let (low, high) = simd.widen_i8x32(lanes);
        [low, high]
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn clamp(self, _simd: S, diff: i8x32<S>, _magnitude: u8x32<S>, clip: u8x32<S>) -> i8x32<S> {
        let clip = clip.bitcast::<i8x32<S>>();
        diff.min(clip).max(-clip)
    }
}

/// Rows 0, 2, 1, 3, the order in which an in-lane AVX2 saturating pack of
/// the `i16` row pairs leaves them.
#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
struct InLane;

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn pack_quad(t: Avx2, low: i16x16<Avx2>, high: i16x16<Avx2>) -> i8x32<Avx2> {
        core::arch::x86_64::_mm256_packs_epi16(low.into(), high.into()).simd_into(t)
    }
);

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn unpack_quad(t: Avx2, lanes: i8x32<Avx2>) -> [i16x16<Avx2>; 2] {
        use core::arch::x86_64::{
            _mm256_cmpgt_epi8, _mm256_setzero_si256, _mm256_unpackhi_epi8, _mm256_unpacklo_epi8,
        };
        let lanes = lanes.into();
        let sign = _mm256_cmpgt_epi8(_mm256_setzero_si256(), lanes);
        [
            _mm256_unpacklo_epi8(lanes, sign).simd_into(t),
            _mm256_unpackhi_epi8(lanes, sign).simd_into(t),
        ]
    }
);

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// The smaller of `magnitude` and `clip` with the sign of `diff`.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn signed_min(
        t: Avx2,
        diff: i8x32<Avx2>,
        magnitude: u8x32<Avx2>,
        clip: u8x32<Avx2>,
    ) -> i8x32<Avx2> {
        use core::arch::x86_64::{_mm256_min_epu8, _mm256_sign_epi8};
        _mm256_sign_epi8(_mm256_min_epu8(magnitude.into(), clip.into()), diff.into()).simd_into(t)
    }
);

#[cfg(target_arch = "x86_64")]
impl QuadLanes<Avx2> for InLane {
    const ROWS: [usize; 4] = [0, 2, 1, 3];

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn narrow(self, simd: Avx2, [low, high]: [i16x16<Avx2>; 2]) -> i8x32<Avx2> {
        pack_quad(simd, low, high)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn widen(self, simd: Avx2, lanes: i8x32<Avx2>) -> [i16x16<Avx2>; 2] {
        unpack_quad(simd, lanes)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn clamp(
        self,
        simd: Avx2,
        diff: i8x32<Avx2>,
        magnitude: u8x32<Avx2>,
        clip: u8x32<Avx2>,
    ) -> i8x32<Avx2> {
        signed_min(simd, diff, magnitude, clip)
    }
}

/// [`CdefConstrain8::apply_diff`] on 32 lanes.
#[derive(Clone, Copy)]
struct CdefConstrainWide<S: SimdToken> {
    threshold: u8x32<S>,
    shift: u32,
}

impl<S: SimdToken> CdefConstrainWide<S> {
    fn new(simd: S, threshold: i32, damping: i32) -> Self {
        Self {
            threshold: u8x32::splat(simd, threshold as u8),
            shift: constrain_damping_adj(threshold, damping) as u32,
        }
    }

    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply_diff<Q: QuadLanes<S>>(self, simd: S, quads: Q, diff: i8x32<S>) -> i8x32<S> {
        let magnitude = diff.abs().bitcast::<u8x32<S>>();
        let clip = self.threshold.saturating_sub(magnitude >> self.shift);
        quads.clamp(simd, diff, magnitude, clip)
    }
}

/// [`cdef_filter_rows_diff8`] over a segment's `ROWS`-row block, 8 lanes
/// wide, four rows per 32-lane vector, for filters [`cdef_wide8_fits`] admits. The constrained
/// taps are summed in `i8` per family, the primary as `2 * pair0 + pair1`
/// for `Cdef_Pri_Taps` `[4, 2]` or `pair0 + pair1` for `[3, 3]`, then widen
/// once and take the remaining weight 2 or 3.
///
/// Kept out of line: inlined into the kernel dispatch it ran more
/// instructions.
#[inline(never)]
fn cdef_filter_rows_wide8<
    S: SimdToken,
    Q: QuadLanes<S>,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const BYTES: bool,
    const SPAN8: usize,
    const AREA8: usize,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    quads: Q,
    bytes: &[u8; AREA8],
    (pad, _, filter, starts, out, out_stride): CdefRowsArgs<'_, AREA>,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<_, STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
    let (center_bytes, pri_bytes, sec_bytes) =
        cdef_tap_views::<_, STRIDE, CENTER, SPAN8, AREA8, PRI, SEC>(bytes, filter.dir, starts)?;
    let heavy_first = (filter.pri_str >> filter.coeff_shift) & 1 == 0;
    let pri_double = i8x32::splat(simd, if heavy_first { -1 } else { 0 });
    let pri_weight: i16 = if heavy_first { 2 } else { 3 };
    let pri = CdefConstrainWide::new(simd, filter.pri_str, filter.damping);
    let sec = CdefConstrainWide::new(simd, filter.sec_str, filter.damping);
    for first in (0..ROWS).step_by(4) {
        let quad = |view: &[u16; SPAN]| -> Option<[i16x16<S>; 2]> {
            Some([
                cdef_row_pair_i16::<S, STRIDE, SPAN>(simd, view, first)?,
                cdef_row_pair_i16::<S, STRIDE, SPAN>(simd, view, first + 2)?,
            ])
        };
        let byte_quad = |view: &[u8; SPAN8]| -> Option<i8x32<S>> {
            let row = |index: usize| -> Option<u64x2<S>> {
                let at = (first + Q::ROWS[index]) * STRIDE;
                Some(u8x16::from_slice(simd, view.get(at..at + 16)?).bitcast())
            };
            let low = simd.zip_low_u64x2(row(0)?, row(1)?);
            let high = simd.zip_low_u64x2(row(2)?, row(3)?);
            Some(simd.combine_u64x2(low, high).bitcast())
        };
        let center_signed = if BYTES {
            byte_quad(center_bytes)?
        } else {
            i8x32::splat(simd, 0)
        };
        let center = if BYTES {
            quads.widen(simd, center_signed).map(|center| center + 128)
        } else {
            quad(center_view)?
        };
        let (mut min, mut max) = (i8x32::splat(simd, 0), i8x32::splat(simd, 0));
        macro_rules! tap {
            ($view:expr, $constrain:expr) => {{
                let diff = if BYTES {
                    byte_quad($view.1)?.saturating_sub(center_signed)
                } else {
                    let [low, high] = quad($view.0)?;
                    quads.narrow(simd, [low - center[0], high - center[1]])
                };
                if PRI && SEC {
                    min = min.min(diff);
                    max = max.max(diff);
                }
                $constrain.apply_diff(simd, quads, diff)
            }};
        }
        let mut sum = [i16x16::splat(simd, 0); 2];
        if PRI {
            let view = |k: usize| (pri_views[k], pri_bytes[k]);
            let pair0 = tap!(view(0), pri) + tap!(view(1), pri);
            let pair1 = tap!(view(2), pri) + tap!(view(3), pri);
            let primary = quads.widen(simd, pair0 + pair1 + (pair0 & pri_double));
            sum = primary.map(|primary| primary * pri_weight);
        }
        if SEC {
            let view = |k: usize| (sec_views[k], sec_bytes[k]);
            let family0 =
                tap!(view(0), sec) + tap!(view(1), sec) + tap!(view(2), sec) + tap!(view(3), sec);
            let family1 =
                tap!(view(4), sec) + tap!(view(5), sec) + tap!(view(6), sec) + tap!(view(7), sec);
            let secondary = quads.widen(simd, family0 + family0 + family1);
            sum = [sum[0] + secondary[0], sum[1] + secondary[1]];
        }
        let offset = sum.map(cdef_round);
        let offset = if PRI && SEC {
            quads.widen(simd, quads.narrow(simd, offset).max(min).min(max))
        } else {
            offset
        };
        for half in 0..2 {
            let filtered = (center[half] + offset[half]).bitcast::<u16x16<S>>();
            cdef_store_row_pair::<S, 8>(filtered, out, out_stride, first + 2 * half, ROWS)?;
        }
    }
    Some(())
}

/// AV2 § 7.18.3 CDEF over interior 8x8 luma block `block` of `segment`.
/// With `use_range`, the block's window range selects the kernel: the
/// linear kernel, which skips `constrain`, when `cdef_window_linear` admits
/// the range; its `CLAMP` mode, which clips each tap to the centre -/+
/// strength, when `cdef_window_clamped` admits it; else the 32-lane kernel
/// when `cdef_wide8_fits` admits the filter, or `u8` lanes when the range is
/// below 256 and the filter fits them. Other blocks take the
/// kernel of [`cdef_filter_block_interior_to`]. Returns `false` without
/// writing when `block` is not below [`CDEF_SEGMENT_BLOCKS`], `out_stride` is
/// below 8, or `out` cannot hold the block at `out_stride`.
pub fn cdef_filter_block_segment(
    segment: &CdefLumaSegment,
    block: usize,
    filter: &CdefBlockFilter,
    use_range: bool,
    out: &mut [u16],
    out_stride: usize,
) -> bool {
    let (Some(pad), Some(range)) = (segment.block(block), segment.range(block)) else {
        return false;
    };
    let end = out_stride
        .checked_mul(7)
        .and_then(|rows| rows.checked_add(8));
    if out_stride < 8 || end.is_none_or(|end| out.len() < end) {
        return false;
    }
    let range = use_range.then_some(range);
    let pad = (pad, segment.block_bytes(block));
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return luma_segment_with(avx2, InLane, pad, range, filter, (out, out_stride));
    }
    baseline_simd!(simd => luma_segment_with(
        simd,
        Ordered,
        pad,
        range,
        filter,
        (out, out_stride),
    ))
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn luma_segment_with<S: SimdToken, Q: QuadLanes<S>>(
    simd: S,
    quads: Q,
    (pad, bytes): (
        &[u16; CDEF_SEGMENT_BLOCK_AREA],
        Option<&[u8; LUMA_BLOCK_BYTES]>,
    ),
    range: Option<[u16; 2]>,
    filter: &CdefBlockFilter,
    (out, out_stride): (&mut [u16], usize),
) -> bool {
    const SPAN: usize = 7 * CDEF_SEGMENT_STRIDE + 8;
    const AREA: usize = CDEF_SEGMENT_BLOCK_AREA;
    const SPAN8: usize = SPAN + 8;
    const AREA8: usize = LUMA_BLOCK_BYTES;
    const STRIDE: usize = CDEF_SEGMENT_STRIDE;
    let starts = &CDEF_SEGMENT_TAP_STARTS;
    let args = (pad, 8, filter, starts, &mut *out, out_stride);
    let Some([base, max]) = range else {
        return cdef_filter_block_rows::<S, 8, false, STRIDE, 2, 8, SPAN, AREA>(simd, args)
            .is_some();
    };
    let range = i32::from(max) - i32::from(base);
    macro_rules! copy {
        () => {
            cdef_copy_centre_rows::<8, STRIDE, 2, 8, AREA>(pad, 8, out, out_stride)
        };
    }
    macro_rules! clamped {
        ($clamp:literal) => {
            cdef_by_families!(
                filter,
                cdef_filter_rows_clamped::<S, 8, STRIDE, 2, 8, SPAN, AREA, $clamp>(simd, args),
                copy!()
            )
            .is_some()
        };
    }
    if cdef_window_linear(filter, range) {
        return clamped!(false);
    }
    if cdef_window_clamped(filter, range) {
        return clamped!(true);
    }
    if cdef_wide8_fits(filter) {
        return match bytes {
            Some(bytes) => cdef_by_families!(
                filter,
                cdef_filter_rows_wide8::<S, Q, STRIDE, 2, 8, SPAN, AREA, true, SPAN8, AREA8>(
                    simd, quads, bytes, args
                ),
                copy!()
            ),
            None => cdef_by_families!(
                filter,
                cdef_filter_rows_wide8::<S, Q, STRIDE, 2, 8, SPAN, AREA, false, SPAN8, AREA8>(
                    simd, quads, &NO_BYTES, args
                ),
                copy!()
            ),
        }
        .is_some();
    }
    if range <= 255 && cdef_u8_lanes_fit(filter) {
        return cdef_by_families!(
            filter,
            cdef_filter_rows_8bit::<S, 8, STRIDE, 2, 8, SPAN, AREA>(simd, args, base),
            copy!()
        )
        .is_some();
    }
    cdef_filter_block_rows::<S, 8, false, STRIDE, 2, 8, SPAN, AREA>(simd, args).is_some()
}

/// The 32-lane kernel of [`luma_segment_with`] alone, for a block of a
/// bytes-only segment; `false` when the filter does not fit it.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn luma_wide8_with<S: SimdToken, Q: QuadLanes<S>>(
    simd: S,
    quads: Q,
    (pad, bytes): (
        &[u16; CDEF_SEGMENT_BLOCK_AREA],
        Option<&[u8; LUMA_BLOCK_BYTES]>,
    ),
    filter: &CdefBlockFilter,
    (out, out_stride): (&mut [u16], usize),
) -> bool {
    const SPAN: usize = 7 * CDEF_SEGMENT_STRIDE + 8;
    const AREA: usize = CDEF_SEGMENT_BLOCK_AREA;
    const SPAN8: usize = SPAN + 8;
    const AREA8: usize = LUMA_BLOCK_BYTES;
    const STRIDE: usize = CDEF_SEGMENT_STRIDE;
    let (Some(bytes), true) = (bytes, cdef_wide8_fits(filter)) else {
        return false;
    };
    let args = (pad, 8, filter, &CDEF_SEGMENT_TAP_STARTS, out, out_stride);
    cdef_by_families!(
        filter,
        cdef_filter_rows_wide8::<S, Q, STRIDE, 2, 8, SPAN, AREA, true, SPAN8, AREA8>(
            simd, quads, bytes, args
        ),
        Some(())
    )
    .is_some()
}

/// A 64x64 unit's § 7.18.1 strengths and damping as signalled, `BitDepth -
/// 8`, and the `Cdef_Uv_Dir[SubsamplingX][SubsamplingY]` row.
#[derive(Clone, Copy, Debug)]
pub struct CdefStrengths {
    /// `cdef_y_pri_strength`.
    pub y_pri: i32,
    /// `cdef_y_sec_strength`.
    pub y_sec: i32,
    /// `cdef_uv_pri_strength`.
    pub uv_pri: i32,
    /// `cdef_uv_sec_strength`.
    pub uv_sec: i32,
    /// `CdefDamping`.
    pub damping: i32,
    /// `BitDepth - 8`.
    pub coeff_shift: u32,
    /// The chroma direction of each luma `yDir`.
    pub uv_dir: [usize; 8],
}

impl CdefStrengths {
    /// The § 7.18.1 luma and chroma filters of a block whose § 7.18.2
    /// direction is `y_dir` with variance `var`.
    #[inline]
    pub fn filters(&self, y_dir: usize, var: i32) -> [CdefBlockFilter; 2] {
        let shift = self.coeff_shift;
        let pri_base = self.y_pri << shift;
        let uv_pri = self.uv_pri << shift;
        let var_str = (var >> 6).checked_ilog2().unwrap_or(0).min(12) as i32;
        let damping = self.damping + shift as i32;
        [
            CdefBlockFilter {
                pri_str: if var != 0 {
                    (pri_base * (4 + var_str) + 8) >> 4
                } else {
                    0
                },
                sec_str: self.y_sec << shift,
                damping,
                dir: if pri_base == 0 { 0 } else { y_dir },
                coeff_shift: shift,
            },
            CdefBlockFilter {
                pri_str: uv_pri,
                sec_str: self.uv_sec << shift,
                damping: damping - 1,
                dir: if uv_pri == 0 {
                    0
                } else {
                    self.uv_dir[y_dir & 7]
                },
                coeff_shift: shift,
            },
        ]
    }

    /// Whether every luma filter [`CdefStrengths::filters`] can give, over
    /// every variance, fits the 32-lane kernel of 8-bit samples, so that a
    /// segment gathered with [`CdefLumaSegment::gather_bytes`] filters.
    pub fn luma_bytes_suffice(&self) -> bool {
        self.coeff_shift == 0
            && (0..=13).all(|var_log| {
                let var = if var_log == 0 { 0 } else { 64 << (var_log - 1) };
                cdef_wide8_fits(&self.filters(0, var)[0])
            })
    }
}

/// Which planes § 7.18.1 filters in one block of a segment: `luma` unless
/// the block is luma-lossless, `chroma` unless both chroma planes are
/// lossless. A block with neither, or one whose 8x8 is all skipped, is left
/// as it is.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdefSegmentBlock {
    /// Whether the luma block is filtered.
    pub luma: bool,
    /// Whether the chroma pair is filtered.
    pub chroma: bool,
}

/// One segment's output rows: luma rows from the segment's first sample on
/// with their stride, and, when its chroma pair was gathered, the pair
/// segment and the U and V rows.
pub type CdefSegmentOutput<'a> = (
    (&'a mut [u16], usize),
    Option<(&'a CdefPairSegment, [(&'a mut [u16], usize); 2])>,
);

/// AV2 § 7.18.2 and § 7.18.3 CDEF over the [`CDEF_SEGMENT_BLOCKS`] blocks of
/// a gathered segment with the unit's `strengths`. Each block takes the
/// direction and kernel of [`cdef_filter_block_segment`] and
/// [`cdef_filter_block_chroma_pair_segment`]; a block whose window is flat
/// filters to itself, and is written only for the planes `fill_flat` marks
/// (luma, U, V). Returns `false` when the rows cannot hold the segment.
pub fn cdef_filter_segment(
    segment: &CdefLumaSegment,
    strengths: &CdefStrengths,
    blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
    fill_flat: [bool; 3],
    output: CdefSegmentOutput<'_>,
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return segment_with(avx2, InLane, segment, strengths, blocks, fill_flat, output);
    }
    baseline_simd!(simd => segment_with(
        simd, Ordered, segment, strengths, blocks, fill_flat, output,
    ))
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn segment_with<S: SimdToken, Q: QuadLanes<S>>(
    simd: S,
    quads: Q,
    segment: &CdefLumaSegment,
    strengths: &CdefStrengths,
    blocks: [CdefSegmentBlock; CDEF_SEGMENT_BLOCKS],
    fill_flat: [bool; 3],
    ((luma_out, luma_stride), mut chroma): CdefSegmentOutput<'_>,
) -> bool {
    const WIDTH: usize = 8 * CDEF_SEGMENT_BLOCKS;
    let fits = |len: usize, stride: usize, rows: usize, width: usize| {
        stride >= width
            && stride
                .checked_mul(rows - 1)
                .and_then(|span| span.checked_add(width))
                .is_some_and(|span| span <= len)
    };
    if !fits(luma_out.len(), luma_stride, 8, WIDTH)
        || chroma.as_ref().is_some_and(|(_, planes)| {
            planes
                .iter()
                .any(|(rows, stride)| !fits(rows.len(), *stride, 4, WIDTH / 2))
        })
    {
        return false;
    }
    let shift = strengths.coeff_shift;
    let pri_base = strengths.y_pri << shift;
    let uv_pri = strengths.uv_pri << shift;
    let luma_active = strengths.y_sec != 0 || pri_base != 0;
    for (block, filters) in blocks.into_iter().enumerate() {
        let chroma_block = chroma.as_mut().filter(|_| filters.chroma);
        if !filters.luma && chroma_block.is_none() {
            continue;
        }
        let (Some(luma), Some([least, greatest])) = (segment.scratch(block), segment.range(block))
        else {
            return false;
        };
        let bytes = segment.block_bytes(block);
        let use_range = filters.luma && luma_active;
        let luma_flat = use_range && least == greatest;
        let (y_dir, var) = if (pri_base == 0 && uv_pri == 0) || luma_flat {
            (0, 0)
        } else if segment.has_pad {
            cdef_direction_strided::<CDEF_SEGMENT_STRIDE, CDEF_SEGMENT_BLOCK_AREA>(luma, shift)
        } else if let Some(bytes) = bytes {
            direction_from_bytes(bytes)
        } else {
            return false;
        };
        let [y_filter, uv_filter] = strengths.filters(y_dir, var);
        if filters.luma && (y_filter.pri_str != 0 || y_filter.sec_str != 0) {
            let Some(out) = luma_out.get_mut(8 * block..) else {
                return false;
            };
            let written = if luma_flat {
                !fill_flat[0] || fill_rows(out, luma_stride, 8, least).is_some()
            } else if segment.has_pad {
                luma_segment_with(
                    simd,
                    quads,
                    (luma, bytes),
                    Some([least, greatest]),
                    &y_filter,
                    (out, luma_stride),
                )
            } else {
                luma_wide8_with(simd, quads, (luma, bytes), &y_filter, (out, luma_stride))
            };
            if !written {
                return false;
            }
        }
        let Some((pair, planes)) = chroma_block else {
            continue;
        };
        if uv_filter.pri_str == 0 && uv_filter.sec_str == 0 {
            continue;
        }
        let (Some(pad), Some(range)) = (pair.block(block), pair.range(block)) else {
            return false;
        };
        let [(u, u_stride), (v, v_stride)] = planes;
        let (Some(u), Some(v)) = (u.get_mut(4 * block..), v.get_mut(4 * block..)) else {
            return false;
        };
        let written = if range == 0 {
            (!fill_flat[1] || fill_rows(u, *u_stride, 4, pad[0]).is_some())
                && (!fill_flat[2] || fill_rows(v, *v_stride, 4, pad[1]).is_some())
        } else {
            let mut output = [0u16; CDEF_PAIR_OUTPUT];
            pair_segment_with(
                simd,
                quads,
                (pad, pair.block_bytes(block)),
                range,
                &uv_filter,
                &mut output,
            ) && publish_pair(simd, &output, [(u, *u_stride), (v, *v_stride)]).is_some()
        };
        if !written {
            return false;
        }
    }
    true
}

/// Fills the first `side` samples of `side` rows of `rows`, `stride` apart,
/// with `value`.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn fill_rows(rows: &mut [u16], stride: usize, side: usize, value: u16) -> Option<()> {
    for row in 0..side {
        rows.get_mut(row * stride..)?.get_mut(..side)?.fill(value);
    }
    Some(())
}

/// Writes the interleaved chroma pair `output` into the four rows of the U
/// and V `planes`. It loads `output` one 8-lane row at a time, as the
/// kernels store it, so that each load forwards from one store.
#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn publish_pair<S: SimdToken>(
    simd: S,
    output: &[u16; CDEF_PAIR_OUTPUT],
    planes: [(&mut [u16], usize); 2],
) -> Option<()> {
    let row = |row: usize| u16x8::from_slice(simd, &output[8 * row..8 * row + 8]);
    let (low, high) = (
        simd.combine_u16x8(row(0), row(1)),
        simd.combine_u16x8(row(2), row(3)),
    );
    let lanes = [
        simd.unzip_low_u16x16(low, high),
        simd.unzip_high_u16x16(low, high),
    ]
    .map(SimdBase::to_array);
    for (lanes, (out, stride)) in lanes.iter().zip(planes) {
        let rows = out.get_mut(..stride.checked_mul(3)?.checked_add(4)?)?;
        for (row, lanes) in lanes.chunks_exact(4).enumerate() {
            let start = row * stride;
            rows.get_mut(start..start + 4)?.copy_from_slice(lanes); // splot-copy-ok: publish the pair's deinterleaved samples
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, seed: u32, value: impl Fn(u32) -> u16) -> Vec<u16> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                value(state)
            })
            .collect()
    }

    fn window_bounds(window: impl Iterator<Item = u16>) -> [u16; 2] {
        window.fold([u16::MAX, 0], |[min, max], sample| {
            [min.min(sample), max.max(sample)]
        })
    }

    #[test]
    fn luma_gather_records_each_block_window_range() {
        let (width, stride) = (44, 48);
        let samples = noise(stride * 14, 0x2468_ace1, |state| (state >> 22) as u16);
        let mut segment = CdefLumaSegment::default();
        assert!(segment.gather(&samples, width, stride, (4, 3)));
        for block in 0..CDEF_SEGMENT_BLOCKS {
            let window = (1..13).flat_map(|y| {
                let start = y * stride + 2 + 8 * block;
                samples[start..start + 12].iter().copied()
            });
            assert_eq!(
                segment.range(block),
                Some(window_bounds(window)),
                "block {block}"
            );
        }
        for row in 0..12 {
            let start = (row + 1) * stride + 2;
            assert_eq!(
                &segment.pad[row * CDEF_SEGMENT_STRIDE..(row + 1) * CDEF_SEGMENT_STRIDE],
                &samples[start..start + CDEF_SEGMENT_STRIDE]
            );
        }
    }

    /// The plane of squares `x^2` makes each block's range depend on where its
    /// window starts, and either plane may hold the larger range.
    #[test]
    fn pair_gather_records_each_block_plane_range() {
        let stride = 24;
        let random = noise(stride * 10, 0x1357_9bdf, |state| {
            300 + ((state >> 13) % 40) as u16
        });
        let square: Vec<u16> = (0..stride * 10)
            .map(|index| 300 + ((index % stride) * (index % stride)) as u16)
            .collect();
        for (u, v) in [(&random, &square), (&square, &random)] {
            let mut segment = CdefPairSegment::default();
            assert!(segment.gather(u, v, stride, stride, (3, 3)));
            for block in 0..CDEF_SEGMENT_BLOCKS {
                let plane_range = |samples: &[u16]| {
                    let [min, max] = window_bounds((1..9).flat_map(|y| {
                        let start = y * stride + 1 + 4 * block;
                        samples[start..start + 8].iter().copied()
                    }));
                    max - min
                };
                assert_eq!(
                    segment.range(block),
                    Some(plane_range(u).max(plane_range(v))),
                    "block {block}"
                );
            }
            for row in 0..8 {
                for col in 0..PAIR_SPAN {
                    let start = (row + 1) * stride + 1 + col;
                    let lanes = &segment.pad[row * CDEF_PAIR_SEGMENT_STRIDE + 2 * col..][..2];
                    assert_eq!(lanes, [u[start], v[start]]);
                }
            }
        }
    }

    #[test]
    fn gather_outside_the_rows_fails_with_full_ranges() {
        let (width, stride) = (44, 48);
        let samples = vec![7u16; stride * 14];
        let mut luma = CdefLumaSegment::default();
        let mut pair = CdefPairSegment::default();
        for (luma_width, origin, len) in [
            (width, (1, 3), samples.len()),
            (width, (4, 1), samples.len()),
            (width, (11, 3), samples.len()),
            (width, (4, 3), 405),
            (width, (4, usize::MAX / 8), samples.len()),
            (usize::MAX, (usize::MAX, 3), samples.len()),
        ] {
            assert!(luma.gather(&samples, width, stride, (4, 3)));
            assert_eq!(luma.range(0), Some([7, 7]));
            assert!(!luma.gather(&samples[..len], luma_width, stride, origin));
            assert_eq!(luma.range(3), Some([0, u16::MAX]), "{origin:?}");
            assert!(pair.gather(&samples, &samples, 24, stride, (3, 3)));
            assert_eq!(pair.range(0), Some(0));
            assert!(!pair.gather(&samples, &samples[..len], 24, stride, origin));
            assert_eq!(pair.range(3), Some(u16::MAX), "{origin:?}");
        }
        assert!(!luma.gather(&samples, width, usize::MAX / 4, (4, 3)));
        assert!(!pair.gather(&samples, &samples, 24, usize::MAX / 4, (3, 3)));
        for narrow in [0, width - 1] {
            assert!(!luma.gather(&samples, width, narrow, (4, 3)));
        }
        for narrow in [0, 23] {
            assert!(!pair.gather(&samples, &samples, 24, narrow, (3, 3)));
        }
    }

    /// The in-lane AVX2 row order of the 32-lane kernel filters like the
    /// portable one.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn wide8_lane_orders_agree() {
        let Some(avx2) = Level::baseline().as_avx2() else {
            return;
        };
        for seed in 0..4 {
            let luma = noise(48 * 14, 0x2545_f491 ^ seed, |state| (state >> 24) as u16);
            let chroma = noise(24 * 10, 0x6c07_8965 ^ seed, |state| (state >> 24) as u16);
            let (mut segment, mut pair) = (CdefLumaSegment::default(), CdefPairSegment::default());
            assert!(segment.gather(&luma, 44, 48, (4, 3)));
            assert!(pair.gather(&chroma, &chroma, 24, 24, (3, 3)));
            let (luma8, chroma8): (Vec<u8>, Vec<u8>) = (
                luma.iter().map(|&v| v as u8).collect(),
                chroma.iter().rev().map(|&v| v as u8).collect(),
            );
            let (mut segment8, mut pair8) =
                (CdefLumaSegment::default(), CdefPairSegment::default());
            assert!(segment8.gather(&luma8, 44, 48, (4, 3)));
            assert!(pair8.gather(&luma8[..240], &chroma8, 24, 24, (3, 3)));
            for (dir, (pri_str, sec_str)) in
                (0..8).zip([(4, 2), (21, 0), (0, 10), (12, 8)].iter().cycle())
            {
                let filter = CdefBlockFilter {
                    pri_str: *pri_str,
                    sec_str: *sec_str,
                    damping: 3 + dir as i32 % 4,
                    dir,
                    coeff_shift: 0,
                };
                assert!(cdef_wide8_fits(&filter));
                for block in 0..CDEF_SEGMENT_BLOCKS {
                    let (Some(luma), Some(chroma)) = (segment.block(block), pair.block(block))
                    else {
                        unreachable!("blocks below CDEF_SEGMENT_BLOCKS exist");
                    };
                    let range = Some([0, 1000]);
                    let (mut ordered, mut packed) = ([0u16; 64], [0u16; 64]);
                    assert!(luma_segment_with(
                        avx2,
                        Ordered,
                        (luma, None),
                        range,
                        &filter,
                        (&mut ordered, 8)
                    ));
                    assert!(luma_segment_with(
                        avx2,
                        InLane,
                        (luma, None),
                        range,
                        &filter,
                        (&mut packed, 8)
                    ));
                    assert_eq!(ordered, packed, "luma block {block}, {filter:?}");
                    let (mut ordered, mut packed) = ([0; CDEF_PAIR_OUTPUT], [0; CDEF_PAIR_OUTPUT]);
                    assert!(pair_segment_with(
                        avx2,
                        Ordered,
                        (chroma, None),
                        1000,
                        &filter,
                        &mut ordered
                    ));
                    assert!(pair_segment_with(
                        avx2,
                        InLane,
                        (chroma, None),
                        1000,
                        &filter,
                        &mut packed
                    ));
                    assert_eq!(ordered, packed, "chroma block {block}, {filter:?}");
                    let (Some(luma), Some(chroma)) = (segment8.block(block), pair8.block(block))
                    else {
                        unreachable!("blocks below CDEF_SEGMENT_BLOCKS exist");
                    };
                    let (luma_bytes, chroma_bytes) =
                        (segment8.block_bytes(block), pair8.block_bytes(block));
                    assert!(luma_bytes.is_some() && chroma_bytes.is_some());
                    let mut lanes = [[0u16; 64]; 3];
                    for (out, (quads, bytes)) in lanes.iter_mut().zip([
                        (true, None),
                        (true, luma_bytes),
                        (false, luma_bytes),
                    ]) {
                        assert!(if quads {
                            luma_segment_with(avx2, InLane, (luma, bytes), range, &filter, (out, 8))
                        } else {
                            luma_segment_with(
                                avx2,
                                Ordered,
                                (luma, bytes),
                                range,
                                &filter,
                                (out, 8),
                            )
                        });
                    }
                    assert!(
                        lanes[1] == lanes[0] && lanes[2] == lanes[0],
                        "luma bytes {filter:?}"
                    );
                    let mut lanes = [[0u16; CDEF_PAIR_OUTPUT]; 3];
                    for (out, (quads, bytes)) in lanes.iter_mut().zip([
                        (true, None),
                        (true, chroma_bytes),
                        (false, chroma_bytes),
                    ]) {
                        assert!(if quads {
                            pair_segment_with(avx2, InLane, (chroma, bytes), 1000, &filter, out)
                        } else {
                            pair_segment_with(avx2, Ordered, (chroma, bytes), 1000, &filter, out)
                        });
                    }
                    assert!(
                        lanes[1] == lanes[0] && lanes[2] == lanes[0],
                        "chroma bytes {filter:?}"
                    );
                }
            }
        }
    }

    /// The segment filter writes what its per-block parts write, for
    /// skipped, lossless, flat and filtered blocks.
    #[test]
    fn segment_filter_matches_its_blocks() {
        let (width, stride) = (44, 48);
        let mut luma = noise(stride * 14, 0x3c6e_f372, |state| 60 + (state >> 26) as u16);
        for row in 1..13 {
            luma[row * stride + 26..row * stride + 38].fill(90);
        }
        let chroma = noise(24 * 10, 0x1f83_d9ab, |state| 40 + (state >> 27) as u16);
        let (mut segment, mut pair) = (CdefLumaSegment::default(), CdefPairSegment::default());
        assert!(segment.gather(&luma[..stride * 14], width, stride, (4, 3)));
        assert!(pair.gather(&chroma, &chroma[2..], 22, 24, (3, 3)));
        let strengths = CdefStrengths {
            y_pri: 6,
            y_sec: 2,
            uv_pri: 3,
            uv_sec: 1,
            damping: 4,
            coeff_shift: 0,
            uv_dir: CDEF_UV_DIR[1][1],
        };
        let blocks = [(true, true), (false, true), (true, false), (true, true)]
            .map(|(luma, chroma)| CdefSegmentBlock { luma, chroma });
        let (mut out, mut u, mut v) = ([7u16; 40 * 8], [7u16; 20 * 4], [7u16; 20 * 4]);
        assert!(cdef_filter_segment(
            &segment,
            &strengths,
            blocks,
            [true; 3],
            ((&mut out, 40), Some((&pair, [(&mut u, 20), (&mut v, 20)]))),
        ));
        let (mut want, mut want_u, mut want_v) = ([7u16; 40 * 8], [7u16; 20 * 4], [7u16; 20 * 4]);
        for (block, filters) in blocks.into_iter().enumerate() {
            let (Some(pad), Some([least, greatest])) = (segment.block(block), segment.range(block))
            else {
                unreachable!("blocks below CDEF_SEGMENT_BLOCKS exist");
            };
            let flat = filters.luma && least == greatest;
            let (y_dir, var) = if flat {
                (0, 0)
            } else {
                cdef_direction_segment(pad, 0)
            };
            let [y_filter, uv_filter] = strengths.filters(y_dir, var);
            if flat {
                for row in 0..8 {
                    want[row * 40 + 8 * block..][..8].fill(pad[0]);
                }
            } else if filters.luma {
                assert!(cdef_filter_block_segment(
                    &segment,
                    block,
                    &y_filter,
                    true,
                    &mut want[8 * block..],
                    40
                ));
            }
            if filters.chroma {
                let mut lanes = [0u16; CDEF_PAIR_OUTPUT];
                assert!(cdef_filter_block_chroma_pair_segment(
                    &pair, block, &uv_filter, &mut lanes
                ));
                for row in 0..4 {
                    for col in 0..4 {
                        want_u[row * 20 + 4 * block + col] = lanes[row * 8 + 2 * col];
                        want_v[row * 20 + 4 * block + col] = lanes[row * 8 + 2 * col + 1];
                    }
                }
            }
        }
        assert_eq!(segment.range(3).map(|[min, max]| min == max), Some(true));
        assert_eq!(out, want);
        assert_eq!((u, v), (want_u, want_v));
        let short = &mut [0u16; 40 * 7 + 31];
        assert!(!cdef_filter_segment(
            &segment,
            &strengths,
            blocks,
            [true; 3],
            ((short, 40), None),
        ));
    }

    /// A bytes-only 8-bit luma gather filters every block like the full
    /// one, flat windows included.
    #[test]
    fn bytes_only_segments_match_full_gathers() {
        let (width, stride) = (44, 48);
        let mut luma: Vec<u8> = noise(stride * 14, 0x51ed_270b, |state| (state >> 24) as u16)
            .into_iter()
            .map(|sample| sample as u8)
            .collect();
        for row in 1..13 {
            luma[row * stride + 10..row * stride + 22].fill(77);
        }
        let chroma: Vec<u8> = noise(24 * 10, 0x2c1b_3c6d, |state| (state >> 24) as u16)
            .into_iter()
            .map(|sample| sample as u8)
            .collect();
        let mut pair = CdefPairSegment::default();
        assert!(pair.gather(&chroma, &chroma[1..], 22, 24, (3, 3)));
        for (y_pri, y_sec, uv_pri, uv_sec, damping) in [
            (15, 4, 15, 4, 6),
            (3, 0, 0, 2, 3),
            (0, 1, 7, 0, 5),
            (9, 2, 2, 1, 4),
        ] {
            let strengths = CdefStrengths {
                y_pri,
                y_sec,
                uv_pri,
                uv_sec,
                damping,
                coeff_shift: 0,
                uv_dir: CDEF_UV_DIR[1][1],
            };
            assert!(strengths.luma_bytes_suffice());
            let blocks = [CdefSegmentBlock {
                luma: true,
                chroma: true,
            }; CDEF_SEGMENT_BLOCKS];
            let mut outputs = [([0u16; 40 * 8], [0u16; 20 * 4], [0u16; 20 * 4]); 2];
            for (bytes_only, (out, u, v)) in [false, true].into_iter().zip(&mut outputs) {
                let mut segment = CdefLumaSegment::default();
                if bytes_only {
                    assert!(segment.gather_bytes(&luma, width, stride, (4, 3)));
                    assert!(segment.block(0).is_none());
                } else {
                    assert!(segment.gather(&luma, width, stride, (4, 3)));
                }
                assert!(cdef_filter_segment(
                    &segment,
                    &strengths,
                    blocks,
                    [true; 3],
                    ((out, 40), Some((&pair, [(u, 20), (v, 20)]))),
                ));
            }
            assert_eq!(outputs[0], outputs[1], "{strengths:?}");
        }
        let deep = CdefStrengths {
            coeff_shift: 2,
            ..CdefStrengths {
                y_pri: 1,
                y_sec: 1,
                uv_pri: 0,
                uv_sec: 0,
                damping: 3,
                coeff_shift: 0,
                uv_dir: CDEF_UV_DIR[1][1],
            }
        };
        assert!(!deep.luma_bytes_suffice());
    }

    #[test]
    fn blocks_past_the_segment_are_rejected() {
        let filter = CdefBlockFilter {
            pri_str: 4,
            sec_str: 2,
            damping: 6,
            dir: 1,
            coeff_shift: 2,
        };
        let (luma, pair) = (CdefLumaSegment::default(), CdefPairSegment::default());
        let (mut out, mut pair_out) = ([0u16; 64], [0u16; CDEF_PAIR_OUTPUT]);
        assert!(cdef_filter_block_segment(
            &luma,
            CDEF_SEGMENT_BLOCKS - 1,
            &filter,
            true,
            &mut out,
            8
        ));
        for block in [CDEF_SEGMENT_BLOCKS, 1 << 61, usize::MAX] {
            assert!(luma.block(block).is_none() && luma.range(block).is_none());
            assert!(pair.block(block).is_none() && pair.range(block).is_none());
            assert!(!cdef_filter_block_segment(
                &luma, block, &filter, true, &mut out, 8
            ));
            assert!(!cdef_filter_block_chroma_pair_segment(
                &pair,
                block,
                &filter,
                &mut pair_out
            ));
        }
        for (len, stride) in [(64, 7), (63, 8), (64, usize::MAX)] {
            let mut short = vec![u16::MAX; len];
            assert!(!cdef_filter_block_segment(
                &luma, 0, &filter, true, &mut short, stride
            ));
            assert!(short.iter().all(|&sample| sample == u16::MAX));
        }
    }
}
