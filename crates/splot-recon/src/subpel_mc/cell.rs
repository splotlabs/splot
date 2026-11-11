// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of 4x4 and 8x8 blocks: both references of
//! a compound cell and their § 7.13.3.15 COMPOUND_AVERAGE, or one eight-bit
//! single-reference block and its `Clip1`.
//!
//! An eight-bit block whose windows lie inside their clip bounds and storage
//! runs every pass in registers, with the arithmetic of the byte-lane
//! kernels: a horizontal pass is `Round2(sum, InterRound0)` of halved taps
//! (or the single-pass put `Round2`), a vertical-only pass weighs the samples
//! by the halved taps, and a two-pass vertical rounds the intermediates.
//! Every other block takes the general path. On AArch64 an eight-bit cell,
//! and everywhere a 16-bit cell, runs as a block of the vector kernels.

#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, clippy::unused_self, reason = "AVX2 and NEON cells only")
)]

use super::block::{TAPS, Taps, even_tap_span};
use super::{
    CellPlacement, InterpolationFilter, NUM_FILTER_TYPES, NUM_PHASES, ReferencePlaneView,
    SCALE_SUBPEL_BITS, SUBPEL_FILTERS, SUBPEL_MASK, SubpelPredictParams,
};
use crate::format::ReconSample;
use fearless_simd::{Simd, SimdFrom, i32x8, prelude::*, u8x16, u16x8, u32x4, u64x2};

/// One reference of a cell: window row `r` starts at `origin + r * stride`,
/// or, for a window that crosses its clip bounds, at the bounded row
/// `Clip3(clamped[1], clamped[2], clamped[0] + r)` and window column `c`
/// at `Clip3(0, clamped[4], c + clamped[3])` from `origin`.
#[derive(Clone, Copy)]
pub(super) struct Cell<'a, T = u8> {
    source: &'a [T],
    origin: u32,
    stride: u32,
    horizontal: Option<&'static Taps>,
    vertical: Option<&'static Taps>,
    clamped: Option<[i32; 5]>,
}

impl<'a, T: ReconSample> Cell<'a, T> {
    /// The cell of a `size`-square block at 1/1024-sample `[x, y]` with clip
    /// bounds `[firstX, firstY, lastX, lastY]`, or `None` when its clip range
    /// is empty or its window leaves the storage of 16 samples per row.
    #[allow(clippy::inline_always, reason = "measured compound cell hot path")]
    #[inline(always)]
    fn at(
        view: &ReferencePlaneView<'a, T>,
        interp: InterpolationFilter,
        size: usize,
        ([start_x, start_y], bounds): CellPlacement,
    ) -> Option<Self> {
        let filters = &TAPS[interp.pass_index(size as u32) as usize];
        let taps = |start: i32| filters[((start >> 6) & SUBPEL_MASK) as usize].as_ref();
        let (horizontal, vertical) = (taps(start_x), taps(start_y));
        let window = |start: i32, taps: Option<&Taps>| {
            let base = start >> SCALE_SUBPEL_BITS;
            taps.map_or((base, size as i32), |taps| {
                (
                    base - 3 + taps.start as i32,
                    (size + taps.span() - 1) as i32,
                )
            })
        };
        let ((column, columns), (row, rows)) =
            (window(start_x, horizontal), window(start_y, vertical));
        let bound = |value: i32, last: usize| value.max(0).min(last as i32 - 1);
        let (width, height) = (view.width, view.readable_rows);
        let (Ok(_), Ok(stride)) = (
            u32::try_from(view.samples.len()),
            u32::try_from(view.stride),
        ) else {
            return None;
        };
        let [first_x, last_x] = [bound(bounds[0], width), bound(bounds[2], width)];
        let [first_y, last_y] = [bound(bounds[1], height), bound(bounds[3], height)];
        if first_x > last_x || first_y > last_y {
            return None;
        }
        let inside = column >= first_x
            && column + columns <= last_x + 1
            && row >= first_y
            && row + rows <= last_y + 1;
        let fits = |row: i32, column: i32| {
            row as usize * view.stride + column as usize + 16 <= view.samples.len()
        };
        let cell = |origin: usize, clamped| Self {
            source: view.samples,
            origin: origin as u32,
            stride,
            horizontal,
            vertical,
            clamped,
        };
        if inside {
            return fits(row + rows - 1, column)
                .then(|| cell(row as usize * view.stride + column as usize, None));
        }
        let base = column.max(first_x).min(last_x);
        let columns = [(column - base).max(-16), (last_x - base).min(15)];
        fits((row + rows - 1).max(first_y).min(last_y), base).then(|| {
            cell(
                base as usize,
                Some([row, first_y, last_y, columns[0], columns[1]]),
            )
        })
    }

    /// This `size`-square cell as a block of the vector kernels: the
    /// bounded rows of a clamped window, and its column `Clip3` as a remap.
    #[inline]
    fn block(&self, size: usize) -> super::block::Block<'a, T> {
        let (top, rows, remap) = match self.clamped {
            Some([row, first, last, offset, limit]) => (
                row as isize,
                [first as isize, last as isize],
                Some(core::array::from_fn(|column| {
                    (column as i32 + offset).max(0).min(limit) as u8
                })),
            ),
            None => (0, [0, isize::MAX], None),
        };
        super::block::Block {
            source: self.source,
            stride: self.stride as usize,
            origin: self.origin as usize,
            top,
            rows,
            w: size,
            h: size,
            horizontal: self.horizontal,
            vertical: self.vertical,
            remap,
        }
    }
}

/// The cell of a 4x4 or 8x8 single-reference block that [`Cell::put`]
/// stores, or `None` outside that subset.
#[inline]
pub(super) fn put_cell<'a>(
    reference: &ReferencePlaneView<'a, u8>,
    params: &SubpelPredictParams,
) -> Option<Cell<'a>> {
    let size = params.w;
    if params.h != size || (size != 4 && size != 8) {
        return None;
    }
    let cell = Cell::at(
        reference,
        params.interp,
        size,
        ([params.start_x, params.start_y], params.bounds()),
    )?;
    if size == 4 && cell.horizontal.is_some_and(|taps| taps.count > 2) {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    if fearless_simd::Level::baseline().as_avx2().is_some() {
        return Some(cell);
    }
    #[cfg(target_arch = "aarch64")]
    if fearless_simd::Level::baseline().as_neon().is_some() {
        return Some(cell);
    }
    None
}

impl<'a> Cell<'a> {
    /// The kernel key `5 * H + V + 25 * clamped` of this cell's tap-pair
    /// counts, zero for a skipped pass.
    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn key(&self) -> i32 {
        (5 * self.horizontal.map_or(0, |taps| taps.count)
            + self.vertical.map_or(0, |taps| taps.count)
            + 25 * usize::from(self.clamped.is_some())) as i32
    }

    /// This cell read from `source`, storage of the same geometry.
    pub(super) fn with_source(&self, source: &'a [u8]) -> Self {
        Self { source, ..*self }
    }

    /// Stores the `Clip1` single-reference prediction of this `size`-square
    /// cell of [`put_cell`].
    pub(super) fn put(&self, size: usize, output: &mut [u8], stride: usize) {
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
            avx2::put(token, self, size, output, stride);
        }
        #[cfg(target_arch = "aarch64")]
        if let Some(token) = fearless_simd::Level::baseline().as_neon() {
            let mut sink = super::block::Sink::Put { output, stride };
            super::eight_bit::neon::predict(token, &self.block(size), &mut sink);
        }
        let _ = (size, output, stride);
    }
}

/// The window origins of a `size`-square cell whose references both sit at
/// full-sample positions inside their clip bounds and storage, or `None`.
#[allow(clippy::inline_always, reason = "full-sample compound cell hot path")]
#[inline(always)]
fn fullpel_origins(
    references: [&ReferencePlaneView<'_, u8>; 2],
    size: usize,
    placements: [CellPlacement; 2],
) -> Option<[usize; 2]> {
    let fraction = SUBPEL_MASK << 6;
    let mut origins = [0; 2];
    for ((view, ([x, y], bounds)), origin) in references.iter().zip(placements).zip(&mut origins) {
        if (x | y) & fraction != 0 {
            return None;
        }
        let bound = |value: i32, last: usize| value.max(0).min(last as i32 - 1);
        let (column, row, size) = (x >> SCALE_SUBPEL_BITS, y >> SCALE_SUBPEL_BITS, size as i32);
        let (width, height) = (view.width, view.readable_rows);
        if column < bound(bounds[0], width)
            || column + size > bound(bounds[2], width) + 1
            || row < bound(bounds[1], height)
            || row + size > bound(bounds[3], height) + 1
        {
            return None;
        }
        *origin = row as usize * view.stride + column as usize;
        if *origin + (size as usize - 1) * view.stride + size as usize > view.samples.len() {
            return None;
        }
    }
    Some(origins)
}

/// Predicts both references of a 4x4 or 8x8 cell and stores their
/// COMPOUND_AVERAGE, or returns `false` without writing outside the subset.
pub(super) fn compound(
    references: [&ReferencePlaneView<'_, u8>; 2],
    params: [&SubpelPredictParams; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let size = params[0].w;
    params[0].h == size
        && params[1].interp == params[0].interp
        && compound_at(
            references,
            params[0].interp,
            size,
            params.map(|params| ([params.start_x, params.start_y], params.bounds())),
            forward,
            output,
            stride,
        )
}

/// [`compound`] of a `size`-square cell whose references share `interp`,
/// from each reference's [`CellPlacement`].
#[allow(clippy::inline_always, reason = "full-sample compound cell hot path")]
#[inline(always)]
pub(super) fn compound_at(
    references: [&ReferencePlaneView<'_, u8>; 2],
    interp: InterpolationFilter,
    size: usize,
    placements: [CellPlacement; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    if size != 4 && size != 8 {
        return false;
    }
    if let Some(origins) = fullpel_origins(references, size, placements)
        && (0..=16).contains(&forward)
    {
        let sources = references.map(|view| (view.samples, view.stride));
        baseline_simd!(simd => if size == 4 {
            fullpel_cell::<_, 4>(simd, sources, origins, forward, output, stride);
        } else {
            fullpel_cell::<_, 8>(simd, sources, origins, forward, output, stride);
        });
        return true;
    }
    filtered_at(
        references, interp, size, placements, forward, output, stride,
    )
}

/// The COMPOUND_AVERAGE `(a * f + b * (16 - f) + 8) >> 4` of two
/// full-sample `N`-square windows, `N` 4 or 8, read from `origins` of their
/// `(samples, stride)` sources, into `output`; `forward` lies in `0..=16`.
#[allow(clippy::inline_always, reason = "full-sample compound cell hot path")]
#[inline(always)]
fn fullpel_cell<S: Simd, const N: usize>(
    simd: S,
    sources: [(&[u8], usize); 2],
    origins: [usize; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    let rows = |(samples, step): (&[u8], usize), origin: usize, row: usize| -> u8x16<S> {
        let at = |lane: usize| origin + (row + lane) * step;
        let word = |at: usize| {
            samples
                .get(at..)
                .and_then(<[u8]>::first_chunk::<8>)
                .map_or(0, |bytes| u64::from_le_bytes(*bytes))
        };
        if N == 8 {
            u64x2::from_slice(simd, &[word(at(0)), word(at(1))]).bitcast()
        } else {
            let lanes: [u32; 4] = core::array::from_fn(|lane| {
                samples
                    .get(at(lane)..)
                    .and_then(<[u8]>::first_chunk::<4>)
                    .map_or(0, |bytes| u32::from_le_bytes(*bytes))
            });
            u32x4::from_slice(simd, &lanes).bitcast()
        }
    };
    for row in (0..N).step_by(16 / N) {
        let (a, b) = (
            rows(sources[0], origins[0], row),
            rows(sources[1], origins[1], row),
        );
        let (low, high) = if forward == 8 {
            let (low, high) = simd.widening_add_u8x16(a, b);
            ((low + 1) >> 1, (high + 1) >> 1)
        } else {
            let weights = (
                u8x16::splat(simd, forward as u8),
                u8x16::splat(simd, (16 - forward) as u8),
            );
            let (a0, a1) = simd.widening_mul_u8x16(a, weights.0);
            let (b0, b1) = simd.widening_mul_u8x16(b, weights.1);
            ((a0 + b0 + 8) >> 4, (a1 + b1 + 8) >> 4)
        };
        let bytes = simd.narrow_u16x8(low, high);
        if N == 8 {
            let words = bytes.bitcast::<u64x2<S>>();
            for lane in 0..2 {
                let at = (row + lane) * stride;
                if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<8>) {
                    *slot = words[lane].to_le_bytes();
                }
            }
        } else {
            let words = bytes.bitcast::<u32x4<S>>();
            for lane in 0..4 {
                let at = (row + lane) * stride;
                if let Some(slot) = output.get_mut(at..).and_then(<[u8]>::first_chunk_mut::<4>) {
                    *slot = words[lane].to_le_bytes();
                }
            }
        }
    }
}

/// [`fullpel_cell`] of 16-bit windows, clipped to `max`.
#[allow(clippy::inline_always, reason = "full-sample compound cell hot path")]
#[inline(always)]
fn fullpel_cell_u16<S: Simd, const N: usize>(
    simd: S,
    sources: [(&[u16], usize); 2],
    origins: [usize; 2],
    (forward, max): (i32, u16),
    output: &mut [u16],
    stride: usize,
) {
    let rows = |(samples, step): (&[u16], usize), origin: usize, row: usize| -> u16x8<S> {
        let at = |lane: usize| origin + (row + lane) * step;
        if N == 8 {
            return samples
                .get(at(0)..)
                .and_then(<[u16]>::first_chunk::<8>)
                .map_or(u16x8::splat(simd, 0), |lanes| {
                    u16x8::load_array_ref(simd, lanes)
                });
        }
        let word = |lane: usize| {
            samples
                .get(at(lane)..)
                .and_then(<[u16]>::first_chunk::<4>)
                .map_or(0, |lanes| {
                    lanes
                        .iter()
                        .rev()
                        .fold(0, |word, &lane| word << 16 | u64::from(lane))
                })
        };
        u64x2::from_slice(simd, &[word(0), word(1)]).bitcast()
    };
    let max = u16x8::splat(simd, max);
    for row in (0..N).step_by(8 / N) {
        let (a, b) = (
            rows(sources[0], origins[0], row),
            rows(sources[1], origins[1], row),
        );
        let average = if forward == 8 {
            (a + b + 1) >> 1
        } else {
            let (a0, a1) = simd.widening_mul_u16x8(a, u16x8::splat(simd, forward as u16));
            let (b0, b1) = simd.widening_mul_u16x8(b, u16x8::splat(simd, (16 - forward) as u16));
            simd.narrow_u32x4((a0 + b0 + 8) >> 4, (a1 + b1 + 8) >> 4)
        }
        .min(max);
        if N == 8 {
            if let Some(slot) = output
                .get_mut(row * stride..)
                .and_then(<[u16]>::first_chunk_mut)
            {
                average.store_array(slot);
            }
            continue;
        }
        let lanes = average.to_array();
        for lane in 0..2 {
            let at = (row + lane) * stride;
            if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<4>) {
                *slot = core::array::from_fn(|column| lanes[4 * lane + column]);
            }
        }
    }
}

/// [`compound_at`] of a cell outside the full-sample register path.
#[inline(never)]
fn filtered_at(
    references: [&ReferencePlaneView<'_, u8>; 2],
    interp: InterpolationFilter,
    size: usize,
    placements: [CellPlacement; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let (Some(first), Some(second)) = (
        Cell::at(references[0], interp, size, placements[0]),
        Cell::at(references[1], interp, size, placements[1]),
    ) else {
        return false;
    };
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        return avx2::compound(
            token,
            [&first, &second],
            placements.map(|(start, _)| start),
            interp,
            size,
            forward,
            output,
            stride,
        );
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(token) = fearless_simd::Level::baseline().as_neon() {
        let blocks = [first.block(size), second.block(size)];
        super::eight_bit::neon::compound(token, [&blocks[0], &blocks[1]], forward, output, stride);
        return true;
    }
    let _ = (first, second, forward, output, stride);
    false
}

/// [`compound_at`] of 16-bit references, clipped to `max`: `false` without
/// writing outside the word-lane kernels or for a column-clamped window
/// without a horizontal pass.
pub(super) fn compound_u16(
    references: [&ReferencePlaneView<'_, u16>; 2],
    interp: InterpolationFilter,
    size: usize,
    placements: [CellPlacement; 2],
    (forward, max): (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    if size != 4 && size != 8 {
        return false;
    }
    let (Some(first), Some(second)) = (
        Cell::at(references[0], interp, size, placements[0]),
        Cell::at(references[1], interp, size, placements[1]),
    ) else {
        return false;
    };
    compound_cells_u16([&first, &second], size, (forward, max), output, stride)
}

/// Stores the COMPOUND_AVERAGE of two 16-bit cells of [`compound_u16`].
fn compound_cells_u16(
    cells: [&Cell<'_, u16>; 2],
    size: usize,
    (forward, max): (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
    let blocks = cells.map(|cell| cell.block(size));
    let blocks = [&blocks[0], &blocks[1]];
    if blocks
        .iter()
        .any(|block| block.remap.is_some() && block.horizontal.is_none())
    {
        return false;
    }
    if blocks
        .iter()
        .all(|block| block.horizontal.is_none() && block.vertical.is_none())
    {
        super::portable::compound_u16(blocks, forward, max, &mut [], output, stride);
        return true;
    }
    #[cfg(target_arch = "x86_64")]
    {
        let len = blocks.iter().fold(0, |len, block| {
            let rows = size + block.vertical.map_or(0, |taps| taps.span() - 1);
            len.max(super::block::stage_len(rows, size, block.vertical))
        });
        super::with_subpel_intermediate(len, |stage| {
            super::sixteen_bit::compound(blocks, forward, max, stage, output, stride)
        })
    }
    #[cfg(not(target_arch = "x86_64"))]
    super::sixteen_bit::compound(blocks, forward, max, &mut [], output, stride)
}

/// The most cells one [`CompoundCells`] program holds.
const PROGRAM_CELLS: usize = 256;

/// A program of unscaled compound cells of one grid plane that share a
/// size and filter: each cell's `[row, col]` output position and both
/// references' [`CellPlacement`], stored field by field so that a run sets
/// up eight cells at a time.
#[derive(Clone)]
pub struct CompoundCells {
    len: usize,
    positions: [[u32; 2]; PROGRAM_CELLS],
    starts: [[[i32; PROGRAM_CELLS]; 2]; 2],
    bounds: [[[i32; PROGRAM_CELLS]; 4]; 2],
    #[cfg_attr(
        not(target_arch = "x86_64"),
        allow(dead_code, reason = "read by the AVX2 program")
    )]
    stash: [Stashed; 32],
}

/// One reference's predictors of a cell between its kernel and the average.
type Stashed = [[i16; 16]; 4];

impl Default for CompoundCells {
    fn default() -> Self {
        Self {
            len: 0,
            positions: [[0; 2]; PROGRAM_CELLS],
            starts: [[[0; PROGRAM_CELLS]; 2]; 2],
            bounds: [[[0; PROGRAM_CELLS]; 4]; 2],
            stash: [[[0; 16]; 4]; 32],
        }
    }
}

impl CompoundCells {
    /// Appends a cell at output `[row, col]`, or returns `false` without
    /// storing it when the program is full.
    #[inline]
    pub fn push(&mut self, [row, col]: [usize; 2], placements: [CellPlacement; 2]) -> bool {
        let index = self.len;
        let (Ok(row), Ok(col)) = (u32::try_from(row), u32::try_from(col)) else {
            return false;
        };
        if index >= PROGRAM_CELLS {
            return false;
        }
        self.positions[index] = [row, col];
        for (reference, ([x, y], bounds)) in placements.into_iter().enumerate() {
            self.starts[reference][0][index] = x;
            self.starts[reference][1][index] = y;
            for (field, value) in bounds.into_iter().enumerate() {
                self.bounds[reference][field][index] = value;
            }
        }
        self.len = index + 1;
        true
    }

    /// The number of cells.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the program holds no cell.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Removes every cell.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// The output position and placements of cell `index`.
    #[must_use]
    #[inline]
    pub fn get(&self, index: usize) -> Option<([usize; 2], [CellPlacement; 2])> {
        (index < self.len.min(PROGRAM_CELLS)).then(|| {
            let [row, col] = self.positions[index];
            (
                [row as usize, col as usize],
                [0, 1].map(|reference| self.placement(index, reference)),
            )
        })
    }

    #[inline]
    fn placement(&self, index: usize, reference: usize) -> CellPlacement {
        let field = |values: &[i32; PROGRAM_CELLS]| values.get(index).copied().unwrap_or(0);
        let (starts, bounds) = (&self.starts[reference & 1], &self.bounds[reference & 1]);
        (starts.each_ref().map(field), bounds.each_ref().map(field))
    }
}

/// Per filter type and phase, the window of a pass packed as the column
/// offset `start - 3` of its first tap (low byte, signed), its extent
/// `span - 1` (second byte) and its tap-pair count (third byte); zero for a
/// skipped pass.
static WINDOWS: [[i32; NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut table = [[0; NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 1;
        while phase < NUM_PHASES {
            let (start, taps) = even_tap_span(&SUBPEL_FILTERS[filter][phase]);
            table[filter][phase] =
                ((start as i32 - 3) & 0xff) | ((taps as i32 - 1) << 8) | ((taps as i32 / 2) << 16);
            phase += 1;
        }
        filter += 1;
    }
    table
};

type CellPlane<'a, 'b, T, O> = (&'a [ReferencePlaneView<'b, T>; 2], &'a mut [O], usize);
type Plane<'a, 'b, P> = ([ReferencePlaneView<'b, P>; 2], &'a mut [P], usize);
type Written = [[u64; PROGRAM_CELLS / 64]; 2];

/// Runs a [`CompoundCells`] program of `size`-square cells into each plane
/// (U and V share one program), clipped to `bit_depth`. Each cell that
/// [`super::subpel_compound_cell`] would store from its position and
/// placements is stored the same way and sets bit `index % 64` of word
/// `index / 64` of its plane's mask;
/// every other cell is left unwritten for the caller to predict.
#[doc(hidden)]
#[inline]
pub fn subpel_compound_cells<T: ReconSample, O: ReconSample>(
    mut planes: [Option<CellPlane<'_, '_, T, O>>; 2],
    interp: InterpolationFilter,
    size: usize,
    cells: &mut CompoundCells,
    (cwp_weight, bit_depth): (i16, crate::format::BitDepth),
) -> Written {
    if size != 4 && size != 8 {
        return [[0; PROGRAM_CELLS / 64]; 2];
    }
    let forward = i32::from(cwp_weight);
    if O::u8_slice(&[]).is_some() {
        let planes = planes.each_mut().map(|plane| {
            let (views, output, stride) = plane.as_mut()?;
            let views = [views[0].as_u8()?, views[1].as_u8()?];
            (*stride >= size).then_some((views, O::u8_slice_mut(output)?, *stride))
        });
        return by_geometry(
            planes,
            |mut planes| baseline_simd!(simd => program(simd, &mut planes, interp, size, cells, forward)),
        );
    }
    let planes = planes.each_mut().map(|plane| {
        let (views, output, stride) = plane.as_mut()?;
        let views = [views[0].as_u16()?, views[1].as_u16()?];
        (*stride >= size).then_some((views, O::u16_slice_mut(output)?, *stride))
    });
    let clip = (forward, bit_depth.max_sample());
    by_geometry(
        planes,
        |mut planes| baseline_simd!(simd => program_u16(simd, &mut planes, interp, size, cells, clip)),
    )
}

/// Runs `run` on both planes when they share their reference geometry,
/// else on each plane alone.
fn by_geometry<P: ReconSample>(
    planes: [Option<Plane<'_, '_, P>>; 2],
    mut run: impl FnMut([Option<Plane<'_, '_, P>>; 2]) -> Written,
) -> Written {
    let geometry = |plane: &Option<Plane<'_, '_, P>>| {
        plane.as_ref().map(|(views, ..)| {
            views.each_ref().map(|view| {
                (
                    view.stride,
                    view.width,
                    view.readable_rows,
                    view.samples.len(),
                )
            })
        })
    };
    if planes[1].is_none() || geometry(&planes[0]) == geometry(&planes[1]) {
        return run(planes);
    }
    let [first, second] = planes;
    [run([first, None])[0], run([second, None])[0]]
}

/// Eight cells of one reference: window origins (the clamped column for
/// a clamped window), tap phases per pass, the [`Cell`] bounds of a
/// clamped window, kernel keys `5 * H + V + 25 * clamped`, and a bit per
/// cell that [`Cell::at`] admits and the kernels of its size cover.
#[derive(Default)]
struct Lanes {
    origin: [i32; 8],
    phases: [[i32; 8]; 2],
    clamped: [[i32; 8]; 5],
    keys: [i32; 8],
    valid: u32,
}

impl Lanes {
    /// The [`Cell`] of `lane` read from `source` at `stride`.
    #[inline]
    fn cell<'a, T>(
        &self,
        lane: usize,
        (source, stride): (&'a [T], u32),
        filters: &'static [Option<Taps>; NUM_PHASES],
        clamped: bool,
    ) -> Cell<'a, T> {
        let taps = |pass: usize| filters[self.phases[pass][lane] as usize & 15].as_ref();
        Cell {
            source,
            origin: self.origin[lane] as u32,
            stride,
            horizontal: taps(0),
            vertical: taps(1),
            clamped: clamped.then(|| self.clamped.map(|field| field[lane])),
        }
    }
}

/// Per plane and reference, the samples [`Lanes`] index, with each
/// reference's stride and the number of planes.
struct Sources<'a> {
    samples: [[&'a [u8]; 2]; 2],
    strides: [u32; 2],
    #[cfg_attr(
        not(target_arch = "x86_64"),
        allow(dead_code, reason = "read by the AVX2 program")
    )]
    planes: usize,
}

/// The [`Lanes`] of `reference` for program cells `batch..batch + 8`:
/// [`Cell::at`] of eight cells at once.
#[allow(clippy::inline_always, reason = "compound cell program setup")]
#[inline(always)]
fn lanes<S: Simd, T: ReconSample>(
    simd: S,
    view: &ReferencePlaneView<'_, T>,
    windows: &[i32; NUM_PHASES],
    size: usize,
    (starts, bounds): (&[[i32; PROGRAM_CELLS]; 2], &[[i32; PROGRAM_CELLS]; 4]),
    batch: usize,
) -> Lanes {
    let (Ok(len), Ok(stride), Ok(width), Ok(rows)) = (
        i32::try_from(view.samples.len()),
        i32::try_from(view.stride),
        i32::try_from(view.width),
        i32::try_from(view.readable_rows),
    ) else {
        return Lanes::default();
    };
    if i64::from(rows) * i64::from(stride) + i64::from(width) + 16 > i64::from(i32::MAX) {
        return Lanes::default();
    }
    let splat = |value: i32| i32x8::splat(simd, value);
    let load = |values: &[i32; PROGRAM_CELLS]| {
        values
            .get(batch..)
            .and_then(<[i32]>::first_chunk::<8>)
            .map_or(splat(0), |values| i32x8::simd_from(simd, *values))
    };
    let (x, y) = (load(&starts[0]), load(&starts[1]));
    let fifteen = splat(15);
    let phases = [(x >> 6) & fifteen, (y >> 6) & fifteen];
    let window = |phase: i32x8<S>| {
        i32x8::simd_from(
            simd,
            core::array::from_fn::<_, 8, _>(|lane| windows[phase[lane] as usize & 15]),
        )
    };
    let (wx, wy) = (window(phases[0]), window(phases[1]));
    let span = |base: i32x8<S>, window: i32x8<S>| {
        let first = base + ((window << 24) >> 24);
        (
            first,
            first + splat(size as i32) + ((window >> 8) & splat(0xff)),
        )
    };
    let (column, column_end) = span(x >> 10, wx);
    let (row, row_end) = span(y >> 10, wy);
    let bound =
        |values: &[i32; PROGRAM_CELLS], last: i32| load(values).max(splat(0)).min(splat(last));
    let [first_x, first_y] = [bound(&bounds[0], width - 1), bound(&bounds[1], rows - 1)];
    let [last_x, last_y] = [bound(&bounds[2], width - 1), bound(&bounds[3], rows - 1)];
    let outside = first_x.simd_gt(column)
        | column_end.simd_gt(last_x + 1)
        | first_y.simd_gt(row)
        | row_end.simd_gt(last_y + 1);
    let base = column.max(first_x).min(last_x);
    let last_row = row_end - 1;
    let fit_row = outside.select(last_row.max(first_y).min(last_y), last_row);
    let stride = splat(stride);
    let end = fit_row * stride + outside.select(base, column) + 16;
    let [horizontal, vertical] = [wx >> 16, wy >> 16];
    let invalid = end.simd_gt(splat(len))
        | horizontal.simd_gt(splat(if size == 4 { 2 } else { 4 }))
        | first_x.simd_gt(last_x)
        | first_y.simd_gt(last_y);
    let array = |values: i32x8<S>| -> [i32; 8] { *values };
    Lanes {
        origin: array(outside.select(base, row * stride + column)),
        phases: phases.map(array),
        clamped: [
            array(row),
            array(first_y),
            array(last_y),
            array((column - base).max(splat(-16))),
            array((last_x - base).min(fifteen)),
        ],
        keys: array(horizontal * 5 + vertical + outside.select(splat(25), splat(0))),
        valid: !(invalid.to_bitmask() as u32) & 0xff,
    }
}

/// Runs `cells` into `planes`: the [`Lanes`] of eight cells at a time, the
/// full-sample average of each cell whose references both need no pass,
/// and the [`predict_items`] and [`store_item`] kernels of every other one.
#[inline(never)]
fn program<S: Simd>(
    simd: S,
    planes: &mut [Option<Plane<'_, '_, u8>>; 2],
    interp: InterpolationFilter,
    size: usize,
    cells: &mut CompoundCells,
    forward: i32,
) -> Written {
    let mut written = [[0; PROGRAM_CELLS / 64]; 2];
    let Some((first_views, ..)) = &planes[0] else {
        return written;
    };
    let first_views = *first_views;
    let second = planes[1].as_ref().map_or(first_views, |(views, ..)| *views);
    let sources = Sources {
        samples: [first_views, second].map(|views| [views[0].samples, views[1].samples]),
        strides: first_views.map(|view| u32::try_from(view.stride).unwrap_or(0)),
        planes: 1 + usize::from(planes[1].is_some()),
    };
    let filter = interp.pass_index(size as u32) as usize;
    let (filters, windows) = (&TAPS[filter], &WINDOWS[filter]);
    let averaged = (0..=16).contains(&forward);
    let len = cells.len.min(PROGRAM_CELLS);
    for batch in (0..len).step_by(8) {
        let lanes = [0, 1].map(|reference| {
            lanes(
                simd,
                &first_views[reference],
                windows,
                size,
                (&cells.starts[reference], &cells.bounds[reference]),
                batch,
            )
        });
        let live = (1 << (len - batch).min(8)) - 1;
        let valid = lanes[0].valid & lanes[1].valid & live;
        let fullpel = (0..8).fold(0u32, |mask, lane| {
            let still = lanes[0].keys[lane] == 0 && lanes[1].keys[lane] == 0;
            mask | u32::from(still && averaged) << lane
        }) & valid;
        let filtered = valid & !fullpel;
        let done = predict_items(
            &lanes,
            filtered | filtered << 8,
            &sources,
            (filters, size),
            &mut cells.stash,
        );
        let mut pending = done & done >> 8 | fullpel;
        while pending != 0 {
            let lane = pending.trailing_zeros() as usize;
            pending &= pending - 1;
            let index = batch + lane;
            let Some(&[row, col]) = cells.positions.get(index) else {
                continue;
            };
            for (plane, (slot, written)) in planes.iter_mut().zip(&mut written).enumerate() {
                let Some((views, output, stride)) = slot else {
                    continue;
                };
                let stride = *stride;
                let at = row as usize * stride + col as usize;
                let Some(output) = output
                    .get_mut(at..)
                    .filter(|output| (size - 1) * stride + size <= output.len())
                else {
                    continue;
                };
                let stored = if fullpel >> lane & 1 != 0 {
                    let sources = views.each_ref().map(|view| (view.samples, view.stride));
                    let origins = lanes.each_ref().map(|lanes| lanes.origin[lane] as usize);
                    if size == 4 {
                        fullpel_cell::<_, 4>(simd, sources, origins, forward, output, stride);
                    } else {
                        fullpel_cell::<_, 8>(simd, sources, origins, forward, output, stride);
                    }
                    true
                } else {
                    store_item(
                        &lanes,
                        &sources,
                        (filters, size),
                        &cells.stash,
                        [plane, lane],
                        forward,
                        output,
                        stride,
                    )
                };
                if stored {
                    written[index / 64 % 4] |= 1 << (index % 64);
                }
            }
        }
    }
    written
}

/// [`program`] of 16-bit planes: every cell the [`Lanes`] admit goes
/// through [`compound_cells_u16`].
#[inline(never)]
fn program_u16<S: Simd>(
    simd: S,
    planes: &mut [Option<Plane<'_, '_, u16>>; 2],
    interp: InterpolationFilter,
    size: usize,
    cells: &CompoundCells,
    clip: (i32, u16),
) -> Written {
    let mut written = [[0; PROGRAM_CELLS / 64]; 2];
    let Some((first_views, ..)) = &planes[0] else {
        return written;
    };
    let first_views = *first_views;
    let strides = first_views.map(|view| u32::try_from(view.stride).unwrap_or(0));
    let filter = interp.pass_index(size as u32) as usize;
    let (filters, windows) = (&TAPS[filter], &WINDOWS[filter]);
    let len = cells.len.min(PROGRAM_CELLS);
    for batch in (0..len).step_by(8) {
        let lanes = [0, 1].map(|reference| {
            let program = (&cells.starts[reference], &cells.bounds[reference]);
            lanes(simd, &first_views[reference], windows, size, program, batch)
        });
        let mut pending = lanes[0].valid & lanes[1].valid & ((1 << (len - batch).min(8)) - 1);
        while pending != 0 {
            let lane = pending.trailing_zeros() as usize;
            pending &= pending - 1;
            let index = batch + lane;
            let Some(&[row, col]) = cells.positions.get(index) else {
                continue;
            };
            let fullpel =
                lanes[0].keys[lane] == 0 && lanes[1].keys[lane] == 0 && (0..=16).contains(&clip.0);
            for (slot, written) in planes.iter_mut().zip(&mut written) {
                let Some((views, output, stride)) = slot else {
                    continue;
                };
                let (at, stride) = (row as usize * *stride + col as usize, *stride);
                let Some(output) = output
                    .get_mut(at..)
                    .filter(|output| (size - 1) * stride + size <= output.len())
                else {
                    continue;
                };
                if fullpel {
                    let sources = views.each_ref().map(|view| (view.samples, view.stride));
                    let origins = lanes.each_ref().map(|lanes| lanes.origin[lane] as usize);
                    if size == 4 {
                        fullpel_cell_u16::<_, 4>(simd, sources, origins, clip, output, stride);
                    } else {
                        fullpel_cell_u16::<_, 8>(simd, sources, origins, clip, output, stride);
                    }
                    written[index / 64 % 4] |= 1 << (index % 64);
                    continue;
                }
                let cells = [0, 1].map(|reference| {
                    let lanes = &lanes[reference];
                    let source = (views[reference].samples, strides[reference]);
                    lanes.cell(lane, source, filters, lanes.keys[lane] >= 25)
                });
                if compound_cells_u16([&cells[0], &cells[1]], size, clip, output, stride) {
                    written[index / 64 % 4] |= 1 << (index % 64);
                }
            }
        }
    }
    written
}

/// Predicts each reference `item >> 3` of lane `item & 7` set in `items`
/// for [`store_item`], and returns the items it predicted.
#[inline]
fn predict_items(
    lanes: &[Lanes; 2],
    items: u32,
    sources: &Sources<'_>,
    (filters, size): (&'static [Option<Taps>; NUM_PHASES], usize),
    stash: &mut [Stashed],
) -> u32 {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        return avx2::run_items(token, items, lanes, sources, filters, size, stash);
    }
    #[cfg(target_arch = "aarch64")]
    if fearless_simd::Level::baseline().as_neon().is_some() {
        return items;
    }
    let _ = (lanes, items, sources, filters, size, stash);
    0
}

/// Stores the COMPOUND_AVERAGE of plane `plane`'s cell `lane`, predicted by
/// [`predict_items`], or returns `false` without writing.
#[allow(clippy::too_many_arguments)]
#[inline]
fn store_item(
    lanes: &[Lanes; 2],
    sources: &Sources<'_>,
    (filters, size): (&'static [Option<Taps>; NUM_PHASES], usize),
    stash: &[Stashed],
    [plane, lane]: [usize; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        return avx2::store_item(token, stash, plane, lane, size, forward, output, stride);
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(token) = fearless_simd::Level::baseline().as_neon() {
        let blocks = [0, 1].map(|reference| {
            let lanes = &lanes[reference];
            let source = sources.samples[plane & 1][reference];
            lanes
                .cell(
                    lane,
                    (source, sources.strides[reference]),
                    filters,
                    lanes.keys[lane] >= 25,
                )
                .block(size)
        });
        super::eight_bit::neon::compound(token, [&blocks[0], &blocks[1]], forward, output, stride);
        return true;
    }
    let _ = (
        lanes, sources, filters, size, stash, plane, lane, forward, output, stride,
    );
    false
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::{Cell, Lanes, NUM_PHASES, Sources, Stashed, TAPS, Taps};
    use crate::subpel_mc::InterpolationFilter;
    use crate::subpel_mc::eight_bit::avx2::{
        Horizontal, average, store_bytes, store_rows, tap_bytes,
    };
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, i16x16, i32x8, u8x32};

    /// The 16 predictors of each group of a cell; a 4x4 cell has one.
    type Groups = [__m256i; 4];

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

    /// Calls `$f::<$w, H, V, CLAMPED>` for the kernel key
    /// `5 * H + V + 25 * CLAMPED` of a reference of a `$w`-square cell, and
    /// is `false` for any other key.
    macro_rules! by_key {
        ($key:expr, 8, $f:ident $args:tt) => {
            by_key!(@ $key, 8, $f $args, [
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
                10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4, 15 3 0, 16 3 1, 17 3 2, 18 3 3,
                19 3 4, 20 4 0, 21 4 1, 22 4 2, 23 4 3, 24 4 4
            ])
        };
        ($key:expr, 4, $f:ident $args:tt) => {
            by_key!(@ $key, 4, $f $args, [
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
                10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4
            ])
        };
        (@ $key:expr, $w:literal, $f:ident $args:tt, [$($c:literal $h:literal $v:literal),*]) => {
            match $key {
                $($c => { $f::<$w, $h, $v, false> $args; true })*
                key => match key - 25 {
                    $($c => { $f::<$w, $h, $v, true> $args; true })*
                    _ => false,
                },
            }
        };
    }

    /// An output stride whose eight rows stay below `2^32`.
    #[inline]
    fn output_stride(stride: usize) -> Option<u32> {
        u32::try_from(stride)
            .ok()
            .filter(|&stride| stride <= u32::MAX / 8)
    }

    /// Stash slot of plane `plane`, cell lane `lane`, reference `reference`.
    const fn slot(plane: usize, lane: usize, reference: usize) -> usize {
        16 * plane + 2 * lane + reference
    }

    /// Predicts each reference `item >> 3` of lane `item & 7` set in `items`
    /// into `stash`, one kernel call per key the items share.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn run(
        t: Avx2,
        items: u32,
        lanes: &[Lanes; 2],
        sources: &Sources<'_>,
        filters: &'static [Option<Taps>; NUM_PHASES],
        size: usize,
        stash: &mut [Stashed],
    ) -> u32 {
        let keys: [__m256i; 2] = [
            i32x8::simd_from(t, lanes[0].keys).into(),
            i32x8::simd_from(t, lanes[1].keys).into(),
        ];
        let (mut pending, mut done) = (items, 0);
        while pending != 0 {
            let first = pending.trailing_zeros() as usize;
            let key = lanes[first >> 3 & 1].keys[first & 7];
            let matches = |keys: __m256i| {
                let equal = _mm256_cmpeq_epi32(keys, _mm256_set1_epi32(key));
                _mm256_movemask_ps(_mm256_castsi256_ps(equal)) as u32
            };
            let mask = (matches(keys[0]) | matches(keys[1]) << 8) & pending;
            pending &= !mask;
            let predicted = if size == 4 {
                by_key!(key, 4, cells(t, mask, lanes, sources, filters, stash))
            } else {
                by_key!(key, 8, cells(t, mask, lanes, sources, filters, stash))
            };
            if predicted {
                done |= mask;
            }
        }
        done
    }

    /// Predicts every item of `items`, all of one kernel key, into `stash`.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn cells<const W: usize, const H: usize, const V: usize, const CLAMPED: bool>(
        t: Avx2,
        mut items: u32,
        lanes: &[Lanes; 2],
        sources: &Sources<'_>,
        filters: &'static [Option<Taps>; NUM_PHASES],
        stash: &mut [Stashed],
    ) {
        while items != 0 {
            let item = items.trailing_zeros() as usize;
            items &= items - 1;
            let (reference, lane) = (item >> 3 & 1, item & 7);
            let lanes = &lanes[reference];
            for plane in 0..sources.planes.min(2) {
                let source = (
                    sources.samples[plane][reference],
                    sources.strides[reference],
                );
                let cell = lanes.cell(lane, source, filters, CLAMPED);
                let groups = predict::<W, H, V, false, CLAMPED>(t, &cell);
                if let Some(slot) = stash.get_mut(slot(plane, lane, reference)) {
                    for (slot, group) in slot.iter_mut().zip(groups).take(W * W / 16) {
                        i16x16::<Avx2>::simd_from(t, group).store_array(slot);
                    }
                }
            }
        }
    }

    /// Stores the COMPOUND_AVERAGE of the stashed references of `plane`'s
    /// cell `lane`.
    #[inline]
    #[target_feature(enable = "avx2")]
    #[allow(clippy::too_many_arguments)]
    fn store(
        t: Avx2,
        stash: &[Stashed],
        plane: usize,
        lane: usize,
        size: usize,
        forward: i32,
        output: &mut [u8],
        stride: u32,
    ) {
        let (Some(first), Some(second)) = (
            stash.get(slot(plane, lane, 0)),
            stash.get(slot(plane, lane, 1)),
        ) else {
            return;
        };
        let load = |values: &[i16; 16]| -> __m256i { i16x16::load_array_ref(t, values).into() };
        if size == 4 {
            let blended = average(load(&first[0]), load(&second[0]), forward);
            store_rows::<4, u32>(t, output, 0, stride, blended);
            return;
        }
        for (group, (first, second)) in (0..).zip(first.iter().zip(second)) {
            let blended = average(load(first), load(second), forward);
            store_rows::<8, u32>(t, output, 2 * group * stride, stride, blended);
        }
    }

    fearless_simd::kernel!(
        #[allow(clippy::too_many_arguments)]
        #[inline]
        pub(super) fn compound(
            t: Avx2,
            cells: [&Cell<'_>; 2],
            starts: [[i32; 2]; 2],
            interp: InterpolationFilter,
            size: usize,
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) -> bool {
            let mut lanes = [Lanes::default(), Lanes::default()];
            for ((lanes, cell), [x, y]) in lanes.iter_mut().zip(cells).zip(starts) {
                lanes.origin[0] = cell.origin as i32;
                lanes.phases = [[(x >> 6) & 15; 8], [(y >> 6) & 15; 8]];
                lanes.keys[0] = cell.key();
                for (field, value) in lanes
                    .clamped
                    .iter_mut()
                    .zip(cell.clamped.unwrap_or_default())
                {
                    field[0] = value;
                }
            }
            let sources = Sources {
                samples: [[cells[0].source, cells[1].source]; 2],
                strides: [cells[0].stride, cells[1].stride],
                planes: 1,
            };
            let filters = &TAPS[interp.pass_index(size as u32) as usize];
            let mut stash = [[[0; 16]; 4]; 2];
            let Some(stride) = output_stride(stride) else {
                return false;
            };
            if run(t, 1 | 1 << 8, &lanes, &sources, filters, size, &mut stash) != 1 | 1 << 8 {
                return false;
            }
            store(t, &stash, 0, 0, size, forward, output, stride);
            true
        }
    );

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn run_items(
            t: Avx2,
            items: u32,
            lanes: &[Lanes; 2],
            sources: &Sources<'_>,
            filters: &'static [Option<Taps>; NUM_PHASES],
            size: usize,
            stash: &mut [Stashed],
        ) -> u32 {
            run(t, items, lanes, sources, filters, size, stash)
        }
    );

    fearless_simd::kernel!(
        #[allow(clippy::too_many_arguments)]
        #[inline]
        pub(super) fn store_item(
            t: Avx2,
            stash: &[Stashed],
            plane: usize,
            lane: usize,
            size: usize,
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) -> bool {
            let Some(stride) = output_stride(stride) else {
                return false;
            };
            store(t, stash, plane, lane, size, forward, output, stride);
            true
        }
    );

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn put(t: Avx2, cell: &Cell<'_>, size: usize, output: &mut [u8], stride: usize) {
            if size == 4 {
                by_key!(cell.key(), 4, put_cell(t, cell, output, stride));
            } else {
                by_key!(cell.key(), 8, put_cell(t, cell, output, stride));
            }
        }
    );

    /// Stores the [`predict`] put of one cell.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn put_cell<const W: usize, const H: usize, const V: usize, const CLAMPED: bool>(
        t: Avx2,
        cell: &Cell<'_>,
        output: &mut [u8],
        stride: usize,
    ) {
        let groups = predict::<W, H, V, true, CLAMPED>(t, cell);
        if W == 4 {
            store_bytes::<4>(t, output, 0, stride, groups[0]);
            return;
        }
        for (group, &values) in groups.iter().enumerate() {
            store_bytes::<8>(t, output, 2 * group * stride, stride, values);
        }
    }

    /// The compound predictors of one `W`-square cell whose passes have `H`
    /// and `V` tap pairs, none for a skipped pass.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn predict<
        const W: usize,
        const H: usize,
        const V: usize,
        const PUT: bool,
        const CLAMPED: bool,
    >(
        t: Avx2,
        cell: &Cell<'_>,
    ) -> Groups {
        let per = 16 / W;
        let rows = if V > 0 { W + 2 * V - 1 } else { W };
        let zero = _mm256_setzero_si256();
        let (bounds, remap) = match cell.clamped.filter(|_| CLAMPED) {
            Some(bounds) => {
                let lanes = _mm_add_epi8(
                    _mm_setr_epi8(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
                    _mm_set1_epi8(bounds[3] as i8),
                );
                let limit = _mm_set1_epi8(bounds[4] as i8);
                (
                    bounds,
                    Some(_mm_min_epi8(
                        _mm_max_epi8(lanes, _mm_setzero_si128()),
                        limit,
                    )),
                )
            }
            None => ([0; 5], None),
        };
        let start = |row: usize| {
            if CLAMPED {
                let row = (bounds[0] + row.min(rows - 1) as i32)
                    .max(bounds[1])
                    .min(bounds[2]);
                cell.origin + row as u32 * cell.stride
            } else {
                cell.origin + row.min(rows - 1) as u32 * cell.stride
            }
        };
        let filter = match cell.horizontal {
            Some(taps) if H > 0 => Some(Horizontal::new::<W, H>(t, taps)),
            _ => None,
        };
        let narrow: __m256i = u8x32::load_array(t, NARROW_ROWS).into();
        let mut lines = [zero; 8];
        for (group, line) in lines.iter_mut().enumerate().take(rows.div_ceil(per)) {
            let row = group * per;
            let starts = [start(row), start(row + 1), start(row + 2), start(row + 3)];
            let Some(bytes) = tap_bytes::<W>(t, cell.source, starts, remap) else {
                return [zero; 4];
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
        let mut out = [zero; 4];
        let Some(taps) = cell.vertical.filter(|_| V > 0) else {
            for (out, &line) in out.iter_mut().zip(&lines) {
                *out = if H > 0 || PUT {
                    line
                } else {
                    _mm256_slli_epi16::<4>(line)
                };
            }
            return out;
        };
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
            core::array::from_fn(|pair| _mm256_set1_epi32(taps.words[pair.min(V.max(1) - 1)]));
        let bias = _mm256_set1_epi32(match (H > 0, PUT) {
            (true, true) => 1 << 9,
            (true, false) | (false, true) => 1 << 5,
            (false, false) => 1 << 1,
        });
        for (group, out) in out.iter_mut().enumerate().take(W * W / 16) {
            let (mut low, mut high) = (bias, bias);
            for (pair, &weights) in weights.iter().enumerate().take(V) {
                let (a, b) = (
                    line(group * per + 2 * pair),
                    line(group * per + 2 * pair + 1),
                );
                low =
                    _mm256_add_epi32(low, _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights));
                high = _mm256_add_epi32(
                    high,
                    _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights),
                );
            }
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
}
