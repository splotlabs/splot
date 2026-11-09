// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One plane's § 7.17.2 per-4x4 inputs, painted straight from the deblock
//! records as they arrive: a transform-base key, an info word and the
//! `qindex` of every cell, row-major at a stride padded so that sixteen-lane
//! loads past a row's last cell stay inside the row.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};

use super::grid::{block_cells, reset, resize};
use super::{DeblockBlock, DeblockError, DeblockPredictionUnit, MI_SIZE};
use crate::support::fill_mi_run;

/// Info bits past the § 7.17.4 size codes (`log2 - 1` of `Tx_Width` in bits
/// `0..3`, of `Tx_Height` in bits `3..6`): the cell's record is skipped or
/// lossless, the cell starts its prediction unit's column (`EDGE_V`) or row
/// (`EDGE_H`), and a record covers the cell.
pub(super) const SKIP: u16 = 1 << 6;
pub(super) const LOSSLESS: u16 = 1 << 7;
pub(super) const EDGE_V: u16 = 1 << 8;
pub(super) const EDGE_H: u16 = 1 << 9;
pub(super) const COVERED: u16 = 1 << 10;
const SIZE_CODES: u16 = 0x3f;

/// The § 7.17.2 `TxRowBase`/`TxColBase` pair of a cell as its low eight bits
/// each. Two neighbouring cells lie inside the blocks their bases start, at
/// most 64 mode-info units across, so their bases differ exactly when these
/// keys do.
const fn key(row: u32, col: u32) -> u16 {
    ((row & 0xff) << 8 | col & 0xff) as u16
}

/// Cells per row of a plane `cols` cells wide: padded so that a sixteen-lane
/// group from any cell of the row stays inside it.
pub(super) const fn padded_stride(cols: usize) -> usize {
    cols.next_multiple_of(16) + 16
}

/// The size codes (`log2 - 1`) of every transform size.
const SIZE_CODES_BY_TX: [u16; TX_WIDTH_LOG2.len()] = {
    let mut codes = [0; TX_WIDTH_LOG2.len()];
    let mut tx = 0;
    while tx < codes.len() {
        codes[tx] = (TX_WIDTH_LOG2[tx] - 1) as u16 | ((TX_HEIGHT_LOG2[tx] - 1) as u16) << 3;
        tx += 1;
    }
    codes
};

/// The size codes of transform `tx`.
fn size_codes(tx: u8) -> u16 {
    SIZE_CODES_BY_TX.get(usize::from(tx)).copied().unwrap_or(0)
}

/// A record's own info bits and `qindex`.
fn block_bits(block: &DeblockBlock) -> (u16, u16) {
    let flags = if block.skip { SKIP } else { 0 } | if block.lossless { LOSSLESS } else { 0 };
    (flags | COVERED, block.qindex.min(0xfff) as u16)
}

/// The plane sample origin of `prediction`, kept to 16 bits as the cell
/// positions it is compared with.
fn origin(prediction: DeblockPredictionUnit, (sub_x, sub_y): (usize, usize)) -> [usize; 2] {
    let at = |base: u32, sub: usize| usize::from(((base as usize * MI_SIZE) >> sub) as u16);
    [at(prediction.base_c, sub_x), at(prediction.base_r, sub_y)]
}

/// The cell from cell `first` on, along one axis, whose cells start the
/// prediction unit at plane sample `origin` on that axis, if any.
fn edge(origin: usize, first: usize) -> Option<usize> {
    origin
        .is_multiple_of(MI_SIZE)
        .then(|| (origin / MI_SIZE).checked_sub(first))
        .flatten()
}

#[derive(Default)]
pub(crate) struct PlaneFacts {
    pub(super) stride: usize,
    pub(super) keys: Vec<u16>,
    pub(super) info: Vec<u16>,
    pub(super) qindex: Vec<u16>,
}

impl PlaneFacts {
    /// Marks every cell of a `rows` by `cols` plane uncovered.
    pub(super) fn reset(
        &mut self,
        (rows, cols): (usize, usize),
        plane: splot_recon::PlaneId,
    ) -> Result<(), DeblockError> {
        self.stride = padded_stride(cols);
        let len = rows
            .checked_mul(self.stride)
            .ok_or(DeblockError::Workspace)?;
        resize(&mut self.keys, len, 0, plane)?;
        reset(&mut self.info, len, 0, plane)?;
        resize(&mut self.qindex, len, 0, plane)
    }

    /// The keys, info and `qindex` of the `len` cells from column `from` of
    /// cell row `row`.
    pub(super) fn row(&self, row: usize, from: usize, len: usize) -> Option<[&[u16]; 3]> {
        let start = row.checked_mul(self.stride)?.checked_add(from)?;
        let cells = start..start.checked_add(len)?;
        Some([
            self.keys.get(cells.clone())?,
            self.info.get(cells.clone())?,
            self.qindex.get(cells)?,
        ])
    }

    /// Paints the luma records `blocks` over a `mi_rows` by `mi_cols` frame
    /// in record order, so that a later record overwrites an earlier one.
    pub(super) fn paint_luma(
        &mut self,
        blocks: &[DeblockBlock],
        (mi_rows, mi_cols): (usize, usize),
    ) {
        let stride = self.stride;
        for block in blocks {
            let (r, c) = (block.r as usize, block.c as usize);
            let row_end = r.saturating_add(block.n4h as usize).min(mi_rows);
            let col_end = c.saturating_add(block.n4w as usize).min(mi_cols);
            if r >= row_end || c >= col_end {
                continue;
            }
            let (bits, qindex) = block_bits(block);
            let [x, y] = origin(block.luma_prediction, (0, 0));
            let rect = Rect {
                at: r * stride + c,
                rows: row_end - r,
                values: [
                    key(block.r, block.c),
                    bits | size_codes(block.luma_tx),
                    qindex,
                ],
                edges: [edge(x, c), edge(y, r)],
            };
            self.paint(rect, col_end - c);
        }
    }

    /// Paints the chroma records `records` in record order: a block record
    /// writes every fact of its cells, a transform-only record, which follows
    /// its block's, their transform base and size.
    pub(super) fn paint_chroma(
        &mut self,
        records: &[DeblockBlock],
        extent: (usize, usize),
        (sub_x, sub_y): (usize, usize),
    ) {
        let stride = self.stride;
        for block in records {
            let [rows, cols] = block_cells(block, extent, (sub_x, sub_y));
            if rows.is_empty() || cols.is_empty() {
                continue;
            }
            let key = key(block.chroma_base_r, block.chroma_base_c);
            if block.chroma_transform_only {
                let codes = block.chroma_tx.map(size_codes);
                for row in rows {
                    self.paint_transform(
                        key,
                        codes,
                        row * stride + cols.start..row * stride + cols.end,
                    );
                }
                continue;
            }
            let (bits, qindex) = block_bits(block);
            let [x, y] = origin(block.chroma_prediction, (sub_x, sub_y));
            let rect = Rect {
                at: rows.start * stride + cols.start,
                rows: rows.len(),
                values: [key, bits | size_codes(block.chroma_tx.unwrap_or(0)), qindex],
                edges: [edge(x, cols.start), edge(y, rows.start)],
            };
            self.paint(rect, cols.len());
        }
    }

    #[allow(clippy::inline_always, reason = "keeps the rect in registers")]
    #[inline(always)]
    fn paint(&mut self, rect: Rect, width: usize) {
        match width {
            1 => self.fill_rect::<1>(rect),
            2 => self.fill_rect::<2>(rect),
            4 => self.fill_rect::<4>(rect),
            8 => self.fill_rect::<8>(rect),
            16 => self.fill_rect::<16>(rect),
            width => self.fill_rect_any(rect, width),
        }
    }

    /// Writes one record's [`Rect`] of `N` cells per row.
    #[allow(clippy::inline_always, reason = "one fixed-width store per row")]
    #[inline(always)]
    fn fill_rect<const N: usize>(&mut self, rect: Rect) {
        let [key, info, qindex] = rect.values;
        let [edge_col, edge_row] = rect.edges;
        let mut row_info = [info; N];
        if let Some(cell) = edge_col.and_then(|col| row_info.get_mut(col)) {
            *cell |= EDGE_V;
        }
        let (keys, infos, qindexes) =
            (&mut self.keys[..], &mut self.info[..], &mut self.qindex[..]);
        let Some(last) = keys
            .len()
            .min(infos.len())
            .min(qindexes.len())
            .checked_sub(N)
        else {
            return;
        };
        let mut at = rect.at;
        for row in 0..rect.rows {
            if at > last {
                return;
            }
            keys[at..at + N].copy_from_slice(&[key; N]);
            let infos = &mut infos[at..at + N];
            infos.copy_from_slice(&row_info);
            qindexes[at..at + N].copy_from_slice(&[qindex; N]);
            if Some(row) == edge_row {
                for cell in infos.iter_mut() {
                    *cell |= EDGE_H;
                }
            }
            at += self.stride;
        }
    }

    fn fill_rect_any(&mut self, rect: Rect, width: usize) {
        let [key, info, qindex] = rect.values;
        let [edge_col, edge_row] = rect.edges;
        let mut at = rect.at;
        for row in 0..rect.rows {
            let (Some(keys), Some(infos), Some(qindexes)) = (
                self.keys.get_mut(at..at + width),
                self.info.get_mut(at..at + width),
                self.qindex.get_mut(at..at + width),
            ) else {
                return;
            };
            fill_mi_run(keys, key);
            let edge = if Some(row) == edge_row { EDGE_H } else { 0 };
            fill_mi_run(infos, info | edge);
            fill_mi_run(qindexes, qindex);
            if let Some(cell) = edge_col.and_then(|col| infos.get_mut(col)) {
                *cell |= EDGE_V;
            }
            at += self.stride;
        }
    }

    /// Writes a transform-only record's base `key` and size `codes` over
    /// cells `cells`.
    fn paint_transform(&mut self, key: u16, codes: Option<u16>, cells: core::ops::Range<usize>) {
        let (Some(keys), Some(infos)) =
            (self.keys.get_mut(cells.clone()), self.info.get_mut(cells))
        else {
            return;
        };
        fill_mi_run(keys, key);
        if let Some(codes) = codes {
            for info in infos {
                *info = *info & !SIZE_CODES | codes;
            }
        }
    }
}

/// One record's cells: the first, its row count, the key, info and `qindex`
/// it writes, and the column and row (from its first) that start its
/// prediction unit.
#[derive(Clone, Copy)]
struct Rect {
    at: usize,
    rows: usize,
    values: [u16; 3],
    edges: [Option<usize>; 2],
}
