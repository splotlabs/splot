// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::derive_optflow_mv_deltas;
use splot_recon::math::round2_signed;

use super::*;
use crate::prediction::inter::mv_scaling::derive_plane_scaling_prescaled;

#[derive(Clone, Debug)]
pub(crate) struct CompoundMotionGrid {
    unit_size: usize,
    columns: usize,
    base_mvs: Vec<[Mv; 2]>,
    mvs: Vec<[[i32; 2]; 2]>,
    refinemv_candidates: Option<[Mv; 2]>,
}

impl CompoundMotionGrid {
    pub(super) fn from_refinemv(
        columns: usize,
        candidates: [Mv; 2],
        refined: Vec<[Mv; 2]>,
    ) -> Self {
        let mvs = refined
            .iter()
            .map(|mvs| {
                core::array::from_fn(|reference| [mvs[reference].row * 2, mvs[reference].col * 2])
            })
            .collect();
        Self {
            unit_size: 16,
            columns,
            base_mvs: refined,
            mvs,
            refinemv_candidates: Some(candidates),
        }
    }

    pub(super) const fn unit_size(&self) -> usize {
        self.unit_size
    }

    pub(super) fn at_luma_offset(&self, x: usize, y: usize) -> splot_recon::Result<[[i32; 2]; 2]> {
        Ok(self.cells_at_luma_offset(x, y)?.1)
    }

    fn cells_at_luma_offset(
        &self,
        x: usize,
        y: usize,
    ) -> splot_recon::Result<([Mv; 2], [[i32; 2]; 2])> {
        let index = self.index_at_luma_offset(x, y)?;
        let base_mvs = self
            .base_mvs
            .get(index)
            .copied()
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound base motion-grid lookup",
            })?;
        let mvs = self
            .mvs
            .get(index)
            .copied()
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound motion-grid lookup",
            })?;
        Ok((base_mvs, mvs))
    }

    pub(super) const fn refinemv_candidates(&self) -> Option<[Mv; 2]> {
        self.refinemv_candidates
    }

    pub(super) fn stored_mvs_at_luma_offset(
        &self,
        x: usize,
        y: usize,
    ) -> splot_recon::Result<[Mv; 2]> {
        let (base_mvs, refined) = self.cells_at_luma_offset(x, y)?;
        Ok(core::array::from_fn(|reference| {
            let base = base_mvs[reference];
            let row_delta = refined[reference][0] - base.row * 2;
            let col_delta = refined[reference][1] - base.col * 2;
            Mv {
                row: base.row + round2_signed(i64::from(row_delta), 1) as i32,
                col: base.col + round2_signed(i64::from(col_delta), 1) as i32,
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
        let (base_mvs, _) = self.cells_at_luma_offset(x, y)?;
        let mut delta_sum = [[0i64; 2]; 2];
        for dy in [0, 4] {
            for dx in [0, 4] {
                let Ok(index) = self.index_at_luma_offset(x + dx, y + dy) else {
                    continue;
                };
                let refined = self.mvs.get(index).ok_or(ReconError::ArithmeticOverflow {
                    context: "optical-flow temporal motion-grid lookup",
                })?;
                let unit_base = self
                    .base_mvs
                    .get(index)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "optical-flow temporal base-grid lookup",
                    })?;
                for reference in 0..2 {
                    delta_sum[reference][0] +=
                        i64::from(refined[reference][0] - unit_base[reference].row * 2);
                    delta_sum[reference][1] +=
                        i64::from(refined[reference][1] - unit_base[reference].col * 2);
                }
            }
        }
        Ok(core::array::from_fn(|reference| Mv {
            row: base_mvs[reference].row + round2_signed(delta_sum[reference][0], 3) as i32,
            col: base_mvs[reference].col + round2_signed(delta_sum[reference][1], 3) as i32,
        }))
    }

    fn index_at_luma_offset(&self, x: usize, y: usize) -> splot_recon::Result<usize> {
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
        if index >= self.mvs.len() {
            return Err(ReconError::ArithmeticOverflow {
                context: "compound motion-grid lookup",
            });
        }
        Ok(index)
    }
}

fn sets_subblock_reference_area(plane: PlaneId, width: usize, height: usize) -> bool {
    plane != PlaneId::Y || (width == 8 && height == 8)
}

pub(super) fn compound_motion_grid<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    block: CompoundMcBlock<'_, T>,
    unit_size: Option<usize>,
    refinemv: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    let Some(distances) = block.optflow_distances else {
        return Ok(refinemv.cloned());
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
        None if block.rect.luma_w <= 8 && block.rect.luma_h <= 8 => 4,
        None => 8,
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
    let mut base_cells = vec![None; cell_count];
    let mut motion_cells = vec![None; cell_count];
    let base_unit = refinemv.map_or(block.rect.luma_w.max(block.rect.luma_h), |grid| {
        grid.unit_size()
    });
    for region_y in (0..block.rect.luma_h).step_by(base_unit) {
        for region_x in (0..block.rect.luma_w).step_by(base_unit) {
            let region_w = (block.rect.luma_w - region_x).min(base_unit);
            let region_h = (block.rect.luma_h - region_y).min(base_unit);
            let base_mvs = if let Some(grid) = refinemv {
                grid.stored_mvs_at_luma_offset(region_x, region_y)?
            } else {
                [block.mv0, block.mv1]
            };
            let candidates = refinemv.and_then(CompoundMotionGrid::refinemv_candidates);
            let mut prediction_rect = block.rect;
            prediction_rect.luma_x += region_x;
            prediction_rect.luma_y += region_y;
            prediction_rect.luma_w = round_up(region_w)?;
            prediction_rect.luma_h = round_up(region_h)?;
            let pred0 = initial_luma_prediction(
                workspace,
                block.reference0,
                prediction_rect,
                base_mvs[0],
                InterpolationFilter::Bilinear,
                candidates.map(|mvs| (mvs[0], region_w, region_h)),
                offset,
            )?;
            let pred1 = initial_luma_prediction(
                workspace,
                block.reference1,
                prediction_rect,
                base_mvs[1],
                InterpolationFilter::Bilinear,
                candidates.map(|mvs| (mvs[1], region_w, region_h)),
                offset,
            )?;
            let deltas = derive_optflow_mv_deltas(
                &pred0,
                &pred1,
                prediction_rect.luma_w,
                prediction_rect.luma_h,
                unit_size,
                workspace.info().bit_depth(),
                distances,
            )?;
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
                let cell =
                    motion_cells
                        .get_mut(global_index)
                        .ok_or(ReconError::ArithmeticOverflow {
                            context: "optical-flow motion-grid write",
                        })?;
                let mut refined = [[0i32; 2]; 2];
                for reference in 0..2 {
                    let base = [base_mvs[reference].row, base_mvs[reference].col];
                    for component in 0..2 {
                        refined[reference][component] = clip3(
                            -(1 << 17),
                            (1 << 17) - 1,
                            i64::from(base[component]) * 2 + i64::from(delta[reference][component]),
                        ) as i32;
                    }
                }
                *cell = Some(refined);
                *base_cells
                    .get_mut(global_index)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "optical-flow base-grid write",
                    })? = Some(base_mvs);
            }
        }
    }
    let base_mvs = base_cells
        .into_iter()
        .map(|cell| {
            cell.ok_or(ReconError::ArithmeticOverflow {
                context: "optical-flow base-grid completeness",
            })
        })
        .collect::<splot_recon::Result<Vec<_>>>()?;
    let mvs = motion_cells
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
        base_mvs,
        mvs,
        refinemv_candidates: refinemv.and_then(|grid| grid.refinemv_candidates),
    }))
}

pub(super) fn initial_luma_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    refinemv_area: Option<(Mv, usize, usize)>,
    offset: ByteOffset,
) -> Result<Vec<u16>> {
    let (view, ref_mi_cols, ref_mi_rows) = reference_plane_view(reference, PlaneId::Y, offset)?;
    let scaling = derive_plane_scaling(
        rect.luma_x as i64,
        rect.luma_y as i64,
        i64::from(mv.row),
        i64::from(mv.col),
        0,
        0,
        ref_mi_cols,
        ref_mi_rows,
        rect.luma_w as i64,
        rect.luma_h as i64,
    );
    let bounds = refinemv_area.map(|(candidate, width, height)| {
        super::refinemv::reference_area_bounds(
            rect.luma_x as i64,
            rect.luma_y as i64,
            width,
            height,
            candidate,
            0,
            0,
            ref_mi_cols,
            ref_mi_rows,
        )
    });
    subpel_predict_block(
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
            bit_depth: workspace.info().bit_depth(),
        },
    )
    .map_err(Into::into)
}

pub(super) fn compound_optflow_plane_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
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
    let mut pred0 = vec![0i32; block_w * block_h];
    let mut pred1 = vec![0i32; block_w * block_h];

    for row in (0..block_h).step_by(subblock_h) {
        for col in (0..block_w).step_by(subblock_w) {
            let width = subblock_w.min(block_w - col);
            let height = subblock_h.min(block_h - row);
            let luma_x = col << sub_x;
            let luma_y = row << sub_y;
            let (base_mvs, mvs) = motion.cells_at_luma_offset(luma_x, luma_y)?;
            let candidates = motion.refinemv_candidates();
            for (reference, (view, ref_mi_cols, ref_mi_rows, output)) in [
                (&view0, ref_mi_cols0, ref_mi_rows0, &mut pred0),
                (&view1, ref_mi_cols1, ref_mi_rows1, &mut pred1),
            ]
            .into_iter()
            .enumerate()
            {
                let scaling = derive_plane_scaling_prescaled(
                    (plane_x + col) as i64,
                    (plane_y + row) as i64,
                    i64::from(mvs[reference][0]),
                    i64::from(mvs[reference][1]),
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
                        (plane_x + refine_col) as i64,
                        (plane_y + refine_row) as i64,
                        refine_w,
                        refine_h,
                        mvs[reference],
                        sub_x,
                        sub_y,
                        ref_mi_cols,
                        ref_mi_rows,
                    ))
                } else if sets_subblock_reference_area(plane, width, height) {
                    Some(super::refinemv::reference_area_bounds(
                        (plane_x + col) as i64,
                        (plane_y + row) as i64,
                        width,
                        height,
                        base_mvs[reference],
                        sub_x,
                        sub_y,
                        ref_mi_cols,
                        ref_mi_rows,
                    ))
                } else {
                    None
                };
                let predicted = subpel_predict_block_compound_intermediate(
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
                        bit_depth: workspace.info().bit_depth(),
                    },
                )?;
                for local_row in 0..height {
                    let source = &predicted[local_row * width..(local_row + 1) * width];
                    let start = (row + local_row) * block_w + col;
                    output[start..start + width].copy_from_slice(source);
                }
            }
        }
    }

    let scaling0 = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(block.mv0.row),
        i64::from(block.mv0.col),
        sub_x,
        sub_y,
        ref_mi_cols0,
        ref_mi_rows0,
        block_w as i64,
        block_h as i64,
    );
    let scaling1 = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(block.mv1.row),
        i64::from(block.mv1.col),
        sub_x,
        sub_y,
        ref_mi_cols1,
        ref_mi_rows1,
        block_w as i64,
        block_h as i64,
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
            base_mvs: vec![[Mv { row: 5, col: -5 }, Mv { row: -5, col: 5 }]],
            mvs: vec![[[7, -7], [-7, 7]]],
            refinemv_candidates: None,
        };

        assert_eq!(
            grid.stored_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 3, col: -3 }, Mv { row: -3, col: 3 }]
        );
    }

    #[test]
    fn temporal_mvs_average_four_by_four_optflow_deltas_over_eight_by_eight() {
        let grid = CompoundMotionGrid {
            unit_size: 4,
            columns: 2,
            base_mvs: vec![[Mv::ZERO; 2]; 4],
            mvs: vec![
                [[1, -1], [4, -4]],
                [[2, -2], [4, -4]],
                [[3, -3], [4, -4]],
                [[4, -4], [4, -4]],
            ],
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
            base_mvs: vec![[Mv::ZERO; 2]; 2],
            mvs: vec![[[4, -4], [0, 0]], [[4, -4], [0, 0]]],
            refinemv_candidates: None,
        };

        assert_eq!(
            grid.temporal_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 1, col: -1 }, Mv::ZERO]
        );
    }

    #[test]
    fn reference_areas_cover_eight_by_eight_luma_and_every_chroma_subblock() {
        assert!(sets_subblock_reference_area(PlaneId::Y, 8, 8));
        assert!(!sets_subblock_reference_area(PlaneId::Y, 4, 4));
        assert!(sets_subblock_reference_area(PlaneId::U, 4, 4));
    }
}
