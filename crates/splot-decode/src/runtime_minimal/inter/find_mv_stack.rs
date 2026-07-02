// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::Mv;

/// AV2 § 3 `MAX_REF_MV_STACK_SIZE`: the maximum number of motion vectors in the
/// stack.
pub(super) const MAX_REF_MV_STACK_SIZE: usize = 6;

const MV_BORDER: i32 = 128;

const MI_SIZE: i32 = 4;

/// AV2 § 6.18 `MotionModes[ r ][ c ]` values in spec order.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub(super) enum MotionMode {
    /// `SIMPLE` (0).
    Simple,
    /// `INTERINTRA` (1). Not yet produced: SIMPLE-path interintra defers.
    #[allow(dead_code)]
    InterIntra,
    /// `LOCALWARP` (2).
    LocalWarp,
    /// `DELTAWARP` (3).
    DeltaWarp,
    /// `EXTENDWARP` (4).
    ExtendWarp,
}

impl MotionMode {
    /// § 8.3.2 warp-context predicate: `MotionModes[ .. ] >= LOCALWARP`.
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
    skip: bool,
    interp_filter: u8,
    use_amvd: bool,
    motion_mode: MotionMode,
    warp_params: Option<[i64; 6]>,
    base_r: usize,
    base_c: usize,
    bw4: usize,
    bh4: usize,
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
    skip: false,
    interp_filter: SWITCHABLE_FILTERS,
    use_amvd: false,
    motion_mode: MotionMode::Simple,
    warp_params: None,
    base_r: 0,
    base_c: 0,
    bw4: 0,
    bh4: 0,
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
            MotionMode::Simple,
            None,
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
            skip,
            interp_filter: interp_filter.min(SWITCHABLE_FILTERS),
            use_amvd,
            motion_mode,
            warp_params,
            base_r: r,
            base_c: c,
            bw4: n4w,
            bh4: n4h,
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
            mv1: None,
            skip,
            interp_filter: interp_filter.min(SWITCHABLE_FILTERS),
            use_amvd,
            motion_mode: MotionMode::Simple,
            warp_params: None,
            base_r: r,
            base_c: c,
            bw4: n4w,
            bh4: n4h,
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
        let (delta_row, delta_col) = self.warp_context_delta(block);
        grid.get(
            block.mi_row as i32 + delta_row,
            block.mi_col as i32 + delta_col,
        )
    }

    /// § 7.11.4 probe delta after the superblock-border column adjustment.
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
    /// § 7.11.4 `WarpSampleFound[ 1 ]`: the same scan matched against the
    /// block's second reference (compound only).
    pub(super) warp_sample_found1: bool,
    /// § 7.11.4 `ExtendDeltaRow` / `ExtendDeltaCol`: the first warp-scan
    /// probe (post superblock-border adjustment) whose cell matched the
    /// block's first reference.
    pub(super) extend_delta: Option<(i32, i32)>,
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
            .filter(|cell| cell.is_warp())
            .count()
    }

    /// AV2 § 8.3.2 `use_local_warp` context: `hasWarp` plus the `NPos`
    /// neighbour count with `MotionModes == LOCALWARP`.
    pub(super) fn use_local_warp_ctx(&self) -> usize {
        let cells = self.npos_cells.iter().take(self.npos_count);
        usize::from(cells.clone().any(|cell| cell.is_warp()))
            + cells
                .filter(|cell| cell.motion_mode == MotionMode::LocalWarp)
                .count()
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
    stack: Vec<(Mv, (i32, i32))>,
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
            .or_else(|| self.stack.last())
            .map_or(Mv::ZERO, |entry| entry.0)
    }

    /// Returns § 7.12.2 `RefStackRowOffset[idx]` / `RefStackColOffset[idx]`
    /// (`(0, 0)` for candidates that did not come from an adjacent scan).
    pub(super) fn candidate_offsets(&self, idx: usize) -> (i32, i32) {
        self.stack
            .get(idx)
            .or_else(|| self.stack.last())
            .map_or((0, 0), |entry| entry.1)
    }
}

/// The § 7.13.3.24 neighbour-parameter lookup at the extend-warp base
/// position: a warp neighbour supplies its stored model, otherwise the
/// neighbour's translational MV lifts to a warp model. The global-motion arm
/// is statically unreachable (frames signalling `use_global_motion` defer at
/// the frame gate, so `GmType` is always `IDENTITY` here).
pub(super) enum ExtendWarpNeighbour {
    /// The § 7 `params` array for the extension math.
    Params([i64; 6]),
    /// The base cell needs the second reference list's MV, which the grid
    /// does not retain yet.
    List1MvUnretained,
    /// No decoded cell at the base position.
    Missing,
}

/// Resolves the § 7.13.3.24 `params` for the neighbour at
/// `(MiRow + deltaRow, MiCol + deltaCol)`.
pub(super) fn extend_warp_neighbour_params(
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

const WARPEDMODEL_PREC_BITS: u32 = 16;

/// AV2 § 3 `LEAST_SQUARES_SAMPLES_MAX`.
const LEAST_SQUARES_SAMPLES_MAX: usize = 8;

/// The § 7.12.3 warp-sample collection outcome.
pub(super) enum WarpSampleCollection {
    /// `CandList[ 0 ][ .. ]` rows: `[ srcY, srcX, dstY, dstX ]` in eighth-pel.
    Samples(Vec<[i64; 4]>),
    /// A candidate matched on the second reference list, whose MV the grid
    /// does not retain yet.
    List1MvUnretained,
}

/// AV2 § 7.12.3 find warp samples for `ref = 0`.
pub(super) fn find_warp_samples(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
) -> WarpSampleCollection {
    let mut samples: Vec<[i64; 4]> = Vec::with_capacity(LEAST_SQUARES_SAMPLES_MAX);
    let mi_row = block.mi_row as i32;
    let mi_col = block.mi_col as i32;
    let w4 = block.bw4 as i32;
    let h4 = block.bh4 as i32;
    let mi_rows = block.mi_rows as i32;
    let mi_cols = block.mi_cols as i32;
    let mut missing_list1 = false;
    let mut add_sample = |samples: &mut Vec<[i64; 4]>, delta_row: i32, delta_col: i32| {
        if samples.len() >= LEAST_SQUARES_SAMPLES_MAX {
            return;
        }
        let Some(cell) = grid.get(mi_row + delta_row, mi_col + delta_col) else {
            return;
        };
        let lists = [
            (cell.ref_frame0 == block.ref_frame0 && cell.is_inter).then_some(Some(cell.mv)),
            (cell.ref_frame1 == Some(block.ref_frame0)).then_some(cell.mv1),
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
            samples.push([
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
    WarpSampleCollection::Samples(samples)
}

/// AV2 § 3 `REF_MV_BANK_SIZE` (entries per bank list).
const REF_MV_BANK_SIZE: usize = 4;
/// AV2 § 3 `BANK_REFS_PER_FRAME`.
const BANK_REFS_PER_FRAME: i32 = 9;
/// AV2 § 3 `MAX_RMB_SB_HITS`.
const MAX_RMB_SB_HITS: u32 = 64;
/// AV2 § 3 `MAX_PR_NUM`.
const MAX_PR_NUM: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RefMvBankEntry {
    key: i32,
    mv0: Mv,
    mv1: Mv,
}

/// AV2 § 7.12.2.21 / § 5.20.2.2 reference motion-vector bank: nine ring
/// buffers of recent block MVs, reset (and re-seeded from the row above) at
/// each superblock, filled into the MV stack after the spatial scan.
/// Compound weights (`cwp`) are not retained: every admitted producer uses
/// `CWP_EQUAL`.
pub(super) struct RefMvBank {
    entries: [[RefMvBankEntry; REF_MV_BANK_SIZE]; BANK_REFS_PER_FRAME as usize],
    sizes: [usize; BANK_REFS_PER_FRAME as usize],
    starts: [usize; BANK_REFS_PER_FRAME as usize],
    sb_hits: u32,
    remain_hits: i32,
    unit_hits: i32,
    current_sb: Option<(usize, usize)>,
}

impl RefMvBank {
    pub(super) fn new() -> Self {
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

    /// § 7.12.2.21 `get_rmb_list_index` for the single-prediction subset.
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

    /// § 5.20.2.2 `reset_refmv_bank`, invoked at the first leaf of each
    /// superblock (detected from the leaf coordinates); re-seeds from the
    /// decoded row above unless this is the top superblock row.
    pub(super) fn reset_for_leaf(
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
        self.current_sb = Some(sb);
        self.sb_hits = 0;
        self.remain_hits = 0;
        self.unit_hits = 0;
        for size in &mut self.sizes {
            *size = 0;
        }
        for start in &mut self.starts {
            *start = 0;
        }
        let sb_row = sb.0 * sb_size4;
        let sb_col = sb.1 * sb_size4;
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
                    self.update(cell.ref_frame0, cell.ref_frame1, cell.mv, cell.mv1, false);
                }
                step = (cell.bw4 as i32).max(1);
            }
            cand_col += step;
        }
    }

    /// § 5.20.7 `update_ref_mv_count` unit-budget bookkeeping.
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

    /// § 5.20.7 per-block bank update (`fromWithinSb == 1`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_for_block(
        &mut self,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        mv: Mv,
        mv1: Option<Mv>,
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
        self.update(ref_frame0, ref_frame1, mv, mv1, true);
    }

    /// § 5.20.7 `update_ref_mv_bank` tail: move-to-tail on match, else append.
    fn update(
        &mut self,
        ref_frame0: i8,
        ref_frame1: Option<i8>,
        mv: Mv,
        mv1: Option<Mv>,
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
        };
        let count = self.sizes[list];
        let start = self.starts[list];
        let mut found = None;
        for i in 0..count {
            let idx = (start + i) % REF_MV_BANK_SIZE;
            if self.entries[list][idx] == entry {
                found = Some(i);
                break;
            }
        }
        if let Some(found) = found {
            for i in found..count.saturating_sub(1) {
                let idx0 = (start + i) % REF_MV_BANK_SIZE;
                let idx1 = (start + i + 1) % REF_MV_BANK_SIZE;
                self.entries[list][idx0] = self.entries[list][idx1];
            }
            let tail = (start + count - 1) % REF_MV_BANK_SIZE;
            self.entries[list][tail] = entry;
        } else if count < REF_MV_BANK_SIZE {
            let tail = (start + count) % REF_MV_BANK_SIZE;
            self.entries[list][tail] = entry;
            self.sizes[list] = count + 1;
        } else {
            self.entries[list][start] = entry;
            self.starts[list] = (start + 1) % REF_MV_BANK_SIZE;
        }
    }

    /// § 7.12.2.21 fill: newest-first bank candidates appended to the stack
    /// with the § check_rmb_cand prune and in-frame bounds checks.
    fn fill(
        &self,
        block: &MvBlockContext,
        entries: &mut Vec<MvStackEntry>,
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
            entries.push(MvStackEntry {
                mv: candidate.mv0,
                weight: 0,
                offsets: (0, 0),
            });
        }
    }
}

/// AV2 § 7.12.2 `find_mv_stack` for the spatial-only single-prediction subset.
pub(super) fn find_mv_stack(
    grid: &NeighbourMvGrid,
    block: &MvBlockContext,
    global_mv: Mv,
    bank: Option<(&RefMvBank, usize)>,
) -> MvStack {
    let mut entries: Vec<MvStackEntry> = Vec::with_capacity(MAX_REF_MV_STACK_SIZE);

    for probe in mv_stack_spatial_probes(block).into_iter().flatten() {
        scan_mv_stack_probe(grid, block, probe, &mut entries);
    }

    // TODO(spec: DECODE-INTER-MVSTACK-SPATIAL): 7.12.2.5 Scan col, 7.12.2.19 Sorting, 7.12.2.20 large-block MVP
    if let Some((bank, max_ref_mv_count)) = bank {
        let mut prune_count = 0usize;
        bank.fill(block, &mut entries, max_ref_mv_count, &mut prune_count);
    }
    extra_search(block, global_mv, &mut entries);

    let stack: Vec<(Mv, (i32, i32))> = entries
        .into_iter()
        .map(|entry| (clamp_mv(block, entry.mv), entry.offsets))
        .collect();

    MvStack { stack }
}

#[derive(Clone, Copy, Debug)]
struct MvStackEntry {
    mv: Mv,
    weight: u32,
    offsets: (i32, i32),
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

    let (_, _, adjusted_delta_col) = probe.stack_target(block);
    entries.push(MvStackEntry {
        mv: cell.mv,
        weight,
        offsets: (probe.delta_row, adjusted_delta_col),
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
                offsets: (0, 0),
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
