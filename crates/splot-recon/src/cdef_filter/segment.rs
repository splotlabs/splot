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
use fearless_simd::{i8x32, u8x32};

const TAP_REACH: usize = 2;
const LUMA_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_SEGMENT_BLOCK_AREA;
const PAIR_SEGMENT_AREA: usize = 8 * (CDEF_SEGMENT_BLOCKS - 1) + CDEF_PAIR_SEGMENT_BLOCK_AREA;
const PAIR_ROWS: usize = 4 + 2 * TAP_REACH;
const PAIR_SPAN: usize = CDEF_PAIR_SEGMENT_STRIDE / 2;

/// The tap rows of one luma row segment of [`CDEF_SEGMENT_BLOCKS`] 8x8
/// blocks at [`CDEF_SEGMENT_STRIDE`] lanes per row, and the least and
/// greatest sample of each block's 12x12 window.
pub struct CdefLumaSegment {
    pad: [u16; LUMA_SEGMENT_AREA],
    ranges: [[u16; 2]; CDEF_SEGMENT_BLOCKS],
}

impl Default for CdefLumaSegment {
    fn default() -> Self {
        Self {
            pad: [0; LUMA_SEGMENT_AREA],
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
        let ranges = gather_luma(samples, width, stride, &mut self.pad, origin);
        self.ranges = ranges.unwrap_or([[0, u16::MAX]; CDEF_SEGMENT_BLOCKS]);
        ranges.is_some()
    }

    /// The scratch from the first tap-reach lane of block `block` on, or
    /// `None` when `block` is not below [`CDEF_SEGMENT_BLOCKS`].
    #[inline]
    pub fn block(&self, block: usize) -> Option<&[u16; CDEF_SEGMENT_BLOCK_AREA]> {
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
    ranges: [u16; CDEF_SEGMENT_BLOCKS],
}

impl Default for CdefPairSegment {
    fn default() -> Self {
        Self {
            pad: [0; PAIR_SEGMENT_AREA],
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
        let ranges = gather_pair(u, v, width, stride, &mut self.pad, origin);
        self.ranges = ranges.unwrap_or([u16::MAX; CDEF_SEGMENT_BLOCKS]);
        ranges.is_some()
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

/// The least and greatest lanes of `min` and `max` over lanes `0..8` and
/// lanes `0..4` of the next eighth `next_min`/`next_max`.
#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn window_bounds<S: SimdToken>(
    simd: S,
    (min, max): (u16x8<S>, u16x8<S>),
    (next_min, next_max): (u16x8<S>, u16x8<S>),
) -> [u16; 2] {
    let high = u16x8::from_slice(simd, &[0, 0, 0, 0, u16::MAX, u16::MAX, u16::MAX, u16::MAX]);
    [
        min.min(next_min | high).reduce_min(),
        max.max(next_max & !high).reduce_max(),
    ]
}

/// Copies the luma segment's tap rows into `pad` and returns the least and
/// greatest sample of each block's 12x12 window.
fn gather_luma<T: crate::ReconSample>(
    samples: &[T],
    width: usize,
    stride: usize,
    pad: &mut [u16; LUMA_SEGMENT_AREA],
    origin: (usize, usize),
) -> Option<[[u16; 2]; CDEF_SEGMENT_BLOCKS]> {
    baseline_simd!(simd => gather_luma_with(simd, samples, width, stride, pad, origin))
}

#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn gather_luma_with<S: SimdToken, T: crate::ReconSample>(
    simd: S,
    samples: &[T],
    width: usize,
    stride: usize,
    pad: &mut [u16; LUMA_SEGMENT_AREA],
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
            let (first, second) = simd.widen_u8x32(u8x32::from_slice(simd, &src[..32]));
            let (_, third) = simd.widen_u8x32(u8x32::from_slice(simd, &src[4..36]));
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
    let eighths = |[a, b, c]: [u16x16<S>; 3]| {
        let (a, b, c) = (
            simd.split_u16x16(a),
            simd.split_u16x16(b),
            simd.split_u16x16(c),
        );
        [a.0, a.1, b.0, b.1, c.1]
    };
    let (min, max) = (eighths(wide_min), eighths(wide_max));
    Some(core::array::from_fn(|block| {
        let next = if block + 1 < CDEF_SEGMENT_BLOCKS {
            (min[block + 1], max[block + 1])
        } else {
            let (min, max) = (min[4], max[4]);
            (
                simd.slide_u16x8::<4>(min, min),
                simd.slide_u16x8::<4>(max, max),
            )
        };
        window_bounds(simd, (min[block], max[block]), next)
    }))
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
    pad: &mut [u16; PAIR_SEGMENT_AREA],
    origin: (usize, usize),
) -> Option<[u16; CDEF_SEGMENT_BLOCKS]> {
    baseline_simd!(simd => gather_pair_with(
        simd, u_samples, v_samples, width, stride, pad, origin,
    ))
}

#[allow(clippy::inline_always, reason = "measured CDEF gather hot path")]
#[inline(always)]
fn gather_pair_with<S: SimdToken, T: crate::ReconSample>(
    simd: S,
    u_samples: &[T],
    v_samples: &[T],
    width: usize,
    stride: usize,
    pad: &mut [u16; PAIR_SEGMENT_AREA],
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
    let ranges = [0, 1].map(|plane| {
        let block = |chunk: usize, half: usize| {
            let (min, max) = (
                simd.split_u16x16(min[plane][chunk]),
                simd.split_u16x16(max[plane][chunk]),
            );
            let (min, max) = if half == 0 {
                (min.0, max.0)
            } else {
                (min.1, max.1)
            };
            max.reduce_max() - min.reduce_min()
        };
        [block(0, 0), block(1, 0), block(0, 1), block(1, 1)]
    });
    Some(core::array::from_fn(|block| {
        ranges[0][block].max(ranges[1][block])
    }))
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
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return pair_segment_with(avx2, InLane, pad, range, filter, out);
    }
    dispatch!(Level::baseline(), simd => pair_segment_with(simd, Ordered, pad, range, filter, out))
}

#[allow(clippy::inline_always, reason = "measured CDEF hot path")]
#[inline(always)]
fn pair_segment_with<S: SimdToken, Q: QuadLanes<S>>(
    simd: S,
    quads: Q,
    pad: &[u16; CDEF_PAIR_SEGMENT_BLOCK_AREA],
    range: u16,
    filter: &CdefBlockFilter,
    out: &mut [u16; CDEF_PAIR_OUTPUT],
) -> bool {
    const SPAN: usize = 3 * CDEF_PAIR_SEGMENT_STRIDE + 8;
    const AREA: usize = CDEF_PAIR_SEGMENT_BLOCK_AREA;
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
        return cdef_by_families!(
            filter,
            cdef_filter_rows_wide8::<S, Q, STRIDE, 4, 4, SPAN, AREA>(simd, quads, args),
            copy!()
        )
        .is_some();
    }
    cdef_filter_block_rows::<S, 8, false, STRIDE, 4, 4, SPAN, AREA>(simd, args).is_some()
}

/// Whether [`cdef_filter_rows_wide8`] computes `filter` exactly: besides
/// [`cdef_i8_diffs_fit`], the primary sum `2 * pair0 + pair1`, at most
/// `6 * pri_str`, and the secondary sum `2 * family0 + family1`, at most
/// `12 * sec_str`, fit `i8`.
fn cdef_wide8_fits(filter: &CdefBlockFilter) -> bool {
    cdef_i8_diffs_fit(filter) && filter.pri_str <= 21 && filter.sec_str <= 10
}

/// How [`cdef_filter_rows_wide8`] packs four 8-lane rows, two `i16` row
/// pairs, into one `i8` vector and back.
trait QuadLanes<S: SimdToken>: Copy {
    /// The row pairs saturated to `i8`.
    fn narrow(self, simd: S, pairs: [i16x16<S>; 2]) -> i8x32<S>;
    /// The inverse of [`QuadLanes::narrow`] for lanes that fit `i8`.
    fn widen(self, simd: S, lanes: i8x32<S>) -> [i16x16<S>; 2];
}

/// Rows in order: the first pair fills lanes `0..16`.
#[derive(Clone, Copy)]
struct Ordered;

impl<S: SimdToken> QuadLanes<S> for Ordered {
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
impl QuadLanes<Avx2> for InLane {
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
    fn apply_diff(self, diff: i8x32<S>) -> i8x32<S> {
        let magnitude = diff.abs().bitcast::<u8x32<S>>();
        let clip = self
            .threshold
            .saturating_sub(magnitude >> self.shift)
            .bitcast::<i8x32<S>>();
        diff.min(clip).max(-clip)
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
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    quads: Q,
    (pad, _, filter, starts, out, out_stride): CdefRowsArgs<'_, AREA>,
) -> Option<()> {
    let (center_view, pri_views, sec_views) =
        cdef_tap_views::<STRIDE, CENTER, SPAN, AREA, PRI, SEC>(pad, filter.dir, starts)?;
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
        let center = quad(center_view)?;
        let (mut min, mut max) = (i8x32::splat(simd, 0), i8x32::splat(simd, 0));
        macro_rules! tap {
            ($view:expr, $constrain:expr) => {{
                let [low, high] = quad($view)?;
                let diff = quads.narrow(simd, [low - center[0], high - center[1]]);
                if PRI && SEC {
                    min = min.min(diff);
                    max = max.max(diff);
                }
                $constrain.apply_diff(diff)
            }};
        }
        let mut sum = [i16x16::splat(simd, 0); 2];
        if PRI {
            let pair0 = tap!(pri_views[0], pri) + tap!(pri_views[1], pri);
            let pair1 = tap!(pri_views[2], pri) + tap!(pri_views[3], pri);
            let primary = quads.widen(simd, pair0 + pair1 + (pair0 & pri_double));
            sum = primary.map(|primary| primary * pri_weight);
        }
        if SEC {
            let family0 = tap!(sec_views[0], sec)
                + tap!(sec_views[1], sec)
                + tap!(sec_views[2], sec)
                + tap!(sec_views[3], sec);
            let family1 = tap!(sec_views[4], sec)
                + tap!(sec_views[5], sec)
                + tap!(sec_views[6], sec)
                + tap!(sec_views[7], sec);
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
    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = Level::baseline().as_avx2() {
        return luma_segment_with(avx2, InLane, pad, range, filter, (out, out_stride));
    }
    dispatch!(Level::baseline(), simd => luma_segment_with(
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
    pad: &[u16; CDEF_SEGMENT_BLOCK_AREA],
    range: Option<[u16; 2]>,
    filter: &CdefBlockFilter,
    (out, out_stride): (&mut [u16], usize),
) -> bool {
    const SPAN: usize = 7 * CDEF_SEGMENT_STRIDE + 8;
    const AREA: usize = CDEF_SEGMENT_BLOCK_AREA;
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
        return cdef_by_families!(
            filter,
            cdef_filter_rows_wide8::<S, Q, STRIDE, 2, 8, SPAN, AREA>(simd, quads, args),
            copy!()
        )
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
                        luma,
                        range,
                        &filter,
                        (&mut ordered, 8)
                    ));
                    assert!(luma_segment_with(
                        avx2,
                        InLane,
                        luma,
                        range,
                        &filter,
                        (&mut packed, 8)
                    ));
                    assert_eq!(ordered, packed, "luma block {block}, {filter:?}");
                    let (mut ordered, mut packed) = ([0; CDEF_PAIR_OUTPUT], [0; CDEF_PAIR_OUTPUT]);
                    assert!(pair_segment_with(
                        avx2,
                        Ordered,
                        chroma,
                        1000,
                        &filter,
                        &mut ordered
                    ));
                    assert!(pair_segment_with(
                        avx2,
                        InLane,
                        chroma,
                        1000,
                        &filter,
                        &mut packed
                    ));
                    assert_eq!(ordered, packed, "chroma block {block}, {filter:?}");
                }
            }
        }
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
