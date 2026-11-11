// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.10.4 TIP motion field: the projected field scaled to the TIP pair,
//! hole-filled and averaged inside each TMVP unit.

use fearless_simd::{Simd, i32x8, prelude::*};

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
    let (unit, holes) = (tmvp_unit_size8, fill_holes);
    crate::baseline_simd!(simd => match projection_step {
        1 if unit <= 8 => prepare_tip_units::<_, 1, 1>(simd, field, references, unit, holes),
        1 => prepare_tip_units::<_, 2, 1>(simd, field, references, unit, holes),
        2 => prepare_tip_units::<_, 1, 2>(simd, field, references, unit, holes),
        _ => Err(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState.into()),
    })?;
    fill_temporal_sampling_gaps(field, projection_step, tmvp_unit_size8);
    Ok(())
}

/// Largest TMVP unit side, in 8x8 cells, that TIP preparation works inside.
const MAX_TIP_UNIT8: usize = 16;
/// Side of the sample grid: one unit plus a zero border, so every sample has
/// four neighbours and an absent one adds nothing to an average.
const TIP_GRID: usize = MAX_TIP_UNIT8 + 2;

/// One unit row of up to `8 * C` samples.
type TipRow<S, const C: usize> = [i32x8<S>; C];

/// A unit's sample valid flags and vector components as zero-bordered
/// planes. An invalid sample carries a zero vector, so summing samples sums
/// the valid vectors and counts them; every offset is the TIP pair's.
type TipPlanes = [[i32; TIP_GRID * TIP_GRID]; 3];

/// AV2 § 7.10.4 TIP motion, one TMVP unit at a time.
///
/// Every sampled cell is scaled to the TIP pair; holes are then filled and the
/// samples averaged. Both read only their own unit, so a unit's samples are
/// gathered into planes a row of up to `8 * C` at a time, settled there and
/// written back once. A row scales lane-parallel while no component exceeds
/// `REFMVS_LIMIT`, so every product stays inside `i32`; a wider row scales
/// through [`scale_tip_mv`]. An empty average is the default cell, because
/// its vector sums to zero.
#[inline(never)]
fn prepare_tip_units<S: Simd, const C: usize, const STEP: usize>(
    simd: S,
    field: &mut ProjectedTemporalMotionField,
    references: TipReferencePair,
    unit: usize,
    fill_holes: bool,
) -> crate::Result<()> {
    let (width8, height8) = (field.width8, field.height8);
    if unit == 0
        || !unit.is_multiple_of(STEP)
        || unit > 8 * C * STEP
        || 8 * C > MAX_TIP_UNIT8
        || width8.checked_mul(height8) != Some(field.cells.len())
    {
        return Err(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState.into());
    }
    let numerator = references
        .ref_offset
        .clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let offset = references.ref_offset as i16;
    let splat = |value: i32| i32x8::splat(simd, value);
    let limit = splat(REFMVS_LIMIT);
    let vector = |lanes: &[i32], at: usize| {
        lanes
            .get(at..)
            .and_then(<[i32]>::first_chunk::<8>)
            .map_or(splat(0), |lanes| i32x8::from_slice(simd, lanes))
    };
    let load = |plane: &[i32], at: usize| -> TipRow<S, C> {
        core::array::from_fn(|chunk| vector(plane, at + 8 * chunk))
    };
    let store = |plane: &mut [i32], at: usize, row: TipRow<S, C>| {
        for (chunk, value) in row.into_iter().enumerate() {
            if let Some(slot) = plane
                .get_mut(at + 8 * chunk..)
                .and_then(<[i32]>::first_chunk_mut::<8>)
            {
                value.store_array(slot);
            }
        }
    };
    let mut planes: TipPlanes = [[0; TIP_GRID * TIP_GRID]; 3];
    for block_y in (0..height8).step_by(unit) {
        let unit_rows = (height8 - block_y).min(unit);
        for block_x in (0..width8).step_by(unit) {
            let unit_cols = (width8 - block_x).min(unit);
            let (rows, cols) = (unit_rows.div_ceil(STEP), unit_cols.div_ceil(STEP));
            let mut valid_count = 0;
            for y in 0..rows {
                let start = (block_y + y * STEP) * width8 + block_x;
                let cells = field.cells[start..start + unit_cols].iter().step_by(STEP);
                let mut lanes = [[0; MAX_TIP_UNIT8]; 4];
                for (lane, cell) in cells.clone().enumerate() {
                    let denominator = cell.ref_offset().clamp(0, MAX_FRAME_DISTANCE) as usize;
                    lanes[0][lane] = i32::from(cell.valid);
                    lanes[1][lane] = i32::from(cell.mv[0]);
                    lanes[2][lane] = i32::from(cell.mv[1]);
                    lanes[3][lane] = DIV_MULT[denominator];
                }
                let [valid, mv_row, mv_col, divisor] = lanes.each_ref().map(|lanes| load(lanes, 0));
                let (mut count, mut wide) = (splat(0), false);
                for chunk in 0..C {
                    count += valid[chunk];
                    wide |= mv_row[chunk]
                        .abs()
                        .max(mv_col[chunk].abs())
                        .simd_gt(limit)
                        .any_true();
                }
                valid_count += count.reduce_sum();
                let scale = |mv: TipRow<S, C>| -> TipRow<S, C> {
                    core::array::from_fn(|chunk| {
                        let product = mv[chunk] * divisor[chunk] * splat(numerator);
                        round2_signed(product, 14).max(-limit).min(limit) & -valid[chunk]
                    })
                };
                let mut scaled = [valid, scale(mv_row), scale(mv_col)];
                if wide {
                    let mut wide = [[0; MAX_TIP_UNIT8]; 2];
                    for (lane, &cell) in cells.enumerate() {
                        let mv = scale_tip_mv(cell, references);
                        wide[0][lane] = mv.row;
                        wide[1][lane] = mv.col;
                    }
                    [scaled[1], scaled[2]] = wide.each_ref().map(|lanes| load(lanes, 0));
                }
                let at = (y + 1) * TIP_GRID + 1;
                for (plane, value) in planes.iter_mut().zip(scaled) {
                    store(plane, at, value);
                }
            }
            let below = (rows + 1) * TIP_GRID + 1;
            for plane in &mut planes {
                plane[below..below + 8 * C].fill(0);
            }
            if fill_holes && valid_count != 0 && valid_count != (rows * cols) as i32 {
                fill_tip_holes(&mut planes, rows, cols);
            }
            for y in 0..unit_rows {
                let start = (block_y + y) * width8 + block_x;
                let cells = &mut field.cells[start..start + unit_cols];
                if !y.is_multiple_of(STEP) {
                    cells.fill(ProjectedTemporalMotionCell::default());
                    continue;
                }
                let at = (y / STEP + 1) * TIP_GRID + 1;
                let rows = if fill_holes {
                    let cross = |plane: &[i32]| -> TipRow<S, C> {
                        let [c, up, left, down, right] =
                            [at, at - TIP_GRID, at - 1, at + TIP_GRID, at + 1]
                                .map(|from| load(plane, from));
                        core::array::from_fn(|i| c[i] + up[i] + left[i] + down[i] + right[i])
                    };
                    let count = cross(&planes[0]);
                    let weight: TipRow<S, C> = core::array::from_fn(|chunk| {
                        let mut weight = splat(0);
                        for (index, &value) in TIP_AVERAGE_WEIGHTS.iter().enumerate().skip(1) {
                            weight = count[chunk]
                                .simd_eq(splat(index as i32))
                                .select(splat(value), weight);
                        }
                        weight
                    });
                    let average = |plane: &[i32]| -> TipRow<S, C> {
                        let sum = cross(plane);
                        core::array::from_fn(|i| round2_signed(sum[i] * weight[i], 16))
                    };
                    [count, average(&planes[1]), average(&planes[2])]
                } else {
                    planes.each_ref().map(|plane| load(plane, at))
                };
                let [valid, mv_row, mv_col] = rows.map(|row| {
                    let mut lanes = [0; MAX_TIP_UNIT8];
                    store(&mut lanes, 0, row);
                    lanes
                });
                if STEP > 1 {
                    cells.fill(ProjectedTemporalMotionCell::default());
                }
                for (lane, cell) in cells.iter_mut().step_by(STEP).enumerate() {
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
#[allow(clippy::inline_always, reason = "TIP field lane arithmetic")]
#[inline(always)]
fn round2_signed<S: Simd>(value: i32x8<S>, shift: u32) -> i32x8<S> {
    (value + (1 << (shift - 1)) + (value >> 31)) >> shift
}

/// A sampled cell's vector scaled to the TIP pair, zero when invalid.
fn scale_tip_mv(cell: ProjectedTemporalMotionCell, references: TipReferencePair) -> Mv {
    if !cell.valid {
        return Mv::ZERO;
    }
    let mv = project_tmvp_mv(cell.mv(), references.ref_offset, cell.ref_offset());
    Mv {
        row: mv.row.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
        col: mv.col.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
    }
}

/// Copies each sample into its invalid neighbours inside the unit, in raster
/// order, so a filled sample can fill further ones.
fn fill_tip_holes(planes: &mut TipPlanes, rows: usize, cols: usize) {
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

/// AV2 § 7.10.4 `Weight_Div_Mult` per sample count.
pub(super) const TIP_AVERAGE_WEIGHTS: [i32; 6] = [0, 65_536, 32_768, 21_845, 16_384, 13_107];
