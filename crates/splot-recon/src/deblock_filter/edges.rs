// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Batched AV2 § 7.17.7 deblocking of runs of four-line edge segments
//! (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-7`).
//!
//! A chunk of four segments holds one sample line per vector lane, so the
//! § 7.17.7.2 width choice of every segment becomes lane masks and the
//! § 7.17.7.1 ramp applies to all sixteen lines at once: no per-edge branch.

use core::num::NonZeroUsize;

use fearless_simd::{Bytes, Level, Simd, SimdBase, SimdInto, dispatch, i16x16, u32x8, u64x4};
use splot_core::tables::conversion::{Q_FIRST, Q_THRESH_MULTS, W_MULT};

use crate::intra_dc_math::validate_sample_type;
use crate::{BitDepth, ReconError, ReconSample, Result};

/// Lines of one edge segment (`MI_SIZE`).
const LINES: usize = 4;
/// Segments one chunk filters.
const SEGMENTS: usize = 4;
/// Lines of one chunk, one per lane.
const LANES: usize = LINES * SEGMENTS;
/// Samples the widest filter reads on either side of an edge.
const REACH: usize = 8;
/// Sample rows across the edge a chunk holds.
const ROWS: usize = 2 * REACH;

/// The filter widths § 7.17.7.2 can choose, by compressed index.
const WIDTHS: [usize; 7] = [0, 1, 2, 3, 4, 6, 8];

/// `W_Mult[w - 1] * (w - j) * 64` by row `j` from the edge and compressed
/// width index: the per-sample ramp weight, scaled so that a high-half
/// multiply leaves `deltaM2 * weight >> 10`.
const RAMP: [[i16; 8]; REACH] = {
    let mut table = [[0; 8]; REACH];
    let mut j = 0;
    while j < REACH {
        let mut index = 1;
        while index < WIDTHS.len() {
            let width = WIDTHS[index];
            if j < width {
                table[j][index] = (W_MULT[width - 1] * (width - j) as i32 * 64) as i16;
            }
            index += 1;
        }
        j += 1;
    }
    table
};

/// `Q_Thresh_Mults[w - 1]` by compressed width index.
const CLAMP: [i16; 8] = {
    let mut table = [0; 8];
    let mut index = 1;
    while index < WIDTHS.len() {
        table[index] = Q_THRESH_MULTS[WIDTHS[index] - 1] as i16;
        index += 1;
    }
    table
};

/// Largest `qThr` an edge may carry: `qThr * Q_First[0]` stays in `i16`.
const MAX_Q_THR: i32 = 700;
/// Largest `sideThr` an edge may carry: `sideThr * 6` stays in `i16`.
const MAX_SIDE: i32 = 4095;

/// One four-line edge segment's § 7.17.7 inputs, packed for the batched
/// kernels: `qThr`, `sideThr`, `maxWidthPos`, `maxWidthNeg` and the two
/// lossless flags. [`DeblockEdge::NONE`] filters nothing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeblockEdge(u64);

impl DeblockEdge {
    /// The edge § 7.17.2 does not filter.
    pub const NONE: Self = Self(0);

    /// Packs one edge's inputs, or `None` when they are outside what § 7.17
    /// derives: `maxWidthPos` in `{1, 3, 4, 6, 8}`, `maxWidthNeg` in
    /// `{1, 2, 3, 4, 6, 8}` and at most `maxWidthPos`, and non-negative
    /// thresholds within the kernels' 16-bit range. A zero threshold is
    /// [`DeblockEdge::NONE`], as § 7.17.7.2 then chooses width 0.
    pub const fn new(
        q_thr: i32,
        side: i32,
        max_width_neg: usize,
        max_width_pos: usize,
        prev_lossless: bool,
        curr_lossless: bool,
    ) -> Option<Self> {
        let pos_ok = matches!(max_width_pos, 1 | 3 | 4 | 6 | 8);
        let neg_ok = matches!(max_width_neg, 1 | 2 | 3 | 4 | 6 | 8);
        if !pos_ok || !neg_ok || max_width_neg > max_width_pos {
            return None;
        }
        if q_thr < 0 || side < 0 || q_thr > MAX_Q_THR || side > MAX_SIDE {
            return None;
        }
        if q_thr == 0 || side == 0 {
            return Some(Self::NONE);
        }
        let flags = prev_lossless as u64 | (curr_lossless as u64) << 1;
        Some(Self(
            q_thr as u64
                | (side as u64) << 16
                | (max_width_pos as u64) << 32
                | (max_width_neg as u64) << 40
                | flags << 48,
        ))
    }

    /// This edge with `qThr` and `sideThr` replaced, each capped at the
    /// largest value [`DeblockEdge::new`] accepts. A zero threshold makes the
    /// edge filter nothing, as § 7.17.7.2 then chooses width 0.
    #[must_use]
    pub const fn with_thresholds(self, q_thr: u16, side: u16) -> Self {
        let q_thr = if q_thr as i32 > MAX_Q_THR {
            MAX_Q_THR as u16
        } else {
            q_thr
        };
        let side = if side as i32 > MAX_SIDE {
            MAX_SIDE as u16
        } else {
            side
        };
        Self(self.0 & !0xffff_ffff | q_thr as u64 | (side as u64) << 16)
    }

    /// This edge with its two lossless flags replaced.
    #[must_use]
    pub const fn with_lossless(self, prev_lossless: bool, curr_lossless: bool) -> Self {
        let flags = prev_lossless as u64 | (curr_lossless as u64) << 1;
        Self(self.0 & !(3 << 48) | flags << 48)
    }

    /// The eight edges whose words are `low` (`qThr | sideThr << 16`) and
    /// `high` (`maxWidthPos | maxWidthNeg << 8 | prev_lossless << 16 |
    /// curr_lossless << 17`) lane by lane, each field clamped to the range
    /// [`DeblockEdge::new`] accepts; an all-zero lane is [`DeblockEdge::NONE`].
    #[inline]
    pub fn from_lanes<S: Simd>(simd: S, low: u32x8<S>, high: u32x8<S>) -> [Self; 8] {
        let q_thr = (low & 0xffff).min(u32x8::splat(simd, MAX_Q_THR as u32));
        let side = (low >> 16).min(u32x8::splat(simd, MAX_SIDE as u32));
        let pos = (high & 0xff).min(u32x8::splat(simd, REACH as u32));
        let neg = ((high >> 8) & 0xff).min(pos);
        let low = q_thr | side << 16;
        let high = pos | neg << 8 | (high & 0x3_0000);
        let (first, second) = simd.interleave_u32x8(low, high);
        let first = first.bitcast::<u64x4<S>>().to_array();
        let second = second.bitcast::<u64x4<S>>().to_array();
        core::array::from_fn(|lane| {
            Self(if lane < 4 {
                first[lane]
            } else {
                second[lane - 4]
            })
        })
    }

    /// Whether the edge filters nothing.
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// The § 7.17.5 `qThr`.
    pub const fn q_thr(self) -> i16 {
        self.0 as u16 as i16
    }

    /// The § 7.17.5 `sideThr`.
    pub const fn side(self) -> i16 {
        (self.0 >> 16) as u16 as i16
    }

    /// The § 7.17.3 `maxWidthPos` (0 for [`DeblockEdge::NONE`]).
    pub const fn max_width_pos(self) -> usize {
        (self.0 >> 32) as u8 as usize
    }

    /// The § 7.17.3 `maxWidthNeg` (0 for [`DeblockEdge::NONE`]).
    pub const fn max_width_neg(self) -> usize {
        (self.0 >> 40) as u8 as usize
    }

    /// Whether the previous-side samples are lossless.
    pub const fn prev_lossless(self) -> bool {
        self.0 >> 48 & 1 != 0
    }

    /// Whether the current-side samples are lossless.
    pub const fn curr_lossless(self) -> bool {
        self.0 >> 49 & 1 != 0
    }
}

/// The rows either side of the edge a chunk must hold: the § 7.17.7.2
/// cascade of its widest edge reads `maxWidthPos + 1` samples past it, at
/// least 3, and the `dist2 = 7` stage reads 8.
#[inline]
fn chunk_reach(edges: &[DeblockEdge; SEGMENTS]) -> usize {
    let widest = edges
        .iter()
        .fold(0, |widest, edge| widest.max(edge.max_width_pos()));
    match widest {
        0 | 1 => 3,
        2..=4 => 5,
        _ => REACH,
    }
}

/// The rows either side of the edge a chunk of `reach` can change: every
/// width of an edge is at most its `maxWidthPos`.
const fn chunk_writes(reach: usize) -> usize {
    match reach {
        3 => 1,
        5 => 4,
        _ => REACH,
    }
}

/// Filters the vertical edges (§ 7.17.1 pass 0) of `edges.len()` segments
/// stacked down one column: segment `k` is the four sample rows from
/// `first + 4 * k * stride`, with `first` the index of its first row's `q0`.
/// Each row is read and written within 8 samples either side of the edge.
///
/// # Errors
/// [`ReconError::SampleTypeUnsupportedBitDepth`] when `T` cannot hold
/// `bit_depth`, [`ReconError::StrideTooSmall`] when a row is narrower than
/// the 16 samples one edge spans, and [`ReconError::DeblockFilterLineTooShort`]
/// when a row's 16 samples leave its row or `samples`.
pub fn deblock_vertical_edges<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge],
    bit_depth: BitDepth,
) -> Result<()> {
    deblock_vertical_edge_column(samples, stride, first, edges, NonZeroUsize::MIN, bit_depth)
}

/// [`deblock_vertical_edges`] over the segments `edges[k * edge_stride]`,
/// so that a column can be read out of a row-major edge grid.
///
/// # Errors
/// As [`deblock_vertical_edges`].
pub fn deblock_vertical_edge_column<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge],
    edge_stride: NonZeroUsize,
    bit_depth: BitDepth,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    if stride < ROWS {
        return Err(ReconError::StrideTooSmall {
            stride_samples: stride,
            storage_width: ROWS,
        });
    }
    let column = first % stride;
    let end = (segments(edges, edge_stride.get()) * LINES)
        .checked_sub(1)
        .and_then(|last| last.checked_mul(stride))
        .and_then(|offset| first.checked_add(offset + REACH));
    let fits =
        column >= REACH && column + REACH <= stride && end.is_none_or(|end| end <= samples.len());
    if !fits {
        return Err(too_short(first, samples.len()));
    }
    let max_sample = bit_depth.max_sample() as i16;
    let run_edges = (edges, edge_stride.get());
    if let Some(samples) = T::u8_slice_mut(samples) {
        run_u8::<true>(samples, stride, first, run_edges, max_sample);
    } else if let Some(samples) = T::u16_slice_mut(samples) {
        run::<u16, true>(samples, stride, first, run_edges, max_sample);
    } else {
        return Err(too_short(first, samples.len()));
    }
    Ok(())
}

/// How many segments `edges[k * edge_stride]` there are.
const fn segments(edges: &[DeblockEdge], edge_stride: usize) -> usize {
    match edges.len() {
        0 => 0,
        len => (len - 1) / edge_stride + 1,
    }
}

/// Filters the horizontal edges (§ 7.17.1 pass 1) of `edges.len()` segments
/// side by side along one row: segment `k` is the four sample columns from
/// `first + 4 * k`, with `first` the index of its first column's `q0`. Each
/// column is read and written within 8 rows either side of the edge.
///
/// # Errors
/// As [`deblock_vertical_edges`], with [`ReconError::StrideTooSmall`] when a
/// row is narrower than the run's columns.
pub fn deblock_horizontal_edges<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge],
    bit_depth: BitDepth,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    let width = edges.len() * LINES;
    if stride < width {
        return Err(ReconError::StrideTooSmall {
            stride_samples: stride,
            storage_width: width,
        });
    }
    let fits = first % stride + width <= stride
        && first >= REACH * stride
        && first + (REACH - 1) * stride + width <= samples.len();
    if !fits {
        return Err(too_short(first, samples.len()));
    }
    let max_sample = bit_depth.max_sample() as i16;
    if let Some(samples) = T::u8_slice_mut(samples) {
        run_u8::<false>(samples, stride, first, (edges, 1), max_sample);
    } else if let Some(samples) = T::u16_slice_mut(samples) {
        run::<u16, false>(samples, stride, first, (edges, 1), max_sample);
    } else {
        return Err(too_short(first, samples.len()));
    }
    Ok(())
}

#[cold]
fn too_short(boundary: usize, len: usize) -> ReconError {
    ReconError::DeblockFilterLineTooShort {
        boundary,
        max_width_neg: REACH,
        width: REACH,
        len,
    }
}

/// Sample storage the chunk kernels filter in place.
trait EdgeSample: Copy + Default {
    fn to_i16(self) -> i16;
    fn from_i16(value: i16) -> Self;
}

impl EdgeSample for u8 {
    fn to_i16(self) -> i16 {
        i16::from(self)
    }

    fn from_i16(value: i16) -> Self {
        value as Self
    }
}

impl EdgeSample for u16 {
    fn to_i16(self) -> i16 {
        self as i16
    }

    fn from_i16(value: i16) -> Self {
        value as Self
    }
}

/// [`run`] over 8-bit samples, through the eight-segment kernels where the
/// level has them.
fn run_u8<const VERTICAL: bool>(
    samples: &mut [u8],
    stride: usize,
    first: usize,
    edges: (&[DeblockEdge], usize),
    max_sample: i16,
) {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = Level::baseline().as_avx2() {
        return avx2_u8::run::<VERTICAL>(token, samples, stride, first, edges);
    }
    run::<u8, VERTICAL>(samples, stride, first, edges, max_sample);
}

/// Runs the validated segments `edges.0[k * edges.1]` in chunks of
/// [`SEGMENTS`]. A last partial chunk filters a copy of its window, so no
/// load or store leaves the run.
fn run<E: Chunk, const VERTICAL: bool>(
    samples: &mut [E],
    stride: usize,
    first: usize,
    (edges, edge_stride): (&[DeblockEdge], usize),
    max_sample: i16,
) {
    let step = if VERTICAL { LANES * stride } else { LANES };
    let count = segments(edges, edge_stride);
    let segment = |k: usize| edges.get(k * edge_stride).copied().unwrap_or_default();
    let chunks = count / SEGMENTS;
    for index in 0..chunks {
        let chunk: [DeblockEdge; SEGMENTS] =
            core::array::from_fn(|lane| segment(index * SEGMENTS + lane));
        if chunk.iter().all(|edge| edge.is_none()) {
            continue;
        }
        E::chunk::<VERTICAL>(samples, stride, first + index * step, &chunk, max_sample);
    }
    let tail = count % SEGMENTS;
    let padded: [DeblockEdge; SEGMENTS] = core::array::from_fn(|lane| {
        if lane < tail {
            segment(chunks * SEGMENTS + lane)
        } else {
            DeblockEdge::NONE
        }
    });
    if padded.iter().all(|edge| edge.is_none()) {
        return;
    }
    let lines = tail * LINES;
    let origin = first + chunks * step;
    let mut window = [E::default(); ROWS * LANES];
    for (line, copy) in window.chunks_exact_mut(ROWS).enumerate() {
        let (start, len) = window_line::<VERTICAL>(origin, stride, line, lines);
        copy[..len].copy_from_slice(&samples[start..start + len]); // splot-copy-ok: stage a partial chunk's window
    }
    let (window_first, window_stride) = if VERTICAL {
        (REACH, ROWS)
    } else {
        (REACH * ROWS, ROWS)
    };
    E::chunk::<VERTICAL>(
        &mut window,
        window_stride,
        window_first,
        &padded,
        max_sample,
    );
    for (line, copy) in window.chunks_exact(ROWS).enumerate() {
        let (start, len) = window_line::<VERTICAL>(origin, stride, line, lines);
        samples[start..start + len].copy_from_slice(&copy[..len]); // splot-copy-ok: publish a partial chunk's window
    }
}

/// Where window row `line` of a partial chunk at `origin` copies from: a
/// whole 16-sample line across a vertical edge, or the run's `lines` columns
/// of one row across a horizontal one. Rows past the run copy nothing.
const fn window_line<const VERTICAL: bool>(
    origin: usize,
    stride: usize,
    line: usize,
    lines: usize,
) -> (usize, usize) {
    if VERTICAL {
        if line < lines {
            (origin - REACH + line * stride, ROWS)
        } else {
            (0, 0)
        }
    } else {
        (origin - REACH * stride + line * stride, lines)
    }
}

/// One chunk's filter for a sample storage type.
trait Chunk: EdgeSample {
    /// Filters the [`SEGMENTS`] edges at `first`, which need not lie in a
    /// run: the window is `first ± 8` across the edge and 16 lines along it.
    fn chunk<const VERTICAL: bool>(
        samples: &mut [Self],
        stride: usize,
        first: usize,
        edges: &[DeblockEdge; SEGMENTS],
        max_sample: i16,
    );
}

impl Chunk for u8 {
    fn chunk<const VERTICAL: bool>(
        samples: &mut [Self],
        stride: usize,
        first: usize,
        edges: &[DeblockEdge; SEGMENTS],
        max_sample: i16,
    ) {
        dispatch!(Level::baseline(), simd => portable_chunk::<_, Self, VERTICAL>(
            simd, samples, stride, first, edges, max_sample,
        ));
    }
}

impl Chunk for u16 {
    fn chunk<const VERTICAL: bool>(
        samples: &mut [Self],
        stride: usize,
        first: usize,
        edges: &[DeblockEdge; SEGMENTS],
        max_sample: i16,
    ) {
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = Level::baseline().as_avx2() {
            let packed = edges.map(|edge| edge.0 as i64);
            let reach = chunk_reach(edges);
            if VERTICAL {
                avx2::vertical_u16(token, samples, stride, first, packed, reach, max_sample);
            } else {
                avx2::horizontal_u16(token, samples, stride, first, packed, reach, max_sample);
            }
            return;
        }
        dispatch!(Level::baseline(), simd => portable_chunk::<_, Self, VERTICAL>(
            simd, samples, stride, first, edges, max_sample,
        ));
    }
}

/// The portable chunk: gathers each row across the edge into a lane vector,
/// filters, and scatters the rows back.
#[inline]
fn portable_chunk<S: Simd, E: EdgeSample, const VERTICAL: bool>(
    simd: S,
    samples: &mut [E],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge; SEGMENTS],
    max_sample: i16,
) {
    let index = |row: usize, lane: usize| {
        if VERTICAL {
            first - REACH + row + lane * stride
        } else {
            first - REACH * stride + row * stride + lane
        }
    };
    let mut rows = [i16x16::splat(simd, 0); ROWS];
    for (row, vector) in rows.iter_mut().enumerate() {
        let lanes: [i16; LANES] = core::array::from_fn(|lane| samples[index(row, lane)].to_i16());
        *vector = lanes.simd_into(simd);
    }
    portable::filter(simd, &mut rows, edges, max_sample);
    for (row, vector) in rows.iter().enumerate() {
        for (lane, value) in vector.to_array().into_iter().enumerate() {
            samples[index(row, lane)] = E::from_i16(value);
        }
    }
}

mod portable {
    use fearless_simd::{Bytes, Simd, SimdBase, i16x16, i32x8, u8x32};

    use super::{CLAMP, DeblockEdge, LANES, LINES, Q_FIRST, RAMP, REACH, ROWS, SEGMENTS};

    /// Byte indices that copy lane `4g` (`FIRST`) or `4g + 3` (`LAST`) of
    /// every four-lane segment `g` over the whole segment.
    const FIRST: [u8; 32] = segment_bytes(0);
    const LAST: [u8; 32] = segment_bytes(3);

    const fn segment_bytes(line: usize) -> [u8; 32] {
        let mut bytes = [0; 32];
        let mut byte = 0;
        while byte < 32 {
            let lane = byte % 16 / 2;
            bytes[byte] = ((lane / LINES * LINES + line) * 2 + byte % 2) as u8;
            byte += 1;
        }
        bytes
    }

    /// The § 7.17.7.2 `(derivS + derivT + 1) >> 1` of each segment's first
    /// and last line, over the segment's lanes.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    fn segment_mean<S: Simd>(simd: S, x: i16x16<S>) -> i16x16<S> {
        let bytes = x.bitcast::<u8x32<S>>();
        let pick = |pattern: &[u8; 32]| {
            simd.swizzle_dyn_within_blocks_u8x32(bytes, u8x32::from_slice(simd, pattern))
                .bitcast::<i16x16<S>>()
        };
        (pick(&FIRST) + pick(&LAST) + 1) >> 1
    }

    /// `Round2(delta * weight / 64, 11)` for the 64-scaled [`RAMP`] weights.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    fn ramp<S: Simd>(simd: S, delta: i16x16<S>, weight: i16x16<S>) -> i16x16<S> {
        let (low, high) = simd.widening_mul_i16x16(delta, weight);
        let round = |x: i32x8<S>| ((x >> 16) + 1) >> 1;
        simd.narrow_i32x8(round(low), round(high))
    }

    /// One [`LANES`]-lane vector of `field` over each segment's lines.
    fn lanes<S: Simd>(
        simd: S,
        edges: &[DeblockEdge; SEGMENTS],
        field: impl Fn(DeblockEdge) -> i16,
    ) -> i16x16<S> {
        let lanes: [i16; LANES] = core::array::from_fn(|lane| field(edges[lane / LINES]));
        i16x16::from_slice(simd, &lanes)
    }

    /// Filters the chunk whose sample rows across the edge are `rows`: row
    /// `REACH + j` is `s[j]` of every line.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    pub(super) fn filter<S: Simd>(
        simd: S,
        rows: &mut [i16x16<S>; ROWS],
        edges: &[DeblockEdge; SEGMENTS],
        max_sample: i16,
    ) {
        let reach = super::chunk_reach(edges);
        let q = lanes(simd, edges, DeblockEdge::q_thr);
        let side = lanes(simd, edges, DeblockEdge::side);
        let max_pos = lanes(simd, edges, |edge| edge.max_width_pos() as i16);
        let max_neg = lanes(simd, edges, |edge| edge.max_width_neg() as i16);
        let keep_prev = lanes(simd, edges, |edge| i16::from(!edge.prev_lossless()));
        let keep_curr = lanes(simd, edges, |edge| i16::from(!edge.curr_lossless()));
        let s = |j: isize| rows[(REACH as isize + j) as usize];
        let second = |j: isize| segment_mean(simd, (s(j - 1) + s(j + 1) - s(j) - s(j)).abs());
        let directional = |edge: isize, inner: isize, outer: isize, n: i16| {
            let edge = s(edge);
            segment_mean(simd, (edge - s(outer) - (edge - s(inner)) * n).abs())
        };
        let zero = i16x16::splat(simd, 0);
        let all = i16x16::splat(simd, -1);
        let mask = |condition| simd.select_i16x16(condition, all, zero);
        let le = |a: i16x16<S>, b: i16x16<S>| mask(a.simd_le(b));
        let high = second(-2).max(second(1));
        let transition = second(-1) + second(0);
        let one = le(high, side) & mask(q.simd_gt(zero)) & mask(side.simd_gt(zero));
        let two = one
            & mask(max_pos.simd_gt(i16x16::splat(simd, 1)))
            & le(high, side >> 2)
            & le(transition, q * 4);
        let three = two & le(high, side >> 3) & le(transition, q * 3);
        let mut index = zero - one - two;
        let neg_ok = |at: i16, deriv: i16x16<S>, threshold: i16x16<S>| {
            mask(max_neg.simd_lt(i16x16::splat(simd, at))) | le(deriv, threshold)
        };
        if reach > 3 {
            let end = (side * 3) >> 4;
            let three = three
                & neg_ok(3, directional(-1, -2, -4, 3), end)
                & le(directional(0, 1, 3, 3), end);
            let wide = |previous: i16x16<S>, dist: i16, q_first: i32, end: i16x16<S>| {
                let dist2 = dist.min(7) as isize;
                previous
                    & mask(max_pos.simd_ge(i16x16::splat(simd, dist)))
                    & le(transition, (q * q_first as i16) >> 4)
                    & neg_ok(
                        dist.min(7),
                        directional(-1, -2, -dist2 - 1, dist2 as i16),
                        end,
                    )
                    & le(directional(0, 1, dist2, dist2 as i16), end)
            };
            let four = wide(three, 4, Q_FIRST[0], side >> 2);
            index = index - three - four;
            if reach > 5 {
                let six = wide(four, 6, Q_FIRST[2], (side * 6) >> 4);
                let eight = wide(six, 8, Q_FIRST[4], side >> 1);
                index = index - six - eight;
            }
        }
        let neg_index = index.min(
            max_neg
                + mask(max_neg.simd_gt(i16x16::splat(simd, 4)))
                + mask(max_neg.simd_gt(i16x16::splat(simd, 6))),
        );
        let table = |values: &[i16; 8], index: i16x16<S>| {
            let lanes = index.to_array().map(|index| values[index as usize & 7]);
            i16x16::from_slice(simd, &lanes)
        };
        let q_clamp = q * table(&CLAMP, index);
        let delta = ((s(-2) - s(1) + (s(0) - s(-1)) * 3) << 2)
            .max(zero - q_clamp)
            .min(q_clamp);
        let high_sample = i16x16::splat(simd, max_sample);
        let writes = super::chunk_writes(reach);
        for j in 0..writes {
            let pos = table(&RAMP[j], index) * keep_curr;
            let neg = table(&RAMP[j], neg_index) * keep_prev;
            let curr = &mut rows[REACH + j];
            *curr = (*curr - ramp(simd, delta, pos)).max(zero).min(high_sample);
            let prev = &mut rows[REACH - 1 - j];
            *prev = (*prev + ramp(simd, delta, neg)).max(zero).min(high_sample);
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;

    use fearless_simd::{SimdBase, SimdFrom, kernel, u16x16};

    use super::{CLAMP, Q_FIRST, RAMP, REACH, ROWS};

    kernel!(
        /// The § 7.17.7.2 `(derivS + derivT + 1) >> 1` of each segment's
        /// first and last line, over the segment's lanes.
        #[inline(always)]
        fn segment_mean(token: Avx2, x: __m256i) -> __m256i {
            let first = _mm256_setr_epi8(
                0, 1, 0, 1, 0, 1, 0, 1, 8, 9, 8, 9, 8, 9, 8, 9, 0, 1, 0, 1, 0, 1, 0, 1, 8, 9, 8, 9,
                8, 9, 8, 9,
            );
            let last = _mm256_setr_epi8(
                6, 7, 6, 7, 6, 7, 6, 7, 14, 15, 14, 15, 14, 15, 14, 15, 6, 7, 6, 7, 6, 7, 6, 7, 14,
                15, 14, 15, 14, 15, 14, 15,
            );
            _mm256_avg_epu16(_mm256_shuffle_epi8(x, first), _mm256_shuffle_epi8(x, last))
        }
    );

    kernel!(
        /// One field of each packed edge over its segment's lanes: bytes
        /// `low` and `high` of the edge, `high < 0` reading zero.
        #[inline(always)]
        fn field(token: Avx2, packed: __m256i, low: i8, high: i8) -> __m256i {
            let (low2, high2) = (low + 8, if high < 0 { high } else { high + 8 });
            let pattern = _mm256_setr_epi8(
                low, high, low, high, low, high, low, high, low2, high2, low2, high2, low2, high2,
                low2, high2, low, high, low, high, low, high, low, high, low2, high2, low2, high2,
                low2, high2, low2, high2,
            );
            _mm256_shuffle_epi8(packed, pattern)
        }
    );

    kernel!(
        /// Looks `values` up in each lane by the byte pair `bytes` holds.
        #[inline(always)]
        fn table(token: Avx2, values: [i16; 8], bytes: __m256i) -> __m256i {
            let table = _mm256_setr_epi16(
                values[0], values[1], values[2], values[3], values[4], values[5], values[6],
                values[7], values[0], values[1], values[2], values[3], values[4], values[5],
                values[6], values[7],
            );
            _mm256_shuffle_epi8(table, bytes)
        }
    );

    kernel!(
        /// The [`table`] byte pairs of a compressed width index.
        #[inline(always)]
        fn table_bytes(token: Avx2, index: __m256i) -> __m256i {
            _mm256_add_epi16(
                _mm256_mullo_epi16(index, _mm256_set1_epi16(0x0202)),
                _mm256_set1_epi16(0x0100),
            )
        }
    );

    /// Filters the chunk whose sample rows across the edge are `$rows`: row
    /// `REACH + j` holds `s[j]` of every line. Rows past `$reach` either side
    /// are neither read nor written. Evaluates to `false`, having changed
    /// nothing, when no line can move: § 7.17.7.1 rounds a ramp step to zero
    /// below `|deltaM2 * W_Mult[w - 1] * w| = 1024`, which needs
    /// `|p1 - q1 + 3 * (q0 - p0)| > 3` at width 1 and `> 2` otherwise. A macro rather than a kernel so that the
    /// rows stay in registers in each kernel that loads them.
    macro_rules! filter_rows {
        ($token:ident, $rows:ident, $packed:expr, $reach:expr) => {{
            let token = $token;
            let reach: usize = $reach;
            let packed: [i64; 4] = $packed;
            let packed = _mm256_setr_epi64x(packed[0], packed[1], packed[2], packed[3]);
            let q = field(token, packed, 0, 1);
            let side = field(token, packed, 2, 3);
            let max_pos = field(token, packed, 4, -128);
            let max_neg = field(token, packed, 5, -128);
            let flags = field(token, packed, 6, -128);
            let zero = _mm256_setzero_si256();
            let ones = _mm256_set1_epi16(-1);
            let one_v = _mm256_set1_epi16(1);
            let lossless_prev = _mm256_cmpeq_epi16(_mm256_and_si256(flags, one_v), one_v);
            let two_v = _mm256_set1_epi16(2);
            let lossless_curr = _mm256_cmpeq_epi16(_mm256_and_si256(flags, two_v), two_v);
            let s = |j: isize| $rows[(REACH as isize + j) as usize];
            let second = |j: isize| {
                let sum = _mm256_sub_epi16(
                    _mm256_add_epi16(s(j - 1), s(j + 1)),
                    _mm256_add_epi16(s(j), s(j)),
                );
                segment_mean(token, _mm256_abs_epi16(sum))
            };
            let directional = |edge: isize, inner: isize, outer: isize, n: i16| {
                let edge = s(edge);
                let step =
                    _mm256_mullo_epi16(_mm256_sub_epi16(edge, s(inner)), _mm256_set1_epi16(n));
                let deriv = _mm256_sub_epi16(_mm256_sub_epi16(edge, s(outer)), step);
                segment_mean(token, _mm256_abs_epi16(deriv))
            };
            let gt = |a, b| _mm256_cmpgt_epi16(a, b);
            let le = |a, b| _mm256_andnot_si256(_mm256_cmpgt_epi16(a, b), ones);
            let high = _mm256_max_epi16(second(-2), second(1));
            let transition = _mm256_add_epi16(second(-1), second(0));
            let one = _mm256_and_si256(
                le(high, side),
                _mm256_and_si256(gt(q, zero), gt(side, zero)),
            );
            let inner = _mm256_add_epi16(
                _mm256_sub_epi16(s(-2), s(1)),
                _mm256_mullo_epi16(_mm256_sub_epi16(s(0), s(-1)), _mm256_set1_epi16(3)),
            );
            let noop = _mm256_sub_epi16(_mm256_set1_epi16(2), _mm256_cmpeq_epi16(max_pos, one_v));
            let moves = _mm256_and_si256(one, gt(_mm256_abs_epi16(inner), noop));
            if _mm256_testz_si256(moves, moves) != 0 {
                false
            } else {
                let two = _mm256_and_si256(
                    _mm256_and_si256(one, gt(max_pos, one_v)),
                    _mm256_and_si256(
                        le(high, _mm256_srai_epi16::<2>(side)),
                        le(transition, _mm256_slli_epi16::<2>(q)),
                    ),
                );
                let mut index = _mm256_sub_epi16(_mm256_sub_epi16(zero, one), two);
                if reach > 3 {
                    let three = _mm256_and_si256(
                        two,
                        _mm256_and_si256(
                            le(high, _mm256_srai_epi16::<3>(side)),
                            le(transition, _mm256_mullo_epi16(q, _mm256_set1_epi16(3))),
                        ),
                    );
                    let neg_ok = |at: i16, deriv, threshold| {
                        _mm256_andnot_si256(
                            _mm256_and_si256(
                                gt(max_neg, _mm256_set1_epi16(at - 1)),
                                gt(deriv, threshold),
                            ),
                            ones,
                        )
                    };
                    let end =
                        _mm256_srai_epi16::<4>(_mm256_mullo_epi16(side, _mm256_set1_epi16(3)));
                    let three = _mm256_and_si256(
                        three,
                        _mm256_and_si256(
                            neg_ok(3, directional(-1, -2, -4, 3), end),
                            le(directional(0, 1, 3, 3), end),
                        ),
                    );
                    let wide = |previous, dist: i16, q_first: i32, end| {
                        let dist2 = dist.min(7);
                        let q_limit = _mm256_srai_epi16::<4>(_mm256_mullo_epi16(
                            q,
                            _mm256_set1_epi16(q_first as i16),
                        ));
                        let outer = dist2 as isize;
                        _mm256_and_si256(
                            _mm256_and_si256(previous, gt(max_pos, _mm256_set1_epi16(dist - 1))),
                            _mm256_and_si256(
                                le(transition, q_limit),
                                _mm256_and_si256(
                                    neg_ok(dist2, directional(-1, -2, -outer - 1, dist2), end),
                                    le(directional(0, 1, outer, dist2), end),
                                ),
                            ),
                        )
                    };
                    let four = wide(three, 4, Q_FIRST[0], _mm256_srai_epi16::<2>(side));
                    index = _mm256_sub_epi16(_mm256_sub_epi16(index, three), four);
                    if reach > 5 {
                        let six_end =
                            _mm256_srai_epi16::<4>(_mm256_mullo_epi16(side, _mm256_set1_epi16(6)));
                        let six = wide(four, 6, Q_FIRST[2], six_end);
                        let eight = wide(six, 8, Q_FIRST[4], _mm256_srai_epi16::<1>(side));
                        index = _mm256_sub_epi16(_mm256_sub_epi16(index, six), eight);
                    }
                }
                let neg_width = _mm256_add_epi16(
                    max_neg,
                    _mm256_add_epi16(
                        gt(max_neg, _mm256_set1_epi16(4)),
                        gt(max_neg, _mm256_set1_epi16(6)),
                    ),
                );
                let pos_bytes = table_bytes(token, index);
                let neg_bytes = table_bytes(token, _mm256_min_epi16(index, neg_width));
                let q_clamp = _mm256_mullo_epi16(q, table(token, CLAMP, pos_bytes));
                let delta = _mm256_min_epi16(
                    _mm256_max_epi16(
                        _mm256_slli_epi16::<2>(inner),
                        _mm256_sub_epi16(zero, q_clamp),
                    ),
                    q_clamp,
                );
                let ramp = |weight| {
                    _mm256_srai_epi16::<1>(_mm256_add_epi16(
                        _mm256_mulhi_epi16(delta, weight),
                        one_v,
                    ))
                };
                for j in 0..super::chunk_writes(reach) {
                    let pos = _mm256_andnot_si256(lossless_curr, table(token, RAMP[j], pos_bytes));
                    let neg = _mm256_andnot_si256(lossless_prev, table(token, RAMP[j], neg_bytes));
                    $rows[REACH + j] = _mm256_sub_epi16($rows[REACH + j], ramp(pos));
                    $rows[REACH - 1 - j] = _mm256_add_epi16($rows[REACH - 1 - j], ramp(neg));
                }
                true
            }
        }};
    }

    kernel!(
        /// Transposes the 16x16 block of 16-bit lanes in place.
        #[inline(always)]
        fn transpose(token: Avx2, v: &mut [__m256i; 16]) {
            let mut low = [_mm256_setzero_si256(); 8];
            let mut high = [_mm256_setzero_si256(); 8];
            for i in 0..8 {
                low[i] = _mm256_permute2x128_si256::<0x20>(v[i], v[i + 8]);
                high[i] = _mm256_permute2x128_si256::<0x31>(v[i], v[i + 8]);
            }
            let low = transpose8(token, low);
            let high = transpose8(token, high);
            v[..8].copy_from_slice(&low); // splot-copy-ok: transposed vectors stay in registers
            v[8..].copy_from_slice(&high); // splot-copy-ok: transposed vectors stay in registers
        }
    );

    kernel!(
        /// Transposes the 8x8 16-bit blocks in both 128-bit halves.
        #[inline(always)]
        fn transpose8(token: Avx2, x: [__m256i; 8]) -> [__m256i; 8] {
            let a = [
                _mm256_unpacklo_epi16(x[0], x[1]),
                _mm256_unpackhi_epi16(x[0], x[1]),
                _mm256_unpacklo_epi16(x[2], x[3]),
                _mm256_unpackhi_epi16(x[2], x[3]),
                _mm256_unpacklo_epi16(x[4], x[5]),
                _mm256_unpackhi_epi16(x[4], x[5]),
                _mm256_unpacklo_epi16(x[6], x[7]),
                _mm256_unpackhi_epi16(x[6], x[7]),
            ];
            let b = [
                _mm256_unpacklo_epi32(a[0], a[2]),
                _mm256_unpackhi_epi32(a[0], a[2]),
                _mm256_unpacklo_epi32(a[1], a[3]),
                _mm256_unpackhi_epi32(a[1], a[3]),
                _mm256_unpacklo_epi32(a[4], a[6]),
                _mm256_unpackhi_epi32(a[4], a[6]),
                _mm256_unpacklo_epi32(a[5], a[7]),
                _mm256_unpackhi_epi32(a[5], a[7]),
            ];
            [
                _mm256_unpacklo_epi64(b[0], b[4]),
                _mm256_unpackhi_epi64(b[0], b[4]),
                _mm256_unpacklo_epi64(b[1], b[5]),
                _mm256_unpackhi_epi64(b[1], b[5]),
                _mm256_unpacklo_epi64(b[2], b[6]),
                _mm256_unpackhi_epi64(b[2], b[6]),
                _mm256_unpacklo_epi64(b[3], b[7]),
                _mm256_unpackhi_epi64(b[3], b[7]),
            ]
        }
    );

    kernel!(
        #[inline(always)]
        fn load_u16(token: Avx2, samples: &[u16], at: usize) -> __m256i {
            u16x16::from_slice(token, &samples[at..at + 16]).into()
        }
    );

    kernel!(
        /// Stores the 16 lanes of `x` clipped to `0..=max`.
        #[inline(always)]
        fn store_u16(token: Avx2, samples: &mut [u16], at: usize, x: __m256i, max: __m256i) {
            let clipped = _mm256_min_epi16(_mm256_max_epi16(x, _mm256_setzero_si256()), max);
            u16x16::simd_from(token, clipped).store_slice(&mut samples[at..at + 16]);
        }
    );

    /// One chunk kernel: filters the four segments at `first` of a run of
    /// vertical or horizontal edges over `$sample` storage whose widest edge
    /// needs `$reach` rows either side.
    macro_rules! chunk_kernel {
        ($name:ident, $sample:ty, $load:ident, $store:ident, $vertical:expr, $reach:expr) => {
            kernel!(
                #[inline]
                fn $name(
                    token: Avx2,
                    samples: &mut [$sample],
                    stride: usize,
                    first: usize,
                    packed: [i64; 4],
                    max_sample: i16,
                ) {
                    let max = _mm256_set1_epi16(max_sample);
                    let mut rows = [_mm256_setzero_si256(); ROWS];
                    if $vertical {
                        let origin = first - REACH;
                        for (line, row) in rows.iter_mut().enumerate() {
                            *row = $load(token, samples, origin + line * stride);
                        }
                        transpose(token, &mut rows);
                        if !filter_rows!(token, rows, packed, $reach) {
                            return;
                        }
                        transpose(token, &mut rows);
                        for (line, row) in rows.iter().enumerate() {
                            $store(token, samples, origin + line * stride, *row, max);
                        }
                    } else {
                        let origin = first - REACH * stride;
                        for j in REACH - $reach..REACH + $reach {
                            rows[j] = $load(token, samples, origin + j * stride);
                        }
                        if !filter_rows!(token, rows, packed, $reach) {
                            return;
                        }
                        let writes = super::chunk_writes($reach);
                        for j in REACH - writes..REACH + writes {
                            $store(token, samples, origin + j * stride, rows[j], max);
                        }
                    }
                }
            );
        };
    }

    /// The chunk kernels of one storage type and direction, by reach.
    macro_rules! chunk_dispatch {
        ($name:ident, $sample:ty, $load:ident, $store:ident, $vertical:expr) => {
            #[inline]
            pub(super) fn $name(
                token: fearless_simd::Avx2,
                samples: &mut [$sample],
                stride: usize,
                first: usize,
                packed: [i64; 4],
                reach: usize,
                max_sample: i16,
            ) {
                chunk_kernel!(narrow, $sample, $load, $store, $vertical, 3);
                chunk_kernel!(middle, $sample, $load, $store, $vertical, 5);
                chunk_kernel!(wide, $sample, $load, $store, $vertical, REACH);
                match reach {
                    3 => narrow(token, samples, stride, first, packed, max_sample),
                    5 => middle(token, samples, stride, first, packed, max_sample),
                    _ => wide(token, samples, stride, first, packed, max_sample),
                }
            }
        };
    }

    chunk_dispatch!(vertical_u16, u16, load_u16, store_u16, true);
    chunk_dispatch!(horizontal_u16, u16, load_u16, store_u16, false);
}

#[cfg(target_arch = "x86_64")]
#[path = "edges_u8.rs"]
mod avx2_u8;

#[cfg(test)]
#[path = "edges_tests.rs"]
mod tests;
