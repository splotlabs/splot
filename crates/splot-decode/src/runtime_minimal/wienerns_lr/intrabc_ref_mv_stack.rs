// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.12.2 IntrABC (IBC) reference-block-vector stack derivation for the
//! ac0ej3 decoder frontier.
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
//! The bank itself is maintained by AV2 § 7.12.2 `av2_update_ref_mv_bank`
//! (`mvref_common.c:4617`): for an intra-only frame the bank is zeroed at the
//! start of each superblock row (`decodeframe.c:4639`) and accumulates each
//! decoded IntrABC block's block vector within the row, gated by the per-unit
//! "remain hits" budget (`decide_rmb_unit_update_count`, `mvref_common.c:4589`).
//!
//! This module models the bank + default-BVP fill exactly, and DEFERS (returns no
//! admissible stack) when the spatial scan could contribute a candidate, so a
//! wrong block vector is never produced.

use super::super::inter::Mv;

/// AVM `REF_MV_BANK_SIZE` (`av2/common/blockd.h:1787`): the IBC ref-MV bank holds
/// at most four entries.
const REF_MV_BANK_SIZE: usize = 4;
/// The AV2 § 7.12.2 `MAX_REF_BV_STACK_SIZE` cap on Num_4x4_Blocks the adjacent
/// SMVP scan reads on each axis (steps 11/12 gate `bh4 <= 16` / `bw4 <= 16`,
/// `mvref_common.c:2403`/`2456`).
const MAX_SMVP_AXIS_MI: usize = 16;
/// AVM `MAX_RMB_SB_HITS` (`av2/common/av2_common_int.h:4271`).
const MAX_RMB_SB_HITS: u32 = 64;
/// AVM `BANK_1ST_UNIT_UPDATE_COUNT` (`av2/common/mvref_common.c:4581`).
const BANK_1ST_UNIT_UPDATE_COUNT: u32 = 4;
/// AVM `BANK_UNIT_MAX_ALLOWED_LEFTOVER_UPDATES` (`av2/common/mvref_common.c:4582`).
const BANK_UNIT_MAX_ALLOWED_LEFTOVER_UPDATES: u32 = 16;
/// AVM `SB_TO_RMB_UNITS_LOG2` (`av2/common/mvref_common.c:4580`): the SB-side is
/// divided into `1 << SB_TO_RMB_UNITS_LOG2` ref-MV-bank update units per axis.
const SB_TO_RMB_UNITS_LOG2: u32 = 3;
/// AV2 § 5.9.15 `MI_SIZE` in luma samples.
const MI_SIZE: i32 = 4;
/// AVM `block_width_type` (`av2/common/mvref_common.c:2003`-`2005`): the index into
/// the `row_smvp_all_states[2][BLOCK_WIDTH_TYPES][4]` table selected by the block's
/// width in MI units (`xd->width`). Selects the per-width above-row SMVP probe
/// availability gates and column offsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlockWidthType {
    /// AVM `BLOCK_WIDTH_4` (`xd->width == 1`, a 4px-wide block).
    Width4,
    /// AVM `BLOCK_WIDTH_8` (`xd->width == 2`, an 8px-wide block).
    Width8,
    /// AVM `BLOCK_WIDTH_OTHERS` (`xd->width >= 3`, i.e. >= 16px in practice — AV2
    /// block widths are powers of two, so the next reachable width after 8px is
    /// 16px / `bw4 == 4`).
    Others,
}

impl BlockWidthType {
    /// AVM `block_width_type` derivation (`mvref_common.c:2003`-`2005`):
    /// `bw4 == 1 ? BLOCK_WIDTH_4 : (bw4 == 2 ? BLOCK_WIDTH_8 : BLOCK_WIDTH_OTHERS)`.
    const fn from_bw4(bw4: usize) -> Self {
        match bw4 {
            1 => Self::Width4,
            2 => Self::Width8,
            _ => Self::Others,
        }
    }
}

/// A tile-local AV2 § 7.12.2 IntrABC reference-MV bank (the single intra list).
///
/// For an intra-only frame the bank is zeroed at the start of each superblock row
/// and accumulates the block vectors of IntrABC blocks decoded earlier in the same
/// row, gated by the AVM per-unit "remain hits" budget. The structure mirrors AVM
/// `REF_MV_BANK` for the `REF_MV_BANK_LIST_FOR_ALL_OTHERS` list with the
/// `INTRA_FRAME` key (`av2/common/mvref_common.c:4617`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IntrabcRefMvBank {
    /// SB side length in MI units (`cm->mib_size`).
    mib_size: usize,
    /// Block vectors in insertion order (oldest first); the AVM `rmb_buffer`
    /// circular queue of up to `REF_MV_BANK_SIZE` entries, materialised oldest-first
    /// so the § 7.12.2.21 fill can iterate it in reverse (LIFO).
    queue: Vec<Mv>,
    /// `remain_hits`: the per-unit leftover budget the within-SB update consumes.
    remain_hits: u32,
    /// `rmb_unit_hits`: leftover updates spent in the current unit.
    unit_hits: u32,
    /// `rmb_sb_hits`: updates spent in the current superblock.
    sb_hits: u32,
    /// The MI row of the superblock row the bank currently covers, or `None`
    /// before the first block.
    current_sb_row: Option<usize>,
    /// The MI column of the superblock the budget counters currently cover.
    current_sb_col: Option<usize>,
}

impl IntrabcRefMvBank {
    /// Builds an empty bank for a tile whose superblocks are `mib_size` MI wide.
    pub(super) const fn new(mib_size: usize) -> Self {
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

    /// The current bank entries, oldest first (the order AVM stores them; the
    /// fill process iterates this in reverse).
    pub(super) fn entries(&self) -> &[Mv] {
        &self.queue
    }

    /// AV2 § 7.12.2 `av2_reset_refmv_bank` superblock-row reset, invoked at block
    /// ENTRY before the § 7.12.2.21 fill reads the bank: for an intra-only frame the
    /// bank is zeroed at the start of each superblock row and the per-SB budget
    /// counters reset at each superblock. Mirrors the `av2_zero(xd->ref_mv_bank)` at
    /// `decodeframe.c:4639` (run before the SB-row's blocks decode) plus the
    /// intra-only early return in `av2_reset_refmv_bank` (`av2_common_int.h:4283`).
    pub(super) fn enter_block_superblock(&mut self, mi_row: usize, mi_col: usize) {
        if self.mib_size == 0 {
            return;
        }
        let sb_row = mi_row / self.mib_size;
        let sb_col = mi_col / self.mib_size;
        let sb_row_start = sb_row * self.mib_size;
        let sb_col_start = sb_col * self.mib_size;
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
    }

    /// AV2 § 7.12.2 `decide_rmb_unit_update_count` (`mvref_common.c:4589`):
    /// updates the per-unit "remain hits" budget for a block at the superblock-
    /// relative position. Runs for EVERY block (IBC or not).
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

    /// AV2 § 7.12.2 `av2_read_mode_info`'s POST-block bank maintenance for an
    /// intra-only frame (`decodemv.c:3197`-`3205`): an IntrABC block runs the
    /// within-SB bank update (`av2_update_ref_mv_bank(.., 1, ..)`); any other block
    /// only runs `decide_rmb_unit_update_count`. The SB-row reset is performed at
    /// block ENTRY by [`Self::enter_block_superblock`] (so the § 7.12.2.21 fill
    /// reads a freshly-zeroed bank for the first block of a new SB row), NOT here.
    pub(super) fn update_after_block(
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

    /// Test-only convenience: the full per-block sequence (SB-row reset at entry
    /// then the post-block bank update), matching the decode walk's call order.
    #[cfg(test)]
    fn record_block(
        &mut self,
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        use_intrabc: bool,
        block_mv: Option<Mv>,
    ) {
        self.enter_block_superblock(mi_row, mi_col);
        self.update_after_block(mi_row, mi_col, n4w, n4h, use_intrabc, block_mv);
    }

    /// AV2 § 7.12.2 `update_ref_mv_bank` with `from_within_sb == 1`
    /// (`mvref_common.c:4623`): consume the per-unit budget gate, then dedup
    /// (move-to-end) or append the block vector. `decide_rmb_unit_update_count`
    /// has already run for this block in [`Self::update_after_block`].
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

    /// The dedup / move-to-end / append tail of AV2 § 7.12.2 `update_ref_mv_bank`.
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

/// The block / frame geometry (luma samples / MI units) the AV2 § 7.12.2.21
/// `check_rmb_cand` frame-boundary test reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RmbCandBounds {
    mi_row: i32,
    mi_col: i32,
    block_w: i32,
    block_h: i32,
    frame_w: i32,
    frame_h: i32,
}

/// AV2 § 7.12.2.21 `check_rmb_cand` (`mvref_common.c:1806`): returns `true` when a
/// candidate block vector is new (not already on the stack) AND its displaced
/// reference block stays inside the frame boundary, in which case the candidate is
/// admitted. The bank is iterated in reverse (LIFO) by the caller.
///
/// The frame-boundary test (`mvref_common.c:1828`-`1832`) is:
/// `refX <= -bw || refY <= -bh || refX >= frameWidth || refY >= frameHeight`,
/// where `refX = MiCol * MI_SIZE + (col / 8)` and `refY = MiRow * MI_SIZE +
/// (row / 8)` for a block `bw` x `bh` samples at `(MiRow, MiCol)`.
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

/// Geometry inputs to the AV2 § 7.12.2 IBC ref-MV stack derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IntrabcStackGeometry {
    /// Block MI row.
    pub(super) mi_row: usize,
    /// Block MI column.
    pub(super) mi_col: usize,
    /// Block width in 4x4 MI units.
    pub(super) n4w: usize,
    /// Block height in 4x4 MI units.
    pub(super) n4h: usize,
    /// Superblock side length in luma samples.
    pub(super) sb_samples: i32,
    /// Frame width in luma samples (`MiCols * MI_SIZE`).
    pub(super) frame_w: i32,
    /// Frame height in luma samples (`MiRows * MI_SIZE`).
    pub(super) frame_h: i32,
    /// `max_bvp_drl_bits_minus_1`.
    pub(super) max_bvp_drl_bits_minus_1: u32,
}

/// Block / tile geometry the AV2 § 7.12.2 spatial SMVP scan reads (MI units).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SpatialScanGeometry {
    /// Block MI row.
    pub(super) mi_row: usize,
    /// Block MI column.
    pub(super) mi_col: usize,
    /// Block width in 4x4 MI units (`bw4`).
    pub(super) n4w: usize,
    /// Block height in 4x4 MI units (`bh4`).
    pub(super) n4h: usize,
    /// Tile MI row count (the ac0ej3 single tile spans the frame).
    pub(super) mi_rows: usize,
    /// Tile MI column count.
    pub(super) mi_cols: usize,
    /// Superblock side length in MI units (`mib_size`).
    pub(super) sb_size4: usize,
}

/// AVM `ADJACENT_SMVP_WEIGHT` (`av2/common/mvref_common.c:107`): the § 7.12.2.6
/// weight of a modelled adjacent SMVP position (`deltaRow >= -1 && deltaCol >= -1`,
/// excluding the above-left corner).
const ADJACENT_SMVP_WEIGHT: u16 = 1;
/// AVM `OTHER_SMVP_WEIGHT` (`av2/common/mvref_common.c:108`): the § 7.12.2.6 weight
/// of the step-14 above-left corner (`deltaRow == -1 && deltaCol == -1`) and the
/// deltaCol < -1 outer-area positions.
const OTHER_SMVP_WEIGHT: u16 = 0;

/// A spatial IntrABC candidate block vector and its accumulated AV2 § 7.12.2.6
/// weight (`WeightStack` entry): the § 7.12.2.19 sort moves the max-weight nearest
/// candidate to slot 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WeightedBv {
    /// The neighbour block vector (`RefStackMv` entry).
    pub(super) mv: Mv,
    /// The accumulated § 7.12.2.6 weight (`WeightStack` entry): `1`
    /// (`ADJACENT_SMVP_WEIGHT`) per modelled adjacent placement, `0`
    /// (`OTHER_SMVP_WEIGHT`) for the step-14 above-left corner (`deltaRow == -1 &&
    /// deltaCol == -1`, or aligned `deltaCol < -1`). A dedup-by-value match ADDS the
    /// new weight to the existing entry (AVM `ref_mv_weight[index] += weight`,
    /// `av2/common/mvref_common.c:873`).
    pub(super) weight: u16,
}

/// Outcome of the AV2 § 7.12.2 spatial SMVP scan for an IntrABC block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SpatialIntrabcScan {
    /// Spatial IntrABC neighbour block vectors in AV2 § 7.12.2 step order
    /// (deduped by value, each carrying its accumulated § 7.12.2.6 weight),
    /// contributed to the ref-MV stack BEFORE the § 7.12.2.21 ref-MV-bank fill and
    /// § 7.12.2.20 default fill. The § 7.12.2.19 max-weight-to-slot-0 reorder is
    /// applied by [`intrabc_ref_stack_admission`] (threading the real
    /// `enable_drl_reorder` flag) before the stack is built.
    pub(super) candidates: Vec<WeightedBv>,
    /// `true` when an unmodelled § 7.12.2 spatial position (an above-row probe
    /// or the § 7.12.2.5 deltaCol = -3 non-adjacent scan) holds an IntrABC
    /// neighbour: the scan cannot be reproduced faithfully there, so the caller
    /// must over-reject (defer) rather than emit a desynced stack.
    pub(super) defer: bool,
}

/// AV2 § 7.12.2 spatial SMVP scan for an IntrABC block (`is_intrabc == 1`).
///
/// The adjacent SMVP search (§ 7.12.2.1 steps 7-14, `mvref_common.c:2361`-`2439`)
/// invokes § 7.12.2.6 Scan point at the left-column and above-row positions
/// `(deltaRow, deltaCol)` in this order (the IBC-relevant steps; TMVP step 13
/// contributes nothing under `INTRA_FRAME`):
///
/// * step 7: `(bh4 - 1, -1)`, gated `left_available` — bottom of the left column;
/// * step 8: above-row `row_smvp_state[0]` (`mvref_common.c:2371`);
/// * step 9 (`bh4 >= 2`): `(0, -1)` — top of the left column;
/// * step 10: above-row `row_smvp_state[1]` (`mvref_common.c:2392`);
/// * step 11: `(bh4, -1)`, gated `has_bottom_left` — below the left column;
/// * step 12: above-row `row_smvp_state[2]`, the top-right probe (`mvref_common.c:2412`);
/// * step 14: above-row `row_smvp_state[3]`, the above-left corner (`mvref_common.c:2428`).
///
/// The four above-row `row_smvp_state[i]` probes (§ 7.12.2.6 `get_row_smvp_states`,
/// `mvref_common.c:1996`-`2077`) all use `deltaRow = -1`. Their `deltaCol` and
/// availability split on whether the block sits on a horizontal superblock border:
///
/// * NOT on an SB border (`MiRow % mib_size != 0`): each probe reads the above row
///   at FULL 4x4 resolution (no 8x8 grid alignment) — `deltaCol = bw4 - 1` (step 8),
///   `0` (step 10), `bw4` (step 12), `-1` (step 14), gated by `up_available`
///   (steps 8/10), `has_top_right` (step 12), `up_available && left_available`
///   (step 14);
/// * ON an SB border: the line buffer only keeps the bottom 4x4 of each above 8x8
///   unit, so AVM aligns `MiCol` to the 8x8 grid (`compute_aligned_offset`) and
///   gates on `is_above_smvp_available` (steps 8/10/14) / `has_top_right` (step 12).
///   This decoder models only the step-8 SB-border column when the 8x8 alignment is
///   a no-op (even `MiCol`, see [`step8_above_row_column`]); every other SB-border
///   above-row probe stays unmodelled and forces a defer when it holds a new BV.
///
/// For `is_intrabc == 1`, § 7.12.2.4 Add reference motion vector only admits a
/// neighbour that is itself an IntrABC block (`add_ref_mv_candidate` requires
/// `is_intrabc == is_intrabc_block(candidate)`, `mvref_common.c:834`), adding its
/// recorded block vector deduped by value (`mvref_common.c:874`).
///
/// `lookup(row, col)` returns the recorded block vector of an IntrABC block at MI
/// `(row, col)`, or `None`. `is_coded(row, col)` is the AVM `is_mi_coded`
/// decode-order signal (`blockd.c:34`): `true` once MI `(row, col)` has been coded
/// earlier in decode order, used for the `has_top_right` per-4x4 availability gate.
/// The CURRENT block is not yet recorded when its stack is built, so `is_coded`
/// excludes it (matching AVM, which marks the block coded only after the stack is
/// built, `decodeframe.c:740` vs `:1355`).
///
/// Each placed candidate carries its accumulated § 7.12.2.6 weight (`WeightStack`
/// entry): `ADJACENT_SMVP_WEIGHT` (1) for every modelled adjacent position EXCEPT
/// the step-14 above-left corner, whose `(deltaRow == -1 && deltaCol == -1)`
/// (within-SB) or aligned `deltaCol < -1` (SB border) gives `OTHER_SMVP_WEIGHT` (0)
/// (`mvref_common.c:1515`-`1523`). A dedup-by-value match ADDS the new weight to the
/// existing entry (`ref_mv_weight[index] += weight`, `mvref_common.c:873`), so a
/// value placed by several adjacent scans accumulates. The candidates are returned
/// in scan ORDER (steps 7 → 8 → 9 → 10 → 11 → 12 → 14); the subsequent § 7.12.2.19
/// max-weight-to-slot-0 reorder is applied by [`intrabc_ref_stack_admission`] (which
/// threads the real `enable_drl_reorder` flag) before the stack is built.
pub(super) fn spatial_intrabc_scan(
    geometry: SpatialScanGeometry,
    lookup: impl Fn(usize, usize) -> Option<Mv>,
    is_coded: impl Fn(usize, usize) -> bool,
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

    let defer = spatial_scan_unmodelled_has_new_bv(&geometry, &lookup, &candidates, &above);

    SpatialIntrabcScan { candidates, defer }
}

/// The AV2 § 7.12.2.6 above-row SMVP probe columns this decoder places faithfully,
/// in step order. `None` marks an above-row step left UNMODELLED (it is unavailable
/// in AVM, or its 8x8 SB-border alignment is not a no-op): the over-scan defer keeps
/// guarding those columns. A `Some(col)` step is read (deduped) by the scan and
/// EXCLUDED from the over-scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AboveRowScan {
    /// The above MI row (`MiRow - 1`), or `None` at the frame/tile top edge (no
    /// above row exists, so every above-row probe is unavailable).
    above_row: Option<usize>,
    /// Step 8 (`row_smvp_state[0]`) column when modelled, else `None`.
    step8: Option<usize>,
    /// Step 10 (`row_smvp_state[1]`) column when modelled, else `None`.
    step10: Option<usize>,
    /// Step 12 (`row_smvp_state[2]`, top-right) column when modelled, else `None`.
    step12: Option<usize>,
    /// Step 14 (`row_smvp_state[3]`, above-left) column when modelled, else `None`.
    step14: Option<usize>,
    /// Whether the block sits on a horizontal superblock border
    /// (`MiRow % mib_size == 0`), so the SB-border 8x8-aligned line-buffer rules
    /// apply (the above-row probe columns are aligned to the 8x8 grid).
    is_sb_border: bool,
}

impl AboveRowScan {
    /// Resolves the modelled above-row probe columns (§ 7.12.2.6 `get_row_smvp_states`,
    /// `mvref_common.c:1996`-`2077`) for the block, using `is_coded` (AVM
    /// `is_mi_coded`) for the within-SB `has_top_right` per-4x4 availability gate.
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

    /// Resolves the SB-border above-row probe columns from the FULL AVM
    /// `row_smvp_all_states[1][block_width_type]` table (`mvref_common.c:2039`-`2069`),
    /// generic over the block's [`BlockWidthType`]. Each modelled column is
    /// `compute_aligned_offset(MiCol, raw)` (the line buffer keeps only the bottom
    /// 4x4 of each above 8x8 unit, so AVM aligns `MiCol` to the 8x8 grid
    /// `(MiCol >> 1) << 1`), gated by `is_above_smvp_available` (`mvref_common.c:1986`)
    /// / `has_top_right` (step 12). This decoder models the alignment-NO-OP case
    /// (even `MiCol`, so `compute_aligned_offset` reduces to the raw offset); an odd
    /// `MiCol` shifts the column and is left unmodelled.
    ///
    /// The raw `col_offset` per `[block_width_type][state]` (faithful to the table):
    ///
    /// | state | `BLOCK_WIDTH_4` (bw4 1) | `BLOCK_WIDTH_8` (bw4 2) | `OTHERS` (bw4>=4) |
    /// |-------|-------------------------|-------------------------|-------------------|
    /// | 8     | `avail(0)`, `co=0`      | `avail(0)`, `co=0`      | `avail(bw4-2)`, `co=bw4-2` |
    /// | 10    | **disabled** `{0,-1,0}` | **disabled** `{0,-1,0}` | `avail(0)`, `co=0` |
    /// | 12    | `htr&&avail(2)`, `co=2` | `htr&&avail(2)`, `co=2` | `htr&&avail(bw4)`, `co=bw4` |
    /// | 14    | `avail(-2)`, `co=-2`    | `avail(-2)`, `co=-2`    | `avail(-2)`, `co=-2` |
    ///
    /// The step-12 raw offset is `Max(2, bw4)` (`BLOCK_WIDTH_4` uses `2`, every wider
    /// width uses `bw4`), matching the § 7.12.2.1 step-12 `isSbBorder ? Max(2,bw4) :
    /// bw4` gate. The step-8 raw offset `Max(0, bw4 - 2)` collapses to `0` for
    /// `BLOCK_WIDTH_4`/`BLOCK_WIDTH_8` (both `bw4 - 2 <= 0`), matching the table's
    /// hardcoded `col_offset = 0` for those widths.
    ///
    /// SPEC-vs-AVM DIVERGENCE (step 8, narrow SB-border): for `BLOCK_WIDTH_4`/`_8`
    /// step 8's `deltaCol` is `Max(0, bw4 - 1 - isSbBorder) = 0`, so the § 7.12.2.6
    /// Scan-point clause "if `isSbBorder` and `deltaCol == 0` and
    /// `Num_4x4_Blocks_Wide[MiSize] <= 2`, terminate immediately"
    /// (`docs/spec/av2/1.0.0/07-decoding-process.md:3648`) reads as if step 8 should
    /// add nothing. AVM does NOT terminate it: `row_smvp_all_states[1][BLOCK_WIDTH_4/8]`
    /// index 0 is `{ is_above_smvp_available(.., 0), -1, compute_aligned_offset(.., 0) }`
    /// — a real availability check, NOT the hardcoded `{0,-1,0}` of index 1 — and
    /// neither `setup_ref_mv_list` nor `scan_blk_mbmi` applies any `deltaCol == 0`
    /// termination (`mvref_common.c:2050`-`2059`, `:2371`-`2375`, `:1491`-`1546`). AVM
    /// disables only the DUPLICATE col-0 read (step 10) via the § 7.12.2.1 step-10
    /// `bw4 >= (isSbBorder ? 4 : 2)` gate, and reads col 0 once via step 8. This
    /// decoder follows AVM (the avmdec bit-exact oracle), so `step8` stays modelled for
    /// all SB-border widths; the over-scan excludes col 0 because it is a REAL AVM
    /// candidate column, not a phantom. (The decode oracle wins over the spec text per
    /// the ac0ej3 mission's avmdec-bit-exact mandate.)
    ///
    /// SB-border `has_top_right` (step 12) short-circuits to `1`: the top-right 4x4 is
    /// at SB-relative `tr_mask_row = mask_row - 1 = -1 < 0`, the SB above, which is
    /// coded (`mvref_common.c:1560`-`1565`); the only remaining step-12 gate is the
    /// `is_above_smvp_available` tile-bound test inside [`sb_border_above_col`].
    fn resolve_sb_border(&mut self, geometry: &SpatialScanGeometry) {
        let col = geometry.mi_col;
        let bw4 = geometry.n4w;
        let width_type = BlockWidthType::from_bw4(bw4);
        if col & 1 != 0 {
            return;
        }
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

    /// Resolves the within-SB (non-SB-border) above-row probe columns at FULL 4x4
    /// resolution (`row_smvp_all_states[0][BLOCK_WIDTH_OTHERS]`,
    /// `mvref_common.c:2026`-`2032`): step 8 `deltaCol = bw4 - 1`, step 10 `0`,
    /// step 12 `bw4` (gated `has_top_right`), step 14 `-1`. Steps 8/10/14 are gated
    /// by `up_available` (an above row exists; `left_available` for step 14 is
    /// `MiCol > 0` for the single tile). Each column is clamped to the tile by
    /// [`tile_above_col`].
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

    /// AV2 § 7.12.2.6 `has_top_right` (`mvref_common.c:1548`-`1579`) for the step-12
    /// top-right probe, restricted to the within-SB case (the SB-edge short-circuits
    /// are subsumed by the SB-border branch in [`Self::resolve`]). The top-right 4x4
    /// is at SB-relative `(mask_row - 1, mask_col + bw4)`: `mask_row - 1 < 0` (the
    /// block is at the SB top) is the SB-border case (not reached here);
    /// `mask_col + bw4 >= sb_size4` puts the top-right in the next superblock (NOT
    /// coded, `has_tr = 0`); otherwise it is `is_mi_coded` at that within-SB 4x4.
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

/// The above MI position `(MiRow - 1, col)` clamped to the tile (`is_inside`): the
/// column must lie inside the tile column range, else the probe is unavailable
/// (`None`).
fn tile_above_col(geometry: &SpatialScanGeometry, col: Option<usize>) -> Option<usize> {
    let col = col?;
    if geometry.mi_row == 0 || col >= geometry.mi_cols {
        return None;
    }
    Some(col)
}

/// The SB-border above-row probe column for an even `MiCol`: the 8x8-aligned column
/// `((MiCol >> 1) << 1) + raw` reduces to `MiCol + raw` (alignment no-op), gated by
/// AV2 § 7.12.2.6 `is_above_smvp_available` (`mvref_common.c:1986`): the aligned
/// column must lie inside the tile column range `[0, mi_cols)` (single tile). `raw`
/// is the un-aligned `col_offset` (`bw4 - 2`, `0`, `bw4`, or `-2`). Returns `None`
/// when the column leaves the tile (the probe is unavailable).
fn sb_border_above_col(geometry: &SpatialScanGeometry, raw: i64) -> Option<usize> {
    if geometry.mi_row == 0 {
        return None;
    }
    let aligned = i64::try_from(geometry.mi_col).ok()?.checked_add(raw)?;
    let col = usize::try_from(aligned).ok()?;
    if col >= geometry.mi_cols {
        return None;
    }
    Some(col)
}

/// Reads a modelled above-row probe (an `AboveRowScan` step column) at `(MiRow - 1,
/// col)` and adds its IntrABC block vector to the candidate list, deduped by value
/// with the § 7.12.2.6 `weight` (accumulated on a value match). A `None` column is
/// an unmodelled / unavailable probe and reads nothing.
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

/// The MI column of the § 7.12.2.1 step-8 above-row SMVP probe when this decoder
/// can place it faithfully, or `None` to leave it for the conservative over-scan
/// defer.
///
/// § 7.12.2.1 step 8 (`docs/spec/av2/1.0.0/07-decoding-process.md`, ~line 3431)
/// invokes § 7.12.2.6 Scan point with `deltaRow = -1` and
/// `deltaCol = Max(0, bw4 - 1 - isSbBorder)`, then § 7.12.2.6 (~line 3766)
/// SB-aligns the column on a superblock border:
///
/// ```text
/// isSbBorder = (MiRow & (Num_4x4_Blocks_High[SbSize] - 1)) == 0
/// if (deltaRow < 0 && isSbBorder) { mvCol = (mvCol >> 1) << 1; deltaCol = mvCol - MiCol }
/// ```
///
/// This matches AVM `get_row_smvp_states` `row_smvp_state[0]` for an SB-border
/// block (`compute_aligned_offset(mi_col, bw4 - 2)`, `mvref_common.c:1979`/`2062`).
///
/// Modelled ONLY when ALL of the following hold (else `None` -> defer-as-before):
///
/// * `isSbBorder == 1` (the block sits on a horizontal superblock border, so the
///   8x8-grid alignment is well defined and the neighbour 8x8 unit is fully
///   decoded);
/// * `up_available` (`MiRow > 0`; ac0ej3 is single-tile so this is the only
///   above-availability clause);
/// * the SB-aligned column equals the un-aligned spec column (true for even
///   `mi_col`; an odd `mi_col` shifts the column under `(mvCol >> 1) << 1`, which
///   this decoder does not place faithfully -> defer).
fn step8_above_row_column(geometry: &SpatialScanGeometry) -> Option<usize> {
    let row = geometry.mi_row;
    let col = geometry.mi_col;
    if row == 0 {
        return None;
    }
    if geometry.sb_size4 == 0 || !row.is_multiple_of(geometry.sb_size4) {
        return None;
    }
    let delta_col = geometry.n4w.saturating_sub(2);
    debug_assert!(
        delta_col.is_multiple_of(2),
        "step-8 deltaCol must be even for the MiCol-parity floor shortcut; \
         an odd deltaCol (odd bw4) needs the full (MiCol+deltaCol)>>1<<1 floor",
    );
    if col & 1 != 0 {
        return None;
    }
    let aligned_col = col.checked_add(delta_col)?;
    if aligned_col >= geometry.mi_cols {
        return None;
    }
    Some(aligned_col)
}

/// Adds an IntrABC neighbour block vector at MI `(row_offset, left_col)` to the
/// running spatial candidate list, deduped by value (AV2 § 7.12.2.4
/// `mvref_common.c:874`). A value already on the list ACCUMULATES the new
/// § 7.12.2.6 `weight` into its existing entry (AVM `ref_mv_weight[index] += weight`,
/// `mvref_common.c:873`); a new value appends with `weight`.
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

/// Whether any AV2 § 7.12.2 spatial position this decoder does NOT model exactly
/// (the still-unmodelled above-row probes and the § 7.12.2.5 deltaCol = -3
/// non-adjacent scan) holds an IntrABC neighbour
/// whose block vector is NOT already a modelled candidate. A duplicate block
/// vector contributes nothing in AVM, so it does not force a defer; a new one
/// would extend the stack unfaithfully, so it does.
///
/// `above` is the resolved above-row scan ([`AboveRowScan`]): the columns it places
/// exactly (its `Some` steps) are EXCLUDED from the above-row over-scan so a hit on
/// a modelled column no longer forces a defer, while every other above-row column
/// (an unmodelled SB-border probe, or any column outside the modelled within-SB
/// steps) continues to defer.
fn spatial_scan_unmodelled_has_new_bv(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    modelled: &[WeightedBv],
    above: &AboveRowScan,
) -> bool {
    let row = geometry.mi_row;
    let col = geometry.mi_col;
    let bw4 = geometry.n4w;
    let trace = crate::trace_flags::trace_flag!("SPLOT_TRACE_INTRABC_REF_STACK");
    let is_new = |mv: Option<Mv>| mv.is_some_and(|mv| !modelled.iter().any(|entry| entry.mv == mv));
    if let Some(above_row) = row.checked_sub(1) {
        let modelled_cols = [above.step8, above.step10, above.step12, above.step14];
        // § 7.12.2.1 step 12: SB-border reach is `Max(2, bw4)`; left reach carries 8x8 parity.
        let extra_left = if above.is_sb_border { 2 + (col & 1) } else { 1 };
        let leftmost = col.saturating_sub(extra_left);
        let right_reach = if above.is_sb_border { bw4.max(2) } else { bw4 };
        let rightmost = col.saturating_add(right_reach); // inclusive of the top-right col
        for c in leftmost..=rightmost {
            if modelled_cols.contains(&Some(c)) {
                continue;
            }
            if let Some(mv) = lookup_in_grid(geometry, lookup, above_row, c)
                && is_new(Some(mv))
            {
                if trace {
                    eprintln!(
                        "intrabc ref_stack unmodelled_probe kind=above_row mi=({}, {}) probe=({}, {}) mv={:?} modelled={:?} modelled_cols={:?} leftmost={} rightmost={} sb_border={}",
                        row,
                        col,
                        above_row,
                        c,
                        mv,
                        modelled,
                        modelled_cols,
                        leftmost,
                        rightmost,
                        above.is_sb_border
                    );
                }
                return true;
            }
        }
    }
    if let Some(deep_col) = col.checked_sub(3) {
        let bottom = row.checked_add(geometry.n4h.saturating_sub(1));
        let probes = [bottom, Some(row)];
        for r in probes.into_iter().flatten() {
            if let Some(mv) = lookup_in_grid(geometry, lookup, r, deep_col)
                && is_new(Some(mv))
            {
                if trace {
                    eprintln!(
                        "intrabc ref_stack unmodelled_probe kind=deep_left mi=({row}, {col}) probe=({r}, {deep_col}) mv={mv:?} modelled={modelled:?}",
                    );
                }
                return true;
            }
        }
    }
    false
}

/// Looks up an IntrABC neighbour block vector at MI `(row, col)`, bounded to the
/// tile (AV2 § 7.12.2 `is_inside`, which for the ac0ej3 single tile reduces to a
/// grid-bounds test).
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

/// Builds the AV2 § 7.12.2 IBC ref-MV stack for an IntrABC block, given the
/// already-populated tile ref-MV bank (the state AVM holds when the block is
/// decoded) and the § 7.12.2 spatial SMVP candidates. Returns the candidate stack
/// (capped at `max_bvp_drl_bits_minus_1 + 2`).
///
/// `enable_refmvbank` is the AV2 sequence-header flag: when `false` the § 7.12.2.21
/// ref-MV-bank fill is skipped entirely (AV2 only runs it when `enable_refmvbank ==
/// 1`), so the stack is spatial + default-fill only.
///
/// `spatial` is the § 7.12.2 spatial SMVP scan result: its candidates are added to
/// the stack first (AVM adds them to `ref_mv_stack` before the § 7.12.2.21 bank
/// fill and the § 7.12.2.20 default fill, `mvref_common.c:2362`-`2711`), and the
/// later bank candidates are deduped against them (`check_rmb_cand`).
pub(super) fn build_intrabc_ref_mv_stack(
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

/// AV2 § 5.4.6 `DrlReorder` mode (the sequence-header `disable_drl_reorder` /
/// `constrain_drl_reorder` derivation), threaded into the § 7.12.2.19 `useSort`
/// gate. Mirrors `splot_core::headers::sequence::DrlReorder` without coupling this
/// decode-internal module to the core header type (the caller maps it).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DrlReorderMode {
    /// `DRL_REORDER_DISABLED`: the § 7.12.2.19 sort never runs for the IBC nearest
    /// prefix (`useSort` is always 0 here — the `>= 4` constraint path is the only
    /// `DRL_REORDER_DISABLED` trigger and IBC nearest counts cannot reach it).
    Disabled,
    /// `DRL_REORDER_CONSTRAINT`: `useSort = (!useTemporalFirst && numNearest >= 4)`.
    /// For IBC `useTemporalFirst` (TMVP high priority) is always 0
    /// (`allow_ref_frame_mvs == 0` for an intra-only frame,
    /// `assign_tmvp_high_priority`, `mvref_common.c:1904`), so `useSort = numNearest
    /// >= 4`.
    Constraint,
    /// `DRL_REORDER_ALWAYS`: `useSort = 1` whenever `numNearest > 1`.
    Always,
}

impl DrlReorderMode {
    /// AV2 § 7.12.2.18 step-17 `useSort` for the IBC nearest prefix of length
    /// `nearest`: `DRL_REORDER_ALWAYS || (DRL_REORDER_CONSTRAINT && !useTemporalFirst
    /// && nearest >= 4)` (`docs/spec/av2/1.0.0/07-decoding-process.md` ~line 3449;
    /// `mvref_common.c:2473`-`2475`). `useTemporalFirst` is 0 for IBC.
    pub(super) const fn use_sort(self, nearest: usize) -> bool {
        match self {
            Self::Disabled => false,
            Self::Constraint => nearest >= 4,
            Self::Always => true,
        }
    }
}

/// AV2 § 7.12.2.19 Sorting process over the nearest/spatial prefix `[0, end)`: moves
/// the highest-weight entry to slot 0 with a SINGLE swap (`docs/spec/av2/1.0.0/
/// 07-decoding-process.md` ~line 4515; `mvref_common.c:2476`-`2491`). The max is
/// found with a STRICT `>` (`maxWeight < WeightStack[idx]`), so the FIRST/lowest
/// index wins ties; the swap runs only when `max_idx != 0`. Operates ONLY on the
/// nearest prefix (BEFORE the § 7.12.2.21 bank fill and § 7.12.2.20 default fill,
/// which are weight-independent).
pub(super) fn sort_nearest_max_weight_to_slot0(candidates: &mut [WeightedBv]) {
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

/// The AV2 § 7.12.2 IntrABC ref-MV stack admission decision for one block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IntrabcStackAdmission {
    /// The real § 7.12.2 stack was built faithfully; `selected` is the predictor
    /// block vector the decoded DRL index picks (`RefStackMv[ref_mv_idx]`).
    Admit {
        /// `RefStackMv[ref_mv_idx]`: the predictor the IntrABC mode adds the MV
        /// delta to (NEWMV) or uses directly (NEARMV).
        selected: Mv,
    },
    /// The § 7.12.2 spatial scan reaches an unmodelled position, or the decoded
    /// DRL index lands outside the built stack: over-reject.
    Defer,
}

/// Decides AV2 § 7.12.2 IntrABC ref-MV stack admission: applies the § 7.12.2.19
/// max-weight-to-slot-0 reorder to the nearest/spatial prefix, builds the real stack
/// (sorted § 7.12.2 spatial scan candidates + § 7.12.2.21 ref-MV bank fill +
/// § 7.12.2.20 default block vectors), and returns the block vector the decoded DRL
/// index `ref_mv_idx` selects, so the caller can use the AVM-faithful candidate.
///
/// The § 7.12.2.19 sort runs ONLY on the nearest prefix, BEFORE the bank/default
/// fill, and only when `useSort` holds (§ 7.12.2.18 step 17: `DRL_REORDER_ALWAYS ||
/// (DRL_REORDER_CONSTRAINT && nearest >= 4)`, with `useTemporalFirst == 0` for IBC)
/// AND `nearest > 1` (`mvref_common.c:2473`-`2475`). Each candidate's accumulated
/// § 7.12.2.6 weight (placed by [`spatial_intrabc_scan`]) drives the single swap.
///
/// Defers (returns `Defer`) when:
///
/// * the § 7.12.2 spatial scan reaches a position this decoder does not model
///   faithfully (`spatial.defer`); or
/// * the decoded DRL index lands outside the built stack.
///
/// `enable_refmvbank` is the AV2 sequence flag gating the § 7.12.2.21 bank fill;
/// `drl_reorder` is the AV2 § 5.4.6 `DrlReorder` mode gating the § 7.12.2.19 sort.
pub(super) fn intrabc_ref_stack_admission(
    bank: &IntrabcRefMvBank,
    geometry: IntrabcStackGeometry,
    spatial: &SpatialIntrabcScan,
    enable_refmvbank: bool,
    drl_reorder: DrlReorderMode,
    ref_mv_idx: usize,
) -> IntrabcStackAdmission {
    if spatial.defer {
        return IntrabcStackAdmission::Defer;
    }
    let mut nearest: Vec<WeightedBv> = spatial.candidates.clone();
    if drl_reorder.use_sort(nearest.len()) && nearest.len() > 1 {
        sort_nearest_max_weight_to_slot0(&mut nearest);
    }
    let sorted: Vec<Mv> = nearest.iter().map(|entry| entry.mv).collect();
    let real_stack = build_intrabc_ref_mv_stack(bank, geometry, enable_refmvbank, &sorted);
    match real_stack.get(ref_mv_idx).copied() {
        Some(selected) => IntrabcStackAdmission::Admit { selected },
        None => IntrabcStackAdmission::Defer,
    }
}

/// AVM `INTRABC_DELAY_PIXELS` (`av2/common/mvref_common.h:610`).
const INTRABC_DELAY_PIXELS: i32 = 256;

/// AV2 § 7.12.2.20 `add_to_ref_bv(dx, dy)`: a default block vector with row
/// `dy << 3` and column `dx << 3` (eighth-pel).
const fn add_to_ref_bv(dx: i32, dy: i32) -> Mv {
    Mv {
        row: dy * 8,
        col: dx * 8,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A weighted spatial candidate with the default `ADJACENT_SMVP_WEIGHT` (1): the
    /// weight most modelled positions place.
    fn adj(mv: Mv) -> WeightedBv {
        WeightedBv {
            mv,
            weight: ADJACENT_SMVP_WEIGHT,
        }
    }

    /// A weighted spatial candidate with an explicit weight.
    const fn wbv(mv: Mv, weight: u16) -> WeightedBv {
        WeightedBv { mv, weight }
    }

    /// ac0ej3 frame-0 SB row 0, mib_size 32 (128x128 SB). Walks the first three
    /// reachable IntrABC blocks and checks the bank + stack against the AVM
    /// `av2_find_mv_refs` dump.
    fn ac0ej3_geometry(mi_row: usize, mi_col: usize) -> IntrabcStackGeometry {
        IntrabcStackGeometry {
            mi_row,
            mi_col,
            n4w: 8,  // 32 px wide
            n4h: 16, // 64 px high
            sb_samples: 128,
            frame_w: 1920,
            frame_h: 1080,
            max_bvp_drl_bits_minus_1: 2,
        }
    }

    #[test]
    fn ac0ej3_mi_0_112_is_default_only() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        assert_eq!(bank.entries(), [Mv { row: -512, col: 0 }]);

        let stack = build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(0, 112), true, &[]);
        assert_eq!(
            stack,
            vec![
                Mv { row: -1024, col: 0 },
                Mv { row: 0, col: -3072 },
                Mv { row: -512, col: 0 },
                Mv { row: 0, col: -256 },
            ]
        );
    }

    #[test]
    fn ac0ej3_mi_0_232_is_reordered_by_bank() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        bank.record_block(0, 112, 8, 16, true, Some(Mv { row: 0, col: -256 }));
        assert_eq!(
            bank.entries(),
            [Mv { row: -512, col: 0 }, Mv { row: 0, col: -256 }]
        );

        let stack = build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(0, 232), true, &[]);
        assert_eq!(
            stack,
            vec![
                Mv { row: 0, col: -256 },
                Mv { row: -1024, col: 0 },
                Mv { row: 0, col: -3072 },
                Mv { row: -512, col: 0 },
            ]
        );
    }

    fn bounds(mi_row: i32, mi_col: i32) -> RmbCandBounds {
        RmbCandBounds {
            mi_row,
            mi_col,
            block_w: 32,
            block_h: 64,
            frame_w: 1920,
            frame_h: 1080,
        }
    }

    #[test]
    fn check_rmb_cand_rejects_frame_boundary_top_edge() {
        assert!(!check_rmb_cand(
            Mv { row: -512, col: 0 },
            &[],
            bounds(0, 112),
        ));
    }

    #[test]
    fn check_rmb_cand_admits_in_bounds_candidate() {
        assert!(check_rmb_cand(
            Mv { row: 0, col: -256 },
            &[],
            bounds(0, 232),
        ));
    }

    #[test]
    fn check_rmb_cand_rejects_duplicate() {
        let cand = Mv { row: 0, col: -256 };
        assert!(!check_rmb_cand(cand, &[cand], bounds(0, 232)));
    }

    #[test]
    fn bank_zeroes_at_new_superblock_row() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(0, 0, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        assert_eq!(bank.entries().len(), 1);
        bank.record_block(32, 0, 8, 16, true, Some(Mv { row: -1024, col: 0 }));
        assert_eq!(bank.entries(), [Mv { row: -1024, col: 0 }]);
    }

    #[test]
    fn first_block_of_new_sb_row_reads_empty_bank() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(0, 0, 8, 16, true, Some(Mv { row: 0, col: -256 }));
        assert_eq!(bank.entries(), [Mv { row: 0, col: -256 }]);
        bank.enter_block_superblock(32, 8);
        assert!(bank.entries().is_empty());
        let stack = build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(32, 8), true, &[]);
        assert_eq!(
            stack,
            vec![
                Mv { row: -1024, col: 0 },
                Mv { row: 0, col: -3072 },
                Mv { row: -512, col: 0 },
                Mv { row: 0, col: -256 },
            ]
        );
    }

    /// No spatial candidate, no defer.
    fn no_spatial() -> SpatialIntrabcScan {
        SpatialIntrabcScan {
            candidates: Vec::new(),
            defer: false,
        }
    }

    #[test]
    fn admission_admits_ac0ej3_mi_0_112_default_only() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 112),
                &no_spatial(),
                true,
                DrlReorderMode::Always,
                3,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -256 },
            }
        );
    }

    #[test]
    fn admission_selects_ac0ej3_mi_0_232_bank_reordered_bv() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        bank.record_block(0, 112, 8, 16, true, Some(Mv { row: 0, col: -256 }));
        let decide = |enable_refmvbank| {
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 232),
                &no_spatial(),
                enable_refmvbank,
                DrlReorderMode::Always,
                2,
            )
        };
        assert_eq!(
            decide(true),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -3072 },
            }
        );
        assert_eq!(
            decide(false),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -512, col: 0 },
            }
        );
    }

    #[test]
    fn admission_selects_ac0ej3_mi_0_240_spatial_bv() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        bank.record_block(0, 112, 8, 16, true, Some(Mv { row: 0, col: -256 }));
        bank.record_block(0, 232, 8, 16, true, Some(Mv { row: 0, col: -3072 }));
        let spatial = SpatialIntrabcScan {
            candidates: vec![adj(Mv { row: 0, col: -3072 })],
            defer: false,
        };
        let stack = build_intrabc_ref_mv_stack(
            &bank,
            ac0ej3_geometry(0, 240),
            true,
            &[Mv { row: 0, col: -3072 }],
        );
        assert_eq!(
            stack,
            vec![
                Mv { row: 0, col: -3072 },
                Mv { row: 0, col: -256 },
                Mv { row: -1024, col: 0 },
                Mv { row: 0, col: -3072 },
            ]
        );
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &spatial,
                true,
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -3072 },
            }
        );
    }

    #[test]
    fn admission_defers_on_unmodelled_spatial_intrabc() {
        let bank = IntrabcRefMvBank::new(32);
        let spatial = SpatialIntrabcScan {
            candidates: Vec::new(),
            defer: true,
        };
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 112),
                &spatial,
                true,
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Defer
        );
    }

    #[test]
    fn admission_forced_swap_places_max_weight_at_slot0() {
        let bank = IntrabcRefMvBank::new(32);
        let unsorted = SpatialIntrabcScan {
            candidates: vec![
                wbv(Mv { row: 0, col: -64 }, 0),
                wbv(Mv { row: -512, col: 0 }, 1),
            ],
            defer: false,
        };
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &unsorted,
                false, // bank fill OFF: the stack is spatial-prefix + defaults only.
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -512, col: 0 },
            },
            "the §7.12.2.19 sort must move the weight-1 candidate to slot 0 (not a passthrough)"
        );
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &unsorted,
                false,
                DrlReorderMode::Always,
                1,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -64 },
            }
        );
    }

    #[test]
    fn admission_no_op_swap_when_slot0_already_max() {
        let bank = IntrabcRefMvBank::new(32);
        let tie = SpatialIntrabcScan {
            candidates: vec![
                wbv(Mv { row: -1024, col: 0 }, 1),
                wbv(Mv { row: -512, col: 0 }, 1),
            ],
            defer: false,
        };
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &tie,
                false,
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -1024, col: 0 },
            },
            "equal weights must keep the lowest index in slot 0 (strict `>` tie-break)"
        );
        let frontier = SpatialIntrabcScan {
            candidates: vec![
                wbv(Mv { row: -1024, col: 0 }, 3),
                wbv(Mv { row: -512, col: 0 }, 1),
            ],
            defer: false,
        };
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &frontier,
                false,
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -1024, col: 0 },
            },
            "the max-weight slot-0 entry stays put (no swap)"
        );
    }

    #[test]
    fn admission_sort_respects_drl_reorder_mode() {
        let bank = IntrabcRefMvBank::new(32);
        let candidates = vec![
            wbv(Mv { row: 0, col: -64 }, 0),
            wbv(Mv { row: -512, col: 0 }, 1),
        ];
        let scan = SpatialIntrabcScan {
            candidates: candidates.clone(),
            defer: false,
        };
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &scan,
                false,
                DrlReorderMode::Disabled,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -64 },
            },
            "DRL_REORDER_DISABLED must NOT sort (slot 0 stays scan-order-first)"
        );
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &scan,
                false,
                DrlReorderMode::Constraint,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -64 },
            },
            "DRL_REORDER_CONSTRAINT with nearest < 4 must NOT sort"
        );
        assert_eq!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &scan,
                false,
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -512, col: 0 },
            },
            "DRL_REORDER_ALWAYS must sort the weight-1 candidate into slot 0"
        );
    }

    #[test]
    fn admission_admits_single_spatial_candidate() {
        let bank = IntrabcRefMvBank::new(32);
        let one = SpatialIntrabcScan {
            candidates: vec![adj(Mv { row: 0, col: -64 })],
            defer: false,
        };
        assert!(matches!(
            intrabc_ref_stack_admission(
                &bank,
                ac0ej3_geometry(0, 240),
                &one,
                true,
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Admit { .. }
        ));
    }

    #[test]
    fn sort_nearest_moves_max_weight_to_slot0_strict() {
        let mut swap = vec![wbv(Mv { row: 1, col: 1 }, 0), wbv(Mv { row: 2, col: 2 }, 1)];
        sort_nearest_max_weight_to_slot0(&mut swap);
        assert_eq!(swap[0], wbv(Mv { row: 2, col: 2 }, 1));
        assert_eq!(swap[1], wbv(Mv { row: 1, col: 1 }, 0));
        let mut tie = vec![wbv(Mv { row: 1, col: 1 }, 2), wbv(Mv { row: 2, col: 2 }, 2)];
        sort_nearest_max_weight_to_slot0(&mut tie);
        assert_eq!(tie[0], wbv(Mv { row: 1, col: 1 }, 2));
        let mut already = vec![wbv(Mv { row: 1, col: 1 }, 3), wbv(Mv { row: 2, col: 2 }, 1)];
        sort_nearest_max_weight_to_slot0(&mut already);
        assert_eq!(already[0], wbv(Mv { row: 1, col: 1 }, 3));
        let mut empty: Vec<WeightedBv> = Vec::new();
        sort_nearest_max_weight_to_slot0(&mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn spatial_scan_adds_left_neighbour_and_admits_modelled_above_neighbour() {
        let geom = SpatialScanGeometry {
            mi_row: 4,
            mi_col: 8,
            n4w: 4,
            n4h: 4,
            mi_rows: 64,
            mi_cols: 64,
            sb_size4: 32,
        };
        let left_only = spatial_intrabc_scan(
            geom,
            |row, col| (row == 7 && col == 7).then_some(Mv { row: 0, col: -64 }),
            |_, _| false,
        );
        assert_eq!(left_only.candidates, vec![adj(Mv { row: 0, col: -64 })]);
        assert!(!left_only.defer);
        let above = spatial_intrabc_scan(
            geom,
            |row, col| (row == 3 && col == 8).then_some(Mv { row: -8, col: 0 }),
            |_, _| false,
        );
        assert_eq!(above.candidates, vec![adj(Mv { row: -8, col: 0 })]);
        assert!(!above.defer);
        let deep_left = spatial_intrabc_scan(
            geom,
            |row, col| (row == 4 && col == 5).then_some(Mv { row: 0, col: -512 }),
            |_, _| false,
        );
        assert!(deep_left.defer);
    }

    /// ac0ej3 frame-0 MI(32,56) geometry for the § 7.12.2.1 step-8 SB-border probe.
    /// mib_size 32, so MiRow 32 sits on a horizontal SB border (`32 % 32 == 0`).
    fn ac0ej3_mi_32_56_scan_geometry() -> SpatialScanGeometry {
        SpatialScanGeometry {
            mi_row: 32,
            mi_col: 56,
            n4w: 8,
            n4h: 16,
            mi_rows: 270,
            mi_cols: 480,
            sb_size4: 32,
        }
    }

    #[test]
    fn spatial_scan_admits_ac0ej3_mi_32_56_step8_above_neighbour() {
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_32_56_scan_geometry(),
            |row, col| (row == 31 && col == 62).then_some(Mv { row: -512, col: 0 }),
            |_, _| false,
        );
        assert_eq!(scan.candidates, vec![adj(Mv { row: -512, col: 0 })]);
        assert!(!scan.defer);
    }

    #[test]
    fn admission_selects_ac0ej3_mi_32_56_step8_bv() {
        let bank = IntrabcRefMvBank::new(32); // freshly zeroed at the SB-row-1 entry.
        let spatial = spatial_intrabc_scan(
            ac0ej3_mi_32_56_scan_geometry(),
            |row, col| (row == 31 && col == 62).then_some(Mv { row: -512, col: 0 }),
            |_, _| false,
        );
        let geometry = IntrabcStackGeometry {
            mi_row: 32,
            mi_col: 56,
            n4w: 8,
            n4h: 16,
            sb_samples: 128,
            frame_w: 1920,
            frame_h: 1080,
            max_bvp_drl_bits_minus_1: 2,
        };
        let stack = build_intrabc_ref_mv_stack(&bank, geometry, true, &[Mv { row: -512, col: 0 }]);
        assert_eq!(
            stack,
            vec![
                Mv { row: -512, col: 0 },
                Mv { row: -1024, col: 0 },
                Mv { row: 0, col: -3072 },
                Mv { row: -512, col: 0 },
            ]
        );
        assert_eq!(
            intrabc_ref_stack_admission(&bank, geometry, &spatial, true, DrlReorderMode::Always, 0),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -512, col: 0 },
            }
        );
    }

    #[test]
    fn spatial_scan_defers_on_other_above_row_column() {
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_32_56_scan_geometry(),
            |row, col| (row == 31 && col == 60).then_some(Mv { row: 7, col: -99 }),
            |_, _| false,
        );
        assert!(scan.candidates.is_empty());
        assert!(scan.defer);
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_32_56_scan_geometry(),
            |row, col| {
                if row == 31 && col == 62 {
                    Some(Mv { row: -512, col: 0 })
                } else if row == 31 && col == 60 {
                    Some(Mv { row: 7, col: -99 })
                } else {
                    None
                }
            },
            |_, _| false,
        );
        assert_eq!(scan.candidates, vec![adj(Mv { row: -512, col: 0 })]);
        assert!(scan.defer);
    }

    fn assert_unmodelled_step8_defers(mi_row: usize, mi_col: usize, above_neighbour_col: usize) {
        let geom = SpatialScanGeometry {
            mi_row,
            mi_col,
            n4w: 8,
            n4h: 16,
            mi_rows: 270,
            mi_cols: 480,
            sb_size4: 32,
        };
        assert!(step8_above_row_column(&geom).is_none());
        let above = mi_row - 1;
        let scan = spatial_intrabc_scan(
            geom,
            |row, col| {
                (row == above && col == above_neighbour_col).then_some(Mv { row: -512, col: 0 })
            },
            |_, _| false,
        );
        assert!(scan.defer);
    }

    #[test]
    fn spatial_scan_defers_on_odd_mi_col_sb_border() {
        assert_unmodelled_step8_defers(32, 57, 63);
    }

    /// ac0ej3 frame-0 MI(48,56) geometry: mib_size 32, MiRow 48, `48 % 32 == 16 != 0`
    /// -> NOT an SB border, so the within-SB 4x4-resolution above-row scan applies.
    /// BLOCK_32X64 (bw4 = 8, bh4 = 16), the new frontier-class block.
    fn ac0ej3_mi_48_56_scan_geometry() -> SpatialScanGeometry {
        SpatialScanGeometry {
            mi_row: 48,
            mi_col: 56,
            n4w: 8,
            n4h: 16,
            mi_rows: 270,
            mi_cols: 480,
            sb_size4: 32,
        }
    }

    #[test]
    fn spatial_scan_admits_ac0ej3_mi_48_56_within_sb_step8() {
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_48_56_scan_geometry(),
            |row, col| (row == 47 && (56..=63).contains(&col)).then_some(Mv { row: -512, col: 0 }),
            |_, _| true,
        );
        assert_eq!(scan.candidates, vec![wbv(Mv { row: -512, col: 0 }, 2)]);
        assert!(!scan.defer);
    }

    #[test]
    fn spatial_scan_step12_top_right_respects_has_top_right() {
        let geom = SpatialScanGeometry {
            mi_row: 20,
            mi_col: 8,
            n4w: 4,
            n4h: 4,
            mi_rows: 64,
            mi_cols: 64,
            sb_size4: 32,
        };
        let bv = Mv { row: -8, col: -8 };
        let not_coded = spatial_intrabc_scan(geom, |_, _| None, |_, _| false);
        assert!(not_coded.candidates.is_empty());
        assert!(!not_coded.defer);
        let coded = spatial_intrabc_scan(
            geom,
            |row, col| (row == 19 && col == 12).then_some(bv),
            |row, col| row == 19 && col == 12,
        );
        assert_eq!(coded.candidates, vec![adj(bv)]);
        assert!(!coded.defer);
    }

    #[test]
    fn spatial_scan_disables_step10_for_block_width_4_within_sb() {
        let narrow = SpatialScanGeometry {
            mi_row: 20,
            mi_col: 8,
            n4w: 1,
            n4h: 4,
            mi_rows: 64,
            mi_cols: 64,
            sb_size4: 32,
        };
        let bv = Mv { row: -8, col: 0 };
        let scan = spatial_intrabc_scan(
            narrow,
            |row, col| (row == 19 && col == 8).then_some(bv),
            |_, _| false,
        );
        assert_eq!(
            scan.candidates,
            vec![wbv(bv, 1)],
            "step 10 disabled for bw4 == 1: the above candidate keeps step-8 weight 1"
        );
        assert!(!scan.defer);
        let wide = SpatialScanGeometry { n4w: 2, ..narrow };
        let wide_scan = spatial_intrabc_scan(
            wide,
            |row, col| (row == 19 && (8..=9).contains(&col)).then_some(bv),
            |_, _| false,
        );
        assert_eq!(
            wide_scan.candidates,
            vec![wbv(bv, 2)],
            "bw4 >= 2 enables step 10: step 8 + step 10 accumulate weight 2"
        );
        assert!(!wide_scan.defer);
    }

    #[test]
    fn spatial_scan_disables_step10_for_block_width_4_sb_border() {
        let narrow = SpatialScanGeometry {
            mi_row: 32,
            mi_col: 8,
            n4w: 1,
            n4h: 4,
            mi_rows: 270,
            mi_cols: 480,
            sb_size4: 32,
        };
        let bv = Mv { row: -8, col: 0 };
        let scan = spatial_intrabc_scan(
            narrow,
            |row, col| (row == 31 && col == 8).then_some(bv),
            |_, _| false,
        );
        assert_eq!(
            scan.candidates,
            vec![wbv(bv, 1)],
            "step 10 disabled for bw4 == 1 on the SB border: the above candidate keeps step-8 weight 1"
        );
        assert!(!scan.defer);
        let wide = SpatialScanGeometry { n4w: 8, ..narrow };
        let wide_scan = spatial_intrabc_scan(
            wide,
            |row, col| (row == 31 && (col == 8 || col == 14)).then_some(bv),
            |_, _| false,
        );
        assert_eq!(
            wide_scan.candidates,
            vec![wbv(bv, 2)],
            "bw4 >= 4 enables step 10 on the SB border: step 8 + step 10 accumulate weight 2"
        );
        assert!(!wide_scan.defer);
    }

    #[test]
    fn spatial_scan_dedups_same_left_neighbour() {
        let geom = SpatialScanGeometry {
            mi_row: 0,
            mi_col: 8,
            n4w: 8,
            n4h: 16,
            mi_rows: 64,
            mi_cols: 64,
            sb_size4: 32,
        };
        let scan = spatial_intrabc_scan(
            geom,
            |row, col| (col == 7 && row < 16).then_some(Mv { row: 0, col: -3072 }),
            |_, _| false,
        );
        assert_eq!(scan.candidates, vec![wbv(Mv { row: 0, col: -3072 }, 2)]);
        assert!(!scan.defer);
    }

    /// ac0ej3 frame-0 MI(192,112) geometry: MiRow 192, `192 % 32 == 0` -> SB border;
    /// MiCol 112 even; BLOCK_64X32 (bw4 = 16, bh4 = 8) — the §7.12.2.19 weight-sort
    /// frontier.
    fn ac0ej3_mi_192_112_scan_geometry() -> SpatialScanGeometry {
        SpatialScanGeometry {
            mi_row: 192,
            mi_col: 112,
            n4w: 16,
            n4h: 8,
            mi_rows: 270,
            mi_cols: 480,
            sb_size4: 32,
        }
    }

    #[test]
    fn admission_admits_ac0ej3_mi_192_112_no_op_weight_sort() {
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_192_112_scan_geometry(),
            |row, col| {
                if (192..=199).contains(&row) && col == 111 {
                    Some(Mv { row: -1024, col: 0 })
                } else if row == 191 && col == 126 {
                    Some(Mv { row: -512, col: 0 })
                } else {
                    None
                }
            },
            |_, _| false,
        );
        assert_eq!(
            scan.candidates,
            vec![
                wbv(Mv { row: -1024, col: 0 }, 2),
                wbv(Mv { row: -512, col: 0 }, 1),
            ],
            "(-1024,0) accumulates step 7 + step 9 weight (2); (-512,0) step 8 weight (1)"
        );
        assert!(
            !scan.defer,
            "the scan itself does not defer (the candidates are placed)"
        );
        let geometry = IntrabcStackGeometry {
            mi_row: 192,
            mi_col: 112,
            n4w: 16,
            n4h: 8,
            sb_samples: 128,
            frame_w: 1920,
            frame_h: 1080,
            max_bvp_drl_bits_minus_1: 2,
        };
        assert_eq!(
            intrabc_ref_stack_admission(
                &IntrabcRefMvBank::new(32),
                geometry,
                &scan,
                true,
                DrlReorderMode::Always,
                1,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -512, col: 0 },
            },
        );
        assert_eq!(
            intrabc_ref_stack_admission(
                &IntrabcRefMvBank::new(32),
                geometry,
                &scan,
                true,
                DrlReorderMode::Always,
                0,
            ),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -1024, col: 0 },
            },
        );
    }

    #[test]
    fn spatial_scan_admits_sb_border_even_mi_col_step14() {
        let geom = SpatialScanGeometry {
            mi_row: 32,
            mi_col: 320,
            n4w: 8,
            n4h: 16,
            mi_rows: 270,
            mi_cols: 480,
            sb_size4: 32,
        };
        let scan = spatial_intrabc_scan(
            geom,
            |row, col| (row == 31 && col == 318).then_some(Mv { row: 0, col: -256 }),
            |_, _| false,
        );
        assert_eq!(scan.candidates, vec![wbv(Mv { row: 0, col: -256 }, 0)]);
        assert!(!scan.defer);
    }

    #[test]
    fn spatial_scan_defers_on_sb_border_odd_mi_col_above_neighbour() {
        let geom = SpatialScanGeometry {
            mi_row: 32,
            mi_col: 321,
            n4w: 8,
            n4h: 16,
            mi_rows: 270,
            mi_cols: 480,
            sb_size4: 32,
        };
        let scan = spatial_intrabc_scan(
            geom,
            |row, col| (row == 31 && col == 320).then_some(Mv { row: 0, col: -256 }),
            |_, _| false,
        );
        assert!(scan.candidates.is_empty());
        assert!(scan.defer);
    }

    /// A synthetic geometry at an even MiCol (so the SB-border 8x8 alignment is a
    /// no-op), large enough that every probe column stays inside the tile.
    fn generic_scan_geom(mi_row: usize, mi_col: usize, bw4: usize) -> SpatialScanGeometry {
        SpatialScanGeometry {
            mi_row,
            mi_col,
            n4w: bw4,
            n4h: 4,
            mi_rows: 64,
            mi_cols: 64,
            sb_size4: 32,
        }
    }

    /// Asserts `AboveRowScan::resolve`'s four above-row probe columns match the AVM
    /// `row_smvp_all_states[is_sb_boundary][block_width_type]` table entry. `tr_col`
    /// is the above-row top-right 4x4 the step-12 `has_top_right` gate consults (`None`
    /// when step 12 is disabled/unavailable), so a single helper drives every
    /// `[is_sb_boundary][block_width_type]` case (`label` cites the AVM row). The
    /// `expected` columns are `[step8, step10, step12, step14]` (`None` = disabled).
    fn assert_row_smvp_table(
        label: &str,
        mi_row: usize,
        bw4: usize,
        tr_col: Option<usize>,
        expected: [Option<usize>; 4],
    ) {
        let geom = generic_scan_geom(mi_row, 8, bw4);
        let scan = AboveRowScan::resolve(&geom, &|r, c| {
            Some((r, c)) == tr_col.map(|t| (mi_row - 1, t))
        });
        assert_eq!(scan.step8, expected[0], "{label} step8 column");
        assert_eq!(
            scan.step10, expected[1],
            "{label} step10 column (is_available)"
        );
        assert_eq!(
            scan.step12, expected[2],
            "{label} step12 column (Max(2,bw4) on border)"
        );
        assert_eq!(scan.step14, expected[3], "{label} step14 column");
        assert_eq!(scan.is_sb_border, mi_row.is_multiple_of(geom.sb_size4));
    }

    #[test]
    fn within_sb_above_row_columns_match_avm_table_all_widths() {
        assert_row_smvp_table(
            "within-SB BLOCK_WIDTH_4 row_smvp_all_states[0][0]",
            20,
            1,
            Some(9),
            [Some(8), None, Some(9), Some(7)],
        );
        assert_row_smvp_table(
            "within-SB BLOCK_WIDTH_8 row_smvp_all_states[0][1]",
            20,
            2,
            Some(10),
            [Some(9), Some(8), Some(10), Some(7)],
        );
        assert_row_smvp_table(
            "within-SB BLOCK_WIDTH_OTHERS row_smvp_all_states[0][2]",
            20,
            4,
            Some(12),
            [Some(11), Some(8), Some(12), Some(7)],
        );
    }

    #[test]
    fn sb_border_above_row_columns_match_avm_table_all_widths() {
        assert_row_smvp_table(
            "SB-border BLOCK_WIDTH_4 row_smvp_all_states[1][0]",
            32,
            1,
            None,
            [Some(8), None, Some(10), Some(6)],
        );
        assert_row_smvp_table(
            "SB-border BLOCK_WIDTH_8 row_smvp_all_states[1][1]",
            32,
            2,
            None,
            [Some(8), None, Some(10), Some(6)],
        );
        assert_row_smvp_table(
            "SB-border BLOCK_WIDTH_OTHERS row_smvp_all_states[1][2]",
            32,
            4,
            None,
            [Some(10), Some(8), Some(12), Some(6)],
        );
    }

    #[test]
    fn sb_border_narrow_disabled_step10_column_still_defers() {
        let geom = generic_scan_geom(32, 8, 2);
        let scan = spatial_intrabc_scan(
            geom,
            |row, col| (row == 31 && col == 7).then_some(Mv { row: 9, col: -9 }),
            |_, _| false,
        );
        assert!(
            scan.candidates.is_empty(),
            "no SB-border state reaches col 7"
        );
        assert!(
            scan.defer,
            "an unmodelled above-row column with a new BV defers"
        );
    }

    #[test]
    fn sb_border_block_width_4_step12_reads_max2_column() {
        let geom = generic_scan_geom(32, 8, 1);
        let at_10 = spatial_intrabc_scan(
            geom,
            |row, col| (row == 31 && col == 10).then_some(Mv { row: -8, col: -8 }),
            |_, _| false,
        );
        assert_eq!(at_10.candidates, vec![adj(Mv { row: -8, col: -8 })]);
        assert!(
            !at_10.defer,
            "step-12 Max(2,1)=2 column is modelled -> admitted"
        );
        let at_9 = spatial_intrabc_scan(
            geom,
            |row, col| (row == 31 && col == 9).then_some(Mv { row: -8, col: -8 }),
            |_, _| false,
        );
        assert!(
            at_9.candidates.is_empty(),
            "no state reaches MiCol+1 for bw4==1"
        );
        assert!(
            at_9.defer,
            "a new BV at the wrong (bw4=1) step-12 column defers"
        );
    }
}
