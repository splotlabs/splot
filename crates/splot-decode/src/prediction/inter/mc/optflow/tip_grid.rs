// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The TIP grid: every plane of a § 7.13.5 TIP block whose 8x8
//! units each carry their own refine-MV candidates, predicted cell by cell
//! straight into the frame, with U and V sharing one pass of placements. A
//! grid of equal full-pel units is predicted as one block.

use super::*;

type Pred = [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2];
type Intermediate = [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE];

/// Predicts luma and both 4:2:0 chroma planes of `block` from a TIP grid of
/// 8x8 units with per-unit refine-MV candidates, or returns `false` for any
/// other block, a scaled reference, or a plane the frame edge clips. A
/// declined block may have had planes written; the general path rewrites
/// them.
pub(in crate::prediction::inter::mc) fn predict_tip_grid<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    motion: &CompoundMotionGrid,
    offset: ByteOffset,
) -> Result<bool> {
    let (
        CompoundBlend::Average {
            implicit_mask,
            cwp_weight,
        },
        RefinemvCandidates::PerCell { unit_size: 8, .. },
    ) = (block.blend, &motion.refinemv_candidates)
    else {
        return Ok(false);
    };
    let info = sink.info();
    let planes = super::super::mc_planes(info.pixel_format());
    if motion.unit_size != 8
        || block.sub8x8_chroma
        || block.warp_params.iter().any(Option::is_some)
        || planes[1..]
            .iter()
            .any(|&(_, sub_x, sub_y)| (sub_x, sub_y) != (1, 1))
    {
        return Ok(false);
    }
    let check_uniform = implicit_mask && cwp_weight == CWP_EQUAL;
    let rect = |plane, sub| {
        let (x, y, w, h) = block.rect.plane_rect(plane, sub, sub);
        PlaneRect::new(x, y, w, h)
    };
    let luma = rect(PlaneId::Y, 0)?;
    let Some(output) = sink.rect_mut(PlaneId::Y, luma) else {
        return Ok(false);
    };
    let pass = GridPass {
        info,
        block,
        motion,
        check_uniform,
        cwp_weight,
        offset,
    };
    if !pass.plane(PlaneId::Y, 0, [output, (&mut [], 0)])? {
        return Ok(false);
    }
    if !block.has_chroma {
        return Ok(true);
    }
    let Some([u, v]) = sink.chroma_rects_mut(rect(PlaneId::U, 1)?) else {
        return Ok(false);
    };
    pass.plane(PlaneId::U, 1, [u, v])
}

/// The block-wide inputs of a [`predict_tip_grid`] plane pass.
struct GridPass<'a, 'b, T: ReconSample> {
    info: DecodedFrameInfo,
    block: CompoundMcBlock<'b, T>,
    motion: &'a CompoundMotionGrid,
    check_uniform: bool,
    cwp_weight: i16,
    offset: ByteOffset,
}

impl<T: ReconSample> GridPass<'_, '_, T> {
    /// Predicts one plane at subsampling `sub` into `outputs[0]`, and for U
    /// the V plane into `outputs[1]` from the same placements; `false`
    /// without writing for a scaled reference or a grid that does not tile
    /// the plane.
    fn plane(&self, plane: PlaneId, sub: u32, outputs: [(&mut [T], usize); 2]) -> Result<bool> {
        let [(first, first_stride), (second, second_stride)] = outputs;
        if let (Some(first), Some(second)) = (T::u8_slice_mut(first), T::u8_slice_mut(second)) {
            return self.plane_into(plane, sub, [(first, first_stride), (second, second_stride)]);
        }
        match (T::u16_slice_mut(first), T::u16_slice_mut(second)) {
            (Some(first), Some(second)) => {
                self.plane_into(plane, sub, [(first, first_stride), (second, second_stride)])
            }
            _ => Ok(false),
        }
    }

    fn plane_into<O: CompoundAverageOutput>(
        &self,
        plane: PlaneId,
        sub: u32,
        mut outputs: [(&mut [O], usize); 2],
    ) -> Result<bool> {
        let prediction = super::super::compound_subpel_plane(
            self.info,
            self.block,
            plane,
            sub,
            sub,
            self.offset,
        )?;
        let size = 8 >> sub;
        let cells = self.motion.cells.as_slice();
        let (candidates, _, _) = self.motion.refinemv_candidate_slice();
        let columns = self.motion.columns;
        if prediction.scalings.into_iter().any(PlaneScaling::is_scaled)
            || columns == 0
            || candidates.len() != cells.len()
            || prediction.block_w != columns * size
            || prediction.block_h * columns != cells.len() * size
        {
            return Ok(false);
        }
        let second = if plane == PlaneId::U {
            Some(super::super::compound_plane_views(
                self.block,
                PlaneId::V,
                prediction.scalings,
                prediction.block_h,
                self.offset,
            )?)
        } else {
            None
        };
        let storage = self.info.storage_luma_size();
        let frame = (
            storage.width().div_ceil(1 << sub),
            storage.height().div_ceil(1 << sub),
        );
        let grid = GridBlock {
            prediction: &prediction,
            motion: self.motion,
            bit_depth: self.info.bit_depth(),
            frame,
            sub: (sub, sub),
            cwp_weight: self.cwp_weight,
        };
        let views = [Some(prediction.views), second];
        if self.motion.fullpel_runs
            && let Some([params0, params1]) = uniform_fullpel_params(
                &prediction,
                cells,
                (candidates, 1, 8),
                (sub, sub),
                (grid.bit_depth, self.block.interp, None),
                (
                    !self.check_uniform,
                    self.check_uniform,
                    self.cwp_weight,
                    frame,
                ),
            )
        {
            let whole = |views: &Option<[ReferencePlaneView<'_, T>; 2]>,
                         (output, stride): &mut (&mut [O], usize)| {
                views.as_ref().map_or(Ok(true), |views| {
                    splot_recon::subpel_predict_block_compound_average_fast_validated_strided_into(
                        &views[0],
                        &params0,
                        &views[1],
                        &params1,
                        self.cwp_weight,
                        output,
                        *stride,
                    )
                })
            };
            let [first, second] = &mut outputs;
            if whole(&views[0], first)? && whole(&views[1], second)? {
                return Ok(true);
            }
        }
        let mut state = CELL_PROGRAM.take().unwrap_or_default();
        let GridProgram {
            cells: program,
            pred,
            intermediate,
        } = &mut *state;
        program.clear();
        let run = |program: &mut CompoundCells,
                   outputs: &mut [(&mut [O], usize); 2],
                   pred: &mut Pred,
                   intermediate: &mut Intermediate|
         -> Result<()> {
            let [first, second] = outputs;
            let planes = [(&views[0], first), (&views[1], second)]
                .map(|(views, (output, stride))| Some((views.as_ref()?, &mut **output, *stride)));
            run_cell_program(
                program,
                planes,
                (&grid, self.block.interp, size),
                pred,
                intermediate,
            )?;
            program.clear();
            Ok(())
        };
        let rows = cells
            .chunks_exact(columns)
            .zip(candidates.chunks_exact(columns));
        let predict = || -> Result<()> {
            for (row, (cells, candidates)) in rows.enumerate() {
                let row = row * size;
                for (col, (&cell, &candidates)) in cells.iter().zip(candidates).enumerate() {
                    let col = col * size;
                    let (placements, uniform) = unscaled_cell_placements(
                        &prediction,
                        cell,
                        candidates,
                        [col, row, size, size],
                        (sub, sub),
                        self.check_uniform,
                        frame,
                    );
                    if !uniform {
                        let params = placements.map(|placement| {
                            unscaled_cell_params(
                                placement,
                                [size; 2],
                                (grid.bit_depth, self.block.interp),
                            )
                        });
                        for (views, (output, stride)) in views.iter().zip(outputs.iter_mut()) {
                            let (Some(views), Some(output)) =
                                (views, output.get_mut(row * *stride + col..))
                            else {
                                continue;
                            };
                            predict_grid_cell(
                                views,
                                (&params, false, [col, row, size, size]),
                                &grid,
                                pred,
                                intermediate,
                                output,
                                *stride,
                            )?;
                        }
                    } else if !program.push([row, col], placements) {
                        run(program, &mut outputs, pred, intermediate)?;
                        program.push([row, col], placements);
                    }
                }
            }
            run(program, &mut outputs, pred, intermediate)
        };
        let result = predict();
        CELL_PROGRAM.set(Some(state));
        result.map(|()| true)
    }
}
