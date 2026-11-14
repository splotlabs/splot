// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The 8x8 units of a § 7.13.3.2 TIP block with optical flow: every unit's
//! refinement, then every plane predicted cell by cell straight into the
//! frame, with U and V sharing one pass of placements. A block of equal
//! full-pel units is predicted as one block.

use super::*;

type Pred = [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2];
type Intermediate = [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE];

/// The refined units of one TIP block and the scratch that predicts them,
/// kept by the caller so that no block allocates or zeroes them.
#[derive(Debug, Default)]
pub(crate) struct TipUnits {
    pub(super) cells: Vec<MotionCell>,
    pub(super) fullpel_runs: bool,
    search: super::super::TipSearchScratch,
    program: Option<Box<GridProgram>>,
}

impl core::fmt::Debug for GridProgram {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GridProgram")
    }
}

impl TipUnits {
    /// The § 5.20.4 stored motion vectors of unit `index`.
    pub(crate) fn stored_mvs(&self, index: usize) -> Option<[Mv; 2]> {
        self.cells.get(index).copied().map(stored_mvs)
    }
}

/// Refines the 8x8 units of the TIP block `block` in raster order of
/// `columns` per row, each from its § 7.11.3 `candidates`.
pub(crate) fn refine_tip_units<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    candidates: &[[Mv; 2]],
    units: &mut TipUnits,
    columns: usize,
    offset: ByteOffset,
) -> Result<()> {
    if candidates.is_empty() || columns == 0 {
        return Err(ReconError::ZeroDimension {
            field: "TIP compound motion grid",
        }
        .into());
    }
    let unit_at = |index: usize| {
        let rect = McBlockRect::from_luma_rect(
            block.rect.luma_x + index % columns * 8,
            block.rect.luma_y + index / columns * 8,
            8,
            8,
        );
        (rect, candidates[index])
    };
    let TipUnits {
        cells,
        fullpel_runs,
        search,
        ..
    } = units;
    let fullpel = candidates
        .iter()
        .any(|&mvs| super::super::refinemv::fullpel_candidates(mvs));
    cells.clear();
    cells
        .try_reserve(candidates.len())
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "TIP compound motion grid",
        })?;
    cells.resize(
        candidates.len(),
        MotionCell::uninitialized([block.mv0, block.mv1]),
    );
    let fast = fullpel
        && super::super::refinemv::tip_fullpel_cells(
            sink,
            block,
            &unit_at,
            candidates,
            (8, offset),
            0,
            cells,
        )?;
    let mut luma = LumaReference::pair(sink.info(), block, offset);
    let mut carries: [BilinearOverlapCarry<'_, T>; 2] = Default::default();
    for (index, cell) in cells.iter_mut().enumerate() {
        if fast && cell.is_initialized() {
            continue;
        }
        let (rect, mvs) = unit_at(index);
        let refined = super::super::refinemv::tip_refinemv_optflow_motion_cell(
            block,
            (rect, mvs),
            search,
            &mut luma,
            &mut carries,
        )?;
        if let Some(refined) = refined {
            *cell = refined;
            continue;
        }
        let mut unit = *block;
        unit.rect = rect;
        unit.mv0 = mvs[0];
        unit.mv1 = mvs[1];
        unit.has_chroma = false;
        unit.sub8x8_chroma = false;
        *cell = tip_unit_motion_cell(sink, &unit, 8, offset, &mut luma)?;
    }
    *fullpel_runs = fullpel && fullpel_run_pairs(cells, columns);
    Ok(())
}

/// Predicts every plane of the TIP block `block` from the units
/// [`refine_tip_units`] refined from `candidates`.
pub(crate) fn predict_tip_units<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    candidates: &mut Vec<[Mv; 2]>,
    units: &mut TipUnits,
    columns: usize,
    offset: ByteOffset,
) -> Result<()> {
    let mut motion = CompoundMotionGrid {
        unit_size: 8,
        columns,
        cells: MotionCells::Heap(core::mem::take(&mut units.cells)),
        refinemv_candidates: RefinemvCandidates::PerCell {
            candidates: core::mem::take(candidates),
            unit_size: 8,
        },
        fullpel_runs: units.fullpel_runs,
    };
    let program = units.program.get_or_insert_default();
    let result = match predict_grid(sink, block, &motion, program, offset) {
        Ok(false) => {
            let mut block = *block;
            block.sub8x8_chroma = false;
            block.optflow_distances = None;
            block.use_refinemv = false;
            block.search_refinemv = false;
            super::super::predict_compound_into(sink, &block, Some(&motion), offset)
        }
        other => other.map(drop),
    };
    units.cells = motion.take_cells();
    *candidates = motion.take_candidates();
    result
}

/// Predicts luma and both 4:2:0 chroma planes of `block` from its TIP units,
/// or returns `false` for a scaled reference, another chroma format, or a
/// plane the frame edge clips. A declined block may have had planes written;
/// the general path rewrites them.
fn predict_grid<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    motion: &CompoundMotionGrid,
    program: &mut GridProgram,
    offset: ByteOffset,
) -> Result<bool> {
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = block.blend
    else {
        return Ok(false);
    };
    let info = sink.info();
    let planes = super::super::mc_planes(info.pixel_format());
    if block.warp_params.iter().any(Option::is_some)
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
    if !pass.plane::<8>(PlaneId::Y, [output, (&mut [], 0)], program)? {
        return Ok(false);
    }
    if !block.has_chroma {
        return Ok(true);
    }
    let Some([u, v]) = sink.chroma_rects_mut(rect(PlaneId::U, 1)?) else {
        return Ok(false);
    };
    pass.plane::<4>(PlaneId::U, [u, v], program)
}

/// The block-wide inputs of a [`predict_grid`] plane pass.
struct GridPass<'a, 'b, T: ReconSample> {
    info: DecodedFrameInfo,
    block: &'a CompoundMcBlock<'b, T>,
    motion: &'a CompoundMotionGrid,
    check_uniform: bool,
    cwp_weight: i16,
    offset: ByteOffset,
}

impl<T: ReconSample> GridPass<'_, '_, T> {
    /// Predicts one plane of `SIZE` cells into `outputs[0]`, and for U
    /// the V plane into `outputs[1]` from the same placements; `false`
    /// without writing for a scaled reference or units that do not tile
    /// the plane.
    fn plane<const SIZE: usize>(
        &self,
        plane: PlaneId,
        outputs: [(&mut [T], usize); 2],
        program: &mut GridProgram,
    ) -> Result<bool> {
        let [(first, first_stride), (second, second_stride)] = outputs;
        if let (Some(first), Some(second)) = (T::u8_slice_mut(first), T::u8_slice_mut(second)) {
            let outputs = [(first, first_stride), (second, second_stride)];
            return self.plane_into::<_, SIZE>(plane, outputs, program);
        }
        match (T::u16_slice_mut(first), T::u16_slice_mut(second)) {
            (Some(first), Some(second)) => {
                let outputs = [(first, first_stride), (second, second_stride)];
                self.plane_into::<_, SIZE>(plane, outputs, program)
            }
            _ => Ok(false),
        }
    }

    fn plane_into<O: CompoundAverageOutput, const SIZE: usize>(
        &self,
        plane: PlaneId,
        mut outputs: [(&mut [O], usize); 2],
        program: &mut GridProgram,
    ) -> Result<bool> {
        let (sub, size) = (u32::from(SIZE == 4), SIZE);
        let prediction = super::super::compound_subpel_plane(
            self.info,
            self.block,
            plane,
            sub,
            sub,
            self.offset,
        )?;
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
        let views = [Some(&prediction.views), second.as_ref()];
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
            let whole = |views: Option<&[ReferencePlaneView<'_, T>; 2]>,
                         (output, stride): &mut (&mut [O], usize)| {
                views.map_or(Ok(true), |views| {
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
            if whole(views[0], first)? && whole(views[1], second)? {
                return Ok(true);
            }
        }
        let GridProgram {
            cells: program,
            pred,
            intermediate,
        } = program;
        program.clear();
        let run = |program: &mut CompoundCells,
                   outputs: &mut [(&mut [O], usize); 2],
                   pred: &mut Pred,
                   intermediate: &mut Intermediate|
         -> Result<()> {
            let [first, second] = outputs;
            let planes = [(views[0], first), (views[1], second)]
                .map(|(views, (output, stride))| Some((views?, &mut **output, *stride)));
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
        run(program, &mut outputs, pred, intermediate)?;
        Ok(true)
    }
}
