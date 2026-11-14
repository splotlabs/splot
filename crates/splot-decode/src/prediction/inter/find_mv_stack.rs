// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::DrlReorder;
pub(crate) use splot_recon::IDENTITY_WARP_PARAMS as DEFAULT_WARP_PARAMS;
use splot_recon::math::round2_signed;

use super::block::{WARP_PARAM_REDUCE_BITS, WARPEDMODEL_PREC_BITS, WARPEDMODEL_TRANS_CLAMP};
use super::{Mv, mc::CWP_EQUAL};
use crate::tile::superblock_of;

pub(crate) const MAX_REF_MV_STACK_SIZE: usize = 6;

pub(crate) const MAX_WARP_REF_CANDIDATES: usize = 4;
/// `need` of [`find_mv_stack_upto`] that builds the whole stack.
pub(crate) const ALL_CANDIDATES: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FixedStack<T, const N: usize> {
    entries: [T; N],
    len: usize,
}

impl<T: Default, const N: usize> FixedStack<T, N> {
    pub(crate) fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| T::default()),
            len: 0,
        }
    }

    pub(crate) fn from_entries(entries: impl IntoIterator<Item = T>) -> Self {
        let mut stack = Self::new();
        for entry in entries {
            if !stack.try_push(entry) {
                break;
            }
        }
        stack
    }
}

impl<T: Copy, const N: usize> FixedStack<T, N> {
    /// An empty stack whose unused slots hold `fill`.
    pub(crate) const fn filled(fill: T) -> Self {
        Self {
            entries: [fill; N],
            len: 0,
        }
    }
}

impl<T, const N: usize> FixedStack<T, N> {
    pub(crate) fn try_push(&mut self, entry: T) -> bool {
        let Some(slot) = self.entries.get_mut(self.len) else {
            return false;
        };
        *slot = entry;
        self.len += 1;
        true
    }

    fn swap_to_front(&mut self, index: usize) {
        if index < self.len {
            self.entries.swap(0, index);
        }
    }
}

impl<T, const N: usize> core::ops::Deref for FixedStack<T, N> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.entries[..self.len]
    }
}

impl<T, const N: usize> core::ops::DerefMut for FixedStack<T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries[..self.len]
    }
}

pub(crate) const TIP_REF_FRAME: i8 = 7;
pub(crate) const INTRABC_REF_FRAME: i8 = -2;

const GM_TRANS_ONLY_PREC_DIFF: u32 = WARPEDMODEL_PREC_BITS - 3;

const MV_BORDER: i32 = 128;

const MI_SIZE: i32 = 4;
mod derived;
mod extra_search;
mod neighbour_grid;
mod temporal;
mod warp_bank;
use derived::{CompoundScanState, DerivedMvState};
use extra_search::{compound_extra_search, extra_search};
use neighbour_grid::{ABSENT_NEIGHBOUR_FLAGS, NeighbourCell, NeighbourFlags, ProbedLeaf};
pub(crate) use neighbour_grid::{
    BlockPrecisionRecord, GridCheckpoint, GridEdge, NON_INTER_FLAG_SYNTAX, NeighbourFlagSyntax,
    NeighbourMotionValues, NeighbourMvGrid, ZERO_NEIGHBOUR_MOTION_VALUES, compound_motion_mode,
};
pub(crate) use temporal::{
    MotionFieldLayout, OrderHintMvContext, TemporalBandPlan, TemporalMotionBand,
    TemporalMotionBlock, TemporalMotionField, TemporalMotionFieldMetadata, TemporalMvContext,
    TemporalProjectionConfig, TipReferencePair, reference_order_hints,
    tip_reference_pair_from_hints,
};
pub(crate) use warp_bank::{WarpParamBank, WarpParamStack};

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub(crate) enum MotionMode {
    Simple,
    InterIntra,
    LocalWarp,
    DeltaWarp,
    ExtendWarp,
}

const SWITCHABLE_FILTERS: u8 = 3;
const INTER_FILTER_COMP_OFFSET: usize = SWITCHABLE_FILTERS as usize + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MvBlockContext {
    pub(crate) mi_row: usize,
    pub(crate) mi_col: usize,
    pub(crate) bw4: usize,
    pub(crate) bh4: usize,
    pub(crate) sb_h4: usize,
    pub(crate) ref_frame0: i8,
    pub(crate) ref_frame1: Option<i8>,
    pub(crate) mi_rows: usize,
    pub(crate) mi_cols: usize,
}

impl MvBlockContext {
    fn is_sb_border(&self) -> bool {
        (self.mi_row & self.sb_h4.saturating_sub(1)) == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RelativeProbe {
    delta_row: i32,
    delta_col: i32,
}

impl RelativeProbe {
    const fn new(delta_row: i32, delta_col: i32) -> Self {
        Self {
            delta_row,
            delta_col,
        }
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: keeps the probed cell in registers"
    )]
    #[inline(always)]
    fn stack_cell(
        self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
    ) -> Option<(NeighbourCell, u32, i32)> {
        let (row, col, delta_col) = self.stack_target(block);
        let zero_weight = (self.delta_row == -1 && delta_col == -1) || delta_col < -1;
        let weight = u32::from(!zero_weight);
        grid.get(row, col).map(|cell| (cell, weight, delta_col))
    }

    /// [`Self::stack_target`] of a probe whose column is already adjusted.
    fn adjusted_target(self, block: &MvBlockContext) -> (i32, i32, i32) {
        (
            block.mi_row as i32 + self.delta_row,
            block.mi_col as i32 + self.delta_col,
            self.delta_col,
        )
    }

    fn stack_target(self, block: &MvBlockContext) -> (i32, i32, i32) {
        let row = block.mi_row as i32 + self.delta_row;
        let mut col = block.mi_col as i32 + self.delta_col;
        let mut delta_col = self.delta_col;

        if self.delta_row < 0 && block.is_sb_border() {
            col = (col >> 1) << 1;
            delta_col = col - block.mi_col as i32;
        }

        (row, col, delta_col)
    }
}

/// The § 7.12.2.6 spatial probes with [`RelativeProbe::stack_target`]'s
/// column adjustment applied, so [`RelativeProbe::adjusted_target`] reads them.
fn mv_stack_spatial_probes(block: &MvBlockContext) -> [Option<RelativeProbe>; 7] {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let is_sb_border_adj = i32::from(block.is_sb_border());
    let odd_col = block.mi_col as i32 & is_sb_border_adj;
    let above = |probe: Option<RelativeProbe>| {
        probe.map(|probe| {
            RelativeProbe::new(
                -1,
                ((odd_col + probe.delta_col) & !is_sb_border_adj) - odd_col,
            )
        })
    };
    let [
        left_bottom,
        above_right,
        left_top,
        above_left,
        bottom_left,
        top_right,
        top_left,
    ] = [
        Some(RelativeProbe::new(bh4 - 1, -1)),
        Some(RelativeProbe::new(-1, (bw4 - 1 - is_sb_border_adj).max(0))),
        optional_probe(bh4 >= 2, 0, -1),
        optional_probe(bw4 >= if is_sb_border_adj == 1 { 4 } else { 2 }, -1, 0),
        optional_probe(bh4 <= 16, bh4, -1),
        optional_probe(
            bw4 <= 16,
            -1,
            if is_sb_border_adj == 1 {
                bw4.max(2)
            } else {
                bw4
            },
        ),
        Some(RelativeProbe::new(-1, -1 - is_sb_border_adj)),
    ];
    [
        left_bottom,
        above(above_right),
        left_top,
        above(above_left),
        bottom_left,
        above(top_right),
        above(top_left),
    ]
}

fn optional_probe(enabled: bool, delta_row: i32, delta_col: i32) -> Option<RelativeProbe> {
    enabled.then_some(RelativeProbe::new(delta_row, delta_col))
}

fn matches_block_ref(cell: NeighbourFlags, block: &MvBlockContext) -> bool {
    neighbour_matches_ref(cell, block.ref_frame0)
}

fn neighbour_matches_ref(cell: NeighbourFlags, ref_frame: i8) -> bool {
    cell.is_inter() && (cell.ref_frame0() == ref_frame || cell.ref_frame1() == Some(ref_frame))
}

fn mode_ctx_match_newmv(
    cell: NeighbourFlags,
    block: &MvBlockContext,
    tip_ref_pair: Option<(i8, i8)>,
) -> Option<bool> {
    if !cell.is_inter() {
        return None;
    }
    let Some(block_ref1) = block.ref_frame1 else {
        if cell.ref_frame0() == block.ref_frame0 {
            return Some(cell.newmv_for_list0());
        }
        if cell.ref_frame1() == Some(block.ref_frame0) {
            return Some(cell.newmv_for_list1());
        }
        return None;
    };
    if cell.ref_frame0() == TIP_REF_FRAME
        && cell.ref_frame1().is_none()
        && tip_ref_pair == Some((block.ref_frame0, block_ref1))
    {
        return Some(cell.newmv_for_list0());
    }
    if cell.ref_frame0() == block.ref_frame0 && cell.ref_frame1() == Some(block_ref1) {
        return Some(cell.newmv_for_list0() || cell.newmv_for_list1());
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModeContext {
    pub(crate) new_mv_context: usize,
    pub(crate) warp_mv_count: usize,
    pub(crate) warp_sample_found: bool,
    pub(crate) warp_sample_found1: bool,
    pub(crate) extend_delta: (i32, i32),
}

pub(crate) fn find_mode_ctx(grid: &NeighbourMvGrid, block: &MvBlockContext) -> ModeContext {
    find_mode_ctx_with_tip(grid, block, None)
}

pub(crate) fn find_mode_ctx_with_tip(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    tip_ref_pair: Option<(i8, i8)>,
) -> ModeContext {
    let neighbours = block_neighbour_ctx(grid, block);
    mode_ctx_from_neighbours(grid, block, tip_ref_pair, &neighbours)
}

/// [`find_mode_ctx_with_tip`] reusing the immediate neighbours that
/// [`block_neighbour_ctx`] read for the same block position.
pub(crate) fn mode_ctx_from_neighbours(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    tip_ref_pair: Option<(i8, i8)>,
    neighbours: &BlockNeighbourContext,
) -> ModeContext {
    ModeContext {
        new_mv_context: new_mv_ctx_from_neighbours(block, tip_ref_pair, neighbours),
        ..warp_ctx_from_neighbours(grid, block, neighbours)
    }
}

/// The § 7.11.2 `NewMvContext` of [`mode_ctx_from_neighbours`].
#[allow(clippy::inline_always, reason = "measured: per-leaf mode context")]
#[inline(always)]
pub(crate) fn new_mv_ctx_from_neighbours(
    block: &MvBlockContext,
    tip_ref_pair: Option<(i8, i8)>,
    neighbours: &BlockNeighbourContext,
) -> usize {
    let (mut found, mut newmv) = (0u32, false);
    for (index, &cell) in neighbours.immediate.iter().enumerate() {
        if let Some(is_newmv) = mode_ctx_match_newmv(cell, block, tip_ref_pair) {
            found |= 1 << index;
            newmv |= is_newmv;
        }
    }
    let nearest_match = usize::from(found & 0b1010 != 0) + usize::from(found & 0b0101 != 0);
    nearest_match + 2 * usize::from(newmv)
}

/// The § 7.11.2 warp outputs of [`mode_ctx_from_neighbours`]; its
/// `new_mv_context` is left 0.
#[allow(clippy::inline_always, reason = "measured: per-leaf mode context")]
#[inline(always)]
pub(crate) fn warp_ctx_from_neighbours(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    neighbours: &BlockNeighbourContext,
) -> ModeContext {
    let mut cells = neighbours.immediate;
    let is_sb_border = block.is_sb_border();
    let (bw4, bh4) = (block.bw4 as i32, block.bh4 as i32);
    let odd_col = (block.mi_col & 1) as i32;
    let (above_right_col, above_left_col) = if is_sb_border {
        ((bw4 - 2).max(0) - odd_col, -odd_col)
    } else {
        (bw4 - 1, 0)
    };
    let deltas = [
        (bh4 - 1, -1),
        (-1, above_right_col),
        (0, -1),
        (-1, above_left_col),
    ];
    if is_sb_border {
        let column = |delta: i32| block.mi_col.wrapping_add_signed(delta as isize);
        [cells[1], cells[3]] = grid.sb_border_flags(
            block.mi_col,
            block.mi_row.wrapping_sub(1),
            [column(above_right_col), column(above_left_col)],
        );
    }
    let probed = 3 + usize::from(bw4 >= if is_sb_border { 4 } else { 2 });
    let (mut samples, mut warp_mv_count, mut warp_sample_found1) = (0u32, 0, false);
    for (index, &cell) in cells.iter().enumerate().take(probed) {
        if matches_block_ref(cell, block) {
            samples |= 1 << index;
            warp_mv_count += usize::from(cell.is_warp());
        }
        warp_sample_found1 |= block
            .ref_frame1
            .is_some_and(|ref1| neighbour_matches_ref(cell, ref1));
    }
    ModeContext {
        new_mv_context: 0,
        warp_mv_count,
        warp_sample_found: samples != 0,
        warp_sample_found1,
        extend_delta: deltas
            .get(samples.trailing_zeros() as usize)
            .copied()
            .unwrap_or((0, 0)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockNeighbourContext {
    cells: [NeighbourFlags; 2],
    cell_count: usize,
    npos_cells: [NeighbourFlags; 2],
    npos_count: usize,
    /// Flags at the four [`NeighbourMvGrid::immediate_flags`] probes.
    immediate: [NeighbourFlags; 4],
}

impl BlockNeighbourContext {
    const MAX_NEIGHBOUR_REFS: usize = 7;

    pub(crate) fn is_inter_ctx(&self) -> usize {
        let intra = |n: usize| self.cell_count > n && !self.cells[n].is_inter();
        match (self.cell_count, intra(0), intra(1)) {
            (2, true, true) => 3,
            (2, first, second) => usize::from(first || second),
            (_, first, _) => 2 * usize::from(first),
        }
    }

    pub(crate) fn skip_mode_ctx(&self) -> usize {
        self.cells
            .map(|cell| usize::from(cell.skip_mode()))
            .iter()
            .sum()
    }

    pub(crate) fn skip_ctx(&self) -> usize {
        self.cells.map(|cell| usize::from(cell.skip())).iter().sum()
    }

    /// The § 8.3.2 `count_refs` of references `0..7`, one per nibble.
    pub(crate) fn ref_counts(&self) -> u32 {
        let mut ref_counts = 0u32;
        for cell in self.cells.iter().take(self.cell_count) {
            if !cell.is_inter() {
                continue;
            }
            for ref_frame in [cell.ref_frame0(), cell.ref_frame1().unwrap_or(-1)] {
                if (ref_frame as u8 as usize) < Self::MAX_NEIGHBOUR_REFS {
                    ref_counts += 1 << (4 * ref_frame as u8);
                }
            }
        }
        ref_counts
    }

    pub(crate) fn skip_mode_ref_pair(
        &self,
        default: (i8, i8),
        tip_ref_pair: Option<(i8, i8)>,
    ) -> (i8, i8) {
        for cell in self.cells.iter().take(self.cell_count) {
            if cell.ref_frame0() < 0 {
                continue;
            }
            if cell.ref_frame0() == TIP_REF_FRAME {
                return tip_ref_pair.map_or(default, |(first, second)| {
                    (first.min(second), first.max(second))
                });
            }
            if let Some(ref_frame1) = cell.ref_frame1().filter(|&ref_frame| ref_frame >= 0) {
                return (cell.ref_frame0(), ref_frame1);
            }
            break;
        }
        default
    }

    pub(crate) fn tip_mode_ctx(&self) -> usize {
        self.cells
            .iter()
            .take(self.cell_count)
            .filter(|cell| cell.ref_frame0() == TIP_REF_FRAME)
            .count()
    }

    /// The § 8.3.2 comp-mode context; `backward[r]` says whether logical
    /// reference `r` lies after the current frame.
    pub(crate) fn comp_mode_ctx(&self, backward: &[bool]) -> usize {
        let is_backward = |cell| is_backward_ref_frame(cell, backward);
        match self.cell_count {
            0 => 1,
            1 => {
                let neighbour = self.cells[0];
                if neighbour.ref_frame1().is_some() {
                    3
                } else {
                    usize::from(is_backward(neighbour))
                }
            }
            _ => {
                let first = self.cells[0];
                let second = self.cells[1];
                match (first.ref_frame1().is_some(), second.ref_frame1().is_some()) {
                    (false, false) => usize::from(is_backward(first) ^ is_backward(second)),
                    (false, true) => 2 + usize::from(is_backward(first) || !first.is_inter()),
                    (true, false) => 2 + usize::from(is_backward(second) || !second.is_inter()),
                    (true, true) => 4,
                }
            }
        }
    }

    pub(crate) fn compound_group_idx_ctx(
        &self,
        equal_ref_distance: bool,
        furthest_future_ref: Option<i8>,
    ) -> usize {
        let ctx0 = self.compound_one_ref_ctx(0, furthest_future_ref);
        let ctx1 = self.compound_one_ref_ctx(1, furthest_future_ref);
        ctx0 + ctx1 + usize::from(ctx0 != 0 && ctx1 != 0) + usize::from(equal_ref_distance) * 6
    }

    fn compound_one_ref_ctx(&self, index: usize, furthest_future_ref: Option<i8>) -> usize {
        let Some(cell) = self
            .cells
            .get(index)
            .copied()
            .filter(|_| index < self.cell_count)
        else {
            return 0;
        };
        if !cell.is_inter() {
            return 0;
        }
        if cell.ref_frame1().is_some() {
            usize::from(cell.masked_compound())
        } else if Some(cell.ref_frame0()) == furthest_future_ref {
            2
        } else {
            0
        }
    }

    pub(crate) fn interp_filter_ctx(&self, ref_frame0: i8, ref_frame1_is_inter: bool) -> usize {
        let mut neighbour_filter_type = [SWITCHABLE_FILTERS; 2];
        for (slot, cell) in self.npos_cells.iter().take(self.npos_count).enumerate() {
            if neighbour_matches_ref(*cell, ref_frame0) {
                neighbour_filter_type[slot] = cell.interp_filter();
            }
        }

        let [left_type, above_type] = neighbour_filter_type;
        let mut ctx = usize::from(ref_frame1_is_inter) * INTER_FILTER_COMP_OFFSET;
        if left_type == above_type {
            ctx += usize::from(left_type);
        } else if left_type == SWITCHABLE_FILTERS {
            ctx += usize::from(above_type);
        } else if above_type == SWITCHABLE_FILTERS {
            ctx += usize::from(left_type);
        } else {
            ctx += usize::from(SWITCHABLE_FILTERS);
        }
        ctx
    }

    pub(crate) fn amvd_ctx(&self, ref_frame0: i8) -> usize {
        self.cells
            .iter()
            .take(self.cell_count)
            .filter(|cell| cell.is_inter() && cell.ref_frame0() == ref_frame0 && cell.use_amvd())
            .count()
    }

    pub(crate) fn use_extend_warp_ctx(&self) -> usize {
        self.npos_cells
            .iter()
            .take(self.npos_count)
            .filter(|cell| cell.is_warp())
            .count()
    }

    pub(crate) fn use_local_warp_ctx(&self) -> usize {
        let cells = self.npos_cells.iter().take(self.npos_count);
        usize::from(cells.clone().any(|cell| cell.is_warp()))
            + cells.filter(|cell| cell.is_local_warp()).count()
    }

    /// The § 8.3.2 `use_intrabc` context.
    pub(crate) fn intrabc_ctx(&self) -> usize {
        self.npos_cells
            .iter()
            .take(self.npos_count)
            .filter(|cell| cell.ref_frame0() == INTRABC_REF_FRAME)
            .count()
    }

    pub(crate) fn most_probable_precision_ctx(&self) -> usize {
        self.npos_cells
            .iter()
            .take(self.npos_count)
            .filter(|cell| cell.use_most_probable_precision())
            .count()
    }

    pub(crate) fn pb_mv_precision_ctx(&self, frame_precision: u8) -> usize {
        usize::from(
            self.npos_cells
                .iter()
                .take(self.npos_count)
                .any(|cell| cell.mv_precision() < frame_precision),
        )
    }
}

/// The § 8.3.2 `single_ref` / `comp_ref` context of `ref_idx` from the
/// [`BlockNeighbourContext::ref_counts`] `counts`.
pub(crate) const fn ref_count_ctx(counts: u32, num_total_refs: usize, ref_idx: usize) -> usize {
    let refs = if num_total_refs < 7 {
        num_total_refs
    } else {
        7
    };
    let this = counts.wrapping_shr(4 * ref_idx as u32) & 0xf;
    let next = (counts & ((1 << (4 * refs)) - 1))
        .wrapping_shr(4 * ref_idx as u32 + 4)
        .wrapping_mul(0x1111_1111)
        >> 28;
    (this >= next) as usize + (this > next) as usize
}

#[allow(
    clippy::inline_always,
    reason = "measured: per-leaf neighbour contexts"
)]
#[inline(always)]
pub(crate) fn block_neighbour_ctx(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
) -> BlockNeighbourContext {
    let cells = grid.immediate_flags(block.mi_row, block.mi_col, block.bw4, block.bh4);
    let present = (0..4).fold(0u32, |mask, i| mask | u32::from(cells[i].is_present()) << i);
    let first_two = |mask: u32| {
        let rest = mask & mask.wrapping_sub(1);
        let pick = |m: u32| {
            if m == 0 {
                ABSENT_NEIGHBOUR_FLAGS
            } else {
                cells[m.trailing_zeros() as usize & 3]
            }
        };
        (
            [pick(mask), pick(rest)],
            (mask.count_ones() as usize).min(2),
        )
    };
    let (buf, num_buf) = first_two(present);
    let (npos_cells, npos_count) = first_two(if block.is_sb_border() {
        present & 0b0101
    } else {
        present
    });
    BlockNeighbourContext {
        cells: buf,
        cell_count: num_buf,
        npos_cells,
        npos_count,
        immediate: cells,
    }
}

fn is_backward_ref_frame(cell: NeighbourFlags, backward: &[bool]) -> bool {
    if !cell.is_inter() || cell.ref_frame0() < 0 {
        return false;
    }
    cell.ref_frame0() == TIP_REF_FRAME
        || backward
            .get(cell.ref_frame0() as usize)
            .copied()
            .unwrap_or(false)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MvStack {
    stack: FixedStack<(Mv, (i32, i32)), MAX_REF_MV_STACK_SIZE>,
    warp: Option<WarpParamStack>,
}

impl MvStack {
    /// § 7.12.2 zero-initializes `RefStackMv`, so an index at or past
    /// `NumMvFound` reads the zero entry.
    fn entry(&self, idx: usize) -> Option<&(Mv, (i32, i32))> {
        self.stack.get(idx)
    }

    pub(crate) fn candidate(&self, idx: usize) -> Mv {
        self.entry(idx).map_or(Mv::ZERO, |entry| entry.0)
    }

    /// The candidates in stack order.
    pub(crate) fn candidates(&self) -> impl Iterator<Item = Mv> + '_ {
        self.stack.iter().map(|&(mv, _)| mv)
    }

    pub(crate) fn candidate_offsets(&self, idx: usize) -> (i32, i32) {
        self.entry(idx).map_or((0, 0), |entry| entry.1)
    }

    pub(crate) fn warp_candidate(&self, idx: usize) -> [i32; 6] {
        self.warp
            .and_then(|warp| warp.slots.get(idx).copied())
            .unwrap_or(DEFAULT_WARP_PARAMS)
    }
}

/// AV2 § 7.12.2.2 `get_warp_motion_vector()` for a block center. The identity
/// model moves no sample, and the precision and border clamps keep zero.
pub(crate) fn warp_predicted_mv(params: [i32; 6], block: &MvBlockContext, precision: u8) -> Mv {
    if params
        .iter()
        .zip(DEFAULT_WARP_PARAMS)
        .fold(0, |diff, (&a, b)| diff | (a ^ b))
        == 0
    {
        return Mv::ZERO;
    }
    let x = block.mi_col as i32 * 4 + (block.bw4 as i32 * 4) / 2 - 1;
    let y = block.mi_row as i32 * 4 + (block.bh4 as i32 * 4) / 2 - 1;
    let (row, col) = projected_warp_mv(params, x, y);
    let (shift, scale) = if precision == super::read_mv::MV_PRECISION_EIGHTH_PEL {
        (WARPEDMODEL_PREC_BITS - 3, 1)
    } else {
        (WARPEDMODEL_PREC_BITS - 2, 2)
    };
    let row = round2_signed(row, shift) * scale;
    let col = round2_signed(col, shift) * scale;
    let mv = clip_and_clamp_projected_mv(block, row, col);
    if precision < super::read_mv::MV_PRECISION_HALF_PEL {
        super::read_mv::lower_mv_precision(precision, mv)
    } else {
        mv
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompoundMvCandidate {
    pub(crate) mvs: [Mv; 2],
    pub(crate) cwp_weight: i16,
}

impl Default for CompoundMvCandidate {
    fn default() -> Self {
        Self {
            mvs: [Mv::ZERO; 2],
            cwp_weight: CWP_EQUAL,
        }
    }
}

pub(crate) struct CompoundMvStack {
    stack: FixedStack<CompoundMvCandidate, MAX_REF_MV_STACK_SIZE>,
}

impl CompoundMvStack {
    #[cfg(test)]
    pub(crate) fn from_candidates(stack: Vec<CompoundMvCandidate>) -> Self {
        Self {
            stack: FixedStack::from_entries(stack),
        }
    }

    pub(crate) fn candidate(&self, idx: usize) -> CompoundMvCandidate {
        self.stack.get(idx).copied().unwrap_or_default()
    }
}

pub(crate) fn extend_warp_neighbour_params(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_row: i32,
    delta_col: i32,
    global_warp_params: [i32; 6],
) -> Option<[i32; 6]> {
    let cell = grid.get(
        block.mi_row as i32 + delta_row,
        block.mi_col as i32 + delta_col,
    )?;
    let list = if cell.flags.ref_frame0() == block.ref_frame0 {
        0
    } else if cell.flags.ref_frame1() == Some(block.ref_frame0) {
        1
    } else {
        return None;
    };
    if cell.flags.is_warp() {
        return cell.motion.warp_params();
    }
    if cell.motion.is_global_mv(list) {
        return Some(global_warp_params);
    }
    let mv = if list == 0 {
        cell.motion.mv
    } else {
        cell.motion.mv1
    };
    let mut params = splot_recon::IDENTITY_WARP_PARAMS;
    let [Some(translation_x), Some(translation_y)] =
        [mv.col, mv.row].map(|value| value.checked_shl(WARPEDMODEL_PREC_BITS - 3))
    else {
        return None;
    };
    params[0] = translation_x;
    params[1] = translation_y;
    Some(params)
}

const LEAST_SQUARES_SAMPLES_MAX: usize = 8;

pub(crate) fn find_warp_samples(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    target_ref: i8,
) -> FixedStack<[i32; 4], LEAST_SQUARES_SAMPLES_MAX> {
    let mut samples = FixedStack::new();
    let mi_row = block.mi_row as i32;
    let mi_col = block.mi_col as i32;
    let w4 = block.bw4 as i32;
    let h4 = block.bh4 as i32;
    let mi_rows = block.mi_rows as i32;
    let mi_cols = block.mi_cols as i32;
    let add_sample = |samples: &mut FixedStack<[i32; 4], LEAST_SQUARES_SAMPLES_MAX>,
                      delta_row: i32,
                      delta_col: i32| {
        if samples.len() >= LEAST_SQUARES_SAMPLES_MAX {
            return;
        }
        let Some(cell) = grid.get(mi_row + delta_row, mi_col + delta_col) else {
            return;
        };
        let lists = [
            (cell.flags.ref_frame0() == target_ref && cell.flags.is_inter())
                .then_some(cell.motion.mv),
            (cell.flags.ref_frame1() == Some(target_ref)).then_some(cell.motion.mv1),
        ];
        for mv in lists.into_iter().flatten() {
            if samples.len() >= LEAST_SQUARES_SAMPLES_MAX {
                return;
            }
            let mid_y = (cell.motion.base_r * 4 + u32::from(cell.motion.bh4) * 2) as i32 - 1;
            let mid_x = (cell.motion.base_c * 4 + u32::from(cell.motion.bw4) * 2) as i32 - 1;
            let _ =
                samples.try_push([mid_y * 8, mid_x * 8, mid_y * 8 + mv.row, mid_x * 8 + mv.col]);
        }
    };
    let above_sample_stored = |delta_col: i32| -> bool {
        let col = mi_col + delta_col;
        if mi_row < 1 || col < 0 || col >= mi_cols {
            return false;
        }
        let sb_mask = block.sb_h4 as i32 - 1;
        if (mi_row & sb_mask) != 0 {
            return true;
        }
        if col % 2 == 0 {
            return true;
        }
        let src_w4 = grid
            .get(mi_row - 1, col)
            .map_or(0, |cell| cell.motion.bw4 as i32);
        if src_w4 == 1 {
            return false;
        }
        col + 1 < mi_cols
    };
    let mut do_top_left = true;
    let mut do_top_right = true;
    if mi_row > 0 {
        let col_offset = grid
            .get(mi_row - 1, mi_col)
            .map_or(0, |cell| cell.motion.base_c as i32 - mi_col);
        if col_offset < 0 {
            do_top_left = false;
        }
        let mut i = col_offset;
        let limit = w4.min(mi_cols - mi_col);
        while i < limit {
            let src_w = grid
                .get(mi_row - 1, mi_col + i)
                .map_or(1, |cell| (cell.motion.bw4 as i32).max(1));
            if above_sample_stored(i) {
                add_sample(&mut samples, -1, i);
            }
            i += src_w;
        }
        do_top_right = i == w4 && i < (mi_cols - mi_col);
    }
    if mi_col > 0 {
        let row_offset = grid
            .get(mi_row, mi_col - 1)
            .map_or(0, |cell| cell.motion.base_r as i32 - mi_row);
        if row_offset < 0 {
            do_top_left = false;
        }
        let mut i = row_offset;
        let limit = h4.min(mi_rows - mi_row);
        while i < limit {
            let src_h = grid
                .get(mi_row + i, mi_col - 1)
                .map_or(1, |cell| (cell.motion.bh4 as i32).max(1));
            add_sample(&mut samples, i, -1);
            i += src_h;
        }
    }
    if do_top_left && above_sample_stored(-1) {
        add_sample(&mut samples, -1, -1);
    }
    if do_top_right && w4 <= 16 && above_sample_stored(w4) {
        add_sample(&mut samples, -1, w4);
    }
    samples
}

const REF_MV_BANK_SIZE: usize = 4;
const BANK_REFS_PER_FRAME: i32 = 9;
const MAX_RMB_SB_HITS: u32 = 64;
const MAX_PR_NUM: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RefMvBankEntry {
    key: i32,
    mv0: Mv,
    mv1: Mv,
    cwp_weight: i16,
}

impl Default for RefMvBankEntry {
    fn default() -> Self {
        Self {
            key: 0,
            mv0: Mv::ZERO,
            mv1: Mv::ZERO,
            cwp_weight: CWP_EQUAL,
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct RefMvBank {
    entries: [[RefMvBankEntry; REF_MV_BANK_SIZE]; BANK_REFS_PER_FRAME as usize],
    sizes: [usize; BANK_REFS_PER_FRAME as usize],
    starts: [usize; BANK_REFS_PER_FRAME as usize],
    sb_hits: u32,
    remain_hits: i32,
    unit_hits: i32,
    current_sb: Option<(usize, usize)>,
}

impl RefMvBank {
    pub(crate) fn new() -> Self {
        Self {
            entries: [[RefMvBankEntry::default(); REF_MV_BANK_SIZE]; BANK_REFS_PER_FRAME as usize],
            sizes: [0; BANK_REFS_PER_FRAME as usize],
            starts: [0; BANK_REFS_PER_FRAME as usize],
            sb_hits: 0,
            remain_hits: 0,
            unit_hits: 0,
            current_sb: None,
        }
    }

    fn list_index(ref_frame0: i8, ref_frame1: Option<i8>) -> usize {
        match (ref_frame0, ref_frame1) {
            (r0, None) if (0..=5).contains(&r0) => r0 as usize,
            (0, Some(0)) => 6,
            (0, Some(1)) => 7,
            _ => 8,
        }
    }

    fn bank_key(ref_frame0: i8, ref_frame1: Option<i8>) -> i32 {
        match ref_frame1 {
            Some(r1) => i32::from(ref_frame0) + (i32::from(r1) + 1) * BANK_REFS_PER_FRAME,
            None => i32::from(ref_frame0),
        }
    }

    #[inline]
    pub(crate) fn reset_for_leaf(
        &mut self,
        grid: &NeighbourMvGrid,
        mi_row: usize,
        mi_col: usize,
        sb_size4: usize,
        seed_from_above: bool,
    ) {
        let sb = superblock_of(mi_row, mi_col, sb_size4);
        if self.current_sb != Some(sb) {
            self.enter_superblock(grid, sb, sb_size4, seed_from_above);
        }
    }

    fn enter_superblock(
        &mut self,
        grid: &NeighbourMvGrid,
        sb: (usize, usize),
        sb_size4: usize,
        seed_from_above: bool,
    ) {
        let new_sb_row = self.current_sb.is_none_or(|(row, _)| row != sb.0);
        self.current_sb = Some(sb);
        self.sb_hits = 0;
        self.remain_hits = 0;
        self.unit_hits = 0;
        if new_sb_row {
            for size in &mut self.sizes {
                *size = 0;
            }
            for start in &mut self.starts {
                *start = 0;
            }
        }
        if seed_from_above {
            seed_walk_from_row_above(grid, sb.0 * sb_size4, sb.1 * sb_size4, sb_size4, |cell| {
                self.update(
                    cell.flags.ref_frame0(),
                    cell.flags.ref_frame1(),
                    cell.motion.mv,
                    cell.flags.ref_frame1().map(|_| cell.motion.mv1),
                    cell.motion.cwp_weight,
                    false,
                );
            });
        }
    }

    fn update_unit_budget(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        sb_size4: usize,
    ) {
        let unit_shift = (sb_size4 >> 3).max(1).trailing_zeros();
        let unit_count = ((n4w >> unit_shift).max(1) * (n4h >> unit_shift).max(1)) as i32;
        if (mi_row | mi_col) & sb_size4.wrapping_sub(1) == 0 {
            self.remain_hits = unit_count.max(4);
            self.unit_hits = 0;
        } else if (mi_row | mi_col) & ((1 << unit_shift) - 1) == 0 {
            self.remain_hits += unit_count;
            self.unit_hits = 0;
        }
    }

    pub(crate) fn update_count_for_non_inter(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        sb_size4: usize,
    ) {
        if self.sb_hits >= MAX_RMB_SB_HITS {
            return;
        }
        self.update_unit_budget(mi_row, mi_col, n4w, n4h, sb_size4);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_for_block(
        &mut self,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        mv: Mv,
        mv1: Option<Mv>,
        cwp_weight: i16,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        sb_size4: usize,
    ) {
        if self.sb_hits >= MAX_RMB_SB_HITS {
            return;
        }
        self.update_unit_budget(mi_row, mi_col, n4w, n4h, sb_size4);
        if self.remain_hits == 0 || self.unit_hits >= 16 {
            return;
        }
        self.remain_hits -= 1;
        self.unit_hits += 1;
        self.update(ref_frame0, ref_frame1, mv, mv1, cwp_weight, true);
    }

    fn update(
        &mut self,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        mv: Mv,
        mv1: Option<Mv>,
        cwp_weight: i16,
        from_within_sb: bool,
    ) {
        if from_within_sb {
            self.sb_hits += 1;
        } else {
            self.sb_hits = self.sb_hits.saturating_add(1);
        }
        let list = Self::list_index(ref_frame0, ref_frame1);
        let entry = RefMvBankEntry {
            key: Self::bank_key(ref_frame0, ref_frame1),
            mv0: mv,
            mv1: mv1.unwrap_or(Mv::ZERO),
            cwp_weight,
        };
        bank_ring_update(
            &mut self.entries[list],
            &mut self.sizes[list],
            &mut self.starts[list],
            entry,
            |candidate| {
                candidate.key == entry.key
                    && candidate.mv0 == entry.mv0
                    && candidate.mv1 == entry.mv1
            },
        );
    }

    fn fill(
        &self,
        block: &MvBlockContext,
        entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
        max_ref_mv_count: usize,
        prune_count: &mut usize,
    ) {
        let list = Self::list_index(block.ref_frame0, block.ref_frame1);
        let key = Self::bank_key(block.ref_frame0, block.ref_frame1);
        let count = self.sizes[list];
        let start = self.starts[list];
        for i in (0..count).rev() {
            if entries.len() >= max_ref_mv_count {
                return;
            }
            let idx = (start + i) % REF_MV_BANK_SIZE;
            let candidate = self.entries[list][idx];
            if candidate.key != key {
                continue;
            }
            let mut duplicate = false;
            if *prune_count < MAX_PR_NUM {
                for entry in entries.iter() {
                    *prune_count += 1;
                    if entry.mv == candidate.mv0 {
                        duplicate = true;
                        break;
                    }
                }
            }
            if duplicate {
                continue;
            }
            let bw = block.bw4 as i32 * MI_SIZE;
            let bh = block.bh4 as i32 * MI_SIZE;
            let ref_y = block.mi_row as i32 * MI_SIZE + candidate.mv0.row / 8;
            let ref_x = block.mi_col as i32 * MI_SIZE + candidate.mv0.col / 8;
            if ref_x <= -bw
                || ref_y <= -bh
                || ref_x >= block.mi_cols as i32 * MI_SIZE
                || ref_y >= block.mi_rows as i32 * MI_SIZE
            {
                continue;
            }
            if !entries.try_push(MvStackEntry {
                mv: candidate.mv0,
                weight: 0,
                offsets: (0, 0),
            }) {
                return;
            }
        }
    }

    pub(crate) fn intrabc_candidates(&self) -> FixedStack<Mv, REF_MV_BANK_SIZE> {
        let list = Self::list_index(INTRABC_REF_FRAME, None);
        let key = Self::bank_key(INTRABC_REF_FRAME, None);
        let count = self.sizes[list];
        let start = self.starts[list];
        let mut candidates = FixedStack::new();
        for i in (0..count).rev() {
            let candidate = self.entries[list][(start + i) % REF_MV_BANK_SIZE];
            if candidate.key != key {
                continue;
            }
            if !candidates.try_push(candidate.mv0) {
                break;
            }
        }
        candidates
    }

    fn fill_compound(
        &self,
        block: &MvBlockContext,
        entries: &mut FixedStack<CompoundMvStackEntry, MAX_REF_MV_STACK_SIZE>,
        max_ref_mv_count: usize,
        prune_count: &mut usize,
    ) {
        let list = Self::list_index(block.ref_frame0, block.ref_frame1);
        let key = Self::bank_key(block.ref_frame0, block.ref_frame1);
        let count = self.sizes[list];
        let start = self.starts[list];
        for i in (0..count).rev() {
            if entries.len() >= max_ref_mv_count {
                return;
            }
            let candidate = self.entries[list][(start + i) % REF_MV_BANK_SIZE];
            if candidate.key != key {
                continue;
            }
            let mvs = [candidate.mv0, candidate.mv1];
            let mut duplicate = false;
            if *prune_count < MAX_PR_NUM {
                for entry in entries.iter() {
                    *prune_count += 1;
                    if entry.candidate.mvs == mvs {
                        duplicate = true;
                        break;
                    }
                }
            }
            if duplicate {
                continue;
            }
            let bw = block.bw4 as i32 * MI_SIZE;
            let bh = block.bh4 as i32 * MI_SIZE;
            if mvs.into_iter().any(|mv| {
                let ref_y = block.mi_row as i32 * MI_SIZE + mv.row / 8;
                let ref_x = block.mi_col as i32 * MI_SIZE + mv.col / 8;
                ref_x <= -bw
                    || ref_y <= -bh
                    || ref_x >= block.mi_cols as i32 * MI_SIZE
                    || ref_y >= block.mi_rows as i32 * MI_SIZE
            }) {
                continue;
            }
            if !entries.try_push(CompoundMvStackEntry {
                candidate: CompoundMvCandidate {
                    mvs,
                    cwp_weight: candidate.cwp_weight,
                },
                weight: 0,
            }) {
                return;
            }
        }
    }
}

fn seed_walk_from_row_above(
    grid: &NeighbourMvGrid,
    sb_row: usize,
    sb_col: usize,
    sb_size4: usize,
    mut visit: impl FnMut(&NeighbourCell),
) {
    if sb_row == 0 {
        return;
    }
    let cand_row = sb_row as i32 - 1;
    let mut cand_col = sb_col as i32;
    let mut row_hits = 0;
    while (cand_col as usize) < grid.origin_col.saturating_add(grid.mi_cols)
        && (cand_col as usize) < sb_col + sb_size4
        && row_hits < 4
    {
        let cand_col2 = (cand_col >> 1) << 1;
        let mut step = 1i32;
        if let Some(cell) = grid.get(cand_row, cand_col2) {
            if cell.flags.is_inter() {
                row_hits += 1;
                visit(&cell);
            }
            step = (cell.motion.bw4 as i32).max(1);
        }
        cand_col += step;
    }
}

fn bank_ring_update<T: Copy>(
    entries: &mut [T],
    size: &mut usize,
    start: &mut usize,
    entry: T,
    matches: impl Fn(&T) -> bool,
) {
    let capacity = entries.len();
    let count = *size;
    let mut found = None;
    for i in 0..count {
        let idx = (*start + i) % capacity;
        if matches(&entries[idx]) {
            found = Some(i);
            break;
        }
    }
    if let Some(found) = found {
        let kept = entries[(*start + found) % capacity];
        for i in found..count.saturating_sub(1) {
            let idx0 = (*start + i) % capacity;
            let idx1 = (*start + i + 1) % capacity;
            entries[idx0] = entries[idx1];
        }
        let tail = (*start + count - 1) % capacity;
        entries[tail] = kept;
    } else if count < capacity {
        let tail = (*start + count) % capacity;
        entries[tail] = entry;
        *size = count + 1;
    } else {
        entries[*start] = entry;
        *start = (*start + 1) % capacity;
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn find_mv_stack(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mv: Mv,
    bank: Option<(&RefMvBank, usize)>,
    warp_bank: &WarpParamBank,
    derive_wrl: bool,
    drl_reorder: DrlReorder,
    use_temporal_first: bool,
) -> MvStack {
    find_mv_stack_with_temporal(
        grid,
        block,
        global_mv,
        DEFAULT_WARP_PARAMS,
        bank,
        warp_bank,
        derive_wrl,
        drl_reorder,
        None,
        None,
        use_temporal_first,
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn find_mv_stack_with_temporal(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mv: Mv,
    global_warp_params: [i32; 6],
    bank: Option<(&RefMvBank, usize)>,
    warp_bank: &WarpParamBank,
    derive_wrl: bool,
    drl_reorder: DrlReorder,
    temporal: Option<&TemporalMvContext>,
    order_hints: Option<OrderHintMvContext<'_>>,
    use_temporal_first: bool,
) -> MvStack {
    find_mv_stack_upto(
        grid,
        block,
        global_mv,
        global_warp_params,
        bank,
        warp_bank,
        derive_wrl,
        drl_reorder,
        temporal,
        order_hints,
        use_temporal_first,
        ALL_CANDIDATES,
    )
}

/// The § 7.12.2 stack up to index `need`, which is all a caller may read: the
/// bank, derived and extra candidates only append after the sort, so they are
/// skipped once the spatial and temporal scans alone reach `need`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_mv_stack_upto(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mv: Mv,
    global_warp_params: [i32; 6],
    bank: Option<(&RefMvBank, usize)>,
    warp_bank: &WarpParamBank,
    derive_wrl: bool,
    drl_reorder: DrlReorder,
    temporal: Option<&TemporalMvContext>,
    order_hints: Option<OrderHintMvContext<'_>>,
    use_temporal_first: bool,
    need: usize,
) -> MvStack {
    let mut entries = FixedStack::new();
    let mut prune_count = 0usize;
    let mut derived = DerivedMvState::new(temporal, order_hints, global_mv);
    let mut warp = derive_wrl.then(WarpParamStack::new);
    if let Some(warp) = warp.as_mut() {
        generate_points_from_corners(grid, block, warp);
    }
    if use_temporal_first {
        scan_temporal_mv_stack(grid, block, temporal, &mut entries, &mut prune_count);
    }

    let probes = mv_stack_spatial_probes(block);
    let probed = |probe: RelativeProbe| {
        let (row, col, _) = probe.adjusted_target(block);
        Some((probe, grid.probe(row, col)?))
    };
    for (probe, leaf) in probes[..6]
        .iter()
        .flatten()
        .filter_map(|&probe| probed(probe))
    {
        scan_mv_stack_probe(
            grid,
            block,
            probe,
            leaf,
            &mut entries,
            &mut prune_count,
            &mut derived,
            warp.as_mut(),
        );
    }
    if !use_temporal_first {
        scan_temporal_mv_stack(grid, block, temporal, &mut entries, &mut prune_count);
    }
    if let Some((probe, leaf)) = probes[6].and_then(probed) {
        scan_mv_stack_probe(
            grid,
            block,
            probe,
            leaf,
            &mut entries,
            &mut prune_count,
            &mut derived,
            warp.as_mut(),
        );
    }

    let num_nearest = entries.len();
    if num_nearest <= need || warp.is_some() {
        scan_mv_stack_col(
            grid,
            block,
            -3,
            &mut entries,
            &mut prune_count,
            &mut derived,
            warp.as_mut(),
        );
    }
    let use_sort = match drl_reorder {
        DrlReorder::Always => true,
        DrlReorder::Constraint => !use_temporal_first && num_nearest >= 4,
        DrlReorder::Disabled => false,
    };
    if use_sort && num_nearest > 1 {
        let max_idx = entries
            .iter()
            .take(num_nearest)
            .enumerate()
            .max_by_key(|(idx, entry)| (entry.weight, core::cmp::Reverse(*idx)))
            .map_or(0, |(idx, _)| idx);
        if max_idx != 0 {
            entries.swap_to_front(max_idx);
        }
    }
    if entries.len() <= need {
        let max_ref_mv_count = bank.map_or(MAX_REF_MV_STACK_SIZE, |(_, count)| count);
        if let Some((bank, _)) = bank {
            bank.fill(block, &mut entries, max_ref_mv_count, &mut prune_count);
        }
        derived.fill(block, &mut entries, max_ref_mv_count, &mut prune_count);
        extra_search(block, global_mv, &mut entries, &mut prune_count);
    }
    if let Some(warp) = warp.as_mut() {
        warp_bank.fill(block.ref_frame0, warp);
        warp.insert(global_warp_params);
        for _ in 0..2 {
            warp.insert(DEFAULT_WARP_PARAMS);
        }
    }

    let stack = FixedStack::from_entries(
        entries
            .iter()
            .take(need.saturating_add(1))
            .map(|entry| (clamp_mv(block, entry.mv), entry.offsets)),
    );

    MvStack { stack, warp }
}

#[cfg(test)]
pub(crate) fn find_compound_mv_stack_with_temporal(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mvs: [Mv; 2],
    bank: Option<(&RefMvBank, usize)>,
    drl_reorder: DrlReorder,
    temporal: Option<&TemporalMvContext>,
) -> CompoundMvStack {
    find_compound_mv_stack_upto(
        grid,
        block,
        global_mvs,
        bank,
        drl_reorder,
        temporal,
        ALL_CANDIDATES,
    )
}

/// [`find_mv_stack_upto`] for a compound reference pair.
pub(crate) fn find_compound_mv_stack_upto(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mvs: [Mv; 2],
    bank: Option<(&RefMvBank, usize)>,
    drl_reorder: DrlReorder,
    temporal: Option<&TemporalMvContext>,
    need: usize,
) -> CompoundMvStack {
    let mut state = CompoundScanState::new(global_mvs);
    let probes = mv_stack_spatial_probes(block);
    for probe in probes.iter().take(6).copied().flatten() {
        scan_compound_mv_stack_probe(grid, block, probe, temporal, &mut state);
    }
    scan_compound_temporal_mv_stack(grid, block, temporal, &mut state);
    if let Some(probe) = probes[6] {
        scan_compound_mv_stack_probe(grid, block, probe, temporal, &mut state);
    }

    let num_nearest = state.entries.len();
    if num_nearest <= need {
        scan_compound_mv_stack_col(grid, block, -3, temporal, &mut state);
    }
    let use_sort = match drl_reorder {
        DrlReorder::Always => true,
        DrlReorder::Constraint => num_nearest >= 4,
        DrlReorder::Disabled => false,
    };
    if use_sort && num_nearest > 1 {
        let max_idx = state
            .entries
            .iter()
            .take(num_nearest)
            .enumerate()
            .max_by_key(|(idx, entry)| (entry.weight, core::cmp::Reverse(*idx)))
            .map_or(0, |(idx, _)| idx);
        if max_idx != 0 {
            state.entries.swap_to_front(max_idx);
        }
    }
    if state.entries.len() <= need {
        let max_ref_mv_count = bank.map_or(MAX_REF_MV_STACK_SIZE, |(_, count)| count);
        state.derived.fill(
            block,
            state.global_mvs,
            temporal,
            &mut state.entries,
            max_ref_mv_count,
            &mut state.prune_count,
        );
        if let Some((bank, _)) = bank {
            bank.fill_compound(
                block,
                &mut state.entries,
                max_ref_mv_count,
                &mut state.prune_count,
            );
        }
        compound_extra_search(
            block,
            global_mvs,
            &mut state.entries,
            &mut state.prune_count,
        );
    }
    let entries = state.entries.iter().take(need.saturating_add(1));
    let stack = FixedStack::from_entries(entries.map(|entry| CompoundMvCandidate {
        mvs: [
            clamp_mv(block, entry.candidate.mvs[0]),
            clamp_mv(block, entry.candidate.mvs[1]),
        ],
        cwp_weight: entry.candidate.cwp_weight,
    }));
    CompoundMvStack { stack }
}

#[derive(Clone, Copy, Debug, Default)]
struct CompoundMvStackEntry {
    candidate: CompoundMvCandidate,
    weight: u32,
}

fn scan_compound_mv_stack_col(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_col: i32,
    temporal: Option<&TemporalMvContext>,
    state: &mut CompoundScanState,
) {
    let delta_col = delta_col + i32::from(block.bw4 == 1 && block.mi_col & 1 == 1);
    let bh4 = block.bh4 as i32;
    for delta_row in [Some(bh4 - 1), (bh4 > 1).then_some(0)]
        .into_iter()
        .flatten()
    {
        let mv_row = block.mi_row as i32 + delta_row;
        let mv_col = block.mi_col as i32 + delta_col;
        let Some(cell_base_c) = grid.base_c_at(mv_row, mv_col) else {
            continue;
        };
        let Some(left_base_c) = grid.base_c_at(mv_row, block.mi_col as i32 - 1) else {
            continue;
        };
        if cell_base_c != left_base_c {
            scan_compound_mv_stack_probe(
                grid,
                block,
                RelativeProbe::new(delta_row, delta_col),
                temporal,
                state,
            );
        }
    }
}

fn scan_compound_mv_stack_probe(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    probe: RelativeProbe,
    temporal: Option<&TemporalMvContext>,
    state: &mut CompoundScanState,
) {
    let Some(ref_frame1) = block.ref_frame1 else {
        return;
    };
    let (row, col, delta_col) = probe.adjusted_target(block);
    let Some(leaf) = grid.probe(row, col) else {
        return;
    };
    let flags = leaf.flags();
    if !flags.is_inter() {
        return;
    }
    let (ref0, ref1) = (flags.ref_frame0(), flags.ref_frame1());
    let zero_weight = (probe.delta_row == -1 && delta_col == -1) || delta_col < -1;
    let weight = u32::from(!zero_weight);
    let candidate = if ref0 == block.ref_frame0 && ref1 == Some(ref_frame1) {
        CompoundMvCandidate {
            mvs: core::array::from_fn(|list| {
                if leaf.is_global_mv(list) {
                    state.global_mvs[list]
                } else {
                    leaf.sub_mv(list)
                }
            }),
            cwp_weight: leaf.cwp_weight(),
        }
    } else if ref0 == TIP_REF_FRAME {
        let Some(temporal) = temporal else {
            return;
        };
        let Some(tip_refs) = temporal.tip_references() else {
            return;
        };
        if ref1.is_some()
            || (tip_refs.past_ref, tip_refs.future_ref) != (block.ref_frame0, ref_frame1)
        {
            return;
        }
        let Some(mvs) = grid
            .get(row, col)
            .and_then(|cell| temporal.tip_spatial_mvs(grid, block, probe, &cell))
        else {
            return;
        };
        CompoundMvCandidate {
            mvs,
            cwp_weight: CWP_EQUAL,
        }
    } else {
        let sub_mvs = leaf.sub_mvs();
        state.derived.add_spatial(
            block,
            [Some((ref0, sub_mvs[0])), ref1.map(|r| (r, sub_mvs[1]))],
            [leaf.is_global_mv(0), leaf.is_global_mv(1)],
            state.global_mvs,
            temporal,
        );
        return;
    };
    insert_compound_mv_stack_entry(
        &mut state.entries,
        &mut state.prune_count,
        candidate,
        weight,
    );
}

fn scan_compound_temporal_mv_stack(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    temporal: Option<&TemporalMvContext>,
    state: &mut CompoundScanState,
) {
    let (Some(temporal), Some(ref_frame1)) = (temporal, block.ref_frame1) else {
        return;
    };
    let row_end = block.bh4.min(16);
    let col_end = block.bw4.min(16);
    let step_h4 = if block.bh4 >= 16 { 4 } else { 2 };
    let step_w4 = if block.bw4 >= 16 { 4 } else { 2 };
    let samples = [
        (row_end >= step_h4 && col_end >= step_w4).then(|| (row_end - step_h4, col_end - step_w4)),
        (row_end >= 3 * step_h4 || col_end >= 3 * step_w4).then_some((row_end >> 1, col_end >> 1)),
    ];
    for (delta_row, delta_col) in samples.into_iter().flatten() {
        let mv_row = block.mi_row.saturating_add(delta_row);
        let mv_col = block.mi_col.saturating_add(delta_col);
        if !grid.is_inside(mv_row, mv_col) {
            continue;
        }
        let Some(mv0) = temporal.motion_field_mv(block.ref_frame0, mv_row >> 1, mv_col >> 1) else {
            continue;
        };
        let Some(mv1) = temporal.motion_field_mv(ref_frame1, mv_row >> 1, mv_col >> 1) else {
            continue;
        };
        if insert_compound_mv_stack_entry(
            &mut state.entries,
            &mut state.prune_count,
            CompoundMvCandidate {
                mvs: [mv0, mv1],
                cwp_weight: CWP_EQUAL,
            },
            1,
        ) == StackInsert::Inserted
        {
            break;
        }
    }
}

fn insert_compound_mv_stack_entry(
    entries: &mut FixedStack<CompoundMvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    candidate: CompoundMvCandidate,
    weight: u32,
) -> StackInsert {
    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        return StackInsert::Skipped;
    }
    if *prune_count < MAX_PR_NUM {
        for entry in entries.iter_mut() {
            *prune_count += 1;
            if entry.candidate.mvs == candidate.mvs {
                entry.weight = entry.weight.saturating_add(weight);
                return StackInsert::Updated;
            }
        }
    }
    if entries.try_push(CompoundMvStackEntry { candidate, weight }) {
        StackInsert::Inserted
    } else {
        StackInsert::Skipped
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct MvStackEntry {
    mv: Mv,
    weight: u32,
    offsets: (i32, i32),
}

fn scan_mv_stack_col(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_col: i32,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    derived: &mut DerivedMvState<'_>,
    mut warp: Option<&mut WarpParamStack>,
) {
    let delta_col = delta_col + i32::from(block.bw4 == 1 && block.mi_col & 1 == 1);
    let bh4 = block.bh4 as i32;
    for delta_row in [Some(bh4 - 1), (bh4 > 1).then_some(0)]
        .into_iter()
        .flatten()
    {
        let mv_row = block.mi_row as i32 + delta_row;
        let mv_col = block.mi_col as i32 + delta_col;
        let Some(leaf) = grid.probe(mv_row, mv_col) else {
            continue;
        };
        let Some(left_base_c) = grid.base_c_at(mv_row, block.mi_col as i32 - 1) else {
            continue;
        };
        if leaf.base_c() == left_base_c {
            continue;
        }
        scan_mv_stack_probe(
            grid,
            block,
            RelativeProbe::new(delta_row, delta_col),
            leaf,
            entries,
            prune_count,
            derived,
            warp.as_deref_mut(),
        );
    }
}

#[allow(
    clippy::inline_always,
    clippy::too_many_arguments,
    reason = "measured: keeps the stack state in registers across probes"
)]
#[inline(always)]
fn scan_mv_stack_probe(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    probe: RelativeProbe,
    leaf: ProbedLeaf<'_>,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    derived: &mut DerivedMvState<'_>,
    warp: Option<&mut WarpParamStack>,
) {
    let adjusted_delta_col = probe.delta_col;
    let flags = leaf.flags();
    if let Some(warp) = warp
        && flags.is_inter()
        && flags.is_warp()
        && flags.ref_frame0() == block.ref_frame0
        && let Some(params) = leaf.stored_warp()
    {
        warp.insert(params);
    }

    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        return;
    }

    let zero_weight =
        (probe.delta_row == -1 && adjusted_delta_col == -1) || adjusted_delta_col < -1;
    let weight = u32::from(!zero_weight);
    let offsets = (probe.delta_row, adjusted_delta_col);
    if flags.ref_frame0() == TIP_REF_FRAME
        && flags.ref_frame1().is_none()
        && block.ref_frame0 != TIP_REF_FRAME
    {
        scan_tip_spatial_probe(grid, block, probe, entries, prune_count, derived, weight);
    }
    let lists = if flags.ref_frame1().is_some() { 2 } else { 1 };
    for list in 0..lists {
        let candidate_ref = flags.list_ref_frame(list);
        if candidate_ref == block.ref_frame0 {
            let candidate_mv = if leaf.is_global_mv(list) {
                derived.global_mv()
            } else {
                leaf.sub_mv(list)
            };
            insert_mv_stack_entry(entries, prune_count, candidate_mv, weight, offsets);
        } else if candidate_ref >= 0 && candidate_ref != TIP_REF_FRAME {
            derived.add_spatial(block, candidate_ref, leaf.sub_mv(list), flags, || {
                leaf.sub_mvs()
            });
        }
    }
}

/// The § 7.12.2.15 candidates of a TIP neighbour of a non-TIP block.
#[inline(never)]
fn scan_tip_spatial_probe(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    probe: RelativeProbe,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    derived: &mut DerivedMvState<'_>,
    weight: u32,
) {
    let Some((cell, _, adjusted_delta_col)) = probe.stack_cell(grid, block) else {
        return;
    };
    let offsets = (probe.delta_row, adjusted_delta_col);
    if let Some(temporal) = derived.temporal()
        && let Some(candidates) = temporal.tip_spatial_single_candidates(grid, block, probe, &cell)
    {
        for (candidate_ref, candidate_mv) in candidates {
            if candidate_ref == block.ref_frame0 {
                insert_mv_stack_entry(entries, prune_count, candidate_mv, weight, offsets);
            } else {
                derived.add_spatial(block, candidate_ref, candidate_mv, cell.flags, || {
                    [cell.motion.sub_mv, cell.motion.sub_mv1]
                });
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StackInsert {
    Inserted,
    Updated,
    Skipped,
}

fn insert_mv_stack_entry(
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    candidate_mv: Mv,
    weight: u32,
    offsets: (i32, i32),
) -> StackInsert {
    if *prune_count < MAX_PR_NUM {
        for entry in entries.iter_mut() {
            *prune_count += 1;
            if entry.mv == candidate_mv {
                entry.weight = entry.weight.saturating_add(weight);
                return StackInsert::Updated;
            }
        }
    }
    if entries.try_push(MvStackEntry {
        mv: candidate_mv,
        weight,
        offsets,
    }) {
        StackInsert::Inserted
    } else {
        StackInsert::Skipped
    }
}

fn scan_temporal_mv_stack(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    temporal: Option<&TemporalMvContext>,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
) {
    let Some(temporal) = temporal.filter(|_| block.ref_frame1.is_none()) else {
        return;
    };
    let row_end = block.bh4.min(16);
    let col_end = block.bw4.min(16);
    let step_h4 = if block.bh4 >= 16 { 4 } else { 2 };
    let step_w4 = if block.bw4 >= 16 { 4 } else { 2 };

    let mut inserted = false;
    if row_end >= step_h4 && col_end >= step_w4 {
        inserted = add_temporal_mv_sample(
            grid,
            block,
            temporal,
            row_end - step_h4,
            col_end - step_w4,
            entries,
            prune_count,
        ) == StackInsert::Inserted;
    }
    if !inserted && (row_end >= 3 * step_h4 || col_end >= 3 * step_w4) {
        add_temporal_mv_sample(
            grid,
            block,
            temporal,
            row_end >> 1,
            col_end >> 1,
            entries,
            prune_count,
        );
    }
}

fn add_temporal_mv_sample(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    temporal: &TemporalMvContext,
    delta_row: usize,
    delta_col: usize,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
) -> StackInsert {
    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        return StackInsert::Skipped;
    }
    let mv_row = block.mi_row.saturating_add(delta_row);
    let mv_col = block.mi_col.saturating_add(delta_col);
    if !grid.is_inside(mv_row, mv_col) {
        return StackInsert::Skipped;
    }
    let Some(candidate_mv) = temporal.motion_field_mv(block.ref_frame0, mv_row >> 1, mv_col >> 1)
    else {
        return StackInsert::Skipped;
    };
    let Some(weight) = temporal.single_ref_weight(block.ref_frame0) else {
        return StackInsert::Skipped;
    };
    insert_mv_stack_entry(entries, prune_count, candidate_mv, weight, (0, 0))
}

fn generate_points_from_corners(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    warp: &mut WarpParamStack,
) {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let Some(bottom_left) = warp_corner(grid, block, bh4 - 1, -1, 0) else {
        return;
    };
    for iter in 0..2 {
        if iter == 1 && (warp.num_found != 0 || block.bw4 > 16) {
            return;
        }
        let Some(top_left) = warp_corner(grid, block, -1, -1, iter) else {
            continue;
        };
        let Some(top_right) = warp_corner(grid, block, -1, bw4 - 1, iter) else {
            continue;
        };
        let [pts, mvs] = [0, 1].map(|i| [top_left[i], top_right[i], bottom_left[i]]);
        model_from_corners(block, pts, mvs, warp);
    }
}

fn model_from_corners(
    block: &MvBlockContext,
    pts: [[i32; 2]; 3],
    mvs: [[i32; 2]; 3],
    warp: &mut WarpParamStack,
) {
    let mut ref_pts = [[0i64; 2]; 3];
    let mut all_mvs_same = true;
    for n in 0..3 {
        for c in 0..2 {
            ref_pts[n][c] = (i64::from(pts[n][c]) << WARPEDMODEL_PREC_BITS)
                + (i64::from(mvs[n][c]) << GM_TRANS_ONLY_PREC_DIFF);
            if mvs[n][c] != mvs[0][c] {
                all_mvs_same = false;
            }
        }
    }
    if all_mvs_same || ref_pts.iter().flatten().any(|&value| value < 0) {
        return;
    }
    let width_log2 = (block.bw4 as u32 * 4).trailing_zeros();
    let height_log2 = (block.bh4 as u32 * 4).trailing_zeros();
    let y0 = pts[0][0];
    let x0 = pts[0][1];
    let raw_params = [
        (ref_pts[1][1] - ref_pts[0][1]) >> width_log2,
        (ref_pts[2][1] - ref_pts[0][1]) >> height_log2,
        (ref_pts[1][0] - ref_pts[0][0]) >> width_log2,
        (ref_pts[2][0] - ref_pts[0][0]) >> height_log2,
    ];
    let [Ok(wmmat2), Ok(wmmat3), Ok(wmmat4), Ok(wmmat5)] = raw_params.map(i32::try_from) else {
        return;
    };
    let mut wmmat = [0, 0, wmmat2, wmmat3, wmmat4, wmmat5];
    let wmmat0 =
        ref_pts[0][1] - i64::from(wmmat[2]) * i64::from(x0) - i64::from(wmmat[3]) * i64::from(y0);
    let wmmat1 =
        ref_pts[0][0] - i64::from(wmmat[4]) * i64::from(x0) - i64::from(wmmat[5]) * i64::from(y0);
    reduce_warp_model(&mut wmmat);
    let high = WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS);
    wmmat[0] = wmmat0.clamp(-i64::from(WARPEDMODEL_TRANS_CLAMP), i64::from(high)) as i32;
    wmmat[1] = wmmat1.clamp(-i64::from(WARPEDMODEL_TRANS_CLAMP), i64::from(high)) as i32;
    warp.insert(wmmat);
}

fn warp_corner(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_row: i32,
    delta_col: i32,
    adjust_col: i32,
) -> Option<[[i32; 2]; 2]> {
    let mv_row = block.mi_row as i32 + delta_row;
    let mv_col = block.mi_col as i32 + delta_col;
    let is_sb_border = block.is_sb_border();
    let delta_col = delta_col + adjust_col;
    let mv_col2 = if delta_row < 0 && is_sb_border {
        let mi_col = block.mi_col as i32;
        (mi_col - (mi_col & 1)) + (delta_col - (delta_col & 1))
    } else {
        block.mi_col as i32 + delta_col
    };
    if is_sb_border && delta_col == 0 && block.bw4 <= 2 {
        return None;
    }
    let leaf = grid.probe(mv_row, mv_col2)?;
    let flags = leaf.flags();
    if !flags.is_inter() {
        return None;
    }
    let corner_mv = if flags.ref_frame0() == block.ref_frame0 {
        if flags.is_warp() {
            warp_motion_vector_at(leaf.stored_warp()?, block, mv_row + 1, mv_col + 1)
        } else {
            leaf.sub_mv(0)
        }
    } else if flags.ref_frame1() == Some(block.ref_frame0) && !flags.is_warp() {
        leaf.sub_mv(1)
    } else {
        return None;
    };
    Some([
        [(mv_row + 1) * 4, (mv_col + 1) * 4],
        [corner_mv.row, corner_mv.col],
    ])
}

pub(crate) fn warp_sub_mv_at(
    params: [i32; 6],
    block_r: usize,
    block_c: usize,
    rr: usize,
    cc: usize,
) -> Mv {
    let i8 = (rr.saturating_sub(block_r)) >> 1;
    let j8 = (cc.saturating_sub(block_c)) >> 1;
    let src_x = (block_c * 4 + j8 * 8 + 4) as i32;
    let src_y = (block_r * 4 + i8 * 8 + 4) as i32;
    let (row, col) = projected_warp_mv(params, src_x, src_y);
    rounded_warp_sub_mv(row, col)
}

/// [`warp_sub_mv_at`] along one row of 8x8 cells, stepping the projection.
#[derive(Clone, Copy)]
pub(super) struct WarpSubMvWalk {
    row: i64,
    col: i64,
    step_row: i64,
    step_col: i64,
}

impl WarpSubMvWalk {
    /// The walk over cell row `i8` of a block at `(block_r, block_c)`.
    pub(super) fn new(params: [i32; 6], block_r: usize, block_c: usize, i8: usize) -> Self {
        let src_x = (block_c * 4 + 4) as i32;
        let src_y = (block_r * 4 + i8 * 8 + 4) as i32;
        let (row, col) = projected_warp_mv(params, src_x, src_y);
        Self {
            row,
            col,
            step_row: 8 * i64::from(params[4]),
            step_col: 8 * (i64::from(params[2]) - (1i64 << WARPEDMODEL_PREC_BITS)),
        }
    }

    /// The sub-MV of the next cell.
    pub(super) fn next(&mut self) -> Mv {
        let mv = rounded_warp_sub_mv(self.row, self.col);
        self.row += self.step_row;
        self.col += self.step_col;
        mv
    }
}

/// `Round2Signed(value, WARPEDMODEL_PREC_BITS - 3)` of a projected position.
const fn round_warp_projection(value: i64) -> i64 {
    const SHIFT: u32 = WARPEDMODEL_PREC_BITS - 3;
    let magnitude = ((value.unsigned_abs() + (1 << (SHIFT - 1))) >> SHIFT) as i64;
    if value < 0 { -magnitude } else { magnitude }
}

/// [`round_warp_projection`] of a projected sub-MV, clamped.
fn rounded_warp_sub_mv(row: i64, col: i64) -> Mv {
    let bound = (1 << 16) - 1;
    Mv {
        row: round_warp_projection(row).clamp(-bound, bound) as i32,
        col: round_warp_projection(col).clamp(-bound, bound) as i32,
    }
}

fn projected_warp_mv(params: [i32; 6], x: i32, y: i32) -> (i64, i64) {
    let x = i64::from(x);
    let y = i64::from(y);
    let one = 1i64 << WARPEDMODEL_PREC_BITS;
    (
        i64::from(params[4]) * x + (i64::from(params[5]) - one) * y + i64::from(params[1]),
        (i64::from(params[2]) - one) * x + i64::from(params[3]) * y + i64::from(params[0]),
    )
}

fn warp_motion_vector_at(
    params: [i32; 6],
    block: &MvBlockContext,
    pos_row: i32,
    pos_col: i32,
) -> Mv {
    let y = pos_row * 4;
    let x = pos_col * 4;
    let (row, col) = projected_warp_mv(params, x, y);
    clip_and_clamp_projected_mv(
        block,
        round_warp_projection(row),
        round_warp_projection(col),
    )
}

fn clip_and_clamp_projected_mv(block: &MvBlockContext, row: i64, col: i64) -> Mv {
    let bound = (1 << 16) - 1;
    let row = row.clamp(-bound, bound);
    let col = col.clamp(-bound, bound);
    clamp_mv(
        block,
        Mv {
            row: row as i32,
            col: col as i32,
        },
    )
}

pub(crate) fn reduce_warp_model(params: &mut [i32; 6]) {
    let max_value = (1i64 << (WARPEDMODEL_PREC_BITS - 1)) - (1i64 << WARP_PARAM_REDUCE_BITS);
    let min_value = -max_value;
    for (index, param) in params.iter_mut().enumerate().skip(2) {
        let offset = if index == 2 || index == 5 {
            1i64 << WARPEDMODEL_PREC_BITS
        } else {
            0
        };
        let original = i64::from(*param) - offset;
        let clamped = original.clamp(min_value, max_value);
        *param = ((round2_signed(clamped, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS)
            + offset) as i32;
    }
}

/// Uses max then min, not `clamp`: `clamp` carries a min > max panic path
/// that keeps this out of line, and these bounds are always ordered.
fn clamp_mv(block: &MvBlockContext, mv: Mv) -> Mv {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let mi_row = block.mi_row as i32;
    let mi_col = block.mi_col as i32;
    let mi_rows = block.mi_rows as i32;
    let mi_cols = block.mi_cols as i32;

    let row_low = -(mi_row + bh4) * MI_SIZE * 8 - MV_BORDER;
    let row_high = (mi_rows - mi_row) * MI_SIZE * 8 + MV_BORDER;
    let col_low = -(mi_col + bw4) * MI_SIZE * 8 - MV_BORDER;
    let col_high = (mi_cols - mi_col) * MI_SIZE * 8 + MV_BORDER;

    Mv {
        row: mv.row.max(row_low).min(row_high),
        col: mv.col.max(col_low).min(col_high),
    }
}

#[cfg(test)]
mod bank_tests;
#[cfg(test)]
mod global_motion_tests;
#[cfg(test)]
mod tests;
