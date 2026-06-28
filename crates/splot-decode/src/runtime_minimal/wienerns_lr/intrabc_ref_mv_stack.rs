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

/// Builds the AV2 § 7.12.2 IBC ref-MV stack for an IntrABC block, given the
/// already-populated tile ref-MV bank (the state AVM holds when the block is
/// decoded). Returns the candidate stack (capped at `max_bvp_drl_bits_minus_1 +
/// 2`).
///
/// `enable_refmvbank` is the AV2 sequence-header flag: when `false` the § 7.12.2.21
/// ref-MV-bank fill is skipped entirely (AV2 only runs it when `enable_refmvbank ==
/// 1`), so the stack is default-fill only.
///
/// The spatial scan is NOT executed here: this is called only after the caller has
/// proven the spatial scan contributes no IBC candidate (every reachable ac0ej3
/// frame-0 IntrABC block has only the bank + default candidates). If the spatial
/// scan could contribute, the caller defers.
pub(super) fn build_intrabc_ref_mv_stack(
    bank: &IntrabcRefMvBank,
    geometry: IntrabcStackGeometry,
    enable_refmvbank: bool,
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

        let stack = build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(0, 112), true);
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

        let stack = build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(0, 232), true);
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
        let stack = build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(32, 8), true);
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

    // `enable_refmvbank == 0` makes AV2 run NO § 7.12.2.21 ref-MV-bank fill, so the
    // bank state is ignored and the stack is the four default block vectors only --
    // even when the bank holds a candidate that would otherwise reorder it. The
    // live path passes this flag through from the sequence header.
    #[test]
    fn stack_is_default_only_when_refmvbank_disabled() {
        let mut bank = IntrabcRefMvBank::new(32);
        bank.record_block(16, 56, 8, 16, true, Some(Mv { row: -512, col: 0 }));
        bank.record_block(0, 112, 8, 16, true, Some(Mv { row: 0, col: -256 }));
        let stack = build_intrabc_ref_mv_stack(&bank, ac0ej3_geometry(0, 232), false);
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
}
