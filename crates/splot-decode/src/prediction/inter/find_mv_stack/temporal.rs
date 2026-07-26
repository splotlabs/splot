// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::{round2_signed, round2_signed_i32};
use std::sync::Arc;

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
const INVALID_TEMPORAL_REF: u8 = u8::MAX;
const DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompressedTemporalMv {
    row: i8,
    col: i8,
}

impl CompressedTemporalMv {
    const ZERO: Self = Self { row: 0, col: 0 };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TemporalMotionCell {
    ref_indices: [u8; 2],
    mvs: [CompressedTemporalMv; 2],
}

impl Default for TemporalMotionCell {
    fn default() -> Self {
        Self {
            ref_indices: [INVALID_TEMPORAL_REF; 2],
            mvs: [CompressedTemporalMv::ZERO; 2],
        }
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
    pending_ref_hints: Option<Vec<[u32; 2]>>,
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
            pending_ref_hints: Some(Vec::new()),
            is_inter: false,
            frame_size: None,
            ref_order_hints: Vec::new(),
        }
    }

    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        let pending_ref_hints = vec![[u32::MAX; 2]; cells.len()];
        Some(Self {
            width8,
            height8,
            cells,
            pending_ref_hints: Some(pending_ref_hints),
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
        if let Some(pending) = self.pending_ref_hints.take() {
            for (cell, hints) in self.cells.iter_mut().zip(pending) {
                for (list, hint) in hints.into_iter().enumerate() {
                    if hint == u32::MAX {
                        continue;
                    }
                    cell.ref_indices[list] = self
                        .ref_order_hints
                        .iter()
                        .position(|&candidate| candidate == Some(hint))
                        .and_then(|index| u8::try_from(index).ok())
                        .unwrap_or(INVALID_TEMPORAL_REF);
                }
            }
        }
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
                let mut hints = [None; 2];
                let mut mvs = [CompressedTemporalMv::ZERO; 2];
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
                    hints[list] = Some(order_hint);
                    mvs[list] = compress_tmvp_mv(mv);
                }
                if hints[0].is_some() && hints[1].is_none() {
                    hints[1] = hints[0];
                    mvs[1] = mvs[0];
                } else if hints[1].is_some() && hints[0].is_none() {
                    hints[0] = hints[1];
                    mvs[0] = mvs[1];
                } else if let [Some(ref0), Some(ref1)] = hints {
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
                        hints.swap(0, 1);
                        mvs.swap(0, 1);
                    }
                }
                let mut cell = TemporalMotionCell {
                    mvs,
                    ..TemporalMotionCell::default()
                };
                for (list, hint) in hints.iter().copied().enumerate() {
                    let Some(ref_index) = hint
                        .and_then(|hint| {
                            self.ref_order_hints
                                .iter()
                                .position(|&candidate| candidate == Some(hint))
                        })
                        .and_then(|index| u8::try_from(index).ok())
                    else {
                        continue;
                    };
                    cell.ref_indices[list] = ref_index;
                }
                let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8) else {
                    continue;
                };
                if let Some(pending) = self.pending_ref_hints.as_mut() {
                    pending[index] = hints.map(|hint| hint.unwrap_or(u32::MAX));
                }
                self.cells[index] = cell;
            }
        }
    }

    #[cfg(test)]
    fn cell(&self, y8: usize, x8: usize) -> Option<TemporalMotionCell> {
        self.cells
            .get(temporal_grid_index(self.width8, self.height8, y8, x8)?)
            .copied()
    }

    #[cfg(test)]
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
struct ProjectedTemporalMotionCell {
    valid: bool,
    mv: Mv,
    ref_offset: i32,
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
            *cell = ProjectedTemporalMotionCell {
                valid,
                mv,
                ref_offset,
            };
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
        let reset_started = crate::timing::start();
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
        crate::timing::report("inter_temporal_reset", reset_started);
        let queue_started = crate::timing::start();
        let projections = projection_queue(
            mi_dimensions,
            current_order_hint,
            config,
            ref_frame_idx,
            &self.ref_order_hints,
            ref_motion_fields,
        );
        crate::timing::report("inter_temporal_queue", queue_started);
        let trajectory_reset_started = crate::timing::start();
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
        crate::timing::report("inter_temporal_trajectory_reset", trajectory_reset_started);
        let projection_started = crate::timing::start();
        for projection in projections {
            let slot = *ref_frame_idx.get(projection.ref_index)?;
            let source_order_hint = self
                .ref_order_hints
                .get(projection.ref_index)
                .copied()
                .flatten()?;
            let Some(source_field) = ref_motion_fields
                .get(slot as usize)
                .and_then(Option::as_deref)
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
                trajectories.as_mut(),
                &mut self.field,
            );
        }
        crate::timing::report("inter_temporal_projection", projection_started);
        let gap_started = crate::timing::start();
        if let Some(trajectories) = trajectories.as_mut() {
            trajectories.fill_gaps();
        }
        crate::timing::report("inter_temporal_trajectory_gaps", gap_started);
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
        debug_assert_eq!(field.width8, self.field.width8);
        debug_assert_eq!(field.height8, self.field.height8);
        for y8 in (0..field.height8).step_by(projection_step) {
            let row_start = y8 * field.width8;
            for x8 in (0..field.width8).step_by(projection_step) {
                let index = row_start + x8;
                let source = self.field.cells[index];
                let projected = if source.valid {
                    project_mv(source.mv, references.ref_offset, source.ref_offset)
                } else {
                    None
                };
                field.cells[index] = ProjectedTemporalMotionCell {
                    valid: projected.is_some(),
                    mv: projected.unwrap_or(Mv::ZERO),
                    ref_offset: references.ref_offset,
                };
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
        let projected = if cell.valid {
            [
                project_mv(cell.mv, references.past_offset, references.ref_offset)
                    .unwrap_or(Mv::ZERO),
                project_mv(cell.mv, references.future_offset, references.ref_offset)
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
        if cell.flags.ref_frame0 != TIP_REF_FRAME || cell.flags.ref_frame1.is_some() {
            return None;
        }
        let (row, col, _) = probe.stack_target(block);
        let (row, col) = (usize::try_from(row).ok()?, usize::try_from(col).ok()?);
        let shift = 1 + usize::from(cell.flags.tip_size_16x16());
        let base_r = usize::try_from(cell.motion.base_r).ok()?;
        let base_c = usize::try_from(cell.motion.base_c).ok()?;
        let row = base_r + ((row.checked_sub(base_r)? >> shift) << shift);
        let col = base_c + ((col.checked_sub(base_c)? >> shift) << shift);
        let base_cell = grid.get(row as i32, col as i32)?;
        self.tip_candidate(row >> 1, col >> 1, base_cell.motion.sub_mv)
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
        if !cell.valid {
            return None;
        }
        let ref_to_dst = super::super::get_relative_dist(
            self.current_order_hint as i32,
            i32::try_from(dst_hint).ok()?,
        );
        project_mv(cell.mv, ref_to_dst, cell.ref_offset)
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
    let sorted = sorted_reference_hints(ref_order_hints);
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

fn sorted_reference_hints(ref_order_hints: &[Option<u32>]) -> Vec<(usize, i32)> {
    let mut sorted = ref_order_hints
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, hint)| Some((index, i32::try_from(hint?).ok()?)))
        .collect::<Vec<_>>();
    for i in 0..sorted.len() {
        for j in i + 1..sorted.len() {
            if super::super::get_relative_dist(sorted[j].1, sorted[i].1) < 0 {
                sorted.swap(i, j);
            }
        }
    }
    sorted
}

fn fill_tip_holes(field: &mut ProjectedTemporalMotionField, step: usize, superblock_size8: usize) {
    let width8 = field.width8;
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let index = y8 * width8 + x8;
                    let source = field.cells[index];
                    let mut fill = |destination: usize| {
                        if !field.cells[destination].valid {
                            field.cells[destination] = source;
                        }
                    };
                    if y8 >= block_y + step {
                        fill(index - step * width8);
                    }
                    if x8 >= block_x + step {
                        fill(index - step);
                    }
                    if y8 + step < end_y {
                        fill(index + step * width8);
                    }
                    if x8 + step < end_x {
                        fill(index + step);
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
    let width8 = field.width8;
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let mut sum = Mv::ZERO;
                    let mut count = 0usize;
                    let index = y8 * width8 + x8;
                    let mut add = |candidate: usize| {
                        let cell = field.cells[candidate];
                        if cell.valid {
                            sum.row += cell.mv.row;
                            sum.col += cell.mv.col;
                            count += 1;
                        }
                    };
                    add(index);
                    if y8 >= block_y + step {
                        add(index - step * width8);
                    }
                    if x8 >= block_x + step {
                        add(index - step);
                    }
                    if y8 + step < end_y {
                        add(index + step * width8);
                    }
                    if x8 + step < end_x {
                        add(index + step);
                    }
                    averaged.cells[index] = if count == 0 {
                        ProjectedTemporalMotionCell::default()
                    } else {
                        ProjectedTemporalMotionCell {
                            valid: true,
                            mv: Mv {
                                row: divide_tip_average(sum.row, count),
                                col: divide_tip_average(sum.col, count),
                            },
                            ref_offset: field.cells[index].ref_offset,
                        }
                    };
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
    let Some(anchor) = field.cell(y8, x8).filter(|cell| cell.valid) else {
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
            let Some(source) = field.cell(source_y, source_x).filter(|cell| cell.valid) else {
                continue;
            };
            let mv = if candidate_y == 0 && candidate_x == 0 {
                source.mv
            } else {
                project_mv(source.mv, anchor.ref_offset, source.ref_offset).map_or(Mv::ZERO, |mv| {
                    Mv {
                        row: mv.row.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                        col: mv.col.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                    }
                })
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
        anchor.ref_offset,
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
    let target_cache = source
        .ref_order_hints
        .iter()
        .map(|&hint| {
            let hint = hint?;
            let target_hint = i32::try_from(hint).unwrap_or(i32::MAX);
            Some((
                hint,
                mapped_reference(source_order_hint, hint, ref_order_hints),
                super::super::get_relative_dist(source_hint, target_hint),
            ))
        })
        .collect::<Vec<_>>();
    let mut last_target = None;
    for (y8, row) in source
        .cells
        .chunks_exact(source.width8)
        .enumerate()
        .step_by(projection_step)
    {
        for (x8, cell) in row.iter().copied().enumerate().step_by(projection_step) {
            let list = side;
            let ref_index = usize::from(cell.ref_indices[list]);
            let (target_hint, end_ref, mut ref_offset, projection_factor) = match last_target {
                Some((cached_ref, target_hint, end_ref, ref_offset, projection_factor))
                    if cached_ref == ref_index =>
                {
                    (target_hint, end_ref, ref_offset, projection_factor)
                }
                _ => {
                    let Some(&(target_hint, end_ref, ref_offset)) =
                        target_cache.get(ref_index).and_then(Option::as_ref)
                    else {
                        continue;
                    };
                    let projection_factor =
                        tmvp_projection_factor(source_to_current, ref_offset, side);
                    last_target = Some((
                        ref_index,
                        target_hint,
                        end_ref,
                        ref_offset,
                        projection_factor,
                    ));
                    (target_hint, end_ref, ref_offset, projection_factor)
                }
            };
            let saved_target_hint = target_hint;
            let mut mv = uncompress_tmvp_mv(cell.mvs[list]);
            let trajectory_target_position = trajectories.as_deref_mut().and_then(|trajectories| {
                trajectories.check_intersection(source_ref, end_ref, y8, x8, mv)
            });
            let Some(projection_factor) = projection_factor else {
                continue;
            };
            let trajectory_mv = mv;
            let trajectory_ref_offset = ref_offset;
            if ref_offset < 0 {
                ref_offset = -ref_offset;
                mv = Mv {
                    row: -mv.row,
                    col: -mv.col,
                };
            }
            let Some(projected_to_current) =
                project_tmvp_mv_with_factor(mv, source_to_current, ref_offset, projection_factor)
            else {
                continue;
            };
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
            if let Some(trajectories) = trajectories.as_deref_mut() {
                trajectories.observe_projection_at(
                    source_ref,
                    end_ref,
                    target_ref,
                    y8,
                    x8,
                    trajectory_mv,
                    projected_to_current,
                    (pos_y8, pos_x8),
                    trajectory_target_position,
                    source_to_current,
                    trajectory_ref_offset.abs(),
                    side == 1,
                );
            }
            let Some(output_cell) = output.cells.get_mut(pos_y8 * output.width8 + pos_x8) else {
                continue;
            };
            let replace = !output_cell.valid
                || (target_order_hint == Some(saved_target_hint)
                    && output_cell.ref_offset != ref_offset);
            if replace {
                *output_cell = ProjectedTemporalMotionCell {
                    valid: true,
                    mv,
                    ref_offset,
                };
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
    debug_assert!(projection_step.is_power_of_two());
    let step_mask = projection_step - 1;
    let projected_y8 = projected_y8 & !step_mask;
    let projected_x8 = projected_x8 & !step_mask;
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
    debug_assert!(tmvp_unit_size8.is_power_of_two());
    let unit_mask = tmvp_unit_size8 - 1;
    let base_y8 = projected_y8 & !unit_mask;
    let base_x8 = projected_x8 & !unit_mask;
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

fn tmvp_projection_factor(numerator: i32, ref_offset: i32, side: usize) -> Option<i32> {
    if ref_offset.abs() > MAX_FRAME_DISTANCE
        || (side == 0 && ref_offset < 0)
        || (side == 1 && ref_offset > 0)
        || numerator.abs() > MAX_FRAME_DISTANCE
    {
        return None;
    }
    let denominator = ref_offset.unsigned_abs() as usize;
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    DIV_MULT
        .get(denominator)
        .copied()
        .map(|scale| numerator * scale)
}

fn project_tmvp_mv_with_factor(
    mv: Mv,
    numerator: i32,
    denominator: i32,
    factor: i32,
) -> Option<Mv> {
    if mv.row.unsigned_abs() > REFMVS_LIMIT as u32 || mv.col.unsigned_abs() > REFMVS_LIMIT as u32 {
        return project_mv(mv, numerator, denominator);
    }
    let project = |component: i32| {
        let scaled = component * factor;
        let magnitude = scaled.abs();
        let rounded = (magnitude + (1 << 13)) >> 14;
        if scaled < 0 { -rounded } else { rounded }.clamp(-MV_LIMIT, MV_LIMIT)
    };
    Some(Mv {
        row: project(mv.row),
        col: project(mv.col),
    })
}

#[cfg(test)]
fn project_tmvp_mv(mv: Mv, numerator: i32, denominator: i32) -> Option<Mv> {
    let denominator = denominator.clamp(0, MAX_FRAME_DISTANCE);
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let factor = numerator * DIV_MULT.get(denominator as usize).copied()?;
    project_tmvp_mv_with_factor(mv, numerator, denominator, factor)
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

fn compress_tmvp_mv(mv: Mv) -> CompressedTemporalMv {
    CompressedTemporalMv {
        row: compress_tmvp_component(mv.row) as i8,
        col: compress_tmvp_component(mv.col) as i8,
    }
}

fn uncompress_tmvp_mv(mv: CompressedTemporalMv) -> Mv {
    Mv {
        row: uncompress_tmvp_component(i32::from(mv.row)),
        col: uncompress_tmvp_component(i32::from(mv.col)),
    }
}

fn compress_tmvp_component(value: i32) -> i32 {
    let abs_value = value.unsigned_abs();
    let msb = 31u32.saturating_sub(abs_value.leading_zeros());
    let step_log2 = msb.saturating_sub(4);
    let compressed = ((abs_value >> step_log2) + (step_log2 << 4)) as i32;
    if value < 0 { -compressed } else { compressed }
}

fn uncompress_tmvp_component(value: i32) -> i32 {
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
    fn single_reference_motion_is_stored_in_both_slots() {
        for source_list in 0..2 {
            let mut field = TemporalMotionField::new(2, 2).unwrap();
            field.set_reference_metadata(true, (8, 8), &[Some(7)]);
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
            assert_eq!(cell.ref_indices, [0; 2]);
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
            field.set_reference_metadata(true, (8, 8), &[Some(1), Some(3), Some(5), Some(6)]);
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
            assert_eq!(
                cell.ref_indices
                    .map(|index| field.ref_order_hints[usize::from(index)]),
                expected
            );
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
        source.set_reference_metadata(true, (32, 32), &[Some(0)]);
        for x8 in 0..source.width8 {
            source.cells[x8] = TemporalMotionCell {
                ref_indices: [0, INVALID_TEMPORAL_REF],
                mvs: [
                    compress_tmvp_mv(Mv { row: 0, col: -64 }),
                    CompressedTemporalMv::ZERO,
                ],
            };
        }
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

        assert!(context.field.cell(0, 0).unwrap().valid);
        assert!(context.field.cell(0, 2).unwrap().valid);
        assert!(!context.field.cell(0, 1).unwrap().valid);
        assert!(!context.field.cell(0, 3).unwrap().valid);

        context.fill_sampling_gaps(2, 16);
        assert_eq!(context.field.cell(0, 1), context.field.cell(0, 0));
        assert_eq!(context.field.cell(0, 3), context.field.cell(0, 2));
    }

    #[test]
    fn backward_projection_preserves_source_to_current_direction() {
        let mut source = TemporalMotionField::new(18, 56).unwrap();
        source.set_reference_metadata(true, (56 * 8, 18 * 8), &[Some(9)]);
        *source.cell_mut(8, 26).unwrap() = TemporalMotionCell {
            ref_indices: [INVALID_TEMPORAL_REF, 0],
            mvs: [
                CompressedTemporalMv::ZERO,
                compress_tmvp_mv(Mv { row: 10, col: 232 }),
            ],
        };
        let mut output = ProjectedTemporalMotionField::new(18, 56).unwrap();

        project_temporal_motion_field(&source, 4, 2, 1, 8, 0, 1, None, &[], None, &mut output);

        assert_eq!(
            output.cell(8, 25),
            Some(ProjectedTemporalMotionCell {
                valid: true,
                mv: Mv {
                    row: -10,
                    col: -232,
                },
                ref_offset: 5,
            })
        );
        assert!(!output.cell(8, 27).unwrap().valid);
    }

    #[test]
    fn projection_records_zero_offset_reference() {
        let mut source = TemporalMotionField::new(4, 4).unwrap();
        source.set_reference_metadata(true, (16, 16), &[Some(4)]);
        *source.cell_mut(0, 0).unwrap() = TemporalMotionCell {
            ref_indices: [0, INVALID_TEMPORAL_REF],
            mvs: [CompressedTemporalMv::ZERO; 2],
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
            None,
            &mut output,
        );

        assert_eq!(
            output.cell(0, 0),
            Some(ProjectedTemporalMotionCell {
                valid: true,
                mv: Mv::ZERO,
                ref_offset: 0,
            })
        );
    }

    #[test]
    fn zero_offset_projection_uses_the_zero_divisor_multiplier() {
        assert_eq!(project_mv(Mv { row: 24, col: -40 }, 3, 0), Some(Mv::ZERO));
    }

    #[test]
    fn bounded_tmvp_projection_matches_wide_projection() {
        let components = [-REFMVS_LIMIT, -1024, -1, 0, 1, 1024, REFMVS_LIMIT];
        for &component in &components {
            let mv = Mv {
                row: component,
                col: -component,
            };
            for numerator in -MAX_FRAME_DISTANCE..=MAX_FRAME_DISTANCE {
                for denominator in 0..=MAX_FRAME_DISTANCE {
                    assert_eq!(
                        project_tmvp_mv(mv, numerator, denominator),
                        project_mv(mv, numerator, denominator),
                    );
                }
            }
        }
        let wide = Mv {
            row: REFMVS_LIMIT + 1,
            col: -(REFMVS_LIMIT + 1),
        };
        assert_eq!(project_tmvp_mv(wide, 31, 1), project_mv(wide, 31, 1));
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
        source.set_reference_metadata(true, (1008, 448), &[Some(0)]);
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
            Some(&mut trajectories),
            &mut output,
        );

        let fields = trajectories.into_fields();
        assert_eq!(fields[5].cell(54, 123), Some(Mv { row: 68, col: -288 }));
        assert!(output.cells.iter().all(|cell| !cell.valid));
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
            ProjectedTemporalMotionCell {
                valid: true,
                mv: Mv { row: 18, col: -36 },
                ref_offset: 9,
            }
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
