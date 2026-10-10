// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::ops::Range;
use std::simd::{
    Mask, Simd,
    cmp::{SimdOrd, SimdPartialEq, SimdPartialOrd},
    num::SimdInt,
};

use super::{
    Mv, ProjectedFieldBand, ProjectedTemporalMotionCell, TemporalMotionCell, TemporalMotionRows,
    TemporalProjectionSource, TrajectoryBand,
};

const LANES: usize = 16;
const GROUPS: usize = 4;
const CHUNK: usize = LANES * GROUPS;

type Lanes = Simd<i32, LANES>;
type Narrow = Simd<i16, LANES>;
type Grid<T> = [[T; LANES]; GROUPS];

/// One source reference's AV2 § 7.9.3 projection constants, indexed by the
/// reference a source cell stores. An index without an order hint is all false.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ProjectionTarget {
    pub(super) end_ref: Option<usize>,
    pub(super) ref_offset: i32,
    /// The projection factor with the sign of `ref_offset` folded in, which
    /// projects the stored vector exactly as the factor projects its negation.
    pub(super) factor: i32,
    pub(super) hint_match: bool,
    /// [`PROJECTS`] when the reference admits a projection factor, and
    /// [`INTERSECTS`] when it maps to a current-frame reference.
    pub(super) flags: u8,
}

pub(super) const PROJECTS: u8 = 1;
pub(super) const INTERSECTS: u8 = 2;

/// The order-free values of up to [`CHUNK`] sampled cells of one source row,
/// reused chunk after chunk; lanes past a chunk's end keep stale values.
#[derive(Default)]
struct ChunkLanes {
    refs: Grid<u8>,
    flags: Grid<u8>,
    factors: Grid<i32>,
    compressed: [Grid<i8>; 2],
    mv: [Grid<i32>; 2],
    projected: [Grid<i32>; 2],
    position: [Grid<i32>; 2],
    /// Where the unprojected vector lands, the end of a § 7.9.8 trajectory.
    end: [Grid<i32>; 2],
}

/// The lanes of one chunk that project, that intersect, and whose trajectory
/// end lands inside the frame.
#[derive(Clone, Copy)]
struct ChunkMasks {
    projects: u64,
    intersects: u64,
    ends: u64,
}

/// Fixed geometry the per-lane arithmetic of one scan shares.
#[derive(Clone, Copy)]
struct LaneGeometry {
    step: usize,
    step_mask: i32,
    unit_mask: i32,
    horizontal_offset8: i32,
    /// Columns a source cell may sit from the left edge of its window.
    window8: u32,
    width8: u32,
    height8: u32,
}

impl LaneGeometry {
    fn new(step: usize, unit_size8: usize, (width8, height8): (usize, usize)) -> Option<Self> {
        let step = step.clamp(1, 2);
        let unit_size8 = unit_size8.max(1);
        debug_assert!(unit_size8.is_power_of_two());
        let unit = i32::try_from(unit_size8).ok()?;
        let horizontal_offset8 = if step > 1 { unit } else { unit / 2 };
        Some(Self {
            step,
            step_mask: step as i32 - 1,
            unit_mask: unit - 1,
            horizontal_offset8,
            window8: u32::try_from(unit + 2 * horizontal_offset8).ok()?,
            width8: u32::try_from(i32::try_from(width8).ok()?).ok()?,
            height8: u32::try_from(i32::try_from(height8).ok()?).ok()?,
        })
    }
}

/// Projects `rows` of one source motion field into the current frame's field.
///
/// Every write this makes — projected cell, trajectory field, trajectory
/// position — lands in the TMVP unit row the scanned cell belongs to: AV2
/// § 7.9.8 admits a sample only when the source row sits inside the projected
/// position's unit, and its vertical bound carries no offset. A caller may
/// therefore replay disjoint unit-aligned row bands in any order and observe
/// the whole-field result.
///
/// Each row is scanned in chunks: the arithmetic every cell needs is computed
/// lane-parallel first, then only cells with an order-dependent effect are
/// walked, in raster order.
pub(super) fn project_temporal_motion_field(
    prepared: &TemporalProjectionSource,
    source: &impl TemporalMotionRows,
    rows: Range<usize>,
    projection_step: usize,
    tmvp_unit_size8: usize,
    mut trajectories: Option<&mut TrajectoryBand<'_>>,
    output: &mut ProjectedFieldBand<'_>,
) {
    debug_assert_eq!(
        (prepared.source_width8, prepared.source_height8),
        source.dimensions8()
    );
    let Some(geometry) = LaneGeometry::new(
        projection_step,
        tmvp_unit_size8,
        (output.width8, output.height8),
    ) else {
        return;
    };
    let step = geometry.step;
    let side = prepared.side & 1;
    let rows = rows.start..rows.end.min(prepared.source_height8);
    let mut lanes = ChunkLanes::default();
    for y8 in rows.step_by(step) {
        let Some(row) = source.row(y8) else {
            continue;
        };
        for (chunk_index, chunk) in row.chunks(CHUNK * step).enumerate() {
            let x_base = chunk_index * CHUNK * step;
            let Some(masks) = chunk_lanes(
                &mut lanes,
                chunk,
                side,
                &prepared.targets,
                y8,
                x_base,
                geometry,
            ) else {
                return;
            };
            let walk = ChunkWalk {
                lanes: &lanes,
                masks,
                y8,
                x_base,
                step,
            };
            match trajectories.as_deref_mut() {
                Some(trajectories) => walk.project_tracked(prepared, trajectories, output),
                None => walk.project(prepared, output),
            }
        }
    }
}

/// One chunk's lanes, walked in raster order for their order-dependent effects.
struct ChunkWalk<'a> {
    lanes: &'a ChunkLanes,
    masks: ChunkMasks,
    y8: usize,
    x_base: usize,
    step: usize,
}

impl ChunkWalk<'_> {
    fn pair(grid: &[Grid<i32>; 2], lane: usize) -> (i32, i32) {
        (grid[0].as_flattened()[lane], grid[1].as_flattened()[lane])
    }

    fn mv(&self, lane: usize) -> Mv {
        let (row, col) = Self::pair(&self.lanes.mv, lane);
        Mv { row, col }
    }

    fn position(&self, lane: usize) -> (usize, usize) {
        let (y8, x8) = Self::pair(&self.lanes.position, lane);
        (y8 as usize, x8 as usize)
    }

    fn target<'t>(
        &self,
        targets: &'t [ProjectionTarget],
        lane: usize,
    ) -> Option<&'t ProjectionTarget> {
        let reference = self.lanes.refs.as_flattened().get(lane)?;
        targets.get(usize::from(*reference))
    }

    /// The walk with trajectories: every intersecting lane runs the § 7.9.8
    /// intersection check, then every projecting lane observes and writes.
    #[inline(never)]
    fn project_tracked(
        &self,
        prepared: &TemporalProjectionSource,
        trajectories: &mut TrajectoryBand<'_>,
        output: &mut ProjectedFieldBand<'_>,
    ) {
        let ChunkMasks {
            projects,
            intersects,
            ends,
        } = self.masks;
        let mut walk = projects | intersects;
        while walk != 0 {
            let lane = walk.trailing_zeros() as usize;
            walk &= walk - 1;
            let Some(target) = self.target(&prepared.targets, lane) else {
                continue;
            };
            let at = (self.y8, self.x_base + lane * self.step);
            let mv = self.mv(lane);
            let trajectory_target_position = if target.flags & INTERSECTS != 0 {
                let (end_y8, end_x8) = Self::pair(&self.lanes.end, lane);
                let end_position =
                    (ends >> lane & 1 != 0).then_some((end_y8 as usize, end_x8 as usize));
                trajectories.check_intersection_at(
                    prepared.source_ref,
                    target.end_ref,
                    at,
                    mv,
                    end_position,
                )
            } else {
                None
            };
            if projects >> lane & 1 == 0 {
                continue;
            }
            let ref_offset = target.ref_offset.abs();
            let position = self.position(lane);
            if trajectories.admits_projection(
                target.end_ref,
                prepared.target_ref,
                position,
                ref_offset,
            ) {
                let (row, col) = Self::pair(&self.lanes.projected, lane);
                trajectories.observe_projection_at(
                    prepared.source_ref,
                    target.end_ref,
                    prepared.target_ref,
                    at.0,
                    at.1,
                    mv,
                    Mv { row, col },
                    position,
                    trajectory_target_position,
                    prepared.source_to_current,
                    ref_offset,
                    prepared.side & 1 == 1,
                );
            }
            write_projection(output, target, mv, position);
        }
    }

    /// The walk without trajectories: only projecting lanes have an effect.
    #[inline(never)]
    fn project(&self, prepared: &TemporalProjectionSource, output: &mut ProjectedFieldBand<'_>) {
        let mut walk = self.masks.projects;
        while walk != 0 {
            let lane = walk.trailing_zeros() as usize;
            walk &= walk - 1;
            if let Some(target) = self.target(&prepared.targets, lane) {
                write_projection(output, target, self.mv(lane), self.position(lane));
            }
        }
    }
}

/// Writes one projection unless the projected cell already holds an earlier
/// one that it does not replace.
fn write_projection(
    output: &mut ProjectedFieldBand<'_>,
    target: &ProjectionTarget,
    mv: Mv,
    (pos_y8, pos_x8): (usize, usize),
) {
    let Some(output_cell) = pos_y8
        .checked_sub(output.row_base)
        .and_then(|row| output.cells.get_mut(row * output.width8 + pos_x8))
    else {
        return;
    };
    let ref_offset = target.ref_offset.abs();
    let replace =
        !output_cell.valid || (target.hint_match && output_cell.ref_offset() != ref_offset);
    if replace {
        let mv = if target.ref_offset < 0 {
            Mv {
                row: -mv.row,
                col: -mv.col,
            }
        } else {
            mv
        };
        *output_cell = ProjectedTemporalMotionCell::new(true, mv, ref_offset);
    }
}

/// Reads the sampled cells of one chunk and computes, for each, its
/// decompressed vector, its projection, its sampled position and its
/// trajectory end.
#[inline(never)]
fn chunk_lanes(
    lanes: &mut ChunkLanes,
    chunk: &[TemporalMotionCell],
    side: usize,
    targets: &[ProjectionTarget],
    y8: usize,
    x_base: usize,
    geometry: LaneGeometry,
) -> Option<ChunkMasks> {
    let miss = u8::try_from(targets.len().checked_sub(1)?).ok()?;
    let count = chunk.len().div_ceil(geometry.step).min(CHUNK);
    let refs: &mut [u8; CHUNK] = lanes.refs.as_flattened_mut().try_into().ok()?;
    let flags: &mut [u8; CHUNK] = lanes.flags.as_flattened_mut().try_into().ok()?;
    let factors: &mut [i32; CHUNK] = lanes.factors.as_flattened_mut().try_into().ok()?;
    let [rows, cols] = &mut lanes.compressed;
    let rows: &mut [i8; CHUNK] = rows.as_flattened_mut().try_into().ok()?;
    let cols: &mut [i8; CHUNK] = cols.as_flattened_mut().try_into().ok()?;
    for lane in 0..count {
        let Some(cell) = chunk.get(lane * geometry.step) else {
            break;
        };
        let reference = cell.ref_indices[side].min(miss);
        let target = targets.get(usize::from(reference))?;
        refs[lane] = reference;
        flags[lane] = target.flags;
        factors[lane] = target.factor;
        rows[lane] = cell.mvs[side].row;
        cols[lane] = cell.mvs[side].col;
    }
    let iota = Lanes::from_array(core::array::from_fn(|lane| lane as i32));
    let y = Lanes::splat(i32::try_from(y8).ok()?);
    let y_unit = y & Lanes::splat(!geometry.unit_mask);
    let (mut projects, mut intersects, mut ends) = (0u64, 0u64, 0u64);
    for group in 0..count.div_ceil(LANES) {
        let first = i32::try_from(x_base + group * LANES * geometry.step).ok()?;
        let x = Lanes::splat(first) + iota * Lanes::splat(geometry.step as i32);
        let factor = Lanes::from_array(lanes.factors[group]);
        let (mut sampled, mut ended) = (Mask::splat(true), Mask::splat(true));
        let mut base = [Lanes::splat(0); 2];
        for (component, base) in base.iter_mut().enumerate() {
            let saved = Simd::<i8, LANES>::from_array(lanes.compressed[component][group]);
            let mv: Lanes = decompress(saved.cast()).cast();
            lanes.mv[component][group] = mv.to_array();
            let projected = project(mv, factor);
            lanes.projected[component][group] = projected.to_array();
            let (origin, limit) = if component == 0 {
                (y, geometry.height8)
            } else {
                (x, geometry.width8)
            };
            let inside = |position: Lanes| position.cast::<u32>().simd_lt(Simd::splat(limit));
            let position = origin + divide_by_64(projected);
            sampled &= inside(position);
            let position = position & Lanes::splat(!geometry.step_mask);
            lanes.position[component][group] = position.to_array();
            *base = position & Lanes::splat(!geometry.unit_mask);
            let end = origin + divide_by_64(mv);
            ended &= inside(end);
            lanes.end[component][group] = (end & Lanes::splat(!geometry.step_mask)).to_array();
        }
        let column = x - base[1] + Lanes::splat(geometry.horizontal_offset8);
        let near =
            base[0].simd_eq(y_unit) & column.cast::<u32>().simd_lt(Simd::splat(geometry.window8));
        let flags = Simd::<u8, LANES>::from_array(lanes.flags[group]);
        let projecting = (flags & Simd::splat(PROJECTS)).simd_ne(Simd::splat(0));
        let intersecting = (flags & Simd::splat(INTERSECTS)).simd_ne(Simd::splat(0));
        let shift = group * LANES;
        projects |= (projecting.to_bitmask() & (sampled & near).to_bitmask()) << shift;
        intersects |= intersecting.to_bitmask() << shift;
        ends |= ended.to_bitmask() << shift;
    }
    let live = u64::MAX >> (CHUNK - count.max(1));
    Some(ChunkMasks {
        projects: projects & live,
        intersects: intersects & live,
        ends,
    })
}

/// AV2 § 7.9.2 decompression of a saved vector component, lane by lane.
fn decompress(value: Narrow) -> Narrow {
    let magnitude = value.abs();
    let step_log2 = ((magnitude >> 4) - Narrow::splat(1)).simd_max(Narrow::splat(0));
    let decompressed = (magnitude - (step_log2 << 4)) << step_log2;
    let sign = value >> 15;
    (decompressed ^ sign) - sign
}

/// `Round2Signed(component * factor, 14)` of a decompressed component. A
/// magnitude of at most 2048 keeps the product inside `i32` and the result
/// under `MV_LIMIT` for every admitted factor, so the scalar clamp never binds.
fn project(component: Lanes, factor: Lanes) -> Lanes {
    let scaled = component * factor;
    (scaled + (scaled >> 31) + Lanes::splat(1 << 13)) >> 14
}

/// Division by 64 that truncates toward zero, as the scalar `/` does.
fn divide_by_64(value: Lanes) -> Lanes {
    (value + ((value >> 31) & Lanes::splat(63))) >> 6
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::{
        CompressedTemporalMv, MAX_FRAME_DISTANCE, MAX_SORTED_REFS, MotionFieldLayout,
        RefOrderHints, TemporalMotionFieldMetadata, project_no_constraint,
        project_tmvp_mv_with_factor, sampled_temporal_position, tmvp_projection_factor,
        uncompress_tmvp_mv,
    };
    use super::*;
    use crate::prediction::inter::get_relative_dist;

    #[test]
    fn lane_projection_matches_the_scalar_projection_for_every_saved_component() {
        let saved: Vec<i32> = (-128..128).collect();
        for numerator in -MAX_FRAME_DISTANCE..=MAX_FRAME_DISTANCE {
            for ref_offset in -MAX_FRAME_DISTANCE..=MAX_FRAME_DISTANCE {
                let side = usize::from(ref_offset < 0);
                let factor = tmvp_projection_factor(numerator, ref_offset, side).unwrap();
                let folded = if ref_offset < 0 { -factor } else { factor };
                for chunk in saved.chunks_exact(LANES) {
                    let narrow =
                        Narrow::from_array(core::array::from_fn(|lane| chunk[lane] as i16));
                    let decompressed: Lanes = decompress(narrow).cast();
                    let projected = project(decompressed, Lanes::splat(folded));
                    for (lane, &value) in chunk.iter().enumerate() {
                        let component = value as i8;
                        let mv = uncompress_tmvp_mv(CompressedTemporalMv {
                            row: component,
                            col: component,
                        });
                        let mv = if ref_offset < 0 {
                            Mv {
                                row: -mv.row,
                                col: -mv.col,
                            }
                        } else {
                            mv
                        };
                        let expected =
                            project_tmvp_mv_with_factor(mv, numerator, ref_offset.abs(), factor);
                        assert_eq!(decompressed[lane].abs(), mv.row.abs());
                        assert_eq!(
                            projected[lane], expected.row,
                            "saved {value} numerator {numerator} offset {ref_offset}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn chunk_lanes_match_the_scalar_cell_projection() {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let mut lanes = ChunkLanes::default();
        for case in 0..3000 {
            let step = 1 + case % 2;
            let unit = 1 << next(5);
            let (height8, width8) = (1 + next(150), 1 + next(300));
            let hint =
                |next: &mut dyn FnMut(usize) -> usize| (next(5) != 0).then(|| next(64) as u32);
            let hints: Vec<Option<u32>> = (0..7).map(|_| hint(&mut next)).collect();
            let source_hints: Vec<Option<u32>> =
                (0..7 + next(6)).map(|_| hint(&mut next)).collect();
            let mut ref_order_hints = RefOrderHints::default();
            ref_order_hints.extend_within(source_hints.iter().copied());
            let metadata = TemporalMotionFieldMetadata {
                is_inter: true,
                frame_size: None,
                ref_order_hints,
            };
            let layout = MotionFieldLayout::new(height8 * 2, width8 * 2, 16).unwrap();
            let (source_hint, side) = (next(64) as u32, next(2));
            let source = TemporalProjectionSource::new(
                &metadata,
                layout,
                source_hint,
                next(64) as u32,
                0,
                side,
                None,
                &hints,
            )
            .unwrap();
            let y8 = next(height8) & !(step - 1);
            let x_base = next(width8.div_ceil(CHUNK * step)) * CHUNK * step;
            let wide = next(2) == 0;
            let cells: Vec<TemporalMotionCell> = (0..=next(CHUNK * step))
                .map(|_| {
                    let mut component = || {
                        if wide {
                            next(256) as i32 - 128
                        } else {
                            next(41) as i32 - 20
                        }
                    };
                    let mvs = [(); 2].map(|()| CompressedTemporalMv {
                        row: component() as i8,
                        col: component() as i8,
                    });
                    let reference = if next(8) == 0 { 255 } else { next(12) as u8 };
                    TemporalMotionCell {
                        ref_indices: [reference; 2],
                        mvs,
                    }
                })
                .collect();
            let geometry = LaneGeometry::new(step, unit, (width8, height8)).unwrap();
            let masks = chunk_lanes(
                &mut lanes,
                &cells,
                side,
                &source.targets,
                y8,
                x_base,
                geometry,
            )
            .unwrap();
            for (lane, cell) in cells.iter().step_by(step).enumerate() {
                let x8 = x_base + lane * step;
                let reference = usize::from(cell.ref_indices[side]).min(MAX_SORTED_REFS);
                let mv = uncompress_tmvp_mv(cell.mvs[side]);
                let lane_mv = Mv {
                    row: lanes.mv[0].as_flattened()[lane],
                    col: lanes.mv[1].as_flattened()[lane],
                };
                assert_eq!(lane_mv, mv, "case {case} lane {lane}");
                let hint = source_hints
                    .get(reference)
                    .filter(|_| reference < MAX_SORTED_REFS);
                let entry = hint.copied().flatten().map(|hint| {
                    let offset = get_relative_dist(source_hint as i32, hint as i32);
                    (
                        offset,
                        tmvp_projection_factor(source.source_to_current, offset, side),
                    )
                });
                let target = source.targets[reference];
                assert_eq!(
                    masks.intersects >> lane & 1 == 1,
                    entry.is_some() && target.end_ref.is_some()
                );
                let expected = entry.and_then(|(offset, factor)| {
                    let mv = if offset < 0 {
                        Mv {
                            row: -mv.row,
                            col: -mv.col,
                        }
                    } else {
                        mv
                    };
                    let projected = project_tmvp_mv_with_factor(
                        mv,
                        source.source_to_current,
                        offset.abs(),
                        factor?,
                    );
                    sampled_temporal_position(y8, x8, projected, step, unit, (width8, height8))
                        .map(|position| (projected, position))
                });
                let end = [(y8, mv.row, height8), (x8, mv.col, width8)]
                    .map(|(origin, delta, max)| project_no_constraint(origin, delta, max));
                assert_eq!(masks.ends >> lane & 1 == 1, end.iter().all(Option::is_some));
                if let [Some(end_y8), Some(end_x8)] = end {
                    assert_eq!(
                        lanes.end[0].as_flattened()[lane] as usize,
                        end_y8 & !(step - 1)
                    );
                    assert_eq!(
                        lanes.end[1].as_flattened()[lane] as usize,
                        end_x8 & !(step - 1)
                    );
                }
                assert_eq!(
                    masks.projects >> lane & 1 == 1,
                    expected.is_some(),
                    "case {case}"
                );
                if let Some((projected, (pos_y8, pos_x8))) = expected {
                    let lane_projected = Mv {
                        row: lanes.projected[0].as_flattened()[lane],
                        col: lanes.projected[1].as_flattened()[lane],
                    };
                    assert_eq!(lane_projected, projected, "case {case} lane {lane}");
                    assert_eq!(lanes.position[0].as_flattened()[lane] as usize, pos_y8);
                    assert_eq!(lanes.position[1].as_flattened()[lane] as usize, pos_x8);
                }
            }
        }
    }
}
