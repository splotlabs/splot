// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed;

use super::{Mv, REFMVS_LIMIT, allocate_temporal_grid, project_mv, temporal_grid_index};

type Position = (usize, usize);
type PhasePositions = [Option<Position>; 3];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TrajectoryMotionField {
    width8: usize,
    height8: usize,
    cells: Vec<Option<Mv>>,
}

impl TrajectoryMotionField {
    fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
        })
    }

    fn index(&self, y8: usize, x8: usize) -> Option<usize> {
        temporal_grid_index(self.width8, self.height8, y8, x8)
    }

    pub(super) fn cell(&self, y8: usize, x8: usize) -> Option<Mv> {
        self.cells.get(self.index(y8, x8)?).copied().flatten()
    }

    fn set(&mut self, y8: usize, x8: usize, mv: Mv) {
        let Some(index) = self.index(y8, x8) else {
            return;
        };
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = Some(mv);
        }
    }
}

pub(super) struct TrajectoryState {
    fields: Vec<TrajectoryMotionField>,
    positions: Vec<Vec<PhasePositions>>,
    projection_offsets: Vec<Option<i32>>,
    step: usize,
    unit_size8: usize,
}

impl TrajectoryState {
    pub(super) fn new(
        mi_dimensions: (usize, usize),
        reference_count: usize,
        step: usize,
    ) -> Option<Self> {
        let template = TrajectoryMotionField::new(mi_dimensions.0, mi_dimensions.1)?;
        let cell_count = template.cells.len();
        let step = step.clamp(1, 2);
        Some(Self {
            fields: vec![template; reference_count],
            positions: vec![vec![[None; 3]; cell_count]; reference_count],
            projection_offsets: vec![None; cell_count],
            step,
            unit_size8: if step == 1 { 8 } else { 16 },
        })
    }

    pub(super) fn into_fields(self) -> Vec<TrajectoryMotionField> {
        self.fields
    }

    fn dimensions(&self) -> Option<(usize, usize)> {
        self.fields
            .first()
            .map(|field| (field.height8, field.width8))
    }

    fn position(
        &self,
        reference: usize,
        y8: usize,
        x8: usize,
        phase: usize,
    ) -> Option<(usize, usize)> {
        let index = self.fields.first()?.index(y8, x8)?;
        self.positions.get(reference)?.get(index)?[phase]
    }

    fn set_position(
        &mut self,
        reference: usize,
        y8: usize,
        x8: usize,
        phase: usize,
        position: (usize, usize),
    ) {
        let Some(index) = self.fields.first().and_then(|field| field.index(y8, x8)) else {
            return;
        };
        if let Some(cell) = self
            .positions
            .get_mut(reference)
            .and_then(|field| field.get_mut(index))
        {
            cell[phase] = Some(position);
        }
    }

    fn phase(&self, x8: usize) -> usize {
        x8 / self.unit_size8 % 3
    }

    fn position_allowed(&self, candidate: (usize, usize), base: (usize, usize)) -> bool {
        let Some((height8, width8)) = self.dimensions() else {
            return false;
        };
        if candidate.0 >= height8 || candidate.1 >= width8 {
            return false;
        }
        let base_y = base.0 / self.unit_size8 * self.unit_size8;
        let base_x = base.1 / self.unit_size8 * self.unit_size8;
        let col_offset = if self.step == 1 {
            self.unit_size8 / 2
        } else {
            self.unit_size8
        };
        candidate.0 >= base_y
            && candidate.0 < base_y + self.unit_size8
            && candidate.1.saturating_add(col_offset) >= base_x
            && candidate.1 < base_x + self.unit_size8 + col_offset
    }

    fn sampled_position(&self, y8: usize, x8: usize, mv: Mv) -> Option<(usize, usize)> {
        let (height8, width8) = self.dimensions()?;
        let offset = |base: usize, delta: i32, limit: usize| {
            let delta8 = delta / (1 << 6);
            let projected = i32::try_from(base).ok()?.checked_add(delta8)?;
            usize::try_from(projected)
                .ok()
                .filter(|&projected| projected < limit)
        };
        let y8 = offset(y8, mv.row, height8)?;
        let x8 = offset(x8, mv.col, width8)?;
        Some((y8 / self.step * self.step, x8 / self.step * self.step))
    }

    fn set_field(&mut self, reference: usize, y8: usize, x8: usize, mv: Mv) {
        if let Some(field) = self.fields.get_mut(reference) {
            field.set(y8, x8, clamp_mv(mv));
        }
    }

    fn check_intersection(
        &mut self,
        source: usize,
        end: Option<usize>,
        y8: usize,
        x8: usize,
        mv: Mv,
    ) {
        let Some(end) = end.filter(|&end| end < self.fields.len()) else {
            return;
        };
        for phase in 0..3 {
            let Some(trajectory) = self.position(source, y8, x8, phase) else {
                continue;
            };
            if self.phase(trajectory.1) != phase
                || !self.position_allowed((y8, x8), trajectory)
                || self.fields[end].cell(trajectory.0, trajectory.1).is_some()
            {
                continue;
            }
            let Some(source_mv) = self.fields[source].cell(trajectory.0, trajectory.1) else {
                continue;
            };
            let end_mv = add_mv(source_mv, mv);
            self.set_field(end, trajectory.0, trajectory.1, end_mv);
            if let Some(position) = self
                .sampled_position(trajectory.0, trajectory.1, end_mv)
                .filter(|&position| self.position_allowed(position, trajectory))
            {
                self.set_position(end, position.0, position.1, phase, trajectory);
            }
        }

        let Some(end_position) = self.sampled_position(y8, x8, mv) else {
            return;
        };
        for phase in 0..3 {
            let Some(trajectory) = self.position(end, end_position.0, end_position.1, phase) else {
                continue;
            };
            if self.phase(trajectory.1) != phase
                || !self.position_allowed((y8, x8), trajectory)
                || !self.position_allowed(end_position, trajectory)
                || self.fields[source]
                    .cell(trajectory.0, trajectory.1)
                    .is_some()
            {
                continue;
            }
            let Some(end_mv) = self.fields[end].cell(trajectory.0, trajectory.1) else {
                continue;
            };
            let source_mv = subtract_mv(end_mv, mv);
            self.set_field(source, trajectory.0, trajectory.1, source_mv);
            if let Some(position) = self
                .sampled_position(trajectory.0, trajectory.1, source_mv)
                .filter(|&position| self.position_allowed(position, trajectory))
            {
                self.set_position(source, position.0, position.1, phase, trajectory);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn observe_projection(
        &mut self,
        source: usize,
        end: Option<usize>,
        target: Option<usize>,
        y8: usize,
        x8: usize,
        mv: Mv,
        source_to_current: i32,
        reference_offset: i32,
        backward: bool,
    ) {
        self.check_intersection(source, end, y8, x8, mv);
        let reference_offset = reference_offset.abs();
        if reference_offset == 0 || reference_offset > super::MAX_FRAME_DISTANCE {
            return;
        }
        let numerator = if backward {
            -source_to_current
        } else {
            source_to_current
        };
        let Some(projected) = project_mv(mv, numerator, reference_offset) else {
            return;
        };
        let Some(position) = self
            .sampled_position(y8, x8, projected)
            .filter(|&position| self.position_allowed(position, (y8, x8)))
        else {
            return;
        };
        let Some(index) = self
            .fields
            .first()
            .and_then(|field| field.index(position.0, position.1))
        else {
            return;
        };
        let replace = self.projection_offsets[index].is_none()
            || (target.is_some()
                && target == end
                && self.projection_offsets[index] != Some(reference_offset));
        if !replace {
            return;
        }
        self.projection_offsets[index] = Some(reference_offset);
        let phase = self.phase(position.1);
        self.set_position(source, y8, x8, phase, position);
        self.set_field(
            source,
            position.0,
            position.1,
            Mv {
                row: -projected.row,
                col: -projected.col,
            },
        );
        let Some(end) = end else {
            return;
        };
        let Some(end_mv) = project_mv(mv, reference_offset - numerator, reference_offset) else {
            return;
        };
        self.set_field(end, position.0, position.1, end_mv);
        if let Some(target_position) = self
            .sampled_position(y8, x8, mv)
            .filter(|&target_position| self.position_allowed(target_position, position))
        {
            self.set_position(end, target_position.0, target_position.1, phase, position);
        }
    }

    pub(super) fn fill_gaps(&mut self) {
        if self.step != 2 {
            return;
        }
        for field in &mut self.fields {
            fill_field_gaps(field, self.unit_size8);
        }
    }
}

fn add_mv(a: Mv, b: Mv) -> Mv {
    Mv {
        row: a.row.saturating_add(b.row),
        col: a.col.saturating_add(b.col),
    }
}

fn subtract_mv(a: Mv, b: Mv) -> Mv {
    Mv {
        row: a.row.saturating_sub(b.row),
        col: a.col.saturating_sub(b.col),
    }
}

fn clamp_mv(mv: Mv) -> Mv {
    Mv {
        row: mv.row.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
        col: mv.col.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
    }
}

fn fill_field_gaps(field: &mut TrajectoryMotionField, unit_size8: usize) {
    for y8 in (0..field.height8).step_by(2) {
        for x8 in (0..field.width8).step_by(2) {
            let Some(anchor) = field.cell(y8, x8) else {
                continue;
            };
            for (dy, dx) in [(0usize, 1usize), (1, 0), (1, 1)] {
                if y8 + dy >= field.height8 || x8 + dx >= field.width8 {
                    continue;
                }
                let mut sum = anchor;
                let mut count = 1;
                for (source_y, source_x, enabled) in [
                    (y8, x8 + 2, dx > 0),
                    (y8 + 2, x8, dy > 0),
                    (y8 + 2, x8 + 2, dy > 0 && dx > 0),
                ] {
                    if !enabled
                        || source_y >= field.height8
                        || source_x >= field.width8
                        || source_y / unit_size8 != y8 / unit_size8
                        || source_x / unit_size8 != x8 / unit_size8
                    {
                        continue;
                    }
                    if let Some(mv) = field.cell(source_y, source_x) {
                        sum = add_mv(sum, mv);
                        count += 1;
                    }
                }
                field.set(
                    y8 + dy,
                    x8 + dx,
                    Mv {
                        row: average(sum.row, count),
                        col: average(sum.col, count),
                    },
                );
            }
        }
    }
}

fn average(value: i32, count: i32) -> i32 {
    match count {
        1 => value,
        2 => round2_signed(i64::from(value), 1) as i32,
        3 => round2_signed(i64::from(value) * 85, 8) as i32,
        _ => round2_signed(i64::from(value), 2) as i32,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn direct_projection_records_reference_specific_trajectories() {
        let mut state = TrajectoryState::new((8, 8), 2, 1).unwrap();
        state.observe_projection(1, Some(0), None, 1, 1, Mv { row: 16, col: 32 }, 2, 4, false);

        assert_eq!(state.fields[1].cell(1, 1), Some(Mv { row: -8, col: -16 }));
        assert_eq!(state.fields[0].cell(1, 1), Some(Mv { row: 8, col: 16 }));
    }

    #[test]
    fn intersecting_projection_extends_the_reference_path() {
        let mut state = TrajectoryState::new((8, 8), 3, 1).unwrap();
        state.observe_projection(0, Some(1), None, 1, 1, Mv { row: 0, col: 64 }, 2, 4, false);
        state.observe_projection(1, Some(2), None, 1, 2, Mv { row: 0, col: 64 }, 2, 4, false);

        assert_eq!(state.fields[2].cell(1, 1), Some(Mv { row: 0, col: 96 }));
    }

    #[test]
    fn trajectory_gap_fill_stays_within_tmvp_units() {
        let mut field = TrajectoryMotionField::new(2, 36).unwrap();
        field.set(0, 14, Mv { row: 8, col: 16 });
        field.set(0, 16, Mv { row: 24, col: 80 });

        fill_field_gaps(&mut field, 16);

        assert_eq!(field.cell(0, 15), field.cell(0, 14));
        assert_eq!(field.cell(0, 17), field.cell(0, 16));
    }
}
