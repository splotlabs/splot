// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::derive_optflow_mv_deltas;
use splot_recon::math::round2_signed_i32;

use super::*;
use crate::prediction::inter::mv_scaling::derive_plane_scaling_prescaled;

#[derive(Clone, Copy, Debug)]
pub(super) struct MotionCell {
    base_mvs: [Mv; 2],
    mvs: [[i32; 2]; 2],
}

impl MotionCell {
    pub(super) fn from_refinemv(base_mvs: [Mv; 2]) -> Self {
        let mvs = core::array::from_fn(|reference| {
            [base_mvs[reference].row * 2, base_mvs[reference].col * 2]
        });
        Self { base_mvs, mvs }
    }
}

#[derive(Debug)]
enum MotionCells {
    Inline(MotionCell),
    Heap(Vec<MotionCell>),
}

impl MotionCells {
    fn from_vec(cells: Vec<MotionCell>) -> Self {
        if let [cell] = cells.as_slice() {
            return Self::Inline(*cell);
        }
        Self::Heap(cells)
    }

    fn as_slice(&self) -> &[MotionCell] {
        match self {
            Self::Inline(cell) => core::slice::from_ref(cell),
            Self::Heap(cells) => cells,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CompoundMotionGrid {
    unit_size: usize,
    columns: usize,
    cells: MotionCells,
    refinemv_candidates: Option<[Mv; 2]>,
}

impl CompoundMotionGrid {
    pub(super) fn from_single_refinemv(candidates: [Mv; 2], cell: MotionCell) -> Self {
        Self {
            unit_size: 16,
            columns: 1,
            cells: MotionCells::Inline(cell),
            refinemv_candidates: Some(candidates),
        }
    }

    pub(super) fn from_refinemv(
        columns: usize,
        candidates: [Mv; 2],
        cells: Vec<MotionCell>,
    ) -> Self {
        Self {
            unit_size: 16,
            columns,
            cells: MotionCells::from_vec(cells),
            refinemv_candidates: Some(candidates),
        }
    }

    pub(super) const fn unit_size(&self) -> usize {
        self.unit_size
    }

    pub(super) fn at_luma_offset(&self, x: usize, y: usize) -> splot_recon::Result<[[i32; 2]; 2]> {
        Ok(self.cell_at_luma_offset(x, y)?.mvs)
    }

    pub(super) const fn refinemv_candidates(&self) -> Option<[Mv; 2]> {
        self.refinemv_candidates
    }

    pub(super) fn uniform_mvs(&self) -> Option<[[i32; 2]; 2]> {
        match self.cells.as_slice() {
            [cell] => Some(cell.mvs),
            _ => None,
        }
    }

    pub(super) fn stored_mvs_at_luma_offset(
        &self,
        x: usize,
        y: usize,
    ) -> splot_recon::Result<[Mv; 2]> {
        let cell = self.cell_at_luma_offset(x, y)?;
        Ok(core::array::from_fn(|reference| {
            let base = cell.base_mvs[reference];
            let row_delta = cell.mvs[reference][0] - base.row * 2;
            let col_delta = cell.mvs[reference][1] - base.col * 2;
            Mv {
                row: base.row + round2_signed_i32(row_delta, 1),
                col: base.col + round2_signed_i32(col_delta, 1),
            }
        }))
    }

    pub(crate) fn temporal_mvs_at_luma_offset(
        &self,
        x: usize,
        y: usize,
    ) -> splot_recon::Result<[Mv; 2]> {
        if self.unit_size != 4 {
            return self.stored_mvs_at_luma_offset(x, y);
        }
        let base_mvs = self.cell_at_luma_offset(x, y)?.base_mvs;
        let mut delta_sum = [[0i32; 2]; 2];
        for dy in [0, 4] {
            for dx in [0, 4] {
                let Ok(cell) = self.cell_at_luma_offset(x + dx, y + dy) else {
                    continue;
                };
                for (reference, sum) in delta_sum.iter_mut().enumerate() {
                    sum[0] += cell.mvs[reference][0] - cell.base_mvs[reference].row * 2;
                    sum[1] += cell.mvs[reference][1] - cell.base_mvs[reference].col * 2;
                }
            }
        }
        Ok(core::array::from_fn(|reference| Mv {
            row: base_mvs[reference].row + round2_signed_i32(delta_sum[reference][0], 3),
            col: base_mvs[reference].col + round2_signed_i32(delta_sum[reference][1], 3),
        }))
    }

    fn cell_at_luma_offset(&self, x: usize, y: usize) -> splot_recon::Result<MotionCell> {
        let column = x / self.unit_size;
        let row = y / self.unit_size;
        if column >= self.columns {
            return Err(ReconError::ArithmeticOverflow {
                context: "compound motion-grid lookup",
            });
        }
        let index = row
            .checked_mul(self.columns)
            .and_then(|row| row.checked_add(column))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound motion-grid index",
            })?;
        self.cells
            .as_slice()
            .get(index)
            .copied()
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound motion-grid lookup",
            })
    }
}

fn subblock_reference_area_size(
    plane: PlaneId,
    width: usize,
    height: usize,
) -> Option<(usize, usize)> {
    (plane != PlaneId::Y || (width == 8 && height == 8)).then_some((width, height))
}

fn normalized_sad(pred0: &[u16], pred1: &[u16], bit_depth: splot_recon::BitDepth) -> u32 {
    let sad = pred0
        .iter()
        .zip(pred1)
        .map(|(&a, &b)| u32::from(a.abs_diff(b)))
        .sum::<u32>();
    sad >> bit_depth.bits().saturating_sub(8)
}

pub(super) fn compound_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    unit_size: Option<usize>,
    refinemv: Option<CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    let Some(distances) = block.optflow_distances else {
        return Ok(refinemv);
    };
    let unit_size = match unit_size {
        Some(unit_size @ (4 | 8)) => unit_size,
        Some(unit_size) => {
            return Err(ReconError::InvalidOptflowUnitSize {
                unit_size,
                width: block.rect.luma_w,
                height: block.rect.luma_h,
            }
            .into());
        }
        None => super::optflow_unit_size(block.rect.luma_w, block.rect.luma_h),
    };
    let round_up = |value: usize| {
        value
            .div_ceil(unit_size)
            .checked_mul(unit_size)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "optical-flow prediction extent",
            })
    };
    let columns = round_up(block.rect.luma_w)? / unit_size;
    let rows = round_up(block.rect.luma_h)? / unit_size;
    let cell_count = columns
        .checked_mul(rows)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "optical-flow motion-grid size",
        })?;
    let mut cells: Option<Vec<Option<MotionCell>>> = None;
    let base_unit = refinemv
        .as_ref()
        .map_or(block.rect.luma_w.max(block.rect.luma_h), |grid| {
            grid.unit_size()
        });
    for region_y in (0..block.rect.luma_h).step_by(base_unit) {
        for region_x in (0..block.rect.luma_w).step_by(base_unit) {
            let region_w = (block.rect.luma_w - region_x).min(base_unit);
            let region_h = (block.rect.luma_h - region_y).min(base_unit);
            let base_mvs = if let Some(grid) = refinemv.as_ref() {
                grid.stored_mvs_at_luma_offset(region_x, region_y)?
            } else {
                [block.mv0, block.mv1]
            };
            let candidates = refinemv
                .as_ref()
                .and_then(CompoundMotionGrid::refinemv_candidates);
            let mut prediction_rect = block.rect;
            prediction_rect.luma_x += region_x;
            prediction_rect.luma_y += region_y;
            prediction_rect.luma_w = round_up(region_w)?;
            prediction_rect.luma_h = round_up(region_h)?;
            let deltas = super::with_initial_luma_predictions(
                prediction_rect.luma_w,
                prediction_rect.luma_h,
                |pred0, pred1| {
                    initial_luma_prediction(
                        sink,
                        block.reference0,
                        prediction_rect,
                        base_mvs[0],
                        InterpolationFilter::Bilinear,
                        candidates.map(|mvs| (mvs[0], region_w, region_h)),
                        offset,
                        pred0,
                    )?;
                    initial_luma_prediction(
                        sink,
                        block.reference1,
                        prediction_rect,
                        base_mvs[1],
                        InterpolationFilter::Bilinear,
                        candidates.map(|mvs| (mvs[1], region_w, region_h)),
                        offset,
                        pred1,
                    )?;
                    if block.optflow_sad_threshold.is_some_and(|threshold| {
                        normalized_sad(pred0, pred1, sink.info().bit_depth()) < threshold
                    }) {
                        return Ok(None);
                    }
                    Ok(Some(derive_optflow_mv_deltas(
                        pred0,
                        pred1,
                        prediction_rect.luma_w,
                        prediction_rect.luma_h,
                        unit_size,
                        sink.info().bit_depth(),
                        distances,
                    )?))
                },
            )?;
            let Some(deltas) = deltas else {
                return Ok(refinemv);
            };
            let cells = cells.get_or_insert_with(|| vec![None; cell_count]);
            let local_columns = prediction_rect.luma_w / unit_size;
            for (index, delta) in deltas.into_iter().enumerate() {
                let local_row = index / local_columns;
                let local_col = index % local_columns;
                let global_row = region_y / unit_size + local_row;
                let global_col = region_x / unit_size + local_col;
                let global_index = global_row
                    .checked_mul(columns)
                    .and_then(|row| row.checked_add(global_col))
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "optical-flow motion-grid index",
                    })?;
                let cell = cells
                    .get_mut(global_index)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "optical-flow motion-grid write",
                    })?;
                let mut refined = [[0i32; 2]; 2];
                for reference in 0..2 {
                    let base = [base_mvs[reference].row, base_mvs[reference].col];
                    for component in 0..2 {
                        refined[reference][component] = (base[component] * 2
                            + delta[reference][component])
                            .clamp(-(1 << 17), (1 << 17) - 1);
                    }
                }
                *cell = Some(MotionCell {
                    base_mvs,
                    mvs: refined,
                });
            }
        }
    }
    let cells = cells
        .unwrap_or_default()
        .into_iter()
        .map(|cell| {
            cell.ok_or(ReconError::ArithmeticOverflow {
                context: "optical-flow motion-grid completeness",
            })
        })
        .collect::<splot_recon::Result<Vec<_>>>()?;
    Ok(Some(CompoundMotionGrid {
        unit_size,
        columns,
        cells: MotionCells::from_vec(cells),
        refinemv_candidates: refinemv.and_then(|grid| grid.refinemv_candidates),
    }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn initial_luma_prediction<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    refinemv_area: Option<(Mv, usize, usize)>,
    offset: ByteOffset,
    output: &mut [u16],
) -> Result<()> {
    let (view, ref_mi_cols, ref_mi_rows) = reference_plane_view(reference, PlaneId::Y, offset)?;
    let scaling = derive_plane_scaling(
        rect.luma_x as i32,
        rect.luma_y as i32,
        mv.row,
        mv.col,
        0,
        0,
        ref_mi_cols,
        ref_mi_rows,
        rect.luma_w as i32,
        rect.luma_h as i32,
    );
    let bounds = refinemv_area.map(|(candidate, width, height)| {
        super::refinemv::reference_area_bounds(
            rect.luma_x as i32,
            rect.luma_y as i32,
            width,
            height,
            candidate,
            0,
            0,
            ref_mi_cols,
            ref_mi_rows,
        )
    });
    subpel_predict_block_into(
        &view,
        &SubpelPredictParams {
            interp,
            w: rect.luma_w,
            h: rect.luma_h,
            start_x: scaling.start_x,
            start_y: scaling.start_y,
            step_x: scaling.step_x,
            step_y: scaling.step_y,
            first_x: bounds.map_or(scaling.first_x, |bounds| bounds.first_x),
            first_y: bounds.map_or(scaling.first_y, |bounds| bounds.first_y),
            last_x: bounds.map_or(scaling.last_x, |bounds| bounds.last_x),
            last_y: bounds.map_or(scaling.last_y, |bounds| bounds.last_y),
            bit_depth: sink.info().bit_depth(),
        },
        output,
    )
    .map_err(Into::into)
}

pub(super) fn compound_optflow_plane_prediction<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let (view0, ref_mi_cols0, ref_mi_rows0) =
        reference_plane_view(block.reference0, plane, offset)?;
    let (view1, ref_mi_cols1, ref_mi_rows1) =
        reference_plane_view(block.reference1, plane, offset)?;
    let (plane_x, plane_y, block_w, block_h) = block.rect.plane_rect(plane, sub_x, sub_y);
    let subblock_w = (motion.unit_size >> sub_x).max(4);
    let subblock_h = (motion.unit_size >> sub_y).max(4);
    let [mut pred0, mut pred1] = super::take_compound_prediction_buffers(block_w * block_h);

    for row in (0..block_h).step_by(subblock_h) {
        for col in (0..block_w).step_by(subblock_w) {
            let width = subblock_w.min(block_w - col);
            let height = subblock_h.min(block_h - row);
            let luma_x = col << sub_x;
            let luma_y = row << sub_y;
            let cell = motion.cell_at_luma_offset(luma_x, luma_y)?;
            let base_mvs = cell.base_mvs;
            let mvs = cell.mvs;
            let candidates = motion.refinemv_candidates();
            for (reference, (view, ref_mi_cols, ref_mi_rows, output)) in [
                (&view0, ref_mi_cols0, ref_mi_rows0, &mut pred0),
                (&view1, ref_mi_cols1, ref_mi_rows1, &mut pred1),
            ]
            .into_iter()
            .enumerate()
            {
                let scaling = derive_plane_scaling_prescaled(
                    (plane_x + col) as i32,
                    (plane_y + row) as i32,
                    mvs[reference][0],
                    mvs[reference][1],
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                );
                let bounds = if let Some(mvs) = candidates {
                    let refine_unit_w = 16usize >> sub_x;
                    let refine_unit_h = 16usize >> sub_y;
                    let refine_col = col / refine_unit_w * refine_unit_w;
                    let refine_row = row / refine_unit_h * refine_unit_h;
                    let refine_w = refine_unit_w.min(block_w - refine_col);
                    let refine_h = refine_unit_h.min(block_h - refine_row);
                    Some(super::refinemv::reference_area_bounds(
                        (plane_x + refine_col) as i32,
                        (plane_y + refine_row) as i32,
                        refine_w,
                        refine_h,
                        mvs[reference],
                        sub_x,
                        sub_y,
                        ref_mi_cols,
                        ref_mi_rows,
                    ))
                } else if let Some((area_width, area_height)) =
                    subblock_reference_area_size(plane, subblock_w, subblock_h)
                {
                    Some(super::refinemv::reference_area_bounds(
                        (plane_x + col) as i32,
                        (plane_y + row) as i32,
                        area_width,
                        area_height,
                        base_mvs[reference],
                        sub_x,
                        sub_y,
                        ref_mi_cols,
                        ref_mi_rows,
                    ))
                } else {
                    None
                };
                let start = row * block_w + col;
                subpel_predict_block_compound_intermediate_into(
                    view,
                    &SubpelPredictParams {
                        interp: block.interp,
                        w: width,
                        h: height,
                        start_x: scaling.start_x,
                        start_y: scaling.start_y,
                        step_x: scaling.step_x,
                        step_y: scaling.step_y,
                        first_x: bounds.map_or(scaling.first_x, |bounds| bounds.first_x),
                        first_y: bounds.map_or(scaling.first_y, |bounds| bounds.first_y),
                        last_x: bounds.map_or(scaling.last_x, |bounds| bounds.last_x),
                        last_y: bounds.map_or(scaling.last_y, |bounds| bounds.last_y),
                        bit_depth: sink.info().bit_depth(),
                    },
                    &mut output[start..],
                    block_w,
                )?;
            }
        }
    }

    let scaling0 = derive_plane_scaling(
        plane_x as i32,
        plane_y as i32,
        block.mv0.row,
        block.mv0.col,
        sub_x,
        sub_y,
        ref_mi_cols0,
        ref_mi_rows0,
        block_w as i32,
        block_h as i32,
    );
    let scaling1 = derive_plane_scaling(
        plane_x as i32,
        plane_y as i32,
        block.mv1.row,
        block.mv1.col,
        sub_x,
        sub_y,
        ref_mi_cols1,
        ref_mi_rows1,
        block_w as i32,
        block_h as i32,
    );
    Ok(CompoundPlanePrediction {
        pred0,
        pred1,
        plane_x,
        plane_y,
        block_w,
        block_h,
        scaling0,
        scaling1,
        recycle_buffers: true,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn stored_mvs_round_refined_sixteenth_pel_values_to_eighth_pel() {
        let grid = CompoundMotionGrid {
            unit_size: 8,
            columns: 1,
            cells: MotionCells::Inline(MotionCell {
                base_mvs: [Mv { row: 5, col: -5 }, Mv { row: -5, col: 5 }],
                mvs: [[7, -7], [-7, 7]],
            }),
            refinemv_candidates: None,
        };

        assert_eq!(
            grid.stored_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 3, col: -3 }, Mv { row: -3, col: 3 }]
        );
    }

    #[test]
    fn uniform_mvs_requires_exactly_one_motion_cell() {
        let mvs = [[3, -5], [-7, 9]];
        let candidates = [Mv::ZERO; 2];
        let grid = CompoundMotionGrid::from_single_refinemv(
            candidates,
            MotionCell {
                base_mvs: candidates,
                mvs,
            },
        );
        assert!(matches!(&grid.cells, MotionCells::Inline(_)));
        assert_eq!(grid.uniform_mvs(), Some(mvs));

        let multiple = CompoundMotionGrid::from_refinemv(
            2,
            candidates,
            vec![
                MotionCell {
                    base_mvs: candidates,
                    mvs,
                };
                2
            ],
        );
        assert!(matches!(&multiple.cells, MotionCells::Heap(_)));
        assert_eq!(multiple.uniform_mvs(), None);
    }

    #[test]
    fn temporal_mvs_average_four_by_four_optflow_deltas_over_eight_by_eight() {
        let grid = CompoundMotionGrid {
            unit_size: 4,
            columns: 2,
            cells: MotionCells::Heap(vec![
                MotionCell {
                    base_mvs: [Mv::ZERO; 2],
                    mvs: [[1, -1], [4, -4]],
                },
                MotionCell {
                    base_mvs: [Mv::ZERO; 2],
                    mvs: [[2, -2], [4, -4]],
                },
                MotionCell {
                    base_mvs: [Mv::ZERO; 2],
                    mvs: [[3, -3], [4, -4]],
                },
                MotionCell {
                    base_mvs: [Mv::ZERO; 2],
                    mvs: [[4, -4], [4, -4]],
                },
            ]),
            refinemv_candidates: None,
        };

        assert_eq!(
            grid.temporal_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 1, col: -1 }, Mv { row: 2, col: -2 }]
        );
    }

    #[test]
    fn temporal_mvs_treat_cropped_four_by_four_units_as_zero_delta() {
        let grid = CompoundMotionGrid {
            unit_size: 4,
            columns: 1,
            cells: MotionCells::Heap(vec![
                MotionCell {
                    base_mvs: [Mv::ZERO; 2],
                    mvs: [[4, -4], [0, 0]],
                };
                2
            ]),
            refinemv_candidates: None,
        };

        assert_eq!(
            grid.temporal_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 1, col: -1 }, Mv::ZERO]
        );
    }

    #[test]
    fn reference_areas_keep_nominal_luma_and_chroma_subblock_sizes() {
        assert_eq!(subblock_reference_area_size(PlaneId::Y, 8, 8), Some((8, 8)));
        assert_eq!(subblock_reference_area_size(PlaneId::Y, 4, 4), None);
        assert_eq!(subblock_reference_area_size(PlaneId::U, 4, 4), Some((4, 4)));
    }
}
