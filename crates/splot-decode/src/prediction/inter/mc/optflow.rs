// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::simd::{Simd, num::SimdUint};

use splot_parallel::prelude::*;
use splot_recon::math::round2_signed_i32;
use splot_recon::{
    OptflowScratch, derive_optflow_mv_delta_8x8_strided_into, derive_optflow_mv_deltas_into,
};

use super::*;

pub(super) trait CompoundAverageOutput: Sized {
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
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MotionCell {
    base_mvs: [Mv; 2],
    mvs: [[i32; 2]; 2],
}

impl MotionCell {
    fn from_optflow(base_mvs: [Mv; 2], delta: [[i32; 2]; 2]) -> Self {
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

    fn is_initialized(&self) -> bool {
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

    fn refinemv_candidates_at_index(&self, index: usize) -> Option<([Mv; 2], usize)> {
        match &self.refinemv_candidates {
            RefinemvCandidates::None => None,
            RefinemvCandidates::Uniform {
                candidates,
                unit_size,
            } => Some((*candidates, *unit_size)),
            RefinemvCandidates::PerCell {
                candidates,
                unit_size,
            } => candidates
                .get(index)
                .copied()
                .map(|candidates| (candidates, *unit_size)),
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
    OPTFLOW_SCRATCH.with(|slot| {
        let mut scratch = slot.take().unwrap_or_default();
        let result = (|| {
            let delta = derive_optflow_mv_delta_8x8_strided_into(
                pred0,
                start0,
                pred1,
                start1,
                stride,
                bit_depth,
                distances,
                &mut scratch,
            )?;
            Ok(MotionCell::from_optflow(base_mvs, delta))
        })();
        slot.set(Some(scratch));
        result
    })
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
                    initial_luma_prediction(
                        sink,
                        block.reference0,
                        prediction_rect,
                        base_mvs[0],
                        InterpolationFilter::Bilinear,
                        candidates.map(|mvs| (mvs[0], region_w, region_h)),
                        offset,
                        false,
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
                        false,
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
        refinemv_candidates: refinemv_candidates.map_or(RefinemvCandidates::None, |candidates| {
            RefinemvCandidates::Uniform {
                candidates,
                unit_size: 16,
            }
        }),
    }))
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
    if unit_count >= 1024
        && splot_parallel::current_pool_width() > 1
        && splot_parallel::on_worker_pool()
    {
        let mut cells = take_motion_cells(
            unit_count,
            MotionCell::uninitialized([block.mv0, block.mv1]),
        );
        cells
            .par_chunks_mut(columns)
            .enumerate()
            .try_for_each(|(row, cells)| {
                let mut initial_predictions = [[0u16; super::refinemv::TIP_PREDICTION_AREA]; 2];
                let mut previous_unit: Option<(McBlockRect, [Mv; 2])> = None;
                let mut previous_refined = false;
                for (column, destination) in cells.iter_mut().enumerate() {
                    let (rect, mvs) = unit_at(row * columns + column);
                    let reuse_horizontal =
                        previous_unit.map_or([false; 2], |(previous_rect, previous_mvs)| {
                            core::array::from_fn(|reference| {
                                previous_refined
                                    && rect.luma_x == previous_rect.luma_x + previous_rect.luma_w
                                    && mvs[reference] == previous_mvs[reference]
                            })
                        });
                    let mut unit = block;
                    unit.rect = rect;
                    unit.mv0 = mvs[0];
                    unit.mv1 = mvs[1];
                    unit.has_chroma = false;
                    unit.sub8x8_chroma = false;
                    let refined = super::refinemv::tip_refinemv_optflow_motion_cell(
                        sink,
                        unit,
                        offset,
                        reuse_horizontal,
                        &mut initial_predictions,
                    )?;
                    previous_unit = Some((rect, mvs));
                    previous_refined = refined.is_some();
                    *destination = if let Some(cell) = refined {
                        cell
                    } else {
                        let refinemv = unit
                            .use_refinemv
                            .then(|| {
                                super::refinemv::compound_default_refinemv_motion_grid(
                                    sink, unit, offset,
                                )
                            })
                            .transpose()?;
                        compound_motion_grid(sink, unit, Some(unit_size), refinemv, offset)?
                            .as_ref()
                            .map(|motion| motion.cell_at_luma_offset(0, 0))
                            .transpose()?
                            .unwrap_or_else(|| MotionCell::from_refinemv(mvs))
                    };
                }
                Ok::<_, crate::error::DecodeError>(())
            })?;
        return Ok(CompoundMotionGrid {
            unit_size,
            columns,
            cells: MotionCells::from_vec(cells),
            refinemv_candidates: RefinemvCandidates::PerCell {
                candidates: {
                    refinemv_candidates.extend((0..unit_count).map(|index| unit_at(index).1));
                    refinemv_candidates
                },
                unit_size,
            },
        });
    }
    let mut cells = take_motion_cells(
        unit_count,
        MotionCell::uninitialized([block.mv0, block.mv1]),
    );
    let mut initial_predictions = [[0u16; super::refinemv::TIP_PREDICTION_AREA]; 2];
    let mut previous_unit: Option<(McBlockRect, [Mv; 2])> = None;
    let mut previous_refined = false;
    for index in 0..unit_count {
        let (rect, mvs) = unit_at(index);
        let reuse_horizontal = previous_unit.map_or([false; 2], |(previous_rect, previous_mvs)| {
            core::array::from_fn(|reference| {
                previous_refined
                    && rect.luma_y == previous_rect.luma_y
                    && rect.luma_x == previous_rect.luma_x + previous_rect.luma_w
                    && mvs[reference] == previous_mvs[reference]
            })
        });
        refinemv_candidates.push(mvs);
        let mut unit = block;
        unit.rect = rect;
        unit.mv0 = mvs[0];
        unit.mv1 = mvs[1];
        unit.has_chroma = false;
        unit.sub8x8_chroma = false;
        let refined = super::refinemv::tip_refinemv_optflow_motion_cell(
            sink,
            unit,
            offset,
            reuse_horizontal,
            &mut initial_predictions,
        )?;
        previous_unit = Some((rect, mvs));
        previous_refined = refined.is_some();
        if let Some(cell) = refined {
            let destination = cells.get_mut(index).ok_or(ReconError::ArithmeticOverflow {
                context: "TIP compound motion-grid write",
            })?;
            *destination = cell;
            continue;
        }
        let refinemv = unit
            .use_refinemv
            .then(|| super::refinemv::compound_default_refinemv_motion_grid(sink, unit, offset))
            .transpose()?;
        let motion = compound_motion_grid(sink, unit, Some(unit_size), refinemv, offset)?;
        let cell = motion
            .as_ref()
            .map(|motion| motion.cell_at_luma_offset(0, 0))
            .transpose()?
            .unwrap_or_else(|| MotionCell::from_refinemv(mvs));
        let destination = cells.get_mut(index).ok_or(ReconError::ArithmeticOverflow {
            context: "TIP compound motion-grid write",
        })?;
        *destination = cell;
    }
    Ok(CompoundMotionGrid {
        unit_size,
        columns,
        cells: MotionCells::from_vec(cells),
        refinemv_candidates: RefinemvCandidates::PerCell {
            candidates: refinemv_candidates,
            unit_size,
        },
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn initial_luma_prediction<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    reference: ReferenceSamples<'_, T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    refinemv_area: Option<(Mv, usize, usize)>,
    offset: ByteOffset,
    reuse_horizontal: bool,
    output: &mut [u16],
) -> Result<()> {
    let reference_size = reference.info().coded_luma_size();
    let frame_size = sink.info().coded_luma_size();
    let scaling = derive_plane_scaling(
        rect.luma_x as i32,
        rect.luma_y as i32,
        mv.row,
        mv.col,
        0,
        0,
        reference_size.width() as i32,
        reference_size.height() as i32,
        frame_size.width() as i32,
        frame_size.height() as i32,
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
            scaling,
        )
    });
    let params = SubpelPredictParams {
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
    };
    let (view, _, _) =
        reference.plane_view(PlaneId::Y, subpel_last_reference_row(&params), offset)?;
    if reuse_horizontal {
        let reused = subpel_predict_16x16_bilinear_horizontal_overlap_into(&view, &params, output)?;
        if reused {
            return Ok(());
        }
    }
    subpel_predict_block_into(&view, &params, output).map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn compound_optflow_subpel_params<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    prediction: &CompoundSubpelPlane<'_, T>,
    cell: MotionCell,
    scalings: [PlaneScaling; 2],
    cell_index: usize,
    row: usize,
    col: usize,
    width: usize,
    height: usize,
) -> [SubpelPredictParams; 2] {
    let bounds =
        if let Some((mvs, refine_unit_size)) = motion.refinemv_candidates_at_index(cell_index) {
            let refine_unit_w = refine_unit_size >> sub_x;
            let refine_unit_h = refine_unit_size >> sub_y;
            let refine_col = col & !(refine_unit_w - 1);
            let refine_row = row & !(refine_unit_h - 1);
            let refine_w = refine_unit_w.min(prediction.block_w - refine_col);
            let refine_h = refine_unit_h.min(prediction.block_h - refine_row);
            core::array::from_fn(|reference| {
                Some(super::refinemv::reference_area_bounds(
                    (prediction.plane_x + refine_col) as i32,
                    (prediction.plane_y + refine_row) as i32,
                    refine_w,
                    refine_h,
                    mvs[reference],
                    sub_x,
                    sub_y,
                    prediction.scalings[reference],
                ))
            })
        } else if let Some((area_width, area_height)) = subblock_reference_area_size(
            plane,
            (motion.unit_size >> sub_x).max(4),
            (motion.unit_size >> sub_y).max(4),
        ) {
            core::array::from_fn(|reference| {
                Some(super::refinemv::reference_area_bounds(
                    (prediction.plane_x + col) as i32,
                    (prediction.plane_y + row) as i32,
                    area_width,
                    area_height,
                    cell.base_mvs[reference],
                    sub_x,
                    sub_y,
                    prediction.scalings[reference],
                ))
            })
        } else {
            [None; 2]
        };
    core::array::from_fn(|reference| {
        let scaling = scalings[reference];
        SubpelPredictParams {
            interp: block.interp,
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
            bit_depth: sink.info().bit_depth(),
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn predict_uniform_motion_compound_average_into<
    T: ReconSample,
    O: CompoundAverageOutput + Send,
>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
    cwp_weight: i16,
    offset: ByteOffset,
    output: &mut [O],
) -> Result<bool> {
    let [cell] = motion.cells.as_slice() else {
        return Ok(false);
    };
    let prediction = super::compound_subpel_plane(sink, block, plane, sub_x, sub_y, offset)?;
    let subblock_w = (motion.unit_size >> sub_x).max(4);
    let subblock_h = (motion.unit_size >> sub_y).max(4);
    if prediction.block_w > subblock_w || prediction.block_h > subblock_h {
        return Ok(false);
    }
    let coded_luma_size = sink.info().coded_luma_size();
    let frame_w = (coded_luma_size.width().div_ceil(4) * 4) >> sub_x;
    let frame_h = (coded_luma_size.height().div_ceil(4) * 4) >> sub_y;
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
        sink,
        block,
        plane,
        sub_x,
        sub_y,
        motion,
        &prediction,
        *cell,
        scalings,
        0,
        0,
        0,
        prediction.block_w,
        prediction.block_h,
    );
    let output_stride = prediction.block_w;
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

#[allow(clippy::too_many_arguments)]
pub(super) fn predict_motion_grid_compound_average_into<
    T: ReconSample,
    O: CompoundAverageOutput + Send,
>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
    cwp_weight: i16,
    offset: ByteOffset,
    output: &mut [O],
) -> Result<bool> {
    if motion.cells.as_slice().len() == 1 {
        return Ok(false);
    }
    let prediction = super::compound_subpel_plane(sink, block, plane, sub_x, sub_y, offset)?;
    let sample_count = prediction.block_w.checked_mul(prediction.block_h).ok_or(
        ReconError::ArithmeticOverflow {
            context: "TIP batched compound output sample count",
        },
    )?;
    if output.len() != sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: output.len(),
        }
        .into());
    }
    let coded_luma_size = sink.info().coded_luma_size();
    let frame_w = (coded_luma_size.width().div_ceil(4) * 4) >> sub_x;
    let frame_h = (coded_luma_size.height().div_ceil(4) * 4) >> sub_y;
    let subblock_w = (motion.unit_size >> sub_x).max(4);
    let subblock_h = (motion.unit_size >> sub_y).max(4);
    let process_row = |cell_row: usize,
                       row: usize,
                       output: &mut [O],
                       pred0_scratch: &mut [i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES],
                       intermediate_scratch: &mut [i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE]|
     -> Result<bool> {
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
            if !super::compound_average_weights_are_uniform(
                implicit_mask,
                cwp_weight,
                width,
                height,
                prediction.scalings,
                Some(scalings),
                (frame_w, frame_h),
            ) {
                return Ok(false);
            }
            let params = compound_optflow_subpel_params(
                sink,
                block,
                plane,
                sub_x,
                sub_y,
                motion,
                &prediction,
                cell,
                scalings,
                cell_index,
                row,
                col,
                width,
                height,
            );
            let subplane = CompoundSubpelPlane {
                views: prediction.views,
                plane_x: prediction.plane_x + col,
                plane_y: prediction.plane_y + row,
                block_w: width,
                block_h: height,
                scalings,
            };
            if O::predict_fast(
                &subplane.views[0],
                &params[0],
                &subplane.views[1],
                &params[1],
                cwp_weight,
                intermediate_scratch,
                &mut output[col..],
                prediction.block_w,
            )? {
                continue;
            }
            super::predict_compound_average_into(
                &subplane,
                &params,
                cwp_weight,
                Some(pred0_scratch),
                Some(intermediate_scratch),
                &mut output[col..],
                prediction.block_w,
            )?;
        }
        Ok(true)
    };
    let parallel = prediction
        .block_w
        .checked_mul(prediction.block_h)
        .is_some_and(|samples| samples >= 256 * 256)
        && splot_parallel::on_worker_pool();
    if parallel {
        let uniform = std::sync::atomic::AtomicBool::new(true);
        let row_samples = prediction.block_w * subblock_h;
        output
            .par_chunks_mut(row_samples)
            .enumerate()
            .try_for_each(|(cell_row, output)| {
                let row = cell_row * subblock_h;
                let mut pred0_scratch = [0i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES];
                let mut intermediate_scratch = [0i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE];
                if !process_row(
                    cell_row,
                    row,
                    output,
                    &mut pred0_scratch,
                    &mut intermediate_scratch,
                )? {
                    uniform.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                Ok::<_, crate::error::DecodeError>(())
            })?;
        return Ok(uniform.load(std::sync::atomic::Ordering::Relaxed));
    }
    let mut pred0_scratch = [0i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES];
    let mut intermediate_scratch = [0i16; MAX_MOTION_GRID_SUBPEL_INTERMEDIATE];
    for (cell_row, row) in (0..prediction.block_h).step_by(subblock_h).enumerate() {
        let output_start = row * prediction.block_w;
        if !process_row(
            cell_row,
            row,
            &mut output[output_start..],
            &mut pred0_scratch,
            &mut intermediate_scratch,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
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
) -> splot_recon::Result<()> {
    if output.is_empty() {
        return Ok(());
    }
    let last_x = frame_w as i32 - 1;
    let last_y = frame_h as i32 - 1;
    let max_sample = i32::from(bit_depth.max_sample());
    let shift = 1 + compound_inter_post_round();
    let blend = |slot: &mut T, left: i32, right: i32, starts: [(i32, i32); 2]| {
        let ref0_onscreen =
            (0..=last_x).contains(&starts[0].0) && (0..=last_y).contains(&starts[0].1);
        let ref1_onscreen =
            (0..=last_x).contains(&starts[1].0) && (0..=last_y).contains(&starts[1].1);
        let mask = match (ref0_onscreen, ref1_onscreen) {
            (true, false) => 2,
            (false, true) => 0,
            _ => 1,
        };
        let sample = round2_i32(mask * left + (2 - mask) * right, shift);
        *slot = T::try_from_u16(sample.clamp(0, max_sample) as u16)?;
        Ok(())
    };
    if let Some(motion) = motion {
        let unit_width = (motion.unit_size >> sub_x).max(1);
        let unit_height = (motion.unit_size >> sub_y).max(1);
        for cell_y in (0..height).step_by(unit_height) {
            for cell_x in (0..width).step_by(unit_width) {
                let mvs = motion.at_luma_offset(cell_x << sub_x, cell_y << sub_y)?;
                let cell_end_x = (cell_x + unit_width).min(width);
                let cell_end_y = (cell_y + unit_height).min(height);
                for row in cell_y..cell_end_y {
                    let start = row * width + cell_x;
                    let end = row * width + cell_end_x;
                    let row_samples = output[start..end]
                        .iter_mut()
                        .zip(pred0[start..end].iter().zip(&pred1[start..end]));
                    for (local_col, (slot, (&left, &right))) in row_samples.enumerate() {
                        let col = cell_x + local_col;
                        let starts = core::array::from_fn(|reference| {
                            let scaling = scaling_templates[reference].with_prescaled_mv(
                                (plane_x + col) as i32,
                                (plane_y + row) as i32,
                                mvs[reference][0],
                                mvs[reference][1],
                                sub_x,
                                sub_y,
                            );
                            (scaling.start_x >> 10, scaling.start_y >> 10)
                        });
                        blend(slot, left, right, starts)?;
                    }
                }
            }
        }
        return Ok(());
    }
    let reference_starts =
        scaling_templates.map(|scaling| (scaling.start_x >> 10, scaling.start_y >> 10));
    for (row, ((output, pred0), pred1)) in output
        .chunks_mut(width)
        .zip(pred0.chunks(width))
        .zip(pred1.chunks(width))
        .enumerate()
    {
        for (col, (slot, (&left, &right))) in
            output.iter_mut().zip(pred0.iter().zip(pred1)).enumerate()
        {
            let starts = reference_starts.map(|(x, y)| (x + col as i32, y + row as i32));
            blend(slot, left, right, starts)?;
        }
    }
    Ok(())
}

pub(super) fn compound_optflow_plane_prediction<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let prediction = super::compound_subpel_plane(sink, block, plane, sub_x, sub_y, offset)?;
    let subblock_w = (motion.unit_size >> sub_x).max(4);
    let subblock_h = (motion.unit_size >> sub_y).max(4);
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
                sink,
                block,
                plane,
                sub_x,
                sub_y,
                motion,
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
            refinemv_candidates: RefinemvCandidates::None,
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
