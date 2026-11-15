// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.10.4 TIP motion field: the projected field scaled to the TIP pair,
//! hole-filled and averaged inside each TMVP unit.

use fearless_simd::{Simd, i32x8, i32x16, prelude::*};

use super::{
    DIV_MULT, MAX_FRAME_DISTANCE, ProjectedTemporalMotionCell, ProjectedTemporalMotionField,
    REFMVS_LIMIT, TipReferencePair, fill_temporal_sampling_gaps,
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
/// written back once. A row scales lane-parallel: a component times its
/// `DIV_MULT` fits `i32`, and split at bit 14 its product by the clamped
/// numerator keeps the exact `Round2Signed` in `i32` too. An empty average
/// is the default cell, because its vector sums to zero.
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
                for (lane, cell) in cells.enumerate() {
                    let denominator = cell.ref_offset().clamp(0, MAX_FRAME_DISTANCE) as usize;
                    lanes[0][lane] = i32::from(cell.valid);
                    lanes[1][lane] = i32::from(cell.mv[0]);
                    lanes[2][lane] = i32::from(cell.mv[1]);
                    lanes[3][lane] = DIV_MULT[denominator];
                }
                let [valid, mv_row, mv_col, divisor] = lanes.each_ref().map(|lanes| load(lanes, 0));
                let mut count = splat(0);
                for chunk in valid {
                    count += chunk;
                }
                valid_count += count.reduce_sum();
                let scale = |mv: TipRow<S, C>| -> TipRow<S, C> {
                    core::array::from_fn(|chunk| {
                        let scaled = mv[chunk] * divisor[chunk];
                        let negative = (scaled * splat(numerator.signum())) >> 31;
                        let low = (scaled & splat(0x3fff)) * splat(numerator);
                        let product = (scaled >> 14) * splat(numerator)
                            + ((low + splat(1 << 13) + negative) >> 14);
                        product.max(-limit).min(limit) & -valid[chunk]
                    })
                };
                let scaled = [valid, scale(mv_row), scale(mv_col)];
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
                fill_tip_holes(simd, &mut planes, rows, cols);
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

/// Copies each sample into its invalid neighbours inside the unit, in raster
/// order, so a filled sample can fill further ones.
///
/// The raster pass settles row by row. A sample enters its row's scan as
/// `own`: its value, else the scan value of the sample above. Scanning left
/// to right, it holds the nearest valid `own` on or before it (`swept`),
/// which it also hands up and down. A sample with none takes `own` of its
/// right neighbour (`held`), and one still invalid takes `swept` of the
/// sample below. Lanes past `cols` stay zero.
///
/// A sample travels packed in one lane as `row << 20 | col << 8 | valid`:
/// both components are clamped to `REFMVS_LIMIT`, so they fit 12 bits, and
/// only an invalid sample packs to zero.
fn fill_tip_holes<S: Simd>(simd: S, planes: &mut TipPlanes, rows: usize, cols: usize) {
    let zero = i32x16::splat(simd, 0);
    let lane = i32x16::load_array(simd, core::array::from_fn(|lane| lane as i32));
    let inside = lane.simd_lt(i32x16::splat(simd, cols as i32));
    let pick = |a: i32x16<S>, b: i32x16<S>| a.simd_ne(zero).select(a, b);
    let load = |planes: &TipPlanes, at: usize| {
        let [valid, row, col] = planes.each_ref().map(|plane| {
            plane
                .get(at..)
                .and_then(<[i32]>::first_chunk::<MAX_TIP_UNIT8>)
                .map_or(zero, |lanes| i32x16::from_slice(simd, lanes))
        });
        (row << 20) | ((col & 0xfff) << 8) | valid
    };
    let (mut swept_above, mut held_above) = (zero, zero);
    for row in 1..=rows {
        let at = row * TIP_GRID + 1;
        let own = pick(load(planes, at), swept_above);
        let mut swept = own;
        if (own.simd_eq(zero) & inside).any_true() {
            swept = pick(swept, zero.slide::<15>(swept));
            swept = pick(swept, zero.slide::<14>(swept));
            swept = pick(swept, zero.slide::<12>(swept));
            swept = inside.select(pick(swept, zero.slide::<8>(swept)), zero);
        }
        let held = pick(swept, own.slide::<1>(zero));
        if row > 1 {
            store_tip_lanes(planes, at - TIP_GRID, pick(held_above, swept));
        }
        (swept_above, held_above) = (swept, held);
    }
    store_tip_lanes(planes, rows * TIP_GRID + 1, held_above);
}

/// Unpacks a row of [`fill_tip_holes`] samples into the planes at `at`.
fn store_tip_lanes<S: Simd>(planes: &mut TipPlanes, at: usize, packed: i32x16<S>) {
    let lanes = [packed & 1, packed >> 20, (packed << 12) >> 20];
    for (plane, value) in planes.iter_mut().zip(lanes) {
        if let Some(slot) = plane
            .get_mut(at..)
            .and_then(<[i32]>::first_chunk_mut::<MAX_TIP_UNIT8>)
        {
            value.store_array(slot);
        }
    }
}

/// AV2 § 7.10.4 `Weight_Div_Mult` per sample count.
pub(super) const TIP_AVERAGE_WEIGHTS: [i32; 6] = [0, 65_536, 32_768, 21_845, 16_384, 13_107];

#[cfg(test)]
mod tests {
    use super::*;

    /// The raster pass of § 7.10.3 as written, one sample at a time.
    fn fill_tip_holes_scalar(planes: &mut TipPlanes, rows: usize, cols: usize) {
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

    #[test]
    fn vector_hole_fill_matches_the_raster_pass_on_random_masks() {
        let mut state = 0x9e37_79b9u32;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state >> 8
        };
        for case in 0..4000 {
            let rows = 1 + next() as usize % MAX_TIP_UNIT8;
            let cols = 1 + next() as usize % MAX_TIP_UNIT8;
            let density = [1, 2, 8, 30, 60][case % 5];
            let mut planes: TipPlanes = [[0; TIP_GRID * TIP_GRID]; 3];
            for row in 1..=rows {
                for col in 1..=cols {
                    if (next() % 64) < density {
                        let at = row * TIP_GRID + col;
                        planes[0][at] = 1;
                        planes[1][at] = next() as i32 % 4095 - 2047;
                        planes[2][at] = [-2047, 2047, 0, 5][case % 4];
                    }
                }
            }
            let mut want = planes;
            fill_tip_holes_scalar(&mut want, rows, cols);
            crate::baseline_simd!(simd => fill_tip_holes(simd, &mut planes, rows, cols));
            assert!(planes == want, "case {case}: {rows}x{cols}");
        }
    }
    /// Lane scaling of every `i16` vector against the scalar § 7.9.2
    /// projection, clamped to `REFMVS_LIMIT`.
    #[test]
    fn tip_scaling_matches_the_scalar_projection_for_every_vector() {
        let mut state = 0x2545_f491u32;
        let next = |state: &mut u32| {
            *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *state >> 8
        };
        let extremes = [i16::MIN, i16::MAX, -2048, 2048, 2047, -2047, 0, 1, -1];
        let component = |state: &mut u32| match next(state) % 3 {
            0 => extremes[next(state) as usize % extremes.len()],
            1 => (next(state) % 65_536) as u16 as i16,
            _ => (next(state) % 4097) as i16 - 2048,
        };
        for case in 0..400 {
            let ref_offset = next(&mut state) as i32 % 81 - 40;
            let references = TipReferencePair {
                past_ref: 0,
                future_ref: 1,
                past_offset: 0,
                future_offset: 0,
                ref_offset,
            };
            let cells: Vec<_> = (0..64)
                .map(|_| ProjectedTemporalMotionCell {
                    valid: next(&mut state) % 4 != 0,
                    ref_offset: (next(&mut state) % 40) as i16 - 4,
                    mv: [component(&mut state), component(&mut state)],
                })
                .collect();
            let mut field = ProjectedTemporalMotionField {
                width8: 8,
                height8: 8,
                cells: cells.clone(),
            };
            let prepared = crate::baseline_simd!(simd => prepare_tip_units::<_, 1, 1>(
                simd, &mut field, references, 8, false,
            ));
            assert!(prepared.is_ok(), "case {case}: {prepared:?}");
            for (index, (got, cell)) in field.cells.iter().zip(&cells).enumerate() {
                let want = if cell.valid {
                    let mv = super::super::project_tmvp_mv(
                        cell.mv(),
                        ref_offset,
                        i32::from(cell.ref_offset),
                    );
                    let clamp = |value: i32| value.clamp(-REFMVS_LIMIT, REFMVS_LIMIT) as i16;
                    [clamp(mv.row), clamp(mv.col)]
                } else {
                    [0, 0]
                };
                assert_eq!(got.mv, want, "case {case} cell {index} {cell:?}");
                assert_eq!(got.valid, cell.valid, "case {case} cell {index}");
            }
        }
    }
}
