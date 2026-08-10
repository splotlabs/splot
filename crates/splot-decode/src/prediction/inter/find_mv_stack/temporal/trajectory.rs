// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed_i32;

use super::{
    Mv, REFMVS_LIMIT, allocate_temporal_grid, project_mv, project_no_constraint,
    temporal_grid_index,
};

type Position = (usize, usize);
type PhasePositions = [PackedPosition; 3];

const INVALID_PROJECTION_OFFSET: i32 = i32::MIN;
const MAX_TRAJECTORY_REFERENCES: usize = 7;
pub(super) const INVALID_TRAJECTORY_MV: Mv = Mv {
    row: i32::MIN,
    col: 0,
};

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
        let mv = clamp_mv(mv);
        Self {
            row: mv.row as i16,
            col: mv.col as i16,
        }
    }

    fn unpack(self) -> Option<Mv> {
        (self != Self::INVALID).then_some(Mv {
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

    #[cfg(test)]
    fn unpack(self) -> Option<Position> {
        (self != Self::INVALID).then_some((self.y as usize, self.x as usize))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TrajectoryPositions {
    phases: PhasePositions,
    mask: u8,
}

impl TrajectoryPositions {
    const EMPTY: Self = Self {
        phases: [PackedPosition::INVALID; 3],
        mask: 0,
    };
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
        self.cells
            .get(index)
            .copied()
            .and_then(PackedTrajectoryMv::unpack)
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
    pub(super) positions: Vec<Vec<TrajectoryPositions>>,
    pub(super) projection_offsets: Vec<i32>,
    step: usize,
    unit_size8: usize,
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
        let unit_size8 = unit_size8.max(1);
        let fields = vec![template; reference_count];
        let (width8, height8) = fields
            .first()
            .map_or((0, 0), |field| (field.width8, field.height8));
        Some(Self {
            fields,
            positions: vec![vec![TrajectoryPositions::EMPTY; cell_count]; reference_count],
            projection_offsets: vec![INVALID_PROJECTION_OFFSET; cell_count],
            step,
            unit_size8,
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
            positions.resize(cell_count, TrajectoryPositions::EMPTY);
            positions.fill(TrajectoryPositions::EMPTY);
        }
        self.projection_offsets
            .resize(cell_count, INVALID_PROJECTION_OFFSET);
        self.projection_offsets.fill(INVALID_PROJECTION_OFFSET);
        self.step = step.clamp(1, 2);
        self.unit_size8 = unit_size8.max(1);
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

    /// Splits every trajectory grid into `band_rows`-tall row bands.
    ///
    /// Bands are unit-aligned, and AV2 § 7.9.8 keeps each projected sample in
    /// the TMVP unit row of the position it was sampled from, so the bands
    /// partition every write the § 7.9.3 scan makes. Returns `None` when a grid
    /// is not sized to this state's geometry, leaving the caller whole-field.
    pub(super) fn bands(&mut self, band_rows: usize) -> Option<Vec<TrajectoryBand<'_>>> {
        let Self {
            fields,
            positions,
            projection_offsets,
            step,
            unit_size8,
            width8,
            height8,
        } = self;
        let (width8, height8) = (*width8, *height8);
        let step_mask = *step - 1;
        let unit_mask = *unit_size8 - 1;
        let unit_shift = unit_size8.trailing_zeros();
        let total = width8.checked_mul(height8)?;
        let stride = band_rows.checked_mul(width8)?;
        if band_rows == 0
            || fields.len() > MAX_TRAJECTORY_REFERENCES
            || positions.len() > MAX_TRAJECTORY_REFERENCES
            || projection_offsets.len() != total
            || fields.iter().any(|field| field.cells.len() != total)
            || positions.iter().any(|slots| slots.len() != total)
        {
            return None;
        }
        let mut bands = projection_offsets
            .chunks_mut(stride.max(1))
            .enumerate()
            .map(|(index, projection_offsets)| TrajectoryBand {
                fields: BandSlices::new(),
                positions: BandSlices::new(),
                projection_offsets,
                row_base: index * band_rows,
                step: *step,
                step_mask,
                unit_size8: *unit_size8,
                unit_mask,
                unit_shift,
                width8,
                height8,
            })
            .collect::<Vec<_>>();
        for field in fields.iter_mut() {
            for (band, cells) in bands.iter_mut().zip(field.cells.chunks_mut(stride.max(1))) {
                band.fields.push(cells)?;
            }
        }
        for slots in positions.iter_mut() {
            for (band, slots) in bands.iter_mut().zip(slots.chunks_mut(stride.max(1))) {
                band.positions.push(slots)?;
            }
        }
        Some(bands)
    }

    #[cfg(test)]
    pub(super) fn whole_band(&mut self) -> Option<TrajectoryBand<'_>> {
        let height8 = self.height8;
        self.bands(height8).and_then(|mut bands| bands.pop())
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

/// One unit-aligned row band of a [`TrajectoryState`].
///
/// Geometry stays whole-frame — projections are sampled against the full grid
/// — while the storage slices cover `row_base .. row_base + band_rows` only.
pub(super) struct TrajectoryBand<'a> {
    fields: BandSlices<'a, PackedTrajectoryMv>,
    positions: BandSlices<'a, TrajectoryPositions>,
    projection_offsets: &'a mut [i32],
    row_base: usize,
    step: usize,
    step_mask: usize,
    unit_size8: usize,
    unit_mask: usize,
    unit_shift: u32,
    width8: usize,
    height8: usize,
}

struct BandSlices<'a, T> {
    slots: [Option<&'a mut [T]>; MAX_TRAJECTORY_REFERENCES],
    len: usize,
}

impl<'a, T> BandSlices<'a, T> {
    fn new() -> Self {
        Self {
            slots: core::array::from_fn(|_| None),
            len: 0,
        }
    }

    fn from_chunks(cells: &'a mut [T], chunk_size: usize) -> Option<Self> {
        let mut slices = Self::new();
        for chunk in cells.chunks_mut(chunk_size.max(1)) {
            slices.push(chunk)?;
        }
        Some(slices)
    }

    fn push(&mut self, cells: &'a mut [T]) -> Option<()> {
        *self.slots.get_mut(self.len)? = Some(cells);
        self.len += 1;
        Some(())
    }

    fn len(&self) -> usize {
        self.len
    }

    fn get(&self, index: usize) -> Option<&[T]> {
        self.slots.get(index)?.as_deref()
    }

    fn get_mut(&mut self, index: usize) -> Option<&mut [T]> {
        self.slots.get_mut(index)?.as_deref_mut()
    }
}

pub(super) struct OwnedTrajectoryBand {
    fields: Vec<PackedTrajectoryMv>,
    positions: Vec<TrajectoryPositions>,
    projection_offsets: Vec<i32>,
    cells_per_reference: usize,
    row_base: usize,
    step: usize,
    unit_size8: usize,
    width8: usize,
    height8: usize,
    row_count: usize,
}

#[derive(Debug)]
pub(super) struct OwnedTrajectoryFields {
    cells: Vec<PackedTrajectoryMv>,
    cells_per_reference: usize,
}

impl OwnedTrajectoryFields {
    pub(super) fn cell(&self, reference: usize, index: usize) -> Option<Mv> {
        let base = reference.checked_mul(self.cells_per_reference)?;
        self.cells
            .get(base.checked_add(index)?)
            .copied()
            .and_then(PackedTrajectoryMv::unpack)
    }
}

impl OwnedTrajectoryBand {
    pub(super) fn new(
        width8: usize,
        height8: usize,
        row_base: usize,
        row_count: usize,
        reference_count: usize,
        step: usize,
        unit_size8: usize,
    ) -> Option<Self> {
        if reference_count > MAX_TRAJECTORY_REFERENCES {
            return None;
        }
        let cell_count = width8.checked_mul(row_count)?;
        let total_cells = cell_count.checked_mul(reference_count)?;
        Some(Self {
            fields: vec![PackedTrajectoryMv::INVALID; total_cells],
            positions: vec![TrajectoryPositions::EMPTY; total_cells],
            projection_offsets: vec![INVALID_PROJECTION_OFFSET; cell_count],
            cells_per_reference: cell_count,
            row_base,
            step: step.clamp(1, 2),
            unit_size8: unit_size8.max(1),
            width8,
            height8,
            row_count,
        })
    }

    pub(super) fn as_band(&mut self) -> Option<TrajectoryBand<'_>> {
        let unit_mask = self.unit_size8 - 1;
        let cells_per_reference = self.cells_per_reference.max(1);
        Some(TrajectoryBand {
            fields: BandSlices::from_chunks(&mut self.fields, cells_per_reference)?,
            positions: BandSlices::from_chunks(&mut self.positions, cells_per_reference)?,
            projection_offsets: &mut self.projection_offsets,
            row_base: self.row_base,
            step: self.step,
            step_mask: self.step - 1,
            unit_size8: self.unit_size8,
            unit_mask,
            unit_shift: self.unit_size8.trailing_zeros(),
            width8: self.width8,
            height8: self.height8,
        })
    }

    pub(super) fn finish(mut self) -> OwnedTrajectoryFields {
        if self.step == 2 {
            for field in self.fields.chunks_mut(self.cells_per_reference.max(1)) {
                fill_band_field_gaps(
                    field,
                    self.width8,
                    self.row_base,
                    self.row_count,
                    self.unit_size8,
                );
            }
        }
        OwnedTrajectoryFields {
            cells: self.fields,
            cells_per_reference: self.cells_per_reference,
        }
    }
}

impl TrajectoryBand<'_> {
    fn band_index(&self, y8: usize, x8: usize) -> Option<usize> {
        y8.checked_sub(self.row_base)
            .map(|row| row * self.width8 + x8)
    }

    fn positions_at(&self, reference: usize, at: Position) -> Option<&TrajectoryPositions> {
        let index = self.band_index(at.0, at.1)?;
        self.positions.get(reference)?.get(index)
    }

    fn trajectory_mv(&self, reference: usize, index: usize) -> Mv {
        self.fields
            .get(reference)
            .and_then(|field| field.get(index))
            .copied()
            .and_then(PackedTrajectoryMv::unpack)
            .unwrap_or(INVALID_TRAJECTORY_MV)
    }

    fn set_position_at(
        &mut self,
        reference: usize,
        index: usize,
        phase: usize,
        position: Position,
    ) {
        let Some(position) = PackedPosition::new(position) else {
            return;
        };
        if let Some(cell) = self
            .positions
            .get_mut(reference)
            .and_then(|field| field.get_mut(index))
            && let Some(slot) = cell.phases.get_mut(phase)
        {
            *slot = position;
            cell.mask |= 1 << phase;
        }
    }

    #[allow(clippy::inline_always, reason = "measured trajectory hot path")]
    #[inline(always)]
    fn set_position(&mut self, reference: usize, at: Position, phase: usize, position: Position) {
        if let Some(index) = self.band_index(at.0, at.1) {
            self.set_position_at(reference, index, phase, position);
        }
    }

    fn phase(&self, x8: usize) -> usize {
        (x8 >> self.unit_shift) % 3
    }

    fn unit_base(&self, value: usize) -> usize {
        value & !self.unit_mask
    }

    fn round_step(&self, value: usize) -> usize {
        value & !self.step_mask
    }

    fn position_bounds(&self, base: Position) -> (usize, usize, usize, usize) {
        let base_y = self.unit_base(base.0);
        let base_x = self.unit_base(base.1);
        let col_offset = if self.step == 1 {
            self.unit_size8 / 2
        } else {
            self.unit_size8
        };
        (
            base_y,
            base_y + self.unit_size8,
            base_x.saturating_sub(col_offset),
            base_x + self.unit_size8 + col_offset,
        )
    }

    fn position_allowed(
        candidate: Position,
        (min_y, max_y, min_x, max_x): (usize, usize, usize, usize),
    ) -> bool {
        candidate.0 >= min_y && candidate.0 < max_y && candidate.1 >= min_x && candidate.1 < max_x
    }

    fn sampled_position(&self, y8: usize, x8: usize, mv: Mv) -> Option<(usize, usize)> {
        let y8 = project_no_constraint(y8, mv.row, self.height8)?;
        let x8 = project_no_constraint(x8, mv.col, self.width8)?;
        Some((self.round_step(y8), self.round_step(x8)))
    }

    fn set_field_at(&mut self, reference: usize, index: usize, mv: Mv) -> Mv {
        let mv = clamp_mv(mv);
        if let Some(slot) = self
            .fields
            .get_mut(reference)
            .and_then(|field| field.get_mut(index))
        {
            *slot = PackedTrajectoryMv::new(mv);
        }
        mv
    }

    pub(super) fn check_intersection(
        &mut self,
        source: usize,
        end: Option<usize>,
        y8: usize,
        x8: usize,
        mv: Mv,
    ) -> Option<Position> {
        let end = end.filter(|&end| end < self.fields.len())?;
        if source >= self.fields.len()
            || source >= self.positions.len()
            || end >= self.positions.len()
            || y8 >= self.height8
            || x8 >= self.width8
        {
            return None;
        }
        let mut source_mask = self
            .positions_at(source, (y8, x8))
            .map_or(0, |slots| slots.mask);
        if source_mask != 0 {
            let phases = self
                .positions_at(source, (y8, x8))
                .map_or(TrajectoryPositions::EMPTY.phases, |slots| slots.phases);
            while source_mask != 0 {
                let phase = source_mask.trailing_zeros() as usize;
                source_mask &= source_mask - 1;
                let Some(&packed) = phases.get(phase) else {
                    break;
                };
                let trajectory = (packed.y as usize, packed.x as usize);
                let bounds = self.position_bounds(trajectory);
                let Some(traj_index) = self.band_index(trajectory.0, trajectory.1) else {
                    continue;
                };
                if self.trajectory_mv(end, traj_index) != INVALID_TRAJECTORY_MV {
                    continue;
                }
                let source_mv = self.trajectory_mv(source, traj_index);
                if source_mv == INVALID_TRAJECTORY_MV {
                    continue;
                }
                let end_mv = self.set_field_at(end, traj_index, add_mv(source_mv, mv));
                if let Some(position) = self
                    .sampled_position(trajectory.0, trajectory.1, end_mv)
                    .filter(|&position| Self::position_allowed(position, bounds))
                {
                    self.set_position(end, position, phase, trajectory);
                }
            }
        }

        let end_position = self.sampled_position(y8, x8, mv)?;
        if self.unit_base(end_position.0) != self.unit_base(y8) {
            return Some(end_position);
        }
        let mut end_mask = self
            .positions_at(end, end_position)
            .map_or(0, |slots| slots.mask);
        if end_mask == 0 {
            return Some(end_position);
        }
        let phases = self
            .positions_at(end, end_position)
            .map_or(TrajectoryPositions::EMPTY.phases, |slots| slots.phases);
        while end_mask != 0 {
            let phase = end_mask.trailing_zeros() as usize;
            end_mask &= end_mask - 1;
            let Some(&packed) = phases.get(phase) else {
                break;
            };
            let trajectory = (packed.y as usize, packed.x as usize);
            let bounds = self.position_bounds(trajectory);
            if !Self::position_allowed((y8, x8), bounds) {
                continue;
            }
            let Some(traj_index) = self.band_index(trajectory.0, trajectory.1) else {
                continue;
            };
            if self.trajectory_mv(source, traj_index) != INVALID_TRAJECTORY_MV {
                continue;
            }
            let end_mv = self.trajectory_mv(end, traj_index);
            if end_mv == INVALID_TRAJECTORY_MV {
                continue;
            }
            let source_mv = self.set_field_at(source, traj_index, subtract_mv(end_mv, mv));
            if let Some(position) = self
                .sampled_position(trajectory.0, trajectory.1, source_mv)
                .filter(|&position| Self::position_allowed(position, bounds))
            {
                self.set_position(source, position, phase, trajectory);
            }
        }
        Some(end_position)
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
        let Some(position) = self.sampled_position(y8, x8, projected) else {
            return;
        };
        let bounds = self.position_bounds(position);
        if !Self::position_allowed((y8, x8), bounds) {
            return;
        }
        let target_position = self.sampled_position(y8, x8, mv);
        self.observe_projection_at(
            source,
            end,
            target,
            y8,
            x8,
            mv,
            projected,
            position,
            target_position,
            source_to_current,
            reference_offset,
            backward,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn observe_projection_at(
        &mut self,
        source: usize,
        end: Option<usize>,
        target: Option<usize>,
        y8: usize,
        x8: usize,
        mv: Mv,
        projected: Mv,
        position: Position,
        target_position: Option<Position>,
        source_to_current: i32,
        reference_offset: i32,
        backward: bool,
    ) {
        if y8 >= self.height8 || x8 >= self.width8 {
            return;
        }
        let Some(recorded_offset) = self
            .band_index(position.0, position.1)
            .and_then(|index| self.projection_offsets.get_mut(index))
        else {
            return;
        };
        let replace = *recorded_offset == INVALID_PROJECTION_OFFSET
            || (target.is_some() && target == end && *recorded_offset != reference_offset);
        if !replace {
            return;
        }
        *recorded_offset = reference_offset;
        let phase = self.phase(position.1);
        let Some(index) = self.band_index(position.0, position.1) else {
            return;
        };
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
        let numerator = if backward {
            -source_to_current
        } else {
            source_to_current
        };
        let Some(end_mv) = project_mv(mv, reference_offset - numerator, reference_offset) else {
            return;
        };
        self.set_field_at(end, index, end_mv);
        let bounds = self.position_bounds(position);
        if let Some(target_position) = target_position
            .filter(|&target_position| Self::position_allowed(target_position, bounds))
        {
            self.set_position(end, target_position, phase, position);
        }
    }
}

fn fill_band_field_gaps(
    cells: &mut [PackedTrajectoryMv],
    width8: usize,
    row_base: usize,
    row_count: usize,
    unit_size8: usize,
) {
    let get = |cells: &[PackedTrajectoryMv], y8: usize, x8: usize| {
        let row = y8.checked_sub(row_base)?;
        cells
            .get(row.checked_mul(width8)?.checked_add(x8)?)
            .copied()
            .and_then(PackedTrajectoryMv::unpack)
    };
    for y8 in (row_base..row_base.saturating_add(row_count)).step_by(2) {
        for x8 in (0..width8).step_by(2) {
            let Some(anchor) = get(cells, y8, x8) else {
                continue;
            };
            for (dy, dx) in [(0usize, 1usize), (1, 0), (1, 1)] {
                if y8 + dy >= row_base + row_count || x8 + dx >= width8 {
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
                        || source_y >= row_base + row_count
                        || source_x >= width8
                        || source_y / unit_size8 != y8 / unit_size8
                        || source_x / unit_size8 != x8 / unit_size8
                    {
                        continue;
                    }
                    if let Some(mv) = get(cells, source_y, source_x) {
                        sum = add_mv(sum, mv);
                        count += 1;
                    }
                }
                let Some(index) = (y8 + dy)
                    .checked_sub(row_base)
                    .and_then(|row| row.checked_mul(width8))
                    .and_then(|base| base.checked_add(x8 + dx))
                else {
                    continue;
                };
                if let Some(cell) = cells.get_mut(index) {
                    *cell = PackedTrajectoryMv::new(Mv {
                        row: average(sum.row, count),
                        col: average(sum.col, count),
                    });
                }
            }
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
        assert_eq!(std::mem::size_of::<TrajectoryPositions>(), 14);
        assert_eq!(std::mem::size_of::<i32>(), 4);
        assert_eq!(
            PackedPosition::new((8191, 8191)).and_then(PackedPosition::unpack),
            Some((8191, 8191))
        );
    }

    #[test]
    fn owned_band_rejects_more_than_seven_references() {
        assert!(OwnedTrajectoryBand::new(1, 1, 0, 1, 8, 1, 8).is_none());
    }

    #[test]
    fn direct_projection_records_reference_specific_trajectories() {
        let mut state = TrajectoryState::new((8, 8), 2, 1, 8).unwrap();
        state.whole_band().unwrap().observe_projection(
            1,
            Some(0),
            None,
            1,
            1,
            Mv { row: 16, col: 32 },
            2,
            4,
            false,
        );

        assert_eq!(state.fields[1].cell(1, 1), Some(Mv { row: -8, col: -16 }));
        assert_eq!(state.fields[0].cell(1, 1), Some(Mv { row: 8, col: 16 }));
    }

    #[test]
    fn zero_offset_projection_records_the_source_trajectory() {
        let mut state = TrajectoryState::new((8, 8), 1, 1, 8).unwrap();

        state
            .whole_band()
            .unwrap()
            .observe_projection(0, None, None, 1, 1, Mv::ZERO, 2, 0, false);

        assert_eq!(state.fields[0].cell(1, 1), Some(Mv::ZERO));
    }

    #[test]
    fn direct_projection_checks_source_against_destination_unit() {
        let mut state = TrajectoryState::new((4, 32), 2, 1, 8).unwrap();
        state.whole_band().unwrap().observe_projection(
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
        state.whole_band().unwrap().observe_projection(
            0,
            Some(1),
            None,
            1,
            1,
            Mv { row: 0, col: 64 },
            2,
            4,
            false,
        );
        state
            .whole_band()
            .unwrap()
            .check_intersection(1, Some(2), 1, 2, Mv { row: 0, col: 64 });

        assert_eq!(state.fields[2].cell(1, 1), Some(Mv { row: 0, col: 96 }));
    }

    #[test]
    fn intersection_visits_only_the_recorded_sparse_phases() {
        let mut state = TrajectoryState::new((8, 8), 2, 1, 8).unwrap();
        let source_index = state.fields[0].index(1, 2).unwrap();
        state.positions[0][source_index] = TrajectoryPositions {
            phases: [
                PackedPosition::new((1, 1)).unwrap(),
                PackedPosition::INVALID,
                PackedPosition::new((1, 3)).unwrap(),
            ],
            mask: 0b101,
        };
        state.fields[0].set(1, 1, Mv { row: 8, col: 16 });
        state.fields[0].set(1, 3, Mv { row: 24, col: 32 });

        state
            .whole_band()
            .unwrap()
            .check_intersection(0, Some(1), 1, 2, Mv::ZERO);

        assert_eq!(state.fields[1].cell(1, 1), Some(Mv { row: 8, col: 16 }));
        assert_eq!(state.fields[1].cell(1, 3), Some(Mv { row: 24, col: 32 }));
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

#[cfg(test)]
mod clamped_sampling_tests;
