// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed;

use super::{
    Mv, MvBlockContext, NeighbourCell, NeighbourMvGrid, RelativeProbe, TIP_REF_FRAME,
    warp_sub_mv_at,
};
use selection::projection_queue;
use trajectory::{TrajectoryMotionField, TrajectoryState};

mod selection;
mod trajectory;

const MAX_FRAME_DISTANCE: i32 = 31;
const REFMVS_LIMIT: i32 = (1 << 11) - 1;
const MV_LIMIT: i32 = (1 << 16) - 1;
const TIP_DIRECTIONS: [(i32, i32); 4] = [(-1, 0), (0, -1), (1, 0), (0, 1)];
const DIV_MULT: [i64; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TemporalMotionCell {
    ref_order_hints: [Option<u32>; 2],
    mvs: [Mv; 2],
}

impl TemporalMotionCell {
    const fn is_valid(self) -> bool {
        self.ref_order_hints[0].is_some() || self.ref_order_hints[1].is_some()
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
                    cell.ref_order_hints[list] = Some(order_hint);
                    cell.mvs[list] = compress_tmvp_mv(mv);
                }
                if cell.ref_order_hints[0].is_some() && cell.ref_order_hints[1].is_none() {
                    cell.ref_order_hints[1] = cell.ref_order_hints[0];
                    cell.mvs[1] = cell.mvs[0];
                } else if cell.ref_order_hints[1].is_some() && cell.ref_order_hints[0].is_none() {
                    cell.ref_order_hints[0] = cell.ref_order_hints[1];
                    cell.mvs[0] = cell.mvs[1];
                } else if let [Some(ref0), Some(ref1)] = cell.ref_order_hints {
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
    pub(crate) warp_params: [Option<[i64; 6]>; 2],
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
    fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
        })
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
struct TipMotionField {
    field: ProjectedTemporalMotionField,
    references: TipReferencePair,
}

impl TipMotionField {
    fn candidate(&self, y8: usize, x8: usize, base_mv: Mv) -> [Mv; 2] {
        let y8 = y8.min(self.field.height8.saturating_sub(1));
        let x8 = x8.min(self.field.width8.saturating_sub(1));
        let cell = self.field.cell(y8, x8).unwrap_or_default();
        let projected = if cell.valid {
            [
                project_mv(
                    cell.mv,
                    self.references.past_offset,
                    self.references.ref_offset,
                )
                .unwrap_or(Mv::ZERO),
                project_mv(
                    cell.mv,
                    self.references.future_offset,
                    self.references.ref_offset,
                )
                .unwrap_or(Mv::ZERO),
            ]
        } else {
            [Mv::ZERO; 2]
        };
        projected.map(|mv| Mv {
            row: (mv.row + base_mv.row).clamp(-MV_LIMIT, MV_LIMIT),
            col: (mv.col + base_mv.col).clamp(-MV_LIMIT, MV_LIMIT),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMvContext {
    current_order_hint: u32,
    ref_order_hints: Vec<Option<u32>>,
    field: ProjectedTemporalMotionField,
    trajectories: Option<Vec<TrajectoryMotionField>>,
    tip: Option<TipMotionField>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemporalProjectionConfig {
    pub(crate) frame_size: (usize, usize),
    pub(crate) step: usize,
    pub(crate) enable_tip: bool,
    pub(crate) enable_trajectory: bool,
    pub(crate) reduced: bool,
}

impl TemporalMvContext {
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
            field: field.clone(),
            trajectories: None,
            tip: Some(TipMotionField { field, references }),
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
            self.trajectories = Some(fields);
        }
        self.trajectories
            .as_mut()?
            .get_mut(reference)?
            .set(y8, x8, mv);
        Some(())
    }

    pub(crate) fn from_references(
        mi_dimensions: (usize, usize),
        current_order_hint: u32,
        config: TemporalProjectionConfig,
        ref_frame_idx: &[u32],
        ref_valid: &[bool],
        ref_order_hint: &[u32],
        ref_motion_fields: &[Option<TemporalMotionField>],
    ) -> Option<Self> {
        let (mi_rows, mi_cols) = mi_dimensions;
        let mut field = ProjectedTemporalMotionField::new(mi_rows, mi_cols)?;
        let ref_order_hints = reference_order_hints(ref_frame_idx, ref_valid, ref_order_hint);
        let projections = projection_queue(
            mi_dimensions,
            current_order_hint,
            config,
            ref_frame_idx,
            &ref_order_hints,
            ref_motion_fields,
        );
        let mut trajectories = if config.enable_trajectory {
            Some(TrajectoryState::new(
                mi_dimensions,
                ref_order_hints.len(),
                config.step,
            )?)
        } else {
            None
        };
        for projection in projections {
            let slot = *ref_frame_idx.get(projection.ref_index)?;
            let source_order_hint = ref_order_hints
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
                projection.ref_index,
                projection.side,
                projection.target_ref,
                &ref_order_hints,
                trajectories.as_mut(),
                &mut field,
            );
        }
        if let Some(trajectories) = trajectories.as_mut() {
            trajectories.fill_gaps();
        }
        Some(Self {
            current_order_hint,
            ref_order_hints,
            field,
            trajectories: trajectories.map(TrajectoryState::into_fields),
            tip: None,
        })
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
        let mut field = ProjectedTemporalMotionField {
            width8: self.field.width8,
            height8: self.field.height8,
            cells: vec![ProjectedTemporalMotionCell::default(); self.field.cells.len()],
        };
        for y8 in (0..field.height8).step_by(projection_step) {
            for x8 in (0..field.width8).step_by(projection_step) {
                field.set(y8, x8, Mv::ZERO, references.ref_offset, false);
                let Some(source) = self.field.cell(y8, x8) else {
                    continue;
                };
                if source.valid
                    && let Some(mv) =
                        project_mv(source.mv, references.ref_offset, source.ref_offset)
                {
                    field.set(y8, x8, mv, references.ref_offset, true);
                }
            }
        }
        if fill_holes {
            fill_tip_holes(&mut field, projection_step, tmvp_unit_size8);
            average_tip_motion(&mut field, projection_step, tmvp_unit_size8);
        }
        fill_temporal_sampling_gaps(&mut field, projection_step, tmvp_unit_size8);
        self.field = field.clone();
        self.tip = Some(TipMotionField { field, references });
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

    pub(crate) fn tip_references(&self) -> Option<TipReferencePair> {
        Some(self.tip.as_ref()?.references)
    }

    pub(crate) fn tip_candidate(&self, y8: usize, x8: usize, base_mv: Mv) -> Option<[Mv; 2]> {
        Some(self.tip.as_ref()?.candidate(y8, x8, base_mv))
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
        let shift = 1 + usize::from(cell.tip_size_16x16);
        let row = cell.base_r + ((row.checked_sub(cell.base_r)? >> shift) << shift);
        let col = cell.base_c + ((col.checked_sub(cell.base_c)? >> shift) << shift);
        let base_cell = grid.get(row as i32, col as i32)?;
        self.tip_candidate(row >> 1, col >> 1, base_cell.sub_mv)
    }

    pub(super) fn tip_spatial_single_candidates(
        &self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
        probe: RelativeProbe,
        cell: NeighbourCell,
    ) -> Option<(Mv, (i8, Mv))> {
        let refs = self.tip_references()?;
        let references = [refs.past_ref, refs.future_ref];
        let mvs = self.tip_spatial_mvs(grid, block, probe, cell)?;
        let target = references
            .iter()
            .position(|&reference| reference == block.ref_frame0)?;
        let other = 1 - target;
        Some((mvs[target], (references[other], mvs[other])))
    }

    pub(super) fn motion_field_mv(&self, ref_frame: i8, y8: usize, x8: usize) -> Option<Mv> {
        let ref_index = usize::try_from(ref_frame).ok()?;
        if let Some(mv) = self
            .trajectories
            .as_ref()
            .and_then(|fields| fields.get(ref_index))
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
        if let Some(fields) = self.trajectories.as_ref()
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

    pub(super) fn derive_compound_spatial_mvs(
        &self,
        dst_refs: [i8; 2],
        candidate_ref: i8,
        candidate_mv: Mv,
        y8: usize,
        x8: usize,
    ) -> Option<[Mv; 2]> {
        let fields = self.trajectories.as_ref()?;
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
    let mut closest_past: Option<(usize, i32, i32)> = None;
    let mut closest_future: Option<(usize, i32, i32)> = None;
    for (index, hint) in ref_order_hints.iter().copied().enumerate() {
        let Some(hint) = hint.and_then(|hint| i32::try_from(hint).ok()) else {
            continue;
        };
        let distance = super::super::get_relative_dist(current, hint);
        if distance > 0 && closest_past.is_none_or(|(_, old, _)| distance < old) {
            closest_past = Some((index, distance, hint));
        } else if distance < 0 && closest_future.is_none_or(|(_, old, _)| distance > old) {
            closest_future = Some((index, distance, hint));
        }
    }
    let (past_ref, past_offset, past_hint) = closest_past?;
    let (future_ref, future_offset, future_hint) = closest_future?;
    Some(TipReferencePair {
        past_ref: i8::try_from(past_ref).ok()?,
        future_ref: i8::try_from(future_ref).ok()?,
        past_offset,
        future_offset,
        ref_offset: super::super::get_relative_dist(future_hint, past_hint).min(MAX_FRAME_DISTANCE),
    })
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
                            && !field.cell(dst_y, dst_x).is_some_and(|cell| cell.valid)
                        {
                            field.set(dst_y, dst_x, source.mv, source.ref_offset, source.valid);
                        }
                    }
                }
            }
        }
    }
}

fn average_tip_motion(
    field: &mut ProjectedTemporalMotionField,
    step: usize,
    superblock_size8: usize,
) {
    let mut averaged = field.clone();
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
                            .filter(|cell| cell.valid)
                        else {
                            continue;
                        };
                        sum.row += cell.mv.row;
                        sum.col += cell.mv.col;
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
                            field.cell(y8, x8).map_or(0, |cell| cell.ref_offset),
                            true,
                        );
                    }
                }
            }
        }
    }
    *field = averaged;
}

#[doc = "AV2 § 7.10.4 Weight_Div_Mult motion-vector average."]
fn divide_tip_average(value: i32, count: usize) -> i32 {
    const WEIGHTS: [i64; 6] = [0, 65_536, 32_768, 21_845, 16_384, 13_107];
    round2_signed(i64::from(value) * WEIGHTS[count], 16) as i32
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
        2 => round2_signed(i64::from(value), 1) as i32,
        3 => round2_signed(i64::from(value) * 85, 8) as i32,
        _ => round2_signed(i64::from(value), 2) as i32,
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
    for y8 in (0..source.height8).step_by(projection_step) {
        for x8 in (0..source.width8).step_by(projection_step) {
            let Some(cell) = source.cell(y8, x8).filter(|cell| cell.is_valid()) else {
                continue;
            };
            let list = side;
            let Some(target_hint) = cell.ref_order_hints[list] else {
                continue;
            };
            let saved_target_hint = target_hint;
            let source_hint = i32::try_from(source_order_hint).unwrap_or(i32::MAX);
            let target_hint = i32::try_from(target_hint).unwrap_or(i32::MAX);
            let current_hint = i32::try_from(current_order_hint).unwrap_or(i32::MAX);
            let mut ref_offset = super::super::get_relative_dist(source_hint, target_hint);
            if ref_offset == 0 || ref_offset.abs() > MAX_FRAME_DISTANCE {
                continue;
            }
            if (side == 0 && ref_offset < 0) || (side == 1 && ref_offset > 0) {
                continue;
            }
            let mut source_to_current = super::super::get_relative_dist(source_hint, current_hint);
            if source_to_current.abs() > MAX_FRAME_DISTANCE {
                continue;
            }
            let mut mv = uncompress_tmvp_mv(cell.mvs[list]);
            let end_ref = mapped_reference(source_order_hint, saved_target_hint, ref_order_hints);
            if let Some(trajectories) = trajectories.as_deref_mut() {
                trajectories.observe_projection(
                    source_ref,
                    end_ref,
                    target_ref,
                    y8,
                    x8,
                    mv,
                    source_to_current,
                    ref_offset,
                    side == 1,
                );
            }
            if ref_offset < 0 {
                ref_offset = -ref_offset;
                source_to_current = -source_to_current;
                mv = Mv {
                    row: -mv.row,
                    col: -mv.col,
                };
            }
            let Some(projected_to_current) = project_mv(mv, source_to_current, ref_offset) else {
                continue;
            };
            let Some((pos_y8, pos_x8)) =
                sampled_temporal_position(y8, x8, projected_to_current, projection_step, output)
            else {
                continue;
            };
            let replace = output.cell(pos_y8, pos_x8).is_none_or(|cell| {
                !cell.valid
                    || (target_order_hint == Some(saved_target_hint)
                        && cell.ref_offset != ref_offset)
            });
            if replace {
                output.set(pos_y8, pos_x8, mv, ref_offset, true);
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
    field: &ProjectedTemporalMotionField,
) -> Option<(usize, usize)> {
    let y8 = project_no_constraint(y8, projected_mv.row, field.height8)?;
    let x8 = project_no_constraint(x8, projected_mv.col, field.width8)?;
    Some((
        y8 / projection_step * projection_step,
        x8 / projection_step * projection_step,
    ))
}

fn project_no_constraint(v8: usize, delta: i32, max8: usize) -> Option<usize> {
    let offset8 = delta / (1 << (3 + 1 + 2));
    let projected = i32::try_from(v8).ok()?.checked_add(offset8)?;
    usize::try_from(projected)
        .ok()
        .filter(|&projected| projected < max8)
}

fn project_mv(mv: Mv, numerator: i32, denominator: i32) -> Option<Mv> {
    let denominator = denominator.clamp(1, MAX_FRAME_DISTANCE) as usize;
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let scale = DIV_MULT.get(denominator).copied()?;
    let bound = (1i64 << 16) - 1;
    let row = round2_signed(i64::from(mv.row) * i64::from(numerator) * scale, 14)
        .clamp(-bound, bound) as i32;
    let col = round2_signed(i64::from(mv.col) * i64::from(numerator) * scale, 14)
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

fn compress_tmvp_mv(mv: Mv) -> Mv {
    Mv {
        row: compress_tmvp_component(mv.row),
        col: compress_tmvp_component(mv.col),
    }
}

fn uncompress_tmvp_mv(mv: Mv) -> Mv {
    Mv {
        row: uncompress_tmvp_component(mv.row),
        col: uncompress_tmvp_component(mv.col),
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
            trajectories: None,
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
        context.trajectories = Some(vec![candidate, dst]);

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
        context.trajectories = Some(vec![candidate, dst0, dst1]);

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
            assert_eq!(cell.ref_order_hints, [Some(7); 2]);
            assert_eq!(cell.mvs, [Mv { row: 8, col: -12 }; 2]);
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
            assert_eq!(cell.ref_order_hints, expected);
            let swapped = input != expected;
            assert_eq!(cell.mvs[0].row, if swapped { 24 } else { 8 });
            assert_eq!(cell.mvs[1].row, if swapped { 8 } else { 24 });
        }
    }

    #[test]
    fn step_two_projection_samples_and_stores_on_the_even_grid() {
        let mut source = TemporalMotionField::new(8, 8).unwrap();
        for x8 in 0..source.width8 {
            source.cells[x8] = TemporalMotionCell {
                ref_order_hints: [Some(0), None],
                mvs: [compress_tmvp_mv(Mv { row: 0, col: -64 }), Mv::ZERO],
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
                enable_tip: false,
                enable_trajectory: false,
                reduced: false,
            },
            &[0, 1],
            &[true, true],
            &[1, 3],
            &[Some(source), Some(other)],
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
    fn sampling_gap_does_not_average_across_tmvp_units() {
        let mut context = tip_context(2, vec![Some(0)], 2, 36);
        context.field.set(0, 14, Mv { row: 8, col: 16 }, 2, true);
        context.field.set(0, 16, Mv { row: 24, col: 80 }, 4, true);

        context.fill_sampling_gaps(2, 16);

        assert_eq!(context.field.cell(0, 15), context.field.cell(0, 14));
        assert_eq!(context.field.cell(0, 17), context.field.cell(0, 16));
    }

    #[test]
    fn tip_projection_fills_unsampled_units_and_adds_the_block_mv() {
        let mut context = tip_context(10, vec![Some(8), Some(12)], 4, 4);
        context.field.set(0, 0, Mv { row: 8, col: -16 }, 4, true);

        assert!(context.prepare_tip(2, 2, false));
        assert_eq!(context.tip_references(), context.tip_reference_pair());
        assert_eq!(context.field, context.tip.as_ref().unwrap().field);
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
        let cell = context.tip.as_ref().unwrap().field.cell(0, 11).unwrap();
        assert_eq!(
            cell,
            ProjectedTemporalMotionCell {
                valid: true,
                mv: Mv { row: 18, col: -36 },
                ref_offset: 9,
            }
        );
    }
}
