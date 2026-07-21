// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed_i32;

use super::{Mv, REFMVS_LIMIT, allocate_temporal_grid, project_mv, temporal_grid_index};

type Position = (usize, usize);
type PhasePositions = [PackedPosition; 3];

const INVALID_PROJECTION_OFFSET: i32 = i32::MIN;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PackedTrajectoryMv {
    row: i16,
    col: i16,
}

impl Default for PackedTrajectoryMv {
    fn default() -> Self {
        Self::INVALID
    }
}

impl PackedTrajectoryMv {
    const INVALID: Self = Self {
        row: i16::MIN,
        col: 0,
    };

    fn new(mv: Mv) -> Self {
        Self {
            row: mv.row.clamp(-REFMVS_LIMIT, REFMVS_LIMIT) as i16,
            col: mv.col.clamp(-REFMVS_LIMIT, REFMVS_LIMIT) as i16,
        }
    }

    fn unpack(self) -> Option<Mv> {
        (self.row != Self::INVALID.row).then_some(Mv {
            row: i32::from(self.row),
            col: i32::from(self.col),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PackedPosition {
    y: u16,
    x: u16,
}

impl PackedPosition {
    const INVALID: Self = Self {
        y: u16::MAX,
        x: u16::MAX,
    };

    fn new(position: Position) -> Option<Self> {
        Some(Self {
            y: u16::try_from(position.0).ok()?,
            x: u16::try_from(position.1).ok()?,
        })
    }

    fn unpack(self) -> Option<Position> {
        (self.y != Self::INVALID.y).then_some((self.y as usize, self.x as usize))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TrajectoryMotionField {
    width8: usize,
    height8: usize,
    pub(super) cells: Vec<PackedTrajectoryMv>,
}

impl TrajectoryMotionField {
    pub(super) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, mut cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        cells.fill(PackedTrajectoryMv::INVALID);
        Some(Self {
            width8,
            height8,
            cells,
        })
    }

    fn reset(&mut self, mi_rows: usize, mi_cols: usize) -> Option<()> {
        let width8 = mi_cols.div_ceil(2);
        let height8 = mi_rows.div_ceil(2);
        let cells = width8.checked_mul(height8)?;
        self.width8 = width8;
        self.height8 = height8;
        self.cells.resize(cells, PackedTrajectoryMv::INVALID);
        self.cells.fill(PackedTrajectoryMv::INVALID);
        Some(())
    }

    fn index(&self, y8: usize, x8: usize) -> Option<usize> {
        temporal_grid_index(self.width8, self.height8, y8, x8)
    }

    pub(super) fn cell(&self, y8: usize, x8: usize) -> Option<Mv> {
        self.cell_at(self.index(y8, x8)?)
    }

    fn cell_at(&self, index: usize) -> Option<Mv> {
        self.cells.get(index).copied()?.unpack()
    }

    pub(super) fn set(&mut self, y8: usize, x8: usize, mv: Mv) {
        let Some(index) = self.index(y8, x8) else {
            return;
        };
        self.set_at(index, mv);
    }

    fn set_at(&mut self, index: usize, mv: Mv) {
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = PackedTrajectoryMv::new(mv);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TrajectoryState {
    pub(super) fields: Vec<TrajectoryMotionField>,
    pub(super) positions: Vec<Vec<PhasePositions>>,
    pub(super) projection_offsets: Vec<i32>,
    step: usize,
    unit_size8: usize,
    unit_shift: Option<u32>,
    col_offset: usize,
    width8: usize,
    height8: usize,
}

impl TrajectoryState {
    pub(super) fn new(
        mi_dimensions: (usize, usize),
        reference_count: usize,
        step: usize,
        unit_size8: usize,
    ) -> Option<Self> {
        let template = TrajectoryMotionField::new(mi_dimensions.0, mi_dimensions.1)?;
        let cell_count = template.cells.len();
        let step = step.clamp(1, 2);
        let fields = vec![template; reference_count];
        let (width8, height8) = fields
            .first()
            .map_or((0, 0), |field| (field.width8, field.height8));
        Some(Self {
            fields,
            positions: vec![vec![[PackedPosition::INVALID; 3]; cell_count]; reference_count],
            projection_offsets: vec![INVALID_PROJECTION_OFFSET; cell_count],
            step,
            unit_size8: unit_size8.max(1),
            unit_shift: Self::unit_shift(unit_size8.max(1)),
            col_offset: Self::col_offset(step, unit_size8.max(1)),
            width8,
            height8,
        })
    }

    pub(super) fn reset(
        &mut self,
        mi_dimensions: (usize, usize),
        reference_count: usize,
        step: usize,
        unit_size8: usize,
    ) -> Option<()> {
        let width8 = mi_dimensions.1.div_ceil(2);
        let height8 = mi_dimensions.0.div_ceil(2);
        let cell_count = width8.checked_mul(height8)?;
        self.fields
            .resize_with(reference_count, || TrajectoryMotionField {
                width8: 0,
                height8: 0,
                cells: Vec::new(),
            });
        for field in &mut self.fields {
            field.reset(mi_dimensions.0, mi_dimensions.1)?;
        }
        self.positions.resize_with(reference_count, Vec::new);
        for positions in &mut self.positions {
            positions.resize(cell_count, [PackedPosition::INVALID; 3]);
            positions.fill([PackedPosition::INVALID; 3]);
        }
        self.projection_offsets
            .resize(cell_count, INVALID_PROJECTION_OFFSET);
        self.projection_offsets.fill(INVALID_PROJECTION_OFFSET);
        self.step = step.clamp(1, 2);
        self.unit_size8 = unit_size8.max(1);
        self.unit_shift = Self::unit_shift(self.unit_size8);
        self.col_offset = Self::col_offset(self.step, self.unit_size8);
        (self.width8, self.height8) = self
            .fields
            .first()
            .map_or((0, 0), |field| (field.width8, field.height8));
        Some(())
    }

    #[cfg(test)]
    pub(super) fn from_fields(fields: Vec<TrajectoryMotionField>) -> Self {
        let (width8, height8) = fields
            .first()
            .map_or((0, 0), |field| (field.width8, field.height8));
        Self {
            fields,
            positions: Vec::new(),
            projection_offsets: Vec::new(),
            step: 1,
            unit_size8: 1,
            unit_shift: Some(0),
            col_offset: 0,
            width8,
            height8,
        }
    }

    #[cfg(test)]
    pub(super) fn into_fields(self) -> Vec<TrajectoryMotionField> {
        self.fields
    }

    pub(super) fn fields(&self) -> &[TrajectoryMotionField] {
        &self.fields
    }

    fn grid_index(&self, y8: usize, x8: usize) -> Option<usize> {
        temporal_grid_index(self.width8, self.height8, y8, x8)
    }

    fn positions_at(&self, reference: usize, index: usize) -> Option<PhasePositions> {
        self.positions.get(reference)?.get(index).copied()
    }

    fn set_position_at(
        &mut self,
        reference: usize,
        index: usize,
        phase: usize,
        position: Position,
    ) {
        if let Some(cell) = self
            .positions
            .get_mut(reference)
            .and_then(|field| field.get_mut(index))
            && let Some(position) = PackedPosition::new(position)
        {
            cell[phase] = position;
        }
    }

    fn div_unit(value: usize, unit: usize) -> usize {
        if unit.is_power_of_two() {
            value >> unit.trailing_zeros()
        } else {
            value / unit
        }
    }

    fn unit_shift(unit_size8: usize) -> Option<u32> {
        unit_size8
            .is_power_of_two()
            .then(|| unit_size8.trailing_zeros())
    }

    fn col_offset(step: usize, unit_size8: usize) -> usize {
        if step == 1 {
            unit_size8 / 2
        } else {
            unit_size8
        }
    }

    fn div_unit_size(&self, value: usize) -> usize {
        match self.unit_shift {
            Some(shift) => value >> shift,
            None => value / self.unit_size8,
        }
    }

    fn phase(&self, x8: usize) -> usize {
        self.div_unit_size(x8) % 3
    }

    fn unit_base(&self, value: usize) -> usize {
        self.div_unit_size(value) * self.unit_size8
    }

    fn round_step(&self, value: usize) -> usize {
        Self::div_unit(value, self.step) * self.step
    }

    fn position_allowed(&self, candidate: (usize, usize), base: (usize, usize)) -> bool {
        if candidate.0 >= self.height8 || candidate.1 >= self.width8 {
            return false;
        }
        let base_y = self.unit_base(base.0);
        let base_x = self.unit_base(base.1);
        let col_offset = self.col_offset;
        candidate.0 >= base_y
            && candidate.0 < base_y + self.unit_size8
            && candidate.1.saturating_add(col_offset) >= base_x
            && candidate.1 < base_x + self.unit_size8 + col_offset
    }

    fn sampled_position(&self, y8: usize, x8: usize, mv: Mv) -> Option<(usize, usize)> {
        let offset = |base: usize, delta: i32, limit: usize| {
            let delta8 = delta / (1 << 6);
            let projected = i32::try_from(base).ok()?.checked_add(delta8)?;
            usize::try_from(projected)
                .ok()
                .filter(|&projected| projected < limit)
        };
        let y8 = offset(y8, mv.row, self.height8)?;
        let x8 = offset(x8, mv.col, self.width8)?;
        Some((self.round_step(y8), self.round_step(x8)))
    }

    fn set_field_at(&mut self, reference: usize, index: usize, mv: Mv) {
        if let Some(field) = self.fields.get_mut(reference) {
            field.set_at(index, mv);
        }
    }

    fn set_position(&mut self, reference: usize, at: Position, phase: usize, position: Position) {
        if let Some(index) = self.grid_index(at.0, at.1) {
            self.set_position_at(reference, index, phase, position);
        }
    }

    pub(super) fn check_intersection(
        &mut self,
        source: usize,
        end: Option<usize>,
        y8: usize,
        x8: usize,
        mv: Mv,
    ) {
        let Some(end) = end.filter(|&end| end < self.fields.len() && source < self.fields.len())
        else {
            return;
        };
        if let Some(start_positions) = self
            .grid_index(y8, x8)
            .and_then(|index| self.positions_at(source, index))
        {
            for (phase, position) in start_positions.into_iter().enumerate() {
                let Some(trajectory) = position.unpack() else {
                    continue;
                };
                if !self.position_allowed((y8, x8), trajectory) {
                    continue;
                }
                let Some(traj_index) = self.grid_index(trajectory.0, trajectory.1) else {
                    continue;
                };
                if self.fields[end].cell_at(traj_index).is_some() {
                    continue;
                }
                let Some(source_mv) = self.fields[source].cell_at(traj_index) else {
                    continue;
                };
                let end_mv = add_mv(source_mv, mv);
                self.fields[end].set_at(traj_index, end_mv);
                if let Some(position) = self
                    .sampled_position(trajectory.0, trajectory.1, end_mv)
                    .filter(|&position| self.position_allowed(position, trajectory))
                {
                    self.set_position(end, position, phase, trajectory);
                }
            }
        }

        let Some(end_position) = self.sampled_position(y8, x8, mv) else {
            return;
        };
        let Some(end_index) = self.grid_index(end_position.0, end_position.1) else {
            return;
        };
        let Some(end_positions) = self.positions_at(end, end_index) else {
            return;
        };
        for (phase, position) in end_positions.into_iter().enumerate() {
            let Some(trajectory) = position.unpack() else {
                continue;
            };
            if !self.position_allowed((y8, x8), trajectory)
                || !self.position_allowed(end_position, trajectory)
            {
                continue;
            }
            let Some(traj_index) = self.grid_index(trajectory.0, trajectory.1) else {
                continue;
            };
            if self.fields[source].cell_at(traj_index).is_some() {
                continue;
            }
            let Some(end_mv) = self.fields[end].cell_at(traj_index) else {
                continue;
            };
            let source_mv = subtract_mv(end_mv, mv);
            self.fields[source].set_at(traj_index, source_mv);
            if let Some(position) = self
                .sampled_position(trajectory.0, trajectory.1, source_mv)
                .filter(|&position| self.position_allowed(position, trajectory))
            {
                self.set_position(source, position, phase, trajectory);
            }
        }
    }

    #[cfg(test)]
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
        let reference_offset = reference_offset.abs();
        if reference_offset > super::MAX_FRAME_DISTANCE {
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
        self.observe_projection_with_projected(
            source,
            end,
            target,
            y8,
            x8,
            mv,
            projected,
            source_to_current,
            reference_offset,
            backward,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn observe_projection_with_projected(
        &mut self,
        source: usize,
        end: Option<usize>,
        target: Option<usize>,
        y8: usize,
        x8: usize,
        mv: Mv,
        projected: Mv,
        source_to_current: i32,
        reference_offset: i32,
        backward: bool,
    ) {
        let numerator = if backward {
            -source_to_current
        } else {
            source_to_current
        };
        let Some(position) = self
            .sampled_position(y8, x8, projected)
            .filter(|&position| self.position_allowed((y8, x8), position))
        else {
            return;
        };
        let Some(index) = self.grid_index(position.0, position.1) else {
            return;
        };
        let Some(recorded_offset) = self.projection_offsets.get_mut(index) else {
            return;
        };
        let replace = *recorded_offset == INVALID_PROJECTION_OFFSET
            || (target.is_some() && target == end && *recorded_offset != reference_offset);
        if !replace {
            return;
        }
        *recorded_offset = reference_offset;
        let phase = self.phase(position.1);
        self.set_position(source, (y8, x8), phase, position);
        self.set_field_at(
            source,
            index,
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
        self.set_field_at(end, index, end_mv);
        if let Some(target_position) = self
            .sampled_position(y8, x8, mv)
            .filter(|&target_position| self.position_allowed(target_position, position))
        {
            self.set_position(end, target_position, phase, position);
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
        2 => round2_signed_i32(value, 1),
        3 => round2_signed_i32(value * 85, 8),
        _ => round2_signed_i32(value, 2),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn trajectory_storage_stays_compact() {
        assert_eq!(std::mem::size_of::<Mv>(), 8);
        assert_eq!(std::mem::size_of::<PackedTrajectoryMv>(), 4);
        assert_eq!(std::mem::size_of::<PackedPosition>(), 4);
        assert_eq!(std::mem::size_of::<PhasePositions>(), 12);
        assert_eq!(std::mem::size_of::<i32>(), 4);
        assert_eq!(PackedTrajectoryMv::INVALID.unpack(), None);
        assert_eq!(PackedPosition::INVALID.unpack(), None);
        assert_eq!(
            PackedPosition::new((8191, 8191)).and_then(PackedPosition::unpack),
            Some((8191, 8191))
        );
    }

    #[test]
    fn direct_projection_records_reference_specific_trajectories() {
        let mut state = TrajectoryState::new((8, 8), 2, 1, 8).unwrap();
        state.observe_projection(1, Some(0), None, 1, 1, Mv { row: 16, col: 32 }, 2, 4, false);

        assert_eq!(state.fields[1].cell(1, 1), Some(Mv { row: -8, col: -16 }));
        assert_eq!(state.fields[0].cell(1, 1), Some(Mv { row: 8, col: 16 }));
    }

    #[test]
    fn zero_offset_projection_records_the_source_trajectory() {
        let mut state = TrajectoryState::new((8, 8), 1, 1, 8).unwrap();

        state.observe_projection(0, None, None, 1, 1, Mv::ZERO, 2, 0, false);

        assert_eq!(state.fields[0].cell(1, 1), Some(Mv::ZERO));
    }

    #[test]
    fn direct_projection_checks_source_against_destination_unit() {
        let mut state = TrajectoryState::new((4, 32), 2, 1, 8).unwrap();
        state.observe_projection(
            0,
            Some(1),
            None,
            1,
            7,
            Mv { row: 0, col: 1120 },
            2,
            5,
            false,
        );

        assert_eq!(state.fields[0].cell(1, 14), Some(Mv { row: 0, col: -448 }));
        assert_eq!(state.fields[1].cell(1, 14), Some(Mv { row: 0, col: 672 }));
    }

    #[test]
    fn intersecting_projection_extends_the_reference_path() {
        let mut state = TrajectoryState::new((8, 8), 3, 1, 8).unwrap();
        state.observe_projection(0, Some(1), None, 1, 1, Mv { row: 0, col: 64 }, 2, 4, false);
        state.check_intersection(1, Some(2), 1, 2, Mv { row: 0, col: 64 });

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

    #[test]
    fn step_two_trajectory_uses_64_pixel_superblock_units() {
        let mut state = TrajectoryState::new((2, 36), 1, 2, 8).unwrap();
        state.fields[0].set(0, 6, Mv { row: 8, col: 16 });
        state.fields[0].set(0, 8, Mv { row: 24, col: 80 });

        state.fill_gaps();

        assert_eq!(state.fields[0].cell(0, 7), state.fields[0].cell(0, 6));
        assert_eq!(state.fields[0].cell(0, 9), state.fields[0].cell(0, 8));
    }
}
