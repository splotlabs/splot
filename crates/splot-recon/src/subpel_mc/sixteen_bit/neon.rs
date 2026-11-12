// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unscaled § 7.13.3.18 interpolation of 16-bit storage in NEON lanes.
//!
//! A group is eight outputs: one row of an 8-column strip, or two rows of a
//! 4-column block. Each tap is one `smlal` by lane into `i32`, each rounding
//! one `srshl` or `rshrn`, and each clip one `sqxtun` and one `umin`. The
//! horizontal rows of a two-pass block stay in a register ring, and a 4x4 or
//! 8x8 compound cell in registers, its window read in place or by clamped
//! rows and `tbl`-gathered columns; the values match the portable kernels.

#![allow(
    clippy::inline_always,
    reason = "measured register-cell placement helpers"
)]

use super::super::block::{Block, Geometry, Sink, Taps, Window};
use super::super::cell::{Cell, Plane, Written};
use super::super::eight_bit::neon::{Reads, Sums, reads, slide, sums32, token};
use super::super::rows::{self, Average, Out, Prep, Put, rows_mut, store_pair};
use super::super::{CellPlacement, ReferencePlaneView};
use core::arch::aarch64::*;
use fearless_simd::{Bytes, Neon, Simd, SimdBase, SimdFrom, i16x8, i32x4, u16x8, u64x2};

/// Both references' samples, the output and its stride of one plane of a
/// cell program.
type ProgramPlane<'a, 'b> = ([&'b [u16]; 2], &'a mut [u16], usize);

/// Predicts `block` into `sink`, as [`super::predict`] admits.
#[inline]
pub(super) fn predict(block: &Block<'_, u16>, sink: &mut Sink<'_>) {
    if let Some(t) = token() {
        predict_lanes(t, block, sink);
    }
}

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of a 4x4 or 8x8
/// register cell pair.
#[inline]
pub(super) fn compound_cell(
    blocks: [&Block<'_, u16>; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) {
    if let Some(t) = token() {
        cell_pair(t, blocks, clip, output, stride);
    }
}

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of the 4x4 or 8x8
/// cell of `windows` within `bounds` read from `views`, or returns `false`
/// without writing when a clamped row cannot load its samples.
#[inline]
pub(super) fn compound_windows(
    views: [&ReferencePlaneView<'_, u16>; 2],
    windows: &[Window; 2],
    bounds: [[i32; 4]; 2],
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    let references = views.map(|view| (view.samples, view.geometry()));
    let windows = [&windows[0], &windows[1]];
    token().is_some_and(|t| windows_cell(t, references, windows, bounds, clip, output, stride))
}

/// Runs the cells of a program of `size`-square cells whose windows are
/// read in place or by clamped rows and columns into both `planes`, and
/// returns their bits.
#[inline]
pub(super) fn program(
    planes: &mut [Option<Plane<'_, '_, u16>>; 2],
    shape: (usize, usize),
    cells: &[Cell],
    clip: (i32, u16),
) -> Written {
    let (Some(t), Some((views, ..))) = (token(), &planes[0]) else {
        return [[0; 4]; 2];
    };
    let geometry = views.each_ref().map(ReferencePlaneView::geometry);
    let mut planes = planes.each_mut().map(|plane| {
        let (views, output, stride) = plane.as_mut()?;
        Some(([views[0].samples, views[1].samples], &mut **output, *stride))
    });
    run_cells(t, &mut planes, geometry, shape, cells, clip)
}

/// Calls `$f::<W, P>` for the run-time group width and pair count.
macro_rules! specialize {
    ($width:expr, $pairs:expr, $f:ident($($arg:expr),*)) => {
        match ($width, $pairs) {
            (4, 1) => $f::<4, 1, _>($($arg),*),
            (4, 2) => $f::<4, 2, _>($($arg),*),
            (4, 3) => $f::<4, 3, _>($($arg),*),
            (4, _) => $f::<4, 4, _>($($arg),*),
            (_, 1) => $f::<8, 1, _>($($arg),*),
            (_, 2) => $f::<8, 2, _>($($arg),*),
            (_, 3) => $f::<8, 3, _>($($arg),*),
            (_, _) => $f::<8, 4, _>($($arg),*),
        }
    };
}

/// Calls `$f::<W, H, V>` for the run-time group width and the tap-pair
/// counts of both passes.
macro_rules! specialize_pairs {
    ($width:expr, $h:expr, $v:expr, $f:ident $args:tt) => {
        specialize_pairs!(@ $width, $h, $v, $f $args,
            [1 1, 1 2, 1 3, 1 4, 2 1, 2 2, 2 3, 2 4, 3 1, 3 2, 3 3, 3 4, 4 1, 4 2, 4 3, 4 4])
    };
    (@ $width:expr, $h:expr, $v:expr, $f:ident $args:tt, [$($hh:literal $vv:literal),*]) => {
        match ($width == 4, $h.min(4), $v.min(4)) {
            $((true, $hh, $vv) => $f::<4, $hh, $vv, _> $args,)*
            $((false, $hh, $vv) => $f::<8, $hh, $vv, _> $args,)*
            _ => {}
        }
    };
}

fearless_simd::kernel!(
    #[inline]
    fn predict_lanes(t: Neon, block: &Block<'_, u16>, sink: &mut Sink<'_>) {
        let w = block.w;
        match sink {
            Sink::Put { output, stride } => {
                shapes(
                    t,
                    block,
                    &mut Put {
                        output,
                        stride: *stride,
                    },
                    255,
                );
            }
            Sink::PutWide {
                output,
                stride,
                max,
            } => {
                shapes(
                    t,
                    block,
                    &mut Put {
                        output,
                        stride: *stride,
                    },
                    *max,
                );
            }
            Sink::Prep { output, stride } => {
                shapes(
                    t,
                    block,
                    &mut Prep {
                        output,
                        stride: *stride,
                    },
                    0,
                );
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (forward, stride) = (*forward, *stride);
                shapes(
                    t,
                    block,
                    &mut Average {
                        pred0,
                        w,
                        forward,
                        output,
                        stride,
                    },
                    255,
                );
            }
            Sink::AverageWide {
                pred0,
                forward,
                output,
                stride,
                max,
            } => {
                let (forward, stride) = (*forward, *stride);
                shapes(
                    t,
                    block,
                    &mut Average {
                        pred0,
                        w,
                        forward,
                        output,
                        stride,
                    },
                    *max,
                );
            }
            Sink::PrepNarrow { .. } | Sink::AverageNarrow { .. } => {}
        }
    }
);

fearless_simd::kernel!(
    /// Stores the COMPOUND_AVERAGE of two `size`-square register cells, 4
    /// or 8, clipped to `max`.
    #[inline]
    fn cell_pair(
        t: Neon,
        blocks: [&Block<'_, u16>; 2],
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) {
        let (forward, max) = (clip.0, vdupq_n_u16(clip.1));
        let reads = blocks.map(|block| Reads {
            base: block.origin,
            stride: block.stride,
            ..Reads::default()
        });
        let [a, b] = blocks;
        let placed = [
            (reads[0], false, a.horizontal, a.vertical),
            (reads[1], false, b.horizontal, b.vertical),
        ];
        let cells = [(a.source, &placed[0]), (b.source, &placed[1])];
        if a.w == 4 {
            store_cell::<4>(t, cells, forward, max, output, stride);
        } else {
            store_cell::<8>(t, cells, forward, max, output, stride);
        }
    }
);

fearless_simd::kernel!(
    /// Runs the cells of a program of `size`-square cells, 4 or 8, whose
    /// windows the kernels read in place or by clamped rows and columns
    /// into both `planes` of reference geometry `geometry`, and returns
    /// their bits.
    #[inline]
    fn run_cells(
        t: Neon,
        planes: &mut [Option<ProgramPlane<'_, '_>>; 2],
        geometry: [Geometry; 2],
        shape: (usize, usize),
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written {
        let (filter, size) = shape;
        if size == 4 {
            run_program::<4>(t, planes, geometry, filter, cells, clip)
        } else {
            run_program::<8>(t, planes, geometry, filter, cells, clip)
        }
    }
);

fearless_simd::kernel!(
    /// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of the 4x4 or 8x8
    /// cell of `windows` within `bounds` read from the samples of each
    /// reference of its geometry, or returns `false` without writing when
    /// the kernels do not cover it.
    #[inline]
    fn windows_cell(
        t: Neon,
        references: [(&[u16], Geometry); 2],
        windows: [&Window; 2],
        bounds: [[i32; 4]; 2],
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) -> bool {
        let max = vdupq_n_u16(clip.1);
        let sources = references.map(|(source, _)| source);
        let geometry = references.map(|(_, geometry)| geometry);
        macro_rules! cell {
            ($w:literal) => {{
                let (Some(a), Some(b)) = (
                    place_window::<$w>(windows[0], bounds[0], geometry[0]),
                    place_window::<$w>(windows[1], bounds[1], geometry[1]),
                ) else {
                    return false;
                };
                store_placed::<$w>(t, sources, [&a, &b], clip.0, max, output, stride);
                true
            }};
        }
        if windows[0].w == 4 {
            cell!(4)
        } else {
            cell!(8)
        }
    }
);

/// Stores the COMPOUND_AVERAGE, clipped to `max`, of a `W`-square cell of
/// both references' [`Placed`] reads of `sources`.
#[inline]
#[target_feature(enable = "neon")]
fn store_placed<const W: usize>(
    t: Neon,
    sources: [&[u16]; 2],
    placed: [&Placed; 2],
    forward: i32,
    max: uint16x8_t,
    output: &mut [u16],
    stride: usize,
) {
    let still = |placed: &Placed| !placed.1 && placed.2.is_none() && placed.3.is_none();
    let [a, b] = placed;
    if still(a) && still(b) {
        let sources = [(sources[0], a.0.stride), (sources[1], b.0.stride)];
        let clip = (forward, vgetq_lane_u16::<0>(max));
        super::super::cell::fullpel_u16(sources, [a.0.base, b.0.base], W, clip, output, stride);
    } else {
        store_cell::<W>(
            t,
            [(sources[0], a), (sources[1], b)],
            forward,
            max,
            output,
            stride,
        );
    }
}

/// [`program`] of `W`-square cells: each reference's window derived once
/// per cell, both planes stored from it.
#[inline(never)]
#[target_feature(enable = "neon")]
fn run_program<const W: usize>(
    t: Neon,
    planes: &mut [Option<ProgramPlane<'_, '_>>; 2],
    geometry: [Geometry; 2],
    filter: usize,
    cells: &[Cell],
    (forward, max): (i32, u16),
) -> Written {
    let mut written = [[0u64; 4]; 2];
    let max = vdupq_n_u16(max);
    for (index, &([row, col], placements)) in cells.iter().enumerate() {
        let (Some(a), Some(b)) = (
            place::<W>(filter, placements[0], geometry[0]),
            place::<W>(filter, placements[1], geometry[1]),
        ) else {
            continue;
        };
        for (plane, written) in planes.iter_mut().zip(&mut written) {
            let Some((sources, output, stride)) = plane else {
                continue;
            };
            let at = row as usize * *stride + col as usize;
            let Some(output) = output
                .get_mut(at..)
                .filter(|output| (W - 1) * *stride + W <= output.len())
            else {
                continue;
            };
            store_placed::<W>(t, *sources, [&a, &b], forward, max, output, *stride);
            written[index / 64 % 4] |= 1 << (index % 64);
        }
    }
    written
}

/// The [`Placed`] reads of the `W`-square cell of `filter` at `placement`
/// in a plane of `geometry`, `None` when the kernels do not cover it.
#[inline(always)]
fn place<const W: usize>(
    filter: usize,
    (start, bounds): CellPlacement,
    geometry: Geometry,
) -> Option<Placed> {
    place_window::<W>(
        &Window::of_filters([filter; 2], W, W, start),
        bounds,
        geometry,
    )
}

/// [`place`] of the window of a `W`-square cell.
#[inline(always)]
fn place_window<const W: usize>(
    window: &Window,
    bounds: [i32; 4],
    geometry: Geometry,
) -> Option<Placed> {
    let small = |taps: Option<&Taps>| W == 8 || pairs(taps) <= 2;
    if !small(window.horizontal) || !small(window.vertical) {
        return None;
    }
    let (reads, clamped) = reads(window, geometry, bounds)?;
    Some((reads, clamped, window.horizontal, window.vertical))
}

/// The tap-pair count of a pass, zero for a skipped pass.
#[inline(always)]
fn pairs(taps: Option<&Taps>) -> usize {
    taps.map_or(0, |taps| taps.count)
}

/// [`weights`] of a pass, zero for a skipped pass.
#[inline]
#[target_feature(enable = "neon")]
fn pass_weights(t: Neon, taps: Option<&Taps>) -> int16x8_t {
    taps.map_or(vdupq_n_s16(0), |taps| weights(t, taps))
}

/// Lane indices `0..8`.
const LANES: [i16; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Calls `$f::<$w, H, V, C>` for the listed tap-pair counts of both passes
/// and both read forms.
macro_rules! by_pairs {
    ($h:expr, $v:expr, $c:expr, $f:ident::<$w:literal> $args:tt, [$($hh:literal $vv:literal),*]) => {
        match ($h, $v, $c) {
            $(($hh, $vv, false) => $f::<$w, $hh, $vv, false> $args,)*
            $(($hh, $vv, true) => $f::<$w, $hh, $vv, true> $args,)*
            _ => [(vdupq_n_s32(0), vdupq_n_s32(0)); $w],
        }
    };
}

/// A reference of a register cell: its [`Reads`], whether they are
/// clamped, and the taps of both passes.
type Placed = (Reads, bool, Option<&'static Taps>, Option<&'static Taps>);

/// The samples of a reference of a register cell and its [`Placed`] reads.
type CellReads<'a> = (&'a [u16], &'a Placed);

/// [`cell`] of a 4x4 cell, whose passes have at most two tap pairs.
#[inline]
#[target_feature(enable = "neon")]
fn cell_of4(t: Neon, (source, placed): CellReads<'_>) -> [Sums; 4] {
    let (reads, clamped, horizontal, vertical) = placed;
    let (clamped, horizontal, vertical) = (*clamped, *horizontal, *vertical);
    let weights = (pass_weights(t, horizontal), pass_weights(t, vertical));
    by_pairs!(pairs(horizontal), pairs(vertical), clamped, cell::<4>(t, source, reads, weights),
        [0 0, 0 1, 0 2, 1 0, 1 1, 1 2, 2 0, 2 1, 2 2])
}

/// [`cell`] of an 8x8 cell.
#[inline]
#[target_feature(enable = "neon")]
fn cell_of8(t: Neon, (source, placed): CellReads<'_>) -> [Sums; 8] {
    let (reads, clamped, horizontal, vertical) = placed;
    let (clamped, horizontal, vertical) = (*clamped, *horizontal, *vertical);
    let weights = (pass_weights(t, horizontal), pass_weights(t, vertical));
    by_pairs!(pairs(horizontal), pairs(vertical), clamped, cell::<8>(t, source, reads, weights),
        [0 0, 0 1, 0 2, 0 3, 0 4, 1 0, 1 1, 1 2, 1 3, 1 4, 2 0, 2 1, 2 2, 2 3, 2 4,
         3 0, 3 1, 3 2, 3 3, 3 4, 4 0, 4 1, 4 2, 4 3, 4 4])
}

/// The compound predictors of a `W`-square register cell whose passes have
/// `H` and `V` tap pairs, none for a skipped pass, read by `reads`, clamped
/// with `C`: one group per row of an 8x8 cell, per row pair of a 4x4 one
/// (the first two).
#[inline(never)]
#[target_feature(enable = "neon")]
fn cell<const W: usize, const H: usize, const V: usize, const C: bool>(
    t: Neon,
    source: &[u16],
    reads: &Reads,
    (h_weights, v_weights): (int16x8_t, int16x8_t),
) -> [Sums; W] {
    let mut groups = [(vdupq_n_s32(0), vdupq_n_s32(0)); W];
    let Reads {
        base,
        stride,
        top,
        first,
        last,
        columns: [start, low, high],
    } = *reads;
    let gather = [0, 8].map(|offset| {
        let lanes = vaddq_s16(
            vdupq_n_s16(start + offset),
            i16x8::load_array(t, LANES).into(),
        );
        let index = vminq_s16(vmaxq_s16(lanes, vdupq_n_s16(low)), vdupq_n_s16(high));
        vreinterpretq_u8_s16(vmlaq_n_s16(vdupq_n_s16(0x100), index, 0x202))
    });
    let at = |row: usize| {
        if C {
            base + top.saturating_add(row as i32).max(first).min(last) as usize * stride
        } else {
            base + row * stride
        }
    };
    let gather8 = |x: int16x8_t| {
        if C {
            vreinterpretq_s16_u8(vqtbl1q_u8(vreinterpretq_u8_s16(x), gather[0]))
        } else {
            x
        }
    };
    let rows = if V > 0 { W + 2 * V - 1 } else { W };
    if W == 8 {
        let mut lines = [vdupq_n_s16(0); 15];
        unroll!(row < rows, [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14] => {
            if H > 0 {
                let (mut a, mut b) = load16(t, source, at(row));
                if C {
                    let table = uint8x16x2_t(vreinterpretq_u8_s16(a), vreinterpretq_u8_s16(b));
                    a = vreinterpretq_s16_u8(vqtbl2q_u8(table, gather[0]));
                    b = vreinterpretq_s16_u8(vqtbl2q_u8(table, gather[1]));
                }
                let (low, high) = sums32::<H>(h_weights, |k| slide(a, b, k));
                if V > 0 {
                    lines[row] = vrshrn_high_n_s32::<3>(vrshrn_n_s32::<3>(low), high);
                } else {
                    groups[row] = (vrshrq_n_s32::<3>(low), vrshrq_n_s32::<3>(high));
                }
            } else {
                lines[row] = gather8(load(t, source, at(row)));
            }
        });
        unroll!(row < 8, [0 1 2 3 4 5 6 7] => {
            let x = lines[row];
            let line = |k: usize| lines[row + k];
            match (H, V) {
                (0, 0) => groups[row] = (vshll_n_s16::<4>(vget_low_s16(x)), vshll_high_n_s16::<4>(x)),
                (0, _) => {
                    let (low, high) = sums32::<V>(v_weights, line);
                    groups[row] = (vrshrq_n_s32::<3>(low), vrshrq_n_s32::<3>(high));
                }
                (_, 0) => {}
                _ => {
                    let (low, high) = sums32::<V>(v_weights, line);
                    groups[row] = (vrshrq_n_s32::<7>(low), vrshrq_n_s32::<7>(high));
                }
            }
        });
        return groups;
    }
    let mut lines = [vdup_n_s16(0); 7];
    let mut first = [vdupq_n_s32(0); 4];
    unroll!(row < rows, [0 1 2 3 4 5 6] => {
        let a = gather8(load(t, source, at(row)));
        if H > 0 {
            let sums = sums4::<H>(h_weights, |k| vget_low_s16(slide(a, a, k)));
            if V > 0 {
                lines[row] = vrshrn_n_s32::<3>(sums);
            } else {
                first[row] = vrshrq_n_s32::<3>(sums);
            }
        } else {
            lines[row] = vget_low_s16(a);
        }
    });
    unroll!(row < 4, [0 1 2 3] => {
        let line = |k: usize| lines[row + k];
        first[row] = match (H, V) {
            (0, 0) => vshll_n_s16::<4>(lines[row]),
            (0, _) => vrshrq_n_s32::<3>(sums4::<V>(v_weights, line)),
            (_, 0) => first[row],
            _ => vrshrq_n_s32::<7>(sums4::<V>(v_weights, line)),
        };
    });
    groups[0] = (first[0], first[1]);
    groups[1] = (first[2], first[3]);
    groups
}

/// Stores the COMPOUND_AVERAGE, clipped to `max`, of the `W`-square
/// register cells of both references.
#[inline]
#[target_feature(enable = "neon")]
fn store_cell<const W: usize>(
    t: Neon,
    cells: [CellReads<'_>; 2],
    forward: i32,
    max: uint16x8_t,
    output: &mut [u16],
    stride: usize,
) {
    if W == 4 {
        let (first, second) = (cell_of4(t, cells[0]), cell_of4(t, cells[1]));
        blend_cell::<4, 2>(t, &first, &second, forward, max, output, stride);
    } else {
        let (first, second) = (cell_of8(t, cells[0]), cell_of8(t, cells[1]));
        blend_cell::<8, 8>(t, &first, &second, forward, max, output, stride);
    }
}

/// Stores the COMPOUND_AVERAGE, clipped to `max`, of the `G` groups of two
/// `W`-square register cells.
#[inline]
#[target_feature(enable = "neon")]
fn blend_cell<const W: usize, const G: usize>(
    t: Neon,
    first: &[Sums; W],
    second: &[Sums; W],
    forward: i32,
    max: uint16x8_t,
    output: &mut [u16],
    stride: usize,
) {
    let Some((mut rows, last)) = rows_mut(output, 0, stride, W, W) else {
        return;
    };
    let mut last = Some(last);
    let mut put = |values| {
        let a = rows.next().or_else(|| last.take());
        let b = if W == 4 {
            rows.next().or_else(|| last.take())
        } else {
            None
        };
        if let Some(a) = a
            && (W == 8 || b.is_some())
        {
            u16::store(t, a, b, vminq_u16(values, max));
        }
    };
    if forward == 8 {
        unroll!(group < G, [0 1 2 3 4 5 6 7] => {
            put(blend(first[group], second[group], 8));
        });
    } else {
        unroll!(group < G, [0 1 2 3 4 5 6 7] => {
            put(blend(first[group], second[group], forward));
        });
    }
}

/// Eight samples from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load(t: Neon, samples: &[u16], at: usize) -> int16x8_t {
    samples
        .get(at..)
        .and_then(|row| row8(t, row))
        .unwrap_or(vdupq_n_s16(0))
}

/// Sixteen samples from `at` as two vectors, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load16(t: Neon, samples: &[u16], at: usize) -> (int16x8_t, int16x8_t) {
    let zero = vdupq_n_s16(0);
    samples
        .get(at..)
        .and_then(|row| row16(t, row))
        .unwrap_or((zero, zero))
}

/// The eight taps of `taps`, zero past its pairs: [`Taps::full`] as lanes.
#[inline]
#[target_feature(enable = "neon")]
fn weights(t: Neon, taps: &Taps) -> int16x8_t {
    vreinterpretq_s16_s32(i32x4::load_array_ref(t, &taps.full).into())
}

/// [`sums32`] of four lanes.
#[inline]
#[target_feature(enable = "neon")]
fn sums4<const P: usize>(weights: int16x8_t, mut row: impl FnMut(usize) -> int16x4_t) -> int32x4_t {
    let mut sum = vmull_laneq_s16::<0>(row(0), weights);
    sum = vmlal_laneq_s16::<1>(sum, row(1), weights);
    if P >= 2 {
        sum = vmlal_laneq_s16::<2>(sum, row(2), weights);
        sum = vmlal_laneq_s16::<3>(sum, row(3), weights);
    }
    if P >= 3 {
        sum = vmlal_laneq_s16::<4>(sum, row(4), weights);
        sum = vmlal_laneq_s16::<5>(sum, row(5), weights);
    }
    if P >= 4 {
        sum = vmlal_laneq_s16::<6>(sum, row(6), weights);
        sum = vmlal_laneq_s16::<7>(sum, row(7), weights);
    }
    sum
}

#[inline]
#[target_feature(enable = "neon")]
fn round((low, high): Sums, shift: int32x4_t) -> Sums {
    (vrshlq_s32(low, shift), vrshlq_s32(high, shift))
}

/// The eighths of the two taps of a BILINEAR pass, whose taps are
/// multiples of 8 that sum to 128.
#[inline]
#[target_feature(enable = "neon")]
fn eighths(weights: int16x8_t) -> uint16x8_t {
    vshrq_n_u16::<3>(vreinterpretq_u16_s16(weights))
}

/// `a * t0 + b * t1` of the [`eighths`] `t0` and `t1` of a BILINEAR pass:
/// the pass sum over 8, exact in `u16` lanes for samples of up to 12 bits.
#[inline]
#[target_feature(enable = "neon")]
fn bilinear(a: int16x8_t, b: int16x8_t, eighths: uint16x8_t) -> uint16x8_t {
    let first = vmulq_laneq_u16::<0>(vreinterpretq_u16_s16(a), eighths);
    vmlaq_laneq_u16::<1>(first, vreinterpretq_u16_s16(b), eighths)
}

/// The COMPOUND_AVERAGE of the first predictors `first` and `sums`,
/// saturated to `u16` before `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn blend((first, second): Sums, (low, high): Sums, forward: i32) -> uint16x8_t {
    if forward == 8 {
        let low = vqrshrun_n_s32::<5>(vaddq_s32(first, low));
        return vqrshrun_high_n_s32::<5>(low, vaddq_s32(second, high));
    }
    let blend = |pred, value| vmlaq_n_s32(vmulq_n_s32(pred, forward), value, 16 - forward);
    vqrshrun_high_n_s32::<8>(vqrshrun_n_s32::<8>(blend(first, low)), blend(second, high))
}

/// Runs the kernel of the shape of `block` into `out`, its samples at
/// most `max`.
#[inline]
#[target_feature(enable = "neon")]
fn shapes(t: Neon, block: &Block<'_, u16>, out: &mut impl WordOut, max: u16) {
    let width = if block.w == 4 { 4 } else { 8 };
    match (block.horizontal, block.vertical) {
        (Some(taps), None) => specialize!(width, taps.count, horizontal(t, block, taps, out, max)),
        (None, Some(taps)) => specialize!(width, taps.count, vertical(t, block, taps, out, max)),
        (Some(h_taps), Some(v_taps)) => specialize_pairs!(
            width,
            h_taps.count,
            v_taps.count,
            two_pass(t, block, (h_taps, v_taps), out, max)
        ),
        (None, None) => {}
    }
}

/// What a kernel stores from word lanes: eight outputs of row `a`, or four
/// of each of rows `a` and `b`, as `Clip1` samples ([`Self::words`]) or
/// compound predictors ([`Self::sums`]).
trait WordOut: Out {
    /// The first predictors of the outputs.
    fn first(t: Neon, _: &Self::Row<'_>, _: Option<&Self::Row<'_>>) -> Sums {
        (i32x4::splat(t, 0).into(), i32x4::splat(t, 0).into())
    }
    /// Stores samples.
    fn words(_: Neon, _: Self::Row<'_>, _: Option<Self::Row<'_>>, _: uint16x8_t) {}
    /// Stores compound predictors.
    fn sums(_: Neon, _: Self::Row<'_>, _: Option<Self::Row<'_>>, _: Sums) {}
}

/// Sample storage of word lanes whose values fit it.
trait Word: Copy {
    /// Stores eight samples into row `a`, or lanes `0..4` into `a` and
    /// `4..8` into `b`.
    fn store(t: Neon, a: &mut [Self], b: Option<&mut [Self]>, values: uint16x8_t);
}

impl Word for u8 {
    #[inline(always)]
    fn store(t: Neon, a: &mut [u8], b: Option<&mut [u8]>, values: uint16x8_t) {
        let values = u16x8::simd_from(t, values);
        let bytes = t.narrow_u16x8(values, values).bitcast::<u64x2<Neon>>()[0].to_le_bytes();
        match (b, a.first_chunk_mut::<8>()) {
            (Some(b), _) => store_pair(a, b, bytes),
            (None, Some(slot)) => *slot = bytes,
            (None, None) => {}
        }
    }
}

impl Word for u16 {
    #[inline(always)]
    fn store(t: Neon, a: &mut [u16], b: Option<&mut [u16]>, values: uint16x8_t) {
        let values = u16x8::<Neon>::simd_from(t, values);
        match (b, a.first_chunk_mut::<8>()) {
            (Some(b), _) => store_pair(a, b, values.to_array()),
            (None, Some(slot)) => values.store_array(slot),
            (None, None) => {}
        }
    }
}

impl<T: Word> WordOut for Put<'_, T> {
    #[inline(always)]
    fn words(t: Neon, a: &mut [T], b: Option<&mut [T]>, values: uint16x8_t) {
        T::store(t, a, b, values);
    }
}

impl WordOut for Prep<'_, i32> {
    #[inline(always)]
    fn sums(t: Neon, a: &mut [i32], b: Option<&mut [i32]>, (low, high): Sums) {
        let store = |slot: Option<&mut [i32; 4]>, lanes| {
            if let Some(slot) = slot {
                i32x4::<Neon>::simd_from(t, lanes).store_array(slot);
            }
        };
        let (a, b) = match b {
            Some(b) => (a.first_chunk_mut::<4>(), b.first_chunk_mut::<4>()),
            None => match a.split_first_chunk_mut::<4>() {
                Some((a, rest)) => (Some(a), rest.first_chunk_mut::<4>()),
                None => (None, None),
            },
        };
        store(a, low);
        store(b, high);
    }
}

impl<T: Word> WordOut for Average<'_, i32, T> {
    #[inline(always)]
    fn first(t: Neon, (a, _): &(&[i32], &mut [T]), b: Option<&(&[i32], &mut [T])>) -> Sums {
        let lanes = |lanes: Option<&[i32; 4]>| {
            lanes
                .map_or(i32x4::splat(t, 0), |lanes| i32x4::load_array_ref(t, lanes))
                .into()
        };
        let high = match b {
            Some((b, _)) => b.first_chunk(),
            None => a.get(4..).and_then(<[i32]>::first_chunk),
        };
        (lanes(a.first_chunk()), lanes(high))
    }
    #[inline(always)]
    fn words(
        t: Neon,
        (_, a): (&[i32], &mut [T]),
        b: Option<(&[i32], &mut [T])>,
        values: uint16x8_t,
    ) {
        T::store(t, a, b.map(|(_, b)| b), values);
    }
}

/// Stores a group of `O` into row `a`, or rows `a` and `b`: the put
/// `values` clipped to `max`, else the compound predictors `sums`, averaged
/// with the first ones when `O` averages.
#[inline]
#[target_feature(enable = "neon")]
fn store<O: WordOut>(
    t: Neon,
    (forward, max): (i32, uint16x8_t),
    (a, b): (O::Row<'_>, Option<O::Row<'_>>),
    values: uint16x8_t,
    sums: Sums,
) {
    if O::PUT {
        O::words(t, a, b, vminq_u16(values, max));
    } else if O::AVERAGE {
        let first = O::first(t, &a, b.as_ref());
        O::words(t, a, b, vminq_u16(blend(first, sums, forward), max));
    } else {
        O::sums(t, a, b, sums);
    }
}

/// The forward weight of `out` and the splat largest sample `max`.
#[inline]
#[target_feature(enable = "neon")]
fn clip(out: &impl Out, max: u16) -> (i32, uint16x8_t) {
    (out.forward(), vdupq_n_u16(max))
}

/// `Round2(sums, S)` saturated to `u16`.
#[inline]
#[target_feature(enable = "neon")]
fn narrow<const S: i32>((low, high): Sums) -> uint16x8_t {
    vqrshrun_high_n_s32::<S>(vqrshrun_n_s32::<S>(low), high)
}

/// The eight `u16` values `s` in `i32` lanes.
#[inline]
#[target_feature(enable = "neon")]
fn widen(s: uint16x8_t) -> Sums {
    (
        vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(s))),
        vreinterpretq_s32_u32(vmovl_high_u16(s)),
    )
}

/// The first eight samples of `row`.
#[inline]
#[target_feature(enable = "neon")]
fn row8(t: Neon, row: &[u16]) -> Option<int16x8_t> {
    Some(vreinterpretq_s16_u16(
        u16x8::load_array_ref(t, row.first_chunk::<8>()?).into(),
    ))
}

/// The first sixteen samples of `row` as two vectors.
#[inline]
#[target_feature(enable = "neon")]
fn row16(t: Neon, row: &[u16]) -> Option<(int16x8_t, int16x8_t)> {
    let [a, b] = row.first_chunk::<16>()?.as_chunks::<8>().0 else {
        return None;
    };
    let load = |lanes| vreinterpretq_s16_u16(u16x8::load_array_ref(t, lanes).into());
    Some((load(a), load(b)))
}

/// The vertical sums of `P` tap pairs of `tap`, for a BILINEAR pass the
/// widened [`bilinear`] sum.
#[inline]
#[target_feature(enable = "neon")]
fn vsum<const P: usize>(
    weights: int16x8_t,
    eighths: uint16x8_t,
    tap: impl Fn(usize) -> int16x8_t,
) -> Sums {
    if P == 1 {
        widen(bilinear(tap(0), tap(1), eighths))
    } else {
        sums32::<P>(weights, tap)
    }
}

/// Tap `k` of a vertical pass of `V` tap pairs over a strip's register
/// `ring` at `phase` and the `next` window row.
#[inline]
#[target_feature(enable = "neon")]
fn ring_tap<const V: usize>(
    phase: usize,
    ring: &[int16x8_t; 8],
    next: int16x8_t,
    k: usize,
) -> int16x8_t {
    if k + 1 < 2 * V {
        ring[(phase + k) % (2 * V)]
    } else {
        next
    }
}

/// Tap `k` of a vertical pass of `V` tap pairs over the `ring` of row
/// pairs of a 4-column block and the `next` pair.
#[inline]
#[target_feature(enable = "neon")]
fn pair_tap<const V: usize>(ring: &[int16x8_t; 4], next: int16x8_t, k: usize) -> int16x8_t {
    let pair = |k: usize| if k < V { ring[k] } else { next };
    if k.is_multiple_of(2) {
        pair(k / 2)
    } else {
        vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1))
    }
}

/// The horizontal pass of `block`, `P` tap pairs, in 8-column strips or
/// 4-column row pairs: `Round2(sum, InterRound0)`, for a BILINEAR pass its
/// exact [`bilinear`] sum, and for a put `Round2` of that by 4.
#[inline(never)]
#[target_feature(enable = "neon")]
fn horizontal<const W: usize, const P: usize, O: WordOut>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    out: &mut O,
    max: u16,
) {
    let (weights, clip) = (weights(t, taps), clip(out, max));
    let (eighths, round0) = (eighths(weights), vdupq_n_s32(-3));
    let none = (vdupq_n_s32(0), vdupq_n_s32(0));
    if W == 4 {
        let row_sums = |row: &[u16]| -> Option<int32x4_t> {
            let (a, b) = if P > 2 {
                row16(t, row)?
            } else {
                row8(t, row).map(|a| (a, a))?
            };
            Some(vrshlq_s32(
                sums4::<P>(weights, |k| vget_low_s16(slide(a, b, k))),
                round0,
            ))
        };
        let line = |a: &[u16], b: &[u16]| -> Option<Sums> {
            if P == 1 {
                let (a, b) = (row8(t, a)?, row8(t, b)?);
                let first = vcombine_s16(vget_low_s16(a), vget_low_s16(b));
                let second = vcombine_s16(
                    vget_low_s16(vextq_s16::<1>(a, a)),
                    vget_low_s16(vextq_s16::<1>(b, b)),
                );
                return Some(widen(bilinear(first, second, eighths)));
            }
            Some((row_sums(a)?, row_sums(b)?))
        };
        let reach = if P > 2 { 16 } else { 8 };
        let emit = |a: O::Row<'_>, b: O::Row<'_>, x: Sums| {
            store::<O>(t, clip, (a, Some(b)), narrow::<4>(x), x);
        };
        rows::pairs::<u16, _, _, O, 0>(block, out, reach, none, line, |_, next| next, emit);
        return;
    }
    let line = |row: &[u16]| -> Option<Sums> {
        let (a, b) = row16(t, row)?;
        Some(if P == 1 {
            widen(bilinear(a, slide(a, b, 1), eighths))
        } else {
            round(sums32::<P>(weights, |k| slide(a, b, k)), round0)
        })
    };
    let emit = |r: O::Row<'_>, x: Sums| store::<O>(t, clip, (r, None), narrow::<4>(x), x);
    rows::strips::<u16, _, _, O, 0>(block, out, (8, 16), none, line, |_, _, next| next, emit);
}

/// The vertical pass of `block`, `P` tap pairs, in 8-column strips or
/// 4-column row pairs, the window rows of a strip in a register ring: the
/// compound predictor `Round2(sum, 3)`, a put `Round2(sum, 7)`, and for a
/// BILINEAR pass the [`bilinear`] sum and its `Round2` by 4.
#[inline(never)]
#[target_feature(enable = "neon")]
fn vertical<const W: usize, const P: usize, O: WordOut>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    out: &mut O,
    max: u16,
) {
    let (weights, clip) = (weights(t, taps), clip(out, max));
    let (eighths, shift) = (eighths(weights), vdupq_n_s32(-3));
    let finish = |y: Sums| {
        if P == 1 {
            (narrow::<4>(y), y)
        } else {
            (narrow::<7>(y), round(y, shift))
        }
    };
    if W == 4 {
        let line = |a: &[u16], b: &[u16]| {
            Some(vcombine_s16(
                vget_low_s16(row8(t, a)?),
                vget_low_s16(row8(t, b)?),
            ))
        };
        let vertical = |ring: &[int16x8_t; 4], next| {
            vsum::<P>(weights, eighths, |k| pair_tap::<P>(ring, next, k))
        };
        let emit = |a: O::Row<'_>, b: O::Row<'_>, y: Sums| {
            let (values, sums) = finish(y);
            store::<O>(t, clip, (a, Some(b)), values, sums);
        };
        rows::pairs::<u16, _, _, O, P>(block, out, 8, vdupq_n_s16(0), line, vertical, emit);
        return;
    }
    let vertical = |phase, ring: &[int16x8_t; 8], next| {
        vsum::<P>(weights, eighths, |k| ring_tap::<P>(phase, ring, next, k))
    };
    let emit = |r: O::Row<'_>, y: Sums| {
        let (values, sums) = finish(y);
        store::<O>(t, clip, (r, None), values, sums);
    };
    let line = |row: &[u16]| row8(t, row);
    rows::strips::<u16, _, _, O, P>(block, out, (8, 8), vdupq_n_s16(0), line, vertical, emit);
}

/// Both passes of `block`, `H` and `V` tap pairs, in 8-column strips or
/// 4-column row pairs: the `Round2(sum, InterRound0)` horizontal rows in a
/// register ring, the vertical sum `Round2`d by 7 for a compound predictor
/// and by 11 for a put.
#[inline(never)]
#[target_feature(enable = "neon")]
fn two_pass<const W: usize, const H: usize, const V: usize, O: WordOut>(
    t: Neon,
    block: &Block<'_, u16>,
    (h_taps, v_taps): (&Taps, &Taps),
    out: &mut O,
    max: u16,
) {
    let (h_weights, v_weights) = (weights(t, h_taps), weights(t, v_taps));
    if H == 1 && V == 1 {
        bilinear_two_pass::<W, O>(t, block, (eighths(h_weights), eighths(v_weights)), out, max);
        return;
    }
    let (clip, shift) = (clip(out, max), vdupq_n_s32(-7));
    let narrow3 = |(low, high): Sums| vrshrn_high_n_s32::<3>(vrshrn_n_s32::<3>(low), high);
    if W == 4 {
        let row_sums = |row: &[u16]| -> Option<int32x4_t> {
            let (a, b) = if H > 2 {
                row16(t, row)?
            } else {
                row8(t, row).map(|a| (a, a))?
            };
            Some(sums4::<H>(h_weights, |k| vget_low_s16(slide(a, b, k))))
        };
        let line = |a: &[u16], b: &[u16]| Some(narrow3((row_sums(a)?, row_sums(b)?)));
        let vertical =
            |ring: &[int16x8_t; 4], next| sums32::<V>(v_weights, |k| pair_tap::<V>(ring, next, k));
        let emit = |a: O::Row<'_>, b: O::Row<'_>, y: Sums| {
            store::<O>(t, clip, (a, Some(b)), narrow::<11>(y), round(y, shift));
        };
        let reach = if H > 2 { 16 } else { 8 };
        rows::pairs::<u16, _, _, O, V>(block, out, reach, vdupq_n_s16(0), line, vertical, emit);
        return;
    }
    let line = |row: &[u16]| {
        let (a, b) = row16(t, row)?;
        Some(narrow3(sums32::<H>(h_weights, |k| slide(a, b, k))))
    };
    let vertical = |phase, ring: &[int16x8_t; 8], next| {
        sums32::<V>(v_weights, |k| ring_tap::<V>(phase, ring, next, k))
    };
    let emit =
        |r: O::Row<'_>, y: Sums| store::<O>(t, clip, (r, None), narrow::<11>(y), round(y, shift));
    rows::strips::<u16, _, _, O, V>(block, out, (8, 16), vdupq_n_s16(0), line, vertical, emit);
}

/// [`two_pass`] of a BILINEAR block: each horizontal row is its exact
/// `Round2(sum, InterRound0)` [`bilinear`] sum, read as the `i16` stage of
/// the general pass, and the vertical sum of the eighths `Round2`s by 8 for
/// a put and by 4 for a compound predictor.
#[inline(never)]
#[target_feature(enable = "neon")]
fn bilinear_two_pass<const W: usize, O: WordOut>(
    t: Neon,
    block: &Block<'_, u16>,
    (h_eighths, v_eighths): (uint16x8_t, uint16x8_t),
    out: &mut O,
    max: u16,
) {
    let (v, clip, shift) = (
        vreinterpretq_s16_u16(v_eighths),
        clip(out, max),
        vdupq_n_s32(-4),
    );
    let sum = |a: int16x8_t, b: int16x8_t| -> Sums {
        (
            vmlal_laneq_s16::<1>(vmull_laneq_s16::<0>(vget_low_s16(a), v), vget_low_s16(b), v),
            vmlal_high_laneq_s16::<1>(vmull_high_laneq_s16::<0>(a, v), b, v),
        )
    };
    let bilinear_row =
        |a: int16x8_t, b: int16x8_t| vreinterpretq_s16_u16(bilinear(a, slide(a, b, 1), h_eighths));
    if W == 4 {
        let half = |row: &[u16]| {
            let a = row8(t, row)?;
            Some(vget_low_s16(bilinear_row(a, a)))
        };
        let line = |a: &[u16], b: &[u16]| Some(vcombine_s16(half(a)?, half(b)?));
        let vertical = |ring: &[int16x8_t; 4], next| sum(ring[0], vextq_s16::<4>(ring[0], next));
        let emit = |a: O::Row<'_>, b: O::Row<'_>, y: Sums| {
            store::<O>(t, clip, (a, Some(b)), narrow::<8>(y), round(y, shift));
        };
        rows::pairs::<u16, _, _, O, 1>(block, out, 8, vdupq_n_s16(0), line, vertical, emit);
        return;
    }
    let line = |row: &[u16]| {
        let (a, b) = row16(t, row)?;
        Some(bilinear_row(a, b))
    };
    let vertical = |phase: usize, ring: &[int16x8_t; 8], next| sum(ring[phase % 2], next);
    let emit =
        |r: O::Row<'_>, y: Sums| store::<O>(t, clip, (r, None), narrow::<8>(y), round(y, shift));
    rows::strips::<u16, _, _, O, 1>(block, out, (8, 16), vdupq_n_s16(0), line, vertical, emit);
}
