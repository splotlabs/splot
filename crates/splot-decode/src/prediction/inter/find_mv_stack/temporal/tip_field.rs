// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.10.4 TIP motion field: the projected field scaled to the TIP pair,
//! hole-filled and averaged inside each TMVP unit.

use std::simd::{
    Select, Simd,
    cmp::{SimdOrd, SimdPartialEq, SimdPartialOrd},
    num::SimdInt,
};

use splot_recon::math::round2_signed;

use super::{
    DIV_MULT, MAX_FRAME_DISTANCE, Mv, ProjectedTemporalMotionCell, ProjectedTemporalMotionField,
    REFMVS_LIMIT, TipReferencePair, fill_temporal_sampling_gaps, project_tmvp_mv,
};

pub(super) fn prepare_tip_field(
    field: &mut ProjectedTemporalMotionField,
    references: TipReferencePair,
    projection_step: usize,
    tmvp_unit_size8: usize,
    fill_holes: bool,
) -> crate::Result<()> {
    match projection_step {
        1 if tmvp_unit_size8 <= 8 => {
            prepare_tip_rows::<8>(field, references, tmvp_unit_size8, fill_holes)
        }
        1 => prepare_tip_rows::<MAX_TIP_UNIT8>(field, references, tmvp_unit_size8, fill_holes),
        2 => prepare_tip_units::<2>(field, references, tmvp_unit_size8, fill_holes),
        _ => Err(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState.into()),
    }?;
    fill_temporal_sampling_gaps(field, projection_step, tmvp_unit_size8);
    Ok(())
}

/// Largest TMVP unit side, in 8x8 cells, that TIP preparation works inside.
const MAX_TIP_UNIT8: usize = 16;
/// Side of the sample grid: one unit plus a zero border, so every sample has
/// four neighbours and an absent one adds nothing to an average.
const TIP_GRID: usize = MAX_TIP_UNIT8 + 2;

/// A TIP sample as `[valid, row, col, ref_offset]`. An invalid sample carries a
/// zero vector, so summing samples sums the valid vectors and counts them.
type TipSample = Simd<i16, 4>;

/// AV2 § 7.10.4 TIP motion, one TMVP unit at a time.
///
/// Every sampled cell is scaled to the TIP pair; holes are then filled and the
/// samples averaged. Both read only their own unit, so a unit's samples are
/// gathered into a dense grid, settled there and written back once.
fn prepare_tip_units<const STEP: usize>(
    field: &mut ProjectedTemporalMotionField,
    references: TipReferencePair,
    unit: usize,
    fill_holes: bool,
) -> crate::Result<()> {
    let (width8, height8) = (field.width8, field.height8);
    if unit == 0
        || !unit.is_multiple_of(STEP)
        || unit > MAX_TIP_UNIT8
        || width8.checked_mul(height8) != Some(field.cells.len())
    {
        return Err(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState.into());
    }
    let mut grid = [TipSample::splat(0); TIP_GRID * TIP_GRID];
    for block_y in (0..height8).step_by(unit) {
        let unit_rows = (height8 - block_y).min(unit);
        for block_x in (0..width8).step_by(unit) {
            let unit_cols = (width8 - block_x).min(unit);
            let (rows, cols) = (unit_rows.div_ceil(STEP), unit_cols.div_ceil(STEP));
            let mut valid = 0;
            for row in 1..=rows {
                let start = (block_y + (row - 1) * STEP) * width8 + block_x;
                let cells = field.cells[start..start + unit_cols].iter().step_by(STEP);
                let samples = &mut grid[row * TIP_GRID + 1..][..cols];
                for (sample, &cell) in samples.iter_mut().zip(cells) {
                    *sample = scale_tip_cell(cell, references);
                    valid += usize::from(sample[0] != 0);
                }
                grid[row * TIP_GRID + cols + 1] = TipSample::splat(0);
            }
            grid[(rows + 1) * TIP_GRID..][..=cols].fill(TipSample::splat(0));
            if fill_holes && valid != 0 && valid != rows * cols {
                fill_tip_holes(&mut grid, rows, cols);
            }
            for y in 0..unit_rows {
                let start = (block_y + y) * width8 + block_x;
                let cells = &mut field.cells[start..start + unit_cols];
                if !y.is_multiple_of(STEP) {
                    cells.fill(ProjectedTemporalMotionCell::default());
                    continue;
                }
                let base = (y / STEP + 1) * TIP_GRID + 1;
                for (x, cell) in cells.iter_mut().enumerate() {
                    *cell = if !x.is_multiple_of(STEP) {
                        ProjectedTemporalMotionCell::default()
                    } else if fill_holes {
                        average_tip_sample(&grid, base + x / STEP)
                    } else {
                        let sample = grid[base + x / STEP];
                        ProjectedTemporalMotionCell {
                            valid: sample[0] != 0,
                            ref_offset: sample[3],
                            mv: [sample[1], sample[2]],
                        }
                    };
                }
            }
        }
    }
    Ok(())
}

/// One unit row of up to `N` step-1 samples.
type TipRow<const N: usize> = Simd<i32, N>;

/// A step-1 unit's valid flags and vector components as zero-bordered
/// planes; every sample's offset is the TIP pair's.
type TipPlanes = [[i32; TIP_GRID * TIP_GRID]; 3];

/// [`prepare_tip_units`] for step 1, one unit row of samples at a time.
///
/// A row scales lane-parallel while no component exceeds `REFMVS_LIMIT`, so
/// every product stays inside `i32`; a wider row scales through
/// [`scale_tip_cell`]. An empty average is the default cell, because its
/// vector sums to zero.
fn prepare_tip_rows<const N: usize>(
    field: &mut ProjectedTemporalMotionField,
    references: TipReferencePair,
    unit: usize,
    fill_holes: bool,
) -> crate::Result<()> {
    let (width8, height8) = (field.width8, field.height8);
    if unit == 0
        || unit > N
        || N > MAX_TIP_UNIT8
        || width8.checked_mul(height8) != Some(field.cells.len())
    {
        return Err(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState.into());
    }
    let numerator = references
        .ref_offset
        .clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let offset = references.ref_offset as i16;
    let limit = TipRow::<N>::splat(REFMVS_LIMIT);
    let load = |plane: &[i32], at: usize| TipRow::<N>::from_slice(&plane[at..at + N]);
    let mut planes: TipPlanes = [[0; TIP_GRID * TIP_GRID]; 3];
    for block_y in (0..height8).step_by(unit) {
        let rows = (height8 - block_y).min(unit);
        for block_x in (0..width8).step_by(unit) {
            let cols = (width8 - block_x).min(unit);
            let mut valid_count = 0;
            for y in 0..rows {
                let start = (block_y + y) * width8 + block_x;
                let cells = &field.cells[start..start + cols];
                let mut lanes = [[0; N]; 4];
                for (lane, cell) in cells.iter().enumerate() {
                    let denominator = cell.ref_offset().clamp(0, MAX_FRAME_DISTANCE) as usize;
                    lanes[0][lane] = i32::from(cell.valid);
                    lanes[1][lane] = i32::from(cell.mv[0]);
                    lanes[2][lane] = i32::from(cell.mv[1]);
                    lanes[3][lane] = DIV_MULT[denominator];
                }
                let [valid, mv_row, mv_col, divisor] = lanes.map(TipRow::from_array);
                valid_count += valid.reduce_sum();
                let factor = divisor * TipRow::splat(numerator);
                let scale = |mv: TipRow<N>| {
                    round2_signed_lanes(mv * factor, 14).simd_clamp(-limit, limit) & -valid
                };
                let mut scaled = [valid, scale(mv_row), scale(mv_col)];
                if mv_row.abs().simd_max(mv_col.abs()).simd_gt(limit).any() {
                    let mut wide = [[0; N]; 2];
                    for (lane, &cell) in cells.iter().enumerate() {
                        let sample = scale_tip_cell(cell, references);
                        wide[0][lane] = i32::from(sample[1]);
                        wide[1][lane] = i32::from(sample[2]);
                    }
                    [scaled[1], scaled[2]] = wide.map(TipRow::from_array);
                }
                let at = (y + 1) * TIP_GRID + 1;
                for (plane, value) in planes.iter_mut().zip(scaled) {
                    value.copy_to_slice(&mut plane[at..at + N]);
                }
            }
            let below = (rows + 1) * TIP_GRID + 1;
            for plane in &mut planes {
                plane[below..below + N].fill(0);
            }
            if fill_holes && valid_count != 0 && valid_count != (rows * cols) as i32 {
                fill_tip_plane_holes(&mut planes, rows, cols);
            }
            for y in 0..rows {
                let at = (y + 1) * TIP_GRID + 1;
                let [valid, mv_row, mv_col] = if fill_holes {
                    let cross = |plane: &[i32]| {
                        load(plane, at)
                            + load(plane, at - TIP_GRID)
                            + load(plane, at - 1)
                            + load(plane, at + TIP_GRID)
                            + load(plane, at + 1)
                    };
                    let count = cross(&planes[0]);
                    let weight = tip_average_weights(count);
                    let average = |plane: &[i32]| round2_signed_lanes(cross(plane) * weight, 16);
                    [count, average(&planes[1]), average(&planes[2])]
                } else {
                    planes.each_ref().map(|plane| load(plane, at))
                }
                .map(TipRow::to_array);
                let start = (block_y + y) * width8 + block_x;
                for (lane, cell) in field.cells[start..start + cols].iter_mut().enumerate() {
                    let valid = valid[lane] != 0;
                    *cell = ProjectedTemporalMotionCell {
                        valid,
                        ref_offset: if valid || !fill_holes { offset } else { 0 },
                        mv: [mv_row[lane] as i16, mv_col[lane] as i16],
                    };
                }
            }
        }
    }
    Ok(())
}

/// `Round2Signed` per lane: a negative value adds one less before the
/// arithmetic shift, so its half rounds away from zero.
fn round2_signed_lanes<const N: usize>(value: TipRow<N>, shift: i32) -> TipRow<N> {
    (value + TipRow::splat(1 << (shift - 1)) + (value >> 31)) >> shift
}

/// [`fill_tip_holes`] on sample planes.
fn fill_tip_plane_holes(planes: &mut TipPlanes, rows: usize, cols: usize) {
    for row in 1..=rows {
        for col in 1..=cols {
            let index = row * TIP_GRID + col;
            let source = planes.each_ref().map(|plane| plane[index]);
            let mut fill = |destination: usize| {
                if planes[0][destination] == 0 {
                    for (plane, value) in planes.iter_mut().zip(source) {
                        plane[destination] = value;
                    }
                }
            };
            if row > 1 {
                fill(index - TIP_GRID);
            }
            if col > 1 {
                fill(index - 1);
            }
            if row < rows {
                fill(index + TIP_GRID);
            }
            if col < cols {
                fill(index + 1);
            }
        }
    }
}

/// [`divide_tip_average`]'s weight for each lane's sample count.
fn tip_average_weights<const N: usize>(count: TipRow<N>) -> TipRow<N> {
    let mut weight = TipRow::splat(0);
    for (index, &value) in TIP_AVERAGE_WEIGHTS.iter().enumerate().skip(1) {
        weight = count
            .simd_eq(TipRow::splat(index as i32))
            .select(TipRow::splat(value), weight);
    }
    weight
}

fn scale_tip_cell(cell: ProjectedTemporalMotionCell, references: TipReferencePair) -> TipSample {
    let ref_offset = references.ref_offset as i16;
    if !cell.valid {
        return TipSample::from_array([0, 0, 0, ref_offset]);
    }
    let mv = project_tmvp_mv(cell.mv(), references.ref_offset, cell.ref_offset());
    let clamp = |value: i32| value.clamp(-REFMVS_LIMIT, REFMVS_LIMIT) as i16;
    TipSample::from_array([1, clamp(mv.row), clamp(mv.col), ref_offset])
}

/// Copies each sample into its invalid neighbours inside the unit, in raster
/// order, so a filled sample can fill further ones.
fn fill_tip_holes(grid: &mut [TipSample], rows: usize, cols: usize) {
    for row in 1..=rows {
        for col in 1..=cols {
            let index = row * TIP_GRID + col;
            let source = grid[index];
            let mut fill = |destination: usize| {
                if grid[destination][0] == 0 {
                    grid[destination] = source;
                }
            };
            if row > 1 {
                fill(index - TIP_GRID);
            }
            if col > 1 {
                fill(index - 1);
            }
            if row < rows {
                fill(index + TIP_GRID);
            }
            if col < cols {
                fill(index + 1);
            }
        }
    }
}

/// One sample's § 7.10.4 average over itself and its four neighbours.
fn average_tip_sample(grid: &[TipSample], index: usize) -> ProjectedTemporalMotionCell {
    let sum = grid[index]
        + grid[index - TIP_GRID]
        + grid[index - 1]
        + grid[index + TIP_GRID]
        + grid[index + 1];
    let count = sum[0] as usize;
    if count == 0 {
        return ProjectedTemporalMotionCell::default();
    }
    ProjectedTemporalMotionCell::new(
        true,
        Mv {
            row: divide_tip_average(i32::from(sum[1]), count),
            col: divide_tip_average(i32::from(sum[2]), count),
        },
        i32::from(grid[index][3]),
    )
}

#[doc = "AV2 § 7.10.4 Weight_Div_Mult motion-vector average."]
pub(super) fn divide_tip_average(value: i32, count: usize) -> i32 {
    round2_signed(i64::from(value) * i64::from(TIP_AVERAGE_WEIGHTS[count]), 16) as i32
}

/// AV2 § 7.10.4 `Weight_Div_Mult` per sample count.
const TIP_AVERAGE_WEIGHTS: [i32; 6] = [0, 65_536, 32_768, 21_845, 16_384, 13_107];
