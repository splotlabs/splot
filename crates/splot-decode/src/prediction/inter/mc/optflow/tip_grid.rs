// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit TIP grid: every plane of a § 7.13.5 TIP block whose 8x8
//! units each carry their own refine-MV candidates, predicted cell by cell
//! straight into the frame, with U and V sharing one pass of placements.

use super::*;

/// Predicts luma and both 4:2:0 chroma planes of `block` from a TIP grid of
/// 8x8 units with per-unit refine-MV candidates, or returns `false` for any
/// other block, a scaled reference, or a plane the frame edge clips. A
/// declined block may have had planes written; the general path rewrites
/// them.
pub(in crate::prediction::inter::mc) fn predict_tip_grid_u8<T: ReconSample>(
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
    if T::u8_slice(&[]).is_none()
        || motion.unit_size != 8
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
    let Some(output) = sink.u8_rect_mut(PlaneId::Y, luma) else {
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
    let Some([u, v]) = sink.u8_chroma_rects_mut(rect(PlaneId::U, 1)?) else {
        return Ok(false);
    };
    pass.plane(PlaneId::U, 1, [u, v])
}

/// The block-wide inputs of a [`predict_tip_grid_u8`] plane pass.
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
    fn plane(
        &self,
        plane: PlaneId,
        sub: u32,
        mut outputs: [(&mut [u8], usize); 2],
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
            let views = super::super::compound_plane_views(
                self.block,
                PlaneId::V,
                prediction.scalings,
                prediction.block_h,
                self.offset,
            )?;
            Some(views)
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
        let mut scratch = (
            [[0i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
            [0i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
        );
        let mut program = CELL_PROGRAM.take().unwrap_or_default();
        program.clear();
        let run = |program: &mut CompoundCells,
                   outputs: &mut [(&mut [u8], usize); 2],
                   (pred, intermediate): &mut (
            [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
            [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
        )|
         -> Result<()> {
            for (views, (output, stride)) in views.iter().zip(outputs.iter_mut()) {
                if let Some(views) = views {
                    run_cell_program(
                        program,
                        views,
                        (&grid, self.block.interp, size),
                        pred,
                        intermediate,
                        output,
                        *stride,
                    )?;
                }
            }
            program.clear();
            Ok(())
        };
        let mut result = Ok(());
        for (index, (&cell, &candidates)) in cells.iter().zip(candidates).enumerate() {
            let (row, col) = (index / columns * size, index % columns * size);
            let (placements, uniform) = unscaled_cell_placements(
                &prediction,
                cell,
                candidates,
                [col, row, size, size],
                (sub, sub),
                self.check_uniform,
                frame,
            );
            if uniform {
                if !program.push([row, col], placements) {
                    result = run(&mut program, &mut outputs, &mut scratch);
                    program.push([row, col], placements);
                }
            } else {
                result = self.mixed_cell(
                    &views,
                    &grid,
                    (placements, [col, row, size]),
                    &mut scratch,
                    &mut outputs,
                );
            }
            if result.is_err() {
                break;
            }
        }
        if result.is_ok() {
            result = run(&mut program, &mut outputs, &mut scratch);
        }
        CELL_PROGRAM.set(Some(program));
        result.map(|()| true)
    }

    /// Predicts one cell whose § 7.13.3.15 weights are not uniform into
    /// each output.
    fn mixed_cell(
        &self,
        views: &[Option<[ReferencePlaneView<'_, T>; 2]>; 2],
        grid: &GridBlock<'_, '_, T>,
        (placements, [col, row, size]): ([CellPlacement; 2], [usize; 3]),
        (pred, intermediate): &mut (
            [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
            [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
        ),
        outputs: &mut [(&mut [u8], usize); 2],
    ) -> Result<()> {
        let params = placements.map(|placement| {
            unscaled_cell_params(placement, [size; 2], (grid.bit_depth, self.block.interp))
        });
        for (views, (output, stride)) in views.iter().zip(outputs.iter_mut()) {
            let (Some(views), Some(output)) = (views, output.get_mut(row * *stride + col..)) else {
                continue;
            };
            predict_grid_cell(
                views,
                (&params, false, [col, row, size, size]),
                grid,
                pred,
                intermediate,
                output,
                *stride,
            )?;
        }
        Ok(())
    }
}
