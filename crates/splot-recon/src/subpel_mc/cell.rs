// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of 4x4 and 8x8 blocks: both references of
//! a compound cell and their § 7.13.3.15 COMPOUND_AVERAGE, or one eight-bit
//! single-reference block and its `Clip1`.
//!
//! A cell whose references both sit at full-sample positions inside their
//! clip bounds averages its windows directly. Every other cell is a pair of
//! [`Block`]s read in place, through a row clamp and column remap for a
//! window that crosses its clip bounds, which the vector kernels of its
//! sample depth predict. One cell path serves the placement entry and the
//! programs of a grid, for eight- and 16-bit planes.

#![cfg_attr(
    not(any(target_arch = "x86_64", target_arch = "aarch64")),
    allow(dead_code, reason = "AVX2 and NEON cells only")
)]

use super::block::{Block, TAPS, Taps, even_tap_span};
use super::{
    CellPlacement, InterpolationFilter, NUM_FILTER_TYPES, NUM_PHASES, ReferencePlaneView,
    SCALE_SUBPEL_BITS, SUBPEL_FILTERS, SUBPEL_MASK, SubpelPredictParams,
};
use crate::format::ReconSample;
use fearless_simd::{Simd, prelude::*, u8x16, u16x8, u32x4, u64x2};

/// The block of one reference of a `size`-square cell of filter row
/// `filter` at 1/1024-sample `[x, y]` with clip bounds `[firstX, firstY,
/// lastX, lastY]`, or `None` when its clip range is empty or its window
/// leaves the storage of 16 samples per row (`size` for a full-sample
/// window inside its clip bounds).
#[allow(clippy::inline_always, reason = "measured compound cell hot path")]
#[inline(always)]
fn cell_block<'a, T: ReconSample>(
    view: &ReferencePlaneView<'a, T>,
    filter: usize,
    size: usize,
    ([x, y], bounds): CellPlacement,
) -> Option<Block<'a, T>> {
    let phase = |start: i32| ((start >> 6) & SUBPEL_MASK) as usize & 15;
    let (phase_x, phase_y) = (phase(x), phase(y));
    let windows = &WINDOWS[filter % NUM_FILTER_TYPES];
    let window = |start: i32, phase: usize| {
        let packed = windows[phase];
        (
            (start >> SCALE_SUBPEL_BITS) + (packed << 24 >> 24),
            size as i32 + (packed >> 8 & 0xff),
        )
    };
    let ((column, columns), (row, rows)) = (window(x, phase_x), window(y, phase_y));
    let bound = |value: i32, last: usize| value.max(0).min(last as i32 - 1);
    let [first_x, last_x] = [bound(bounds[0], view.width), bound(bounds[2], view.width)];
    let [first_y, last_y] = [
        bound(bounds[1], view.readable_rows),
        bound(bounds[3], view.readable_rows),
    ];
    if first_x > last_x || first_y > last_y {
        return None;
    }
    let fits = |row: i32, column: i32, reach: usize| {
        row as usize * view.stride + column as usize + reach <= view.samples.len()
    };
    let taps = &TAPS[filter % NUM_FILTER_TYPES];
    let block = Block {
        source: view.samples,
        stride: view.stride,
        origin: 0,
        top: 0,
        rows: [0, isize::MAX],
        w: size,
        h: size,
        horizontal: taps[phase_x].as_ref(),
        vertical: taps[phase_y].as_ref(),
        remap: None,
    };
    if column >= first_x
        && column + columns <= last_x + 1
        && row >= first_y
        && row + rows <= last_y + 1
    {
        let reach = if phase_x | phase_y == 0 { size } else { 16 };
        return fits(row + rows - 1, column, reach).then_some(Block {
            origin: row as usize * view.stride + column as usize,
            ..block
        });
    }
    let base = column.max(first_x).min(last_x);
    let remap = [(column - base).clamp(-16, 16), (last_x - base).min(15)];
    fits((row + rows - 1).max(first_y).min(last_y), base, 16).then(|| Block {
        origin: base as usize,
        top: row as isize,
        rows: [first_y as isize, last_y as isize],
        remap: Some(remap.map(|value| value as i8)),
        ..block
    })
}

/// The block of a filtered 4x4 or 8x8 single-reference block that [`put`]
/// stores, or `None` outside that subset.
#[inline]
pub(super) fn put_cell<'a>(
    reference: &ReferencePlaneView<'a, u8>,
    params: &SubpelPredictParams,
) -> Option<Block<'a, u8>> {
    let size = params.w;
    if params.h != size || (size != 4 && size != 8) {
        return None;
    }
    let filter = params.interp.pass_index(size as u32) as usize;
    let placement = ([params.start_x, params.start_y], params.bounds());
    let block = cell_block(reference, filter, size, placement)?;
    let still = block.horizontal.is_none() && block.vertical.is_none() && block.remap.is_none();
    if still || size == 4 && block.horizontal.is_some_and(|taps| taps.count > 2) {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    if fearless_simd::Level::baseline().as_avx2().is_some() && avx2::addressable(&block) {
        return Some(block);
    }
    #[cfg(target_arch = "aarch64")]
    if fearless_simd::Level::baseline().as_neon().is_some() {
        return Some(block);
    }
    None
}

/// Stores the `Clip1` single-reference prediction of a `size`-square block
/// of [`put_cell`].
pub(super) fn put(block: &Block<'_, u8>, size: usize, output: &mut [u8], stride: usize) {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        avx2::put(token, block, size, output, stride);
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(token) = fearless_simd::Level::baseline().as_neon() {
        super::eight_bit::neon::put_cell(token, block, output, stride);
    }
    let _ = (block, size, output, stride);
}

/// The cell kernels of a sample depth.
pub(super) trait CellSample: ReconSample {
    /// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of two
    /// `size`-square full-sample windows from `origins` of `sources`.
    fn fullpel(
        sources: [(&[Self], usize); 2],
        origins: [usize; 2],
        size: usize,
        clip: (i32, u16),
        output: &mut [Self],
        stride: usize,
    );

    /// Runs the cells of a program that the vector kernels read in place
    /// into `planes`, and returns their bits; none by default.
    fn program(
        _: &mut [Option<Plane<'_, '_, Self>>; 2],
        _: usize,
        _: usize,
        _: &[Cell],
        _: (i32, u16),
    ) -> Written {
        [[0; PROGRAM_CELLS / 64]; 2]
    }

    /// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of a cell's blocks,
    /// or returns `false` without writing.
    fn compound(
        blocks: [&Block<'_, Self>; 2],
        size: usize,
        clip: (i32, u16),
        output: &mut [Self],
        stride: usize,
    ) -> bool;
}

impl CellSample for u8 {
    #[cfg(target_arch = "aarch64")]
    #[inline]
    fn program(
        planes: &mut [Option<Plane<'_, '_, u8>>; 2],
        filter: usize,
        size: usize,
        cells: &[Cell],
        (forward, _): (i32, u16),
    ) -> Written {
        let (Some(token), Some((views, ..))) =
            (fearless_simd::Level::baseline().as_neon(), &planes[0])
        else {
            return [[0; PROGRAM_CELLS / 64]; 2];
        };
        let view = &views[0];
        let geometry = Geometry {
            stride: view.stride,
            width: view.width,
            rows: view.readable_rows,
            len: view.samples.len(),
        };
        let planes = planes.each_mut().map(|plane| {
            let (views, output, stride) = plane.as_mut()?;
            Some(([views[0].samples, views[1].samples], &mut **output, *stride))
        });
        let program = super::eight_bit::neon::Program {
            planes,
            geometry,
            filter,
            cells,
            forward,
        };
        super::eight_bit::neon::program(token, program, size)
    }

    #[inline]
    fn fullpel(
        sources: [(&[u8], usize); 2],
        origins: [usize; 2],
        size: usize,
        (forward, _): (i32, u16),
        output: &mut [u8],
        stride: usize,
    ) {
        baseline_simd!(simd => if size == 4 {
            fullpel_cell::<_, 4>(simd, sources, origins, forward, output, stride);
        } else {
            fullpel_cell::<_, 8>(simd, sources, origins, forward, output, stride);
        });
    }

    #[inline]
    fn compound(
        blocks: [&Block<'_, u8>; 2],
        size: usize,
        (forward, _): (i32, u16),
        output: &mut [u8],
        stride: usize,
    ) -> bool {
        compound_u8(blocks, size, forward, output, stride)
    }
}

impl CellSample for u16 {
    #[inline]
    fn fullpel(
        sources: [(&[u16], usize); 2],
        origins: [usize; 2],
        size: usize,
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) {
        baseline_simd!(simd => if size == 4 {
            fullpel_cell_u16::<_, 4>(simd, sources, origins, clip, output, stride);
        } else {
            fullpel_cell_u16::<_, 8>(simd, sources, origins, clip, output, stride);
        });
    }

    #[inline]
    fn compound(
        blocks: [&Block<'_, u16>; 2],
        size: usize,
        clip: (i32, u16),
        output: &mut [u16],
        stride: usize,
    ) -> bool {
        compound_u16(blocks, size, clip, output, stride)
    }
}

/// Stores the COMPOUND_AVERAGE, clipped to `clip.1`, of a `size`-square
/// cell whose blocks [`cell_block`] read from `views`, or returns `false`
/// without writing outside the kernels: two full-sample windows inside their
/// clip bounds with a weight in `0..=16` average directly; every other cell
/// needs 16 readable samples per row in each window.
#[allow(clippy::inline_always, reason = "measured compound cell hot path")]
#[inline(always)]
fn store<P: CellSample>(
    blocks: [&Block<'_, P>; 2],
    views: &[ReferencePlaneView<'_, P>; 2],
    size: usize,
    clip: (i32, u16),
    output: &mut [P],
    stride: usize,
) -> bool {
    let still = |block: &Block<'_, P>| {
        block.horizontal.is_none() && block.vertical.is_none() && block.remap.is_none()
    };
    if still(blocks[0]) && still(blocks[1]) && (0..=16).contains(&clip.0) {
        let sources = [
            (views[0].samples, views[0].stride),
            (views[1].samples, views[1].stride),
        ];
        let origins = blocks.map(|block| block.origin);
        P::fullpel(sources, origins, size, clip, output, stride);
        return true;
    }
    let reach = |block: &Block<'_, P>| {
        !still(block) || block.origin + (size - 1) * block.stride + 16 <= block.source.len()
    };
    reach(blocks[0]) && reach(blocks[1]) && P::compound(blocks, size, clip, output, stride)
}

/// The COMPOUND_AVERAGE of an eight-bit cell's blocks in the NEON, AVX2 or
/// portable kernels, or `false` without writing.
#[inline]
fn compound_u8(
    blocks: [&Block<'_, u8>; 2],
    size: usize,
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        return avx2::compound(token, blocks, size, forward, output, stride);
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(token) = fearless_simd::Level::baseline().as_neon() {
        super::eight_bit::neon::compound_cell(token, blocks, size, forward, output, stride);
        return true;
    }
    super::with_subpel_intermediate(stage_len(blocks, size), |stage| {
        super::portable::compound_u8(blocks, forward, stage, output, stride);
        true
    })
}

/// The stage the kernels take for the blocks of a `size`-square cell.
fn stage_len<T>(blocks: [&Block<'_, T>; 2], size: usize) -> usize {
    blocks.iter().fold(0, |len, block| {
        let rows = size + block.vertical.map_or(0, |taps| taps.span() - 1);
        len.max(super::block::stage_len(rows, size, block.vertical))
    })
}

/// The COMPOUND_AVERAGE, clipped to `max`, of a 16-bit cell's blocks in the
/// word-lane or portable kernels: `false` without writing for a
/// column-clamped window without a horizontal pass.
fn compound_u16(
    blocks: [&Block<'_, u16>; 2],
    size: usize,
    (forward, max): (i32, u16),
    output: &mut [u16],
    stride: usize,
) -> bool {
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
    if cfg!(not(target_arch = "x86_64"))
        && super::sixteen_bit::compound(blocks, forward, max, &mut [], output, stride)
    {
        return true;
    }
    super::with_subpel_intermediate(stage_len(blocks, size), |stage| {
        if !super::sixteen_bit::compound(blocks, forward, max, stage, output, stride) {
            super::portable::compound_u16(blocks, forward, max, stage, output, stride);
        }
        true
    })
}

/// Predicts both references of a 4x4 or 8x8 eight-bit cell, or of two
/// full-sample windows at least 16 wide inside their clip bounds, and stores
/// their COMPOUND_AVERAGE, or returns `false` without writing outside that
/// subset.
pub(super) fn compound(
    references: [&ReferencePlaneView<'_, u8>; 2],
    params: [&SubpelPredictParams; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let size = params[0].w;
    if size >= 16 {
        return wide_fullpel(references, params, forward, output, stride);
    }
    params[0].h == size
        && params[1].interp == params[0].interp
        && compound_at(
            [*references[0], *references[1]],
            params[0].interp,
            size,
            params.map(|params| ([params.start_x, params.start_y], params.bounds())),
            (forward, u16::from(u8::MAX)),
            output,
            stride,
        )
}

/// [`compound`] of two full-sample windows of `w >= 16` columns inside their
/// clip bounds and storage, whose rows the kernel reads as whole slices.
fn wide_fullpel(
    references: [&ReferencePlaneView<'_, u8>; 2],
    params: [&SubpelPredictParams; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    fn window<'a>(
        view: &ReferencePlaneView<'a, u8>,
        params: &SubpelPredictParams,
        [w, h]: [usize; 2],
    ) -> Option<Block<'a, u8>> {
        let fraction = SUBPEL_MASK << 6;
        let ([x, y], bounds) = ([params.start_x, params.start_y], params.bounds());
        let bound = |value: i32, last: usize| value.max(0).min(last as i32 - 1);
        let (column, row) = (x >> SCALE_SUBPEL_BITS, y >> SCALE_SUBPEL_BITS);
        let inside = (x | y) & fraction == 0
            && (params.w, params.h) == (w, h)
            && column >= bound(bounds[0], view.width)
            && column + w as i32 <= bound(bounds[2], view.width) + 1
            && row >= bound(bounds[1], view.readable_rows)
            && row + h as i32 <= bound(bounds[3], view.readable_rows) + 1;
        if !inside {
            return None;
        }
        let origin = row as usize * view.stride + column as usize;
        (origin + (h - 1) * view.stride + w <= view.samples.len()).then_some(Block {
            source: view.samples,
            stride: view.stride,
            origin,
            top: 0,
            rows: [0, isize::MAX],
            w,
            h,
            horizontal: None,
            vertical: None,
            remap: None,
        })
    }
    let (w, h) = (params[0].w, params[0].h);
    let (Some(first), Some(second)) = (
        window(references[0], params[0], [w, h]),
        window(references[1], params[1], [w, h]),
    ) else {
        return false;
    };
    super::eight_bit::compound([&first, &second], forward, &mut [], output, stride)
}

/// [`compound`] of a `size`-square cell of either sample depth whose
/// references share `interp`, from each reference's [`CellPlacement`].
#[inline]
pub(super) fn compound_at<P: CellSample>(
    references: [ReferencePlaneView<'_, P>; 2],
    interp: InterpolationFilter,
    size: usize,
    placements: [CellPlacement; 2],
    clip: (i32, u16),
    output: &mut [P],
    stride: usize,
) -> bool {
    if size != 4 && size != 8 {
        return false;
    }
    let filter = interp.pass_index(size as u32) as usize;
    let (Some(first), Some(second)) = (
        cell_block(&references[0], filter, size, placements[0]),
        cell_block(&references[1], filter, size, placements[1]),
    ) else {
        return false;
    };
    store([&first, &second], &references, size, clip, output, stride)
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

/// The most cells one [`CompoundCells`] program holds.
const PROGRAM_CELLS: usize = 256;

/// A program of unscaled compound cells of one grid plane that share a
/// size and filter: each cell's `[row, col]` output position and both
/// references' [`CellPlacement`].
#[derive(Clone)]
pub struct CompoundCells {
    len: usize,
    cells: [([u32; 2], [CellPlacement; 2]); PROGRAM_CELLS],
}

impl Default for CompoundCells {
    fn default() -> Self {
        Self {
            len: 0,
            cells: [([0; 2], [([0; 2], [0; 4]); 2]); PROGRAM_CELLS],
        }
    }
}

impl CompoundCells {
    /// Appends a cell at output `[row, col]`, or returns `false` without
    /// storing it when the program is full.
    #[inline]
    pub fn push(&mut self, [row, col]: [usize; 2], placements: [CellPlacement; 2]) -> bool {
        let (Ok(row), Ok(col)) = (u32::try_from(row), u32::try_from(col)) else {
            return false;
        };
        let Some(slot) = self.cells.get_mut(self.len) else {
            return false;
        };
        *slot = ([row, col], placements);
        self.len += 1;
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
        let ([row, col], placements) = *self.cells.get(index.min(self.len))?;
        (index < self.len).then_some(([row as usize, col as usize], placements))
    }
}

/// Per filter type and phase, the window of a pass packed as the column
/// offset `start - 3` of its first tap (low byte, signed) and its extent
/// `span - 1` (second byte); zero for a skipped pass.
static WINDOWS: [[i32; NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut table = [[0; NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 1;
        while phase < NUM_PHASES {
            let (start, taps) = even_tap_span(&SUBPEL_FILTERS[filter][phase]);
            table[filter][phase] = ((start as i32 - 3) & 0xff) | ((taps as i32 - 1) << 8);
            phase += 1;
        }
        filter += 1;
    }
    table
};

type CellPlane<'a, 'b, T, O> = (&'a [ReferencePlaneView<'b, T>; 2], &'a mut [O], usize);
type Plane<'a, 'b, P> = ([ReferencePlaneView<'b, P>; 2], &'a mut [P], usize);
pub(super) type Written = [[u64; PROGRAM_CELLS / 64]; 2];
/// A [`CompoundCells`] cell: output `[row, col]` and both placements.
pub(super) type Cell = ([u32; 2], [CellPlacement; 2]);

/// The reference geometry the planes of a program share.
#[cfg_attr(
    not(target_arch = "aarch64"),
    allow(dead_code, reason = "the NEON program kernel reads it")
)]
#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub(super) stride: usize,
    pub(super) width: usize,
    pub(super) rows: usize,
    pub(super) len: usize,
}

/// One reference of a cell read in place: window row 0, column 0 at `base`.
#[cfg_attr(
    not(target_arch = "aarch64"),
    allow(dead_code, reason = "the NEON program kernel reads it")
)]
#[derive(Clone, Copy)]
pub(super) struct Placed {
    pub(super) base: usize,
    pub(super) horizontal: Option<&'static Taps>,
    pub(super) vertical: Option<&'static Taps>,
}

/// The in-place window of a `size`-square cell reference of filter row
/// `filter`, as [`cell_block`] reads it without a row clamp or column
/// remap: `None` when its window crosses the clip bounds or the storage of
/// 16 samples per row.
#[cfg_attr(
    not(target_arch = "aarch64"),
    allow(dead_code, reason = "the NEON program kernel reads it")
)]
#[allow(clippy::inline_always, reason = "per-cell program hot path")]
#[inline(always)]
pub(super) fn placed(
    geometry: Geometry,
    filter: usize,
    size: usize,
    ([x, y], bounds): CellPlacement,
) -> Option<Placed> {
    let phase = |start: i32| ((start >> 6) & SUBPEL_MASK) as usize & 15;
    let (phase_x, phase_y) = (phase(x), phase(y));
    let windows = &WINDOWS[filter % NUM_FILTER_TYPES];
    let window = |start: i32, phase: usize| {
        let packed = windows[phase];
        (
            (start >> SCALE_SUBPEL_BITS) + (packed << 24 >> 24),
            size as i32 + (packed >> 8 & 0xff),
        )
    };
    let ((column, columns), (row, rows)) = (window(x, phase_x), window(y, phase_y));
    let bound = |value: i32, last: usize| value.max(0).min(last as i32 - 1);
    let inside = column >= bound(bounds[0], geometry.width)
        && column + columns <= bound(bounds[2], geometry.width) + 1
        && row >= bound(bounds[1], geometry.rows)
        && row + rows <= bound(bounds[3], geometry.rows) + 1;
    if !inside {
        return None;
    }
    let base = row as usize * geometry.stride + column as usize;
    let taps = &TAPS[filter % NUM_FILTER_TYPES];
    (base + (rows as usize - 1) * geometry.stride + 16 <= geometry.len).then(|| Placed {
        base,
        horizontal: taps[phase_x].as_ref(),
        vertical: taps[phase_y].as_ref(),
    })
}

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
    let clip = (i32::from(cwp_weight), bit_depth.max_sample());
    if O::u8_slice(&[]).is_some() {
        let planes = planes.each_mut().map(|plane| {
            let (views, output, stride) = plane.as_mut()?;
            let views = [views[0].as_u8()?, views[1].as_u8()?];
            (*stride >= size).then_some((views, O::u8_slice_mut(output)?, *stride))
        });
        return by_geometry(planes, |planes| program(planes, interp, size, cells, clip));
    }
    let planes = planes.each_mut().map(|plane| {
        let (views, output, stride) = plane.as_mut()?;
        let views = [views[0].as_u16()?, views[1].as_u16()?];
        (*stride >= size).then_some((views, O::u16_slice_mut(output)?, *stride))
    });
    by_geometry(planes, |planes| program(planes, interp, size, cells, clip))
}

/// Runs `run` on both planes when they share their reference geometry,
/// else on each plane alone.
fn by_geometry<P: CellSample>(
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

/// Runs `cells` into `planes` of one geometry: each cell is prepared once
/// from the first plane's references and stored into every plane.
#[inline(never)]
fn program<P: CellSample>(
    mut planes: [Option<Plane<'_, '_, P>>; 2],
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
    let filter = interp.pass_index(size as u32) as usize;
    let len = cells.len.min(PROGRAM_CELLS);
    let done = P::program(&mut planes, filter, size, &cells.cells[..len], clip);
    written = done;
    for (index, &([row, col], placements)) in cells.cells[..len].iter().enumerate() {
        if done[0][index / 64 % 4] & 1 << (index % 64) != 0 {
            continue;
        }
        let (Some(first), Some(second)) = (
            cell_block(&first_views[0], filter, size, placements[0]),
            cell_block(&first_views[1], filter, size, placements[1]),
        ) else {
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
            let stored = if plane == 0 {
                store([&first, &second], views, size, clip, output, stride)
            } else {
                let blocks = [
                    first.with_source(views[0].samples),
                    second.with_source(views[1].samples),
                ];
                store([&blocks[0], &blocks[1]], views, size, clip, output, stride)
            };
            if stored {
                written[index / 64 % 4] |= 1 << (index % 64);
            }
        }
    }
    written
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::Block;
    use crate::subpel_mc::eight_bit::avx2::{
        Horizontal, average, store_bytes, store_rows, tap_bytes,
    };
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, u8x16, u8x32};

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

    /// `Some` of `$f::<$w, H, V, CLAMPED>` for the kernel key
    /// `5 * H + V + 25 * CLAMPED` of a reference of a `$w`-square cell, and
    /// `None` for any other key.
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
                $($c => Some($f::<$w, $h, $v, false> $args),)*
                key => match key - 25 {
                    $($c => Some($f::<$w, $h, $v, true> $args),)*
                    _ => None,
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

    /// Whether the `u32` window starts of the kernels address `block`.
    pub(super) fn addressable<T>(block: &Block<'_, T>) -> bool {
        u32::try_from(block.source.len()).is_ok()
    }

    /// The kernel key `5 * H + V + 25 * clamped` of a block's tap-pair
    /// counts, zero for a skipped pass.
    fn key(block: &Block<'_, u8>) -> i32 {
        (5 * block.horizontal.map_or(0, |taps| taps.count)
            + block.vertical.map_or(0, |taps| taps.count)
            + 25 * usize::from(block.remap.is_some())) as i32
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
            let Some(stride) =
                output_stride(stride).filter(|_| blocks.iter().all(|block| addressable(block)))
            else {
                return false;
            };
            let groups = |block: &Block<'_, u8>| {
                if size == 4 {
                    by_key!(key(block), 4, groups(t, block))
                } else {
                    by_key!(key(block), 8, groups(t, block))
                }
            };
            let (Some(first), Some(second)) = (groups(blocks[0]), groups(blocks[1])) else {
                return false;
            };
            if size == 4 {
                store_rows::<4, u32>(t, output, 0, stride, average(first[0], second[0], forward));
                return true;
            }
            for (group, (first, second)) in (0..).zip(first.into_iter().zip(second)) {
                let blended = average(first, second, forward);
                store_rows::<8, u32>(t, output, 2 * group * stride, stride, blended);
            }
            true
        }
    );

    /// The compound [`predict`] of one block.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn groups<const W: usize, const H: usize, const V: usize, const CLAMPED: bool>(
        t: Avx2,
        block: &Block<'_, u8>,
    ) -> Groups {
        predict::<W, H, V, false, CLAMPED>(t, block)
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn put(
            t: Avx2,
            block: &Block<'_, u8>,
            size: usize,
            output: &mut [u8],
            stride: usize,
        ) {
            let _ = if size == 4 {
                by_key!(key(block), 4, put_cell(t, block, output, stride))
            } else {
                by_key!(key(block), 8, put_cell(t, block, output, stride))
            };
        }
    );

    /// Stores the [`predict`] put of one cell.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn put_cell<const W: usize, const H: usize, const V: usize, const CLAMPED: bool>(
        t: Avx2,
        block: &Block<'_, u8>,
        output: &mut [u8],
        stride: usize,
    ) {
        let groups = predict::<W, H, V, true, CLAMPED>(t, block);
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
        block: &Block<'_, u8>,
    ) -> Groups {
        let per = 16 / W;
        let rows = if V > 0 { W + 2 * V - 1 } else { W };
        let zero = _mm256_setzero_si256();
        let remap = block
            .remap_lanes()
            .filter(|_| CLAMPED)
            .map(|remap| -> __m128i { u8x16::load_array(t, remap).into() });
        let start = |row: usize| {
            let row = row.min(rows - 1);
            if CLAMPED {
                block.start32(row)
            } else {
                block.origin as u32 + row as u32 * block.stride as u32
            }
        };
        let filter = match block.horizontal {
            Some(taps) if H > 0 => Some(Horizontal::new::<W, H>(t, taps)),
            _ => None,
        };
        let narrow: __m256i = u8x32::load_array(t, NARROW_ROWS).into();
        let mut lines = [zero; 8];
        for (group, line) in lines.iter_mut().enumerate().take(rows.div_ceil(per)) {
            let row = group * per;
            let starts = [start(row), start(row + 1), start(row + 2), start(row + 3)];
            let Some(bytes) = tap_bytes::<W>(t, block.source, starts, remap) else {
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
        let Some(taps) = block.vertical.filter(|_| V > 0) else {
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
