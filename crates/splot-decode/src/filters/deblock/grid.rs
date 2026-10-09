// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use core::ops::Range;

use super::{
    COVERED_CANDIDATE, ChromaDeblockRecords, DeblockBlock, DeblockError, EdgeBlock,
    HORIZONTAL_TX_CANDIDATE, SUB_PU_CANDIDATE, VERTICAL_TX_CANDIDATE,
};

const NO_BLOCK_INDEX: u32 = u32::MAX;

#[derive(Clone, Copy)]
pub(super) struct MiCell {
    pub(super) base: u32,
}

impl Default for MiCell {
    fn default() -> Self {
        Self {
            base: NO_BLOCK_INDEX,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ChromaMiCell {
    pub(super) overlay: u32,
    pub(super) chroma_transform: u32,
}

impl Default for ChromaMiCell {
    fn default() -> Self {
        Self {
            overlay: NO_BLOCK_INDEX,
            chroma_transform: NO_BLOCK_INDEX,
        }
    }
}

/// The luma grid over the mode-info rows its window has built; reads outside
/// them find no cell.
pub(super) struct MiGridStorage {
    pub(super) mi_cols: usize,
    pub(super) window: Window,
    pub(super) fully_covered: bool,
    pub(super) cells: Vec<MiCell>,
    pub(super) candidates: Vec<u8>,
}

pub(super) struct ChromaMiGridStorage {
    pub(super) fully_covered: bool,
    /// One cell per chroma mode-info unit, not per luma one. A chroma deblock
    /// record covers a chroma-aligned luma extent, so a luma-resolution grid
    /// stored each block index `1 << (sub_x + sub_y)` times over.
    pub(super) cells: Vec<ChromaMiCell>,
    pub(super) cell_cols: usize,
    pub(super) window: Window,
    pub(super) sub_x: usize,
    pub(super) sub_y: usize,
    /// Edge flags stay at luma resolution: a vertical edge at luma column `c`
    /// is distinct from the one at `c - 1`, which `is_candidate` reads.
    pub(super) candidates: Vec<u8>,
}

pub(super) struct MiGrid<'a> {
    pub(super) base: &'a MiGridStorage,
    pub(super) chroma: Option<&'a ChromaMiGridStorage>,
    pub(super) candidates: &'a [u8],
    pub(super) fully_covered: bool,
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
            candidates: chroma.map_or(&base.candidates, |grid| &grid.candidates),
            fully_covered: chroma.map_or(base.fully_covered, |grid| grid.fully_covered),
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

    /// One mode-info row's edge flags, or `None` outside the window.
    pub(super) fn candidate_row(&self, row: usize) -> Option<&[u8]> {
        let start = self.index(row, 0);
        self.candidates
            .get(start..start.wrapping_add(self.base.mi_cols))
    }

    #[allow(clippy::inline_always, reason = "measured luma deblock hot path")]
    #[inline(always)]
    pub(super) fn get_luma_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let cell = self.base.cells.get(self.index(row, col))?;
        Some(EdgeBlock {
            block: self.base_blocks.get(cell.base as usize)?,
            chroma_transform: None,
        })
    }

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
            Some(overlay) if overlay != NO_BLOCK_INDEX => {
                self.overlay_blocks.get(overlay as usize)?
            }
            _ => self.base_blocks.get(base.base as usize)?,
        };
        let chroma_transform = match chroma.map(|cell| cell.chroma_transform) {
            Some(transform) if transform != NO_BLOCK_INDEX => {
                Some(self.overlay_blocks.get(transform as usize)?)
            }
            _ => None,
        };
        Some(EdgeBlock {
            block,
            chroma_transform,
        })
    }

    #[cfg(test)]
    pub(super) fn is_candidate(
        &self,
        row: usize,
        col: usize,
        pass: usize,
        allow_sub_pu: bool,
        plane_sub_x: usize,
        plane_sub_y: usize,
    ) -> bool {
        let candidate = if pass == 0 {
            VERTICAL_TX_CANDIDATE
        } else {
            HORIZONTAL_TX_CANDIDATE
        };
        let index = self.index(row, col);
        let Some(&current) = self.candidates.get(index) else {
            return true;
        };
        if !self.fully_covered && current & COVERED_CANDIDATE == 0 {
            return true;
        }
        if current & candidate != 0 || allow_sub_pu && current & SUB_PU_CANDIDATE != 0 {
            return true;
        }
        if pass == 0 && plane_sub_x != 0 && col != 0 {
            return self
                .candidates
                .get(index - 1)
                .is_none_or(|flags| flags & VERTICAL_TX_CANDIDATE != 0);
        }
        if pass == 1 && plane_sub_y != 0 && row != 0 {
            return index
                .checked_sub(self.base.mi_cols)
                .and_then(|above| self.candidates.get(above))
                .is_none_or(|flags| flags & HORIZONTAL_TX_CANDIDATE != 0);
        }
        false
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
    pub(super) candidates: Vec<u8>,
    pub(super) chroma: [(Vec<ChromaMiCell>, Vec<u8>); 2],
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
    if current.wrapping_add(1) <= index {
        index
    } else {
        current
    }
}

fn all_covered(candidates: &[u8]) -> bool {
    candidates
        .iter()
        .fold(u8::MAX, |all, candidate| all & candidate)
        & COVERED_CANDIDATE
        != 0
}

impl MiGridStorage {
    pub(super) fn new(mi_cols: usize, storage: &mut DeblockGridStorage) -> Self {
        Self {
            mi_cols,
            window: Window::default(),
            fully_covered: true,
            cells: core::mem::take(&mut storage.cells),
            candidates: core::mem::take(&mut storage.candidates),
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
        slide_cells(
            &mut self.candidates,
            dropped,
            self.mi_cols,
            new.len(),
            plane,
        )?;
        if dropped.is_none() {
            self.fully_covered = true;
        }
        let base = self.window.row_base;
        for &index in order.reaching(&new) {
            let block = blocks.get(index as usize).ok_or(DeblockError::Workspace)?;
            for (start, end) in block_row_spans(block, mi_rows, self.mi_cols, &new, base) {
                if let Some(cells) = self.cells.get_mut(start..end) {
                    for cell in cells {
                        cell.base = later(cell.base, index);
                    }
                }
                if let Some(candidates) = self.candidates.get_mut(start..end) {
                    for candidate in candidates {
                        *candidate |= COVERED_CANDIDATE;
                    }
                }
            }
            mark_block_candidates(
                &mut self.candidates,
                block,
                mi_rows,
                self.mi_cols,
                &new,
                base,
            );
        }
        let built = (new.start - base) * self.mi_cols;
        self.fully_covered &= all_covered(self.candidates.get(built..).unwrap_or_default());
        Ok(())
    }
}

impl ChromaMiGridStorage {
    pub(super) fn new(
        mi_cols: usize,
        (sub_x, sub_y): (usize, usize),
        storage: &mut (Vec<ChromaMiCell>, Vec<u8>),
    ) -> Self {
        Self {
            fully_covered: true,
            cells: core::mem::take(&mut storage.0),
            cell_cols: mi_cols.div_ceil(1 << sub_x),
            window: Window::default(),
            sub_x,
            sub_y,
            candidates: core::mem::take(&mut storage.1),
        }
    }

    /// Moves one chroma plane's overlay window to mode-info `rows` after the
    /// luma grid's own move. `rows` starts on a chroma cell row and ends on
    /// one or at the frame bottom.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill(
        &mut self,
        base: &MiGridStorage,
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
        slide_cells(&mut self.candidates, dropped, mi_cols, 0, plane_id)?;
        let built = (new.start - row_base) * mi_cols;
        let luma = (new.start - base.window.row_base) * mi_cols;
        self.candidates
            .extend_from_slice(base.candidates.get(luma..).ok_or(DeblockError::Workspace)?);
        if dropped.is_none() {
            self.fully_covered = true;
        }
        for &index in order.reaching(&new) {
            let record = records
                .blocks
                .get(index as usize)
                .ok_or(DeblockError::Workspace)?;
            if record.planes & mask == 0 {
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
            for (start, end) in spans {
                if let Some(cells) = self.cells.get_mut(start..end) {
                    for cell in cells {
                        if block.chroma_transform_only {
                            cell.chroma_transform = later(cell.chroma_transform, index);
                        } else {
                            cell.overlay = later(cell.overlay, index);
                        }
                    }
                }
            }
            if !block.chroma_transform_only {
                for (start, end) in block_row_spans(block, mi_rows, mi_cols, &new, row_base) {
                    if let Some(candidates) = self.candidates.get_mut(start..end) {
                        for candidate in candidates {
                            *candidate |= COVERED_CANDIDATE;
                        }
                    }
                }
            }
            mark_block_candidates(
                &mut self.candidates,
                block,
                mi_rows,
                mi_cols,
                &new,
                row_base,
            );
        }
        self.fully_covered &=
            base.fully_covered || all_covered(self.candidates.get(built..).unwrap_or_default());
        Ok(())
    }
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
    if index == NO_BLOCK_INDEX {
        return Err(DeblockError::Workspace);
    }
    Ok(index)
}

/// Marks one block's edges in `rows`, offset from row `first`.
fn mark_block_candidates(
    candidates: &mut [u8],
    block: &DeblockBlock,
    mi_rows: usize,
    mi_cols: usize,
    rows: &Range<usize>,
    first: usize,
) {
    let (r, c) = (block.r as usize, block.c as usize);
    let row_end = r.saturating_add(block.n4h as usize).min(mi_rows);
    let col_end = c.saturating_add(block.n4w as usize).min(mi_cols);
    let row_start = r.min(row_end);
    let col_start = c.min(col_end);
    let mut mark = |row: usize, cols: Range<usize>, flag: u8| {
        if rows.contains(&row) {
            let base = (row - first) * mi_cols;
            if let Some(flags) = candidates.get_mut(base + cols.start..base + cols.end) {
                for candidate in flags {
                    *candidate |= flag;
                }
            }
        }
    };
    let window_rows = row_start.max(rows.start)..row_end.min(rows.end);
    for row in window_rows.clone() {
        for col in [col_start, col_end] {
            if col < mi_cols {
                mark(row, col..col + 1, VERTICAL_TX_CANDIDATE);
            }
        }
    }
    for row in [row_start, row_end] {
        mark(row, col_start..col_end, HORIZONTAL_TX_CANDIDATE);
    }
    if block.sub_pu_size.is_some() {
        for row in window_rows {
            mark(row, col_start..col_end, SUB_PU_CANDIDATE);
        }
    }
}
