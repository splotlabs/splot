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
    pub(super) overlay: u32,
    pub(super) chroma_transform: u32,
}

impl Default for MiCell {
    fn default() -> Self {
        Self {
            base: NO_BLOCK_INDEX,
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

pub(super) struct MiGrid<'a> {
    pub(super) storage: &'a MiGridStorage,
    pub(super) base_blocks: &'a [DeblockBlock],
    pub(super) overlay_blocks: &'a ChromaDeblockRecords,
}

impl MiGrid<'_> {
    #[allow(clippy::inline_always, reason = "measured luma deblock hot path")]
    #[inline(always)]
    pub(super) fn get_luma_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let cell = self.storage.cells.get(row * self.storage.mi_cols + col)?;
        Some(EdgeBlock {
            block: self.base_blocks.get(cell.base as usize)?,
            chroma_transform: None,
        })
    }

    pub(super) fn get_edge(&self, row: usize, col: usize) -> Option<EdgeBlock<'_>> {
        let cell = self.storage.cells.get(row * self.storage.mi_cols + col)?;
        let block = if cell.overlay != NO_BLOCK_INDEX {
            self.overlay_blocks.get(cell.overlay as usize)?
        } else {
            self.base_blocks.get(cell.base as usize)?
        };
        let chroma_transform = if cell.chroma_transform != NO_BLOCK_INDEX {
            Some(self.overlay_blocks.get(cell.chroma_transform as usize)?)
        } else {
            None
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
        let index = row * self.storage.mi_cols + col;
        let Some(&current) = self.storage.candidates.get(index) else {
            return true;
        };
        if !self.storage.fully_covered && current & COVERED_CANDIDATE == 0 {
            return true;
        }
        if current & candidate != 0 || allow_sub_pu && current & SUB_PU_CANDIDATE != 0 {
            return true;
        }
        if pass == 0 && plane_sub_x != 0 && col != 0 {
            return self.storage.candidates[index - 1] & VERTICAL_TX_CANDIDATE != 0;
        }
        if pass == 1 && plane_sub_y != 0 && row != 0 {
            return self.storage.candidates[index - self.storage.mi_cols] & HORIZONTAL_TX_CANDIDATE
                != 0;
        }
        false
    }
}

const MAX_RETAINED_DEBLOCK_GRIDS: usize = 4;
const MAX_RETAINED_DEBLOCK_CELLS: usize = 1 << 22;
static RETAINED_DEBLOCK_GRIDS: std::sync::Mutex<Vec<(Vec<MiCell>, Vec<u8>)>> =
    std::sync::Mutex::new(Vec::new());

fn take_deblock_grid_scratch() -> (Vec<MiCell>, Vec<u8>) {
    RETAINED_DEBLOCK_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop()
        .unwrap_or_default()
}

pub(super) fn recycle_deblock_grid_scratch(mut cells: Vec<MiCell>, mut candidates: Vec<u8>) {
    if cells.capacity() == 0 || cells.capacity() > MAX_RETAINED_DEBLOCK_CELLS {
        return;
    }
    let mut pool = RETAINED_DEBLOCK_GRIDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if pool.len() < MAX_RETAINED_DEBLOCK_GRIDS {
        cells.clear();
        candidates.clear();
        pool.push((cells, candidates));
    }
}

impl MiGridStorage {
    pub(super) fn into_scratch(self) -> (Vec<MiCell>, Vec<u8>) {
        (self.cells, self.candidates)
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
    let (mut cells, mut candidates) = take_deblock_grid_scratch();
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
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    let index = rr * mi_cols + cc;
                    cells[index].base = block_index;
                    candidates[index] |= COVERED_CANDIDATE;
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
) -> Result<MiGridStorage, DeblockError> {
    let (mut cells, mut candidates) = take_deblock_grid_scratch();
    cells.clone_from(&base.cells);
    candidates.clone_from(&base.candidates);
    let mut grid = MiGridStorage {
        mi_cols: base.mi_cols,
        fully_covered: base.fully_covered,
        cells,
        candidates,
    };
    for (block_index, block) in blocks
        .iter_plane(plane)
        .filter(|(_, block)| !block.chroma_transform_only)
    {
        let block_index = mi_block_index(block_index)?;
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    let index = rr * mi_cols + cc;
                    grid.cells[index].overlay = block_index;
                    grid.candidates[index] |= COVERED_CANDIDATE;
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
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    grid.cells[rr * mi_cols + cc].chroma_transform = block_index;
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
