// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AVX2 4x4 and 8x8 register compound cells of [`super`]: both passes
//! of each reference in registers, the window read in place or by clamped
//! row offsets and a column gather, so a border cell needs no copy; and the
//! program kernel that runs a grid of cells from their placements.

use super::super::super::block::{Block, Geometry, Taps, Window};
use super::super::super::cell::{Cell as ProgramCell, Plane, Written};
use super::super::super::rows;
use super::super::super::{CellPlacement, ReferencePlaneView};
use super::{
    Word, average, generic, interleave, load8, madds, pair, pairs_of, round, sums_of, token,
    weights,
};
use core::arch::x86_64::*;
use fearless_simd::Avx2;

/// The COMPOUND_AVERAGE, clipped to `clip.1`, of a 4x4 or 8x8 cell pair
/// in registers.
pub(in crate::subpel_mc::sixteen_bit) fn compound_cell(
    blocks: [&Block<'_, u16>; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) {
    match token() {
        Some(t) => cell_pair(t, blocks, clip, output, stride),
        None => generic::compound_cell(blocks, clip, output, stride),
    }
}

/// Where a cell reads window row `r`: `base + r * stride`, or when clamped
/// `base + clamp(top + r, first, last) * stride`, window column `c` from
/// sample `clamp(start + c, 0, high)` of the row.
#[derive(Clone, Copy)]
struct Reads {
    base: usize,
    stride: usize,
    top: i32,
    first: i32,
    last: i32,
    start: i16,
    high: i16,
}

impl Reads {
    const fn in_place(base: usize, stride: usize) -> Self {
        Self {
            base,
            stride,
            top: 0,
            first: 0,
            last: 0,
            start: 0,
            high: 15,
        }
    }
}

/// A reference of a register cell: its [`Reads`], whether they are
/// clamped, and the taps of both passes.
type Placed = (Reads, bool, Option<&'static Taps>, Option<&'static Taps>);

/// The [`Placed`] reads of the `W`-square [`super::super::block::is_cell`]
/// `window` in a plane of `geometry` within `bounds`: in place when every
/// read is inside the clip bounds, else the § 7.13.3.18 `Clip3` reads;
/// `None` for an empty clip range or a clamped row that cannot load 16
/// samples.
#[inline]
fn place<const W: usize>(window: &Window, geometry: Geometry, bounds: [i32; 4]) -> Option<Placed> {
    let small = |taps: Option<&Taps>| W == 8 || taps.is_none_or(|taps| taps.count <= 2);
    if window.w != W || !small(window.horizontal) || !small(window.vertical) {
        return None;
    }
    let taps = (window.horizontal, window.vertical);
    let [stride, width, rows, len] = geometry;
    if let Some(base) = window.origin(geometry, bounds) {
        return Some((Reads::in_place(base, stride), false, taps.0, taps.1));
    }
    let bound = |value: i32, size: usize| value.max(0).min(size as i32 - 1);
    let (first_x, last_x) = (bound(bounds[0], width), bound(bounds[2], width));
    let (first, last) = (bound(bounds[1], rows), bound(bounds[3], rows));
    if first_x > last_x || first > last {
        return None;
    }
    let column = window.column.max(first_x).min(last_x);
    let end = (last as usize)
        .checked_mul(stride)?
        .checked_add(column as usize + 16)?;
    if end > len {
        return None;
    }
    let reads = Reads {
        base: column as usize,
        stride,
        top: window.row,
        first,
        last,
        start: (window.column.max(first_x - 16).min(last_x + 16) - column) as i16,
        high: (last_x - column).min(15) as i16,
    };
    Some((reads, true, taps.0, taps.1))
}

fearless_simd::kernel!(
    #[inline]
    fn cell_pair(
        t: Avx2,
        blocks: [&Block<'_, u16>; 2],
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) {
        let [a, b] = blocks.map(|block| {
            let reads = Reads::in_place(block.origin, block.stride);
            (reads, false, block.horizontal, block.vertical)
        });
        let sources = [blocks[0].source, blocks[1].source];
        if blocks[0].w == 4 {
            store_cell::<4>(t, sources, [&a, &b], clip, output, stride);
        } else {
            store_cell::<8>(t, sources, [&a, &b], clip, output, stride);
        }
    }
);

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of the 4x4 or 8x8
/// cell of `windows` within `bounds`, each read in place or by clamped rows
/// and columns of `views`; `false` without writing for a cell no kernel
/// takes.
pub(in crate::subpel_mc::sixteen_bit) fn compound_windows(
    views: [&ReferencePlaneView<'_, u16>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    match token() {
        Some(t) => windows_cell(t, views, windows, bounds, clip, output, stride),
        None => generic::compound_windows(views, windows, bounds, clip, output, stride),
    }
}

fearless_simd::kernel!(
    #[inline]
    fn windows_cell(
        t: Avx2,
        views: [&ReferencePlaneView<'_, u16>; 2],
        windows: &[Window; 2],
        bounds: [[i32; 4]; 2],
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) -> bool {
        let sources = [views[0].samples, views[1].samples];
        macro_rules! cell {
            ($w:literal) => {{
                let (Some(a), Some(b)) = (
                    place::<$w>(&windows[0], views[0].geometry(), bounds[0]),
                    place::<$w>(&windows[1], views[1].geometry(), bounds[1]),
                ) else {
                    return false;
                };
                store_cell::<$w>(t, sources, [&a, &b], clip, output, stride)
            }};
        }
        if windows[0].w == 4 {
            cell!(4)
        } else {
            cell!(8)
        }
    }
);

/// Runs the cells of a program of `size`-square cells whose windows are
/// read in place or by clamped rows and columns into every plane, and
/// returns their bits.
pub(in crate::subpel_mc::sixteen_bit) fn program(
    planes: &mut [Option<Plane<'_, '_, u16>>; 2],
    shape: (usize, usize),
    cells: &[ProgramCell],
    clip: (i32, u16),
) -> Written {
    match token() {
        Some(t) => run_cells(t, planes, shape, cells, clip),
        None => generic::program(planes, shape, cells, clip),
    }
}

fearless_simd::kernel!(
    #[inline]
    fn run_cells(
        t: Avx2,
        planes: &mut [Option<Plane<'_, '_, u16>>; 2],
        shape: (usize, usize),
        cells: &[ProgramCell],
        clip: (i32, u16),
    ) -> Written {
        let (filter, size) = shape;
        match size {
            4 => run_program::<4>(t, planes, filter, cells, clip),
            8 => run_program::<8>(t, planes, filter, cells, clip),
            _ => [[0; 4]; 2],
        }
    }
);

/// [`program`] of `W`-square cells of `filter`: each reference's reads
/// derived once per cell, every plane stored from them.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn run_program<const W: usize>(
    t: Avx2,
    planes: &mut [Option<Plane<'_, '_, u16>>; 2],
    filter: usize,
    cells: &[ProgramCell],
    clip: (i32, u16),
) -> Written {
    let mut written = [[0u64; 4]; 2];
    let Some((views, ..)) = &planes[0] else {
        return written;
    };
    let geometry = [views[0].geometry(), views[1].geometry()];
    for (index, &([row, col], placements)) in cells.iter().enumerate() {
        let window = |(start, bounds): CellPlacement, geometry: Geometry| {
            place::<W>(
                &Window::of_filters([filter; 2], W, W, start),
                geometry,
                bounds,
            )
        };
        let (Some(a), Some(b)) = (
            window(placements[0], geometry[0]),
            window(placements[1], geometry[1]),
        ) else {
            continue;
        };
        for (plane, written) in planes.iter_mut().zip(&mut written) {
            let Some((views, output, stride)) = plane else {
                continue;
            };
            let at = row as usize * *stride + col as usize;
            let Some(output) = output.get_mut(at..) else {
                continue;
            };
            let sources = [views[0].samples, views[1].samples];
            if store_cell::<W>(t, sources, [&a, &b], clip, output, *stride) {
                written[index / 64 % 4] |= 1 << (index % 64);
            }
        }
    }
    written
}

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of a `W`-square cell
/// of both references' [`Placed`] reads of `sources`; `false` without
/// writing when a read leaves its source or the output does not hold the
/// cell.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn store_cell<const W: usize>(
    t: Avx2,
    sources: [&[u16]; 2],
    [a, b]: [&Placed; 2],
    (forward, max): (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    let span = (W - 1).checked_mul(stride);
    if stride < W
        || span.is_none_or(|span| output.len().checked_sub(W).is_none_or(|room| span > room))
    {
        return false;
    }
    let still = |placed: &Placed| !placed.1 && placed.2.is_none() && placed.3.is_none();
    if still(a) && still(b) && (0..=16).contains(&forward) {
        let sources = [(sources[0], a.0.stride), (sources[1], b.0.stride)];
        let origins = [a.0.base, b.0.base];
        super::super::super::cell::fullpel_u16(sources, origins, W, (forward, max), output, stride);
        return true;
    }
    let (Some(first), Some(second)) = (
        cell_of::<W>(t, sources[0], a),
        cell_of::<W>(t, sources[1], b),
    ) else {
        return false;
    };
    let Some((mut rows, last)) = rows::rows_mut(output, 0, stride, W, W) else {
        return false;
    };
    let mut last = Some(last);
    let max = _mm_set1_epi16(max as i16);
    let average = |a, b| {
        let x = average(a, b, forward);
        _mm_min_epu16(
            _mm_packus_epi32(_mm256_castsi256_si128(x), _mm256_extracti128_si256::<1>(x)),
            max,
        )
    };
    for (&a, &b) in first.iter().zip(&second).take(if W == 4 { 2 } else { 8 }) {
        let values = average(a, b);
        let Some(row) = rows.next().or_else(|| last.take()) else {
            break;
        };
        let next = if W == 4 {
            let Some(next) = rows.next().or_else(|| last.take()) else {
                break;
            };
            Some(next)
        } else {
            None
        };
        u16::store(t, row, next, values);
    }
    true
}

/// Calls `cell::<$w, H, V, C>` for the listed tap-pair counts of both
/// passes and both read forms, the in-place cell of `$common` pairs inline.
macro_rules! by_pairs {
    ($h:expr, $v:expr, $c:expr, $w:literal, $common:literal, $args:tt, [$($hh:literal $vv:literal),*]) => {
        #[allow(unreachable_patterns, reason = "the inline key is listed again")]
        match ($h, $v, $c) {
            ($common, $common, false) => cell::<$w, $common, $common, false> $args,
            $(($hh, $vv, false) => cell_out::<$w, $hh, $vv, false> $args,)*
            $(($hh, $vv, true) => cell_out::<$w, $hh, $vv, true> $args,)*
            _ => None,
        }
    };
}

/// The compound predictors of a `W`-square register cell of `source` read
/// by `placed`: one group per row of an 8x8 cell, per row pair of a 4x4 one
/// (the first two).
#[inline]
#[target_feature(enable = "avx2")]
fn cell_of<const W: usize>(t: Avx2, source: &[u16], placed: &Placed) -> Option<[__m256i; 8]> {
    let &(ref reads, clamped, horizontal, vertical) = placed;
    let pairs = |taps: Option<&Taps>| taps.map_or(0, |taps| taps.count);
    let (h, v) = (pairs(horizontal), pairs(vertical));
    if W == 4 {
        by_pairs!(h, v, clamped, 4, 2, (t, source, reads, (horizontal, vertical)), [0 0, 0 1, 0 2, 1 0, 1 1, 1 2, 2 0, 2 1, 2 2])
    } else {
        by_pairs!(h, v, clamped, 8, 4, (t, source, reads, (horizontal, vertical)),
            [0 0, 0 1, 0 2, 0 3, 0 4, 1 0, 1 1, 1 2, 1 3, 1 4, 2 0, 2 1, 2 2, 2 3, 2 4,
             3 0, 3 1, 3 2, 3 3, 3 4, 4 0, 4 1, 4 2, 4 3, 4 4])
    }
}

/// [`cell`] out of line.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn cell_out<const W: usize, const H: usize, const V: usize, const C: bool>(
    t: Avx2,
    source: &[u16],
    reads: &Reads,
    taps: (Option<&Taps>, Option<&Taps>),
) -> Option<[__m256i; 8]> {
    cell::<W, H, V, C>(t, source, reads, taps)
}

/// The `pshufb` pattern of a clamped row load and the lanes it takes from
/// the second eight samples: word `j` from sample `clamp(start + j, 0,
/// high)`.
#[inline]
#[target_feature(enable = "avx2")]
fn gather(reads: &Reads) -> (__m256i, __m256i) {
    let lanes = _mm256_setr_epi16(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
    let index = _mm256_add_epi16(lanes, _mm256_set1_epi16(reads.start));
    let index = _mm256_min_epi16(
        _mm256_max_epi16(index, _mm256_setzero_si256()),
        _mm256_set1_epi16(reads.high),
    );
    let low = _mm256_and_si256(index, _mm256_set1_epi16(7));
    let pattern = _mm256_add_epi16(
        _mm256_mullo_epi16(low, _mm256_set1_epi16(0x202)),
        _mm256_set1_epi16(0x100),
    );
    (pattern, _mm256_cmpgt_epi16(index, _mm256_set1_epi16(7)))
}

/// The predictors of [`cell_of`] of a cell whose passes have `H` and `V`
/// tap pairs, none for a skipped pass, its rows and columns clamped with
/// `C`: the window rows read once, each vertical tap pair one blend of two
/// of them.
#[inline]
#[target_feature(enable = "avx2")]
fn cell<const W: usize, const H: usize, const V: usize, const C: bool>(
    t: Avx2,
    source: &[u16],
    reads: &Reads,
    (horizontal, vertical): (Option<&Taps>, Option<&Taps>),
) -> Option<[__m256i; 8]> {
    let zero = _mm256_setzero_si256();
    let h_weights = horizontal.map_or([zero; 4], |taps| weights(taps));
    let v_weights = vertical.map_or([zero; 4], |taps| weights(taps));
    let rows = if V > 0 { W + 2 * V - 1 } else { W };
    let stride = if C {
        reads.stride
    } else {
        u32::try_from(reads.stride).ok()? as usize
    };
    let window = if C {
        source
    } else {
        source.get(reads.base..)?.get(..(rows - 1) * stride + 16)?
    };
    let at = |r: usize| {
        if C {
            let row = reads
                .top
                .saturating_add(r as i32)
                .max(reads.first)
                .min(reads.last);
            reads.base + row as usize * stride
        } else {
            r.min(rows - 1) * stride
        }
    };
    let (pattern, upper) = if C { gather(reads) } else { (zero, zero) };
    let narrow = |x: __m128i| {
        if C {
            _mm_shuffle_epi8(x, _mm256_castsi256_si128(pattern))
        } else {
            x
        }
    };
    let words8 = |r: usize| Some(narrow(load8(t, window.get(at(r)..)?, 0)?));
    let words16 = |r: usize| {
        let row = window.get(at(r)..)?.first_chunk::<16>()?;
        if !C {
            return Some((load8(t, row, 0)?, load8(t, row, 1)?, load8(t, row, 8)?));
        }
        let row = _mm256_set_m128i(load8(t, row, 8)?, load8(t, row, 0)?);
        let from = |lane| _mm256_shuffle_epi8(lane, pattern);
        let row = _mm256_blendv_epi8(
            from(_mm256_permute2x128_si256::<0x00>(row, row)),
            from(_mm256_permute2x128_si256::<0x11>(row, row)),
            upper,
        );
        let (low, high) = (
            _mm256_castsi256_si128(row),
            _mm256_extracti128_si256::<1>(row),
        );
        Some((low, _mm_alignr_epi8::<2>(high, low), high))
    };
    let finish = |x: __m256i| match (H, V) {
        (0, 0) => _mm256_slli_epi32::<4>(x),
        (_, 0) | (0, _) => round::<3>(x),
        _ => round::<7>(x),
    };
    let mut groups = [zero; 8];
    let mut lines = [zero; 16];
    if W == 8 {
        unroll!(r < rows, [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14] => {
            let line = if H > 0 {
                sums_of::<H>(words16(r)?, &h_weights)
            } else {
                _mm256_cvtepu16_epi32(words8(r)?)
            };
            if V == 0 {
                let line = if H > 0 { interleave(line) } else { line };
                groups[r] = finish(line);
            } else {
                let line = if H > 0 { round::<3>(line) } else { line };
                lines[r + 1] = line;
                lines[r] = pair(lines[r], line);
                if r + 1 >= 2 * V {
                    let y = r + 1 - 2 * V;
                    let sum = madds::<V>(&v_weights, |p| lines[y + 2 * p + 1]);
                    groups[y] = finish(if H > 0 { interleave(sum) } else { sum });
                }
            }
        });
        return Some(groups);
    }
    unroll!(m < if V == 0 { 2 } else { V + 2 }, [0 1 2 3] => {
        let rows = _mm256_set_m128i(words8(2 * m + 1)?, words8(2 * m)?);
        let line = if H > 0 {
            pairs_of::<H>(t, (rows, rows), &h_weights)
        } else {
            _mm256_cvtepu16_epi32(_mm_unpacklo_epi64(
                _mm256_castsi256_si128(rows),
                _mm256_extracti128_si256::<1>(rows),
            ))
        };
        if V == 0 {
            if m < 2 {
                groups[m] = finish(line);
            }
        } else {
            let line = if H > 0 { round::<3>(line) } else { line };
            if m > 0 {
                lines[m - 1] = pair(lines[m - 1], _mm256_permute2x128_si256::<0x21>(lines[m - 1], line));
            }
            lines[m] = line;
            if m >= V && m - V < 2 {
                let g = m - V;
                groups[g] = finish(madds::<V>(&v_weights, |p| lines[g + p]));
            }
        }
    });
    Some(groups)
}
