// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 7.20.4 `get_tx_skip` sums of the 6x6 windows of a classification grid.

use fearless_simd::{SimdBase, mask8x16, prelude::*, u8x16};

use super::{
    PC_WIENER_LEAD, PcWienerClassifyParams, PcWienerTxSkipRun, check_tx_skip_cells, coordinate_add,
    usize_to_isize,
};
use crate::{ReconError, Result};

/// Cells one vector covers.
const LANES: usize = 16;
/// Cells across one § 7.20.4 64-sample block.
const BLOCK_CELLS: u8 = 16;

/// The row stride of the sums [`window_sums`] writes for `cell_cols` cells.
pub(super) fn sums_stride(cell_cols: usize) -> usize {
    cell_cols.div_ceil(LANES) * LANES
}

/// The window tx-skip sum of every cell, written into `buffer`; row `r` is
/// at `r * sums_stride(cell_cols)`.
///
/// `get_tx_skip` clips x to `[BlockStartX, BlockEndX]` before `>> 2`, and the
/// clip commutes with the shift, so the six feature columns of the cell at
/// grid column `g` read columns `g - 1`, `g`, `g + 1` clipped to its block,
/// weighted by `x & 3`. Each grid row's weighted column sums are built once
/// from a run clipped to the span's outer bounds; with `split_blocks`, a
/// cell on an inner 64-sample block edge takes its own column for the
/// neighbor outside its block. A cell's sum then adds the rows of its six
/// feature rows, each clipped to the stripe, then to the tile, which never
/// decrease down the grid.
pub(super) fn window_sums<'b, 'g, FT>(
    params: &PcWienerClassifyParams,
    cell_cols: usize,
    cell_rows: usize,
    split_blocks: bool,
    tx_skip: &mut FT,
    buffer: &'b mut Vec<u8>,
) -> Result<&'b [u8]>
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
    let phase = (params.x & 3) as u8;
    let left = u8::from(phase == 0);
    let weights = [left, 5 - phase - left, phase + 1];
    let stripe = [
        bound(params.luma_stripe_start_y)?,
        bound(params.luma_stripe_end_y)?,
    ];
    let row_of = |y: isize| {
        (y.max(stripe[0]).min(stripe[1]) as usize)
            .max(params.tile_start_y)
            .min(params.tile_end_y)
    };
    let y0 = coordinate_add(params.y, -PC_WIENER_LEAD, "PC-Wiener tx-skip y")?;
    let y_end = coordinate_add(y0, bound(4 * cell_rows + 1)?, "PC-Wiener tx-skip y")?;
    let first_y = row_of(y0);
    let first_row = first_y >> 2;
    let row_count = (row_of(y_end) >> 2) - first_row + 1;
    let padded = stride + 2;
    let total = padded + LANES + (row_count + cell_rows) * stride;
    buffer
        .try_reserve(total.saturating_sub(buffer.len()))
        .map_err(|_| ReconError::WorkspaceAllocationFailed {
            plane: crate::PlaneId::Y,
            context: "PC-Wiener tx-skip sums",
        })?;
    buffer.resize(total, 0);
    let (clipped, rest) = buffer.split_at_mut(padded + LANES);
    let (column_sums, sums) = rest.split_at_mut(row_count * stride);
    let pre = ((col - (first - 1)).max(0) as usize).min(padded);
    let take = len.min(padded - pre);
    let mut fetched = Vec::new();
    for (index, sums_row) in column_sums.chunks_exact_mut(stride).enumerate() {
        let row = first_row + index;
        let run = PcWienerTxSkipRun {
            x: (4 * col as usize).max(params.block_start_x),
            y: (4 * row).max(first_y),
            row,
            col: col as usize,
            len,
        };
        let cells = tx_skip(run, &mut fetched)?.unwrap_or(&fetched);
        let cells = cells.get(..len).ok_or(ReconError::BufferLengthMismatch {
            expected: len,
            actual: cells.len(),
        })?;
        crate::lr_madd::lr_simd!(simd => replicate_run(simd, clipped, pre, &cells[..take]));
        if crate::lr_madd::lr_simd!(simd => column_row(simd, clipped, sums_row, weights, split_blocks.then_some(first)))
        {
            check_tx_skip_cells(run, cells)?;
        }
    }
    let rows = column_sums.as_chunks::<LANES>().0;
    let row_stride = stride / LANES;
    let phase_y = (y0 & 3) as usize;
    let last_row = row_count as isize - 1;
    for (cell_row, out) in sums
        .as_chunks_mut::<LANES>()
        .0
        .chunks_exact_mut(row_stride)
        .enumerate()
    {
        let base = (y0 >> 2) - first_row as isize + cell_row as isize;
        let row = |offset: isize| {
            let start = (base + offset).clamp(0, last_row) as usize * row_stride;
            rows.get(start..start + row_stride).unwrap_or_default()
        };
        let rows = [row(0), row(1), row(2)];
        crate::lr_madd::lr_simd!(simd => match phase_y {
            0 => add_rows::<_, 4, 2, 0>(simd, rows, out),
            1 => add_rows::<_, 3, 3, 0>(simd, rows, out),
            2 => add_rows::<_, 2, 4, 0>(simd, rows, out),
            _ => add_rows::<_, 1, 4, 1>(simd, rows, out),
        });
    }
    Ok(sums)
}

/// Writes `cells` at `clipped[pre..]` and replicates its end values over the
/// rest; `clipped` holds `LANES` bytes of slack past the edge-replicated run.
#[inline]
fn replicate_run<S: fearless_simd::Simd>(simd: S, clipped: &mut [u8], pre: usize, cells: &[u8]) {
    let (Some(&first), Some(&last)) = (cells.first(), cells.last()) else {
        return;
    };
    let end = pre + cells.len();
    let Some((run, rest)) = clipped
        .split_at_mut_checked(end)
        .filter(|(run, _)| pre <= LANES && run.len() >= LANES)
    else {
        for (index, slot) in clipped.iter_mut().enumerate() {
            *slot = cells
                .get(index.saturating_sub(pre))
                .copied()
                .unwrap_or(last);
        }
        return;
    };
    u8x16::splat(simd, first).store_slice(&mut run[..LANES]);
    match cells.last_chunk::<LANES>() {
        Some(tail) => {
            for (lanes, out) in cells
                .chunks_exact(LANES)
                .zip(run[pre..].chunks_exact_mut(LANES))
            {
                u8x16::from_slice(simd, lanes).store_slice(out);
            }
            u8x16::from_slice(simd, tail).store_slice(&mut run[end - LANES..]);
        }
        None => run[pre..].copy_from_slice(cells), // splot-copy-ok: short edge-replicated run
    }
    for out in rest.chunks_exact_mut(LANES) {
        u8x16::splat(simd, last).store_slice(out);
    }
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
    let none = mask8x16::splat(simd, false);
    let (block_start, block_end) = if let Some(first) = split_first {
        let last = u8x16::splat(simd, BLOCK_CELLS - 1);
        let lanes = (u8x16::from_slice(simd, &IOTA) + u8x16::splat(simd, first as u8)) & last;
        (lanes.simd_eq(u8x16::splat(simd, 0)), lanes.simd_eq(last))
    } else {
        (none, none)
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

/// Sums grid rows `rows` with the weights of the six feature rows they hold.
#[inline]
fn add_rows<S: fearless_simd::Simd, const W0: u8, const W1: u8, const W2: u8>(
    simd: S,
    [a, b, c]: [&[[u8; LANES]]; 3],
    out: &mut [[u8; LANES]],
) {
    let weigh = |lanes: &[u8; LANES], weight: u8| {
        let row = u8x16::from_slice(simd, lanes);
        (0..weight).fold(u8x16::splat(simd, 0), |sum, _| sum + row)
    };
    for (((out, a), b), c) in out.iter_mut().zip(a).zip(b).zip(c) {
        (weigh(a, W0) + weigh(b, W1) + weigh(c, W2)).store_slice(out);
    }
}
