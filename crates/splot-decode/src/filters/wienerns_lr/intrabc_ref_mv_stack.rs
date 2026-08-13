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
//!    blocks contribute, with dedup-by-value while the block's § 7.12.2
//!    `PruneCount` stays below `MAX_PR_NUM`; every comparison spends the budget
//!    and once it is exhausted candidates append without the duplicate scan.
//! 2. The ref-MV-bank fill (§ 7.12.2.21, `add_ref_mv_bank_candidates` ->
//!    `check_rmb_cand`, `mvref_common.c:1943` / `:1806`): the bank is iterated in
//!    reverse (LIFO), each candidate deduped under the same `PruneCount` budget
//!    and rejected — after the budget-gated scan — if its displaced reference
//!    leaves the frame boundary (`mvref_common.c:1828`-`1832`).
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
//! This module models the spatial scan, bank fill, and default-BVP fill. The
//! parser carries a bounded selection, and the four default BVPs make every
//! reachable selection total.

use super::intrabc_records::IntrabcRefSelection;
use crate::prediction::inter::{FixedStack, Mv};

const REF_MV_BANK_SIZE: usize = 4;
const MAX_SMVP_AXIS_MI: usize = 16;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RmbCandBounds {
    mi_row: i32,
    mi_col: i32,
    block_w: i32,
    block_h: i32,
    frame_w: i32,
    frame_h: i32,
}

const MAX_PR_NUM: u32 = 16;

/// Capacity of one § 7.12.2.6 / § 7.12.2.5 spatial IntrABC scan.
///
/// AV2 bounds the stack this scan feeds: § 7.12.2.12 Search stack process
/// appends only while `NumMvFound < MAX_REF_MV_STACK_SIZE`
/// (`docs/spec/av2/1.0.0/07-decoding-process.md:4085`), and § 7.12.2.6 Scan
/// point process terminates once `NumMvFound` reaches it (`:3784`), with
/// `MAX_REF_MV_STACK_SIZE` equal to 6 (`docs/spec/av2/1.0.0/03-symbols.md:514`).
/// This scan does not apply that cap yet (tracked separately), so the container
/// keeps headroom above six. The looser in-code bound is that
/// `spatial_intrabc_scan_with_base_col` reaches `merge_or_push_weighted` from
/// seven scan-point calls and two scan-col calls, each appending at most one
/// candidate.
const MAX_SPATIAL_INTRABC_CANDIDATES: usize = 12;

pub(crate) type SpatialBvStack = FixedStack<WeightedBv, MAX_SPATIAL_INTRABC_CANDIDATES>;

fn merge_or_push_weighted(
    candidates: &mut SpatialBvStack,
    mv: Mv,
    weight: u16,
    comparisons: &mut u32,
) {
    let matched = (*comparisons < MAX_PR_NUM).then(|| {
        let found = candidates.iter().position(|entry| entry.mv == mv);
        *comparisons = comparisons.saturating_add(match found {
            Some(at) => at as u32 + 1,
            None => candidates.len() as u32,
        });
        found
    });
    if let Some(Some(at)) = matched {
        candidates[at].weight = candidates[at].weight.saturating_add(weight);
    } else {
        let pushed = candidates.try_push(WeightedBv { mv, weight });
        debug_assert!(
            pushed,
            "spatial IntrABC scan exceeded MAX_SPATIAL_INTRABC_CANDIDATES"
        );
    }
}

fn budgeted_dedup_finds(stack: &[Mv], cand: Mv, comparisons: &mut u32) -> bool {
    if *comparisons >= MAX_PR_NUM {
        return false;
    }
    for (visited, entry) in stack.iter().enumerate() {
        if *entry == cand {
            *comparisons = comparisons.saturating_add(visited as u32 + 1);
            return true;
        }
    }
    *comparisons = comparisons.saturating_add(stack.len() as u32);
    false
}

fn check_rmb_cand(cand: Mv, stack: &[Mv], bounds: RmbCandBounds, comparisons: &mut u32) -> bool {
    if budgeted_dedup_finds(stack, cand, comparisons) {
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WeightedBv {
    pub(crate) mv: Mv,
    pub(crate) weight: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpatialIntrabcScan {
    pub(crate) candidates: SpatialBvStack,
    pub(crate) nearest_len: usize,
    /// § 7.12.2 `PruneCount` after the spatial scan: every duplicate-check
    /// comparison spends this budget, and once `MAX_PR_NUM` is spent later
    /// stages append without checking for duplicates.
    pub(crate) comparisons: u32,
}

/// Parse-time record of which neighbours the spatial scan visited, replayed
/// against resolved block vectors once the neighbours are decoded.
///
/// Each probe carries its grid position in `WeightedBv::mv` under the
/// `PROBE_SERIAL_SHIFT` encoding that `capture_spatial_intrabc_probes` applies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpatialIntrabcProbes {
    probes: SpatialBvStack,
    nearest_len: usize,
}

impl SpatialIntrabcProbes {
    pub(crate) fn resolve(
        &self,
        lookup: impl Fn(usize, usize) -> Option<Mv>,
    ) -> SpatialIntrabcScan {
        let mut candidates = SpatialBvStack::new();
        let mut nearest_len = 0;
        let mut comparisons = 0u32;
        for (index, probe) in self.probes.iter().enumerate() {
            if index == self.nearest_len {
                nearest_len = candidates.len();
            }
            let Some(mv) = decode_probe_position(probe.mv).and_then(|(row, col)| lookup(row, col))
            else {
                continue;
            };
            merge_or_push_weighted(&mut candidates, mv, probe.weight, &mut comparisons);
        }
        if self.nearest_len == self.probes.len() {
            nearest_len = candidates.len();
        }
        SpatialIntrabcScan {
            candidates,
            nearest_len,
            comparisons,
        }
    }
}

const PROBE_SERIAL_SHIFT: u32 = 16;
const PROBE_COL_MASK: i32 = (1 << PROBE_SERIAL_SHIFT) - 1;

fn decode_probe_position(encoded: Mv) -> Option<(usize, usize)> {
    let row = usize::try_from(encoded.row).ok()?;
    let col = usize::try_from(encoded.col & PROBE_COL_MASK).ok()?;
    Some((row, col))
}

pub(crate) fn capture_spatial_intrabc_probes(
    geometry: SpatialScanGeometry,
    is_coded: impl Fn(usize, usize) -> bool,
    block_base_col: impl Fn(usize, usize) -> Option<usize>,
) -> SpatialIntrabcProbes {
    let is_coded = &is_coded;
    let serial = core::cell::Cell::new(0i32);
    let encoded = spatial_intrabc_scan_with_base_col(
        geometry,
        |row, col| {
            if !is_coded(row, col) {
                return None;
            }
            let tag = serial.get();
            serial.set(tag.wrapping_add(1));
            Some(Mv {
                row: i32::try_from(row).ok()?,
                col: i32::try_from(col)
                    .ok()
                    .filter(|&col| col <= PROBE_COL_MASK)?
                    | (tag << PROBE_SERIAL_SHIFT),
            })
        },
        is_coded,
        block_base_col,
    );
    SpatialIntrabcProbes {
        nearest_len: encoded.nearest_len,
        probes: encoded.candidates,
    }
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

    let mut candidates = SpatialBvStack::new();
    let mut comparisons = 0u32;
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
            &mut comparisons,
        );
    }
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step8,
        &mut comparisons,
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
            &mut comparisons,
        );
    }
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step10,
        &mut comparisons,
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
            &mut comparisons,
        );
    }
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step12,
        &mut comparisons,
    );
    push_above_probe(
        &geometry,
        &lookup,
        &mut candidates,
        above.step14,
        &mut comparisons,
    );
    let nearest_len = candidates.len();
    push_scan_col(
        &geometry,
        &lookup,
        &block_base_col,
        &mut candidates,
        &mut comparisons,
    );

    SpatialIntrabcScan {
        candidates,
        nearest_len,
        comparisons,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AboveRowScan {
    above_row: Option<usize>,
    step8: Option<usize>,
    step10: Option<usize>,
    step12: Option<usize>,
    step14: Option<usize>,
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
            let raw = i32::try_from(bw4.max(2)).unwrap_or(i32::MAX);
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

fn sb_border_above_col(geometry: &SpatialScanGeometry, raw: i32) -> Option<usize> {
    if geometry.mi_row == 0 {
        return None;
    }
    let aligned_base = i32::try_from((geometry.mi_col >> 1) << 1).ok()?;
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
    candidates: &mut SpatialBvStack,
    col: Option<usize>,
    comparisons: &mut u32,
) {
    if let (Some(above), Some(c)) = (geometry.mi_row.checked_sub(1), col) {
        // AV2 § 7.12.2.6 assigns weight from the post-alignment deltaCol.
        let weight = if c < geometry.mi_col {
            OTHER_SMVP_WEIGHT
        } else {
            ADJACENT_SMVP_WEIGHT
        };
        push_deduped(geometry, lookup, candidates, above, c, weight, comparisons);
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
    sb_border_above_col(geometry, i32::try_from(delta_col).ok()?)
}

fn push_deduped(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    candidates: &mut SpatialBvStack,
    row_offset: usize,
    left_col: usize,
    weight: u16,
    comparisons: &mut u32,
) {
    if let Some(mv) = lookup_in_grid(geometry, lookup, row_offset, left_col) {
        merge_or_push_weighted(candidates, mv, weight, comparisons);
    }
}

fn push_scan_col(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    block_base_col: &impl Fn(usize, usize) -> Option<usize>,
    candidates: &mut SpatialBvStack,
    comparisons: &mut u32,
) {
    let mut delta_col = -3i32;
    if geometry.n4w == 1 {
        delta_col += i32::try_from(geometry.mi_col & 1).unwrap_or(0);
    }
    if let Some(bottom_row) = geometry.mi_row.checked_add(geometry.n4h.saturating_sub(1)) {
        push_scan_col_point(
            geometry,
            lookup,
            block_base_col,
            candidates,
            bottom_row,
            delta_col,
            comparisons,
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
            comparisons,
        );
    }
}

fn push_scan_col_point(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    block_base_col: &impl Fn(usize, usize) -> Option<usize>,
    candidates: &mut SpatialBvStack,
    mv_row: usize,
    delta_col: i32,
    comparisons: &mut u32,
) {
    let Some(mv_other_col) = geometry.mi_col.checked_sub(1) else {
        return;
    };
    let Some(mv_col_i32) = i32::try_from(geometry.mi_col)
        .ok()
        .and_then(|col| col.checked_add(delta_col))
    else {
        return;
    };
    let Ok(mv_col) = usize::try_from(mv_col_i32) else {
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
        comparisons,
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

#[cfg(test)]
pub(crate) fn build_intrabc_ref_mv_stack_from_candidates(
    bank_candidates: &[Mv],
    geometry: IntrabcStackGeometry,
    enable_refmvbank: bool,
    spatial: &[Mv],
    spatial_comparisons: u32,
) -> Vec<Mv> {
    build_intrabc_ref_mv_stack(
        bank_candidates,
        geometry,
        enable_refmvbank,
        spatial.iter().copied(),
        spatial_comparisons,
        REF_MV_BANK_SIZE,
    )
    .as_slice()
    .to_vec()
}

struct IntrabcRefMvStack {
    entries: [Mv; REF_MV_BANK_SIZE],
    len: usize,
}

impl IntrabcRefMvStack {
    const fn new() -> Self {
        Self {
            entries: [Mv { row: 0, col: 0 }; REF_MV_BANK_SIZE],
            len: 0,
        }
    }

    fn push(&mut self, mv: Mv, max_count: usize) {
        if self.len < max_count {
            self.entries[self.len] = mv;
            self.len += 1;
        }
    }

    fn as_slice(&self) -> &[Mv] {
        &self.entries[..self.len]
    }

    const fn selected(&self, selection: IntrabcRefSelection) -> Mv {
        match selection.index() {
            0 => self.entries[0],
            1 => self.entries[1],
            2 => self.entries[2],
            _ => self.entries[3],
        }
    }
}

fn build_intrabc_ref_mv_stack(
    bank_candidates: &[Mv],
    geometry: IntrabcStackGeometry,
    enable_refmvbank: bool,
    spatial: impl IntoIterator<Item = Mv>,
    spatial_comparisons: u32,
    max_count: usize,
) -> IntrabcRefMvStack {
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
    let mut stack = IntrabcRefMvStack::new();

    for cand in spatial {
        if stack.len >= max_count {
            break;
        }
        stack.push(cand, max_count);
    }

    let mut comparisons = spatial_comparisons;
    if enable_refmvbank {
        for &cand in bank_candidates {
            if stack.len >= max_count {
                break;
            }
            if check_rmb_cand(cand, stack.as_slice(), bounds, &mut comparisons) {
                stack.push(cand, max_count);
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
        if stack.len >= max_count {
            break;
        }
        stack.push(mv, max_count);
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

pub(crate) fn select_intrabc_ref_mv_from_candidates(
    bank_candidates: &[Mv],
    geometry: IntrabcStackGeometry,
    spatial: &SpatialIntrabcScan,
    enable_refmvbank: bool,
    drl_reorder: DrlReorderMode,
    selection: IntrabcRefSelection,
) -> Mv {
    let mut ordered = spatial.candidates;
    let nearest_len = spatial.nearest_len.min(ordered.len());
    if drl_reorder.use_sort(nearest_len) && nearest_len > 1 {
        sort_nearest_max_weight_to_slot0(&mut ordered[..nearest_len]);
    }
    build_intrabc_ref_mv_stack(
        bank_candidates,
        geometry,
        enable_refmvbank,
        ordered.iter().map(|entry| entry.mv),
        spatial.comparisons,
        selection.max_count(),
    )
    .selected(selection)
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
