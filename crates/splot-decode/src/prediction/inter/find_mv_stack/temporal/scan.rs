// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::ops::Range;

use fearless_simd::{
    Level, Simd, dispatch, i16x8, i32x4, i32x16, mask32x16, prelude::*, u8x16, u32x4, u32x16, u64x2,
};

use super::trajectory::BandGeometry;
use super::{
    Mv, ProjectedFieldBand, ProjectedTemporalMotionCell, StoredTemporalCell, TemporalMotionRows,
    TemporalProjectionSource, TrajectoryBand,
};

const LANES: usize = 16;
const GROUPS: usize = 4;
const CHUNK: usize = LANES * GROUPS;
/// Bytes of the stored cells a chunk spans at the widest step.
const WINDOW: usize = CHUNK * 2 * size_of::<StoredTemporalCell>();

type Lanes<S> = i32x16<S>;
type Grid<T> = [[T; LANES]; GROUPS];

/// One source reference's AV2 § 7.9.3 projection constants, indexed by the
/// reference a source cell stores. An index without an order hint is all false.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ProjectionTarget {
    pub(super) end_ref: Option<usize>,
    pub(super) ref_offset: i32,
    /// `DIV_MULT[|ref_offset|]` with the sign of `ref_offset` when the reference
    /// admits a projection factor, else 0. Times `source_to_current` it is the
    /// projection factor with the offset sign folded in, which projects the
    /// stored vector exactly as the factor projects its negation.
    pub(super) divisor: i16,
    /// `DIV_MULT[|ref_offset|]` when the offset is a frame distance, else 0.
    /// With `end_numerator` it is the factor that projects the stored vector
    /// onto the § 7.9.8 trajectory end, the vector `observe_projection_at`
    /// records for `end_ref`.
    pub(super) end_divisor: i16,
    /// `clamp(|ref_offset| - observed, ±MAX_FRAME_DISTANCE)`, the numerator of
    /// the same projection.
    pub(super) end_numerator: i16,
    /// Whether `end_ref` is the projection's target reference.
    pub(super) end_is_target: bool,
    /// [`PROJECTS`] when the reference admits a projection factor, and
    /// [`INTERSECTS`] when it maps to a current-frame reference.
    pub(super) flags: u8,
}

pub(super) const PROJECTS: u8 = 1;
pub(super) const INTERSECTS: u8 = 2;

/// The trajectory end of an intersecting lane is in the scanned cell's TMVP
/// unit row and in the band, so the reverse § 7.9.8 walk reads its record.
const REVERSE: u8 = 1;
/// The trajectory end of an intersecting lane is in the band and inside the
/// projected position's window, so an observation records it for the end.
const END_WRITE: u8 = 2;
/// The lane projects.
const PROJECT: u8 = 4;
/// The lane runs the § 7.9.8 intersection check.
const INTERSECT: u8 = 8;
/// The lane projects from a cell inside the trajectory grid, so it observes.
const OBSERVE: u8 = 16;

/// Rows of [`ChunkLanes::ints`]: the decompressed vector, its projection and
/// the sampled position (two rows each, row then column).
const MV: usize = 0;
const PROJECTED: usize = 2;
const POSITION: usize = 4;
/// The projected position's cell in the output band, past it when the
/// position sits above the band.
const OUTPUT_CELL: usize = 6;
/// The same cell in the trajectory band.
const TRAJECTORY_CELL: usize = 7;
/// The trajectory band cell where the unprojected vector lands, the end of a
/// § 7.9.8 trajectory.
const END_CELL: usize = 8;
/// The § 7.9.8 trajectory-end projection of the decompressed vector (two
/// rows), tracked scans only.
const END_MV: usize = 10;
const INT_ROWS: usize = 12;
/// One lane's view of [`ChunkLanes::ints`], from its first row to its last.
const LANE_SPAN: usize = (INT_ROWS - 1) * CHUNK + 1;

/// The order-free values of up to [`CHUNK`] sampled cells of one source row,
/// reused chunk after chunk; lanes past a chunk's end keep stale values.
#[derive(Default)]
struct ChunkLanes {
    window: Window,
    refs: Grid<u8>,
    /// One row per integer value, so one lane reads every value at a fixed
    /// offset from one address.
    ints: [Grid<i32>; INT_ROWS],
    /// [`REVERSE`], [`END_WRITE`], [`PROJECT`], [`INTERSECT`] and [`OBSERVE`]
    /// for a tracked scan.
    bits: Grid<u8>,
}

/// One lane's integer values.
#[derive(Clone, Copy)]
struct LaneInts<'a>(&'a [i32; LANE_SPAN]);

impl LaneInts<'_> {
    fn get(self, row: usize) -> i32 {
        self.0[row * CHUNK]
    }

    fn pair(self, row: usize) -> (i32, i32) {
        (self.get(row), self.get(row + 1))
    }

    /// A band cell; -1 reads as a cell past every band.
    fn cell(self, row: usize) -> usize {
        self.get(row) as u32 as usize
    }

    fn mv(self) -> Mv {
        let (row, col) = self.pair(MV);
        Mv { row, col }
    }
}

/// One source's [`ProjectionTarget`] flags and divisor bytes as lane tables
/// indexed by the stored reference. Entries past the targets, and lookups past
/// the table, read 0, as the empty target does, except that every reference
/// admits [`REVERSE`] and [`END_WRITE`]. [`INTERSECT`] is admitted only when
/// the source and the end reference both index the trajectory grid.
struct TargetTables<S: Simd> {
    simd: S,
    /// The lane bits each reference admits.
    allow: u8x16<S>,
    divisor: [u8x16<S>; 2],
    end_divisor: [u8x16<S>; 2],
    end_numerator: u8x16<S>,
    source_to_current: i32,
}

impl<S: Simd> TargetTables<S> {
    fn new(simd: S, prepared: &TemporalProjectionSource, trajectory_refs: usize) -> Self {
        let entry = |index: usize| prepared.targets.get(index).copied().unwrap_or_default();
        let byte = |index: usize, shift: u32| (entry(index).divisor >> shift) as u8;
        let end_byte = |index: usize, shift: u32| (entry(index).end_divisor >> shift) as u8;
        let end_num = |index: usize| entry(index).end_numerator as u8;
        let tracked = |target: ProjectionTarget| {
            prepared.source_ref < trajectory_refs
                && target.end_ref.is_some_and(|end| end < trajectory_refs)
        };
        let allow = |index: usize| {
            let target = entry(index);
            let projects = if target.flags & PROJECTS != 0 {
                PROJECT | OBSERVE
            } else {
                0
            };
            let intersects = target.flags & INTERSECTS != 0 && tracked(target);
            projects | if intersects { INTERSECT } else { 0 }
        };
        Self {
            simd,
            allow: u8x16::from_fn(simd, allow),
            divisor: [0, 8].map(|shift| u8x16::from_fn(simd, |i| byte(i, shift))),
            end_divisor: [0, 8].map(|shift| u8x16::from_fn(simd, |i| end_byte(i, shift))),
            end_numerator: u8x16::from_fn(simd, end_num),
            source_to_current: prepared.source_to_current,
        }
    }

    /// The folded factor of `table` at each lane's reference, signed bytes
    /// reassembled into one `i16` per lane.
    fn folded(&self, [low, high]: [u8x16<S>; 2], refs: u8x16<S>) -> Lanes<S> {
        let (low, high) = (
            low.swizzle_dyn_precise(refs),
            high.swizzle_dyn_precise(refs),
        );
        widen_i16(
            self.simd,
            low.zip_low(high).bitcast(),
            low.zip_high(high).bitcast(),
        )
    }

    /// The admitted lane bits, the folded projection factor and the § 7.9.8
    /// trajectory-end factor of each lane's reference.
    fn lookup(&self, refs: u8x16<S>) -> (u8x16<S>, Lanes<S>, Lanes<S>) {
        let factor = self.folded(self.divisor, refs) * self.source_to_current;
        let end_numerator = widen_i8(self.simd, self.end_numerator.swizzle_dyn_precise(refs));
        let end_factor = self.folded(self.end_divisor, refs) * end_numerator;
        let allow = u8x16::splat(self.simd, REVERSE | END_WRITE);
        (
            self.allow.swizzle_dyn_precise(refs) | allow,
            factor,
            end_factor,
        )
    }
}

/// A row band's origin and width, which turn a position into its band cell.
#[derive(Clone, Copy, Default)]
struct BandLanes {
    row_base: i32,
    width8: i32,
}

impl BandLanes {
    fn new(row_base: usize, width8: usize) -> Option<Self> {
        Some(Self {
            row_base: i32::try_from(row_base).ok()?,
            width8: i32::try_from(width8).ok()?,
        })
    }

    /// The band cell of `(y8, x8)`, or -1 for a row above the band.
    fn index<S: Simd>(self, [y8, x8]: [Lanes<S>; 2]) -> Lanes<S> {
        let row = y8 - self.row_base;
        (row * self.width8 + x8) | (row >> 31)
    }
}

/// The trajectory band geometry the per-lane arithmetic of a tracked scan uses.
#[derive(Clone, Copy, Default)]
struct TrackLanes {
    band: BandLanes,
    width8: u32,
    height8: u32,
    cells: u32,
    unit_mask: i32,
    unit8: u32,
    col_offset8: i32,
    window8: u32,
}

impl TrackLanes {
    fn new(geometry: BandGeometry) -> Option<Self> {
        let unit = i32::try_from(geometry.unit_size8).ok()?;
        let col_offset8 = i32::try_from(geometry.col_offset8).ok()?;
        Some(Self {
            band: BandLanes::new(geometry.row_base, geometry.width8)?,
            width8: u32::try_from(geometry.width8).ok()?,
            height8: u32::try_from(geometry.height8).ok()?,
            cells: u32::try_from(geometry.cells).ok()?,
            unit_mask: unit - 1,
            unit8: unit.cast_unsigned(),
            col_offset8,
            window8: u32::try_from(unit + 2 * col_offset8).ok()?,
        })
    }
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
    output: BandLanes,
    track: TrackLanes,
}

impl LaneGeometry {
    /// `None` when a band cell of an in-frame position could overflow `i32`.
    fn new(
        step: usize,
        unit_size8: usize,
        output: &ProjectedFieldBand<'_>,
        band: Option<BandGeometry>,
    ) -> Option<Self> {
        let step = step.clamp(1, 2);
        let unit_size8 = unit_size8.max(1);
        debug_assert!(unit_size8.is_power_of_two());
        let unit = i32::try_from(unit_size8).ok()?;
        let horizontal_offset8 = if step > 1 { unit } else { unit / 2 };
        let band = band.unwrap_or_default();
        let widest = output.width8.max(band.width8);
        let cells = output.height8.max(band.height8).checked_mul(widest)?;
        i32::try_from(cells.checked_add(output.width8)?).ok()?;
        Some(Self {
            step,
            step_mask: step as i32 - 1,
            unit_mask: unit - 1,
            horizontal_offset8,
            window8: u32::try_from(unit + 2 * horizontal_offset8).ok()?,
            width8: u32::try_from(i32::try_from(output.width8).ok()?).ok()?,
            height8: u32::try_from(i32::try_from(output.height8).ok()?).ok()?,
            output: BandLanes::new(output.row_base, output.width8)?,
            track: TrackLanes::new(band)?,
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
    trajectories: Option<&mut TrajectoryBand<'_>>,
    output: &mut ProjectedFieldBand<'_>,
) {
    debug_assert_eq!(
        (prepared.source_width8, prepared.source_height8),
        source.dimensions8()
    );
    let band = trajectories.as_deref().map(TrajectoryBand::geometry);
    let Some(geometry) = LaneGeometry::new(projection_step, tmvp_unit_size8, output, band) else {
        return;
    };
    let refs = band.map_or(0, |band| band.reference_count);
    let rows = rows.start..rows.end.min(prepared.source_height8);
    #[cfg(target_arch = "x86_64")]
    if let Some(simd) = Level::baseline().as_avx2() {
        let tables = TargetTables::new(simd, prepared, refs);
        return project_rows(
            &tables,
            prepared,
            source,
            rows,
            geometry,
            trajectories,
            output,
        );
    }
    dispatch!(Level::baseline(), simd => {
        let tables = TargetTables::new(simd, prepared, refs);
        project_rows(&tables, prepared, source, rows, geometry, trajectories, output);
    });
}

/// [`project_temporal_motion_field`] with the SIMD token bound.
fn project_rows<S: Simd>(
    tables: &TargetTables<S>,
    prepared: &TemporalProjectionSource,
    source: &impl TemporalMotionRows,
    rows: Range<usize>,
    geometry: LaneGeometry,
    mut trajectories: Option<&mut TrajectoryBand<'_>>,
    output: &mut ProjectedFieldBand<'_>,
) {
    let step = geometry.step;
    let side = prepared.side & 1;
    let mut lanes = ChunkLanes::default();
    for y8 in rows.step_by(step) {
        let Some(row) = source.row(y8) else {
            continue;
        };
        for (chunk_index, chunk) in row.chunks(CHUNK * step).enumerate() {
            let x_base = chunk_index * CHUNK * step;
            if let Some(trajectories) = trajectories.as_deref_mut() {
                let Some(walk) =
                    chunk_lanes::<S, true>(&mut lanes, chunk, side, tables, y8, x_base, geometry)
                else {
                    return;
                };
                ChunkWalk::new(&lanes, walk, y8, x_base, step).project_tracked(
                    prepared,
                    trajectories,
                    output,
                );
            } else {
                let Some(walk) =
                    chunk_lanes::<S, false>(&mut lanes, chunk, side, tables, y8, x_base, geometry)
                else {
                    return;
                };
                ChunkWalk::new(&lanes, walk, y8, x_base, step).project(prepared, output);
            }
        }
    }
}

/// One chunk's lanes, walked in raster order for their order-dependent effects.
struct ChunkWalk<'a> {
    lanes: &'a ChunkLanes,
    /// The lanes with an effect: those that project, and for a tracked scan
    /// those that intersect.
    walk: u64,
    y8: usize,
    x_base: usize,
    step: usize,
}

impl<'a> ChunkWalk<'a> {
    fn new(lanes: &'a ChunkLanes, walk: u64, y8: usize, x_base: usize, step: usize) -> Self {
        Self {
            lanes,
            walk,
            y8,
            x_base,
            step,
        }
    }

    fn ints(&self, lane: usize) -> Option<LaneInts<'a>> {
        let ints = self.lanes.ints.as_flattened().as_flattened();
        ints.get(lane..)?.first_chunk().map(LaneInts)
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
        let source = prepared.source_ref;
        let scanned_slot = trajectories.row_slot(source, self.y8, self.x_base);
        let mut walk = self.walk;
        while walk != 0 {
            let lane = walk.trailing_zeros() as usize;
            walk &= walk - 1;
            let (Some(target), Some(ints)) =
                (self.target(&prepared.targets, lane), self.ints(lane))
            else {
                continue;
            };
            let bits = self.lanes.bits.as_flattened()[lane];
            let source_slot = scanned_slot + lane * self.step;
            let mv = ints.mv();
            let end_slot = match target.end_ref {
                Some(end) if bits & INTERSECT != 0 => {
                    let end_slot = trajectories.slot(end, ints.cell(END_CELL));
                    trajectories.extend_trajectories(source, end, source_slot, None, mv);
                    if bits & REVERSE != 0 {
                        let back = Mv {
                            row: -mv.row,
                            col: -mv.col,
                        };
                        let x8 = self.x_base + lane * self.step;
                        trajectories.extend_trajectories(end, source, end_slot, Some(x8), back);
                    }
                    (bits & END_WRITE != 0).then_some(end_slot)
                }
                _ => None,
            };
            if bits & PROJECT != 0
                && write_projection(output, target, mv, ints.cell(OUTPUT_CELL))
                && bits & OBSERVE != 0
            {
                let (row, col) = ints.pair(PROJECTED);
                let (y8, x8) = ints.pair(POSITION);
                trajectories.observe_projection_at(
                    source,
                    target.end_ref,
                    source_slot,
                    ints.cell(TRAJECTORY_CELL),
                    Mv { row, col },
                    || {
                        let (row, col) = ints.pair(END_MV);
                        Mv { row, col }
                    },
                    (y8 as usize, x8 as usize),
                    end_slot,
                );
            }
        }
    }

    /// The walk without trajectories: only projecting lanes have an effect.
    #[inline(never)]
    fn project(&self, prepared: &TemporalProjectionSource, output: &mut ProjectedFieldBand<'_>) {
        let mut walk = self.walk;
        while walk != 0 {
            let lane = walk.trailing_zeros() as usize;
            walk &= walk - 1;
            if let (Some(target), Some(ints)) =
                (self.target(&prepared.targets, lane), self.ints(lane))
            {
                write_projection(output, target, ints.mv(), ints.cell(OUTPUT_CELL));
            }
        }
    }
}

/// Writes one projection into the output band cell `index` when AV2 § 7.9.3
/// admits it there, and returns whether it wrote.
fn write_projection(
    output: &mut ProjectedFieldBand<'_>,
    target: &ProjectionTarget,
    mv: Mv,
    index: usize,
) -> bool {
    let Some(output_cell) = output.cells.get_mut(index) else {
        return false;
    };
    let ref_offset = target.ref_offset.abs();
    if output_cell.valid && !(target.end_is_target && output_cell.ref_offset() != ref_offset) {
        return false;
    }
    let mv = if target.ref_offset < 0 {
        Mv {
            row: -mv.row,
            col: -mv.col,
        }
    } else {
        mv
    };
    *output_cell = ProjectedTemporalMotionCell::new(true, mv, ref_offset);
    true
}

/// Reads the sampled cells of one chunk and computes, for each, its
/// decompressed vector, its projection, its sampled position and its band
/// cell; a `TRACKED` scan also places the § 7.9.8 trajectory end. The lane
/// window only needs the scanned list's `count * step` cells: a step-1 group
/// spans one six-byte list per cell and a step-2 group skips every other
/// cell, so lanes past that read stale bytes that the walk mask drops.
#[inline(never)]
fn chunk_lanes<S: Simd, const TRACKED: bool>(
    lanes: &mut ChunkLanes,
    chunk: &[StoredTemporalCell],
    side: usize,
    tables: &TargetTables<S>,
    y8: usize,
    x_base: usize,
    geometry: LaneGeometry,
) -> Option<u64> {
    let simd = tables.simd;
    let count = chunk.len().div_ceil(geometry.step).min(CHUNK);
    let stored = chunk
        .as_flattened()
        .get(side * size_of::<StoredTemporalCell>() / 2..)
        .unwrap_or_default();
    let window = &mut lanes.window.0;
    let needed = count * geometry.step * size_of::<StoredTemporalCell>();
    let copied = stored.len().min(window.len()).min(needed);
    window[..copied].copy_from_slice(&stored[..copied]);
    let track = geometry.track;
    let lane_x = Lanes::from_fn(simd, |lane| (lane * geometry.step) as i32);
    let y = Lanes::splat(simd, i32::try_from(y8).ok()?);
    let y_unit = y & !geometry.unit_mask;
    let y_track_unit = y & !track.unit_mask;
    let row_inside = simd.splat_mask32x16(u32::try_from(y8).is_ok_and(|y8| y8 < track.height8));
    let mut walk = 0;
    for group in 0..count.div_ceil(LANES) {
        let first = i32::try_from(x_base + group * LANES * geometry.step).ok()?;
        let x = lane_x + first;
        let [refs, saved_rows, saved_cols] =
            deinterleave(simd, &lanes.window, group, geometry.step)?;
        lanes.refs[group] = refs.into();
        let (allow, factor, end_factor) = tables.lookup(refs);
        let (mut sampled, mut ended) = (simd.splat_mask32x16(true), simd.splat_mask32x16(true));
        let mut base = [Lanes::splat(simd, 0); 2];
        let mut position = [Lanes::splat(simd, 0); 2];
        let mut end = [Lanes::splat(simd, 0); 2];
        for (component, saved) in [saved_rows, saved_cols].into_iter().enumerate() {
            let mv = decompress(widen_i8(simd, saved));
            lanes.ints[MV + component][group] = mv.into();
            let projected = project(mv, factor);
            lanes.ints[PROJECTED + component][group] = projected.into();
            if TRACKED {
                lanes.ints[END_MV + component][group] = project(mv, end_factor).into();
            }
            let (origin, limit) = if component == 0 {
                (y, geometry.height8)
            } else {
                (x, geometry.width8)
            };
            let inside = |position: Lanes<S>| position.bitcast::<u32x16<S>>().simd_lt(limit);
            let unrounded = origin + divide_by_64(projected);
            sampled &= inside(unrounded);
            position[component] = unrounded & !geometry.step_mask;
            lanes.ints[POSITION + component][group] = position[component].into();
            base[component] = position[component] & !geometry.unit_mask;
            let unrounded = origin + divide_by_64(mv);
            ended &= inside(unrounded);
            end[component] = unrounded & !geometry.step_mask;
        }
        lanes.ints[OUTPUT_CELL][group] = geometry.output.index(position).into();
        let column = x - base[1] + geometry.horizontal_offset8;
        let near =
            base[0].simd_eq(y_unit) & column.bitcast::<u32x16<S>>().simd_lt(geometry.window8);
        let zero = Lanes::splat(simd, 0);
        let bit = |mask: mask32x16<S>, bit: u8| mask.select(Lanes::splat(simd, bit.into()), zero);
        let projecting = sampled & near;
        let mut bits = bit(projecting, PROJECT);
        if TRACKED {
            let inside = x.bitcast::<u32x16<S>>().simd_lt(track.width8) & row_inside;
            lanes.ints[TRAJECTORY_CELL][group] = track.band.index(position).into();
            let end_index = track.band.index(end);
            lanes.ints[END_CELL][group] = end_index.into();
            let in_band = ended & end_index.bitcast::<u32x16<S>>().simd_lt(track.cells);
            let reverse = (end[0] & !track.unit_mask).simd_eq(y_track_unit);
            let unit = position.map(|value| value & !track.unit_mask);
            let window = (end[0] - unit[0])
                .bitcast::<u32x16<S>>()
                .simd_lt(track.unit8)
                & (end[1] - unit[1] + track.col_offset8)
                    .bitcast::<u32x16<S>>()
                    .simd_lt(track.window8);
            bits = bits
                | bit(reverse & in_band, REVERSE)
                | bit(window & in_band, END_WRITE)
                | bit(inside, INTERSECT)
                | bit(projecting & inside, OBSERVE);
        }
        let bits = narrow_bytes(simd, bits) & allow;
        if TRACKED {
            lanes.bits[group] = bits.into();
        }
        walk |= (bits & (PROJECT | INTERSECT)).simd_ne(0).to_bitmask() << (group * LANES);
    }
    let live = u64::MAX >> (CHUNK - count.max(1));
    Some(walk & live)
}

/// A chunk's stored cells from the scanned list's first byte on.
struct Window([u8; WINDOW]);

impl Default for Window {
    fn default() -> Self {
        Self([0; WINDOW])
    }
}

/// The stored reference and saved row and column of each lane of `group`,
/// read from a chunk window whose cells start at the scanned list. A step-2
/// group is four runs of four cells; a step-1 group's even cells and its odd
/// cells, six bytes further, each form two such runs, whose bytes interleave.
#[inline]
fn deinterleave<S: Simd>(
    simd: S,
    window: &Window,
    group: usize,
    step: usize,
) -> Option<[u8x16<S>; 3]> {
    if step == 1 {
        let span: &[u8; 112] = window.0.get(96 * group..)?.first_chunk()?;
        let [even_low, even_high, odd_low, odd_high] = [
            compact(simd, span, 0)?,
            compact(simd, span, 48)?,
            compact(simd, span, 6)?,
            compact(simd, span, 54)?,
        ];
        Some(transpose(
            [even_low.zip_low(odd_low), even_high.zip_low(odd_high)],
            [even_low.zip_high(odd_low), even_high.zip_high(odd_high)],
        ))
    } else {
        let span: &[u8; 192] = window.0.get(192 * group..)?.first_chunk()?;
        let [first, second, third, fourth] = [
            compact(simd, span, 0)?.bitcast::<u32x4<S>>(),
            compact(simd, span, 48)?.bitcast(),
            compact(simd, span, 96)?.bitcast(),
            compact(simd, span, 144)?.bitcast(),
        ];
        Some(transpose(
            [first.zip_low(second), third.zip_low(fourth)].map(Bytes::bitcast),
            [first.zip_high(second), third.zip_high(fourth)].map(Bytes::bitcast),
        ))
    }
}

/// The run of four cells twelve bytes apart at `start`: each cell's three
/// bytes sit in their own dword of the run's three blocks, so dword blends
/// collect them in one vector, whose bytes are then grouped as four
/// references, four rows and four columns.
#[inline]
fn compact<S: Simd, const N: usize>(simd: S, span: &[u8; N], start: usize) -> Option<u8x16<S>> {
    const ORDER: [u8; LANES] = [0, 12, 8, 4, 1, 13, 9, 5, 2, 14, 10, 6, 3, 7, 11, 15];
    let block = |offset: usize| {
        let bytes = span.get(start + offset..)?.first_chunk::<16>()?;
        Some(u8x16::simd_from(simd, *bytes).bitcast::<u32x4<S>>())
    };
    let dwords = |lanes: [i32; 4]| simd.from_vector_mask32x4(i32x4::simd_from(simd, lanes));
    let run = dwords([0, -1, 0, 0]).select(
        block(32)?,
        dwords([0, 0, -1, 0]).select(block(16)?, block(0)?),
    );
    Some(
        run.bitcast::<u8x16<S>>()
            .swizzle_dyn(u8x16::simd_from(simd, ORDER)),
    )
}

/// The references, rows and columns of two vectors holding lanes 0..8 and
/// 8..16 of the references and rows, and two holding their columns.
fn transpose<S: Simd>(low: [u8x16<S>; 2], high: [u8x16<S>; 2]) -> [u8x16<S>; 3] {
    let [first, second] = low.map(Bytes::bitcast::<u64x2<S>>);
    let [low_columns, high_columns] = high.map(Bytes::bitcast::<u64x2<S>>);
    let columns = low_columns.zip_low(high_columns);
    [first.zip_low(second), first.zip_high(second), columns].map(Bytes::bitcast)
}

/// `i8` lanes sign-extended to `i32`.
fn widen_i8<S: Simd>(simd: S, bytes: u8x16<S>) -> Lanes<S> {
    let bytes = bytes.bitcast();
    let words = simd.widen_i8x32(simd.combine_i8x16(bytes, bytes)).0;
    let (low, high) = simd.split_i16x16(words);
    widen_i16(simd, low, high)
}

/// Two halves of `i16` lanes sign-extended to `i32`, each through the lower
/// half of a doubled vector, which AVX2 widens in one instruction.
fn widen_i16<S: Simd>(simd: S, low: i16x8<S>, high: i16x8<S>) -> Lanes<S> {
    let widen = |half: i16x8<S>| simd.widen_i16x16(simd.combine_i16x8(half, half)).0;
    simd.combine_i32x8(widen(low), widen(high))
}

/// `i32` lanes saturated to `i8` lanes.
fn narrow_bytes<S: Simd>(simd: S, lanes: Lanes<S>) -> u8x16<S> {
    let (low, high) = simd.split_i32x16(lanes);
    let (low, high) = simd.split_i16x16(low.saturating_narrow(high));
    low.saturating_narrow(high).bitcast()
}

/// AV2 § 7.9.2 decompression of a saved vector component, lane by lane.
fn decompress<S: Simd>(value: Lanes<S>) -> Lanes<S> {
    let magnitude = value.abs();
    let step_log2 = ((magnitude >> 4) - 1).max(0);
    let decompressed = (magnitude - (step_log2 << 4)) << step_log2;
    let sign = value >> 31;
    (decompressed ^ sign) - sign
}

/// `Round2Signed(component * factor, 14)` of a decompressed component. A
/// magnitude of at most 2048 keeps the product inside `i32` and the result
/// under `MV_LIMIT` for every admitted factor, so the scalar clamp never binds.
fn project<S: Simd>(component: Lanes<S>, factor: Lanes<S>) -> Lanes<S> {
    let scaled = component * factor;
    (scaled + (scaled >> 31) + (1 << 13)) >> 14
}

/// [`project`] for one component.
#[cfg(test)]
fn project_component(component: i32, factor: i32) -> i32 {
    let scaled = component * factor;
    (scaled + (scaled >> 31) + (1 << 13)) >> 14
}

/// Division by 64 that truncates toward zero, as the scalar `/` does.
fn divide_by_64<S: Simd>(value: Lanes<S>) -> Lanes<S> {
    (value + ((value >> 31) & 63)) >> 6
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::{
        CompressedTemporalMv, MAX_FRAME_DISTANCE, MAX_SORTED_REFS, MotionFieldLayout,
        RefOrderHints, TemporalMotionFieldMetadata, clamped_tmvp_factor, project_no_constraint,
        project_tmvp_mv, project_tmvp_mv_with_factor, sampled_temporal_position,
        tmvp_projection_factor, uncompress_tmvp_mv,
    };
    use super::*;
    use crate::prediction::inter::get_relative_dist;

    #[test]
    fn lane_projection_matches_the_scalar_projection_for_every_saved_component() {
        #[cfg(target_arch = "x86_64")]
        if let Some(simd) = Level::baseline().as_sse4_2() {
            lane_projection_matches_scalar(simd);
        }
        dispatch!(Level::baseline(), simd => lane_projection_matches_scalar(simd));
    }

    #[test]
    fn chunk_lanes_match_the_scalar_cell_projection() {
        #[cfg(target_arch = "x86_64")]
        if let Some(simd) = Level::baseline().as_sse4_2() {
            chunk_lanes_match_scalar(simd);
        }
        dispatch!(Level::baseline(), simd => chunk_lanes_match_scalar(simd));
    }

    fn lane_projection_matches_scalar<S: Simd>(simd: S) {
        let saved: Vec<i32> = (-128..128).collect();
        for numerator in -MAX_FRAME_DISTANCE..=MAX_FRAME_DISTANCE {
            for ref_offset in -MAX_FRAME_DISTANCE..=MAX_FRAME_DISTANCE {
                let side = usize::from(ref_offset < 0);
                let factor = tmvp_projection_factor(numerator, ref_offset, side).unwrap();
                let folded = if ref_offset < 0 { -factor } else { factor };
                let observed = if side == 1 { -numerator } else { numerator };
                let end_numerator = ref_offset.abs() - observed;
                let end_factor = clamped_tmvp_factor(end_numerator, ref_offset.abs());
                for chunk in saved.chunks_exact(LANES) {
                    let saved = u8x16::from_fn(simd, |lane| chunk[lane] as u8);
                    let decompressed = decompress(widen_i8(simd, saved));
                    let projected = project(decompressed, Lanes::splat(simd, folded));
                    let end_projected = project(decompressed, Lanes::splat(simd, end_factor));
                    for (lane, &value) in chunk.iter().enumerate() {
                        let component = value as i8;
                        let mv = uncompress_tmvp_mv(CompressedTemporalMv {
                            row: component,
                            col: component,
                        });
                        let end = project_tmvp_mv(mv, end_numerator, ref_offset.abs());
                        assert_eq!(project_component(mv.row, end_factor), end.row);
                        assert_eq!(end_projected[lane], end.row);
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

    fn chunk_lanes_match_scalar<S: Simd>(simd: S) {
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
            let source_hints: Vec<Option<u32>> = (0..7 + next(MAX_SORTED_REFS - 6))
                .map(|_| hint(&mut next))
                .collect();
            let mut ref_order_hints = RefOrderHints::default();
            ref_order_hints.extend_within(source_hints.iter().copied());
            let metadata = TemporalMotionFieldMetadata {
                is_inter: true,
                frame_size: None,
                ref_order_hints,
            };
            let layout = MotionFieldLayout::new(height8 * 2, width8 * 2, 16).unwrap();
            let (source_hint, current_hint, side) = (next(64) as u32, next(64) as u32, next(2));
            let source_to_current = get_relative_dist(source_hint as i32, current_hint as i32);
            let source = TemporalProjectionSource::new(
                &metadata,
                layout,
                source_hint,
                current_hint,
                0,
                side,
                None,
                &hints,
            )
            .unwrap();
            let y8 = next(height8) & !(step - 1);
            let x_base = next(width8.div_ceil(CHUNK * step)) * CHUNK * step;
            let wide = next(2) == 0;
            let cells: Vec<super::super::TemporalMotionCell> = (0..=next(CHUNK * step))
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
                    super::super::TemporalMotionCell {
                        ref_indices: [reference; 2],
                        mvs,
                    }
                })
                .collect();
            let (band_base, output_base) = (next(height8 + 1), next(height8 + 1));
            let band_cells = next(height8 - band_base + 1) * width8;
            let col_offset8 = if step == 1 { unit / 2 } else { unit };
            let band = BandGeometry {
                row_base: band_base,
                width8,
                height8,
                cells: band_cells,
                unit_size8: unit,
                col_offset8,
                reference_count: hints.len(),
            };
            let output = ProjectedFieldBand {
                cells: &mut [],
                width8,
                height8,
                row_base: output_base,
            };
            let geometry = LaneGeometry::new(step, unit, &output, Some(band)).unwrap();
            let stored: Vec<StoredTemporalCell> = cells.iter().map(|cell| cell.store()).collect();
            let walk = chunk_lanes::<S, true>(
                &mut lanes,
                &stored,
                side,
                &TargetTables::new(simd, &source, hints.len()),
                y8,
                x_base,
                geometry,
            )
            .unwrap();
            let cell_index = |(y8, x8): (usize, usize), base: usize| {
                y8.checked_sub(base)
                    .map_or(u32::MAX as usize, |row| row * width8 + x8)
            };
            for (lane, cell) in cells.iter().step_by(step).enumerate() {
                let x8 = x_base + lane * step;
                let raw = usize::from(cell.ref_indices[side]);
                let reference = raw.min(MAX_SORTED_REFS);
                let mv = uncompress_tmvp_mv(cell.mvs[side]);
                let lane_mv = Mv {
                    row: lanes.ints[MV].as_flattened()[lane],
                    col: lanes.ints[MV + 1].as_flattened()[lane],
                };
                assert_eq!(lane_mv, mv, "case {case} lane {lane}");
                let hint = source_hints
                    .get(reference)
                    .filter(|_| reference < MAX_SORTED_REFS);
                let offset = hint
                    .copied()
                    .flatten()
                    .map(|hint| get_relative_dist(source_hint as i32, hint as i32))
                    .or((raw == MAX_SORTED_REFS).then_some(0));
                let entry = offset.map(|offset| {
                    (
                        offset,
                        tmvp_projection_factor(source_to_current, offset, side),
                    )
                });
                let target = source.targets[reference];
                let bits = lanes.bits.as_flattened()[lane];
                let intersects = entry.is_some() && target.end_ref.is_some() && x8 < width8;
                assert_eq!(bits & INTERSECT != 0, intersects);
                let expected = entry.and_then(|(offset, factor)| {
                    let mv = if offset < 0 {
                        Mv {
                            row: -mv.row,
                            col: -mv.col,
                        }
                    } else {
                        mv
                    };
                    let projected =
                        project_tmvp_mv_with_factor(mv, source_to_current, offset.abs(), factor?);
                    sampled_temporal_position(y8, x8, projected, step, unit, (width8, height8))
                        .map(|position| (projected, position))
                });
                let end = [(y8, mv.row, height8), (x8, mv.col, width8)]
                    .map(|(origin, delta, max)| project_no_constraint(origin, delta, max));
                let end = match end {
                    [Some(end_y8), Some(end_x8)] => {
                        Some((end_y8 & !(step - 1), end_x8 & !(step - 1)))
                    }
                    _ => None,
                };
                let end_index = end.map(|end| cell_index(end, band_base));
                let in_band = end_index.filter(|&index| index < band_cells);
                if let Some(index) = in_band {
                    assert_eq!(lanes.ints[END_CELL].as_flattened()[lane] as usize, index);
                }
                let unit_base = |value: usize| value & !(unit - 1);
                assert_eq!(
                    bits & REVERSE != 0,
                    in_band.is_some() && end.is_some_and(|end| unit_base(end.0) == unit_base(y8)),
                    "case {case} lane {lane}"
                );
                if let Some((_, position)) = expected {
                    assert_eq!(bits & OBSERVE != 0, x8 < width8);
                    let observed = if side == 1 {
                        -source_to_current
                    } else {
                        source_to_current
                    };
                    let offset = offset.unwrap();
                    let observed_end = project_tmvp_mv(mv, offset.abs() - observed, offset.abs());
                    let lane_end = Mv {
                        row: lanes.ints[END_MV].as_flattened()[lane],
                        col: lanes.ints[END_MV + 1].as_flattened()[lane],
                    };
                    assert_eq!(lane_end, observed_end, "case {case} lane {lane}");
                    let cell = |row: usize| lanes.ints[row].as_flattened()[lane] as u32 as usize;
                    assert_eq!(cell(OUTPUT_CELL), cell_index(position, output_base));
                    assert_eq!(cell(TRAJECTORY_CELL), cell_index(position, band_base));
                    let (base_y8, base_x8) = (unit_base(position.0), unit_base(position.1));
                    let window = end.is_some_and(|(end_y8, end_x8)| {
                        (base_y8..base_y8 + unit).contains(&end_y8)
                            && (base_x8.saturating_sub(col_offset8)..base_x8 + unit + col_offset8)
                                .contains(&end_x8)
                    });
                    assert_eq!(bits & END_WRITE != 0, in_band.is_some() && window);
                }
                assert_eq!(bits & PROJECT != 0, expected.is_some(), "case {case}");
                assert_eq!(walk >> lane & 1 == 1, expected.is_some() || intersects);
                if let Some((projected, (pos_y8, pos_x8))) = expected {
                    let lane_projected = Mv {
                        row: lanes.ints[PROJECTED].as_flattened()[lane],
                        col: lanes.ints[PROJECTED + 1].as_flattened()[lane],
                    };
                    assert_eq!(lane_projected, projected, "case {case} lane {lane}");
                    assert_eq!(lanes.ints[POSITION].as_flattened()[lane] as usize, pos_y8);
                    assert_eq!(
                        lanes.ints[POSITION + 1].as_flattened()[lane] as usize,
                        pos_x8
                    );
                }
            }
        }
    }
}
