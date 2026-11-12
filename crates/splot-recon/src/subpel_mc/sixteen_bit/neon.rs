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

use super::super::CellPlacement;
use super::super::block::{Block, Geometry, Sink, Taps, Window};
use super::super::cell::{Cell, Written};
use core::arch::aarch64::*;
use fearless_simd::{Neon, SimdBase, SimdFrom, i16x8, i32x4, u16x8};

/// Both references' samples, the output and its stride of one plane of a
/// cell program.
pub(super) type ProgramPlane<'a, 'b> = ([&'b [u16]; 2], &'a mut [u16], usize);

/// The `i32` sums of a group: outputs `0..4` and `4..8`.
type Sums = (int32x4_t, int32x4_t);

/// The first columns of the 8-column strips of a block at least 8 wide:
/// the last strip of a width that is not a multiple of 8 overlaps the one
/// before it and stores the same values again.
#[inline]
fn strips(w: usize) -> impl Iterator<Item = usize> {
    (0..w).step_by(8).map(move |col| col.min(w - 8))
}

/// Calls `$f::<W, P>` for the run-time group width and pair count.
macro_rules! specialize {
    ($width:expr, $pairs:expr, $f:ident($($arg:expr),*)) => {
        match ($width, $pairs) {
            (4, 1) => $f::<4, 1>($($arg),*),
            (4, 2) => $f::<4, 2>($($arg),*),
            (4, 3) => $f::<4, 3>($($arg),*),
            (4, _) => $f::<4, 4>($($arg),*),
            (_, 1) => $f::<8, 1>($($arg),*),
            (_, 2) => $f::<8, 2>($($arg),*),
            (_, 3) => $f::<8, 3>($($arg),*),
            (_, _) => $f::<8, 4>($($arg),*),
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
            $((true, $hh, $vv) => $f::<4, $hh, $vv> $args,)*
            $((false, $hh, $vv) => $f::<8, $hh, $vv> $args,)*
            _ => {}
        }
    };
}

fearless_simd::kernel!(
    #[inline]
    pub(super) fn predict(
        t: Neon,
        block: &Block<'_, u16>,
        sink: &mut Sink<'_>,
        _stage: &mut [i16],
    ) {
        let w = block.w;
        match sink {
            Sink::Put { output, stride } => {
                let (output, stride, max) = (&mut **output, *stride, vdupq_n_u16(255));
                put(t, block, &mut move |width, row, col, values| {
                    let values = vminq_u16(values, max);
                    store_bytes(output, width, row * stride + col, stride, values);
                });
            }
            Sink::PutWide {
                output,
                stride,
                max,
            } => {
                let (output, stride, max) = (&mut **output, *stride, vdupq_n_u16(*max));
                put(t, block, &mut move |width, row, col, values| {
                    let values = vminq_u16(values, max);
                    store_words(t, output, width, row * stride + col, stride, values);
                });
            }
            Sink::Prep { output, stride } => {
                let (output, stride) = (&mut **output, *stride);
                split(t, block, false, &mut move |width, row, col, sums| {
                    store_prep(t, output, width, row * stride + col, stride, sums);
                });
            }
            Sink::Average {
                pred0,
                forward,
                output,
                stride,
            } => {
                let (pred0, forward, output, stride) = (&**pred0, *forward, &mut **output, *stride);
                let max = vdupq_n_u16(255);
                passes(t, block, false, &mut move |width, row, col, sums| {
                    let values = vminq_u16(average(t, pred0, row * w + col, sums, forward), max);
                    store_bytes(output, width, row * stride + col, stride, values);
                });
            }
            Sink::AverageWide {
                pred0,
                forward,
                output,
                stride,
                max,
            } => {
                let (pred0, forward, output, stride) = (&**pred0, *forward, &mut **output, *stride);
                let max = vdupq_n_u16(*max);
                passes(t, block, false, &mut move |width, row, col, sums| {
                    let values = vminq_u16(average(t, pred0, row * w + col, sums, forward), max);
                    store_words(t, output, width, row * stride + col, stride, values);
                });
            }
            Sink::PrepNarrow { .. } | Sink::AverageNarrow { .. } => {}
        }
    }
);

fearless_simd::kernel!(
    /// Stores the COMPOUND_AVERAGE of two `size`-square register cells, 4
    /// or 8, clipped to `max`.
    #[inline]
    pub(super) fn compound_cell(
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
    pub(super) fn program(
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
    pub(super) fn compound_windows(
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

/// Whether `block` is a `w`-square register cell: 8x8, or 4x4 with at
/// most two tap pairs in each pass.
#[inline(always)]
pub(super) fn is_cell(block: &Block<'_, u16>, w: usize) -> bool {
    let small = |taps: Option<&Taps>| pairs(taps) <= 2;
    block.w == w
        && block.h == w
        && (w == 8 || (w == 4 && small(block.horizontal) && small(block.vertical)))
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

/// Where a register cell reads window row `r`: `base + r * stride`, or
/// when clamped row `top + r` clamped to `first..=last` from column `base`,
/// window column `c` from column `clamp(start + c, low, high)` of the load.
#[derive(Clone, Copy, Default)]
struct Reads {
    base: usize,
    stride: usize,
    top: i32,
    first: i32,
    last: i32,
    columns: [i16; 3],
}

/// Lane indices `0..8`.
const LANES: [i16; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// The [`Reads`] of `window` in a plane of `geometry` within `bounds`, and
/// whether they are clamped: in place when every read is inside the clip
/// bounds, else the § 7.13.3.18 `Clip3` reads; `None` when the clip range
/// is empty or a clamped row cannot load 16 samples.
#[inline(always)]
fn reads(window: &Window, geometry: Geometry, bounds: [i32; 4]) -> Option<(Reads, bool)> {
    let [stride, width, rows, len] = geometry;
    if let Some(base) = window.origin(geometry, bounds) {
        let reads = Reads {
            base,
            stride,
            ..Reads::default()
        };
        return Some((reads, false));
    }
    let bound = |value: i32, size: usize| value.max(0).min(size as i32 - 1);
    let (first_x, last_x) = (bound(bounds[0], width), bound(bounds[2], width));
    let (first, last) = (bound(bounds[1], rows), bound(bounds[3], rows));
    if first_x > last_x || first > last || last_x > i32::from(i16::MAX) - 32 {
        return None;
    }
    let column = window.column.max(first_x).min(last_x);
    let end = (last as usize)
        .checked_mul(stride)?
        .checked_add(column as usize + 16)?;
    if end > len {
        return None;
    }
    let start = window.column.max(first_x - 16).min(last_x + 16) - column;
    let reads = Reads {
        base: column as usize,
        stride,
        top: window.row,
        first,
        last,
        columns: [start, first_x - column, last_x - column].map(|value| value as i16),
    };
    Some((reads, true))
}

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
                let (low, high) = sums::<H>(h_weights, |k| slide(a, b, k));
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
                    let (low, high) = sums::<V>(v_weights, line);
                    groups[row] = (vrshrq_n_s32::<3>(low), vrshrq_n_s32::<3>(high));
                }
                (_, 0) => {}
                _ => {
                    let (low, high) = sums::<V>(v_weights, line);
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
    let rows = 8 / W;
    if forward == 8 {
        unroll!(group < G, [0 1 2 3 4 5 6 7] => {
            let values = vminq_u16(blend(first[group], second[group], 8), max);
            store_words(t, output, W, rows * group * stride, stride, values);
        });
    } else {
        unroll!(group < G, [0 1 2 3 4 5 6 7] => {
            let values = vminq_u16(blend(first[group], second[group], forward), max);
            store_words(t, output, W, rows * group * stride, stride, values);
        });
    }
}

/// Runs the passes of a single-reference `block`, handing `out` each group
/// as `(W, row, col, values)` saturated to `u16` before `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn put(t: Neon, block: &Block<'_, u16>, out: &mut impl FnMut(usize, usize, usize, uint16x8_t)) {
    if block.horizontal.is_some() || block.vertical.is_some() {
        split(t, block, true, &mut |width, row, col, (low, high)| {
            out(width, row, col, vqmovun_high_s32(vqmovun_s32(low), high));
        });
    } else if block.w == 4 {
        copy::<4>(t, block, &mut |w, row, col, x| {
            out(w, row, col, vreinterpretq_u16_s16(x));
        });
    } else {
        copy::<8>(t, block, &mut |w, row, col, x| {
            out(w, row, col, vreinterpretq_u16_s16(x));
        });
    }
}

/// [`passes`] of a block whose width is 4 more than a multiple of 8, in
/// pairs of rows, as 8-column strips and a 4-column tail, else [`passes`];
/// `out` stores each group at its own rows.
#[inline]
#[target_feature(enable = "neon")]
fn split(
    t: Neon,
    block: &Block<'_, u16>,
    put: bool,
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let body = block.w & !7;
    if block.w % 8 != 4 || body == 0 || !block.h.is_multiple_of(2) {
        passes(t, block, put, out);
        return;
    }
    let part = |origin: usize, w: usize| Block {
        source: block.source,
        stride: block.stride,
        origin,
        w,
        h: block.h,
        horizontal: block.horizontal,
        vertical: block.vertical,
    };
    passes(t, &part(block.origin, body), put, out);
    passes(
        t,
        &part(block.origin + body, 4),
        put,
        &mut |width, row, col, sums| {
            out(width, row, col + body, sums);
        },
    );
}

/// Runs the passes of `block`, handing `out` each group as
/// `(W, row, col, sums)`: compound predictors, or with `put` single-reference
/// predictions before `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn passes(
    t: Neon,
    block: &Block<'_, u16>,
    put: bool,
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let width = if block.w == 4 { 4 } else { 8 };
    match (block.horizontal, block.vertical) {
        (None, None) => {
            let mut widen = |w, row, col, x: int16x8_t| {
                let sums = if put {
                    let x = vreinterpretq_u16_s16(x);
                    (
                        vreinterpretq_s32_u32(vmovl_u16(vget_low_u16(x))),
                        vreinterpretq_s32_u32(vmovl_high_u16(x)),
                    )
                } else {
                    (vshll_n_s16::<4>(vget_low_s16(x)), vshll_high_n_s16::<4>(x))
                };
                out(w, row, col, sums);
            };
            if width == 4 {
                copy::<4>(t, block, &mut widen);
            } else {
                copy::<8>(t, block, &mut widen);
            }
        }
        (Some(taps), None) => {
            specialize!(width, taps.count, horizontal(t, block, taps, put, out));
        }
        (None, Some(taps)) => {
            specialize!(width, taps.count, vertical(t, block, taps, put, out));
        }
        (Some(h_taps), Some(v_taps)) => {
            specialize_pairs!(
                width,
                h_taps.count,
                v_taps.count,
                two_pass(t, block, (h_taps, v_taps), put, out)
            );
        }
    }
}

/// Eight samples from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load(t: Neon, samples: &[u16], at: usize) -> int16x8_t {
    match samples.get(at..).and_then(<[u16]>::first_chunk::<8>) {
        Some(lanes) => vreinterpretq_s16_u16(u16x8::load_array_ref(t, lanes).into()),
        None => vdupq_n_s16(0),
    }
}

/// Sixteen samples from `at` as two vectors, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load16(t: Neon, samples: &[u16], at: usize) -> (int16x8_t, int16x8_t) {
    match samples.get(at..).and_then(<[u16]>::first_chunk::<16>) {
        Some(lanes) => {
            let [a, b] = lanes.as_chunks::<8>().0 else {
                return (vdupq_n_s16(0), vdupq_n_s16(0));
            };
            let load = |lanes| vreinterpretq_s16_u16(u16x8::load_array_ref(t, lanes).into());
            (load(a), load(b))
        }
        None => (vdupq_n_s16(0), vdupq_n_s16(0)),
    }
}

/// Four `i32` values from `at`, zero past the slice.
#[inline]
#[target_feature(enable = "neon")]
fn load_i32(t: Neon, values: &[i32], at: usize) -> int32x4_t {
    match values.get(at..).and_then(<[i32]>::first_chunk::<4>) {
        Some(lanes) => i32x4::load_array_ref(t, lanes).into(),
        None => vdupq_n_s32(0),
    }
}

/// The eight samples of the group at window row `row`, column `col`.
#[inline]
#[target_feature(enable = "neon")]
fn group<const W: usize>(t: Neon, block: &Block<'_, u16>, row: usize, col: usize) -> int16x8_t {
    let first = load(t, block.source, block.start(row) + col);
    if W == 8 {
        return first;
    }
    let second = load(t, block.source, block.start(row + 1) + col);
    vcombine_s16(vget_low_s16(first), vget_low_s16(second))
}

/// The eight taps of `taps`, zero past its pairs: [`Taps::full`] as lanes.
#[inline]
#[target_feature(enable = "neon")]
fn weights(t: Neon, taps: &Taps) -> int16x8_t {
    vreinterpretq_s16_s32(i32x4::load_array_ref(t, &taps.full).into())
}

/// Window columns `k..k + 8` of the 16 samples `(a, b)`.
#[inline]
#[target_feature(enable = "neon")]
fn slide(a: int16x8_t, b: int16x8_t, k: usize) -> int16x8_t {
    match k {
        0 => a,
        1 => vextq_s16::<1>(a, b),
        2 => vextq_s16::<2>(a, b),
        3 => vextq_s16::<3>(a, b),
        4 => vextq_s16::<4>(a, b),
        5 => vextq_s16::<5>(a, b),
        6 => vextq_s16::<6>(a, b),
        _ => vextq_s16::<7>(a, b),
    }
}

#[inline]
#[target_feature(enable = "neon")]
fn mac<const L: i32>((low, high): Sums, x: int16x8_t, weights: int16x8_t) -> Sums {
    (
        vmlal_laneq_s16::<L>(low, vget_low_s16(x), weights),
        vmlal_high_laneq_s16::<L>(high, x, weights),
    )
}

/// `Σ` over `2 * P` taps of `row(k) * weights[k]`.
#[inline]
#[target_feature(enable = "neon")]
fn sums<const P: usize>(weights: int16x8_t, mut row: impl FnMut(usize) -> int16x8_t) -> Sums {
    let x = row(0);
    let mut sums = (
        vmull_laneq_s16::<0>(vget_low_s16(x), weights),
        vmull_high_laneq_s16::<0>(x, weights),
    );
    sums = mac::<1>(sums, row(1), weights);
    if P >= 2 {
        sums = mac::<2>(sums, row(2), weights);
        sums = mac::<3>(sums, row(3), weights);
    }
    if P >= 3 {
        sums = mac::<4>(sums, row(4), weights);
        sums = mac::<5>(sums, row(5), weights);
    }
    if P >= 4 {
        sums = mac::<6>(sums, row(6), weights);
        sums = mac::<7>(sums, row(7), weights);
    }
    sums
}

/// [`sums`] of four lanes.
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

/// The horizontal sums of four outputs of the window row from `start`.
#[inline]
#[target_feature(enable = "neon")]
fn horizontal_sums4<const P: usize>(
    t: Neon,
    source: &[u16],
    start: usize,
    weights: int16x8_t,
) -> int32x4_t {
    let a = load(t, source, start);
    let b = if P <= 2 {
        a
    } else {
        load(t, source, start + 8)
    };
    sums4::<P>(weights, |k| vget_low_s16(slide(a, b, k)))
}

#[inline]
#[target_feature(enable = "neon")]
fn round((low, high): Sums, shift: int32x4_t) -> Sums {
    (vrshlq_s32(low, shift), vrshlq_s32(high, shift))
}

#[inline(never)]
#[target_feature(enable = "neon")]
fn copy<const W: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    out: &mut impl FnMut(usize, usize, usize, int16x8_t),
) {
    for row in (0..block.h).step_by(8 / W) {
        if W == 4 {
            out(W, row, 0, group::<W>(t, block, row, 0));
            continue;
        }
        for col in strips(block.w) {
            out(W, row, col, group::<W>(t, block, row, col));
        }
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
fn horizontal<const W: usize, const P: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    put: bool,
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let weights = weights(t, taps);
    let (round0, round1) = (vdupq_n_s32(-3), vdupq_n_s32(-4));
    let finish = |sums: Sums| {
        let sums = round(sums, round0);
        if put { round(sums, round1) } else { sums }
    };
    if W == 4 {
        for row in (0..block.h).step_by(2) {
            let sums =
                |row: usize| horizontal_sums4::<P>(t, block.source, block.start(row), weights);
            out(4, row, 0, finish((sums(row), sums(row + 1))));
        }
        return;
    }
    for row in 0..block.h {
        let start = block.start(row);
        let mut a = load(t, block.source, start);
        for col in strips(block.w) {
            if !col.is_multiple_of(8) {
                a = load(t, block.source, start + col);
            }
            let b = load(t, block.source, start + col + 8);
            let window = (a, b);
            a = b;
            out(
                8,
                row,
                col,
                finish(sums::<P>(weights, |k| slide(window.0, window.1, k))),
            );
        }
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
fn vertical<const W: usize, const P: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    taps: &Taps,
    put: bool,
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let weights = weights(t, taps);
    let shift = vdupq_n_s32(if put { -7 } else { -3 });
    let line = |row: usize, col: usize| load(t, block.source, block.start(row) + col);
    if W == 4 {
        let mut r: [int16x4_t; 7] = core::array::from_fn(|k| {
            if k + 1 < 2 * P {
                vget_low_s16(line(k, 0))
            } else {
                vdup_n_s16(0)
            }
        });
        for row in (0..block.h).step_by(2) {
            let next = [
                vget_low_s16(line(row + 2 * P - 1, 0)),
                vget_low_s16(line(row + 2 * P, 0)),
            ];
            let tap = |k: usize| {
                if k + 1 < 2 * P {
                    r[k]
                } else {
                    next[k + 1 - 2 * P]
                }
            };
            let low = sums4::<P>(weights, tap);
            let high = sums4::<P>(weights, |k| tap(k + 1));
            out(4, row, 0, round((low, high), shift));
            r = core::array::from_fn(|k| if k + 1 < 2 * P { tap(k + 2) } else { r[k] });
        }
        return;
    }
    for col in strips(block.w) {
        ring::<P>(
            block.h,
            weights,
            |row| line(row, col),
            |row, sums| out(8, row, col, round(sums, shift)),
        );
    }
}

/// The vertical tap sums of `h` output rows of one 8-column strip whose
/// window row `row` is `line(row)`, handed to `out` by output row: the last
/// `2 * P - 1` rows stay in a register ring unrolled by its period.
#[inline]
#[target_feature(enable = "neon")]
fn ring<const P: usize>(
    h: usize,
    weights: int16x8_t,
    mut line: impl FnMut(usize) -> int16x8_t,
    mut out: impl FnMut(usize, Sums),
) {
    let period = 2 * P;
    let mut ring = [vdupq_n_s16(0); 8];
    unroll!(k < period - 1, [0 1 2 3 4 5 6] => {
        ring[k] = line(k);
    });
    let mut row = 0;
    loop {
        unroll!(phase < period, [0 1 2 3 4 5 6 7] => {
            if row == h {
                return;
            }
            let next = line(row + period - 1);
            let tap = |k: usize| if k + 1 < period { ring[(phase + k) % period] } else { next };
            out(row, sums::<P>(weights, tap));
            ring[(phase + period - 1) % period] = next;
            row += 1;
        });
    }
}

/// Both passes of `block`, `H` and `V` tap pairs: the `Round2(sum,
/// InterRound0)` horizontal rows of each strip in a register ring, two rows
/// per vector for a 4-column block, whose ragged last pair repeats the last
/// row.
#[inline(never)]
#[target_feature(enable = "neon")]
fn two_pass<const W: usize, const H: usize, const V: usize>(
    t: Neon,
    block: &Block<'_, u16>,
    (h_taps, v_taps): (&Taps, &Taps),
    put: bool,
    out: &mut impl FnMut(usize, usize, usize, Sums),
) {
    let (h_weights, v_weights) = (weights(t, h_taps), weights(t, v_taps));
    let shift = vdupq_n_s32(if put { -11 } else { -7 });
    let narrow = |(low, high): Sums| vrshrn_high_n_s32::<3>(vrshrn_n_s32::<3>(low), high);
    if W == 4 {
        let rows = block.h + 2 * V - 1;
        let row_sums =
            |row: usize| horizontal_sums4::<H>(t, block.source, block.start(row), h_weights);
        let line = |row: usize| narrow((row_sums(row), row_sums((row + 1).min(rows - 1))));
        let mut pairs: [int16x8_t; 4] =
            core::array::from_fn(|k| if k < V { line(2 * k) } else { vdupq_n_s16(0) });
        for row in (0..block.h).step_by(2) {
            let next = line(row + 2 * V);
            let pair = |k: usize| if k < V { pairs[k] } else { next };
            let sums = sums::<V>(v_weights, |k| {
                if k % 2 == 0 {
                    pair(k / 2)
                } else {
                    vextq_s16::<4>(pair(k / 2), pair(k / 2 + 1))
                }
            });
            out(4, row, 0, round(sums, shift));
            pairs = core::array::from_fn(|k| pair(k + 1));
        }
        return;
    }
    for col in strips(block.w) {
        let line = |row: usize| {
            let (a, b) = load16(t, block.source, block.start(row) + col);
            narrow(sums::<H>(h_weights, |k| slide(a, b, k)))
        };
        ring::<V>(block.h, v_weights, line, |row, sums| {
            out(8, row, col, round(sums, shift));
        });
    }
}

/// The COMPOUND_AVERAGE of the eight first predictors at `at` of `pred0`
/// and `sums`, saturated to `u16` before `Clip1`.
#[inline]
#[target_feature(enable = "neon")]
fn average(t: Neon, pred0: &[i32], at: usize, sums: Sums, forward: i32) -> uint16x8_t {
    blend(
        (load_i32(t, pred0, at), load_i32(t, pred0, at + 4)),
        sums,
        forward,
    )
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

/// The compound predictors of a group into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_prep(
    t: Neon,
    output: &mut [i32],
    width: usize,
    at: usize,
    stride: usize,
    (low, high): Sums,
) {
    let mut put = |at: usize, values: int32x4_t| {
        if let Some(slot) = output.get_mut(at..).and_then(<[i32]>::first_chunk_mut::<4>) {
            i32x4::<Neon>::simd_from(t, values).store_array(slot);
        }
    };
    put(at, low);
    put(if width == 4 { at + stride } else { at + 4 }, high);
}

/// The eight words `values` of a group into `output` from `at`.
#[inline]
#[target_feature(enable = "neon")]
fn store_words(
    t: Neon,
    output: &mut [u16],
    width: usize,
    at: usize,
    stride: usize,
    values: uint16x8_t,
) {
    let values = u16x8::<Neon>::simd_from(t, values);
    if width == 8 {
        if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>) {
            values.store_array(slot);
        }
        return;
    }
    let lanes = values.to_array();
    for (at, lanes) in [at, at + stride].into_iter().zip(lanes.as_chunks::<4>().0) {
        if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<4>) {
            *slot = *lanes;
        }
    }
}

/// The eight words `values`, at most 255, of a group as bytes into `output`.
#[inline]
#[target_feature(enable = "neon")]
fn store_bytes(output: &mut [u8], width: usize, at: usize, stride: usize, values: uint16x8_t) {
    let bytes = vget_lane_u64::<0>(vreinterpret_u64_u8(vmovn_u16(values))).to_le_bytes();
    if width == 8 {
        if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
            *slot = bytes;
        }
        return;
    }
    for (at, lanes) in [at, at + stride].into_iter().zip(bytes.as_chunks::<4>().0) {
        if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
            *slot = *lanes;
        }
    }
}
