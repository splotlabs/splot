// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed;

use super::{Mv, warp_sub_mv_at};

const MAX_FRAME_DISTANCE: i32 = 31;
const REFMVS_LIMIT: i32 = (1 << 11) - 1;
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
        let width8 = mi_cols.div_ceil(2);
        let height8 = mi_rows.div_ceil(2);
        let cells = width8.checked_mul(height8)?;
        Some(Self {
            width8,
            height8,
            cells: vec![TemporalMotionCell::default(); cells],
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
        if y8 >= self.height8 || x8 >= self.width8 {
            return None;
        }
        self.cells.get(y8 * self.width8 + x8).copied()
    }

    fn cell_mut(&mut self, y8: usize, x8: usize) -> Option<&mut TemporalMotionCell> {
        if y8 >= self.height8 || x8 >= self.width8 {
            return None;
        }
        self.cells.get_mut(y8 * self.width8 + x8)
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
        let width8 = mi_cols.div_ceil(2);
        let height8 = mi_rows.div_ceil(2);
        let cells = width8.checked_mul(height8)?;
        Some(Self {
            width8,
            height8,
            cells: vec![ProjectedTemporalMotionCell::default(); cells],
        })
    }

    fn cell(&self, y8: usize, x8: usize) -> Option<ProjectedTemporalMotionCell> {
        if y8 >= self.height8 || x8 >= self.width8 {
            return None;
        }
        self.cells.get(y8 * self.width8 + x8).copied()
    }

    fn set_if_empty(&mut self, y8: usize, x8: usize, mv: Mv, ref_offset: i32) {
        if y8 >= self.height8 || x8 >= self.width8 {
            return;
        }
        let Some(cell) = self.cells.get_mut(y8 * self.width8 + x8) else {
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMvContext {
    current_order_hint: u32,
    ref_order_hints: Vec<Option<u32>>,
    field: ProjectedTemporalMotionField,
}

impl TemporalMvContext {
    pub(crate) fn from_references(
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
        ref_frame_idx: &[u32],
        ref_order_hint: &[u32],
        ref_motion_fields: &[Option<TemporalMotionField>],
    ) -> Option<Self> {
        let mut field = ProjectedTemporalMotionField::new(mi_rows, mi_cols)?;
        for &slot in ref_frame_idx {
            let Some(source_order_hint) = ref_order_hint.get(slot as usize).copied() else {
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
        let ref_order_hints = ref_frame_idx
            .iter()
            .map(|&slot| ref_order_hint.get(slot as usize).copied())
            .collect();
        Some(Self {
            current_order_hint,
            ref_order_hints,
            field,
        })
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
