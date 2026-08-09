// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lossless block masks shared by final in-loop filters.

use splot_recon::PlaneId;

use crate::filters::deblock::{ChromaDeblockRecords, DeblockBlock};

const MI_SIZE: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LosslessBlockGrid {
    mi_rows: usize,
    mi_cols: usize,
    luma: Vec<bool>,
    chroma: [Vec<bool>; 2],
}

impl LosslessBlockGrid {
    #[cfg(test)]
    pub(crate) fn from_deblock_blocks(
        mi_rows: usize,
        mi_cols: usize,
        luma_blocks: &[DeblockBlock],
        chroma_blocks: [&[DeblockBlock]; 2],
    ) -> Result<Self, LosslessGridError> {
        Ok(Self {
            mi_rows,
            mi_cols,
            luma: lossless_cells(mi_rows, mi_cols, luma_blocks.iter())?,
            chroma: [
                lossless_cells(mi_rows, mi_cols, chroma_blocks[0].iter())?,
                lossless_cells(mi_rows, mi_cols, chroma_blocks[1].iter())?,
            ],
        })
    }

    pub(crate) fn from_deblock_records(
        mi_rows: usize,
        mi_cols: usize,
        luma_blocks: &[DeblockBlock],
        chroma_blocks: &ChromaDeblockRecords,
    ) -> Result<Self, LosslessGridError> {
        Ok(Self {
            mi_rows,
            mi_cols,
            luma: lossless_cells(mi_rows, mi_cols, luma_blocks.iter())?,
            chroma: [
                lossless_cells(
                    mi_rows,
                    mi_cols,
                    chroma_blocks.iter_plane(0).map(|(_, block)| block),
                )?,
                lossless_cells(
                    mi_rows,
                    mi_cols,
                    chroma_blocks.iter_plane(1).map(|(_, block)| block),
                )?,
            ],
        })
    }

    pub(crate) fn cdef_luma_lossless(&self, mi_row: usize, mi_col: usize) -> bool {
        self.any_luma_mi(mi_row, mi_col, 2, 2)
    }

    pub(crate) fn cdef_chroma_lossless(
        &self,
        plane_id: PlaneId,
        mi_row: usize,
        mi_col: usize,
    ) -> bool {
        self.any_chroma_mi(plane_id, mi_row, mi_col, 2, 2)
    }

    pub(crate) fn plane_sample_lossless(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        sub_x: usize,
        sub_y: usize,
    ) -> bool {
        let luma_x = x.checked_shl(sub_x as u32).unwrap_or(usize::MAX);
        let luma_y = y.checked_shl(sub_y as u32).unwrap_or(usize::MAX);
        let mi_col = luma_x / MI_SIZE;
        let mi_row = luma_y / MI_SIZE;
        match plane_id {
            PlaneId::Y => self.luma_cell(mi_row, mi_col),
            PlaneId::U | PlaneId::V => self.chroma_cell(plane_id, mi_row, mi_col),
        }
    }

    fn any_luma_mi(&self, row: usize, col: usize, rows: usize, cols: usize) -> bool {
        any_cells(&self.luma, self.mi_rows, self.mi_cols, row, col, rows, cols)
    }

    fn any_chroma_mi(
        &self,
        plane_id: PlaneId,
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
    ) -> bool {
        let cells = match plane_id {
            PlaneId::Y => &self.luma,
            PlaneId::U => &self.chroma[0],
            PlaneId::V => &self.chroma[1],
        };
        any_cells(cells, self.mi_rows, self.mi_cols, row, col, rows, cols)
    }

    fn luma_cell(&self, row: usize, col: usize) -> bool {
        cell(&self.luma, self.mi_rows, self.mi_cols, row, col)
    }

    fn chroma_cell(&self, plane_id: PlaneId, row: usize, col: usize) -> bool {
        let cells = match plane_id {
            PlaneId::Y => &self.luma,
            PlaneId::U => &self.chroma[0],
            PlaneId::V => &self.chroma[1],
        };
        cell(cells, self.mi_rows, self.mi_cols, row, col)
    }
}

fn lossless_cells<'a>(
    mi_rows: usize,
    mi_cols: usize,
    blocks: impl IntoIterator<Item = &'a DeblockBlock>,
) -> Result<Vec<bool>, LosslessGridError> {
    let count = mi_rows
        .checked_mul(mi_cols)
        .ok_or(LosslessGridError::Geometry)?;
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(count)
        .map_err(|_| LosslessGridError::Allocation)?;
    cells.resize(count, false);

    for block in blocks {
        if !block.lossless {
            continue;
        }
        let row_end = block
            .r
            .checked_add(block.n4h)
            .ok_or(LosslessGridError::Geometry)?
            .min(mi_rows);
        let col_end = block
            .c
            .checked_add(block.n4w)
            .ok_or(LosslessGridError::Geometry)?
            .min(mi_cols);
        for row in block.r.min(mi_rows)..row_end {
            for col in block.c.min(mi_cols)..col_end {
                cells[row * mi_cols + col] = true;
            }
        }
    }
    Ok(cells)
}

fn any_cells(
    cells: &[bool],
    mi_rows: usize,
    mi_cols: usize,
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
) -> bool {
    let row_end = row.saturating_add(rows).min(mi_rows);
    let col_end = col.saturating_add(cols).min(mi_cols);
    for rr in row..row_end {
        for cc in col..col_end {
            if cell(cells, mi_rows, mi_cols, rr, cc) {
                return true;
            }
        }
    }
    false
}

fn cell(cells: &[bool], mi_rows: usize, mi_cols: usize, row: usize, col: usize) -> bool {
    if row >= mi_rows || col >= mi_cols {
        return false;
    }
    cells
        .get(row.saturating_mul(mi_cols).saturating_add(col))
        .copied()
        .unwrap_or(false)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LosslessGridError {
    #[error("lossless grid geometry is inconsistent")]
    Geometry,
    #[error("lossless grid storage could not be reserved")]
    Allocation,
}

#[cfg(test)]
#[path = "lossless_tests.rs"]
mod tests;
