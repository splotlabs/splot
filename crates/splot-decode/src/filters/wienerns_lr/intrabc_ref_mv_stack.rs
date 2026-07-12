// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.12.2 IntrABC (IBC) reference-block-vector stack derivation for the
//! decoder.
//!
//! For an IntrABC block (`use_intrabc == 1`, single prediction, reference frame
//! `INTRA_FRAME`), AV2 § 7.12.2 Find MV stack process builds `RefStackMv` in this
//! order (the steps that can contribute for IBC, grounded in the committed mirror
//! `docs/spec/av2/1.0.0/07-decoding-process.md` and AVM
//! `av2/common/mvref_common.c`):
//!
//! 1. The spatial SMVP scan (§ 7.12.2.6 Scan point / § 7.12.2.5 Scan col,
//!    `scan_blk_mbmi` -> `add_ref_mv_candidate`, `mvref_common.c:1491` /
//!    `:820`). For `is_intrabc == 1` only neighbours that are themselves IntrABC
//!    blocks contribute, with dedup-by-value.
//! 2. The ref-MV-bank fill (§ 7.12.2.21, `add_ref_mv_bank_candidates` ->
//!    `check_rmb_cand`, `mvref_common.c:1943` / `:1806`): the bank is iterated in
//!    reverse (LIFO), each candidate deduped against the stack and rejected if its
//!    displaced reference leaves the frame boundary
//!    (`mvref_common.c:1828`-`1832`).
//! 3. The default-BVP extra search (§ 7.12.2.20, the `use_intrabc` `add_to_ref_bv`
//!    tail, `mvref_common.c:2711`-`2732`): the four default block vectors fill the
//!    remaining slots with NO dedup, up to `max_bvp_drl_bits_minus_1 + 2`.
//!
//! The bank itself is maintained by AV2 § 7.12.2 `av2_update_ref_mv_bank`:
//! intra-only frames zero it at each superblock row, while inter frames seed up
//! to four eligible blocks from the preceding row at every superblock entry
//! (`av2_common_int.h:4283`), then accumulate decoded IntrABC block vectors under
//! the `decide_rmb_unit_update_count` per-unit budget (`mvref_common.c:4589`).
//!
//! This module models the spatial scan, bank fill, and default-BVP fill, and
//! DEFERS (returns no admissible stack) only when `ref_mv_idx` selects beyond
//! the derived stack.

use crate::prediction::inter::Mv;

const REF_MV_BANK_SIZE: usize = 4;
const MAX_SMVP_AXIS_MI: usize = 16;
const MAX_RMB_SB_HITS: u32 = 64;
pub(crate) const BANK_SB_ABOVE_ROW_MAX_HITS: usize = 4;
const BANK_1ST_UNIT_UPDATE_COUNT: u32 = 4;
const BANK_UNIT_MAX_ALLOWED_LEFTOVER_UPDATES: u32 = 16;
const SB_TO_RMB_UNITS_LOG2: u32 = 3;
const MI_SIZE: i32 = 4;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlockWidthType {
    Width4,
    Width8,
    Others,
}

impl BlockWidthType {
    const fn from_bw4(bw4: usize) -> Self {
        match bw4 {
            1 => Self::Width4,
            2 => Self::Width8,
            _ => Self::Others,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcRefMvBank {
    mib_size: usize,
    queue: Vec<Mv>,
    remain_hits: u32,
    unit_hits: u32,
    sb_hits: u32,
    current_sb_row: Option<usize>,
    current_sb_col: Option<usize>,
}

impl IntrabcRefMvBank {
    pub(crate) const fn new(mib_size: usize) -> Self {
        Self {
            mib_size,
            queue: Vec::new(),
            remain_hits: 0,
            unit_hits: 0,
            sb_hits: 0,
            current_sb_row: None,
            current_sb_col: None,
        }
    }

    pub(crate) fn entries(&self) -> &[Mv] {
        &self.queue
    }

    pub(crate) fn enter_block_superblock(&mut self, mi_row: usize, mi_col: usize) -> bool {
        if self.mib_size == 0 {
            return false;
        }
        let sb_row = mi_row / self.mib_size;
        let sb_col = mi_col / self.mib_size;
        let sb_row_start = sb_row * self.mib_size;
        let sb_col_start = sb_col * self.mib_size;
        let entered =
            self.current_sb_row != Some(sb_row_start) || self.current_sb_col != Some(sb_col_start);
        if self.current_sb_row != Some(sb_row_start) {
            self.queue.clear();
            self.remain_hits = 0;
            self.unit_hits = 0;
            self.sb_hits = 0;
            self.current_sb_row = Some(sb_row_start);
            self.current_sb_col = Some(sb_col_start);
        } else if self.current_sb_col != Some(sb_col_start) {
            self.sb_hits = 0;
            self.remain_hits = 0;
            self.unit_hits = 0;
            self.current_sb_col = Some(sb_col_start);
        }
        entered
    }

    pub(crate) fn seed_from_above_row(&mut self, mv: Option<Mv>) {
        if self.sb_hits >= MAX_RMB_SB_HITS {
            return;
        }
        self.sb_hits += 1;
        if let Some(mv) = mv {
            self.append_or_move_to_end(mv);
        }
    }

    fn decide_unit_update_count(&mut self, mi_row: usize, mi_col: usize, n4w: usize, n4h: usize) {
        if self.mib_size == 0 {
            return;
        }
        let mib_size_log2 = self.mib_size.trailing_zeros();
        let rmb_unit_mi_size_log2 = mib_size_log2.saturating_sub(SB_TO_RMB_UNITS_LOG2);
        let rmb_unit_mi_size = 1usize << rmb_unit_mi_size_log2;
        let mi_row_in_sb = mi_row % self.mib_size;
        let mi_col_in_sb = mi_col % self.mib_size;
        let units_w = (n4w >> rmb_unit_mi_size_log2).max(1);
        let units_h = (n4h >> rmb_unit_mi_size_log2).max(1);
        let rmb_units_count = u32::try_from(units_w.saturating_mul(units_h)).unwrap_or(u32::MAX);
        if mi_row_in_sb == 0 && mi_col_in_sb == 0 {
            self.remain_hits = rmb_units_count.max(BANK_1ST_UNIT_UPDATE_COUNT);
            self.unit_hits = 0;
        } else if mi_row_in_sb.is_multiple_of(rmb_unit_mi_size)
            && mi_col_in_sb.is_multiple_of(rmb_unit_mi_size)
        {
            self.remain_hits = self.remain_hits.saturating_add(rmb_units_count);
            self.unit_hits = 0;
        }
    }

    pub(crate) fn update_after_block(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        use_intrabc: bool,
        block_mv: Option<Mv>,
    ) {
        self.decide_unit_update_count(mi_row, mi_col, n4w, n4h);
        if let (true, Some(mv)) = (use_intrabc, block_mv) {
            self.update_within_sb(mv);
        }
    }

    fn update_within_sb(&mut self, mv: Mv) {
        if self.remain_hits == 0
            || self.unit_hits >= BANK_UNIT_MAX_ALLOWED_LEFTOVER_UPDATES
            || self.sb_hits >= MAX_RMB_SB_HITS
        {
            return;
        }
        self.remain_hits -= 1;
        self.unit_hits += 1;
        self.sb_hits += 1;
        self.append_or_move_to_end(mv);
    }

    fn append_or_move_to_end(&mut self, mv: Mv) {
        if let Some(pos) = self.queue.iter().position(|&entry| entry == mv) {
            let entry = self.queue.remove(pos);
            self.queue.push(entry);
            return;
        }
        if self.queue.len() < REF_MV_BANK_SIZE {
            self.queue.push(mv);
        } else {
            self.queue.remove(0);
            self.queue.push(mv);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RmbCandBounds {
    mi_row: i32,
    mi_col: i32,
    block_w: i32,
    block_h: i32,
    frame_w: i32,
    frame_h: i32,
}

fn check_rmb_cand(cand: Mv, stack: &[Mv], bounds: RmbCandBounds) -> bool {
    if stack.contains(&cand) {
        return false;
    }
    let ref_x = bounds.mi_col * MI_SIZE + cand.col / 8;
    let ref_y = bounds.mi_row * MI_SIZE + cand.row / 8;
    if ref_x <= -bounds.block_w
        || ref_y <= -bounds.block_h
        || ref_x >= bounds.frame_w
        || ref_y >= bounds.frame_h
    {
        return false;
    }
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntrabcStackGeometry {
    pub(crate) mi_row: usize,
    pub(crate) mi_col: usize,
    pub(crate) n4w: usize,
    pub(crate) n4h: usize,
    pub(crate) sb_samples: i32,
    pub(crate) frame_w: i32,
    pub(crate) frame_h: i32,
    pub(crate) max_bvp_drl_bits_minus_1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SpatialScanGeometry {
    pub(crate) mi_row: usize,
    pub(crate) mi_col: usize,
    pub(crate) n4w: usize,
    pub(crate) n4h: usize,
    pub(crate) mi_rows: usize,
    pub(crate) mi_cols: usize,
    pub(crate) sb_size4: usize,
}

const ADJACENT_SMVP_WEIGHT: u16 = 1;
const OTHER_SMVP_WEIGHT: u16 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WeightedBv {
    pub(crate) mv: Mv,
    pub(crate) weight: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpatialIntrabcScan {
    pub(crate) candidates: Vec<WeightedBv>,
    pub(crate) nearest_len: usize,
}

pub(crate) fn spatial_intrabc_scan_with_base_col(
    geometry: SpatialScanGeometry,
    lookup: impl Fn(usize, usize) -> Option<Mv>,
    is_coded: impl Fn(usize, usize) -> bool,
    block_base_col: impl Fn(usize, usize) -> Option<usize>,
) -> SpatialIntrabcScan {
    let row = geometry.mi_row;
    let col = geometry.mi_col;
    let bh4 = geometry.n4h;

    let mut candidates: Vec<WeightedBv> = Vec::new();
    let above = AboveRowScan::resolve(&geometry, &is_coded);

    if let Some(left_col) = col.checked_sub(1)
        && let Some(r) = row.checked_add(bh4.saturating_sub(1))
    {
        push_deduped(
            &geometry,
            &lookup,
            &mut candidates,
            r,
            left_col,
            ADJACENT_SMVP_WEIGHT,
        );
    }
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step8,
        ADJACENT_SMVP_WEIGHT,
    );
    if bh4 >= 2
        && let Some(left_col) = col.checked_sub(1)
    {
        push_deduped(
            &geometry,
            &lookup,
            &mut candidates,
            row,
            left_col,
            ADJACENT_SMVP_WEIGHT,
        );
    }
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step10,
        ADJACENT_SMVP_WEIGHT,
    );
    if bh4 <= MAX_SMVP_AXIS_MI
        && let Some(left_col) = col.checked_sub(1)
        && let Some(r) = row.checked_add(bh4)
    {
        push_deduped(
            &geometry,
            &lookup,
            &mut candidates,
            r,
            left_col,
            ADJACENT_SMVP_WEIGHT,
        );
    }
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step12,
        ADJACENT_SMVP_WEIGHT,
    );
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step14,
        OTHER_SMVP_WEIGHT,
    );
    let nearest_len = candidates.len();
    push_scan_col(&geometry, &lookup, &block_base_col, &mut candidates);

    SpatialIntrabcScan {
        candidates,
        nearest_len,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AboveRowScan {
    above_row: Option<usize>,
    step8: Option<usize>,
    step10: Option<usize>,
    step12: Option<usize>,
    step14: Option<usize>,
    is_sb_border: bool,
}

impl AboveRowScan {
    fn resolve(geometry: &SpatialScanGeometry, is_coded: &impl Fn(usize, usize) -> bool) -> Self {
        let row = geometry.mi_row;
        let above_row = row.checked_sub(1);
        let is_sb_border = geometry.sb_size4 != 0 && row.is_multiple_of(geometry.sb_size4);
        let mut scan = Self {
            above_row,
            step8: None,
            step10: None,
            step12: None,
            step14: None,
            is_sb_border,
        };
        if above_row.is_none() {
            return scan;
        }
        if is_sb_border {
            scan.resolve_sb_border(geometry);
        } else {
            scan.resolve_within_sb(geometry, is_coded);
        }
        scan
    }

    fn resolve_sb_border(&mut self, geometry: &SpatialScanGeometry) {
        let bw4 = geometry.n4w;
        let width_type = BlockWidthType::from_bw4(bw4);
        self.step8 = step8_above_row_column(geometry);
        if width_type == BlockWidthType::Others {
            self.step10 = sb_border_above_col(geometry, 0);
        }
        if bw4 <= MAX_SMVP_AXIS_MI {
            let raw = i64::try_from(bw4.max(2)).unwrap_or(i64::MAX);
            self.step12 = sb_border_above_col(geometry, raw);
        }
        self.step14 = sb_border_above_col(geometry, -2);
    }

    fn resolve_within_sb(
        &mut self,
        geometry: &SpatialScanGeometry,
        is_coded: &impl Fn(usize, usize) -> bool,
    ) {
        let col = geometry.mi_col;
        let bw4 = geometry.n4w;
        self.step8 = tile_above_col(geometry, col.checked_add(bw4.saturating_sub(1)));
        if BlockWidthType::from_bw4(bw4) != BlockWidthType::Width4 {
            self.step10 = tile_above_col(geometry, Some(col));
        }
        if bw4 <= MAX_SMVP_AXIS_MI
            && self.has_top_right(geometry, is_coded)
            && let Some(c) = tile_above_col(geometry, col.checked_add(bw4))
        {
            self.step12 = Some(c);
        }
        self.step14 = col
            .checked_sub(1)
            .and_then(|c| tile_above_col(geometry, Some(c)));
    }

    fn has_top_right(
        &self,
        geometry: &SpatialScanGeometry,
        is_coded: &impl Fn(usize, usize) -> bool,
    ) -> bool {
        let Some(above_row) = self.above_row else {
            return false;
        };
        let sb_size4 = geometry.sb_size4;
        if sb_size4 == 0 {
            return false;
        }
        let mask_col = geometry.mi_col % sb_size4;
        let tr_mask_col = mask_col + geometry.n4w;
        if tr_mask_col >= sb_size4 {
            return false;
        }
        let Some(tr_col) = geometry.mi_col.checked_add(geometry.n4w) else {
            return false;
        };
        if above_row >= geometry.mi_rows || tr_col >= geometry.mi_cols {
            return false;
        }
        is_coded(above_row, tr_col)
    }
}

fn tile_above_col(geometry: &SpatialScanGeometry, col: Option<usize>) -> Option<usize> {
    let col = col?;
    if geometry.mi_row == 0 || col >= geometry.mi_cols {
        return None;
    }
    Some(col)
}

fn sb_border_above_col(geometry: &SpatialScanGeometry, raw: i64) -> Option<usize> {
    if geometry.mi_row == 0 {
        return None;
    }
    let aligned_base = i64::try_from((geometry.mi_col >> 1) << 1).ok()?;
    let aligned = aligned_base.checked_add(raw)?;
    let col = usize::try_from(aligned).ok()?;
    if col >= geometry.mi_cols {
        return None;
    }
    Some(col)
}

fn push_above_probe(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    candidates: &mut Vec<WeightedBv>,
    col: Option<usize>,
    weight: u16,
) {
    if let (Some(above), Some(c)) = (geometry.mi_row.checked_sub(1), col) {
        push_deduped(geometry, lookup, candidates, above, c, weight);
    }
}

fn step8_above_row_column(geometry: &SpatialScanGeometry) -> Option<usize> {
    let row = geometry.mi_row;
    if row == 0 {
        return None;
    }
    if geometry.sb_size4 == 0 || !row.is_multiple_of(geometry.sb_size4) {
        return None;
    }
    let delta_col = geometry.n4w.saturating_sub(2);
    sb_border_above_col(geometry, i64::try_from(delta_col).ok()?)
}

fn push_deduped(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    candidates: &mut Vec<WeightedBv>,
    row_offset: usize,
    left_col: usize,
    weight: u16,
) {
    if let Some(mv) = lookup_in_grid(geometry, lookup, row_offset, left_col) {
        if let Some(existing) = candidates.iter_mut().find(|entry| entry.mv == mv) {
            existing.weight = existing.weight.saturating_add(weight);
        } else {
            candidates.push(WeightedBv { mv, weight });
        }
    }
}

fn push_scan_col(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    block_base_col: &impl Fn(usize, usize) -> Option<usize>,
    candidates: &mut Vec<WeightedBv>,
) {
    let mut delta_col = -3i64;
    if geometry.n4w == 1 {
        delta_col += i64::try_from(geometry.mi_col & 1).unwrap_or(0);
    }
    if let Some(bottom_row) = geometry.mi_row.checked_add(geometry.n4h.saturating_sub(1)) {
        push_scan_col_point(
            geometry,
            lookup,
            block_base_col,
            candidates,
            bottom_row,
            delta_col,
        );
    }
    if geometry.n4h > 1 {
        push_scan_col_point(
            geometry,
            lookup,
            block_base_col,
            candidates,
            geometry.mi_row,
            delta_col,
        );
    }
}

fn push_scan_col_point(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    block_base_col: &impl Fn(usize, usize) -> Option<usize>,
    candidates: &mut Vec<WeightedBv>,
    mv_row: usize,
    delta_col: i64,
) {
    let Some(mv_other_col) = geometry.mi_col.checked_sub(1) else {
        return;
    };
    let Some(mv_col_i64) = i64::try_from(geometry.mi_col)
        .ok()
        .and_then(|col| col.checked_add(delta_col))
    else {
        return;
    };
    let Ok(mv_col) = usize::try_from(mv_col_i64) else {
        return;
    };
    if mv_row >= geometry.mi_rows || mv_col >= geometry.mi_cols {
        return;
    }
    let Some(candidate_base) = block_base_col(mv_row, mv_col) else {
        return;
    };
    let Some(other_base) = block_base_col(mv_row, mv_other_col) else {
        return;
    };
    if candidate_base == other_base {
        return;
    }
    push_deduped(
        geometry,
        lookup,
        candidates,
        mv_row,
        mv_col,
        OTHER_SMVP_WEIGHT,
    );
}

fn lookup_in_grid(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    row: usize,
    col: usize,
) -> Option<Mv> {
    if row >= geometry.mi_rows || col >= geometry.mi_cols {
        return None;
    }
    lookup(row, col)
}

pub(crate) fn build_intrabc_ref_mv_stack(
    bank: &IntrabcRefMvBank,
    geometry: IntrabcStackGeometry,
    enable_refmvbank: bool,
    spatial: &[Mv],
) -> Vec<Mv> {
    let max_count = usize::try_from(geometry.max_bvp_drl_bits_minus_1)
        .ok()
        .and_then(|bits| bits.checked_add(2))
        .unwrap_or(REF_MV_BANK_SIZE);
    let block_w = i32::try_from(geometry.n4w.saturating_mul(4)).unwrap_or(i32::MAX);
    let block_h = i32::try_from(geometry.n4h.saturating_mul(4)).unwrap_or(i32::MAX);
    let mi_row = i32::try_from(geometry.mi_row).unwrap_or(i32::MAX);
    let mi_col = i32::try_from(geometry.mi_col).unwrap_or(i32::MAX);

    let bounds = RmbCandBounds {
        mi_row,
        mi_col,
        block_w,
        block_h,
        frame_w: geometry.frame_w,
        frame_h: geometry.frame_h,
    };
    let mut stack: Vec<Mv> = Vec::new();

    for &cand in spatial {
        if stack.len() >= max_count {
            break;
        }
        stack.push(cand);
    }

    if enable_refmvbank {
        for &cand in bank.entries().iter().rev() {
            if stack.len() >= max_count {
                break;
            }
            if check_rmb_cand(cand, &stack, bounds) {
                stack.push(cand);
            }
        }
    }

    let sb = geometry.sb_samples;
    let w = block_w;
    let h = block_h;
    let defaults = [
        add_to_ref_bv(0, -sb),
        add_to_ref_bv(-sb - INTRABC_DELAY_PIXELS, 0),
        add_to_ref_bv(0, -h),
        add_to_ref_bv(-w, 0),
    ];
    for mv in defaults {
        if stack.len() >= max_count {
            break;
        }
        stack.push(mv);
    }
    stack
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DrlReorderMode {
    Disabled,
    Constraint,
    Always,
}

impl DrlReorderMode {
    pub(crate) const fn use_sort(self, nearest: usize) -> bool {
        match self {
            Self::Disabled => false,
            Self::Constraint => nearest >= 4,
            Self::Always => true,
        }
    }
}

pub(crate) fn sort_nearest_max_weight_to_slot0(candidates: &mut [WeightedBv]) {
    let Some((first, rest)) = candidates.split_first() else {
        return;
    };
    let mut max_weight = first.weight;
    let mut max_idx = 0usize;
    for (offset, entry) in rest.iter().enumerate() {
        if entry.weight > max_weight {
            max_weight = entry.weight;
            max_idx = offset + 1;
        }
    }
    if max_idx != 0 {
        candidates.swap(0, max_idx);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntrabcStackAdmission {
    Admit { selected: Mv },
    Defer,
}

pub(crate) fn intrabc_ref_stack_admission(
    bank: &IntrabcRefMvBank,
    geometry: IntrabcStackGeometry,
    spatial: &SpatialIntrabcScan,
    enable_refmvbank: bool,
    drl_reorder: DrlReorderMode,
    ref_mv_idx: usize,
) -> IntrabcStackAdmission {
    let mut ordered: Vec<WeightedBv> = spatial.candidates.clone();
    let nearest_len = spatial.nearest_len.min(ordered.len());
    if drl_reorder.use_sort(nearest_len) && nearest_len > 1 {
        sort_nearest_max_weight_to_slot0(&mut ordered[..nearest_len]);
    }
    let sorted: Vec<Mv> = ordered.iter().map(|entry| entry.mv).collect();
    let real_stack = build_intrabc_ref_mv_stack(bank, geometry, enable_refmvbank, &sorted);
    match real_stack.get(ref_mv_idx).copied() {
        Some(selected) => IntrabcStackAdmission::Admit { selected },
        None => IntrabcStackAdmission::Defer,
    }
}

const INTRABC_DELAY_PIXELS: i32 = 256;

const fn add_to_ref_bv(dx: i32, dy: i32) -> Mv {
    Mv {
        row: dy * 8,
        col: dx * 8,
    }
}

#[cfg(test)]
#[path = "intrabc_ref_mv_stack_tests.rs"]
mod tests;
