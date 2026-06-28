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
            // Start of a new superblock row: zero the whole bank.
            self.queue.clear();
            self.remain_hits = 0;
            self.unit_hits = 0;
            self.sb_hits = 0;
            self.current_sb_row = Some(sb_row_start);
            self.current_sb_col = Some(sb_col_start);
        } else if self.current_sb_col != Some(sb_col_start) {
            // A new superblock in the same row: reset the per-SB hit counters
            // (the queue carries across superblocks within a row).
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
        // Every block (IBC or not) runs `decide_rmb_unit_update_count`: a non-IBC
        // block calls it directly (`decodemv.c:3204`); an IBC block calls it from
        // inside `av2_update_ref_mv_bank` (`mvref_common.c:4621`).
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
            // Move the existing entry to the end (most recent).
            let entry = self.queue.remove(pos);
            self.queue.push(entry);
            return;
        }
        if self.queue.len() < REF_MV_BANK_SIZE {
            self.queue.push(mv);
        } else {
            // Full buffer: drop the oldest, append the new entry.
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
    // C integer division truncates toward zero; `i32` division matches.
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

/// Outcome of the AV2 § 7.12.2 spatial SMVP scan for an IntrABC block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SpatialIntrabcScan {
    /// Spatial IntrABC neighbour block vectors in AV2 § 7.12.2 step order
    /// (deduped by value), contributed to the ref-MV stack BEFORE the
    /// § 7.12.2.21 ref-MV-bank fill and § 7.12.2.20 default fill.
    pub(super) candidates: Vec<Mv>,
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
/// NOTE: this reproduces the scan ORDER (steps 7 → 8 → 9 → 10 → 12 → 14), but NOT
/// the subsequent § 7.12.2.19 weight sort. AVM weights each candidate per § 7.12.2.6
/// and swaps the max-weight NEAREST candidate into slot 0 when `nearest_refmv_count
/// > 1` (`mvref_common.c:2472`-`2493`). The candidates returned here are in scan
/// order, UNSORTED; the > 1-candidate case is therefore not faithful and is guarded
/// by a fail-closed defer in [`intrabc_ref_stack_admission`]. With a single nearest
/// candidate the sort is a no-op, so the scan order is exact for the admitted set.
pub(super) fn spatial_intrabc_scan(
    geometry: SpatialScanGeometry,
    lookup: impl Fn(usize, usize) -> Option<Mv>,
    is_coded: impl Fn(usize, usize) -> bool,
) -> SpatialIntrabcScan {
    let row = geometry.mi_row;
    let col = geometry.mi_col;
    let bh4 = geometry.n4h;

    let mut candidates: Vec<Mv> = Vec::new();
    // The above-row columns this decoder models faithfully (with each probe's
    // availability): for a NON-SB-border block, the full within-SB scan (steps 8/10/
    // 12/14 at 4x4 resolution); for an SB-border block, only the step-8 aligned
    // column (#531). Columns NOT in this set stay unmodelled, so the over-scan keeps
    // deferring on any above-row new BV outside them.
    let above = AboveRowScan::resolve(&geometry, &is_coded);

    // Modelled positions in AV2 § 7.12.2.1 step order: step 7, step 8, step 9,
    // then the remaining within-SB above-row probes (steps 10, 12, 14) interleaved
    // with step 11 in AVM order. Step 11 (below-bottom-left) stays unmodelled.
    if let Some(left_col) = col.checked_sub(1) {
        // Step 7: (bh4 - 1, -1), gated left_available (col > 0 for the single tile).
        if let Some(r) = row.checked_add(bh4.saturating_sub(1)) {
            push_deduped(&geometry, &lookup, &mut candidates, r, left_col);
        }
    }
    // Step 8: row_smvp_state[0], before step 9 to match the AVM scan order
    // (`mvref_common.c:2371` before `:2382`).
    push_above_probe(&geometry, &lookup, &mut candidates, above.step8);
    // Step 9: (0, -1) when bh4 >= 2.
    if bh4 >= 2
        && let Some(left_col) = col.checked_sub(1)
    {
        push_deduped(&geometry, &lookup, &mut candidates, row, left_col);
    }
    // Step 10: row_smvp_state[1] (above-row deltaCol = 0).
    push_above_probe(&geometry, &lookup, &mut candidates, above.step10);
    // Step 11 (below-bottom-left) stays unmodelled — handled by the over-scan defer.
    // Step 12: row_smvp_state[2] (above-row top-right, deltaCol = bw4).
    push_above_probe(&geometry, &lookup, &mut candidates, above.step12);
    // Step 14: row_smvp_state[3] (above-left corner, deltaCol = -1).
    push_above_probe(&geometry, &lookup, &mut candidates, above.step14);

    // Unmodelled positions: defer only if one holds a NEW IntrABC block vector
    // (excluding the above-row columns now placed exactly by the scan above).
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
    /// apply (only step 8 is modellable here, and only when alignment is a no-op).
    is_sb_border: bool,
}

impl AboveRowScan {
    /// Resolves the modelled above-row probe columns (§ 7.12.2.6 `get_row_smvp_states`,
    /// `mvref_common.c:1996`-`2077`) for the block, using `is_coded` (AVM
    /// `is_mi_coded`) for the `has_top_right` per-4x4 availability gate.
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
        // No above row (frame/tile top): every above-row probe is unavailable, so
        // nothing is modelled and nothing is over-scanned.
        if above_row.is_none() {
            return scan;
        }
        if is_sb_border {
            // SB border: only the step-8 8x8-aligned column is modelled (#531); the
            // remaining SB-border above-row probes stay unmodelled (defer-as-before).
            scan.step8 = step8_above_row_column(geometry);
        } else {
            // Within the superblock: model the full 4x4-resolution above-row scan.
            scan.resolve_within_sb(geometry, is_coded);
        }
        scan
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
        // Step 8: deltaCol = bw4 - 1, gated up_available (the above row exists here).
        self.step8 = tile_above_col(geometry, col.checked_add(bw4.saturating_sub(1)));
        // Step 10: deltaCol = 0, gated up_available.
        self.step10 = tile_above_col(geometry, Some(col));
        // Step 12: deltaCol = bw4, the top-right probe, gated has_top_right (and the
        // AVM `bw4 <= Num_4x4_Blocks_Wide[BLOCK_64X64]` cap, `mvref_common.c:1554`).
        if bw4 <= MAX_SMVP_AXIS_MI
            && self.has_top_right(geometry, is_coded)
            && let Some(c) = tile_above_col(geometry, col.checked_add(bw4))
        {
            self.step12 = Some(c);
        }
        // Step 14: deltaCol = -1, gated up_available && left_available (MiCol > 0).
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
            // The top-right 4x4 is in the superblock on the right: not coded yet.
            return false;
        }
        // Within the current SB: consult the `is_mi_coded` decode-order signal at the
        // top-right 4x4 (MiRow - 1, MiCol + bw4). `mask_row - 1 >= 0` here (the SB-top
        // case is the SB-border branch), so the above MI is within the same SB.
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

/// Reads a modelled above-row probe (an `AboveRowScan` step column) at `(MiRow - 1,
/// col)` and adds its IntrABC block vector to the candidate list, deduped by value.
/// A `None` column is an unmodelled / unavailable probe and reads nothing.
fn push_above_probe(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    candidates: &mut Vec<Mv>,
    col: Option<usize>,
) {
    if let (Some(above), Some(c)) = (geometry.mi_row.checked_sub(1), col) {
        push_deduped(geometry, lookup, candidates, above, c);
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
    // up_available: there is an above row at all (frame/tile top has none).
    if row == 0 {
        return None;
    }
    // isSbBorder == 1: MiRow is a multiple of the SB side (Num_4x4_Blocks_High).
    if geometry.sb_size4 == 0 || !row.is_multiple_of(geometry.sb_size4) {
        return None;
    }
    // § 7.12.2.6 floors the probe column: `mvCol = (MiCol + deltaCol) >> 1 << 1`.
    // We model the case where that floor is a no-op (`aligned_col == MiCol +
    // deltaCol`) by checking `MiCol` parity below, which is sufficient for the
    // reachable set BECAUSE `deltaCol = Max(0, bw4 - 2)` is always EVEN here: every
    // reachable IBC block has an even `bw4` (the smallest IBC `bw4` is 2 = 8px; a
    // `bw4 == 1` (4px) block would give `deltaCol = 0`, still even). With an even
    // `deltaCol`, `MiCol + deltaCol` is even iff `MiCol` is even, so the `col & 1`
    // parity guard alone makes the floor a no-op. If a future odd `deltaCol` ever
    // became reachable (an odd `bw4`), the parity guard would NOT suffice and the
    // full floor `(MiCol + deltaCol) >> 1 << 1` would be required — assert it stays
    // even so that case fails loudly in debug instead of silently mis-aligning.
    let delta_col = geometry.n4w.saturating_sub(2);
    debug_assert!(
        delta_col.is_multiple_of(2),
        "step-8 deltaCol must be even for the MiCol-parity floor shortcut; \
         an odd deltaCol (odd bw4) needs the full (MiCol+deltaCol)>>1<<1 floor",
    );
    // The (mvCol >> 1) << 1 alignment must not shift the spec column, i.e. mi_col
    // must be even (given the even deltaCol above); an odd mi_col is not modelled.
    if col & 1 != 0 {
        return None;
    }
    let aligned_col = col.checked_add(delta_col)?;
    // is_inside: the neighbour column must lie inside the tile.
    if aligned_col >= geometry.mi_cols {
        return None;
    }
    Some(aligned_col)
}

/// Adds an IntrABC neighbour block vector at MI `(row_offset, left_col)` to the
/// running spatial candidate list, deduped by value (AV2 § 7.12.2.4
/// `mvref_common.c:874`).
fn push_deduped(
    geometry: &SpatialScanGeometry,
    lookup: &impl Fn(usize, usize) -> Option<Mv>,
    candidates: &mut Vec<Mv>,
    row_offset: usize,
    left_col: usize,
) {
    if let Some(mv) = lookup_in_grid(geometry, lookup, row_offset, left_col)
        && !candidates.contains(&mv)
    {
        candidates.push(mv);
    }
}

/// Whether any AV2 § 7.12.2 spatial position this decoder does NOT model exactly
/// (the step-11 below-bottom-left probe, the still-unmodelled above-row probes,
/// and the § 7.12.2.5 deltaCol = -3 non-adjacent scan) holds an IntrABC neighbour
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
    modelled: &[Mv],
    above: &AboveRowScan,
) -> bool {
    let row = geometry.mi_row;
    let col = geometry.mi_col;
    let bw4 = geometry.n4w;
    let is_new = |mv: Option<Mv>| mv.is_some_and(|mv| !modelled.contains(&mv));
    // Step-11 below-bottom-left probe (bh4, -1), gated by has_bottom_left state.
    if geometry.n4h <= MAX_SMVP_AXIS_MI
        && let Some(left_col) = col.checked_sub(1)
        && let Some(r) = row.checked_add(geometry.n4h)
        && is_new(lookup_in_grid(geometry, lookup, r, left_col))
    {
        return true;
    }
    // Above-row probes (deltaRow = -1). At the frame top edge there is no above
    // row, so nothing is read; otherwise over-scan the above row from the
    // step-14 above-left column out to the top-right column, EXCLUDING the columns
    // the [`AboveRowScan`] now places faithfully above.
    if let Some(above_row) = row.checked_sub(1) {
        let modelled_cols = [above.step8, above.step10, above.step12, above.step14];
        // The SB-border path aligns the above row to the 8x8 grid; its unmodelled
        // probes can reach MiCol - 2 - (MiCol & 1). The within-SB path reads the
        // above row at 4x4 resolution from MiCol - 1 (step 14). Over-scan the wider
        // SB-border span when on a border so an unmodelled SB-border probe still
        // defers; otherwise the within-SB span from the step-14 column.
        let extra_left = if above.is_sb_border { 2 + (col & 1) } else { 1 };
        let leftmost = col.saturating_sub(extra_left);
        let rightmost = col.saturating_add(bw4); // inclusive of the top-right col
        for c in leftmost..=rightmost {
            if modelled_cols.contains(&Some(c)) {
                continue;
            }
            if is_new(lookup_in_grid(geometry, lookup, above_row, c)) {
                return true;
            }
        }
    }
    // § 7.12.2.5 deltaCol = -3 non-adjacent left scan: positions (bh4 - 1, -3) and
    // (0, -3). AVM's `is_valid_candidate` skips a position that is the same block
    // as the adjacent deltaCol = -1 column; a same-block read has the same recorded
    // block vector, so the modelled-dedup test below subsumes that skip.
    if let Some(deep_col) = col.checked_sub(3) {
        let bottom = row.checked_add(geometry.n4h.saturating_sub(1));
        let probes = [bottom, Some(row)];
        for r in probes.into_iter().flatten() {
            if is_new(lookup_in_grid(geometry, lookup, r, deep_col)) {
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

    // § 7.12.2 spatial SMVP scan candidates (already deduped, in step order).
    for &cand in spatial {
        if stack.len() >= max_count {
            break;
        }
        stack.push(cand);
    }

    // § 7.12.2.21 fill from ref mv bank: iterate the bank in reverse (LIFO). Only
    // when `enable_refmvbank == 1` (AV2 gates both the fill and the bank update on
    // this flag); otherwise the bank contributes nothing.
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

    // § 7.12.2.20 extra search: the four IBC default block vectors, no dedup.
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

/// Decides AV2 § 7.12.2 IntrABC ref-MV stack admission: builds the real stack
/// (§ 7.12.2 spatial scan candidates + § 7.12.2.21 ref-MV bank fill + § 7.12.2.20
/// default block vectors) and returns the block vector the decoded DRL index
/// `ref_mv_idx` selects, so the caller can use the AVM-faithful candidate directly.
///
/// Defers (returns `Defer`) when:
///
/// * the § 7.12.2 spatial scan reaches a position this decoder does not model
///   faithfully (`spatial.defer`); or
/// * the spatial scan admits MORE THAN ONE distinct nearest candidate
///   (`spatial.candidates.len() > 1`): AVM § 7.12.2.19 sorts the NEAREST spatial
///   candidates by § 7.12.2.6 weight and swaps the max-weight one into slot 0 when
///   `nearest_refmv_count > 1` (`mvref_common.c:2472`-`2493`, gated by
///   `enable_drl_reorder`). This decoder does NOT model the per-candidate weights
///   or that reorder, so a >1-candidate stack could place the wrong block vector in
///   the DRL-selected slot. Fail closed until weights/sort are modelled. (For a
///   single nearest candidate the swap loop is a no-op, so it stays admissible.) or
/// * the decoded DRL index lands outside the built stack.
///
/// `enable_refmvbank` is the AV2 sequence flag gating the § 7.12.2.21 bank fill.
pub(super) fn intrabc_ref_stack_admission(
    bank: &IntrabcRefMvBank,
    geometry: IntrabcStackGeometry,
    spatial: &SpatialIntrabcScan,
    enable_refmvbank: bool,
    ref_mv_idx: usize,
) -> IntrabcStackAdmission {
    if spatial.defer || spatial.candidates.len() > 1 {
        return IntrabcStackAdmission::Defer;
    }
    let real_stack =
        build_intrabc_ref_mv_stack(bank, geometry, enable_refmvbank, &spatial.candidates);
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
        // AVM dump: at MI(0,112) the bank holds only MI(16,56)'s BV (-512, 0),
        // which check_rmb_cand REJECTS on the frame-boundary test (ref_y = 0*4 +
        // (-512/8) = -64 <= -block_height(64)). So the stack is default-only.
        let mut bank = IntrabcRefMvBank::new(32);
        // MI(16,56) records BV (-512, 0).
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
        // AVM dump: at MI(0,232) the bank holds [(-512,0), (0,-256)] (MI(16,56),
        // MI(0,112)). The LIFO fill tries (0,-256) first: ref_x = 232*4 + (-256/8)
        // = 928 - 32 = 896, in bounds -> ADMIT. Then (-512,0): ref_y = -64 <= -64
        // -> REJECT. Defaults fill the rest, giving the reordered stack.
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
        // (-512, 0) at MI(0, 112), block 64 high: ref_y = -64 <= -64 -> reject.
        assert!(!check_rmb_cand(
            Mv { row: -512, col: 0 },
            &[],
            bounds(0, 112),
        ));
    }

    #[test]
    fn check_rmb_cand_admits_in_bounds_candidate() {
        // (0, -256) at MI(0, 232), block 32 wide: ref_x = 896, in bounds -> admit.
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
        // A block in the next SB row (mi_row 32) zeroes the bank.
        bank.record_block(32, 0, 8, 16, true, Some(Mv { row: -1024, col: 0 }));
        assert_eq!(bank.entries(), [Mv { row: -1024, col: 0 }]);
    }

    // Finding 2: the FIRST block of a new SB row reads an EMPTY bank. After SB row 0
    // populates the bank, entering a block in SB row 1 (the entry-time reset) clears
    // the bank BEFORE the § 7.12.2.21 fill runs, so the built stack is default-only
    // (no stale previous-row candidate would defer a valid block).
    #[test]
    fn first_block_of_new_sb_row_reads_empty_bank() {
        let mut bank = IntrabcRefMvBank::new(32);
        // SB row 0 records a BV whose displaced ref would be in-bounds at SB row 1.
        bank.record_block(0, 0, 8, 16, true, Some(Mv { row: 0, col: -256 }));
        assert_eq!(bank.entries(), [Mv { row: 0, col: -256 }]);
        // Entering the first block of SB row 1 (mi_row 32) zeroes the bank...
        bank.enter_block_superblock(32, 8);
        assert!(bank.entries().is_empty());
        // ...so the stack built for that block is default-only (no stale candidate).
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

    // ac0ej3 frame-0 MI(0,112): the bank's MI(16,56) candidate is rejected on the
    // frame boundary, so the real stack is default-only and the DRL index (3)
    // selects the default tail (0,-256) -> ADMIT with that BV.
    #[test]
    fn admission_admits_ac0ej3_mi_0_112_default_only() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        assert_eq!(
            intrabc_ref_stack_admission(&bank, ac0ej3_geometry(0, 112), &no_spatial(), true, 3),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -256 },
            }
        );
    }

    // ac0ej3 frame-0 MI(0,232): the bank [(-512,0),(0,-256)] LIFO-reorders the stack
    // to [(0,-256),(-1024,0),(0,-3072),(-512,0)]; DRL index 2 selects (0,-3072), the
    // BV AVM records for this block. With `enable_refmvbank` OFF, AVM runs no bank
    // fill, so the stack is the default-only [(-1024,0),(0,-3072),(-512,0),(0,-256)]
    // and DRL index 2 selects the default (-512,0).
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

    // ac0ej3 frame-0 MI(0,240): the § 7.12.2 spatial scan adds the left neighbour
    // MI(0,232)'s BV (0,-3072) first; the bank [(-512,0),(0,-256),(0,-3072)] LIFO
    // fill dedups (0,-3072), admits (0,-256), rejects (-512,0); defaults fill the
    // rest. The DRL index 0 selects the spatial BV (0,-3072), matching the AVM
    // `setup_ref_mv_list` dump `stack=[(r0,c-3072)(r0,c-256)(r-1024,c0)(r0,c-3072)]
    // drl=0`.
    #[test]
    fn admission_selects_ac0ej3_mi_0_240_spatial_bv() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        bank.record_block(0, 112, 8, 16, true, Some(Mv { row: 0, col: -256 }));
        bank.record_block(0, 232, 8, 16, true, Some(Mv { row: 0, col: -3072 }));
        let spatial = SpatialIntrabcScan {
            candidates: vec![Mv { row: 0, col: -3072 }],
            defer: false,
        };
        let stack =
            build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(0, 240), true, &spatial.candidates);
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
            intrabc_ref_stack_admission(&bank, ac0ej3_geometry(0, 240), &spatial, true, 0),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: 0, col: -3072 },
            }
        );
    }

    // An unmodelled § 7.12.2 spatial position holding an IntrABC neighbour defers.
    #[test]
    fn admission_defers_on_unmodelled_spatial_intrabc() {
        let bank = IntrabcRefMvBank::new(32);
        let spatial = SpatialIntrabcScan {
            candidates: Vec::new(),
            defer: true,
        };
        assert_eq!(
            intrabc_ref_stack_admission(&bank, ac0ej3_geometry(0, 112), &spatial, true, 0),
            IntrabcStackAdmission::Defer
        );
    }

    // A block whose spatial scan admits MORE THAN ONE distinct nearest candidate
    // DEFERS (fail-closed): AVM § 7.12.2.19 may swap the max-weight candidate into
    // slot 0 (`nearest_refmv_count > 1`), and this decoder does not model the
    // weights/sort. A single nearest candidate still admits (the swap is a no-op).
    #[test]
    fn admission_defers_on_multiple_spatial_candidates() {
        let bank = IntrabcRefMvBank::new(32);
        let two = SpatialIntrabcScan {
            candidates: vec![Mv { row: 0, col: -64 }, Mv { row: -512, col: 0 }],
            defer: false,
        };
        assert_eq!(
            intrabc_ref_stack_admission(&bank, ac0ej3_geometry(0, 240), &two, true, 0),
            IntrabcStackAdmission::Defer,
            "two distinct spatial candidates must defer until the §7.12.2.19 sort is modelled"
        );
        // The single-candidate case is unaffected (admits).
        let one = SpatialIntrabcScan {
            candidates: vec![Mv { row: 0, col: -64 }],
            defer: false,
        };
        assert!(matches!(
            intrabc_ref_stack_admission(&bank, ac0ej3_geometry(0, 240), &one, true, 0),
            IntrabcStackAdmission::Admit { .. }
        ));
    }

    // The § 7.12.2 spatial scan: a left-column IntrABC neighbour at the bottom-left
    // position (step 7) contributes its BV; a within-SB above-row IntrABC neighbour
    // at a MODELLED column (step 10, deltaCol = 0) is admitted, but one at a column
    // NO modelled step reaches (e.g. the § 7.12.2.5 deltaCol = -3 deep-left scan)
    // forces a (safe) defer.
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
        // Left neighbour at the bottom-left (step 7) position (mi_row+bh4-1, mi_col-1)
        // = (7, 7) is IntrABC with BV (0, -64).
        let left_only = spatial_intrabc_scan(
            geom,
            |row, col| (row == 7 && col == 7).then_some(Mv { row: 0, col: -64 }),
            |_, _| false,
        );
        assert_eq!(left_only.candidates, vec![Mv { row: 0, col: -64 }]);
        assert!(!left_only.defer);
        // A within-SB above-row IntrABC neighbour at the step-10 column (3, 8)
        // (deltaCol = 0) is now MODELLED -> admitted, no defer.
        let above = spatial_intrabc_scan(
            geom,
            |row, col| (row == 3 && col == 8).then_some(Mv { row: -8, col: 0 }),
            |_, _| false,
        );
        assert_eq!(above.candidates, vec![Mv { row: -8, col: 0 }]);
        assert!(!above.defer);
        // An IntrABC neighbour at the § 7.12.2.5 deltaCol = -3 deep-left scan position
        // (mi_row, mi_col - 3) = (4, 5), which no modelled step reaches, still defers.
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

    // The § 7.12.2.1 step-8 above-row SMVP candidate, SB-border + even-mi_col case:
    // MI(32,56) on an SB border reads the SB-aligned above neighbour at
    // (row-1, mi_col + Max(0,bw4-1-isSbBorder)) = (31, 56+6) = (31,62), which lies
    // inside the SB-row-0 owner block MI(16,56) carrying its IntrABC BV (-512,0).
    // The scan admits it (no defer), matching the avmdec SPLOT_IBC_DUMP
    // `PREDEF count=1 : [0](-512,0 ro=-1 co=6)`.
    #[test]
    fn spatial_scan_admits_ac0ej3_mi_32_56_step8_above_neighbour() {
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_32_56_scan_geometry(),
            |row, col| (row == 31 && col == 62).then_some(Mv { row: -512, col: 0 }),
            |_, _| false,
        );
        assert_eq!(scan.candidates, vec![Mv { row: -512, col: 0 }]);
        assert!(!scan.defer);
    }

    // The full MI(32,56) admission: with the modelled step-8 candidate (-512,0)
    // first, the SB-row-1 bank is empty (zeroed at SB-row entry), so the stack is
    // [(-512,0)] + the four § 7.12.2.20 defaults, capped at max_bvp_drl_bits_minus_1
    // + 2 = 4 -> [(-512,0),(-1024,0),(0,-3072),(-512,0)]. DRL index 0 selects
    // (-512,0), bit-exact vs the avmdec dump
    // `FINAL [0](-512,0)[1](-1024,0)[2](0,-3072)[3](-512,0)` drl=0 decoded=(-512,0).
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
        let stack = build_intrabc_ref_mv_stack(&bank, geometry, true, &spatial.candidates);
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
            intrabc_ref_stack_admission(&bank, geometry, &spatial, true, 0),
            IntrabcStackAdmission::Admit {
                selected: Mv { row: -512, col: 0 },
            }
        );
    }

    // Narrowing precision: a still-unmodelled above-row column (NOT the modelled
    // step-8 aligned column) holding a NEW IntrABC BV STILL defers. For MI(32,56)
    // the modelled step-8 column is 62; a NEW BV at a DIFFERENT above-row column
    // (e.g. the step-12 `Max(2,bw4)` probe at col 64, or the step-14 above-left)
    // must keep deferring — proving the exclusion is precise, not a blanket open.
    #[test]
    fn spatial_scan_defers_on_other_above_row_column() {
        // A new BV at above-row col 64 (an unmodelled SB-border step-12-class column)
        // defers — on the SB-border path only step 8 is modelled.
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_32_56_scan_geometry(),
            |row, col| (row == 31 && col == 64).then_some(Mv { row: 7, col: -99 }),
            |_, _| false,
        );
        assert!(scan.candidates.is_empty());
        assert!(scan.defer);
        // The modelled step-8 column AND an unmodelled column together: the modelled
        // BV is admitted, but the distinct unmodelled-column BV still forces a defer.
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
        assert_eq!(scan.candidates, vec![Mv { row: -512, col: 0 }]);
        assert!(scan.defer);
    }

    // Asserts that a block whose step-8 above-row probe is unmodelled (an SB-border
    // case) both reports no modelled step-8 column AND defers when its above row
    // holds an IntrABC neighbour.
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

    // An odd-mi_col SB-border block does NOT model step 8 (the `(mvCol >> 1) << 1`
    // alignment would shift the spec column), so an above-row IntrABC neighbour at
    // any column forces a conservative defer.
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

    // The within-SB (non-SB-border) above-row scan now models step 8 at 4x4
    // resolution: MI(48,56) reads the step-8 above neighbour at
    // (MiRow - 1, MiCol + bw4 - 1) = (47, 63), inside the SB-row-1 owner block
    // MI(32,56) carrying BV (-512,0). Steps 10/14 read (47,56)/(47,55); the step-10
    // (47,56) duplicates (-512,0); step 12's top-right (47,64) is in the next SB
    // (has_top_right = 0) so it is not read. The scan admits the single distinct
    // candidate (-512,0), bit-exact vs the avmdec dump
    // `PREDEF count=1 : [0](-512,0 ro=-1 co=7)` decoded=(-512,0).
    #[test]
    fn spatial_scan_admits_ac0ej3_mi_48_56_within_sb_step8() {
        let scan = spatial_intrabc_scan(
            ac0ej3_mi_48_56_scan_geometry(),
            |row, col| {
                // The whole SB-row-1 owner block MI(32,56) (rows 32..=47, cols 56..=63)
                // carries BV (-512,0); the step-14 left col 55 and step-12 top-right
                // col 64 belong to OTHER (non-IBC here) blocks.
                (row == 47 && (56..=63).contains(&col)).then_some(Mv { row: -512, col: 0 })
            },
            // Within-SB top-right (47,64) is in the next SB, so has_top_right's
            // tr_mask_col >= sb_size4 short-circuit returns 0 regardless of is_coded.
            |_, _| true,
        );
        assert_eq!(scan.candidates, vec![Mv { row: -512, col: 0 }]);
        assert!(!scan.defer);
    }

    // The within-SB step-12 top-right probe (deltaCol = bw4) is gated by
    // has_top_right (is_mi_coded at the within-SB top-right 4x4). When the top-right
    // is NOT coded yet, AVM does not read it: the probe is unavailable, so a NEW BV
    // there is neither admitted nor a defer trigger (the not-yet-coded position holds
    // no candidate AVM sees). When it IS coded AND within the same SB, the probe is
    // modelled and its BV is admitted.
    #[test]
    fn spatial_scan_step12_top_right_respects_has_top_right() {
        // A within-SB top-right at (MiRow-1, MiCol+bw4) that stays inside the SB.
        // MI(20,8), bw4 = 4, sb_size4 = 32: mask_col = 8, tr_mask_col = 12 < 32 -> the
        // top-right 4x4 is (19, 12), within the SB. With is_coded false there, AVM's
        // has_top_right is 0 -> the probe is unavailable (no admit, no defer).
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
        let not_coded = spatial_intrabc_scan(
            geom,
            |row, col| (row == 19 && col == 12).then_some(bv),
            |_, _| false, // top-right 4x4 not coded -> has_top_right = 0
        );
        assert!(not_coded.candidates.is_empty());
        assert!(!not_coded.defer);
        // With the top-right 4x4 coded, has_top_right = 1 -> the step-12 probe is
        // modelled and its distinct BV is admitted.
        let coded = spatial_intrabc_scan(
            geom,
            |row, col| (row == 19 && col == 12).then_some(bv),
            |row, col| row == 19 && col == 12,
        );
        assert_eq!(coded.candidates, vec![bv]);
        assert!(!coded.defer);
    }

    // The § 7.12.2 spatial scan dedups by value: the same neighbour read at both the
    // step-7 (bh4-1,-1) and step-9 (0,-1) left-column positions contributes once.
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
        // The left column the block spans (rows 0..=15) holds the SAME IntrABC
        // neighbour BV; the step-11 below-bottom-left position (16,7) is NOT IBC.
        // MiRow 0 -> no above row, so the above-row scan reads nothing.
        let scan = spatial_intrabc_scan(
            geom,
            |row, col| (col == 7 && row < 16).then_some(Mv { row: 0, col: -3072 }),
            |_, _| false,
        );
        assert_eq!(scan.candidates, vec![Mv { row: 0, col: -3072 }]);
        assert!(!scan.defer);
    }
}
