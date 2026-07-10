// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed;

use super::{Mv, warp_sub_mv_at};

const MAX_FRAME_DISTANCE: i32 = 31;
const REFMVS_LIMIT: i32 = (1 << 11) - 1;
const MV_LIMIT: i32 = (1 << 16) - 1;
const TIP_DIRECTIONS: [(i32, i32); 4] = [(-1, 0), (0, -1), (1, 0), (0, 1)];
const DIV_MULT: [i64; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TemporalMotionCell {
    ref_order_hints: [Option<u32>; 2],
    mvs: [Mv; 2],
}

impl TemporalMotionCell {
    const fn is_valid(self) -> bool {
        self.ref_order_hints[0].is_some() || self.ref_order_hints[1].is_some()
    }
}

fn allocate_temporal_grid<T>(mi_rows: usize, mi_cols: usize) -> Option<(usize, usize, Vec<T>)>
where
    T: Clone + Default,
{
    let width8 = mi_cols.div_ceil(2);
    let height8 = mi_rows.div_ceil(2);
    let cells = width8.checked_mul(height8)?;
    Some((width8, height8, vec![T::default(); cells]))
}

fn temporal_grid_index(width8: usize, height8: usize, y8: usize, x8: usize) -> Option<usize> {
    if y8 >= height8 || x8 >= width8 {
        return None;
    }
    Some(y8 * width8 + x8)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionField {
    width8: usize,
    height8: usize,
    cells: Vec<TemporalMotionCell>,
}

impl TemporalMotionField {
    pub(crate) fn empty() -> Self {
        Self {
            width8: 0,
            height8: 0,
            cells: Vec::new(),
        }
    }

    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
        })
    }

    pub(crate) fn record_block(&mut self, block: TemporalMotionBlock) {
        let Some(row_end) = block.mi_row.checked_add(block.n4h) else {
            return;
        };
        let Some(col_end) = block.mi_col.checked_add(block.n4w) else {
            return;
        };
        let row_end = row_end.min(block.mi_rows);
        let col_end = col_end.min(block.mi_cols);
        if row_end <= block.mi_row || col_end <= block.mi_col {
            return;
        }
        let row8_start = block.mi_row >> 1;
        let col8_start = block.mi_col >> 1;
        let row8_end = row_end.div_ceil(2).min(self.height8);
        let col8_end = col_end.div_ceil(2).min(self.width8);

        for y8 in row8_start..row8_end {
            for x8 in col8_start..col8_end {
                let mut cell = TemporalMotionCell::default();
                for list in 0..2 {
                    let Some(order_hint) = block.ref_order_hints[list] else {
                        continue;
                    };
                    let mv = if let Some(params) = block.warp_params[list] {
                        warp_sub_mv_at(params, block.mi_row, block.mi_col, y8 * 2, x8 * 2)
                    } else {
                        block.mvs[list]
                    };
                    if mv.row.abs() > REFMVS_LIMIT || mv.col.abs() > REFMVS_LIMIT {
                        continue;
                    }
                    cell.ref_order_hints[list] = Some(order_hint);
                    cell.mvs[list] = compress_tmvp_mv(mv);
                }
                if let Some(slot) = self.cell_mut(y8, x8) {
                    *slot = cell;
                }
            }
        }
    }

    fn cell(&self, y8: usize, x8: usize) -> Option<TemporalMotionCell> {
        self.cells
            .get(temporal_grid_index(self.width8, self.height8, y8, x8)?)
            .copied()
    }

    fn cell_mut(&mut self, y8: usize, x8: usize) -> Option<&mut TemporalMotionCell> {
        self.cells
            .get_mut(temporal_grid_index(self.width8, self.height8, y8, x8)?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionBlock {
    pub(crate) mi_row: usize,
    pub(crate) mi_col: usize,
    pub(crate) n4w: usize,
    pub(crate) n4h: usize,
    pub(crate) mi_rows: usize,
    pub(crate) mi_cols: usize,
    pub(crate) ref_order_hints: [Option<u32>; 2],
    pub(crate) mvs: [Mv; 2],
    pub(crate) warp_params: [Option<[i64; 6]>; 2],
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProjectedTemporalMotionCell {
    valid: bool,
    mv: Mv,
    ref_offset: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProjectedTemporalMotionField {
    width8: usize,
    height8: usize,
    cells: Vec<ProjectedTemporalMotionCell>,
}

impl ProjectedTemporalMotionField {
    fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
        })
    }

    fn cell(&self, y8: usize, x8: usize) -> Option<ProjectedTemporalMotionCell> {
        self.cells
            .get(temporal_grid_index(self.width8, self.height8, y8, x8)?)
            .copied()
    }

    fn set_if_empty(&mut self, y8: usize, x8: usize, mv: Mv, ref_offset: i32) {
        let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8) else {
            return;
        };
        let Some(cell) = self.cells.get_mut(index) else {
            return;
        };
        if !cell.valid {
            *cell = ProjectedTemporalMotionCell {
                valid: true,
                mv,
                ref_offset,
            };
        }
    }

    fn set(&mut self, y8: usize, x8: usize, mv: Mv, ref_offset: i32, valid: bool) {
        let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8) else {
            return;
        };
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = ProjectedTemporalMotionCell {
                valid,
                mv,
                ref_offset,
            };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TipReferencePair {
    pub(crate) past_ref: i8,
    pub(crate) future_ref: i8,
    pub(crate) past_offset: i32,
    pub(crate) future_offset: i32,
    pub(crate) ref_offset: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TipMotionField {
    field: ProjectedTemporalMotionField,
    references: TipReferencePair,
}

impl TipMotionField {
    fn candidate(&self, y8: usize, x8: usize, base_mv: Mv) -> [Mv; 2] {
        let cell = self.field.cell(y8, x8).unwrap_or_default();
        let projected = if cell.valid {
            [
                project_mv(
                    cell.mv,
                    self.references.past_offset,
                    self.references.ref_offset,
                )
                .unwrap_or(Mv::ZERO),
                project_mv(
                    cell.mv,
                    self.references.future_offset,
                    self.references.ref_offset,
                )
                .unwrap_or(Mv::ZERO),
            ]
        } else {
            [Mv::ZERO; 2]
        };
        projected.map(|mv| Mv {
            row: (mv.row + base_mv.row).clamp(-MV_LIMIT, MV_LIMIT),
            col: (mv.col + base_mv.col).clamp(-MV_LIMIT, MV_LIMIT),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMvContext {
    current_order_hint: u32,
    ref_order_hints: Vec<Option<u32>>,
    field: ProjectedTemporalMotionField,
    tip: Option<TipMotionField>,
}

impl TemporalMvContext {
    pub(crate) fn from_references(
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
        ref_frame_idx: &[u32],
        ref_valid: &[bool],
        ref_order_hint: &[u32],
        ref_motion_fields: &[Option<TemporalMotionField>],
    ) -> Option<Self> {
        let mut field = ProjectedTemporalMotionField::new(mi_rows, mi_cols)?;
        let ref_order_hints = reference_order_hints(ref_frame_idx, ref_valid, ref_order_hint);
        for (&slot, source_order_hint) in ref_frame_idx.iter().zip(ref_order_hints.iter().copied())
        {
            let Some(source_order_hint) = source_order_hint else {
                continue;
            };
            let Some(source_field) = ref_motion_fields
                .get(slot as usize)
                .and_then(Option::as_ref)
            else {
                continue;
            };
            project_temporal_motion_field(
                source_field,
                source_order_hint,
                current_order_hint,
                &mut field,
            );
        }
        Some(Self {
            current_order_hint,
            ref_order_hints,
            field,
            tip: None,
        })
    }

    pub(crate) fn prepare_tip(
        &mut self,
        projection_step: usize,
        superblock_size8: usize,
        fill_holes: bool,
    ) -> bool {
        let Some(references) = self.tip_reference_pair() else {
            return false;
        };
        let projection_step = projection_step.clamp(1, 2);
        let tmvp_unit_size8 = if projection_step == 1 {
            8
        } else {
            superblock_size8.max(1)
        };
        let mut field = ProjectedTemporalMotionField {
            width8: self.field.width8,
            height8: self.field.height8,
            cells: vec![ProjectedTemporalMotionCell::default(); self.field.cells.len()],
        };
        for y8 in (0..field.height8).step_by(projection_step) {
            for x8 in (0..field.width8).step_by(projection_step) {
                let Some(source) = self.field.cell(y8, x8) else {
                    continue;
                };
                if source.valid
                    && let Some(mv) =
                        project_mv(source.mv, references.ref_offset, source.ref_offset)
                {
                    field.set(y8, x8, mv, references.ref_offset, true);
                }
            }
        }
        if fill_holes {
            fill_tip_holes(&mut field, projection_step, tmvp_unit_size8);
            average_tip_motion(&mut field, projection_step, tmvp_unit_size8);
        }
        fill_tip_sampling_gaps(&mut field, projection_step);
        self.tip = Some(TipMotionField { field, references });
        true
    }

    pub(crate) fn tip_reference_pair(&self) -> Option<TipReferencePair> {
        tip_reference_pair_from_hints(self.current_order_hint, &self.ref_order_hints)
    }

    pub(crate) fn tip_references(&self) -> Option<TipReferencePair> {
        Some(self.tip.as_ref()?.references)
    }

    pub(crate) fn tip_candidate(&self, y8: usize, x8: usize, base_mv: Mv) -> Option<[Mv; 2]> {
        Some(self.tip.as_ref()?.candidate(y8, x8, base_mv))
    }

    pub(super) fn motion_field_mv(&self, ref_frame: i8, y8: usize, x8: usize) -> Option<Mv> {
        let dst_hint = usize::try_from(ref_frame)
            .ok()
            .and_then(|idx| self.ref_order_hints.get(idx))
            .copied()
            .flatten()?;
        let cell = self.field.cell(y8, x8)?;
        if !cell.valid {
            return None;
        }
        let ref_to_dst = super::super::get_relative_dist(
            self.current_order_hint as i32,
            i32::try_from(dst_hint).ok()?,
        );
        project_mv(cell.mv, ref_to_dst, cell.ref_offset)
    }

    pub(super) fn single_ref_weight(&self, ref_frame: i8) -> Option<u32> {
        let dst_hint = usize::try_from(ref_frame)
            .ok()
            .and_then(|idx| self.ref_order_hints.get(idx))
            .copied()
            .flatten()?;
        let dist = super::super::get_relative_dist(
            self.current_order_hint as i32,
            i32::try_from(dst_hint).ok()?,
        );
        Some(if dist.abs() <= 2 { 2 } else { 1 })
    }
}

pub(crate) fn reference_order_hints(
    ref_frame_idx: &[u32],
    ref_valid: &[bool],
    ref_order_hint: &[u32],
) -> Vec<Option<u32>> {
    ref_frame_idx
        .iter()
        .map(|&slot| {
            ref_valid
                .get(slot as usize)
                .copied()
                .filter(|valid| *valid)
                .and_then(|_| ref_order_hint.get(slot as usize).copied())
        })
        .collect()
}

pub(crate) fn tip_reference_pair_from_hints(
    current_order_hint: u32,
    ref_order_hints: &[Option<u32>],
) -> Option<TipReferencePair> {
    let current = i32::try_from(current_order_hint).ok()?;
    let mut closest_past: Option<(usize, i32, i32)> = None;
    let mut closest_future: Option<(usize, i32, i32)> = None;
    for (index, hint) in ref_order_hints.iter().copied().enumerate() {
        let Some(hint) = hint.and_then(|hint| i32::try_from(hint).ok()) else {
            continue;
        };
        let distance = super::super::get_relative_dist(current, hint);
        if distance > 0 && closest_past.is_none_or(|(_, old, _)| distance < old) {
            closest_past = Some((index, distance, hint));
        } else if distance < 0 && closest_future.is_none_or(|(_, old, _)| distance > old) {
            closest_future = Some((index, distance, hint));
        }
    }
    let (past_ref, past_offset, past_hint) = closest_past?;
    let (future_ref, future_offset, future_hint) = closest_future?;
    Some(TipReferencePair {
        past_ref: i8::try_from(past_ref).ok()?,
        future_ref: i8::try_from(future_ref).ok()?,
        past_offset,
        future_offset,
        ref_offset: super::super::get_relative_dist(future_hint, past_hint).min(MAX_FRAME_DISTANCE),
    })
}

fn fill_tip_holes(field: &mut ProjectedTemporalMotionField, step: usize, superblock_size8: usize) {
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let source = field.cell(y8, x8).unwrap_or_default();
                    for (dy, dx) in TIP_DIRECTIONS {
                        let dst_y = y8 as i32 + dy * step as i32;
                        let dst_x = x8 as i32 + dx * step as i32;
                        let Ok(dst_y) = usize::try_from(dst_y) else {
                            continue;
                        };
                        let Ok(dst_x) = usize::try_from(dst_x) else {
                            continue;
                        };
                        if dst_y >= block_y
                            && dst_y < end_y
                            && dst_x >= block_x
                            && dst_x < end_x
                            && !field.cell(dst_y, dst_x).is_some_and(|cell| cell.valid)
                        {
                            field.set(dst_y, dst_x, source.mv, source.ref_offset, source.valid);
                        }
                    }
                }
            }
        }
    }
}

fn average_tip_motion(
    field: &mut ProjectedTemporalMotionField,
    step: usize,
    superblock_size8: usize,
) {
    let mut averaged = field.clone();
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let mut sum = Mv::ZERO;
                    let mut count = 0usize;
                    for (dy, dx) in TIP_DIRECTIONS.into_iter().chain([(0, 0)]) {
                        let candidate_y = y8 as i32 + dy * step as i32;
                        let candidate_x = x8 as i32 + dx * step as i32;
                        let (Ok(candidate_y), Ok(candidate_x)) =
                            (usize::try_from(candidate_y), usize::try_from(candidate_x))
                        else {
                            continue;
                        };
                        if candidate_y < block_y
                            || candidate_y >= end_y
                            || candidate_x < block_x
                            || candidate_x >= end_x
                        {
                            continue;
                        }
                        let Some(cell) = field
                            .cell(candidate_y, candidate_x)
                            .filter(|cell| cell.valid)
                        else {
                            continue;
                        };
                        sum.row += cell.mv.row;
                        sum.col += cell.mv.col;
                        count += 1;
                    }
                    if count == 0 {
                        averaged.set(y8, x8, Mv::ZERO, 0, false);
                    } else {
                        averaged.set(
                            y8,
                            x8,
                            Mv {
                                row: divide_tip_average(sum.row, count),
                                col: divide_tip_average(sum.col, count),
                            },
                            field.cell(y8, x8).map_or(0, |cell| cell.ref_offset),
                            true,
                        );
                    }
                }
            }
        }
    }
    *field = averaged;
}

#[doc = "AV2 § 7.10.4 Weight_Div_Mult motion-vector average."]
fn divide_tip_average(value: i32, count: usize) -> i32 {
    const WEIGHTS: [i64; 6] = [0, 65_536, 32_768, 21_845, 16_384, 13_107];
    round2_signed(i64::from(value) * WEIGHTS[count], 16) as i32
}

fn fill_tip_sampling_gaps(field: &mut ProjectedTemporalMotionField, step: usize) {
    if step != 2 {
        return;
    }
    for y8 in (0..field.height8).step_by(2) {
        for x8 in (0..field.width8).step_by(2) {
            for (dy, dx) in [(0usize, 1usize), (1, 0), (1, 1)] {
                fill_tip_sampling_gap(field, y8, x8, dy, dx);
            }
        }
    }
}

#[doc = "AV2 § 7.10.5 fill_tpl and calc_avg motion-vector gap fill."]
fn fill_tip_sampling_gap(
    field: &mut ProjectedTemporalMotionField,
    y8: usize,
    x8: usize,
    dy: usize,
    dx: usize,
) {
    let Some(anchor) = field.cell(y8, x8).filter(|cell| cell.valid) else {
        return;
    };
    if y8 + dy >= field.height8 || x8 + dx >= field.width8 {
        return;
    }
    let mut sum = Mv::ZERO;
    let mut count = 0i32;
    for candidate_y in 0..=1 {
        for candidate_x in 0..=1 {
            if dy < candidate_y || dx < candidate_x {
                continue;
            }
            let source_y = y8 + 2 * candidate_y;
            let source_x = x8 + 2 * candidate_x;
            let Some(source) = field.cell(source_y, source_x).filter(|cell| cell.valid) else {
                continue;
            };
            let mv = if candidate_y == 0 && candidate_x == 0 {
                source.mv
            } else {
                project_mv(source.mv, anchor.ref_offset, source.ref_offset).unwrap_or(Mv::ZERO)
            };
            sum.row += mv.row;
            sum.col += mv.col;
            count += 1;
        }
    }
    let average = |value: i32| match count {
        1 => value,
        2 => round2_signed(i64::from(value), 1) as i32,
        3 => round2_signed(i64::from(value) * 85, 8) as i32,
        _ => round2_signed(i64::from(value), 2) as i32,
    };
    field.set(
        y8 + dy,
        x8 + dx,
        Mv {
            row: average(sum.row),
            col: average(sum.col),
        },
        anchor.ref_offset,
        true,
    );
}

fn project_temporal_motion_field(
    source: &TemporalMotionField,
    source_order_hint: u32,
    current_order_hint: u32,
    output: &mut ProjectedTemporalMotionField,
) {
    for y8 in 0..source.height8 {
        for x8 in 0..source.width8 {
            let Some(cell) = source.cell(y8, x8).filter(|cell| cell.is_valid()) else {
                continue;
            };
            for list in 0..2 {
                let Some(target_hint) = cell.ref_order_hints[list] else {
                    continue;
                };
                let source_hint = i32::try_from(source_order_hint).unwrap_or(i32::MAX);
                let target_hint = i32::try_from(target_hint).unwrap_or(i32::MAX);
                let current_hint = i32::try_from(current_order_hint).unwrap_or(i32::MAX);
                let mut ref_offset = super::super::get_relative_dist(source_hint, target_hint);
                if ref_offset == 0 || ref_offset.abs() > MAX_FRAME_DISTANCE {
                    continue;
                }
                let mut source_to_current =
                    super::super::get_relative_dist(source_hint, current_hint);
                if source_to_current.abs() > MAX_FRAME_DISTANCE {
                    continue;
                }
                let mut mv = uncompress_tmvp_mv(cell.mvs[list]);
                if ref_offset < 0 {
                    ref_offset = -ref_offset;
                    source_to_current = -source_to_current;
                    mv = Mv {
                        row: -mv.row,
                        col: -mv.col,
                    };
                }
                let Some(projected_to_current) = project_mv(mv, source_to_current, ref_offset)
                else {
                    continue;
                };
                let Some((pos_y8, pos_x8)) =
                    sampled_temporal_position(y8, x8, projected_to_current, output)
                else {
                    continue;
                };
                output.set_if_empty(pos_y8, pos_x8, mv, ref_offset);
            }
        }
    }
}

fn sampled_temporal_position(
    y8: usize,
    x8: usize,
    projected_mv: Mv,
    field: &ProjectedTemporalMotionField,
) -> Option<(usize, usize)> {
    let y8 = project_no_constraint(y8, projected_mv.row, field.height8)?;
    let x8 = project_no_constraint(x8, projected_mv.col, field.width8)?;
    Some((y8, x8))
}

fn project_no_constraint(v8: usize, delta: i32, max8: usize) -> Option<usize> {
    let offset8 = delta / (1 << (3 + 1 + 2));
    let projected = i32::try_from(v8).ok()?.checked_add(offset8)?;
    usize::try_from(projected)
        .ok()
        .filter(|&projected| projected < max8)
}

fn project_mv(mv: Mv, numerator: i32, denominator: i32) -> Option<Mv> {
    let denominator = denominator.clamp(1, MAX_FRAME_DISTANCE) as usize;
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let scale = DIV_MULT.get(denominator).copied()?;
    let bound = (1i64 << 16) - 1;
    let row = round2_signed(i64::from(mv.row) * i64::from(numerator) * scale, 14)
        .clamp(-bound, bound) as i32;
    let col = round2_signed(i64::from(mv.col) * i64::from(numerator) * scale, 14)
        .clamp(-bound, bound) as i32;
    Some(Mv { row, col })
}

fn compress_tmvp_mv(mv: Mv) -> Mv {
    Mv {
        row: compress_tmvp_component(mv.row),
        col: compress_tmvp_component(mv.col),
    }
}

fn uncompress_tmvp_mv(mv: Mv) -> Mv {
    Mv {
        row: uncompress_tmvp_component(mv.row),
        col: uncompress_tmvp_component(mv.col),
    }
}

fn compress_tmvp_component(value: i32) -> i32 {
    let abs_value = value.unsigned_abs();
    let msb = 31u32.saturating_sub(abs_value.leading_zeros());
    let step_log2 = msb.saturating_sub(4);
    let compressed = ((abs_value >> step_log2) + (step_log2 << 4)) as i32;
    if value < 0 { -compressed } else { compressed }
}

fn uncompress_tmvp_component(value: i32) -> i32 {
    let abs_value = value.unsigned_abs();
    let step_log2 = ((abs_value >> 4) as i32 - 1).max(0) as u32;
    let uncompressed = ((abs_value - (step_log2 << 4)) << step_log2) as i32;
    if value < 0 {
        -uncompressed
    } else {
        uncompressed
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn tip_context(
        current_order_hint: u32,
        ref_order_hints: Vec<Option<u32>>,
        mi_rows: usize,
        mi_cols: usize,
    ) -> TemporalMvContext {
        TemporalMvContext {
            current_order_hint,
            ref_order_hints,
            field: ProjectedTemporalMotionField::new(mi_rows, mi_cols).unwrap(),
            tip: None,
        }
    }

    #[test]
    fn tip_reference_pair_uses_the_nearest_past_and_future_references() {
        let context = tip_context(10, vec![Some(6), Some(9), Some(12), Some(15)], 4, 4);

        assert_eq!(
            context.tip_reference_pair(),
            Some(TipReferencePair {
                past_ref: 1,
                future_ref: 2,
                past_offset: 1,
                future_offset: -2,
                ref_offset: 3,
            })
        );
    }

    #[test]
    fn reference_order_hints_exclude_invalid_slots() {
        assert_eq!(
            reference_order_hints(&[0, 1, 2], &[true, false, true], &[8, 9, 12]),
            vec![Some(8), None, Some(12)]
        );
    }

    #[test]
    fn tip_projection_fills_unsampled_units_and_adds_the_block_mv() {
        let mut context = tip_context(10, vec![Some(8), Some(12)], 4, 4);
        context.field.set(0, 0, Mv { row: 8, col: -16 }, 4, true);

        assert!(context.prepare_tip(2, 2, false));
        assert_eq!(context.tip_references(), context.tip_reference_pair());
        let expected = [Mv { row: 5, col: -6 }, Mv { row: -3, col: 10 }];
        for y8 in 0..2 {
            for x8 in 0..2 {
                assert_eq!(
                    context.tip_candidate(y8, x8, Mv { row: 1, col: 2 }),
                    Some(expected)
                );
            }
        }
    }

    #[test]
    fn tip_step_one_hole_fill_stays_within_64_pixel_tmvp_units() {
        let mut context = tip_context(10, vec![Some(8), Some(12)], 2, 32);
        context.field.set(0, 7, Mv { row: 8, col: -16 }, 4, true);

        assert!(context.prepare_tip(1, 16, true));
        assert_eq!(context.tip_candidate(0, 8, Mv::ZERO), Some([Mv::ZERO; 2]));
    }
}
