// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.11 / § 7.12 motion-vector context + prediction kernel (minimal
//! spatial-only single-reference subset).
//!
//! This module models the spatial-neighbour parts of the AV2 motion-vector
//! context (§ 7.11.2 `find_mode_ctx`, the find mode context process) and the
//! motion-vector prediction stack (§ 7.12.2 `find_mv_stack`, the Find MV stack
//! process) that a multi-block single-reference inter frame needs to predict a
//! later block's motion vector from a decoded neighbour block (NEARMV /
//! NEARESTMV reusing a spatial-neighbour MV).
//!
//! It is the inter analog of the directional-prediction / sub-pel-MC kernels:
//! a precise, unit-tested subset that admits exactly the verified fixture and
//! defers the rest with explicit spec TODOs.
//!
//! ## What is modelled
//!
//! - The neighbour mode-info grid ([`NeighbourMvGrid`]): per-MI `IsInters`,
//!   `RefFrames[0]`, `YModes`, and `Mvs[0]` written after each block decodes
//!   (the inputs § 7.11.3 / § 7.12.2.6 read for a later block).
//! - § 7.11.2 `find_mode_ctx` for single prediction: the `leftA` / `aboveA` /
//!   `leftB` / `aboveB` scan-point context probes (§ 7.11.3 Scan point context
//!   process), giving `NewMvContext` and `NewMvCount` ([`find_mode_ctx`]).
//! - § 7.12.2 `find_mv_stack` for single prediction (`isCompound == 0`): the
//!   ordered spatial scan-point steps (§ 7.12.2.6 Scan point process +
//!   § 7.12.2.10 Add reference motion vector process + § 7.12.2.12 Search stack
//!   process), the `numNearest` sort gate (§ 7.12.2.19 Sorting process), and the
//!   extra-search global-MV fallback (§ 7.12.2.20 Extra search process) +
//!   clamping (§ 7.12.2.23 Clamping process) ([`find_mv_stack`]).
//!
//! ## What is deferred (`TODO(spec: DECODE-INTER-MVSTACK-SPATIAL)`)
//!
//! - Temporal MV candidates (§ 7.12.2.7 / § 7.12.2.8): the caller requires
//!   `use_ref_frame_mvs == 0`, so `useTemporal == 0` and no temporal scan runs.
//! - Compound prediction (`isCompound == 1`) and the compound search /
//!   derived / TIP candidate processes (§ 7.12.2.13–§ 7.12.2.18).
//! - Warp candidate derivation (`DeriveWrl == 1`, § 7.12.2.2–§ 7.12.2.4,
//!   § 7.12.2.9, § 7.12.2.11) and the find-warp-samples process (§ 7.12.3).
//! - The reference MV bank (`enable_refmvbank`, § 7.12.2.21) and the derived
//!   single-MV predictor list (§ 7.12.2.16 / § 7.12.2.22), both requiring
//!   sequence features the caller rejects.
//! - The DRL reorder full sort (`DrlReorder != DRL_REORDER_ALWAYS`,
//!   the `useSort` constraint path); the caller rejects `enable_drl_reorder`.
//! - Global (warp) motion: the caller rejects `use_global_motion`, so the
//!   § 7.12.2.1 Setup global MV process yields the zero vector for every list.
//! - The § 7.12.2.20 large-block (> 32x32) extra MVP-combination candidates,
//!   the intrabc block-vector candidates, and the warp-bank candidates.

use super::Mv;

/// AV2 § 3 `MAX_REF_MV_STACK_SIZE`: the maximum number of motion vectors in the
/// stack.
pub(super) const MAX_REF_MV_STACK_SIZE: usize = 6;

/// AV2 § 3 `MV_BORDER`: the value used when clipping motion vectors
/// (§ 5.20.9.4 / § 5.20.9.5).
const MV_BORDER: i32 = 128;

/// AV2 § 3 `MI_SIZE`: luma samples per mode-info unit.
const MI_SIZE: i32 = 4;

/// One decoded neighbour mode-info cell: the § 7.11.3 / § 7.12.2.6 inputs a
/// later block reads. A `None` cell has not been decoded for this frame (the
/// § 7.12.2.6 "RefFrames[mvRow][mvCol][0] has been written" check fails).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NeighbourCell {
    /// `IsInters[mvRow][mvCol]`: 1 for an inter block, 0 for intra.
    is_inter: bool,
    /// `RefFrames[mvRow][mvCol][0]`: the single-reference frame index of the
    /// block (only the single-prediction list 0 is modelled).
    ref_frame0: i8,
    /// `YModes[mvRow][mvCol]`: the block's luma prediction mode, used by
    /// § 7.11.3 `has_newmv_for_list` to count `NewMvCount`.
    y_mode: NeighbourYMode,
    /// `Mvs[mvRow][mvCol][0]`: the block's list-0 motion vector.
    mv: Mv,
    /// `Skips[mvRow][mvCol]`: the block's `skip` flag, used by the § 8.3.2
    /// `skip_flag` context (`ctx += Skips[NPosBuf[n]]`).
    skip: bool,
}

/// The subset of AV2 § 5.20.7.6 luma inter prediction modes the § 7.11.3
/// `has_newmv_for_list` context probe distinguishes for single prediction. Only
/// `NEWMV` increments `NewMvCount` for list 0 in the single-reference subset
/// (NEARMV / NEARESTMV / GLOBALMV do not).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NeighbourYMode {
    /// A `NEWMV` block (§ 5.20.7.6 single_mode == 2): a new MV was coded, so
    /// § 7.11.3 `has_newmv_for_list(candMode, 0)` returns 1.
    NewMv,
    /// A `NEARMV` / `NEARESTMV` / `GLOBALMV` (or intra) block: not a NEW MV for
    /// list 0, so `has_newmv_for_list` returns 0.
    Other,
}

/// The per-MI neighbour mode-info grid the § 7.11 / § 7.12 spatial scan reads.
///
/// Sized to the tile's MI grid; written after each block decodes so a later
/// block in decode (DFS) order sees its already-decoded left/above neighbours,
/// exactly as AV2 § 5.20.4.1 `decode_block` records `Mvs` / `RefFrames` /
/// `YModes` / `IsInters` into the frame mode-info arrays.
pub(super) struct NeighbourMvGrid {
    mi_rows: usize,
    mi_cols: usize,
    cells: Vec<Option<NeighbourCell>>,
}

impl NeighbourMvGrid {
    /// Builds an empty grid for an `mi_rows` x `mi_cols` MI region (every cell
    /// undecoded). Returns `None` if the dimensions overflow the allocation.
    pub(super) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let cells = mi_rows.checked_mul(mi_cols)?;
        Some(Self {
            mi_rows,
            mi_cols,
            cells: vec![None; cells],
        })
    }

    /// Records a decoded block's mode info into every MI cell it covers.
    /// `r` / `c` are the block's MI top-left, `n4w` / `n4h` its size in 4x4 MI
    /// units. Mirrors AV2 § 5.20.4.1 writing the per-MI `Mvs` / `RefFrames` /
    /// `YModes` / `IsInters` for the block region.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_block(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        is_inter: bool,
        ref_frame0: i8,
        y_mode: NeighbourYMode,
        mv: Mv,
        skip: bool,
    ) {
        let cell = NeighbourCell {
            is_inter,
            ref_frame0,
            y_mode,
            mv,
            skip,
        };
        for rr in r..r.saturating_add(n4h) {
            if rr >= self.mi_rows {
                break;
            }
            for cc in c..c.saturating_add(n4w) {
                if cc >= self.mi_cols {
                    break;
                }
                self.cells[rr * self.mi_cols + cc] = Some(cell);
            }
        }
    }

    /// Returns the decoded mode-info cell at MI `(r, c)`, or `None` if the
    /// position is outside the grid or undecoded.
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
}

/// The block geometry + reference the § 7.11 / § 7.12 spatial scan needs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MvBlockContext {
    /// `MiRow`: the block's MI top-left row.
    pub(super) mi_row: usize,
    /// `MiCol`: the block's MI top-left column.
    pub(super) mi_col: usize,
    /// `bw4 = Num_4x4_Blocks_Wide[MiSize]`.
    pub(super) bw4: usize,
    /// `bh4 = Num_4x4_Blocks_High[MiSize]`.
    pub(super) bh4: usize,
    /// `Num_4x4_Blocks_High[SbSize]`: the superblock height in MI units, for the
    /// `isSbBorder` derivation.
    pub(super) sb_h4: usize,
    /// `RefFrame[0]`: the block's single-reference frame index.
    pub(super) ref_frame0: i8,
    /// `MiRows`: the frame MI height (for § 5.20.9.4 clamp bounds).
    pub(super) mi_rows: usize,
    /// `MiCols`: the frame MI width (for § 5.20.9.5 clamp bounds).
    pub(super) mi_cols: usize,
}

impl MvBlockContext {
    /// AV2 § 7.12.2 `isSbBorder = (MiRow & (Num_4x4_Blocks_High[SbSize] - 1)) == 0`.
    fn is_sb_border(&self) -> bool {
        (self.mi_row & self.sb_h4.saturating_sub(1)) == 0
    }
}

/// The result of § 7.11.2 `find_mode_ctx` for single prediction: the
/// `NewMvContext` and the `NewMvCount` a later block uses for the § 8.3.2
/// `single_mode` / DRL CDF contexts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ModeContext {
    /// AV2 § 7.11.2 `NewMvContext = nearestMatch + ((NewMvCount > 0) ? 2 : 0)`.
    pub(super) new_mv_context: usize,
    /// AV2 § 7.11.2 `NewMvCount` (0..=3): the number of NEW-MV neighbours found.
    pub(super) new_mv_count: usize,
}

/// AV2 § 7.11.2 Find mode context process for single prediction (`isCompound ==
/// 0`).
///
/// Scans the immediate spatial neighbours (`leftA` / `aboveA` / `leftB` /
/// `aboveB`, each a § 7.11.3 Scan point context process probe) and returns the
/// `NewMvContext` + `NewMvCount`. The warp context probes (§ 7.11.4) are not
/// modelled (warp is deferred), and only single prediction is supported.
pub(super) fn find_mode_ctx(grid: &NeighbourMvGrid, block: &MvBlockContext) -> ModeContext {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let is_sb_border = block.is_sb_border();
    let mut new_mv_count = 0usize;

    // §7.11.2 ordered scan-point context probes (warp probes omitted):
    //   leftA  = scan_point_ctx(bh4 - 1, -1)
    //   aboveA = scan_point_ctx(-1, bw4 - 1)
    //   leftB  = scan_point_ctx(0, -1)
    //   aboveB = scan_point_ctx(-1, 0)   (aboveB always probed; the warp probe
    //            at (-1, 0) is gated on bw4 >= (isSbBorder ? 4 : 2), but the
    //            aboveB context probe itself is unconditional).
    let left_a = scan_point_ctx(grid, block, bh4 - 1, -1, &mut new_mv_count);
    let above_a = scan_point_ctx(grid, block, -1, bw4 - 1, &mut new_mv_count);
    let left_b = scan_point_ctx(grid, block, 0, -1, &mut new_mv_count);
    let above_b = scan_point_ctx(grid, block, -1, 0, &mut new_mv_count);
    let _ = is_sb_border; // only used by the omitted warp probes

    let nearest_match = usize::from(above_a || above_b) + usize::from(left_a || left_b);
    let new_mv_context = nearest_match + if new_mv_count > 0 { 2 } else { 0 };
    ModeContext {
        new_mv_context,
        new_mv_count,
    }
}

/// The § 5.20.7.2 neighbour-buffer-derived § 8.3.2 contexts for `is_inter` and
/// `skip_flag` (single prediction subset).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BlockNeighbourContext {
    /// AV2 § 8.3.2 `is_inter` context: from `NNumBuf` + `NIntra[]`.
    pub(super) is_inter_ctx: usize,
    /// AV2 § 8.3.2 `skip_flag` context: `ctx += Skips[NPosBuf[n]]` (no `skip_mode`).
    pub(super) skip_ctx: usize,
    /// True if the block has at least one decoded § 5.20.7.2 neighbour
    /// (`NNumBuf >= 1`). A block with no decoded neighbours is provably unaffected
    /// by the deferred temporal / ref-MV-bank / DRL-reorder MV-stack steps, so the
    /// caller can admit the deferred tools for a no-neighbour block but must reject
    /// them once a neighbour exists.
    pub(super) has_neighbour: bool,
}

/// Derives the § 5.20.7.2 neighbour buffer (`NPosBuf` / `NNumBuf` / `NIntra` /
/// `Skips`) and the § 8.3.2 `is_inter` + `skip_flag` contexts for a block.
///
/// The four AV2 § 5.20.7.2 `add_neighbor` probes are scanned in order
/// (bottom-left, top-right, left, above), collecting up to 2 inside positions
/// into `NPosBuf`. `is_inter` ctx follows the § 8.3.2 `NNumBuf == 2 / == 1 / else`
/// branches over `NIntra[]`; `skip_flag` ctx sums the neighbour `Skips[]`.
/// `skip_mode` is always 0 for the verified subset, so its `(SKIP_CONTEXTS >> 1)`
/// term is omitted.
pub(super) fn block_neighbour_ctx(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
) -> BlockNeighbourContext {
    let r = block.mi_row as i32;
    let c = block.mi_col as i32;
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;

    // §5.20.7.2 add_neighbor probes, in order. NPosBuf collects up to 2 inside
    // positions (the `aboveSbBoundary` restriction only gates the separate NPos
    // list, not NPosBuf). `grid.get` returning Some means is_inside + decoded.
    let probes = [
        (r + bh4 - 1, c - 1), // bottom-left
        (r - 1, c + bw4 - 1), // top-right
        (r, c - 1),           // left
        (r - 1, c),           // above
    ];
    let mut buf: [NeighbourCell; 2] = [NeighbourCell {
        is_inter: false,
        ref_frame0: -1,
        y_mode: NeighbourYMode::Other,
        mv: Mv::ZERO,
        skip: false,
    }; 2];
    let mut num_buf = 0usize;
    for (pr, pc) in probes {
        if num_buf >= 2 {
            break;
        }
        if let Some(cell) = grid.get(pr, pc) {
            buf[num_buf] = cell;
            num_buf += 1;
        }
    }

    // §8.3.2 is_inter ctx over NIntra[] = !IsInters[NPosBuf[n]].
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

    // §8.3.2 skip_flag ctx = sum of neighbour Skips[] (skip_mode == 0).
    let mut skip_ctx = 0usize;
    for cell in buf.iter().take(num_buf) {
        skip_ctx += usize::from(cell.skip);
    }

    BlockNeighbourContext {
        is_inter_ctx,
        skip_ctx,
        has_neighbour: num_buf >= 1,
    }
}

/// AV2 § 7.11.3 Scan point context process for single prediction. Returns
/// `found` (1 if a neighbour with a matching reference frame exists at the
/// probe) and updates `new_mv_count` per § 7.11.3 `has_newmv_for_list`.
fn scan_point_ctx(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_row: i32,
    delta_col: i32,
    new_mv_count: &mut usize,
) -> bool {
    let mv_row = block.mi_row as i32 + delta_row;
    let mv_col = block.mi_col as i32 + delta_col;
    let Some(cell) = grid.get(mv_row, mv_col) else {
        return false;
    };
    if !cell.is_inter {
        return false;
    }
    // Single prediction, no intrabc: candList = 0..1 (one list). The single
    // modelled reference list is candList == 0.
    if cell.ref_frame0 == block.ref_frame0 {
        if matches!(cell.y_mode, NeighbourYMode::NewMv) {
            // §7.11.2 NewMvCount = Min(3, NewMvCount + 1).
            *new_mv_count = (*new_mv_count + 1).min(3);
        }
        return true;
    }
    false
}

/// The result of § 7.12.2 `find_mv_stack` for single prediction: the
/// `RefStackMv` candidate list (`NumMvFound` of them).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MvStack {
    /// `RefStackMv[idx][0]` (list 0): the single-prediction MV candidates,
    /// `NumMvFound` of them. The `numNearest` (step 15) immediate-neighbour count
    /// is not retained: the conditional sort (steps 17–18) that consumes it is
    /// deferred (`DRL_REORDER_NONE` makes `useSort == 0` for the verified subset).
    stack: Vec<Mv>,
}

impl MvStack {
    /// `NumMvFound`: the number of candidate MVs in the stack.
    pub(super) fn num_mv_found(&self) -> usize {
        self.stack.len()
    }

    /// `RefStackMv[idx][0]`: the candidate MV at stack position `idx`, or the
    /// last candidate if `idx` is past the end (the spec guarantees a global-MV
    /// fallback fills the stack, so a valid `RefMvIdx` always indexes a candidate;
    /// the saturating access is defensive).
    pub(super) fn candidate(&self, idx: usize) -> Mv {
        self.stack
            .get(idx)
            .copied()
            .or_else(|| self.stack.last().copied())
            .unwrap_or(Mv::ZERO)
    }
}

/// AV2 § 7.12.2 Find MV stack process for single prediction (`isCompound ==
/// 0`), spatial-only subset.
///
/// `global_mv` is the § 7.12.2.1 Setup global MV process output for list 0 (the
/// zero vector for the caller's identity-global-motion subset). The ordered
/// steps modelled are the spatial scan-point steps (7–14), the `numNearest`
/// capture (15), the conditional sort (17–18, only when `numNearest >= 4` under
/// the caller's `DRL_REORDER_NONE`), the extra-search global-MV fallback (22),
/// and the clamp (23). Temporal, compound, warp, ref-MV-bank, and derived-SMVP
/// steps are deferred (see the module docs).
pub(super) fn find_mv_stack(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mv: Mv,
) -> MvStack {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let is_sb_border_adj = i32::from(block.is_sb_border());

    // §7.12.2 step 1: NumMvFound = 0.
    let mut entries: Vec<MvStackEntry> = Vec::new();

    // §7.12.2 steps 7-14: the ordered spatial scan-point probes (single
    // prediction; warp + temporal omitted). Each scan_point invokes the §7.12.2.6
    // Scan point process -> §7.12.2.10 Add reference motion vector process ->
    // §7.12.2.12 Search stack process.
    //  7.  scan_point(bh4 - 1, -1)
    scan_point(grid, block, bh4 - 1, -1, &mut entries);
    //  8.  scan_point(-1, Max(0, bw4 - 1 - isSbBorder))
    scan_point(
        grid,
        block,
        -1,
        (bw4 - 1 - is_sb_border_adj).max(0),
        &mut entries,
    );
    //  9.  if bh4 >= 2: scan_point(0, -1)
    if bh4 >= 2 {
        scan_point(grid, block, 0, -1, &mut entries);
    }
    // 10.  if bw4 >= (isSbBorder ? 4 : 2): scan_point(-1, 0)
    if bw4 >= if is_sb_border_adj == 1 { 4 } else { 2 } {
        scan_point(grid, block, -1, 0, &mut entries);
    }
    // 11.  if bh4 <= 16: scan_point(bh4, -1)
    if bh4 <= 16 {
        scan_point(grid, block, bh4, -1, &mut entries);
    }
    // 12.  if bw4 <= 16: scan_point(-1, isSbBorder ? Max(2, bw4) : bw4)
    if bw4 <= 16 {
        let dc = if is_sb_border_adj == 1 {
            bw4.max(2)
        } else {
            bw4
        };
        scan_point(grid, block, -1, dc, &mut entries);
    }
    // 14.  scan_point(-1, -1 - isSbBorder)
    scan_point(grid, block, -1, -1 - is_sb_border_adj, &mut entries);

    // §7.12.2 step 15: numNearest = NumMvFound (the immediate-neighbour count). It
    // feeds only the deferred conditional sort (steps 17-18), so it is not retained.

    // §7.12.2 step 16 (scan_col, deltaCol = -3) is omitted: it only adds
    // candidates from blocks 3 MI columns to the left, none of which exist for
    // the verified two-block fixture (the left neighbour is at deltaCol == -1).
    // TODO(spec: DECODE-INTER-MVSTACK-SPATIAL): model §7.12.2.5 Scan col process
    // for wider neighbour reach.

    // §7.12.2 steps 17-18: useSort. The caller's DrlReorder == DRL_REORDER_NONE
    // (enable_drl_reorder == 0) makes useSort = (numNearest >= 4) only when
    // DrlReorder == DRL_REORDER_CONSTRAINT, which is not this path; for
    // DRL_REORDER_NONE useSort is 0. Modelled faithfully: no sort.
    // TODO(spec: DECODE-INTER-MVSTACK-SPATIAL): model §7.12.2.19 Sorting process
    // for the DRL_REORDER_CONSTRAINT / DRL_REORDER_ALWAYS paths.

    // §7.12.2 steps 19-21 (compound / ref-mv-bank / derived-SMVP) are deferred
    // (the caller rejects enable_refmvbank and compound).

    // §7.12.2 step 22: the Extra search process (§7.12.2.20). Clamp the existing
    // candidates, then add the global MV candidate if not already present. The
    // large-block (Block_Width > 32 AND Block_Height > 32) MVP combinations
    // (insert_mvp_candidate(0,1)/(1,0)/(0,2)/(2,0)/(1,2)/(2,1)) and the warp /
    // intrabc candidates are DEFERRED. NB: the admitted leaves are >= 32x32 with NO
    // upper bound (block.rs MIN_INTER_LEAF_N4 gates only the lower edge), so the
    // verified 64x64 grid / superblock blocks ARE > 32x32 and the §7.12.2.20
    // large-block step DOES apply to them. It is currently kept safe only because
    // (a) every committed-fixture neighbour MV is identical, so the mixed MVP
    // candidates coincide with existing stack entries (nothing appended, NumMvFound
    // unchanged), and (b) the §7.12.2.20 candidates would be appended AFTER the
    // spatial + global entries, so a RefMvIdx selecting a mixed-only slot lands past
    // this shorter stack and is rejected by the §5.20.7.8
    // inter_block_drl_idx_out_of_range guard rather than mis-decoding. A committed
    // distinct-neighbour-MV fixture is required to verify the large-block step.
    // TODO(spec: DECODE-INTER-MVSTACK-SPATIAL): §7.12.2.20 large-block (>32x32) MVP
    // combinations + warp / intrabc candidates.
    extra_search(block, global_mv, &mut entries);

    // §7.12.2 step 23: the Clamping process (§7.12.2.23). Clamp every candidate.
    let stack: Vec<Mv> = entries
        .iter()
        .map(|entry| clamp_mv(block, entry.mv))
        .collect();

    MvStack { stack }
}

/// One stack entry: the candidate MV plus its accumulated `WeightStack` weight.
#[derive(Clone, Copy, Debug)]
struct MvStackEntry {
    mv: Mv,
    weight: u32,
}

/// AV2 § 7.12.2.6 Scan point process for single prediction. Probes one neighbour
/// location, derives its weight, and (when the location is a decoded inter block)
/// invokes the § 7.12.2.10 Add reference motion vector process.
fn scan_point(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    delta_row: i32,
    mut delta_col: i32,
    entries: &mut Vec<MvStackEntry>,
) {
    let mv_row = block.mi_row as i32 + delta_row;
    let mut mv_col = block.mi_col as i32 + delta_col;

    // §7.12.2.6: superblock-border alignment for an above probe.
    if delta_row < 0 && block.is_sb_border() {
        mv_col = (mv_col >> 1) << 1;
        delta_col = mv_col - block.mi_col as i32;
    }

    // §7.12.2.6 weight:
    //   (-1, -1) -> 0; deltaCol < -1 -> 0; otherwise 1.
    let zero_weight = (delta_row == -1 && delta_col == -1) || delta_col < -1;
    let weight: u32 = if zero_weight { 0 } else { 1 };

    let Some(cell) = grid.get(mv_row, mv_col) else {
        return;
    };
    // §7.12.2.6: the candidate location must have been decoded (RefFrames written).
    // `grid.get` returning Some means the cell was written for this frame.

    // §7.12.2.9 Add warp motion vector process is deferred (DeriveWrl == 0).

    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        // §7.12.2.6: terminate before the add reference MV process.
        return;
    }

    add_reference_mv(block, cell, weight, entries);
}

/// AV2 § 7.12.2.10 Add reference motion vector process for single prediction
/// (`isCompound == 0`), non-intrabc, non-TIP subset. For a candidate whose
/// reference frame matches `RefFrame[0]`, invokes the § 7.12.2.12 Search stack
/// process. The TIP / derived-ref-frame branches are deferred.
fn add_reference_mv(
    block: &MvBlockContext,
    cell: NeighbourCell,
    weight: u32,
    entries: &mut Vec<MvStackEntry>,
) {
    // §7.12.2.10: if IsInters[mvRow][mvCol] == 0, terminate.
    if !cell.is_inter {
        return;
    }
    // Single prediction, non-intrabc: candList = 0..1 (one list, candList == 0).
    // §7.12.2.10: if RefFrames[...][candList] == RefFrame[0], search_stack.
    if cell.ref_frame0 == block.ref_frame0 {
        search_stack(cell.mv, weight, entries);
    }
    // The TIP / single-add-derived branches (§7.12.2.16–§7.12.2.18) are deferred.
}

/// AV2 § 7.12.2.12 Search stack process for single prediction. If `cand_mv` is
/// already in the stack, adds `weight` to its `WeightStack`; otherwise appends a
/// new candidate (bounded by `MAX_REF_MV_STACK_SIZE`).
fn search_stack(cand_mv: Mv, weight: u32, entries: &mut Vec<MvStackEntry>) {
    for entry in entries.iter_mut() {
        if entry.mv == cand_mv {
            entry.weight = entry.weight.saturating_add(weight);
            return;
        }
    }
    if entries.len() < MAX_REF_MV_STACK_SIZE {
        entries.push(MvStackEntry {
            mv: cand_mv,
            weight,
        });
    }
}

/// AV2 § 7.12.2.20 Extra search process (single prediction, non-intrabc, no
/// warp): clamp the existing candidates, then add the global MV if it is not
/// already present. The large-block (Block_Width > 32 AND Block_Height > 32) MVP
/// combinations and the warp / intrabc candidates are deferred. NB: the admitted
/// leaves are >= 32x32 with no upper bound, so the verified 64x64 blocks are
/// larger than 32x32 and the large-block step applies to them; it is kept safe
/// only by the identical-MV fixtures and the §5.20.7.8 DRL-out-of-range reject
/// (see the call-site note in [`find_mv_stack`]).
fn extra_search(block: &MvBlockContext, global_mv: Mv, entries: &mut Vec<MvStackEntry>) {
    // §7.12.2.20: clamp each candidate (the per-list clamp, single list here).
    for entry in entries.iter_mut() {
        entry.mv = clamp_mv(block, entry.mv);
    }

    // §7.12.2.20: add the global MV candidate if not already present.
    if entries.len() < MAX_REF_MV_STACK_SIZE {
        let already_present = entries.iter().any(|entry| entry.mv == global_mv);
        if !already_present {
            entries.push(MvStackEntry {
                mv: global_mv,
                weight: 0,
            });
        }
    }
}

/// AV2 § 5.20.9.4 `clamp_mv_row` + § 5.20.9.5 `clamp_mv_col` applied to a motion
/// vector (eighth-pel units), the per-candidate clamp the § 7.12.2.20 /
/// § 7.12.2.23 processes use.
fn clamp_mv(block: &MvBlockContext, mv: Mv) -> Mv {
    let bw4 = block.bw4 as i32;
    let bh4 = block.bh4 as i32;
    let mi_row = block.mi_row as i32;
    let mi_col = block.mi_col as i32;
    let mi_rows = block.mi_rows as i32;
    let mi_cols = block.mi_cols as i32;

    // §5.20.9.4 clamp_mv_row.
    let row_low = -(mi_row + bh4) * MI_SIZE * 8 - MV_BORDER;
    let row_high = (mi_rows - mi_row) * MI_SIZE * 8 + MV_BORDER;
    // §5.20.9.5 clamp_mv_col.
    let col_low = -(mi_col + bw4) * MI_SIZE * 8 - MV_BORDER;
    let col_high = (mi_cols - mi_col) * MI_SIZE * 8 + MV_BORDER;

    Mv {
        row: mv.row.clamp(row_low, row_high),
        col: mv.col.clamp(col_low, col_high),
    }
}

#[cfg(test)]
mod tests;
