// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 tile-local coefficient context state.
//!
//! Feature tracking: `DECODE-TILE-COEFF-STATE-BUFFERS`,
//! `DECODE-COEFF-ALL-ZERO-BLOCK-STATE`.

use core::ops::Range;
use std::collections::TryReserveError;
use std::sync::{Mutex, MutexGuard};

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_recon::PlaneId;

use crate::support::reusable_scratch::with_reusable_scratch;
use crate::tile::block_context::ChromaSampling;

const PLANE_COUNT: usize = 3;
const MAX_ADJUSTED_TX_EXTENT: usize = 32;
const MAX_RETAINED_COEFF_BUFFERS_PER_WORKER: usize = 64;
const MAX_RETAINED_SHARED_QUANT_BUFFERS: usize = 2_560;
const MAX_RETAINED_COEFF_BUFFER_CAPACITY: usize = MAX_ADJUSTED_TX_EXTENT * MAX_ADJUSTED_TX_EXTENT;
static ZERO_QUANT_SIGN: [i32; MAX_ADJUSTED_TX_EXTENT * MAX_ADJUSTED_TX_EXTENT] =
    [0; MAX_ADJUSTED_TX_EXTENT * MAX_ADJUSTED_TX_EXTENT];
const PLANES: [PlaneId; PLANE_COUNT] = [PlaneId::Y, PlaneId::U, PlaneId::V];

#[derive(Default)]
struct TransformCoeffBufferRecycler {
    levels: Vec<Vec<u32>>,
    signed: Vec<Vec<i32>>,
}

thread_local! {
    static TRANSFORM_COEFF_BUFFERS: core::cell::RefCell<TransformCoeffBufferRecycler> =
        const { core::cell::RefCell::new(TransformCoeffBufferRecycler {
            levels: Vec::new(),
            signed: Vec::new(),
        }) };
}

type SharedQuantBuffers = Mutex<Vec<Vec<i32>>>;

static TRANSFORM_COEFF_QUANT_BUFFERS: SharedQuantBuffers = Mutex::new(Vec::new());

fn lock_transform_coeff_quant_buffers(
    buffers: &SharedQuantBuffers,
) -> MutexGuard<'_, Vec<Vec<i32>>> {
    buffers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn take_zeroed_buffer<T: Default + Copy>(
    buffers: &mut Vec<Vec<T>>,
    len: usize,
) -> Result<Vec<T>, TryReserveError> {
    zero_buffer(buffers.pop().unwrap_or_default(), len)
}

fn zero_buffer<T: Default + Copy>(
    mut buffer: Vec<T>,
    len: usize,
) -> Result<Vec<T>, TryReserveError> {
    buffer.clear();
    buffer.try_reserve_exact(len)?;
    buffer.resize(len, T::default());
    Ok(buffer)
}

fn take_zeroed_quant_buffer_from(
    buffers: &SharedQuantBuffers,
    len: usize,
) -> Result<Vec<i32>, TryReserveError> {
    let mut buffers = lock_transform_coeff_quant_buffers(buffers);
    let index = buffers
        .iter()
        .enumerate()
        .filter(|(_, buffer)| buffer.capacity() >= len)
        .min_by_key(|(_, buffer)| buffer.capacity())
        .or_else(|| {
            buffers
                .iter()
                .enumerate()
                .max_by_key(|(_, buffer)| buffer.capacity())
        })
        .map(|(index, _)| index);
    let buffer = index
        .map(|index| buffers.swap_remove(index))
        .unwrap_or_default();
    drop(buffers);
    zero_buffer(buffer, len)
}

fn take_zeroed_quant_buffer(len: usize) -> Result<Vec<i32>, TryReserveError> {
    take_zeroed_quant_buffer_from(&TRANSFORM_COEFF_QUANT_BUFFERS, len)
}

fn recycle_buffer<T>(buffers: &mut Vec<Vec<T>>, mut buffer: Vec<T>) {
    if buffer.capacity() == 0
        || buffers.len() == MAX_RETAINED_COEFF_BUFFERS_PER_WORKER
        || buffers.try_reserve(1).is_err()
    {
        return;
    }
    buffer.clear();
    buffers.push(buffer);
}

fn recycle_coeff_quant_into(buffers: &SharedQuantBuffers, buffer: Vec<i32>) {
    if buffer.capacity() == 0 || buffer.capacity() > MAX_RETAINED_COEFF_BUFFER_CAPACITY {
        return;
    }
    let mut buffer = buffer;
    buffer.clear();
    let mut buffers = lock_transform_coeff_quant_buffers(buffers);
    if buffers.len() < MAX_RETAINED_SHARED_QUANT_BUFFERS && buffers.try_reserve(1).is_ok() {
        buffers.push(buffer);
    } else if let Some((index, smallest)) = buffers
        .iter()
        .enumerate()
        .min_by_key(|(_, retained)| retained.capacity())
        && buffer.capacity() > smallest.capacity()
    {
        buffers[index] = buffer;
    }
}

pub(crate) fn recycle_coeff_quant(buffer: Vec<i32>) {
    recycle_coeff_quant_into(&TRANSFORM_COEFF_QUANT_BUFFERS, buffer);
}

impl Drop for super::general_intra_residual::LumaCoeffBlock {
    fn drop(&mut self) {
        recycle_coeff_quant(core::mem::take(&mut self.quant));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformCoeffBlockState {
    width: usize,
    height: usize,
    level: Vec<u32>,
    quant_sign: Vec<i32>,
    quant: Vec<i32>,
}

impl TransformCoeffBlockState {
    pub(crate) fn allocation(
        width: usize,
        height: usize,
    ) -> Result<TransformCoeffBlockAllocation, TileCoeffStateError> {
        validate_adjusted_extent("width", width)?;
        validate_adjusted_extent("height", height)?;
        let coeff_count = checked_mul_usize("width * height", width, height)?;
        Ok(TransformCoeffBlockAllocation { coeff_count })
    }

    pub(crate) fn new(width: usize, height: usize) -> Result<Self, TileCoeffStateError> {
        let allocation = Self::allocation(width, height)?;
        let level = with_reusable_scratch(&TRANSFORM_COEFF_BUFFERS, |buffers| {
            take_zeroed_buffer(&mut buffers.levels, allocation.coeff_count)
        })?;
        let quant = match take_zeroed_quant_buffer(allocation.coeff_count) {
            Ok(quant) => quant,
            Err(error) => {
                with_reusable_scratch(&TRANSFORM_COEFF_BUFFERS, |buffers| {
                    recycle_buffer(&mut buffers.levels, level);
                });
                return Err(error.into());
            }
        };
        Ok(Self {
            width,
            height,
            level,
            quant_sign: Vec::new(),
            quant,
        })
    }

    pub(crate) fn ensure_quant_sign(&mut self) -> Result<(), TileCoeffStateError> {
        if self.quant_sign.is_empty() {
            self.quant_sign = with_reusable_scratch(&TRANSFORM_COEFF_BUFFERS, |buffers| {
                take_zeroed_buffer(&mut buffers.signed, self.level.len())
            })?;
        }
        Ok(())
    }

    #[must_use]
    pub(crate) const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub(crate) const fn height(&self) -> usize {
        self.height
    }

    #[must_use]
    pub(crate) fn level(&self) -> &[u32] {
        &self.level
    }

    #[must_use]
    pub(crate) fn quant_sign(&self) -> &[i32] {
        if self.quant_sign.is_empty() {
            &ZERO_QUANT_SIGN[..self.level.len()]
        } else {
            &self.quant_sign
        }
    }

    #[must_use]
    pub(crate) fn quant(&self) -> &[i32] {
        &self.quant
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
        self.level[idx] = value;
        Ok(())
    }

    pub(crate) fn set_quant_sign(
        &mut self,
        row: usize,
        col: usize,
        value: i32,
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

    pub(crate) fn level_at(&self, row: usize, col: usize) -> Result<u32, TileCoeffStateError> {
        Ok(self.level[self.index(row, col)?])
    }

    pub(crate) fn quant_sign_at(&self, row: usize, col: usize) -> Result<i32, TileCoeffStateError> {
        Ok(self.quant_sign()[self.index(row, col)?])
    }

    pub(crate) fn quant_at(&self, pos: usize) -> Result<i32, TileCoeffStateError> {
        Ok(self.quant[self.quant_index(pos)?])
    }

    fn index(&self, row: usize, col: usize) -> Result<usize, TileCoeffStateError> {
        if row >= self.height || col >= self.width {
            return Err(TileCoeffStateError::TransformCoordinateOutOfBounds {
                row,
                col,
                height: self.height,
                width: self.width,
            });
        }
        row.checked_mul(self.width)
            .and_then(|base| base.checked_add(col))
            .ok_or(TileCoeffStateError::ArithmeticOverflow {
                operation: "row * width + col",
                left: row,
                right: self.width,
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

impl Drop for TransformCoeffBlockState {
    fn drop(&mut self) {
        let level = core::mem::take(&mut self.level);
        let quant_sign = core::mem::take(&mut self.quant_sign);
        let quant = core::mem::take(&mut self.quant);
        with_reusable_scratch(&TRANSFORM_COEFF_BUFFERS, |buffers| {
            recycle_buffer(&mut buffers.levels, level);
            recycle_buffer(&mut buffers.signed, quant_sign);
        });
        recycle_coeff_quant(quant);
    }
}

#[cfg(test)]
fn clear_transform_coeff_buffers() {
    TRANSFORM_COEFF_BUFFERS
        .with(|slot| *slot.borrow_mut() = TransformCoeffBufferRecycler::default());
}

#[cfg(test)]
fn transform_coeff_buffer_counts() -> (usize, usize) {
    with_reusable_scratch(&TRANSFORM_COEFF_BUFFERS, |buffers| {
        (buffers.levels.len(), buffers.signed.len())
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransformCoeffBlockAllocation {
    coeff_count: usize,
}

impl TransformCoeffBlockAllocation {
    #[must_use]
    pub(crate) const fn coeff_count(self) -> usize {
        self.coeff_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffContextState {
    mi_rows: usize,
    mi_cols: usize,
    plane_row_origins: [usize; PLANE_COUNT],
    plane_col_origins: [usize; PLANE_COUNT],
    plane_rows: [usize; PLANE_COUNT],
    plane_cols: [usize; PLANE_COUNT],
    above_level: [Vec<u32>; PLANE_COUNT],
    left_level: [Vec<u32>; PLANE_COUNT],
    above_dc: [Vec<u8>; PLANE_COUNT],
    left_dc: [Vec<u8>; PLANE_COUNT],
}

impl TileCoeffContextState {
    pub(crate) fn allocation(
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<TileCoeffContextAllocation, TileCoeffStateError> {
        if mi_rows == 0 || mi_cols == 0 {
            return Err(TileCoeffStateError::EmptyTileDimensions { mi_rows, mi_cols });
        }
        let line_entries = checked_add_usize("mi_rows + mi_cols", mi_rows, mi_cols)?;
        let plane_entries = checked_mul_usize("line_entries * planes", line_entries, PLANE_COUNT)?;
        let level_entries = plane_entries;
        let dc_entries = plane_entries;
        let total_entries =
            checked_add_usize("level_entries + dc_entries", level_entries, dc_entries)?;
        Ok(TileCoeffContextAllocation {
            above_len: mi_cols,
            left_len: mi_rows,
            total_entries,
        })
    }

    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileCoeffStateError> {
        Self::new_with_chroma_sampling(mi_rows, mi_cols, ChromaSampling::Yuv444)
    }

    pub(crate) fn new_with_chroma_sampling(
        mi_rows: usize,
        mi_cols: usize,
        chroma: ChromaSampling,
    ) -> Result<Self, TileCoeffStateError> {
        Self::new_for_tile_with_chroma_sampling(0..mi_rows, 0..mi_cols, chroma)
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
            mi_rows: rows,
            mi_cols: cols,
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

    #[must_use]
    pub(crate) const fn mi_rows(&self) -> usize {
        self.mi_rows
    }

    #[must_use]
    pub(crate) const fn mi_cols(&self) -> usize {
        self.mi_cols
    }

    pub(crate) fn above_level(&self, plane: usize) -> Result<&[u32], TileCoeffStateError> {
        Ok(&self.above_level[validate_plane(plane)?])
    }

    pub(crate) fn left_level(&self, plane: usize) -> Result<&[u32], TileCoeffStateError> {
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
pub(crate) struct TileCoeffContextAllocation {
    above_len: usize,
    left_len: usize,
    total_entries: usize,
}

impl TileCoeffContextAllocation {
    #[must_use]
    pub(crate) const fn above_len(self) -> usize {
        self.above_len
    }

    #[must_use]
    pub(crate) const fn left_len(self) -> usize {
        self.left_len
    }

    #[must_use]
    pub(crate) const fn total_entries(self) -> usize {
        self.total_entries
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffContextUpdate {
    pub(crate) plane: usize,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
    pub(crate) cul_level: u32,
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
    level: &mut [u32],
    dc: &mut [u8],
    range: Range<usize>,
    level_value: u32,
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
