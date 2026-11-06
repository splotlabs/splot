// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use core::ops::Range;

use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use super::{ChromaDeblockRecords, DeblockBlock, DeblockError, EdgeBlock, MI_SIZE};

/// A cell holds its record index plus one, so that an empty cell is zero and a
/// window grows by zeroing.
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

/// What one plane 4x4 cell's records give the § 7.17.2 decisions of the
/// edges it is a side of: its transform block's base and size, its
/// prediction unit's plane origin, and its skip, lossless and `qindex`.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(super) struct CellFacts {
    /// Transform base mode-info row and column, packed `row << 16 | col`.
    pub(super) tx_base: u32,
    /// Prediction unit origin in plane samples, `x` then `y`.
    pub(super) prediction: [u16; 2],
    /// `Tx_Width` then `Tx_Height` of the transform.
    pub(super) tx_size: [u8; 2],
    pub(super) qindex: u16,
    pub(super) flags: u8,
}

impl CellFacts {
    pub(super) const SKIP: u8 = 1;
    pub(super) const LOSSLESS: u8 = 2;
    pub(super) const COVERED: u8 = 4;

    /// The facts `cell` gives plane `plane` with subsampling (`sub_x`, `sub_y`).
    fn of(cell: EdgeBlock<'_>, plane: usize, (sub_x, sub_y): (usize, usize)) -> Self {
        let (tx_row, tx_col) = cell.tx_base(plane);
        let prediction = cell.prediction(plane);
        let tx = cell.tx(plane);
        let dimension = |table: &[i32]| table.get(tx).map_or(0, |&size| size.clamp(0, 255) as u8);
        let block = cell.block;
        Self {
            tx_base: (tx_row as u32) << 16 | (tx_col as u32 & 0xffff),
            prediction: [
                ((prediction.base_c as usize * MI_SIZE) >> sub_x) as u16,
                ((prediction.base_r as usize * MI_SIZE) >> sub_y) as u16,
            ],
            tx_size: [dimension(&TX_WIDTH), dimension(&TX_HEIGHT)],
            qindex: block.qindex.min(u32::from(u16::MAX)) as u16,
            flags: Self::COVERED
                | if block.skip { Self::SKIP } else { 0 }
                | if block.lossless { Self::LOSSLESS } else { 0 },
        }
    }
}

/// The record a cell value names; an empty cell names none.
const fn record(cell: u32) -> usize {
    (cell as usize).wrapping_sub(1)
}

/// The luma grid over the mode-info rows its window has built; reads outside
/// them find no cell.
pub(super) struct MiGridStorage {
    pub(super) mi_cols: usize,
    pub(super) window: Window,
    pub(super) cells: Vec<MiCell>,
    /// The luma [`CellFacts`] of every cell.
    pub(super) facts: Vec<CellFacts>,
}

pub(super) struct ChromaMiGridStorage {
    /// One cell per chroma mode-info unit, not per luma one. A chroma deblock
    /// record covers a chroma-aligned luma extent, so a luma-resolution grid
    /// stored each block index `1 << (sub_x + sub_y)` times over.
    pub(super) cells: Vec<ChromaMiCell>,
    /// The chroma [`CellFacts`] of every cell.
    pub(super) facts: Vec<CellFacts>,
    pub(super) cell_cols: usize,
    pub(super) window: Window,
    pub(super) sub_x: usize,
    pub(super) sub_y: usize,
}

pub(super) struct MiGrid<'a> {
    pub(super) base: &'a MiGridStorage,
    pub(super) chroma: Option<&'a ChromaMiGridStorage>,
    pub(super) base_blocks: &'a [DeblockBlock],
    pub(super) overlay_blocks: &'a ChromaDeblockRecords,
    /// The window's first luma and chroma cell; an index below it wraps past
    /// every cell, so a read outside the window finds none.
    offset: usize,
    chroma_offset: usize,
}

impl MiGrid<'_> {
    pub(super) fn new<'a>(
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
            offset: base.window.row_base * base.mi_cols,
            chroma_offset: chroma.map_or(0, |grid| {
                (grid.window.row_base >> grid.sub_y) * grid.cell_cols
            }),
        }
    }

    #[allow(clippy::inline_always, reason = "measured luma deblock hot path")]
    #[inline(always)]
    const fn index(&self, row: usize, col: usize) -> usize {
        row.wrapping_mul(self.base.mi_cols)
            .wrapping_add(col)
            .wrapping_sub(self.offset)
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
                    .wrapping_add(col >> grid.sub_x)
                    .wrapping_sub(self.chroma_offset),
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

    /// The [`CellFacts`] of plane cell row `row`: luma cells, or chroma cells
    /// when the grid has a chroma overlay. `None` outside the window.
    pub(super) fn facts_row(&self, row: usize) -> Option<&[CellFacts]> {
        let (facts, cols, first) = match self.chroma {
            Some(grid) => (
                &grid.facts,
                grid.cell_cols,
                grid.window.row_base >> grid.sub_y,
            ),
            None => (
                &self.base.facts,
                self.base.mi_cols,
                self.base.window.row_base,
            ),
        };
        let start = row.checked_sub(first)?.checked_mul(cols)?;
        facts.get(start..start.checked_add(cols)?)
    }
}

/// The mode-info rows a grid holds: `row_base` up to the rows built so far.
///
/// Deblock advances down the frame, so a window only drops rows off its front
/// and builds new ones past its end; every row is built once.
#[derive(Clone, Copy, Default)]
pub(super) struct Window {
    pub(super) row_base: usize,
    built_end: usize,
}

impl Window {
    /// Moves the window to `rows`, returning the rows to drop from its front
    /// (`None`: all of them) and the rows to build, or `None` when `rows` is
    /// already built.
    fn slide(&mut self, rows: &Range<usize>) -> Option<(Option<usize>, Range<usize>)> {
        if rows.start >= self.row_base && rows.end <= self.built_end {
            return None;
        }
        let moved = if rows.start >= self.row_base && rows.start < self.built_end {
            (Some(rows.start - self.row_base), self.built_end..rows.end)
        } else {
            (None, rows.clone())
        };
        self.row_base = rows.start;
        self.built_end = rows.end;
        Some(moved)
    }
}

/// Record indices bucketed by first mode-info row, so a window finds the
/// records that reach it without walking the frame's whole list.
#[derive(Default)]
pub(crate) struct RowOrder {
    /// `ends[row]` is the end of bucket `row` in `entries`; the last bucket
    /// holds every record starting at or past the frame bottom.
    ends: Vec<u32>,
    entries: Vec<u32>,
    tallest: usize,
}

impl RowOrder {
    pub(super) fn sort<'a>(
        &mut self,
        blocks: impl ExactSizeIterator<Item = &'a DeblockBlock> + Clone,
        mi_rows: usize,
    ) -> Result<(), DeblockError> {
        let allocation = |_| DeblockError::Allocation {
            plane: splot_recon::PlaneId::Y,
            context: "deblock record order",
        };
        let bucket = |block: &DeblockBlock| (block.r as usize).min(mi_rows);
        self.ends.clear();
        self.ends
            .try_reserve_exact(mi_rows + 2)
            .map_err(allocation)?;
        self.ends.resize(mi_rows + 2, 0);
        self.entries.clear();
        self.entries.try_reserve(blocks.len()).map_err(allocation)?;
        self.entries.resize(blocks.len(), 0);
        self.tallest = 0;
        for block in blocks.clone() {
            self.ends[bucket(block) + 1] += 1;
            self.tallest = self.tallest.max(block.n4h as usize);
        }
        for row in 1..self.ends.len() {
            self.ends[row] += self.ends[row - 1];
        }
        for (index, block) in blocks.enumerate() {
            let slot = &mut self.ends[bucket(block)];
            *self
                .entries
                .get_mut(*slot as usize)
                .ok_or(DeblockError::Workspace)? = mi_block_index(index)?;
            *slot += 1;
        }
        Ok(())
    }

    /// Records whose rows, or whose bottom edge, fall in `rows`, in no
    /// particular order.
    fn reaching(&self, rows: &Range<usize>) -> &[u32] {
        let begin = |row: usize| match row.checked_sub(1) {
            Some(previous) => self
                .ends
                .get(previous)
                .map_or(self.entries.len(), |&end| end as usize),
            None => 0,
        };
        let to = if rows.end + 2 >= self.ends.len() {
            self.entries.len()
        } else {
            begin(rows.end)
        };
        self.entries
            .get(begin(rows.start.saturating_sub(self.tallest))..to)
            .unwrap_or_default()
    }
}

/// The grid vectors and record orders one frame's deblock fills.
///
/// They travel on the frame filter records, so the next frame lays its grids
/// out over the ones the last frame left rather than sizing new ones.
#[derive(Default)]
pub(crate) struct DeblockGridStorage {
    pub(super) cells: Vec<MiCell>,
    pub(super) facts: Vec<CellFacts>,
    pub(super) chroma: [(Vec<ChromaMiCell>, Vec<CellFacts>); 2],
    pub(super) order: [RowOrder; 2],
}

/// Drops `dropped` rows of `row_len` cells off the front (all on `None`) and
/// appends `rows` default rows.
fn slide_cells<T: Clone + Default>(
    cells: &mut Vec<T>,
    dropped: Option<usize>,
    row_len: usize,
    rows: usize,
    plane: splot_recon::PlaneId,
) -> Result<(), DeblockError> {
    match dropped {
        Some(dropped) => drop(cells.drain(..(dropped * row_len).min(cells.len()))),
        None => cells.clear(),
    }
    let count = rows.checked_mul(row_len).ok_or(DeblockError::Workspace)?;
    cells
        .try_reserve(count)
        .map_err(|_| DeblockError::Allocation {
            plane,
            context: "deblock MI grid",
        })?;
    cells.resize(cells.len() + count, T::default());
    Ok(())
}

/// Records overwrite each other in record order, so the higher index wins
/// whichever order a window visits them in.
fn later(current: u32, index: u32) -> u32 {
    current.max(index + 1)
}

impl MiGridStorage {
    pub(super) fn new(mi_cols: usize, storage: &mut DeblockGridStorage) -> Self {
        Self {
            mi_cols,
            window: Window::default(),
            cells: core::mem::take(&mut storage.cells),
            facts: core::mem::take(&mut storage.facts),
        }
    }

    /// Moves the grid window to mode-info `rows`, building the rows it lacks
    /// from the records that reach them.
    pub(super) fn fill(
        &mut self,
        blocks: &[DeblockBlock],
        order: &RowOrder,
        mi_rows: usize,
        rows: &Range<usize>,
    ) -> Result<(), DeblockError> {
        let Some((dropped, new)) = self.window.slide(rows) else {
            return Ok(());
        };
        let plane = splot_recon::PlaneId::Y;
        slide_cells(&mut self.cells, dropped, self.mi_cols, new.len(), plane)?;
        slide_cells(&mut self.facts, dropped, self.mi_cols, new.len(), plane)?;
        let base = self.window.row_base;
        for &index in order.reaching(&new) {
            let block = blocks.get(index as usize).ok_or(DeblockError::Workspace)?;
            if ends_above(block, new.start) {
                continue;
            }
            for (start, end) in block_row_spans(block, mi_rows, self.mi_cols, &new, base) {
                if let Some(cells) = self.cells.get_mut(start..end) {
                    for cell in cells {
                        cell.base = later(cell.base, index);
                    }
                }
            }
        }
        let built = (new.start - base) * self.mi_cols;
        let mut last = (NO_BLOCK, CellFacts::default());
        for (cell, facts) in self.cells[built..].iter().zip(&mut self.facts[built..]) {
            if cell.base != last.0 {
                let Some(block) = blocks.get(record(cell.base)) else {
                    *facts = CellFacts::default();
                    continue;
                };
                let edge = EdgeBlock {
                    block,
                    chroma_transform: None,
                };
                last = (cell.base, CellFacts::of(edge, 0, (0, 0)));
            }
            *facts = last.1;
        }
        Ok(())
    }
}

impl ChromaMiGridStorage {
    pub(super) fn new(
        mi_cols: usize,
        (sub_x, sub_y): (usize, usize),
        storage: &mut (Vec<ChromaMiCell>, Vec<CellFacts>),
    ) -> Self {
        Self {
            cells: core::mem::take(&mut storage.0),
            facts: core::mem::take(&mut storage.1),
            cell_cols: mi_cols.div_ceil(1 << sub_x),
            window: Window::default(),
            sub_x,
            sub_y,
        }
    }

    /// Moves one chroma plane's overlay window to mode-info `rows` after the
    /// luma grid's own move. `rows` starts on a chroma cell row and ends on
    /// one or at the frame bottom.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill(
        &mut self,
        base: &MiGridStorage,
        blocks: &[DeblockBlock],
        records: &ChromaDeblockRecords,
        order: &RowOrder,
        plane: usize,
        mi_rows: usize,
        rows: &Range<usize>,
    ) -> Result<(), DeblockError> {
        let plane_id = match plane {
            0 => splot_recon::PlaneId::U,
            1 => splot_recon::PlaneId::V,
            _ => return Err(DeblockError::Workspace),
        };
        let Some((dropped, new)) = self.window.slide(rows) else {
            return Ok(());
        };
        let mask = [ChromaDeblockRecords::U, ChromaDeblockRecords::V][plane];
        let (mi_cols, sub_x, sub_y) = (base.mi_cols, self.sub_x, self.sub_y);
        let row_base = self.window.row_base;
        let cell_rows = (new.start >> sub_y)..new.end.div_ceil(1 << sub_y);
        slide_cells(
            &mut self.cells,
            dropped.map(|rows| rows >> sub_y),
            self.cell_cols,
            cell_rows.len(),
            plane_id,
        )?;
        slide_cells(
            &mut self.facts,
            dropped.map(|rows| rows >> sub_y),
            self.cell_cols,
            cell_rows.len(),
            plane_id,
        )?;
        for &index in order.reaching(&new) {
            let record = records
                .blocks
                .get(index as usize)
                .ok_or(DeblockError::Workspace)?;
            if record.planes & mask == 0 || ends_above(&record.block, new.start) {
                continue;
            }
            let block = &record.block;
            let spans = block_chroma_spans(
                block,
                mi_rows,
                mi_cols,
                (sub_x, sub_y, self.cell_cols),
                &cell_rows,
                row_base >> sub_y,
            );
            let marks = (later(NO_BLOCK, index), NO_BLOCK);
            let (overlay, transform) = if block.chroma_transform_only {
                (marks.1, marks.0)
            } else {
                marks
            };
            for (start, end) in spans {
                if let Some(cells) = self.cells.get_mut(start..end) {
                    for cell in cells {
                        cell.overlay = cell.overlay.max(overlay);
                        cell.chroma_transform = cell.chroma_transform.max(transform);
                    }
                }
            }
        }
        self.resolve_facts(base, blocks, records, &cell_rows)
    }

    /// Resolves the [`CellFacts`] of chroma `cell_rows`, which the window
    /// has just built, from their overlay cells and the luma cells under them.
    fn resolve_facts(
        &mut self,
        base: &MiGridStorage,
        blocks: &[DeblockBlock],
        records: &ChromaDeblockRecords,
        cell_rows: &Range<usize>,
    ) -> Result<(), DeblockError> {
        let (sub_x, sub_y, cols) = (self.sub_x, self.sub_y, self.cell_cols);
        let first = self.window.row_base >> sub_y;
        let mut last = ((NO_BLOCK, NO_BLOCK, NO_BLOCK), CellFacts::default());
        for row in cell_rows.clone() {
            let luma_row = (row << sub_y)
                .checked_sub(base.window.row_base)
                .ok_or(DeblockError::Workspace)?;
            let at = (row - first) * cols;
            let (Some(cells), Some(facts)) = (
                self.cells.get(at..at + cols),
                self.facts.get_mut(at..at + cols),
            ) else {
                return Err(DeblockError::Workspace);
            };
            for (col, (cell, facts)) in cells.iter().zip(facts).enumerate() {
                let luma = base
                    .cells
                    .get(luma_row * base.mi_cols + (col << sub_x))
                    .map_or(NO_BLOCK, |cell| cell.base);
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
                    let facts = block.map_or_else(CellFacts::default, |block| {
                        let edge = EdgeBlock {
                            block,
                            chroma_transform,
                        };
                        CellFacts::of(edge, 1, (sub_x, sub_y))
                    });
                    last = (key, facts);
                }
                *facts = last.1;
            }
        }
        Ok(())
    }
}

/// Whether `block` ends above mode-info row `row`: a window built from `row`
/// on holds neither its cells nor its bottom edge. [`RowOrder::reaching`]
/// finds such records when a taller record of the frame widens its reach.
fn ends_above(block: &DeblockBlock, row: usize) -> bool {
    (block.r as usize).saturating_add(block.n4h as usize) < row
}

/// The chroma-resolution cell spans one block covers in chroma `cell_rows`,
/// one per chroma row, offset from chroma row `first`.
fn block_chroma_spans(
    block: &DeblockBlock,
    mi_rows: usize,
    mi_cols: usize,
    (sub_x, sub_y, cell_cols): (usize, usize, usize),
    cell_rows: &Range<usize>,
    first: usize,
) -> impl Iterator<Item = (usize, usize)> + use<> {
    let (r, c) = (block.r as usize, block.c as usize);
    let row_end = r.saturating_add(block.n4h as usize).min(mi_rows);
    let col_end = c.saturating_add(block.n4w as usize).min(mi_cols);
    let col_start = c.min(col_end);
    let chroma_col_start = col_start >> sub_x;
    let chroma_col_end = col_end.div_ceil(1 << sub_x);
    let chroma_rows =
        (r >> sub_y).max(cell_rows.start)..row_end.div_ceil(1 << sub_y).min(cell_rows.end);
    chroma_rows.map(move |chroma_row| {
        let base = (chroma_row - first) * cell_cols;
        (base + chroma_col_start, base + chroma_col_end)
    })
}

/// The luma cell spans one block covers in `rows`, one per mode-info row,
/// offset from row `first`.
fn block_row_spans(
    block: &DeblockBlock,
    mi_rows: usize,
    mi_cols: usize,
    rows: &Range<usize>,
    first: usize,
) -> impl Iterator<Item = (usize, usize)> + use<> {
    let (r, c) = (block.r as usize, block.c as usize);
    let row_end = r.saturating_add(block.n4h as usize).min(mi_rows);
    let col_end = c.saturating_add(block.n4w as usize).min(mi_cols);
    let col_start = c.min(col_end);
    (r.max(rows.start)..row_end.min(rows.end)).map(move |row| {
        let base = (row - first) * mi_cols;
        (base + col_start, base + col_end)
    })
}

fn mi_block_index(index: usize) -> Result<u32, DeblockError> {
    let index = u32::try_from(index).map_err(|_| DeblockError::Workspace)?;
    if index == u32::MAX {
        return Err(DeblockError::Workspace);
    }
    Ok(index)
}
