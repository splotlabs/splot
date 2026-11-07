// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use super::{ChromaDeblockRecords, DeblockBlock, DeblockError, EdgeBlock, MI_SIZE};

/// A cell holds its record index plus one, so that an empty cell is zero.
const NO_BLOCK: u32 = 0;

#[derive(Clone, Copy, Default)]
pub(super) struct MiCell {
    pub(super) base: u32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct ChromaMiCell {
    pub(super) overlay: u32,
    pub(super) chroma_transform: u32,
}

/// [`CellPlane::info`] bits past the two transform sizes: the cell's record
/// is skipped or lossless, or the cell starts its prediction unit's column
/// (`EDGE_V`) or row (`EDGE_H`).
pub(super) const SKIP: u32 = 1 << 16;
pub(super) const LOSSLESS: u32 = 1 << 17;
pub(super) const EDGE_V: u32 = 1 << 18;
pub(super) const EDGE_H: u32 = 1 << 19;

/// Where [`CellPlane::info`] keeps the record's `qindex`, saturated at the
/// field's 12 bits: every § 7.17.5 level clamps far below that.
pub(super) const QINDEX_SHIFT: u32 = 20;

/// The transform base of a cell no record covers.
pub(super) const UNCOVERED: u32 = u32::MAX;

/// The § 7.17.2 inputs of one plane's 4x4 cells, laid out as the spec's
/// per-cell arrays: the transform base (`TxRowBase << 16 | TxColBase`) and an
/// info word of `Tx_Width | Tx_Height << 8`, the [`SKIP`] to [`EDGE_H`] bits
/// and the record's `qindex` from bit [`QINDEX_SHIFT`].
#[derive(Default)]
pub(super) struct CellPlane {
    pub(super) bases: Vec<u32>,
    pub(super) info: Vec<u32>,
}

impl CellPlane {
    /// Marks every cell uncovered. The info words keep whatever they held:
    /// no decision reads the info of an uncovered cell.
    fn reset(&mut self, len: usize, plane: splot_recon::PlaneId) -> Result<(), DeblockError> {
        reset(&mut self.bases, len, UNCOVERED, plane)?;
        if self.info.len() != len {
            reset(&mut self.info, len, 0, plane)?;
        }
        Ok(())
    }

    /// The bases and info of cells `start..end`.
    pub(super) fn span(&self, start: usize, end: usize) -> Option<[&[u32]; 2]> {
        Some([self.bases.get(start..end)?, self.info.get(start..end)?])
    }

    /// Writes `facts` (base, info) over cells `cells`, with `flag` added to
    /// the info of their `flagged`th cell.
    fn fill_span(
        &mut self,
        cells: core::ops::Range<usize>,
        facts: (u32, u32),
        (flagged, flag): (usize, u32),
    ) {
        let (Some(bases), Some(info)) =
            (self.bases.get_mut(cells.clone()), self.info.get_mut(cells))
        else {
            return;
        };
        for (index, (base, info)) in bases.iter_mut().zip(info.iter_mut()).enumerate() {
            *base = facts.0;
            *info = facts.1 | if index == flagged { flag } else { 0 };
        }
    }
}

fn reset<T: Copy>(
    cells: &mut Vec<T>,
    len: usize,
    fill: T,
    plane: splot_recon::PlaneId,
) -> Result<(), DeblockError> {
    cells.clear();
    cells
        .try_reserve(len)
        .map_err(|_| DeblockError::Allocation {
            plane,
            context: "deblock MI grid",
        })?;
    cells.resize(len, fill);
    Ok(())
}

/// What one record gives the cells it covers in one plane.
struct Facts {
    base: u32,
    info: u32,
    /// Prediction unit origin in plane samples, `x` then `y`, kept to 16 bits.
    prediction: [usize; 2],
}

impl Facts {
    fn of(cell: EdgeBlock<'_>, plane: usize, (sub_x, sub_y): (usize, usize)) -> Self {
        let (tx_row, tx_col) = cell.tx_base(plane);
        let prediction = cell.prediction(plane);
        let tx = cell.tx(plane);
        let dimension = |table: &[i32]| table.get(tx).map_or(0, |&size| size.clamp(0, 255) as u32);
        let block = cell.block;
        let base = (tx_row as u32) << 16 | (tx_col as u32 & 0xffff);
        let origin = |base: u32, sub: usize| usize::from(((base as usize * MI_SIZE) >> sub) as u16);
        Self {
            base: base.min(UNCOVERED - 1),
            info: dimension(&TX_WIDTH)
                | dimension(&TX_HEIGHT) << 8
                | if block.skip { SKIP } else { 0 }
                | if block.lossless { LOSSLESS } else { 0 }
                | block.qindex.min(0xfff) << QINDEX_SHIFT,
            prediction: [
                origin(prediction.base_c, sub_x),
                origin(prediction.base_r, sub_y),
            ],
        }
    }

    /// The base and info word of the cell at plane sample `(x, y)`.
    fn at(&self, x: usize, y: usize) -> (u32, u32) {
        let info = self.info
            | if x == self.prediction[0] { EDGE_V } else { 0 }
            | if y == self.prediction[1] { EDGE_H } else { 0 };
        (self.base, info)
    }
}

/// The record a cell value names; an empty cell names none.
const fn record(cell: u32) -> usize {
    (cell as usize).wrapping_sub(1)
}

/// The luma grid: the record of every mode-info cell and the luma
/// [`CellPlane`].
pub(super) struct MiGridStorage {
    pub(super) mi_cols: usize,
    pub(super) cells: Vec<MiCell>,
    pub(super) plane: CellPlane,
}

pub(super) struct ChromaMiGridStorage {
    /// One cell per chroma mode-info unit, not per luma one.
    pub(super) cells: Vec<ChromaMiCell>,
    pub(super) plane: CellPlane,
    pub(super) cell_cols: usize,
    pub(super) sub_x: usize,
    pub(super) sub_y: usize,
}

pub(super) struct MiGrid<'a> {
    pub(super) base: &'a MiGridStorage,
    pub(super) chroma: Option<&'a ChromaMiGridStorage>,
    pub(super) base_blocks: &'a [DeblockBlock],
    pub(super) overlay_blocks: &'a ChromaDeblockRecords,
}

impl MiGrid<'_> {
    pub(super) const fn new<'a>(
        base: &'a MiGridStorage,
        chroma: Option<&'a ChromaMiGridStorage>,
        base_blocks: &'a [DeblockBlock],
        overlay_blocks: &'a ChromaDeblockRecords,
    ) -> MiGrid<'a> {
        MiGrid {
            base,
            chroma,
            base_blocks,
            overlay_blocks,
        }
    }

    const fn index(&self, row: usize, col: usize) -> usize {
        row.wrapping_mul(self.base.mi_cols).wrapping_add(col)
    }

    #[allow(clippy::inline_always, reason = "measured luma deblock hot path")]
    #[inline(always)]
    pub(super) fn get_luma_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let cell = self.base.cells.get(self.index(row, col))?;
        Some(EdgeBlock {
            block: self.base_blocks.get(record(cell.base))?,
            chroma_transform: None,
        })
    }

    #[allow(clippy::inline_always, reason = "measured chroma deblock hot path")]
    #[inline(always)]
    pub(super) fn get_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let base = self.base.cells.get(self.index(row, col))?;
        let chroma = self.chroma.and_then(|grid| {
            grid.cells.get(
                (row >> grid.sub_y)
                    .wrapping_mul(grid.cell_cols)
                    .wrapping_add(col >> grid.sub_x),
            )
        });
        let block = match chroma.map(|cell| cell.overlay) {
            Some(overlay) if overlay != NO_BLOCK => self.overlay_blocks.get(record(overlay))?,
            _ => self.base_blocks.get(record(base.base))?,
        };
        let chroma_transform = match chroma.map(|cell| cell.chroma_transform) {
            Some(transform) if transform != NO_BLOCK => {
                Some(self.overlay_blocks.get(record(transform))?)
            }
            _ => None,
        };
        Some(EdgeBlock {
            block,
            chroma_transform,
        })
    }

    /// The bases and info of plane cell row `row`: luma cells, or chroma
    /// cells when the grid has a chroma overlay.
    pub(super) fn cell_row(&self, row: usize) -> Option<[&[u32]; 2]> {
        let (plane, cols) = match self.chroma {
            Some(grid) => (&grid.plane, grid.cell_cols),
            None => (&self.base.plane, self.base.mi_cols),
        };
        let start = row.checked_mul(cols)?;
        plane.span(start, start.checked_add(cols)?)
    }
}

/// The grid vectors one frame's deblock fills.
///
/// They travel on the frame filter records, so the next frame lays its grids
/// out over the ones the last frame left rather than sizing new ones.
#[derive(Default)]
pub(crate) struct DeblockGridStorage {
    pub(super) cells: Vec<MiCell>,
    pub(super) plane: CellPlane,
    pub(super) chroma: [ChromaGridVecs; 2],
}

/// One chroma plane's grid vectors.
pub(super) type ChromaGridVecs = (Vec<ChromaMiCell>, CellPlane);

/// The cell spans `block` covers, one per cell row, in a grid of `cols`
/// cells per row whose cells are `1 << sub` mode-info units wide and high.
fn block_spans(
    block: &DeblockBlock,
    (mi_rows, mi_cols): (usize, usize),
    (sub_x, sub_y): (usize, usize),
    cols: usize,
) -> impl Iterator<Item = (usize, core::ops::Range<usize>)> + use<> {
    let (r, c) = (block.r as usize, block.c as usize);
    let row_end = r.saturating_add(block.n4h as usize).min(mi_rows);
    let col_end = c.saturating_add(block.n4w as usize).min(mi_cols);
    let col_start = c.min(col_end) >> sub_x;
    let col_end = col_end.div_ceil(1 << sub_x);
    ((r >> sub_y)..row_end.div_ceil(1 << sub_y)).map(move |row| {
        let base = row * cols;
        (row, base + col_start..base + col_end)
    })
}

impl MiGridStorage {
    pub(super) fn new(mi_cols: usize, storage: &mut DeblockGridStorage) -> Self {
        Self {
            mi_cols,
            cells: core::mem::take(&mut storage.cells),
            plane: core::mem::take(&mut storage.plane),
        }
    }

    /// Lays every record out over the frame in record order, so that a later
    /// record overwrites an earlier one.
    pub(super) fn fill(
        &mut self,
        blocks: &[DeblockBlock],
        mi_rows: usize,
    ) -> Result<(), DeblockError> {
        let plane = splot_recon::PlaneId::Y;
        let cols = self.mi_cols;
        let len = mi_rows.checked_mul(cols).ok_or(DeblockError::Workspace)?;
        reset(&mut self.cells, len, MiCell::default(), plane)?;
        self.plane.reset(len, plane)?;
        for (index, block) in blocks.iter().enumerate() {
            let mark = MiCell {
                base: mi_block_index(index)? + 1,
            };
            let edge = EdgeBlock {
                block,
                chroma_transform: None,
            };
            let facts = Facts::of(edge, 0, (0, 0));
            let x = facts.prediction[0];
            let mut spans = block_spans(block, (mi_rows, cols), (0, 0), cols).peekable();
            let Some((_, first)) = spans.peek().cloned() else {
                continue;
            };
            let flagged = (x / MI_SIZE)
                .checked_sub(block.c as usize)
                .filter(|_| x.is_multiple_of(MI_SIZE))
                .unwrap_or(usize::MAX);
            let rect = Rect {
                rows: spans.map(|(row, _)| row),
                start: first.start,
                cols,
                mark,
                flagged,
            };
            let width = first.len();
            let grids = (self.cells.as_mut_slice(), &mut self.plane);
            match width {
                1 => rect.fill::<1>(grids, &facts),
                2 => rect.fill::<2>(grids, &facts),
                4 => rect.fill::<4>(grids, &facts),
                8 => rect.fill::<8>(grids, &facts),
                16 => rect.fill::<16>(grids, &facts),
                _ => rect.fill_any(grids, &facts, width),
            }
        }
        Ok(())
    }
}

/// One record's cells in the luma grid: its cell rows `rows` of the
/// `cols`-cell grid from cell `start` on, the cell value `mark` naming it,
/// and the column (from its first) of its prediction unit's left edge.
struct Rect<R> {
    rows: R,
    start: usize,
    cols: usize,
    mark: MiCell,
    flagged: usize,
}

impl<R: Iterator<Item = usize>> Rect<R> {
    #[allow(clippy::inline_always, reason = "one fixed-width store per row")]
    #[inline(always)]
    fn fill<const N: usize>(self, (cells, plane): (&mut [MiCell], &mut CellPlane), facts: &Facts) {
        let mut at = self.start;
        for row in self.rows {
            let (_, info) = facts.at(usize::MAX, row * MI_SIZE);
            let (Some(cells), Some(bases), Some(infos)) = (
                cells
                    .get_mut(at..)
                    .and_then(|cells| cells.first_chunk_mut::<N>()),
                plane
                    .bases
                    .get_mut(at..)
                    .and_then(|cells| cells.first_chunk_mut::<N>()),
                plane
                    .info
                    .get_mut(at..)
                    .and_then(|cells| cells.first_chunk_mut::<N>()),
            ) else {
                return;
            };
            *cells = [self.mark; N];
            *bases = [facts.base; N];
            *infos =
                core::array::from_fn(|col| info | if col == self.flagged { EDGE_V } else { 0 });
            at += self.cols;
        }
    }

    fn fill_any(
        self,
        (cells, plane): (&mut [MiCell], &mut CellPlane),
        facts: &Facts,
        width: usize,
    ) {
        let mut at = self.start;
        for row in self.rows {
            if let Some(cells) = cells.get_mut(at..at + width) {
                cells.fill(self.mark);
            }
            let facts = facts.at(usize::MAX, row * MI_SIZE);
            plane.fill_span(at..at + width, facts, (self.flagged, EDGE_V));
            at += self.cols;
        }
    }
}

impl ChromaMiGridStorage {
    pub(super) fn new(
        mi_cols: usize,
        (sub_x, sub_y): (usize, usize),
        storage: &mut ChromaGridVecs,
    ) -> Self {
        Self {
            cells: core::mem::take(&mut storage.0),
            plane: core::mem::take(&mut storage.1),
            cell_cols: mi_cols.div_ceil(1 << sub_x),
            sub_x,
            sub_y,
        }
    }

    /// Lays one chroma plane's overlay records out over the frame in record
    /// order after the luma grid's own fill, then resolves every cell's
    /// [`CellPlane`] entry.
    pub(super) fn fill(
        &mut self,
        base: &MiGridStorage,
        (blocks, records): (&[DeblockBlock], &ChromaDeblockRecords),
        plane: usize,
        mi_rows: usize,
    ) -> Result<(), DeblockError> {
        let plane_id = match plane {
            0 => splot_recon::PlaneId::U,
            1 => splot_recon::PlaneId::V,
            _ => return Err(DeblockError::Workspace),
        };
        let mask = [ChromaDeblockRecords::U, ChromaDeblockRecords::V][plane];
        let (sub_x, sub_y, cols) = (self.sub_x, self.sub_y, self.cell_cols);
        let len = mi_rows
            .div_ceil(1 << sub_y)
            .checked_mul(cols)
            .ok_or(DeblockError::Workspace)?;
        reset(&mut self.cells, len, ChromaMiCell::default(), plane_id)?;
        self.plane.reset(len, plane_id)?;
        let extent = (mi_rows, base.mi_cols);
        for (index, record) in records.blocks.iter().enumerate() {
            if record.planes & mask == 0 {
                continue;
            }
            let mark = mi_block_index(index)? + 1;
            let block = &record.block;
            for (_, span) in block_spans(block, extent, (sub_x, sub_y), cols) {
                if let Some(cells) = self.cells.get_mut(span) {
                    for cell in cells {
                        if block.chroma_transform_only {
                            cell.chroma_transform = mark;
                        } else {
                            cell.overlay = mark;
                        }
                    }
                }
            }
        }
        self.resolve(base, blocks, records);
        Ok(())
    }

    /// Resolves every chroma cell's [`CellPlane`] entry from its overlay
    /// cells and the luma cell under it.
    fn resolve(
        &mut self,
        base: &MiGridStorage,
        blocks: &[DeblockBlock],
        records: &ChromaDeblockRecords,
    ) {
        let (sub_x, sub_y, cols) = (self.sub_x, self.sub_y, self.cell_cols);
        let mut last = ((NO_BLOCK, NO_BLOCK, NO_BLOCK), None);
        let plane = &mut self.plane;
        let rows = self
            .cells
            .chunks_exact(cols.max(1))
            .zip(plane.bases.chunks_exact_mut(cols.max(1)))
            .zip(plane.info.chunks_exact_mut(cols.max(1)))
            .enumerate();
        for (row, ((cells, bases), info)) in rows {
            let luma = base
                .cells
                .get((row << sub_y) * base.mi_cols..)
                .unwrap_or_default();
            for (col, (cell, (base_cell, info_cell))) in cells
                .iter()
                .zip(bases.iter_mut().zip(info.iter_mut()))
                .enumerate()
            {
                let luma = luma.get(col << sub_x).map_or(NO_BLOCK, |cell| cell.base);
                let key = (luma, cell.overlay, cell.chroma_transform);
                if key != last.0 {
                    let block = if cell.overlay != NO_BLOCK {
                        records.get(record(cell.overlay))
                    } else {
                        blocks.get(record(luma))
                    };
                    let chroma_transform = if cell.chroma_transform != NO_BLOCK {
                        records.get(record(cell.chroma_transform))
                    } else {
                        None
                    };
                    let facts = block.map(|block| {
                        let edge = EdgeBlock {
                            block,
                            chroma_transform,
                        };
                        Facts::of(edge, 1, (sub_x, sub_y))
                    });
                    last = (key, facts);
                }
                if let Some(facts) = &last.1 {
                    (*base_cell, *info_cell) = facts.at(col * MI_SIZE, row * MI_SIZE);
                }
            }
        }
    }
}

fn mi_block_index(index: usize) -> Result<u32, DeblockError> {
    let index = u32::try_from(index).map_err(|_| DeblockError::Workspace)?;
    if index == u32::MAX {
        return Err(DeblockError::Workspace);
    }
    Ok(index)
}
