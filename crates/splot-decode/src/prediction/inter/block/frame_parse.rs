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
//! [`InterFrameParse::prepare_scheduled`] converts that work into the row graph
//! once the frame becomes admissible.

use super::super::MotionFieldHandle;
use super::super::find_mv_stack::TemporalMotionFieldMetadata;
use super::*;
use std::sync::Arc;

#[cfg(test)]
mod scheduled_frame;

/// One inter frame after its entropy pass, owned so its reconstruction can run
/// after the driver has moved on to the next frame's parse.
pub(crate) struct InterFrameParse {
    parsed: tile::ParsedTile,
    records: crate::filters::wienerns_lr::FrameFilterRecords,
    params: tile::TileWalkParams,
    prelude: TemporalPrelude,
    motion_field: TemporalMotionField,
    /// The frame's end-of-walk CDF subset, which the reference update reads
    /// while the reconstruction is still owed.
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    cdef_grid: crate::filters::cdef::CdefUnitGrid,
    /// The walk-parsed CCSO unit grid, retained for the reference update.
    pub(crate) ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    pub(crate) segment_ids: FrameSegmentIdMap,
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
}

/// One parsed frame whose reconstruction units are ready for admission.
pub(crate) struct ScheduledInterReconstruction<T: ReconSample> {
    tile: tile::ScheduledTileRecon<T>,
}

/// Filter work made ready by one canonical reconstruction commit.
pub(crate) struct ScheduledFrameProgress<T: ReconSample> {
    pub(crate) filters: Vec<crate::filters::wienerns_lr::recon::OwnedFilterJob<T>>,
    pub(crate) output: Option<crate::filters::wienerns_lr::recon::OwnedFilterFinish<T>>,
}

impl<T: ReconSample> From<tile::ScheduledTileProgress<T>> for ScheduledFrameProgress<T> {
    fn from(progress: tile::ScheduledTileProgress<T>) -> Self {
        Self {
            filters: progress.filters,
            output: progress.output.map(|output| output.filter),
        }
    }
}

impl<T: ReconSample> ScheduledInterReconstruction<T> {
    /// Number of independently admitted reconstruction units.
    pub(crate) const fn len(&self) -> usize {
        self.tile.len()
    }

    /// Number of independently scheduled final-filter stripes.
    pub(crate) const fn filter_count(&self) -> usize {
        self.tile.filter_count()
    }

    pub(crate) fn resolve_len(&self) -> usize {
        self.tile.resolve_len()
    }

    pub(crate) fn resolve_conditions(&self, index: usize) -> Vec<splot_parallel::Condition<'_>> {
        self.tile.resolve_conditions(index)
    }

    pub(crate) fn resolve(&self, index: usize) -> Result<core::ops::Range<usize>> {
        self.tile.resolve(index)
    }

    pub(crate) fn fail_temporal(&self) {
        self.tile.fail_temporal();
    }

    /// Cross-frame conditions for one reconstruction unit.
    pub(crate) fn conditions(&self, index: usize) -> Vec<splot_parallel::Condition<'_>> {
        self.tile.conditions(index)
    }

    /// Precomputes one admitted reconstruction unit.
    pub(crate) fn precompute(&self, index: usize) -> Result<()> {
        self.tile.precompute(index)
    }

    /// Commits one precomputed unit and returns the frontier links its
    /// canonical rows released.
    pub(crate) fn commit(&self, index: usize) -> Result<tile::ScheduledCommitProgress> {
        self.tile.commit(index)
    }

    pub(crate) fn take_scheduled_scratch(&self) -> Result<InterDecodeScratch<T>> {
        self.tile
            .take_scheduled_scratch()
            .map(InterDecodeScratch::from_scheduled_tile_scratch)
    }

    /// Number of ordered links in this frame's § 7.17 frontier chain.
    pub(crate) const fn frontier_len(&self) -> usize {
        self.tile.frontier_len()
    }

    /// Advances the § 7.17 frontier over one sealed superblock row and returns
    /// the completed frame products after the final link.
    pub(crate) fn frontier(&self, row: usize) -> Result<ScheduledFrameProgress<T>> {
        Ok(self.tile.frontier(row)?.into())
    }
}

/// Runs one inter frame's entropy pass to the end.
///
/// The frame must have exactly one tile: a multi-tile frame already parses its
/// tiles in parallel, and the driver gates on the header's tile counts before
/// choosing this path, so a work-unit count of anything but one is an internal
/// invariant violation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_inter_frame_blocks<T: ReconSample>(
    tile_plan: &mut crate::bitstream::tile_payload::DecodeTilePayloadPlan<'_>,
    mut records: crate::filters::wienerns_lr::FrameFilterRecords,
    frame_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    options: &DecodeOptions,
    facts: InterBlockFacts,
    ref_frame_idx: &[u32],
    reference: &InterReferenceState<T>,
    workspace: &CurrentFrameWorkspace<T>,
) -> Result<InterFrameParse> {
    let offset = frame_envelope.offset;
    let work_units = tile_plan.work_units_mut();
    let setup = derive_inter_block_setup(
        work_units,
        frame_envelope,
        sequence,
        core,
        options,
        facts,
        ref_frame_idx,
        reference,
        workspace,
    )?;
    let [tile] = work_units else {
        return Err(inter_cap!(
            "inter_split_walk_tile_count",
            offset,
            "inter.tile_count == 1 for the split walk",
            SPEC_MODE_INFO
        ));
    };
    let InterBlockSetup {
        params,
        prelude,
        mut cdef_state,
        mut gdf_state,
        mut ccso_state,
        motion_field,
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
        true,
    )?;
    let mut segment_ids = FrameSegmentIdMap::new(params.mi_rows, params.mi_cols).map_err(|_| {
        inter_missing!(
            "inter_segment_id_frame_grid",
            offset,
            "inter.segment_id_frame_grid",
            SPEC_MODE_INFO
        )
    })?;
    records.clear();
    parsed.merge_filter_state(
        &mut records,
        &mut cdef_state,
        &mut gdf_state,
        &mut ccso_state,
        &mut segment_ids,
    )?;
    let frame_cdfs = finish_frame_cdfs(&initial_frame_cdfs, work_units, qindex);
    let ccso_grid = ccso_state.into_grid()?;
    let segment_ids =
        final_segment_ids(core, reference, params.mi_rows, params.mi_cols, segment_ids);
    Ok(InterFrameParse {
        parsed,
        records,
        params,
        prelude,
        motion_field,
        frame_cdfs,
        cdef_grid: cdef_state.into_grid()?,
        ccso_grid,
        segment_ids,
        gdf_grid: gdf_state.into_grid()?,
    })
}

impl InterFrameParse {
    /// How many parsed unit buffers the frame is holding, which bounds the
    /// split path's per-frame memory.
    pub(crate) fn unit_count(&self) -> usize {
        self.parsed.unit_count()
    }

    /// Returns the semantic motion metadata derived by this entropy pass.
    pub(crate) fn motion_field_metadata(&self) -> TemporalMotionFieldMetadata {
        self.motion_field.metadata()
    }

    /// Runs the temporal prelude and resolve/motion half-pass, then returns
    /// owned reconstruction units for the admission scheduler.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_scheduled<T: ReconSample>(
        self,
        scratch: InterDecodeScratch<T>,
        filter_sink_setup: crate::pipeline::frame_engine::finish::FilterSinkSetup,
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
        sequence: Arc<SequenceHeader>,
        core: Arc<FrameHeaderCore>,
        ref_frame_idx: Arc<[u32]>,
        reference: Arc<InterReferenceState<T>>,
        workspace: CurrentFrameWorkspace<T>,
        motion_handle: MotionFieldHandle,
    ) -> Result<(
        ScheduledInterReconstruction<T>,
        super::super::find_mv_stack::TemporalMvScratch,
    )> {
        let Self {
            mut parsed,
            mut records,
            params,
            prelude,
            motion_field,
            frame_cdfs: _,
            cdef_grid,
            ccso_grid,
            segment_ids: _,
            gdf_grid,
        } = self;
        let InterDecodeScratch {
            tile,
            temporal_context,
            frame_filter_records: _,
        } = scratch;
        let mut temporal = temporal_context.unwrap_or_else(TemporalMvContext::empty);
        let temporal_plan =
            prelude.begin_scheduled(&mut temporal, &core, &ref_frame_idx, &reference)?;
        let temporal_scratch = temporal.take_scratch();
        let temporal = Arc::new(temporal);
        parsed.detach_filter_records(&mut records);
        let has_active_deblock = core
            .deblocking_filter_params
            .as_ref()
            .is_some_and(|filter| {
                std::env::var_os("SPLOT_DECODE_SKIP_FILTERS").is_none()
                    && filter.apply_deblocking_filter != [false; 4]
            });
        let (mut filter_setup, workspace, deblock_quant_deltas) = filter_sink_setup
            .owned_filter_setup(
                workspace,
                InterFilterInputs {
                    records,
                    cdef_grid,
                    ccso_grid,
                    gdf_grid,
                    motion_field: TemporalMotionField::empty(),
                },
                Arc::clone(&core),
                progress,
            )?;
        let deblock_records = has_active_deblock.then(|| filter_setup.detach_deblock_records());
        let tile = tile::prepare_scheduled_tile(
            tile,
            parsed,
            params,
            sequence,
            core,
            Arc::clone(&temporal),
            reference,
            ref_frame_idx,
            workspace,
            filter_setup,
            deblock_records,
            deblock_quant_deltas,
            motion_field,
            motion_handle,
            temporal_plan,
        )?;
        Ok((ScheduledInterReconstruction { tile }, temporal_scratch))
    }
}
