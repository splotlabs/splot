// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The block-level parse/reconstruct seam of one inter frame.
//!
//! The entropy pass reads no reference sample and no projected motion field —
//! its § 7.12.2 TIP reference pair comes from the header's order hints — so it
//! settles by the bitstream alone. [`InterFrameParser`] advances one unit at a
//! time, then finalizes the frame's CDF subset and filter
//! grids, which are entropy-pass products too. What is still owed is the § 7.9
//! temporal prelude, the § 7.12 resolve pass and reconstruction.
//! [`prepare_scheduled_recon`] converts that work into the row graph
//! once the frame becomes admissible.

use super::super::MotionFieldHandle;
use super::*;
use splot_core::headers::frame::RefIdxBuf;
use std::sync::Arc;

#[cfg(test)]
mod scheduled_frame;

/// One inter frame after its entropy pass, owned so its reconstruction can run
/// after the driver has moved on to the next frame's parse.
pub(crate) struct InterFrameParse {
    unit_count: usize,
    records: crate::filters::wienerns_lr::FrameFilterRecords,
    /// The end-of-walk CDF subset published to the canonical `PipelineFrame`.
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    cdef_grid: crate::filters::cdef::CdefUnitGrid,
    /// The walk-parsed CCSO unit grid published to the canonical `PipelineFrame`.
    pub(crate) ccso_grid: Option<Arc<crate::filters::ccso::CcsoUnitGrid>>,
    pub(crate) segment_ids: Arc<FrameSegmentIdMap>,
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
    products: Option<super::super::FrameProductWriters>,
}

/// Resumable entropy state; input bytes are borrowed, mutable tile state is not.
pub(crate) struct InterFrameParser<'payload> {
    parser: Option<tile::TileParser<'payload>>,
    setup: super::InterParseSetup,
}

impl<'payload> InterFrameParser<'payload> {
    pub(crate) const fn new(setup: super::InterParseSetup) -> Self {
        Self {
            parser: None,
            setup,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_unit<T: ReconSample>(
        &mut self,
        tile: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit,
        tile_bytes: &'payload [u8],
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        reference: &InterReferenceState<T>,
        parse_progress: &tile::ParseProgress,
    ) -> Result<bool> {
        let context = &self
            .setup
            .params
            .context(sequence, core, reference, ref_frame_idx);
        if self.parser.is_none() {
            let (cdef_state, gdf_state, ccso_state) = self
                .setup
                .filter_states
                .take()
                .ok_or_else(tile::invalid_inter_tile_scheduling_state)?;
            self.parser = Some(tile::TileParser::scheduled(
                tile,
                tile_bytes,
                context,
                cdef_state,
                gdf_state,
                ccso_state,
                parse_progress,
            )?);
        }
        self.parser
            .as_mut()
            .ok_or_else(tile::invalid_inter_tile_scheduling_state)?
            .parse_scheduled_unit(tile, context, parse_progress)
    }

    pub(crate) fn finish<T: ReconSample>(
        self,
        tile: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit,
        mut records: crate::filters::wienerns_lr::FrameFilterRecords,
        core: &FrameHeaderCore,
        reference: &InterReferenceState<T>,
        parse_progress: &tile::ParseProgress,
        mut products: super::super::FrameProductWriters,
    ) -> Result<InterFrameParse> {
        let super::InterParseSetup {
            params,
            filter_states: _,
            initial_frame_cdfs,
            qindex,
        } = self.setup;
        let parsed = self
            .parser
            .ok_or_else(tile::invalid_inter_tile_scheduling_state)?
            .finish_scheduled(parse_progress)?;
        let previous = final_segment_ids(core, reference, params.mi_rows, params.mi_cols);
        let segment_ids = if previous.is_some() {
            None
        } else {
            Some(products.segment_ids(params.mi_rows, params.mi_cols)?)
        };
        records.clear();
        let (unit_count, cdef_state, gdf_state, ccso_state) =
            parsed.finish_single_tile_filter_state(parse_progress, &mut records, segment_ids)?;
        let frame_cdfs = finish_frame_cdfs(
            &initial_frame_cdfs,
            std::slice::from_mut(tile),
            qindex,
            &mut products,
        )?;
        let ccso_grid = products.finish_ccso(ccso_state)?;
        if let Some(previous) = previous {
            products.inherit_segment_ids(previous)?;
        }
        let segment_ids = products.finish_segment_ids()?;
        Ok(InterFrameParse {
            unit_count,
            records,
            frame_cdfs,
            cdef_grid: cdef_state.into_grid()?,
            ccso_grid,
            segment_ids,
            gdf_grid: gdf_state.into_grid()?,
            products: Some(products),
        })
    }
}

impl InterFrameParse {
    pub(crate) fn publish_products(&mut self) -> Result<super::super::FrameProducts> {
        let products = self
            .products
            .take()
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        Ok(products.settle(
            Arc::clone(&self.frame_cdfs),
            self.ccso_grid.clone(),
            Arc::clone(&self.segment_ids),
        ))
    }
    /// Hands the frontier the filter state the § 8.2 pass settles last.
    ///
    /// Reconstruction is already admitted by the time this runs; only the
    /// § 7.17 frontier chain waits on it.
    pub(in crate::prediction::inter) fn attach_filters<T: ReconSample>(
        self,
        pending: PendingFilterAttach<T>,
        tile: &tile::ScheduledTileRecon<T>,
        parse_progress: &Arc<super::tile::ParseProgress>,
        filter_shell: crate::filters::wienerns_lr::recon::OwnedFilterShell<T>,
    ) -> Result<()> {
        let Self {
            unit_count,
            mut records,
            frame_cdfs: _,
            cdef_grid,
            ccso_grid,
            segment_ids: _,
            gdf_grid,
            products: _,
        } = self;
        let PendingFilterAttach {
            info,
            plane_sizes,
            filter_sink_setup,
            core,
            progress,
        } = pending;
        if parse_progress
            .geometry()
            .is_none_or(|geometry| geometry.unit_count != unit_count)
        {
            return Err(tile::invalid_inter_tile_scheduling_state());
        }
        parse_progress.append_records(&mut records);
        let has_active_deblock = core
            .deblocking_filter_params
            .as_ref()
            .is_some_and(|filter| filter.apply_deblocking_filter != [false; 4]);
        let (mut filter_setup, deblock_quant_deltas) = filter_sink_setup.deferred_filter_setup(
            info,
            plane_sizes,
            InterFilterInputs {
                records,
                cdef_grid,
                ccso_grid,
                gdf_grid,
                motion_field: TemporalMotionField::empty(),
            },
            core,
            progress,
        )?;
        let deblock_records = has_active_deblock.then(|| filter_setup.detach_deblock_records());
        tile.attach_filters(
            filter_setup,
            filter_shell,
            deblock_records,
            deblock_quant_deltas,
        )
    }
}

/// The frame-level filter inputs a scheduled walk keeps until its § 8.2 pass
/// has settled the grids they are built from.
pub(crate) struct PendingFilterAttach<T: ReconSample> {
    info: splot_recon::DecodedFrameInfo,
    plane_sizes: [Option<splot_recon::PlaneSize>; 3],
    filter_sink_setup: crate::pipeline::frame_engine::finish::FilterSinkSetup,
    core: Arc<FrameHeaderCore>,
    progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
}

/// Runs the temporal prelude and builds the admission scheduler from the half
/// of the walk that is settled before the entropy pass reads a unit.
#[allow(clippy::too_many_arguments)]
pub(in crate::prediction::inter) fn prepare_scheduled_recon<T: ReconSample>(
    scratch: InterDecodeScratch<T>,
    reusable: &mut tile::ScheduledTileWorkspace<T>,
    temporal: &mut Arc<TemporalMvContext>,
    workers: Arc<tile::InterReconScratchPool<T>>,
    filter_sink_setup: crate::pipeline::frame_engine::finish::FilterSinkSetup,
    progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    sequence: Arc<SequenceHeader>,
    core: Arc<FrameHeaderCore>,
    ref_frame_idx: RefIdxBuf,
    reference: Arc<InterReferenceState<T>>,
    workspace: CurrentFrameWorkspace<T>,
    motion_handle: MotionFieldHandle,
    parse_progress: &Arc<super::tile::ParseProgress>,
    params: &tile::TileWalkParams,
    prelude: TemporalPrelude,
    motion_field: TemporalMotionField,
) -> Result<(tile::ScheduledTileRecon<T>, PendingFilterAttach<T>)> {
    let InterDecodeScratch {
        tile,
        temporal_context: _,
        frame_filter_records: _,
        buffers,
    } = scratch;
    let mut tile = tile.unwrap_or_default();
    tile.buffers = buffers;
    let context =
        Arc::get_mut(temporal).ok_or(DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
    let temporal_plan =
        prelude.begin_scheduled(context, &core, ref_frame_idx.as_slice(), &reference)?;
    let info = workspace.info();
    let plane_sizes = crate::filters::wienerns_lr::recon::plane_storage_sizes(&workspace);
    let filter_count = crate::filters::gdf::stripe_ranges(&core, filter_sink_setup.luma_height)
        .try_fold(0, |count, range| range.map(|_| count + 1))?;
    let tile = tile::prepare_scheduled_tile(
        tile,
        reusable,
        workers,
        *params,
        sequence,
        Arc::clone(&core),
        Arc::clone(temporal),
        reference,
        ref_frame_idx,
        workspace,
        filter_count,
        motion_field,
        motion_handle,
        temporal_plan,
        Arc::clone(parse_progress),
    )?;
    Ok((
        tile,
        PendingFilterAttach {
            info,
            plane_sizes,
            filter_sink_setup,
            core,
            progress,
        },
    ))
}
