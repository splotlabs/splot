// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

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

pub(super) struct MiGridStorage {
    pub(super) mi_cols: usize,
    pub(super) fully_covered: bool,
    pub(super) cells: Vec<MiCell>,
    pub(super) candidates: Vec<u8>,
}

pub(super) struct ChromaMiGridStorage {
    pub(super) fully_covered: bool,
    pub(super) cells: Vec<ChromaMiCell>,
    pub(super) candidates: Vec<u8>,
}

pub(super) struct MiGrid<'a> {
    pub(super) base: &'a MiGridStorage,
    pub(super) chroma: Option<&'a ChromaMiGridStorage>,
    pub(super) candidates: &'a [u8],
    pub(super) fully_covered: bool,
    pub(super) base_blocks: &'a [DeblockBlock],
    pub(super) overlay_blocks: &'a ChromaDeblockRecords,
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
        }
    }

    #[allow(clippy::inline_always, reason = "measured luma deblock hot path")]
    #[inline(always)]
    pub(super) fn get_luma_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let cell = self.base.cells.get(row * self.base.mi_cols + col)?;
        Some(EdgeBlock {
            block: self.base_blocks.get(cell.base as usize)?,
            chroma_transform: None,
        })
    }

    pub(super) fn get_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let index = row * self.base.mi_cols + col;
        let base = self.base.cells.get(index)?;
        let chroma = self.chroma.and_then(|grid| grid.cells.get(index));
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

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
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
        let index = row * self.base.mi_cols + col;
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
            return self.candidates[index - 1] & VERTICAL_TX_CANDIDATE != 0;
        }
        if pass == 1 && plane_sub_y != 0 && row != 0 {
            return self.candidates[index - self.base.mi_cols] & HORIZONTAL_TX_CANDIDATE != 0;
        }
        false
    }
}

/// Fewest deblock grids retained between frames, and the floor the pool-width
/// bound never drops below.
const MIN_RETAINED_DEBLOCK_GRIDS: usize = 4;

/// Retains one grid per worker so a wide pool does not reallocate the grids its
/// extra workers need, with [`MIN_RETAINED_DEBLOCK_GRIDS`] as the floor.
/// Scales per worker only on a pool thread; off-pool callers get the floor.
fn max_retained_deblock_grids() -> usize {
    splot_parallel::current_pool_width().max(MIN_RETAINED_DEBLOCK_GRIDS)
}
const MAX_RETAINED_DEBLOCK_CELLS: usize = 1 << 22;
static RETAINED_DEBLOCK_GRIDS: std::sync::Mutex<Vec<(Vec<MiCell>, Vec<u8>)>> =
    std::sync::Mutex::new(Vec::new());
static RETAINED_CHROMA_DEBLOCK_GRIDS: std::sync::Mutex<Vec<(Vec<ChromaMiCell>, Vec<u8>)>> =
    std::sync::Mutex::new(Vec::new());

fn take_deblock_grid_scratch<C>(
    pool: &std::sync::Mutex<Vec<(Vec<C>, Vec<u8>)>>,
) -> (Vec<C>, Vec<u8>) {
    pool.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .unwrap_or_default()
}

fn recycle_deblock_grid_scratch<C>(
    pool: &std::sync::Mutex<Vec<(Vec<C>, Vec<u8>)>>,
    mut cells: Vec<C>,
    mut candidates: Vec<u8>,
) {
    if cells.capacity() == 0 || cells.capacity() > MAX_RETAINED_DEBLOCK_CELLS {
        return;
    }
    let mut retained = pool
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if retained.len() < max_retained_deblock_grids() {
        cells.clear();
        candidates.clear();
        retained.push((cells, candidates));
    }
}

impl MiGridStorage {
    pub(super) fn recycle(self) {
        recycle_deblock_grid_scratch(&RETAINED_DEBLOCK_GRIDS, self.cells, self.candidates);
    }
}

impl ChromaMiGridStorage {
    pub(super) fn recycle(self) {
        recycle_deblock_grid_scratch(&RETAINED_CHROMA_DEBLOCK_GRIDS, self.cells, self.candidates);
    }
}

pub(super) fn build_mi_grid(
    blocks: &[DeblockBlock],
    mi_rows: usize,
    mi_cols: usize,
) -> Result<MiGridStorage, DeblockError> {
    let count = mi_rows
        .checked_mul(mi_cols)
        .ok_or(DeblockError::Workspace)?;
    let (mut cells, mut candidates) = take_deblock_grid_scratch(&RETAINED_DEBLOCK_GRIDS);
    cells.clear();
    cells
        .try_reserve_exact(count)
        .map_err(|_| DeblockError::Allocation {
            plane: splot_recon::PlaneId::Y,
            context: "deblock MI grid",
        })?;
    cells.resize(count, MiCell::default());
    candidates.clear();
    candidates
        .try_reserve_exact(count)
        .map_err(|_| DeblockError::Allocation {
            plane: splot_recon::PlaneId::Y,
            context: "deblock MI grid",
        })?;
    candidates.resize(count, 0);

    for (block_index, block) in blocks.iter().enumerate() {
        let block_index = mi_block_index(block_index)?;
        for (start, end) in block_row_spans(block, mi_rows, mi_cols) {
            if let Some(cells) = cells.get_mut(start..end) {
                for cell in cells {
                    cell.base = block_index;
                }
            }
            if let Some(candidates) = candidates.get_mut(start..end) {
                for candidate in candidates {
                    *candidate |= COVERED_CANDIDATE;
                }
            }
        }
        mark_block_candidates(&mut candidates, block, mi_rows, mi_cols);
    }
    let fully_covered = candidates
        .iter()
        .all(|candidate| candidate & COVERED_CANDIDATE != 0);
    Ok(MiGridStorage {
        mi_cols,
        fully_covered,
        cells,
        candidates,
    })
}

pub(super) fn overlay_mi_grid(
    base: &MiGridStorage,
    blocks: &ChromaDeblockRecords,
    plane: usize,
    mi_rows: usize,
    mi_cols: usize,
) -> Result<ChromaMiGridStorage, DeblockError> {
    let plane_id = match plane {
        0 => splot_recon::PlaneId::U,
        1 => splot_recon::PlaneId::V,
        _ => return Err(DeblockError::Workspace),
    };
    let count = mi_rows
        .checked_mul(mi_cols)
        .ok_or(DeblockError::Workspace)?;
    let (mut cells, mut candidates) = take_deblock_grid_scratch(&RETAINED_CHROMA_DEBLOCK_GRIDS);
    cells.clear();
    cells
        .try_reserve_exact(count)
        .map_err(|_| DeblockError::Allocation {
            plane: plane_id,
            context: "chroma deblock MI grid",
        })?;
    cells.resize(count, ChromaMiCell::default());
    candidates.clone_from(&base.candidates);
    let mut grid = ChromaMiGridStorage {
        fully_covered: base.fully_covered,
        cells,
        candidates,
    };
    for (block_index, block) in blocks
        .iter_plane(plane)
        .filter(|(_, block)| !block.chroma_transform_only)
    {
        let block_index = mi_block_index(block_index)?;
        for (start, end) in block_row_spans(block, mi_rows, mi_cols) {
            if let Some(cells) = grid.cells.get_mut(start..end) {
                for cell in cells {
                    cell.overlay = block_index;
                }
            }
            if let Some(candidates) = grid.candidates.get_mut(start..end) {
                for candidate in candidates {
                    *candidate |= COVERED_CANDIDATE;
                }
            }
        }
        mark_block_candidates(&mut grid.candidates, block, mi_rows, mi_cols);
    }
    for (block_index, block) in blocks
        .iter_plane(plane)
        .filter(|(_, block)| block.chroma_transform_only)
    {
        let block_index = mi_block_index(block_index)?;
        for (start, end) in block_row_spans(block, mi_rows, mi_cols) {
            if let Some(cells) = grid.cells.get_mut(start..end) {
                for cell in cells {
                    cell.chroma_transform = block_index;
                }
            }
        }
        mark_block_candidates(&mut grid.candidates, block, mi_rows, mi_cols);
    }
    if !grid.fully_covered {
        grid.fully_covered = grid
            .candidates
            .iter()
            .all(|candidate| candidate & COVERED_CANDIDATE != 0);
    }
    Ok(grid)
}

fn block_row_spans(
    block: &DeblockBlock,
    mi_rows: usize,
    mi_cols: usize,
) -> impl Iterator<Item = (usize, usize)> {
    let row_end = block.r.saturating_add(block.n4h).min(mi_rows);
    let col_end = block.c.saturating_add(block.n4w).min(mi_cols);
    let col_start = block.c.min(col_end);
    (block.r..row_end).map(move |row| {
        let base = row * mi_cols;
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

fn mark_block_candidates(
    candidates: &mut [u8],
    block: &DeblockBlock,
    mi_rows: usize,
    mi_cols: usize,
) {
    let row_end = block.r.saturating_add(block.n4h).min(mi_rows);
    let col_end = block.c.saturating_add(block.n4w).min(mi_cols);
    let row_start = block.r.min(row_end);
    let col_start = block.c.min(col_end);

    for row in row_start..row_end {
        mark_vertical_candidate(candidates, row, col_start, mi_cols);
        mark_vertical_candidate(candidates, row, col_end, mi_cols);
    }
    for col in col_start..col_end {
        mark_horizontal_candidate(candidates, row_start, col, mi_rows, mi_cols);
        mark_horizontal_candidate(candidates, row_end, col, mi_rows, mi_cols);
    }
    if block.sub_pu_size.is_some() {
        for row in row_start..row_end {
            let start = row * mi_cols + col_start;
            let end = row * mi_cols + col_end;
            for candidate in &mut candidates[start..end] {
                *candidate |= SUB_PU_CANDIDATE;
            }
        }
    }
}

fn mark_vertical_candidate(candidates: &mut [u8], row: usize, col: usize, mi_cols: usize) {
    if col < mi_cols {
        candidates[row * mi_cols + col] |= VERTICAL_TX_CANDIDATE;
    }
}

fn mark_horizontal_candidate(
    candidates: &mut [u8],
    row: usize,
    col: usize,
    mi_rows: usize,
    mi_cols: usize,
) {
    if row < mi_rows {
        candidates[row * mi_cols + col] |= HORIZONTAL_TX_CANDIDATE;
    }
}
