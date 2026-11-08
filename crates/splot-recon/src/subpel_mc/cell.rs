// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of 4x4 and 8x8 eight-bit blocks: both
//! references of a compound cell and their § 7.13.3.15 COMPOUND_AVERAGE, or
//! one single-reference block and its `Clip1`.
//!
//! A block whose windows lie inside their clip bounds and storage runs every
//! pass in registers, with the arithmetic of the byte-lane kernels: a
//! horizontal pass is `Round2(sum, InterRound0)` of halved taps (or the
//! single-pass put `Round2`), a vertical-only pass weighs the samples by the
//! halved taps, and a two-pass vertical rounds the intermediates. Every other
//! block takes the general path.

use super::block::{TAPS, Taps, even_tap_span};
use super::{
    CellPlacement, InterpolationFilter, NUM_FILTER_TYPES, NUM_PHASES, ReferencePlaneView,
    SCALE_SUBPEL_BITS, SUBPEL_FILTERS, SUBPEL_MASK, SubpelPredictParams,
};
use crate::format::ReconSample;

/// One reference of a cell: window row `r` starts at `origin + r * stride`,
/// or, for a window that crosses its clip bounds, at the bounded row
/// `Clip3(clamped[1], clamped[2], clamped[0] + r)` and window column `c`
/// at `Clip3(0, clamped[4], c + clamped[3])` from `origin`.
#[derive(Clone, Copy)]
pub(super) struct Cell<'a> {
    source: &'a [u8],
    origin: usize,
    stride: usize,
    horizontal: Option<&'static Taps>,
    vertical: Option<&'static Taps>,
    clamped: Option<[i32; 5]>,
}

impl<'a> Cell<'a> {
    /// The cell of a `size`-square block, or `None` when its clip range is
    /// empty or its window leaves the storage of 16 bytes per row.
    #[allow(clippy::inline_always, reason = "measured compound cell hot path")]
    #[inline(always)]
    fn new(
        view: &ReferencePlaneView<'a, u8>,
        params: &SubpelPredictParams,
        size: usize,
    ) -> Option<Self> {
        Self::at(
            view,
            params.interp,
            size,
            ([params.start_x, params.start_y], params.bounds()),
        )
    }

    /// [`Self::new`] of a cell at 1/1024-sample `[x, y]` with clip bounds
    /// `[firstX, firstY, lastX, lastY]`.
    #[allow(clippy::inline_always, reason = "measured compound cell hot path")]
    #[inline(always)]
    fn at(
        view: &ReferencePlaneView<'a, u8>,
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
            origin,
            stride: view.stride,
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
    let cell = Cell::new(reference, params, size)?;
    if size == 4 && cell.horizontal.is_some_and(|taps| taps.count > 2) {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    if fearless_simd::Level::baseline().as_avx2().is_some() {
        return Some(cell);
    }
    None
}

impl<'a> Cell<'a> {
    /// The tap-pair counts of the horizontal and vertical passes, zero for
    /// a skipped pass.
    #[inline]
    fn counts(&self) -> [usize; 2] {
        [
            self.horizontal.map_or(0, |taps| taps.count),
            self.vertical.map_or(0, |taps| taps.count),
        ]
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
        let _ = (size, output, stride);
    }
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
        && (size == 4 || size == 8)
        && match (
            Cell::new(references[0], params[0], size),
            Cell::new(references[1], params[1], size),
        ) {
            (Some(first), Some(second)) => {
                compound_cells([first, second], size, forward, output, stride)
            }
            _ => false,
        }
}

/// [`compound`] of a `size`-square cell whose references share `interp`,
/// from each reference's [`CellPlacement`].
#[inline]
pub(super) fn compound_at(
    references: [&ReferencePlaneView<'_, u8>; 2],
    interp: InterpolationFilter,
    size: usize,
    placements: [CellPlacement; 2],
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    (size == 4 || size == 8)
        && match (
            Cell::at(references[0], interp, size, placements[0]),
            Cell::at(references[1], interp, size, placements[1]),
        ) {
            (Some(first), Some(second)) => {
                compound_cells([first, second], size, forward, output, stride)
            }
            _ => false,
        }
}

/// Stores the COMPOUND_AVERAGE of two `size`-square cells, or returns
/// `false` without writing for a 4x4 cell outside the narrow kernel.
#[inline]
fn compound_cells(
    [first, second]: [Cell<'_>; 2],
    size: usize,
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    if size == 4
        && [&first, &second]
            .iter()
            .any(|cell| cell.horizontal.is_some_and(|taps| taps.count > 2))
    {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        avx2::compound(token, [&first, &second], size, forward, output, stride);
        return true;
    }
    let _ = (first, second, forward, output, stride);
    false
}

/// The most cells one [`CompoundCells`] program holds.
const PROGRAM_CELLS: usize = 64;

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
}

impl Default for CompoundCells {
    fn default() -> Self {
        Self {
            len: 0,
            positions: [[0; 2]; PROGRAM_CELLS],
            starts: [[[0; PROGRAM_CELLS]; 2]; 2],
            bounds: [[[0; PROGRAM_CELLS]; 4]; 2],
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
    pub fn get(&self, index: usize) -> Option<([usize; 2], [CellPlacement; 2])> {
        (index < self.len.min(PROGRAM_CELLS)).then(|| {
            let [row, col] = self.positions[index];
            (
                [row as usize, col as usize],
                [0, 1].map(|reference| self.placement(index, reference)),
            )
        })
    }

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

/// Runs a [`CompoundCells`] program of eight-bit `size`-square cells into
/// `output` rows `stride` apart. Each cell that
/// [`super::subpel_compound_cell_u8`] would store from its position and
/// placements is stored the same way and sets its bit in the returned mask;
/// every other cell is left unwritten for the caller to predict.
#[doc(hidden)]
#[inline]
pub fn subpel_compound_cells_u8<T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    interp: InterpolationFilter,
    size: usize,
    cells: &CompoundCells,
    cwp_weight: i16,
    output: &mut [u8],
    stride: usize,
) -> u64 {
    let [Some(first), Some(second)] = references.map(ReferencePlaneView::as_u8) else {
        return 0;
    };
    if (size != 4 && size != 8) || stride < size {
        return 0;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = fearless_simd::Level::baseline().as_avx2() {
        return avx2::program(
            token,
            [&first, &second],
            interp,
            size,
            cells,
            i32::from(cwp_weight),
            output,
            stride,
        );
    }
    let _ = (first, second, interp, cells, cwp_weight, output);
    0
}

/// [`compound_at`] of program cell `index`, out of the program's loop.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn program_cell(
    references: [&ReferencePlaneView<'_, u8>; 2],
    interp: InterpolationFilter,
    size: usize,
    cells: &CompoundCells,
    index: usize,
    forward: i32,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let placements = [cells.placement(index, 0), cells.placement(index, 1)];
    compound_at(
        references, interp, size, placements, forward, output, stride,
    )
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::{Cell, CompoundCells, PROGRAM_CELLS, TAPS, WINDOWS};
    use crate::subpel_mc::eight_bit::avx2::{Horizontal, average, store_bytes, tap_bytes};
    use crate::subpel_mc::{InterpolationFilter, ReferencePlaneView};
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, i32x8, u8x32};

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

    /// Calls `$f::<$w, H, V, $put, CLAMPED>` for the tap-pair counts of a
    /// cell's passes and whether its window is clamped.
    macro_rules! by_taps {
        ($w:literal, $put:literal, $h:expr, $v:expr, $clamped:expr, $f:ident($($arg:expr),*)) => {
            if $clamped {
                by_taps!(@ $w, $put, true, $h, $v, $f($($arg),*))
            } else {
                by_taps!(@ $w, $put, false, $h, $v, $f($($arg),*))
            }
        };
        (@ $w:literal, $put:literal, $c:literal, $h:expr, $v:expr, $f:ident($($arg:expr),*)) => {
            match ($h, $v) {
                (0, 0) => $f::<$w, 0, 0, $put, $c>($($arg),*),
                (0, 1) => $f::<$w, 0, 1, $put, $c>($($arg),*),
                (0, 2) => $f::<$w, 0, 2, $put, $c>($($arg),*),
                (0, 3) => $f::<$w, 0, 3, $put, $c>($($arg),*),
                (0, _) => $f::<$w, 0, 4, $put, $c>($($arg),*),
                (1, 0) => $f::<$w, 1, 0, $put, $c>($($arg),*),
                (1, 1) => $f::<$w, 1, 1, $put, $c>($($arg),*),
                (1, 2) => $f::<$w, 1, 2, $put, $c>($($arg),*),
                (1, 3) => $f::<$w, 1, 3, $put, $c>($($arg),*),
                (1, _) => $f::<$w, 1, 4, $put, $c>($($arg),*),
                (2, 0) => $f::<$w, 2, 0, $put, $c>($($arg),*),
                (2, 1) => $f::<$w, 2, 1, $put, $c>($($arg),*),
                (2, 2) => $f::<$w, 2, 2, $put, $c>($($arg),*),
                (2, 3) => $f::<$w, 2, 3, $put, $c>($($arg),*),
                (2, _) => $f::<$w, 2, 4, $put, $c>($($arg),*),
                (3, 0) => $f::<$w, 3, 0, $put, $c>($($arg),*),
                (3, 1) => $f::<$w, 3, 1, $put, $c>($($arg),*),
                (3, 2) => $f::<$w, 3, 2, $put, $c>($($arg),*),
                (3, 3) => $f::<$w, 3, 3, $put, $c>($($arg),*),
                (3, _) => $f::<$w, 3, 4, $put, $c>($($arg),*),
                (_, 0) => $f::<$w, 4, 0, $put, $c>($($arg),*),
                (_, 1) => $f::<$w, 4, 1, $put, $c>($($arg),*),
                (_, 2) => $f::<$w, 4, 2, $put, $c>($($arg),*),
                (_, 3) => $f::<$w, 4, 3, $put, $c>($($arg),*),
                (_, _) => $f::<$w, 4, 4, $put, $c>($($arg),*),
            }
        };
    }

    /// [`by_taps`] of a 4x4 cell, whose horizontal pass has at most two
    /// tap pairs.
    macro_rules! by_narrow_taps {
        ($put:literal, $h:expr, $v:expr, $clamped:expr, $f:ident($($arg:expr),*)) => {
            if $clamped {
                by_narrow_taps!(@ $put, true, $h, $v, $f($($arg),*))
            } else {
                by_narrow_taps!(@ $put, false, $h, $v, $f($($arg),*))
            }
        };
        (@ $put:literal, $c:literal, $h:expr, $v:expr, $f:ident($($arg:expr),*)) => {
            match ($h, $v) {
                (0, 0) => $f::<4, 0, 0, $put, $c>($($arg),*),
                (0, 1) => $f::<4, 0, 1, $put, $c>($($arg),*),
                (0, 2) => $f::<4, 0, 2, $put, $c>($($arg),*),
                (0, 3) => $f::<4, 0, 3, $put, $c>($($arg),*),
                (0, _) => $f::<4, 0, 4, $put, $c>($($arg),*),
                (1, 0) => $f::<4, 1, 0, $put, $c>($($arg),*),
                (1, 1) => $f::<4, 1, 1, $put, $c>($($arg),*),
                (1, 2) => $f::<4, 1, 2, $put, $c>($($arg),*),
                (1, 3) => $f::<4, 1, 3, $put, $c>($($arg),*),
                (1, _) => $f::<4, 1, 4, $put, $c>($($arg),*),
                (_, 0) => $f::<4, 2, 0, $put, $c>($($arg),*),
                (_, 1) => $f::<4, 2, 1, $put, $c>($($arg),*),
                (_, 2) => $f::<4, 2, 2, $put, $c>($($arg),*),
                (_, 3) => $f::<4, 2, 3, $put, $c>($($arg),*),
                (_, _) => $f::<4, 2, 4, $put, $c>($($arg),*),
            }
        };
    }

    /// Calls `$f::<8, H, V, false, false>` for the kernel class `5 * H + V`
    /// of an unclamped 8x8 cell.
    macro_rules! by_class {
        ($class:expr, $f:ident($($arg:expr),*)) => {
            by_class!(@ $class, $f($($arg),*), [
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
                10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4, 15 3 0, 16 3 1, 17 3 2, 18 3 3,
                19 3 4, 20 4 0, 21 4 1, 22 4 2, 23 4 3
            ], 4 4, 8)
        };
        (@ $class:expr, $f:ident $args:tt, [$($c:literal $h:literal $v:literal),*], $lh:literal $lv:literal, $w:literal) => {
            match $class {
                $($c => $f::<$w, $h, $v, false, false> $args,)*
                _ => $f::<$w, $lh, $lv, false, false> $args,
            }
        };
    }

    /// [`by_class`] of an unclamped 4x4 cell, whose horizontal pass has at
    /// most two tap pairs.
    macro_rules! by_narrow_class {
        ($class:expr, $f:ident($($arg:expr),*)) => {
            by_class!(@ $class, $f($($arg),*), [
                0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
                10 2 0, 11 2 1, 12 2 2, 13 2 3
            ], 2 4, 4)
        };
    }

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn compound(
            t: Avx2,
            cells: [&Cell<'_>; 2],
            size: usize,
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) {
            let ([h0, v0], [h1, v1]) = (cells[0].counts(), cells[1].counts());
            if size == 4 {
                let first = by_narrow_taps!(
                    false,
                    h0,
                    v0,
                    cells[0].clamped.is_some(),
                    predicted(t, cells[0])
                );
                let second = by_narrow_taps!(
                    false,
                    h1,
                    v1,
                    cells[1].clamped.is_some(),
                    predicted(t, cells[1])
                );
                store_bytes::<4>(t, output, 0, stride, average(first[0], second[0], forward));
                return;
            }
            let first = by_taps!(
                8,
                false,
                h0,
                v0,
                cells[0].clamped.is_some(),
                predicted(t, cells[0])
            );
            let second = by_taps!(
                8,
                false,
                h1,
                v1,
                cells[1].clamped.is_some(),
                predicted(t, cells[1])
            );
            for (group, (&first, &second)) in first.iter().zip(&second).enumerate() {
                let blended = average(first, second, forward);
                store_bytes::<8>(t, output, 2 * group * stride, stride, blended);
            }
        }
    );

    /// The kernel inputs of eight program cells of one reference: window
    /// origins, tap phases per pass, the kernel class `5 * H + V` of their
    /// tap-pair counts, and a bit per cell whose window lies inside its clip
    /// bounds and storage and that the kernels of its size cover.
    #[derive(Default)]
    struct Lanes {
        origin: [i32; 8],
        phases: [[i32; 8]; 2],
        classes: [i32; 8],
        inside: u32,
    }

    /// The [`Lanes`] of `reference` for program cells `batch..batch + 8`:
    /// [`Cell::at`] of eight cells at once, keeping only the unclamped
    /// windows; every other cell has no bit.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn lanes(
        t: Avx2,
        view: &ReferencePlaneView<'_, u8>,
        windows: [__m256i; 2],
        size: usize,
        cells: &CompoundCells,
        reference: usize,
        batch: usize,
    ) -> Lanes {
        let zero = _mm256_setzero_si256();
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
        let load = |values: &[i32; PROGRAM_CELLS]| -> __m256i {
            values
                .get(batch..)
                .and_then(<[i32]>::first_chunk::<8>)
                .map_or(zero, |values| i32x8::simd_from(t, *values).into())
        };
        let array = |values: __m256i| -> [i32; 8] { *i32x8::<Avx2>::simd_from(t, values) };
        let (starts, bounds) = (&cells.starts[reference & 1], &cells.bounds[reference & 1]);
        let (x, y) = (load(&starts[0]), load(&starts[1]));
        let fifteen = _mm256_set1_epi32(15);
        let phases = [
            _mm256_and_si256(_mm256_srai_epi32::<6>(x), fifteen),
            _mm256_and_si256(_mm256_srai_epi32::<6>(y), fifteen),
        ];
        let window = |phase: __m256i| {
            let low = _mm256_permutevar8x32_epi32(windows[0], phase);
            let high = _mm256_permutevar8x32_epi32(windows[1], phase);
            _mm256_castps_si256(_mm256_blendv_ps(
                _mm256_castsi256_ps(low),
                _mm256_castsi256_ps(high),
                _mm256_castsi256_ps(_mm256_slli_epi32::<28>(phase)),
            ))
        };
        let (wx, wy) = (window(phases[0]), window(phases[1]));
        let reach = _mm256_set1_epi32(size as i32);
        let span = |base: __m256i, window: __m256i| {
            let first = _mm256_add_epi32(
                base,
                _mm256_srai_epi32::<24>(_mm256_slli_epi32::<24>(window)),
            );
            let extent = _mm256_and_si256(_mm256_srli_epi32::<8>(window), _mm256_set1_epi32(0xff));
            (
                first,
                _mm256_add_epi32(first, _mm256_add_epi32(reach, extent)),
            )
        };
        let (column, column_end) = span(_mm256_srai_epi32::<10>(x), wx);
        let (row, row_end) = span(_mm256_srai_epi32::<10>(y), wy);
        let bound = |values: &[i32; PROGRAM_CELLS], last: i32| {
            _mm256_min_epi32(
                _mm256_max_epi32(load(values), zero),
                _mm256_set1_epi32(last),
            )
        };
        let one = _mm256_set1_epi32(1);
        let [first_x, first_y] = [bound(&bounds[0], width - 1), bound(&bounds[1], rows - 1)];
        let last_x = _mm256_add_epi32(bound(&bounds[2], width - 1), one);
        let last_y = _mm256_add_epi32(bound(&bounds[3], rows - 1), one);
        let stride = _mm256_set1_epi32(stride);
        let end = _mm256_add_epi32(
            _mm256_mullo_epi32(_mm256_sub_epi32(row_end, one), stride),
            _mm256_add_epi32(column, _mm256_set1_epi32(16)),
        );
        let [horizontal, vertical] = [_mm256_srli_epi32::<16>(wx), _mm256_srli_epi32::<16>(wy)];
        let narrow = _mm256_set1_epi32(if size == 4 { 2 } else { 4 });
        let outside = _mm256_or_si256(
            _mm256_or_si256(
                _mm256_or_si256(
                    _mm256_cmpgt_epi32(first_x, column),
                    _mm256_cmpgt_epi32(column_end, last_x),
                ),
                _mm256_cmpgt_epi32(horizontal, narrow),
            ),
            _mm256_or_si256(
                _mm256_or_si256(
                    _mm256_cmpgt_epi32(first_y, row),
                    _mm256_cmpgt_epi32(row_end, last_y),
                ),
                _mm256_cmpgt_epi32(end, _mm256_set1_epi32(len)),
            ),
        );
        Lanes {
            origin: array(_mm256_add_epi32(_mm256_mullo_epi32(row, stride), column)),
            phases: [array(phases[0]), array(phases[1])],
            classes: array(_mm256_add_epi32(
                _mm256_add_epi32(_mm256_slli_epi32::<2>(horizontal), horizontal),
                vertical,
            )),
            inside: !(_mm256_movemask_ps(_mm256_castsi256_ps(outside)) as u32) & 0xff,
        }
    }

    fearless_simd::kernel!(
        #[allow(clippy::too_many_arguments)]
        #[inline(never)]
        pub(super) fn program(
            t: Avx2,
            views: [&ReferencePlaneView<'_, u8>; 2],
            interp: InterpolationFilter,
            size: usize,
            cells: &CompoundCells,
            forward: i32,
            output: &mut [u8],
            stride: usize,
        ) -> u64 {
            let filter = interp.pass_index(size as u32) as usize;
            let (filters, windows) = (&TAPS[filter], &WINDOWS[filter]);
            let half = |half: usize| -> __m256i {
                windows
                    .get(8 * half..)
                    .and_then(<[i32]>::first_chunk::<8>)
                    .map_or(_mm256_setzero_si256(), |values| {
                        i32x8::simd_from(t, *values).into()
                    })
            };
            let windows = [half(0), half(1)];
            let len = cells.len.min(PROGRAM_CELLS);
            let mut written = 0u64;
            for batch in (0..len).step_by(8) {
                let lanes = [0, 1].map(|reference| {
                    lanes(t, views[reference], windows, size, cells, reference, batch)
                });
                let inside = lanes[0].inside & lanes[1].inside;
                for lane in 0..(len - batch).min(8) {
                    let index = batch + lane;
                    let [row, col] = cells.positions[index];
                    let at = row as usize * stride + col as usize;
                    let Some(output) = output
                        .get_mut(at..)
                        .filter(|output| (size - 1) * stride + size <= output.len())
                    else {
                        continue;
                    };
                    if inside >> lane & 1 == 0 {
                        let stored = super::program_cell(
                            views, interp, size, cells, index, forward, output, stride,
                        );
                        written |= u64::from(stored) << index;
                        continue;
                    }
                    let cell = |reference: usize| {
                        let (view, lanes) = (views[reference], &lanes[reference]);
                        let taps =
                            |pass: usize| filters[lanes.phases[pass][lane] as usize & 15].as_ref();
                        Cell {
                            source: view.samples,
                            origin: lanes.origin[lane] as usize,
                            stride: view.stride,
                            horizontal: taps(0),
                            vertical: taps(1),
                            clamped: None,
                        }
                    };
                    let (first, second) = (cell(0), cell(1));
                    let classes = [lanes[0].classes[lane], lanes[1].classes[lane]];
                    if size == 4 {
                        let first = by_narrow_class!(classes[0], predicted(t, &first));
                        let second = by_narrow_class!(classes[1], predicted(t, &second));
                        store_bytes::<4>(
                            t,
                            output,
                            0,
                            stride,
                            average(first[0], second[0], forward),
                        );
                    } else {
                        let first = by_class!(classes[0], predicted(t, &first));
                        let second = by_class!(classes[1], predicted(t, &second));
                        for (group, (&first, &second)) in first.iter().zip(&second).enumerate() {
                            let blended = average(first, second, forward);
                            store_bytes::<8>(t, output, 2 * group * stride, stride, blended);
                        }
                    }
                    written |= 1 << index;
                }
            }
            written
        }
    );

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn put(t: Avx2, cell: &Cell<'_>, size: usize, output: &mut [u8], stride: usize) {
            let [h, v] = cell.counts();
            if size == 4 {
                by_narrow_taps!(
                    true,
                    h,
                    v,
                    cell.clamped.is_some(),
                    put_cell(t, cell, output, stride)
                );
            } else {
                by_taps!(
                    8,
                    true,
                    h,
                    v,
                    cell.clamped.is_some(),
                    put_cell(t, cell, output, stride)
                );
            }
        }
    );

    /// Stores the [`predict`] put of one cell.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn put_cell<
        const W: usize,
        const H: usize,
        const V: usize,
        const PUT: bool,
        const CLAMPED: bool,
    >(
        t: Avx2,
        cell: &Cell<'_>,
        output: &mut [u8],
        stride: usize,
    ) {
        let groups = predict::<W, H, V, PUT, CLAMPED>(t, cell);
        if W == 4 {
            store_bytes::<4>(t, output, 0, stride, groups[0]);
            return;
        }
        for (group, &values) in groups.iter().enumerate() {
            store_bytes::<8>(t, output, 2 * group * stride, stride, values);
        }
    }

    /// [`predict`] out of line, for the two references of a compound cell.
    #[inline(never)]
    #[target_feature(enable = "avx2")]
    fn predicted<
        const W: usize,
        const H: usize,
        const V: usize,
        const PUT: bool,
        const CLAMPED: bool,
    >(
        t: Avx2,
        cell: &Cell<'_>,
    ) -> Groups {
        predict::<W, H, V, PUT, CLAMPED>(t, cell)
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
                cell.origin + row as usize * cell.stride
            } else {
                cell.origin + row.min(rows - 1) * cell.stride
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
