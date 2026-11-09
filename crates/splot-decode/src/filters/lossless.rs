// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lossless block masks shared by final in-loop filters.

use splot_recon::PlaneId;

use crate::filters::deblock::DeblockBlock;

const MI_SIZE: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LosslessBlockGrid {
    mi_rows: usize,
    mi_cols: usize,
    luma: Vec<bool>,
    /// Shared by U and V, whose records are one list.
    chroma: Vec<bool>,
}

impl LosslessBlockGrid {
    pub(crate) fn from_deblock_blocks(
        mi_rows: usize,
        mi_cols: usize,
        luma_blocks: &[DeblockBlock],
        chroma_blocks: &[DeblockBlock],
    ) -> Result<Self, LosslessGridError> {
        Ok(Self {
            mi_rows,
            mi_cols,
            luma: lossless_cells(mi_rows, mi_cols, luma_blocks.iter())?,
            chroma: lossless_cells(mi_rows, mi_cols, chroma_blocks.iter())?,
        })
    }

    pub(crate) fn cdef_luma_lossless(&self, mi_row: usize, mi_col: usize) -> bool {
        any_cells(&self.luma, self.mi_rows, self.mi_cols, mi_row, mi_col, 2, 2)
    }

    pub(crate) fn cdef_chroma_lossless(
        &self,
        plane_id: PlaneId,
        mi_row: usize,
        mi_col: usize,
    ) -> bool {
        any_cells(
            self.cells(plane_id),
            self.mi_rows,
            self.mi_cols,
            mi_row,
            mi_col,
            2,
            2,
        )
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
        cell(
            self.cells(plane_id),
            self.mi_rows,
            self.mi_cols,
            mi_row,
            mi_col,
        )
    }

    fn cells(&self, plane_id: PlaneId) -> &[bool] {
        if plane_id == PlaneId::Y {
            &self.luma
        } else {
            &self.chroma
        }
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
        .map_err(|_| LosslessGridError::Allocation(splot_recon::PlaneId::Y))?;
    cells.resize(count, false);

    for block in blocks {
        if !block.lossless {
            continue;
        }
        let (r, c) = (block.r as usize, block.c as usize);
        let row_end = r
            .checked_add(block.n4h as usize)
            .ok_or(LosslessGridError::Geometry)?
            .min(mi_rows);
        let col_end = c
            .checked_add(block.n4w as usize)
            .ok_or(LosslessGridError::Geometry)?
            .min(mi_cols);
        for row in r.min(mi_rows)..row_end {
            for col in c.min(mi_cols)..col_end {
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
    Allocation(splot_recon::PlaneId),
}

#[cfg(test)]
#[path = "lossless_tests.rs"]
mod tests;
