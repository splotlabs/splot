// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of 4x4 and 8x8 compound cells: both
//! references and their § 7.13.3.15 COMPOUND_AVERAGE.
//!
//! A cell is a pair of [`Window`]s that the compound kernels of
//! [`super::block`] predict, in place or from a copy of a window that
//! crosses its clip bounds. One cell path serves the placement entry and the
//! programs of a grid, for eight- and 16-bit planes; on AArch64 the
//! programs of eight-bit planes run their in-place cells in one NEON kernel.

use super::block::{self, Window};
use super::{CellPlacement, InterpolationFilter, ReferencePlaneView};
use crate::format::ReconSample;
use fearless_simd::{Simd, prelude::*, u8x16, u16x8, u32x4, u64x2};

/// The cell kernels of a sample depth.
pub(super) trait CellSample: block::Sample {
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
        let geometry = [
            view.stride,
            view.width,
            view.readable_rows,
            view.samples.len(),
        ];
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
}

impl CellSample for u16 {
    #[cfg(target_arch = "aarch64")]
    #[inline]
    fn program(
        planes: &mut [Option<Plane<'_, '_, u16>>; 2],
        filter: usize,
        size: usize,
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written {
        let Some((views, ..)) = &planes[0] else {
            return [[0; PROGRAM_CELLS / 64]; 2];
        };
        let geometry = views.each_ref().map(|view| {
            [
                view.stride,
                view.width,
                view.readable_rows,
                view.samples.len(),
            ]
        });
        let mut planes = planes.each_mut().map(|plane| {
            let (views, output, stride) = plane.as_mut()?;
            Some(([views[0].samples, views[1].samples], &mut **output, *stride))
        });
        super::sixteen_bit::program(&mut planes, geometry, (filter, size), cells, clip)
    }
}

/// The COMPOUND_AVERAGE, clipped to `clip.1`, of a `size`-square cell, 4
/// or 8, of either sample depth whose references share `interp`, from each
/// reference's [`CellPlacement`]; `false` without writing for any other size
/// or an empty clip range.
#[inline]
pub(super) fn compound_at<P: CellSample>(
    references: [&ReferencePlaneView<'_, P>; 2],
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
    let windows = placements.map(|(start, _)| Window::new(interp, size, size, start));
    let bounds = placements.map(|(_, bounds)| bounds);
    block::compound(references, windows, bounds, clip, output, stride)
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

/// The COMPOUND_AVERAGE of two full-sample eight-bit `size`-square cells,
/// 4 or 8, from `origins` of `sources`, `forward` in `0..=16`.
pub(super) fn fullpel_u8(
    sources: [(&[u8], usize); 2],
    origins: [usize; 2],
    size: usize,
    forward: i32,
    output: &mut [u8],
    stride: usize,
) {
    baseline_simd!(simd => match (size, forward) {
        (4, 8) => fullpel_cell::<_, 4>(simd, sources, origins, 8, output, stride),
        (4, _) => fullpel_cell::<_, 4>(simd, sources, origins, forward, output, stride),
        (_, 8) => fullpel_cell::<_, 8>(simd, sources, origins, 8, output, stride),
        _ => fullpel_cell::<_, 8>(simd, sources, origins, forward, output, stride),
    });
}

/// [`fullpel_u8`] of 16-bit cells, clipped to `clip.1`.
pub(super) fn fullpel_u16(
    sources: [(&[u16], usize); 2],
    origins: [usize; 2],
    size: usize,
    clip: (i32, u16),
    output: &mut [u16],
    stride: usize,
) {
    let (forward, max) = clip;
    baseline_simd!(simd => match (size, forward) {
        (4, 8) => fullpel_cell_u16::<_, 4>(simd, sources, origins, (8, max), output, stride),
        (4, _) => fullpel_cell_u16::<_, 4>(simd, sources, origins, clip, output, stride),
        (_, 8) => fullpel_cell_u16::<_, 8>(simd, sources, origins, (8, max), output, stride),
        _ => fullpel_cell_u16::<_, 8>(simd, sources, origins, clip, output, stride),
    });
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

type CellPlane<'a, 'b, T, O> = (&'a [ReferencePlaneView<'b, T>; 2], &'a mut [O], usize);
type Plane<'a, 'b, P> = ([ReferencePlaneView<'b, P>; 2], &'a mut [P], usize);
pub(super) type Written = [[u64; PROGRAM_CELLS / 64]; 2];
/// A [`CompoundCells`] cell: output `[row, col]` and both placements.
pub(super) type Cell = ([u32; 2], [CellPlacement; 2]);

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

/// Runs `cells` into `planes` of one geometry: the cells the program kernel
/// leaves, each from the windows of its placements.
#[inline(never)]
fn program<P: CellSample>(
    mut planes: [Option<Plane<'_, '_, P>>; 2],
    interp: InterpolationFilter,
    size: usize,
    cells: &CompoundCells,
    clip: (i32, u16),
) -> Written {
    let filter = usize::from(interp.pass_index(size as u32));
    let len = cells.len.min(PROGRAM_CELLS);
    let mut written = P::program(&mut planes, filter, size, &cells.cells[..len], clip);
    for (index, &([row, col], placements)) in cells.cells[..len].iter().enumerate() {
        if written[0][index / 64 % 4] & 1 << (index % 64) != 0 {
            continue;
        }
        let windows =
            placements.map(|(start, _)| Window::of_filters([filter; 2], size, size, start));
        let bounds = placements.map(|(_, bounds)| bounds);
        for (slot, written) in planes.iter_mut().zip(&mut written) {
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
            if block::compound(
                [&views[0], &views[1]],
                windows,
                bounds,
                clip,
                output,
                stride,
            ) {
                written[index / 64 % 4] |= 1 << (index % 64);
            }
        }
    }
    written
}

/// Stores the COMPOUND_AVERAGE of the blocks of a 4x4 or 8x8 cell in the
/// AVX2 cell kernels, or returns `false` without writing.
#[cfg(target_arch = "x86_64")]
pub(super) fn compound_x86(
    blocks: [&block::Block<'_, u8>; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let size = blocks[0].w;
    let cell = |block: &block::Block<'_, u8>| block.w == size && block.h == size;
    (size == 4 || size == 8)
        && blocks.iter().all(|block| cell(block))
        && fearless_simd::Level::baseline()
            .as_avx2()
            .is_some_and(|token| avx2::compound(token, blocks, size, forward, output, stride))
}

/// Stores the `Clip1` single-reference prediction of a filtered 4x4 or 8x8
/// block in the AVX2 cell kernels, or returns `false` without writing.
#[cfg(target_arch = "x86_64")]
pub(super) fn put_x86(block: &block::Block<'_, u8>, output: &mut [u8], stride: usize) -> bool {
    let size = block.w;
    let filtered = block.horizontal.is_some() || block.vertical.is_some();
    let narrow = size == 4 && block.horizontal.is_some_and(|taps| taps.count > 2);
    if block.h != size || (size != 4 && size != 8) || !filtered || narrow {
        return false;
    }
    let Some(token) = fearless_simd::Level::baseline().as_avx2() else {
        return false;
    };
    avx2::addressable(block) && avx2::put(token, block, size, output, stride)
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::block::Block;
    use crate::subpel_mc::eight_bit::avx2::{
        Horizontal, average, store_bytes, store_rows, tap_bytes,
    };
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, u8x32};

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

    /// `Some` of `$f::<$w, H, V>` for the kernel key `5 * H + V` of a
    /// reference of a `$w`-square cell, and `None` for any other key.
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
                $($c => Some($f::<$w, $h, $v> $args),)*
                _ => None,
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

    /// The kernel key `5 * H + V` of a block's tap-pair counts, zero for a
    /// skipped pass.
    fn key(block: &Block<'_, u8>) -> i32 {
        (5 * block.horizontal.map_or(0, |taps| taps.count)
            + block.vertical.map_or(0, |taps| taps.count)) as i32
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
    fn groups<const W: usize, const H: usize, const V: usize>(
        t: Avx2,
        block: &Block<'_, u8>,
    ) -> Groups {
        predict::<W, H, V, false>(t, block)
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn put(
            t: Avx2,
            block: &Block<'_, u8>,
            size: usize,
            output: &mut [u8],
            stride: usize,
        ) -> bool {
            if size == 4 {
                by_key!(key(block), 4, put_cell(t, block, output, stride)).is_some()
            } else {
                by_key!(key(block), 8, put_cell(t, block, output, stride)).is_some()
            }
        }
    );

    /// Stores the [`predict`] put of one cell.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn put_cell<const W: usize, const H: usize, const V: usize>(
        t: Avx2,
        block: &Block<'_, u8>,
        output: &mut [u8],
        stride: usize,
    ) {
        let groups = predict::<W, H, V, true>(t, block);
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
    fn predict<const W: usize, const H: usize, const V: usize, const PUT: bool>(
        t: Avx2,
        block: &Block<'_, u8>,
    ) -> Groups {
        let per = 16 / W;
        let rows = if V > 0 { W + 2 * V - 1 } else { W };
        let zero = _mm256_setzero_si256();
        let start = |row: usize| block.start32(row.min(rows - 1));
        let filter = match block.horizontal {
            Some(taps) if H > 0 => Some(Horizontal::new::<W, H>(t, taps)),
            _ => None,
        };
        let narrow: __m256i = u8x32::load_array(t, NARROW_ROWS).into();
        let mut lines = [zero; 8];
        for (group, line) in lines.iter_mut().enumerate().take(rows.div_ceil(per)) {
            let row = group * per;
            let starts = [start(row), start(row + 1), start(row + 2), start(row + 3)];
            let Some(bytes) = tap_bytes::<W>(t, block.source, starts) else {
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
