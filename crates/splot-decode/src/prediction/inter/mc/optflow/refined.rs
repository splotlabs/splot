// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! A compound block that is not TIP and refines its motion: the § 7.13.3.3
//! refine-MV units and the § 7.13.3.8 optical-flow cells of its luma, then
//! every plane predicted cell by cell straight into the frame, with U and V
//! sharing one pass of placements.

use super::super::refinemv::{SEARCH_AREA, SearchArea, search_unit};
use super::*;
use crate::DecodeHeaderStateError;

type Pred = [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2];
type Intermediate = [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE];

/// The cells of one refined block and the scratch that derives and predicts
/// them, kept by the caller so that no block allocates or zeroes them.
#[derive(Debug)]
pub(crate) struct RefinedUnits {
    cells: Vec<MotionCell>,
    columns: usize,
    /// The luma size cells are indexed by: the optical-flow cell, or 16 for
    /// the refine-MV units of a block without optical flow.
    unit_size: usize,
    fullpel_runs: bool,
    optflow: OptflowScratch,
    initial: [Vec<u16>; 2],
    bytes: [[u8; SEARCH_AREA]; 2],
    samples: [[u16; SEARCH_AREA]; 2],
    program: Option<Box<GridProgram>>,
}

impl Default for RefinedUnits {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            columns: 0,
            unit_size: 0,
            fullpel_runs: false,
            optflow: OptflowScratch::default(),
            initial: [Vec::new(), Vec::new()],
            bytes: [[0; SEARCH_AREA]; 2],
            samples: [[0; SEARCH_AREA]; 2],
            program: None,
        }
    }
}

impl RefinedUnits {
    /// The § 7.22 motion vectors of the 8x8 luma unit at offset `(x, y)`:
    /// the stored vectors of its cell, or for 4x4 cells the base vectors
    /// moved by the rounded mean of the four deltas, a cell outside the
    /// block counting as a zero delta.
    pub(crate) fn temporal_mvs_at_luma_offset(
        &self,
        x: usize,
        y: usize,
    ) -> splot_recon::Result<[Mv; 2]> {
        let cell = |x: usize, y: usize| {
            let column = x / self.unit_size.max(1);
            (column < self.columns)
                .then(|| {
                    self.cells
                        .get(y / self.unit_size.max(1) * self.columns + column)
                })
                .flatten()
                .copied()
        };
        let lookup = ReconError::ArithmeticOverflow {
            context: "refined motion lookup",
        };
        let first = cell(x, y).ok_or(lookup)?;
        if self.unit_size != 4 {
            return Ok(stored_mvs(first));
        }
        let mut delta_sum = [[0i32; 2]; 2];
        for (dx, dy) in [(0, 0), (4, 0), (0, 4), (4, 4)] {
            let Some(cell) = cell(x + dx, y + dy) else {
                continue;
            };
            for (reference, sum) in delta_sum.iter_mut().enumerate() {
                sum[0] += cell.mvs[reference][0] - cell.base_mvs[reference].row * 2;
                sum[1] += cell.mvs[reference][1] - cell.base_mvs[reference].col * 2;
            }
        }
        Ok(core::array::from_fn(|reference| Mv {
            row: first.base_mvs[reference].row + round2_signed_i32(delta_sum[reference][0], 3),
            col: first.base_mvs[reference].col + round2_signed_i32(delta_sum[reference][1], 3),
        }))
    }

    #[cfg(test)]
    pub(crate) fn cell_mvs(&self, index: usize) -> Option<[[i32; 2]; 2]> {
        self.cells.get(index).map(|cell| cell.mvs)
    }

    #[cfg(test)]
    pub(in crate::prediction::inter::mc) fn with_cells(
        cells: Vec<MotionCell>,
        columns: usize,
        unit_size: usize,
        fullpel_runs: bool,
    ) -> Self {
        Self {
            cells,
            columns,
            unit_size,
            fullpel_runs,
            ..Self::default()
        }
    }
}

/// Derives the luma motion of the refined compound `block` into `units`:
/// per § 7.13.3.3 refine-MV unit (16x16 at most) its § 7.13.3.6 search and,
/// with optical flow, the § 7.13.3.9 delta of each 8x8 cell, from the
/// unit's bilinear search area when the search built it; with optical flow
/// alone, the delta of each cell of the whole block (§ 7.13.3.8: 4x4 cells
/// in an 8x8 block, else 8x8). Optical flow and refine-MV need unscaled
/// references (§ 5.20.7.6 `opfl_allowed_for_refs`, § 5.20.7.17
/// `is_refinemv_allowed_reference`), so a scaled one is a state error.
pub(crate) fn refine_block_units<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    units: &mut RefinedUnits,
    offset: ByteOffset,
) -> Result<()> {
    let (width, height) = (block.rect.luma_w, block.rect.luma_h);
    let candidates = [block.mv0, block.mv1];
    let bit_depth = sink.info().bit_depth();
    let mut luma = LumaReference::pair(sink.info(), block, offset);
    if luma.iter().any(|luma| luma.scaling.is_scaled()) || width == 0 || height == 0 {
        return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
    }
    let RefinedUnits {
        cells,
        columns,
        unit_size,
        fullpel_runs,
        optflow,
        initial,
        bytes,
        samples,
        ..
    } = units;
    let [first, second] = initial;
    if !block.use_refinemv {
        let Some(distances) = block.optflow_distances else {
            return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
        };
        let size = super::optflow_unit_size(width, height);
        let len = width * height;
        let [pred0, pred1] = [first, second].map(|buffer| grown(buffer, len));
        for ((luma, mv), prediction) in luma
            .iter_mut()
            .zip(candidates)
            .zip([&mut *pred0, &mut *pred1])
        {
            initial_luma_prediction::<_, 0>(
                luma,
                block.rect,
                mv,
                InterpolationFilter::Bilinear,
                None,
                None,
                prediction,
            )?;
        }
        let deltas = derive_optflow_mv_deltas_into(
            pred0, pred1, width, height, size, bit_depth, distances, optflow,
        )?;
        cells.clear();
        cells.extend(
            deltas
                .iter()
                .map(|&delta| MotionCell::from_optflow(candidates, delta)),
        );
        (*columns, *unit_size) = (width / size, size);
        *fullpel_runs = fullpel_run_pairs(cells, *columns);
        return Ok(());
    }
    let (unit_w, unit_h) = (width.min(16), height.min(16));
    let (cell_w, cell_h) = match block.optflow_distances {
        Some(_) => (8, 8),
        None => (unit_w, unit_h),
    };
    (*columns, *unit_size) = (
        width / cell_w,
        if cell_w == 8 && cell_h == 8 { 8 } else { 16 },
    );
    cells.clear();
    cells.resize(
        *columns * (height / cell_h),
        MotionCell::from_refinemv(candidates),
    );
    let len = unit_w * unit_h;
    let [pred0, pred1] = [first, second].map(|buffer| grown(buffer, len));
    let allow_center = !block.refinemv_switchable;
    for y in (0..height).step_by(unit_h) {
        for x in (0..width).step_by(unit_w) {
            let rect = McBlockRect::from_luma_rect(
                block.rect.luma_x + x,
                block.rect.luma_y + y,
                unit_w,
                unit_h,
            );
            let (mvs, area) = if block.search_refinemv {
                search_unit(&mut luma, (rect, candidates, allow_center), bytes, samples)?
            } else {
                (candidates, SearchArea::None)
            };
            let Some(distances) = block.optflow_distances else {
                if let Some(cell) = cells.get_mut(y / unit_h * *columns + x / unit_w) {
                    *cell = MotionCell::from_refinemv(mvs);
                }
                continue;
            };
            let stride = unit_w + 8;
            let offsets = [
                (mvs[0].col - candidates[0].col) / 8,
                (mvs[0].row - candidates[0].row) / 8,
            ];
            for (reference, prediction) in [&mut *pred0, &mut *pred1].into_iter().enumerate() {
                let sign = if reference == 0 { 1 } else { -1 };
                let start =
                    (4 + sign * offsets[1]) as usize * stride + (4 + sign * offsets[0]) as usize;
                let copied = match area {
                    SearchArea::Bytes => {
                        copy_area(&bytes[reference], start, stride, prediction, unit_w)
                    }
                    SearchArea::Samples => {
                        copy_area(&samples[reference], start, stride, prediction, unit_w)
                    }
                    SearchArea::None => false,
                };
                if !copied {
                    initial_luma_prediction::<_, 0>(
                        &mut luma[reference],
                        rect,
                        mvs[reference],
                        InterpolationFilter::Bilinear,
                        Some((candidates[reference], unit_w, unit_h)),
                        None,
                        prediction,
                    )?;
                }
            }
            let deltas = derive_optflow_mv_deltas_into(
                pred0, pred1, unit_w, unit_h, 8, bit_depth, distances, optflow,
            )?;
            let unit_columns = unit_w / 8;
            for (index, &delta) in deltas.iter().enumerate() {
                let at = (y / 8 + index / unit_columns) * *columns + x / 8 + index % unit_columns;
                if let Some(cell) = cells.get_mut(at) {
                    *cell = MotionCell::from_optflow(mvs, delta);
                }
            }
        }
    }
    *fullpel_runs = fullpel_run_pairs(cells, *columns);
    Ok(())
}

/// The first `len` samples of `buffer`, grown without shrinking.
fn grown(buffer: &mut Vec<u16>, len: usize) -> &mut [u16] {
    if buffer.len() < len {
        buffer.resize(len, 0);
    }
    &mut buffer[..len]
}

/// Copies the `width`-wide rows of a search area from `start` at `stride`
/// into the packed `output`, or returns `false` when the area is too short.
fn copy_area<S: Copy + Into<u16>>(
    area: &[S],
    start: usize,
    stride: usize,
    output: &mut [u16],
    width: usize,
) -> bool {
    for (row, output) in output.chunks_exact_mut(width).enumerate() {
        let Some(source) = area
            .get(start + row * stride..)
            .and_then(|row| row.get(..width))
        else {
            return false;
        };
        for (sample, &source) in output.iter_mut().zip(source) {
            *sample = source.into();
        }
    }
    true
}

/// Predicts luma and both chroma planes of the refined `block` from the cells
/// [`refine_block_units`] derived; the frame storage (§ 5.18.3) keeps the
/// visible part. These blocks average (§ 5.20.7.16 sets COMPOUND_AVERAGE for
/// `use_optflow` and a switchable refine-MV, and clears a non-switchable
/// refine-MV for any other blend) without warp (§ 7.13.3.14 `useWarp = 0`),
/// so any other block is a state error. A sub-8x8 chroma block (§ 7.13.3.1
/// `sub8x8Inter`) predicts its chroma from list 0 alone, as AVM does.
pub(crate) fn predict_refined_block<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: &CompoundMcBlock<'_, T>,
    units: &mut RefinedUnits,
    offset: ByteOffset,
) -> Result<()> {
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = block.blend
    else {
        return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
    };
    if block.warp_params.iter().any(Option::is_some) {
        return Err(DecodeHeaderStateError::InvalidInterBlockModeState.into());
    }
    let info = sink.info();
    let program = units.program.get_or_insert_default();
    let pass = RefinedPass {
        info,
        block,
        cells: &units.cells,
        columns: units.columns,
        fullpel_runs: units.fullpel_runs,
        check_uniform: implicit_mask && cwp_weight == CWP_EQUAL,
        cwp_weight,
        offset,
    };
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
    let luma = sink
        .rect_mut(PlaneId::Y, rect(PlaneId::Y, (0, 0))?)
        .ok_or(DecodeHeaderStateError::InvalidBlockGeometry)?;
    let outputs = [luma, (&mut [][..], 0)];
    let (unit_w, unit_h) = (block.rect.luma_w.min(16), block.rect.luma_h.min(16));
    let refine_only = block.optflow_distances.is_none();
    match (units.unit_size, refine_only, unit_w, unit_h) {
        (4, false, ..) => pass.plane::<4, 4>(PlaneId::Y, outputs, program),
        (8, false, ..) | (_, true, 8, 8) => pass.plane::<8, 8>(PlaneId::Y, outputs, program),
        (_, true, 8, 16) => pass.plane::<8, 16>(PlaneId::Y, outputs, program),
        (_, true, 16, 8) => pass.plane::<16, 8>(PlaneId::Y, outputs, program),
        (_, true, 16, 16) => pass.plane::<16, 16>(PlaneId::Y, outputs, program),
        _ => Err(DecodeHeaderStateError::InvalidBlockGeometry.into()),
    }?;
    if !block.has_chroma {
        return Ok(());
    }
    let [_, (_, sub_x, sub_y), _] = super::super::mc_planes(info.pixel_format());
    if block.sub8x8_chroma {
        for plane in [PlaneId::U, PlaneId::V] {
            let (reference, rect, mv, interp) =
                (block.reference0, block.rect, block.mv0, block.interp);
            super::super::predict_plane(
                sink, reference, plane, rect, mv, interp, sub_x, sub_y, offset,
            )?;
        }
        return Ok(());
    }
    let chroma = sink
        .chroma_rects_mut(rect(PlaneId::U, (sub_x, sub_y))?)
        .ok_or(DecodeHeaderStateError::InvalidBlockGeometry)?;
    let cell = if refine_only {
        (unit_w >> sub_x, unit_h >> sub_y)
    } else {
        let size = units.unit_size;
        ((size >> sub_x).max(4), (size >> sub_y).max(4))
    };
    match cell {
        (4, 4) => pass.plane::<4, 4>(PlaneId::U, chroma, program),
        (4, 8) => pass.plane::<4, 8>(PlaneId::U, chroma, program),
        (8, 4) => pass.plane::<8, 4>(PlaneId::U, chroma, program),
        (8, 8) => pass.plane::<8, 8>(PlaneId::U, chroma, program),
        (4, 16) => pass.plane::<4, 16>(PlaneId::U, chroma, program),
        (8, 16) => pass.plane::<8, 16>(PlaneId::U, chroma, program),
        (16, 8) => pass.plane::<16, 8>(PlaneId::U, chroma, program),
        (16, 16) => pass.plane::<16, 16>(PlaneId::U, chroma, program),
        _ => Err(DecodeHeaderStateError::InvalidBlockGeometry.into()),
    }
}

/// The block-wide inputs of a [`predict_refined_block`] plane pass.
struct RefinedPass<'a, 'b, T: ReconSample> {
    info: DecodedFrameInfo,
    block: &'a CompoundMcBlock<'b, T>,
    cells: &'a [MotionCell],
    columns: usize,
    fullpel_runs: bool,
    check_uniform: bool,
    cwp_weight: i16,
    offset: ByteOffset,
}

impl<T: ReconSample> RefinedPass<'_, '_, T> {
    /// Predicts one plane of `W` x `H` cells into the visible rectangle
    /// `outputs[0]`, and for U the V plane into `outputs[1]` from the same
    /// placements.
    fn plane<const W: usize, const H: usize>(
        &self,
        plane: PlaneId,
        outputs: [(&mut [T], usize); 2],
        program: &mut GridProgram,
    ) -> Result<()> {
        let [(first, first_stride), (second, second_stride)] = outputs;
        if let (Some(first), Some(second)) = (T::u8_slice_mut(first), T::u8_slice_mut(second)) {
            let outputs = [(first, first_stride), (second, second_stride)];
            return self.plane_into::<_, W, H>(plane, outputs, program);
        }
        match (T::u16_slice_mut(first), T::u16_slice_mut(second)) {
            (Some(first), Some(second)) => {
                let outputs = [(first, first_stride), (second, second_stride)];
                self.plane_into::<_, W, H>(plane, outputs, program)
            }
            _ => Err(DecodeHeaderStateError::InvalidInterBlockModeState.into()),
        }
    }

    fn plane_into<O: CompoundAverageOutput, const W: usize, const H: usize>(
        &self,
        plane: PlaneId,
        mut outputs: [(&mut [O], usize); 2],
        program: &mut GridProgram,
    ) -> Result<()> {
        let [_, (_, sub_x, sub_y), _] = super::super::mc_planes(self.info.pixel_format());
        let sub = if plane == PlaneId::Y {
            (0, 0)
        } else {
            (sub_x, sub_y)
        };
        let block = self.block;
        let prediction = super::super::compound_subpel_plane(
            self.info,
            block,
            plane,
            sub.0,
            sub.1,
            self.offset,
        )?;
        let (block_w, block_h) = (prediction.block_w, prediction.block_h);
        if !block_w.is_multiple_of(W) || !block_h.is_multiple_of(H) {
            return Err(DecodeHeaderStateError::InvalidBlockGeometry.into());
        }
        let second = if plane == PlaneId::U {
            Some(super::super::compound_plane_views(
                block,
                PlaneId::V,
                prediction.scalings,
                block_h,
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
            block_w.min(frame.0.saturating_sub(prediction.plane_x)),
            block_h.min(frame.1.saturating_sub(prediction.plane_y)),
        ];
        let clipped = visible != [block_w, block_h];
        let bit_depth = self.info.bit_depth();
        let candidates = [block.mv0, block.mv1];
        let refine = if block.use_refinemv {
            (core::slice::from_ref(&candidates), 0, 16)
        } else {
            (&[][..], 0, 0)
        };
        let subblock_area = subblock_reference_area_size(plane, W, H);
        let area = match (block.use_refinemv, subblock_area) {
            (true, _) => Some([16 >> sub.0, 16 >> sub.1]),
            (false, Some((width, height))) => Some([width, height]),
            (false, None) => None,
        };
        let weights = (
            !self.check_uniform,
            self.check_uniform,
            self.cwp_weight,
            frame,
        );
        let fullpel = (bit_depth, block.interp, subblock_area);
        let views = [Some(&prediction.views), second.as_ref()];
        if self.fullpel_runs
            && !clipped
            && let Some([params0, params1]) =
                uniform_fullpel_params(&prediction, self.cells, refine, sub, fullpel, weights)
        {
            let whole = |views: Option<&[ReferencePlaneView<'_, T>; 2]>,
                         (output, stride): &mut (&mut [O], usize)| {
                views.map_or(Ok(true), |views| {
                    O::predict_fast(
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
        let cell_in = CellInputs {
            prediction: &prediction,
            bit_depth,
            interp: block.interp,
            cwp_weight: self.cwp_weight,
            frame,
            sub,
        };
        let run = |program: &mut CompoundCells,
                   outputs: &mut [(&mut [O], usize); 2],
                   pred: &mut Pred,
                   intermediate: &mut Intermediate|
         -> Result<()> {
            let [first, second] = outputs;
            let planes = [(views[0], first), (views[1], second)]
                .map(|(views, (output, stride))| Some((views?, &mut **output, *stride)));
            run_program::<_, _, W>(program, planes, &cell_in, pred, intermediate)?;
            program.clear();
            Ok(())
        };
        let origin = [prediction.plane_x, prediction.plane_y].map(|value| (value as i32) << 4);
        let mut bounds = [[0, 0, 0, 0]; 2];
        for (bounds, scaling) in bounds.iter_mut().zip(&prediction.scalings) {
            *bounds = [0, 0, scaling.last_x, scaling.last_y];
        }
        let runs = self.fullpel_runs && !clipped;
        let luma = views[1].is_none();
        let columns = self.columns;
        for cell_row in 0..block_h / H {
            let row = cell_row * H;
            if row >= visible[1] {
                break;
            }
            let mut merged = [0, u64::from(luma).wrapping_neg()];
            if runs {
                let [(first, first_stride), (second, second_stride)] = &mut outputs;
                let second = views[1]
                    .map(|views| (views, &mut second[row * *second_stride..], *second_stride));
                let found = predict_fullpel_runs(
                    &prediction,
                    self.cells,
                    refine,
                    cell_row * columns,
                    [row, W, H],
                    sub,
                    fullpel,
                    weights,
                    &mut first[row * *first_stride..],
                    *first_stride,
                    second,
                )?;
                merged = [merged[0] | found[0], merged[1] | found[1]];
            }
            for cell_col in 0..block_w / W {
                let col = cell_col * W;
                if col >= visible[0] {
                    break;
                }
                if let Some([area_w, area_h]) = area
                    && (col.is_multiple_of(area_w) || cell_col == 0)
                {
                    let (area_x, area_y) = (col / area_w * area_w, row / area_h * area_h);
                    let size = [area_w.min(block_w - area_x), area_h.min(block_h - area_y)];
                    let at = [
                        origin[0] + ((area_x as i32) << 4),
                        origin[1] + ((area_y as i32) << 4),
                    ];
                    for ((bounds, scaling), candidate) in
                        bounds.iter_mut().zip(&prediction.scalings).zip(candidates)
                    {
                        let area = super::super::refinemv::unscaled_area_bounds(
                            at,
                            size,
                            candidate,
                            [sub.0, sub.1],
                            scaling,
                        );
                        *bounds = [area.first_x, area.first_y, area.last_x, area.last_y];
                    }
                }
                let skip = merged.map(|mask| mask >> cell_col.min(63) & 1 != 0);
                if skip == [true; 2] {
                    continue;
                }
                let cell = *self.cells.get(cell_row * columns + cell_col).ok_or(
                    ReconError::ArithmeticOverflow {
                        context: "refined motion lookup",
                    },
                )?;
                let at = [
                    origin[0] + ((col as i32) << 4),
                    origin[1] + ((row as i32) << 4),
                ];
                let placements: [CellPlacement; 2] = core::array::from_fn(|reference| {
                    (
                        unscaled_cell_start(at, cell.mvs[reference], sub),
                        bounds[reference],
                    )
                });
                let uniform = !self.check_uniform || placements_inside(&placements, [W, H], frame);
                let edge = col + W > visible[0] || row + H > visible[1];
                if uniform && W == H && W <= 8 && !edge && skip == [false, luma] {
                    if !program.push([row, col], placements) {
                        run(program, &mut outputs, pred, intermediate)?;
                        program.push([row, col], placements);
                    }
                    continue;
                }
                let target = (cell, placements, uniform, [col, row, W, H]);
                for ((views, (output, stride)), skip) in
                    views.iter().zip(outputs.iter_mut()).zip(skip)
                {
                    let (Some(views), Some(output), false) =
                        (views, output.get_mut(row * *stride + col..), skip)
                    else {
                        continue;
                    };
                    if !edge {
                        predict_cell(views, target, &cell_in, pred, intermediate, output, *stride)?;
                        continue;
                    }
                    let mut staged = [O::default(); MAX_MOTION_GRID_SUBBLOCK_SAMPLES];
                    predict_cell(views, target, &cell_in, pred, intermediate, &mut staged, W)?;
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

/// The plane-wide inputs every cell of one plane pass shares.
struct CellInputs<'a, 'b, T: ReconSample> {
    prediction: &'a CompoundSubpelPlane<'b, T>,
    bit_depth: splot_recon::BitDepth,
    interp: InterpolationFilter,
    cwp_weight: i16,
    frame: (usize, usize),
    sub: (u32, u32),
}

/// One cell's motion, placements, uniform-weight flag and `[col, row,
/// width, height]`.
type Target = (MotionCell, [CellPlacement; 2], bool, [usize; 4]);

/// Runs a program of `SIZE`-square uniform cells into each plane (or U and
/// V) and predicts each cell it declines through [`predict_cell`].
fn run_program<T: ReconSample, O: CompoundAverageOutput, const SIZE: usize>(
    program: &mut CompoundCells,
    mut planes: [Option<GridPlane<'_, '_, T, O>>; 2],
    cell_in: &CellInputs<'_, '_, T>,
    pred: &mut Pred,
    intermediate: &mut Intermediate,
) -> Result<()> {
    if program.is_empty() {
        return Ok(());
    }
    let written = splot_recon::subpel_compound_cells(
        planes.each_mut().map(|plane| {
            let (views, output, stride) = plane.as_mut()?;
            Some((*views, &mut **output, *stride))
        }),
        cell_in.interp,
        SIZE,
        program,
        (cell_in.cwp_weight, cell_in.bit_depth),
    );
    for (plane, written) in planes.into_iter().zip(written) {
        let Some((views, output, stride)) = plane else {
            continue;
        };
        let stored = |index: usize| {
            written
                .get(index / 64)
                .is_some_and(|bits| bits >> (index % 64) & 1 != 0)
        };
        for index in (0..program.len()).filter(|&index| !stored(index)) {
            let Some(([row, col], placements)) = program.get(index) else {
                continue;
            };
            let target = (
                MotionCell::from_refinemv([Mv::ZERO; 2]),
                placements,
                true,
                [col, row, SIZE, SIZE],
            );
            predict_cell(
                views,
                target,
                cell_in,
                pred,
                intermediate,
                &mut output[row * stride + col..],
                stride,
            )?;
        }
    }
    Ok(())
}

/// Predicts one cell from `views` into `output`: the compound average of
/// § 7.13.3.14 when its weights are uniform, else the per-sample implicit
/// mask of the cell's own reference starts.
#[inline(never)]
fn predict_cell<T: ReconSample, O: CompoundAverageOutput>(
    views: &[ReferencePlaneView<'_, T>; 2],
    (cell, placements, uniform, [col, row, width, height]): Target,
    cell_in: &CellInputs<'_, '_, T>,
    pred: &mut Pred,
    intermediate: &mut Intermediate,
    output: &mut [O],
    output_stride: usize,
) -> Result<()> {
    let prediction = cell_in.prediction;
    let params = placements.map(|placement| {
        unscaled_cell_params(
            placement,
            [width, height],
            (cell_in.bit_depth, cell_in.interp),
        )
    });
    let (plane_x, plane_y) = (prediction.plane_x + col, prediction.plane_y + row);
    if uniform {
        let subplane = CompoundSubpelPlane {
            views: *views,
            plane_x,
            plane_y,
            block_w: width,
            block_h: height,
            scalings: prediction.scalings,
        };
        return Ok(super::super::predict_compound_average_into(
            &subplane,
            &params,
            cell_in.cwp_weight,
            Some(&mut pred[0]),
            Some(intermediate),
            output,
            output_stride,
        )?);
    }
    let [pred0, pred1] = pred;
    for ((view, pred), params) in views.iter().zip([&mut *pred0, &mut *pred1]).zip(&params) {
        subpel_predict_block_compound_intermediate_into(
            view,
            params,
            Some(intermediate),
            &mut pred[..width * height],
            width,
        )?;
    }
    let (sub_x, sub_y) = cell_in.sub;
    let starts = core::array::from_fn(|reference| {
        prediction.scalings[reference].with_prescaled_mv(
            plane_x as i32,
            plane_y as i32,
            cell.mvs[reference][0],
            cell.mvs[reference][1],
            sub_x,
            sub_y,
        )
    });
    Ok(super::super::blend::blend_nonuniform_implicit_mask(
        &pred0[..width * height],
        &pred1[..width * height],
        (cell_in.bit_depth, width),
        starts,
        cell_in.frame,
        output,
        output_stride,
    )?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn units(columns: usize, cells: Vec<MotionCell>) -> RefinedUnits {
        RefinedUnits {
            cells,
            columns,
            unit_size: 4,
            ..RefinedUnits::default()
        }
    }

    #[test]
    fn temporal_mvs_average_four_by_four_optflow_deltas_over_eight_by_eight() {
        let cell = |mvs| MotionCell {
            base_mvs: [Mv::ZERO; 2],
            mvs,
        };
        let units = units(
            2,
            vec![
                cell([[1, -1], [4, -4]]),
                cell([[2, -2], [4, -4]]),
                cell([[3, -3], [4, -4]]),
                cell([[4, -4], [4, -4]]),
            ],
        );
        assert_eq!(
            units.temporal_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 1, col: -1 }, Mv { row: 2, col: -2 }]
        );
    }

    #[test]
    fn temporal_mvs_treat_cropped_four_by_four_units_as_zero_delta() {
        let cell = MotionCell {
            base_mvs: [Mv::ZERO; 2],
            mvs: [[4, -4], [0, 0]],
        };
        assert_eq!(
            units(1, vec![cell; 2])
                .temporal_mvs_at_luma_offset(0, 0)
                .unwrap(),
            [Mv { row: 1, col: -1 }, Mv::ZERO]
        );
    }
}
