// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed_i32;
use splot_recon::{
    BilinearOverlapCarry, CompoundCells, OptflowScratch, derive_optflow_mv_delta_8x8_strided_into,
    derive_optflow_mv_deltas_into,
};

use super::blend::{ImplicitMaskBlend, blend_implicit_mask_region};
use super::*;

pub(super) trait CompoundAverageOutput: ReconSample {
    fn predict_second<T: ReconSample>(
        reference: &ReferencePlaneView<'_, T>,
        params: &SubpelPredictParams,
        pred0: &[i32],
        cwp_weight: i16,
        scratch: Option<&mut [i16]>,
        output: &mut [Self],
        output_stride: usize,
    ) -> splot_recon::Result<()>;

    #[allow(clippy::too_many_arguments)]
    fn predict_fast<T: ReconSample>(
        _reference0: &ReferencePlaneView<'_, T>,
        _params0: &SubpelPredictParams,
        _reference1: &ReferencePlaneView<'_, T>,
        _params1: &SubpelPredictParams,
        _cwp_weight: i16,
        _output: &mut [Self],
        _output_stride: usize,
    ) -> splot_recon::Result<bool> {
        Ok(false)
    }
}

impl CompoundAverageOutput for u16 {
    fn predict_second<T: ReconSample>(
        reference: &ReferencePlaneView<'_, T>,
        params: &SubpelPredictParams,
        pred0: &[i32],
        cwp_weight: i16,
        scratch: Option<&mut [i16]>,
        output: &mut [Self],
        output_stride: usize,
    ) -> splot_recon::Result<()> {
        subpel_predict_block_compound_average_strided_into(
            reference,
            params,
            pred0,
            cwp_weight,
            scratch,
            output,
            output_stride,
        )
    }

    #[allow(clippy::inline_always, reason = "per-cell grid hot path")]
    #[inline(always)]
    fn predict_fast<T: ReconSample>(
        reference0: &ReferencePlaneView<'_, T>,
        params0: &SubpelPredictParams,
        reference1: &ReferencePlaneView<'_, T>,
        params1: &SubpelPredictParams,
        cwp_weight: i16,
        output: &mut [Self],
        output_stride: usize,
    ) -> splot_recon::Result<bool> {
        subpel_predict_block_compound_average_fast_validated_strided_into(
            reference0,
            params0,
            reference1,
            params1,
            cwp_weight,
            output,
            output_stride,
        )
    }
}

impl CompoundAverageOutput for u8 {
    fn predict_second<T: ReconSample>(
        reference: &ReferencePlaneView<'_, T>,
        params: &SubpelPredictParams,
        pred0: &[i32],
        cwp_weight: i16,
        scratch: Option<&mut [i16]>,
        output: &mut [Self],
        output_stride: usize,
    ) -> splot_recon::Result<()> {
        subpel_predict_block_compound_average_strided_into_u8(
            reference,
            params,
            pred0,
            cwp_weight,
            scratch,
            output,
            output_stride,
        )
    }

    #[allow(clippy::inline_always, reason = "per-cell grid hot path")]
    #[inline(always)]
    fn predict_fast<T: ReconSample>(
        reference0: &ReferencePlaneView<'_, T>,
        params0: &SubpelPredictParams,
        reference1: &ReferencePlaneView<'_, T>,
        params1: &SubpelPredictParams,
        cwp_weight: i16,
        output: &mut [Self],
        output_stride: usize,
    ) -> splot_recon::Result<bool> {
        subpel_predict_block_compound_average_fast_validated_strided_into(
            reference0,
            params0,
            reference1,
            params1,
            cwp_weight,
            output,
            output_stride,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MotionCell {
    base_mvs: [Mv; 2],
    mvs: [[i32; 2]; 2],
}

impl MotionCell {
    pub(super) fn from_optflow(base_mvs: [Mv; 2], delta: [[i32; 2]; 2]) -> Self {
        let mut refined = [[0i32; 2]; 2];
        for reference in 0..2 {
            let base = [base_mvs[reference].row, base_mvs[reference].col];
            for component in 0..2 {
                refined[reference][component] = (base[component] * 2 + delta[reference][component])
                    .clamp(-(1 << 17), (1 << 17) - 1);
            }
        }
        Self {
            base_mvs,
            mvs: refined,
        }
    }

    fn uninitialized(base_mvs: [Mv; 2]) -> Self {
        Self {
            base_mvs,
            mvs: [[i32::MIN; 2]; 2],
        }
    }

    pub(super) fn is_initialized(&self) -> bool {
        self.mvs[0][0] != i32::MIN
    }

    pub(super) fn from_refinemv(base_mvs: [Mv; 2]) -> Self {
        let mvs = core::array::from_fn(|reference| {
            [base_mvs[reference].row * 2, base_mvs[reference].col * 2]
        });
        Self { base_mvs, mvs }
    }
}

/// The TIP plane pass's cells, which it lends back to the caller's scratch.
#[derive(Debug)]
enum MotionCells {
    Heap(Vec<MotionCell>),
}

/// The § 7.11.3 candidates of each TIP unit, whose refine-MV area clips it.
#[derive(Debug)]
enum RefinemvCandidates {
    PerCell {
        candidates: Vec<[Mv; 2]>,
        unit_size: usize,
    },
}

/// Largest motion-grid subblock (refine-MV unit): 16x16 samples.
const MAX_MOTION_GRID_SUBBLOCK_SAMPLES: usize = 256;
/// Horizontal-pass rows for an unscaled 16-sample-tall subblock: 16 + 7 taps.
const MAX_MOTION_GRID_SUBPEL_INTERMEDIATE: usize = 16 * (16 + 7);

/// A grid plane's cell program and the scratch of the cells it declines,
/// kept by the caller so that a plane neither zeroes nor allocates them.
struct GridProgram {
    cells: CompoundCells,
    pred: [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
    intermediate: [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
}

impl Default for GridProgram {
    fn default() -> Self {
        Self {
            cells: CompoundCells::default(),
            pred: [[0; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
            intermediate: [0; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
        }
    }
}

impl MotionCells {
    fn as_slice(&self) -> &[MotionCell] {
        match self {
            Self::Heap(cells) => cells,
        }
    }
}

/// The units of one TIP block in raster order, as the TIP plane pass reads
/// them.
#[derive(Debug)]
pub(crate) struct CompoundMotionGrid {
    unit_size: usize,
    columns: usize,
    cells: MotionCells,
    refinemv_candidates: RefinemvCandidates,
    /// Whether some row holds two adjacent equal full-pel cells, the
    /// precondition of every merged run.
    fullpel_runs: bool,
}

impl CompoundMotionGrid {
    /// Takes the per-cell candidate list back so the caller's context keeps it.
    fn take_candidates(&mut self) -> Vec<[Mv; 2]> {
        match &mut self.refinemv_candidates {
            RefinemvCandidates::PerCell { candidates, .. } => core::mem::take(candidates),
        }
    }

    fn take_cells(&mut self) -> Vec<MotionCell> {
        match &mut self.cells {
            MotionCells::Heap(cells) => core::mem::take(cells),
        }
    }

    pub(super) const fn unit_size(&self) -> usize {
        self.unit_size
    }

    pub(super) fn at_luma_offset(&self, x: usize, y: usize) -> splot_recon::Result<[[i32; 2]; 2]> {
        let column = x / self.unit_size;
        let index = (y / self.unit_size)
            .checked_mul(self.columns)
            .and_then(|row| row.checked_add(column))
            .filter(|_| column < self.columns);
        index
            .and_then(|index| self.cells.as_slice().get(index))
            .map(|cell| cell.mvs)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound motion-grid lookup",
            })
    }

    /// The refine-MV candidates as a slice that cell `index` reads at
    /// `index * step`, and their unit size.
    fn refinemv_candidate_slice(&self) -> (&[[Mv; 2]], usize, usize) {
        match &self.refinemv_candidates {
            RefinemvCandidates::PerCell {
                candidates,
                unit_size,
            } => (candidates, 1, *unit_size),
        }
    }
}

fn stored_mvs(cell: MotionCell) -> [Mv; 2] {
    core::array::from_fn(|reference| {
        let base = cell.base_mvs[reference];
        let row_delta = cell.mvs[reference][0] - base.row * 2;
        let col_delta = cell.mvs[reference][1] - base.col * 2;
        Mv {
            row: base.row + round2_signed_i32(row_delta, 1),
            col: base.col + round2_signed_i32(col_delta, 1),
        }
    })
}

fn subblock_reference_area_size(
    plane: PlaneId,
    width: usize,
    height: usize,
) -> Option<(usize, usize)> {
    (plane != PlaneId::Y || (width == 8 && height == 8)).then_some((width, height))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn tip_optflow_motion_cell_strided(
    pred0: &[u16],
    start0: usize,
    pred1: &[u16],
    start1: usize,
    stride: usize,
    bit_depth: splot_recon::BitDepth,
    distances: [i32; 2],
    sad_threshold: Option<u32>,
    base_mvs: [Mv; 2],
) -> Result<MotionCell> {
    if sad_threshold.is_some_and(|threshold| {
        let rows = (0..8).map(|row| {
            let at = |start: usize| start + row * stride..;
            let (left, right) = (pred0.get(at(start0)), pred1.get(at(start1)));
            (left.unwrap_or_default(), right.unwrap_or_default())
        });
        let sad = super::refinemv::sad_rows::<8>(rows, u16::MAX);
        sad >> bit_depth.bits().saturating_sub(8) < threshold
    }) {
        return Ok(MotionCell::from_refinemv(base_mvs));
    }
    let delta = derive_optflow_mv_delta_8x8_strided_into(
        pred0, start0, pred1, start1, stride, bit_depth, distances,
    )?;
    Ok(MotionCell::from_optflow(base_mvs, delta))
}

/// The motion cell of one 8x8 optical-flow TIP unit that the refine-MV
/// optical-flow search did not take: an unsearched unit, or one whose
/// § 7.13.3.6 search ends at once at its candidates (out of range). Its two
/// initial predictions and § 7.13.3.9 delta stay on the stack.
pub(super) fn tip_unit_motion_cell<'a, T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    unit: &CompoundMcBlock<'a, T>,
    unit_size: usize,
    _offset: ByteOffset,
    luma: &mut [LumaReference<'a, T>; 2],
) -> Result<MotionCell> {
    let mvs = [unit.mv0, unit.mv1];
    let (Some(distances), 8, 8, 8) = (
        unit.optflow_distances,
        unit_size,
        unit.rect.luma_w,
        unit.rect.luma_h,
    ) else {
        return Err(crate::DecodeHeaderStateError::InvalidInterTipPredictionState.into());
    };
    let mut predictions = [[0u16; 64]; 2];
    for (reference, (luma, prediction)) in luma.iter_mut().zip(&mut predictions).enumerate() {
        initial_luma_prediction::<_, 0>(
            luma,
            unit.rect,
            mvs[reference],
            InterpolationFilter::Bilinear,
            unit.use_refinemv.then_some((mvs[reference], 8, 8)),
            None,
            prediction,
        )?;
    }
    let [pred0, pred1] = &predictions;
    let bit_depth = sink.info().bit_depth();
    if unit.optflow_sad_threshold.is_some_and(|threshold| {
        let rows = pred0.chunks(8).zip(pred1.chunks(8));
        super::refinemv::sad_rows::<8>(rows, u16::MAX) >> bit_depth.bits().saturating_sub(8)
            < threshold
    }) {
        return Ok(MotionCell::from_refinemv(mvs));
    }
    let delta =
        derive_optflow_mv_delta_8x8_strided_into(pred0, 0, pred1, 0, 8, bit_depth, distances)?;
    Ok(MotionCell::from_optflow(mvs, delta))
}

/// Whether some row holds two adjacent equal cells, or every cell is equal,
/// with motion full-pel under either subsampling.
fn fullpel_run_pairs(cells: &[MotionCell], columns: usize) -> bool {
    let fullpel = |cell: &MotionCell| {
        cell.mvs
            .as_flattened()
            .iter()
            .all(|&mv| fullpel_phase(mv, 0) == 0 || fullpel_phase(mv, 1) == 0)
    };
    cells.chunks(columns.max(1)).any(|row| {
        row.windows(2)
            .any(|pair| pair[0] == pair[1] && fullpel(&pair[0]))
    }) || cells
        .first()
        .is_some_and(|first| fullpel(first) && cells.iter().all(|cell| cell == first))
}

/// One luma reference of a block's refine-MV and optical-flow predictions,
/// with the setup [`initial_luma_prediction`] would repeat for each unit: the
/// unit-independent plane scaling and the last plane view it fetched.
#[derive(Clone, Copy)]
pub(super) struct LumaReference<'a, T: ReconSample> {
    samples: ReferenceSamples<'a, T>,
    scaling: PlaneScaling,
    view: Option<ReferencePlaneView<'a, T>>,
    bit_depth: splot_recon::BitDepth,
    offset: ByteOffset,
}

impl<'a, T: ReconSample> LumaReference<'a, T> {
    pub(super) fn new(
        info: DecodedFrameInfo,
        samples: ReferenceSamples<'a, T>,
        offset: ByteOffset,
    ) -> Self {
        Self {
            samples,
            scaling: PlaneScaling::of_reference(samples.info(), info.coded_luma_size(), 0, 0),
            view: None,
            bit_depth: info.bit_depth(),
            offset,
        }
    }

    pub(super) fn pair(
        info: DecodedFrameInfo,
        block: &CompoundMcBlock<'a, T>,
        offset: ByteOffset,
    ) -> [Self; 2] {
        [block.reference0, block.reference1].map(|samples| Self::new(info, samples, offset))
    }

    pub(super) const fn bit_depth(&self) -> splot_recon::BitDepth {
        self.bit_depth
    }

    /// The luma view, published down to `last_row`: the last fetched view
    /// when its published rows reach that row, else a new one (with the same
    /// errors as fetching it per unit).
    fn view(&mut self, last_row: i32) -> Result<ReferencePlaneView<'a, T>> {
        if let Some(view) = self.view {
            let row = (last_row.max(0) as usize).min(view.height() - 1);
            if view.readable_row(row).is_some() {
                return Ok(view);
            }
        }
        let view = self.samples.plane_view(PlaneId::Y, last_row, self.offset)?;
        self.view = Some(view);
        Ok(view)
    }
}

/// Predicts the bilinear refine-MV or optical-flow luma area `rect`, `INSET`
/// samples in from each edge. `tip_centre` holds the carry of the 12x12 TIP
/// refine-MV centre, which then takes the dedicated kernel when it applies.
pub(super) fn initial_luma_prediction<'a, T: ReconSample, const INSET: usize>(
    luma: &mut LumaReference<'a, T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    refinemv_area: Option<(Mv, usize, usize)>,
    tip_centre: Option<&mut BilinearOverlapCarry<'a, T>>,
    output: &mut [u16],
) -> Result<()> {
    let scaling =
        luma.scaling
            .with_mv(rect.luma_x as i32, rect.luma_y as i32, mv.row, mv.col, 0, 0);
    let bounds = refinemv_area.map(|(candidate, width, height)| {
        if scaling.is_scaled() {
            return super::refinemv::reference_area_bounds(
                rect.luma_x as i32,
                rect.luma_y as i32,
                width,
                height,
                candidate,
                0,
                0,
                &scaling,
            );
        }
        super::refinemv::unscaled_area_bounds(
            [(rect.luma_x as i32) << 4, (rect.luma_y as i32) << 4],
            [width, height],
            candidate,
            [0, 0],
            &scaling,
        )
    });
    let params = SubpelPredictParams {
        interp,
        w: rect.luma_w.saturating_sub(2 * INSET),
        h: rect.luma_h.saturating_sub(2 * INSET),
        start_x: scaling.start_x + INSET as i32 * scaling.step_x,
        start_y: scaling.start_y + INSET as i32 * scaling.step_y,
        step_x: scaling.step_x,
        step_y: scaling.step_y,
        first_x: bounds.map_or(scaling.first_x, |bounds| bounds.first_x),
        first_y: bounds.map_or(scaling.first_y, |bounds| bounds.first_y),
        last_x: bounds.map_or(scaling.last_x, |bounds| bounds.last_x),
        last_y: bounds.map_or(scaling.last_y, |bounds| bounds.last_y),
        bit_depth: luma.bit_depth,
    };
    let view = luma.view(subpel_last_reference_row(&params))?;
    let (first, available) = (INSET * rect.luma_w + INSET, output.len());
    let output = output
        .get_mut(first..)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: first,
            actual: available,
        })?;
    if let Some(carry) = tip_centre
        && subpel_predict_12x12_bilinear_overlap_into(&view, &params, output, rect.luma_w, carry)?
    {
        return Ok(());
    }
    if INSET == 0 && super::refinemv::bilinear_search_window(&view, &params, output) {
        return Ok(());
    }
    subpel_predict_block_strided_into(&view, &params, output, rect.luma_w).map_err(Into::into)
}

/// Whether the § 7.13.3.6 search of the eight-bit refine-MV unit `rect` ends
/// at its biased centre SAD, read straight from the references when both
/// candidates are full-pel: the centre area of a full-sample window
/// [`search_windows_u8`] builds holds the reference samples unclipped.
/// `None` when a candidate is not full-pel or the byte kernel would not take
/// its window.
pub(super) fn fullpel_center_ends_search<T: ReconSample>(
    luma: &mut [LumaReference<'_, T>; 2],
    rect: McBlockRect,
    candidates: [Mv; 2],
) -> Result<Option<bool>> {
    let unit = [rect.luma_w.saturating_sub(8), rect.luma_h.saturating_sub(8)];
    if !matches!(unit, [8 | 16, 8 | 16]) {
        return Ok(None);
    }
    let [width, height] = [rect.luma_w, rect.luma_h];
    let origin = [(rect.luma_x as i32) << 4, (rect.luma_y as i32) << 4];
    let padding = super::refinemv::SEARCH_PADDING;
    let mut corners = [[0usize; 2]; 2];
    let mut views = [None; 2];
    for (((luma, candidate), corner), view) in luma
        .iter_mut()
        .zip(candidates)
        .zip(&mut corners)
        .zip(&mut views)
    {
        let scaling = luma.scaling;
        let bounds =
            super::refinemv::unscaled_area_bounds(origin, unit, candidate, [0, 0], &scaling);
        let mv = [2 * (candidate.row - padding), 2 * (candidate.col - padding)];
        let [x, y] = unscaled_cell_start(origin, mv, (0, 0));
        let [x0, y0] = [x >> 10, y >> 10];
        if scaling.is_scaled()
            || ((x | y) >> 6) & 15 != 0
            || [bounds.first_x, bounds.last_x, bounds.first_y, bounds.last_y]
                != [
                    x0 + 1,
                    x0 + width as i32 - 1,
                    y0 + 1,
                    y0 + height as i32 - 1,
                ]
            || x0 + 1 < 0
            || y0 + 1 < 0
        {
            return Ok(None);
        }
        *corner = [(x0 + 2) as usize, (y0 + 2) as usize];
        *view = Some(luma.view(bounds.last_y)?);
    }
    let [Some(first), Some(second)] = views else {
        return Ok(None);
    };
    let (sad_w, sad_h) = (width - 4, height - 4);
    let Some(sad) = splot_recon::refinemv_area_sad_u8([&first, &second], corners, [sad_w, sad_h])
    else {
        return Ok(None);
    };
    Ok(Some(sad - (sad >> 3) < (2 * sad_w * sad_h) as u32))
}

/// Predicts the eight-bit bilinear § 7.13.3.6 search windows `rect` of the
/// refine-MV unit four samples in from each edge, around `candidates`, into
/// `windows` as bytes, or returns `false` when the byte kernel does not
/// cover one: a scaled reference, or a unit whose § 7.13.3.5 reference area
/// the plane clamps.
pub(super) fn search_windows_u8<T: ReconSample, const N: usize>(
    luma: &mut [LumaReference<'_, T>; 2],
    rect: McBlockRect,
    candidates: [Mv; 2],
    windows: &mut [[u8; N]; 2],
) -> Result<bool> {
    let unit = [rect.luma_w.saturating_sub(8), rect.luma_h.saturating_sub(8)];
    if !matches!(unit, [8 | 16, 8 | 16]) {
        return Ok(false);
    }
    let origin = [(rect.luma_x as i32) << 4, (rect.luma_y as i32) << 4];
    let padding = super::refinemv::SEARCH_PADDING;
    for ((luma, candidate), window) in luma.iter_mut().zip(candidates).zip(windows) {
        let scaling = luma.scaling;
        if scaling.is_scaled() {
            return Ok(false);
        }
        let bounds =
            super::refinemv::unscaled_area_bounds(origin, unit, candidate, [0, 0], &scaling);
        let mv = [2 * (candidate.row - padding), 2 * (candidate.col - padding)];
        let params = unscaled_cell_params(
            (
                unscaled_cell_start(origin, mv, (0, 0)),
                [bounds.first_x, bounds.first_y, bounds.last_x, bounds.last_y],
            ),
            [rect.luma_w, rect.luma_h],
            (luma.bit_depth, InterpolationFilter::Bilinear),
        );
        let view = luma.view(params.last_y)?;
        if !splot_recon::subpel_predict_bilinear_search_window_u8(&view, &params, window) {
            return Ok(false);
        }
    }
    Ok(true)
}

type GridPlane<'a, 'b, T, O> = (&'a [ReferencePlaneView<'b, T>; 2], &'a mut [O], usize);

/// Runs a program of the uniform cells of a grid plane (or U and V) into
/// each plane and predicts each cell it declines through [`predict_grid_cell`].
fn run_cell_program<T: ReconSample, O: CompoundAverageOutput>(
    program: &mut CompoundCells,
    mut planes: [Option<GridPlane<'_, '_, T, O>>; 2],
    (grid, interp, size): (&GridBlock<'_, '_, T>, InterpolationFilter, usize),
    pred_scratch: &mut [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
    intermediate_scratch: &mut [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
) -> Result<()> {
    if program.is_empty() {
        return Ok(());
    }
    let written = splot_recon::subpel_compound_cells(
        planes.each_mut().map(|plane| {
            let (views, output, stride) = plane.as_mut()?;
            Some((*views, &mut **output, *stride))
        }),
        interp,
        size,
        program,
        (grid.cwp_weight, grid.bit_depth),
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
            let params = placements.map(|placement| {
                unscaled_cell_params(placement, [size; 2], (grid.bit_depth, interp))
            });
            predict_grid_cell(
                views,
                (&params, true, [col, row, size, size]),
                grid,
                pred_scratch,
                intermediate_scratch,
                &mut output[row * stride + col..],
                stride,
            )?;
        }
    }
    Ok(())
}

/// The placements of an unscaled cell whose refine-MV unit is the cell, and
/// whether its weights are uniform: each start is its 1/16-sample position,
/// and each refine-MV area starts at the candidate's integer sample (as in
/// `reference_area_bounds`).
#[allow(clippy::inline_always)]
#[inline(always)]
fn unscaled_cell_placements<T: ReconSample>(
    prediction: &CompoundSubpelPlane<'_, T>,
    cell: MotionCell,
    candidates: [Mv; 2],
    [col, row, width, height]: [usize; 4],
    sub: (u32, u32),
    check_uniform: bool,
    frame: (usize, usize),
) -> ([CellPlacement; 2], bool) {
    let origin = [
        ((prediction.plane_x + col) as i32) << 4,
        ((prediction.plane_y + row) as i32) << 4,
    ];
    let unit = (origin, [width, height], sub);
    let placements = [
        unscaled_cell_placement(&prediction.scalings[0], cell.mvs[0], candidates[0], unit),
        unscaled_cell_placement(&prediction.scalings[1], cell.mvs[1], candidates[1], unit),
    ];
    let uniform = !check_uniform || placements_inside(&placements, [width, height], frame);
    (placements, uniform)
}

/// Whether both placed `width` x `height` cells lie inside the frame, so
/// that § 7.13.3.15 weighs them uniformly.
#[allow(clippy::inline_always, reason = "per-cell grid hot path")]
#[inline(always)]
fn placements_inside(
    placements: &[CellPlacement; 2],
    [width, height]: [usize; 2],
    (frame_w, frame_h): (usize, usize),
) -> bool {
    let inside = |([x, y], _): &CellPlacement| {
        let (x, y) = (x >> 10, y >> 10);
        x >= 0
            && y >= 0
            && x + width as i32 <= frame_w as i32
            && y + height as i32 <= frame_h as i32
    };
    inside(&placements[0]) && inside(&placements[1])
}

/// The unscaled 1/1024-sample start of a prescaled MV from a cell's
/// 1/16-sample origin.
#[allow(clippy::inline_always, reason = "per-cell grid hot path")]
#[inline(always)]
fn unscaled_cell_start(
    [x16, y16]: [i32; 2],
    [mv_row, mv_col]: [i32; 2],
    (sub_x, sub_y): (u32, u32),
) -> [i32; 2] {
    [
        ((x16 + round2_signed_sub(mv_col, sub_x)) << 6) + 32,
        ((y16 + round2_signed_sub(mv_row, sub_y)) << 6) + 32,
    ]
}

/// A cell's 1/16-sample origin, size and subsampling.
type CellUnit = ([i32; 2], [usize; 2], (u32, u32));

/// One reference's placement of [`unscaled_cell_placements`] for the cell at
/// 1/16-sample `[x16, y16]`.
#[allow(clippy::inline_always, reason = "per-cell grid hot path")]
#[inline(always)]
fn unscaled_cell_placement(
    scaling: &PlaneScaling,
    [mv_row, mv_col]: [i32; 2],
    candidate: Mv,
    ([x16, y16], [width, height], (sub_x, sub_y)): CellUnit,
) -> CellPlacement {
    let bounds = super::refinemv::unscaled_area_bounds(
        [x16, y16],
        [width, height],
        candidate,
        [sub_x, sub_y],
        scaling,
    );
    (
        unscaled_cell_start([x16, y16], [mv_row, mv_col], (sub_x, sub_y)),
        [bounds.first_x, bounds.first_y, bounds.last_x, bounds.last_y],
    )
}

/// The unscaled parameters of a [`CellPlacement`].
#[allow(clippy::inline_always, reason = "per-cell grid hot path")]
#[inline(always)]
fn unscaled_cell_params(
    ([start_x, start_y], [first_x, first_y, last_x, last_y]): CellPlacement,
    [width, height]: [usize; 2],
    (bit_depth, interp): (splot_recon::BitDepth, InterpolationFilter),
) -> SubpelPredictParams {
    SubpelPredictParams {
        interp,
        w: width,
        h: height,
        start_x,
        start_y,
        step_x: 1 << 10,
        step_y: 1 << 10,
        first_x,
        first_y,
        last_x,
        last_y,
        bit_depth,
    }
}

/// `Round2Signed(mv, sub)` of a chroma subsampling shift `sub` (0 or 1): a
/// negative `mv` shifted down without the rounding bit is the negated
/// `Round2` of its magnitude.
#[inline]
fn round2_signed_sub(mv: i32, sub: u32) -> i32 {
    (mv + (((1 << sub) >> 1) & !(mv >> 31))) >> sub
}

/// One grid cell's derived parameters, uniform-weight flag and
/// `[col, row, width, height]`.
type GridCell<'a> = (&'a [SubpelPredictParams; 2], bool, [usize; 4]);

/// The block-level inputs every cell of one grid plane shares.
struct GridBlock<'a, 'b, T: ReconSample> {
    prediction: &'a CompoundSubpelPlane<'b, T>,
    motion: &'a CompoundMotionGrid,
    bit_depth: splot_recon::BitDepth,
    frame: (usize, usize),
    sub: (u32, u32),
    cwp_weight: i16,
}

/// Predicts one grid cell from `views` into `output`.
#[allow(clippy::inline_always, reason = "per-cell motion-grid hot path")]
#[inline(always)]
fn predict_grid_cell<T: ReconSample, O: CompoundAverageOutput>(
    views: &[ReferencePlaneView<'_, T>; 2],
    (params, uniform, [col, row, width, height]): GridCell<'_>,
    block: &GridBlock<'_, '_, T>,
    pred_scratch: &mut [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
    intermediate_scratch: &mut [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
    output: &mut [O],
    output_stride: usize,
) -> Result<()> {
    let prediction = block.prediction;
    if !uniform {
        let [pred0, pred1] = pred_scratch;
        let preds = views.iter().zip([&mut *pred0, &mut *pred1]);
        for ((view, pred), params) in preds.zip(params) {
            subpel_predict_block_compound_intermediate_into(
                view,
                params,
                Some(intermediate_scratch),
                &mut pred[..width * height],
                width,
            )?;
        }
        let (frame_w, frame_h) = block.frame;
        return Ok(blend_implicit_mask_region(
            [&pred0[..], &pred1[..]],
            width,
            [col, row, width, height],
            block.motion,
            (prediction.plane_x, prediction.plane_y),
            prediction.scalings,
            ImplicitMaskBlend::new(block.bit_depth, frame_w, frame_h),
            block.sub,
            output,
            output_stride,
        )?);
    }
    let subplane = CompoundSubpelPlane {
        views: *views,
        plane_x: prediction.plane_x + col,
        plane_y: prediction.plane_y + row,
        block_w: width,
        block_h: height,
        scalings: prediction.scalings,
    };
    Ok(super::predict_compound_average_into(
        &subplane,
        params,
        block.cwp_weight,
        Some(&mut pred_scratch[0]),
        Some(intermediate_scratch),
        output,
        output_stride,
    )?)
}

/// The unscaled start phase `(start >> 6) & 15` of a prescaled MV component:
/// the low four bits of `Round2Signed(mv, sub)`, which are zero exactly when
/// they are for `|mv|`.
#[inline]
fn fullpel_phase(mv: i32, sub: u32) -> u32 {
    ((mv.unsigned_abs() + ((1 << sub) >> 1)) >> sub) & 15
}

#[inline]
fn fullpel_motion(mvs: [[i32; 2]; 2], sub_x: u32, sub_y: u32) -> bool {
    mvs.iter()
        .all(|mv| fullpel_phase(mv[0], sub_y) | fullpel_phase(mv[1], sub_x) == 0)
}

/// The reference views, packed output and output stride of the V plane that
/// a shared U pass predicts too.
type SecondPlane<'a, 'b, T, O> = (&'a [ReferencePlaneView<'b, T>; 2], &'a mut [O], usize);

mod fullpel_runs;
mod refined;
mod tip_units;

use fullpel_runs::{predict_fullpel_runs, uniform_fullpel_params};
pub(crate) use refined::{RefinedUnits, predict_refined_block, refine_block_units};
pub(crate) use tip_units::{TipUnits, predict_tip_units, refine_tip_units};

#[cfg(test)]
#[path = "optflow_run_tests.rs"]
mod run_tests;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::blend::blend_nonuniform_implicit_mask;
    use super::*;
    use crate::prediction::inter::mv_scaling::derive_plane_scaling;

    #[test]
    fn subsampling_rounding_matches_round2_signed() {
        for mv in -(1 << 17)..=(1 << 17) {
            for sub in 0..=1 {
                assert_eq!(round2_signed_sub(mv, sub), round2_signed_i32(mv, sub));
            }
        }
    }

    #[test]
    fn stored_mvs_round_refined_sixteenth_pel_values_to_eighth_pel() {
        let cell = MotionCell {
            base_mvs: [Mv { row: 5, col: -5 }, Mv { row: -5, col: 5 }],
            mvs: [[7, -7], [-7, 7]],
        };
        assert_eq!(
            stored_mvs(cell),
            [Mv { row: 3, col: -3 }, Mv { row: -3, col: 3 }]
        );
    }

    /// Both references leave the frame on different sides, so a row holds
    /// all three masks; the sums exceed both clamp bounds.
    fn translational_rows_match_the_per_sample_blend<T: ReconSample>(
        bit_depth: splot_recon::BitDepth,
    ) {
        for w in [3usize, 4, 6, 8, 17, 24] {
            let (h, frame_w, frame_h) = (7usize, 20usize, 5usize);
            let pred0: Vec<i32> = (0..w * h).map(|i| (i as i32 * 7 % 900) * 48).collect();
            let pred1: Vec<i32> = (0..w * h)
                .map(|i| (i as i32 * 11 % 800) * 48 - 900)
                .collect();
            let scalings = [(-8, -24), (16, 40)].map(|(mv_row, mv_col)| {
                let (frame_w, frame_h) = (frame_w as i32, frame_h as i32);
                derive_plane_scaling(
                    0, 0, mv_row, mv_col, 0, 0, frame_w, frame_h, frame_w, frame_h,
                )
            });
            let blend = ImplicitMaskBlend::new(bit_depth, frame_w, frame_h);
            let mut output = vec![T::default(); w * h];
            blend_nonuniform_implicit_mask(
                &pred0,
                &pred1,
                (bit_depth, w),
                scalings,
                (frame_w, frame_h),
                &mut output,
                w,
            )
            .unwrap();
            for (index, sample) in output.iter().enumerate() {
                let (row, col) = ((index / w) as i32, (index % w) as i32);
                let starts = scalings.map(|s| ((s.start_x >> 10) + col, (s.start_y >> 10) + row));
                let want: T = blend.sample(pred0[index], pred1[index], starts).unwrap();
                assert_eq!(sample.to_u16(), want.to_u16(), "{bit_depth:?} {index}");
            }
            let samples: Vec<u16> = output.iter().map(|sample| sample.to_u16()).collect();
            assert!(w < 24 || samples.contains(&0) && samples.contains(&bit_depth.max_sample()));
        }
    }

    #[test]
    fn translational_implicit_mask_rows_match_the_per_sample_blend() {
        translational_rows_match_the_per_sample_blend::<u8>(splot_recon::BitDepth::Eight);
        translational_rows_match_the_per_sample_blend::<u16>(splot_recon::BitDepth::Ten);
    }

    #[test]
    fn reference_areas_keep_nominal_luma_and_chroma_subblock_sizes() {
        assert_eq!(subblock_reference_area_size(PlaneId::Y, 8, 8), Some((8, 8)));
        assert_eq!(subblock_reference_area_size(PlaneId::Y, 4, 4), None);
        assert_eq!(subblock_reference_area_size(PlaneId::U, 4, 4), Some((4, 4)));
    }
}
