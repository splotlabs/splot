// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 tile-local coefficient context state.
//!
//! Feature tracking: `DECODE-TILE-COEFF-STATE-BUFFERS`,
//! `DECODE-COEFF-ALL-ZERO-BLOCK-STATE`.

use core::ops::Range;
use std::collections::TryReserveError;

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_recon::PlaneId;

use crate::tile::block_context::ChromaSampling;

const PLANE_COUNT: usize = 3;
const MAX_ADJUSTED_TX_EXTENT: usize = 32;
/// Zero padding rows/columns kept below and right of the level and sign
/// grids so AV2 § 8.3.2 neighbor reads (offsets up to +4 per
/// `Sig_Ref_Diff_Offset`) never need per-sample bounds handling.
pub(crate) const LEVEL_GRID_PAD: usize = 4;
const MAX_PADDED_COEFF_LEN: usize =
    (MAX_ADJUSTED_TX_EXTENT + LEVEL_GRID_PAD) * (MAX_ADJUSTED_TX_EXTENT + LEVEL_GRID_PAD);

static ZERO_QUANT_SIGN: [i8; MAX_PADDED_COEFF_LEN] = [0; MAX_PADDED_COEFF_LEN];
const PLANES: [PlaneId; PLANE_COUNT] = [PlaneId::Y, PlaneId::U, PlaneId::V];

fn zero_buffer<T: Default + Copy>(
    mut buffer: Vec<T>,
    len: usize,
) -> Result<Vec<T>, TryReserveError> {
    buffer.clear();
    buffer.try_reserve_exact(len)?;
    buffer.resize(len, T::default());
    Ok(buffer)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformCoeffBlockState {
    height: usize,
    stride: usize,
    level: Vec<u8>,
    quant_sign: Vec<i8>,
    quant: Vec<i32>,
}

impl TransformCoeffBlockState {
    pub(crate) fn allocation(width: usize, height: usize) -> Result<usize, TileCoeffStateError> {
        validate_adjusted_extent("width", width)?;
        validate_adjusted_extent("height", height)?;
        checked_mul_usize("width * height", width, height)
    }

    pub(crate) fn new(width: usize, height: usize) -> Result<Self, TileCoeffStateError> {
        let allocation = Self::allocation(width, height)?;
        let stride = width + LEVEL_GRID_PAD;
        let level_len = stride * (height + LEVEL_GRID_PAD);
        let level = zero_buffer(Vec::new(), level_len)?;
        let quant = zero_buffer(Vec::new(), allocation)?;
        Ok(Self {
            height,
            stride,
            level,
            quant_sign: Vec::new(),
            quant,
        })
    }

    pub(crate) fn ensure_quant_sign(&mut self) -> Result<(), TileCoeffStateError> {
        if self.quant_sign.is_empty() {
            self.quant_sign = zero_buffer(Vec::new(), self.level.len())?;
        }
        Ok(())
    }

    #[must_use]
    pub(crate) const fn width(&self) -> usize {
        self.stride - LEVEL_GRID_PAD
    }

    #[must_use]
    pub(crate) const fn height(&self) -> usize {
        self.height
    }

    #[must_use]
    pub(crate) fn level(&self) -> &[u8] {
        &self.level
    }

    /// Row stride of the padded level and sign grids.
    #[must_use]
    pub(crate) const fn level_stride(&self) -> usize {
        self.stride
    }

    /// Number of real (unpadded) coefficient positions in the block.
    #[must_use]
    pub(crate) const fn coeff_count(&self) -> usize {
        self.quant.len()
    }

    #[must_use]
    pub(crate) fn quant_sign(&self) -> &[i8] {
        if self.quant_sign.is_empty() {
            &ZERO_QUANT_SIGN[..self.level.len()]
        } else {
            &self.quant_sign
        }
    }

    #[must_use]
    pub(crate) fn into_quant(mut self) -> Vec<i32> {
        core::mem::take(&mut self.quant)
    }

    pub(crate) fn set_level(
        &mut self,
        row: usize,
        col: usize,
        value: u32,
    ) -> Result<(), TileCoeffStateError> {
        let idx = self.index(row, col)?;
        debug_assert!(u8::try_from(value).is_ok());
        self.level[idx] = value as u8;
        Ok(())
    }

    pub(crate) fn set_quant_sign(
        &mut self,
        row: usize,
        col: usize,
        value: i8,
    ) -> Result<(), TileCoeffStateError> {
        let idx = self.index(row, col)?;
        self.ensure_quant_sign()?;
        self.quant_sign[idx] = value;
        Ok(())
    }

    pub(crate) fn set_quant(&mut self, pos: usize, value: i32) -> Result<(), TileCoeffStateError> {
        let idx = self.quant_index(pos)?;
        self.quant[idx] = value;
        Ok(())
    }

    #[inline]
    pub(crate) fn level_at(&self, row: usize, col: usize) -> Result<u32, TileCoeffStateError> {
        Ok(u32::from(self.level[self.index(row, col)?]))
    }

    pub(crate) fn quant_sign_at(&self, row: usize, col: usize) -> Result<i8, TileCoeffStateError> {
        Ok(self.quant_sign()[self.index(row, col)?])
    }

    pub(crate) fn quant_at(&self, pos: usize) -> Result<i32, TileCoeffStateError> {
        Ok(self.quant[self.quant_index(pos)?])
    }

    fn index(&self, row: usize, col: usize) -> Result<usize, TileCoeffStateError> {
        let width = self.width();
        if row >= self.height || col >= width {
            return Err(TileCoeffStateError::TransformCoordinateOutOfBounds {
                row,
                col,
                height: self.height,
                width,
            });
        }
        row.checked_mul(self.stride)
            .and_then(|base| base.checked_add(col))
            .ok_or(TileCoeffStateError::ArithmeticOverflow {
                operation: "row * stride + col",
                left: row,
                right: self.stride,
            })
    }

    fn quant_index(&self, pos: usize) -> Result<usize, TileCoeffStateError> {
        if pos >= self.quant.len() {
            return Err(TileCoeffStateError::QuantPositionOutOfBounds {
                pos,
                len: self.quant.len(),
            });
        }
        Ok(pos)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffContextState {
    plane_row_origins: [usize; PLANE_COUNT],
    plane_col_origins: [usize; PLANE_COUNT],
    plane_rows: [usize; PLANE_COUNT],
    plane_cols: [usize; PLANE_COUNT],
    above_level: [Vec<u8>; PLANE_COUNT],
    left_level: [Vec<u8>; PLANE_COUNT],
    above_dc: [Vec<u8>; PLANE_COUNT],
    left_dc: [Vec<u8>; PLANE_COUNT],
}

impl TileCoeffContextState {
    pub(crate) fn allocation(mi_rows: usize, mi_cols: usize) -> Result<(), TileCoeffStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileCoeffStateError::EmptyTileDimensions { mi_rows, mi_cols });
        }
        let line_entries = checked_add_usize("mi_rows + mi_cols", mi_rows, mi_cols)?;
        let plane_entries = checked_mul_usize("line_entries * planes", line_entries, PLANE_COUNT)?;
        let level_entries = plane_entries;
        let dc_entries = plane_entries;
        checked_add_usize("level_entries + dc_entries", level_entries, dc_entries)?;
        Ok(())
    }

    pub(crate) fn new_for_tile_chroma(
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        chroma: ChromaFormatIdc,
    ) -> Result<Self, TileCoeffStateError> {
        Self::new_for_tile_with_chroma_sampling(
            mi_rows,
            mi_cols,
            ChromaSampling::from_chroma_format_idc(chroma),
        )
    }

    fn new_for_tile_with_chroma_sampling(
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        chroma: ChromaSampling,
    ) -> Result<Self, TileCoeffStateError> {
        let rows = mi_rows.end.saturating_sub(mi_rows.start);
        let cols = mi_cols.end.saturating_sub(mi_cols.start);
        Self::allocation(rows, cols)?;
        let mut plane_row_origins = [0; PLANE_COUNT];
        let mut plane_col_origins = [0; PLANE_COUNT];
        let mut plane_rows = [0; PLANE_COUNT];
        let mut plane_cols = [0; PLANE_COUNT];
        for (index, plane) in PLANES.iter().copied().enumerate() {
            let (sub_x, sub_y) = chroma.subsampling(plane);
            plane_row_origins[index] = mi_rows.start >> sub_y;
            plane_col_origins[index] = mi_cols.start >> sub_x;
            plane_rows[index] = (subsampled_context_len(mi_rows.end, sub_y)?)
                .saturating_sub(plane_row_origins[index]);
            plane_cols[index] = (subsampled_context_len(mi_cols.end, sub_x)?)
                .saturating_sub(plane_col_origins[index]);
        }
        Ok(Self {
            plane_row_origins,
            plane_col_origins,
            plane_rows,
            plane_cols,
            above_level: zeroed_plane_lines(plane_cols)?,
            left_level: zeroed_plane_lines(plane_rows)?,
            above_dc: zeroed_plane_lines(plane_cols)?,
            left_dc: zeroed_plane_lines(plane_rows)?,
        })
    }

    pub(crate) fn above_level(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.above_level[validate_plane(plane)?])
    }

    pub(crate) fn left_level(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.left_level[validate_plane(plane)?])
    }

    pub(crate) fn above_dc(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.above_dc[validate_plane(plane)?])
    }

    pub(crate) fn left_dc(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.left_dc[validate_plane(plane)?])
    }

    pub(crate) fn local_x4(&self, plane: usize, x4: usize) -> Result<usize, TileCoeffStateError> {
        let plane = validate_plane(plane)?;
        local_context_coordinate(
            "above",
            x4,
            self.plane_col_origins[plane],
            self.plane_cols[plane],
        )
    }

    pub(crate) fn local_y4(&self, plane: usize, y4: usize) -> Result<usize, TileCoeffStateError> {
        let plane = validate_plane(plane)?;
        local_context_coordinate(
            "left",
            y4,
            self.plane_row_origins[plane],
            self.plane_rows[plane],
        )
    }

    pub(crate) fn update_after_coeffs(
        &mut self,
        input: CoeffContextUpdate,
    ) -> Result<(), TileCoeffStateError> {
        let plane = validate_plane(input.plane)?;
        validate_dc_category(input.dc_category)?;
        if input.w4 == 0 {
            return Err(TileCoeffStateError::EmptyContextRange { axis: "columns" });
        }
        if input.h4 == 0 {
            return Err(TileCoeffStateError::EmptyContextRange { axis: "rows" });
        }
        let above_start = local_context_coordinate(
            "above",
            input.x4,
            self.plane_col_origins[plane],
            self.plane_cols[plane],
        )?;
        let left_start = local_context_coordinate(
            "left",
            input.y4,
            self.plane_row_origins[plane],
            self.plane_rows[plane],
        )?;
        let above = edge_clamped_range("above", above_start, input.w4, self.plane_cols[plane])?;
        let left = edge_clamped_range("left", left_start, input.h4, self.plane_rows[plane])?;

        fill_context_line(
            &mut self.above_level[plane],
            &mut self.above_dc[plane],
            above,
            input.cul_level,
            input.dc_category,
        );
        fill_context_line(
            &mut self.left_level[plane],
            &mut self.left_dc[plane],
            left,
            input.cul_level,
            input.dc_category,
        );
        Ok(())
    }

    pub(crate) fn reset_block_context_plane(
        &mut self,
        input: CoeffContextReset,
    ) -> Result<(), TileCoeffStateError> {
        let plane = validate_plane(input.plane)?;
        validate_subsampling("x", input.sub_x)?;
        validate_subsampling("y", input.sub_y)?;
        let above_start = local_context_coordinate(
            "above reset",
            shifted(input.c, input.sub_x)?,
            self.plane_col_origins[plane],
            self.plane_cols[plane],
        )?;
        let left_start = local_context_coordinate(
            "left reset",
            shifted(input.r, input.sub_y)?,
            self.plane_row_origins[plane],
            self.plane_rows[plane],
        )?;
        let above_unshifted_end = checked_coordinate_add("column", input.c, input.w4)?;
        let left_unshifted_end = checked_coordinate_add("row", input.r, input.h4)?;
        let above_end = shifted(above_unshifted_end, input.sub_x)?
            .saturating_sub(self.plane_col_origins[plane]);
        let left_end =
            shifted(left_unshifted_end, input.sub_y)?.saturating_sub(self.plane_row_origins[plane]);
        let above = edge_clamped_existing_range(
            "above reset",
            above_start,
            above_end,
            self.plane_cols[plane],
        )?;
        let left = edge_clamped_existing_range(
            "left reset",
            left_start,
            left_end,
            self.plane_rows[plane],
        )?;

        fill_context_line(
            &mut self.above_level[plane],
            &mut self.above_dc[plane],
            above,
            0,
            0,
        );
        fill_context_line(
            &mut self.left_level[plane],
            &mut self.left_dc[plane],
            left,
            0,
            0,
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffContextUpdate {
    pub(crate) plane: usize,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
    pub(crate) cul_level: u8,
    pub(crate) dc_category: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffContextReset {
    pub(crate) plane: usize,
    pub(crate) c: usize,
    pub(crate) r: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
    pub(crate) sub_x: u32,
    pub(crate) sub_y: u32,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TileCoeffStateError {
    #[error("coefficient context state dimensions must be nonzero, got {mi_rows}x{mi_cols}")]
    EmptyTileDimensions { mi_rows: usize, mi_cols: usize },
    #[error("invalid adjusted transform {axis} {value}; expected 1..={MAX_ADJUSTED_TX_EXTENT}")]
    InvalidAdjustedTransformExtent { axis: &'static str, value: usize },
    #[error("{operation} overflow: left {left}, right {right}")]
    ArithmeticOverflow {
        operation: &'static str,
        left: usize,
        right: usize,
    },
    #[error("coefficient state allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("invalid coefficient context plane {plane}")]
    InvalidPlane { plane: usize },
    #[error("invalid coefficient DC category {dc_category}")]
    InvalidDcCategory { dc_category: u8 },
    #[error("empty coefficient context {axis} range")]
    EmptyContextRange { axis: &'static str },
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        coordinate: &'static str,
        base: usize,
        offset: usize,
    },
    #[error("{context} range {start}..{end} exceeds coefficient context line length {len}")]
    ContextRangeOutOfBounds {
        context: &'static str,
        start: usize,
        end: usize,
        len: usize,
    },
    #[error(
        "coefficient transform coordinate ({row},{col}) exceeds adjusted block {height}x{width}"
    )]
    TransformCoordinateOutOfBounds {
        row: usize,
        col: usize,
        height: usize,
        width: usize,
    },
    #[error("coefficient Quant position {pos} exceeds adjusted block coefficient count {len}")]
    QuantPositionOutOfBounds { pos: usize, len: usize },
    #[error("invalid coefficient context subsampling {axis} shift {value}")]
    InvalidSubsampling { axis: &'static str, value: u32 },
}

fn validate_adjusted_extent(axis: &'static str, value: usize) -> Result<(), TileCoeffStateError> {
    if (1..=MAX_ADJUSTED_TX_EXTENT).contains(&value) {
        Ok(())
    } else {
        Err(TileCoeffStateError::InvalidAdjustedTransformExtent { axis, value })
    }
}

fn validate_plane(plane: usize) -> Result<usize, TileCoeffStateError> {
    if plane < PLANE_COUNT {
        Ok(plane)
    } else {
        Err(TileCoeffStateError::InvalidPlane { plane })
    }
}

fn validate_dc_category(dc_category: u8) -> Result<(), TileCoeffStateError> {
    if dc_category <= 2 {
        Ok(())
    } else {
        Err(TileCoeffStateError::InvalidDcCategory { dc_category })
    }
}

fn validate_subsampling(axis: &'static str, value: u32) -> Result<(), TileCoeffStateError> {
    if value <= 1 {
        Ok(())
    } else {
        Err(TileCoeffStateError::InvalidSubsampling { axis, value })
    }
}

fn subsampled_context_len(value: usize, shift: u32) -> Result<usize, TileCoeffStateError> {
    validate_subsampling("context", shift)?;
    let round = 1usize
        .checked_shl(shift)
        .and_then(|scale| scale.checked_sub(1))
        .ok_or(TileCoeffStateError::ArithmeticOverflow {
            operation: "1 << chroma shift - 1",
            left: 1,
            right: shift as usize,
        })?;
    Ok(checked_add_usize("context length + chroma round", value, round)? >> shift)
}

fn checked_add_usize(
    operation: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TileCoeffStateError> {
    left.checked_add(right)
        .ok_or(TileCoeffStateError::ArithmeticOverflow {
            operation,
            left,
            right,
        })
}

fn checked_mul_usize(
    operation: &'static str,
    left: usize,
    right: usize,
) -> Result<usize, TileCoeffStateError> {
    left.checked_mul(right)
        .ok_or(TileCoeffStateError::ArithmeticOverflow {
            operation,
            left,
            right,
        })
}

fn checked_coordinate_add(
    coordinate: &'static str,
    base: usize,
    offset: usize,
) -> Result<usize, TileCoeffStateError> {
    base.checked_add(offset)
        .ok_or(TileCoeffStateError::CoordinateOverflow {
            coordinate,
            base,
            offset,
        })
}

fn edge_clamped_range(
    context: &'static str,
    start: usize,
    len: usize,
    line_len: usize,
) -> Result<Range<usize>, TileCoeffStateError> {
    let end = checked_coordinate_add(context, start, len)?;
    edge_clamped_existing_range(context, start, end, line_len)
}

fn local_context_coordinate(
    context: &'static str,
    coordinate: usize,
    origin: usize,
    line_len: usize,
) -> Result<usize, TileCoeffStateError> {
    coordinate
        .checked_sub(origin)
        .ok_or(TileCoeffStateError::ContextRangeOutOfBounds {
            context,
            start: coordinate,
            end: coordinate,
            len: line_len,
        })
}

fn edge_clamped_existing_range(
    context: &'static str,
    start: usize,
    end: usize,
    line_len: usize,
) -> Result<Range<usize>, TileCoeffStateError> {
    // Frame-edge overhang has no backing context storage; reject only impossible origins.
    if start > end || start >= line_len {
        return Err(TileCoeffStateError::ContextRangeOutOfBounds {
            context,
            start,
            end,
            len: line_len,
        });
    }
    Ok(start..end.min(line_len))
}

fn shifted(value: usize, shift: u32) -> Result<usize, TileCoeffStateError> {
    value
        .checked_shr(shift)
        .ok_or(TileCoeffStateError::InvalidSubsampling {
            axis: "shift",
            value: shift,
        })
}

fn fill_context_line(
    level: &mut [u8],
    dc: &mut [u8],
    range: Range<usize>,
    level_value: u8,
    dc_value: u8,
) {
    for idx in range {
        level[idx] = level_value;
        dc[idx] = dc_value;
    }
}

fn zeroed_plane_lines<T: Default>(
    lengths: [usize; PLANE_COUNT],
) -> Result<[Vec<T>; PLANE_COUNT], TileCoeffStateError> {
    Ok([
        zeroed_vec(lengths[0])?,
        zeroed_vec(lengths[1])?,
        zeroed_vec(lengths[2])?,
    ])
}

fn zeroed_vec<T: Default>(len: usize) -> Result<Vec<T>, TileCoeffStateError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    values.resize_with(len, T::default);
    Ok(values)
}

#[cfg(test)]
#[path = "coeff_state_tests.rs"]
mod tests;
