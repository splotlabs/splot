// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::sync::Arc;

use splot_recon::math::{round2_signed, round2_signed_i32};

use super::{
    Mv, MvBlockContext, NeighbourCell, NeighbourMvGrid, RelativeProbe, TIP_REF_FRAME,
    warp_sub_mv_at,
};
use selection::projection_queue;
#[cfg(test)]
use trajectory::TrajectoryMotionField;
use trajectory::TrajectoryState;

mod selection;
mod trajectory;

const MAX_FRAME_DISTANCE: i32 = 31;
const REFMVS_LIMIT: i32 = (1 << 11) - 1;
const MV_LIMIT: i32 = (1 << 16) - 1;
const INVALID_ORDER_HINT: u32 = u32::MAX;
const TIP_DIRECTIONS: [(i32, i32); 4] = [(-1, 0), (0, -1), (1, 0), (0, 1)];
const DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CompressedMv {
    row: i16,
    col: i16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TemporalMotionCell {
    ref_order_hints: [u32; 2],
    mvs: [CompressedMv; 2],
}

impl Default for TemporalMotionCell {
    fn default() -> Self {
        Self {
            ref_order_hints: [INVALID_ORDER_HINT; 2],
            mvs: [CompressedMv::default(); 2],
        }
    }
}

impl TemporalMotionCell {
    const fn is_valid(self) -> bool {
        self.ref_order_hints[0] != INVALID_ORDER_HINT
            || self.ref_order_hints[1] != INVALID_ORDER_HINT
    }
}

fn allocate_temporal_grid<T>(mi_rows: usize, mi_cols: usize) -> Option<(usize, usize, Vec<T>)>
where
    T: Clone + Default,
{
    let width8 = mi_cols.div_ceil(2);
    let height8 = mi_rows.div_ceil(2);
    let cells = width8.checked_mul(height8)?;
    Some((width8, height8, vec![T::default(); cells]))
}

fn temporal_grid_index(width8: usize, height8: usize, y8: usize, x8: usize) -> Option<usize> {
    if y8 >= height8 || x8 >= width8 {
        return None;
    }
    Some(y8 * width8 + x8)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionField {
    width8: usize,
    height8: usize,
    cells: Vec<TemporalMotionCell>,
    is_inter: bool,
    frame_size: Option<(usize, usize)>,
    ref_order_hints: Vec<Option<u32>>,
}

impl TemporalMotionField {
    pub(crate) fn empty() -> Self {
        Self {
            width8: 0,
            height8: 0,
            cells: Vec::new(),
            is_inter: false,
            frame_size: None,
            ref_order_hints: Vec::new(),
        }
    }

    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
            is_inter: false,
            frame_size: None,
            ref_order_hints: Vec::new(),
        })
    }

    pub(crate) fn set_reference_metadata(
        &mut self,
        is_inter: bool,
        frame_size: (usize, usize),
        ref_order_hints: &[Option<u32>],
    ) {
        self.is_inter = is_inter;
        self.frame_size = Some(frame_size);
        self.ref_order_hints = ref_order_hints.to_vec();
    }

    pub(crate) fn record_block(&mut self, block: TemporalMotionBlock) {
        let Some(row_end) = block.mi_row.checked_add(block.n4h) else {
            return;
        };
        let Some(col_end) = block.mi_col.checked_add(block.n4w) else {
            return;
        };
        let row_end = row_end.min(block.mi_rows);
        let col_end = col_end.min(block.mi_cols);
        if row_end <= block.mi_row || col_end <= block.mi_col {
            return;
        }
        let row8_start = block.mi_row >> 1;
        let col8_start = block.mi_col >> 1;
        let row8_end = row_end.div_ceil(2).min(self.height8);
        let col8_end = col_end.div_ceil(2).min(self.width8);

        for y8 in row8_start..row8_end {
            for x8 in col8_start..col8_end {
                let mut cell = TemporalMotionCell::default();
                for list in 0..2 {
                    let Some(order_hint) = block.ref_order_hints[list] else {
                        continue;
                    };
                    let mv = if let Some(params) = block.warp_params[list] {
                        warp_sub_mv_at(params, block.mi_row, block.mi_col, y8 * 2, x8 * 2)
                    } else {
                        block.mvs[list]
                    };
                    if mv.row.abs() > REFMVS_LIMIT || mv.col.abs() > REFMVS_LIMIT {
                        continue;
                    }
                    cell.ref_order_hints[list] = order_hint;
                    cell.mvs[list] = compress_tmvp_mv(mv);
                }
                if cell.ref_order_hints[0] != INVALID_ORDER_HINT
                    && cell.ref_order_hints[1] == INVALID_ORDER_HINT
                {
                    cell.ref_order_hints[1] = cell.ref_order_hints[0];
                    cell.mvs[1] = cell.mvs[0];
                } else if cell.ref_order_hints[1] != INVALID_ORDER_HINT
                    && cell.ref_order_hints[0] == INVALID_ORDER_HINT
                {
                    cell.ref_order_hints[0] = cell.ref_order_hints[1];
                    cell.mvs[0] = cell.mvs[1];
                } else if let [Some(ref0), Some(ref1)] = block.ref_order_hints {
                    let ref0 = i32::try_from(ref0).unwrap_or(i32::MAX);
                    let ref1 = i32::try_from(ref1).unwrap_or(i32::MAX);
                    let current = i32::try_from(block.current_order_hint).unwrap_or(i32::MAX);
                    let ref0_to_current = super::super::get_relative_dist(ref0, current);
                    let ref1_to_current = super::super::get_relative_dist(ref1, current);
                    let same_side = (ref0_to_current < 0 && ref1_to_current < 0)
                        || (ref0_to_current > 0 && ref1_to_current > 0);
                    let should_swap = if same_side {
                        super::super::get_relative_dist(ref0, ref1) < 0
                    } else {
                        ref0_to_current > 0 && ref1_to_current < 0
                    };
                    if should_swap {
                        cell.ref_order_hints.swap(0, 1);
                        cell.mvs.swap(0, 1);
                    }
                }
                if let Some(slot) = self.cell_mut(y8, x8) {
                    *slot = cell;
                }
            }
        }
    }

    #[cfg(test)]
    fn cell(&self, y8: usize, x8: usize) -> Option<TemporalMotionCell> {
        self.cells
            .get(temporal_grid_index(self.width8, self.height8, y8, x8)?)
            .copied()
    }

    fn cell_mut(&mut self, y8: usize, x8: usize) -> Option<&mut TemporalMotionCell> {
        self.cells
            .get_mut(temporal_grid_index(self.width8, self.height8, y8, x8)?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionBlock {
    pub(crate) mi_row: usize,
    pub(crate) mi_col: usize,
    pub(crate) n4w: usize,
    pub(crate) n4h: usize,
    pub(crate) mi_rows: usize,
    pub(crate) mi_cols: usize,
    pub(crate) current_order_hint: u32,
    pub(crate) ref_order_hints: [Option<u32>; 2],
    pub(crate) mvs: [Mv; 2],
    pub(crate) warp_params: [Option<[i32; 6]>; 2],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProjectedTemporalMotionCell(u64);

impl ProjectedTemporalMotionCell {
    const MV_BITS: u32 = 17;
    const MV_MASK: u64 = (1 << Self::MV_BITS) - 1;
    const REF_OFFSET_BITS: u32 = 6;
    const REF_OFFSET_SHIFT: u32 = Self::MV_BITS * 2;
    const REF_OFFSET_MASK: u64 = (1 << Self::REF_OFFSET_BITS) - 1;
    const VALID: u64 = 1 << (Self::REF_OFFSET_SHIFT + Self::REF_OFFSET_BITS);

    fn new(mv: Mv, ref_offset: i32, valid: bool) -> Self {
        debug_assert!((-MAX_FRAME_DISTANCE..=MAX_FRAME_DISTANCE).contains(&ref_offset));
        let mv = if valid { mv } else { Mv::ZERO };
        debug_assert!((-MV_LIMIT..=MV_LIMIT).contains(&mv.row));
        debug_assert!((-MV_LIMIT..=MV_LIMIT).contains(&mv.col));
        Self(
            (mv.row as u64 & Self::MV_MASK)
                | ((mv.col as u64 & Self::MV_MASK) << Self::MV_BITS)
                | ((ref_offset as u64 & Self::REF_OFFSET_MASK) << Self::REF_OFFSET_SHIFT)
                | if valid { Self::VALID } else { 0 },
        )
    }

    const fn is_valid(self) -> bool {
        self.0 & Self::VALID != 0
    }

    const fn mv(self) -> Mv {
        Mv {
            row: Self::unpack_signed(self.0, Self::MV_BITS),
            col: Self::unpack_signed(self.0 >> Self::MV_BITS, Self::MV_BITS),
        }
    }

    const fn ref_offset(self) -> i32 {
        Self::unpack_signed(self.0 >> Self::REF_OFFSET_SHIFT, Self::REF_OFFSET_BITS)
    }

    const fn unpack_signed(value: u64, bits: u32) -> i32 {
        ((value << (64 - bits)) as i64 >> (64 - bits)) as i32
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectedTemporalMotionField {
    width8: usize,
    height8: usize,
    cells: Vec<ProjectedTemporalMotionCell>,
}

impl ProjectedTemporalMotionField {
    #[cfg(test)]
    fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
        })
    }

    fn reset(&mut self, mi_rows: usize, mi_cols: usize) -> Option<()> {
        self.width8 = mi_cols.div_ceil(2);
        self.height8 = mi_rows.div_ceil(2);
        let cells = self.width8.checked_mul(self.height8)?;
        self.cells
            .resize(cells, ProjectedTemporalMotionCell::default());
        self.cells.fill(ProjectedTemporalMotionCell::default());
        Some(())
    }

    fn cell(&self, y8: usize, x8: usize) -> Option<ProjectedTemporalMotionCell> {
        self.cells
            .get(temporal_grid_index(self.width8, self.height8, y8, x8)?)
            .copied()
    }

    fn set(&mut self, y8: usize, x8: usize, mv: Mv, ref_offset: i32, valid: bool) {
        let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8) else {
            return;
        };
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = ProjectedTemporalMotionCell::new(mv, ref_offset, valid);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TipReferencePair {
    pub(crate) past_ref: i8,
    pub(crate) future_ref: i8,
    pub(crate) past_offset: i32,
    pub(crate) future_offset: i32,
    pub(crate) ref_offset: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMvContext {
    current_order_hint: u32,
    ref_order_hints: Vec<Option<u32>>,
    field: ProjectedTemporalMotionField,
    projection_scratch: ProjectedTemporalMotionField,
    average_scratch: ProjectedTemporalMotionField,
    target_cache: Vec<(u32, Option<usize>, i32)>,
    trajectories: Option<TrajectoryState>,
    trajectory_scratch: Option<TrajectoryState>,
    tip: Option<TipReferencePair>,
}

#[derive(Clone, Copy)]
pub(crate) struct OrderHintMvContext<'a> {
    current_order_hint: u32,
    ref_order_hints: &'a [Option<u32>],
}

impl OrderHintMvContext<'_> {
    pub(super) fn derive_spatial_mv(
        self,
        dst_ref: i8,
        candidate_ref: i8,
        candidate_mv: Mv,
    ) -> Option<Mv> {
        let dst = usize::try_from(dst_ref).ok()?;
        let candidate = usize::try_from(candidate_ref).ok()?;
        let current = i32::try_from(self.current_order_hint).ok()?;
        let distance = |index: usize| {
            self.ref_order_hints
                .get(index)
                .copied()
                .flatten()
                .and_then(|hint| i32::try_from(hint).ok())
                .map(|hint| super::super::get_relative_dist(current, hint))
        };
        let dst_distance = distance(dst)?;
        let candidate_distance = distance(candidate)?;
        let same_side = (dst_distance > 0 && candidate_distance > 0)
            || (dst_distance < 0 && candidate_distance < 0);
        same_side
            .then(|| project_mv(candidate_mv, dst_distance.abs(), candidate_distance.abs()))
            .flatten()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemporalProjectionConfig {
    pub(crate) frame_size: (usize, usize),
    pub(crate) step: usize,
    pub(crate) unit_size8: usize,
    pub(crate) enable_tip: bool,
    pub(crate) enable_trajectory: bool,
    pub(crate) reduced: bool,
}

impl TemporalMvContext {
    pub(crate) fn empty() -> Self {
        Self {
            current_order_hint: 0,
            ref_order_hints: Vec::new(),
            field: ProjectedTemporalMotionField {
                width8: 0,
                height8: 0,
                cells: Vec::new(),
            },
            projection_scratch: ProjectedTemporalMotionField {
                width8: 0,
                height8: 0,
                cells: Vec::new(),
            },
            average_scratch: ProjectedTemporalMotionField {
                width8: 0,
                height8: 0,
                cells: Vec::new(),
            },
            target_cache: Vec::new(),
            trajectories: None,
            trajectory_scratch: None,
            tip: None,
        }
    }

    #[cfg(test)]
    pub(super) fn with_tip_sample(
        mi_rows: usize,
        mi_cols: usize,
        references: TipReferencePair,
        y8: usize,
        x8: usize,
        mv: Mv,
    ) -> Option<Self> {
        let mut field = ProjectedTemporalMotionField::new(mi_rows, mi_cols)?;
        field.set(y8, x8, mv, references.ref_offset, true);
        Some(Self {
            current_order_hint: 0,
            ref_order_hints: Vec::new(),
            field,
            projection_scratch: ProjectedTemporalMotionField::new(0, 0)?,
            average_scratch: ProjectedTemporalMotionField::new(0, 0)?,
            target_cache: Vec::new(),
            trajectories: None,
            trajectory_scratch: None,
            tip: Some(references),
        })
    }

    #[cfg(test)]
    pub(super) fn set_trajectory_sample(
        &mut self,
        reference: usize,
        y8: usize,
        x8: usize,
        mv: Mv,
    ) -> Option<()> {
        if self.trajectories.is_none() {
            let references = self.tip_references()?;
            let count = references.future_ref.max(references.past_ref);
            let count = usize::try_from(count).ok()?.checked_add(1)?;
            let mut fields = Vec::with_capacity(count);
            for _ in 0..count {
                fields.push(TrajectoryMotionField::new(
                    self.field.height8 * 2,
                    self.field.width8 * 2,
                )?);
            }
            self.trajectories = Some(TrajectoryState::from_fields(fields));
        }
        self.trajectories
            .as_mut()?
            .fields
            .get_mut(reference)?
            .set(y8, x8, mv);
        Some(())
    }

    #[cfg(test)]
    pub(super) fn set_order_hint_context(
        &mut self,
        current_order_hint: u32,
        ref_order_hints: Vec<Option<u32>>,
    ) {
        self.current_order_hint = current_order_hint;
        self.ref_order_hints = ref_order_hints;
    }

    #[cfg(test)]
    pub(crate) fn from_references(
        mi_dimensions: (usize, usize),
        current_order_hint: u32,
        config: TemporalProjectionConfig,
        ref_frame_idx: &[u32],
        ref_valid: &[bool],
        ref_order_hint: &[u32],
        ref_motion_fields: &[Option<Arc<TemporalMotionField>>],
    ) -> Option<Self> {
        let mut context = Self::empty();
        context.refresh_from_references(
            mi_dimensions,
            current_order_hint,
            config,
            ref_frame_idx,
            ref_valid,
            ref_order_hint,
            ref_motion_fields,
        )?;
        Some(context)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refresh_from_references(
        &mut self,
        mi_dimensions: (usize, usize),
        current_order_hint: u32,
        config: TemporalProjectionConfig,
        ref_frame_idx: &[u32],
        ref_valid: &[bool],
        ref_order_hint: &[u32],
        ref_motion_fields: &[Option<Arc<TemporalMotionField>>],
    ) -> Option<()> {
        let (mi_rows, mi_cols) = mi_dimensions;
        self.field.reset(mi_rows, mi_cols)?;
        self.ref_order_hints.clear();
        self.ref_order_hints
            .extend(ref_frame_idx.iter().map(|&slot| {
                ref_valid
                    .get(slot as usize)
                    .copied()
                    .filter(|valid| *valid)
                    .and_then(|_| ref_order_hint.get(slot as usize).copied())
                    .filter(|&hint| hint != u32::MAX)
            }));
        let projections = projection_queue(
            mi_dimensions,
            current_order_hint,
            config,
            ref_frame_idx,
            &self.ref_order_hints,
            ref_motion_fields,
        );
        let mut trajectories = if config.enable_trajectory {
            let mut trajectories = match self
                .trajectories
                .take()
                .or_else(|| self.trajectory_scratch.take())
            {
                Some(trajectories) => trajectories,
                None => TrajectoryState::new(
                    mi_dimensions,
                    self.ref_order_hints.len(),
                    config.step,
                    config.unit_size8,
                )?,
            };
            trajectories.reset(
                mi_dimensions,
                self.ref_order_hints.len(),
                config.step,
                config.unit_size8,
            )?;
            Some(trajectories)
        } else {
            self.trajectory_scratch = self.trajectories.take();
            None
        };
        for projection in projections {
            let slot = *ref_frame_idx.get(projection.ref_index)?;
            let source_order_hint = self
                .ref_order_hints
                .get(projection.ref_index)
                .copied()
                .flatten()?;
            let Some(source_field) = ref_motion_fields
                .get(slot as usize)
                .and_then(Option::as_ref)
            else {
                continue;
            };
            project_temporal_motion_field(
                source_field,
                source_order_hint,
                current_order_hint,
                config.step,
                config.unit_size8,
                projection.ref_index,
                projection.side,
                projection.target_ref,
                &self.ref_order_hints,
                &mut self.target_cache,
                trajectories.as_mut(),
                &mut self.field,
            );
        }
        if let Some(trajectories) = trajectories.as_mut() {
            trajectories.fill_gaps();
        }
        self.current_order_hint = current_order_hint;
        self.trajectories = trajectories;
        self.tip = None;
        Some(())
    }

    pub(crate) fn prepare_tip(
        &mut self,
        projection_step: usize,
        superblock_size8: usize,
        fill_holes: bool,
    ) -> bool {
        let Some(references) = self.tip_reference_pair() else {
            return false;
        };
        let projection_step = projection_step.clamp(1, 2);
        let tmvp_unit_size8 = if projection_step == 1 {
            8
        } else {
            superblock_size8.max(1)
        };
        if self
            .projection_scratch
            .reset(self.field.height8 * 2, self.field.width8 * 2)
            .is_none()
        {
            return false;
        }
        let field = &mut self.projection_scratch;
        for y8 in (0..field.height8).step_by(projection_step) {
            for x8 in (0..field.width8).step_by(projection_step) {
                field.set(y8, x8, Mv::ZERO, references.ref_offset, false);
                let Some(source) = self.field.cell(y8, x8) else {
                    continue;
                };
                if source.is_valid()
                    && let Some(mv) =
                        project_mv(source.mv(), references.ref_offset, source.ref_offset())
                {
                    field.set(y8, x8, mv, references.ref_offset, true);
                }
            }
        }
        if fill_holes {
            fill_tip_holes(field, projection_step, tmvp_unit_size8);
            average_tip_motion(
                field,
                &mut self.average_scratch,
                projection_step,
                tmvp_unit_size8,
            );
            std::mem::swap(field, &mut self.average_scratch);
        }
        fill_temporal_sampling_gaps(field, projection_step, tmvp_unit_size8);
        std::mem::swap(&mut self.field, field);
        self.tip = Some(references);
        true
    }

    pub(crate) fn fill_sampling_gaps(&mut self, projection_step: usize, tmvp_unit_size8: usize) {
        fill_temporal_sampling_gaps(&mut self.field, projection_step, tmvp_unit_size8);
    }

    pub(crate) fn tip_reference_pair(&self) -> Option<TipReferencePair> {
        tip_reference_pair_from_hints(self.current_order_hint, &self.ref_order_hints)
    }

    pub(crate) fn reference_order_hints(&self) -> &[Option<u32>] {
        &self.ref_order_hints
    }

    pub(crate) fn order_hint_mv_context(&self) -> OrderHintMvContext<'_> {
        OrderHintMvContext {
            current_order_hint: self.current_order_hint,
            ref_order_hints: &self.ref_order_hints,
        }
    }

    pub(crate) fn tip_references(&self) -> Option<TipReferencePair> {
        self.tip
    }

    pub(crate) fn tip_candidate(&self, y8: usize, x8: usize, base_mv: Mv) -> Option<[Mv; 2]> {
        let references = self.tip?;
        let y8 = y8.min(self.field.height8.saturating_sub(1));
        let x8 = x8.min(self.field.width8.saturating_sub(1));
        let cell = self.field.cell(y8, x8).unwrap_or_default();
        let projected = if cell.is_valid() {
            [
                project_mv(cell.mv(), references.past_offset, references.ref_offset)
                    .unwrap_or(Mv::ZERO),
                project_mv(cell.mv(), references.future_offset, references.ref_offset)
                    .unwrap_or(Mv::ZERO),
            ]
        } else {
            [Mv::ZERO; 2]
        };
        Some(projected.map(|mv| Mv {
            row: (mv.row + base_mv.row).clamp(-MV_LIMIT, MV_LIMIT),
            col: (mv.col + base_mv.col).clamp(-MV_LIMIT, MV_LIMIT),
        }))
    }

    pub(super) fn tip_spatial_mvs(
        &self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
        probe: RelativeProbe,
        cell: NeighbourCell,
    ) -> Option<[Mv; 2]> {
        if cell.ref_frame0 != TIP_REF_FRAME || cell.ref_frame1.is_some() {
            return None;
        }
        let (row, col, _) = probe.stack_target(block);
        let (row, col) = (usize::try_from(row).ok()?, usize::try_from(col).ok()?);
        let shift = 1 + usize::from(cell.tip_size_16x16());
        let base_r = cell.base_r as usize;
        let base_c = cell.base_c as usize;
        let row = base_r + ((row.checked_sub(base_r)? >> shift) << shift);
        let col = base_c + ((col.checked_sub(base_c)? >> shift) << shift);
        let base_cell = grid.get(row as i32, col as i32)?;
        self.tip_candidate(row >> 1, col >> 1, base_cell.sub_mv)
    }

    pub(super) fn tip_spatial_single_candidates(
        &self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
        probe: RelativeProbe,
        cell: NeighbourCell,
    ) -> Option<[(i8, Mv); 2]> {
        let refs = self.tip_references()?;
        let mvs = self.tip_spatial_mvs(grid, block, probe, cell)?;
        Some([(refs.past_ref, mvs[0]), (refs.future_ref, mvs[1])])
    }

    pub(super) fn derive_tip_base_mv(&self, references: [i8; 2], mvs: [Mv; 2]) -> Option<Mv> {
        let tip = self.tip_references()?;
        if references != [tip.past_ref, tip.future_ref] {
            return None;
        }
        let linear = Mv {
            row: mvs[0].row.saturating_sub(mvs[1].row),
            col: mvs[0].col.saturating_sub(mvs[1].col),
        };
        let projected = project_mv(linear, tip.past_offset, tip.ref_offset)?;
        Some(Mv {
            row: mvs[0]
                .row
                .saturating_sub(projected.row)
                .clamp(-MV_LIMIT, MV_LIMIT),
            col: mvs[0]
                .col
                .saturating_sub(projected.col)
                .clamp(-MV_LIMIT, MV_LIMIT),
        })
    }

    pub(super) fn motion_field_mv(&self, ref_frame: i8, y8: usize, x8: usize) -> Option<Mv> {
        let ref_index = usize::try_from(ref_frame).ok()?;
        if let Some(mv) = self
            .trajectories
            .as_ref()
            .and_then(|state| state.fields().get(ref_index))
            .and_then(|field| field.cell(y8, x8))
        {
            return Some(mv);
        }
        let dst_hint = usize::try_from(ref_frame)
            .ok()
            .and_then(|idx| self.ref_order_hints.get(idx))
            .copied()
            .flatten()?;
        let cell = self.field.cell(y8, x8)?;
        if !cell.is_valid() {
            return None;
        }
        let ref_to_dst = super::super::get_relative_dist(
            self.current_order_hint as i32,
            i32::try_from(dst_hint).ok()?,
        );
        project_mv(cell.mv(), ref_to_dst, cell.ref_offset())
    }

    pub(super) fn derive_spatial_mv(
        &self,
        dst_ref: i8,
        candidate_ref: i8,
        candidate_mv: Mv,
        y8: usize,
        x8: usize,
    ) -> Option<Mv> {
        let dst = usize::try_from(dst_ref).ok()?;
        let candidate = usize::try_from(candidate_ref).ok()?;
        if let Some(fields) = self.trajectories.as_ref().map(TrajectoryState::fields)
            && let (Some(dst_mv), Some(candidate_trajectory)) = (
                fields.get(dst)?.cell(y8, x8),
                fields.get(candidate)?.cell(y8, x8),
            )
        {
            return Some(derive_mv_from_trajectories(
                candidate_mv,
                dst_mv,
                candidate_trajectory,
            ));
        }

        self.order_hint_mv_context()
            .derive_spatial_mv(dst_ref, candidate_ref, candidate_mv)
    }

    pub(super) fn derive_compound_spatial_mvs(
        &self,
        dst_refs: [i8; 2],
        candidate_ref: i8,
        candidate_mv: Mv,
        y8: usize,
        x8: usize,
    ) -> Option<[Mv; 2]> {
        let fields = self.trajectories.as_ref()?.fields();
        let candidate = usize::try_from(candidate_ref).ok()?;
        let candidate_trajectory = fields.get(candidate)?.cell(y8, x8)?;
        let mut derived = [Mv::ZERO; 2];
        for (index, dst_ref) in dst_refs.into_iter().enumerate() {
            let dst = usize::try_from(dst_ref).ok()?;
            derived[index] = derive_mv_from_trajectories(
                candidate_mv,
                fields.get(dst)?.cell(y8, x8)?,
                candidate_trajectory,
            );
        }
        Some(derived)
    }

    pub(super) fn single_ref_weight(&self, ref_frame: i8) -> Option<u32> {
        let dst_hint = usize::try_from(ref_frame)
            .ok()
            .and_then(|idx| self.ref_order_hints.get(idx))
            .copied()
            .flatten()?;
        let dist = super::super::get_relative_dist(
            self.current_order_hint as i32,
            i32::try_from(dst_hint).ok()?,
        );
        Some(if dist.abs() <= 2 { 2 } else { 1 })
    }
}

pub(crate) fn reference_order_hints(
    ref_frame_idx: &[u32],
    ref_valid: &[bool],
    ref_order_hint: &[u32],
) -> Vec<Option<u32>> {
    ref_frame_idx
        .iter()
        .map(|&slot| {
            ref_valid
                .get(slot as usize)
                .copied()
                .filter(|valid| *valid)
                .and_then(|_| ref_order_hint.get(slot as usize).copied())
                .filter(|&hint| hint != u32::MAX)
        })
        .collect()
}

pub(crate) fn tip_reference_pair_from_hints(
    current_order_hint: u32,
    ref_order_hints: &[Option<u32>],
) -> Option<TipReferencePair> {
    let current = i32::try_from(current_order_hint).ok()?;
    let hints = sorted_reference_hints(ref_order_hints);
    let sorted = hints.as_slice();
    let past_index = sorted
        .iter()
        .rposition(|&(_, hint)| super::super::get_relative_dist(hint, current) < 0)?;
    let has_future = sorted
        .iter()
        .any(|&(_, hint)| super::super::get_relative_dist(hint, current) > 0);
    let future_index = if has_future {
        past_index.checked_add(1)?
    } else {
        past_index.checked_sub(1)?
    };
    let &(past_ref, past_hint) = sorted.get(past_index)?;
    let &(future_ref, future_hint) = sorted.get(future_index)?;
    let past_offset = super::super::get_relative_dist(current, past_hint);
    let future_offset = super::super::get_relative_dist(current, future_hint);
    let ref_offset = if future_offset < 0 {
        super::super::get_relative_dist(future_hint, past_hint)
    } else {
        super::super::get_relative_dist(past_hint, future_hint)
    };
    Some(TipReferencePair {
        past_ref: i8::try_from(past_ref).ok()?,
        future_ref: i8::try_from(future_ref).ok()?,
        past_offset,
        future_offset,
        ref_offset: ref_offset.min(MAX_FRAME_DISTANCE),
    })
}

/// Fixed-capacity buffer for sorted `(reference index, order hint)` pairs;
/// decode reference lists are parse-bounded to seven slots, so eight entries
/// always suffice and sorting never allocates.
pub(super) struct SortedReferenceHints {
    entries: [(usize, i32); MAX_SORTED_REFERENCE_HINTS],
    len: usize,
}

pub(super) const MAX_SORTED_REFERENCE_HINTS: usize = 8;

impl SortedReferenceHints {
    pub(super) fn as_slice(&self) -> &[(usize, i32)] {
        &self.entries[..self.len]
    }
}

fn sorted_reference_hints(ref_order_hints: &[Option<u32>]) -> SortedReferenceHints {
    debug_assert!(ref_order_hints.len() <= MAX_SORTED_REFERENCE_HINTS);
    let mut entries = [(0usize, 0i32); MAX_SORTED_REFERENCE_HINTS];
    let mut len = 0;
    let pairs = ref_order_hints
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, hint)| Some((index, i32::try_from(hint?).ok()?)));
    for pair in pairs.take(MAX_SORTED_REFERENCE_HINTS) {
        entries[len] = pair;
        len += 1;
    }
    let sorted = &mut entries[..len];
    for i in 0..sorted.len() {
        for j in i + 1..sorted.len() {
            if super::super::get_relative_dist(sorted[j].1, sorted[i].1) < 0 {
                sorted.swap(i, j);
            }
        }
    }
    SortedReferenceHints { entries, len }
}

fn fill_tip_holes(field: &mut ProjectedTemporalMotionField, step: usize, superblock_size8: usize) {
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let source = field.cell(y8, x8).unwrap_or_default();
                    for (dy, dx) in TIP_DIRECTIONS {
                        let dst_y = y8 as i32 + dy * step as i32;
                        let dst_x = x8 as i32 + dx * step as i32;
                        let Ok(dst_y) = usize::try_from(dst_y) else {
                            continue;
                        };
                        let Ok(dst_x) = usize::try_from(dst_x) else {
                            continue;
                        };
                        if dst_y >= block_y
                            && dst_y < end_y
                            && dst_x >= block_x
                            && dst_x < end_x
                            && !field
                                .cell(dst_y, dst_x)
                                .is_some_and(ProjectedTemporalMotionCell::is_valid)
                        {
                            field.set(
                                dst_y,
                                dst_x,
                                source.mv(),
                                source.ref_offset(),
                                source.is_valid(),
                            );
                        }
                    }
                }
            }
        }
    }
}

fn average_tip_motion(
    field: &ProjectedTemporalMotionField,
    averaged: &mut ProjectedTemporalMotionField,
    step: usize,
    superblock_size8: usize,
) {
    averaged.width8 = field.width8;
    averaged.height8 = field.height8;
    averaged
        .cells
        .resize(field.cells.len(), ProjectedTemporalMotionCell::default());
    averaged.cells.copy_from_slice(&field.cells);
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let mut sum = Mv::ZERO;
                    let mut count = 0usize;
                    for (dy, dx) in TIP_DIRECTIONS.into_iter().chain([(0, 0)]) {
                        let candidate_y = y8 as i32 + dy * step as i32;
                        let candidate_x = x8 as i32 + dx * step as i32;
                        let (Ok(candidate_y), Ok(candidate_x)) =
                            (usize::try_from(candidate_y), usize::try_from(candidate_x))
                        else {
                            continue;
                        };
                        if candidate_y < block_y
                            || candidate_y >= end_y
                            || candidate_x < block_x
                            || candidate_x >= end_x
                        {
                            continue;
                        }
                        let Some(cell) = field
                            .cell(candidate_y, candidate_x)
                            .filter(|cell| cell.is_valid())
                        else {
                            continue;
                        };
                        let mv = cell.mv();
                        sum.row += mv.row;
                        sum.col += mv.col;
                        count += 1;
                    }
                    if count == 0 {
                        averaged.set(y8, x8, Mv::ZERO, 0, false);
                    } else {
                        averaged.set(
                            y8,
                            x8,
                            Mv {
                                row: divide_tip_average(sum.row, count),
                                col: divide_tip_average(sum.col, count),
                            },
                            field
                                .cell(y8, x8)
                                .map_or(0, ProjectedTemporalMotionCell::ref_offset),
                            true,
                        );
                    }
                }
            }
        }
    }
}

#[doc = "AV2 § 7.10.4 Weight_Div_Mult motion-vector average."]
fn divide_tip_average(value: i32, count: usize) -> i32 {
    const WEIGHTS: [i32; 6] = [0, 65_536, 32_768, 21_845, 16_384, 13_107];
    round2_signed(i64::from(value) * i64::from(WEIGHTS[count]), 16) as i32
}

fn fill_temporal_sampling_gaps(
    field: &mut ProjectedTemporalMotionField,
    step: usize,
    tmvp_unit_size8: usize,
) {
    if step != 2 {
        return;
    }
    let tmvp_unit_size8 = tmvp_unit_size8.max(1);
    for y8 in (0..field.height8).step_by(2) {
        for x8 in (0..field.width8).step_by(2) {
            for (dy, dx) in [(0usize, 1usize), (1, 0), (1, 1)] {
                fill_temporal_sampling_gap(field, y8, x8, dy, dx, tmvp_unit_size8);
            }
        }
    }
}

#[doc = "AV2 § 7.10.5 fill_tpl and calc_avg motion-vector gap fill."]
fn fill_temporal_sampling_gap(
    field: &mut ProjectedTemporalMotionField,
    y8: usize,
    x8: usize,
    dy: usize,
    dx: usize,
    tmvp_unit_size8: usize,
) {
    let Some(anchor) = field.cell(y8, x8).filter(|cell| cell.is_valid()) else {
        return;
    };
    if y8 + dy >= field.height8 || x8 + dx >= field.width8 {
        return;
    }
    let mut sum = Mv::ZERO;
    let mut count = 0i32;
    for candidate_y in 0..=1 {
        for candidate_x in 0..=1 {
            if dy < candidate_y || dx < candidate_x {
                continue;
            }
            let source_y = y8 + 2 * candidate_y;
            let source_x = x8 + 2 * candidate_x;
            if source_y / tmvp_unit_size8 != y8 / tmvp_unit_size8
                || source_x / tmvp_unit_size8 != x8 / tmvp_unit_size8
            {
                continue;
            }
            let Some(source) = field
                .cell(source_y, source_x)
                .filter(|cell| cell.is_valid())
            else {
                continue;
            };
            let mv = if candidate_y == 0 && candidate_x == 0 {
                source.mv()
            } else {
                project_mv(source.mv(), anchor.ref_offset(), source.ref_offset()).map_or(
                    Mv::ZERO,
                    |mv| Mv {
                        row: mv.row.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                        col: mv.col.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                    },
                )
            };
            sum.row += mv.row;
            sum.col += mv.col;
            count += 1;
        }
    }
    let average = |value: i32| match count {
        1 => value,
        2 => round2_signed_i32(value, 1),
        3 => round2_signed_i32(value * 85, 8),
        _ => round2_signed_i32(value, 2),
    };
    field.set(
        y8 + dy,
        x8 + dx,
        Mv {
            row: average(sum.row),
            col: average(sum.col),
        },
        anchor.ref_offset(),
        true,
    );
}

#[allow(clippy::too_many_arguments)]
fn project_temporal_motion_field(
    source: &TemporalMotionField,
    source_order_hint: u32,
    current_order_hint: u32,
    projection_step: usize,
    tmvp_unit_size8: usize,
    source_ref: usize,
    side: usize,
    target_ref: Option<usize>,
    ref_order_hints: &[Option<u32>],
    target_cache: &mut Vec<(u32, Option<usize>, i32)>,
    mut trajectories: Option<&mut TrajectoryState>,
    output: &mut ProjectedTemporalMotionField,
) {
    let projection_step = projection_step.clamp(1, 2);
    let target_order_hint =
        target_ref.and_then(|target| ref_order_hints.get(target).copied().flatten());
    if source.width8 == 0 {
        return;
    }
    let source_hint = i32::try_from(source_order_hint).unwrap_or(i32::MAX);
    let current_hint = i32::try_from(current_order_hint).unwrap_or(i32::MAX);
    let source_to_current = super::super::get_relative_dist(source_hint, current_hint);
    target_cache.clear();
    target_cache.reserve(source.ref_order_hints.len());
    target_cache.extend(
        source
            .ref_order_hints
            .iter()
            .flatten()
            .copied()
            .map(|hint| {
                let target_hint = i32::try_from(hint).unwrap_or(i32::MAX);
                (
                    hint,
                    mapped_reference(source_order_hint, hint, ref_order_hints),
                    super::super::get_relative_dist(source_hint, target_hint),
                )
            }),
    );
    for (y8, row) in source
        .cells
        .chunks_exact(source.width8)
        .enumerate()
        .step_by(projection_step)
    {
        for (x8, cell) in row.iter().copied().enumerate().step_by(projection_step) {
            if !cell.is_valid() {
                continue;
            }
            let list = side;
            let target_hint = cell.ref_order_hints[list];
            if target_hint == INVALID_ORDER_HINT {
                continue;
            }
            let saved_target_hint = target_hint;
            let mut mv = uncompress_tmvp_mv(cell.mvs[list]);
            let (end_ref, mut ref_offset) = target_cache
                .iter()
                .find(|(hint, _, _)| *hint == saved_target_hint)
                .map_or_else(
                    || {
                        let target_hint = i32::try_from(target_hint).unwrap_or(i32::MAX);
                        (
                            mapped_reference(source_order_hint, saved_target_hint, ref_order_hints),
                            super::super::get_relative_dist(source_hint, target_hint),
                        )
                    },
                    |(_, end_ref, ref_offset)| (*end_ref, *ref_offset),
                );
            if let Some(trajectories) = trajectories.as_deref_mut() {
                trajectories.check_intersection(source_ref, end_ref, y8, x8, mv);
            }
            if ref_offset.abs() > MAX_FRAME_DISTANCE {
                continue;
            }
            if (side == 0 && ref_offset < 0) || (side == 1 && ref_offset > 0) {
                continue;
            }
            if source_to_current.abs() > MAX_FRAME_DISTANCE {
                continue;
            }
            let trajectory_mv = mv;
            let trajectory_ref_offset = ref_offset;
            if ref_offset < 0 {
                ref_offset = -ref_offset;
                mv = Mv {
                    row: -mv.row,
                    col: -mv.col,
                };
            }
            let Some(projected_to_current) = project_mv(mv, source_to_current, ref_offset) else {
                continue;
            };
            if let Some(trajectories) = trajectories.as_deref_mut() {
                trajectories.observe_projection_with_projected(
                    source_ref,
                    end_ref,
                    target_ref,
                    y8,
                    x8,
                    trajectory_mv,
                    projected_to_current,
                    source_to_current,
                    trajectory_ref_offset.abs(),
                    side == 1,
                );
            }
            let Some((pos_y8, pos_x8)) = sampled_temporal_position(
                y8,
                x8,
                projected_to_current,
                projection_step,
                tmvp_unit_size8,
                output,
            ) else {
                continue;
            };
            let Some(output_cell) = output.cells.get_mut(pos_y8 * output.width8 + pos_x8) else {
                continue;
            };
            let replace = !output_cell.is_valid()
                || (target_order_hint == Some(saved_target_hint)
                    && output_cell.ref_offset() != ref_offset);
            if replace {
                *output_cell = ProjectedTemporalMotionCell::new(mv, ref_offset, true);
            }
        }
    }
}

fn mapped_reference(
    source_order_hint: u32,
    target_order_hint: u32,
    ref_order_hints: &[Option<u32>],
) -> Option<usize> {
    ref_order_hints.iter().position(|hint| {
        hint.is_some_and(|hint| {
            let hint = i32::try_from(hint).unwrap_or(i32::MAX);
            super::super::get_relative_dist(
                hint,
                i32::try_from(target_order_hint).unwrap_or(i32::MAX),
            ) == 0
                && super::super::get_relative_dist(
                    hint,
                    i32::try_from(source_order_hint).unwrap_or(i32::MAX),
                ) != 0
        })
    })
}

fn sampled_temporal_position(
    y8: usize,
    x8: usize,
    projected_mv: Mv,
    projection_step: usize,
    tmvp_unit_size8: usize,
    field: &ProjectedTemporalMotionField,
) -> Option<(usize, usize)> {
    let projected_y8 = project_no_constraint(y8, projected_mv.row, field.height8)?;
    let projected_x8 = project_no_constraint(x8, projected_mv.col, field.width8)?;
    let projected_y8 = projected_y8 / projection_step * projection_step;
    let projected_x8 = projected_x8 / projection_step * projection_step;
    tmvp_position_is_near(
        y8,
        x8,
        projected_y8,
        projected_x8,
        projection_step,
        tmvp_unit_size8,
    )
    .then_some((projected_y8, projected_x8))
}

fn tmvp_position_is_near(
    source_y8: usize,
    source_x8: usize,
    projected_y8: usize,
    projected_x8: usize,
    projection_step: usize,
    tmvp_unit_size8: usize,
) -> bool {
    let tmvp_unit_size8 = tmvp_unit_size8.max(1);
    let base_y8 = projected_y8 / tmvp_unit_size8 * tmvp_unit_size8;
    let base_x8 = projected_x8 / tmvp_unit_size8 * tmvp_unit_size8;
    let horizontal_offset8 = if projection_step > 1 {
        tmvp_unit_size8
    } else {
        tmvp_unit_size8 / 2
    };
    source_y8 >= base_y8
        && source_y8 < base_y8.saturating_add(tmvp_unit_size8)
        && source_x8 >= base_x8.saturating_sub(horizontal_offset8)
        && source_x8
            < base_x8
                .saturating_add(tmvp_unit_size8)
                .saturating_add(horizontal_offset8)
}

fn project_no_constraint(v8: usize, delta: i32, max8: usize) -> Option<usize> {
    let offset8 = delta / (1 << (3 + 1 + 2));
    let projected = i32::try_from(v8).ok()?.checked_add(offset8)?;
    usize::try_from(projected)
        .ok()
        .filter(|&projected| projected < max8)
}

fn project_mv(mv: Mv, numerator: i32, denominator: i32) -> Option<Mv> {
    let denominator = denominator.clamp(0, MAX_FRAME_DISTANCE) as usize;
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let scale = DIV_MULT.get(denominator).copied()?;
    let bound = (1 << 16) - 1;
    let row = round2_signed(
        i64::from(mv.row) * i64::from(numerator) * i64::from(scale),
        14,
    )
    .clamp(-bound, bound) as i32;
    let col = round2_signed(
        i64::from(mv.col) * i64::from(numerator) * i64::from(scale),
        14,
    )
    .clamp(-bound, bound) as i32;
    Some(Mv { row, col })
}

fn derive_mv_from_trajectories(candidate: Mv, dst: Mv, candidate_trajectory: Mv) -> Mv {
    Mv {
        row: candidate
            .row
            .saturating_add(dst.row)
            .saturating_sub(candidate_trajectory.row)
            .clamp(-MV_LIMIT, MV_LIMIT),
        col: candidate
            .col
            .saturating_add(dst.col)
            .saturating_sub(candidate_trajectory.col)
            .clamp(-MV_LIMIT, MV_LIMIT),
    }
}

fn compress_tmvp_mv(mv: Mv) -> CompressedMv {
    CompressedMv {
        row: compress_tmvp_component(mv.row),
        col: compress_tmvp_component(mv.col),
    }
}

fn uncompress_tmvp_mv(mv: CompressedMv) -> Mv {
    Mv {
        row: uncompress_tmvp_component(mv.row),
        col: uncompress_tmvp_component(mv.col),
    }
}

fn compress_tmvp_component(value: i32) -> i16 {
    let abs_value = value.unsigned_abs();
    let msb = 31u32.saturating_sub(abs_value.leading_zeros());
    let step_log2 = msb.saturating_sub(4);
    let compressed = ((abs_value >> step_log2) + (step_log2 << 4)) as i16;
    if value < 0 { -compressed } else { compressed }
}

fn uncompress_tmvp_component(value: i16) -> i32 {
    let value = i32::from(value);
    let abs_value = value.unsigned_abs();
    let step_log2 = ((abs_value >> 4) as i32 - 1).max(0) as u32;
    let uncompressed = ((abs_value - (step_log2 << 4)) << step_log2) as i32;
    if value < 0 {
        -uncompressed
    } else {
        uncompressed
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn tip_context(
        current_order_hint: u32,
        ref_order_hints: Vec<Option<u32>>,
        mi_rows: usize,
        mi_cols: usize,
    ) -> TemporalMvContext {
        TemporalMvContext {
            current_order_hint,
            ref_order_hints,
            field: ProjectedTemporalMotionField::new(mi_rows, mi_cols).unwrap(),
            projection_scratch: ProjectedTemporalMotionField::new(0, 0).unwrap(),
            average_scratch: ProjectedTemporalMotionField::new(0, 0).unwrap(),
            target_cache: Vec::new(),
            trajectories: None,
            trajectory_scratch: None,
            tip: None,
        }
    }

    #[test]
    fn spatial_derivation_uses_reference_trajectories() {
        let mut candidate = TrajectoryMotionField::new(2, 2).unwrap();
        candidate.set(
            0,
            0,
            Mv {
                row: -10,
                col: -192,
            },
        );
        let mut dst = TrajectoryMotionField::new(2, 2).unwrap();
        dst.set(0, 0, Mv { row: 12, col: 240 });
        let mut context = tip_context(4, vec![Some(0), Some(9)], 2, 2);
        context.trajectories = Some(TrajectoryState::from_fields(vec![candidate, dst]));

        assert_eq!(
            context.derive_spatial_mv(1, 0, Mv { row: -12, col: -67 }, 0, 0),
            Some(Mv { row: 10, col: 365 })
        );
    }

    #[test]
    fn compound_spatial_derivation_uses_both_reference_trajectories() {
        let mut candidate = TrajectoryMotionField::new(2, 2).unwrap();
        candidate.set(
            0,
            0,
            Mv {
                row: -10,
                col: -192,
            },
        );
        let mut dst0 = TrajectoryMotionField::new(2, 2).unwrap();
        dst0.set(0, 0, Mv { row: 12, col: 240 });
        let mut dst1 = TrajectoryMotionField::new(2, 2).unwrap();
        dst1.set(0, 0, Mv { row: -8, col: 100 });
        let mut context = tip_context(4, vec![Some(0), Some(9), Some(6)], 2, 2);
        context.trajectories = Some(TrajectoryState::from_fields(vec![candidate, dst0, dst1]));

        assert_eq!(
            context.derive_compound_spatial_mvs([1, 2], 0, Mv { row: -12, col: -67 }, 0, 0,),
            Some([Mv { row: 10, col: 365 }, Mv { row: -10, col: 225 }])
        );
    }

    #[test]
    fn spatial_derivation_projects_references_on_the_same_side() {
        let context = tip_context(10, vec![Some(8), Some(6)], 2, 2);

        assert_eq!(
            context.derive_spatial_mv(0, 1, Mv { row: 8, col: 12 }, 0, 0),
            Some(Mv { row: 4, col: 6 })
        );
    }

    #[test]
    fn trajectory_derivation_clamps_to_the_motion_vector_domain() {
        assert_eq!(
            derive_mv_from_trajectories(
                Mv {
                    row: MV_LIMIT,
                    col: -MV_LIMIT,
                },
                Mv {
                    row: MV_LIMIT,
                    col: -MV_LIMIT,
                },
                Mv {
                    row: -MV_LIMIT,
                    col: MV_LIMIT,
                },
            ),
            Mv {
                row: MV_LIMIT,
                col: -MV_LIMIT,
            }
        );
    }

    #[test]
    fn tip_reference_pair_uses_the_nearest_past_and_future_references() {
        let context = tip_context(10, vec![Some(6), Some(9), Some(12), Some(15)], 4, 4);

        assert_eq!(
            context.tip_reference_pair(),
            Some(TipReferencePair {
                past_ref: 1,
                future_ref: 2,
                past_offset: 1,
                future_offset: -2,
                ref_offset: 3,
            })
        );
    }

    #[test]
    fn tip_reference_pair_matches_sort_ref_order_for_equal_future_hints() {
        let context = tip_context(
            8,
            vec![
                Some(10),
                Some(7),
                Some(6),
                Some(10),
                Some(5),
                Some(4),
                Some(3),
            ],
            4,
            4,
        );

        assert_eq!(
            context.tip_reference_pair(),
            Some(TipReferencePair {
                past_ref: 1,
                future_ref: 3,
                past_offset: 1,
                future_offset: -2,
                ref_offset: 3,
            })
        );
    }

    #[test]
    fn tip_reference_pair_uses_the_two_nearest_past_references() {
        let context = tip_context(10, vec![Some(2), Some(6), Some(9)], 4, 4);

        assert_eq!(
            context.tip_reference_pair(),
            Some(TipReferencePair {
                past_ref: 2,
                future_ref: 1,
                past_offset: 1,
                future_offset: 4,
                ref_offset: 3,
            })
        );
    }

    #[test]
    fn reference_order_hints_exclude_invalid_slots() {
        assert_eq!(
            reference_order_hints(&[0, 1, 2], &[true, false, true], &[8, 9, 12]),
            vec![Some(8), None, Some(12)]
        );
    }

    #[test]
    fn temporal_motion_storage_stays_compact() {
        assert_eq!(std::mem::size_of::<TemporalMotionCell>(), 16);
        assert_eq!(std::mem::size_of::<ProjectedTemporalMotionCell>(), 8);
    }

    #[test]
    fn single_reference_motion_is_stored_in_both_slots() {
        for source_list in 0..2 {
            let mut field = TemporalMotionField::new(2, 2).unwrap();
            let mut ref_order_hints = [None; 2];
            let mut mvs = [Mv::ZERO; 2];
            ref_order_hints[source_list] = Some(7);
            mvs[source_list] = Mv { row: 8, col: -12 };

            field.record_block(TemporalMotionBlock {
                mi_row: 0,
                mi_col: 0,
                n4w: 2,
                n4h: 2,
                mi_rows: 2,
                mi_cols: 2,
                current_order_hint: 8,
                ref_order_hints,
                mvs,
                warp_params: [None; 2],
            });

            let cell = field.cell(0, 0).unwrap();
            assert_eq!(cell.ref_order_hints, [7; 2]);
            assert_eq!(
                cell.mvs.map(uncompress_tmvp_mv),
                [Mv { row: 8, col: -12 }; 2]
            );
        }
    }

    #[test]
    fn compound_references_are_stored_in_temporal_slot_order() {
        let cases = [
            ([Some(1), Some(3)], [Some(3), Some(1)]),
            ([Some(5), Some(6)], [Some(6), Some(5)]),
            ([Some(6), Some(3)], [Some(3), Some(6)]),
            ([Some(3), Some(6)], [Some(3), Some(6)]),
        ];
        for (input, expected) in cases {
            let mut field = TemporalMotionField::new(2, 2).unwrap();
            field.record_block(TemporalMotionBlock {
                mi_row: 0,
                mi_col: 0,
                n4w: 2,
                n4h: 2,
                mi_rows: 2,
                mi_cols: 2,
                current_order_hint: 4,
                ref_order_hints: input,
                mvs: [Mv { row: 8, col: 16 }, Mv { row: 24, col: 32 }],
                warp_params: [None; 2],
            });

            let cell = field.cell(0, 0).unwrap();
            let stored_hints = cell.ref_order_hints.map(Some);
            assert_eq!(stored_hints, expected);
            let swapped = input != expected;
            assert_eq!(
                uncompress_tmvp_mv(cell.mvs[0]).row,
                if swapped { 24 } else { 8 }
            );
            assert_eq!(
                uncompress_tmvp_mv(cell.mvs[1]).row,
                if swapped { 8 } else { 24 }
            );
        }
    }

    #[test]
    fn step_two_projection_samples_and_stores_on_the_even_grid() {
        let mut source = TemporalMotionField::new(8, 8).unwrap();
        for x8 in 0..source.width8 {
            source.cells[x8] = TemporalMotionCell {
                ref_order_hints: [0, INVALID_ORDER_HINT],
                mvs: [
                    compress_tmvp_mv(Mv { row: 0, col: -64 }),
                    CompressedMv::default(),
                ],
            };
        }
        source.set_reference_metadata(true, (32, 32), &[Some(0)]);
        let mut other = TemporalMotionField::new(8, 8).unwrap();
        other.set_reference_metadata(true, (32, 32), &[]);

        let mut context = TemporalMvContext::from_references(
            (8, 8),
            2,
            TemporalProjectionConfig {
                frame_size: (32, 32),
                step: 2,
                unit_size8: 8,
                enable_tip: false,
                enable_trajectory: false,
                reduced: false,
            },
            &[0, 1],
            &[true, true],
            &[1, 3],
            &[Some(Arc::new(source)), Some(Arc::new(other))],
        )
        .unwrap();

        assert!(context.field.cell(0, 0).unwrap().is_valid());
        assert!(context.field.cell(0, 2).unwrap().is_valid());
        assert!(!context.field.cell(0, 1).unwrap().is_valid());
        assert!(!context.field.cell(0, 3).unwrap().is_valid());

        context.fill_sampling_gaps(2, 16);
        assert_eq!(context.field.cell(0, 1), context.field.cell(0, 0));
        assert_eq!(context.field.cell(0, 3), context.field.cell(0, 2));
    }

    #[test]
    fn backward_projection_preserves_source_to_current_direction() {
        let mut source = TemporalMotionField::new(18, 56).unwrap();
        *source.cell_mut(8, 26).unwrap() = TemporalMotionCell {
            ref_order_hints: [INVALID_ORDER_HINT, 9],
            mvs: [
                CompressedMv::default(),
                compress_tmvp_mv(Mv { row: 10, col: 232 }),
            ],
        };
        let mut output = ProjectedTemporalMotionField::new(18, 56).unwrap();

        project_temporal_motion_field(
            &source,
            4,
            2,
            1,
            8,
            0,
            1,
            None,
            &[],
            &mut Vec::new(),
            None,
            &mut output,
        );

        assert_eq!(
            output.cell(8, 25),
            Some(ProjectedTemporalMotionCell::new(
                Mv {
                    row: -10,
                    col: -232,
                },
                5,
                true,
            ))
        );
        assert!(!output.cell(8, 27).unwrap().is_valid());
    }

    #[test]
    fn projection_records_zero_offset_reference() {
        let mut source = TemporalMotionField::new(4, 4).unwrap();
        *source.cell_mut(0, 0).unwrap() = TemporalMotionCell {
            ref_order_hints: [4, INVALID_ORDER_HINT],
            mvs: [CompressedMv::default(); 2],
        };
        let mut output = ProjectedTemporalMotionField::new(4, 4).unwrap();

        project_temporal_motion_field(
            &source,
            4,
            2,
            1,
            8,
            0,
            0,
            None,
            &[Some(4)],
            &mut Vec::new(),
            None,
            &mut output,
        );

        assert_eq!(
            output.cell(0, 0),
            Some(ProjectedTemporalMotionCell::new(Mv::ZERO, 0, true))
        );
    }

    #[test]
    fn zero_offset_projection_uses_the_zero_divisor_multiplier() {
        assert_eq!(project_mv(Mv { row: 24, col: -40 }, 3, 0), Some(Mv::ZERO));
    }

    #[test]
    fn side_rejected_projection_still_extends_existing_trajectory() {
        let mut trajectories = TrajectoryState::new((112, 252), 6, 1, 8).unwrap();
        trajectories.observe_projection(
            0,
            Some(1),
            Some(1),
            54,
            125,
            Mv { row: 64, col: -256 },
            1,
            2,
            false,
        );
        let mut source = TemporalMotionField::new(112, 252).unwrap();
        source.record_block(TemporalMotionBlock {
            mi_row: 110,
            mi_col: 242,
            n4w: 2,
            n4h: 2,
            mi_rows: 112,
            mi_cols: 252,
            current_order_hint: 4,
            ref_order_hints: [Some(0), None],
            mvs: [Mv { row: 36, col: -160 }, Mv::ZERO],
            warp_params: [None; 2],
        });
        let mut output = ProjectedTemporalMotionField::new(112, 252).unwrap();

        project_temporal_motion_field(
            &source,
            4,
            5,
            1,
            8,
            1,
            1,
            None,
            &[Some(6), Some(4), None, None, None, Some(0)],
            &mut Vec::new(),
            Some(&mut trajectories),
            &mut output,
        );

        let fields = trajectories.into_fields();
        assert_eq!(fields[5].cell(54, 123), Some(Mv { row: 68, col: -288 }));
        assert!(output.cells.iter().all(|cell| !cell.is_valid()));
    }

    #[test]
    fn sampling_gap_does_not_average_across_tmvp_units() {
        let mut context = tip_context(2, vec![Some(0)], 2, 36);
        context.field.set(0, 14, Mv { row: 8, col: 16 }, 2, true);
        context.field.set(0, 16, Mv { row: 24, col: 80 }, 4, true);

        context.fill_sampling_gaps(2, 16);

        assert_eq!(context.field.cell(0, 15), context.field.cell(0, 14));
        assert_eq!(context.field.cell(0, 17), context.field.cell(0, 16));
    }

    #[test]
    fn temporal_projection_stays_within_the_vertical_tmvp_unit() {
        assert!(!tmvp_position_is_near(52, 107, 47, 107, 2, 16));
        assert!(tmvp_position_is_near(47, 107, 47, 107, 2, 16));
    }

    #[test]
    fn step_one_projection_uses_64_pixel_tmvp_unit_bounds() {
        assert!(!tmvp_position_is_near(52, 107, 47, 107, 1, 8));
        assert!(tmvp_position_is_near(47, 107, 47, 107, 1, 8));
        assert!(tmvp_position_is_near(47, 100, 47, 107, 1, 8));
        assert!(!tmvp_position_is_near(47, 99, 47, 107, 1, 8));
        assert!(tmvp_position_is_near(47, 115, 47, 107, 1, 8));
        assert!(!tmvp_position_is_near(47, 116, 47, 107, 1, 8));
    }

    #[test]
    fn tip_projection_fills_unsampled_units_and_adds_the_block_mv() {
        let mut context = tip_context(10, vec![Some(8), Some(12)], 4, 4);
        context.field.set(0, 0, Mv { row: 8, col: -16 }, 4, true);

        assert!(context.prepare_tip(2, 2, false));
        assert_eq!(context.tip_references(), context.tip_reference_pair());
        let expected = [Mv { row: 5, col: -6 }, Mv { row: -3, col: 10 }];
        for y8 in 0..2 {
            for x8 in 0..2 {
                assert_eq!(
                    context.tip_candidate(y8, x8, Mv { row: 1, col: 2 }),
                    Some(expected)
                );
            }
        }
    }

    #[test]
    fn tip_step_one_hole_fill_stays_within_64_pixel_tmvp_units() {
        let mut context = tip_context(10, vec![Some(8), Some(12)], 2, 32);
        context.field.set(0, 7, Mv { row: 8, col: -16 }, 4, true);

        assert!(context.prepare_tip(1, 16, true));
        assert_eq!(context.tip_candidate(0, 8, Mv::ZERO), Some([Mv::ZERO; 2]));
    }

    #[test]
    fn tip_candidate_clamps_motion_field_coordinates_to_the_frame() {
        let references = TipReferencePair {
            past_ref: 0,
            future_ref: 1,
            past_offset: -1,
            future_offset: 1,
            ref_offset: 1,
        };
        let context =
            TemporalMvContext::with_tip_sample(4, 4, references, 1, 1, Mv { row: 16, col: 32 })
                .unwrap();

        assert_eq!(
            context.tip_candidate(usize::MAX, usize::MAX, Mv::ZERO),
            context.tip_candidate(1, 1, Mv::ZERO)
        );
    }

    #[test]
    fn tip_newly_averaged_sample_keeps_the_scaled_reference_offset() {
        let mut context = tip_context(10, vec![Some(6), Some(15)], 2, 32);
        context.field.set(0, 14, Mv { row: 18, col: -36 }, 9, true);

        assert!(context.prepare_tip(2, 16, true));
        let cell = context.field.cell(0, 11).unwrap();
        assert_eq!(
            cell,
            ProjectedTemporalMotionCell::new(Mv { row: 18, col: -36 }, 9, true)
        );
    }

    #[test]
    fn refresh_reuses_projected_and_trajectory_storage() {
        let config = TemporalProjectionConfig {
            frame_size: (64, 64),
            step: 1,
            unit_size8: 8,
            enable_tip: false,
            enable_trajectory: true,
            reduced: false,
        };
        let ref_frame_idx = [0];
        let ref_valid = [false];
        let ref_order_hint = [u32::MAX];
        let ref_motion_fields = [None];
        let mut context = TemporalMvContext::from_references(
            (16, 16),
            0,
            config,
            &ref_frame_idx,
            &ref_valid,
            &ref_order_hint,
            &ref_motion_fields,
        )
        .unwrap();
        let field_ptr = context.field.cells.as_ptr();
        let trajectories = context.trajectories.as_ref().unwrap();
        let trajectory_ptr = trajectories.fields[0].cells.as_ptr();
        let positions_ptr = trajectories.positions[0].as_ptr();
        let offsets_ptr = trajectories.projection_offsets.as_ptr();

        context
            .refresh_from_references(
                (16, 16),
                1,
                config,
                &ref_frame_idx,
                &ref_valid,
                &ref_order_hint,
                &ref_motion_fields,
            )
            .unwrap();

        let trajectories = context.trajectories.as_ref().unwrap();
        assert_eq!(context.field.cells.as_ptr(), field_ptr);
        assert_eq!(trajectories.fields[0].cells.as_ptr(), trajectory_ptr);
        assert_eq!(trajectories.positions[0].as_ptr(), positions_ptr);
        assert_eq!(trajectories.projection_offsets.as_ptr(), offsets_ptr);
    }
}
