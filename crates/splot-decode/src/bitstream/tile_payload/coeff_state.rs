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
/// Row stride of every level and sign grid, so neighbour offsets are
/// constants.
pub(crate) const LEVEL_STRIDE: usize = MAX_ADJUSTED_TX_EXTENT + LEVEL_GRID_PAD;
const MAX_PADDED_COEFF_LEN: usize = LEVEL_STRIDE * (MAX_ADJUSTED_TX_EXTENT + LEVEL_GRID_PAD);
/// Mask that keeps a block position or a neighbour offset below half the
/// grid, a power of two at least [`MAX_PADDED_COEFF_LEN`].
pub(crate) const LEVEL_INDEX_MASK: usize = MAX_PADDED_COEFF_LEN.next_power_of_two() - 1;
/// Length of every level grid: a masked position plus a masked offset
/// always indexes it, so neighbour reads need no bounds checks.
pub(crate) const LEVEL_GRID_LEN: usize = 2 * (LEVEL_INDEX_MASK + 1);
/// A block's level grid; entries past its padded extent are zero.
pub(crate) type LevelGrid = [u8; LEVEL_GRID_LEN];
static ZERO_LEVEL_GRID: LevelGrid = [0; LEVEL_GRID_LEN];

static ZERO_QUANT_SIGN: [i8; MAX_PADDED_COEFF_LEN] = [0; MAX_PADDED_COEFF_LEN];
/// Mask that keeps a block position of the ordinary coefficient pass's level
/// grid below its padded words.
pub(crate) const LEVEL_WORDS_MASK: usize = 2047;
/// The ordinary coefficient pass's level grid: one word per position at
/// [`LEVEL_STRIDE`], zero outside a block while no block is being decoded,
/// with room past every masked position for the largest neighbour offset.
pub(crate) type CoeffLevelWords = [u32; LEVEL_WORDS_MASK + 1 + 256];
/// The scan indices above 0 of one block's nonzero levels, in decode order.
pub(crate) type CoeffNonzeroList = [u32; 1024];
const LEVEL_WORDS: usize = core::mem::size_of::<CoeffLevelWords>() / 4;
const PLANES: [PlaneId; PLANE_COUNT] = [PlaneId::Y, PlaneId::U, PlaneId::V];

use core::cell::Cell;

#[inline]
fn zero_buffer<T: Default + Copy>(
    mut buffer: Vec<T>,
    len: usize,
) -> Result<Vec<T>, TryReserveError> {
    buffer.clear();
    if buffer.capacity() < len {
        buffer.try_reserve_exact(len)?;
    }
    buffer.resize(len, T::default());
    Ok(buffer)
}

/// A transform block's level grid with the length of its last used
/// extent, and its sign buffer.
///
/// The two extents are one past the highest index the last block wrote, so
/// the next block only re-zeroes that dirt instead of the whole arrays.
type CoeffBlockBuffers = (Vec<u8>, usize, Vec<i8>, usize);

std::thread_local! {
    /// One transform block's buffers, reused by the next block on this worker.
    static COEFF_BLOCK_BUFFERS: Cell<Option<CoeffBlockBuffers>> = const { Cell::new(None) };
}

impl Drop for TransformCoeffBlockState {
    fn drop(&mut self) {
        if self.level.len() != LEVEL_GRID_LEN {
            return;
        }
        COEFF_BLOCK_BUFFERS.with(|slot| {
            slot.set(Some((
                core::mem::take(&mut self.level),
                self.level_len,
                core::mem::take(&mut self.quant_sign),
                self.sign_end,
            )));
        });
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformCoeffBlockState {
    width: usize,
    height: usize,
    /// Padded extent of the block inside `level`.
    level_len: usize,
    /// [`LEVEL_GRID_LEN`] entries, zero past `level_len`.
    level: Vec<u8>,
    quant_sign: Vec<i8>,
    /// One past the highest `quant_sign` index the block wrote.
    sign_end: usize,
}

impl TransformCoeffBlockState {
    pub(crate) fn allocation(width: usize, height: usize) -> Result<usize, TileCoeffStateError> {
        validate_adjusted_extent("width", width)?;
        validate_adjusted_extent("height", height)?;
        checked_mul_usize("width * height", width, height)
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: an out-of-line return spills the state through memory"
    )]
    #[inline(always)]
    pub(crate) fn new(width: usize, height: usize) -> Result<Self, TileCoeffStateError> {
        Self::allocation(width, height)?;
        let level_len = LEVEL_STRIDE * (height + LEVEL_GRID_PAD);
        let (mut level, used_len, mut quant_sign, sign_end) =
            COEFF_BLOCK_BUFFERS.with(Cell::take).unwrap_or_default();
        if level.len() == LEVEL_GRID_LEN {
            if let Some(used) = level.get_mut(..used_len.max(level_len)) {
                used.fill(0);
            }
        } else {
            level = zero_buffer(level, LEVEL_GRID_LEN)?;
        }
        quant_sign.clear();
        Ok(Self {
            width,
            height,
            level_len,
            level,
            quant_sign,
            sign_end,
        })
    }

    pub(crate) fn ensure_quant_sign(&mut self) {
        if self.quant_sign.is_empty() {
            let mut buffer = core::mem::take(&mut self.quant_sign);
            if buffer.len() > self.level_len {
                buffer.truncate(self.level_len);
            }
            let dirty = self.sign_end.min(buffer.len());
            buffer[..dirty].fill(0);
            if buffer.len() < self.level_len {
                buffer.resize(self.level_len, 0);
            }
            self.quant_sign = buffer;
            self.sign_end = 0;
        }
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
    #[inline]
    pub(crate) fn level(&self) -> &LevelGrid {
        self.level.first_chunk().unwrap_or(&ZERO_LEVEL_GRID)
    }

    pub(crate) fn level_mut(&mut self) -> &mut [u8] {
        &mut self.level
    }

    /// Row stride of the padded level and sign grids.
    #[must_use]
    #[allow(
        clippy::unused_self,
        reason = "callers stay agnostic of the fixed stride"
    )]
    pub(crate) const fn level_stride(&self) -> usize {
        LEVEL_STRIDE
    }

    #[must_use]
    pub(crate) fn quant_sign(&self) -> &[i8] {
        if self.quant_sign.is_empty() {
            &ZERO_QUANT_SIGN[..self.level_len]
        } else {
            &self.quant_sign
        }
    }

    #[inline]
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

    #[inline]
    pub(crate) fn set_quant_sign(
        &mut self,
        row: usize,
        col: usize,
        value: i8,
    ) -> Result<(), TileCoeffStateError> {
        let idx = self.index(row, col)?;
        self.ensure_quant_sign();
        self.quant_sign[idx] = value;
        self.sign_end = self.sign_end.max(idx + 1);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn level_at(&self, row: usize, col: usize) -> Result<u32, TileCoeffStateError> {
        Ok(u32::from(self.level[self.index(row, col)?]))
    }

    #[cfg(test)]
    pub(crate) fn quant_sign_at(&self, row: usize, col: usize) -> Result<i8, TileCoeffStateError> {
        Ok(self.quant_sign()[self.index(row, col)?])
    }

    #[inline]
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
        Ok(row * LEVEL_STRIDE + col)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub(crate) struct TileCoeffContextState {
    plane_row_origins: [usize; PLANE_COUNT],
    plane_col_origins: [usize; PLANE_COUNT],
    plane_rows: [usize; PLANE_COUNT],
    plane_cols: [usize; PLANE_COUNT],
    /// Per plane 4x4 column (`above`) and row (`left`): the last block's
    /// `culLevel` with its `dcCategory` at [`DC_CATEGORY_SHIFT`].
    above: [Vec<u8>; PLANE_COUNT],
    left: [Vec<u8>; PLANE_COUNT],
    levels: Vec<u32>,
}

/// Where a context-line byte keeps the `dcCategory` (0 to 2), above the
/// `culLevel` (0 to 4) in [`CUL_LEVEL_MASK`].
pub(crate) const DC_CATEGORY_SHIFT: u32 = 3;
pub(crate) const CUL_LEVEL_MASK: u8 = (1 << DC_CATEGORY_SHIFT) - 1;

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

    /// A state laid out for one tile, for tests that want a fresh one.
    #[cfg(test)]
    pub(crate) fn new_for_tile_chroma(
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        chroma: ChromaFormatIdc,
    ) -> Result<Self, TileCoeffStateError> {
        let mut state = Self::default();
        state.reset_for_tile_chroma(mi_rows, mi_cols, chroma)?;
        Ok(state)
    }

    /// Lays this state out for another tile, keeping the context lines.
    ///
    /// The decoder holds one of these for the whole stream, so a steady-state
    /// tile zeroes the lines the last one left rather than sizing new ones.
    pub(crate) fn reset_for_tile_chroma(
        &mut self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        chroma: ChromaFormatIdc,
    ) -> Result<(), TileCoeffStateError> {
        self.reset_for_tile_with_chroma_sampling(
            mi_rows,
            mi_cols,
            ChromaSampling::from_chroma_format_idc(chroma),
        )
    }

    fn reset_for_tile_with_chroma_sampling(
        &mut self,
        mi_rows: Range<usize>,
        mi_cols: Range<usize>,
        chroma: ChromaSampling,
    ) -> Result<(), TileCoeffStateError> {
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
        self.plane_row_origins = plane_row_origins;
        self.plane_col_origins = plane_col_origins;
        self.plane_rows = plane_rows;
        self.plane_cols = plane_cols;
        zero_plane_lines(&mut self.above, plane_cols)?;
        zero_plane_lines(&mut self.left, plane_rows)?;
        self.ensure_levels()?;
        Ok(())
    }

    fn ensure_levels(&mut self) -> Result<(), TileCoeffStateError> {
        const LEN: usize = LEVEL_WORDS + core::mem::size_of::<CoeffNonzeroList>() / 4;
        if self.levels.len() != LEN {
            self.levels = zero_buffer(core::mem::take(&mut self.levels), LEN)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn above_level(&self, plane: usize) -> Result<Vec<u8>, TileCoeffStateError> {
        Ok(cul_levels(&self.above[validate_plane(plane)?]))
    }

    #[cfg(test)]
    pub(crate) fn left_level(&self, plane: usize) -> Result<Vec<u8>, TileCoeffStateError> {
        Ok(cul_levels(&self.left[validate_plane(plane)?]))
    }

    #[cfg(test)]
    pub(crate) fn above_dc(&self, plane: usize) -> Result<Vec<u8>, TileCoeffStateError> {
        Ok(dc_categories(&self.above[validate_plane(plane)?]))
    }

    #[cfg(test)]
    pub(crate) fn left_dc(&self, plane: usize) -> Result<Vec<u8>, TileCoeffStateError> {
        Ok(dc_categories(&self.left[validate_plane(plane)?]))
    }

    /// The above context line of `plane`, see [`DC_CATEGORY_SHIFT`].
    pub(crate) fn above_line(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.above[validate_plane(plane)?])
    }

    /// The left context line of `plane`, see [`DC_CATEGORY_SHIFT`].
    pub(crate) fn left_line(&self, plane: usize) -> Result<&[u8], TileCoeffStateError> {
        Ok(&self.left[validate_plane(plane)?])
    }

    /// The above and left context lines of `plane` with the level grid and
    /// nonzero list of the coefficient passes, allocated on first use.
    #[allow(clippy::type_complexity)]
    pub(crate) fn dc_lines_and_levels(
        &mut self,
        plane: usize,
    ) -> Result<(&[u8], &[u8], &mut CoeffLevelWords, &mut CoeffNonzeroList), TileCoeffStateError>
    {
        let plane = validate_plane(plane)?;
        self.ensure_levels()?;
        let (levels, nonzero) = self.levels.split_at_mut(LEVEL_WORDS);
        let (Some(levels), Some(nonzero)) = (levels.first_chunk_mut(), nonzero.first_chunk_mut())
        else {
            return Err(TileCoeffStateError::InvalidPlane { plane });
        };
        Ok((&self.above[plane], &self.left[plane], levels, nonzero))
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

    /// Calls `visit` with the context entries over luma 4x4 rows `rows` and
    /// columns `cols`, a chroma plane's over that span shifted by the chroma
    /// subsampling `(x, y)` and rounded outwards, for the encoder facade's rollback.
    pub(crate) fn visit_region(
        &mut self,
        rows: &Range<usize>,
        cols: &Range<usize>,
        (sub_x, sub_y): (usize, usize),
        mut visit: impl FnMut(&mut [u8]),
    ) {
        for plane in 0..PLANE_COUNT {
            let (shift_x, shift_y) = if plane == 0 { (0, 0) } else { (sub_x, sub_y) };
            let local = |range: &Range<usize>, shift: usize, origin: usize, len: usize| {
                let start = (range.start >> shift).saturating_sub(origin).min(len);
                let end = range.end.div_ceil(1 << shift).saturating_sub(origin);
                start..end.min(len)
            };
            let r = local(
                rows,
                shift_y,
                self.plane_row_origins[plane],
                self.plane_rows[plane],
            );
            let c = local(
                cols,
                shift_x,
                self.plane_col_origins[plane],
                self.plane_cols[plane],
            );
            for (line, span) in [(&mut self.above[plane], &c), (&mut self.left[plane], &r)] {
                if let Some(cells) = line.get_mut(span.clone()) {
                    visit(cells);
                }
            }
        }
    }

    #[inline]
    pub(crate) fn update_after_coeffs(
        &mut self,
        input: CoeffContextUpdate,
    ) -> Result<(), TileCoeffStateError> {
        validate_dc_category(input.dc_category)?;
        let lines = self.block_lines(input.plane, [input.x4, input.y4, input.w4, input.h4])?;
        self.commit_block(&lines, input.cul_level, input.dc_category);
        Ok(())
    }

    /// The above and left context-line spans of the transform block at
    /// `[x4, y4, w4, h4]` (plane 4x4 units), checked once so the block's
    /// context reads and its commit need no further checks.
    #[inline]
    pub(crate) fn block_lines(
        &self,
        plane: usize,
        [x4, y4, w4, h4]: [usize; 4],
    ) -> Result<CoeffBlockLines, TileCoeffStateError> {
        let plane = validate_plane(plane)?;
        if w4 == 0 {
            return Err(TileCoeffStateError::EmptyContextRange { axis: "columns" });
        }
        if h4 == 0 {
            return Err(TileCoeffStateError::EmptyContextRange { axis: "rows" });
        }
        let (cols, rows) = (self.plane_cols[plane], self.plane_rows[plane]);
        let x4 = local_context_coordinate("above", x4, self.plane_col_origins[plane], cols)?;
        let y4 = local_context_coordinate("left", y4, self.plane_row_origins[plane], rows)?;
        Ok(CoeffBlockLines {
            plane,
            above: edge_clamped_range("above", x4, w4, cols)?,
            left: edge_clamped_range("left", y4, h4, rows)?,
            x4,
            y4,
            w4,
            h4,
        })
    }

    /// The OR of the block's above and left context bytes; see
    /// [`DC_CATEGORY_SHIFT`].
    #[allow(clippy::inline_always, reason = "measured: per-block context read")]
    #[inline(always)]
    pub(crate) fn line_or(&self, lines: &CoeffBlockLines) -> [u8; 2] {
        let plane = lines.plane.min(PLANE_COUNT - 1);
        let (above, left) = (&self.above[plane], &self.left[plane]);
        [
            above
                .get(lines.above.clone())
                .map_or(0, crate::support::or_mi_run),
            left.get(lines.left.clone())
                .map_or(0, crate::support::or_mi_run),
        ]
    }

    /// Records the block's `culLevel` and `dcCategory` in its spans.
    #[allow(clippy::inline_always, reason = "measured: per-block context commit")]
    #[inline(always)]
    pub(crate) fn commit_block(&mut self, lines: &CoeffBlockLines, level: u8, dc_category: u8) {
        let plane = lines.plane.min(PLANE_COUNT - 1);
        let value = level | dc_category << DC_CATEGORY_SHIFT;
        for (line, range) in [
            (&mut self.above[plane], &lines.above),
            (&mut self.left[plane], &lines.left),
        ] {
            if let Some(run) = line.get_mut(range.clone()) {
                crate::support::fill_mi_run(run, value);
            }
        }
    }

    pub(crate) fn reset_block_context_plane(
        &mut self,
        input: CoeffContextReset,
    ) -> Result<(), TileCoeffStateError> {
        let (plane, above, left) = self.reset_spans(input)?;
        zero_context_span(&mut self.above[plane], above)?;
        zero_context_span(&mut self.left[plane], left)
    }

    /// [`Self::reset_block_context_plane`] of every chroma plane at once:
    /// they share one line layout, so `input` (of plane 1) sizes them all.
    pub(crate) fn reset_chroma_block_context(
        &mut self,
        input: CoeffContextReset,
    ) -> Result<(), TileCoeffStateError> {
        let (_, above, left) = self.reset_spans(input)?;
        for plane in 1..PLANE_COUNT {
            zero_context_span(&mut self.above[plane], above.clone())?;
            zero_context_span(&mut self.left[plane], left.clone())?;
        }
        Ok(())
    }

    fn reset_spans(
        &self,
        input: CoeffContextReset,
    ) -> Result<(usize, Range<usize>, Range<usize>), TileCoeffStateError> {
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
        Ok((plane, above, left))
    }
}

/// A transform block's checked spans of its plane's context lines, with
/// its tile-local 4x4 origin and its 4x4 extent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBlockLines {
    pub(crate) plane: usize,
    above: Range<usize>,
    left: Range<usize>,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
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

#[inline]
fn zero_context_span(line: &mut [u8], range: Range<usize>) -> Result<(), TileCoeffStateError> {
    let len = line.len();
    let Some(span) = line.get_mut(range.clone()) else {
        return Err(TileCoeffStateError::ContextRangeOutOfBounds {
            context: "context line",
            start: range.start,
            end: range.end,
            len,
        });
    };
    crate::support::fill_mi_run(span, 0);
    Ok(())
}

#[cfg(test)]
fn cul_levels(line: &[u8]) -> Vec<u8> {
    line.iter().map(|&byte| byte & CUL_LEVEL_MASK).collect()
}

#[cfg(test)]
fn dc_categories(line: &[u8]) -> Vec<u8> {
    line.iter().map(|&byte| byte >> DC_CATEGORY_SHIFT).collect()
}

fn zero_plane_lines<T: Default>(
    lines: &mut [Vec<T>; PLANE_COUNT],
    lengths: [usize; PLANE_COUNT],
) -> Result<(), TileCoeffStateError> {
    for (line, len) in lines.iter_mut().zip(lengths) {
        zero_line(line, len)?;
    }
    Ok(())
}

fn zero_line<T: Default>(values: &mut Vec<T>, len: usize) -> Result<(), TileCoeffStateError> {
    values.clear();
    if values.capacity() < len {
        values.try_reserve_exact(len)?;
    }
    values.resize_with(len, T::default);
    Ok(())
}

#[cfg(test)]
#[path = "coeff_state_tests.rs"]
mod tests;
