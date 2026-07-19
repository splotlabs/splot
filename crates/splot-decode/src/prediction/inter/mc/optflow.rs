// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::round2_signed_i32;
use splot_recon::{OptflowScratch, derive_optflow_mv_deltas_into};

use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct MotionCell {
    base_mvs: [Mv; 2],
    mvs: [[i32; 2]; 2],
}

impl MotionCell {
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
/// Horizontal-pass rows plus one materialized predictor for a 16x16 subblock.
const MAX_MOTION_GRID_SUBPEL_SCRATCH: usize = 16 * (16 + 7) + MAX_MOTION_GRID_SUBBLOCK_SAMPLES;

std::thread_local! {
    static OPTFLOW_SCRATCH: std::cell::Cell<Option<OptflowScratch>> =
        const { std::cell::Cell::new(None) };
    static OPTFLOW_MOTION_CELLS: std::cell::RefCell<[Option<Vec<MotionCell>>; 2]> =
        const { std::cell::RefCell::new([None, None]) };
    static TIP_REFINEMV_CANDIDATES: std::cell::Cell<Option<Vec<[Mv; 2]>>> =
        const { std::cell::Cell::new(None) };
}

fn take_tip_refinemv_candidates(capacity: usize) -> Vec<[Mv; 2]> {
    TIP_REFINEMV_CANDIDATES.with(|slot| {
        let mut candidates = slot.take().unwrap_or_default();
        candidates.clear();
        candidates.reserve(capacity);
        candidates
    })
}

fn recycle_tip_refinemv_candidates(mut candidates: Vec<[Mv; 2]>) {
    candidates.clear();
    TIP_REFINEMV_CANDIDATES.with(|slot| {
        let saved = slot.take();
        if saved
            .as_ref()
            .is_none_or(|saved| saved.capacity() < candidates.capacity())
        {
            slot.set(Some(candidates));
        } else {
            slot.set(saved);
        }
    });
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

impl Drop for RefinemvCandidates {
    fn drop(&mut self) {
        if let Self::PerCell { candidates, .. } = self {
            recycle_tip_refinemv_candidates(core::mem::take(candidates));
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

    fn refinemv_candidates_at_luma_offset(&self, x: usize, y: usize) -> Option<([Mv; 2], usize)> {
        match &self.refinemv_candidates {
            RefinemvCandidates::None => None,
            RefinemvCandidates::Uniform {
                candidates,
                unit_size,
            } => Some((*candidates, *unit_size)),
            RefinemvCandidates::PerCell {
                candidates,
                unit_size,
            } => {
                let column = x / self.unit_size;
                let row = y / self.unit_size;
                row.checked_mul(self.columns)
                    .and_then(|row| row.checked_add(column))
                    .and_then(|index| candidates.get(index))
                    .copied()
                    .map(|candidates| (candidates, *unit_size))
            }
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
        Ok(core::array::from_fn(|reference| {
            let base = cell.base_mvs[reference];
            let row_delta = cell.mvs[reference][0] - base.row * 2;
            let col_delta = cell.mvs[reference][1] - base.col * 2;
            Mv {
                row: base.row + round2_signed_i32(row_delta, 1),
                col: base.col + round2_signed_i32(col_delta, 1),
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
        self.cells
            .as_slice()
            .get(index)
            .copied()
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound motion-grid lookup",
            })
    }
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

pub(super) fn tip_optflow_motion_cell(
    pred0: &[u16],
    pred1: &[u16],
    bit_depth: splot_recon::BitDepth,
    distances: [i32; 2],
    sad_threshold: Option<u32>,
    base_mvs: [Mv; 2],
) -> Result<MotionCell> {
    if sad_threshold.is_some_and(|threshold| normalized_sad(pred0, pred1, bit_depth) < threshold) {
        return Ok(MotionCell::from_refinemv(base_mvs));
    }
    OPTFLOW_SCRATCH.with(|slot| {
        let mut scratch = slot.take().unwrap_or_default();
        let result = (|| {
            let deltas = derive_optflow_mv_deltas_into(
                pred0,
                pred1,
                8,
                8,
                8,
                bit_depth,
                distances,
                &mut scratch,
            )?;
            let delta = deltas
                .first()
                .copied()
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "TIP optical-flow delta",
                })?;
            let mut refined = [[0i32; 2]; 2];
            for reference in 0..2 {
                let base = [base_mvs[reference].row, base_mvs[reference].col];
                for component in 0..2 {
                    refined[reference][component] = (base[component] * 2
                        + delta[reference][component])
                        .clamp(-(1 << 17), (1 << 17) - 1);
                }
            }
            Ok(MotionCell {
                base_mvs,
                mvs: refined,
            })
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
                                let mut refined = [[0i32; 2]; 2];
                                for reference in 0..2 {
                                    let base = [base_mvs[reference].row, base_mvs[reference].col];
                                    for component in 0..2 {
                                        refined[reference][component] = (base[component] * 2
                                            + delta[reference][component])
                                            .clamp(-(1 << 17), (1 << 17) - 1);
                                    }
                                }
                                *cell = MotionCell {
                                    base_mvs,
                                    mvs: refined,
                                };
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

pub(super) fn tip_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    unit_size: usize,
    columns: usize,
    units: impl ExactSizeIterator<Item = (McBlockRect, [Mv; 2])>,
    offset: ByteOffset,
) -> Result<CompoundMotionGrid> {
    let unit_count = units.len();
    if unit_count == 0 || columns == 0 {
        return Err(ReconError::ZeroDimension {
            field: "TIP compound motion grid",
        }
        .into());
    }
    let mut cells = take_motion_cells(
        unit_count,
        MotionCell::uninitialized([block.mv0, block.mv1]),
    );
    let mut refinemv_candidates = take_tip_refinemv_candidates(unit_count);
    let mut initial_predictions = [[0u16; super::refinemv::TIP_PREDICTION_AREA]; 2];
    let mut previous_unit: Option<(McBlockRect, [Mv; 2])> = None;
    let mut previous_refined = false;
    for (index, (rect, mvs)) in units.enumerate() {
        let reuse_horizontal = previous_unit.map_or([false; 2], |(previous_rect, previous_mvs)| {
            core::array::from_fn(|reference| {
                previous_refined
                    && rect.luma_y == previous_rect.luma_y
                    && rect.luma_x == previous_rect.luma_x + previous_rect.luma_w
                    && mvs[reference] == previous_mvs[reference]
                    && mvs[reference].row % 8 == 0
                    && mvs[reference].col % 8 == 0
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
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    refinemv_area: Option<(Mv, usize, usize)>,
    offset: ByteOffset,
    reuse_horizontal: bool,
    output: &mut [u16],
) -> Result<()> {
    let (view, _, _) = reference_plane_view(reference, PlaneId::Y, offset)?;
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
    if reuse_horizontal
        && subpel_predict_16x16_fullpel_horizontal_overlap_into(&view, &params, output)?
    {
        return Ok(());
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
    reference: usize,
    scaling: PlaneScaling,
    row: usize,
    col: usize,
    width: usize,
    height: usize,
) -> SubpelPredictParams {
    let scaling_template = prediction.scalings[reference];
    let bounds = if let Some((mvs, refine_unit_size)) =
        motion.refinemv_candidates_at_luma_offset(col << sub_x, row << sub_y)
    {
        let refine_unit_w = refine_unit_size >> sub_x;
        let refine_unit_h = refine_unit_size >> sub_y;
        let refine_col = col / refine_unit_w * refine_unit_w;
        let refine_row = row / refine_unit_h * refine_unit_h;
        let refine_w = refine_unit_w.min(prediction.block_w - refine_col);
        let refine_h = refine_unit_h.min(prediction.block_h - refine_row);
        Some(super::refinemv::reference_area_bounds(
            (prediction.plane_x + refine_col) as i32,
            (prediction.plane_y + refine_row) as i32,
            refine_w,
            refine_h,
            mvs[reference],
            sub_x,
            sub_y,
            scaling_template,
        ))
    } else if let Some((area_width, area_height)) = subblock_reference_area_size(
        plane,
        (motion.unit_size >> sub_x).max(4),
        (motion.unit_size >> sub_y).max(4),
    ) {
        Some(super::refinemv::reference_area_bounds(
            (prediction.plane_x + col) as i32,
            (prediction.plane_y + row) as i32,
            area_width,
            area_height,
            cell.base_mvs[reference],
            sub_x,
            sub_y,
            scaling_template,
        ))
    } else {
        None
    };
    SubpelPredictParams {
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
        bit_depth: sink.info().bit_depth(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn predict_uniform_motion_compound_average_into<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
    cwp_weight: i16,
    offset: ByteOffset,
    output: &mut [u16],
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
    let params0 = compound_optflow_subpel_params(
        sink,
        block,
        plane,
        sub_x,
        sub_y,
        motion,
        &prediction,
        *cell,
        0,
        scalings[0],
        0,
        0,
        prediction.block_w,
        prediction.block_h,
    );
    let params1 = compound_optflow_subpel_params(
        sink,
        block,
        plane,
        sub_x,
        sub_y,
        motion,
        &prediction,
        *cell,
        1,
        scalings[1],
        0,
        0,
        prediction.block_w,
        prediction.block_h,
    );
    let output_stride = prediction.block_w;
    let mut pred0_scratch = [0i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES];
    let mut intermediate_scratch = [0i32; MAX_MOTION_GRID_SUBPEL_SCRATCH];
    super::predict_compound_average_into(
        &prediction,
        &[params0, params1],
        cwp_weight,
        Some(&mut pred0_scratch),
        Some(&mut intermediate_scratch),
        output,
        output_stride,
    )?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn predict_motion_grid_compound_average_into<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
    cwp_weight: i16,
    offset: ByteOffset,
    output: &mut [u16],
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
    let mut pred0_scratch = [0i32; MAX_MOTION_GRID_SUBBLOCK_SAMPLES];
    let mut intermediate_scratch = [0i32; MAX_MOTION_GRID_SUBPEL_SCRATCH];
    for row in (0..prediction.block_h).step_by(subblock_h) {
        for col in (0..prediction.block_w).step_by(subblock_w) {
            let width = subblock_w.min(prediction.block_w - col);
            let height = subblock_h.min(prediction.block_h - row);
            let cell = motion.cell_at_luma_offset(col << sub_x, row << sub_y)?;
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
            let params: [SubpelPredictParams; 2] = core::array::from_fn(|reference| {
                compound_optflow_subpel_params(
                    sink,
                    block,
                    plane,
                    sub_x,
                    sub_y,
                    motion,
                    &prediction,
                    cell,
                    reference,
                    scalings[reference],
                    row,
                    col,
                    width,
                    height,
                )
            });
            let subplane = CompoundSubpelPlane {
                views: prediction.views,
                plane_x: prediction.plane_x + col,
                plane_y: prediction.plane_y + row,
                block_w: width,
                block_h: height,
                scalings,
            };
            let output_start = row * prediction.block_w + col;
            if subpel_predict_block_compound_average_fullpel_strided_into(
                &subplane.views[0],
                &params[0],
                &subplane.views[1],
                &params[1],
                cwp_weight,
                &mut output[output_start..],
                prediction.block_w,
            )? {
                continue;
            }
            super::predict_compound_average_into(
                &subplane,
                &params,
                cwp_weight,
                Some(&mut pred0_scratch),
                Some(&mut intermediate_scratch),
                &mut output[output_start..],
                prediction.block_w,
            )?;
        }
    }
    Ok(true)
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

    for row in (0..prediction.block_h).step_by(subblock_h) {
        for col in (0..prediction.block_w).step_by(subblock_w) {
            let width = subblock_w.min(prediction.block_w - col);
            let height = subblock_h.min(prediction.block_h - row);
            let luma_x = col << sub_x;
            let luma_y = row << sub_y;
            let cell = motion.cell_at_luma_offset(luma_x, luma_y)?;
            for (reference, (view, output)) in prediction
                .views
                .iter()
                .zip([&mut pred0, &mut pred1])
                .enumerate()
            {
                let scaling = prediction.scalings[reference].with_prescaled_mv(
                    (prediction.plane_x + col) as i32,
                    (prediction.plane_y + row) as i32,
                    cell.mvs[reference][0],
                    cell.mvs[reference][1],
                    sub_x,
                    sub_y,
                );
                let params = compound_optflow_subpel_params(
                    sink,
                    block,
                    plane,
                    sub_x,
                    sub_y,
                    motion,
                    &prediction,
                    cell,
                    reference,
                    scaling,
                    row,
                    col,
                    width,
                    height,
                );
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
    fn per_cell_refinemv_candidates_are_independent_of_searched_base_mvs() {
        let candidates = [
            [Mv { row: 1, col: 2 }, Mv { row: 3, col: 4 }],
            [Mv { row: 5, col: 6 }, Mv { row: 7, col: 8 }],
        ];
        let grid = CompoundMotionGrid {
            unit_size: 8,
            columns: 2,
            cells: MotionCells::Heap(vec![
                MotionCell::from_refinemv([Mv::ZERO; 2]);
                candidates.len()
            ]),
            refinemv_candidates: RefinemvCandidates::PerCell {
                candidates: candidates.to_vec(),
                unit_size: 8,
            },
        };

        assert_eq!(
            grid.refinemv_candidates_at_luma_offset(0, 0),
            Some((candidates[0], 8))
        );
        assert_eq!(
            grid.refinemv_candidates_at_luma_offset(8, 0),
            Some((candidates[1], 8))
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
