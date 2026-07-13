// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::DrlReorder;
use splot_recon::math::round2_signed;

use super::block::{WARP_PARAM_REDUCE_BITS, WARPEDMODEL_PREC_BITS, WARPEDMODEL_TRANS_CLAMP};
use super::{Mv, mc::CWP_EQUAL};

pub(crate) const MAX_REF_MV_STACK_SIZE: usize = 6;

pub(crate) const MAX_WARP_REF_CANDIDATES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FixedStack<T, const N: usize> {
    entries: [T; N],
    len: usize,
}

impl<T: Default, const N: usize> FixedStack<T, N> {
    fn new() -> Self {
        Self {
            entries: core::array::from_fn(|_| T::default()),
            len: 0,
        }
    }

    fn from_entries(entries: impl IntoIterator<Item = T>) -> Self {
        let mut stack = Self::new();
        for entry in entries {
            if !stack.try_push(entry) {
                break;
            }
        }
        stack
    }
}

impl<T, const N: usize> FixedStack<T, N> {
    fn try_push(&mut self, entry: T) -> bool {
        let Some(slot) = self.entries.get_mut(self.len) else {
            return false;
        };
        *slot = entry;
        self.len += 1;
        true
    }

    fn try_swap(&mut self, a: usize, b: usize) -> bool {
        if a >= self.len || b >= self.len {
            return false;
        }
        self.entries.swap(a, b);
        true
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

pub(crate) const DEFAULT_WARP_PARAMS: [i64; 6] = [
    0,
    0,
    1 << WARPEDMODEL_PREC_BITS,
    0,
    0,
    1 << WARPEDMODEL_PREC_BITS,
];

const GM_TRANS_ONLY_PREC_DIFF: u32 = WARPEDMODEL_PREC_BITS - 3;

const MV_BORDER: i32 = 128;

const MI_SIZE: i32 = 4;
mod derived;
mod extra_search;
mod temporal;
mod warp_bank;
use derived::{CompoundScanState, DerivedMvState};
use extra_search::{compound_extra_search, extra_search};
pub(crate) use temporal::{
    OrderHintMvContext, TemporalMotionBlock, TemporalMotionField, TemporalMvContext,
    TemporalProjectionConfig, reference_order_hints, tip_reference_pair_from_hints,
};
pub(crate) use warp_bank::{WarpParamBank, WarpParamStack};

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub(crate) enum MotionMode {
    Simple,
    #[allow(dead_code)]
    InterIntra,
    LocalWarp,
    DeltaWarp,
    ExtendWarp,
}

impl MotionMode {
    const fn is_warp(self) -> bool {
        matches!(self, Self::LocalWarp | Self::DeltaWarp | Self::ExtendWarp)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NeighbourCell {
    is_inter: bool,
    ref_frame0: i8,
    ref_frame1: Option<i8>,
    y_mode: NeighbourYMode,
    newmv_for_list0: bool,
    newmv_for_list1: bool,
    mv: Mv,
    mv1: Option<Mv>,
    skip_mode: bool,
    skip: bool,
    interp_filter: u8,
    use_amvd: bool,
    masked_compound: bool,
    cwp_weight: i16,
    motion_mode: MotionMode,
    warp_params: Option<[i64; 6]>,
    sub_mv: Mv,
    sub_mv1: Mv,
    base_r: usize,
    base_c: usize,
    bw4: usize,
    bh4: usize,
    tip_size_16x16: bool,
    precision: BlockPrecisionRecord,
}

impl NeighbourCell {
    const fn is_warp(self) -> bool {
        self.motion_mode.is_warp()
    }
}

const EMPTY_NEIGHBOUR_CELL: NeighbourCell = NeighbourCell {
    is_inter: false,
    ref_frame0: -1,
    ref_frame1: None,
    y_mode: NeighbourYMode::Other,
    newmv_for_list0: false,
    newmv_for_list1: false,
    mv: Mv::ZERO,
    mv1: None,
    skip_mode: false,
    skip: false,
    interp_filter: SWITCHABLE_FILTERS,
    use_amvd: false,
    masked_compound: false,
    cwp_weight: CWP_EQUAL,
    motion_mode: MotionMode::Simple,
    warp_params: None,
    sub_mv: Mv::ZERO,
    sub_mv1: Mv::ZERO,
    base_r: 0,
    base_c: 0,
    bw4: 0,
    bh4: 0,
    tip_size_16x16: false,
    precision: BlockPrecisionRecord {
        use_most_probable_precision: false,
        mv_precision: 0,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockPrecisionRecord {
    pub(crate) use_most_probable_precision: bool,
    pub(crate) mv_precision: u8,
}

impl BlockPrecisionRecord {
    pub(crate) const fn most_probable(mv_precision: u8) -> Self {
        Self {
            use_most_probable_precision: true,
            mv_precision,
        }
    }

    pub(crate) const fn explicit(mv_precision: u8) -> Self {
        Self {
            use_most_probable_precision: false,
            mv_precision,
        }
    }
}

impl Default for BlockPrecisionRecord {
    fn default() -> Self {
        Self::most_probable(super::read_mv::MV_PRECISION_EIGHTH_PEL)
    }
}

const SWITCHABLE_FILTERS: u8 = 3;
const INTER_FILTER_COMP_OFFSET: usize = SWITCHABLE_FILTERS as usize + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NeighbourYMode {
    NewMv,
    Other,
}

pub(crate) struct NeighbourMvGrid {
    mi_rows: usize,
    mi_cols: usize,
    cells: Vec<Option<NeighbourCell>>,
}

impl NeighbourMvGrid {
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let cells = mi_rows.checked_mul(mi_cols)?;
        Some(Self {
            mi_rows,
            mi_cols,
            cells: vec![None; cells],
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        is_inter: bool,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        y_mode: NeighbourYMode,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        precision: BlockPrecisionRecord,
    ) {
        self.record_block_with_warp(
            r,
            c,
            n4w,
            n4h,
            is_inter,
            ref_frame0,
            ref_frame1,
            y_mode,
            mv,
            skip,
            interp_filter,
            use_amvd,
            MotionMode::Simple,
            None,
            false,
            precision,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_warp_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        ref_frame0: i8,
        y_mode: NeighbourYMode,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        motion_mode: MotionMode,
        warp_params: [i64; 6],
        precision: BlockPrecisionRecord,
    ) {
        self.record_block_with_warp(
            r,
            c,
            n4w,
            n4h,
            true,
            ref_frame0,
            None,
            y_mode,
            mv,
            skip,
            interp_filter,
            use_amvd,
            motion_mode,
            Some(warp_params),
            false,
            precision,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn record_block_with_warp(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        is_inter: bool,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        y_mode: NeighbourYMode,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        motion_mode: MotionMode,
        warp_params: Option<[i64; 6]>,
        tip_size_16x16: bool,
        precision: BlockPrecisionRecord,
    ) {
        let cell = NeighbourCell {
            is_inter,
            ref_frame0,
            ref_frame1,
            y_mode,
            newmv_for_list0: matches!(y_mode, NeighbourYMode::NewMv),
            newmv_for_list1: false,
            mv,
            mv1: None,
            skip_mode: false,
            skip,
            interp_filter: interp_filter.min(SWITCHABLE_FILTERS),
            use_amvd,
            masked_compound: false,
            cwp_weight: CWP_EQUAL,
            motion_mode,
            warp_params,
            sub_mv: mv,
            sub_mv1: Mv::ZERO,
            base_r: r,
            base_c: c,
            bw4: n4w,
            bh4: n4h,
            tip_size_16x16,
            precision,
        };
        for rr in r..r.saturating_add(n4h) {
            if rr >= self.mi_rows {
                break;
            }
            for cc in c..c.saturating_add(n4w) {
                if cc >= self.mi_cols {
                    break;
                }
                let mut cell = cell;
                if motion_mode.is_warp()
                    && let Some(params) = warp_params
                {
                    cell.sub_mv = warp_sub_mv_at(params, r, c, rr, cc);
                }
                self.cells[rr * self.mi_cols + cc] = Some(cell);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_tip_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        y_mode: NeighbourYMode,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        tip_size_16x16: bool,
        precision: BlockPrecisionRecord,
    ) {
        self.record_block_with_warp(
            r,
            c,
            n4w,
            n4h,
            true,
            TIP_REF_FRAME,
            None,
            y_mode,
            mv,
            skip,
            interp_filter,
            use_amvd,
            MotionMode::Simple,
            None,
            tip_size_16x16,
            precision,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_compound_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        ref_frame0: i8,
        ref_frame1: i8,
        list0_is_newmv: bool,
        list1_is_newmv: bool,
        mv0: Mv,
        mv1: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
        masked_compound: bool,
        cwp_weight: i16,
        skip_mode: bool,
        precision: BlockPrecisionRecord,
        warp_params: [Option<[i64; 6]>; 2],
    ) {
        let cell = NeighbourCell {
            is_inter: true,
            ref_frame0,
            ref_frame1: Some(ref_frame1),
            y_mode: if list0_is_newmv {
                NeighbourYMode::NewMv
            } else {
                NeighbourYMode::Other
            },
            newmv_for_list0: list0_is_newmv,
            newmv_for_list1: list1_is_newmv,
            mv: mv0,
            mv1: Some(mv1),
            skip_mode,
            skip,
            interp_filter: interp_filter.min(SWITCHABLE_FILTERS),
            use_amvd,
            masked_compound,
            cwp_weight,
            motion_mode: if warp_params[0].is_some() || warp_params[1].is_some() {
                MotionMode::LocalWarp
            } else {
                MotionMode::Simple
            },
            // Neighbour-facing derivations read only the first model (AVM `wm_params[0]`).
            warp_params: warp_params[0],
            sub_mv: mv0,
            sub_mv1: mv1,
            base_r: r,
            base_c: c,
            bw4: n4w,
            bh4: n4h,
            tip_size_16x16: false,
            precision,
        };
        for rr in r..r.saturating_add(n4h) {
            if rr >= self.mi_rows {
                break;
            }
            for cc in c..c.saturating_add(n4w) {
                if cc >= self.mi_cols {
                    break;
                }
                let mut cell = cell;
                if let Some(params) = warp_params[0] {
                    cell.sub_mv = warp_sub_mv_at(params, r, c, rr, cc);
                }
                if let Some(params) = warp_params[1] {
                    cell.sub_mv1 = warp_sub_mv_at(params, r, c, rr, cc);
                }
                self.cells[rr * self.mi_cols + cc] = Some(cell);
            }
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_block_with_newmv_lists(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        ref_frame0: i8,
        ref_frame1: i8,
        list0_is_newmv: bool,
        list1_is_newmv: bool,
        mv: Mv,
        skip: bool,
        interp_filter: u8,
        use_amvd: bool,
    ) {
        self.record_compound_block(
            r,
            c,
            n4w,
            n4h,
            ref_frame0,
            ref_frame1,
            list0_is_newmv,
            list1_is_newmv,
            mv,
            mv,
            skip,
            interp_filter,
            use_amvd,
            false,
            CWP_EQUAL,
            false,
            BlockPrecisionRecord::default(),
            [None, None],
        );
    }

    fn get(&self, r: i32, c: i32) -> Option<NeighbourCell> {
        if r < 0 || c < 0 {
            return None;
        }
        let (r, c) = (r as usize, c as usize);
        if r >= self.mi_rows || c >= self.mi_cols {
            return None;
        }
        self.cells[r * self.mi_cols + c]
    }
    pub(crate) fn is_non_tip_at(&self, r: i32, c: i32) -> bool {
        matches!(self.get(r, c), Some(cell) if cell.ref_frame0 != TIP_REF_FRAME)
    }
}

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

    fn cell(self, grid: &NeighbourMvGrid, block: &MvBlockContext) -> Option<NeighbourCell> {
        let row = block.mi_row as i32 + self.delta_row;
        let col = block.mi_col as i32 + self.delta_col;
        grid.get(row, col)
    }

    fn stack_cell(
        self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
    ) -> Option<(NeighbourCell, u32)> {
        let (row, col, delta_col) = self.stack_target(block);
        let zero_weight = (self.delta_row == -1 && delta_col == -1) || delta_col < -1;
        let weight = u32::from(!zero_weight);
        grid.get(row, col).map(|cell| (cell, weight))
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

    fn warp_context_cell(
        self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
    ) -> Option<NeighbourCell> {
        let (delta_row, delta_col) = self.warp_context_delta(block);
        grid.get(
            block.mi_row as i32 + delta_row,
            block.mi_col as i32 + delta_col,
        )
    }

    fn warp_context_delta(self, block: &MvBlockContext) -> (i32, i32) {
        let mut delta_col = self.delta_col;
        if self.delta_row < 0 && block.is_sb_border() {
            delta_col -= (block.mi_col & 1) as i32;
        }
        (self.delta_row, delta_col)
    }
}

fn immediate_spatial_probes(block: &MvBlockContext) -> [RelativeProbe; 4] {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    [
        RelativeProbe::new(bh4 - 1, -1),
        RelativeProbe::new(-1, bw4 - 1),
        RelativeProbe::new(0, -1),
        RelativeProbe::new(-1, 0),
    ]
}

fn warp_context_spatial_probes(block: &MvBlockContext) -> [Option<RelativeProbe>; 4] {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let is_sb_border = block.is_sb_border();
    [
        Some(RelativeProbe::new(bh4 - 1, -1)),
        Some(RelativeProbe::new(
            -1,
            if is_sb_border {
                (bw4 - 2).max(0)
            } else {
                bw4 - 1
            },
        )),
        Some(RelativeProbe::new(0, -1)),
        (bw4 >= if is_sb_border { 4 } else { 2 }).then_some(RelativeProbe::new(-1, 0)),
    ]
}

fn mv_stack_spatial_probes(block: &MvBlockContext) -> [Option<RelativeProbe>; 7] {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let is_sb_border_adj = i32::from(block.is_sb_border());
    [
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
    ]
}

fn optional_probe(enabled: bool, delta_row: i32, delta_col: i32) -> Option<RelativeProbe> {
    enabled.then_some(RelativeProbe::new(delta_row, delta_col))
}

fn matches_block_ref(cell: NeighbourCell, block: &MvBlockContext) -> bool {
    neighbour_matches_ref(cell, block.ref_frame0)
}

fn neighbour_matches_ref(cell: NeighbourCell, ref_frame: i8) -> bool {
    cell.is_inter && (cell.ref_frame0 == ref_frame || cell.ref_frame1 == Some(ref_frame))
}

fn mode_ctx_match_newmv(
    cell: NeighbourCell,
    block: &MvBlockContext,
    tip_ref_pair: Option<(i8, i8)>,
) -> Option<bool> {
    if !cell.is_inter {
        return None;
    }
    let Some(block_ref1) = block.ref_frame1 else {
        if cell.ref_frame0 == block.ref_frame0 {
            return Some(cell.newmv_for_list0);
        }
        if cell.ref_frame1 == Some(block.ref_frame0) && cell.ref_frame0 != block.ref_frame0 {
            return Some(cell.newmv_for_list1);
        }
        return None;
    };
    if cell.ref_frame0 == TIP_REF_FRAME
        && cell.ref_frame1.is_none()
        && tip_ref_pair == Some((block.ref_frame0, block_ref1))
    {
        return Some(cell.newmv_for_list0);
    }
    if cell.ref_frame0 == block.ref_frame0 && cell.ref_frame1 == Some(block_ref1) {
        return Some(cell.newmv_for_list0 || cell.newmv_for_list1);
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModeContext {
    pub(crate) new_mv_context: usize,
    pub(crate) new_mv_count: usize,
    pub(crate) warp_mv_count: usize,
    pub(crate) warp_sample_found: bool,
    pub(crate) warp_sample_found1: bool,
    pub(crate) extend_delta: Option<(i32, i32)>,
}

pub(crate) fn find_mode_ctx(grid: &NeighbourMvGrid, block: &MvBlockContext) -> ModeContext {
    find_mode_ctx_with_tip(grid, block, None)
}

pub(crate) fn find_mode_ctx_with_tip(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    tip_ref_pair: Option<(i8, i8)>,
) -> ModeContext {
    let mut new_mv_count = 0usize;
    let mut warp_mv_count = 0usize;
    let mut found = [false; 4];

    for (slot, probe) in found.iter_mut().zip(immediate_spatial_probes(block)) {
        let Some(cell) = probe.cell(grid, block) else {
            continue;
        };
        let Some(is_newmv) = mode_ctx_match_newmv(cell, block, tip_ref_pair) else {
            continue;
        };
        if is_newmv {
            new_mv_count = (new_mv_count + 1).min(3);
        }
        *slot = true;
    }
    let mut warp_sample_found = false;
    let mut warp_sample_found1 = false;
    let mut extend_delta = None;
    for probe in warp_context_spatial_probes(block).into_iter().flatten() {
        let Some(cell) = probe.warp_context_cell(grid, block) else {
            continue;
        };
        if matches_block_ref(cell, block) {
            if !warp_sample_found {
                extend_delta = Some(probe.warp_context_delta(block));
            }
            warp_sample_found = true;
            if cell.is_warp() {
                warp_mv_count = (warp_mv_count + 1).min(4);
            }
        }
        if let Some(ref1) = block.ref_frame1
            && neighbour_matches_ref(cell, ref1)
        {
            warp_sample_found1 = true;
        }
    }

    let [left_a, above_a, left_b, above_b] = found;
    let nearest_match = usize::from(above_a || above_b) + usize::from(left_a || left_b);
    let new_mv_context = nearest_match + if new_mv_count > 0 { 2 } else { 0 };
    ModeContext {
        new_mv_context,
        new_mv_count,
        warp_mv_count,
        warp_sample_found,
        warp_sample_found1,
        extend_delta,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlockNeighbourContext {
    pub(crate) is_inter_ctx: usize,
    pub(crate) skip_mode_ctx: usize,
    pub(crate) skip_ctx: usize,
    pub(crate) has_neighbour: bool,
    ref_counts: [u8; BlockNeighbourContext::MAX_NEIGHBOUR_REFS],
    cells: [NeighbourCell; 2],
    cell_count: usize,
    npos_cells: [NeighbourCell; 2],
    npos_count: usize,
}

impl BlockNeighbourContext {
    const MAX_NEIGHBOUR_REFS: usize = 7;

    pub(crate) fn single_ref_ctx(&self, ref_idx: usize, num_total_refs: usize) -> Option<usize> {
        let this_count = u32::from(*self.ref_counts.get(ref_idx)?);
        let next_start = ref_idx.checked_add(1)?;
        let next_count = if next_start >= num_total_refs {
            0
        } else {
            self.ref_counts
                .get(next_start..num_total_refs)?
                .iter()
                .map(|&count| u32::from(count))
                .sum()
        };
        Some(match this_count.cmp(&next_count) {
            core::cmp::Ordering::Equal => 1,
            core::cmp::Ordering::Less => 0,
            core::cmp::Ordering::Greater => 2,
        })
    }

    pub(crate) fn skip_mode_ref_pair(&self, default: (i8, i8)) -> (i8, i8) {
        for cell in self.cells.iter().take(self.cell_count) {
            if !cell.is_inter {
                continue;
            }
            if let Some(ref_frame1) = cell.ref_frame1 {
                return (cell.ref_frame0, ref_frame1);
            }
            break;
        }
        default
    }

    pub(crate) fn tip_mode_ctx(&self) -> usize {
        self.cells
            .iter()
            .take(self.cell_count)
            .filter(|cell| cell.ref_frame0 == TIP_REF_FRAME)
            .count()
    }

    pub(crate) fn comp_mode_ctx(
        &self,
        ref_frame_idx: &[u32],
        ref_order_hint: &[u32],
        current_order_hint: i32,
    ) -> usize {
        match self.cell_count {
            0 => 1,
            1 => {
                let neighbour = self.cells[0];
                if neighbour.ref_frame1.is_some() {
                    3
                } else {
                    usize::from(is_backward_ref_frame(
                        neighbour,
                        ref_frame_idx,
                        ref_order_hint,
                        current_order_hint,
                    ))
                }
            }
            _ => {
                let first = self.cells[0];
                let second = self.cells[1];
                match (first.ref_frame1.is_some(), second.ref_frame1.is_some()) {
                    (false, false) => usize::from(
                        is_backward_ref_frame(
                            first,
                            ref_frame_idx,
                            ref_order_hint,
                            current_order_hint,
                        ) ^ is_backward_ref_frame(
                            second,
                            ref_frame_idx,
                            ref_order_hint,
                            current_order_hint,
                        ),
                    ),
                    (false, true) => {
                        2 + usize::from(
                            is_backward_ref_frame(
                                first,
                                ref_frame_idx,
                                ref_order_hint,
                                current_order_hint,
                            ) || !first.is_inter,
                        )
                    }
                    (true, false) => {
                        2 + usize::from(
                            is_backward_ref_frame(
                                second,
                                ref_frame_idx,
                                ref_order_hint,
                                current_order_hint,
                            ) || !second.is_inter,
                        )
                    }
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
        if !cell.is_inter {
            return 0;
        }
        if cell.ref_frame1.is_some() {
            usize::from(cell.masked_compound)
        } else if Some(cell.ref_frame0) == furthest_future_ref {
            2
        } else {
            0
        }
    }

    pub(crate) fn interp_filter_ctx(&self, ref_frame0: i8, ref_frame1_is_inter: bool) -> usize {
        let mut neighbour_filter_type = [SWITCHABLE_FILTERS; 2];
        for (slot, cell) in self.npos_cells.iter().take(self.npos_count).enumerate() {
            if neighbour_matches_ref(*cell, ref_frame0) {
                neighbour_filter_type[slot] = cell.interp_filter.min(SWITCHABLE_FILTERS);
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
            .filter(|cell| cell.is_inter && cell.ref_frame0 == ref_frame0 && cell.use_amvd)
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
            + cells
                .filter(|cell| cell.motion_mode == MotionMode::LocalWarp)
                .count()
    }

    pub(crate) fn most_probable_precision_ctx(&self) -> usize {
        self.npos_cells
            .iter()
            .take(self.npos_count)
            .filter(|cell| cell.precision.use_most_probable_precision)
            .count()
    }

    pub(crate) fn pb_mv_precision_ctx(&self, frame_precision: u8) -> usize {
        usize::from(
            self.npos_cells
                .iter()
                .take(self.npos_count)
                .any(|cell| cell.precision.mv_precision < frame_precision),
        )
    }
}

pub(crate) fn block_neighbour_ctx(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
) -> BlockNeighbourContext {
    let lists = collect_neighbour_context_cells(grid, block);
    let (buf, num_buf) = (lists.buf, lists.buf_len);

    let n_intra_0 = num_buf >= 1 && !buf[0].is_inter;
    let n_intra_1 = num_buf >= 2 && !buf[1].is_inter;
    let is_inter_ctx = if num_buf == 2 {
        if n_intra_0 && n_intra_1 {
            3
        } else {
            usize::from(n_intra_0 || n_intra_1)
        }
    } else if num_buf == 1 {
        2 * usize::from(n_intra_0)
    } else {
        0
    };

    let mut skip_ctx = 0usize;
    let mut skip_mode_ctx = 0usize;
    for cell in buf.iter().take(num_buf) {
        skip_ctx += usize::from(cell.skip);
        skip_mode_ctx += usize::from(cell.skip_mode);
    }

    let mut ref_counts = [0u8; BlockNeighbourContext::MAX_NEIGHBOUR_REFS];
    for cell in buf.iter().take(num_buf) {
        if !cell.is_inter {
            continue;
        }
        for ref_frame in [Some(cell.ref_frame0), cell.ref_frame1] {
            let Some(ref_frame) = ref_frame.filter(|&ref_frame| ref_frame >= 0) else {
                continue;
            };
            if let Some(slot) = ref_counts.get_mut(ref_frame as usize) {
                *slot = slot.saturating_add(1);
            }
        }
    }

    BlockNeighbourContext {
        is_inter_ctx,
        skip_mode_ctx,
        skip_ctx,
        has_neighbour: num_buf >= 1,
        ref_counts,
        cells: buf,
        cell_count: num_buf,
        npos_cells: lists.npos,
        npos_count: lists.npos_len,
    }
}

fn is_backward_ref_frame(
    cell: NeighbourCell,
    ref_frame_idx: &[u32],
    ref_order_hint: &[u32],
    current_order_hint: i32,
) -> bool {
    if !cell.is_inter || cell.ref_frame0 < 0 {
        return false;
    }
    if cell.ref_frame0 == TIP_REF_FRAME {
        return true;
    }
    let Some(&slot) = ref_frame_idx.get(cell.ref_frame0 as usize) else {
        return false;
    };
    let Some(&order_hint) = ref_order_hint.get(slot as usize) else {
        return false;
    };
    let Ok(order_hint) = i32::try_from(order_hint) else {
        return false;
    };
    (order_hint - current_order_hint).clamp(-127, 127) > 0
}

struct NeighbourContextLists {
    buf: [NeighbourCell; 2],
    buf_len: usize,
    npos: [NeighbourCell; 2],
    npos_len: usize,
}

fn collect_neighbour_context_cells(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
) -> NeighbourContextLists {
    let mut lists = NeighbourContextLists {
        buf: [EMPTY_NEIGHBOUR_CELL; 2],
        buf_len: 0,
        npos: [EMPTY_NEIGHBOUR_CELL; 2],
        npos_len: 0,
    };

    for probe in immediate_spatial_probes(block) {
        let Some(cell) = probe.cell(grid, block) else {
            continue;
        };
        if lists.buf_len < lists.buf.len() {
            lists.buf[lists.buf_len] = cell;
            lists.buf_len += 1;
        }
        let above_sb_boundary = probe.delta_row < 0 && block.is_sb_border();
        if !above_sb_boundary && lists.npos_len < lists.npos.len() {
            lists.npos[lists.npos_len] = cell;
            lists.npos_len += 1;
        }
    }

    lists
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MvStack {
    stack: FixedStack<(Mv, (i32, i32)), MAX_REF_MV_STACK_SIZE>,
    warp: WarpParamStack,
    block: MvBlockContext,
}

impl MvStack {
    fn entry(&self, idx: usize) -> Option<&(Mv, (i32, i32))> {
        self.stack.get(idx).or_else(|| self.stack.last())
    }

    pub(crate) fn candidate(&self, idx: usize) -> Mv {
        self.entry(idx).map_or(Mv::ZERO, |entry| entry.0)
    }

    pub(crate) fn candidate_offsets(&self, idx: usize) -> (i32, i32) {
        self.entry(idx).map_or((0, 0), |entry| entry.1)
    }

    pub(crate) fn warp_candidate(&self, idx: usize) -> [i64; 6] {
        self.warp
            .slots
            .get(idx)
            .copied()
            .unwrap_or(DEFAULT_WARP_PARAMS)
    }

    pub(crate) fn warp_predicted_mv(&self, idx: usize, precision: u8) -> Mv {
        let params = self.warp_candidate(idx);
        let block = &self.block;
        let x = block.mi_col as i64 * 4 + (block.bw4 as i64 * 4) / 2 - 1;
        let y = block.mi_row as i64 * 4 + (block.bh4 as i64 * 4) / 2 - 1;
        let one = 1i64 << WARPEDMODEL_PREC_BITS;
        let xc = (params[2] - one) * x + params[3] * y + params[0];
        let yc = params[4] * x + (params[5] - one) * y + params[1];
        let (row, col) = if precision == super::read_mv::MV_PRECISION_EIGHTH_PEL {
            (
                round2_signed(yc, WARPEDMODEL_PREC_BITS - 3),
                round2_signed(xc, WARPEDMODEL_PREC_BITS - 3),
            )
        } else {
            (
                round2_signed(yc, WARPEDMODEL_PREC_BITS - 2) * 2,
                round2_signed(xc, WARPEDMODEL_PREC_BITS - 2) * 2,
            )
        };
        let mv = clip_and_clamp_projected_mv(block, row, col);
        if precision < super::read_mv::MV_PRECISION_HALF_PEL {
            super::read_mv::lower_mv_precision(precision, mv)
        } else {
            mv
        }
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
        self.stack
            .get(idx)
            .or_else(|| self.stack.last())
            .copied()
            .unwrap_or_default()
    }
}

pub(crate) enum ExtendWarpNeighbour {
    Params([i64; 6]),
    List1MvUnretained,
    Missing,
}

pub(crate) fn extend_warp_neighbour_params(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_row: i32,
    delta_col: i32,
) -> ExtendWarpNeighbour {
    let Some(cell) = grid.get(
        block.mi_row as i32 + delta_row,
        block.mi_col as i32 + delta_col,
    ) else {
        return ExtendWarpNeighbour::Missing;
    };
    if cell.is_warp()
        && let Some(params) = cell.warp_params
    {
        return ExtendWarpNeighbour::Params(params);
    }
    let neighbour_mv = if cell.ref_frame0 == block.ref_frame0 {
        Some(cell.mv)
    } else {
        cell.mv1
    };
    let Some(mv) = neighbour_mv else {
        return ExtendWarpNeighbour::List1MvUnretained;
    };
    let mut params = splot_recon::IDENTITY_WARP_PARAMS;
    params[0] = i64::from(mv.col) << (WARPEDMODEL_PREC_BITS - 3);
    params[1] = i64::from(mv.row) << (WARPEDMODEL_PREC_BITS - 3);
    ExtendWarpNeighbour::Params(params)
}

const LEAST_SQUARES_SAMPLES_MAX: usize = 8;

pub(crate) struct WarpSamples(FixedStack<[i64; 4], LEAST_SQUARES_SAMPLES_MAX>);

impl WarpSamples {
    pub(crate) fn as_slice(&self) -> &[[i64; 4]] {
        &self.0
    }
}

#[allow(
    clippy::large_enum_variant,
    reason = "warp samples intentionally stay stack allocated"
)]
pub(crate) enum WarpSampleCollection {
    Samples(WarpSamples),
    List1MvUnretained,
}

pub(crate) fn find_warp_samples(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    target_ref: i8,
) -> WarpSampleCollection {
    let mut samples = FixedStack::new();
    let mi_row = block.mi_row as i32;
    let mi_col = block.mi_col as i32;
    let w4 = block.bw4 as i32;
    let h4 = block.bh4 as i32;
    let mi_rows = block.mi_rows as i32;
    let mi_cols = block.mi_cols as i32;
    let mut missing_list1 = false;
    let mut add_sample = |samples: &mut FixedStack<[i64; 4], LEAST_SQUARES_SAMPLES_MAX>,
                          delta_row: i32,
                          delta_col: i32| {
        if samples.len() >= LEAST_SQUARES_SAMPLES_MAX {
            return;
        }
        let Some(cell) = grid.get(mi_row + delta_row, mi_col + delta_col) else {
            return;
        };
        let lists = [
            (cell.ref_frame0 == target_ref && cell.is_inter).then_some(Some(cell.mv)),
            (cell.ref_frame1 == Some(target_ref)).then_some(cell.mv1),
        ];
        for list_mv in lists.into_iter().flatten() {
            if samples.len() >= LEAST_SQUARES_SAMPLES_MAX {
                return;
            }
            let Some(mv) = list_mv else {
                missing_list1 = true;
                continue;
            };
            let mid_y = (cell.base_r * 4 + cell.bh4 * 2) as i64 - 1;
            let mid_x = (cell.base_c * 4 + cell.bw4 * 2) as i64 - 1;
            let _ = samples.try_push([
                mid_y * 8,
                mid_x * 8,
                mid_y * 8 + i64::from(mv.row),
                mid_x * 8 + i64::from(mv.col),
            ]);
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
        let src_w4 = grid.get(mi_row - 1, col).map_or(0, |cell| cell.bw4 as i32);
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
            .map_or(0, |cell| cell.base_c as i32 - mi_col);
        if col_offset < 0 {
            do_top_left = false;
        }
        let mut i = col_offset;
        let limit = w4.min(mi_cols - mi_col);
        while i < limit {
            let src_w = grid
                .get(mi_row - 1, mi_col + i)
                .map_or(1, |cell| (cell.bw4 as i32).max(1));
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
            .map_or(0, |cell| cell.base_r as i32 - mi_row);
        if row_offset < 0 {
            do_top_left = false;
        }
        let mut i = row_offset;
        let limit = h4.min(mi_rows - mi_row);
        while i < limit {
            let src_h = grid
                .get(mi_row + i, mi_col - 1)
                .map_or(1, |cell| (cell.bh4 as i32).max(1));
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
    if missing_list1 {
        return WarpSampleCollection::List1MvUnretained;
    }
    WarpSampleCollection::Samples(WarpSamples(samples))
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

    pub(crate) fn reset_for_leaf(
        &mut self,
        grid: &NeighbourMvGrid,
        mi_row: usize,
        mi_col: usize,
        sb_size4: usize,
    ) {
        let sb = (mi_row / sb_size4.max(1), mi_col / sb_size4.max(1));
        if self.current_sb == Some(sb) {
            return;
        }
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
        seed_walk_from_row_above(grid, sb.0 * sb_size4, sb.1 * sb_size4, sb_size4, |cell| {
            self.update(
                cell.ref_frame0,
                cell.ref_frame1,
                cell.mv,
                cell.mv1,
                cell.cwp_weight,
                false,
            );
        });
    }

    fn update_unit_budget(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        sb_size4: usize,
    ) {
        let unit_size4 = (sb_size4 >> 3).max(1);
        let unit_count = ((n4w / unit_size4).max(1) * (n4h / unit_size4).max(1)) as i32;
        if mi_row.is_multiple_of(sb_size4) && mi_col.is_multiple_of(sb_size4) {
            self.remain_hits = unit_count.max(4);
            self.unit_hits = 0;
        } else if mi_row.is_multiple_of(unit_size4) && mi_col.is_multiple_of(unit_size4) {
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
            let bw = block.bw4 as i32 * MI_SIZE;
            let bh = block.bh4 as i32 * MI_SIZE;
            if [candidate.mv0, candidate.mv1].into_iter().any(|mv| {
                let ref_y = block.mi_row as i32 * MI_SIZE + mv.row / 8;
                let ref_x = block.mi_col as i32 * MI_SIZE + mv.col / 8;
                ref_x <= -bw
                    || ref_y <= -bh
                    || ref_x >= block.mi_cols as i32 * MI_SIZE
                    || ref_y >= block.mi_rows as i32 * MI_SIZE
            }) {
                continue;
            }
            insert_compound_mv_stack_entry(
                entries,
                prune_count,
                CompoundMvCandidate {
                    mvs: [candidate.mv0, candidate.mv1],
                    cwp_weight: candidate.cwp_weight,
                },
                0,
            );
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
    while (cand_col as usize) < grid.mi_cols
        && (cand_col as usize) < sb_col + sb_size4
        && row_hits < 4
    {
        let cand_col2 = (cand_col >> 1) << 1;
        let mut step = 1i32;
        if let Some(cell) = grid.get(cand_row, cand_col2) {
            if cell.is_inter {
                row_hits += 1;
                visit(&cell);
            }
            step = (cell.bw4 as i32).max(1);
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
pub(crate) fn find_mv_stack_with_temporal(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mv: Mv,
    bank: Option<(&RefMvBank, usize)>,
    warp_bank: &WarpParamBank,
    derive_wrl: bool,
    drl_reorder: DrlReorder,
    temporal: Option<&TemporalMvContext>,
    order_hints: Option<OrderHintMvContext<'_>>,
    use_temporal_first: bool,
) -> MvStack {
    let mut entries = FixedStack::new();
    let mut prune_count = 0usize;
    let mut derived = DerivedMvState::new(temporal, order_hints);
    let mut warp = derive_wrl.then(WarpParamStack::new);
    if let Some(warp) = warp.as_mut() {
        generate_points_from_corners(grid, block, 0, warp);
        if warp.num_found == 0 && block.bw4 <= 16 {
            generate_points_from_corners(grid, block, 1, warp);
        }
    }
    if use_temporal_first {
        scan_temporal_mv_stack(block, temporal, &mut entries, &mut prune_count);
    }

    let probes = mv_stack_spatial_probes(block);
    for probe in probes.iter().take(6).copied().flatten() {
        scan_mv_stack_probe(
            grid,
            block,
            probe,
            &mut entries,
            &mut prune_count,
            &mut derived,
            warp.as_mut(),
        );
    }
    if !use_temporal_first {
        scan_temporal_mv_stack(block, temporal, &mut entries, &mut prune_count);
    }
    if let Some(probe) = probes[6] {
        scan_mv_stack_probe(
            grid,
            block,
            probe,
            &mut entries,
            &mut prune_count,
            &mut derived,
            warp.as_mut(),
        );
    }

    let num_nearest = entries.len();
    scan_mv_stack_col(
        grid,
        block,
        -3,
        &mut entries,
        &mut prune_count,
        &mut derived,
        warp.as_mut(),
    );
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
            let _ = entries.try_swap(0, max_idx);
        }
    }
    let max_ref_mv_count = bank.map_or(MAX_REF_MV_STACK_SIZE, |(_, count)| count);
    if let Some((bank, _)) = bank {
        bank.fill(block, &mut entries, max_ref_mv_count, &mut prune_count);
    }
    derived.fill(&mut entries, max_ref_mv_count, &mut prune_count);
    extra_search(block, global_mv, &mut entries, &mut prune_count);
    if let Some(warp) = warp.as_mut() {
        warp_bank.fill(block.ref_frame0, warp);
        for _ in 0..3 {
            warp.insert(DEFAULT_WARP_PARAMS);
        }
    }

    let stack = FixedStack::from_entries(
        entries
            .iter()
            .map(|entry| (clamp_mv(block, entry.mv), entry.offsets)),
    );

    MvStack {
        stack,
        warp: warp.unwrap_or_else(WarpParamStack::new),
        block: *block,
    }
}

pub(crate) fn find_compound_mv_stack_with_temporal(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mvs: [Mv; 2],
    bank: Option<(&RefMvBank, usize)>,
    drl_reorder: DrlReorder,
    temporal: Option<&TemporalMvContext>,
) -> CompoundMvStack {
    let mut state = CompoundScanState::new();
    let probes = mv_stack_spatial_probes(block);
    for probe in probes.iter().take(6).copied().flatten() {
        scan_compound_mv_stack_probe(grid, block, probe, temporal, &mut state);
    }
    scan_compound_temporal_mv_stack(block, temporal, &mut state);
    if let Some(probe) = probes[6] {
        scan_compound_mv_stack_probe(grid, block, probe, temporal, &mut state);
    }

    let num_nearest = state.entries.len();
    scan_compound_mv_stack_col(grid, block, -3, temporal, &mut state);
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
            let _ = state.entries.try_swap(0, max_idx);
        }
    }
    let max_ref_mv_count = bank.map_or(MAX_REF_MV_STACK_SIZE, |(_, count)| count);
    state
        .derived
        .fill(&mut state.entries, max_ref_mv_count, &mut state.prune_count);
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
    let stack = FixedStack::from_entries(state.entries.iter().map(|entry| CompoundMvCandidate {
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
        let Some(cell) = grid.get(mv_row, mv_col) else {
            continue;
        };
        let Some(left) = grid.get(mv_row, block.mi_col as i32 - 1) else {
            continue;
        };
        if cell.base_c != left.base_c {
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
    let Some((cell, weight)) = probe.stack_cell(grid, block) else {
        return;
    };
    let Some(ref_frame1) = block.ref_frame1 else {
        return;
    };
    if !cell.is_inter {
        return;
    }
    let candidate = if cell.ref_frame0 == block.ref_frame0 && cell.ref_frame1 == Some(ref_frame1) {
        CompoundMvCandidate {
            mvs: [cell.sub_mv, cell.sub_mv1],
            cwp_weight: cell.cwp_weight,
        }
    } else if cell.ref_frame0 == TIP_REF_FRAME {
        let Some(temporal) = temporal else {
            return;
        };
        let Some(tip_refs) = temporal.tip_references() else {
            return;
        };
        if cell.ref_frame0 != TIP_REF_FRAME
            || cell.ref_frame1.is_some()
            || (tip_refs.past_ref, tip_refs.future_ref) != (block.ref_frame0, ref_frame1)
        {
            return;
        }
        let Some(mvs) = temporal.tip_spatial_mvs(grid, block, probe, cell) else {
            return;
        };
        CompoundMvCandidate {
            mvs,
            cwp_weight: CWP_EQUAL,
        }
    } else {
        state.derived.add_spatial(
            block,
            [
                Some((cell.ref_frame0, cell.sub_mv)),
                cell.ref_frame1.map(|r| (r, cell.sub_mv1)),
            ],
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
        if mv_row >= block.mi_rows || mv_col >= block.mi_cols {
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
        let Some(cell) = grid.get(mv_row, mv_col) else {
            continue;
        };
        let Some(left) = grid.get(mv_row, block.mi_col as i32 - 1) else {
            continue;
        };
        if cell.base_c == left.base_c {
            continue;
        }
        scan_mv_stack_probe(
            grid,
            block,
            RelativeProbe::new(delta_row, delta_col),
            entries,
            prune_count,
            derived,
            warp.as_deref_mut(),
        );
    }
}

fn scan_mv_stack_probe(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    probe: RelativeProbe,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
    derived: &mut DerivedMvState<'_>,
    warp: Option<&mut WarpParamStack>,
) {
    let Some((cell, weight)) = probe.stack_cell(grid, block) else {
        return;
    };

    if let Some(warp) = warp {
        warp.add_scan_point(cell, block);
    }

    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        return;
    }

    let (_, _, adjusted_delta_col) = probe.stack_target(block);
    let offsets = (probe.delta_row, adjusted_delta_col);
    if cell.ref_frame0 == TIP_REF_FRAME
        && cell.ref_frame1.is_none()
        && block.ref_frame0 != TIP_REF_FRAME
        && let Some(temporal) = derived.temporal()
        && let Some(candidates) = temporal.tip_spatial_single_candidates(grid, block, probe, cell)
    {
        for (candidate_ref, candidate_mv) in candidates {
            if candidate_ref == block.ref_frame0 {
                insert_mv_stack_entry(entries, prune_count, candidate_mv, weight, offsets);
            } else {
                derived.add_spatial(block, candidate_ref, candidate_mv, cell);
            }
        }
    }
    let candidates = [
        Some((cell.ref_frame0, cell.sub_mv)),
        cell.ref_frame1.map(|ref_frame1| (ref_frame1, cell.sub_mv1)),
    ];
    for (candidate_ref, candidate_mv) in candidates.into_iter().flatten() {
        if candidate_ref == block.ref_frame0 {
            insert_mv_stack_entry(entries, prune_count, candidate_mv, weight, offsets);
        } else if candidate_ref >= 0 && candidate_ref != TIP_REF_FRAME {
            derived.add_spatial(block, candidate_ref, candidate_mv, cell);
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
    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        return StackInsert::Skipped;
    }
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
    block: &MvBlockContext,
    temporal: &TemporalMvContext,
    delta_row: usize,
    delta_col: usize,
    entries: &mut FixedStack<MvStackEntry, MAX_REF_MV_STACK_SIZE>,
    prune_count: &mut usize,
) -> StackInsert {
    let mv_row = block.mi_row.saturating_add(delta_row);
    let mv_col = block.mi_col.saturating_add(delta_col);
    if mv_row >= block.mi_rows || mv_col >= block.mi_cols {
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
    iter: i32,
    warp: &mut WarpParamStack,
) {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let mut pts = [[0i64; 2]; 3];
    let mut mvs = [[0i64; 2]; 3];
    let mut found = 0usize;
    for (delta_row, delta_col, adjust_col) in
        [(-1, -1, iter), (-1, bw4 - 1, iter), (bh4 - 1, -1, 0)]
    {
        warp_corner(
            grid, block, delta_row, delta_col, adjust_col, &mut pts, &mut mvs, &mut found,
        );
    }
    if found != 3 {
        return;
    }
    let mut ref_pts = [[0i64; 2]; 3];
    let mut all_mvs_same = true;
    for n in 0..3 {
        for c in 0..2 {
            ref_pts[n][c] =
                (pts[n][c] << WARPEDMODEL_PREC_BITS) + (mvs[n][c] << GM_TRANS_ONLY_PREC_DIFF);
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
    let mut wmmat = [0i64; 6];
    wmmat[2] = (ref_pts[1][1] - ref_pts[0][1]) >> width_log2;
    wmmat[4] = (ref_pts[1][0] - ref_pts[0][0]) >> width_log2;
    wmmat[3] = (ref_pts[2][1] - ref_pts[0][1]) >> height_log2;
    wmmat[5] = (ref_pts[2][0] - ref_pts[0][0]) >> height_log2;
    let wmmat0 = ref_pts[0][1] - wmmat[2] * x0 - wmmat[3] * y0;
    let wmmat1 = ref_pts[0][0] - wmmat[4] * x0 - wmmat[5] * y0;
    reduce_warp_model(&mut wmmat);
    let high = WARPEDMODEL_TRANS_CLAMP - (1 << WARP_PARAM_REDUCE_BITS);
    wmmat[0] = wmmat0.clamp(-WARPEDMODEL_TRANS_CLAMP, high);
    wmmat[1] = wmmat1.clamp(-WARPEDMODEL_TRANS_CLAMP, high);
    warp.insert(wmmat);
}

#[allow(clippy::too_many_arguments)]
fn warp_corner(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_row: i32,
    delta_col: i32,
    adjust_col: i32,
    pts: &mut [[i64; 2]; 3],
    mvs: &mut [[i64; 2]; 3],
    found: &mut usize,
) {
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
        return;
    }
    let Some(cell) = grid.get(mv_row, mv_col2) else {
        return;
    };
    if !cell.is_inter || *found >= 3 {
        return;
    }
    for ref_list in 0..2usize {
        let ref_matches = if ref_list == 0 {
            cell.ref_frame0 == block.ref_frame0
        } else {
            cell.ref_frame1 == Some(block.ref_frame0)
        };
        if !ref_matches {
            continue;
        }
        let corner_mv = if cell.is_warp() {
            if ref_list > 0 {
                return;
            }
            let Some(params) = cell.warp_params else {
                return;
            };
            warp_motion_vector_at(params, block, mv_row + 1, mv_col + 1)
        } else if ref_list == 0 {
            cell.sub_mv
        } else {
            cell.sub_mv1
        };
        pts[*found] = [i64::from(mv_row + 1) * 4, i64::from(mv_col + 1) * 4];
        mvs[*found] = [i64::from(corner_mv.row), i64::from(corner_mv.col)];
        *found += 1;
        return;
    }
}

pub(crate) fn warp_sub_mv_at(
    params: [i64; 6],
    block_r: usize,
    block_c: usize,
    rr: usize,
    cc: usize,
) -> Mv {
    let i8 = (rr.saturating_sub(block_r)) >> 1;
    let j8 = (cc.saturating_sub(block_c)) >> 1;
    let src_x = (block_c * 4 + j8 * 8 + 4) as i64;
    let src_y = (block_r * 4 + i8 * 8 + 4) as i64;
    let dst_x = params[2] * src_x + params[3] * src_y + params[0];
    let dst_y = params[4] * src_x + params[5] * src_y + params[1];
    let bound = (1i64 << 16) - 1;
    Mv {
        row: round2_signed(
            dst_y - (src_y << WARPEDMODEL_PREC_BITS),
            WARPEDMODEL_PREC_BITS - 3,
        )
        .clamp(-bound, bound) as i32,
        col: round2_signed(
            dst_x - (src_x << WARPEDMODEL_PREC_BITS),
            WARPEDMODEL_PREC_BITS - 3,
        )
        .clamp(-bound, bound) as i32,
    }
}

fn warp_motion_vector_at(
    params: [i64; 6],
    block: &MvBlockContext,
    pos_row: i32,
    pos_col: i32,
) -> Mv {
    let y = i64::from(pos_row) * 4;
    let x = i64::from(pos_col) * 4;
    let xc = (params[2] * x + params[3] * y + params[0]) - (x << WARPEDMODEL_PREC_BITS);
    let yc = (params[4] * x + params[5] * y + params[1]) - (y << WARPEDMODEL_PREC_BITS);
    clip_and_clamp_projected_mv(
        block,
        round2_signed(yc, WARPEDMODEL_PREC_BITS - 3),
        round2_signed(xc, WARPEDMODEL_PREC_BITS - 3),
    )
}

fn clip_and_clamp_projected_mv(block: &MvBlockContext, row: i64, col: i64) -> Mv {
    let bound = (1i64 << 16) - 1;
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

pub(crate) fn reduce_warp_model(params: &mut [i64; 6]) {
    let max_value = (1i64 << (WARPEDMODEL_PREC_BITS - 1)) - (1i64 << WARP_PARAM_REDUCE_BITS);
    let min_value = -max_value;
    for (index, param) in params.iter_mut().enumerate().skip(2) {
        let offset = if index == 2 || index == 5 {
            1i64 << WARPEDMODEL_PREC_BITS
        } else {
            0
        };
        let original = *param - offset;
        let clamped = original.clamp(min_value, max_value);
        *param =
            (round2_signed(clamped, WARP_PARAM_REDUCE_BITS) << WARP_PARAM_REDUCE_BITS) + offset;
    }
}

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
        row: mv.row.clamp(row_low, row_high),
        col: mv.col.clamp(col_low, col_high),
    }
}

#[cfg(test)]
mod tests;
