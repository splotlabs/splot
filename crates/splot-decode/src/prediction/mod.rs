// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Decode-side prediction selection and prediction helper orchestration.

pub(crate) mod chroma;
pub(crate) mod inter;
pub(crate) mod intra_edge;

/// Failure to construct a tile-local grid from validated geometry.
#[derive(Debug)]
pub(crate) enum TileGridConstructionError {
    EmptyDimensions,
    ReversedDimensions,
    AreaOverflow,
    Allocation,
}

pub(crate) fn tile_grid_dimensions(
    mi_rows: &core::ops::Range<usize>,
    mi_cols: &core::ops::Range<usize>,
) -> Result<(usize, usize, usize), TileGridConstructionError> {
    let rows = mi_rows
        .end
        .checked_sub(mi_rows.start)
        .ok_or(TileGridConstructionError::ReversedDimensions)?;
    let cols = mi_cols
        .end
        .checked_sub(mi_cols.start)
        .ok_or(TileGridConstructionError::ReversedDimensions)?;
    if rows == 0 || cols == 0 {
        return Err(TileGridConstructionError::EmptyDimensions);
    }
    let cells = rows
        .checked_mul(cols)
        .ok_or(TileGridConstructionError::AreaOverflow)?;
    Ok((rows, cols, cells))
}
