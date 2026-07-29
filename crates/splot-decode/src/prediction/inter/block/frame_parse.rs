// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The block-level parse/reconstruct seam of one inter frame.
//!
//! The entropy pass reads no reference sample and no projected motion field —
//! its § 7.12.2 TIP reference pair comes from the header's order hints — so it
//! settles by the bitstream alone. [`parse_inter_frame_blocks`] runs it to the
//! end and keeps every unit, along with the frame's CDF subset and filter
//! grids, which are entropy-pass products too. What is still owed is the § 7.9
//! temporal prelude, the § 7.12 resolve pass and reconstruction, which
//! [`InterFrameParse::reconstruct`] runs once the driver reaches them.

use super::super::MotionFieldHandle;
use super::*;
use std::sync::{Mutex, PoisonError};

struct ScheduledFrameOutput {
    cdef: crate::filters::cdef::CdefUnitGrid,
    ccso: Option<crate::filters::ccso::CcsoUnitGrid>,
    gdf: Option<crate::filters::gdf::GdfBlockGrid>,
}

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
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
}

/// One parsed frame whose reconstruction units are ready for admission.
pub(crate) struct ScheduledInterReconstruction<T: ReconSample> {
    tile: tile::ScheduledTileRecon<T>,
    output: Mutex<Option<ScheduledFrameOutput>>,
}

impl<T: ReconSample> ScheduledInterReconstruction<T> {
    /// Number of independently admitted reconstruction units.
    pub(crate) const fn len(&self) -> usize {
        self.tile.len()
    }

    /// Cross-frame conditions for one reconstruction unit.
    pub(crate) fn conditions(&self, index: usize) -> Vec<splot_parallel::Condition<'_>> {
        self.tile.conditions(index)
    }

    /// Precomputes one admitted reconstruction unit.
    pub(crate) fn precompute(&self, index: usize) -> Result<()> {
        self.tile.precompute(index)
    }

    /// Commits one precomputed unit and returns the completed frame products
    /// after the final ordered commit.
    pub(crate) fn commit(
        &self,
        index: usize,
    ) -> Result<Option<(CurrentFrameWorkspace<T>, InterFilterInputs)>> {
        let Some(output) = self.tile.commit(index)? else {
            return Ok(None);
        };
        let grids = self
            .output
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| {
                inter_cap!(
                    "inter_admission_frame_output",
                    ByteOffset::new(0),
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                )
            })?;
        Ok(Some((
            output.workspace,
            InterFilterInputs {
                records: output.records,
                cdef_grid: grids.cdef,
                ccso_grid: grids.ccso,
                gdf_grid: grids.gdf,
                motion_field: TemporalMotionField::empty(),
            },
        )))
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
        first_tile_offset,
        offset: setup_offset,
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
    records.clear();
    parsed.merge_filter_state(
        &mut records,
        &mut cdef_state,
        &mut gdf_state,
        &mut ccso_state,
    )?;
    let frame_cdfs = finish_frame_cdfs(&initial_frame_cdfs, work_units, qindex, setup_offset)?;
    Ok(InterFrameParse {
        parsed,
        records,
        params,
        prelude,
        motion_field,
        frame_cdfs,
        cdef_grid: cdef_state.into_grid(first_tile_offset)?,
        ccso_grid: ccso_state.into_grid(first_tile_offset)?,
        gdf_grid: gdf_state.into_grid(first_tile_offset)?,
    })
}

impl InterFrameParse {
    /// How many parsed unit buffers the frame is holding, which bounds the
    /// split path's per-frame memory.
    pub(crate) fn unit_count(&self) -> usize {
        self.parsed.unit_count()
    }

    /// Runs the § 7.9 temporal prelude, the § 7.12 resolve pass and the frame's
    /// reconstruction, and returns what the § 7.2 filter chain reads.
    ///
    /// The prelude reads the reference frames' published motion fields, so the
    /// caller must have reconstructed every earlier frame in decode order. This
    /// frame's own field publishes through `motion_handle` as soon as its last
    /// parse unit's records land, which is before the ordered pixel commit ends.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconstruct<T: ReconSample>(
        self,
        scratch: &mut InterDecodeScratch<T>,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        reference: &InterReferenceState<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        motion_handle: MotionFieldHandle,
    ) -> Result<InterFilterInputs> {
        let Self {
            parsed,
            mut records,
            params,
            prelude,
            motion_field,
            frame_cdfs: _,
            cdef_grid,
            ccso_grid,
            gdf_grid,
        } = self;
        let InterDecodeScratch {
            tile: tile_scratch,
            temporal_context: temporal_slot,
            ..
        } = scratch;
        let temporal_context = prelude.run(
            temporal_slot.get_or_insert_with(TemporalMvContext::empty),
            core,
            ref_frame_idx,
            reference,
        )?;
        let motion_field = tile::reconstruct_parsed_tile(
            tile_scratch,
            &mut records,
            parsed,
            &params,
            sequence,
            core,
            temporal_context,
            reference,
            ref_frame_idx,
            workspace,
            motion_field,
            motion_handle,
        )?;
        Ok(InterFilterInputs {
            records,
            cdef_grid,
            ccso_grid,
            gdf_grid,
            motion_field,
        })
    }

    /// Runs the temporal prelude and resolve/motion half-pass, then returns
    /// owned reconstruction units for the admission scheduler.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_scheduled<T: ReconSample>(
        self,
        scratch: InterDecodeScratch<T>,
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
            parsed,
            records,
            params,
            prelude,
            motion_field,
            frame_cdfs: _,
            cdef_grid,
            ccso_grid,
            gdf_grid,
        } = self;
        let InterDecodeScratch {
            tile,
            mut temporal_context,
            frame_filter_records: _,
        } = scratch;
        let temporal = prelude.run(
            temporal_context.get_or_insert_with(TemporalMvContext::empty),
            &core,
            &ref_frame_idx,
            &reference,
        )?;
        let temporal_scratch = temporal.take_scratch();
        let temporal = Arc::new(core::mem::replace(temporal, TemporalMvContext::empty()));
        let tile = tile::prepare_scheduled_tile(
            tile,
            records,
            parsed,
            params,
            sequence,
            core,
            Arc::clone(&temporal),
            reference,
            ref_frame_idx,
            workspace,
            motion_field,
            motion_handle,
        )?;
        Ok((
            ScheduledInterReconstruction {
                tile,
                output: Mutex::new(Some(ScheduledFrameOutput {
                    cdef: cdef_grid,
                    ccso: ccso_grid,
                    gdf: gdf_grid,
                })),
            },
            temporal_scratch,
        ))
    }
}
