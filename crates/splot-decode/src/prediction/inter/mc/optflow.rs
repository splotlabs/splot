// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{Simd, num::SimdUint};

use splot_recon::math::round2_signed_i32;
use splot_recon::{
    BilinearOverlapCarry, OptflowScratch, derive_optflow_mv_delta_8x8_strided_into,
    derive_optflow_mv_deltas_into,
};

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
        _scratch: &mut [i16],
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
        scratch: &mut [i16],
        output: &mut [Self],
        output_stride: usize,
    ) -> splot_recon::Result<bool> {
        subpel_predict_block_compound_average_fast_validated_strided_into(
            reference0,
            params0,
            reference1,
            params1,
            cwp_weight,
            scratch,
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
        scratch: &mut [i16],
        output: &mut [Self],
        output_stride: usize,
    ) -> splot_recon::Result<bool> {
        subpel_predict_block_compound_average_fast_validated_strided_into(
            reference0,
            params0,
            reference1,
            params1,
            cwp_weight,
            scratch,
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

#[derive(Debug)]
enum MotionCells {
    Inline(MotionCell),
    Heap(Vec<MotionCell>),
}

#[derive(Debug)]
enum RefinemvCandidates {
    None,
    Uniform {
        candidates: [Mv; 2],
        unit_size: usize,
    },
    PerCell {
        candidates: Vec<[Mv; 2]>,
        unit_size: usize,
    },
}

/// Largest motion-grid subblock (refine-MV unit): 16x16 samples.
const MAX_MOTION_GRID_SUBBLOCK_SAMPLES: usize = 256;
/// Horizontal-pass rows for an unscaled 16-sample-tall subblock: 16 + 7 taps.
const MAX_MOTION_GRID_SUBPEL_INTERMEDIATE: usize = 16 * (16 + 7);

std::thread_local! {
    static OPTFLOW_SCRATCH: std::cell::Cell<Option<OptflowScratch>> =
        const { std::cell::Cell::new(None) };
    static OPTFLOW_MOTION_CELLS: std::cell::RefCell<[Option<Vec<MotionCell>>; 2]> =
        const { std::cell::RefCell::new([None, None]) };
}

pub(super) fn take_motion_cells(len: usize, value: MotionCell) -> Vec<MotionCell> {
    OPTFLOW_MOTION_CELLS.with(|slot| {
        let mut slots = slot.borrow_mut();
        let fitting = slots
            .iter()
            .enumerate()
            .filter_map(|(index, cells)| {
                cells
                    .as_ref()
                    .filter(|cells| cells.capacity() >= len)
                    .map(|cells| (index, cells.capacity()))
            })
            .min_by_key(|&(_, capacity)| capacity)
            .map(|(index, _)| index);
        let fallback = slots
            .iter()
            .enumerate()
            .filter_map(|(index, cells)| cells.as_ref().map(|cells| (index, cells.capacity())))
            .max_by_key(|&(_, capacity)| capacity)
            .map(|(index, _)| index);
        let mut cells = fitting
            .or(fallback)
            .and_then(|index| slots[index].take())
            .unwrap_or_default();
        cells.resize(len, value);
        cells
    })
}

fn recycle_motion_cells(mut cells: Vec<MotionCell>) {
    cells.clear();
    OPTFLOW_MOTION_CELLS.with(|slot| {
        let mut slots = slot.borrow_mut();
        if let Some(empty) = slots.iter_mut().find(|slot| slot.is_none()) {
            *empty = Some(cells);
            return;
        }
        let Some((smallest, capacity)) = slots
            .iter()
            .enumerate()
            .filter_map(|(index, cells)| cells.as_ref().map(|cells| (index, cells.capacity())))
            .min_by_key(|&(_, capacity)| capacity)
        else {
            return;
        };
        if cells.capacity() > capacity {
            slots[smallest] = Some(cells);
        }
    });
}

pub(super) fn swap_thread_locals(
    scratch: &mut Option<OptflowScratch>,
    motion_cells: &mut [Option<Vec<MotionCell>>; 2],
) {
    OPTFLOW_SCRATCH.with(|slot| {
        let mut active = slot.take();
        std::mem::swap(&mut active, scratch);
        slot.set(active);
    });
    OPTFLOW_MOTION_CELLS.with(|slot| {
        std::mem::swap(&mut *slot.borrow_mut(), motion_cells);
    });
}

impl MotionCells {
    fn from_vec(cells: Vec<MotionCell>) -> Self {
        if let [cell] = cells.as_slice() {
            let cell = *cell;
            recycle_motion_cells(cells);
            return Self::Inline(cell);
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

impl Drop for MotionCells {
    fn drop(&mut self) {
        if let Self::Heap(cells) = self {
            recycle_motion_cells(core::mem::take(cells));
        }
    }
}

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
    pub(crate) fn take_candidates(&mut self) -> Vec<[Mv; 2]> {
        match &mut self.refinemv_candidates {
            RefinemvCandidates::PerCell { candidates, .. } => core::mem::take(candidates),
            RefinemvCandidates::None | RefinemvCandidates::Uniform { .. } => Vec::new(),
        }
    }

    pub(super) fn from_single_refinemv(candidates: [Mv; 2], cell: MotionCell) -> Self {
        Self {
            unit_size: 16,
            columns: 1,
            cells: MotionCells::Inline(cell),
            fullpel_runs: false,
            refinemv_candidates: RefinemvCandidates::Uniform {
                candidates,
                unit_size: 16,
            },
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
            fullpel_runs: false,
            refinemv_candidates: RefinemvCandidates::Uniform {
                candidates,
                unit_size: 16,
            },
        }
    }

    pub(super) const fn unit_size(&self) -> usize {
        self.unit_size
    }

    pub(super) fn at_luma_offset(&self, x: usize, y: usize) -> splot_recon::Result<[[i32; 2]; 2]> {
        Ok(self.cell_at_luma_offset(x, y)?.mvs)
    }

    fn uniform_refinemv_candidates(&self) -> Option<[Mv; 2]> {
        match &self.refinemv_candidates {
            RefinemvCandidates::Uniform { candidates, .. } => Some(*candidates),
            RefinemvCandidates::None | RefinemvCandidates::PerCell { .. } => None,
        }
    }

    /// The refine-MV candidates as a slice that cell `index` reads at
    /// `index * step` (a uniform list has step 0), and their unit size.
    fn refinemv_candidate_slice(&self) -> (&[[Mv; 2]], usize, usize) {
        match &self.refinemv_candidates {
            RefinemvCandidates::None => (&[], 0, 0),
            RefinemvCandidates::Uniform {
                candidates,
                unit_size,
            } => (core::slice::from_ref(candidates), 0, *unit_size),
            RefinemvCandidates::PerCell {
                candidates,
                unit_size,
            } => (candidates, 1, *unit_size),
        }
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
        Ok(stored_mvs(cell))
    }

    pub(in crate::prediction::inter) fn stored_mvs_at_index(
        &self,
        index: usize,
    ) -> splot_recon::Result<[Mv; 2]> {
        self.cell_at_index(index).map(stored_mvs)
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
        self.cell_at_index(index)
    }

    fn cell_at_index(&self, index: usize) -> splot_recon::Result<MotionCell> {
        self.cells
            .as_slice()
            .get(index)
            .copied()
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound motion-grid lookup",
            })
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

fn normalized_sad(pred0: &[u16], pred1: &[u16], bit_depth: splot_recon::BitDepth) -> u32 {
    let sad = pred0
        .iter()
        .zip(pred1)
        .map(|(&a, &b)| u32::from(a.abs_diff(b)))
        .sum::<u32>();
    sad >> bit_depth.bits().saturating_sub(8)
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
        let mut sad = Simd::<u32, 8>::splat(0);
        for row in 0..8 {
            let offset = row * stride;
            let left = Simd::<u16, 8>::from_slice(&pred0[start0 + offset..]);
            let right = Simd::<u16, 8>::from_slice(&pred1[start1 + offset..]);
            sad += left.abs_diff(right).cast();
        }
        sad.reduce_sum() >> bit_depth.bits().saturating_sub(8) < threshold
    }) {
        return Ok(MotionCell::from_refinemv(base_mvs));
    }
    let delta = derive_optflow_mv_delta_8x8_strided_into(
        pred0, start0, pred1, start1, stride, bit_depth, distances,
    )?;
    Ok(MotionCell::from_optflow(base_mvs, delta))
}

pub(super) fn compound_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
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
    let mut cells: Option<Vec<MotionCell>> = None;
    let mut written_cells = 0usize;
    let base_unit = refinemv
        .as_ref()
        .map_or(block.rect.luma_w.max(block.rect.luma_h), |grid| {
            grid.unit_size()
        });
    let [mut luma0, mut luma1] = LumaReference::pair(sink.info(), &block, offset);
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
                .and_then(CompoundMotionGrid::uniform_refinemv_candidates);
            let mut prediction_rect = block.rect;
            prediction_rect.luma_x += region_x;
            prediction_rect.luma_y += region_y;
            prediction_rect.luma_w = round_up(region_w)?;
            prediction_rect.luma_h = round_up(region_h)?;
            let refined = super::with_initial_luma_predictions(
                prediction_rect.luma_w,
                prediction_rect.luma_h,
                |pred0, pred1| {
                    initial_luma_prediction::<_, 0>(
                        &mut luma0,
                        prediction_rect,
                        base_mvs[0],
                        InterpolationFilter::Bilinear,
                        candidates.map(|mvs| (mvs[0], region_w, region_h)),
                        None,
                        pred0,
                    )?;
                    initial_luma_prediction::<_, 0>(
                        &mut luma1,
                        prediction_rect,
                        base_mvs[1],
                        InterpolationFilter::Bilinear,
                        candidates.map(|mvs| (mvs[1], region_w, region_h)),
                        None,
                        pred1,
                    )?;
                    if block.optflow_sad_threshold.is_some_and(|threshold| {
                        normalized_sad(pred0, pred1, sink.info().bit_depth()) < threshold
                    }) {
                        return Ok(false);
                    }
                    OPTFLOW_SCRATCH.with(|slot| {
                        let mut scratch = slot.take().unwrap_or_default();
                        let result = (|| {
                            let deltas = derive_optflow_mv_deltas_into(
                                pred0,
                                pred1,
                                prediction_rect.luma_w,
                                prediction_rect.luma_h,
                                unit_size,
                                sink.info().bit_depth(),
                                distances,
                                &mut scratch,
                            )?;
                            let cells = cells.get_or_insert_with(|| {
                                take_motion_cells(
                                    cell_count,
                                    MotionCell::uninitialized([block.mv0, block.mv1]),
                                )
                            });
                            let local_columns = prediction_rect.luma_w / unit_size;
                            for (index, delta) in deltas.iter().copied().enumerate() {
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
                                let cell = cells.get_mut(global_index).ok_or(
                                    ReconError::ArithmeticOverflow {
                                        context: "optical-flow motion-grid write",
                                    },
                                )?;
                                *cell = MotionCell::from_optflow(base_mvs, delta);
                                written_cells = written_cells.checked_add(1).ok_or(
                                    ReconError::ArithmeticOverflow {
                                        context: "optical-flow motion-grid completeness",
                                    },
                                )?;
                            }
                            Ok(true)
                        })();
                        slot.set(Some(scratch));
                        result
                    })
                },
            )?;
            if !refined {
                if let Some(cells) = cells.take() {
                    recycle_motion_cells(cells);
                }
                return Ok(refinemv);
            }
        }
    }
    if written_cells != cell_count
        || cells
            .as_deref()
            .is_some_and(|cells| cells.iter().any(|cell| !cell.is_initialized()))
    {
        return Err(ReconError::ArithmeticOverflow {
            context: "optical-flow motion-grid completeness",
        }
        .into());
    }
    let cells = cells.unwrap_or_default();
    let refinemv_candidates = refinemv
        .as_ref()
        .and_then(CompoundMotionGrid::uniform_refinemv_candidates);
    Ok(Some(CompoundMotionGrid {
        unit_size,
        columns,
        cells: MotionCells::from_vec(cells),
        fullpel_runs: false,
        refinemv_candidates: refinemv_candidates.map_or(RefinemvCandidates::None, |candidates| {
            RefinemvCandidates::Uniform {
                candidates,
                unit_size: 16,
            }
        }),
    }))
}

/// The motion cell of one TIP unit that the refine-MV optical-flow search did
/// not take. An 8x8 unit is a single optical-flow unit, so its two initial
/// predictions and § 7.13.3.9 delta stay on the stack instead of going through
/// a motion grid.
pub(super) fn tip_unit_motion_cell<'a, T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    unit: CompoundMcBlock<'a, T>,
    unit_size: usize,
    offset: ByteOffset,
    luma: &mut [LumaReference<'a, T>; 2],
) -> Result<MotionCell> {
    let mvs = [unit.mv0, unit.mv1];
    let refinemv = unit
        .use_refinemv
        .then(|| super::refinemv::compound_default_refinemv_motion_grid(sink, unit, offset))
        .transpose()?;
    let refined_cell = |refinemv: Option<CompoundMotionGrid>| {
        refinemv
            .map(|motion| motion.cell_at_luma_offset(0, 0))
            .transpose()
            .map(|cell| cell.unwrap_or_else(|| MotionCell::from_refinemv(mvs)))
    };
    let (Some(distances), 8, 8, 8) = (
        unit.optflow_distances,
        unit_size,
        unit.rect.luma_w,
        unit.rect.luma_h,
    ) else {
        let motion = compound_motion_grid(sink, unit, Some(unit_size), refinemv, offset)?;
        return Ok(refined_cell(motion)?);
    };
    let base_mvs = refinemv
        .as_ref()
        .map_or(Ok(mvs), |grid| grid.stored_mvs_at_luma_offset(0, 0))?;
    let candidates = refinemv
        .as_ref()
        .and_then(CompoundMotionGrid::uniform_refinemv_candidates);
    let bit_depth = sink.info().bit_depth();
    let mut predictions = [[0u16; 64]; 2];
    for (reference, (luma, prediction)) in luma.iter_mut().zip(&mut predictions).enumerate() {
        initial_luma_prediction::<_, 0>(
            luma,
            unit.rect,
            base_mvs[reference],
            InterpolationFilter::Bilinear,
            candidates.map(|mvs| (mvs[reference], 8, 8)),
            None,
            prediction,
        )?;
    }
    let [pred0, pred1] = &predictions;
    if unit
        .optflow_sad_threshold
        .is_some_and(|threshold| normalized_sad(pred0, pred1, bit_depth) < threshold)
    {
        return Ok(refined_cell(refinemv)?);
    }
    let delta =
        derive_optflow_mv_delta_8x8_strided_into(pred0, 0, pred1, 0, 8, bit_depth, distances)?;
    Ok(MotionCell::from_optflow(base_mvs, delta))
}

/// Whether some row holds two adjacent equal cells whose motion is full-pel
/// under either subsampling.
fn fullpel_run_pairs(cells: &[MotionCell], columns: usize) -> bool {
    cells.chunks(columns.max(1)).any(|row| {
        row.windows(2).any(|pair| {
            pair[0] == pair[1]
                && pair[0]
                    .mvs
                    .as_flattened()
                    .iter()
                    .all(|&mv| fullpel_phase(mv, 0) == 0 || fullpel_phase(mv, 1) == 0)
        })
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn tip_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    unit_size: usize,
    columns: usize,
    unit_count: usize,
    unit_at: impl Fn(usize) -> (McBlockRect, [Mv; 2]) + Sync,
    offset: ByteOffset,
    mut refinemv_candidates: Vec<[Mv; 2]>,
) -> Result<CompoundMotionGrid> {
    refinemv_candidates.clear();
    refinemv_candidates
        .try_reserve_exact(unit_count)
        .map_err(|_| ReconError::ArithmeticOverflow {
            context: "TIP refine-MV candidate list",
        })?;
    if unit_count == 0 || columns == 0 {
        return Err(ReconError::ZeroDimension {
            field: "TIP compound motion grid",
        }
        .into());
    }
    let mut fullpel = false;
    refinemv_candidates.extend((0..unit_count).map(|index| {
        let mvs = unit_at(index).1;
        fullpel |= super::refinemv::fullpel_candidates(mvs);
        mvs
    }));
    let mut cells = take_motion_cells(
        unit_count,
        MotionCell::uninitialized([block.mv0, block.mv1]),
    );
    let fast = fullpel
        && super::refinemv::tip_fullpel_cells(
            sink,
            &block,
            &unit_at,
            &refinemv_candidates,
            (unit_size, offset),
            0,
            &mut cells,
        )?;
    let mut initial_predictions = [[0u16; super::refinemv::TIP_PREDICTION_AREA]; 2];
    let mut luma = LumaReference::pair(sink.info(), &block, offset);
    let mut carries: [BilinearOverlapCarry<'_, T>; 2] = Default::default();
    for index in 0..unit_count {
        let (rect, mvs) = unit_at(index);
        if fast && cells.get(index).is_some_and(MotionCell::is_initialized) {
            continue;
        }
        let mut unit = block;
        unit.rect = rect;
        unit.mv0 = mvs[0];
        unit.mv1 = mvs[1];
        unit.has_chroma = false;
        unit.sub8x8_chroma = false;
        let refined = super::refinemv::tip_refinemv_optflow_motion_cell(
            unit,
            &mut initial_predictions,
            &mut luma,
            &mut carries,
        )?;
        if let Some(cell) = refined {
            let destination = cells.get_mut(index).ok_or(ReconError::ArithmeticOverflow {
                context: "TIP compound motion-grid write",
            })?;
            *destination = cell;
            continue;
        }
        let cell = tip_unit_motion_cell(sink, unit, unit_size, offset, &mut luma)?;
        let destination = cells.get_mut(index).ok_or(ReconError::ArithmeticOverflow {
            context: "TIP compound motion-grid write",
        })?;
        *destination = cell;
    }
    let fullpel_runs = fullpel && fullpel_run_pairs(&cells, columns);
    Ok(CompoundMotionGrid {
        unit_size,
        columns,
        cells: MotionCells::from_vec(cells),
        fullpel_runs,
        refinemv_candidates: RefinemvCandidates::PerCell {
            candidates: refinemv_candidates,
            unit_size,
        },
    })
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
        let (reference, frame) = (samples.info().coded_luma_size(), info.coded_luma_size());
        let scaling = derive_plane_scaling(
            0,
            0,
            0,
            0,
            0,
            0,
            reference.width() as i32,
            reference.height() as i32,
            frame.width() as i32,
            frame.height() as i32,
        )
        .with_reference_storage(samples.info().storage_luma_size(), 0, 0);
        Self {
            samples,
            scaling,
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
        let (view, _, _) = self.samples.plane_view(PlaneId::Y, last_row, self.offset)?;
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
        super::refinemv::reference_area_bounds(
            rect.luma_x as i32,
            rect.luma_y as i32,
            width,
            height,
            candidate,
            0,
            0,
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
    subpel_predict_block_strided_into(&view, &params, output, rect.luma_w).map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "per-cell grid hot path")]
#[inline(always)]
fn compound_optflow_subpel_params<T: ReconSample>(
    bit_depth: splot_recon::BitDepth,
    interp: InterpolationFilter,
    subblock_area: Option<(usize, usize)>,
    sub_x: u32,
    sub_y: u32,
    (refine_candidates, refine_step, refine_unit_size): (&[[Mv; 2]], usize, usize),
    prediction: &CompoundSubpelPlane<'_, T>,
    cell: MotionCell,
    scalings: [PlaneScaling; 2],
    cell_index: usize,
    row: usize,
    col: usize,
    width: usize,
    height: usize,
) -> [SubpelPredictParams; 2] {
    let bounds = if let Some(mvs) = refine_candidates.get(cell_index * refine_step) {
        let refine_unit_w = refine_unit_size >> sub_x;
        let refine_unit_h = refine_unit_size >> sub_y;
        let refine_col = col & !(refine_unit_w - 1);
        let refine_row = row & !(refine_unit_h - 1);
        let refine_w = refine_unit_w.min(prediction.block_w - refine_col);
        let refine_h = refine_unit_h.min(prediction.block_h - refine_row);
        let bounds = |reference: usize| {
            Some(super::refinemv::reference_area_bounds(
                (prediction.plane_x + refine_col) as i32,
                (prediction.plane_y + refine_row) as i32,
                refine_w,
                refine_h,
                mvs[reference],
                sub_x,
                sub_y,
                &prediction.scalings[reference],
            ))
        };
        [bounds(0), bounds(1)]
    } else if let Some((area_width, area_height)) = subblock_area {
        let bounds = |reference: usize| {
            Some(super::refinemv::reference_area_bounds(
                (prediction.plane_x + col) as i32,
                (prediction.plane_y + row) as i32,
                area_width,
                area_height,
                cell.base_mvs[reference],
                sub_x,
                sub_y,
                &prediction.scalings[reference],
            ))
        };
        [bounds(0), bounds(1)]
    } else {
        [None; 2]
    };
    // Explicit pairs: `array::from_fn` stays an out-of-line call per cell.
    let params = |reference: usize| {
        let scaling = scalings[reference];
        SubpelPredictParams {
            interp,
            w: width,
            h: height,
            start_x: scaling.start_x,
            start_y: scaling.start_y,
            step_x: scaling.step_x,
            step_y: scaling.step_y,
            first_x: bounds[reference].map_or(scaling.first_x, |bounds| bounds.first_x),
            first_y: bounds[reference].map_or(scaling.first_y, |bounds| bounds.first_y),
            last_x: bounds[reference].map_or(scaling.last_x, |bounds| bounds.last_x),
            last_y: bounds[reference].map_or(scaling.last_y, |bounds| bounds.last_y),
            bit_depth,
        }
    };
    [params(0), params(1)]
}

#[allow(clippy::too_many_arguments)]
pub(super) fn predict_uniform_motion_compound_average_into<
    T: ReconSample,
    O: CompoundAverageOutput + Send,
>(
    info: DecodedFrameInfo,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
    cwp_weight: i16,
    offset: ByteOffset,
    output: &mut [O],
    output_stride: usize,
) -> Result<bool> {
    let [cell] = motion.cells.as_slice() else {
        return Ok(false);
    };
    let prediction = super::compound_subpel_plane(info, block, plane, sub_x, sub_y, offset)?;
    let subblock_w = (motion.unit_size >> sub_x).max(4);
    let subblock_h = (motion.unit_size >> sub_y).max(4);
    if prediction.block_w > subblock_w || prediction.block_h > subblock_h {
        return Ok(false);
    }
    let storage_luma_size = info.storage_luma_size();
    let frame_w = storage_luma_size.width().div_ceil(1 << sub_x);
    let frame_h = storage_luma_size.height().div_ceil(1 << sub_y);
    let Some(scalings) = super::compound_uniform_scalings(
        Some(motion),
        prediction.plane_x,
        prediction.plane_y,
        prediction.scalings,
        sub_x,
        sub_y,
    ) else {
        return Ok(false);
    };
    if !super::compound_average_weights_are_uniform(
        implicit_mask,
        cwp_weight,
        prediction.block_w,
        prediction.block_h,
        prediction.scalings,
        Some(scalings),
        (frame_w, frame_h),
    ) {
        return Ok(false);
    }
    let params = compound_optflow_subpel_params(
        info.bit_depth(),
        block.interp,
        subblock_reference_area_size(plane, subblock_w, subblock_h),
        sub_x,
        sub_y,
        motion.refinemv_candidate_slice(),
        &prediction,
        *cell,
        scalings,
        0,
        0,
        0,
        prediction.block_w,
        prediction.block_h,
    );
    let mut pred0_scratch = [0i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES];
    let mut intermediate_scratch = [0i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE];
    super::predict_compound_average_into(
        &prediction,
        &params,
        cwp_weight,
        Some(&mut pred0_scratch),
        Some(&mut intermediate_scratch),
        output,
        output_stride,
    )?;
    Ok(true)
}

/// Predicts a multi-cell grid plane and returns whether it did. With
/// `chroma_v` while `plane` is U, the V plane goes into that packed
/// `block_w`-stride output too, from the same per-cell parameters: U and V share
/// the plane geometry, scalings and motion, and only the reference views differ.
#[allow(clippy::too_many_arguments)]
pub(super) fn predict_motion_grid_compound_average_into<
    T: ReconSample,
    O: CompoundAverageOutput + Send,
>(
    info: DecodedFrameInfo,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
    cwp_weight: i16,
    offset: ByteOffset,
    output: &mut [O],
    output_stride: usize,
    chroma_v: Option<&mut [O]>,
) -> Result<bool> {
    if motion.cells.as_slice().len() == 1 {
        return Ok(false);
    }
    let prediction = super::compound_subpel_plane(info, block, plane, sub_x, sub_y, offset)?;
    let mut chroma_v = chroma_v;
    let mut second = None;
    if let Some(v_output) = chroma_v.take_if(|_| plane == PlaneId::U) {
        if v_output.len() >= prediction.block_w * prediction.block_h {
            let views = super::compound_plane_views(
                block,
                PlaneId::V,
                prediction.scalings,
                prediction.block_h,
                offset,
            )?;
            let v = CompoundSubpelPlane {
                views,
                plane_x: prediction.plane_x,
                plane_y: prediction.plane_y,
                block_w: prediction.block_w,
                block_h: prediction.block_h,
                scalings: prediction.scalings,
            };
            second = Some((v, v_output));
        } else {
            chroma_v = Some(v_output);
        }
    }
    let sample_count = (prediction.block_h.saturating_sub(1))
        .checked_mul(output_stride)
        .and_then(|rows| rows.checked_add(prediction.block_w))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "TIP batched compound output sample count",
        })?;
    if output_stride < prediction.block_w || output.len() < sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: output.len(),
        }
        .into());
    }
    let storage_luma_size = info.storage_luma_size();
    let frame_w = storage_luma_size.width().div_ceil(1 << sub_x);
    let frame_h = storage_luma_size.height().div_ceil(1 << sub_y);
    let subblock_w = (motion.unit_size >> sub_x).max(4);
    let subblock_h = (motion.unit_size >> sub_y).max(4);
    let subblock_area = subblock_reference_area_size(plane, subblock_w, subblock_h);
    let bit_depth = info.bit_depth();
    let uniform_everywhere = !implicit_mask
        || cwp_weight != CWP_EQUAL
        || prediction.scalings.into_iter().any(PlaneScaling::is_scaled);
    let cells = motion.cells.as_slice();
    let refine = motion.refinemv_candidate_slice();
    let merge_runs = motion.fullpel_runs
        && !prediction.scalings.into_iter().any(PlaneScaling::is_scaled)
        && refine.1 == 1
        && refine.2 >> sub_x == subblock_w;
    let grid_block = GridBlock {
        prediction: &prediction,
        motion,
        bit_depth,
        frame: (frame_w, frame_h),
        sub: (sub_x, sub_y),
        cwp_weight,
    };
    let fullpel_runs = |cell_row: usize,
                        [row, height]: [usize; 2],
                        intermediate_scratch: &mut [i16],
                        output: &mut [O],
                        output_stride: usize,
                        second: Option<SecondPlane<'_, '_, T, O>>|
     -> Result<[u64; 2]> {
        if !merge_runs {
            return Ok([0; 2]);
        }
        predict_fullpel_runs(
            &prediction,
            cells,
            refine,
            cell_row * motion.columns,
            [row, subblock_w, height],
            (sub_x, sub_y),
            (bit_depth, block.interp, subblock_area),
            (
                uniform_everywhere,
                implicit_mask,
                cwp_weight,
                (frame_w, frame_h),
            ),
            intermediate_scratch,
            output,
            output_stride,
            second,
        )
    };
    macro_rules! cell_inputs {
        ($cell_row:expr, $cell_col:expr, [$col:expr, $row:expr, $height:expr]) => {{
            let (col, row, height) = ($col, $row, $height);
            let width = subblock_w.min(prediction.block_w - col);
            let cell_index = $cell_row * motion.columns + $cell_col;
            let cell = *cells
                .get(cell_index)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "compound motion-grid lookup",
                })?;
            let scalings = core::array::from_fn(|reference| {
                prediction.scalings[reference].with_prescaled_mv(
                    (prediction.plane_x + col) as i32,
                    (prediction.plane_y + row) as i32,
                    cell.mvs[reference][0],
                    cell.mvs[reference][1],
                    sub_x,
                    sub_y,
                )
            });
            let uniform = uniform_everywhere
                || super::compound_average_weights_are_uniform(
                    implicit_mask,
                    cwp_weight,
                    width,
                    height,
                    prediction.scalings,
                    Some(scalings),
                    (frame_w, frame_h),
                );
            let params = compound_optflow_subpel_params(
                bit_depth,
                block.interp,
                subblock_area,
                sub_x,
                sub_y,
                refine,
                &prediction,
                cell,
                scalings,
                cell_index,
                row,
                col,
                width,
                height,
            );
            (params, uniform, scalings, [col, row, width, height])
        }};
    }
    let process_row = |cell_row: usize,
                       row: usize,
                       output: &mut [O],
                       pred_scratch: &mut [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
                       intermediate_scratch: &mut [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE]|
     -> Result<()> {
        let height = subblock_h.min(prediction.block_h - row);
        let [mut merged, _] = fullpel_runs(
            cell_row,
            [row, height],
            intermediate_scratch,
            output,
            output_stride,
            None,
        )?;
        for (cell_col, col) in (0..prediction.block_w).step_by(subblock_w).enumerate() {
            let skip = merged & 1 != 0;
            merged >>= 1;
            if skip {
                continue;
            }
            let (params, uniform, scalings, rect) =
                cell_inputs!(cell_row, cell_col, [col, row, height]);
            predict_grid_cell(
                &prediction.views,
                (&params, uniform, &scalings, rect),
                &grid_block,
                pred_scratch,
                intermediate_scratch,
                &mut output[col..],
                output_stride,
            )?;
        }
        Ok(())
    };
    let mut pred_scratch = [[0i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2];
    let mut intermediate_scratch = [0i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE];
    let Some((second, second_output)) = second else {
        for (cell_row, row) in (0..prediction.block_h).step_by(subblock_h).enumerate() {
            process_row(
                cell_row,
                row,
                &mut output[row * output_stride..],
                &mut pred_scratch,
                &mut intermediate_scratch,
            )?;
        }
        return predict_chroma_v_separately(
            info,
            block,
            (sub_x, sub_y),
            motion,
            (implicit_mask, cwp_weight),
            offset,
            chroma_v,
        );
    };
    for (cell_row, row) in (0..prediction.block_h).step_by(subblock_h).enumerate() {
        let height = subblock_h.min(prediction.block_h - row);
        let output = &mut output[row * output_stride..];
        let second_output = &mut second_output[row * second.block_w..];
        let [mut merged, mut second_merged] = fullpel_runs(
            cell_row,
            [row, height],
            &mut intermediate_scratch,
            output,
            output_stride,
            Some((&second.views, &mut *second_output, second.block_w)),
        )?;
        for (cell_col, col) in (0..prediction.block_w).step_by(subblock_w).enumerate() {
            let skip = [merged & 1 != 0, second_merged & 1 != 0];
            merged >>= 1;
            second_merged >>= 1;
            if skip == [true; 2] {
                continue;
            }
            let (params, uniform, scalings, rect) =
                cell_inputs!(cell_row, cell_col, [col, row, height]);
            let cell = (&params, uniform, &scalings, rect);
            if !skip[0] {
                predict_grid_cell(
                    &prediction.views,
                    cell,
                    &grid_block,
                    &mut pred_scratch,
                    &mut intermediate_scratch,
                    &mut output[col..],
                    output_stride,
                )?;
            }
            if !skip[1] {
                predict_grid_cell(
                    &second.views,
                    cell,
                    &grid_block,
                    &mut pred_scratch,
                    &mut intermediate_scratch,
                    &mut second_output[col..],
                    second.block_w,
                )?;
            }
        }
    }
    predict_chroma_v_separately(
        info,
        block,
        (sub_x, sub_y),
        motion,
        (implicit_mask, cwp_weight),
        offset,
        chroma_v,
    )
}

/// Predicts the V plane into a packed `chroma_v` output that the shared U pass
/// declined, so that a grid prediction always fills `chroma_v` when given one.
fn predict_chroma_v_separately<T: ReconSample, O: CompoundAverageOutput + Send>(
    info: DecodedFrameInfo,
    block: CompoundMcBlock<'_, T>,
    (sub_x, sub_y): (u32, u32),
    motion: &CompoundMotionGrid,
    (implicit_mask, cwp_weight): (bool, i16),
    offset: ByteOffset,
    chroma_v: Option<&mut [O]>,
) -> Result<bool> {
    if let Some(v_output) = chroma_v {
        let (_, _, block_w, _) = block.rect.plane_rect(PlaneId::V, sub_x, sub_y);
        predict_motion_grid_compound_average_into(
            info,
            block,
            PlaneId::V,
            sub_x,
            sub_y,
            motion,
            implicit_mask,
            cwp_weight,
            offset,
            v_output,
            block_w,
            None,
        )?;
    }
    Ok(true)
}

/// One grid cell's derived parameters, uniform-weight flag, scalings and
/// `[col, row, width, height]`.
type GridCell<'a> = (
    &'a [SubpelPredictParams; 2],
    bool,
    &'a [PlaneScaling; 2],
    [usize; 4],
);

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
    (params, uniform, scalings, [col, row, width, height]): GridCell<'_>,
    block: &GridBlock<'_, '_, T>,
    pred_scratch: &mut [[i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES]; 2],
    intermediate_scratch: &mut [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE],
    output: &mut [O],
    output_stride: usize,
) -> Result<()> {
    let prediction = block.prediction;
    if !uniform {
        let [pred0, pred1] = &mut *pred_scratch;
        let preds = views.iter().zip([&mut *pred0, &mut *pred1]);
        for ((view, pred), params) in preds.zip(params) {
            subpel_predict_block_compound_intermediate_into(
                view,
                params,
                Some(&mut *intermediate_scratch),
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
    if O::predict_fast(
        &views[0],
        &params[0],
        &views[1],
        &params[1],
        block.cwp_weight,
        intermediate_scratch,
        output,
        output_stride,
    )? {
        return Ok(());
    }
    let subplane = CompoundSubpelPlane {
        views: *views,
        plane_x: prediction.plane_x + col,
        plane_y: prediction.plane_y + row,
        block_w: width,
        block_h: height,
        scalings: *scalings,
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

/// Predicts each run of full-width, full-pel cells in one grid row that share
/// their motion and clipping-bound source as one block of at most 64 samples,
/// and returns a mask of the cells it predicted. With `second`, the same runs
/// are predicted from those views into that output too (the V plane of a
/// shared U pass), and the second mask holds its cells.
///
/// Each cell's bounds are the run's first cell's unclamped bounds shifted by
/// the cell's offset, so when the first cell reads inside its bounds and the
/// whole run reads inside the plane, no cell clamps a column. Rows clamp
/// identically.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
fn predict_fullpel_runs<T: ReconSample, O: CompoundAverageOutput>(
    prediction: &CompoundSubpelPlane<'_, T>,
    cells: &[MotionCell],
    refine: (&[[Mv; 2]], usize, usize),
    row_index: usize,
    [row, subblock_w, height]: [usize; 3],
    (sub_x, sub_y): (u32, u32),
    (bit_depth, interp, subblock_area): (
        splot_recon::BitDepth,
        InterpolationFilter,
        Option<(usize, usize)>,
    ),
    (uniform_everywhere, implicit_mask, cwp_weight, frame): (bool, bool, i16, (usize, usize)),
    intermediate_scratch: &mut [i16],
    output: &mut [O],
    output_stride: usize,
    mut second: Option<SecondPlane<'_, '_, T, O>>,
) -> Result<[u64; 2]> {
    let columns = (prediction.block_w / subblock_w).min(64);
    let row_cells = cells
        .get(row_index..row_index + columns)
        .unwrap_or_default();
    let candidate = |column: usize| refine.0.get((row_index + column) * refine.1);
    let mut merged = [0u64; 2];
    let mut start = 0;
    while start + 1 < row_cells.len() {
        let cell = row_cells[start];
        let mut run = 1;
        if row_cells[start + 1] == cell && fullpel_motion(cell.mvs, sub_x, sub_y) {
            while start + run < row_cells.len()
                && (run + 1) * subblock_w <= 64
                && row_cells[start + run] == cell
                && candidate(start + run) == candidate(start)
            {
                run += 1;
            }
        }
        let col = start * subblock_w;
        if run > 1
            && let Some(params) = fullpel_run_params(
                prediction,
                cell,
                refine,
                row_index + start,
                [col, row, subblock_w, run * subblock_w, height],
                (sub_x, sub_y),
                (bit_depth, interp, subblock_area),
                (uniform_everywhere, implicit_mask, cwp_weight, frame),
            )
        {
            let run_cells = ((1 << run) - 1) << start;
            if O::predict_fast(
                &prediction.views[0],
                &params[0],
                &prediction.views[1],
                &params[1],
                cwp_weight,
                intermediate_scratch,
                &mut output[col..],
                output_stride,
            )? {
                merged[0] |= run_cells;
            }
            if let Some((views, output, output_stride)) = &mut second
                && O::predict_fast(
                    &views[0],
                    &params[0],
                    &views[1],
                    &params[1],
                    cwp_weight,
                    intermediate_scratch,
                    &mut output[col..],
                    *output_stride,
                )?
            {
                merged[1] |= run_cells;
            }
        }
        start += run;
    }
    Ok(merged)
}

/// The parameters of one full-pel run, or `None` when the run reads outside
/// its bounds or the plane, or its weights are not uniform.
#[allow(clippy::too_many_arguments)]
fn fullpel_run_params<T: ReconSample>(
    prediction: &CompoundSubpelPlane<'_, T>,
    cell: MotionCell,
    refine: (&[[Mv; 2]], usize, usize),
    cell_index: usize,
    [col, row, subblock_w, run_w, height]: [usize; 5],
    (sub_x, sub_y): (u32, u32),
    (bit_depth, interp, subblock_area): (
        splot_recon::BitDepth,
        InterpolationFilter,
        Option<(usize, usize)>,
    ),
    (uniform_everywhere, implicit_mask, cwp_weight, frame): (bool, bool, i16, (usize, usize)),
) -> Option<[SubpelPredictParams; 2]> {
    let scalings = core::array::from_fn(|reference| {
        prediction.scalings[reference].with_prescaled_mv(
            (prediction.plane_x + col) as i32,
            (prediction.plane_y + row) as i32,
            cell.mvs[reference][0],
            cell.mvs[reference][1],
            sub_x,
            sub_y,
        )
    });
    let mut params = compound_optflow_subpel_params(
        bit_depth,
        interp,
        subblock_area,
        sub_x,
        sub_y,
        refine,
        prediction,
        cell,
        scalings,
        cell_index,
        row,
        col,
        subblock_w,
        height,
    );
    let inside = params
        .iter()
        .zip(&prediction.views)
        .zip(&prediction.scalings)
        .all(|((params, view), scaling)| {
            let x = params.start_x >> 10;
            ((params.start_x | params.start_y) >> 6).trailing_zeros() >= 4
                && x >= params.first_x
                && x + subblock_w as i32 - 1 <= params.last_x
                && x + run_w as i32 - 1 <= scaling.last_x.min(view.width() as i32 - 1)
        });
    if !inside
        || !(uniform_everywhere
            || super::compound_average_weights_are_uniform(
                implicit_mask,
                cwp_weight,
                run_w,
                height,
                prediction.scalings,
                Some(scalings),
                frame,
            ))
    {
        return None;
    }
    for params in &mut params {
        params.w = run_w;
        params.last_x = (params.start_x >> 10) + run_w as i32 - 1;
    }
    Some(params)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn blend_nonuniform_implicit_mask<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    width: usize,
    height: usize,
    motion: Option<&CompoundMotionGrid>,
    plane_x: usize,
    plane_y: usize,
    scaling_templates: [PlaneScaling; 2],
    frame_w: usize,
    frame_h: usize,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
    output_stride: usize,
) -> splot_recon::Result<()> {
    if output.is_empty() {
        return Ok(());
    }
    let blend = ImplicitMaskBlend::new(bit_depth, frame_w, frame_h);
    if let Some(motion) = motion {
        return blend_implicit_mask_region(
            [pred0, pred1],
            width,
            [0, 0, width, height],
            motion,
            (plane_x, plane_y),
            scaling_templates,
            blend,
            (sub_x, sub_y),
            output,
            output_stride,
        );
    }
    let reference_starts =
        scaling_templates.map(|scaling| (scaling.start_x >> 10, scaling.start_y >> 10));
    for (row, ((output, pred0), pred1)) in output
        .chunks_mut(output_stride.max(1))
        .zip(pred0.chunks(width))
        .zip(pred1.chunks(width))
        .enumerate()
    {
        let starts = reference_starts.map(|(x, y)| (x, y + row as i32));
        blend.row(pred0, pred1, starts, &mut output[..width])?;
    }
    Ok(())
}

/// The implicit-mask weights: a reference whose sample position falls outside
/// the frame yields its whole weight to the other reference.
#[derive(Clone, Copy)]
struct ImplicitMaskBlend {
    last: (i32, i32),
    max_sample: i32,
}

impl ImplicitMaskBlend {
    fn new(bit_depth: splot_recon::BitDepth, frame_w: usize, frame_h: usize) -> Self {
        Self {
            last: (frame_w as i32 - 1, frame_h as i32 - 1),
            max_sample: i32::from(bit_depth.max_sample()),
        }
    }

    #[allow(
        clippy::inline_always,
        reason = "per-sample blend; must inline in every crate's copy of the decoder"
    )]
    #[inline(always)]
    fn sample<T: ReconSample>(
        self,
        left: i32,
        right: i32,
        starts: [(i32, i32); 2],
    ) -> splot_recon::Result<T> {
        let onscreen =
            starts.map(|(x, y)| (0..=self.last.0).contains(&x) && (0..=self.last.1).contains(&y));
        let mask = match onscreen {
            [true, false] => 2,
            [false, true] => 0,
            _ => 1,
        };
        let sample = round2_i32(
            mask * left + (2 - mask) * right,
            1 + compound_inter_post_round(),
        );
        T::try_from_u16(sample.clamp(0, self.max_sample) as u16)
    }

    /// [`Self::sample`] over one row whose first sample reads the reference
    /// positions `starts` and whose later samples step one column right.
    #[allow(
        clippy::inline_always,
        reason = "per-row blend; the loop must vectorize in every caller"
    )]
    #[inline(always)]
    fn row<T: ReconSample>(
        self,
        left: &[i32],
        right: &[i32],
        starts: [(i32, i32); 2],
        output: &mut [T],
    ) -> splot_recon::Result<()> {
        let row_onscreen = starts.map(|(_, y)| (0..=self.last.1).contains(&y));
        let samples = left
            .iter()
            .zip(right)
            .enumerate()
            .map(|(col, (&left, &right))| {
                let onscreen = |reference: usize| {
                    row_onscreen[reference]
                        && (0..=self.last.0).contains(&(starts[reference].0 + col as i32))
                };
                let mask = 1 + i32::from(onscreen(0)) - i32::from(onscreen(1));
                round2_i32(
                    mask * left + (2 - mask) * right,
                    1 + compound_inter_post_round(),
                )
            });
        super::blend::store_clamped_samples(output, self.max_sample, samples)
    }
}

/// Blends the `[x, y, width, height]` region of a motion-grid plane, whose
/// predictions and output are region-local with their own row strides, with
/// each sample's implicit mask taken from its own motion-grid cell. With
/// unscaled references a start is the sample position plus one per-cell
/// offset, so only scaled references derive it per sample.
#[allow(clippy::too_many_arguments)]
fn blend_implicit_mask_region<T: ReconSample>(
    preds: [&[i32]; 2],
    pred_stride: usize,
    [x, y, width, height]: [usize; 4],
    motion: &CompoundMotionGrid,
    (plane_x, plane_y): (usize, usize),
    scaling_templates: [PlaneScaling; 2],
    blend: ImplicitMaskBlend,
    (sub_x, sub_y): (u32, u32),
    output: &mut [T],
    output_stride: usize,
) -> splot_recon::Result<()> {
    let unit_width = (motion.unit_size >> sub_x).max(4);
    let unit_height = (motion.unit_size >> sub_y).max(4);
    let unscaled = !scaling_templates.iter().any(|scaling| scaling.is_scaled());
    for cell_y in (y..y + height).step_by(unit_height) {
        for cell_x in (x..x + width).step_by(unit_width) {
            let mvs = motion.at_luma_offset(cell_x << sub_x, cell_y << sub_y)?;
            let start_at = |reference: usize, col: usize, row: usize| {
                let scaling = scaling_templates[reference].with_prescaled_mv(
                    (plane_x + col) as i32,
                    (plane_y + row) as i32,
                    mvs[reference][0],
                    mvs[reference][1],
                    sub_x,
                    sub_y,
                );
                (scaling.start_x >> 10, scaling.start_y >> 10)
            };
            let offsets: [(i32, i32); 2] = core::array::from_fn(|reference| {
                let (start_x, start_y) = start_at(reference, cell_x, cell_y);
                (
                    start_x - (plane_x + cell_x) as i32,
                    start_y - (plane_y + cell_y) as i32,
                )
            });
            let cols = cell_x - x..(cell_x + unit_width).min(x + width) - x;
            for row in cell_y..(cell_y + unit_height).min(y + height) {
                let source = (row - y) * pred_stride;
                let destination = (row - y) * output_stride;
                if unscaled {
                    let starts = offsets.map(|(offset_x, offset_y)| {
                        (
                            (plane_x + x + cols.start) as i32 + offset_x,
                            (plane_y + row) as i32 + offset_y,
                        )
                    });
                    blend.row(
                        &preds[0][source + cols.start..source + cols.end],
                        &preds[1][source + cols.start..source + cols.end],
                        starts,
                        &mut output[destination + cols.start..destination + cols.end],
                    )?;
                    continue;
                }
                for col in cols.clone() {
                    let starts =
                        core::array::from_fn(|reference| start_at(reference, x + col, row));
                    output[destination + col] =
                        blend.sample(preds[0][source + col], preds[1][source + col], starts)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn compound_optflow_plane_prediction<T: ReconSample>(
    info: DecodedFrameInfo,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let prediction = super::compound_subpel_plane(info, block, plane, sub_x, sub_y, offset)?;
    let subblock_w = (motion.unit_size >> sub_x).max(4);
    let subblock_h = (motion.unit_size >> sub_y).max(4);
    let subblock_area = subblock_reference_area_size(plane, subblock_w, subblock_h);
    let bit_depth = info.bit_depth();
    let refine = motion.refinemv_candidate_slice();
    let [mut pred0, mut pred1] =
        super::take_compound_prediction_buffers(prediction.block_w * prediction.block_h);

    for (cell_row, row) in (0..prediction.block_h).step_by(subblock_h).enumerate() {
        for (cell_col, col) in (0..prediction.block_w).step_by(subblock_w).enumerate() {
            let width = subblock_w.min(prediction.block_w - col);
            let height = subblock_h.min(prediction.block_h - row);
            let cell_index = cell_row * motion.columns + cell_col;
            let cell = motion.cell_at_index(cell_index)?;
            let scalings = core::array::from_fn(|reference| {
                prediction.scalings[reference].with_prescaled_mv(
                    (prediction.plane_x + col) as i32,
                    (prediction.plane_y + row) as i32,
                    cell.mvs[reference][0],
                    cell.mvs[reference][1],
                    sub_x,
                    sub_y,
                )
            });
            let params = compound_optflow_subpel_params(
                bit_depth,
                block.interp,
                subblock_area,
                sub_x,
                sub_y,
                refine,
                &prediction,
                cell,
                scalings,
                cell_index,
                row,
                col,
                width,
                height,
            );
            for ((view, output), params) in prediction
                .views
                .iter()
                .zip([&mut pred0, &mut pred1])
                .zip(params)
            {
                let start = row * prediction.block_w + col;
                subpel_predict_block_compound_intermediate_into(
                    view,
                    &params,
                    None,
                    &mut output[start..],
                    prediction.block_w,
                )?;
            }
        }
    }

    Ok(CompoundPlanePrediction {
        pred0,
        pred1,
        plane_x: prediction.plane_x,
        plane_y: prediction.plane_y,
        block_w: prediction.block_w,
        block_h: prediction.block_h,
        scaling0: prediction.scalings[0],
        scaling1: prediction.scalings[1],
        recycle_buffers: true,
    })
}

#[cfg(test)]
#[path = "optflow_run_tests.rs"]
mod run_tests;

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
            fullpel_runs: false,
            refinemv_candidates: RefinemvCandidates::None,
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
            fullpel_runs: false,
            refinemv_candidates: RefinemvCandidates::None,
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
            fullpel_runs: false,
            refinemv_candidates: RefinemvCandidates::None,
        };

        assert_eq!(
            grid.temporal_mvs_at_luma_offset(0, 0).unwrap(),
            [Mv { row: 1, col: -1 }, Mv::ZERO]
        );
    }

    /// Both references leave the frame on different sides, so a row holds
    /// all three masks; the sums exceed both clamp bounds.
    fn translational_rows_match_the_per_sample_blend<T: ReconSample>(
        bit_depth: splot_recon::BitDepth,
    ) {
        let (w, h, frame_w, frame_h) = (24usize, 7usize, 20usize, 5usize);
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
            bit_depth,
            w,
            h,
            None,
            0,
            0,
            scalings,
            frame_w,
            frame_h,
            0,
            0,
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
        assert!(samples.contains(&0) && samples.contains(&bit_depth.max_sample()));
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
