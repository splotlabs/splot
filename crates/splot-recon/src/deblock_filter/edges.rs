// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Batched AV2 § 7.17.7 deblocking of runs of four-line edge segments
//! (`docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-7`).
//!
//! A chunk of four segments holds one sample line per vector lane, so the
//! § 7.17.7.2 width choice of every segment becomes lane masks and the
//! § 7.17.7.1 ramp applies to all sixteen lines at once: no per-edge branch.

#[cfg(target_arch = "x86_64")]
use fearless_simd::Level;
use fearless_simd::{Bytes, Simd, SimdBase, i16x8, i16x16, u16x8, u16x16, u64x4};
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
#[cfg(target_arch = "x86_64")]
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

    /// Writes to `out` the sixteen edges whose `qThr`, `sideThr`,
    /// `maxWidthPos | maxWidthNeg << 8` and lossless flags (`prev | curr <<
    /// 1`) are the lanes of `words`, the thresholds capped at the largest
    /// values [`DeblockEdge::new`] accepts and the widths taken as § 7.17.3
    /// gives them.
    #[inline]
    pub fn from_words<S: Simd>(simd: S, words: [u16x16<S>; 4], out: &mut [Self; 16]) {
        let [q_thr, side, widths, flags] = words;
        let q_thr = q_thr.min(u16x16::splat(simd, MAX_Q_THR as u16));
        let side = side.min(u16x16::splat(simd, MAX_SIDE as u16));
        let (low, low_high) = simd.interleave_u16x16(q_thr, side);
        let (high, high_high) = simd.interleave_u16x16(widths, flags & 3);
        let edges = |low: u16x16<S>, high: u16x16<S>| {
            let (first, second) = simd.interleave_u32x8(low.bitcast(), high.bitcast());
            (
                first.bitcast::<u64x4<S>>().to_array(),
                second.bitcast::<u64x4<S>>().to_array(),
            )
        };
        let (a, b) = edges(low, high);
        let (c, d) = edges(low_high, high_high);
        for (out, quad) in out.chunks_exact_mut(4).zip([a, b, c, d]) {
            for (out, edge) in out.iter_mut().zip(quad) {
                *out = Self(edge);
            }
        }
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
#[cfg(target_arch = "x86_64")]
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
#[cfg(target_arch = "x86_64")]
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
    deblock_vertical_edge_band(
        samples,
        stride,
        first,
        edges,
        (edges.len(), 1),
        &[true],
        bit_depth,
    )
}

/// [`deblock_vertical_edges`] over `live.len()` columns 4 samples apart
/// whose edges are interleaved row by row: with `(segments, edge_stride)` =
/// `runs`, segment `k` of column `c` is `edges[c + k * edge_stride]`, its
/// first row's `q0` is `first + 4 * c + 4 * k * stride`, and the column
/// filters only when `live[c]`. Columns filter left to right within each
/// row of segments, as § 7.17.1 orders them.
///
/// # Errors
/// As [`deblock_vertical_edges`], for every column, and
/// [`ReconError::DeblockFilterLineTooShort`] when `edges` holds fewer
/// segments than the columns name or a row of `live` columns overlaps the
/// next (`edge_stride < live.len()` with more than one segment).
pub fn deblock_vertical_edge_band<T: ReconSample>(
    samples: &mut [T],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge],
    (segments, edge_stride): (usize, usize),
    live: &[bool],
    bit_depth: BitDepth,
) -> Result<()> {
    validate_sample_type::<T>(bit_depth)?;
    if stride < ROWS {
        return Err(ReconError::StrideTooSmall {
            stride_samples: stride,
            storage_width: ROWS,
        });
    }
    let span = LINES * live.len().saturating_sub(1);
    let column = first % stride;
    let end = (segments * LINES)
        .checked_sub(1)
        .and_then(|last| last.checked_mul(stride))
        .and_then(|offset| first.checked_add(offset + span + REACH));
    let last = segments
        .checked_sub(1)
        .and_then(|row| row.checked_mul(edge_stride))
        .and_then(|row| row.checked_add(live.len()));
    let fits = column >= REACH
        && column + span + REACH <= stride
        && end.is_none_or(|end| end <= samples.len())
        && (segments <= 1 || edge_stride >= live.len())
        && last.is_none_or(|last| live.is_empty() || last <= edges.len());
    if !fits {
        return Err(too_short(first, samples.len()));
    }
    if segments == 0 {
        return Ok(());
    }
    let max_sample = bit_depth.max_sample() as i16;
    let runs = (edges, segments, edge_stride);
    if let Some(samples) = T::u8_slice_mut(samples) {
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = Level::baseline().as_avx2() {
            avx2_u8::band(token, samples, stride, first, runs, live);
            return Ok(());
        }
        band::<u8>(samples, stride, first, runs, live, max_sample);
    } else if let Some(samples) = T::u16_slice_mut(samples) {
        band::<u16>(samples, stride, first, runs, live, max_sample);
    } else {
        return Err(too_short(first, samples.len()));
    }
    Ok(())
}

/// [`deblock_vertical_edge_band`] through [`run`], one live column at a
/// time.
fn band<E: Chunk>(
    samples: &mut [E],
    stride: usize,
    first: usize,
    (edges, segments, edge_stride): (&[DeblockEdge], usize, usize),
    live: &[bool],
    max_sample: i16,
) {
    for (column, _) in live.iter().enumerate().filter(|(_, live)| **live) {
        let column_edges = |k: usize| {
            edges
                .get(column + k * edge_stride)
                .copied()
                .unwrap_or_default()
        };
        let first = first + LINES * column;
        run::<E, true>(samples, stride, first, segments, column_edges, max_sample);
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
    let row_edges = |k: usize| edges.get(k).copied().unwrap_or_default();
    if let Some(samples) = T::u8_slice_mut(samples) {
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = Level::baseline().as_avx2() {
            avx2_u8::horizontal_run(token, samples, stride, first, edges);
            return Ok(());
        }
        run::<u8, false>(samples, stride, first, edges.len(), row_edges, max_sample);
    } else if let Some(samples) = T::u16_slice_mut(samples) {
        run::<u16, false>(samples, stride, first, edges.len(), row_edges, max_sample);
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
    fn load16<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x16<S>;
    fn store16<S: Simd>(simd: S, samples: &mut [Self], at: usize, values: i16x16<S>);
    fn load8<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S>;
    fn store8<S: Simd>(simd: S, samples: &mut [Self], at: usize, values: i16x8<S>);
}

impl EdgeSample for u8 {
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn load16<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x16<S> {
        let bytes = fearless_simd::u8x16::from_slice(simd, &samples[at..at + LANES]);
        let (low, high) = simd.widen_u8x16(bytes);
        simd.combine_u16x8(low, high).bitcast()
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn store16<S: Simd>(simd: S, samples: &mut [Self], at: usize, values: i16x16<S>) {
        let (low, high) = simd.split_u16x16(values.bitcast());
        simd.narrow_u16x8(low, high)
            .store_slice(&mut samples[at..at + LANES]);
    }

    fn load8<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S> {
        let mut bytes = [0; LANES];
        for (byte, &sample) in bytes.iter_mut().zip(&samples[at..at + 8]) {
            *byte = sample;
        }
        simd.widen_u8x16(fearless_simd::u8x16::from_slice(simd, &bytes))
            .0
            .bitcast()
    }

    fn store8<S: Simd>(simd: S, samples: &mut [Self], at: usize, values: i16x8<S>) {
        let values = values.bitcast();
        let bytes = simd.narrow_u16x8(values, values).to_array();
        for (sample, byte) in samples[at..at + 8].iter_mut().zip(bytes) {
            *sample = byte;
        }
    }
}

impl EdgeSample for u16 {
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn load16<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x16<S> {
        u16x16::from_slice(simd, &samples[at..at + LANES]).bitcast()
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn store16<S: Simd>(_simd: S, samples: &mut [Self], at: usize, values: i16x16<S>) {
        values
            .bitcast::<u16x16<S>>()
            .store_slice(&mut samples[at..at + LANES]);
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn load8<S: Simd>(simd: S, samples: &[Self], at: usize) -> i16x8<S> {
        u16x8::from_slice(simd, &samples[at..at + 8]).bitcast()
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn store8<S: Simd>(_simd: S, samples: &mut [Self], at: usize, values: i16x8<S>) {
        values
            .bitcast::<u16x8<S>>()
            .store_slice(&mut samples[at..at + 8]);
    }
}

/// Runs the `count` validated segments `edge(0..count)` in chunks of
/// [`SEGMENTS`]. A last partial chunk filters a copy of its window, so no
/// load or store leaves the run.
fn run<E: Chunk, const VERTICAL: bool>(
    samples: &mut [E],
    stride: usize,
    first: usize,
    count: usize,
    edge: impl Fn(usize) -> DeblockEdge,
    max_sample: i16,
) {
    let step = if VERTICAL { LANES * stride } else { LANES };
    let chunks = count / SEGMENTS;
    let mut at = first;
    for index in 0..chunks {
        let k = index * SEGMENTS;
        let chunk = [edge(k), edge(k + 1), edge(k + 2), edge(k + 3)];
        if chunk.iter().any(|edge| !edge.is_none()) {
            E::chunk::<VERTICAL>(samples, stride, at, &chunk, max_sample);
        }
        at += step;
    }
    let tail = count % SEGMENTS;
    let padded: [DeblockEdge; SEGMENTS] = core::array::from_fn(|lane| {
        if lane < tail {
            edge(chunks * SEGMENTS + lane)
        } else {
            DeblockEdge::NONE
        }
    });
    if padded.iter().all(|edge| edge.is_none()) {
        return;
    }
    let lines = tail * LINES;
    let origin = at;
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
        baseline_simd!(simd => portable_chunk::<_, Self, VERTICAL>(
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
        if !VERTICAL && let Some(token) = Level::baseline().as_avx2() {
            let packed = edges.map(|edge| edge.0 as i64);
            avx2::horizontal_u16(
                token,
                samples,
                stride,
                first,
                packed,
                chunk_reach(edges),
                max_sample,
            );
            return;
        }
        baseline_simd!(simd => portable_chunk::<_, Self, VERTICAL>(
            simd, samples, stride, first, edges, max_sample,
        ));
    }
}

/// The portable chunk: decides the four segments on one vector of their
/// first and last lines, then filters eight lines at a time across the
/// widest width chosen, storing back the lines of a half that moved.
#[inline]
fn portable_chunk<S: Simd, E: EdgeSample, const VERTICAL: bool>(
    simd: S,
    samples: &mut [E],
    stride: usize,
    first: usize,
    edges: &[DeblockEdge; SEGMENTS],
    max_sample: i16,
) {
    let origin = if VERTICAL {
        first - REACH
    } else {
        first - REACH * stride
    };
    let Ok(stride) = u32::try_from(stride).map(|stride| stride as usize) else {
        return;
    };
    let Some(samples) = samples.get_mut(origin..) else {
        return;
    };
    if samples.len() < (ROWS - 1) * stride + ROWS {
        return;
    }
    let chunk = (samples, stride, edges, max_sample);
    match edges
        .iter()
        .fold(0, |widest, edge| widest.max(edge.max_width_pos()))
    {
        0 => {}
        1 => portable_class::<S, E, VERTICAL, 1>(simd, chunk),
        3 => portable_class::<S, E, VERTICAL, 3>(simd, chunk),
        4 => portable_class::<S, E, VERTICAL, 4>(simd, chunk),
        6 => portable_class::<S, E, VERTICAL, 6>(simd, chunk),
        _ => portable_class::<S, E, VERTICAL, 8>(simd, chunk),
    }
}

/// [`portable_chunk`] for the window from `s[-8]` of the chunk's first line
/// (vertical edges) or row (horizontal edges), whose widest `maxWidthPos`
/// is `WIDEST`.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn portable_class<S: Simd, E: EdgeSample, const VERTICAL: bool, const WIDEST: usize>(
    simd: S,
    (samples, stride, edges, max_sample): (&mut [E], usize, &[DeblockEdge; SEGMENTS], i16),
) {
    use portable::{
        DECISION_LINES, HALF_SEGMENTS, OTHER_LINES, OTHER_SEGMENTS, decide, filter, load_lines,
        still, store_lines,
    };
    let read = (WIDEST.min(7) + 1).max(3);
    let wide = read > REACH / 2;
    let zero = i16x8::splat(simd, 0);
    if VERTICAL {
        let mut rows = [zero; ROWS];
        load_lines(simd, samples, stride, &DECISION_LINES, &mut rows, wide);
        if still(simd, &rows) {
            let mut other = [zero; ROWS];
            load_lines(simd, samples, stride, &OTHER_LINES, &mut other, false);
            if still(simd, &other) {
                return;
            }
        }
        let Some((width, ramp)) = decide(simd, &rows, edges, WIDEST) else {
            return;
        };
        let narrow = width <= REACH / 2;
        let mut rows = [rows];
        if filter::<S, WIDEST, 1>(simd, &mut rows, [ramp.0], width, max_sample) {
            store_lines(
                simd,
                samples,
                stride,
                &DECISION_LINES,
                &rows[0],
                wide && !narrow,
            );
        }
        load_lines(simd, samples, stride, &OTHER_LINES, &mut rows[0], !narrow);
        if filter::<S, WIDEST, 1>(
            simd,
            &mut rows,
            [ramp.spread(simd, &OTHER_SEGMENTS)],
            width,
            max_sample,
        ) {
            store_lines(simd, samples, stride, &OTHER_LINES, &rows[0], !narrow);
        }
        return;
    }
    let (mut halves, mut sides) = ([[zero; ROWS]; 2], [zero; ROWS]);
    #[allow(
        clippy::needless_range_loop,
        reason = "iterators here measured 20% more instructions per decided chunk"
    )]
    for j in REACH - read..REACH + read {
        (halves[0][j], halves[1][j]) = simd.split_i16x16(E::load16(simd, samples, j * stride));
    }
    if still(simd, &halves[0]) && still(simd, &halves[1]) {
        return;
    }
    for j in REACH - read..REACH + read {
        sides[j] = portable::sides(simd, halves[0][j], halves[1][j]);
    }
    let Some((width, ramp)) = decide(simd, &sides, edges, WIDEST) else {
        return;
    };
    let ramp = HALF_SEGMENTS.map(|segments| ramp.spread(simd, &segments));
    if filter::<S, WIDEST, 2>(simd, &mut halves, ramp, width, max_sample) {
        let [low, high] = halves;
        for j in REACH - width..REACH + width {
            E::store16(
                simd,
                samples,
                j * stride,
                simd.combine_i16x8(low[j], high[j]),
            );
        }
    }
}

mod portable {
    use fearless_simd::{Bytes, Simd, SimdBase, i16x8, i32x4, i64x2, u8x16, u16x8, u64x2};

    use super::{
        CLAMP, DeblockEdge, EdgeSample, LINES, Q_FIRST, REACH, ROWS, SEGMENTS, W_MULT, WIDTHS,
    };

    /// The lines whose samples decide the segments, in the lane order of a
    /// decision vector: each segment's first line, then each one's last.
    pub(super) const DECISION_LINES: [usize; 8] = [0, 4, 8, 12, 3, 7, 11, 15];
    /// The chunk's other lines.
    pub(super) const OTHER_LINES: [usize; 8] = [1, 2, 5, 6, 9, 10, 13, 14];

    /// Byte indices that copy each segment's decision word to the lanes of
    /// [`OTHER_LINES`], then to the lanes of the two halves of a row across a
    /// horizontal edge (lines `0..8` and `8..16`).
    pub(super) const OTHER_SEGMENTS: [u8; 16] = segments(OTHER_LINES);
    pub(super) const HALF_SEGMENTS: [[u8; 16]; 2] = [
        segments([0, 1, 2, 3, 4, 5, 6, 7]),
        segments([8, 9, 10, 11, 12, 13, 14, 15]),
    ];

    const fn segments(lines: [usize; 8]) -> [u8; 16] {
        let mut segments = [0; 8];
        let mut lane = 0;
        while lane < 8 {
            segments[lane] = lines[lane] / LINES;
            lane += 1;
        }
        words(segments)
    }

    /// Byte indices that pick each lane's 16-bit word from a vector by the
    /// word index `words[lane]`.
    const fn words(words: [usize; 8]) -> [u8; 16] {
        let mut bytes = [0; 16];
        let mut byte = 0;
        while byte < 16 {
            bytes[byte] = (words[byte / 2] * 2 + byte % 2) as u8;
            byte += 1;
        }
        bytes
    }

    /// The [`DECISION_LINES`] of the two halves of a row of 16 lines.
    const SIDES: [u8; 16] = words(DECISION_LINES);

    /// The bytes of the four packed edges that set decision word `w` to
    /// bytes `low` and `high` of edge `w % 4`; byte 7 of an edge is zero.
    const fn field(low: u8, high: u8) -> [u8; 16] {
        let mut bytes = [0; 16];
        let mut byte = 0;
        while byte < 16 {
            let edge = (byte / 2 % SEGMENTS * 8) as u8;
            bytes[byte] = edge + if byte % 2 == 0 { low } else { high };
            byte += 1;
        }
        bytes
    }

    /// `W_Mult[w - 1] * w * 64` and `W_Mult[w - 1] * 64` by compressed width
    /// index: the ramp weight of row 0 and its step outwards.
    const START: [i16; 8] = ramp_table(true);
    const STEP: [i16; 8] = ramp_table(false);

    const fn ramp_table(start: bool) -> [i16; 8] {
        let mut table = [0; 8];
        let mut index = 1;
        while index < WIDTHS.len() {
            let width = WIDTHS[index];
            let scale = if start { width as i32 } else { 1 };
            table[index] = (W_MULT[width - 1] * scale * 64) as i16;
            index += 1;
        }
        table
    }

    /// The § 7.17.7.1 ramp inputs of the four segments, one word each in a
    /// decision vector: `qThrClamp`, then the weights of row 0 and their
    /// steps, current side then previous.
    pub(super) struct Ramp<S: Simd>(pub(super) [i16x8<S>; 5]);

    impl<S: Simd> Ramp<S> {
        /// The ramp over lanes whose segment words `pattern` picks.
        #[allow(
            clippy::inline_always,
            reason = "keeps the portable lanes in registers"
        )]
        #[inline(always)]
        pub(super) fn spread(&self, simd: S, pattern: &[u8; 16]) -> [i16x8<S>; 5] {
            let pattern = u8x16::from_slice(simd, pattern);
            self.0.map(|words| {
                simd.swizzle_dyn_within_blocks_u8x16(words.bitcast(), pattern)
                    .bitcast()
            })
        }
    }

    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    fn transpose8<S: Simd>(simd: S, x: [i16x8<S>; 8]) -> [i16x8<S>; 8] {
        let a = [
            simd.zip_low_i16x8(x[0], x[1]).bitcast::<i32x4<S>>(),
            simd.zip_high_i16x8(x[0], x[1]).bitcast::<i32x4<S>>(),
            simd.zip_low_i16x8(x[2], x[3]).bitcast::<i32x4<S>>(),
            simd.zip_high_i16x8(x[2], x[3]).bitcast::<i32x4<S>>(),
            simd.zip_low_i16x8(x[4], x[5]).bitcast::<i32x4<S>>(),
            simd.zip_high_i16x8(x[4], x[5]).bitcast::<i32x4<S>>(),
            simd.zip_low_i16x8(x[6], x[7]).bitcast::<i32x4<S>>(),
            simd.zip_high_i16x8(x[6], x[7]).bitcast::<i32x4<S>>(),
        ];
        let b = [
            simd.zip_low_i32x4(a[0], a[2]).bitcast::<i64x2<S>>(),
            simd.zip_high_i32x4(a[0], a[2]).bitcast::<i64x2<S>>(),
            simd.zip_low_i32x4(a[1], a[3]).bitcast::<i64x2<S>>(),
            simd.zip_high_i32x4(a[1], a[3]).bitcast::<i64x2<S>>(),
            simd.zip_low_i32x4(a[4], a[6]).bitcast::<i64x2<S>>(),
            simd.zip_high_i32x4(a[4], a[6]).bitcast::<i64x2<S>>(),
            simd.zip_low_i32x4(a[5], a[7]).bitcast::<i64x2<S>>(),
            simd.zip_high_i32x4(a[5], a[7]).bitcast::<i64x2<S>>(),
        ];
        [
            simd.zip_low_i64x2(b[0], b[4]).bitcast(),
            simd.zip_high_i64x2(b[0], b[4]).bitcast(),
            simd.zip_low_i64x2(b[1], b[5]).bitcast(),
            simd.zip_high_i64x2(b[1], b[5]).bitcast(),
            simd.zip_low_i64x2(b[2], b[6]).bitcast(),
            simd.zip_high_i64x2(b[2], b[6]).bitcast(),
            simd.zip_low_i64x2(b[3], b[7]).bitcast(),
            simd.zip_high_i64x2(b[3], b[7]).bitcast(),
        ]
    }

    /// Loads `lines` across a vertical edge into `rows`: row `REACH + j`
    /// holds `s[j]` of each line, over `s[-8..8]` when `wide`, else over
    /// `s[-4..4]` (the other rows are left as they were).
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    pub(super) fn load_lines<S: Simd, E: EdgeSample>(
        simd: S,
        samples: &[E],
        stride: usize,
        lines: &[usize; 8],
        rows: &mut [i16x8<S>; ROWS],
        wide: bool,
    ) {
        let zero = i16x8::splat(simd, 0);
        let (mut low, mut high) = ([zero; 8], [zero; 8]);
        if wide {
            for (k, &line) in lines.iter().enumerate() {
                (low[k], high[k]) = simd.split_i16x16(E::load16(simd, samples, line * stride));
            }
            let (low, high) = (transpose8(simd, low), transpose8(simd, high));
            for (j, (low, high)) in low.into_iter().zip(high).enumerate() {
                (rows[j], rows[REACH + j]) = (low, high);
            }
        } else {
            for (k, &line) in lines.iter().enumerate() {
                low[k] = E::load8(simd, samples, line * stride + REACH / 2);
            }
            for (row, values) in rows[REACH / 2..].iter_mut().zip(transpose8(simd, low)) {
                *row = values;
            }
        }
    }

    /// Stores `rows` back to `lines` as [`load_lines`] loaded them.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    pub(super) fn store_lines<S: Simd, E: EdgeSample>(
        simd: S,
        samples: &mut [E],
        stride: usize,
        lines: &[usize; 8],
        rows: &[i16x8<S>; ROWS],
        wide: bool,
    ) {
        let block = |at: usize| core::array::from_fn(|k| rows[at + k]);
        if wide {
            let (low, high) = (transpose8(simd, block(0)), transpose8(simd, block(REACH)));
            for (k, &line) in lines.iter().enumerate() {
                E::store16(
                    simd,
                    samples,
                    line * stride,
                    simd.combine_i16x8(low[k], high[k]),
                );
            }
        } else {
            for (k, values) in transpose8(simd, block(REACH / 2)).into_iter().enumerate() {
                E::store8(simd, samples, lines[k] * stride + REACH / 2, values);
            }
        }
    }

    /// The [`DECISION_LINES`] of a row of 16 lines split into halves.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    pub(super) fn sides<S: Simd>(simd: S, low: i16x8<S>, high: i16x8<S>) -> i16x8<S> {
        let pattern = u8x16::from_slice(simd, &SIDES);
        simd.concat_swizzle_dyn_u8x16(low.bitcast(), high.bitcast(), pattern)
            .bitcast()
    }

    /// Whether no line whose rows across the edge are `rows` (row `REACH + j`
    /// is `s[j]` of every line) can move: § 7.17.7.1 rounds every ramp step
    /// to zero when `|p1 - q1 + 3 * (q0 - p0)| <= 2`, as `W_Mult[w - 1] * w`
    /// stays below 128.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    pub(super) fn still<S: Simd>(simd: S, rows: &[i16x8<S>; ROWS]) -> bool {
        let s = |j: usize| rows[REACH - 2 + j];
        let inner = (s(0) - s(3) + (s(2) - s(1)) * 3).abs();
        !simd.any_true_mask16x8(inner.simd_gt(i16x8::splat(simd, 2)))
    }

    /// Looks the 8-entry `table` up by the compressed width index in each
    /// word of `index`.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    fn lookup<S: Simd>(simd: S, table: &[i16; 8], index: i16x8<S>) -> i16x8<S> {
        let bytes = (index * 0x0202 + 0x0100).bitcast::<u8x16<S>>();
        let table = i16x8::from_slice(simd, table).bitcast::<u8x16<S>>();
        simd.swizzle_dyn_within_blocks_u8x16(table, bytes).bitcast()
    }

    /// The § 7.17.7.2 widths of the four segments whose decision vectors are
    /// `sides`: vector `REACH + j` holds `s[j]` of every [`DECISION_LINES`]
    /// line, over `s[-n..n]`, `n = Clip3(3, 8, widest + 1)`. Returns the widest
    /// width chosen and the segments' [`Ramp`], or `None` when every width is 0.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    pub(super) fn decide<S: Simd>(
        simd: S,
        sides: &[i16x8<S>; ROWS],
        edges: &[DeblockEdge; SEGMENTS],
        widest: usize,
    ) -> Option<(usize, Ramp<S>)> {
        let packed = edges.map(|edge| edge.0);
        let (low, high) = (
            u64x2::from_slice(simd, &packed[..2]).bitcast::<u8x16<S>>(),
            u64x2::from_slice(simd, &packed[2..]).bitcast::<u8x16<S>>(),
        );
        let pick = |pattern: &[u8; 16]| -> i16x8<S> {
            let pattern = u8x16::from_slice(simd, pattern);
            simd.concat_swizzle_dyn_u8x16(low, high, pattern).bitcast()
        };
        let (q, side) = (pick(&field(0, 1)), pick(&field(2, 3)));
        let (max_pos, max_neg) = (pick(&field(4, 7)), pick(&field(5, 7)));
        let flags = pick(&field(6, 7));
        let e = |j: isize| sides[(REACH as isize + j) as usize];
        let splat = |value: i16| i16x8::splat(simd, value);
        let zero = splat(0);
        let count = |mask| simd.select_i16x8(mask, splat(1), zero);
        let mean = |x: i16x8<S>| {
            let x = x.bitcast::<u16x8<S>>();
            ((x + simd.slide_u16x8::<4>(x, x) + 1) >> 1).bitcast::<i16x8<S>>()
        };
        let second = |a: i16x8<S>, b: i16x8<S>, c: i16x8<S>| mean((a + c - b - b).abs());
        let (em2, em1, e0, e1) = (e(-2), e(-1), e(0), e(1));
        let high_deriv = second(e(-3), em2, em1).max(second(e0, e1, e(2)));
        let one = q.min(side).simd_gt(zero) & high_deriv.simd_le(side);
        let mut index = count(one);
        if widest > 1 {
            let transition = second(em2, em1, e0) + second(em1, e0, e1);
            let passes = |side_limit: i16x8<S>, q_limit: i16x8<S>| {
                high_deriv.simd_le(side_limit) & transition.simd_le(q_limit)
            };
            let two = one & max_pos.simd_gt(splat(1)) & passes(side >> 2, q << 2);
            let three = two & passes(side >> 3, q * 3);
            let toward = |edge: i16x8<S>, inner: i16x8<S>, outer: i16x8<S>, n: i16| {
                mean((edge - outer - (edge - inner) * n).abs())
            };
            let ends = |dist: i16, end: i16x8<S>| {
                let reach = dist as isize;
                let neg = max_neg.simd_lt(splat(dist))
                    | toward(em1, em2, e(-reach - 1), dist).simd_le(end);
                neg & toward(e0, e1, e(reach), dist).simd_le(end)
            };
            let three = three & ends(3, (side * 3) >> 4);
            index += count(two) + count(three);
            if widest > 3 {
                let wide = |previous, dist: i16, q_first: i32, end| {
                    let q_limit = (q * q_first as i16) >> 4;
                    previous
                        & max_pos.simd_ge(splat(dist))
                        & transition.simd_le(q_limit)
                        & ends(dist.min(7), end)
                };
                let four = wide(three, 4, Q_FIRST[0], side >> 2);
                index += count(four);
                if widest > 4 {
                    let six = wide(four, 6, Q_FIRST[2], (side * 6) >> 4);
                    index += count(six);
                    if widest > 6 {
                        index += count(wide(six, 8, Q_FIRST[4], side >> 1));
                    }
                }
            }
        }
        let widest = simd.reduce_max_i16x8(index);
        if widest <= 0 {
            return None;
        }
        let neg_cap = max_neg - count(max_neg.simd_gt(splat(4))) - count(max_neg.simd_gt(splat(6)));
        let keep =
            |index: i16x8<S>, bit: i16| simd.select_i16x8((flags & bit).simd_eq(zero), index, zero);
        let (pos, neg) = (keep(index, 2), keep(index.min(neg_cap), 1));
        let ramp = Ramp([
            q * lookup(simd, &CLAMP, index),
            lookup(simd, &START, pos),
            lookup(simd, &STEP, pos),
            lookup(simd, &START, neg),
            lookup(simd, &STEP, neg),
        ]);
        Some((WIDTHS[widest as usize], ramp))
    }

    /// Filters the lines of `N` halves whose sample rows across the edge are
    /// `halves` (row `REACH + j` is `s[j]` of every line) over
    /// `s[-width..width]` by their [`Ramp::spread`]. Returns `false`, having
    /// changed nothing, when no line moves.
    #[allow(
        clippy::inline_always,
        reason = "keeps the portable lanes in registers"
    )]
    #[inline(always)]
    pub(super) fn filter<S: Simd, const WIDEST: usize, const N: usize>(
        simd: S,
        halves: &mut [[i16x8<S>; ROWS]; N],
        mut ramps: [[i16x8<S>; 5]; N],
        width: usize,
        max_sample: i16,
    ) -> bool {
        let zero = i16x8::splat(simd, 0);
        let high_sample = i16x8::splat(simd, max_sample);
        let deltas: [i16x8<S>; N] = core::array::from_fn(|h| {
            let s = |j: isize| halves[h][(REACH as isize + j) as usize];
            let clamp = ramps[h][0];
            ((s(-2) - s(1) + (s(0) - s(-1)) * 3) << 2)
                .max(zero - clamp)
                .min(clamp)
        });
        let diff = |delta: i16x8<S>, weight: i16x8<S>| {
            let (low, high) = simd.widening_mul_i16x8(delta, weight);
            (simd.unzip_high_i16x8(low.bitcast(), high.bitcast()) + 1) >> 1
        };
        for j in 0..WIDEST {
            if j == width {
                break;
            }
            let diffs: [[i16x8<S>; 2]; N] = core::array::from_fn(|h| {
                [diff(deltas[h], ramps[h][1]), diff(deltas[h], ramps[h][3])]
            });
            if j == 0 {
                let moved = diffs
                    .iter()
                    .fold(zero, |moved, [curr, prev]| moved | *curr | *prev);
                if !simd.any_true_mask16x8(moved.simd_ne(zero)) {
                    return false;
                }
            }
            for (h, [curr, prev]) in diffs.into_iter().enumerate() {
                let rows = &mut halves[h];
                rows[REACH + j] = (rows[REACH + j] - curr).max(zero).min(high_sample);
                rows[REACH - 1 - j] = (rows[REACH - 1 - j] + prev).max(zero).min(high_sample);
                let ramp = &mut ramps[h];
                for k in [1, 3] {
                    ramp[k] = simd
                        .saturating_sub_u16x8(ramp[k].bitcast(), ramp[k + 1].bitcast())
                        .bitcast();
                }
            }
        }
        true
    }
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use core::arch::x86_64::*;

    use fearless_simd::{SimdBase, SimdFrom, kernel, u16x16};

    use super::avx2_u8::MAX_STRIDE;
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

    /// One chunk kernel: filters the four horizontal-edge segments at
    /// `first` whose widest edge needs `$reach` rows either side.
    macro_rules! chunk_kernel {
        ($name:ident, $reach:expr) => {
            kernel!(
                #[inline]
                fn $name(
                    token: Avx2,
                    samples: &mut [u16],
                    stride: usize,
                    first: usize,
                    packed: [i64; 4],
                    max_sample: i16,
                ) {
                    let max = _mm256_set1_epi16(max_sample);
                    let mut rows = [_mm256_setzero_si256(); ROWS];
                    let origin = first - REACH * stride;
                    let Some(samples) =
                        samples.get_mut(origin..origin + (ROWS - 1) * stride + ROWS)
                    else {
                        return;
                    };
                    if stride > MAX_STRIDE {
                        return;
                    }
                    for j in REACH - $reach..REACH + $reach {
                        rows[j] = load_u16(token, samples, j * stride);
                    }
                    if !filter_rows!(token, rows, packed, $reach) {
                        return;
                    }
                    let writes = super::chunk_writes($reach);
                    for j in REACH - writes..REACH + writes {
                        store_u16(token, samples, j * stride, rows[j], max);
                    }
                }
            );
        };
    }

    /// The horizontal-edge chunk kernels over `u16` storage, by reach.
    #[inline]
    pub(super) fn horizontal_u16(
        token: fearless_simd::Avx2,
        samples: &mut [u16],
        stride: usize,
        first: usize,
        packed: [i64; 4],
        reach: usize,
        max_sample: i16,
    ) {
        chunk_kernel!(narrow, 3);
        chunk_kernel!(middle, 5);
        chunk_kernel!(wide, REACH);
        match reach {
            3 => narrow(token, samples, stride, first, packed, max_sample),
            5 => middle(token, samples, stride, first, packed, max_sample),
            _ => wide(token, samples, stride, first, packed, max_sample),
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[path = "edges_u8.rs"]
mod avx2_u8;

#[cfg(test)]
#[path = "edges_tests.rs"]
mod tests;
