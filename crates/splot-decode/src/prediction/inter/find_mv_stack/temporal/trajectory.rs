// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed_i32;

use super::{
    Mv, REFMVS_LIMIT, allocate_temporal_grid, project_no_constraint, project_tmvp_mv,
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
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
        })
    }

    #[cfg(test)]
    pub(super) fn cell(&self, y8: usize, x8: usize) -> Option<Mv> {
        let index = temporal_grid_index(self.width8, self.height8, y8, x8)?;
        self.cells
            .get(index)
            .copied()
            .and_then(PackedTrajectoryMv::unpack)
    }

    #[cfg(test)]
    pub(super) fn set(&mut self, y8: usize, x8: usize, mv: Mv) {
        if let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8)
            && let Some(cell) = self.cells.get_mut(index)
        {
            *cell = PackedTrajectoryMv::new(mv);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TrajectoryState {
    /// Every reference's § 7.9.8 trajectory motion vector for one cell, adjacent.
    ///
    /// The § 7.9.3 walk reads two references' vectors at the same cell before it
    /// writes either, so reference-major planes put those two reads a whole
    /// plane apart — hundreds of kilobytes at 1080p, and a separate line every
    /// time. Interleaving by reference lands them in one line: pushing the
    /// planes 64 KiB further apart measured **+1.80%** against an allocation-
    /// matched control, which is the cost this layout reclaims.
    pub(super) cells: Vec<PackedTrajectoryMv>,
    pub(super) reference_count: usize,
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
        let (width8, height8) = (template.width8, template.height8);
        Some(Self {
            cells: vec![PackedTrajectoryMv::INVALID; cell_count.checked_mul(reference_count)?],
            reference_count,
            positions: vec![vec![TrajectoryPositions::EMPTY; cell_count]; reference_count],
            projection_offsets: vec![INVALID_PROJECTION_OFFSET; cell_count],
            step,
            unit_size8,
            width8,
            height8,
        })
    }

    /// Writes one reference's trajectory vector at a whole-field cell.
    #[cfg(test)]
    pub(super) fn set_trajectory_cell(&mut self, reference: usize, y8: usize, x8: usize, mv: Mv) {
        if reference >= self.reference_count {
            return;
        }
        if let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8)
            && let Some(slot) = self.cells.get_mut(index * self.reference_count + reference)
        {
            *slot = PackedTrajectoryMv::new(mv);
        }
    }

    /// Reads one reference's trajectory vector at a whole-field cell.
    pub(super) fn trajectory_cell(&self, reference: usize, y8: usize, x8: usize) -> Option<Mv> {
        if reference >= self.reference_count {
            return None;
        }
        let index = temporal_grid_index(self.width8, self.height8, y8, x8)?;
        self.cells
            .get(index * self.reference_count + reference)
            .copied()
            .and_then(PackedTrajectoryMv::unpack)
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
        let total = cell_count.checked_mul(reference_count)?;
        self.reference_count = reference_count;
        self.cells.resize(total, PackedTrajectoryMv::INVALID);
        self.cells.fill(PackedTrajectoryMv::INVALID);
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
        (self.width8, self.height8) = (width8, height8);
        Some(())
    }

    #[cfg(test)]
    pub(super) fn from_fields(fields: &[TrajectoryMotionField]) -> Self {
        let (width8, height8) = fields
            .first()
            .map_or((0, 0), |field| (field.width8, field.height8));
        let reference_count = fields.len();
        let cell_count = fields.first().map_or(0, |field| field.cells.len());
        let mut cells = vec![PackedTrajectoryMv::INVALID; cell_count * reference_count];
        for (reference, field) in fields.iter().enumerate() {
            for (index, &cell) in field.cells.iter().enumerate() {
                cells[index * reference_count + reference] = cell;
            }
        }
        Self {
            cells,
            reference_count,
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
        (0..self.reference_count)
            .map(|reference| TrajectoryMotionField {
                width8: self.width8,
                height8: self.height8,
                cells: self
                    .cells
                    .iter()
                    .skip(reference)
                    .step_by(self.reference_count)
                    .copied()
                    .collect(),
            })
            .collect()
    }

    /// Splits every trajectory grid into `band_rows`-tall row bands.
    ///
    /// Bands are unit-aligned, and AV2 § 7.9.8 keeps each projected sample in
    /// the TMVP unit row of the position it was sampled from, so the bands
    /// partition every write the § 7.9.3 scan makes. Returns `None` when a grid
    /// is not sized to this state's geometry, leaving the caller whole-field.
    pub(super) fn bands(&mut self, band_rows: usize) -> Option<Vec<TrajectoryBand<'_>>> {
        let Self {
            cells,
            reference_count,
            positions,
            projection_offsets,
            step,
            unit_size8,
            width8,
            height8,
        } = self;
        let reference_count = *reference_count;
        let (width8, height8) = (*width8, *height8);
        let step_mask = *step - 1;
        let unit_mask = *unit_size8 - 1;
        let unit_shift = unit_size8.trailing_zeros();
        let total = width8.checked_mul(height8)?;
        let stride = band_rows.checked_mul(width8)?;
        if band_rows == 0
            || reference_count > MAX_TRAJECTORY_REFERENCES
            || positions.len() != reference_count
            || projection_offsets.len() != total
            || cells.len() != total.checked_mul(reference_count)?
            || positions.iter().any(|slots| slots.len() != total)
        {
            return None;
        }
        let mut field_bands = cells.chunks_mut(stride.checked_mul(reference_count)?.max(1));
        let mut bands = projection_offsets
            .chunks_mut(stride.max(1))
            .enumerate()
            .map(|(index, projection_offsets)| TrajectoryBand {
                fields: field_bands.next().unwrap_or_default(),
                reference_count,
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
        for reference in 0..self.reference_count {
            fill_band_field_gaps(
                &mut self.cells,
                reference,
                self.reference_count,
                self.width8,
                0,
                self.height8,
                self.unit_size8,
            );
        }
    }
}

/// One unit-aligned row band of a [`TrajectoryState`].
///
/// Geometry stays whole-frame — projections are sampled against the full grid
/// — while the storage slices cover `row_base .. row_base + band_rows` only.
pub(super) struct TrajectoryBand<'a> {
    fields: &'a mut [PackedTrajectoryMv],
    reference_count: usize,
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
    reference_count: usize,
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
    reference_count: usize,
}

impl OwnedTrajectoryFields {
    pub(super) fn cell(&self, reference: usize, index: usize) -> Option<Mv> {
        if reference >= self.reference_count {
            return None;
        }
        self.cells
            .get(
                index
                    .checked_mul(self.reference_count)?
                    .checked_add(reference)?,
            )
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
    ) -> crate::Result<Self> {
        if reference_count > MAX_TRAJECTORY_REFERENCES {
            return Err(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState.into());
        }
        let cell_count = width8
            .checked_mul(row_count)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        let total_cells = cell_count
            .checked_mul(reference_count)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        let allocation = || {
            crate::DecodeError::from(splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: splot_recon::PlaneId::Y,
                context: "inter temporal trajectory band",
            })
        };
        let mut fields = Vec::new();
        fields
            .try_reserve_exact(total_cells)
            .map_err(|_| allocation())?;
        fields.resize(total_cells, PackedTrajectoryMv::INVALID);
        let mut positions = Vec::new();
        positions
            .try_reserve_exact(total_cells)
            .map_err(|_| allocation())?;
        positions.resize(total_cells, TrajectoryPositions::EMPTY);
        let mut projection_offsets = Vec::new();
        projection_offsets
            .try_reserve_exact(cell_count)
            .map_err(|_| allocation())?;
        projection_offsets.resize(cell_count, INVALID_PROJECTION_OFFSET);
        Ok(Self {
            fields,
            positions,
            projection_offsets,
            cells_per_reference: cell_count,
            reference_count,
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
            fields: &mut self.fields,
            reference_count: self.reference_count,
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
            for reference in 0..self.reference_count {
                fill_band_field_gaps(
                    &mut self.fields,
                    reference,
                    self.reference_count,
                    self.width8,
                    self.row_base,
                    self.row_count,
                    self.unit_size8,
                );
            }
        }
        OwnedTrajectoryFields {
            cells: core::mem::take(&mut self.fields),
            reference_count: self.reference_count,
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

    /// Reads one reference's trajectory vector at a band-relative cell.
    ///
    /// See [`TrajectoryState::cells`] for why the reference is the minor axis.
    /// A reference past `reference_count` addresses a later cell's slot rather
    /// than running off the grid, so callers carry that bound: `check_intersection`
    /// tests it outright, and `observe_projection_at` inherits it from the
    /// reference tables its `source` and `end` are resolved through.
    fn trajectory_mv(&self, reference: usize, index: usize) -> Mv {
        debug_assert!(reference < self.reference_count);
        self.fields
            .get(index * self.reference_count + reference)
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

    /// Writes one reference's trajectory vector at a band-relative cell.
    ///
    /// Carries [`Self::trajectory_mv`]'s reference bound for the same reason.
    fn set_field_at(&mut self, reference: usize, index: usize, mv: Mv) -> Mv {
        debug_assert!(reference < self.reference_count);
        let mv = clamp_mv(mv);
        if let Some(slot) = self
            .fields
            .get_mut(index * self.reference_count + reference)
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
        let end = end.filter(|&end| end < self.reference_count)?;
        if source >= self.reference_count
            || source >= self.positions.len()
            || end >= self.positions.len()
            || y8 >= self.height8
            || x8 >= self.width8
        {
            return None;
        }
        let source_slots = self
            .positions_at(source, (y8, x8))
            .copied()
            .unwrap_or(TrajectoryPositions::EMPTY);
        let mut source_mask = source_slots.mask;
        while source_mask != 0 {
            let phase = source_mask.trailing_zeros() as usize;
            source_mask &= source_mask - 1;
            let Some(&packed) = source_slots.phases.get(phase) else {
                break;
            };
            let trajectory = (packed.y as usize, packed.x as usize);
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
            let bounds = self.position_bounds(trajectory);
            let end_mv = self.set_field_at(end, traj_index, add_mv(source_mv, mv));
            if let Some(position) = self
                .sampled_position(trajectory.0, trajectory.1, end_mv)
                .filter(|&position| Self::position_allowed(position, bounds))
            {
                self.set_position(end, position, phase, trajectory);
            }
        }

        let end_position = self.sampled_position(y8, x8, mv)?;
        if self.unit_base(end_position.0) != self.unit_base(y8) {
            return Some(end_position);
        }
        let end_slots = self
            .positions_at(end, end_position)
            .copied()
            .unwrap_or(TrajectoryPositions::EMPTY);
        let mut end_mask = end_slots.mask;
        while end_mask != 0 {
            let phase = end_mask.trailing_zeros() as usize;
            end_mask &= end_mask - 1;
            let Some(&packed) = end_slots.phases.get(phase) else {
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
        let projected = project_tmvp_mv(mv, numerator, reference_offset);
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

    /// Reports whether a projected position still admits a § 7.9.8 observation.
    ///
    /// [`Self::observe_projection_at`] settles this from data the § 7.9.3 scan
    /// already holds, and settles it against the caller 59% of the time, so the
    /// scan tests it here instead: the rejected majority never marshals that
    /// call's arguments or builds its frame. Returning `false` is exactly the
    /// condition under which the call would return having written nothing.
    #[allow(clippy::inline_always, reason = "measured trajectory scan guard")]
    #[inline(always)]
    pub(super) fn admits_projection(
        &self,
        end: Option<usize>,
        target: Option<usize>,
        position: Position,
        reference_offset: i32,
    ) -> bool {
        let Some(index) = self.band_index(position.0, position.1) else {
            return false;
        };
        let Some(&recorded_offset) = self.projection_offsets.get(index) else {
            return false;
        };
        recorded_offset == INVALID_PROJECTION_OFFSET
            || (target.is_some() && target == end && recorded_offset != reference_offset)
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
        let Some(index) = self.band_index(position.0, position.1) else {
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
        let numerator = if backward {
            -source_to_current
        } else {
            source_to_current
        };
        let end_mv = project_tmvp_mv(mv, reference_offset - numerator, reference_offset);
        self.set_field_at(end, index, end_mv);
        let Some(target_position) = target_position else {
            return;
        };
        let bounds = self.position_bounds(position);
        if Self::position_allowed(target_position, bounds) {
            self.set_position(end, target_position, phase, position);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_band_field_gaps(
    cells: &mut [PackedTrajectoryMv],
    reference: usize,
    reference_count: usize,
    width8: usize,
    row_base: usize,
    row_count: usize,
    unit_size8: usize,
) {
    let slot = |y8: usize, x8: usize| {
        let row = y8.checked_sub(row_base)?;
        row.checked_mul(width8)?
            .checked_add(x8)?
            .checked_mul(reference_count)?
            .checked_add(reference)
    };
    let get = |cells: &[PackedTrajectoryMv], y8: usize, x8: usize| {
        cells
            .get(slot(y8, x8)?)
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
                let Some(index) = slot(y8 + dy, x8 + dx) else {
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
        assert!(OwnedTrajectoryBand::new(1, 1, 0, 1, 8, 1, 8).is_err());
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

        assert_eq!(
            state.trajectory_cell(1, 1, 1),
            Some(Mv { row: -8, col: -16 })
        );
        assert_eq!(state.trajectory_cell(0, 1, 1), Some(Mv { row: 8, col: 16 }));
    }

    #[test]
    fn zero_offset_projection_records_the_source_trajectory() {
        let mut state = TrajectoryState::new((8, 8), 1, 1, 8).unwrap();

        state
            .whole_band()
            .unwrap()
            .observe_projection(0, None, None, 1, 1, Mv::ZERO, 2, 0, false);

        assert_eq!(state.trajectory_cell(0, 1, 1), Some(Mv::ZERO));
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

        assert_eq!(
            state.trajectory_cell(0, 1, 14),
            Some(Mv { row: 0, col: -448 })
        );
        assert_eq!(
            state.trajectory_cell(1, 1, 14),
            Some(Mv { row: 0, col: 672 })
        );
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

        assert_eq!(state.trajectory_cell(2, 1, 1), Some(Mv { row: 0, col: 96 }));
    }

    #[test]
    fn intersection_visits_only_the_recorded_sparse_phases() {
        let mut state = TrajectoryState::new((8, 8), 2, 1, 8).unwrap();
        let source_index = temporal_grid_index(state.width8, state.height8, 1, 2).unwrap();
        state.positions[0][source_index] = TrajectoryPositions {
            phases: [
                PackedPosition::new((1, 1)).unwrap(),
                PackedPosition::INVALID,
                PackedPosition::new((1, 3)).unwrap(),
            ],
            mask: 0b101,
        };
        state.set_trajectory_cell(0, 1, 1, Mv { row: 8, col: 16 });
        state.set_trajectory_cell(0, 1, 3, Mv { row: 24, col: 32 });

        state
            .whole_band()
            .unwrap()
            .check_intersection(0, Some(1), 1, 2, Mv::ZERO);

        assert_eq!(state.trajectory_cell(1, 1, 1), Some(Mv { row: 8, col: 16 }));
        assert_eq!(
            state.trajectory_cell(1, 1, 3),
            Some(Mv { row: 24, col: 32 })
        );
    }

    #[test]
    fn trajectory_gap_fill_stays_within_tmvp_units() {
        let mut state = TrajectoryState::new((2, 36), 1, 2, 16).unwrap();
        state.set_trajectory_cell(0, 0, 14, Mv { row: 8, col: 16 });
        state.set_trajectory_cell(0, 0, 16, Mv { row: 24, col: 80 });

        state.fill_gaps();

        assert_eq!(
            state.trajectory_cell(0, 0, 15),
            state.trajectory_cell(0, 0, 14)
        );
        assert_eq!(
            state.trajectory_cell(0, 0, 17),
            state.trajectory_cell(0, 0, 16)
        );
    }

    #[test]
    fn step_two_trajectory_uses_64_pixel_superblock_units() {
        let mut state = TrajectoryState::new((2, 36), 1, 2, 8).unwrap();
        state.set_trajectory_cell(0, 0, 6, Mv { row: 8, col: 16 });
        state.set_trajectory_cell(0, 0, 8, Mv { row: 24, col: 80 });

        state.fill_gaps();

        assert_eq!(
            state.trajectory_cell(0, 0, 7),
            state.trajectory_cell(0, 0, 6)
        );
        assert_eq!(
            state.trajectory_cell(0, 0, 9),
            state.trajectory_cell(0, 0, 8)
        );
    }
}

#[cfg(test)]
mod clamped_sampling_tests;
