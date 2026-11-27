// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The IntraBC block-vector conformance checks: § 6.19.7.12 `is_mv_valid`
//! with `check_valid_local_ibc`, § 6.19.5.4 `is_offset_mv_valid`, and the
//! § 5.20.4.1 `update_ibc_buffers` state that the local area reads.

use core::ops::Range;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;

use super::intrabc_records::{
    IntrabcInfo, frame_allows_local_intrabc, intrabc_invalid_block_vector_error,
    tile_interval_for_block,
};
use super::tx_records::selectable_state_error;
use crate::bitstream::tile_payload::DecodeBlockFrontier;
use crate::error::Result;

/// `MI_SIZE` (§ 3).
const MI_SIZE: i64 = 4;
/// `IBC_BUFFER_SIZE_LOG2` (§ 3).
const IBC_BUFFER_SIZE_LOG2: u32 = 6;
/// `IBC_NUM_BUFFERS` (§ 3).
const IBC_NUM_BUFFERS: usize = 4;
/// `INTRABC_DELAY_SB64` (§ 3).
const INTRABC_DELAY_SB64: i64 = 4;

/// `IBCBufferValid`, `IBCBufferRow`, `IBCBufferCol`, `IBCBufferCurRow` and
/// `IBCBufferCurCol` of one tile, in `IBC_BUFFER_SIZE` units.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IbcBuffers {
    sb64: bool,
    cur: (usize, usize),
    stored: [Option<(usize, usize)>; IBC_NUM_BUFFERS],
}

impl IbcBuffers {
    /// The § 5.20.2.1 reset before the superblock row at `mi_row`, with
    /// `sb64` for `SbSize == BLOCK_64X64`.
    #[cfg(test)]
    pub(crate) fn start_sb_row(mi_row: usize, sb64: bool) -> Self {
        Self::start_sb_row_at(mi_row, 0, sb64)
    }

    /// Tile-local § 5.20.2.1 superblock-row reset starting at column `mi_col`.
    pub(crate) fn start_sb_row_at(mi_row: usize, mi_col: usize, sb64: bool) -> Self {
        let shift = IBC_BUFFER_SIZE_LOG2 - 2;
        Self {
            sb64,
            cur: (mi_row >> shift, mi_col >> shift),
            stored: [None; IBC_NUM_BUFFERS],
        }
    }

    /// § 5.20.4.1 `update_ibc_buffers(miRow, miCol)`. Returns whether `cur`
    /// moved to a different buffer; when `sb64` holds and the old buffer's
    /// superblock is `BRU_INACTIVE`, all four buffer-valid entries are cleared.
    #[inline]
    pub(crate) fn update(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        is_bru_inactive: impl FnOnce(usize, usize) -> bool,
    ) -> bool {
        let shift = IBC_BUFFER_SIZE_LOG2 - 2;
        let at = (mi_row >> shift, mi_col >> shift);
        if at == self.cur {
            return false;
        }
        let old = self.cur;
        self.stored[self.index(old)] = Some(old);
        if self.sb64 && is_bru_inactive(old.0 << shift, old.1 << shift) {
            self.stored = [None; IBC_NUM_BUFFERS];
        }
        self.cur = at;
        true
    }

    /// § 5.20.4.1 `ibc_buffer_index(row, col)`.
    const fn index(&self, (row, col): (usize, usize)) -> usize {
        if self.sb64 {
            col & 3
        } else {
            (col & 1) | ((row & 1) << 1)
        }
    }

    #[cfg(test)]
    pub(crate) const fn cur(&self) -> (usize, usize) {
        self.cur
    }

    #[cfg(test)]
    pub(crate) const fn stored(&self) -> [Option<(usize, usize)>; IBC_NUM_BUFFERS] {
        self.stored
    }
}

/// Sentinel `cur_sb_row` before the first `begin_superblock` call, chosen so
/// `mi_row.wrapping_sub(UNINITIALIZED_SB_ROW) >= sb_h4` for all frame coords.
const UNINITIALIZED_SB_ROW: usize = usize::MAX / 2;

/// The leaf-order state that `check_valid_local_ibc` reads in unit tests.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct LocalAreaState<'a> {
    /// `enable_sdp && FrameIsIntra`.
    pub(crate) sdp_intra: bool,
    pub(crate) buffers: &'a IbcBuffers,
    /// `IBCCoded[row][col]` in 4x4 units.
    pub(crate) coded: &'a dyn Fn(usize, usize) -> bool,
}

/// Per-tile § 5.20.4.1 / § 6.19.7.12 local IntraBC buffer and `IBCCoded` state
/// for one superblock row at a time, with a single pre-allocated bitset.
#[derive(Clone, Debug)]
pub(crate) struct TileLocalIbcState {
    sb_h4: usize,
    sb64: bool,
    sdp_intra: bool,
    tile_rows: Range<usize>,
    tile_cols: Range<usize>,
    row_words: usize,
    cur_sb_row: usize,
    cur_bru_inactive: bool,
    buffers: IbcBuffers,
    coded: Vec<u64>,
}

impl TileLocalIbcState {
    /// Allocates one superblock-row `IBCCoded` bitset covering `tile_cols`,
    /// aligned up to `sb_h4` for clipped rightmost superblocks, and retains the
    /// tile bounds for block-vector validation.
    pub(crate) fn new(
        tile_rows: Range<usize>,
        tile_cols: Range<usize>,
        sb_h4: usize,
        sdp_intra: bool,
    ) -> Result<Self> {
        let sb_h4 = sb_h4.max(1);
        let tile_col_start = tile_cols.start;
        let cols_span = tile_cols.end.saturating_sub(tile_col_start);
        let cols_aligned = cols_span.div_ceil(sb_h4).saturating_mul(sb_h4).max(sb_h4);
        let row_words = cols_aligned.div_ceil(64).max(1);
        let len = sb_h4.saturating_mul(row_words);
        let mut coded = Vec::new();
        coded.try_reserve_exact(len).map_err(|_| {
            crate::error::DecodeError::from(splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: splot_recon::PlaneId::Y,
                context: "local IBC coded grid",
            })
        })?;
        coded.resize(len, 0);
        Ok(Self {
            sb_h4,
            sb64: sb_h4 == 16,
            sdp_intra,
            tile_rows,
            tile_cols,
            row_words,
            cur_sb_row: UNINITIALIZED_SB_ROW,
            cur_bru_inactive: false,
            buffers: IbcBuffers::start_sb_row_at(0, tile_col_start, sb_h4 == 16),
            coded,
        })
    }

    /// Resets row-scoped `IBCCoded` flags and `IbcBuffers` when entering a new
    /// superblock row; stays a no-op within the same superblock row so
    /// transitions still inspect the previous buffer's `cur_bru_inactive`.
    #[inline]
    pub(crate) fn begin_superblock(&mut self, sb_row: usize, bru_inactive: bool) {
        if sb_row != self.cur_sb_row {
            self.cur_sb_row = sb_row;
            self.coded.fill(0);
            self.buffers = IbcBuffers::start_sb_row_at(sb_row, self.tile_cols.start, self.sb64);
            self.cur_bru_inactive = bru_inactive;
        }
    }

    /// Runs the § 5.20.4.1 `update_ibc_buffers` and `IBCCoded` marks before a
    /// decoded luma-tree leaf executes block-vector validation.
    #[inline]
    pub(crate) fn enter_leaf(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        bru_inactive: bool,
    ) {
        self.update_buffer(mi_row, mi_col, bru_inactive);
        self.mark_coded(mi_row, mi_col, n4w, n4h);
    }

    /// Runs the § 5.20.9 2x2-grouped 64x64 chunk `update_ibc_buffers` walk for
    /// blocks wider or taller than 64 luma samples (`n4w > 16 || n4h > 16`).
    #[inline]
    pub(crate) fn finish_large_leaf(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        bru_inactive: bool,
    ) {
        if n4w <= 16 && n4h <= 16 {
            return;
        }
        let width_chunks = (n4w >> 4).max(1);
        let height_chunks = (n4h >> 4).max(1);
        for start_y in (0..height_chunks).step_by(2) {
            for start_x in (0..width_chunks).step_by(2) {
                for chunk_y in start_y..(start_y + 2).min(height_chunks) {
                    for chunk_x in start_x..(start_x + 2).min(width_chunks) {
                        self.update_buffer(
                            mi_row + (chunk_y << 4),
                            mi_col + (chunk_x << 4),
                            bru_inactive,
                        );
                    }
                }
            }
        }
    }

    /// Runs § 5.20.3.1 `if (r >= MiRows || c >= MiCols)` for an out-of-frame
    /// partition node: marks padded 4x4 cells coded and updates 64x64 chunks.
    pub(crate) fn record_out_of_frame(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        bru_inactive: bool,
    ) {
        self.mark_coded(mi_row, mi_col, n4w, n4h);
        let width_chunks = (n4w >> 4).max(1);
        let height_chunks = (n4h >> 4).max(1);
        for chunk_y in 0..height_chunks {
            for chunk_x in 0..width_chunks {
                self.update_buffer(
                    mi_row + (chunk_y << 4),
                    mi_col + (chunk_x << 4),
                    bru_inactive,
                );
            }
        }
    }

    /// Returns `IBCCoded[mi_row][mi_col]` inside the active superblock row.
    #[inline]
    pub(crate) fn is_coded(&self, mi_row: usize, mi_col: usize) -> bool {
        let row = mi_row.wrapping_sub(self.cur_sb_row);
        let col = mi_col.wrapping_sub(self.tile_cols.start);
        let word = col >> 6;
        if row >= self.sb_h4 || word >= self.row_words {
            return false;
        }
        self.coded
            .get(row * self.row_words + word)
            .is_some_and(|&bits| ((bits >> (col & 63)) & 1) == 1)
    }

    #[inline]
    fn update_buffer(&mut self, mi_row: usize, mi_col: usize, bru_inactive: bool) {
        let old_inactive = self.cur_bru_inactive;
        if self.buffers.update(mi_row, mi_col, |_, _| old_inactive) {
            self.cur_bru_inactive = bru_inactive;
        }
    }

    #[inline]
    fn mark_coded(&mut self, mi_row: usize, mi_col: usize, n4w: usize, n4h: usize) {
        let Some(row0) = mi_row.checked_sub(self.cur_sb_row) else {
            return;
        };
        let Some(col0) = mi_col.checked_sub(self.tile_cols.start) else {
            return;
        };
        let row_end = row0.saturating_add(n4h).min(self.sb_h4);
        let col_end = col0
            .saturating_add(n4w)
            .min(self.row_words.saturating_mul(64));
        if row0 >= row_end || col0 >= col_end {
            return;
        }
        let Some(slice) = self
            .coded
            .get_mut(row0 * self.row_words..row_end * self.row_words)
        else {
            return;
        };
        let bit = col0 & 63;
        let width = col_end - col0;
        if bit + width <= 64 {
            let mask = if width == 64 {
                u64::MAX
            } else {
                ((1u64 << width) - 1) << bit
            };
            let mut word = col0 >> 6;
            while let Some(cell) = slice.get_mut(word) {
                *cell |= mask;
                word += self.row_words;
            }
            return;
        }
        for row in slice.chunks_exact_mut(self.row_words) {
            let mut c = col0;
            while c < col_end {
                let word = c >> 6;
                let bit = c & 63;
                let take = (64 - bit).min(col_end - c);
                let mask = if take == 64 {
                    u64::MAX
                } else {
                    ((1u64 << take) - 1) << bit
                };
                if let Some(cell) = row.get_mut(word) {
                    *cell |= mask;
                }
                c += take;
            }
        }
    }

    #[cfg(test)]
    pub(crate) const fn buffers(&self) -> &IbcBuffers {
        &self.buffers
    }
}

/// The block and frame facts that § 6.19.7.12 `is_mv_valid` reads, in luma
/// samples and 4x4 units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcMvBlock {
    /// `MiRow`, `MiCol`.
    pub(crate) mi: (usize, usize),
    /// `Block_Width[MiSize]`, `Block_Height[MiSize]`.
    pub(crate) size: (usize, usize),
    /// With `HasChroma`: `ChromaMiRow`, `ChromaMiCol` and
    /// `Block_Width[ChromaMiSize]`, `Block_Height[ChromaMiSize]`.
    pub(crate) chroma: Option<((usize, usize), (usize, usize))>,
    /// `MiRowStart..MiRowEnd`, `MiColStart..MiColEnd`.
    pub(crate) tile: (Range<usize>, Range<usize>),
    /// `Block_Width[SbSize]` (= `Block_Height[SbSize]`).
    pub(crate) sb_size: usize,
    pub(crate) allow_local_intrabc: bool,
    pub(crate) allow_global_intrabc: bool,
    /// `AllowExtraIBCRange`.
    pub(crate) allow_extra_ibc_range: bool,
}

#[derive(Clone, Copy)]
struct LocalAreaRef<'a, F> {
    sdp_intra: bool,
    buffers: &'a IbcBuffers,
    coded: F,
}

/// § 6.19.5.4 `is_offset_mv_valid(dx, dy)` of the block vector `mv`
/// (`[row, col]` in 1/8 samples).
#[cfg(test)]
pub(crate) fn is_offset_mv_valid(
    block: &IntrabcMvBlock,
    local: Option<&LocalAreaState<'_>>,
    mv: [i32; 2],
    (dx, dy): (i32, i32),
) -> bool {
    let offset = [
        i64::from(mv[0]) + i64::from(dy) * 8,
        i64::from(mv[1]) + i64::from(dx) * 8,
    ];
    is_mv_valid_wide(
        block,
        local.map(|state| LocalAreaRef {
            sdp_intra: state.sdp_intra,
            buffers: state.buffers,
            coded: state.coded,
        }),
        offset,
    )
}

/// § 6.19.7.12 `is_mv_valid(mv)` for a block that codes `use_intrabc`.
#[cfg(test)]
pub(crate) fn is_mv_valid(
    block: &IntrabcMvBlock,
    local: Option<&LocalAreaState<'_>>,
    mv: [i32; 2],
) -> bool {
    is_mv_valid_wide(
        block,
        local.map(|state| LocalAreaRef {
            sdp_intra: state.sdp_intra,
            buffers: state.buffers,
            coded: state.coded,
        }),
        mv.map(i64::from),
    )
}

/// The source edges `[srcTopEdge, srcLeftEdge, srcBottomEdge, srcRightEdge]`
/// of § 6.19.7.12 `is_mv_valid` for the leaf at `mi` of `size` samples, with
/// `HasChroma` at `chroma_mi`.
fn source_edges(
    mi: (usize, usize),
    size: (usize, usize),
    chroma_mi: Option<(usize, usize)>,
    mv: [i64; 2],
) -> [i64; 4] {
    let bottom_border = i64::from(mv[0] & 7 != 0);
    let right_border = i64::from(mv[1] & 7 != 0);
    let (delta_row, delta_col) = (mv[0] >> 3, mv[1] >> 3);
    let top = mi.0 as i64 * MI_SIZE + delta_row;
    let left = mi.1 as i64 * MI_SIZE + delta_col;
    let bottom = top + size.1 as i64 + bottom_border;
    let right = left + size.0 as i64 + right_border;
    let (top, left) = match chroma_mi {
        Some((row, col)) => (
            row as i64 * MI_SIZE + delta_row,
            col as i64 * MI_SIZE + delta_col,
        ),
        None => (top, left),
    };
    [top, left, bottom, right]
}

/// The tile-edge test of § 6.19.7.12 `is_mv_valid`: the source edges of the
/// block vector `mv` lie in the tile `[top, left, bottom, right]` (luma
/// samples).
#[inline]
pub(crate) fn source_within_tile(
    mi: (usize, usize),
    size: (usize, usize),
    chroma_mi: Option<(usize, usize)>,
    tile: [usize; 4],
    mv: [i32; 2],
) -> bool {
    let [top, left, bottom, right] = source_edges(mi, size, chroma_mi, mv.map(i64::from));
    top >= tile[0] as i64
        && left >= tile[1] as i64
        && bottom <= tile[2] as i64
        && right <= tile[3] as i64
}

#[inline]
fn is_mv_valid_wide<F: Fn(usize, usize) -> bool + Copy>(
    block: &IntrabcMvBlock,
    local: Option<LocalAreaRef<'_, F>>,
    mv: [i64; 2],
) -> bool {
    let bottom_border = i64::from(mv[0] & 7 != 0);
    let right_border = i64::from(mv[1] & 7 != 0);
    let delta_row = mv[0] >> 3;
    let delta_col = mv[1] >> 3;
    let (mi_row, mi_col) = (block.mi.0 as i64, block.mi.1 as i64);
    let chroma_mi = block.chroma.map(|(mi, _)| mi);
    let [src_top, src_left, src_bottom, src_right] =
        source_edges(block.mi, block.size, chroma_mi, mv);
    let (rows, cols) = &block.tile;
    if src_top < rows.start as i64 * MI_SIZE
        || src_left < cols.start as i64 * MI_SIZE
        || src_bottom > rows.end as i64 * MI_SIZE
        || src_right > cols.end as i64 * MI_SIZE
    {
        return false;
    }
    if block.allow_local_intrabc {
        let valid = local.is_some_and(|area| {
            let chroma = block.chroma.filter(|_| !area.sdp_intra);
            let ((row, col), (w, h)) = chroma.unwrap_or((block.mi, block.size));
            let top = row as i64 * MI_SIZE + delta_row;
            let left = col as i64 * MI_SIZE + delta_col;
            let bottom = top + h as i64 - 1 + bottom_border;
            let right = left + w as i64 - 1 + right_border;
            let act = (row as i64, col as i64);
            check_valid_local_ibc(area.buffers, area.coded, act, left, top)
                && check_valid_local_ibc(area.buffers, area.coded, act, right, bottom)
        });
        if valid {
            return true;
        }
    }
    if !block.allow_global_intrabc {
        return false;
    }
    let sb_h = block.sb_size as i64;
    let active_sb_row = mi_row * MI_SIZE / sb_h;
    let active_sb64_col = (mi_col * MI_SIZE) >> 6;
    let src_sb_row = (src_bottom - 1) / sb_h;
    let src_sb64_col = (src_right - 1) >> 6;
    let active_sb64_row = (mi_row * MI_SIZE) >> 6;
    let is_bottom_left = active_sb64_col & 1 == 0 && active_sb64_row & 1 == 1;
    let sb64_residual = if block.allow_extra_ibc_range && is_bottom_left {
        -1
    } else {
        0
    };
    let total_sb64_per_row = ((cols.end as i64 - cols.start as i64 - 1) >> 4) + 1;
    let active_sb64 = active_sb_row * total_sb64_per_row + active_sb64_col;
    let src_sb64 = src_sb_row * total_sb64_per_row + src_sb64_col;
    if src_sb64 >= active_sb64 - INTRABC_DELAY_SB64 - sb64_residual {
        return false;
    }
    let gradient = INTRABC_DELAY_SB64 + sb_h / 64;
    let wf_offset = gradient * (active_sb_row - src_sb_row);
    !(src_sb_row > active_sb_row
        || src_sb64_col >= active_sb64_col - INTRABC_DELAY_SB64 + wf_offset - sb64_residual)
}

/// § 6.19.7.12 `check_valid_local_ibc(x, y)`; `act` is `(actRow, actCol)`.
#[inline]
fn check_valid_local_ibc<F: Fn(usize, usize) -> bool>(
    buffers: &IbcBuffers,
    coded: F,
    act: (i64, i64),
    x: i64,
    y: i64,
) -> bool {
    if x >= act.1 * MI_SIZE && y >= act.0 * MI_SIZE {
        return false;
    }
    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
        return false;
    };
    if !coded(y >> 2, x >> 2) {
        return false;
    }
    let buf = (y >> IBC_BUFFER_SIZE_LOG2, x >> IBC_BUFFER_SIZE_LOG2);
    let buf_idx = buffers.index(buf);
    let in_current = buf == buffers.cur;
    if !in_current && buffers.stored[buf_idx] != Some(buf) {
        return false;
    }
    if buf_idx == buffers.index(buffers.cur) && !in_current {
        let mask = (1 << IBC_BUFFER_SIZE_LOG2) - 1;
        let colo_y = (y & mask) | (buffers.cur.0 << IBC_BUFFER_SIZE_LOG2);
        let colo_x = (x & mask) | (buffers.cur.1 << IBC_BUFFER_SIZE_LOG2);
        if coded(colo_y >> 2, colo_x >> 2) {
            return false;
        }
    }
    true
}

/// Checks the block vector of the IntraBC leaf at `frontier` of `n4w x n4h`
/// 4x4 units in superblocks of `sb_h4` 4x4 rows with `AllowExtraIBCRange`:
/// § 6.19.7.12 `is_mv_valid` and, with `morph_pred`, § 6.19.5.4
/// `is_offset_mv_valid(-1, -1)`. Missing local-area state fails closed when
/// `allow_local_intrabc` holds.
pub(crate) fn check_intrabc_block_vector(
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    (n4w, n4h): (usize, usize),
    (sb_h4, allow_extra_ibc_range): (usize, bool),
    info: IntrabcInfo,
    local: Option<&TileLocalIbcState>,
    tile_offset: ByteOffset,
) -> Result<()> {
    let tile = if let Some(state) = local {
        (state.tile_rows.clone(), state.tile_cols.clone())
    } else {
        let tile_info = core.tile_info.as_ref().ok_or_else(selectable_state_error)?;
        let rows = tile_interval_for_block(&tile_info.mi_row_starts, frontier.r, n4h)?;
        let cols = tile_interval_for_block(&tile_info.mi_col_starts, frontier.c, n4w)?;
        (rows.0..rows.1, cols.0..cols.1)
    };
    let chroma = if frontier.has_chroma {
        let geometry = frontier.chroma_ref_geometry();
        let size = geometry.size();
        let n4 = |n: core::result::Result<usize, _>| n.map_err(|_| selectable_state_error());
        let samples = (n4(size.num_4x4_wide())? * 4, n4(size.num_4x4_high())? * 4);
        Some(((geometry.row(), geometry.col()), samples))
    } else {
        None
    };
    let params = core.intrabc;
    let block = IntrabcMvBlock {
        mi: (frontier.r, frontier.c),
        size: (n4w * 4, n4h * 4),
        chroma,
        tile,
        sb_size: sb_h4 * 4,
        allow_local_intrabc: frame_allows_local_intrabc(core),
        allow_global_intrabc: core.frame_is_intra != Some(false)
            && params.and_then(|p| p.allow_global_intrabc) == Some(true),
        allow_extra_ibc_range,
    };
    let mv = [i64::from(info.block_mv.row), i64::from(info.block_mv.col)];
    let (mv_ok, offset_ok) = if let Some(state) = local {
        let local_area = Some(LocalAreaRef {
            sdp_intra: state.sdp_intra,
            buffers: &state.buffers,
            coded: |r, c| state.is_coded(r, c),
        });
        (
            is_mv_valid_wide(&block, local_area, mv),
            !info.morph_pred || is_mv_valid_wide(&block, local_area, [mv[0] - 8, mv[1] - 8]),
        )
    } else {
        type NoCoded = fn(usize, usize) -> bool;
        (
            is_mv_valid_wide::<NoCoded>(&block, None, mv),
            !info.morph_pred || is_mv_valid_wide::<NoCoded>(&block, None, [mv[0] - 8, mv[1] - 8]),
        )
    };
    if !mv_ok {
        return Err(intrabc_invalid_block_vector_error(tile_offset));
    }
    if !offset_ok {
        return Err(crate::pipeline::malformed_tile_payload(
            tile_offset,
            "6.19.5.4",
            "IntraBC block vector with morph_pred fails the is_offset_mv_valid(-1, -1) conformance check",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "intrabc_mv_valid_tests.rs"]
mod tests;
