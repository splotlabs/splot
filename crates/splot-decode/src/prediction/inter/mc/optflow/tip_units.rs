// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The 8x8 or 16x16 units of a § 7.13.3.2 TIP block: every unit's
//! refinement, then every plane predicted cell by cell straight into the
//! frame, with U and V sharing one pass of placements. A block of equal
//! full-pel units is predicted as one block.

use super::*;
use crate::DecodeHeaderStateError;

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

/// Refines the `unit_size` units of the TIP block `block` in raster order of
/// `columns` per row, each from its § 7.11.3 `candidates`: optical flow
/// (8x8 units only), else the § 7.13.3.3 refine-MV search when the block
/// searches, else the candidates.
pub(crate) fn refine_tip_units<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    candidates: &[[Mv; 2]],
    units: &mut TipUnits,
    (columns, unit_size): (usize, usize),
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
    if block.optflow_distances.is_none() {
        search_tip_units(sink, block, candidates, cells, (columns, unit_size), offset)?;
        *fullpel_runs = fullpel && fullpel_run_pairs(cells, columns);
        return Ok(());
    }
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

/// The cells of units without optical flow: each unit's § 7.13.3.6 search
/// result when the block searches, else its candidates.
#[inline(never)]
fn search_tip_units<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    candidates: &[[Mv; 2]],
    cells: &mut [MotionCell],
    (columns, unit_size): (usize, usize),
    offset: ByteOffset,
) -> Result<()> {
    for (index, (cell, &mvs)) in cells.iter_mut().zip(candidates).enumerate() {
        let rect = McBlockRect::from_luma_rect(
            block.rect.luma_x + index % columns * unit_size,
            block.rect.luma_y + index / columns * unit_size,
            unit_size,
            unit_size,
        );
        let mut unit = *block;
        (unit.rect, unit.mv0, unit.mv1) = (rect, mvs[0], mvs[1]);
        *cell = MotionCell::from_refinemv(if block.search_refinemv {
            super::super::refinemv::search_refinemv(sink, &unit, rect, offset)?
        } else {
            mvs
        });
    }
    Ok(())
}

/// Predicts every plane of the TIP block `block` from the units
/// [`refine_tip_units`] refined from `candidates`.
pub(crate) fn predict_tip_units<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    candidates: &mut Vec<[Mv; 2]>,
    units: &mut TipUnits,
    (columns, unit_size): (usize, usize),
    offset: ByteOffset,
) -> Result<()> {
    let mut motion = CompoundMotionGrid {
        unit_size,
        columns,
        cells: MotionCells::Heap(core::mem::take(&mut units.cells)),
        refinemv_candidates: RefinemvCandidates::PerCell {
            candidates: core::mem::take(candidates),
            unit_size,
        },
        fullpel_runs: units.fullpel_runs,
    };
    let program = units.program.get_or_insert_default();
    let result = predict_grid(sink, block, &motion, program, offset);
    units.cells = motion.take_cells();
    *candidates = motion.take_candidates();
    result
}

/// Predicts luma and both chroma planes of `block` from its TIP units, of
/// which the frame storage (§ 5.18.3) keeps the visible part. A TIP block
/// averages (§ 7.13.3.2) unscaled references (§ 7.10.1), so any other block
/// is a state error.
fn predict_grid<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    motion: &CompoundMotionGrid,
    program: &mut GridProgram,
    offset: ByteOffset,
) -> Result<()> {
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = block.blend
    else {
        return Err(DecodeHeaderStateError::InvalidInterTipPredictionState.into());
    };
    debug_assert!(block.warp_params.iter().all(Option::is_none));
    let info = sink.info();
    let check_uniform = implicit_mask && cwp_weight == CWP_EQUAL;
    let [_, (_, sub_x, sub_y), _] = super::super::mc_planes(info.pixel_format());
    let storage = info.storage_luma_size();
    let rect = |plane, (sub_x, sub_y): (u32, u32)| {
        let (x, y, w, h) = block.rect.plane_rect(plane, sub_x, sub_y);
        let end_x = storage.width().div_ceil(1 << sub_x);
        let end_y = storage.height().div_ceil(1 << sub_y);
        PlaneRect::new(
            x,
            y,
            w.min(end_x.saturating_sub(x)),
            h.min(end_y.saturating_sub(y)),
        )
    };
    let luma = rect(PlaneId::Y, (0, 0))?;
    let output = sink
        .rect_mut(PlaneId::Y, luma)
        .ok_or(DecodeHeaderStateError::InvalidBlockGeometry)?;
    let pass = GridPass {
        info,
        block,
        motion,
        check_uniform,
        cwp_weight,
        offset,
    };
    let luma_outputs = [output, (&mut [][..], 0)];
    match motion.unit_size {
        8 => pass.plane::<8, 8, false>(PlaneId::Y, luma_outputs, program),
        16 => pass.plane::<16, 16, true>(PlaneId::Y, luma_outputs, program),
        _ => Err(DecodeHeaderStateError::InvalidBlockGeometry.into()),
    }?;
    if !block.has_chroma {
        return Ok(());
    }
    let chroma = sink
        .chroma_rects_mut(rect(PlaneId::U, (sub_x, sub_y))?)
        .ok_or(DecodeHeaderStateError::InvalidBlockGeometry)?;
    match (info.pixel_format(), motion.unit_size) {
        (PixelFormat::Yuv444, 8) => pass.plane::<8, 8, false>(PlaneId::U, chroma, program),
        (PixelFormat::Yuv444, _) => pass.plane::<16, 16, true>(PlaneId::U, chroma, program),
        (PixelFormat::Yuv422, 8) => pass.plane::<4, 8, false>(PlaneId::U, chroma, program),
        (PixelFormat::Yuv422, _) => pass.plane::<8, 16, true>(PlaneId::U, chroma, program),
        (_, 8) => pass.plane::<4, 4, false>(PlaneId::U, chroma, program),
        _ => pass.plane::<8, 8, true>(PlaneId::U, chroma, program),
    }
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
    /// Predicts one plane of `W` x `H` cells into the
    /// visible rectangle `outputs[0]`, and for U the V plane into
    /// `outputs[1]` from the same placements. Only `WIDE` (16x16) units cross
    /// the storage edge or merge full-pel runs per row.
    fn plane<const W: usize, const H: usize, const WIDE: bool>(
        &self,
        plane: PlaneId,
        outputs: [(&mut [T], usize); 2],
        program: &mut GridProgram,
    ) -> Result<()> {
        let [(first, first_stride), (second, second_stride)] = outputs;
        if let (Some(first), Some(second)) = (T::u8_slice_mut(first), T::u8_slice_mut(second)) {
            let outputs = [(first, first_stride), (second, second_stride)];
            return self.plane_into::<_, W, H, WIDE>(plane, outputs, program);
        }
        match (T::u16_slice_mut(first), T::u16_slice_mut(second)) {
            (Some(first), Some(second)) => {
                let outputs = [(first, first_stride), (second, second_stride)];
                self.plane_into::<_, W, H, WIDE>(plane, outputs, program)
            }
            _ => Err(DecodeHeaderStateError::InvalidInterTipPredictionState.into()),
        }
    }

    fn plane_into<O: CompoundAverageOutput, const W: usize, const H: usize, const WIDE: bool>(
        &self,
        plane: PlaneId,
        mut outputs: [(&mut [O], usize); 2],
        program: &mut GridProgram,
    ) -> Result<()> {
        let unit = if WIDE { 16 } else { 8 };
        let sub = (u32::from(W < unit), u32::from(H < unit));
        let prediction = super::super::compound_subpel_plane(
            self.info,
            self.block,
            plane,
            sub.0,
            sub.1,
            self.offset,
        )?;
        let cells = self.motion.cells.as_slice();
        let (candidates, _, _) = self.motion.refinemv_candidate_slice();
        let columns = self.motion.columns;
        if prediction.scalings.into_iter().any(PlaneScaling::is_scaled) {
            return Err(DecodeHeaderStateError::InvalidInterTipPredictionState.into());
        }
        if columns == 0
            || candidates.len() != cells.len()
            || prediction.block_w != columns * W
            || prediction.block_h * columns != cells.len() * H
        {
            return Err(DecodeHeaderStateError::InvalidBlockGeometry.into());
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
            storage.width().div_ceil(1 << sub.0),
            storage.height().div_ceil(1 << sub.1),
        );
        let visible = [
            prediction
                .block_w
                .min(frame.0.saturating_sub(prediction.plane_x)),
            prediction
                .block_h
                .min(frame.1.saturating_sub(prediction.plane_y)),
        ];
        let clipped = visible != [prediction.block_w, prediction.block_h];
        if clipped && !WIDE {
            return Err(DecodeHeaderStateError::InvalidBlockGeometry.into());
        }
        let grid = GridBlock {
            prediction: &prediction,
            motion: self.motion,
            bit_depth: self.info.bit_depth(),
            frame,
            sub,
            cwp_weight: self.cwp_weight,
        };
        let views = [Some(&prediction.views), second.as_ref()];
        if self.motion.fullpel_runs
            && !clipped
            && let Some([params0, params1]) = uniform_fullpel_params(
                &prediction,
                cells,
                (candidates, 1, self.motion.unit_size),
                sub,
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
                return Ok(());
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
                (&grid, self.block.interp, W),
                pred,
                intermediate,
            )?;
            program.clear();
            Ok(())
        };
        let runs = WIDE && self.motion.fullpel_runs && !clipped;
        let luma = views[1].is_none();
        let rows = cells
            .chunks_exact(columns)
            .zip(candidates.chunks_exact(columns));
        for (index, (row_cells, row_candidates)) in rows.enumerate() {
            let row = index * H;
            let mut merged = [0, u64::from(WIDE && luma).wrapping_neg()];
            if runs {
                let [(first, first_stride), (second, second_stride)] = &mut outputs;
                let second = views[1]
                    .map(|views| (views, &mut second[row * *second_stride..], *second_stride));
                let found = predict_fullpel_runs(
                    &prediction,
                    cells,
                    (candidates, 1, self.motion.unit_size),
                    index * columns,
                    [row, W, H],
                    sub,
                    (grid.bit_depth, self.block.interp, None),
                    (
                        !self.check_uniform,
                        self.check_uniform,
                        self.cwp_weight,
                        frame,
                    ),
                    &mut first[row * *first_stride..],
                    *first_stride,
                    second,
                )?;
                merged = [merged[0] | found[0], merged[1] | found[1]];
            }
            for (col, (&cell, &candidates)) in row_cells.iter().zip(row_candidates).enumerate() {
                let skip = merged.map(|mask| mask >> col.min(63) & 1 != 0);
                if WIDE && skip == [true; 2] {
                    continue;
                }
                let col = col * W;
                let (placements, uniform) = unscaled_cell_placements(
                    &prediction,
                    cell,
                    candidates,
                    [col, row, W, H],
                    sub,
                    self.check_uniform,
                    frame,
                );
                let edge = WIDE && (col + W > visible[0] || row + H > visible[1]);
                if uniform && W == H && W <= 8 && !edge && (!WIDE || skip == [false, luma]) {
                    if !program.push([row, col], placements) {
                        run(program, &mut outputs, pred, intermediate)?;
                        program.push([row, col], placements);
                    }
                    continue;
                }
                let params = placements.map(|placement| {
                    unscaled_cell_params(placement, [W, H], (grid.bit_depth, self.block.interp))
                });
                let cell = (&params, uniform, [col, row, W, H]);
                for ((views, (output, stride)), skip) in
                    views.iter().zip(outputs.iter_mut()).zip(skip)
                {
                    let (Some(views), Some(output), false) =
                        (views, output.get_mut(row * *stride + col..), skip)
                    else {
                        continue;
                    };
                    if !edge {
                        predict_grid_cell(views, cell, &grid, pred, intermediate, output, *stride)?;
                        continue;
                    }
                    let mut staged = [O::default(); MAX_MOTION_GRID_SUBBLOCK_SAMPLES];
                    predict_grid_cell(views, cell, &grid, pred, intermediate, &mut staged, W)?;
                    let width = W.min(visible[0] - col);
                    for (source, target) in staged
                        .chunks_exact(W)
                        .zip(output.chunks_mut(*stride))
                        .take(H.min(visible[1] - row))
                    {
                        target[..width].copy_from_slice(&source[..width]);
                    }
                }
            }
        }
        run(program, &mut outputs, pred, intermediate)
    }
}
