// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{DeblockingFilterParams, QuantizationParams, TileInfo};
use splot_core::tables::conversion::{
    Q_FIRST, Q_THRESH_MULTS, SIDE_THRESHOLDS, TX_HEIGHT, TX_WIDTH, W_MULT,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DeblockEdge, DeblockFilterChoice, DeblockSampleFilter,
    PixelFormat, PlaneId, ReconSample, deblock_adaptive_filter_strength, deblock_filter_choice,
    deblock_filter_choice_strided, deblock_filter_max_width, deblock_horizontal_edges,
    deblock_sample_filter, deblock_sample_filter_strided, deblock_sample_filter_strided_4,
    deblock_side_threshold_index, deblock_vertical_edges, max_quantizer_index,
};
use std::{cell::Cell, num::NonZeroUsize, ops::Range};

mod edges;
mod grid;

use edges::{PassEdges, PassGeometry};
pub(crate) use grid::DeblockGridStorage;

#[cfg(test)]
use grid::MiCell;
use grid::{ChromaMiGridStorage, MiGrid, MiGridStorage};

use crate::pipeline::frame_progress::FrontierRows;

const MI_SIZE: usize = 4;

const SB_SIZE: usize = 64;

/// Mode-info positions and transform sizes are stored narrow: a frame holds
/// one record per transform block, so the record size is the frame's
/// deblock-record footprint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeblockPredictionUnit {
    pub(crate) base_r: u32,
    pub(crate) base_c: u32,
    pub(crate) default_sub_pu_tx: u8,
}

impl DeblockPredictionUnit {
    pub(crate) const fn new(base_r: usize, base_c: usize, default_sub_pu_tx: usize) -> Self {
        Self {
            base_r: base_r as u32,
            base_c: base_c as u32,
            default_sub_pu_tx: default_sub_pu_tx as u8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeblockSubPuSize {
    pub(crate) width: u16,
    pub(crate) height: u16,
}

impl DeblockSubPuSize {
    pub(crate) const fn new(width: usize, height: usize) -> Self {
        Self {
            width: width as u16,
            height: height as u16,
        }
    }

    pub(crate) const fn square(size: usize) -> Self {
        Self::new(size, size)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DeblockBlock {
    pub(crate) r: u32,
    pub(crate) c: u32,
    pub(crate) luma_prediction: DeblockPredictionUnit,
    pub(crate) chroma_prediction: DeblockPredictionUnit,
    pub(crate) chroma_base_r: u32,
    pub(crate) chroma_base_c: u32,
    pub(crate) n4w: u32,
    pub(crate) n4h: u32,
    pub(crate) luma_tx: u8,
    pub(crate) chroma_tx: Option<u8>,
    pub(crate) sub_pu_size: Option<DeblockSubPuSize>,
    pub(crate) chroma_transform_only: bool,
    pub(crate) qindex: u32,
    pub(crate) skip: bool,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug)]
struct ChromaDeblockBlock {
    block: DeblockBlock,
    planes: u8,
}

#[derive(Clone)]
pub(crate) struct ChromaDeblockRecords {
    blocks: Vec<ChromaDeblockBlock>,
    /// Records that name only one of U and V.
    single: usize,
}

impl Default for ChromaDeblockRecords {
    fn default() -> Self {
        Self::new()
    }
}

impl ChromaDeblockRecords {
    const U: u8 = 1;
    const V: u8 = 2;

    pub(crate) const fn new() -> Self {
        Self {
            blocks: Vec::new(),
            single: 0,
        }
    }

    pub(crate) fn push(&mut self, plane: usize, block: DeblockBlock) {
        let planes = [Self::U, Self::V][plane];
        self.single += 1;
        self.blocks.push(ChromaDeblockBlock { block, planes });
    }

    pub(crate) fn push_both(&mut self, block: DeblockBlock) {
        self.blocks.push(ChromaDeblockBlock {
            block,
            planes: Self::U | Self::V,
        });
    }

    pub(crate) fn append(&mut self, other: &mut Self) {
        self.blocks.append(&mut other.blocks);
        self.single += core::mem::take(&mut other.single);
    }

    /// Copies `other`'s records `range` onto the end of this list.
    pub(crate) fn extend_from_range(&mut self, other: &Self, range: core::ops::Range<usize>) {
        let Some(blocks) = other.blocks.get(range) else {
            return;
        };
        self.single += blocks
            .iter()
            .filter(|record| record.planes != Self::U | Self::V)
            .count();
        self.blocks.extend_from_slice(blocks);
    }

    /// The record capacity this list is holding.
    pub(crate) fn capacity(&self) -> usize {
        self.blocks.capacity()
    }

    /// Reserves `records`, leaving the list as it was when that fails.
    pub(crate) fn reserve_records(&mut self, records: usize) {
        let _ = self.blocks.try_reserve_exact(records);
    }

    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.single = 0;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Pairs the U and V records from `cursors` on and returns whether every
    /// pair matches with none left over, which is [`Self::uv_twins`] for a
    /// list that grows: `cursors` keeps the next unpaired U and V record.
    fn twins_through(&self, cursors: &mut [usize; 2]) -> bool {
        let next = |from: usize, mask: u8| {
            self.blocks
                .iter()
                .skip(from)
                .position(|record| record.planes & mask != 0)
                .map(|offset| from + offset)
        };
        loop {
            match (next(cursors[0], Self::U), next(cursors[1], Self::V)) {
                (None, None) => return true,
                (Some(u), Some(v)) if self.blocks[u].block == self.blocks[v].block => {
                    *cursors = [u + 1, v + 1];
                }
                _ => return false,
            }
        }
    }

    fn get(&self, index: usize) -> Option<&DeblockBlock> {
        self.blocks.get(index).map(|record| &record.block)
    }

    /// Whether the U and V record lists match record for record. Each plane's
    /// grid is then the same function of its list, so V can read U's grid.
    fn uv_twins(&self) -> bool {
        if self.single == 0 {
            return true;
        }
        let plane = |mask: u8| {
            self.blocks
                .iter()
                .filter(move |record| record.planes & mask != 0)
                .map(|record| &record.block)
        };
        plane(Self::U).eq(plane(Self::V))
    }

    pub(crate) fn iter_plane(&self, plane: usize) -> impl Iterator<Item = (usize, &DeblockBlock)> {
        let mask = [Self::U, Self::V][plane];
        self.blocks
            .iter()
            .enumerate()
            .filter(move |(_, record)| record.planes & mask != 0)
            .map(|(index, record)| (index, &record.block))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeblockQuantDeltas {
    u_ac: i32,
    v_ac: i32,
}

impl DeblockQuantDeltas {
    pub(crate) const ZERO: Self = Self { u_ac: 0, v_ac: 0 };

    pub(crate) const fn from_frame_quant(
        quant: QuantizationParams,
        base_uv_ac_delta_q: i32,
    ) -> Self {
        Self {
            u_ac: quant.delta_q_u_ac + base_uv_ac_delta_q,
            v_ac: quant.delta_q_v_ac + base_uv_ac_delta_q,
        }
    }

    const fn ac_delta(self, plane: usize) -> i32 {
        match plane {
            1 => self.u_ac,
            2 => self.v_ac,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy)]
struct EdgeBlock<'a> {
    block: &'a DeblockBlock,
    chroma_transform: Option<&'a DeblockBlock>,
}

impl EdgeBlock<'_> {
    fn prediction(self, plane: usize) -> DeblockPredictionUnit {
        if plane == 0 {
            self.block.luma_prediction
        } else {
            self.block.chroma_prediction
        }
    }

    fn tx_base(self, plane: usize) -> (usize, usize) {
        let (r, c) = if plane == 0 {
            (self.block.r, self.block.c)
        } else if let Some(transform) = self.chroma_transform {
            (transform.chroma_base_r, transform.chroma_base_c)
        } else {
            (self.block.chroma_base_r, self.block.chroma_base_c)
        };
        (r as usize, c as usize)
    }

    fn tx(self, plane: usize) -> usize {
        usize::from(if plane == 0 {
            self.block.luma_tx
        } else {
            self.chroma_transform
                .and_then(|transform| transform.chroma_tx)
                .or(self.block.chroma_tx)
                .unwrap_or(0)
        })
    }
}

/// One frame's deblock, prepared once and applied over ascending mode-info row
/// sections.
///
/// § 7.14 filters every vertical edge of a plane before any horizontal one, and
/// walks each pass in ascending row order. Splitting that walk by rows keeps
/// both properties: a section runs pass 0 then pass 1 over its own rows only,
/// and a vertical edge is confined to the four rows of its own mode-info unit,
/// so no section reaches a row a later one still owes. A horizontal edge does
/// reach [`DEBLOCK_PASS_1_REACH`] plane rows back, which is what
/// [`FrameDeblock::final_luma_rows`] subtracts from the section frontier before
/// a row counts as final.
pub(crate) struct FrameDeblock {
    records: OwnedDeblockRecords,
    grid: MiGridStorage,
    chroma: [Option<ChromaMiGridStorage>; 2],
    /// Whether the grids are sized and empty yet.
    grids_reset: bool,
    /// How many luma records, then U and V overlay records, the grids hold.
    laid_out: [usize; 3],
    /// The mode-info rows every record of which has arrived, and the rows
    /// each chroma grid has resolved.
    final_mi_rows: usize,
    resolved_mi_rows: [usize; 2],
    /// The next U and V record a frame whose V reads U's grid has not paired.
    twin_cursors: [usize; 2],
    mi_rows: usize,
    filter: DeblockingFilterParams,
    tile_info: Option<TileInfo>,
    disable_loopfilters_across_tiles: bool,
    quant_deltas: DeblockQuantDeltas,
    next_pass_0_mi_row: usize,
    next_pass_1_mi_row: usize,
    /// The edge decisions of each pass of the plane being filtered, which a
    /// V plane reading U's grid with U's strengths reuses.
    edges: [PassEdges; 2],
}

/// One plane band's share of a deblock section: the rows it owns and the passes
/// it still owes over them.
struct PlaneJob<'a, T> {
    band: PlaneBand<'a, T>,
    grid: MiGrid<'a>,
    passes: [Option<PlanePass>; 2],
}

pub(crate) struct OwnedDeblockRecords {
    pub(crate) grids: DeblockGridStorage,
    pub(crate) blocks: Vec<DeblockBlock>,
    pub(crate) chroma: ChromaDeblockRecords,
}

impl OwnedDeblockRecords {
    /// Takes a frame's deblock storage with its record lists emptied.
    pub(crate) fn take_storage(
        records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    ) -> Self {
        let mut storage = Self {
            grids: core::mem::take(&mut records.deblock_grids),
            blocks: core::mem::take(&mut records.deblock_blocks),
            chroma: core::mem::take(&mut records.chroma_deblock_blocks),
        };
        storage.blocks.clear();
        storage.chroma.clear();
        storage
    }
}

/// The interior tile starts one pass filters across, or `None` when the frame
/// has no interior tile boundary in that direction.
fn pass_tile_starts(tile_info: Option<&TileInfo>, pass: usize) -> Option<&[u32]> {
    tile_info.and_then(|tile_info| {
        let starts = if pass == 0 {
            &tile_info.mi_col_starts
        } else {
            &tile_info.mi_row_starts
        };
        starts
            .get(1..starts.len().saturating_sub(1))
            .filter(|starts| !starts.is_empty())
    })
}

/// How many plane rows a § 7.14 edge filter reaches on either side of its edge,
/// which is the widest filter's half width and the gathered line's half length.
pub(crate) const DEBLOCK_PASS_1_REACH: usize = 8;

/// How many mode-info rows the vertical pass keeps ahead of the horizontal one.
///
/// A horizontal edge reads [`DEBLOCK_PASS_1_REACH`] plane rows past itself, so
/// the vertical pass must already have filtered them; on a vertically
/// subsampled plane those rows span four mode-info rows. Keeping exactly that
/// lead is what makes the sectioned walk read the same samples the whole-frame
/// walk does, since a vertical edge stays inside its own mode-info unit and so
/// never reaches back into a row the horizontal pass has already rewritten.
const PASS_0_LEAD_MI_ROWS: usize = 4;

impl FrameDeblock {
    /// Takes ownership of detached deblock records; the first advance lays
    /// the frame's mode-info grids out from them.
    #[allow(clippy::too_many_arguments)]
    #[allow(
        clippy::unnecessary_wraps,
        reason = "callers keep preparation fallible across frame setup paths"
    )]
    pub(crate) fn prepare_owned(
        mut records: OwnedDeblockRecords,
        mi_rows: usize,
        mi_cols: usize,
        filter: DeblockingFilterParams,
        tile_info: Option<TileInfo>,
        disable_loopfilters_across_tiles: bool,
        quant_deltas: DeblockQuantDeltas,
        chroma_subsampling: (usize, usize),
    ) -> Result<Option<Self>, DeblockError> {
        if filter.apply_deblocking_filter == [false; 4] {
            return Ok(None);
        }
        let (grid, chroma) = take_grids(
            &records.chroma,
            mi_cols,
            filter,
            chroma_subsampling,
            &mut records.grids,
        );
        Ok(Some(Self {
            records,
            grid,
            chroma,
            grids_reset: false,
            laid_out: [0; 3],
            final_mi_rows: mi_rows,
            resolved_mi_rows: [0; 2],
            twin_cursors: [0; 2],
            mi_rows,
            filter,
            tile_info,
            disable_loopfilters_across_tiles,
            quant_deltas,
            next_pass_0_mi_row: 0,
            next_pass_1_mi_row: 0,
            edges: Default::default(),
        }))
    }

    /// Deblocks the frame rows past its final prefix in place, then publishes
    /// the newly final prefix for stripe windows.
    pub(crate) fn advance_source<T: ReconSample>(
        &mut self,
        source: &mut FrontierRows<T>,
        mi_row_end: usize,
        bit_depth: BitDepth,
    ) -> Result<(), DeblockError> {
        let (ranges, pass_0_end, pass_1_end) = self.next_ranges(mi_row_end);
        if ranges.iter().any(|rows| !rows.is_empty()) {
            self.lay_out()?;
        }
        self.run_ranges_source(source, &ranges, bit_depth)?;
        self.next_pass_0_mi_row = pass_0_end;
        self.next_pass_1_mi_row = pass_1_end;
        let sub_y = usize::from(source.info().pixel_format().subsampling_y());
        if !source.publish_final_rows(self.final_luma_rows(sub_y)) {
            return Err(DeblockError::Workspace);
        }
        Ok(())
    }

    fn next_ranges(&self, mi_row_end: usize) -> ([Range<usize>; 2], usize, usize) {
        let pass_1_end = mi_row_end.min(self.mi_rows).max(self.next_pass_1_mi_row);
        let pass_0_end = mi_row_end
            .saturating_add(PASS_0_LEAD_MI_ROWS)
            .min(self.mi_rows)
            .max(self.next_pass_0_mi_row);
        (
            [
                self.next_pass_0_mi_row..pass_0_end,
                self.next_pass_1_mi_row..pass_1_end,
            ],
            pass_0_end,
            pass_1_end,
        )
    }

    fn run_ranges_source<T: ReconSample>(
        &mut self,
        source: &mut FrontierRows<T>,
        ranges: &[core::ops::Range<usize>; 2],
        bit_depth: BitDepth,
    ) -> Result<(), DeblockError> {
        if ranges.iter().all(Range::is_empty) {
            return Ok(());
        }
        let pixel_format = source.info().pixel_format();
        let mut edges = core::mem::take(&mut self.edges);
        let mut u_passes = [None; 2];
        let result = (0..3).try_for_each(|plane| {
            let plane_id = plane_index_to_id(plane);
            if plane != 0 && !self.filter.apply_deblocking_filter[plane + 1] {
                return Ok(());
            }
            let passes = [0usize, 1].map(|pass| {
                (!ranges[pass].is_empty())
                    .then(|| {
                        PlanePass::active(
                            plane,
                            pass,
                            self.filter,
                            self.quant_deltas,
                            bit_depth,
                            pixel_format,
                            &ranges[pass],
                        )
                    })
                    .flatten()
            });
            if passes.iter().all(Option::is_none) {
                return Ok(());
            }
            let same = |u: Option<PlanePass>, v: Option<PlanePass>| match (u, v) {
                (Some(u), Some(v)) => {
                    (u.df_delta_q, u.quant_delta) == (v.df_delta_q, v.quant_delta)
                }
                (None, None) => true,
                _ => false,
            };
            let reuse = plane == 2
                && self.chroma[1].is_none()
                && same(u_passes[0], passes[0])
                && same(u_passes[1], passes[1]);
            let Some((_, height)) = source.plane_size(plane_id) else {
                return Ok(());
            };
            if plane == 1 {
                u_passes = passes;
            }
            let (start, end) =
                deblock_plane_row_bounds(passes, height).ok_or(DeblockError::Workspace)?;
            source
                .with_plane_rows_mut(
                    plane_id,
                    start,
                    end,
                    |samples, stride, width, height, y_origin| {
                        let band = PlaneBand {
                            storage: PlaneRows { samples, stride },
                            width,
                            height,
                            y_origin,
                            row_count: end - start,
                        };
                        self.run_plane_job(
                            PlaneJob {
                                band,
                                grid: self.plane_grid(plane),
                                passes,
                            },
                            &mut edges,
                            reuse,
                        )
                    },
                )
                .ok_or(DeblockError::Workspace)?
        });
        self.edges = edges;
        result
    }

    /// Runs one plane's owed passes over their mode-info row ranges: decides
    /// each pass's edges, then filters them. Within a plane the passes stay
    /// ordered.
    fn run_plane_job<T: ReconSample>(
        &self,
        job: PlaneJob<'_, T>,
        edges: &mut [PassEdges; 2],
        reuse: bool,
    ) -> Result<(), DeblockError> {
        let PlaneJob {
            mut band,
            grid,
            passes,
        } = job;
        for plane_pass in passes.into_iter().flatten() {
            let geometry = PassGeometry {
                mi_rows: self.mi_rows,
                mi_cols: self.grid.mi_cols,
                tile_starts: pass_tile_starts(self.tile_info(), plane_pass.pass),
                disable_loopfilters_across_tiles: self.disable_loopfilters_across_tiles,
                width: band.width,
                height: band.height,
            };
            let edges = &mut edges[plane_pass.pass];
            if !reuse {
                edges.build(&grid, &plane_pass, &geometry)?;
            }
            let mut ctx = PlaneCtx::new(&mut band)?;
            if plane_pass.pass == 0 {
                edges.apply::<T, 0>(&mut ctx, plane_pass.bit_depth)?;
            } else {
                edges.apply::<T, 1>(&mut ctx, plane_pass.bit_depth)?;
            }
        }
        Ok(())
    }

    /// The mode-info grid one plane's edges read. An active V plane without a
    /// grid of its own reads U's (see [`ChromaDeblockRecords::uv_twins`]).
    fn plane_grid(&self, plane: usize) -> MiGrid<'_> {
        let chroma = match plane.checked_sub(1) {
            Some(chroma) => self.chroma[chroma].as_ref().or(self.chroma[0].as_ref()),
            None => None,
        };
        MiGrid::new(
            &self.grid,
            chroma,
            &self.records.blocks,
            &self.records.chroma,
        )
    }

    /// The luma rows whose deblocked samples are final now, given how far
    /// [`Self::advance_source`] has got.
    ///
    /// Every remaining horizontal edge sits at or below the frontier and
    /// reaches at most [`DEBLOCK_PASS_1_REACH`] plane rows back, which is twice
    /// as many luma rows on a vertically subsampled chroma plane. A frontier at
    /// the frame bottom leaves nothing owed, so the whole frame is final.
    pub(crate) const fn final_luma_rows(&self, subsampling_y: usize) -> usize {
        let frontier = self.next_pass_1_mi_row * MI_SIZE;
        if self.next_pass_1_mi_row >= self.mi_rows {
            return frontier;
        }
        frontier.saturating_sub(DEBLOCK_PASS_1_REACH << subsampling_y)
    }

    /// The luma rows one [`Self::advance_source`] to `mi_row_end` can touch.
    ///
    /// The vertical pass runs [`PASS_0_LEAD_MI_ROWS`] mode-info rows ahead of
    /// the horizontal one and every edge it filters stays inside its own
    /// mode-info row, so that lead is the whole advance's reach. A caller that
    /// filters a copy of the reconstructed rows must have sealed this many.
    pub(crate) fn data_reach_luma_rows(&self, mi_row_end: usize) -> usize {
        mi_row_end
            .saturating_add(PASS_0_LEAD_MI_ROWS)
            .min(self.mi_rows)
            .saturating_mul(MI_SIZE)
    }

    /// The frame's luma row count, which is the frontier a completed deblock
    /// reaches.
    pub(crate) const fn luma_rows(&self) -> usize {
        self.mi_rows * MI_SIZE
    }

    /// The luma and chroma records taken so far.
    pub(crate) fn records(&self) -> (&[DeblockBlock], &ChromaDeblockRecords) {
        (&self.records.blocks, &self.records.chroma)
    }

    /// Takes the records a frame still being parsed has published since the
    /// last call, in decode order; every record of the mode-info rows below
    /// `final_mi_rows` has then arrived.
    pub(crate) fn append_records(
        &mut self,
        blocks: &mut Vec<DeblockBlock>,
        chroma: &mut ChromaDeblockRecords,
        final_mi_rows: usize,
    ) -> Result<(), DeblockError> {
        self.records.blocks.append(blocks);
        self.records.chroma.append(chroma);
        self.final_mi_rows = final_mi_rows.min(self.mi_rows);
        if self.chroma[1].is_some()
            || !self.filter.apply_deblocking_filter[3]
            || self.records.chroma.twins_through(&mut self.twin_cursors)
        {
            return Ok(());
        }
        let Some(subsampling) = self.chroma[0].as_ref().map(|u| (u.sub_x, u.sub_y)) else {
            return Ok(());
        };
        self.chroma[1] = Some(ChromaMiGridStorage::new(
            self.grid.mi_cols,
            subsampling,
            &mut self.records.grids.chroma[1],
        ));
        self.laid_out[2] = 0;
        self.resolved_mi_rows[1] = 0;
        match self.chroma[1].as_mut() {
            Some(grid) if self.grids_reset => grid.reset(1, self.mi_rows),
            _ => Ok(()),
        }
    }

    /// Lays the records that arrived since the last call out over every
    /// active plane's grid, and resolves the chroma rows they completed.
    fn lay_out(&mut self) -> Result<(), DeblockError> {
        if !self.grids_reset {
            self.grid.reset(self.mi_rows)?;
            for (plane, slot) in self.chroma.iter_mut().enumerate() {
                if let Some(grid) = slot {
                    grid.reset(plane, self.mi_rows)?;
                }
            }
            self.grids_reset = true;
        }
        let blocks = &self.records.blocks;
        self.grid.fill(blocks, self.laid_out[0], self.mi_rows)?;
        self.laid_out[0] = blocks.len();
        let (mi_rows, final_rows) = (self.mi_rows, self.final_mi_rows);
        for (plane, slot) in self.chroma.iter_mut().enumerate() {
            let Some(grid) = slot else { continue };
            grid.fill(
                self.grid.mi_cols,
                &self.records.chroma,
                (plane, self.laid_out[plane + 1]),
                mi_rows,
            )?;
            self.laid_out[plane + 1] = self.records.chroma.len();
            let cell_rows = |mi: usize| {
                if mi >= mi_rows {
                    mi_rows.div_ceil(1 << grid.sub_y)
                } else {
                    mi >> grid.sub_y
                }
            };
            let rows = cell_rows(self.resolved_mi_rows[plane])..cell_rows(final_rows);
            grid.resolve(&self.grid, (blocks, &self.records.chroma), rows);
            self.resolved_mi_rows[plane] = final_rows;
        }
        Ok(())
    }

    /// Hands the grid vectors back for the next frame to lay out over.
    pub(crate) fn release_grids(&mut self, storage: &mut DeblockGridStorage) {
        storage.cells = core::mem::take(&mut self.grid.cells);
        storage.plane = core::mem::take(&mut self.grid.plane);
        for (slot, storage) in self.chroma.iter_mut().zip(&mut storage.chroma) {
            if let Some(grid) = slot.as_mut() {
                storage.0 = core::mem::take(&mut grid.cells);
                storage.1 = core::mem::take(&mut grid.plane);
            }
        }
    }

    pub(crate) fn finish(mut self) -> OwnedDeblockRecords {
        let mut grids = core::mem::take(&mut self.records.grids);
        self.release_grids(&mut grids);
        let mut records = self.records;
        records.grids = grids;
        records
    }

    /// The interior tile boundaries, when the frame has more than one tile.
    fn tile_info(&self) -> Option<&TileInfo> {
        self.tile_info
            .as_ref()
            .filter(|info| info.mi_col_starts.len() > 2 || info.mi_row_starts.len() > 2)
    }
}

#[cfg(test)]
impl FrameDeblock {
    /// Builds a plan over copies of borrowed records, for tests that keep
    /// their records.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        blocks: &[DeblockBlock],
        chroma_blocks: &ChromaDeblockRecords,
        mi_rows: usize,
        mi_cols: usize,
        filter: DeblockingFilterParams,
        tile_info: Option<&TileInfo>,
        disable_loopfilters_across_tiles: bool,
        quant_deltas: DeblockQuantDeltas,
        chroma_subsampling: (usize, usize),
        grids: &mut DeblockGridStorage,
    ) -> Result<Option<Self>, DeblockError> {
        Self::prepare_owned(
            OwnedDeblockRecords {
                grids: core::mem::take(grids),
                blocks: blocks.to_vec(),
                chroma: chroma_blocks.clone(),
            },
            mi_rows,
            mi_cols,
            filter,
            tile_info.cloned(),
            disable_loopfilters_across_tiles,
            quant_deltas,
            chroma_subsampling,
        )
    }
}

type FrameGrids = (MiGridStorage, [Option<ChromaMiGridStorage>; 2]);

/// Takes the frame's grid vectors; the grids stay empty until the first
/// advance lays the records out.
fn take_grids(
    chroma_blocks: &ChromaDeblockRecords,
    mi_cols: usize,
    filter: DeblockingFilterParams,
    chroma_subsampling: (usize, usize),
    storage: &mut DeblockGridStorage,
) -> FrameGrids {
    let mut chroma = [None, None];
    let v_shares_u = filter.apply_deblocking_filter[2..] == [true; 2] && chroma_blocks.uv_twins();
    for (plane, (slot, storage)) in chroma.iter_mut().zip(&mut storage.chroma).enumerate() {
        if filter.apply_deblocking_filter[plane + 2] && !(plane == 1 && v_shares_u) {
            *slot = Some(ChromaMiGridStorage::new(
                mi_cols,
                chroma_subsampling,
                storage,
            ));
        }
    }
    (MiGridStorage::new(mi_cols, storage), chroma)
}

/// Deblocks the TIP unit edges of a `TIP_FRAME_AS_OUTPUT` frame. Each unit
/// column, then each unit row, goes to the run kernels as one run when its
/// samples 8 either side of the edge are in the plane. An edge changes and
/// reads only its own 4 lines, so the result is the same as in the per-edge
/// order.
pub(crate) fn deblock_tip_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    luma_unit_size: usize,
    quant: QuantizationParams,
    base_uv_ac_delta_q: i32,
    tile_starts: Option<(&[u32], &[u32])>,
    disable_loopfilters_across_tiles: bool,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    let pixel_format = workspace.info().pixel_format();
    let mut dimensions = [None; 3];
    for (plane, dimension) in dimensions.iter_mut().enumerate() {
        let plane_id = plane_index_to_id(plane);
        if workspace.plane(plane_id).is_ok() {
            *dimension = Some(coded_plane_dimensions(workspace, plane_id)?);
        }
    }
    let frame = workspace
        .as_frame_mut()
        .map_err(|_| DeblockError::Workspace)?;
    let (y, u, v) = frame.into_planes();
    let views = [Some(y), u, v];
    if views
        .iter()
        .zip(&dimensions)
        .any(|(view, size)| view.is_none() && size.is_some())
    {
        return Err(DeblockError::Workspace);
    }
    let mut planes = views.into_iter().enumerate().map(|(plane, view)| {
        view.zip(dimensions[plane])
            .map(|(view, size)| (plane, view, size))
    });
    let mut slots: [_; 3] = core::array::from_fn(|_| planes.next().flatten());
    let deblock_plane =
        |slot: &mut Option<(usize, splot_recon::PlaneMut<'_, T>, (usize, usize))>| {
            let Some((plane, view, (width, height))) = slot.as_mut() else {
                return Ok(());
            };
            let plane = *plane;
            let (width, height) = (*width, *height);
            let (sub_x, sub_y) = if plane == 0 {
                (0, 0)
            } else {
                (
                    usize::from(pixel_format.subsampling_x()),
                    usize::from(pixel_format.subsampling_y()),
                )
            };
            let unit_width = (luma_unit_size >> sub_x).max(1);
            let unit_height = (luma_unit_size >> sub_y).max(1);
            let quant_delta = match plane {
                1 => quant.delta_q_u_ac + base_uv_ac_delta_q,
                2 => quant.delta_q_v_ac + base_uv_ac_delta_q,
                _ => 0,
            };
            let (q_thr, side) = adaptive_strength(
                q_clamped(quant.base_q_idx, quant_delta, bit_depth),
                bit_depth,
            );
            let stride = view.stride_samples();
            let mut band = PlaneBand::plane(view.samples_mut(), stride, width, height);
            let mut plane_ctx = PlaneCtx::new(&mut band)?;
            let mut run = Vec::new();
            let mut run_edges =
                |plane_ctx: &mut PlaneCtx<'_, '_, T>,
                 vertical: bool,
                 first: usize,
                 edges: usize,
                 (max_width_neg, max_width_pos): (usize, usize)| {
                    let edge =
                        DeblockEdge::new(q_thr, side, max_width_neg, max_width_pos, false, false)
                            .ok_or(DeblockError::SampleFilter)?;
                    run.clear();
                    run.resize(edges, edge);
                    let (samples, stride) = plane_ctx.rows.contiguous_mut();
                    let kernel = if vertical {
                        deblock_vertical_edges::<T>
                    } else {
                        deblock_horizontal_edges::<T>
                    };
                    kernel(samples, stride, first, &run, bit_depth)
                        .map_err(|_| DeblockError::SampleFilter)
                };
            let full_rows = height / MI_SIZE;
            for x in (unit_width..width).step_by(unit_width) {
                let tile_edge = tip_tile_edge(tile_starts.map(|(cols, _)| cols), x, sub_x);
                if disable_loopfilters_across_tiles && tile_edge {
                    continue;
                }
                let max_widths = deblock_filter_max_width(unit_width, plane != 0, tile_edge);
                let mut first_y = 0;
                if full_rows > 0 && x >= GATHER_HALF && x <= width.saturating_sub(GATHER_HALF) {
                    run_edges(&mut plane_ctx, true, x, full_rows, max_widths)?;
                    first_y = full_rows * MI_SIZE;
                }
                for y in (first_y..height).step_by(MI_SIZE) {
                    apply_tip_filter_edge(
                        &mut plane_ctx,
                        x,
                        y,
                        1,
                        0,
                        height.saturating_sub(y).min(MI_SIZE),
                        q_thr,
                        side,
                        max_widths,
                        bit_depth,
                    )?;
                }
            }
            let full_cols = width / MI_SIZE;
            for y in (unit_height..height).step_by(unit_height) {
                let tile_edge = tip_tile_edge(tile_starts.map(|(_, rows)| rows), y, sub_y);
                if disable_loopfilters_across_tiles && tile_edge {
                    continue;
                }
                let max_widths = deblock_filter_max_width(
                    unit_height,
                    plane != 0,
                    y.is_multiple_of(64 >> sub_y),
                );
                let mut first_x = 0;
                if y >= GATHER_HALF && y <= height.saturating_sub(GATHER_HALF) {
                    run_edges(&mut plane_ctx, false, y * stride, full_cols, max_widths)?;
                    first_x = full_cols * MI_SIZE;
                }
                for x in (first_x..width).step_by(MI_SIZE) {
                    apply_tip_filter_edge(
                        &mut plane_ctx,
                        x,
                        y,
                        0,
                        1,
                        width.saturating_sub(x).min(MI_SIZE),
                        q_thr,
                        side,
                        max_widths,
                        bit_depth,
                    )?;
                }
            }
            Ok(())
        };
    slots.iter_mut().try_for_each(deblock_plane)
}

fn tip_tile_edge(starts: Option<&[u32]>, coordinate: usize, subsampling: usize) -> bool {
    let Some(starts) = starts else {
        return false;
    };
    let Some(luma_coordinate) = coordinate.checked_mul(1 << subsampling) else {
        return false;
    };
    let Ok(mi_coordinate) = u32::try_from(luma_coordinate / MI_SIZE) else {
        return false;
    };
    starts
        .get(1..starts.len().saturating_sub(1))
        .is_some_and(|starts| starts.contains(&mi_coordinate))
}

#[allow(clippy::too_many_arguments)]
fn apply_tip_filter_edge<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    x: usize,
    y: usize,
    dx: usize,
    dy: usize,
    lanes: usize,
    q_thr: i32,
    side: i32,
    (max_width_neg, max_width_pos): (usize, usize),
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    let width = choose_filter_width(
        plane_ctx,
        x,
        y,
        dx,
        dy,
        q_thr,
        side,
        max_width_neg,
        max_width_pos,
    )?;
    if width == 0 {
        return Ok(());
    }
    let eff_neg = width.min(max_width_neg);
    let eff_pos = width.min(max_width_pos);
    let params = DeblockSampleFilter {
        boundary: GATHER_HALF,
        q_thr,
        max_width_neg: eff_neg,
        max_width_pos: eff_pos,
        q_thresh_mult: Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1],
        w_mult_neg: W_MULT[eff_neg - 1],
        w_mult_pos: W_MULT[eff_pos - 1],
        prev_lossless: false,
        curr_lossless: false,
        bit_depth,
    };
    apply_edge_samples(plane_ctx, PerpLine::new(x, y, dx, dy), lanes, params)
}

struct PlaneRows<'samples, T> {
    samples: &'samples mut [T],
    stride: usize,
}

impl<T> PlaneRows<'_, T> {
    fn row(&self, index: usize) -> Option<&[T]> {
        let start = index.checked_mul(self.stride)?;
        let end = start.checked_add(self.stride)?;
        self.samples.get(start..end)
    }

    fn row_mut(&mut self, index: usize) -> Option<&mut [T]> {
        let start = index.checked_mul(self.stride)?;
        let end = start.checked_add(self.stride)?;
        self.samples.get_mut(start..end)
    }

    fn contiguous(&self) -> (&[T], usize) {
        (self.samples, self.stride)
    }

    fn contiguous_mut(&mut self) -> (&mut [T], usize) {
        (self.samples, self.stride)
    }
}

struct PlaneCtx<'rows, 'samples, T: ReconSample> {
    rows: &'rows mut PlaneRows<'samples, T>,
    width: usize,
    height: usize,
    y_origin: usize,
    band_rows: usize,
}

/// The plane rows one deblock job filters: a whole plane, or a contiguous band
/// of one.
///
/// `height` stays the plane's own height so edge geometry is unchanged by
/// banding, while `y_origin` and `rows` name the rows the job owns.
struct PlaneBand<'a, T> {
    storage: PlaneRows<'a, T>,
    width: usize,
    height: usize,
    y_origin: usize,
    row_count: usize,
}

impl<'a, T> PlaneBand<'a, T> {
    const fn plane(samples: &'a mut [T], stride: usize, width: usize, height: usize) -> Self {
        Self {
            storage: PlaneRows { samples, stride },
            width,
            height,
            y_origin: 0,
            row_count: height,
        }
    }
}

impl<'rows, 'samples, T: ReconSample> PlaneCtx<'rows, 'samples, T> {
    /// Views the plane rows `y_origin..y_origin + rows` of a `height`-row plane.
    ///
    /// Coordinates stay in plane space, so a band filters exactly the samples
    /// the whole-plane view does; a row outside the band is not addressable.
    fn new(band: &'rows mut PlaneBand<'samples, T>) -> Result<Self, DeblockError> {
        let (stride, width, height) = (band.storage.stride, band.width, band.height);
        let (y_origin, rows) = (band.y_origin, band.row_count);
        if width > stride || stride == 0 || y_origin.checked_add(rows) > Some(height) {
            return Err(DeblockError::Workspace);
        }
        let required = band
            .storage
            .stride
            .checked_mul(rows)
            .ok_or(DeblockError::Workspace)?;
        band.storage
            .samples
            .get_mut(..required)
            .ok_or(DeblockError::Workspace)?;
        Ok(Self {
            rows: &mut band.storage,
            width,
            height,
            y_origin,
            band_rows: rows,
        })
    }

    fn local_coords(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        let row = y
            .checked_sub(self.y_origin)
            .filter(|row| *row < self.band_rows)?;
        let column = (x < self.width).then_some(x)?;
        Some((row, column))
    }

    fn sample(&self, x: usize, y: usize) -> T {
        self.local_coords(x, y)
            .and_then(|(row, column)| self.rows.row(row)?.get(column))
            .copied()
            .unwrap_or_default()
    }

    fn set_sample(&mut self, x: usize, y: usize, value: T) -> Result<(), DeblockError> {
        let (row, column) = self.local_coords(x, y).ok_or(DeblockError::Workspace)?;
        *self
            .rows
            .row_mut(row)
            .and_then(|row| row.get_mut(column))
            .ok_or(DeblockError::Workspace)? = value;
        Ok(())
    }
}

struct StrengthCache {
    /// `(qThr + 1, side)` per qindex, zero until derived: qThr is never negative.
    values: [Cell<(i32, i32)>; max_quantizer_index(BitDepth::Ten) as usize + 1],
    quant_delta: i32,
    df_delta_q: i32,
    bit_depth: BitDepth,
}

impl StrengthCache {
    fn new(quant_delta: i32, df_delta_q: i32, bit_depth: BitDepth) -> Self {
        Self {
            values: [const { Cell::new((0, 0)) }; max_quantizer_index(BitDepth::Ten) as usize + 1],
            quant_delta,
            df_delta_q,
            bit_depth,
        }
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn get(&self, qindex: u32) -> (i32, i32) {
        let calculate = || {
            adaptive_strength(
                deblock_level(qindex, self.quant_delta, self.df_delta_q, self.bit_depth),
                self.bit_depth,
            )
        };
        let Some(entry) = usize::try_from(qindex)
            .ok()
            .and_then(|index| self.values.get(index))
        else {
            return calculate();
        };
        let (q_thr, side) = entry.get();
        if q_thr != 0 {
            return (q_thr - 1, side);
        }
        let (q_thr, side) = calculate();
        entry.set((q_thr + 1, side));
        (q_thr, side)
    }
}

#[derive(Clone, Copy)]
struct PlanePass {
    plane: usize,
    /// The mode-info rows this pass still owes, as a half-open range.
    mi_row_range: (usize, usize),
    pass: usize,
    plane_sub_x: usize,
    plane_sub_y: usize,
    row_step: usize,
    df_delta_q: i32,
    quant_delta: i32,
    bit_depth: BitDepth,
    allow_df_sub_pu: bool,
}

fn deblock_plane_row_bounds(
    passes: [Option<PlanePass>; 2],
    height: usize,
) -> Option<(usize, usize)> {
    let mut bounds: Option<(usize, usize)> = None;
    for pass in passes.into_iter().flatten() {
        let (mi_start, mi_end) = pass.mi_row_range;
        if mi_start >= mi_end {
            continue;
        }
        let first = (mi_start * MI_SIZE) >> pass.plane_sub_y;
        let last = ((mi_end - 1) * MI_SIZE) >> pass.plane_sub_y;
        let (start, end) = if pass.pass == 0 {
            (first, last.saturating_add(MI_SIZE).min(height))
        } else {
            (
                first.saturating_sub(GATHER_HALF),
                last.saturating_add(GATHER_HALF).min(height),
            )
        };
        bounds = Some(bounds.map_or((start, end), |(low, high)| (low.min(start), high.max(end))));
    }
    bounds.filter(|(start, end)| start < end)
}

impl PlanePass {
    fn active(
        plane: usize,
        pass: usize,
        filter: DeblockingFilterParams,
        quant_deltas: DeblockQuantDeltas,
        bit_depth: BitDepth,
        pixel_format: PixelFormat,
        mi_row_range: &Range<usize>,
    ) -> Option<Self> {
        let apply_index = if plane == 0 { pass } else { plane + 1 };
        if !filter.apply_deblocking_filter[apply_index] {
            return None;
        }

        let (plane_sub_x, plane_sub_y) = if plane == 0 {
            (0, 0)
        } else {
            (
                usize::from(pixel_format.subsampling_x()),
                usize::from(pixel_format.subsampling_y()),
            )
        };
        Some(Self {
            plane,
            mi_row_range: (mi_row_range.start, mi_row_range.end),
            pass,
            plane_sub_x,
            plane_sub_y,
            row_step: 1 << plane_sub_y,
            df_delta_q: filter.df_delta_q[apply_index],
            quant_delta: quant_deltas.ac_delta(plane),
            bit_depth,
            allow_df_sub_pu: filter.allow_df_sub_pu,
        })
    }

    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn edge_context(self, row: usize, col: usize, tile_edge: bool) -> EdgeContext {
        EdgeContext {
            row,
            col,
            plane_sub_x: self.plane_sub_x,
            plane_sub_y: self.plane_sub_y,
            allow_df_sub_pu: self.allow_df_sub_pu,
            tile_edge,
        }
    }
}

/// Whether mode-info `coordinate` is one of the interior tile `starts`.
fn starts_tile(starts: Option<&[u32]>, coordinate: usize) -> bool {
    starts.is_some_and(|starts| {
        u32::try_from(coordinate).is_ok_and(|coordinate| starts.contains(&coordinate))
    })
}

#[derive(Clone, Copy)]
struct EdgeContext {
    row: usize,
    col: usize,
    plane_sub_x: usize,
    plane_sub_y: usize,
    allow_df_sub_pu: bool,
    tile_edge: bool,
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn sub_pu_dimension(
    info: EdgeBlock<'_>,
    plane: usize,
    pass: usize,
    sub_x: usize,
    sub_y: usize,
) -> usize {
    if let Some(size) = info.block.sub_pu_size {
        let (dimension, subsampling) = if pass == 0 {
            (size.width, sub_x)
        } else {
            (size.height, sub_y)
        };
        return (usize::from(dimension) >> subsampling).max(1);
    }
    let tx = if plane == 0 {
        info.block.luma_prediction.default_sub_pu_tx
    } else {
        info.block.chroma_prediction.default_sub_pu_tx
    };
    let dimensions = if pass == 0 { &TX_WIDTH } else { &TX_HEIGHT };
    dimensions
        .get(usize::from(tx))
        .and_then(|&size| usize::try_from(size).ok())
        .unwrap_or(1)
}

#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn sub_pu_base(
    info: EdgeBlock<'_>,
    plane: usize,
    x: usize,
    y: usize,
    sub_x: usize,
    sub_y: usize,
) -> (usize, usize) {
    let prediction = info.prediction(plane);
    let block_x = (prediction.base_c as usize * MI_SIZE) >> sub_x;
    let block_y = (prediction.base_r as usize * MI_SIZE) >> sub_y;
    let Some(size) = info.block.sub_pu_size else {
        return (block_x, block_y);
    };
    let width = (usize::from(size.width) >> sub_x).max(1);
    let height = (usize::from(size.height) >> sub_y).max(1);
    (
        block_x + round_down(x.saturating_sub(block_x), width),
        block_y + round_down(y.saturating_sub(block_y), height),
    )
}

/// `value / step * step`, without the divide for the power-of-two sub-PU
/// sizes.
#[inline]
const fn round_down(value: usize, step: usize) -> usize {
    if step.is_power_of_two() {
        value & !(step - 1)
    } else {
        value / step * step
    }
}

fn sub_pu_filter_dimension(tx_size: usize, sub_pu_size: usize, is_tx_edge: bool) -> (usize, bool) {
    if tx_size < sub_pu_size {
        (tx_size, false)
    } else if !is_tx_edge && tx_size == 8 {
        (4, true)
    } else if !is_tx_edge && tx_size == 16 && sub_pu_size == 16 {
        (8, true)
    } else {
        (sub_pu_size, true)
    }
}

/// Filters the luma vertical edge `ctx` names on its own, as the § 7.17 pass
/// loop visits it.
#[cfg(test)]
fn deblock_filter_edge<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    grid: &MiGrid,
    ctx: EdgeContext,
    disable_loopfilters_across_tiles: bool,
    strengths: &StrengthCache,
) -> Result<(), DeblockError> {
    let geometry = PassGeometry {
        mi_rows: usize::MAX,
        mi_cols: usize::MAX,
        tile_starts: None,
        disable_loopfilters_across_tiles,
        width: plane_ctx.width,
        height: plane_ctx.height,
    };
    let edge = edges::edge_at::<0, 0>(grid, ctx, &geometry, strengths)?;
    if edge.is_none() {
        return Ok(());
    }
    let (x_p, y_p) = (ctx.col * MI_SIZE, ctx.row * MI_SIZE);
    edges::LoneEdge { x_p, y_p, edge }.filter::<T, 0>(plane_ctx, BitDepth::Eight)
}

/// What the records on both sides of an edge decide, before the clamps that
/// depend on where along its row or column the edge sits.
#[derive(Clone, Copy)]
struct EdgeDecision {
    filter_size: usize,
    q_thr: i32,
    side: i32,
    prev_lossless: bool,
    curr_lossless: bool,
}

/// The current and previous records across the edge at mode-info
/// (`row`, `col`); the walk visits no edge on the plane's first row or column.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn edge_blocks<'g, const PLANE: usize, const PASS: usize>(
    grid: &'g MiGrid,
    row: usize,
    col: usize,
    sub_x: usize,
    sub_y: usize,
) -> Result<(EdgeBlock<'g>, EdgeBlock<'g>), DeblockError> {
    let edge = |row: usize, col: usize| {
        let edge = if PLANE == 0 {
            grid.get_luma_edge(row, col)
        } else {
            grid.get_edge(row, col)
        };
        edge.ok_or(DeblockError::UncoveredMi { row, col })
    };
    let prev = if PASS == 0 {
        col.checked_sub(1 << sub_x).map(|col| (row, col))
    } else {
        row.checked_sub(1 << sub_y).map(|row| (row, col))
    };
    let (prev_row, prev_col) = prev.ok_or(DeblockError::UncoveredMi { row, col })?;
    Ok((edge(row, col)?, edge(prev_row, prev_col)?))
}

impl EdgeBlock<'_> {
    fn same(self, other: Self) -> bool {
        core::ptr::eq(self.block, other.block)
            && match (self.chroma_transform, other.chroma_transform) {
                (Some(a), Some(b)) => core::ptr::eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// [`deblock_filter_max_width`] by chroma plane, super-block edge and filter
/// size up to 64, which every larger size maps as.
const MAX_WIDTHS: [[[(u8, u8); 65]; 2]; 2] = {
    let mut table = [[[(0, 0); 65]; 2]; 2];
    let mut entry = 0;
    while entry < 4 * 65 {
        let (chroma, sb_edge, size) = (entry / 130, entry / 65 % 2, entry % 65);
        let (neg, pos) = deblock_filter_max_width(size, chroma == 1, sb_edge == 1);
        table[chroma][sb_edge][size] = (neg as u8, pos as u8);
        entry += 1;
    }
    table
};

/// Derives § 7.17.2 `applyFilter`, `filterSize`, `qThr` and `side` from the
/// two records of one edge.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn edge_decision<const PLANE: usize, const PASS: usize>(
    curr: EdgeBlock<'_>,
    prev: EdgeBlock<'_>,
    x_p: usize,
    y_p: usize,
    ctx: EdgeContext,
    strengths: &StrengthCache,
) -> Option<EdgeDecision> {
    let EdgeContext {
        plane_sub_x,
        plane_sub_y,
        allow_df_sub_pu,
        ..
    } = ctx;
    let plane = PLANE;
    let pass = PASS;
    let (dx, dy) = if pass == 0 { (1usize, 0usize) } else { (0, 1) };
    if curr.same(prev) {
        return same_record_decision::<PLANE, PASS>(curr, x_p, y_p, ctx, strengths);
    }

    let (tx_row_base, tx_col_base) = curr.tx_base(plane);
    let (prev_tx_row_base, prev_tx_col_base) = prev.tx_base(plane);
    let prediction = curr.prediction(plane);
    let block_y = (prediction.base_r as usize * MI_SIZE) >> plane_sub_y;
    let block_x = (prediction.base_c as usize * MI_SIZE) >> plane_sub_x;
    let skip = curr.block.skip;
    let tx_sz = curr.tx(plane);
    let prev_tx_sz = prev.tx(plane);
    let curr_tx_size = usize::try_from(if pass == 0 {
        TX_WIDTH[tx_sz]
    } else {
        TX_HEIGHT[tx_sz]
    })
    .unwrap_or(0);
    let prev_tx_size = usize::try_from(if pass == 0 {
        TX_WIDTH[prev_tx_sz]
    } else {
        TX_HEIGHT[prev_tx_sz]
    })
    .unwrap_or(0);
    let sub_pu_sizes = if allow_df_sub_pu {
        let curr_sub_pu_base = sub_pu_base(curr, plane, x_p, y_p, plane_sub_x, plane_sub_y);
        let prev_sub_pu_base = sub_pu_base(
            prev,
            plane,
            x_p.saturating_sub(dx),
            y_p.saturating_sub(dy),
            plane_sub_x,
            plane_sub_y,
        );
        (curr_sub_pu_base != prev_sub_pu_base).then(|| {
            (
                sub_pu_dimension(curr, plane, pass, plane_sub_x, plane_sub_y),
                sub_pu_dimension(prev, plane, pass, plane_sub_x, plane_sub_y),
            )
        })
    } else {
        None
    };
    let is_block_edge = (pass == 0 && x_p == block_x) || (pass == 1 && y_p == block_y);
    let is_tx_edge = tx_col_base != prev_tx_col_base || tx_row_base != prev_tx_row_base;
    let (curr_filter_size, curr_sub_pu_edge) = if let Some((curr_sub_pu_size, _)) = sub_pu_sizes {
        sub_pu_filter_dimension(curr_tx_size, curr_sub_pu_size, is_tx_edge)
    } else {
        (curr_tx_size, false)
    };
    let prev_filter_size = if let Some((_, prev_sub_pu_size)) = sub_pu_sizes {
        sub_pu_filter_dimension(prev_tx_size, prev_sub_pu_size, is_tx_edge).0
    } else {
        prev_tx_size
    };
    let is_sub_pu_edge = curr_sub_pu_edge && !is_block_edge;

    let curr_strength = strengths.get(curr.block.qindex);
    let prev_strength = strengths.get(prev.block.qindex);
    let strong = |(q_thr, side): (i32, i32)| q_thr != 0 && side != 0;
    let apply_filter = (is_tx_edge || is_sub_pu_edge)
        && (strong(curr_strength) || strong(prev_strength))
        && (is_block_edge || !skip || is_sub_pu_edge);
    if !apply_filter {
        return None;
    }

    let (q_thr, side) = edge_strengths(curr_strength, prev_strength, is_sub_pu_edge && !is_tx_edge);
    Some(EdgeDecision {
        filter_size: curr_filter_size.min(prev_filter_size),
        q_thr,
        side,
        prev_lossless: prev.block.lossless,
        curr_lossless: curr.block.lossless,
    })
}

/// The § 7.17.2 `qThr` and `side` of an edge from its two records' own,
/// scaled down on an edge that is a sub-PU edge only.
fn edge_strengths(
    (curr_q, curr_side): (i32, i32),
    (prev_q, prev_side): (i32, i32),
    sub_pu_only: bool,
) -> (i32, i32) {
    let (q_thr, side) = combine_strengths(curr_q, prev_q, curr_side, prev_side);
    if sub_pu_only {
        (q_thr >> 3, side >> 3)
    } else {
        (q_thr, side)
    }
}

/// [`edge_decision`] for an edge whose two sides are the same record. Their
/// transform and prediction bases match, so the edge filters only where the
/// sub-PU base changes along the pass: on a multiple of the sub-PU size past
/// the prediction base, which is never the block edge.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn same_record_decision<const PLANE: usize, const PASS: usize>(
    record: EdgeBlock<'_>,
    x_p: usize,
    y_p: usize,
    ctx: EdgeContext,
    strengths: &StrengthCache,
) -> Option<EdgeDecision> {
    let size = record.block.sub_pu_size.filter(|_| ctx.allow_df_sub_pu)?;
    let prediction = record.prediction(PLANE);
    let (at, base, sub, size) = if PASS == 0 {
        (x_p, prediction.base_c, ctx.plane_sub_x, size.width)
    } else {
        (y_p, prediction.base_r, ctx.plane_sub_y, size.height)
    };
    let base = (base as usize * MI_SIZE) >> sub;
    let sub_pu_size = (usize::from(size) >> sub).max(1);
    if at <= base || round_down(at - base, sub_pu_size) != at - base {
        return None;
    }
    let tx = record.tx(PLANE);
    let tx_size = usize::try_from(if PASS == 0 {
        TX_WIDTH[tx]
    } else {
        TX_HEIGHT[tx]
    })
    .unwrap_or(0);
    let (filter_size, sub_pu_edge) = sub_pu_filter_dimension(tx_size, sub_pu_size, false);
    let strength = strengths.get(record.block.qindex);
    if !sub_pu_edge || strength.0 == 0 || strength.1 == 0 {
        return None;
    }
    let (q_thr, side) = edge_strengths(strength, strength, true);
    Some(EdgeDecision {
        filter_size,
        q_thr,
        side,
        prev_lossless: record.block.lossless,
        curr_lossless: record.block.lossless,
    })
}

#[allow(clippy::too_many_arguments)]
fn choose_filter_width<T: ReconSample>(
    plane_ctx: &PlaneCtx<'_, '_, T>,
    x_p: usize,
    y_p: usize,
    dx: usize,
    dy: usize,
    q_thr: i32,
    side: i32,
    max_width_neg: usize,
    max_width_pos: usize,
) -> Result<usize, DeblockError> {
    if q_thr == 0 || side == 0 {
        return Ok(0);
    }
    let boundary = GATHER_HALF;
    let horizontal = dx == 1
        && dy == 0
        && x_p >= GATHER_HALF
        && x_p <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y_p >= plane_ctx.y_origin
        && y_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.height);
    let vertical = dx == 0
        && dy == 1
        && y_p >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y_p <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x_p
            .checked_add(MI_SIZE)
            .is_some_and(|end| end <= plane_ctx.width);
    if horizontal || vertical {
        let (samples, stride) = plane_ctx.rows.contiguous();
        let first_boundary = (y_p - plane_ctx.y_origin) * stride + x_p;
        let perpendicular_stride = if horizontal { 1 } else { stride };
        let lane_stride = if horizontal { stride } else { 1 };
        let params = DeblockFilterChoice {
            boundary: first_boundary,
            q_thr,
            side_thr: side,
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        };
        return deblock_filter_choice_strided(
            samples,
            first_boundary + (MI_SIZE - 1) * lane_stride,
            NonZeroUsize::new(perpendicular_stride).ok_or(DeblockError::Workspace)?,
            &params,
        )
        .map_err(|_| DeblockError::FilterChoice);
    }

    let s = gather_line(plane_ctx, PerpLine::new(x_p, y_p, dx, dy));
    let end = MI_SIZE - 1;
    let t_x = x_p + dy * end;
    let t_y = y_p + dx * end;
    let t = gather_line(plane_ctx, PerpLine::new(t_x, t_y, dx, dy));

    let width = deblock_filter_choice(
        &s,
        &t,
        &DeblockFilterChoice {
            boundary,
            q_thr,
            side_thr: side,
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        },
    )
    .map_err(|_| DeblockError::FilterChoice)?;
    Ok(width)
}

fn coded_plane_dimensions<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
) -> Result<(usize, usize), DeblockError> {
    let plane = workspace
        .plane(plane_id)
        .map_err(|_| DeblockError::Workspace)?;
    let storage_size = plane.storage_size();
    Ok((storage_size.width(), storage_size.height()))
}

#[derive(Clone, Copy)]
struct PerpLine {
    x: usize,
    y: usize,
    dx: usize,
    dy: usize,
}

impl PerpLine {
    const fn new(x: usize, y: usize, dx: usize, dy: usize) -> Self {
        Self { x, y, dx, dy }
    }

    fn offset(self, offset: isize) -> Result<(usize, usize), DeblockError> {
        let fx = (self.x as isize + offset * self.dx as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        let fy = (self.y as isize + offset * self.dy as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        Ok((fx, fy))
    }
}

const GATHER_HALF: usize = 8;

fn apply_edge_samples<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    perp: PerpLine,
    lanes: usize,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let PerpLine { x, y, dx, dy } = perp;
    let lanes = lanes.min(if dx == 1 {
        plane_ctx.height.saturating_sub(y)
    } else {
        plane_ctx.width.saturating_sub(x)
    });
    if lanes == 0 {
        return Ok(());
    }
    let horizontal = dx == 1
        && dy == 0
        && x >= GATHER_HALF
        && x <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && y >= plane_ctx.y_origin
        && y.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.height);
    let vertical = dx == 0
        && dy == 1
        && y >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && y <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && x.checked_add(lanes)
            .is_some_and(|end| end <= plane_ctx.width);
    if lanes <= MI_SIZE && params.boundary == GATHER_HALF && (horizontal || vertical) {
        let y_origin = plane_ctx.y_origin;
        let (samples, stride) = plane_ctx.rows.contiguous_mut();
        let boundary = (y - y_origin) * stride + x;
        let perpendicular_stride = if horizontal { 1 } else { stride };
        let lane_stride = if horizontal { stride } else { 1 };
        let perpendicular_stride =
            NonZeroUsize::new(perpendicular_stride).ok_or(DeblockError::Workspace)?;
        if lanes == MI_SIZE {
            deblock_sample_filter_strided_4(
                samples,
                perpendicular_stride,
                NonZeroUsize::new(lane_stride).ok_or(DeblockError::Workspace)?,
                &DeblockSampleFilter { boundary, ..params },
            )
            .map_err(|_| DeblockError::SampleFilter)?;
        } else {
            for lane in 0..lanes {
                deblock_sample_filter_strided(
                    samples,
                    perpendicular_stride,
                    &DeblockSampleFilter {
                        boundary: boundary + lane * lane_stride,
                        ..params
                    },
                )
                .map_err(|_| DeblockError::SampleFilter)?;
            }
        }
        return Ok(());
    }
    for lane in 0..lanes {
        apply_sample_filter(
            plane_ctx,
            PerpLine::new(x + dy * lane, y + dx * lane, dx, dy),
            params,
        )?;
    }
    Ok(())
}

fn apply_sample_filter<T: ReconSample>(
    plane_ctx: &mut PlaneCtx<'_, '_, T>,
    perp: PerpLine,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let before = gather_line(plane_ctx, perp);
    let mut line = before;

    deblock_sample_filter(&mut line, &params).map_err(|_| DeblockError::SampleFilter)?;

    for (idx, (&new, &old)) in line.iter().zip(before.iter()).enumerate() {
        let changed = new.to_u16() != old.to_u16();
        if !changed {
            continue;
        }
        let (fx, fy) = perp.offset(idx as isize - params.boundary as isize)?;
        if fx >= plane_ctx.width || fy >= plane_ctx.height {
            return Err(DeblockError::Workspace);
        }
        plane_ctx.set_sample(fx, fy, new)?;
    }
    Ok(())
}

fn gather_line<T: ReconSample>(
    plane_ctx: &PlaneCtx<'_, '_, T>,
    perp: PerpLine,
) -> [T; 2 * GATHER_HALF] {
    let mut line = [T::default(); 2 * GATHER_HALF];
    if perp.x >= GATHER_HALF
        && perp.y >= plane_ctx.y_origin
        && perp.x <= plane_ctx.width.saturating_sub(GATHER_HALF)
        && perp.dx == 1
        && perp.dy == 0
    {
        let row = perp.y - plane_ctx.y_origin;
        let start = perp.x - GATHER_HALF;
        if let Some(src) = plane_ctx
            .rows
            .row(row)
            .and_then(|row| row.get(start..start + 2 * GATHER_HALF))
        {
            line.copy_from_slice(src);
            return line;
        }
    }
    if perp.y >= plane_ctx.y_origin.saturating_add(GATHER_HALF)
        && perp.x < plane_ctx.width
        && perp.y <= plane_ctx.height.saturating_sub(GATHER_HALF)
        && perp.dx == 0
        && perp.dy == 1
    {
        let start = perp.y - GATHER_HALF - plane_ctx.y_origin;
        let x = perp.x;
        let row_range = start..start + 2 * GATHER_HALF;
        if row_range
            .clone()
            .all(|row| plane_ctx.rows.row(row).is_some_and(|row| x < row.len()))
        {
            for (lane, row) in line.iter_mut().zip(row_range) {
                if let Some(row) = plane_ctx.rows.row(row) {
                    *lane = row[x];
                }
            }
            return line;
        }
    }
    let max_x = plane_ctx.width.saturating_sub(1) as isize;
    let min_y = plane_ctx.y_origin as isize;
    let max_y = (plane_ctx.y_origin + plane_ctx.band_rows).saturating_sub(1) as isize;
    for (idx, lane) in line.iter_mut().enumerate() {
        let offset = idx as isize - GATHER_HALF as isize;
        let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
        let sy = (perp.y as isize + offset * perp.dy as isize).clamp(min_y, max_y) as usize;
        *lane = plane_ctx.sample(sx, sy);
    }
    line
}

const DF_DELTA_SCALE: i32 = 8;

fn deblock_level(qindex: u32, quant_delta: i32, df_delta_q: i32, bit_depth: BitDepth) -> u32 {
    let q_clamped = q_clamped(qindex, quant_delta, bit_depth);
    q_clamped.saturating_add_signed(df_delta_q.saturating_mul(DF_DELTA_SCALE))
}

fn q_clamped(qindex: u32, delta: i32, bit_depth: BitDepth) -> u32 {
    if qindex == 0 && delta <= 0 {
        return 0;
    }
    let adjusted = qindex.saturating_add_signed(delta);
    adjusted.clamp(1, max_quantizer_index(bit_depth))
}

fn adaptive_strength(lvl: u32, bit_depth: BitDepth) -> (i32, i32) {
    let q_ind = deblock_side_threshold_index(lvl, bit_depth);
    let side_threshold = SIDE_THRESHOLDS[q_ind];
    deblock_adaptive_filter_strength(lvl, side_threshold, bit_depth)
}

fn combine_strengths(curr_q: i32, prev_q: i32, curr_side: i32, prev_side: i32) -> (i32, i32) {
    let q_thr = if curr_q != 0 && prev_q != 0 {
        (curr_q + prev_q + 1) >> 1
    } else {
        curr_q.max(prev_q)
    };
    let side = if curr_side != 0 && prev_side != 0 {
        (curr_side + prev_side + 1) >> 1
    } else {
        curr_side.max(prev_side)
    };
    (q_thr, side)
}

fn plane_index_to_id(plane: usize) -> PlaneId {
    match plane {
        0 => PlaneId::Y,
        1 => PlaneId::U,
        _ => PlaneId::V,
    }
}

impl From<crate::filters::source::StripeCopyError> for DeblockError {
    fn from(error: crate::filters::source::StripeCopyError) -> Self {
        match error {
            crate::filters::source::StripeCopyError::Allocation(plane) => Self::Allocation {
                plane,
                context: "deblocked stripe window",
            },
            crate::filters::source::StripeCopyError::Geometry => Self::Workspace,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DeblockError {
    #[error("deblocking MI ({row}, {col}) is not covered by any decoded block")]
    UncoveredMi { row: usize, col: usize },
    #[error("deblocking filter-choice primitive rejected its inputs")]
    FilterChoice,
    #[error("deblocking sample-filter primitive rejected its inputs")]
    SampleFilter,
    #[error("deblocking workspace sample access went out of bounds")]
    Workspace,
    #[error("deblocking {context} storage could not be reserved")]
    Allocation {
        plane: splot_recon::PlaneId,
        context: &'static str,
    },
}

#[cfg(test)]
#[path = "deblock_tests.rs"]
mod tests;

#[cfg(test)]
mod sub_pu_tests;

#[cfg(test)]
mod tip_tests;
