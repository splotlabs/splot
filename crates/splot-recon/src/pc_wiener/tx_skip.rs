// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.20.4 `get_tx_skip` sums of the 6x6 windows of a classification grid.

use fearless_simd::{SimdBase, prelude::*, u8x16};

use super::{
    PC_WIENER_LEAD, PcWienerClassifyParams, PcWienerTxSkipRun, check_tx_skip_cells, coordinate_add,
    usize_to_isize,
};
use crate::{ReconError, Result};

/// Cells one vector covers.
const LANES: usize = 16;

/// The row stride of the sums [`window_sums`] writes for `cell_cols` cells.
pub(super) fn sums_stride(cell_cols: usize) -> usize {
    cell_cols.div_ceil(LANES) * LANES
}

/// Writes the window tx-skip sum of every cell into `buffer` and returns
/// where they start; row `r` is at `r * sums_stride(cell_cols)` from there.
///
/// `get_tx_skip` clips x to `[BlockStartX, BlockEndX]` before `>> 2`, and the
/// clip commutes with the shift, so the six feature columns of the cell at
/// grid column `g` read columns `g - 1`, `g`, `g + 1` clipped to its block,
/// weighted by `x & 3`. Each grid row's weighted column sums are built once
/// from a run clipped to the span's outer bounds; with `split_blocks`, a
/// cell on an inner 64-sample block edge takes its own column for the
/// neighbor outside its block. A cell's sum then adds the rows of its six
/// feature rows, each clipped to the stripe, then to the tile; clipped rows
/// never decrease down the grid, so the column sums of each grid row are
/// built once.
#[allow(clippy::too_many_arguments)]
pub(super) fn window_sums<'g, FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    split_blocks: bool,
    tx_skip: &mut FT,
    buffer: &mut Vec<u8>,
) -> Result<usize>
where
    FT: FnMut(PcWienerTxSkipRun, &mut Vec<u8>) -> Result<Option<&'g [u8]>>,
{
    let bound = |value: usize| usize_to_isize(value, "PC-Wiener tx-skip bounds");
    let stride = sums_stride(cell_cols);
    let first = params.x >> 2;
    let [lo, hi] = [
        bound(params.block_start_x >> 2)?,
        bound(params.block_end_x >> 2)?,
    ];
    let col = (first - 1).clamp(lo, hi);
    let last = coordinate_add(first, bound(cell_cols)?, "PC-Wiener tx-skip x")?.clamp(lo, hi);
    let len = (last - col) as usize + 1;
    let pre = (col - (first - 1)).max(0) as usize;
    let phase = (params.x & 3) as u8;
    let left = u8::from(phase == 0);
    let weights = [left, 5 - phase - left, phase + 1];
    let row_of = |y: isize| {
        let y = (y
            .max(bound(params.luma_stripe_start_y)?)
            .min(bound(params.luma_stripe_end_y)?) as usize)
            .max(params.tile_start_y)
            .min(params.tile_end_y);
        Ok::<_, ReconError>(y)
    };
    let y0 = coordinate_add(params.y, -PC_WIENER_LEAD, "PC-Wiener tx-skip y")?;
    let y_end = coordinate_add(y0, bound(4 * cell_rows + 1)?, "PC-Wiener tx-skip y")?;
    let first_row = row_of(y0)? >> 2;
    let row_count = (row_of(y_end)? >> 2) - first_row + 1;
    let padded = stride + 2;
    let sums_start = padded + row_count * stride;
    let total = sums_start + cell_rows * stride;
    let missing = || ReconError::WorkspaceAllocationFailed {
        plane: crate::PlaneId::Y,
        context: "PC-Wiener tx-skip sums",
    };
    buffer
        .try_reserve(total.saturating_sub(buffer.len()))
        .map_err(|_| missing())?;
    buffer.resize(total, 0);
    let (clipped, rest) = buffer.split_at_mut(padded);
    let (column_sums, sums) = rest.split_at_mut(row_count * stride);
    let mut fetched = Vec::new();
    for (index, sums_row) in column_sums.chunks_exact_mut(stride).enumerate() {
        let row = first_row + index;
        let run = PcWienerTxSkipRun {
            x: (4 * col as usize).max(params.block_start_x),
            y: (4 * row).max(row_of(y0)?),
            row,
            col: col as usize,
            len,
        };
        let cells = match tx_skip(run, &mut fetched)? {
            Some(cells) => cells,
            None => &fetched,
        };
        let cells = cells.get(..len).ok_or(ReconError::BufferLengthMismatch {
            expected: len,
            actual: cells.len(),
        })?;
        let pre = pre.min(padded);
        let take = len.min(padded - pre);
        clipped[..pre].fill(cells[0]);
        clipped[pre..pre + take].copy_from_slice(&cells[..take]); // splot-copy-ok: edge-replicated run every lane loads
        clipped[pre + take..].fill(cells[take.max(1) - 1]);
        if crate::lr_madd::lr_simd!(simd => column_row(simd, clipped, sums_row, weights, split_blocks.then_some(first)))
        {
            check_tx_skip_cells(run, cells)?;
        }
    }
    let column_sums: &[u8] = column_sums;
    for (cell_row, out) in sums.chunks_exact_mut(stride).enumerate() {
        let mut taps = [0; 6];
        for (k, tap) in taps.iter_mut().enumerate() {
            let y = y0 + (4 * cell_row + k) as isize;
            *tap = ((row_of(y)? >> 2) - first_row) * stride;
        }
        crate::lr_madd::lr_simd!(simd => add_rows(simd, column_sums, taps, out));
    }
    Ok(sums_start)
}

/// One grid row's weighted column sums from its edge-replicated run
/// `clipped` (cell `c` at `clipped[c + 1]`); returns whether a value
/// exceeds 1.
#[inline]
fn column_row<S: fearless_simd::Simd>(
    simd: S,
    clipped: &[u8],
    sums: &mut [u8],
    [left_weight, center_weight, right_weight]: [u8; 3],
    split_first: Option<isize>,
) -> bool {
    const IOTA: [u8; LANES] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let lanes = u8x16::from_slice(simd, &IOTA) + u8x16::splat(simd, split_first.unwrap_or(1) as u8);
    let fifteen = u8x16::splat(simd, 15);
    let block_start = (lanes & fifteen).simd_eq(u8x16::splat(simd, 0));
    let block_end = (lanes & fifteen).simd_eq(fifteen);
    let none = u8x16::splat(simd, 0).simd_eq(u8x16::splat(simd, 1));
    let (block_start, block_end) = match split_first {
        Some(_) => (block_start, block_end),
        None => (none, none),
    };
    let weigh = |cells: u8x16<S>, weight: u8| (u8x16::splat(simd, 0) - cells) & weight;
    let mut any = u8x16::splat(simd, 0);
    for (c0, out) in (0..).step_by(LANES).zip(sums.chunks_exact_mut(LANES)) {
        let Some(window) = clipped.get(c0..c0 + LANES + 2) else {
            return false;
        };
        let load = |offset: usize| u8x16::from_slice(simd, &window[offset..offset + LANES]);
        let center = load(1);
        let left = block_start.select(center, load(0));
        let right = block_end.select(center, load(2));
        any |= left | center | right;
        (weigh(left, left_weight) + weigh(center, center_weight) + weigh(right, right_weight))
            .store_slice(out);
    }
    any.reduce_max() > 1
}

/// Sums the six grid rows at `taps` of `rows` into `out`.
#[inline]
fn add_rows<S: fearless_simd::Simd>(simd: S, rows: &[u8], taps: [usize; 6], out: &mut [u8]) {
    for (c0, out) in (0..).step_by(LANES).zip(out.chunks_exact_mut(LANES)) {
        let mut sum = u8x16::splat(simd, 0);
        for tap in taps {
            if let Some(lanes) = rows.get(tap + c0..tap + c0 + LANES) {
                sum += u8x16::from_slice(simd, lanes);
            }
        }
        sum.store_slice(out);
    }
}
