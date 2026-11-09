// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::facts::PlaneFacts;
use super::{DeblockBlock, DeblockError, EdgeBlock};

/// A cell holds its record index plus one, so that an empty cell is zero.
const NO_BLOCK: u32 = 0;

#[derive(Clone, Copy, Default)]
pub(super) struct MiCell {
    pub(super) base: u32,
}

pub(super) fn reset<T: Copy>(
    cells: &mut Vec<T>,
    len: usize,
    fill: T,
    plane: splot_recon::PlaneId,
) -> Result<(), DeblockError> {
    cells.clear();
    resize(cells, len, fill, plane)
}

/// Sizes `cells` to `len`, keeping the values of the cells it already had.
pub(super) fn resize<T: Copy>(
    cells: &mut Vec<T>,
    len: usize,
    fill: T,
    plane: splot_recon::PlaneId,
) -> Result<(), DeblockError> {
    cells.truncate(len);
    cells
        .try_reserve(len - cells.len())
        .map_err(|_| DeblockError::Allocation {
            plane,
            context: "deblock MI grid",
        })?;
    cells.resize(len, fill);
    Ok(())
}

/// The record a cell value names; an empty cell names none.
const fn record(cell: u32) -> usize {
    (cell as usize).wrapping_sub(1)
}

/// The luma grid: the record of every mode-info cell, which the sub-PU
/// edge decisions read.
pub(super) struct MiGridStorage {
    pub(super) mi_cols: usize,
    pub(super) cells: Vec<MiCell>,
}

pub(super) struct ChromaMiGridStorage {
    /// One cell per chroma mode-info unit, not per luma one.
    pub(super) cells: Vec<MiCell>,
    pub(super) cell_cols: usize,
    pub(super) sub_x: usize,
    pub(super) sub_y: usize,
}

pub(super) struct MiGrid<'a> {
    pub(super) base: &'a MiGridStorage,
    pub(super) chroma: Option<&'a ChromaMiGridStorage>,
    pub(super) base_blocks: &'a [DeblockBlock],
    pub(super) overlay_blocks: &'a [DeblockBlock],
    /// The plane's cell facts, which the edge decisions of a frame without
    /// sub-PU edges read instead of the records.
    pub(super) facts: Option<&'a PlaneFacts>,
}

impl MiGrid<'_> {
    pub(super) const fn new<'a>(
        base: &'a MiGridStorage,
        chroma: Option<&'a ChromaMiGridStorage>,
        base_blocks: &'a [DeblockBlock],
        overlay_blocks: &'a [DeblockBlock],
    ) -> MiGrid<'a> {
        MiGrid {
            base,
            chroma,
            base_blocks,
            overlay_blocks,
            facts: None,
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
        let block = match chroma.map(|cell| cell.base) {
            Some(overlay) if overlay != NO_BLOCK => self.overlay_blocks.get(record(overlay))?,
            _ => self.base_blocks.get(record(base.base))?,
        };
        Some(EdgeBlock { block })
    }
}

/// The grid vectors one frame's deblock fills.
///
/// They travel on the frame filter records, so the next frame lays its grids
/// out over the ones the last frame left rather than sizing new ones.
#[derive(Default)]
pub(crate) struct DeblockGridStorage {
    pub(super) cells: Vec<MiCell>,
    pub(super) chroma: Vec<MiCell>,
    pub(super) facts: [PlaneFacts; 2],
}

/// The cell rows and columns `block` covers in a grid whose cells are
/// `1 << sub` mode-info units wide and high.
pub(super) fn block_cells(
    block: &DeblockBlock,
    (mi_rows, mi_cols): (usize, usize),
    (sub_x, sub_y): (usize, usize),
) -> [core::ops::Range<usize>; 2] {
    let (r, c) = (block.r as usize, block.c as usize);
    let row_end = r.saturating_add(block.n4h as usize).min(mi_rows);
    let col_end = c.saturating_add(block.n4w as usize).min(mi_cols);
    [
        (r >> sub_y)..row_end.div_ceil(1 << sub_y),
        (c.min(col_end) >> sub_x)..col_end.div_ceil(1 << sub_x),
    ]
}

/// The cell spans `block` covers, one per cell row, in a grid of `cols`
/// cells per row whose cells are `1 << sub` mode-info units wide and high.
pub(super) fn block_spans(
    block: &DeblockBlock,
    extent: (usize, usize),
    sub: (usize, usize),
    cols: usize,
) -> impl Iterator<Item = (usize, core::ops::Range<usize>)> + use<> {
    let [rows, columns] = block_cells(block, extent, sub);
    rows.map(move |row| {
        let base = row * cols;
        (row, base + columns.start..base + columns.end)
    })
}

impl MiGridStorage {
    pub(super) fn new(mi_cols: usize, storage: &mut DeblockGridStorage) -> Self {
        Self {
            mi_cols,
            cells: core::mem::take(&mut storage.cells),
        }
    }

    /// Marks every cell of a `mi_rows`-row frame empty.
    pub(super) fn reset(&mut self, mi_rows: usize) -> Result<(), DeblockError> {
        let plane = splot_recon::PlaneId::Y;
        let len = mi_rows
            .checked_mul(self.mi_cols)
            .ok_or(DeblockError::Workspace)?;
        reset(&mut self.cells, len, MiCell::default(), plane)
    }

    /// Lays the records from `blocks[start]` on out over the frame in record
    /// order, so that a later record overwrites an earlier one.
    pub(super) fn fill(
        &mut self,
        blocks: &[DeblockBlock],
        start: usize,
        mi_rows: usize,
    ) -> Result<(), DeblockError> {
        let cols = self.mi_cols;
        for (index, block) in blocks.iter().enumerate().skip(start) {
            let mark = MiCell {
                base: mi_block_index(index)? + 1,
            };
            for (_, span) in block_spans(block, (mi_rows, cols), (0, 0), cols) {
                if let Some(cells) = self.cells.get_mut(span) {
                    cells.fill(mark);
                }
            }
        }
        Ok(())
    }
}

impl ChromaMiGridStorage {
    pub(super) fn new(
        mi_cols: usize,
        (sub_x, sub_y): (usize, usize),
        storage: &mut Vec<MiCell>,
    ) -> Self {
        Self {
            cells: core::mem::take(storage),
            cell_cols: mi_cols.div_ceil(1 << sub_x),
            sub_x,
            sub_y,
        }
    }

    /// Marks every cell of a `mi_rows`-row frame empty.
    pub(super) fn reset(&mut self, mi_rows: usize) -> Result<(), DeblockError> {
        let len = mi_rows
            .div_ceil(1 << self.sub_y)
            .checked_mul(self.cell_cols)
            .ok_or(DeblockError::Workspace)?;
        reset(&mut self.cells, len, MiCell::default(), splot_recon::PlaneId::U)
    }

    /// Lays the chroma overlay records `records`, the first of which is
    /// record `first`, out over the frame in record order.
    pub(super) fn fill(
        &mut self,
        mi_cols: usize,
        records: &[DeblockBlock],
        first: usize,
        mi_rows: usize,
    ) -> Result<(), DeblockError> {
        let (sub_x, sub_y, cols) = (self.sub_x, self.sub_y, self.cell_cols);
        let extent = (mi_rows, mi_cols);
        for (index, block) in (first..).zip(records) {
            let mark = MiCell {
                base: mi_block_index(index)? + 1,
            };
            for (_, span) in block_spans(block, extent, (sub_x, sub_y), cols) {
                if let Some(cells) = self.cells.get_mut(span) {
                    cells.fill(mark);
                }
            }
        }
        Ok(())
    }
}

fn mi_block_index(index: usize) -> Result<u32, DeblockError> {
    let index = u32::try_from(index).map_err(|_| DeblockError::Workspace)?;
    if index == u32::MAX {
        return Err(DeblockError::Workspace);
    }
    Ok(index)
}
