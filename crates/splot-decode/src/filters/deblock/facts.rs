// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One plane's § 7.17.2 per-4x4 inputs, painted straight from the deblock
//! records as they arrive: a transform-base key, an info word and the
//! `qindex` of every cell, row-major at a stride padded so that sixteen-lane
//! loads past a row's last cell stay inside the row.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};

use super::grid::{block_spans, reset};
use super::{ChromaDeblockRecords, DeblockBlock, DeblockError, DeblockPredictionUnit, MI_SIZE};
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

/// What painted a chroma cell: an overlay record (which a luma record does
/// not replace), and a transform-only record's base and size (which no
/// block record replaces).
const OVERLAY: u8 = 1;
const TX_KEY: u8 = 2;
const TX_CODE: u8 = 4;

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

/// The column from cell column `first` on whose cells start the prediction
/// unit at `origin`, if any.
fn edge_col([x, _]: [usize; 2], first: usize) -> Option<usize> {
    (x % MI_SIZE == 0)
        .then(|| (x / MI_SIZE).checked_sub(first))
        .flatten()
}

/// The prediction-unit edge bit of plane cell row `row`.
const fn row_edge([_, y]: [usize; 2], row: usize) -> u16 {
    if row * MI_SIZE == y { EDGE_H } else { 0 }
}

#[derive(Default)]
pub(crate) struct PlaneFacts {
    pub(super) stride: usize,
    pub(super) keys: Vec<u16>,
    pub(super) info: Vec<u16>,
    pub(super) qindex: Vec<u16>,
    layers: Vec<u8>,
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
        reset(&mut self.keys, len, 0, plane)?;
        reset(&mut self.info, len, 0, plane)?;
        reset(&mut self.qindex, len, 0, plane)?;
        let chroma = plane != splot_recon::PlaneId::Y;
        reset(&mut self.layers, if chroma { len } else { 0 }, 0, plane)
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
            let info = bits | size_codes(block.luma_tx);
            let origin = origin(block.luma_prediction, (0, 0));
            let key = key(block.r, block.c);
            let rect = LumaRect {
                at: r * stride + c,
                rows: r..row_end,
                values: [key, info, qindex],
                origin,
                edge_col: edge_col(origin, c),
            };
            match col_end - c {
                1 => self.fill_rect::<1>(rect),
                2 => self.fill_rect::<2>(rect),
                4 => self.fill_rect::<4>(rect),
                8 => self.fill_rect::<8>(rect),
                16 => self.fill_rect::<16>(rect),
                width => self.fill_rect_any(rect, width),
            }
        }
    }

    /// Writes one luma record's [`LumaRect`] of `N` cells per row.
    #[allow(clippy::inline_always, reason = "one fixed-width store per row")]
    #[inline(always)]
    fn fill_rect<const N: usize>(&mut self, rect: LumaRect) {
        let [key, info, qindex] = rect.values;
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
        for row in rect.rows {
            if at > last {
                return;
            }
            keys[at..at + N].copy_from_slice(&[key; N]);
            let infos = &mut infos[at..at + N];
            infos.copy_from_slice(&[info | row_edge(rect.origin, row); N]);
            qindexes[at..at + N].copy_from_slice(&[qindex; N]);
            if let Some(cell) = rect.edge_col.and_then(|col| infos.get_mut(col)) {
                *cell |= EDGE_V;
            }
            at += self.stride;
        }
    }

    fn fill_rect_any(&mut self, rect: LumaRect, width: usize) {
        let [key, info, qindex] = rect.values;
        let mut at = rect.at;
        for row in rect.rows {
            let (Some(keys), Some(infos), Some(qindexes)) = (
                self.keys.get_mut(at..at + width),
                self.info.get_mut(at..at + width),
                self.qindex.get_mut(at..at + width),
            ) else {
                return;
            };
            fill_mi_run(keys, key);
            fill_mi_run(infos, info | row_edge(rect.origin, row));
            fill_mi_run(qindexes, qindex);
            if let Some(cell) = rect.edge_col.and_then(|col| infos.get_mut(col)) {
                *cell |= EDGE_V;
            }
            at += self.stride;
        }
    }

    /// Paints the luma records `blocks` over the chroma cells whose top-left
    /// mode-info unit they cover, under any overlay or transform-only record.
    pub(super) fn paint_chroma_from_luma(
        &mut self,
        blocks: &[DeblockBlock],
        (mi_rows, mi_cols): (usize, usize),
        (sub_x, sub_y): (usize, usize),
    ) {
        for block in blocks {
            let row_end = (block.r as usize)
                .saturating_add(block.n4h as usize)
                .min(mi_rows);
            let col_end = (block.c as usize)
                .saturating_add(block.n4w as usize)
                .min(mi_cols);
            let rows = (block.r as usize).div_ceil(1 << sub_y)..row_end.div_ceil(1 << sub_y);
            let cols = (block.c as usize).div_ceil(1 << sub_x)..col_end.div_ceil(1 << sub_x);
            let stride = self.stride;
            let overlaid = |row: usize| {
                let cells = self
                    .layers
                    .get(row * stride + cols.start..row * stride + cols.end);
                cells.is_none_or(|cells| cells.iter().all(|&layer| layer & OVERLAY != 0))
            };
            if rows.clone().all(overlaid) {
                continue;
            }
            let paint = Paint::new(block, (sub_x, sub_y), 0);
            for row in rows {
                let at = row * self.stride;
                self.paint_cells(&paint, row, at + cols.start..at + cols.end);
            }
        }
    }

    /// Paints plane `plane` (0 for U) of the chroma records from `start` on:
    /// overlays replace the block a cell reads, transform-only records the
    /// transform base and size.
    pub(super) fn paint_overlays(
        &mut self,
        records: &ChromaDeblockRecords,
        (plane, start): (usize, usize),
        (mi_rows, mi_cols): (usize, usize),
        (sub_x, sub_y): (usize, usize),
    ) {
        let mask = [ChromaDeblockRecords::U, ChromaDeblockRecords::V][plane];
        let records = records.blocks.get(start..).unwrap_or_default();
        for block in records.iter().filter(|record| record.planes & mask != 0) {
            let block = &block.block;
            let spans = block_spans(block, (mi_rows, mi_cols), (sub_x, sub_y), self.stride);
            if block.chroma_transform_only {
                for (_, cells) in spans {
                    self.paint_transform(block, cells);
                }
                continue;
            }
            let paint = Paint::new(block, (sub_x, sub_y), OVERLAY);
            for (row, cells) in spans {
                self.paint_cells(&paint, row, cells);
            }
        }
    }

    /// Writes `paint` over the cells `cells` of cell row `row` that no overlay
    /// holds (or every one, for an overlay), keeping a transform-only
    /// record's base and size.
    #[allow(clippy::inline_always, reason = "most spans are one cell")]
    #[inline(always)]
    fn paint_cells(&mut self, paint: &Paint, row: usize, cells: core::ops::Range<usize>) {
        let first = cells.start - row * self.stride;
        if cells.len() != 1 {
            return self.paint_run(paint, (row, first), cells);
        }
        let at = cells.start;
        let (Some(layer), Some(key), Some(info), Some(qindex)) = (
            self.layers.get_mut(at),
            self.keys.get_mut(at),
            self.info.get_mut(at),
            self.qindex.get_mut(at),
        ) else {
            return;
        };
        let spans = [key, info, qindex].map(core::slice::from_mut);
        paint_span(paint, (row, first), spans, core::slice::from_mut(layer));
    }

    #[inline(never)]
    fn paint_run(&mut self, paint: &Paint, at: (usize, usize), cells: core::ops::Range<usize>) {
        let (layers, keys, infos, qindexes) = (
            &mut self.layers[..],
            &mut self.keys[..],
            &mut self.info[..],
            &mut self.qindex[..],
        );
        let len = layers
            .len()
            .min(keys.len())
            .min(infos.len())
            .min(qindexes.len());
        if cells.start > cells.end || cells.end > len {
            return;
        }
        let spans = [
            &mut keys[cells.clone()],
            &mut infos[cells.clone()],
            &mut qindexes[cells.clone()],
        ];
        let layers = &mut layers[cells];
        match layers.len() {
            2 => paint_fixed::<2>(paint, at, spans, layers),
            4 => paint_fixed::<4>(paint, at, spans, layers),
            8 => paint_fixed::<8>(paint, at, spans, layers),
            _ => paint_span(paint, at, spans, layers),
        }
    }

    /// Writes the transform-only record `block`'s base and size over cells
    /// `cells`.
    fn paint_transform(&mut self, block: &DeblockBlock, cells: core::ops::Range<usize>) {
        let key = key(block.chroma_base_r, block.chroma_base_c);
        let codes = block.chroma_tx.map(size_codes);
        for cell in cells {
            let (Some(layer), Some(info), Some(cell_key)) = (
                self.layers.get_mut(cell),
                self.info.get_mut(cell),
                self.keys.get_mut(cell),
            ) else {
                return;
            };
            *cell_key = key;
            *layer |= TX_KEY;
            if let Some(codes) = codes {
                *info = *info & !SIZE_CODES | codes;
                *layer |= TX_CODE;
            }
        }
    }
}

/// Writes `paint` over the cells of one span of cell row `row`, from cell
/// column `first`, that no overlay holds (or every one, for an overlay),
/// keeping a transform-only record's base and size.
#[allow(clippy::inline_always, reason = "specialized by span width")]
#[inline(always)]
fn paint_span(
    paint: &Paint,
    (row, first): (usize, usize),
    [keys, infos, qindexes]: [&mut [u16]; 3],
    layers: &mut [u8],
) {
    let bits = paint.bits | row_edge(paint.origin, row);
    let edge_col = edge_col(paint.origin, first);
    let held = if paint.layer == OVERLAY {
        TX_KEY | TX_CODE
    } else {
        u8::MAX
    };
    if layers.iter().all(|&layer| layer & held == 0) {
        fill_mi_run(keys, paint.key);
        fill_mi_run(infos, paint.codes | bits);
        fill_mi_run(qindexes, paint.qindex);
        fill_mi_run(layers, paint.layer);
        if let Some(info) = edge_col.and_then(|col| infos.get_mut(col)) {
            *info |= EDGE_V;
        }
        return;
    }
    let cells = layers
        .iter_mut()
        .zip(keys)
        .zip(infos.iter_mut().zip(qindexes));
    for (col, ((layer, key), (info, qindex))) in cells.enumerate() {
        if *layer & OVERLAY > paint.layer {
            continue;
        }
        let size = if *layer & TX_CODE == 0 {
            paint.codes
        } else {
            *info & SIZE_CODES
        };
        let edge = if Some(col) == edge_col { EDGE_V } else { 0 };
        *info = size | bits | edge;
        *qindex = paint.qindex;
        if *layer & TX_KEY == 0 {
            *key = paint.key;
        }
        *layer |= paint.layer;
    }
}

/// [`paint_span`] over spans of `N` cells.
#[allow(clippy::inline_always, reason = "fixed-width stores per span")]
#[inline(always)]
fn paint_fixed<const N: usize>(
    paint: &Paint,
    at: (usize, usize),
    [keys, infos, qindexes]: [&mut [u16]; 3],
    layers: &mut [u8],
) {
    let (Some(keys), Some(infos), Some(qindexes), Some(layers)) = (
        keys.first_chunk_mut::<N>(),
        infos.first_chunk_mut::<N>(),
        qindexes.first_chunk_mut::<N>(),
        layers.first_chunk_mut::<N>(),
    ) else {
        return;
    };
    paint_span(paint, at, [keys, infos, qindexes], layers);
}

/// One luma record's cells: the first, its rows, the key, info and
/// `qindex` it writes, its prediction origin and the column (from its first)
/// of that origin's left edge.
struct LumaRect {
    at: usize,
    rows: core::ops::Range<usize>,
    values: [u16; 3],
    origin: [usize; 2],
    edge_col: Option<usize>,
}

/// What a block record paints over chroma cells: its key, size codes, info
/// bits and `qindex`, its prediction origin in the plane and its layer.
struct Paint {
    key: u16,
    codes: u16,
    bits: u16,
    qindex: u16,
    origin: [usize; 2],
    layer: u8,
}

impl Paint {
    fn new(block: &DeblockBlock, sub: (usize, usize), layer: u8) -> Self {
        let (bits, qindex) = block_bits(block);
        Self {
            key: key(block.chroma_base_r, block.chroma_base_c),
            codes: size_codes(block.chroma_tx.unwrap_or(0)),
            bits,
            qindex,
            origin: origin(block.chroma_prediction, sub),
            layer,
        }
    }
}
