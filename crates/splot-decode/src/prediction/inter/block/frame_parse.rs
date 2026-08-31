// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The block-level parse/reconstruct seam of one inter frame.
//!
//! The entropy pass reads no reference sample and no projected motion field —
//! its § 7.12.2 TIP reference pair comes from the header's order hints — so it
//! settles by the bitstream alone. [`parse_inter_frame_blocks`] runs it to the
//! end and keeps every unit, along with the frame's CDF subset and filter
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
    parsed: tile::ParsedTile,
    records: crate::filters::wienerns_lr::FrameFilterRecords,
    /// The end-of-walk CDF subset published to the canonical `PipelineFrame`.
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    cdef_grid: crate::filters::cdef::CdefUnitGrid,
    /// The walk-parsed CCSO unit grid published to the canonical `PipelineFrame`.
    pub(crate) ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    pub(crate) segment_ids: Arc<FrameSegmentIdMap>,
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
}

/// Runs one inter frame's entropy pass to the end.
///
/// The frame must have exactly one tile: a multi-tile frame already parses its
/// tiles in parallel, and the driver gates on the header's tile counts before
/// choosing this path, so a work-unit count of anything but one is an internal
/// invariant violation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_inter_frame_blocks<T: ReconSample>(
    tile: &mut crate::bitstream::tile_payload::DecodeTileWorkUnit<'_>,
    mut records: crate::filters::wienerns_lr::FrameFilterRecords,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    parse_progress: &Arc<tile::ParseProgress>,
    setup: super::InterParseSetup,
) -> Result<InterFrameParse> {
    let super::InterParseSetup {
        params,
        mut cdef_state,
        mut gdf_state,
        mut ccso_state,
        initial_frame_cdfs,
        qindex,
    } = setup;
    let mut parsed = tile::parse_tile_units(
        tile,
        &params,
        sequence,
        core,
        reference,
        ref_frame_idx,
        &cdef_state,
        &gdf_state,
        &ccso_state,
        parse_progress,
    )?;
    let mut segment_ids = frame_segment_id_map(params.mi_rows, params.mi_cols)?;
    records.clear();
    parsed.merge_filter_state(
        &mut records,
        &mut cdef_state,
        &mut gdf_state,
        &mut ccso_state,
        &mut segment_ids,
    )?;
    let frame_cdfs = finish_frame_cdfs(&initial_frame_cdfs, std::slice::from_mut(tile), qindex);
    let ccso_grid = ccso_state.into_grid()?;
    let segment_ids =
        final_segment_ids(core, reference, params.mi_rows, params.mi_cols, segment_ids);
    Ok(InterFrameParse {
        parsed,
        records,
        frame_cdfs,
        cdef_grid: cdef_state.into_grid()?,
        ccso_grid,
        segment_ids,
        gdf_grid: gdf_state.into_grid()?,
    })
}

impl InterFrameParse {
    /// Hands the frontier the filter state the § 8.2 pass settles last.
    ///
    /// Reconstruction is already admitted by the time this runs; only the
    /// § 7.17 frontier chain waits on it.
    pub(in crate::prediction::inter) fn attach_filters<T: ReconSample>(
        self,
        pending: PendingFilterAttach<T>,
        tile: &tile::ScheduledTileRecon<T>,
        parse_progress: &Arc<super::tile::ParseProgress>,
    ) -> Result<()> {
        let Self {
            parsed,
            mut records,
            frame_cdfs: _,
            cdef_grid,
            ccso_grid,
            segment_ids: _,
            gdf_grid,
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
            .is_none_or(|geometry| geometry.unit_count != parsed.unit_count())
        {
            return Err(tile::invalid_inter_tile_scheduling_state());
        }
        records.append(&mut parse_progress.take_records());
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
        tile.attach_filters(filter_setup, deblock_records, deblock_quant_deltas)
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
) -> Result<(
    tile::ScheduledTileRecon<T>,
    super::super::find_mv_stack::TemporalMvScratch,
    PendingFilterAttach<T>,
)> {
    let InterDecodeScratch {
        tile,
        temporal_context,
        frame_filter_records: _,
    } = scratch;
    let mut temporal = temporal_context.unwrap_or_else(TemporalMvContext::empty);
    let temporal_plan =
        prelude.begin_scheduled(&mut temporal, &core, ref_frame_idx.as_slice(), &reference)?;
    let temporal_scratch = temporal.take_scratch();
    let temporal = Arc::new(temporal);
    let info = workspace.info();
    let plane_sizes = crate::filters::wienerns_lr::recon::plane_storage_sizes(&workspace);
    let filter_count =
        crate::filters::gdf::stripe_ranges(&core, filter_sink_setup.luma_height)?.len();
    let tile = tile::prepare_scheduled_tile(
        tile,
        *params,
        sequence,
        Arc::clone(&core),
        Arc::clone(&temporal),
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
        temporal_scratch,
        PendingFilterAttach {
            info,
            plane_sizes,
            filter_sink_setup,
            core,
            progress,
        },
    ))
}
