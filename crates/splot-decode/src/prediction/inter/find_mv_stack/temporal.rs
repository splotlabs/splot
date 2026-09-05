// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_parallel::CompletionCell;
use splot_recon::math::{round2_signed, round2_signed_i32};
use std::ops::Range;
use std::sync::Arc;

use super::{
    Mv, MvBlockContext, NeighbourCell, NeighbourMvGrid, RelativeProbe, TIP_REF_FRAME,
    warp_sub_mv_at,
};
use selection::projection_queue;
#[cfg(test)]
use trajectory::TrajectoryMotionField;
use trajectory::{OwnedTrajectoryBand, OwnedTrajectoryFields, TrajectoryBand, TrajectoryState};

mod selection;
mod trajectory;

const MAX_FRAME_DISTANCE: i32 = 31;
const REFMVS_LIMIT: i32 = (1 << 11) - 1;
const MV_LIMIT: i32 = (1 << 16) - 1;
const INVALID_TEMPORAL_REF: u8 = u8::MAX;
const DIV_MULT: [i32; 32] = [
    0, 16384, 8192, 5461, 4096, 3276, 2730, 2340, 2048, 1820, 1638, 1489, 1365, 1260, 1170, 1092,
    1024, 963, 910, 862, 819, 780, 744, 712, 682, 655, 630, 606, 585, 564, 546, 528,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompressedTemporalMv {
    row: i8,
    col: i8,
}

impl CompressedTemporalMv {
    const ZERO: Self = Self { row: 0, col: 0 };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TemporalMotionCell {
    ref_indices: [u8; 2],
    mvs: [CompressedTemporalMv; 2],
}

impl Default for TemporalMotionCell {
    fn default() -> Self {
        Self {
            ref_indices: [INVALID_TEMPORAL_REF; 2],
            mvs: [CompressedTemporalMv::ZERO; 2],
        }
    }
}

/// The order hints one motion field carries, one per reference slot.
///
/// A sequence bounds the slot count at
/// [`splot_core::headers::sequence::MAX_REF_FRAMES`], so the list is inline and
/// a field's metadata costs no allocation to build or to hand to a band.
pub(crate) type RefOrderHints =
    splot_core::tile::InlineVec<Option<u32>, { splot_core::headers::sequence::MAX_REF_FRAMES }>;

fn empty_ref_order_hints() -> RefOrderHints {
    RefOrderHints::default()
}

fn allocate_temporal_grid<T>(mi_rows: usize, mi_cols: usize) -> Option<(usize, usize, Vec<T>)>
where
    T: Clone + Default + Send + 'static,
{
    let width8 = mi_cols.div_ceil(2);
    let height8 = mi_rows.div_ceil(2);
    let cells = width8.checked_mul(height8)?;
    let mut grid = crate::support::buffer_pool::take::<T>(cells);
    if grid.capacity() < cells {
        grid.try_reserve_exact(cells).ok()?;
    }
    grid.clear();
    grid.resize(cells, T::default());
    Some((width8, height8, grid))
}

fn temporal_grid_index(width8: usize, height8: usize, y8: usize, x8: usize) -> Option<usize> {
    if y8 >= height8 || x8 >= width8 {
        return None;
    }
    Some(y8 * width8 + x8)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionField {
    width8: usize,
    height8: usize,
    band_rows8: usize,
    storage: TemporalMotionStorage,
    pending_ref_hints: Option<Vec<[u32; 2]>>,
    is_inter: bool,
    frame_size: Option<(usize, usize)>,
    ref_order_hints: RefOrderHints,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TemporalMotionStorage {
    Contiguous(Vec<TemporalMotionCell>),
    Bands(Vec<TemporalMotionBand>),
}

/// Fixed geometry of one frame's motion-field publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MotionFieldLayout {
    width8: usize,
    height8: usize,
    band_rows8: usize,
}

impl MotionFieldLayout {
    pub(crate) fn new(mi_rows: usize, mi_cols: usize, sb_h4: usize) -> Option<Self> {
        let width8 = mi_cols.div_ceil(2);
        let height8 = mi_rows.div_ceil(2);
        let band_rows8 = sb_h4.checked_div(2)?;
        (width8 > 0 && height8 > 0 && matches!(band_rows8, 8 | 16 | 32)).then_some(Self {
            width8,
            height8,
            band_rows8,
        })
    }

    pub(crate) const fn width8(self) -> usize {
        self.width8
    }

    pub(crate) const fn height8(self) -> usize {
        self.height8
    }

    pub(crate) const fn band_rows8(self) -> usize {
        self.band_rows8
    }

    pub(crate) fn band_count(self) -> usize {
        self.height8.div_ceil(self.band_rows8)
    }

    pub(crate) fn rows8(self, index: usize) -> Range<usize> {
        let start = index.saturating_mul(self.band_rows8).min(self.height8);
        start..start.saturating_add(self.band_rows8).min(self.height8)
    }
}

/// Semantic metadata required to select and project a reference field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionFieldMetadata {
    is_inter: bool,
    frame_size: Option<(usize, usize)>,
    ref_order_hints: RefOrderHints,
}

/// One immutable full-width source superblock row of temporal motion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionBand {
    layout: MotionFieldLayout,
    metadata: TemporalMotionFieldMetadata,
    row_base8: usize,
    cells: Vec<TemporalMotionCell>,
}

impl TemporalMotionBand {
    pub(crate) fn row_end8(&self) -> usize {
        self.row_base8
            .saturating_add(self.cells.len().div_ceil(self.layout.width8.max(1)))
            .min(self.layout.height8)
    }

    #[allow(
        clippy::inline_always,
        reason = "TMVP projection reads one row at a time"
    )]
    #[inline(always)]
    fn row(&self, y8: usize) -> Option<&[TemporalMotionCell]> {
        let row = y8.checked_sub(self.row_base8)?;
        let start = row.checked_mul(self.layout.width8)?;
        let end = start.checked_add(self.layout.width8)?;
        self.cells.get(start..end)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub(crate) fn record_block(&mut self, block: TemporalMotionBlock) {
        let row_base8 = self.row_base8;
        let row_end8 = self.row_end8();
        let width8 = self.layout.width8;
        let resolved = resolve_block_refs(block.ref_order_hints, &self.metadata.ref_order_hints);
        let cells = &mut self.cells;
        visit_temporal_block_cells(block, width8, row_end8, |y8, x8, cell, hints| {
            let Some(row) = y8.checked_sub(row_base8) else {
                return;
            };
            let Some(index) = row
                .checked_mul(width8)
                .and_then(|base| base.checked_add(x8))
            else {
                return;
            };
            if y8 >= row_base8
                && let Some(target) = cells.get_mut(index)
            {
                *target = resolve_temporal_refs(cell, hints, &resolved);
            }
        });
    }
}

impl TemporalMotionField {
    pub(crate) fn empty() -> Self {
        Self {
            width8: 0,
            height8: 0,
            band_rows8: 8,
            storage: TemporalMotionStorage::Contiguous(Vec::new()),
            pending_ref_hints: None,
            is_inter: false,
            frame_size: None,
            ref_order_hints: empty_ref_order_hints(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        let pending_ref_hints = vec![[u32::MAX; 2]; cells.len()];
        Some(Self {
            width8,
            height8,
            band_rows8: 8,
            storage: TemporalMotionStorage::Contiguous(cells),
            pending_ref_hints: Some(pending_ref_hints),
            is_inter: false,
            frame_size: None,
            ref_order_hints: empty_ref_order_hints(),
        })
    }

    pub(crate) fn new_with_metadata(
        mi_rows: usize,
        mi_cols: usize,
        is_inter: bool,
        frame_size: (usize, usize),
        ref_order_hints: &[Option<u32>],
    ) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        let mut owned_ref_order_hints = RefOrderHints::default();
        owned_ref_order_hints.extend_within(ref_order_hints.iter().copied());
        Some(Self {
            width8,
            height8,
            band_rows8: 8,
            storage: TemporalMotionStorage::Contiguous(cells),
            pending_ref_hints: None,
            is_inter,
            frame_size: Some(frame_size),
            ref_order_hints: owned_ref_order_hints,
        })
    }

    #[cfg(test)]
    pub(crate) fn set_reference_metadata(
        &mut self,
        is_inter: bool,
        frame_size: (usize, usize),
        ref_order_hints: &[Option<u32>],
    ) {
        self.is_inter = is_inter;
        self.frame_size = Some(frame_size);
        self.ref_order_hints = RefOrderHints::default();
        self.ref_order_hints
            .extend_within(ref_order_hints.iter().copied());
        if let Some(pending) = self.pending_ref_hints.take()
            && let TemporalMotionStorage::Contiguous(cells) = &mut self.storage
        {
            for (cell, hints) in cells.iter_mut().zip(pending) {
                for (list, hint) in hints.into_iter().enumerate() {
                    if hint == u32::MAX {
                        continue;
                    }
                    cell.ref_indices[list] = self
                        .ref_order_hints
                        .iter()
                        .position(|&candidate| candidate == Some(hint))
                        .and_then(|index| u8::try_from(index).ok())
                        .unwrap_or(INVALID_TEMPORAL_REF);
                }
            }
        }
    }

    pub(crate) fn set_band_rows8(&mut self, band_rows8: usize) {
        debug_assert!(matches!(band_rows8, 8 | 16 | 32));
        self.band_rows8 = band_rows8;
    }

    pub(crate) const fn layout(&self) -> MotionFieldLayout {
        MotionFieldLayout {
            width8: self.width8,
            height8: self.height8,
            band_rows8: self.band_rows8,
        }
    }

    pub(crate) fn metadata(&self) -> TemporalMotionFieldMetadata {
        TemporalMotionFieldMetadata {
            is_inter: self.is_inter,
            frame_size: self.frame_size,
            ref_order_hints: self.ref_order_hints,
        }
    }

    pub(crate) fn into_bands(self) -> Vec<TemporalMotionBand> {
        if let TemporalMotionStorage::Bands(bands) = self.storage {
            return bands;
        }
        self.bands()
    }

    /// This field's cells split into bands, without consuming it.
    ///
    /// A band owns a copy of its cells either way, so a publisher that also
    /// keeps the field reads it here instead of cloning the whole field first.
    pub(crate) fn bands(&self) -> Vec<TemporalMotionBand> {
        let layout = self.layout();
        let metadata = self.metadata();
        let stride = layout.width8.saturating_mul(layout.band_rows8).max(1);
        let cells = match &self.storage {
            TemporalMotionStorage::Contiguous(cells) => cells,
            TemporalMotionStorage::Bands(bands) => return bands.clone(),
        };
        cells
            .chunks(stride)
            .enumerate()
            .map(|(index, cells)| TemporalMotionBand {
                layout,
                metadata: metadata.clone(),
                row_base8: index.saturating_mul(layout.band_rows8),
                cells: cells.to_vec(),
            })
            .collect()
    }

    pub(crate) fn from_bands(
        layout: MotionFieldLayout,
        metadata: &TemporalMotionFieldMetadata,
        bands: Vec<TemporalMotionBand>,
    ) -> Option<Self> {
        let cells = bands
            .iter()
            .try_fold(0usize, |cells, band| cells.checked_add(band.cells.len()))?;
        if cells != layout.width8.checked_mul(layout.height8)? {
            return None;
        }
        Some(Self {
            width8: layout.width8,
            height8: layout.height8,
            band_rows8: layout.band_rows8,
            storage: TemporalMotionStorage::Bands(bands),
            pending_ref_hints: None,
            is_inter: metadata.is_inter,
            frame_size: metadata.frame_size,
            ref_order_hints: metadata.ref_order_hints,
        })
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub(crate) fn record_block(&mut self, block: TemporalMotionBlock) {
        let width8 = self.width8;
        let height8 = self.height8;
        let resolved = resolve_block_refs(block.ref_order_hints, &self.ref_order_hints);
        visit_temporal_block_cells(block, width8, height8, |y8, x8, cell, hints| {
            let cell = resolve_temporal_refs(cell, hints, &resolved);
            let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8) else {
                return;
            };
            if let Some(pending) = self.pending_ref_hints.as_mut() {
                pending[index] = hints.map(|hint| hint.unwrap_or(u32::MAX));
            }
            if let TemporalMotionStorage::Contiguous(cells) = &mut self.storage
                && let Some(target) = cells.get_mut(index)
            {
                *target = cell;
            }
        });
    }

    #[cfg(test)]
    fn cell(&self, y8: usize, x8: usize) -> Option<TemporalMotionCell> {
        self.row(y8)?.get(x8).copied()
    }

    #[cfg(test)]
    fn cell_mut(&mut self, y8: usize, x8: usize) -> Option<&mut TemporalMotionCell> {
        let index = temporal_grid_index(self.width8, self.height8, y8, x8)?;
        let TemporalMotionStorage::Contiguous(cells) = &mut self.storage else {
            return None;
        };
        cells.get_mut(index)
    }
}

#[allow(clippy::inline_always)]
#[inline(always)]
fn visit_temporal_block_cells(
    block: TemporalMotionBlock,
    width8: usize,
    height8: usize,
    mut visit: impl FnMut(usize, usize, TemporalMotionCell, [Option<u32>; 2]),
) {
    let Some(row_end) = block.mi_row.checked_add(block.n4h) else {
        return;
    };
    let Some(col_end) = block.mi_col.checked_add(block.n4w) else {
        return;
    };
    let row_end = row_end.min(block.mi_rows);
    let col_end = col_end.min(block.mi_cols);
    if row_end <= block.mi_row || col_end <= block.mi_col {
        return;
    }
    let row8_start = block.mi_row >> 1;
    let col8_start = block.mi_col >> 1;
    let row8_end = row_end.div_ceil(2).min(height8);
    let col8_end = col_end.div_ceil(2).min(width8);
    let swap_lists = temporal_lists_swap(block);
    let derive = |y8: usize, x8: usize| {
        let mut hints = [None; 2];
        let mut mvs = [CompressedTemporalMv::ZERO; 2];
        for list in 0..2 {
            let Some(order_hint) = block.ref_order_hints[list] else {
                continue;
            };
            let mv = block
                .motion
                .mv_at(list, block.mi_row, block.mi_col, y8 * 2, x8 * 2);
            if mv.row.abs() > REFMVS_LIMIT || mv.col.abs() > REFMVS_LIMIT {
                continue;
            }
            hints[list] = Some(order_hint);
            mvs[list] = compress_tmvp_mv(mv);
        }
        if hints[0].is_some() && hints[1].is_none() {
            hints[1] = hints[0];
            mvs[1] = mvs[0];
        } else if hints[1].is_some() && hints[0].is_none() {
            hints[0] = hints[1];
            mvs[0] = mvs[1];
        } else if swap_lists && hints[0].is_some() && hints[1].is_some() {
            hints.swap(0, 1);
            mvs.swap(0, 1);
        }
        (
            TemporalMotionCell {
                mvs,
                ..TemporalMotionCell::default()
            },
            hints,
        )
    };
    let uniform =
        matches!(block.motion, TemporalBlockMotion::Mvs(_)).then(|| derive(row8_start, col8_start));
    for y8 in row8_start..row8_end {
        for x8 in col8_start..col8_end {
            let (cell, hints) = uniform.unwrap_or_else(|| derive(y8, x8));
            visit(y8, x8, cell, hints);
        }
    }
}

/// AV2 § 7.9 list ordering for a block whose two references both survive.
fn temporal_lists_swap(block: TemporalMotionBlock) -> bool {
    let [Some(ref0), Some(ref1)] = block.ref_order_hints else {
        return false;
    };
    let ref0 = i32::try_from(ref0).unwrap_or(i32::MAX);
    let ref1 = i32::try_from(ref1).unwrap_or(i32::MAX);
    let current = i32::try_from(block.current_order_hint).unwrap_or(i32::MAX);
    let ref0_to_current = super::super::get_relative_dist(ref0, current);
    let ref1_to_current = super::super::get_relative_dist(ref1, current);
    let same_side = (ref0_to_current < 0 && ref1_to_current < 0)
        || (ref0_to_current > 0 && ref1_to_current > 0);
    if same_side {
        super::super::get_relative_dist(ref0, ref1) < 0
    } else {
        ref0_to_current > 0 && ref1_to_current < 0
    }
}

fn resolve_block_refs(
    block_hints: [Option<u32>; 2],
    ref_order_hints: &[Option<u32>],
) -> [(Option<u32>, u8); 2] {
    block_hints.map(|hint| {
        let index = hint
            .and_then(|hint| {
                ref_order_hints
                    .iter()
                    .position(|&candidate| candidate == Some(hint))
            })
            .and_then(|index| u8::try_from(index).ok())
            .unwrap_or(INVALID_TEMPORAL_REF);
        (hint, index)
    })
}

fn resolve_temporal_refs(
    mut cell: TemporalMotionCell,
    hints: [Option<u32>; 2],
    resolved: &[(Option<u32>, u8); 2],
) -> TemporalMotionCell {
    for (list, hint) in hints.into_iter().enumerate() {
        if hint.is_none() {
            continue;
        }
        for &(candidate, ref_index) in resolved {
            if candidate == hint {
                cell.ref_indices[list] = ref_index;
                break;
            }
        }
    }
    cell
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemporalMotionBlock {
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    ref_order_hints: [Option<u32>; 2],
    motion: TemporalBlockMotion,
}

impl TemporalMotionBlock {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        mi_row: usize,
        mi_col: usize,
        n4w: usize,
        n4h: usize,
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
        ref_order_hints: [Option<u32>; 2],
        mvs: [Mv; 2],
        warp_params: [Option<[i32; 6]>; 2],
    ) -> Self {
        Self {
            mi_row,
            mi_col,
            n4w,
            n4h,
            mi_rows,
            mi_cols,
            current_order_hint,
            ref_order_hints,
            motion: TemporalBlockMotion::new(mvs, warp_params),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TemporalBlockMotion {
    Mvs([Mv; 2]),
    Warp0 { params: [i32; 6], mv1: Mv },
    Warp1 { mv0: Mv, params: [i32; 6] },
    WarpBoth([[i32; 6]; 2]),
}

impl TemporalBlockMotion {
    fn new(mvs: [Mv; 2], warp_params: [Option<[i32; 6]>; 2]) -> Self {
        match warp_params {
            [None, None] => Self::Mvs(mvs),
            [Some(params), None] => Self::Warp0 {
                params,
                mv1: mvs[1],
            },
            [None, Some(params)] => Self::Warp1 {
                mv0: mvs[0],
                params,
            },
            [Some(first), Some(second)] => Self::WarpBoth([first, second]),
        }
    }

    fn mv_at(
        self,
        list: usize,
        mi_row: usize,
        mi_col: usize,
        cell_row: usize,
        cell_col: usize,
    ) -> Mv {
        let warp = |params| warp_sub_mv_at(params, mi_row, mi_col, cell_row, cell_col);
        match self {
            Self::Mvs(mvs) => mvs[list],
            Self::Warp0 { params, mv1 } => {
                if list == 0 {
                    warp(params)
                } else {
                    mv1
                }
            }
            Self::Warp1 { mv0, params } => {
                if list == 0 {
                    mv0
                } else {
                    warp(params)
                }
            }
            Self::WarpBoth(params) => warp(params[list]),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ProjectedTemporalMotionCell {
    valid: bool,
    mv: Mv,
    ref_offset: i32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ProjectedTemporalMotionField {
    width8: usize,
    height8: usize,
    cells: Vec<ProjectedTemporalMotionCell>,
}

impl ProjectedTemporalMotionField {
    #[cfg(test)]
    fn new(mi_rows: usize, mi_cols: usize) -> Option<Self> {
        let (width8, height8, cells) = allocate_temporal_grid(mi_rows, mi_cols)?;
        Some(Self {
            width8,
            height8,
            cells,
        })
    }

    /// Sizes the field for the frame, taking a roomier buffer from the store
    /// rather than growing this one, so the old one serves the next frame that
    /// wants its shape instead of being freed.
    fn reset(&mut self, mi_rows: usize, mi_cols: usize) -> crate::Result<()> {
        self.width8 = mi_cols.div_ceil(2);
        self.height8 = mi_rows.div_ceil(2);
        let cells = self
            .width8
            .checked_mul(self.height8)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        if self.cells.capacity() < cells {
            let mut roomier = crate::support::buffer_pool::take(cells);
            roomier.clear();
            self.cells = roomier;
        }
        self.cells
            .try_reserve_exact(cells.saturating_sub(self.cells.len()))
            .map_err(|_| {
                crate::DecodeError::from(splot_recon::ReconError::WorkspaceAllocationFailed {
                    plane: splot_recon::PlaneId::Y,
                    context: "inter projected temporal motion field",
                })
            })?;
        self.cells
            .resize(cells, ProjectedTemporalMotionCell::default());
        self.cells.fill(ProjectedTemporalMotionCell::default());
        Ok(())
    }

    fn cell(&self, y8: usize, x8: usize) -> Option<ProjectedTemporalMotionCell> {
        self.cells
            .get(temporal_grid_index(self.width8, self.height8, y8, x8)?)
            .copied()
    }

    fn set(&mut self, y8: usize, x8: usize, mv: Mv, ref_offset: i32, valid: bool) {
        let Some(index) = temporal_grid_index(self.width8, self.height8, y8, x8) else {
            return;
        };
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = ProjectedTemporalMotionCell {
                valid,
                mv,
                ref_offset,
            };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TipReferencePair {
    pub(crate) past_ref: i8,
    pub(crate) future_ref: i8,
    pub(crate) past_offset: i32,
    pub(crate) future_offset: i32,
    pub(crate) ref_offset: i32,
}

#[derive(Debug)]
pub(crate) struct TemporalMvContext {
    current_order_hint: u32,
    ref_order_hints: Vec<Option<u32>>,
    field: ProjectedTemporalMotionField,
    projection_scratch: ProjectedTemporalMotionField,
    average_scratch: ProjectedTemporalMotionField,
    trajectories: Option<TrajectoryState>,
    trajectory_scratch: Option<TrajectoryState>,
    tip: Option<TipReferencePair>,
    banded: Option<Arc<BandedTemporalContext>>,
}

#[derive(Debug)]
struct BandedTemporalContext {
    layout: MotionFieldLayout,
    bands: Vec<CompletionCell<Option<Arc<TemporalBandResult>>>>,
}

#[derive(Debug)]
struct TemporalBandResult {
    row_base8: usize,
    field: Vec<ProjectedTemporalMotionCell>,
    trajectories: Option<OwnedTrajectoryFields>,
}

pub(crate) struct TemporalBandPlan {
    projections: Vec<ScheduledTemporalProjection>,
    config: TemporalProjectionConfig,
    layout: MotionFieldLayout,
    tip: Option<TipReferencePair>,
    tip_mode: bool,
    fill_tip_holes: bool,
}

struct ScheduledTemporalProjection {
    slot: usize,
    source: TemporalProjectionSource,
    source_layout: MotionFieldLayout,
}

impl BandedTemporalContext {
    fn cell(&self, y8: usize, x8: usize) -> Option<ProjectedTemporalMotionCell> {
        if y8 >= self.layout.height8() || x8 >= self.layout.width8() {
            return None;
        }
        let band = self
            .bands
            .get(y8 / self.layout.band_rows8())?
            .get()?
            .as_ref()?;
        let row = y8.checked_sub(band.row_base8)?;
        band.field
            .get(row.checked_mul(self.layout.width8())?.checked_add(x8)?)
            .copied()
    }

    fn trajectory_cell(&self, reference: usize, y8: usize, x8: usize) -> Option<Mv> {
        if y8 >= self.layout.height8() || x8 >= self.layout.width8() {
            return None;
        }
        let band = self
            .bands
            .get(y8 / self.layout.band_rows8())?
            .get()?
            .as_ref()?;
        let row = y8.checked_sub(band.row_base8)?;
        band.trajectories
            .as_ref()?
            .cell(
                reference,
                row.checked_mul(self.layout.width8())?.checked_add(x8)?,
            )
            .filter(|mv| *mv != trajectory::INVALID_TRAJECTORY_MV)
    }

    fn fail(&self) {
        for band in &self.bands {
            let _ = band.set(None);
        }
    }
}

impl TemporalBandPlan {
    pub(crate) fn len(&self) -> usize {
        self.layout.band_count()
    }

    pub(crate) fn rows8(&self, index: usize) -> Range<usize> {
        self.layout.rows8(index)
    }

    /// Collects the reference bands this band's projection reads into `out`.
    ///
    /// Into a caller's buffer, because the list is read once and dropped, and
    /// the callers ask per unit.
    pub(crate) fn requirements(&self, index: usize, out: &mut Vec<(usize, usize)>) {
        out.clear();
        for projection in &self.projections {
            if index >= projection.source_layout.band_count() {
                continue;
            }
            let requirement = (projection.slot, index);
            if !out.contains(&requirement) {
                out.push(requirement);
            }
        }
    }

    pub(crate) fn project(
        &self,
        context: &TemporalMvContext,
        index: usize,
        mut source_band: impl FnMut(usize, usize) -> Option<TemporalMotionBand>,
    ) -> crate::Result<()> {
        let rows = self.rows8(index);
        if rows.is_empty() {
            return Err(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into());
        }
        let row_count = rows.len();
        let width8 = self.layout.width8();
        let cells = width8
            .checked_mul(row_count)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        let mut field_cells = crate::support::buffer_pool::take(cells);
        field_cells.clear();
        field_cells
            .try_reserve_exact(cells.saturating_sub(field_cells.capacity()))
            .map_err(|_| {
                crate::DecodeError::from(splot_recon::ReconError::WorkspaceAllocationFailed {
                    plane: splot_recon::PlaneId::Y,
                    context: "inter temporal motion band",
                })
            })?;
        field_cells.resize(cells, ProjectedTemporalMotionCell::default());
        let mut output = ProjectedFieldBand {
            cells: &mut field_cells,
            width8,
            height8: self.layout.height8(),
            row_base: rows.start,
        };
        let mut trajectories = if self.config.enable_trajectory {
            Some(OwnedTrajectoryBand::new(
                width8,
                self.layout.height8(),
                rows.start,
                row_count,
                context.ref_order_hints.len(),
                self.config.step,
                self.config.unit_size8,
            )?)
        } else {
            None
        };
        for projection in &self.projections {
            if index >= projection.source_layout.band_count() {
                continue;
            }
            let source = source_band(projection.slot, index)
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
            if source.layout != projection.source_layout
                || source.row_base8 != projection.source_layout.rows8(index).start
            {
                return Err(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into());
            }
            let mut trajectory = match trajectories.as_mut() {
                Some(trajectories) => Some(
                    trajectories
                        .as_band()
                        .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?,
                ),
                None => None,
            };
            project_temporal_motion_field(
                &projection.source,
                &source,
                rows.clone(),
                self.config.step,
                self.config.unit_size8,
                trajectory.as_mut(),
                &mut output,
            );
        }
        let mut field = ProjectedTemporalMotionField {
            width8,
            height8: row_count,
            cells: field_cells,
        };
        if self.tip_mode {
            if let Some(references) = self.tip {
                let mut projection = ProjectedTemporalMotionField::default();
                let mut average = ProjectedTemporalMotionField::default();
                prepare_tip_field(
                    &mut field,
                    &mut projection,
                    &mut average,
                    references,
                    self.config.step,
                    self.config.unit_size8,
                    self.fill_tip_holes,
                )?;
            }
        } else {
            fill_temporal_sampling_gaps(&mut field, self.config.step, self.config.unit_size8);
        }
        let result = TemporalBandResult {
            row_base8: rows.start,
            field: core::mem::take(&mut field.cells),
            trajectories: trajectories.map(OwnedTrajectoryBand::finish),
        };
        let banded = context
            .banded
            .as_ref()
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        let band = banded
            .bands
            .get(index)
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        let _ = band.set(Some(Arc::new(result)));
        Ok(())
    }

    pub(crate) fn fail(context: &TemporalMvContext) {
        if let Some(banded) = &context.banded {
            banded.fail();
        }
    }
}

#[derive(Default)]
pub(crate) struct TemporalMvScratch {
    projection: ProjectedTemporalMotionField,
    average: ProjectedTemporalMotionField,
}

#[derive(Clone, Copy)]
pub(crate) struct OrderHintMvContext<'a> {
    current_order_hint: u32,
    ref_order_hints: &'a [Option<u32>],
}

impl OrderHintMvContext<'_> {
    pub(super) fn derive_spatial_mv(
        self,
        dst_ref: i8,
        candidate_ref: i8,
        candidate_mv: Mv,
    ) -> Option<Mv> {
        let dst = usize::try_from(dst_ref).ok()?;
        let candidate = usize::try_from(candidate_ref).ok()?;
        let current = i32::try_from(self.current_order_hint).ok()?;
        let distance = |index: usize| {
            self.ref_order_hints
                .get(index)
                .copied()
                .flatten()
                .and_then(|hint| i32::try_from(hint).ok())
                .map(|hint| super::super::get_relative_dist(current, hint))
        };
        let dst_distance = distance(dst)?;
        let candidate_distance = distance(candidate)?;
        let same_side = (dst_distance > 0 && candidate_distance > 0)
            || (dst_distance < 0 && candidate_distance < 0);
        same_side.then(|| project_mv(candidate_mv, dst_distance.abs(), candidate_distance.abs()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemporalProjectionConfig {
    pub(crate) frame_size: (usize, usize),
    pub(crate) step: usize,
    pub(crate) unit_size8: usize,
    pub(crate) enable_tip: bool,
    pub(crate) enable_trajectory: bool,
    pub(crate) reduced: bool,
}

impl TemporalMvContext {
    pub(crate) fn from_scratch(scratch: TemporalMvScratch) -> Self {
        Self {
            projection_scratch: scratch.projection,
            average_scratch: scratch.average,
            ..Self::empty()
        }
    }

    pub(crate) fn take_scratch(&mut self) -> TemporalMvScratch {
        TemporalMvScratch {
            projection: core::mem::take(&mut self.projection_scratch),
            average: core::mem::take(&mut self.average_scratch),
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            current_order_hint: 0,
            ref_order_hints: Vec::new(),
            field: ProjectedTemporalMotionField {
                width8: 0,
                height8: 0,
                cells: Vec::new(),
            },
            projection_scratch: ProjectedTemporalMotionField {
                width8: 0,
                height8: 0,
                cells: Vec::new(),
            },
            average_scratch: ProjectedTemporalMotionField {
                width8: 0,
                height8: 0,
                cells: Vec::new(),
            },
            trajectories: None,
            trajectory_scratch: None,
            tip: None,
            banded: None,
        }
    }

    #[cfg(test)]
    pub(in crate::prediction::inter) fn with_tip_sample(
        mi_rows: usize,
        mi_cols: usize,
        references: TipReferencePair,
        y8: usize,
        x8: usize,
        mv: Mv,
    ) -> Option<Self> {
        let mut field = ProjectedTemporalMotionField::new(mi_rows, mi_cols)?;
        field.set(y8, x8, mv, references.ref_offset, true);
        Some(Self {
            current_order_hint: 0,
            ref_order_hints: Vec::new(),
            field,
            projection_scratch: ProjectedTemporalMotionField::new(0, 0)?,
            average_scratch: ProjectedTemporalMotionField::new(0, 0)?,
            trajectories: None,
            trajectory_scratch: None,
            tip: Some(references),
            banded: None,
        })
    }

    #[cfg(test)]
    pub(super) fn set_trajectory_sample(
        &mut self,
        reference: usize,
        y8: usize,
        x8: usize,
        mv: Mv,
    ) -> Option<()> {
        if self.trajectories.is_none() {
            let references = self.tip_references()?;
            let count = references.future_ref.max(references.past_ref);
            let count = usize::try_from(count).ok()?.checked_add(1)?;
            let mut fields = Vec::with_capacity(count);
            for _ in 0..count {
                fields.push(TrajectoryMotionField::new(
                    self.field.height8 * 2,
                    self.field.width8 * 2,
                )?);
            }
            self.trajectories = Some(TrajectoryState::from_fields(&fields));
        }
        self.trajectories
            .as_mut()?
            .set_trajectory_cell(reference, y8, x8, mv);
        Some(())
    }

    #[cfg(test)]
    pub(super) fn set_order_hint_context(
        &mut self,
        current_order_hint: u32,
        ref_order_hints: Vec<Option<u32>>,
    ) {
        self.current_order_hint = current_order_hint;
        self.ref_order_hints = ref_order_hints;
    }

    #[cfg(test)]
    pub(crate) fn from_references(
        mi_dimensions: (usize, usize),
        current_order_hint: u32,
        config: TemporalProjectionConfig,
        ref_frame_idx: &[u32],
        ref_valid: &[bool],
        ref_order_hint: &[u32],
        ref_motion_fields: &[Option<Arc<TemporalMotionField>>],
    ) -> Option<Self> {
        let mut context = Self::empty();
        context
            .refresh_from_references(
                mi_dimensions,
                current_order_hint,
                config,
                ref_frame_idx,
                ref_valid,
                ref_order_hint,
                ref_motion_fields,
            )
            .ok()?;
        Some(context)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refresh_from_references(
        &mut self,
        mi_dimensions: (usize, usize),
        current_order_hint: u32,
        config: TemporalProjectionConfig,
        ref_frame_idx: &[u32],
        ref_valid: &[bool],
        ref_order_hint: &[u32],
        ref_motion_fields: &[Option<Arc<TemporalMotionField>>],
    ) -> crate::Result<()> {
        let (mi_rows, mi_cols) = mi_dimensions;
        self.field.reset(mi_rows, mi_cols)?;
        self.ref_order_hints.clear();
        self.ref_order_hints
            .extend(ref_frame_idx.iter().map(|&slot| {
                ref_valid
                    .get(slot as usize)
                    .copied()
                    .filter(|valid| *valid)
                    .and_then(|_| ref_order_hint.get(slot as usize).copied())
                    .filter(|&hint| hint != u32::MAX)
            }));
        let mut ref_motion_metadata =
            crate::reference::buffer::RefSlots::<Option<TemporalMotionFieldMetadata>>::default();
        ref_motion_metadata.extend_within(
            ref_motion_fields
                .iter()
                .map(|field| field.as_ref().map(|field| field.metadata())),
        );
        let mut ref_motion_layouts =
            crate::reference::buffer::RefSlots::<Option<MotionFieldLayout>>::default();
        ref_motion_layouts.extend_within(
            ref_motion_fields
                .iter()
                .map(|field| field.as_ref().map(|field| field.layout())),
        );
        let projections = projection_queue(
            mi_dimensions,
            current_order_hint,
            config,
            ref_frame_idx,
            &self.ref_order_hints,
            &ref_motion_metadata,
            &ref_motion_layouts,
        );
        let mut trajectories = if config.enable_trajectory {
            let mut trajectories = match self
                .trajectories
                .take()
                .or_else(|| self.trajectory_scratch.take())
            {
                Some(trajectories) => trajectories,
                None => TrajectoryState::new(
                    mi_dimensions,
                    self.ref_order_hints.len(),
                    config.step,
                    config.unit_size8,
                )
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?,
            };
            trajectories
                .reset(
                    mi_dimensions,
                    self.ref_order_hints.len(),
                    config.step,
                    config.unit_size8,
                )
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            Some(trajectories)
        } else {
            self.trajectory_scratch = self.trajectories.take();
            None
        };
        // An `Option` is `Default` whatever it holds, so the bounded list is
        // inline even though a projection borrows its source field.
        let mut prepared = PreparedProjections::default();
        for projection in projections.iter().copied() {
            let slot = *ref_frame_idx
                .get(projection.ref_index)
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            let source_order_hint = self
                .ref_order_hints
                .get(projection.ref_index)
                .copied()
                .flatten()
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            let source_field = ref_motion_fields
                .get(slot as usize)
                .and_then(Option::as_deref)
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            let source_metadata = ref_motion_metadata
                .get(slot as usize)
                .and_then(Option::as_ref)
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            let source = TemporalProjectionSource::new(
                source_metadata,
                source_field.layout(),
                source_order_hint,
                current_order_hint,
                projection.ref_index,
                projection.side,
                projection.target_ref,
                &self.ref_order_hints,
            )
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
            prepared
                .push(Some(PreparedTemporalProjection {
                    source,
                    field: source_field,
                }))
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
        }
        run_band_projections(&prepared, config, trajectories.as_mut(), &mut self.field);
        if let Some(trajectories) = trajectories.as_mut() {
            trajectories.fill_gaps();
        }
        self.current_order_hint = current_order_hint;
        self.trajectories = trajectories;
        self.tip = None;
        self.banded = None;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin_banded_refresh(
        &mut self,
        target_layout: MotionFieldLayout,
        current_order_hint: u32,
        config: TemporalProjectionConfig,
        ref_frame_idx: &[u32],
        ref_valid: &[bool],
        ref_order_hint: &[u32],
        ref_motion_metadata: &[Option<TemporalMotionFieldMetadata>],
        ref_motion_layouts: &[Option<MotionFieldLayout>],
        tip: Option<TipReferencePair>,
        tip_mode: bool,
        fill_tip_holes: bool,
    ) -> Option<TemporalBandPlan> {
        let mi_dimensions = (
            target_layout.height8().checked_mul(2)?,
            target_layout.width8().checked_mul(2)?,
        );
        self.field.width8 = target_layout.width8();
        self.field.height8 = target_layout.height8();
        self.field.cells.clear();
        self.ref_order_hints.clear();
        self.ref_order_hints
            .extend(ref_frame_idx.iter().map(|&slot| {
                ref_valid
                    .get(slot as usize)
                    .copied()
                    .filter(|valid| *valid)
                    .and_then(|_| ref_order_hint.get(slot as usize).copied())
                    .filter(|&hint| hint != u32::MAX)
            }));
        let projections = projection_queue(
            mi_dimensions,
            current_order_hint,
            config,
            ref_frame_idx,
            &self.ref_order_hints,
            ref_motion_metadata,
            ref_motion_layouts,
        );
        let mut prepared = Vec::with_capacity(projections.len());
        for projection in projections.iter().copied() {
            let slot = usize::try_from(*ref_frame_idx.get(projection.ref_index)?).ok()?;
            let source_order_hint = self
                .ref_order_hints
                .get(projection.ref_index)
                .copied()
                .flatten()?;
            let metadata = ref_motion_metadata.get(slot)?.as_ref()?;
            let source_layout = ref_motion_layouts.get(slot)?.as_ref().copied()?;
            if source_layout.band_rows8() != target_layout.band_rows8() {
                return None;
            }
            let source = TemporalProjectionSource::new(
                metadata,
                source_layout,
                source_order_hint,
                current_order_hint,
                projection.ref_index,
                projection.side,
                projection.target_ref,
                &self.ref_order_hints,
            )?;
            prepared.push(ScheduledTemporalProjection {
                slot,
                source,
                source_layout,
            });
        }
        self.current_order_hint = current_order_hint;
        self.trajectories = None;
        self.tip = tip;
        self.banded = Some(Arc::new(BandedTemporalContext {
            layout: target_layout,
            bands: (0..target_layout.band_count())
                .map(|_| CompletionCell::new())
                .collect(),
        }));
        Some(TemporalBandPlan {
            projections: prepared,
            config,
            layout: target_layout,
            tip,
            tip_mode,
            fill_tip_holes,
        })
    }

    fn projected_cell(&self, y8: usize, x8: usize) -> Option<ProjectedTemporalMotionCell> {
        if let Some(banded) = &self.banded {
            return banded.cell(y8, x8);
        }
        self.field.cell(y8, x8)
    }

    fn trajectory_cell(&self, reference: usize, y8: usize, x8: usize) -> Option<Mv> {
        if let Some(banded) = &self.banded {
            return banded.trajectory_cell(reference, y8, x8);
        }
        self.trajectories
            .as_ref()?
            .trajectory_cell(reference, y8, x8)
    }

    pub(crate) fn prepare_tip(
        &mut self,
        references: TipReferencePair,
        projection_step: usize,
        superblock_size8: usize,
        fill_holes: bool,
    ) -> crate::Result<()> {
        let projection_step = projection_step.clamp(1, 2);
        let tmvp_unit_size8 = if projection_step == 1 {
            8
        } else {
            superblock_size8.max(1)
        };
        prepare_tip_field(
            &mut self.field,
            &mut self.projection_scratch,
            &mut self.average_scratch,
            references,
            projection_step,
            tmvp_unit_size8,
            fill_holes,
        )?;
        self.tip = Some(references);
        Ok(())
    }

    pub(crate) fn fill_sampling_gaps(&mut self, projection_step: usize, tmvp_unit_size8: usize) {
        fill_temporal_sampling_gaps(&mut self.field, projection_step, tmvp_unit_size8);
    }

    #[cfg(test)]
    pub(crate) fn tip_reference_pair(&self) -> Option<TipReferencePair> {
        tip_reference_pair_from_hints(self.current_order_hint, &self.ref_order_hints)
    }

    pub(crate) fn reference_order_hints(&self) -> &[Option<u32>] {
        &self.ref_order_hints
    }

    pub(crate) fn order_hint_mv_context(&self) -> OrderHintMvContext<'_> {
        OrderHintMvContext {
            current_order_hint: self.current_order_hint,
            ref_order_hints: &self.ref_order_hints,
        }
    }

    pub(crate) fn tip_references(&self) -> Option<TipReferencePair> {
        self.tip
    }

    pub(crate) fn tip_candidate(&self, y8: usize, x8: usize, base_mv: Mv) -> Option<[Mv; 2]> {
        let references = self.tip?;
        let y8 = y8.min(self.field.height8.saturating_sub(1));
        let x8 = x8.min(self.field.width8.saturating_sub(1));
        let cell = self.projected_cell(y8, x8)?;
        let projected = if cell.valid {
            [
                project_mv(cell.mv, references.past_offset, references.ref_offset),
                project_mv(cell.mv, references.future_offset, references.ref_offset),
            ]
        } else {
            [Mv::ZERO; 2]
        };
        Some(projected.map(|mv| Mv {
            row: (mv.row + base_mv.row).clamp(-MV_LIMIT, MV_LIMIT),
            col: (mv.col + base_mv.col).clamp(-MV_LIMIT, MV_LIMIT),
        }))
    }

    pub(super) fn tip_spatial_mvs(
        &self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
        probe: RelativeProbe,
        cell: NeighbourCell,
    ) -> Option<[Mv; 2]> {
        if cell.flags.ref_frame0 != TIP_REF_FRAME || cell.flags.ref_frame1.is_some() {
            return None;
        }
        let (row, col, _) = probe.stack_target(block);
        let (row, col) = (usize::try_from(row).ok()?, usize::try_from(col).ok()?);
        let shift = 1 + usize::from(cell.flags.tip_size_16x16());
        let base_r = usize::try_from(cell.motion.base_r).ok()?;
        let base_c = usize::try_from(cell.motion.base_c).ok()?;
        let row = base_r + ((row.checked_sub(base_r)? >> shift) << shift);
        let col = base_c + ((col.checked_sub(base_c)? >> shift) << shift);
        let base_cell = grid.get(row as i32, col as i32)?;
        self.tip_candidate(row >> 1, col >> 1, base_cell.motion.sub_mv)
    }

    pub(super) fn tip_spatial_single_candidates(
        &self,
        grid: &NeighbourMvGrid,
        block: &MvBlockContext,
        probe: RelativeProbe,
        cell: NeighbourCell,
    ) -> Option<[(i8, Mv); 2]> {
        let refs = self.tip_references()?;
        let mvs = self.tip_spatial_mvs(grid, block, probe, cell)?;
        Some([(refs.past_ref, mvs[0]), (refs.future_ref, mvs[1])])
    }

    pub(super) fn derive_tip_base_mv(&self, references: [i8; 2], mvs: [Mv; 2]) -> Option<Mv> {
        let tip = self.tip_references()?;
        if references != [tip.past_ref, tip.future_ref] {
            return None;
        }
        let linear = Mv {
            row: mvs[0].row.saturating_sub(mvs[1].row),
            col: mvs[0].col.saturating_sub(mvs[1].col),
        };
        let projected = project_mv(linear, tip.past_offset, tip.ref_offset);
        Some(Mv {
            row: mvs[0]
                .row
                .saturating_sub(projected.row)
                .clamp(-MV_LIMIT, MV_LIMIT),
            col: mvs[0]
                .col
                .saturating_sub(projected.col)
                .clamp(-MV_LIMIT, MV_LIMIT),
        })
    }

    pub(super) fn motion_field_mv(&self, ref_frame: i8, y8: usize, x8: usize) -> Option<Mv> {
        let ref_index = usize::try_from(ref_frame).ok()?;
        if let Some(mv) = self.trajectory_cell(ref_index, y8, x8) {
            return Some(mv);
        }
        let dst_hint = usize::try_from(ref_frame)
            .ok()
            .and_then(|idx| self.ref_order_hints.get(idx))
            .copied()
            .flatten()?;
        let cell = self.projected_cell(y8, x8)?;
        if !cell.valid {
            return None;
        }
        let ref_to_dst = super::super::get_relative_dist(
            self.current_order_hint as i32,
            i32::try_from(dst_hint).ok()?,
        );
        Some(project_mv(cell.mv, ref_to_dst, cell.ref_offset))
    }

    pub(super) fn derive_spatial_mv(
        &self,
        dst_ref: i8,
        candidate_ref: i8,
        candidate_mv: Mv,
        y8: usize,
        x8: usize,
    ) -> Option<Mv> {
        let dst = usize::try_from(dst_ref).ok()?;
        let candidate = usize::try_from(candidate_ref).ok()?;
        if let (Some(dst_mv), Some(candidate_trajectory)) = (
            self.trajectory_cell(dst, y8, x8),
            self.trajectory_cell(candidate, y8, x8),
        ) {
            return Some(derive_mv_from_trajectories(
                candidate_mv,
                dst_mv,
                candidate_trajectory,
            ));
        }

        self.order_hint_mv_context()
            .derive_spatial_mv(dst_ref, candidate_ref, candidate_mv)
    }

    pub(super) fn derive_compound_spatial_mvs(
        &self,
        dst_refs: [i8; 2],
        candidate_ref: i8,
        candidate_mv: Mv,
        y8: usize,
        x8: usize,
    ) -> Option<[Mv; 2]> {
        let candidate = usize::try_from(candidate_ref).ok()?;
        let candidate_trajectory = self.trajectory_cell(candidate, y8, x8)?;
        let mut derived = [Mv::ZERO; 2];
        for (index, dst_ref) in dst_refs.into_iter().enumerate() {
            let dst = usize::try_from(dst_ref).ok()?;
            derived[index] = derive_mv_from_trajectories(
                candidate_mv,
                self.trajectory_cell(dst, y8, x8)?,
                candidate_trajectory,
            );
        }
        Some(derived)
    }

    pub(super) fn single_ref_weight(&self, ref_frame: i8) -> Option<u32> {
        let dst_hint = usize::try_from(ref_frame)
            .ok()
            .and_then(|idx| self.ref_order_hints.get(idx))
            .copied()
            .flatten()?;
        let dist = super::super::get_relative_dist(
            self.current_order_hint as i32,
            i32::try_from(dst_hint).ok()?,
        );
        Some(if dist.abs() <= 2 { 2 } else { 1 })
    }
}

pub(crate) fn reference_order_hints(
    ref_frame_idx: &[u32],
    ref_valid: &[bool],
    ref_order_hint: &[u32],
) -> Vec<Option<u32>> {
    ref_frame_idx
        .iter()
        .map(|&slot| {
            ref_valid
                .get(slot as usize)
                .copied()
                .filter(|valid| *valid)
                .and_then(|_| ref_order_hint.get(slot as usize).copied())
                .filter(|&hint| hint != u32::MAX)
        })
        .collect()
}

pub(crate) fn tip_reference_pair_from_hints(
    current_order_hint: u32,
    ref_order_hints: &[Option<u32>],
) -> Option<TipReferencePair> {
    let current = i32::try_from(current_order_hint).ok()?;
    let mut sorted_buffer = [(0usize, 0i32); MAX_SORTED_REFS];
    let sorted_len = sorted_reference_hints(ref_order_hints, &mut sorted_buffer);
    let sorted = &sorted_buffer[..sorted_len];
    let past_index = sorted
        .iter()
        .rposition(|&(_, hint)| super::super::get_relative_dist(hint, current) < 0)?;
    let has_future = sorted
        .iter()
        .any(|&(_, hint)| super::super::get_relative_dist(hint, current) > 0);
    let future_index = if has_future {
        past_index.checked_add(1)?
    } else {
        past_index.checked_sub(1)?
    };
    let &(past_ref, past_hint) = sorted.get(past_index)?;
    let &(future_ref, future_hint) = sorted.get(future_index)?;
    let past_offset = super::super::get_relative_dist(current, past_hint);
    let future_offset = super::super::get_relative_dist(current, future_hint);
    let ref_offset = if future_offset < 0 {
        super::super::get_relative_dist(future_hint, past_hint)
    } else {
        super::super::get_relative_dist(past_hint, future_hint)
    };
    Some(TipReferencePair {
        past_ref: i8::try_from(past_ref).ok()?,
        future_ref: i8::try_from(future_ref).ok()?,
        past_offset,
        future_offset,
        ref_offset: ref_offset.min(MAX_FRAME_DISTANCE),
    })
}

/// Reference slots AV2 § 6.8.2 `NUM_REF_FRAMES` allows, so the sort fits a
/// caller's array and needs no heap.
const MAX_SORTED_REFS: usize = 8;

fn sorted_reference_hints(
    ref_order_hints: &[Option<u32>],
    out: &mut [(usize, i32); MAX_SORTED_REFS],
) -> usize {
    let mut len = 0;
    for (index, hint) in ref_order_hints.iter().copied().enumerate() {
        if len == MAX_SORTED_REFS {
            break;
        }
        let Some(hint) = hint.and_then(|hint| i32::try_from(hint).ok()) else {
            continue;
        };
        out[len] = (index, hint);
        len = len.saturating_add(1);
    }
    for i in 0..len {
        for j in i + 1..len {
            if super::super::get_relative_dist(out[j].1, out[i].1) < 0 {
                out.swap(i, j);
            }
        }
    }
    len
}

fn prepare_tip_field(
    source: &mut ProjectedTemporalMotionField,
    projection: &mut ProjectedTemporalMotionField,
    average: &mut ProjectedTemporalMotionField,
    references: TipReferencePair,
    projection_step: usize,
    tmvp_unit_size8: usize,
    fill_holes: bool,
) -> crate::Result<()> {
    let mi_rows = source
        .height8
        .checked_mul(2)
        .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
    let mi_cols = source
        .width8
        .checked_mul(2)
        .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
    projection.reset(mi_rows, mi_cols)?;
    debug_assert_eq!(projection.width8, source.width8);
    debug_assert_eq!(projection.height8, source.height8);
    for y8 in (0..projection.height8).step_by(projection_step) {
        let row_start = y8 * projection.width8;
        for x8 in (0..projection.width8).step_by(projection_step) {
            let index = row_start + x8;
            let source = source.cells[index];
            let projected = source.valid.then(|| {
                let mv = project_tmvp_mv(source.mv, references.ref_offset, source.ref_offset);
                Mv {
                    row: mv.row.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                    col: mv.col.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                }
            });
            projection.cells[index] = ProjectedTemporalMotionCell {
                valid: projected.is_some(),
                mv: projected.unwrap_or(Mv::ZERO),
                ref_offset: references.ref_offset,
            };
        }
    }
    if fill_holes {
        fill_tip_holes(projection, projection_step, tmvp_unit_size8);
        average_tip_motion(projection, average, projection_step, tmvp_unit_size8)?;
        std::mem::swap(projection, average);
    }
    fill_temporal_sampling_gaps(projection, projection_step, tmvp_unit_size8);
    std::mem::swap(source, projection);
    Ok(())
}

fn fill_tip_holes(field: &mut ProjectedTemporalMotionField, step: usize, superblock_size8: usize) {
    let width8 = field.width8;
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let index = y8 * width8 + x8;
                    let source = field.cells[index];
                    let mut fill = |destination: usize| {
                        if !field.cells[destination].valid {
                            field.cells[destination] = source;
                        }
                    };
                    if y8 >= block_y + step {
                        fill(index - step * width8);
                    }
                    if x8 >= block_x + step {
                        fill(index - step);
                    }
                    if y8 + step < end_y {
                        fill(index + step * width8);
                    }
                    if x8 + step < end_x {
                        fill(index + step);
                    }
                }
            }
        }
    }
}

/// Averages the § 7.10.4 TIP motion of every sampled cell into `averaged`.
///
/// The destination is reset rather than resized: above a projection step of one
/// this writes only the sampled cells, so a reused scratch would carry another
/// frame's motion in the cells between them, and
/// [`fill_temporal_sampling_gaps`] overwrites those only where the sampled
/// anchor is valid.
fn average_tip_motion(
    field: &ProjectedTemporalMotionField,
    averaged: &mut ProjectedTemporalMotionField,
    step: usize,
    superblock_size8: usize,
) -> crate::Result<()> {
    let mi_rows = field
        .height8
        .checked_mul(2)
        .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
    let mi_cols = field
        .width8
        .checked_mul(2)
        .ok_or(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState)?;
    averaged.reset(mi_rows, mi_cols)?;
    let width8 = field.width8;
    for block_y in (0..field.height8).step_by(superblock_size8) {
        for block_x in (0..field.width8).step_by(superblock_size8) {
            let end_y = (block_y + superblock_size8).min(field.height8);
            let end_x = (block_x + superblock_size8).min(field.width8);
            for y8 in (block_y..end_y).step_by(step) {
                for x8 in (block_x..end_x).step_by(step) {
                    let mut sum = Mv::ZERO;
                    let mut count = 0usize;
                    let index = y8 * width8 + x8;
                    let mut add = |candidate: usize| {
                        let cell = field.cells[candidate];
                        if cell.valid {
                            sum.row += cell.mv.row;
                            sum.col += cell.mv.col;
                            count += 1;
                        }
                    };
                    add(index);
                    if y8 >= block_y + step {
                        add(index - step * width8);
                    }
                    if x8 >= block_x + step {
                        add(index - step);
                    }
                    if y8 + step < end_y {
                        add(index + step * width8);
                    }
                    if x8 + step < end_x {
                        add(index + step);
                    }
                    averaged.cells[index] = if count == 0 {
                        ProjectedTemporalMotionCell::default()
                    } else {
                        ProjectedTemporalMotionCell {
                            valid: true,
                            mv: Mv {
                                row: divide_tip_average(sum.row, count),
                                col: divide_tip_average(sum.col, count),
                            },
                            ref_offset: field.cells[index].ref_offset,
                        }
                    };
                }
            }
        }
    }
    Ok(())
}

#[doc = "AV2 § 7.10.4 Weight_Div_Mult motion-vector average."]
fn divide_tip_average(value: i32, count: usize) -> i32 {
    const WEIGHTS: [i32; 6] = [0, 65_536, 32_768, 21_845, 16_384, 13_107];
    round2_signed(i64::from(value) * i64::from(WEIGHTS[count]), 16) as i32
}

fn fill_temporal_sampling_gaps(
    field: &mut ProjectedTemporalMotionField,
    step: usize,
    tmvp_unit_size8: usize,
) {
    if step != 2 {
        return;
    }
    let tmvp_unit_size8 = tmvp_unit_size8.max(1);
    for y8 in (0..field.height8).step_by(2) {
        for x8 in (0..field.width8).step_by(2) {
            for (dy, dx) in [(0usize, 1usize), (1, 0), (1, 1)] {
                fill_temporal_sampling_gap(field, y8, x8, dy, dx, tmvp_unit_size8);
            }
        }
    }
}

#[doc = "AV2 § 7.10.5 fill_tpl and calc_avg motion-vector gap fill."]
fn fill_temporal_sampling_gap(
    field: &mut ProjectedTemporalMotionField,
    y8: usize,
    x8: usize,
    dy: usize,
    dx: usize,
    tmvp_unit_size8: usize,
) {
    let Some(anchor) = field.cell(y8, x8).filter(|cell| cell.valid) else {
        return;
    };
    if y8 + dy >= field.height8 || x8 + dx >= field.width8 {
        return;
    }
    let mut sum = Mv::ZERO;
    let mut count = 0i32;
    for candidate_y in 0..=1 {
        for candidate_x in 0..=1 {
            if dy < candidate_y || dx < candidate_x {
                continue;
            }
            let source_y = y8 + 2 * candidate_y;
            let source_x = x8 + 2 * candidate_x;
            if source_y / tmvp_unit_size8 != y8 / tmvp_unit_size8
                || source_x / tmvp_unit_size8 != x8 / tmvp_unit_size8
            {
                continue;
            }
            let Some(source) = field.cell(source_y, source_x).filter(|cell| cell.valid) else {
                continue;
            };
            let mv = if candidate_y == 0 && candidate_x == 0 {
                source.mv
            } else {
                let mv = project_mv(source.mv, anchor.ref_offset, source.ref_offset);
                Mv {
                    row: mv.row.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                    col: mv.col.clamp(-REFMVS_LIMIT, REFMVS_LIMIT),
                }
            };
            sum.row += mv.row;
            sum.col += mv.col;
            count += 1;
        }
    }
    let average = |value: i32| match count {
        1 => value,
        2 => round2_signed_i32(value, 1),
        3 => round2_signed_i32(value * 85, 8),
        _ => round2_signed_i32(value, 2),
    };
    field.set(
        y8 + dy,
        x8 + dx,
        Mv {
            row: average(sum.row),
            col: average(sum.col),
        },
        anchor.ref_offset,
        true,
    );
}

/// Height of the TMVP unit row bands that [`project_temporal_motion_field`]
/// keeps its writes inside.
///
/// A band is one AV2 § 7.9.8 TMVP unit tall, so bands stay aligned to the unit
/// grid that check-block-position measures against. A unit size that does not
/// cover the sampling step cannot bound a band, and yields the whole field.
fn projection_band_rows(height8: usize, config: TemporalProjectionConfig) -> usize {
    let step = config.step.clamp(1, 2);
    let unit = config.unit_size8.max(1);
    if unit.is_multiple_of(step) {
        unit
    } else {
        height8
    }
    .max(1)
}

/// Runs every queued projection over each TMVP unit row band.
///
/// Bands partition the writes, and each band replays the queue in its original
/// order, so the result matches the whole-field scan whatever order the bands
/// run in. An installed pool runs the same band tasks at every worker count;
/// direct callers outside the owned pool replay those bands in place.
fn run_band_projections(
    prepared: &[Option<PreparedTemporalProjection<'_>>],
    config: TemporalProjectionConfig,
    trajectories: Option<&mut TrajectoryState>,
    field: &mut ProjectedTemporalMotionField,
) {
    let band_rows = projection_band_rows(field.height8, config);
    let run = |band: &mut ProjectedFieldBand<'_>, mut rows: Option<&mut TrajectoryBand<'_>>| {
        let rows8 = band.row_base..band.row_base + band_rows;
        for prepared in prepared.iter().flatten() {
            project_temporal_motion_field(
                &prepared.source,
                prepared.field,
                rows8.clone(),
                config.step,
                config.unit_size8,
                rows.as_deref_mut(),
                band,
            );
        }
    };
    let mut trajectory_bands = trajectories.and_then(|state| state.bands(band_rows));
    let mut field_bands = field.bands(band_rows);
    let mut trajectory_slots = trajectory_bands
        .as_deref_mut()
        .map_or_else(Vec::new, |bands| bands.iter_mut().map(Some).collect());
    let scheduled = if splot_parallel::current_pool_width() <= 1 {
        for (index, band) in field_bands.iter_mut().enumerate() {
            let rows = trajectory_slots.get_mut(index).and_then(Option::take);
            run(band, rows);
        }
        Ok(())
    } else {
        splot_parallel::ready_task_scope(|scope| {
            for (index, band) in field_bands.iter_mut().enumerate() {
                let rows = trajectory_slots.get_mut(index).and_then(Option::take);
                let run = &run;
                scope.spawn(move |_| run(band, rows));
            }
        })
    };
    if scheduled.is_err() {
        for (index, band) in field_bands.iter_mut().enumerate() {
            run(
                band,
                trajectory_bands
                    .as_mut()
                    .and_then(|bands| bands.get_mut(index)),
            );
        }
    }
}

/// Whole-field projection of one source, for direct unit tests.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn project_whole_temporal_motion_field(
    source: &TemporalMotionField,
    source_order_hint: u32,
    current_order_hint: u32,
    projection_step: usize,
    tmvp_unit_size8: usize,
    source_ref: usize,
    side: usize,
    target_ref: Option<usize>,
    ref_order_hints: &[Option<u32>],
    trajectories: Option<&mut TrajectoryState>,
    output: &mut ProjectedTemporalMotionField,
) {
    let config = TemporalProjectionConfig {
        frame_size: (0, 0),
        step: projection_step,
        unit_size8: tmvp_unit_size8,
        enable_tip: false,
        enable_trajectory: trajectories.is_some(),
        reduced: false,
    };
    let prepared = TemporalProjectionSource::new(
        &source.metadata(),
        source.layout(),
        source_order_hint,
        current_order_hint,
        source_ref,
        side,
        target_ref,
        ref_order_hints,
    );
    let prepared = prepared.map(|source_info| PreparedTemporalProjection {
        source: source_info,
        field: source,
    });
    run_band_projections(
        core::slice::from_ref(&prepared),
        config,
        trajectories,
        output,
    );
}

/// One unit-aligned row band of a projected motion field.
struct ProjectedFieldBand<'a> {
    cells: &'a mut [ProjectedTemporalMotionCell],
    width8: usize,
    height8: usize,
    row_base: usize,
}

impl ProjectedTemporalMotionField {
    fn bands(&mut self, band_rows: usize) -> Vec<ProjectedFieldBand<'_>> {
        let (width8, height8) = (self.width8, self.height8);
        self.cells
            .chunks_mut(band_rows.saturating_mul(width8).max(1))
            .enumerate()
            .map(|(index, cells)| ProjectedFieldBand {
                cells,
                width8,
                height8,
                row_base: index * band_rows,
            })
            .collect()
    }
}

/// One queued AV2 § 7.9.3 motion-field projection with its per-source preamble
/// resolved once, so the scan can be replayed band by band.
/// One frame's prepared projections, bounded like the queue that names them.
type PreparedProjections<'a> = splot_core::tile::InlineVec<
    Option<PreparedTemporalProjection<'a>>,
    { selection::MFMV_STACK_SIZE },
>;

struct PreparedTemporalProjection<'a> {
    source: TemporalProjectionSource,
    field: &'a TemporalMotionField,
}

struct TemporalProjectionSource {
    source_width8: usize,
    source_height8: usize,
    source_ref: usize,
    side: usize,
    target_ref: Option<usize>,
    target_order_hint: Option<u32>,
    source_to_current: i32,
    /// One entry per reference slot, so it sits inline rather than in a vector
    /// built for every projection of every frame.
    target_cache: [Option<(u32, Option<usize>, i32)>; MAX_SORTED_REFS],
}

impl TemporalProjectionSource {
    #[allow(clippy::too_many_arguments)]
    fn new(
        source: &TemporalMotionFieldMetadata,
        layout: MotionFieldLayout,
        source_order_hint: u32,
        current_order_hint: u32,
        source_ref: usize,
        side: usize,
        target_ref: Option<usize>,
        ref_order_hints: &[Option<u32>],
    ) -> Option<Self> {
        if layout.width8() == 0 {
            return None;
        }
        let source_hint = i32::try_from(source_order_hint).unwrap_or(i32::MAX);
        let current_hint = i32::try_from(current_order_hint).unwrap_or(i32::MAX);
        Some(Self {
            source_width8: layout.width8(),
            source_height8: layout.height8(),
            source_ref,
            side,
            target_ref,
            target_order_hint: target_ref
                .and_then(|target| ref_order_hints.get(target).copied().flatten()),
            source_to_current: super::super::get_relative_dist(source_hint, current_hint),
            target_cache: {
                let mut cache = [const { None }; MAX_SORTED_REFS];
                for (slot, &hint) in cache.iter_mut().zip(source.ref_order_hints.iter()) {
                    *slot = hint.map(|hint| {
                        let target_hint = i32::try_from(hint).unwrap_or(i32::MAX);
                        (
                            hint,
                            mapped_reference(source_order_hint, hint, ref_order_hints),
                            super::super::get_relative_dist(source_hint, target_hint),
                        )
                    });
                }
                cache
            },
        })
    }
}

/// Projects `rows` of one source motion field into the current frame's field.
///
/// Every write this makes — projected cell, trajectory field, trajectory
/// position — lands in the TMVP unit row the scanned cell belongs to: AV2
/// § 7.9.8 admits a sample only when the source row sits inside the projected
/// position's unit, and its vertical bound carries no offset. A caller may
/// therefore replay disjoint unit-aligned row bands in any order and observe
/// the whole-field result.
fn project_temporal_motion_field(
    prepared: &TemporalProjectionSource,
    source: &impl TemporalMotionRows,
    rows: Range<usize>,
    projection_step: usize,
    tmvp_unit_size8: usize,
    mut trajectories: Option<&mut TrajectoryBand<'_>>,
    output: &mut ProjectedFieldBand<'_>,
) {
    let TemporalProjectionSource {
        source_width8,
        source_height8,
        source_ref,
        side,
        target_ref,
        target_order_hint,
        source_to_current,
        target_cache,
    } = prepared;
    let (source_ref, side, target_ref) = (*source_ref, *side, *target_ref);
    let (target_order_hint, source_to_current) = (*target_order_hint, *source_to_current);
    let projection_step = projection_step.clamp(1, 2);
    debug_assert_eq!((*source_width8, *source_height8), source.dimensions8());
    let rows = rows.start..rows.end.min(*source_height8);
    let mut last_target = None;
    for y8 in rows.step_by(projection_step) {
        let Some(row) = source.row(y8) else {
            continue;
        };
        for (x8, cell) in row.iter().copied().enumerate().step_by(projection_step) {
            let list = side;
            let ref_index = usize::from(cell.ref_indices[list]);
            let (target_hint, end_ref, mut ref_offset, projection_factor) = match last_target {
                Some((cached_ref, target_hint, end_ref, ref_offset, projection_factor))
                    if cached_ref == ref_index =>
                {
                    (target_hint, end_ref, ref_offset, projection_factor)
                }
                _ => {
                    let Some(&(target_hint, end_ref, ref_offset)) =
                        target_cache.get(ref_index).and_then(Option::as_ref)
                    else {
                        continue;
                    };
                    let projection_factor =
                        tmvp_projection_factor(source_to_current, ref_offset, side);
                    last_target = Some((
                        ref_index,
                        target_hint,
                        end_ref,
                        ref_offset,
                        projection_factor,
                    ));
                    (target_hint, end_ref, ref_offset, projection_factor)
                }
            };
            let saved_target_hint = target_hint;
            let mut mv = uncompress_tmvp_mv(cell.mvs[list]);
            let trajectory_target_position = trajectories.as_deref_mut().and_then(|trajectories| {
                trajectories.check_intersection(source_ref, end_ref, y8, x8, mv)
            });
            let Some(projection_factor) = projection_factor else {
                continue;
            };
            let trajectory_mv = mv;
            let trajectory_ref_offset = ref_offset;
            if ref_offset < 0 {
                ref_offset = -ref_offset;
                mv = Mv {
                    row: -mv.row,
                    col: -mv.col,
                };
            }
            let projected_to_current =
                project_tmvp_mv_with_factor(mv, source_to_current, ref_offset, projection_factor);
            let Some((pos_y8, pos_x8)) = sampled_temporal_position(
                y8,
                x8,
                projected_to_current,
                projection_step,
                tmvp_unit_size8,
                (output.width8, output.height8),
            ) else {
                continue;
            };
            if let Some(trajectories) = trajectories.as_deref_mut()
                && trajectories.admits_projection(
                    end_ref,
                    target_ref,
                    (pos_y8, pos_x8),
                    trajectory_ref_offset.abs(),
                )
            {
                trajectories.observe_projection_at(
                    source_ref,
                    end_ref,
                    target_ref,
                    y8,
                    x8,
                    trajectory_mv,
                    projected_to_current,
                    (pos_y8, pos_x8),
                    trajectory_target_position,
                    source_to_current,
                    trajectory_ref_offset.abs(),
                    side == 1,
                );
            }
            let Some(output_cell) = pos_y8
                .checked_sub(output.row_base)
                .and_then(|row| output.cells.get_mut(row * output.width8 + pos_x8))
            else {
                continue;
            };
            let replace = !output_cell.valid
                || (target_order_hint == Some(saved_target_hint)
                    && output_cell.ref_offset != ref_offset);
            if replace {
                *output_cell = ProjectedTemporalMotionCell {
                    valid: true,
                    mv,
                    ref_offset,
                };
            }
        }
    }
}

trait TemporalMotionRows {
    fn dimensions8(&self) -> (usize, usize);
    fn row(&self, y8: usize) -> Option<&[TemporalMotionCell]>;
}

impl TemporalMotionRows for TemporalMotionField {
    fn dimensions8(&self) -> (usize, usize) {
        (self.width8, self.height8)
    }

    fn row(&self, y8: usize) -> Option<&[TemporalMotionCell]> {
        match &self.storage {
            TemporalMotionStorage::Contiguous(cells) => {
                let start = y8.checked_mul(self.width8)?;
                cells.get(start..start.checked_add(self.width8)?)
            }
            TemporalMotionStorage::Bands(bands) => bands.get(y8 / self.band_rows8.max(1))?.row(y8),
        }
    }
}

impl TemporalMotionRows for TemporalMotionBand {
    fn dimensions8(&self) -> (usize, usize) {
        (self.layout.width8(), self.layout.height8())
    }

    fn row(&self, y8: usize) -> Option<&[TemporalMotionCell]> {
        TemporalMotionBand::row(self, y8)
    }
}

fn mapped_reference(
    source_order_hint: u32,
    target_order_hint: u32,
    ref_order_hints: &[Option<u32>],
) -> Option<usize> {
    ref_order_hints.iter().position(|hint| {
        hint.is_some_and(|hint| {
            let hint = i32::try_from(hint).unwrap_or(i32::MAX);
            super::super::get_relative_dist(
                hint,
                i32::try_from(target_order_hint).unwrap_or(i32::MAX),
            ) == 0
                && super::super::get_relative_dist(
                    hint,
                    i32::try_from(source_order_hint).unwrap_or(i32::MAX),
                ) != 0
        })
    })
}

fn sampled_temporal_position(
    y8: usize,
    x8: usize,
    projected_mv: Mv,
    projection_step: usize,
    tmvp_unit_size8: usize,
    (width8, height8): (usize, usize),
) -> Option<(usize, usize)> {
    let projected_y8 = project_no_constraint(y8, projected_mv.row, height8)?;
    let projected_x8 = project_no_constraint(x8, projected_mv.col, width8)?;
    debug_assert!(projection_step.is_power_of_two());
    let step_mask = projection_step - 1;
    let projected_y8 = projected_y8 & !step_mask;
    let projected_x8 = projected_x8 & !step_mask;
    tmvp_position_is_near(
        y8,
        x8,
        projected_y8,
        projected_x8,
        projection_step,
        tmvp_unit_size8,
    )
    .then_some((projected_y8, projected_x8))
}

fn tmvp_position_is_near(
    source_y8: usize,
    source_x8: usize,
    projected_y8: usize,
    projected_x8: usize,
    projection_step: usize,
    tmvp_unit_size8: usize,
) -> bool {
    let tmvp_unit_size8 = tmvp_unit_size8.max(1);
    debug_assert!(tmvp_unit_size8.is_power_of_two());
    let unit_mask = tmvp_unit_size8 - 1;
    let base_y8 = projected_y8 & !unit_mask;
    let base_x8 = projected_x8 & !unit_mask;
    let horizontal_offset8 = if projection_step > 1 {
        tmvp_unit_size8
    } else {
        tmvp_unit_size8 / 2
    };
    source_y8 >= base_y8
        && source_y8 < base_y8.saturating_add(tmvp_unit_size8)
        && source_x8 >= base_x8.saturating_sub(horizontal_offset8)
        && source_x8
            < base_x8
                .saturating_add(tmvp_unit_size8)
                .saturating_add(horizontal_offset8)
}

fn project_no_constraint(v8: usize, delta: i32, max8: usize) -> Option<usize> {
    let offset8 = delta / (1 << (3 + 1 + 2));
    let projected = i32::try_from(v8).ok()?.checked_add(offset8)?;
    usize::try_from(projected)
        .ok()
        .filter(|&projected| projected < max8)
}

fn project_mv(mv: Mv, numerator: i32, denominator: i32) -> Mv {
    let denominator = denominator.clamp(0, MAX_FRAME_DISTANCE) as usize;
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let scale = DIV_MULT[denominator];
    let bound = (1 << 16) - 1;
    let row = round2_signed(
        i64::from(mv.row) * i64::from(numerator) * i64::from(scale),
        14,
    )
    .clamp(-bound, bound) as i32;
    let col = round2_signed(
        i64::from(mv.col) * i64::from(numerator) * i64::from(scale),
        14,
    )
    .clamp(-bound, bound) as i32;
    Mv { row, col }
}

fn tmvp_projection_factor(numerator: i32, ref_offset: i32, side: usize) -> Option<i32> {
    if ref_offset.abs() > MAX_FRAME_DISTANCE
        || (side == 0 && ref_offset < 0)
        || (side == 1 && ref_offset > 0)
        || numerator.abs() > MAX_FRAME_DISTANCE
    {
        return None;
    }
    let denominator = ref_offset.unsigned_abs() as usize;
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    DIV_MULT
        .get(denominator)
        .copied()
        .map(|scale| numerator * scale)
}

fn project_tmvp_mv_with_factor(mv: Mv, numerator: i32, denominator: i32, factor: i32) -> Mv {
    if mv.row.unsigned_abs() > REFMVS_LIMIT as u32 || mv.col.unsigned_abs() > REFMVS_LIMIT as u32 {
        return project_mv(mv, numerator, denominator);
    }
    let project = |component: i32| {
        let scaled = component * factor;
        let magnitude = scaled.abs();
        let rounded = (magnitude + (1 << 13)) >> 14;
        if scaled < 0 { -rounded } else { rounded }.clamp(-MV_LIMIT, MV_LIMIT)
    };
    Mv {
        row: project(mv.row),
        col: project(mv.col),
    }
}

fn project_tmvp_mv(mv: Mv, numerator: i32, denominator: i32) -> Mv {
    let denominator = denominator.clamp(0, MAX_FRAME_DISTANCE);
    let numerator = numerator.clamp(-MAX_FRAME_DISTANCE, MAX_FRAME_DISTANCE);
    let factor = numerator * DIV_MULT[denominator as usize];
    project_tmvp_mv_with_factor(mv, numerator, denominator, factor)
}

fn derive_mv_from_trajectories(candidate: Mv, dst: Mv, candidate_trajectory: Mv) -> Mv {
    Mv {
        row: candidate
            .row
            .saturating_add(dst.row)
            .saturating_sub(candidate_trajectory.row)
            .clamp(-MV_LIMIT, MV_LIMIT),
        col: candidate
            .col
            .saturating_add(dst.col)
            .saturating_sub(candidate_trajectory.col)
            .clamp(-MV_LIMIT, MV_LIMIT),
    }
}

fn compress_tmvp_mv(mv: Mv) -> CompressedTemporalMv {
    CompressedTemporalMv {
        row: compress_tmvp_component(mv.row) as i8,
        col: compress_tmvp_component(mv.col) as i8,
    }
}

fn uncompress_tmvp_mv(mv: CompressedTemporalMv) -> Mv {
    Mv {
        row: uncompress_tmvp_component(i32::from(mv.row)),
        col: uncompress_tmvp_component(i32::from(mv.col)),
    }
}

fn compress_tmvp_component(value: i32) -> i32 {
    let abs_value = value.unsigned_abs();
    let msb = 31u32.saturating_sub(abs_value.leading_zeros());
    let step_log2 = msb.saturating_sub(4);
    let compressed = ((abs_value >> step_log2) + (step_log2 << 4)) as i32;
    if value < 0 { -compressed } else { compressed }
}

fn uncompress_tmvp_component(value: i32) -> i32 {
    let abs_value = value.unsigned_abs();
    let step_log2 = ((abs_value >> 4) as i32 - 1).max(0) as u32;
    let uncompressed = ((abs_value - (step_log2 << 4)) << step_log2) as i32;
    if value < 0 {
        -uncompressed
    } else {
        uncompressed
    }
}

#[cfg(test)]
mod tests;
