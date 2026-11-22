// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The IntraBC block-vector conformance checks: § 6.19.7.12 `is_mv_valid`
//! with `check_valid_local_ibc`, § 6.19.5.4 `is_offset_mv_valid`, and the
//! § 5.20.4.1 `update_ibc_buffers` state that the local area reads.

use core::ops::Range;

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::span::ByteOffset;

use super::intrabc_records::{
    IntrabcInfo, intrabc_invalid_block_vector_error, tile_interval_for_block,
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

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the leaf-order local-area state is not wired yet")
)]
impl IbcBuffers {
    /// The § 5.20.2.1 reset before the superblock row at `mi_row`, with
    /// `sb64` for `SbSize == BLOCK_64X64`.
    pub(crate) fn start_sb_row(mi_row: usize, sb64: bool) -> Self {
        Self {
            sb64,
            cur: (mi_row >> (IBC_BUFFER_SIZE_LOG2 - 2), 0),
            stored: [None; IBC_NUM_BUFFERS],
        }
    }

    /// § 5.20.4.1 `update_ibc_buffers(miRow, miCol)`. Its `BRU_INACTIVE`
    /// reset is not reached: the decoder refuses `use_bru` frames that allow
    /// IntraBC.
    pub(crate) fn update(&mut self, mi_row: usize, mi_col: usize) {
        let shift = IBC_BUFFER_SIZE_LOG2 - 2;
        let at = (mi_row >> shift, mi_col >> shift);
        if at != self.cur {
            self.stored[self.index(self.cur)] = Some(self.cur);
            self.cur = at;
        }
    }

    /// § 5.20.4.1 `ibc_buffer_index(row, col)`.
    const fn index(&self, (row, col): (usize, usize)) -> usize {
        if self.sb64 {
            col & 3
        } else {
            (col & 1) | ((row & 1) << 1)
        }
    }
}

/// The leaf-order state that `check_valid_local_ibc` reads.
#[derive(Clone, Copy)]
pub(crate) struct LocalAreaState<'a> {
    /// `enable_sdp && FrameIsIntra`.
    pub(crate) sdp_intra: bool,
    pub(crate) buffers: &'a IbcBuffers,
    /// `IBCCoded[row][col]` in 4x4 units.
    pub(crate) coded: &'a dyn Fn(usize, usize) -> bool,
}

/// What `check_valid_local_ibc` reads, if the caller has it.
#[derive(Clone, Copy)]
pub(crate) enum LocalArea<'a> {
    /// The leaf-order `IBCCoded` and IBC buffer state are not tracked yet:
    /// `check_valid_local_ibc` then accepts, so `is_mv_valid` accepts every
    /// vector that passes the tile-edge test while `allow_local_intrabc` is 1.
    StateNotAvailable,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the leaf-order local-area state is not wired yet")
    )]
    State(LocalAreaState<'a>),
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

/// § 6.19.5.4 `is_offset_mv_valid(dx, dy)` of the block vector `mv`
/// (`[row, col]` in 1/8 samples).
pub(crate) fn is_offset_mv_valid(
    block: &IntrabcMvBlock,
    local: LocalArea<'_>,
    mv: [i32; 2],
    (dx, dy): (i32, i32),
) -> bool {
    let offset = [
        i64::from(mv[0]) + i64::from(dy) * 8,
        i64::from(mv[1]) + i64::from(dx) * 8,
    ];
    is_mv_valid_wide(block, local, offset)
}

/// § 6.19.7.12 `is_mv_valid(mv)` for a block that codes `use_intrabc`.
pub(crate) fn is_mv_valid(block: &IntrabcMvBlock, local: LocalArea<'_>, mv: [i32; 2]) -> bool {
    is_mv_valid_wide(block, local, mv.map(i64::from))
}

fn is_mv_valid_wide(block: &IntrabcMvBlock, local: LocalArea<'_>, mv: [i64; 2]) -> bool {
    let (bw, bh) = (block.size.0 as i64, block.size.1 as i64);
    let bottom_border = i64::from(mv[0] & 7 != 0);
    let right_border = i64::from(mv[1] & 7 != 0);
    let delta_row = mv[0] >> 3;
    let delta_col = mv[1] >> 3;
    let (mi_row, mi_col) = (block.mi.0 as i64, block.mi.1 as i64);
    let mut src_top = mi_row * MI_SIZE + delta_row;
    let mut src_left = mi_col * MI_SIZE + delta_col;
    let src_bottom = src_top + bh + bottom_border;
    let src_right = src_left + bw + right_border;
    if let Some(((chroma_row, chroma_col), _)) = block.chroma {
        src_left = chroma_col as i64 * MI_SIZE + delta_col;
        src_top = chroma_row as i64 * MI_SIZE + delta_row;
    }
    let (rows, cols) = &block.tile;
    if src_top < rows.start as i64 * MI_SIZE
        || src_left < cols.start as i64 * MI_SIZE
        || src_bottom > rows.end as i64 * MI_SIZE
        || src_right > cols.end as i64 * MI_SIZE
    {
        return false;
    }
    if block.allow_local_intrabc {
        let valid = match local {
            LocalArea::StateNotAvailable => true,
            LocalArea::State(state) => {
                let chroma = block.chroma.filter(|_| !state.sdp_intra);
                let ((row, col), (w, h)) = chroma.unwrap_or((block.mi, block.size));
                let top = row as i64 * MI_SIZE + delta_row;
                let left = col as i64 * MI_SIZE + delta_col;
                let bottom = top + h as i64 - 1 + bottom_border;
                let right = left + w as i64 - 1 + right_border;
                let act = (row as i64, col as i64);
                check_valid_local_ibc(&state, act, left, top)
                    && check_valid_local_ibc(&state, act, right, bottom)
            }
        };
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
fn check_valid_local_ibc(state: &LocalAreaState<'_>, act: (i64, i64), x: i64, y: i64) -> bool {
    if x >= act.1 * MI_SIZE && y >= act.0 * MI_SIZE {
        return false;
    }
    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
        return false;
    };
    if !(state.coded)(y >> 2, x >> 2) {
        return false;
    }
    let buffers = state.buffers;
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
        if (state.coded)(colo_y >> 2, colo_x >> 2) {
            return false;
        }
    }
    true
}

/// Checks the block vector of the IntraBC leaf at `frontier` of `n4w x n4h`
/// 4x4 units in superblocks of `sb_h4` 4x4 rows: § 6.19.7.12 `is_mv_valid`
/// and, with `morph_pred`, § 6.19.5.4 `is_offset_mv_valid(-1, -1)`.
pub(crate) fn check_intrabc_block_vector(
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    (n4w, n4h): (usize, usize),
    sb_h4: usize,
    info: IntrabcInfo,
    tile_offset: ByteOffset,
) -> Result<()> {
    let tile_info = core.tile_info.as_ref().ok_or_else(selectable_state_error)?;
    let rows = tile_interval_for_block(&tile_info.mi_row_starts, frontier.r, n4h)?;
    let cols = tile_interval_for_block(&tile_info.mi_col_starts, frontier.c, n4w)?;
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
        tile: (rows.0..rows.1, cols.0..cols.1),
        sb_size: sb_h4 * 4,
        allow_local_intrabc: params.and_then(|p| p.allow_local_intrabc) != Some(false),
        allow_global_intrabc: params.and_then(|p| p.allow_global_intrabc) == Some(true),
        allow_extra_ibc_range: frontier.allow_extra_ibc_range(),
    };
    let mv = [info.block_mv.row, info.block_mv.col];
    let local = LocalArea::StateNotAvailable;
    if !is_mv_valid(&block, local, mv) {
        return Err(intrabc_invalid_block_vector_error(tile_offset));
    }
    if info.morph_pred && !is_offset_mv_valid(&block, local, mv, (-1, -1)) {
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
