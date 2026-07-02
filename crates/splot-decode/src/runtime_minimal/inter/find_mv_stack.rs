// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::Mv;

/// AV2 § 3 `MAX_REF_MV_STACK_SIZE`: the maximum number of motion vectors in the
/// stack.
pub(super) const MAX_REF_MV_STACK_SIZE: usize = 6;

const MV_BORDER: i32 = 128;

const MI_SIZE: i32 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NeighbourCell {
    is_inter: bool,
    ref_frame0: i8,
    ref_frame1: Option<i8>,
    y_mode: NeighbourYMode,
    newmv_for_list0: bool,
    newmv_for_list1: bool,
    mv: Mv,
    skip: bool,
    interp_filter: u8,
    use_amvd: bool,
    is_warp: bool,
    precision: BlockPrecisionRecord,
}

const EMPTY_NEIGHBOUR_CELL: NeighbourCell = NeighbourCell {
    is_inter: false,
    ref_frame0: -1,
    ref_frame1: None,
    y_mode: NeighbourYMode::Other,
    newmv_for_list0: false,
    newmv_for_list1: false,
    mv: Mv::ZERO,
    skip: false,
    interp_filter: SWITCHABLE_FILTERS,
    use_amvd: false,
    is_warp: false,
    precision: BlockPrecisionRecord {
        use_most_probable_precision: false,
        mv_precision: 0,
    },
};

/// § 5.20.7.13 `UseMostProbablePrecisions` / `MvPrecisions` grid values for one block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BlockPrecisionRecord {
    /// `UseMostProbablePrecisions[ r ][ c ]`.
    pub(super) use_most_probable_precision: bool,
    /// `MvPrecisions[ r ][ c ]` (Table 6.19 code).
    pub(super) mv_precision: u8,
}

impl BlockPrecisionRecord {
    /// The § 5.20.7.13 inter path that keeps `MvPrecision = FrameMvPrecision`
    /// (`use_most_probable_precision = 1`).
    pub(super) const fn most_probable(mv_precision: u8) -> Self {
        Self {
            use_most_probable_precision: true,
            mv_precision,
        }
    }

    /// The § 5.20.5.3 / § 5.20.7.12 intra and IntrABC grid values and the
    /// explicit `pb_mv_precision` path (`use_most_probable_precision = 0`).
    pub(super) const fn explicit(mv_precision: u8) -> Self {
        Self {
            use_most_probable_precision: false,
            mv_precision,
        }
    }
}

impl Default for BlockPrecisionRecord {
    /// The § 5.20.7.13 non-flex inter path: `use_most_probable_precision = 1`
    /// at the `MV_PRECISION_EIGHTH_PEL` frame default.
    fn default() -> Self {
        Self::most_probable(super::read_mv::MV_PRECISION_EIGHTH_PEL)
    }
}

const SWITCHABLE_FILTERS: u8 = 3;
const INTER_FILTER_COMP_OFFSET: usize = SWITCHABLE_FILTERS as usize + 1;

/// Luma mode class needed by § 7.11.3 `has_newmv_for_list`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NeighbourYMode {
    /// The neighbour coded a new list-0 MV.
    NewMv,
    /// Any neighbour mode that does not increment `NewMvCount`.
    Other,
}

/// Per-MI mode-info grid read by the § 7.11 / § 7.12 spatial scans.
pub(super) struct NeighbourMvGrid {
    mi_rows: usize,
    mi_cols: usize,
    cells: Vec<Option<NeighbourCell>>,
}

impl NeighbourMvGrid {
    /// Builds an empty MI grid, returning `None` if the dimensions overflow.
    pub(super) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let cells = mi_rows.checked_mul(mi_cols)?;
        Some(Self {
            mi_rows,
            mi_cols,
            cells: vec![None; cells],
        })
    }

    /// Records a decoded block's mode info into every covered MI cell.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_block(
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
            false,
            precision,
        );
    }

    /// Records a decoded warp block's mode info into every covered MI cell.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_warp_block(
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
            true,
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
        is_warp: bool,
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
            skip,
            interp_filter: interp_filter.min(SWITCHABLE_FILTERS),
            use_amvd,
            is_warp,
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
                self.cells[rr * self.mi_cols + cc] = Some(cell);
            }
        }
    }

    /// Records decoded compound mode info with per-reference-list NEWMV state.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_block_with_newmv_lists(
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
            mv,
            skip,
            interp_filter: interp_filter.min(SWITCHABLE_FILTERS),
            use_amvd,
            is_warp: false,
            precision: BlockPrecisionRecord::default(),
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
    /// `RefFrame[1]`, or `None` for single-reference mode context.
    pub(super) ref_frame1: Option<i8>,
    /// `MiRows`: the frame MI height (for § 5.20.9.4 clamp bounds).
    pub(super) mi_rows: usize,
    /// `MiCols`: the frame MI width (for § 5.20.9.5 clamp bounds).
    pub(super) mi_cols: usize,
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
        let row = block.mi_row as i32 + self.delta_row;
        let mut delta_col = self.delta_col;
        if self.delta_row < 0 && block.is_sb_border() {
            delta_col -= (block.mi_col & 1) as i32;
        }
        let col = block.mi_col as i32 + delta_col;
        grid.get(row, col)
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

fn mode_ctx_match_newmv(cell: NeighbourCell, block: &MvBlockContext) -> Option<bool> {
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
    if cell.ref_frame0 == block.ref_frame0 && cell.ref_frame1 == Some(block_ref1) {
        return Some(cell.newmv_for_list0 || cell.newmv_for_list1);
    }
    None
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
    /// AV2 § 7.11.2 `WarpMvCount`: matching warp-mode neighbours.
    pub(super) warp_mv_count: usize,
    /// § 7.11.4 `WarpSampleFound[ 0 ]`: a warp-scan probe hit an inter cell
    /// whose reference matches the block's first reference.
    pub(super) warp_sample_found: bool,
}

/// AV2 § 7.11.2 `find_mode_ctx` for single prediction.
pub(super) fn find_mode_ctx(grid: &NeighbourMvGrid, block: &MvBlockContext) -> ModeContext {
    let mut new_mv_count = 0usize;
    let mut warp_mv_count = 0usize;
    let mut found = [false; 4];

    for (slot, probe) in found.iter_mut().zip(immediate_spatial_probes(block)) {
        let Some(cell) = probe.cell(grid, block) else {
            continue;
        };
        let Some(is_newmv) = mode_ctx_match_newmv(cell, block) else {
            continue;
        };
        if is_newmv {
            new_mv_count = (new_mv_count + 1).min(3);
        }
        *slot = true;
    }
    let mut warp_sample_found = false;
    for probe in warp_context_spatial_probes(block).into_iter().flatten() {
        let Some(cell) = probe.warp_context_cell(grid, block) else {
            continue;
        };
        if matches_block_ref(cell, block) {
            warp_sample_found = true;
            if cell.is_warp {
                warp_mv_count = (warp_mv_count + 1).min(4);
            }
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
    }
}

/// § 5.20.7.2 neighbour-buffer-derived § 8.3.2 contexts.
///
/// Carries both spec neighbour lists: `NPosBuf` (any in-frame neighbour) feeds
/// `is_inter`/`skip_flag`/`use_amvd`/`comp_mode`/`single_ref`; `NPos` (which
/// also drops the row above the superblock) feeds `interp_filter` and the
/// per-block MV-precision contexts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BlockNeighbourContext {
    /// AV2 § 8.3.2 `is_inter` context: from `NNumBuf` + `NIntra[]`.
    pub(super) is_inter_ctx: usize,
    /// AV2 § 8.3.2 `skip_flag` context: `ctx += Skips[NPosBuf[n]]` (no `skip_mode`).
    pub(super) skip_ctx: usize,
    /// True when `NNumBuf >= 1`.
    pub(super) has_neighbour: bool,
    ref_counts: [u8; BlockNeighbourContext::MAX_NEIGHBOUR_REFS],
    cells: [NeighbourCell; 2],
    cell_count: usize,
    npos_cells: [NeighbourCell; 2],
    npos_count: usize,
}

impl BlockNeighbourContext {
    const MAX_NEIGHBOUR_REFS: usize = 2;

    /// AV2 § 8.3.2 `single_ref` context for `ref_idx`.
    pub(super) fn single_ref_ctx(&self, ref_idx: usize, num_total_refs: usize) -> Option<usize> {
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

    /// AV2 `comp_inter` / reference-mode context used when `reference_select` is active.
    pub(super) fn comp_mode_ctx(
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

    /// AV2 § 8.3.2 `interp_filter` context for switchable interpolation.
    pub(super) fn interp_filter_ctx(&self, ref_frame0: i8, ref_frame1_is_inter: bool) -> usize {
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

    /// AV2 § 8.3.2 `use_amvd` context for the current single-reference block.
    pub(super) fn amvd_ctx(&self, ref_frame0: i8) -> usize {
        self.cells
            .iter()
            .take(self.cell_count)
            .filter(|cell| cell.is_inter && cell.ref_frame0 == ref_frame0 && cell.use_amvd)
            .count()
    }

    /// AV2 § 8.3.2 `use_extend_warp` context: `NPos` neighbour count with
    /// `MotionModes >= LOCALWARP` (every recorded warp block resolves to a
    /// warp motion mode, so `is_warp` models the comparison exactly).
    pub(super) fn use_extend_warp_ctx(&self) -> usize {
        self.npos_cells
            .iter()
            .take(self.npos_count)
            .filter(|cell| cell.is_warp)
            .count()
    }

    /// AV2 § 8.3.2 `use_local_warp` context: `hasWarp` plus the `NPos`
    /// neighbour count with `MotionModes == LOCALWARP`. LOCALWARP blocks are
    /// beyond the frontier, so no recorded neighbour can hold that mode yet;
    /// widen the cell record with the motion mode when LOCALWARP decode lands.
    pub(super) fn use_local_warp_ctx(&self) -> usize {
        usize::from(
            self.npos_cells
                .iter()
                .take(self.npos_count)
                .any(|cell| cell.is_warp),
        )
    }

    /// AV2 § 8.3.2 `use_most_probable_precision` context: neighbour count with
    /// `UseMostProbablePrecisions[ NPos ]` set.
    pub(super) fn most_probable_precision_ctx(&self) -> usize {
        self.npos_cells
            .iter()
            .take(self.npos_count)
            .filter(|cell| cell.precision.use_most_probable_precision)
            .count()
    }

    /// AV2 § 8.3.2 `pb_mv_precision` context: `1` when any neighbour's
    /// `MvPrecisions[ NPos ]` is below `FrameMvPrecision`.
    pub(super) fn pb_mv_precision_ctx(&self, frame_precision: u8) -> usize {
        usize::from(
            self.npos_cells
                .iter()
                .take(self.npos_count)
                .any(|cell| cell.precision.mv_precision < frame_precision),
        )
    }
}

/// Derives the § 5.20.7.2 neighbour buffer contexts for a block.
pub(super) fn block_neighbour_ctx(
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
    for cell in buf.iter().take(num_buf) {
        skip_ctx += usize::from(cell.skip);
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

    trace_neighbour_context(block, &buf, num_buf, is_inter_ctx, skip_ctx);

    BlockNeighbourContext {
        is_inter_ctx,
        skip_ctx,
        has_neighbour: num_buf >= 1,
        ref_counts,
        cells: buf,
        cell_count: num_buf,
        npos_cells: lists.npos,
        npos_count: lists.npos_len,
    }
}

fn trace_neighbour_context(
    block: &MvBlockContext,
    cells: &[NeighbourCell; 2],
    cell_count: usize,
    is_inter_ctx: usize,
    skip_ctx: usize,
) {
    let Some(target) = std::env::var("SPLOT_TRACE_NEIGHBOUR_CTX").ok() else {
        return;
    };
    let Some((row, col)) = target.split_once(':') else {
        return;
    };
    let Ok(row) = row.parse::<usize>() else {
        return;
    };
    let Ok(col) = col.parse::<usize>() else {
        return;
    };
    if block.mi_row != row || block.mi_col != col {
        return;
    }
    eprintln!(
        "neighbour ctx r={} c={} bw4={} bh4={} count={} is_inter_ctx={} skip_ctx={} cells={:?}",
        block.mi_row,
        block.mi_col,
        block.bw4,
        block.bh4,
        cell_count,
        is_inter_ctx,
        skip_ctx,
        &cells[..cell_count],
    );
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

/// The two § 5.20.7.2 neighbour lists: `buf` = `NPosBuf` (any in-frame
/// neighbour) and `npos` = `NPos` (additionally drops the row above the
/// superblock boundary).
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

/// § 7.12.2 `RefStackMv` candidates for single prediction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MvStack {
    stack: Vec<Mv>,
}

impl MvStack {
    /// `NumMvFound`: the number of candidate MVs.
    #[cfg(test)]
    pub(super) fn num_mv_found(&self) -> usize {
        self.stack.len()
    }

    /// Returns `RefStackMv[idx][0]`, saturating to the final fallback candidate.
    pub(super) fn candidate(&self, idx: usize) -> Mv {
        self.stack
            .get(idx)
            .copied()
            .or_else(|| self.stack.last().copied())
            .unwrap_or(Mv::ZERO)
    }
}

/// AV2 § 7.12.2 `find_mv_stack` for the spatial-only single-prediction subset.
pub(super) fn find_mv_stack(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mv: Mv,
) -> MvStack {
    let mut entries: Vec<MvStackEntry> = Vec::with_capacity(MAX_REF_MV_STACK_SIZE);

    for probe in mv_stack_spatial_probes(block).into_iter().flatten() {
        scan_mv_stack_probe(grid, block, probe, &mut entries);
    }

    // TODO(spec: DECODE-INTER-MVSTACK-SPATIAL): model §7.12.2.5 Scan col,
    // §7.12.2.19 Sorting, and §7.12.2.20 large-block (>32x32) MVP processes
    extra_search(block, global_mv, &mut entries);

    let stack: Vec<Mv> = entries
        .into_iter()
        .map(|entry| clamp_mv(block, entry.mv))
        .collect();

    MvStack { stack }
}

#[derive(Clone, Copy, Debug)]
struct MvStackEntry {
    mv: Mv,
    weight: u32,
}

fn scan_mv_stack_probe(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    probe: RelativeProbe,
    entries: &mut Vec<MvStackEntry>,
) {
    let Some((cell, weight)) = probe.stack_cell(grid, block) else {
        return;
    };

    if entries.len() >= MAX_REF_MV_STACK_SIZE {
        return;
    }

    if !matches_block_ref(cell, block) {
        return;
    }

    for entry in entries.iter_mut() {
        if entry.mv == cell.mv {
            entry.weight = entry.weight.saturating_add(weight);
            return;
        }
    }

    entries.push(MvStackEntry {
        mv: cell.mv,
        weight,
    });
}

fn extra_search(block: &MvBlockContext, global_mv: Mv, entries: &mut Vec<MvStackEntry>) {
    for entry in entries.iter_mut() {
        entry.mv = clamp_mv(block, entry.mv);
    }

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
