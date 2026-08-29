// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Wiener-NS loop-restoration final-filter sink.
//!
//! [`WienerNsLrReconSink`] wraps a reconstructed [`CurrentFrameWorkspace`] produced
//! by the unified decode engine and runs the shared §7.2 in-loop filter chain
//! (deblock → CDEF → CCSO → loop-restoration) over it via
//! [`WienerNsLrReconSink::into_filtered_frame`]. The module also exposes the
//! §7.13.2.17 intra edge-filter-strength selection and the §7.17 chroma-transform
//! deblock helper used by the surrounding filter code.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{BitDepth, CurrentFrameWorkspace, DecodedFrame, PlaneId, ReconSample};
use std::sync::{Arc, Mutex};

use crate::Result;

const MI_SIZE: usize = 4;

/// Where one frame's filtered stripes are published.
///
/// A pipelined frame publishes into the [`FrameProgress`] its slot already
/// shares, so a consumer can read the published prefix before the freeze. An
/// inline frame creates the same sink privately, keeping one direct stripe
/// output path.
///
/// [`FrameProgress`]: crate::pipeline::frame_progress::FrameProgress
struct FilteredFrameSink<'a, 'job, T: ReconSample> {
    progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    admit: Option<&'a dyn splot_parallel::Admit<'job>>,
}

impl<'a, 'job, T: ReconSample> FilteredFrameSink<'a, 'job, T> {
    fn open(
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
        admit: Option<&'a dyn splot_parallel::Admit<'job>>,
        info: splot_recon::DecodedFrameInfo,
        ranges: &[(usize, usize)],
    ) -> Result<Self> {
        let progress = match progress {
            Some(progress) => progress,
            None => Arc::new(crate::pipeline::frame_progress::FrameProgress::new(info)?),
        };
        if !progress.begin(ranges) {
            return Err(lr_pipeline_state_error());
        }
        Ok(Self { progress, admit })
    }

    /// Lends one stripe's exact output region.
    fn direct_stripe(
        &self,
        stripe: usize,
    ) -> Result<crate::pipeline::frame_progress::DirectStripeLease<T>> {
        self.progress
            .direct_stripe(stripe)
            .ok_or_else(lr_pipeline_state_error)
    }

    fn publish_stripe(
        &self,
        mut samples: final_filters::FilteredStripe,
        direct: crate::pipeline::frame_progress::DirectStripeLease<T>,
    ) -> Result<()> {
        if !samples.y.is_direct()
            || samples.u.as_ref().is_some_and(|plane| !plane.is_direct())
            || samples.v.as_ref().is_some_and(|plane| !plane.is_direct())
        {
            return Err(lr_pipeline_state_error());
        }
        samples
            .y
            .finish_direct()
            .map_err(|_| lr_pipeline_state_error())?;
        if let Some(plane) = samples.u.as_mut() {
            plane
                .finish_direct()
                .map_err(|_| lr_pipeline_state_error())?;
        }
        if let Some(plane) = samples.v.as_mut() {
            plane
                .finish_direct()
                .map_err(|_| lr_pipeline_state_error())?;
        }
        drop(samples);
        if !direct.submit() {
            return Err(lr_pipeline_state_error());
        }
        if let Some(admit) = self.admit {
            admit.admit_ready();
        }
        Ok(())
    }

    /// Freezes the filtered output and hands the frozen frame to `publish`.
    ///
    /// A published frame freezes under its own lock, so the slot `publish`
    /// settles is visible to every consumer before the published prefix stops
    /// being readable.
    fn freeze<R>(self, publish: impl FnOnce(DecodedFrame<T>) -> R) -> Result<R> {
        self.progress.freeze_workspace(publish)
    }
}

pub(crate) struct WienerNsLrReconSink<T: ReconSample> {
    workspace: Option<CurrentFrameWorkspace<T>>,
    info: splot_recon::DecodedFrameInfo,
    plane_sizes: [Option<splot_recon::PlaneSize>; 3],
    bit_depth: BitDepth,
    cfl_ds_filter_index: u8,
    luma_width: usize,
    luma_height: usize,
    filter_records: super::FrameFilterRecords,
    cdef_grid: Option<crate::filters::cdef::CdefUnitGrid>,
    ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
    tx_skip_grid: Option<super::WienerNsLrTxSkipGrid>,
    gdf_reference: Option<crate::filters::gdf::GdfReferenceContext>,
    lossless_grid: Option<crate::filters::lossless::LosslessBlockGrid>,
}

/// One frame's owned final-filter state after reconstruction and deblock have
/// separated their sample ownership.
///
/// Every immutable grid and filter record lives here, so an owned deblocked
/// window can run on any worker without borrowing the reconstructed workspace.
/// The output sink and pending stripe list stay frame-owned until the single
/// terminal freeze consumes this value.
pub(crate) struct OwnedFilterSetup<'progress, 'job, T: ReconSample> {
    core: Arc<splot_core::headers::frame::FrameHeaderCore>,
    disable_loopfilters_across_tiles: bool,
    mi_rows: usize,
    mi_cols: usize,
    subsampling: (usize, usize),
    bit_depth: BitDepth,
    cfl_ds_filter_index: u8,
    luma_width: usize,
    luma_height: usize,
    pixel_format: splot_recon::PixelFormat,
    cdef_grid: Option<crate::filters::cdef::CdefUnitGrid>,
    cdef_skip_grid: Option<crate::filters::cdef::CdefSkipGrid>,
    cdef_strengths: Option<Vec<crate::filters::cdef::CdefFrameParams>>,
    ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    ccso_config: Option<crate::filters::ccso::CcsoFrameConfig>,
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
    tx_skip_grid: Option<super::WienerNsLrTxSkipGrid>,
    gdf_reference: Option<crate::filters::gdf::GdfReferenceContext>,
    lossless_grid: Option<crate::filters::lossless::LosslessBlockGrid>,
    plane_sizes: [Option<splot_recon::PlaneSize>; 3],
    max_sample_fits: bool,
    lr_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    lr_plane_ends: [usize; 2],
    lr_unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    ranges: Vec<(usize, usize)>,
    filter_records: super::FrameFilterRecords,
    sink: FilteredFrameSink<'progress, 'job, T>,
    window_sequence: Mutex<crate::filters::source::DeblockedWindowSequence<T>>,
    stripe_state: Mutex<Vec<StripeLifecycle>>,
    deblock_records: Mutex<Option<crate::filters::deblock::OwnedDeblockRecords>>,
}

/// One completed stripe whose index and samples move together into publication.
pub(crate) struct OwnedFilteredStripe<T: ReconSample> {
    stripe: usize,
    frame: final_filters::FilteredStripe,
    direct: crate::pipeline::frame_progress::DirectStripeLease<T>,
}

/// One scheduled stripe with sole ownership of its deblocked input window.
pub(crate) struct OwnedFilterJob<T: ReconSample> {
    setup: Arc<OwnedFilterSetup<'static, 'static, T>>,
    stripe: usize,
    source: crate::filters::source::DeblockedStripe<T>,
}

/// The sole setup owner after every scheduled stripe has settled.
pub(crate) struct OwnedFilterFinish<T: ReconSample> {
    setup: Arc<OwnedFilterSetup<'static, 'static, T>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StripeLifecycle {
    Pending,
    Claimed,
    Submitted,
}

#[allow(clippy::if_same_then_else)]
pub(crate) fn intra_edge_filter_strength(w: u32, h: u32, filter_type: u8, delta: i32) -> u8 {
    let d = delta.unsigned_abs();
    let blk_wh = w + h;
    let mut strength = 0u8;
    if filter_type == 0 {
        if blk_wh <= 8 {
            if d >= 56 {
                strength = 1;
            }
        } else if blk_wh <= 12 {
            if d >= 40 {
                strength = 1;
            }
        } else if blk_wh <= 16 {
            if d >= 40 {
                strength = 1;
            }
        } else if blk_wh <= 24 {
            if d >= 8 {
                strength = 1;
            }
            if d >= 16 {
                strength = 2;
            }
            if d >= 32 {
                strength = 3;
            }
        } else if blk_wh <= 32 {
            strength = 1;
            if d >= 4 {
                strength = 2;
            }
            if d >= 32 {
                strength = 3;
            }
        } else {
            strength = 3;
        }
    } else if blk_wh <= 8 {
        if d >= 40 {
            strength = 1;
        }
        if d >= 64 {
            strength = 2;
        }
    } else if blk_wh <= 16 {
        if d >= 20 {
            strength = 1;
        }
        if d >= 48 {
            strength = 2;
        }
    } else if blk_wh <= 24 {
        if d >= 4 {
            strength = 3;
        }
    } else {
        strength = 3;
    }
    strength
}

/// The three plane storage sizes a filter setup needs from a workspace.
pub(crate) fn plane_storage_sizes<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
) -> [Option<splot_recon::PlaneSize>; 3] {
    [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| {
        workspace
            .plane(plane)
            .ok()
            .map(splot_recon::CurrentFramePlane::storage_size)
    })
}

impl<T: ReconSample> WienerNsLrReconSink<T> {
    pub(crate) fn for_final_filtering(
        workspace: CurrentFrameWorkspace<T>,
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
    ) -> Self {
        let info = workspace.info();
        let plane_sizes = plane_storage_sizes(&workspace);
        Self {
            workspace: Some(workspace),
            ..Self::for_deferred_filtering(info, plane_sizes, luma_width, luma_height, bit_depth)
        }
    }

    /// Builds the sink for a frame whose workspace the reconstruction already
    /// owns, which is every scheduled walk: the setup only ever reads the two
    /// snapshots taken here.
    pub(crate) fn for_deferred_filtering(
        info: splot_recon::DecodedFrameInfo,
        plane_sizes: [Option<splot_recon::PlaneSize>; 3],
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
    ) -> Self {
        Self {
            workspace: None,
            info,
            plane_sizes,
            bit_depth,
            cfl_ds_filter_index: 0,
            luma_width,
            luma_height,
            filter_records: super::FrameFilterRecords::default(),
            cdef_grid: None,
            ccso_grid: None,
            gdf_grid: None,
            tx_skip_grid: None,
            gdf_reference: None,
            lossless_grid: None,
        }
    }

    pub(crate) fn set_filter_records(&mut self, records: super::FrameFilterRecords) {
        self.filter_records = records;
    }

    /// Returns the geometry the frozen frame will report: the filter chain
    /// publishes into a workspace built from this one's metadata, so the
    /// decoded-frame info is known before the samples are filtered.
    pub(crate) const fn frame_info(&self) -> splot_recon::DecodedFrameInfo {
        self.info
    }

    pub(crate) fn set_cdef_grid(&mut self, grid: Option<crate::filters::cdef::CdefUnitGrid>) {
        self.cdef_grid = grid;
    }

    pub(crate) fn set_ccso_grid(&mut self, grid: Option<crate::filters::ccso::CcsoUnitGrid>) {
        self.ccso_grid = grid;
    }

    pub(crate) fn set_gdf_grid(&mut self, grid: Option<crate::filters::gdf::GdfBlockGrid>) {
        self.gdf_grid = grid;
    }

    pub(crate) const fn set_cfl_ds_filter_index(&mut self, index: u8) {
        self.cfl_ds_filter_index = index;
    }

    pub(crate) const fn set_gdf_reference_context(
        &mut self,
        context: Option<crate::filters::gdf::GdfReferenceContext>,
    ) {
        self.gdf_reference = context;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn into_filtered_frame<R>(
        self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
        admit: Option<&dyn splot_parallel::Admit<'_>>,
        publish: impl FnOnce(DecodedFrame<T>) -> R,
    ) -> Result<(R, super::FrameFilterRecords)> {
        self.into_filtered_frame_inner(
            core,
            disable_loopfilters_across_tiles,
            deblock_quant_deltas,
            progress,
            admit,
            false,
            publish,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn into_filtered_frame_from_deblocked<R>(
        self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
        admit: Option<&dyn splot_parallel::Admit<'_>>,
        publish: impl FnOnce(DecodedFrame<T>) -> R,
    ) -> Result<(R, super::FrameFilterRecords)> {
        self.into_filtered_frame_inner(
            core,
            disable_loopfilters_across_tiles,
            deblock_quant_deltas,
            progress,
            admit,
            true,
            publish,
        )
    }

    pub(crate) fn into_owned_filter_setup<'progress, 'job>(
        self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
        admit: Option<&'progress dyn splot_parallel::Admit<'job>>,
    ) -> Result<(
        OwnedFilterSetup<'progress, 'job, T>,
        Option<CurrentFrameWorkspace<T>>,
    )> {
        self.into_owned_filter_setup_inner(core, disable_loopfilters_across_tiles, progress, admit)
    }

    /// Builds a filter setup whose progressive output handle is owned by
    /// scheduled row tasks.
    pub(crate) fn into_owned_filter_setup_published(
        self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    ) -> Result<(
        OwnedFilterSetup<'static, 'static, T>,
        Option<CurrentFrameWorkspace<T>>,
    )> {
        self.into_owned_filter_setup_inner(
            core,
            disable_loopfilters_across_tiles,
            Some(progress),
            None,
        )
    }

    fn into_owned_filter_setup_inner<'progress, 'job>(
        mut self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
        admit: Option<&'progress dyn splot_parallel::Admit<'job>>,
    ) -> Result<(
        OwnedFilterSetup<'progress, 'job, T>,
        Option<CurrentFrameWorkspace<T>>,
    )> {
        let mi_rows = self.luma_height.div_ceil(MI_SIZE);
        let mi_cols = self.luma_width.div_ceil(MI_SIZE);
        if core
            .lossless_info
            .as_ref()
            .is_some_and(|lossless| lossless.has_lossless_segment)
        {
            self.lossless_grid = Some(
                crate::filters::lossless::LosslessBlockGrid::from_deblock_records(
                    mi_rows,
                    mi_cols,
                    &self.filter_records.deblock_blocks,
                    &self.filter_records.chroma_deblock_blocks,
                )
                .map_err(|error| match error {
                    crate::filters::lossless::LosslessGridError::Allocation(plane) => {
                        splot_recon::ReconError::WorkspaceAllocationFailed {
                            plane,
                            context: "lossless block grid",
                        }
                        .into()
                    }
                    crate::filters::lossless::LosslessGridError::Geometry => {
                        lr_pipeline_state_error()
                    }
                })?,
            );
        }
        if self.needs_lr_tx_skip_grid(&core) {
            self.ensure_tx_skip_grid(mi_rows, mi_cols)?;
        }
        let cdef_skip_grid = self.cdef_skip_grid(&core, mi_rows, mi_cols)?;
        let cdef_strengths = crate::filters::cdef::cdef_frame_strengths(&core);
        let lr_source_blocks = core::mem::take(&mut self.filter_records.lr_source_blocks);
        let lr_unit_filters = core::mem::take(&mut self.filter_records.lr_unit_filters);
        let (lr_source_blocks, lr_plane_ends) =
            final_filters::coalesced_lr_source_rows_all(lr_source_blocks);
        let ranges = crate::filters::gdf::stripe_ranges(&core, self.luma_height)?;
        let info = self.info;
        let pixel_format = info.pixel_format();
        let subsampling = (
            usize::from(pixel_format.subsampling_x()),
            usize::from(pixel_format.subsampling_y()),
        );
        let ccso_config = self
            .ccso_grid
            .as_ref()
            .map(|grid| {
                crate::filters::ccso::prepare_ccso(&core, grid, self.bit_depth, subsampling)
            })
            .transpose()
            .map_err(|error| ccso_filter_error(&error))?;
        let sink = FilteredFrameSink::open(progress, admit, info, &ranges)?;
        let stripe_count = ranges.len();
        let plane_sizes = self.plane_sizes;

        let Self {
            workspace,
            info: _,
            plane_sizes: _,
            bit_depth,
            cfl_ds_filter_index,
            luma_width,
            luma_height,
            filter_records,
            cdef_grid,
            ccso_grid,
            gdf_grid,
            tx_skip_grid,
            gdf_reference,
            lossless_grid,
        } = self;
        Ok((
            OwnedFilterSetup {
                core,
                disable_loopfilters_across_tiles,
                mi_rows,
                mi_cols,
                subsampling,
                bit_depth,
                cfl_ds_filter_index,
                luma_width,
                luma_height,
                pixel_format,
                cdef_grid,
                cdef_skip_grid,
                cdef_strengths,
                ccso_grid,
                ccso_config,
                gdf_grid,
                tx_skip_grid,
                gdf_reference,
                lossless_grid,
                plane_sizes,
                max_sample_fits: T::try_from_u16(bit_depth.max_sample()).is_ok(),
                lr_source_blocks,
                lr_plane_ends,
                lr_unit_filters,
                ranges,
                filter_records,
                sink,
                window_sequence: Mutex::new(
                    crate::filters::source::DeblockedWindowSequence::default(),
                ),
                stripe_state: Mutex::new(vec![StripeLifecycle::Pending; stripe_count]),
                deblock_records: Mutex::new(None),
            },
            workspace,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn into_filtered_frame_inner<R>(
        self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
        admit: Option<&dyn splot_parallel::Admit<'_>>,
        deblocked: bool,
        publish: impl FnOnce(DecodedFrame<T>) -> R,
    ) -> Result<(R, super::FrameFilterRecords)> {
        if std::env::var_os("SPLOT_DECODE_SKIP_FILTERS").is_some() {
            let workspace = self.workspace.ok_or_else(lr_pipeline_state_error)?;
            return Ok((publish(workspace.freeze()?), self.filter_records));
        }
        let (setup, workspace) =
            self.into_owned_filter_setup(core, disable_loopfilters_across_tiles, progress, admit)?;
        let mut workspace = workspace.ok_or_else(lr_pipeline_state_error)?;
        let mi_rows = setup.mi_rows;
        let mi_cols = setup.mi_cols;
        let bit_depth = setup.bit_depth;
        let filter_timer = crate::timing::start();
        let mut sections = if deblocked {
            None
        } else {
            match setup.core.deblocking_filter_params {
                Some(filter) => crate::filters::deblock::FrameDeblock::prepare(
                    &setup.filter_records.deblock_blocks,
                    &setup.filter_records.chroma_deblock_blocks,
                    mi_rows,
                    mi_cols,
                    filter,
                    setup.core.tile_info.as_ref(),
                    disable_loopfilters_across_tiles,
                    deblock_quant_deltas,
                )
                .map_err(|error| deblock_prepare_error(&error))?,
                None => None,
            }
        };
        if setup.stripe_ranges().len() > 1 && splot_parallel::on_multiworker_pool() {
            if let Some(sections) = sections.as_mut() {
                let prime_timer = crate::timing::start();
                sections
                    .prime_vertical_pass(&mut workspace, bit_depth)
                    .map_err(|_| lr_pipeline_state_error())?;
                crate::timing::report("filter_deblock_prime", prime_timer);
            }
            let mut source = crate::filters::source::DeblockedSource::new(workspace);
            if sections.is_none() && !source.publish_final_rows(setup.luma_height) {
                return Err(lr_pipeline_state_error());
            }
            let mut slots: Vec<Option<Result<()>>> =
                (0..setup.stripe_ranges().len()).map(|_| None).collect();
            let mut owed: Option<crate::error::DecodeError> = None;
            let scheduled = splot_parallel::ready_task_scope(|scope| {
                for ((stripe, range), slot) in
                    setup.stripe_ranges().iter().enumerate().zip(&mut slots)
                {
                    if let Err(error) = advance_deblock_for_stripe(
                        sections.as_mut(),
                        &mut source,
                        range,
                        setup.subsampling.1,
                        bit_depth,
                    ) {
                        owed = Some(error);
                        return;
                    }
                    let lease = match sections.as_ref() {
                        Some(sections) => match setup.lease_ready_rows(stripe, sections, &source) {
                            Ok(Some(lease)) => lease,
                            Ok(None) => {
                                owed = Some(lr_pipeline_state_error());
                                return;
                            }
                            Err(error) => {
                                owed = Some(error);
                                return;
                            }
                        },
                        None => match setup.lease_terminal_rows(stripe, &source) {
                            Ok(lease) => lease,
                            Err(error) => {
                                owed = Some(error);
                                return;
                            }
                        },
                    };
                    let source = crate::filters::source::DeblockedStripe::Lease(lease);
                    let setup = &setup;
                    scope.spawn(move |_| {
                        *slot = Some(
                            setup
                                .run_owned_source(stripe, source)
                                .and_then(|filtered| setup.publish(filtered)),
                        );
                    });
                }
            });
            if let Some(error) = owed {
                return Err(error);
            }
            let missing = lr_pipeline_state_error;
            scheduled.map_err(|_| missing())?;
            for slot in slots {
                slot.unwrap_or_else(|| Err(missing()))?;
            }
        } else {
            let mut source = crate::filters::source::DeblockedSource::new(workspace);
            if let Some(sections) = sections.as_mut() {
                let deblock_timer = crate::timing::start();
                sections
                    .advance_source(&mut source, mi_rows, bit_depth)
                    .map_err(|_| lr_pipeline_state_error())?;
                crate::timing::accumulate(crate::timing::Phase::FilterDeblock, deblock_timer);
            } else if !source.publish_final_rows(setup.luma_height) {
                return Err(lr_pipeline_state_error());
            }
            let mut lease = None;
            for stripe in 0..setup.stripe_ranges().len() {
                match lease.as_mut() {
                    Some(lease) => setup.retarget_terminal_rows(stripe, &source, lease)?,
                    None => lease = Some(setup.lease_terminal_rows(stripe, &source)?),
                }
                let filtered = setup.run_borrowed_lease(
                    stripe,
                    lease.as_ref().ok_or_else(lr_pipeline_state_error)?,
                )?;
                setup.publish(filtered)?;
            }
        }
        if let Some(sections) = sections {
            sections.finish();
        }
        crate::timing::report("filter_stripes", filter_timer);
        let freeze_timer = crate::timing::start();
        let frame = setup.finish(publish)?;
        crate::timing::accumulate(crate::timing::Phase::FilterFreeze, freeze_timer);
        Ok(frame)
    }
}

impl<T: ReconSample> OwnedFilterSetup<'_, '_, T> {
    /// Transfers deblock geometry to the incremental deblock owner after every
    /// setup grid that reads it has been derived.
    pub(crate) fn detach_deblock_records(
        &mut self,
    ) -> crate::filters::deblock::OwnedDeblockRecords {
        crate::filters::deblock::OwnedDeblockRecords {
            blocks: core::mem::take(&mut self.filter_records.deblock_blocks),
            chroma: core::mem::take(&mut self.filter_records.chroma_deblock_blocks),
        }
    }

    fn chain(&self) -> StripeChain<'_> {
        StripeChain {
            bit_depth: self.bit_depth,
            cfl_ds_filter_index: self.cfl_ds_filter_index,
            luma_width: self.luma_width,
            luma_height: self.luma_height,
            pixel_format: self.pixel_format,
            cdef_grid: self.cdef_grid.as_ref(),
            ccso_grid: self.ccso_grid.as_ref(),
            gdf_grid: self.gdf_grid.as_ref(),
            tx_skip_grid: self.tx_skip_grid.as_ref(),
            gdf_reference: self.gdf_reference,
            lossless_grid: self.lossless_grid.as_ref(),
            plane_sizes: self.plane_sizes,
            max_sample_fits: self.max_sample_fits,
        }
    }

    /// Returns the ordered luma ranges this frame still owes.
    pub(crate) fn stripe_ranges(&self) -> &[(usize, usize)] {
        &self.ranges
    }

    /// Leases one stripe's final rows directly from contiguous deblock storage.
    pub(crate) fn lease_ready_rows(
        &self,
        stripe: usize,
        deblock: &crate::filters::deblock::FrameDeblock<'_>,
        source: &crate::filters::source::DeblockedSource<T>,
    ) -> Result<Option<crate::filters::source::DeblockedReadLease<T>>> {
        let Some((start, end)) = self.ready_stripe(stripe, deblock)? else {
            return Ok(None);
        };
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::FilterStripeWindow);
        source
            .lease(start, end, STRIPE_WINDOW_MARGIN)
            .ok_or_else(lr_pipeline_state_error)
            .map(Some)
    }

    pub(crate) fn lease_terminal_rows(
        &self,
        stripe: usize,
        source: &crate::filters::source::DeblockedSource<T>,
    ) -> Result<crate::filters::source::DeblockedReadLease<T>> {
        let (start, end) = self.stripe_bounds(stripe)?;
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::FilterStripeWindow);
        source
            .lease(start, end, STRIPE_WINDOW_MARGIN)
            .ok_or_else(lr_pipeline_state_error)
    }

    fn retarget_terminal_rows(
        &self,
        stripe: usize,
        source: &crate::filters::source::DeblockedSource<T>,
        lease: &mut crate::filters::source::DeblockedReadLease<T>,
    ) -> Result<()> {
        let (start, end) = self.stripe_bounds(stripe)?;
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::FilterStripeWindow);
        source
            .retarget_lease(lease, start, end, STRIPE_WINDOW_MARGIN)
            .then_some(())
            .ok_or_else(lr_pipeline_state_error)
    }

    /// Extracts one ready stripe directly from segmented canonical row bands.
    pub(crate) fn extract_ready_band_window(
        &self,
        stripe: usize,
        deblock: &crate::filters::deblock::FrameDeblock<'_>,
        frame: &splot_recon::OwnedFrameBands<T>,
    ) -> Result<Option<crate::filters::source::DeblockedWindow<T>>> {
        let Some(_) = self.ready_stripe(stripe, deblock)? else {
            return Ok(None);
        };
        self.extract_window_with(stripe, |sequence| {
            sequence.extract_bands(frame, &self.ranges, stripe, STRIPE_WINDOW_MARGIN)
        })
        .map(Some)
    }

    fn ready_stripe(
        &self,
        stripe: usize,
        deblock: &crate::filters::deblock::FrameDeblock<'_>,
    ) -> Result<Option<(usize, usize)>> {
        let (start, end) = self.stripe_bounds(stripe)?;
        let needed = end
            .checked_add(STRIPE_WINDOW_MARGIN << self.subsampling.1)
            .ok_or_else(lr_pipeline_state_error)?
            .min(self.luma_height);
        Ok((deblock
            .final_luma_rows(self.subsampling.1)
            .min(self.luma_height)
            >= needed)
            .then_some((start, end)))
    }

    /// Extracts one terminal stripe directly from segmented canonical bands
    /// when no active deblock plan exists.
    pub(crate) fn extract_terminal_band_window(
        &self,
        stripe: usize,
        frame: &splot_recon::OwnedFrameBands<T>,
    ) -> Result<crate::filters::source::DeblockedWindow<T>> {
        self.extract_window_with(stripe, |sequence| {
            sequence.extract_bands(frame, &self.ranges, stripe, STRIPE_WINDOW_MARGIN)
        })
    }

    fn extract_window_with(
        &self,
        stripe: usize,
        extract: impl FnOnce(
            &mut crate::filters::source::DeblockedWindowSequence<T>,
        ) -> core::result::Result<
            crate::filters::source::DeblockedWindow<T>,
            crate::filters::source::StripeCopyError,
        >,
    ) -> Result<crate::filters::source::DeblockedWindow<T>> {
        self.stripe_bounds(stripe)?;
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::FilterStripeWindow);
        let mut sequence = self
            .window_sequence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        extract(&mut sequence).map_err(stripe_copy_error)
    }

    fn stripe_bounds(&self, stripe: usize) -> Result<(usize, usize)> {
        self.ranges
            .get(stripe)
            .copied()
            .ok_or_else(lr_pipeline_state_error)
    }

    /// Restores the moved deblock vectors exactly once before terminal freeze.
    pub(crate) fn restore_deblock_records(
        &self,
        records: crate::filters::deblock::OwnedDeblockRecords,
    ) -> Result<()> {
        let mut slot = self
            .deblock_records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_some() {
            return Err(lr_pipeline_state_error());
        }
        *slot = Some(records);
        Ok(())
    }

    /// Runs one stripe from the owned deblocked rows it needs.
    ///
    /// The window is consumed by this call, so no task can retain a borrow of
    /// reconstruction storage or accidentally run the same input owner twice.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the scheduled task transfers its sole window owner here"
    )]
    pub(crate) fn run_owned_source(
        &self,
        stripe: usize,
        source: crate::filters::source::DeblockedStripe<T>,
    ) -> Result<OwnedFilteredStripe<T>> {
        let range = self.claim(stripe)?;
        let deblocked = source.planes().ok_or_else(lr_pipeline_state_error)?;
        self.run_claimed_planes(stripe, range, deblocked)
    }

    fn run_borrowed_lease(
        &self,
        stripe: usize,
        lease: &crate::filters::source::DeblockedReadLease<T>,
    ) -> Result<OwnedFilteredStripe<T>> {
        let range = self.claim(stripe)?;
        let deblocked = lease.planes().ok_or_else(lr_pipeline_state_error)?;
        self.run_claimed_planes(stripe, range, deblocked)
    }

    fn run_claimed_planes(
        &self,
        stripe: usize,
        range: &(usize, usize),
        deblocked: crate::filters::source::DeblockedPlanes<'_, T>,
    ) -> Result<OwnedFilteredStripe<T>> {
        let mut direct = self.sink.direct_stripe(stripe)?;
        let target = direct.take_target().ok_or_else(lr_pipeline_state_error)?;
        Ok(OwnedFilteredStripe {
            stripe,
            frame: self.run_planes(range, deblocked, target)?,
            direct,
        })
    }

    #[cfg(test)]
    pub(crate) fn run_owned_window(
        &self,
        stripe: usize,
        window: crate::filters::source::DeblockedWindow<T>,
    ) -> Result<OwnedFilteredStripe<T>> {
        self.run_owned_source(
            stripe,
            crate::filters::source::DeblockedStripe::Window(window),
        )
    }

    /// Moves one completed stripe into the frame output exactly once.
    pub(crate) fn publish(&self, stripe: OwnedFilteredStripe<T>) -> Result<()> {
        let chain = self.chain();
        chain.validate_filter_stripe(PlaneId::Y, &stripe.frame.y)?;
        if let Some(plane) = stripe.frame.u.as_ref() {
            chain.validate_filter_stripe(PlaneId::U, plane)?;
        }
        if let Some(plane) = stripe.frame.v.as_ref() {
            chain.validate_filter_stripe(PlaneId::V, plane)?;
        }
        {
            let mut state = self
                .stripe_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(lifecycle) = state.get_mut(stripe.stripe) else {
                return Err(lr_pipeline_state_error());
            };
            if *lifecycle != StripeLifecycle::Claimed {
                return Err(lr_pipeline_state_error());
            }
            *lifecycle = StripeLifecycle::Submitted;
        }
        self.sink.publish_stripe(stripe.frame, stripe.direct)
    }

    fn claim(&self, stripe: usize) -> Result<&(usize, usize)> {
        let range = self
            .ranges
            .get(stripe)
            .ok_or_else(lr_pipeline_state_error)?;
        let mut state = self
            .stripe_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let lifecycle = state.get_mut(stripe).ok_or_else(lr_pipeline_state_error)?;
        if *lifecycle != StripeLifecycle::Pending {
            return Err(lr_pipeline_state_error());
        }
        *lifecycle = StripeLifecycle::Claimed;
        Ok(range)
    }

    /// Runs § 7.5's CDEF-then-CCSO sequence over one luma row range.
    ///
    /// GDF reads `CdefFrame`, which § 7.5 orders after CCSO, so the overlap
    /// rows in [`Self::cdef_overlap_planes`] must come through this same path
    /// rather than a bare CDEF pass.
    /// Tile MI starts when § 7.5's in-loop filters must stay inside their tile.
    fn tile_starts(&self) -> Option<(&[u32], &[u32])> {
        if !self.disable_loopfilters_across_tiles {
            return None;
        }
        self.core
            .tile_info
            .as_ref()
            .map(|tile| (tile.mi_row_starts.as_slice(), tile.mi_col_starts.as_slice()))
    }

    fn cdef_ccso_range<'d>(
        &self,
        deblocked: crate::filters::source::DeblockedPlanes<'d, T>,
        chain: &StripeChain<'_>,
        start: usize,
        end: usize,
        target: Option<crate::pipeline::frame_progress::DirectStripeTarget>,
    ) -> Result<crate::filters::cdef::CdefFrame<'d, T>> {
        let cdef_timer = crate::timing::start();
        let mut cdef = crate::filters::cdef::cdef_stripe_into(
            deblocked,
            self.cdef_strengths.as_deref(),
            chain.cdef_grid,
            self.cdef_skip_grid.as_ref(),
            chain.lossless_grid,
            (self.mi_rows, self.mi_cols),
            self.subsampling,
            self.bit_depth,
            self.tile_starts(),
            start,
            end,
            target,
        )
        .map_err(|error| cdef_filter_error(&error))?;
        crate::timing::accumulate(crate::timing::Phase::FilterCdefStripe, cdef_timer);
        if let Some((grid, config)) = chain.ccso_grid.zip(self.ccso_config.as_ref()) {
            let ccso_timer = crate::timing::start();
            crate::filters::ccso::ccso_stripe(
                &mut cdef,
                grid,
                config,
                chain.lossless_grid,
                self.tile_starts(),
            )
            .map_err(|error| ccso_filter_error(&error))?;
            crate::timing::accumulate(crate::timing::Phase::FilterCcsoStripe, ccso_timer);
        }
        Ok(cdef)
    }

    /// Builds the post-CCSO rows GDF and § 7.20.2 read outside this stripe.
    ///
    /// § 7.20.1 clips a stripe to its tile row, but § 7.20.2 still sources the
    /// in-stripe filter neighbourhood from `CdefFrame`, so a stripe abutting a
    /// tile row boundary needs rows the stripe itself never covers.
    /// Ranges are aligned to CDEF's `STEP4 * MI_SIZE` step on both ends; rows
    /// the stripe already holds are resolved from it first, so any overlap
    /// between the two is redundant rather than wrong. `GDF_READ_RADIUS` is the
    /// widest of the stripe-crossing readers, so one fringe serves them all.
    ///
    /// `mi_row_starts` carries an end sentinel, so a single tile row is two
    /// entries. With one tile row § 7.20.1's clip never crosses a tile, the
    /// stripe already holds every row § 7.20.2 asks for, and rebuilding the
    /// fringes would be pure waste.
    fn cdef_overlap_planes(
        &self,
        deblocked: crate::filters::source::DeblockedPlanes<'_, T>,
        chain: &StripeChain<'_>,
        start: usize,
        end: usize,
    ) -> Result<final_filters::CdefOverlap> {
        const CDEF_START_ALIGN: usize = 8;
        let mut overlap = final_filters::CdefOverlap::default();
        if self
            .core
            .tile_info
            .as_ref()
            .is_none_or(|tile| tile.mi_row_starts.len() <= 2)
        {
            return Ok(overlap);
        }
        let frame_height = self.ranges.last().map_or(0, |&(_, last_end)| last_end);
        let push_range = |overlap: &mut final_filters::CdefOverlap,
                          range_start: usize,
                          range_end: usize|
         -> Result<()> {
            let fringe = self.cdef_ccso_range(deblocked, chain, range_start, range_end, None)?;
            overlap.y.push(fringe.filtered_y);
            overlap.u.extend(fringe.filtered_u);
            overlap.v.extend(fringe.filtered_v);
            Ok(())
        };
        let back_start = start.saturating_sub(crate::filters::gdf::GDF_READ_RADIUS)
            / CDEF_START_ALIGN
            * CDEF_START_ALIGN;
        if back_start < start {
            push_range(&mut overlap, back_start, start)?;
        }
        let forward_start = end / CDEF_START_ALIGN * CDEF_START_ALIGN;
        let forward_end = end
            .saturating_add(crate::filters::gdf::GDF_READ_RADIUS)
            .next_multiple_of(CDEF_START_ALIGN)
            .min(frame_height);
        if forward_end > end && forward_start < forward_end {
            push_range(&mut overlap, forward_start, forward_end)?;
        }
        Ok(overlap)
    }

    fn run_planes(
        &self,
        &(start, end): &(usize, usize),
        deblocked: crate::filters::source::DeblockedPlanes<'_, T>,
        target: crate::pipeline::frame_progress::DirectStripeTarget,
    ) -> Result<final_filters::FilteredStripe> {
        let chain = self.chain();
        let [y_end, u_end] = self.lr_plane_ends;
        let y_runs = &self.lr_source_blocks[..y_end];
        let u_runs = &self.lr_source_blocks[y_end..u_end];
        let v_runs = &self.lr_source_blocks[u_end..];
        let plane_blocks = [y_runs, u_runs, v_runs];
        let active_lr = chain.active_lr_planes(start, end, plane_blocks);
        let gdf_active = crate::filters::gdf::is_active(
            &self.core,
            chain.gdf_grid,
            self.bit_depth,
            chain.gdf_reference,
        )?;
        let direct_u8_lr = core::array::from_fn(|index| {
            let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][index];
            if self.bit_depth != BitDepth::Eight || !active_lr[index] {
                return false;
            }
            let Some(target) = target.get(plane) else {
                return false;
            };
            let frame_type = self
                .core
                .lr_params
                .as_ref()
                .and_then(|params| params.planes.get(index))
                .map(|plane| plane.restoration_type);
            if plane == PlaneId::Y {
                return final_filters::terminal_luma_wiener_direct_u8(
                    self.bit_depth,
                    frame_type,
                    gdf_active,
                    plane_blocks[index],
                    target,
                );
            }
            let Some(target_end_y) = target.end_y().filter(|_| !target.is_u16()) else {
                return false;
            };
            frame_type.is_some_and(|frame_type| {
                final_filters::lr_plane_fully_overwritten(
                    plane_blocks[index],
                    plane,
                    frame_type,
                    target.width(),
                    target.frame_height(),
                    target.origin_y(),
                    target_end_y,
                )
            })
        });
        let lr_initializations =
            final_filters::lr_initializations(&self.core, active_lr, plane_blocks, &target);
        let (cdef_target, lr_target) = target.split(active_lr);
        let cdef = self.cdef_ccso_range(deblocked, &chain, start, end, Some(cdef_target))?;
        let cdef_overlap = self.cdef_overlap_planes(deblocked, &chain, start, end)?;
        let lr_timer = crate::timing::start();
        let mut frame = chain.apply_lr_stripe(
            &self.core,
            cdef,
            &cdef_overlap,
            plane_blocks,
            &self.lr_unit_filters,
            final_filters::LrStripeOutput {
                active_planes: active_lr,
                direct_u8_planes: direct_u8_lr,
                initializations: lr_initializations,
                target: lr_target,
            },
        )?;
        crate::timing::accumulate(crate::timing::Phase::FilterLrStripe, lr_timer);
        if gdf_active {
            let (separate_cdef_luma, output_luma) =
                if let Some(post_lr_y) = frame.post_lr_y.as_mut() {
                    (
                        Some(&frame.cdef_y),
                        post_lr_y.as_u16_mut().ok_or_else(lr_pipeline_state_error)?,
                    )
                } else {
                    (None, &mut frame.cdef_y)
                };
            let gdf_timer = crate::timing::start();
            crate::filters::gdf::apply_stripe(
                &self.core,
                frame.deblocked_y,
                separate_cdef_luma,
                &cdef_overlap.y,
                output_luma,
                chain.gdf_grid,
                chain.lossless_grid,
                self.bit_depth,
                self.disable_loopfilters_across_tiles,
                chain.gdf_reference,
            )?;
            crate::timing::accumulate(crate::timing::Phase::FilterGdfStripe, gdf_timer);
        }
        Ok(frame.into_filtered())
    }

    /// Publishes every completed stripe, then consumes the sole output owner to
    /// freeze exactly once.
    pub(crate) fn finish<R>(
        mut self,
        publish: impl FnOnce(DecodedFrame<T>) -> R,
    ) -> Result<(R, super::FrameFilterRecords)> {
        let complete = self
            .stripe_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .all(|lifecycle| *lifecycle == StripeLifecycle::Submitted);
        if !complete {
            return Err(lr_pipeline_state_error());
        }
        self.filter_records.lr_source_blocks = self.lr_source_blocks;
        self.filter_records.lr_unit_filters = self.lr_unit_filters;
        let has_restored_deblock = self
            .deblock_records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        if has_restored_deblock
            && (!self.filter_records.deblock_blocks.is_empty()
                || !self.filter_records.chroma_deblock_blocks.is_empty())
        {
            return Err(lr_pipeline_state_error());
        }
        if let Some(records) = self
            .deblock_records
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            self.filter_records.deblock_blocks = records.blocks;
            self.filter_records.chroma_deblock_blocks = records.chroma;
        }
        let frame = self.sink.freeze(publish)?;
        Ok((frame, self.filter_records))
    }
}

impl<T: ReconSample> OwnedFilterJob<T> {
    /// Stable stripe index used by the scheduler's completion table.
    pub(crate) const fn stripe(&self) -> usize {
        self.stripe
    }

    /// Claims and runs one stripe, then publishes it exactly once.
    pub(crate) fn run(self) -> Result<()> {
        let filtered = self.setup.run_owned_source(self.stripe, self.source)?;
        self.setup.publish(filtered)
    }
}

impl<T: ReconSample> OwnedFilterFinish<T> {
    /// Freezes only when this is the last live setup owner.
    pub(crate) fn finish<R>(
        self,
        publish: impl FnOnce(DecodedFrame<T>) -> R,
    ) -> Result<(R, super::FrameFilterRecords)> {
        let setup = Arc::try_unwrap(self.setup).map_err(|_| lr_pipeline_state_error())?;
        setup.finish(publish)
    }
}

impl<T: ReconSample> OwnedFilterSetup<'static, 'static, T> {
    /// Transfers one ready stripe to a scheduler job.
    #[cfg(test)]
    pub(crate) fn owned_job(
        self: &Arc<Self>,
        stripe: usize,
        window: crate::filters::source::DeblockedWindow<T>,
    ) -> OwnedFilterJob<T> {
        self.source_job(
            stripe,
            crate::filters::source::DeblockedStripe::Window(window),
        )
    }

    pub(crate) fn source_job(
        self: &Arc<Self>,
        stripe: usize,
        source: crate::filters::source::DeblockedStripe<T>,
    ) -> OwnedFilterJob<T> {
        OwnedFilterJob {
            setup: Arc::clone(self),
            stripe,
            source,
        }
    }

    /// Transfers terminal ownership to the exactly-once freeze job.
    pub(crate) fn owned_finish(self: Arc<Self>) -> OwnedFilterFinish<T> {
        OwnedFilterFinish { setup: self }
    }
}

/// How many plane rows past each end of a stripe the § 7.2 chain reads.
///
/// CDEF reaches two samples past the stripe, § 7.17 loop restoration clamps its
/// own reads to the stripe it is filtering plus one row, and a tile end that is
/// not stripe aligned adds eight. Ten rows of each plane covers all of them,
/// and every read outside the window is refused rather than served from another
/// stripe's rows.
const STRIPE_WINDOW_MARGIN: usize = 10;

/// Deblocks far enough for one stripe's window to be final, then copies it out.
///
/// The window is what lets the stripe chain run while the deblock is still
/// filtering the rows below it: the stripe owns its rows, so the deblock keeps
/// the frame to itself and neither waits for the other.
fn advance_deblock_for_stripe<T: ReconSample>(
    sections: Option<&mut crate::filters::deblock::FrameDeblock<'_>>,
    source: &mut crate::filters::source::DeblockedSource<T>,
    range: &(usize, usize),
    subsampling_y: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    let needed = range.1 + (STRIPE_WINDOW_MARGIN << subsampling_y);
    if let Some(sections) = sections {
        let deblock_timer = crate::timing::start();
        let reach = crate::filters::deblock::DEBLOCK_PASS_1_REACH << subsampling_y;
        sections
            .advance_source(
                source,
                needed.saturating_add(reach).div_ceil(MI_SIZE),
                bit_depth,
            )
            .map_err(|_| lr_pipeline_state_error())?;
        crate::timing::accumulate(crate::timing::Phase::FilterDeblock, deblock_timer);
        if sections.final_luma_rows(subsampling_y) < needed.min(sections.luma_rows()) {
            return Err(lr_pipeline_state_error());
        }
    }
    Ok(())
}

pub(crate) fn deblock_prepare_error(
    error: &crate::filters::deblock::DeblockError,
) -> crate::error::DecodeError {
    match error {
        crate::filters::deblock::DeblockError::Allocation { plane, context } => {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: *plane,
                context,
            }
            .into()
        }
        _ => lr_pipeline_state_error(),
    }
}

fn cdef_filter_error(error: &crate::filters::cdef::CdefError) -> crate::error::DecodeError {
    match error {
        crate::filters::cdef::CdefError::Allocation(plane) => {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: *plane,
                context: "CDEF stripe output",
            }
            .into()
        }
        _ => lr_pipeline_state_error(),
    }
}

fn ccso_filter_error(error: &crate::filters::ccso::CcsoError) -> crate::error::DecodeError {
    match error {
        crate::filters::ccso::CcsoError::Allocation(plane) => {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: *plane,
                context: "CCSO lookup table",
            }
            .into()
        }
        _ => lr_pipeline_state_error(),
    }
}

pub(crate) fn stripe_copy_error(
    error: crate::filters::source::StripeCopyError,
) -> crate::error::DecodeError {
    match error {
        crate::filters::source::StripeCopyError::Allocation(plane) => {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane,
                context: "deblocked stripe window",
            }
            .into()
        }
        crate::filters::source::StripeCopyError::Geometry => lr_pipeline_state_error(),
    }
}

pub(crate) fn lr_pipeline_state_error() -> crate::error::DecodeError {
    crate::error::DecodeHeaderStateError::InvalidLoopRestorationFilterState.into()
}

/// Everything one filter stripe reads, borrowed out of the sink so the deblock
/// keeps the reconstructed frame it writes.
pub(crate) struct StripeChain<'a> {
    pub(crate) bit_depth: BitDepth,
    pub(crate) cfl_ds_filter_index: u8,
    pub(crate) luma_width: usize,
    pub(crate) luma_height: usize,
    pub(crate) pixel_format: splot_recon::PixelFormat,
    pub(crate) cdef_grid: Option<&'a crate::filters::cdef::CdefUnitGrid>,
    pub(crate) ccso_grid: Option<&'a crate::filters::ccso::CcsoUnitGrid>,
    pub(crate) gdf_grid: Option<&'a crate::filters::gdf::GdfBlockGrid>,
    pub(crate) tx_skip_grid: Option<&'a super::WienerNsLrTxSkipGrid>,
    pub(crate) gdf_reference: Option<crate::filters::gdf::GdfReferenceContext>,
    pub(crate) lossless_grid: Option<&'a crate::filters::lossless::LosslessBlockGrid>,
    pub(crate) plane_sizes: [Option<splot_recon::PlaneSize>; 3],
    /// Whether the sink's sample type can hold the frame's largest sample,
    /// which the stripe validation reports without naming that type.
    pub(crate) max_sample_fits: bool,
}

#[cfg(test)]
impl<T: ReconSample> WienerNsLrReconSink<T> {
    /// Borrows the sink's stripe-side state, which is what the filter phase
    /// hands the stripes once the deblock has taken the frame.
    pub(crate) fn stripe_chain(&self) -> StripeChain<'_> {
        StripeChain {
            bit_depth: self.bit_depth,
            cfl_ds_filter_index: self.cfl_ds_filter_index,
            luma_width: self.luma_width,
            luma_height: self.luma_height,
            pixel_format: self.info.pixel_format(),
            cdef_grid: self.cdef_grid.as_ref(),
            ccso_grid: self.ccso_grid.as_ref(),
            gdf_grid: self.gdf_grid.as_ref(),
            tx_skip_grid: self.tx_skip_grid.as_ref(),
            gdf_reference: self.gdf_reference,
            lossless_grid: self.lossless_grid.as_ref(),
            plane_sizes: self.plane_sizes,
            max_sample_fits: T::try_from_u16(self.bit_depth.max_sample()).is_ok(),
        }
    }
}

impl StripeChain<'_> {
    fn validate_filter_stripe(
        &self,
        plane: PlaneId,
        stripe: &crate::filters::source::StripeOutputPlane,
    ) -> Result<()> {
        let error = || lr_pipeline_state_error();
        let size = self.plane_sizes[plane.index()].ok_or_else(&error)?;
        let end_y = stripe.end_y().ok_or_else(&error)?;
        if stripe.width() != size.width()
            || stripe.frame_height() != size.height()
            || stripe.origin_y() > end_y
            || end_y > size.height()
            || !self.max_sample_fits
        {
            return Err(error());
        }
        Ok(())
    }
}

impl<T: ReconSample> WienerNsLrReconSink<T> {
    fn needs_lr_tx_skip_grid(&self, core: &splot_core::headers::frame::FrameHeaderCore) -> bool {
        core.lr_params.as_ref().is_some_and(|lr| {
            lr.planes.get(PlaneId::Y.index()).is_some_and(|plane| {
                matches!(
                    plane.restoration_type,
                    splot_core::headers::frame::FrameRestorationType::PcWiener
                        | splot_core::headers::frame::FrameRestorationType::Switchable
                ) || (plane.restoration_type
                    == splot_core::headers::frame::FrameRestorationType::WienerNonsep
                    && plane.frame_filters_on
                    && plane.num_filter_classes.unwrap_or(1) > 1)
            })
        }) && self
            .filter_records
            .lr_source_blocks
            .iter()
            .any(|block| block.plane == PlaneId::Y.index())
    }

    fn ensure_tx_skip_grid(&mut self, mi_rows: usize, mi_cols: usize) -> Result<()> {
        if self.tx_skip_grid.is_some() {
            return Ok(());
        }
        let grid = super::derive_wienerns_lr_tx_skip_grid_retention(
            mi_rows,
            mi_cols,
            &self.filter_records.tx_skip_records,
        )
        .map_err(|_| lr_pipeline_state_error())?;
        self.tx_skip_grid = Some(grid);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn chroma_transform_deblock_block(
    plane_id: PlaneId,
    x: usize,
    y: usize,
    chroma_tx: usize,
    log2_dimensions: Option<(u32, u32)>,
    chroma_subsampling: (u32, u32),
    qindex: u32,
    lossless: bool,
) -> Option<(usize, crate::filters::deblock::DeblockBlock)> {
    let (log2_width, log2_height) = log2_dimensions.or_else(|| tx_size_log2(chroma_tx))?;
    let plane_index = match plane_id {
        PlaneId::U => 0,
        PlaneId::V => 1,
        PlaneId::Y => return None,
    };
    let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
    let scale_x = 1usize.checked_shl(chroma_subsampling.0)?;
    let scale_y = 1usize.checked_shl(chroma_subsampling.1)?;
    let r = (y / MI_SIZE).saturating_mul(scale_y);
    let c = (x / MI_SIZE).saturating_mul(scale_x);
    Some((
        plane_index,
        crate::filters::deblock::DeblockBlock {
            r,
            c,
            luma_prediction: crate::filters::deblock::DeblockPredictionUnit {
                base_r: r,
                base_c: c,
                default_sub_pu_tx: chroma_tx,
            },
            chroma_prediction: crate::filters::deblock::DeblockPredictionUnit {
                base_r: r,
                base_c: c,
                default_sub_pu_tx: chroma_tx,
            },
            chroma_base_r: r,
            chroma_base_c: c,
            n4w: mi_w.saturating_mul(scale_x),
            n4h: mi_h.saturating_mul(scale_y),
            luma_tx: chroma_tx,
            chroma_tx: Some(chroma_tx),
            sub_pu_size: None,
            chroma_transform_only: false,
            qindex,
            skip: false,
            lossless,
        },
    ))
}

fn tx_size_log2(tx_size: usize) -> Option<(u32, u32)> {
    let w = u32::try_from(*TX_WIDTH_LOG2.get(tx_size)?).ok()?;
    let h = u32::try_from(*TX_HEIGHT_LOG2.get(tx_size)?).ok()?;
    Some((w, h))
}

fn mi_extent(log2_width: u32, log2_height: u32) -> (usize, usize) {
    let mi_w = (1usize << log2_width >> 2).max(1);
    let mi_h = (1usize << log2_height >> 2).max(1);
    (mi_w, mi_h)
}

mod final_filters;
pub(crate) mod full_recon;
