// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::ops::Range;
use std::simd::{
    Mask, Select, Simd, Swizzle,
    cmp::{SimdOrd, SimdPartialEq, SimdPartialOrd},
    num::{SimdInt, SimdUint},
};

use super::trajectory::BandGeometry;
use super::{
    Mv, ProjectedFieldBand, ProjectedTemporalMotionCell, StoredTemporalCell, TemporalMotionRows,
    TemporalProjectionSource, TrajectoryBand,
};

const LANES: usize = 16;
const GROUPS: usize = 4;
const CHUNK: usize = LANES * GROUPS;
/// 64-byte blocks of the stored cells a chunk spans at the widest step.
const BLOCKS: usize = CHUNK * 2 * size_of::<StoredTemporalCell>() / 64;

type Lanes = Simd<i32, LANES>;
type Narrow = Simd<i16, LANES>;
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
    /// The factor that projects the stored vector onto the § 7.9.8 trajectory
    /// end, the vector `observe_projection_at` records for `end_ref`.
    pub(super) end_factor: i32,
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
const INT_ROWS: usize = 9;
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
/// the table, read 0, as the empty target does. [`INTERSECTS`] stays set only
/// when the source and the end reference both index the trajectory grid.
struct TargetTables {
    flags: Simd<u8, LANES>,
    divisor: [Simd<u8, LANES>; 2],
    source_to_current: i32,
}

impl TargetTables {
    fn new(prepared: &TemporalProjectionSource, trajectory_refs: usize) -> Self {
        let entry = |index: usize| prepared.targets.get(index).copied().unwrap_or_default();
        let byte = |index: usize, shift: u32| (entry(index).divisor >> shift) as u8;
        let tracked = |target: ProjectionTarget| {
            prepared.source_ref < trajectory_refs
                && target.end_ref.is_some_and(|end| end < trajectory_refs)
        };
        let flags = |index: usize| {
            let target = entry(index);
            target.flags
                & if tracked(target) {
                    u8::MAX
                } else {
                    !INTERSECTS
                }
        };
        Self {
            flags: Simd::from_array(core::array::from_fn(flags)),
            divisor: [0, 8].map(|shift| Simd::from_array(core::array::from_fn(|i| byte(i, shift)))),
            source_to_current: prepared.source_to_current,
        }
    }

    /// The flags and the folded projection factor of each lane's reference.
    fn lookup(&self, refs: Simd<u8, LANES>) -> (Simd<u8, LANES>, Lanes) {
        let [low, high] = self
            .divisor
            .map(|table| table.swizzle_dyn(refs).cast::<u16>());
        let divisor: Lanes = (low | high << 8).cast::<i16>().cast();
        let factor = divisor * Lanes::splat(self.source_to_current);
        (self.flags.swizzle_dyn(refs), factor)
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
    fn index(self, [y8, x8]: [Lanes; 2]) -> Lanes {
        let row = y8 - Lanes::splat(self.row_base);
        row.simd_ge(Lanes::splat(0))
            .select(row * Lanes::splat(self.width8) + x8, Lanes::splat(-1))
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
    mut trajectories: Option<&mut TrajectoryBand<'_>>,
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
    let step = geometry.step;
    let side = prepared.side & 1;
    let rows = rows.start..rows.end.min(prepared.source_height8);
    let tables = TargetTables::new(prepared, band.map_or(0, |band| band.reference_count));
    let mut lanes = ChunkLanes::default();
    for y8 in rows.step_by(step) {
        let Some(row) = source.row(y8) else {
            continue;
        };
        for (chunk_index, chunk) in row.chunks(CHUNK * step).enumerate() {
            let x_base = chunk_index * CHUNK * step;
            if let Some(trajectories) = trajectories.as_deref_mut() {
                let Some(walk) =
                    chunk_lanes::<true>(&mut lanes, chunk, side, &tables, y8, x_base, geometry)
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
                    chunk_lanes::<false>(&mut lanes, chunk, side, &tables, y8, x_base, geometry)
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
                    || Mv {
                        row: project_component(mv.row, target.end_factor),
                        col: project_component(mv.col, target.end_factor),
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
/// cell; a `TRACKED` scan also places the § 7.9.8 trajectory end.
#[inline(never)]
fn chunk_lanes<const TRACKED: bool>(
    lanes: &mut ChunkLanes,
    chunk: &[StoredTemporalCell],
    side: usize,
    tables: &TargetTables,
    y8: usize,
    x_base: usize,
    geometry: LaneGeometry,
) -> Option<u64> {
    let count = chunk.len().div_ceil(geometry.step).min(CHUNK);
    let stored = chunk
        .as_flattened()
        .get(side * size_of::<StoredTemporalCell>() / 2..)
        .unwrap_or_default();
    let window = lanes.window.0.as_flattened_mut();
    let copied = stored.len().min(window.len());
    window[..copied].copy_from_slice(&stored[..copied]);
    let track = geometry.track;
    let iota = Lanes::from_array(core::array::from_fn(|lane| lane as i32));
    let y = Lanes::splat(i32::try_from(y8).ok()?);
    let y_unit = y & Lanes::splat(!geometry.unit_mask);
    let y_track_unit = y & Lanes::splat(!track.unit_mask);
    let row_inside = Mask::splat(u32::try_from(y8).is_ok_and(|y8| y8 < track.height8));
    let (mut projects, mut intersects) = (0u64, 0u64);
    for group in 0..count.div_ceil(LANES) {
        let first = i32::try_from(x_base + group * LANES * geometry.step).ok()?;
        let x = Lanes::splat(first) + iota * Lanes::splat(geometry.step as i32);
        let [refs, saved_rows, saved_cols] = deinterleave(&lanes.window.0, group, geometry.step);
        lanes.refs[group] = refs.to_array();
        let (flags, factor) = tables.lookup(refs);
        let (mut sampled, mut ended) = (Mask::splat(true), Mask::splat(true));
        let mut base = [Lanes::splat(0); 2];
        let mut position = [Lanes::splat(0); 2];
        let mut end = [Lanes::splat(0); 2];
        for (component, saved) in [saved_rows, saved_cols].into_iter().enumerate() {
            let mv: Lanes = decompress(saved.cast::<i8>().cast()).cast();
            lanes.ints[MV + component][group] = mv.to_array();
            let projected = project(mv, factor);
            lanes.ints[PROJECTED + component][group] = projected.to_array();
            let (origin, limit) = if component == 0 {
                (y, geometry.height8)
            } else {
                (x, geometry.width8)
            };
            let inside = |position: Lanes| position.cast::<u32>().simd_lt(Simd::splat(limit));
            let unrounded = origin + divide_by_64(projected);
            sampled &= inside(unrounded);
            position[component] = unrounded & Lanes::splat(!geometry.step_mask);
            lanes.ints[POSITION + component][group] = position[component].to_array();
            base[component] = position[component] & Lanes::splat(!geometry.unit_mask);
            let unrounded = origin + divide_by_64(mv);
            ended &= inside(unrounded);
            end[component] = unrounded & Lanes::splat(!geometry.step_mask);
        }
        lanes.ints[OUTPUT_CELL][group] = geometry.output.index(position).to_array();
        let column = x - base[1] + Lanes::splat(geometry.horizontal_offset8);
        let near =
            base[0].simd_eq(y_unit) & column.cast::<u32>().simd_lt(Simd::splat(geometry.window8));
        let projecting =
            (flags & Simd::splat(PROJECTS)).simd_ne(Simd::splat(0)) & (sampled & near).cast();
        let shift = group * LANES;
        projects |= projecting.to_bitmask() << shift;
        if TRACKED {
            let inside = (x.cast::<u32>().simd_lt(Simd::splat(track.width8)) & row_inside).cast();
            let intersecting = (flags & Simd::splat(INTERSECTS)).simd_ne(Simd::splat(0)) & inside;
            intersects |= intersecting.to_bitmask() << shift;
            lanes.ints[TRAJECTORY_CELL][group] = track.band.index(position).to_array();
            let end_index = track.band.index(end);
            lanes.ints[END_CELL][group] = end_index.to_array();
            let in_band = ended & end_index.cast::<u32>().simd_lt(Simd::splat(track.cells));
            let reverse = (end[0] & Lanes::splat(!track.unit_mask)).simd_eq(y_track_unit);
            let unit = position.map(|value| value & Lanes::splat(!track.unit_mask));
            let window = (end[0] - unit[0])
                .cast::<u32>()
                .simd_lt(Simd::splat(track.unit8))
                & (end[1] - unit[1] + Lanes::splat(track.col_offset8))
                    .cast::<u32>()
                    .simd_lt(Simd::splat(track.window8));
            let bit = |mask: Mask<i32, LANES>, bit: u8| {
                (mask & in_band)
                    .cast::<i8>()
                    .select(Simd::splat(bit), Simd::splat(0))
            };
            let flag =
                |mask: Mask<i8, LANES>, bit: u8| mask.select(Simd::splat(bit), Simd::splat(0));
            lanes.bits[group] = (bit(reverse, REVERSE)
                | bit(window, END_WRITE)
                | flag(projecting, PROJECT)
                | flag(intersecting, INTERSECT)
                | flag(projecting & inside, OBSERVE))
            .to_array();
        }
    }
    let live = u64::MAX >> (CHUNK - count.max(1));
    Some((projects | intersects) & live)
}

/// A chunk's stored cells from the scanned list's first byte on.
struct Window([[u8; 64]; BLOCKS]);

impl Default for Window {
    fn default() -> Self {
        Self([[0; 64]; BLOCKS])
    }
}

/// Lane `i` reads byte `OFFSET + STRIDE * i - BASE` of two concatenated
/// blocks, or byte 0 when that byte lies outside them.
struct Strided<const OFFSET: usize, const STRIDE: usize, const BASE: usize>;

impl<const OFFSET: usize, const STRIDE: usize, const BASE: usize> Swizzle<LANES>
    for Strided<OFFSET, STRIDE, BASE>
{
    const INDEX: [usize; LANES] = {
        let mut index = [0; LANES];
        let mut lane = 0;
        while lane < LANES {
            let byte = OFFSET + STRIDE * lane;
            if byte >= BASE && byte - BASE < 128 {
                index[lane] = byte - BASE;
            }
            lane += 1;
        }
        index
    };
}

/// The stored reference and saved row and column of each lane of `group`,
/// read from a chunk window whose cells start at the scanned list. A step-1
/// group spans 96 bytes, so even groups start on a block and odd groups 32
/// bytes in; a step-2 group spans three blocks, and its last five lanes read
/// the third.
fn deinterleave(window: &[[u8; 64]; BLOCKS], group: usize, step: usize) -> [Simd<u8, LANES>; 3] {
    type Block = Simd<u8, 64>;
    let block = |index: usize| Block::from_array(window[index]);
    if step == 1 {
        let first = 3 * group / 2;
        let (low, high) = (block(first), block(first + 1));
        if group.is_multiple_of(2) {
            [
                Strided::<0, 6, 0>::concat_swizzle(low, high),
                Strided::<1, 6, 0>::concat_swizzle(low, high),
                Strided::<2, 6, 0>::concat_swizzle(low, high),
            ]
        } else {
            [
                Strided::<32, 6, 0>::concat_swizzle(low, high),
                Strided::<33, 6, 0>::concat_swizzle(low, high),
                Strided::<34, 6, 0>::concat_swizzle(low, high),
            ]
        }
    } else {
        let [low, middle, high] = [0, 1, 2].map(|offset| block(3 * group + offset));
        let tail = Mask::<i8, LANES>::from_array(core::array::from_fn(|lane| lane >= 11));
        [
            tail.select(
                Strided::<0, 12, 128>::concat_swizzle(high, high),
                Strided::<0, 12, 0>::concat_swizzle(low, middle),
            ),
            tail.select(
                Strided::<1, 12, 128>::concat_swizzle(high, high),
                Strided::<1, 12, 0>::concat_swizzle(low, middle),
            ),
            tail.select(
                Strided::<2, 12, 128>::concat_swizzle(high, high),
                Strided::<2, 12, 0>::concat_swizzle(low, middle),
            ),
        ]
    }
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

/// [`project`] for one component.
fn project_component(component: i32, factor: i32) -> i32 {
    let scaled = component * factor;
    (scaled + (scaled >> 31) + (1 << 13)) >> 14
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
        RefOrderHints, TemporalMotionFieldMetadata, clamped_tmvp_factor, project_no_constraint,
        project_tmvp_mv, project_tmvp_mv_with_factor, sampled_temporal_position,
        tmvp_projection_factor, uncompress_tmvp_mv,
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
                let observed = if side == 1 { -numerator } else { numerator };
                let end_numerator = ref_offset.abs() - observed;
                let end_factor = clamped_tmvp_factor(end_numerator, ref_offset.abs());
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
                        let end = project_tmvp_mv(mv, end_numerator, ref_offset.abs());
                        assert_eq!(project_component(mv.row, end_factor), end.row);
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
            let walk = chunk_lanes::<true>(
                &mut lanes,
                &stored,
                side,
                &TargetTables::new(&source, hints.len()),
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
                let reference = usize::from(cell.ref_indices[side]).min(MAX_SORTED_REFS);
                let mv = uncompress_tmvp_mv(cell.mvs[side]);
                let lane_mv = Mv {
                    row: lanes.ints[MV].as_flattened()[lane],
                    col: lanes.ints[MV + 1].as_flattened()[lane],
                };
                assert_eq!(lane_mv, mv, "case {case} lane {lane}");
                let hint = source_hints
                    .get(reference)
                    .filter(|_| reference < MAX_SORTED_REFS);
                let entry = hint.copied().flatten().map(|hint| {
                    let offset = get_relative_dist(source_hint as i32, hint as i32);
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
