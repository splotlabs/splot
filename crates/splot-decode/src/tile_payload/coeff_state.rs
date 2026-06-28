// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 tile-local coefficient context state.
//!
//! Feature tracking: `DECODE-TILE-COEFF-STATE-BUFFERS`,
//! `DECODE-COEFF-ALL-ZERO-BLOCK-STATE`.

use std::collections::TryReserveError;

const PLANE_COUNT: usize = 3;
const MAX_ADJUSTED_TX_EXTENT: usize = 32;

/// Transform-block-local coefficient state for AV2 § 5.20.7.27 `coeffs()`.
///
/// The arrays are row-major, `width` wide, and sized to the adjusted transform
/// block extent used by `Level[]`, `QuantSign[]`, and `Quant[]` in
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformCoeffBlockState {
    width: usize,
    height: usize,
    level: Vec<u32>,
    quant_sign: Vec<i32>,
    quant: Vec<i32>,
}

impl TransformCoeffBlockState {
    /// Computes allocation for a transform-block coefficient state.
    pub(crate) fn allocation(
        width: usize,
        height: usize,
    ) -> Result<TransformCoeffBlockAllocation, TileCoeffStateError> {
        validate_adjusted_extent("width", width)?;
        validate_adjusted_extent("height", height)?;
        let coeff_count = checked_mul_usize("width * height", width, height)?;
        Ok(TransformCoeffBlockAllocation { coeff_count })
    }

    /// Creates a zero-initialized transform-block coefficient state.
    pub(crate) fn new(width: usize, height: usize) -> Result<Self, TileCoeffStateError> {
        let allocation = Self::allocation(width, height)?;
        Ok(Self {
            width,
            height,
            level: zeroed_u32_vec(allocation.coeff_count)?,
            quant_sign: zeroed_i32_vec(allocation.coeff_count)?,
            quant: zeroed_i32_vec(allocation.coeff_count)?,
        })
    }

    /// Adjusted transform width in coefficients.
    #[must_use]
    pub(crate) const fn width(&self) -> usize {
        self.width
    }

    /// Adjusted transform height in coefficients.
    #[must_use]
    pub(crate) const fn height(&self) -> usize {
        self.height
    }

    /// Row-major `Level[]` magnitude slice.
    #[must_use]
    pub(crate) fn level(&self) -> &[u32] {
        &self.level
    }

    /// Row-major `QuantSign[]` sign slice.
    #[must_use]
    pub(crate) fn quant_sign(&self) -> &[i32] {
        &self.quant_sign
    }

    /// Row-major `Quant[]` coefficient slice, indexed by raster position.
    #[must_use]
    pub(crate) fn quant(&self) -> &[i32] {
        &self.quant
    }

    /// Writes one `Level[row][col]` magnitude.
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

    /// Writes one `QuantSign[row][col]` sign value.
    pub(crate) fn set_quant_sign(
        &mut self,
        row: usize,
        col: usize,
        value: i32,
    ) -> Result<(), TileCoeffStateError> {
        let idx = self.index(row, col)?;
        self.quant_sign[idx] = value;
        Ok(())
    }

    /// Writes one `Quant[pos]` coefficient value.
    pub(crate) fn set_quant(&mut self, pos: usize, value: i32) -> Result<(), TileCoeffStateError> {
        let idx = self.quant_index(pos)?;
        self.quant[idx] = value;
        Ok(())
    }

    /// Reads one `Level[row][col]` magnitude.
    pub(crate) fn level_at(&self, row: usize, col: usize) -> Result<u32, TileCoeffStateError> {
        Ok(self.level[self.index(row, col)?])
    }

    /// Reads one `QuantSign[row][col]` sign value.
    pub(crate) fn quant_sign_at(&self, row: usize, col: usize) -> Result<i32, TileCoeffStateError> {
        Ok(self.quant_sign[self.index(row, col)?])
    }

    /// Reads one `Quant[pos]` coefficient value.
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

/// Allocation accounting for [`TransformCoeffBlockState`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TransformCoeffBlockAllocation {
    coeff_count: usize,
}

impl TransformCoeffBlockAllocation {
    /// Coefficient entries in each row-major local block array.
    #[must_use]
    pub(crate) const fn coeff_count(self) -> usize {
        self.coeff_count
    }
}

/// Tile-local neighbour state for coefficient CDF context derivation.
///
/// This owns the § 5.20 context lines read by the § 8.3.2 coefficient contexts:
/// `AboveLevelContext`, `LeftLevelContext`, `AboveDcContext`, and
/// `LeftDcContext` for planes 0, 1, and 2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffContextState {
    mi_rows: usize,
    mi_cols: usize,
    above_level: [Vec<u32>; PLANE_COUNT],
    left_level: [Vec<u32>; PLANE_COUNT],
    above_dc: [Vec<u8>; PLANE_COUNT],
    left_dc: [Vec<u8>; PLANE_COUNT],
}

impl TileCoeffContextState {
    /// Computes allocation for tile coefficient context lines.
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

    /// Creates zero-initialized tile coefficient context lines.
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Result<Self, TileCoeffStateError> {
        let allocation = Self::allocation(mi_rows, mi_cols)?;
        Ok(Self {
            mi_rows,
            mi_cols,
            above_level: [
                zeroed_u32_vec(allocation.above_len)?,
                zeroed_u32_vec(allocation.above_len)?,
                zeroed_u32_vec(allocation.above_len)?,
            ],
            left_level: [
                zeroed_u32_vec(allocation.left_len)?,
                zeroed_u32_vec(allocation.left_len)?,
                zeroed_u32_vec(allocation.left_len)?,
            ],
            above_dc: [
                zeroed_u8_vec(allocation.above_len)?,
                zeroed_u8_vec(allocation.above_len)?,
                zeroed_u8_vec(allocation.above_len)?,
            ],
            left_dc: [
                zeroed_u8_vec(allocation.left_len)?,
                zeroed_u8_vec(allocation.left_len)?,
                zeroed_u8_vec(allocation.left_len)?,
            ],
        })
    }

    /// Tile MI rows represented by the left context lines.
    #[must_use]
    pub(crate) const fn mi_rows(&self) -> usize {
        self.mi_rows
    }

    /// Tile MI columns represented by the above context lines.
    #[must_use]
    pub(crate) const fn mi_cols(&self) -> usize {
        self.mi_cols
    }

    /// `AboveLevelContext[plane]`.
    pub(crate) fn above_level(&self, plane: usize) -> Result<&[u32], TileCoeffStateError> {
        Ok(&self.above_level[validate_plane(plane)?])
    }

    /// `LeftLevelContext[plane]`.
    pub(crate) fn left_level(&self, plane: usize) -> Result<&[u32], TileCoeffStateError> {
        Ok(&self.left_level[validate_plane(plane)?])
    }

    /// `AboveDcContext[plane]`.
    pub(crate) fn above_dc(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.above_dc[validate_plane(plane)?])
    }

    /// `LeftDcContext[plane]`.
    pub(crate) fn left_dc(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.left_dc[validate_plane(plane)?])
    }

    /// Applies the AV2 § 5.20.7.27 end-of-`coeffs()` context writes.
    ///
    /// The spec writes `culLevel` / `dcCategory` over `x4 .. x4 + w4` above
    /// columns and `y4 .. y4 + h4` left rows
    /// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). A transform
    /// block on the bottom or right frame edge overhangs the tile by up to one
    /// transform extent, so the literal span exceeds this tile-global context
    /// line. AVM's `av2_set_entropy_contexts` (`av2/common/blockd.c:138-166`)
    /// clamps the on-frame portion with `AVMMIN(txs_*, blocks_* - off)` and
    /// zero-fills the remainder; because AVM's entropy lines are SB-local the
    /// off-frame indices are never observed, and the OR-reduce reads on the
    /// splot side already clamp with `skip(start).take(len)`. This helper models
    /// that clamp: the write covers only the on-tile indices and the overhang
    /// (which has no backing storage and is never read) is skipped. A genuine
    /// out-of-tile origin (`x4 >= mi_cols` or `y4 >= mi_rows`) is still a hard
    /// error, as AVM never produces one.
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
        let above = edge_clamped_range("above", input.x4, input.w4, self.mi_cols)?;
        let left = edge_clamped_range("left", input.y4, input.h4, self.mi_rows)?;

        for idx in above {
            self.above_level[plane][idx] = input.cul_level;
            self.above_dc[plane][idx] = input.dc_category;
        }
        for idx in left {
            self.left_level[plane][idx] = input.cul_level;
            self.left_dc[plane][idx] = input.dc_category;
        }
        Ok(())
    }

    /// Applies one plane of the AV2 § 5.20 block-context reset.
    pub(crate) fn reset_block_context_plane(
        &mut self,
        input: CoeffContextReset,
    ) -> Result<(), TileCoeffStateError> {
        let plane = validate_plane(input.plane)?;
        validate_subsampling("x", input.sub_x)?;
        validate_subsampling("y", input.sub_y)?;
        let above_start = shifted(input.c, input.sub_x)?;
        let left_start = shifted(input.r, input.sub_y)?;
        let above_unshifted_end = checked_add_usize("c + w4", input.c, input.w4).map_err(|_| {
            TileCoeffStateError::CoordinateOverflow {
                coordinate: "column",
                base: input.c,
                offset: input.w4,
            }
        })?;
        let left_unshifted_end = checked_add_usize("r + h4", input.r, input.h4).map_err(|_| {
            TileCoeffStateError::CoordinateOverflow {
                coordinate: "row",
                base: input.r,
                offset: input.h4,
            }
        })?;
        let above_end = shifted(above_unshifted_end, input.sub_x)?;
        let left_end = shifted(left_unshifted_end, input.sub_y)?;
        validate_existing_range("above reset", above_start, above_end, self.mi_cols)?;
        validate_existing_range("left reset", left_start, left_end, self.mi_rows)?;

        for idx in above_start..above_end {
            self.above_level[plane][idx] = 0;
            self.above_dc[plane][idx] = 0;
        }
        for idx in left_start..left_end {
            self.left_level[plane][idx] = 0;
            self.left_dc[plane][idx] = 0;
        }
        Ok(())
    }
}

/// Allocation accounting for [`TileCoeffContextState`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileCoeffContextAllocation {
    above_len: usize,
    left_len: usize,
    total_entries: usize,
}

impl TileCoeffContextAllocation {
    /// Entries in each above context line.
    #[must_use]
    pub(crate) const fn above_len(self) -> usize {
        self.above_len
    }

    /// Entries in each left context line.
    #[must_use]
    pub(crate) const fn left_len(self) -> usize {
        self.left_len
    }

    /// Total scalar entries across level and DC context lines.
    #[must_use]
    pub(crate) const fn total_entries(self) -> usize {
        self.total_entries
    }
}

/// Inputs for § 5.20.7.27 context writes after one coefficient block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffContextUpdate {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
    /// Caller-clamped `culLevel` written into level context lines.
    pub(crate) cul_level: u32,
    /// `dcCategory` written into DC-context lines.
    pub(crate) dc_category: u8,
}

/// Inputs for one plane of § 5.20 `reset_block_context`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffContextReset {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// Plane-local column coordinate before subsampling adjustment.
    pub(crate) c: usize,
    /// Plane-local row coordinate before subsampling adjustment.
    pub(crate) r: usize,
    /// Plane-local block width in 4x4 units before subsampling adjustment.
    pub(crate) w4: usize,
    /// Plane-local block height in 4x4 units before subsampling adjustment.
    pub(crate) h4: usize,
    /// Horizontal subsampling shift, valid AV2 values are 0 or 1.
    pub(crate) sub_x: u32,
    /// Vertical subsampling shift, valid AV2 values are 0 or 1.
    pub(crate) sub_y: u32,
}

/// Error returned by tile coefficient state helpers.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TileCoeffStateError {
    /// Tile context dimensions were empty.
    #[error("coefficient context state dimensions must be nonzero, got {mi_rows}x{mi_cols}")]
    EmptyTileDimensions {
        /// Tile MI rows.
        mi_rows: usize,
        /// Tile MI columns.
        mi_cols: usize,
    },
    /// Transform dimensions were outside the adjusted §5.20.7.27 extent.
    #[error("invalid adjusted transform {axis} {value}; expected 1..={MAX_ADJUSTED_TX_EXTENT}")]
    InvalidAdjustedTransformExtent {
        /// Axis name.
        axis: &'static str,
        /// Rejected extent.
        value: usize,
    },
    /// Allocation or indexing arithmetic overflowed.
    #[error("{operation} overflow: left {left}, right {right}")]
    ArithmeticOverflow {
        /// Operation name.
        operation: &'static str,
        /// Left operand.
        left: usize,
        /// Right operand.
        right: usize,
    },
    /// Allocation failed.
    #[error("coefficient state allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// Plane index was not one of the three AV2 planes.
    #[error("invalid coefficient context plane {plane}")]
    InvalidPlane {
        /// Rejected plane index.
        plane: usize,
    },
    /// DC category was outside the §5.20.7.27 0/1/2 categories.
    #[error("invalid coefficient DC category {dc_category}")]
    InvalidDcCategory {
        /// Rejected DC category.
        dc_category: u8,
    },
    /// A required context update range was empty.
    #[error("empty coefficient context {axis} range")]
    EmptyContextRange {
        /// Axis name.
        axis: &'static str,
    },
    /// Coordinate addition overflowed.
    #[error("{coordinate} coordinate overflow: {base} + {offset}")]
    CoordinateOverflow {
        /// Coordinate name.
        coordinate: &'static str,
        /// Base coordinate.
        base: usize,
        /// Derived offset.
        offset: usize,
    },
    /// A context range exceeded the owned tile line.
    #[error("{context} range {start}..{end} exceeds coefficient context line length {len}")]
    ContextRangeOutOfBounds {
        /// Context line name.
        context: &'static str,
        /// Inclusive start coordinate.
        start: usize,
        /// Exclusive end coordinate.
        end: usize,
        /// Owned line length.
        len: usize,
    },
    /// A transform-block coordinate exceeded the local block arrays.
    #[error(
        "coefficient transform coordinate ({row},{col}) exceeds adjusted block {height}x{width}"
    )]
    TransformCoordinateOutOfBounds {
        /// Row coordinate.
        row: usize,
        /// Column coordinate.
        col: usize,
        /// Local block height.
        height: usize,
        /// Local block width.
        width: usize,
    },
    /// A flat `Quant[]` coefficient position exceeded the local block array.
    #[error("coefficient Quant position {pos} exceeds adjusted block coefficient count {len}")]
    QuantPositionOutOfBounds {
        /// Rejected coefficient position.
        pos: usize,
        /// Local block coefficient count.
        len: usize,
    },
    /// Subsampling shift was not a valid AV2 4:2:0/4:4:4 shift.
    #[error("invalid coefficient context subsampling {axis} shift {value}")]
    InvalidSubsampling {
        /// Axis name.
        axis: &'static str,
        /// Rejected shift.
        value: u32,
    },
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

/// Builds a § 5.20.7.27 context-write range clamped to the bottom/right frame
/// edge, modelling AVM `av2_set_entropy_contexts` (`av2/common/blockd.c`).
///
/// The unclamped span is `start .. start + len`. A transform block straddling
/// the tile's right (above axis) or bottom (left axis) edge has an `end` that
/// exceeds `line_len`; the on-frame portion is `start .. line_len` and the
/// overhang is dropped (it has no backing storage and the OR-reduce reads
/// already clamp, so it is never observed). The `start + len` addition keeps the
/// existing overflow guard. A genuine out-of-tile origin (`start >= line_len`)
/// is still rejected, matching AVM which never emits such a write.
fn edge_clamped_range(
    context: &'static str,
    start: usize,
    len: usize,
    line_len: usize,
) -> Result<core::ops::Range<usize>, TileCoeffStateError> {
    let end = start
        .checked_add(len)
        .ok_or(TileCoeffStateError::CoordinateOverflow {
            coordinate: context,
            base: start,
            offset: len,
        })?;
    if start >= line_len {
        return Err(TileCoeffStateError::ContextRangeOutOfBounds {
            context,
            start,
            end,
            len: line_len,
        });
    }
    Ok(start..end.min(line_len))
}

fn validate_existing_range(
    context: &'static str,
    start: usize,
    end: usize,
    len: usize,
) -> Result<(), TileCoeffStateError> {
    if start > end || end > len {
        Err(TileCoeffStateError::ContextRangeOutOfBounds {
            context,
            start,
            end,
            len,
        })
    } else {
        Ok(())
    }
}

fn shifted(value: usize, shift: u32) -> Result<usize, TileCoeffStateError> {
    value
        .checked_shr(shift)
        .ok_or(TileCoeffStateError::InvalidSubsampling {
            axis: "shift",
            value: shift,
        })
}

fn zeroed_u32_vec(len: usize) -> Result<Vec<u32>, TileCoeffStateError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    values.resize(len, 0);
    Ok(values)
}

fn zeroed_i32_vec(len: usize) -> Result<Vec<i32>, TileCoeffStateError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    values.resize(len, 0);
    Ok(values)
}

fn zeroed_u8_vec(len: usize) -> Result<Vec<u8>, TileCoeffStateError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    values.resize(len, 0);
    Ok(values)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn update(plane: usize, x4: usize, y4: usize, w4: usize, h4: usize) -> CoeffContextUpdate {
        CoeffContextUpdate {
            plane,
            x4,
            y4,
            w4,
            h4,
            cul_level: 4,
            dc_category: 2,
        }
    }

    fn reset(
        plane: usize,
        c: usize,
        r: usize,
        w4: usize,
        h4: usize,
        sub_x: u32,
        sub_y: u32,
    ) -> CoeffContextReset {
        CoeffContextReset {
            plane,
            c,
            r,
            w4,
            h4,
            sub_x,
            sub_y,
        }
    }

    #[test]
    fn transform_block_state_is_zero_initialized_and_row_major() {
        let mut state = TransformCoeffBlockState::new(4, 3).unwrap();

        assert_eq!(state.width(), 4);
        assert_eq!(state.height(), 3);
        assert_eq!(state.level(), &[0; 12]);
        assert_eq!(state.quant_sign(), &[0; 12]);
        assert_eq!(state.quant(), &[0; 12]);

        state.set_level(2, 1, 7).unwrap();
        state.set_quant_sign(2, 1, -1).unwrap();
        state.set_quant(9, -12).unwrap();

        assert_eq!(state.level_at(2, 1).unwrap(), 7);
        assert_eq!(state.quant_sign_at(2, 1).unwrap(), -1);
        assert_eq!(state.quant_at(9).unwrap(), -12);
        assert_eq!(state.level()[9], 7);
        assert_eq!(state.quant_sign()[9], -1);
        assert_eq!(state.quant()[9], -12);
    }

    #[test]
    fn transform_block_state_rejects_invalid_extents_and_coordinates() {
        assert!(matches!(
            TransformCoeffBlockState::new(0, 4).unwrap_err(),
            TileCoeffStateError::InvalidAdjustedTransformExtent {
                axis: "width",
                value: 0
            }
        ));
        assert!(matches!(
            TransformCoeffBlockState::new(33, 4).unwrap_err(),
            TileCoeffStateError::InvalidAdjustedTransformExtent {
                axis: "width",
                value: 33
            }
        ));

        let state = TransformCoeffBlockState::new(4, 4).unwrap();
        assert!(matches!(
            state.level_at(4, 0).unwrap_err(),
            TileCoeffStateError::TransformCoordinateOutOfBounds {
                row: 4,
                col: 0,
                height: 4,
                width: 4
            }
        ));
        assert!(matches!(
            state.quant_at(16).unwrap_err(),
            TileCoeffStateError::QuantPositionOutOfBounds { pos: 16, len: 16 }
        ));
    }

    #[test]
    fn allocation_accounting_covers_transform_and_context_lines() {
        let block = TransformCoeffBlockState::allocation(32, 32).unwrap();
        assert_eq!(block.coeff_count(), 1024);

        let context = TileCoeffContextState::allocation(6, 8).unwrap();
        assert_eq!(context.above_len(), 8);
        assert_eq!(context.left_len(), 6);
        assert_eq!(context.total_entries(), 3 * (6 + 8) * 2);
    }

    #[test]
    fn tile_context_state_initializes_three_zero_planes() {
        let state = TileCoeffContextState::new(2, 3).unwrap();

        assert_eq!(state.mi_rows(), 2);
        assert_eq!(state.mi_cols(), 3);
        for plane in 0..3 {
            assert_eq!(state.above_level(plane).unwrap(), &[0, 0, 0]);
            assert_eq!(state.left_level(plane).unwrap(), &[0, 0]);
            assert_eq!(state.above_dc(plane).unwrap(), &[0, 0, 0]);
            assert_eq!(state.left_dc(plane).unwrap(), &[0, 0]);
        }
    }

    #[test]
    fn update_after_coeffs_writes_above_and_left_ranges_only() {
        // A fully-on-frame block writes its full span on both axes (the §5.20.7.27
        // frame-edge clamp in `update_after_coeffs` is a no-op here).
        let mut state = TileCoeffContextState::new(5, 6).unwrap();

        state.update_after_coeffs(update(0, 2, 1, 3, 2)).unwrap();

        assert_eq!(state.above_level(0).unwrap(), &[0, 0, 4, 4, 4, 0]);
        assert_eq!(state.above_dc(0).unwrap(), &[0, 0, 2, 2, 2, 0]);
        assert_eq!(state.left_level(0).unwrap(), &[0, 4, 4, 0, 0]);
        assert_eq!(state.left_dc(0).unwrap(), &[0, 2, 2, 0, 0]);
        assert_eq!(state.above_level(1).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.left_dc(2).unwrap(), &[0, 0, 0, 0, 0]);
    }

    #[test]
    fn update_after_coeffs_rejects_bad_facts_without_mutation() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        let before = state.clone();

        assert!(matches!(
            state
                .update_after_coeffs(update(3, 0, 0, 1, 1))
                .unwrap_err(),
            TileCoeffStateError::InvalidPlane { plane: 3 }
        ));
        assert_eq!(state, before);

        assert!(matches!(
            state
                .update_after_coeffs(CoeffContextUpdate {
                    dc_category: 3,
                    ..update(0, 0, 0, 1, 1)
                })
                .unwrap_err(),
            TileCoeffStateError::InvalidDcCategory { dc_category: 3 }
        ));
        assert_eq!(state, before);

        // An origin AT or beyond the line length is a genuine out-of-tile write
        // AVM never produces; it stays a hard error (not an edge clamp).
        assert!(matches!(
            state
                .update_after_coeffs(update(0, 2, 0, 1, 1))
                .unwrap_err(),
            TileCoeffStateError::ContextRangeOutOfBounds {
                context: "above",
                start: 2,
                end: 3,
                len: 2
            }
        ));
        assert_eq!(state, before);

        assert!(matches!(
            state
                .update_after_coeffs(update(0, 0, 2, 1, 1))
                .unwrap_err(),
            TileCoeffStateError::ContextRangeOutOfBounds {
                context: "left",
                start: 2,
                end: 3,
                len: 2
            }
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn update_after_coeffs_clamps_bottom_edge_overhang_to_on_tile_rows() {
        // Models the ac0ej3 frontier: a TX_64X64 luma block (h4 = 16) whose
        // MI-row origin overhangs the tile bottom by 2 rows. AVM
        // `av2_set_entropy_contexts` (av2/common/blockd.c:138-166) clamps the
        // left write to `AVMMIN(txs_high, blocks_high - loff)`; splot writes
        // cul_level over only the on-tile rows `y4 .. mi_rows` and never touches
        // the overhang (which has no backing storage and the reads also clamp).
        let mi_rows = 270;
        let mi_cols = 480;
        let mut state = TileCoeffContextState::new(mi_rows, mi_cols).unwrap();
        let y4 = mi_rows - 14; // 256: a 16-tall transform overhangs by 2 rows.

        state
            .update_after_coeffs(CoeffContextUpdate {
                plane: 0,
                x4: 0,
                y4,
                w4: 16,
                h4: 16,
                cul_level: 3,
                dc_category: 2,
            })
            .unwrap();

        let left_level = state.left_level(0).unwrap();
        assert_eq!(left_level.len(), mi_rows);
        for (row, &value) in left_level.iter().enumerate() {
            let expected = if (y4..mi_rows).contains(&row) { 3 } else { 0 };
            assert_eq!(value, expected, "left_level[{row}]");
        }
        let left_dc = state.left_dc(0).unwrap();
        for (row, &value) in left_dc.iter().enumerate() {
            let expected = if (y4..mi_rows).contains(&row) { 2 } else { 0 };
            assert_eq!(value, expected, "left_dc[{row}]");
        }
        // The above axis is fully on-tile, so it is written unclamped.
        assert!(state.above_level(0).unwrap()[..16].iter().all(|&v| v == 3));
        assert_eq!(state.above_level(0).unwrap()[16], 0);
    }

    #[test]
    fn update_after_coeffs_clamps_right_edge_overhang_to_on_tile_cols() {
        // Right-edge analogue: a wide transform whose column origin overhangs the
        // tile right edge. AVM clamps the above write with
        // `AVMMIN(txs_wide, blocks_wide - aoff)`.
        let mi_rows = 270;
        let mi_cols = 480;
        let mut state = TileCoeffContextState::new(mi_rows, mi_cols).unwrap();
        let x4 = mi_cols - 14; // 466: a 16-wide transform overhangs by 2 cols.

        state
            .update_after_coeffs(CoeffContextUpdate {
                plane: 0,
                x4,
                y4: 0,
                w4: 16,
                h4: 16,
                cul_level: 4,
                dc_category: 1,
            })
            .unwrap();

        let above_level = state.above_level(0).unwrap();
        assert_eq!(above_level.len(), mi_cols);
        for (col, &value) in above_level.iter().enumerate() {
            let expected = if (x4..mi_cols).contains(&col) { 4 } else { 0 };
            assert_eq!(value, expected, "above_level[{col}]");
        }
        // The left axis is fully on-tile, so it is written unclamped.
        assert!(state.left_level(0).unwrap()[..16].iter().all(|&v| v == 4));
        assert_eq!(state.left_level(0).unwrap()[16], 0);
    }

    #[test]
    fn reset_block_context_plane_zeros_subsampled_ranges() {
        let mut state = TileCoeffContextState::new(6, 8).unwrap();
        state
            .update_after_coeffs(CoeffContextUpdate {
                plane: 1,
                x4: 0,
                y4: 0,
                w4: 8,
                h4: 6,
                cul_level: 4,
                dc_category: 2,
            })
            .unwrap();

        state
            .reset_block_context_plane(reset(1, 2, 2, 4, 4, 1, 1))
            .unwrap();

        assert_eq!(state.above_level(1).unwrap(), &[4, 0, 0, 4, 4, 4, 4, 4]);
        assert_eq!(state.above_dc(1).unwrap(), &[2, 0, 0, 2, 2, 2, 2, 2]);
        assert_eq!(state.left_level(1).unwrap(), &[4, 0, 0, 4, 4, 4]);
        assert_eq!(state.left_dc(1).unwrap(), &[2, 0, 0, 2, 2, 2]);
    }

    #[test]
    fn reset_block_context_plane_handles_empty_shifted_range() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();

        state
            .reset_block_context_plane(reset(2, 0, 0, 1, 1, 1, 1))
            .unwrap();

        assert_eq!(state.above_level(2).unwrap(), &[0, 0]);
        assert_eq!(state.left_dc(2).unwrap(), &[0, 0]);
    }

    #[test]
    fn reset_block_context_plane_rejects_overflow_and_bad_subsampling() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();

        assert!(matches!(
            state
                .reset_block_context_plane(reset(0, usize::MAX, 0, 1, 1, 0, 0))
                .unwrap_err(),
            TileCoeffStateError::CoordinateOverflow {
                coordinate: "column",
                base: usize::MAX,
                offset: 1
            }
        ));
        assert!(matches!(
            state
                .reset_block_context_plane(reset(0, 0, 0, 1, 1, 2, 0))
                .unwrap_err(),
            TileCoeffStateError::InvalidSubsampling {
                axis: "x",
                value: 2
            }
        ));
    }

    #[test]
    fn rejects_empty_tile_context_dimensions_and_invalid_plane_views() {
        assert!(matches!(
            TileCoeffContextState::new(0, 1).unwrap_err(),
            TileCoeffStateError::EmptyTileDimensions {
                mi_rows: 0,
                mi_cols: 1
            }
        ));

        let state = TileCoeffContextState::new(1, 1).unwrap();
        assert!(matches!(
            state.above_dc(3).unwrap_err(),
            TileCoeffStateError::InvalidPlane { plane: 3 }
        ));
    }
}
