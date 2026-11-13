// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AVX2 4x4 and 8x8 register cells of [`super`]: both passes of a
//! cell in registers, the vertical pass reading its rows from the
//! horizontal groups. A cell reads its window in place, or by clamped row
//! addresses and a `pshufb` column gather, so a border cell needs no copy.

use super::{Horizontal, SKIPPED, load16, store_rows};
use crate::subpel_mc::ReferencePlaneView;
use crate::subpel_mc::block::{Block, Geometry, Taps, Window, is_cell};
use crate::subpel_mc::cell::{Cell, Plane, Written};
use core::arch::x86_64::*;
use fearless_simd::{Avx2, SimdBase, SimdFrom, u8x32, u32x8};

/// `pshufb` pattern widening columns `0..4` of the two 8-byte rows of
/// each lane to words.
const NARROW_ROWS: [u8; 32] = {
    let mut out = [0x80; 32];
    let mut lane = 0;
    while lane < 32 {
        if lane % 2 == 0 {
            let word = (lane % 16) / 2;
            out[lane] = ((word / 4) * 8 + word % 4) as u8;
        }
        lane += 1;
    }
    out
};

/// `Some` of `$f::<$w, G, H, V, $put, $c>` for the kernel key `5 * H + V`
/// of a reference of a `$w`-square cell, `None` for any other key.
macro_rules! by_key {
    ($key:expr, 8, $put:literal, $c:expr, $f:ident $args:tt) => {
        by_key!(@ $key, 8, $put, $c, $f $args, [
            0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
            10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4, 15 3 0, 16 3 1, 17 3 2, 18 3 3,
            19 3 4, 20 4 0, 21 4 1, 22 4 2, 23 4 3, 24 4 4
        ])
    };
    ($key:expr, 4, $put:literal, $c:expr, $f:ident $args:tt) => {
        by_key!(@ $key, 4, $put, $c, $f $args, [
            0 0 0, 1 0 1, 2 0 2, 5 1 0, 6 1 1, 7 1 2, 10 2 0, 11 2 1, 12 2 2
        ])
    };
    (@ $key:expr, $w:literal, $put:literal, $c:expr, $f:ident $args:tt,
        [$($k:literal $h:literal $v:literal),*]) => {
        match ($key, $c) {
            $(($k, false) => Some($f::<$w, G, $h, $v, $put, false> $args),)*
            $(($k, true) => Some($f::<$w, G, $h, $v, $put, true> $args),)*
            _ => None,
        }
    };
}

/// Where a cell reads window row `r`: `base + r * stride`, or when
/// clamped `base + rows[r]`, window column `c` from byte
/// `clamp(start + c, 0, high)` of the load.
#[derive(Clone, Copy)]
struct Reads {
    base: u32,
    stride: u32,
    rows: [u32; 16],
    start: i8,
    high: i8,
}

impl Reads {
    /// The in-place reads of `block`, `None` when its storage reaches
    /// `2^32` or its stride is below 16 or reaches `2^32`.
    fn of(block: &Block<'_, u8>) -> Option<Self> {
        u32::try_from(block.source.len()).ok()?;
        let stride = u32::try_from(block.stride)
            .ok()
            .filter(|&stride| stride >= 16)?;
        Some(Self::in_place(block.origin as u32, stride))
    }

    const fn in_place(base: u32, stride: u32) -> Self {
        Self {
            base,
            stride,
            rows: [0; 16],
            start: 0,
            high: 15,
        }
    }
}

/// The [`Reads`] of a `W`-square [`is_cell`] `window` in a plane of
/// `geometry` within `bounds` and whether they are clamped: in place when
/// every read is inside the clip bounds, else the § 7.13.3.18 `Clip3`
/// reads; `None` for another window, an empty clip range, a clamped row
/// that cannot load 16 samples or storage of `2^32` samples or more.
#[inline]
#[target_feature(enable = "avx2")]
fn reads<const W: usize>(
    t: Avx2,
    window: &Window,
    geometry: Geometry,
    bounds: [i32; 4],
) -> Option<(Reads, bool)> {
    let [stride, width, rows, len] = geometry;
    if window.w != W || !is_cell(W, W, window.horizontal, window.vertical) {
        return None;
    }
    u32::try_from(len).ok()?;
    let stride32 = u32::try_from(stride).ok().filter(|&stride| stride >= 16)?;
    if let Some(base) = window.origin(geometry, bounds) {
        return Some((Reads::in_place(base as u32, stride32), false));
    }
    let bound = |value: i32, size: usize| value.max(0).min(size as i32 - 1);
    let (first_x, last_x) = (bound(bounds[0], width), bound(bounds[2], width));
    let (first, last) = (bound(bounds[1], rows), bound(bounds[3], rows));
    if first_x > last_x || first > last {
        return None;
    }
    let column = window.column.max(first_x).min(last_x);
    if last as usize * stride + column as usize + 16 > len {
        return None;
    }
    let start = window.column.max(first_x - 16).min(last_x + 16) - column;
    let reads = Reads {
        base: column as u32,
        stride: stride32,
        rows: row_offsets(t, window.row, [first, last], stride32),
        start: start as i8,
        high: (last_x - column).min(15) as i8,
    };
    Some((reads, true))
}

/// The offsets `clamp(top + r, first, last) * stride` of window rows
/// `r` in `0..16`; `top` is first clamped so the sums cannot overflow.
#[inline]
#[target_feature(enable = "avx2")]
fn row_offsets(t: Avx2, top: i32, [first, last]: [i32; 2], stride: u32) -> [u32; 16] {
    let top = _mm256_set1_epi32(top.max(first - 16).min(last));
    let (first, last) = (_mm256_set1_epi32(first), _mm256_set1_epi32(last));
    let mut out = [0; 16];
    for (half, lanes) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        let rows = _mm256_add_epi32(top, _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7));
        let rows = _mm256_add_epi32(rows, _mm256_set1_epi32(8 * half as i32));
        let rows = _mm256_min_epi32(_mm256_max_epi32(rows, first), last);
        let offsets = _mm256_mullo_epi32(rows, _mm256_set1_epi32(stride as i32));
        u32x8::<Avx2>::simd_from(t, offsets).store_array(lanes);
    }
    out
}

/// The kernel key `5 * H + V` of a reference's tap-pair counts, zero for
/// a skipped pass.
fn key(horizontal: Option<&Taps>, vertical: Option<&Taps>) -> usize {
    5 * horizontal.map_or(0, |taps| taps.count) + vertical.map_or(0, |taps| taps.count)
}

/// The groups of one reference of a `W`-square cell of `G` groups, `None`
/// for a key no kernel takes.
#[inline]
#[target_feature(enable = "avx2")]
fn groups<const W: usize, const G: usize, const PUT: bool>(
    t: Avx2,
    source: &[u8],
    (reads, clamped): (&Reads, bool),
    horizontal: Option<&Taps>,
    vertical: Option<&Taps>,
) -> Option<[__m256i; G]> {
    let key = key(horizontal, vertical);
    let taps = [horizontal.unwrap_or(&SKIPPED), vertical.unwrap_or(&SKIPPED)];
    match (W, PUT) {
        (8, false) => by_key!(key, 8, false, clamped, predict(t, source, reads, taps)),
        (8, true) => by_key!(key, 8, true, clamped, predict(t, source, reads, taps)),
        (_, false) => by_key!(key, 4, false, clamped, predict(t, source, reads, taps)),
        (_, true) => by_key!(key, 4, true, clamped, predict(t, source, reads, taps)),
    }
}

/// An output stride whose eight rows stay below `2^32`.
#[inline]
fn output_stride(stride: usize) -> Option<u32> {
    u32::try_from(stride)
        .ok()
        .filter(|&stride| stride <= u32::MAX / 8)
}

/// Stores the COMPOUND_AVERAGE of the groups `[first, second]` of a
/// `$w`-square cell, the average form chosen once per cell, in the caller.
macro_rules! store_average {
    ($t:expr, $w:ident, [$first:expr, $second:expr], $forward:expr, $output:expr, $stride:expr) => {{
        let (t, forward, output, stride) = ($t, $forward, $output, $stride);
        let pairs = $first.into_iter().zip($second);
        if forward == 8 {
            let round = _mm256_set1_epi16(1 << 10);
            for (group, (a, b)) in (0..).zip(pairs) {
                let blended = _mm256_mulhrs_epi16(_mm256_add_epi16(a, b), round);
                store_rows::<$w, u32>(t, output, 2 * group * stride, stride, blended);
            }
        } else {
            let weights = _mm256_set1_epi32((forward & 0xffff) | ((16 - forward) << 16));
            let bias = _mm256_set1_epi32(1 << 7);
            for (group, (a, b)) in (0..).zip(pairs) {
                let low = _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights);
                let high = _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights);
                let blended = _mm256_packs_epi32(
                    _mm256_srai_epi32::<8>(_mm256_add_epi32(low, bias)),
                    _mm256_srai_epi32::<8>(_mm256_add_epi32(high, bias)),
                );
                store_rows::<$w, u32>(t, output, 2 * group * stride, stride, blended);
            }
        }
    }};
}

/// The COMPOUND_AVERAGE of the in-place blocks of a `W`-square cell pair,
/// `None` without writing.
#[inline]
#[target_feature(enable = "avx2")]
fn compound_blocks<const W: usize, const G: usize>(
    t: Avx2,
    blocks: [&Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: u32,
) -> Option<()> {
    let first = groups::<W, G, false>(
        t,
        blocks[0].source,
        (&Reads::of(blocks[0])?, false),
        blocks[0].horizontal,
        blocks[0].vertical,
    )?;
    let second = groups::<W, G, false>(
        t,
        blocks[1].source,
        (&Reads::of(blocks[1])?, false),
        blocks[1].horizontal,
        blocks[1].vertical,
    )?;
    store_average!(t, W, [first, second], forward, output, stride);
    Some(())
}

fearless_simd::kernel!(
    /// Stores the COMPOUND_AVERAGE of the blocks of a `size`-square
    /// cell, or returns `false` without writing.
    #[inline]
    pub(super) fn compound(
        t: Avx2,
        blocks: [&Block<'_, u8>; 2],
        size: usize,
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) -> bool {
        let Some(stride) = output_stride(stride) else {
            return false;
        };
        if size == 4 {
            compound_blocks::<4, 1>(t, blocks, forward, output, stride)
        } else {
            compound_blocks::<8, 4>(t, blocks, forward, output, stride)
        }
        .is_some()
    }
);

fearless_simd::kernel!(
    /// Stores the `Clip1` prediction of a filtered `size`-square block, or
    /// returns `false` without writing.
    #[inline]
    pub(super) fn put(
        t: Avx2,
        block: &Block<'_, u8>,
        size: usize,
        output: &mut [u8],
        stride: usize,
    ) -> bool {
        let Some(reads) = Reads::of(block) else {
            return false;
        };
        let (source, h, v) = (block.source, block.horizontal, block.vertical);
        if size == 4 {
            let Some([values]) = groups::<4, 1, true>(t, source, (&reads, false), h, v) else {
                return false;
            };
            store_rows::<4, usize>(t, output, 0, stride, values);
            return true;
        }
        let Some(groups) = groups::<8, 4, true>(t, source, (&reads, false), h, v) else {
            return false;
        };
        for (group, values) in groups.into_iter().enumerate() {
            store_rows::<8, usize>(t, output, 2 * group * stride, stride, values);
        }
        true
    }
);

/// The groups of the `W`-square cell of `window` of `view` within
/// `bounds`, in place or clamped; `None` for a cell no kernel takes.
#[inline]
#[target_feature(enable = "avx2")]
fn window_groups<const W: usize, const G: usize>(
    t: Avx2,
    view: &ReferencePlaneView<'_, u8>,
    window: &Window,
    bounds: [i32; 4],
) -> Option<[__m256i; G]> {
    let (reads, clamped) = reads::<W>(t, window, view.geometry(), bounds)?;
    groups::<W, G, false>(
        t,
        view.samples,
        (&reads, clamped),
        window.horizontal,
        window.vertical,
    )
}

fearless_simd::kernel!(
    /// Stores the COMPOUND_AVERAGE of the 4x4 or 8x8 cell of `windows`
    /// within `bounds`, each read in place or by clamped rows and columns
    /// of `views`; `false` without writing for a cell no kernel takes.
    #[inline]
    pub(in crate::subpel_mc) fn compound_windows(
        t: Avx2,
        views: [&ReferencePlaneView<'_, u8>; 2],
        windows: &[Window; 2],
        bounds: [[i32; 4]; 2],
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) -> bool {
        let size = windows[0].w;
        let fits = (size - 1) * stride + size <= output.len() && stride >= size;
        let Some(stride) = output_stride(stride).filter(|_| fits && windows[1].w == size) else {
            return false;
        };
        if size == 4 {
            windows_cell::<4, 1>(t, views, windows, bounds, forward, output, stride)
        } else {
            windows_cell::<8, 4>(t, views, windows, bounds, forward, output, stride)
        }
        .is_some()
    }
);

/// [`compound_windows`] of a `W`-square cell of `G` groups.
#[inline]
#[target_feature(enable = "avx2")]
fn windows_cell<const W: usize, const G: usize>(
    t: Avx2,
    views: [&ReferencePlaneView<'_, u8>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    forward: i32,
    output: &mut [u8],
    stride: u32,
) -> Option<()> {
    let first = window_groups::<W, G>(t, views[0], &windows[0], bounds[0])?;
    let second = window_groups::<W, G>(t, views[1], &windows[1], bounds[1])?;
    store_average!(t, W, [first, second], forward, output, stride);
    Some(())
}

fearless_simd::kernel!(
    /// Runs the cells of a program of `size`-square cells, 4 or 8, whose
    /// windows are read in place or by clamped rows and columns into every
    /// plane, and returns their bits.
    #[inline]
    pub(in crate::subpel_mc) fn program(
        t: Avx2,
        planes: &mut [Option<Plane<'_, '_, u8>>; 2],
        shape: (usize, usize),
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written {
        if shape.1 == 4 {
            run_program::<4, 1>(t, planes, shape.0, cells, clip.0)
        } else if shape.1 == 8 {
            run_program::<8, 4>(t, planes, shape.0, cells, clip.0)
        } else {
            [[0; 4]; 2]
        }
    }
);

/// The program of `W`-square cells of `filter`: the reads of each window
/// derived once, both planes stored from them.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn run_program<const W: usize, const G: usize>(
    t: Avx2,
    planes: &mut [Option<Plane<'_, '_, u8>>; 2],
    filter: usize,
    cells: &[Cell],
    forward: i32,
) -> Written {
    let mut written = [[0u64; 4]; 2];
    let Some((views, ..)) = &planes[0] else {
        return written;
    };
    let geometry = [views[0].geometry(), views[1].geometry()];
    for (index, &([row, col], placements)) in cells.iter().enumerate() {
        let windows = placements.map(|(start, _)| Window::of_filters([filter; 2], W, W, start));
        let (Some(first), Some(second)) = (
            reads::<W>(t, &windows[0], geometry[0], placements[0].1),
            reads::<W>(t, &windows[1], geometry[1], placements[1].1),
        ) else {
            continue;
        };
        for (plane, written) in planes.iter_mut().zip(&mut written) {
            let Some((views, output, stride)) = plane else {
                continue;
            };
            let at = row as usize * *stride + col as usize;
            let (Some(out_stride), Some(output)) = (
                output_stride(*stride),
                output
                    .get_mut(at..)
                    .filter(|output| (W - 1) * *stride + W <= output.len()),
            ) else {
                continue;
            };
            let (Some(a), Some(b)) = (
                groups::<W, G, false>(
                    t,
                    views[0].samples,
                    (&first.0, first.1),
                    windows[0].horizontal,
                    windows[0].vertical,
                ),
                groups::<W, G, false>(
                    t,
                    views[1].samples,
                    (&second.0, second.1),
                    windows[1].horizontal,
                    windows[1].vertical,
                ),
            ) else {
                continue;
            };
            store_average!(t, W, [a, b], forward, output, out_stride);
            written[index / 64 % 4] |= 1 << (index % 64);
        }
    }
    written
}

/// The 16-byte loads of window rows `0..16` of a cell of `rows` rows read
/// by `reads`, rows past the window repeating its last: in place one
/// bounds check per window, clamped one per row.
#[allow(clippy::inline_always, reason = "one bounds check per window")]
#[inline(always)]
fn rows_of<'a, const C: bool>(
    source: &'a [u8],
    reads: &Reads,
    rows: usize,
) -> Option<[&'a [u8; 16]; 16]> {
    let stride = reads.stride as usize;
    if C {
        let offsets = reads.rows.get(..rows)?;
        let low = reads.base as usize + *offsets.first()? as usize;
        let span = (reads.base as usize + *offsets.last()? as usize).checked_sub(low)?;
        let window = source.get(low..)?.get(..span + 16)?;
        let mut out = [&[0u8; 16]; 16];
        for (slot, &row) in out.iter_mut().zip(offsets) {
            let at = (reads.base as usize + row as usize)
                .wrapping_sub(low)
                .min(span);
            *slot = window.get(at..)?.first_chunk::<16>()?;
        }
        let last = out[rows - 1];
        for slot in &mut out[rows..] {
            *slot = last;
        }
        return Some(out);
    }
    if stride < 16 {
        return None;
    }
    let window = source.get(reads.base as usize..)?;
    let (body, last) = window.split_at_checked((rows - 1) * stride)?;
    let mut out = [last.first_chunk::<16>()?; 16];
    for (slot, row) in out.iter_mut().zip(body.chunks_exact(stride)) {
        *slot = row.first_chunk::<16>()?;
    }
    Some(out)
}

/// The predictors of one `W`-square cell of `G` groups whose passes have
/// `H` and `V` tap pairs, none for a skipped pass, read by `reads`, with
/// `C` clamped; with `PUT` single-reference predictions before `Clip1`.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn predict<
    const W: usize,
    const G: usize,
    const H: usize,
    const V: usize,
    const PUT: bool,
    const C: bool,
>(
    t: Avx2,
    source: &[u8],
    reads: &Reads,
    [horizontal, vertical]: [&Taps; 2],
) -> [__m256i; G] {
    let per = 16 / W;
    let rows = if V > 0 { W + 2 * V - 1 } else { W };
    let zero = _mm256_setzero_si256();
    let (start, high) = (reads.start, reads.high);
    let Some(rows_of) = rows_of::<C>(source, reads, rows) else {
        return [zero; G];
    };
    let gather = {
        let lanes = if W == 4 {
            _mm_set_epi64x(0x0706_0504_0302_0100, 0x0706_0504_0302_0100)
        } else {
            _mm_set_epi64x(0x0f0e_0d0c_0b0a_0908, 0x0706_0504_0302_0100)
        };
        let limit = if W == 4 { high.min(7) } else { high };
        let lanes = _mm_add_epi8(lanes, _mm_set1_epi8(start));
        let lanes = _mm_min_epi8(
            _mm_max_epi8(lanes, _mm_setzero_si128()),
            _mm_set1_epi8(limit),
        );
        let halves = if W == 4 {
            _mm_set_epi64x(0x0808_0808_0808_0808, 0)
        } else {
            _mm_setzero_si128()
        };
        _mm256_broadcastsi128_si256(_mm_add_epi8(lanes, halves))
    };
    let filter = (H > 0).then(|| Horizontal::new::<W, H>(t, horizontal));
    let narrow: __m256i = u8x32::load_array(t, NARROW_ROWS).into();
    let mut lines = [zero; 8];
    for (group, line) in lines.iter_mut().enumerate().take(rows.div_ceil(per)) {
        let row = group * per;
        let bytes = if W == 4 {
            let load = |row: usize| {
                rows_of[row]
                    .first_chunk::<8>()
                    .map_or(0, |b| i64::from_le_bytes(*b))
            };
            _mm256_set_epi64x(load(row + 3), load(row + 2), load(row + 1), load(row))
        } else {
            _mm256_set_m128i(load16(t, rows_of[row + 1]), load16(t, rows_of[row]))
        };
        let bytes = if C {
            _mm256_shuffle_epi8(bytes, gather)
        } else {
            bytes
        };
        *line = match &filter {
            Some(filter) if PUT && V == 0 => _mm256_srai_epi16::<6>(_mm256_add_epi16(
                filter.sums::<H>(bytes),
                _mm256_set1_epi16(34),
            )),
            Some(filter) => {
                _mm256_mulhrs_epi16(filter.sums::<H>(bytes), _mm256_set1_epi16(1 << 13))
            }
            None if W == 4 => _mm256_shuffle_epi8(bytes, narrow),
            None => _mm256_unpacklo_epi8(bytes, zero),
        };
    }
    let mut out = [zero; G];
    if V == 0 {
        for (out, &line) in out.iter_mut().zip(&lines) {
            *out = if H > 0 || PUT {
                line
            } else {
                _mm256_slli_epi16::<4>(line)
            };
        }
        return out;
    }
    let line = |row: usize| -> __m256i {
        let (index, offset) = (row / per, row % per);
        let first = lines[index.min(7)];
        if offset == 0 {
            return first;
        }
        let second = lines[(index + 1).min(7)];
        let middle = _mm256_permute2x128_si256::<0x21>(first, second);
        match (W, offset) {
            (8, _) | (_, 2) => middle,
            (_, 1) => _mm256_alignr_epi8::<8>(middle, first),
            _ => _mm256_alignr_epi8::<8>(second, middle),
        }
    };
    let weights: [__m256i; 4] =
        core::array::from_fn(|pair| _mm256_set1_epi32(vertical.words[pair.min(V.max(1) - 1)]));
    let bias = _mm256_set1_epi32(match (H > 0, PUT) {
        (true, true) => 1 << 9,
        (true, false) | (false, true) => 1 << 5,
        (false, false) => 1 << 1,
    });
    let mut sums = [[bias; 2]; G];
    for row in 0..G + V - 1 {
        let (a, b) = (line(2 * row), line(2 * row + 1));
        let (low, high) = (_mm256_unpacklo_epi16(a, b), _mm256_unpackhi_epi16(a, b));
        for (group, sums) in sums.iter_mut().enumerate() {
            if let Some(&weights) = row
                .checked_sub(group)
                .and_then(|pair| weights[..V].get(pair))
            {
                sums[0] = _mm256_add_epi32(sums[0], _mm256_madd_epi16(low, weights));
                sums[1] = _mm256_add_epi32(sums[1], _mm256_madd_epi16(high, weights));
            }
        }
    }
    for (out, [low, high]) in out.iter_mut().zip(sums) {
        *out = match (H > 0, PUT) {
            (true, true) => {
                _mm256_packs_epi32(_mm256_srai_epi32::<10>(low), _mm256_srai_epi32::<10>(high))
            }
            (true, false) | (false, true) => {
                _mm256_packs_epi32(_mm256_srai_epi32::<6>(low), _mm256_srai_epi32::<6>(high))
            }
            (false, false) => {
                _mm256_packs_epi32(_mm256_srai_epi32::<2>(low), _mm256_srai_epi32::<2>(high))
            }
        };
    }
    out
}
