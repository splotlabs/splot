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

/// What `check_valid_local_ibc` reads; `None` refuses every local source.
pub(crate) type LocalArea<'a> = Option<LocalAreaState<'a>>;

/// One § 5.20.3.1 `decode_partition` call that returns at once because it
/// lies outside the frame, in the walk order of a unit's entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutOfFrameCall {
    /// The number of the unit's entries that the walk stored before the call.
    pub(crate) entry: usize,
    /// `[r, c]` in 4x4 units.
    pub(crate) origin: [usize; 2],
    /// `Num_4x4_Blocks_Wide[ bSize ]`, `Num_4x4_Blocks_High[ bSize ]`.
    pub(crate) size4: [usize; 2],
}

/// The order of the `update_ibc_buffers` calls of one block's 64x64 chunks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChunkOrder {
    /// § 5.20.3.1, outside the frame: rows of chunks.
    Raster,
    /// § 5.20.7.23 `residual()`: 2x2 groups of chunks.
    Grouped,
}

/// The leaf-order local-area state of one tile: § 5.20.4.1 IBC buffers and
/// `IBCCoded` of the current superblock.
///
/// Only the current superblock needs `IBCCoded`: § 5.20.2.1 clears every
/// buffer at each superblock row, so a point outside the current superblock
/// row fails the buffer test of `check_valid_local_ibc`; the earlier
/// superblocks of the row are fully coded; `coloY`/`coloX` lie in the current
/// buffer.
#[derive(Clone, Debug)]
pub(crate) struct IbcLocalReplay {
    /// `enable_sdp && FrameIsIntra`.
    sdp_intra: bool,
    buffers: IbcBuffers,
    /// The origin and side (4x4 units) of the current superblock.
    sb: Option<([usize; 2], usize)>,
    /// The next entry of the unit to replay.
    next: usize,
    /// Whether entry `next` waits only for its `residual()` chunk updates.
    chunks_owed: bool,
    /// `IBCCoded` of the current superblock: bit `x` of row `y`.
    coded: [u64; 64],
}

impl IbcLocalReplay {
    pub(crate) const fn new(sdp_intra: bool) -> Self {
        Self {
            sdp_intra,
            buffers: IbcBuffers {
                sb64: false,
                cur: (0, 0),
                stored: [None; IBC_NUM_BUFFERS],
            },
            sb: None,
            next: 0,
            chunks_owed: false,
            coded: [0; 64],
        }
    }

    /// Replays the superblock at `origin` (side `sb4`, first entry `first`)
    /// in walk order through entry `leaf`, and of that entry only its
    /// `update_ibc_buffers(r, c)` and `IBCCoded` writes: the state that
    /// § 6.19.7.12 `is_mv_valid` of that leaf reads. `block` gives an entry's
    /// `[r, c, w4, h4]`; `calls` holds the unit's out-of-frame calls not yet
    /// replayed.
    #[inline(never)]
    pub(crate) fn replay_to_leaf(
        &mut self,
        superblock: ([usize; 2], usize, usize),
        leaf: usize,
        block: impl Fn(usize) -> Option<[usize; 4]>,
        calls: &mut &[OutOfFrameCall],
    ) -> Option<()> {
        self.replay_before(superblock, leaf, &block, calls)?;
        let [r, c, w4, h4] = block(leaf)?;
        self.code([r, c], [w4, h4])?;
        self.chunks_owed = true;
        Some(())
    }

    /// Brings the state to the end of the superblock, whose entries end
    /// before `end`, and drops its out-of-frame calls.
    ///
    /// Up to 128x128 only the set of 64x64 buffers that the superblock
    /// visits is visible later, as the next `update_ibc_buffers` call stores
    /// the current buffer, and each buffer has a call in it (see the test
    /// `the_superblock_summary_matches_the_leaf_replay`). A 256x256
    /// superblock has four buffers for each index, so it is replayed.
    #[inline(never)]
    pub(crate) fn finish_superblock(
        &mut self,
        superblock: ([usize; 2], usize, usize),
        end: usize,
        block: impl Fn(usize) -> Option<[usize; 4]>,
        calls: &mut &[OutOfFrameCall],
    ) -> Option<()> {
        let (origin, sb4, _) = superblock;
        if sb4 > 32 {
            return self.replay_before(superblock, end, &block, calls);
        }
        self.enter(superblock);
        for [dr, dc] in [[0, 0], [0, 16], [16, 0], [16, 16]] {
            if dr < sb4 && dc < sb4 {
                self.buffers.update(origin[0] + dr, origin[1] + dc);
            }
        }
        let done = calls.iter().take_while(|call| call.entry <= end).count();
        *calls = calls.get(done..)?;
        self.next = end;
        self.chunks_owed = false;
        Some(())
    }

    /// Makes the superblock at `origin` (side `sb4`, first entry `first`)
    /// current, if it is not.
    fn enter(&mut self, (origin, sb4, first): ([usize; 2], usize, usize)) {
        if self.sb == Some((origin, sb4)) {
            return;
        }
        if self.sb.is_none_or(|(at, _)| at[0] != origin[0]) {
            self.buffers = IbcBuffers::start_sb_row(origin[0], sb4 == 16);
        }
        self.sb = Some((origin, sb4));
        self.next = first;
        self.chunks_owed = false;
        self.coded = [0; 64];
    }

    /// Replays every call before entry `stop` that is not replayed yet.
    fn replay_before(
        &mut self,
        superblock: ([usize; 2], usize, usize),
        stop: usize,
        block: &impl Fn(usize) -> Option<[usize; 4]>,
        calls: &mut &[OutOfFrameCall],
    ) -> Option<()> {
        self.enter(superblock);
        if self.chunks_owed {
            let [r, c, w4, h4] = block(self.next)?;
            self.update_chunks([r, c], [w4, h4], ChunkOrder::Grouped);
            self.next += 1;
            self.chunks_owed = false;
        }
        while self.next < stop {
            self.out_of_frame_through(self.next, calls)?;
            let [r, c, w4, h4] = block(self.next)?;
            self.code([r, c], [w4, h4])?;
            self.update_chunks([r, c], [w4, h4], ChunkOrder::Grouped);
            self.next += 1;
        }
        self.out_of_frame_through(stop, calls)
    }

    /// Replays the out-of-frame calls that the walk made before it stored
    /// entry `entry`.
    fn out_of_frame_through(&mut self, entry: usize, calls: &mut &[OutOfFrameCall]) -> Option<()> {
        while let Some((call, rest)) = calls.split_first()
            && call.entry <= entry
        {
            self.code(call.origin, call.size4)?;
            self.update_chunks(call.origin, call.size4, ChunkOrder::Raster);
            *calls = rest;
        }
        Some(())
    }

    /// `update_ibc_buffers(r, c)` and `IBCCoded = 1` over the block.
    fn code(&mut self, [r, c]: [usize; 2], [w4, h4]: [usize; 2]) -> Option<()> {
        self.buffers.update(r, c);
        let ([r0, c0], sb4) = self.sb?;
        let (y, x) = (r.checked_sub(r0)?, c.checked_sub(c0)?);
        if y + h4 > sb4 || x + w4 > sb4 {
            return None;
        }
        let bits = u64::MAX.checked_shr(64 - w4 as u32).unwrap_or(0) << x;
        for row in self.coded.get_mut(y..y + h4)? {
            *row |= bits;
        }
        Some(())
    }

    /// The `update_ibc_buffers` calls of the block's 64x64 chunks after the
    /// first, which [`Self::code`] made.
    fn update_chunks(&mut self, origin: [usize; 2], size4: [usize; 2], order: ChunkOrder) {
        if size4[0] <= 16 && size4[1] <= 16 {
            return;
        }
        for [r, c] in chunk_origins(origin, size4, order) {
            self.buffers.update(r, c);
        }
    }

    /// `IBCCoded[row][col]` wherever `check_valid_local_ibc` depends on it:
    /// the current superblock, and the coded superblocks left of it in its
    /// row.
    fn coded(&self, row: usize, col: usize) -> bool {
        let Some(([r0, c0], sb4)) = self.sb else {
            return false;
        };
        if row < r0 || row >= r0 + sb4 {
            return false;
        }
        if col < c0 {
            return true;
        }
        let x = col - c0;
        x < sb4 && self.coded[row - r0] >> x & 1 == 1
    }
}

/// The `[miRowChunk, miColChunk]` of each 64x64 chunk of the block at
/// `[r, c]` of `[w4, h4]` 4x4 units, in `order`.
fn chunk_origins(
    [r, c]: [usize; 2],
    [w4, h4]: [usize; 2],
    order: ChunkOrder,
) -> impl Iterator<Item = [usize; 2]> {
    let (wide, high) = ((w4 >> 4).max(1), (h4 >> 4).max(1));
    let step = match order {
        ChunkOrder::Grouped => 2,
        ChunkOrder::Raster => wide.max(high),
    };
    (0..high).step_by(step).flat_map(move |start_y| {
        (0..wide).step_by(step).flat_map(move |start_x| {
            (start_y..high.min(start_y + step)).flat_map(move |y| {
                (start_x..wide.min(start_x + step)).map(move |x| [r + (y << 4), c + (x << 4)])
            })
        })
    })
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

fn is_mv_valid_wide(block: &IntrabcMvBlock, local: LocalArea<'_>, mv: [i64; 2]) -> bool {
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
        let valid = match local {
            None => false,
            Some(state) => {
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
/// 4x4 units in superblocks of `sb_h4` 4x4 rows with `AllowExtraIBCRange`:
/// § 6.19.7.12 `is_mv_valid` and, with `morph_pred`, § 6.19.5.4
/// `is_offset_mv_valid(-1, -1)`. `local` is the leaf-order state of the
/// frame's local area, present whenever `allow_local_intrabc` is 1.
#[inline(never)]
pub(crate) fn check_intrabc_block_vector(
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    (n4w, n4h): (usize, usize),
    (sb_h4, allow_extra_ibc_range): (usize, bool),
    info: IntrabcInfo,
    tile_offset: ByteOffset,
    local: Option<&IbcLocalReplay>,
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
        allow_extra_ibc_range,
    };
    if block.allow_local_intrabc && local.is_none() {
        return Err(selectable_state_error());
    }
    let coded = |row, col| local.is_some_and(|replay| replay.coded(row, col));
    let local = local.map(|replay| LocalAreaState {
        sdp_intra: replay.sdp_intra,
        buffers: &replay.buffers,
        coded: &coded,
    });
    let mv = [info.block_mv.row, info.block_mv.col];
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
