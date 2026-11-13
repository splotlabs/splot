// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of 4x4 and 8x8 compound cells: both
//! references and their § 7.13.3.15 COMPOUND_AVERAGE.
//!
//! A cell is a pair of [`Window`]s that the compound kernels of
//! [`super::block`] predict, in place or from a copy of a window that
//! crosses its clip bounds. One cell path serves the placement entry and the
//! programs of a grid, for eight- and 16-bit planes: a program kernel first
//! runs the cells it covers, then each cell left takes the placement path.

use super::block::{self, Window};
use super::{CellPlacement, InterpolationFilter, ReferencePlaneView};
use crate::format::ReconSample;
use fearless_simd::{Simd, prelude::*, u8x16, u16x8, u32x4, u64x2};

/// The program kernel of a sample depth.
pub(super) trait CellSample: block::Sample {
    /// Runs the cells of a program of `(filter, size)` that the program
    /// kernel covers into `planes`, and returns their bits.
    fn program(
        planes: &mut [Option<Plane<'_, '_, Self>>; 2],
        shape: (usize, usize),
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written;
}

impl CellSample for u8 {
    #[inline]
    fn program(
        planes: &mut [Option<Plane<'_, '_, u8>>; 2],
        shape: (usize, usize),
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written {
        super::eight_bit::program(planes, shape, cells, clip)
    }
}

impl CellSample for u16 {
    #[inline]
    fn program(
        planes: &mut [Option<Plane<'_, '_, u16>>; 2],
        shape: (usize, usize),
        cells: &[Cell],
        clip: (i32, u16),
    ) -> Written {
        super::sixteen_bit::program(planes, shape, cells, clip)
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
/// Both references' views, the output and its stride of one program plane.
pub(super) type Plane<'a, 'b, P> = ([ReferencePlaneView<'b, P>; 2], &'a mut [P], usize);
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
    let shared = match &planes {
        [Some((first, ..)), Some((second, ..))] => {
            first
                .iter()
                .zip(second)
                .flat_map(|(a, b)| a.geometry().into_iter().zip(b.geometry()))
                .fold(0, |diff, (a, b)| diff | (a ^ b))
                == 0
        }
        [None, Some(_)] => false,
        [_, None] => true,
    };
    if shared {
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
    let cells = &cells.cells[..len];
    let written = P::program(&mut planes, (filter, size), cells, clip);
    cell_by_cell(&mut planes, (filter, size), cells, clip, written)
}

/// The cells of a program whose bits `written` does not set, each through
/// [`block::compound`] into every plane, and the bits of all stored cells.
pub(super) fn cell_by_cell<P: CellSample>(
    planes: &mut [Option<Plane<'_, '_, P>>; 2],
    (filter, size): (usize, usize),
    cells: &[Cell],
    clip: (i32, u16),
    mut written: Written,
) -> Written {
    for (index, &([row, col], placements)) in cells.iter().enumerate() {
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
