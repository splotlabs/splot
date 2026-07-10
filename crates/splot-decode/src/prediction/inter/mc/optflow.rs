// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::derive_optflow_mv_deltas;
use splot_recon::math::round2_signed;

use super::*;
use crate::prediction::inter::mv_scaling::derive_plane_scaling_prescaled;

#[derive(Clone, Debug)]
pub(super) struct OptflowMotionGrid {
    unit_size: usize,
    columns: usize,
    base_mvs: [Mv; 2],
    mvs: Vec<[[i32; 2]; 2]>,
}

impl OptflowMotionGrid {
    fn at_luma_offset(&self, x: usize, y: usize) -> splot_recon::Result<[[i32; 2]; 2]> {
        let index = (y / self.unit_size)
            .checked_mul(self.columns)
            .and_then(|row| row.checked_add(x / self.unit_size))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "optical-flow motion-grid index",
            })?;
        self.mvs
            .get(index)
            .copied()
            .ok_or(ReconError::ArithmeticOverflow {
                context: "optical-flow motion-grid lookup",
            })
    }

    pub(super) fn stored_mvs_at_luma_offset(
        &self,
        x: usize,
        y: usize,
    ) -> splot_recon::Result<[Mv; 2]> {
        let refined = self.at_luma_offset(x, y)?;
        Ok(core::array::from_fn(|reference| {
            let base = self.base_mvs[reference];
            let row_delta = refined[reference][0] - base.row * 2;
            let col_delta = refined[reference][1] - base.col * 2;
            Mv {
                row: base.row + round2_signed(i64::from(row_delta), 1) as i32,
                col: base.col + round2_signed(i64::from(col_delta), 1) as i32,
            }
        }))
    }
}

pub(super) fn compound_optflow_motion_grid<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    block: CompoundMcBlock<'_, T>,
    unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<OptflowMotionGrid>> {
    let Some(distances) = block.optflow_distances else {
        return Ok(None);
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
    let mut prediction_rect = block.rect;
    let round_up = |value: usize| {
        value
            .div_ceil(unit_size)
            .checked_mul(unit_size)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "optical-flow prediction extent",
            })
    };
    prediction_rect.luma_w = round_up(prediction_rect.luma_w)?;
    prediction_rect.luma_h = round_up(prediction_rect.luma_h)?;
    let pred0 = initial_luma_prediction(
        workspace,
        block.reference0,
        prediction_rect,
        block.mv0,
        block.interp,
        offset,
    )?;
    let pred1 = initial_luma_prediction(
        workspace,
        block.reference1,
        prediction_rect,
        block.mv1,
        block.interp,
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
    let base = [
        [block.mv0.row, block.mv0.col],
        [block.mv1.row, block.mv1.col],
    ];
    let mvs = deltas
        .into_iter()
        .map(|delta| {
            let mut refined = [[0i32; 2]; 2];
            for reference in 0..2 {
                for component in 0..2 {
                    refined[reference][component] = clip3(
                        -(1 << 17),
                        (1 << 17) - 1,
                        i64::from(base[reference][component]) * 2
                            + i64::from(delta[reference][component]),
                    ) as i32;
                }
            }
            refined
        })
        .collect();
    Ok(Some(OptflowMotionGrid {
        unit_size,
        columns: prediction_rect.luma_w / unit_size,
        base_mvs: [block.mv0, block.mv1],
        mvs,
    }))
}

fn initial_luma_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
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
            first_x: scaling.first_x,
            first_y: scaling.first_y,
            last_x: scaling.last_x,
            last_y: scaling.last_y,
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
    optflow: &OptflowMotionGrid,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let (view0, ref_mi_cols0, ref_mi_rows0) =
        reference_plane_view(block.reference0, plane, offset)?;
    let (view1, ref_mi_cols1, ref_mi_rows1) =
        reference_plane_view(block.reference1, plane, offset)?;
    let (plane_x, plane_y, block_w, block_h) = block.rect.plane_rect(plane, sub_x, sub_y);
    let subblock_w = (optflow.unit_size >> sub_x).max(4);
    let subblock_h = (optflow.unit_size >> sub_y).max(4);
    let mut pred0 = vec![0i32; block_w * block_h];
    let mut pred1 = vec![0i32; block_w * block_h];

    for row in (0..block_h).step_by(subblock_h) {
        for col in (0..block_w).step_by(subblock_w) {
            let width = subblock_w.min(block_w - col);
            let height = subblock_h.min(block_h - row);
            let mvs = optflow.at_luma_offset(col << sub_x, row << sub_y)?;
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
                        first_x: scaling.first_x,
                        first_y: scaling.first_y,
                        last_x: scaling.last_x,
                        last_y: scaling.last_y,
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
        let grid = OptflowMotionGrid {
            unit_size: 8,
            columns: 1,
            base_mvs: [Mv { row: 5, col: -5 }, Mv { row: -5, col: 5 }],
            mvs: vec![[[7, -7], [-7, 7]]],
        };

        assert_eq!(
            grid.stored_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 3, col: -3 }, Mv { row: -3, col: 3 }]
        );
    }
}
