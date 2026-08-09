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
/// shares, so a consumer can read the published prefix before the freeze; every
/// other path keeps its filtered workspace local to the filter chain.
///
/// [`FrameProgress`]: crate::pipeline::frame_progress::FrameProgress
enum FilteredFrameSink<'a, 'job, T: ReconSample> {
    /// The filter chain owns the output until the freeze.
    Local(Box<Mutex<CurrentFrameWorkspace<T>>>),
    /// The frame's slot shares the output, stripe by stripe.
    Published {
        progress: &'a crate::pipeline::frame_progress::FrameProgress<T>,
        admit: Option<&'a dyn splot_parallel::Admit<'job>>,
    },
    /// Scheduled row tasks own the publication handle for their full lifetime.
    OwnedPublished {
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    },
}

enum FilteredFrameSinkSource<'a, 'job, T: ReconSample> {
    Borrowed {
        progress: Option<&'a crate::pipeline::frame_progress::FrameProgress<T>>,
        admit: Option<&'a dyn splot_parallel::Admit<'job>>,
    },
    Owned {
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    },
}

impl<'a, 'job, T: ReconSample> FilteredFrameSinkSource<'a, 'job, T> {
    fn open(
        self,
        info: splot_recon::DecodedFrameInfo,
        ranges: &[(usize, usize)],
    ) -> Result<FilteredFrameSink<'a, 'job, T>> {
        match self {
            Self::Borrowed { progress, admit } => {
                FilteredFrameSink::open(progress, admit, info, ranges)
            }
            Self::Owned { progress } => {
                if !progress.begin(ranges) {
                    return Err(lr_pipeline_state_error());
                }
                Ok(FilteredFrameSink::OwnedPublished { progress })
            }
        }
    }
}

impl<'a, 'job, T: ReconSample> FilteredFrameSink<'a, 'job, T> {
    fn open(
        progress: Option<&'a crate::pipeline::frame_progress::FrameProgress<T>>,
        admit: Option<&'a dyn splot_parallel::Admit<'job>>,
        info: splot_recon::DecodedFrameInfo,
        ranges: &[(usize, usize)],
    ) -> Result<Self> {
        match progress {
            Some(progress) => {
                if !progress.begin(ranges) {
                    return Err(lr_pipeline_state_error());
                }
                Ok(Self::Published { progress, admit })
            }
            None => Ok(Self::Local(Box::new(Mutex::new(
                CurrentFrameWorkspace::new(info, T::default())?,
            )))),
        }
    }

    /// Moves one finished stripe's samples into the output exactly once.
    ///
    /// A published output is shared with the blocks of the next frame reading
    /// this frame's published prefix, so the copy is queued rather than waited
    /// for: a stripe that took a turn behind those readers would stall its own
    /// worker and, under a writer-preferring lock, hold up every reader arriving
    /// behind it. Whichever thread next finds the output free copies the whole
    /// queue, so nothing lands twice and the copies stay off every wait path.
    /// A local output has no other user and is copied into straight away.
    fn publish_stripe(&self, stripe: usize, samples: final_filters::FilteredStripe) -> Result<()> {
        let copy = move |output: &mut CurrentFrameWorkspace<T>| {
            publish_filter_stripe_to(output, PlaneId::Y, &samples.y)?;
            if let Some(plane) = samples.u.as_ref() {
                publish_filter_stripe_to(output, PlaneId::U, plane)?;
            }
            if let Some(plane) = samples.v.as_ref() {
                publish_filter_stripe_to(output, PlaneId::V, plane)?;
            }
            Ok(())
        };
        match self {
            Self::Local(workspace) => copy(
                &mut workspace
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
            Self::Published { progress, admit } => {
                let published = progress.publish_stripe(stripe, Box::new(copy));
                if let Some(admit) = admit {
                    admit.admit_ready();
                }
                published
            }
            Self::OwnedPublished { progress } => progress.publish_stripe(stripe, Box::new(copy)),
        }
    }

    /// Copies every stripe still queued into the output, waiting for it.
    ///
    /// This is what makes every stripe's samples present before the freeze even
    /// when the output was busy each time a stripe finished. Waiting is safe
    /// only here: the filter phase is over, so no stripe can queue behind it.
    fn drain_before_freeze(&self) -> Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Published { progress, .. } => progress.drain_pending_blocking(),
            Self::OwnedPublished { progress } => progress.drain_pending_blocking(),
        }
    }

    /// Freezes the filtered output and hands the frozen frame to `publish`.
    ///
    /// A published frame freezes under its own lock, so the slot `publish`
    /// settles is visible to every consumer before the published prefix stops
    /// being readable.
    fn freeze<R>(self, publish: impl FnOnce(DecodedFrame<T>) -> R) -> Result<R> {
        match self {
            Self::Local(workspace) => Ok(publish(
                workspace
                    .into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .freeze()?,
            )),
            Self::Published { progress, .. } => progress.freeze_workspace(publish),
            Self::OwnedPublished { progress } => progress.freeze_workspace(publish),
        }
    }
}

pub(crate) struct WienerNsLrReconSink<T: ReconSample> {
    workspace: CurrentFrameWorkspace<T>,
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
    stripe_state: Mutex<Vec<StripeLifecycle>>,
    deblock_records: Mutex<Option<crate::filters::deblock::OwnedDeblockRecords>>,
}

/// One completed stripe whose index and samples move together into publication.
pub(crate) struct OwnedFilteredStripe {
    stripe: usize,
    frame: final_filters::FilteredStripe,
}

/// One scheduled stripe with sole ownership of its deblocked input window.
pub(crate) struct OwnedFilterJob<T: ReconSample> {
    setup: Arc<OwnedFilterSetup<'static, 'static, T>>,
    stripe: usize,
    window: crate::filters::source::DeblockedWindow<T>,
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

impl<T: ReconSample> WienerNsLrReconSink<T> {
    pub(crate) fn for_final_filtering(
        workspace: CurrentFrameWorkspace<T>,
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
    ) -> Self {
        Self {
            workspace,
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
    pub(crate) fn frame_info(&self) -> splot_recon::DecodedFrameInfo {
        self.workspace.info()
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
        progress: Option<&crate::pipeline::frame_progress::FrameProgress<T>>,
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
        progress: Option<&crate::pipeline::frame_progress::FrameProgress<T>>,
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
        progress: Option<&'progress crate::pipeline::frame_progress::FrameProgress<T>>,
        admit: Option<&'progress dyn splot_parallel::Admit<'job>>,
    ) -> Result<(
        OwnedFilterSetup<'progress, 'job, T>,
        CurrentFrameWorkspace<T>,
    )> {
        self.into_owned_filter_setup_inner(
            core,
            disable_loopfilters_across_tiles,
            FilteredFrameSinkSource::Borrowed { progress, admit },
        )
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
        CurrentFrameWorkspace<T>,
    )> {
        self.into_owned_filter_setup_inner(
            core,
            disable_loopfilters_across_tiles,
            FilteredFrameSinkSource::Owned { progress },
        )
    }

    fn into_owned_filter_setup_inner<'progress, 'job>(
        mut self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        sink_source: FilteredFrameSinkSource<'progress, 'job, T>,
    ) -> Result<(
        OwnedFilterSetup<'progress, 'job, T>,
        CurrentFrameWorkspace<T>,
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
        if self.needs_tx_skip_grid(&core) {
            self.ensure_tx_skip_grid(mi_rows, mi_cols)?;
        }
        let cdef_skip_grid = self.cdef_skip_grid(&core, mi_rows, mi_cols)?;
        let cdef_strengths = crate::filters::cdef::cdef_frame_strengths(&core);
        let lr_source_blocks = core::mem::take(&mut self.filter_records.lr_source_blocks);
        let lr_unit_filters = core::mem::take(&mut self.filter_records.lr_unit_filters);
        let (lr_source_blocks, lr_plane_ends) =
            final_filters::coalesced_lr_source_rows_all(lr_source_blocks);
        let ranges = crate::filters::gdf::stripe_ranges(&core, self.luma_height)?;
        let info = self.workspace.info();
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
        let sink = sink_source.open(info, &ranges)?;
        let stripe_count = ranges.len();
        let plane_sizes = [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| {
            self.workspace
                .plane(plane)
                .ok()
                .map(splot_recon::CurrentFramePlane::storage_size)
        });

        let Self {
            workspace,
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
        progress: Option<&crate::pipeline::frame_progress::FrameProgress<T>>,
        admit: Option<&dyn splot_parallel::Admit<'_>>,
        deblocked: bool,
        publish: impl FnOnce(DecodedFrame<T>) -> R,
    ) -> Result<(R, super::FrameFilterRecords)> {
        if std::env::var_os("SPLOT_DECODE_SKIP_FILTERS").is_some() {
            return Ok((publish(self.workspace.freeze()?), self.filter_records));
        }
        let (setup, mut workspace) =
            self.into_owned_filter_setup(core, disable_loopfilters_across_tiles, progress, admit)?;
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
            let mut slots: Vec<Option<Result<()>>> =
                (0..setup.stripe_ranges().len()).map(|_| None).collect();
            let mut owed: Option<crate::error::DecodeError> = None;
            let scheduled = splot_parallel::ready_task_scope(|scope| {
                for ((stripe, range), slot) in
                    setup.stripe_ranges().iter().enumerate().zip(&mut slots)
                {
                    let window = match deblock_stripe_window(
                        sections.as_mut(),
                        &mut workspace,
                        range,
                        setup.subsampling.1,
                        bit_depth,
                    ) {
                        Ok(window) => window,
                        Err(error) => {
                            owed = Some(error);
                            return;
                        }
                    };
                    let setup = &setup;
                    scope.spawn(move |_| {
                        *slot = Some(
                            setup
                                .run_owned_window(stripe, window)
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
            if let Some(sections) = sections.as_mut() {
                let deblock_timer = crate::timing::start();
                sections
                    .advance(&mut workspace, mi_rows, bit_depth)
                    .map_err(|_| lr_pipeline_state_error())?;
                crate::timing::accumulate(crate::timing::Phase::FilterDeblock, deblock_timer);
            }
            let deblocked = crate::filters::source::DeblockedPlanes::frame(&workspace)
                .ok_or_else(lr_pipeline_state_error)?;
            for (stripe, range) in setup.stripe_ranges().iter().enumerate() {
                let filtered = setup.run_borrowed_planes(stripe, range, deblocked)?;
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
        workspace.recycle_planes();
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

    /// Extracts one stripe window only after incremental deblock has made every
    /// source row and halo final.
    pub(crate) fn extract_ready_window(
        &self,
        stripe: usize,
        deblock: &crate::filters::deblock::FrameDeblock<'_>,
        workspace: &CurrentFrameWorkspace<T>,
    ) -> Result<Option<crate::filters::source::DeblockedWindow<T>>> {
        self.extract_ready_with(stripe, deblock, |start, end| {
            deblock.extract_window(workspace, start, end, STRIPE_WINDOW_MARGIN)
        })
    }

    /// Extracts one ready stripe directly from segmented canonical row bands.
    pub(crate) fn extract_ready_band_window(
        &self,
        stripe: usize,
        deblock: &crate::filters::deblock::FrameDeblock<'_>,
        frame: &splot_recon::OwnedFrameBands<T>,
    ) -> Result<Option<crate::filters::source::DeblockedWindow<T>>> {
        self.extract_ready_with(stripe, deblock, |start, end| {
            deblock.extract_band_window(frame, start, end, STRIPE_WINDOW_MARGIN)
        })
    }

    fn extract_ready_with(
        &self,
        stripe: usize,
        deblock: &crate::filters::deblock::FrameDeblock<'_>,
        extract: impl FnOnce(
            usize,
            usize,
        ) -> core::result::Result<
            crate::filters::source::DeblockedWindow<T>,
            crate::filters::deblock::DeblockError,
        >,
    ) -> Result<Option<crate::filters::source::DeblockedWindow<T>>> {
        let Some((start, end)) = self.ready_stripe(stripe, deblock)? else {
            return Ok(None);
        };
        extract(start, end)
            .map(Some)
            .map_err(|error| deblock_prepare_error(&error))
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

    /// Extracts one terminal window when the frame has no active deblock plan.
    pub(crate) fn extract_terminal_window(
        &self,
        stripe: usize,
        workspace: &CurrentFrameWorkspace<T>,
    ) -> Result<crate::filters::source::DeblockedWindow<T>> {
        self.extract_terminal_with(stripe, |start, end| {
            crate::filters::source::DeblockedWindow::extract(
                workspace,
                start,
                end,
                STRIPE_WINDOW_MARGIN,
            )
        })
    }

    /// Extracts one terminal stripe directly from segmented canonical bands
    /// when no active deblock plan exists.
    pub(crate) fn extract_terminal_band_window(
        &self,
        stripe: usize,
        frame: &splot_recon::OwnedFrameBands<T>,
    ) -> Result<crate::filters::source::DeblockedWindow<T>> {
        self.extract_terminal_with(stripe, |start, end| {
            crate::filters::source::DeblockedWindow::extract_bands(
                frame,
                start,
                end,
                STRIPE_WINDOW_MARGIN,
            )
        })
    }

    fn extract_terminal_with(
        &self,
        stripe: usize,
        extract: impl FnOnce(
            usize,
            usize,
        ) -> core::result::Result<
            crate::filters::source::DeblockedWindow<T>,
            crate::filters::source::StripeCopyError,
        >,
    ) -> Result<crate::filters::source::DeblockedWindow<T>> {
        let (start, end) = self.stripe_bounds(stripe)?;
        extract(start, end).map_err(stripe_copy_error)
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
    pub(crate) fn run_owned_window(
        &self,
        stripe: usize,
        window: crate::filters::source::DeblockedWindow<T>,
    ) -> Result<OwnedFilteredStripe> {
        let range = self.claim(stripe)?;
        let deblocked = window.planes().ok_or_else(lr_pipeline_state_error)?;
        Ok(OwnedFilteredStripe {
            stripe,
            frame: self.run_planes(range, deblocked)?,
        })
    }

    fn run_borrowed_planes(
        &self,
        stripe: usize,
        range: &(usize, usize),
        deblocked: crate::filters::source::DeblockedPlanes<'_, T>,
    ) -> Result<OwnedFilteredStripe> {
        let claimed = self.claim(stripe)?;
        if claimed != range {
            return Err(lr_pipeline_state_error());
        }
        Ok(OwnedFilteredStripe {
            stripe,
            frame: self.run_planes(range, deblocked)?,
        })
    }

    /// Moves one completed stripe into the frame output exactly once.
    pub(crate) fn publish(&self, stripe: OwnedFilteredStripe) -> Result<()> {
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
        self.sink.publish_stripe(stripe.stripe, stripe.frame)
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
    ) -> Result<crate::filters::cdef::CdefFrame<'d, T>> {
        let cdef_timer = crate::timing::start();
        let mut cdef = crate::filters::cdef::cdef_stripe(
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
            let fringe = self.cdef_ccso_range(deblocked, chain, range_start, range_end)?;
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
    ) -> Result<final_filters::FilteredStripe> {
        let chain = self.chain();
        let cdef = self.cdef_ccso_range(deblocked, &chain, start, end)?;
        let cdef_overlap = self.cdef_overlap_planes(deblocked, &chain, start, end)?;
        let [y_end, u_end] = self.lr_plane_ends;
        let y_runs = &self.lr_source_blocks[..y_end];
        let u_runs = &self.lr_source_blocks[y_end..u_end];
        let v_runs = &self.lr_source_blocks[u_end..];
        let lr_timer = crate::timing::start();
        let mut frame = chain.apply_lr_stripe(
            &self.core,
            cdef,
            &cdef_overlap,
            [y_runs, u_runs, v_runs],
            &self.lr_unit_filters,
        )?;
        crate::timing::accumulate(crate::timing::Phase::FilterLrStripe, lr_timer);
        let (separate_cdef_luma, output_luma) = if let Some(post_lr_y) = frame.post_lr_y.as_mut() {
            (Some(&frame.cdef_y), post_lr_y)
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
        self.sink.drain_before_freeze()?;
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
        let filtered = self.setup.run_owned_window(self.stripe, self.window)?;
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
    pub(crate) fn owned_job(
        self: &Arc<Self>,
        stripe: usize,
        window: crate::filters::source::DeblockedWindow<T>,
    ) -> OwnedFilterJob<T> {
        OwnedFilterJob {
            setup: Arc::clone(self),
            stripe,
            window,
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
/// not stripe aligned adds eight. Eight rows of each plane covers all of them,
/// and every read outside the window is refused rather than served from another
/// stripe's rows.
const STRIPE_WINDOW_MARGIN: usize = 16;

/// Deblocks far enough for one stripe's window to be final, then copies it out.
///
/// The window is what lets the stripe chain run while the deblock is still
/// filtering the rows below it: the stripe owns its rows, so the deblock keeps
/// the frame to itself and neither waits for the other.
fn deblock_stripe_window<T: ReconSample>(
    sections: Option<&mut crate::filters::deblock::FrameDeblock<'_>>,
    workspace: &mut CurrentFrameWorkspace<T>,
    range: &(usize, usize),
    subsampling_y: usize,
    bit_depth: BitDepth,
) -> Result<crate::filters::source::DeblockedWindow<T>> {
    let needed = range.1 + (STRIPE_WINDOW_MARGIN << subsampling_y);
    if let Some(sections) = sections {
        let deblock_timer = crate::timing::start();
        let reach = crate::filters::deblock::DEBLOCK_PASS_1_REACH << subsampling_y;
        sections
            .advance(
                workspace,
                needed.saturating_add(reach).div_ceil(MI_SIZE),
                bit_depth,
            )
            .map_err(|_| lr_pipeline_state_error())?;
        crate::timing::accumulate(crate::timing::Phase::FilterDeblock, deblock_timer);
        if sections.final_luma_rows(subsampling_y) < needed.min(sections.luma_rows()) {
            return Err(lr_pipeline_state_error());
        }
    }
    let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::FilterStripeWindow);
    crate::filters::source::DeblockedWindow::extract(
        workspace,
        range.0,
        range.1,
        STRIPE_WINDOW_MARGIN,
    )
    .map_err(stripe_copy_error)
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
            pixel_format: self.workspace.info().pixel_format(),
            cdef_grid: self.cdef_grid.as_ref(),
            ccso_grid: self.ccso_grid.as_ref(),
            gdf_grid: self.gdf_grid.as_ref(),
            tx_skip_grid: self.tx_skip_grid.as_ref(),
            gdf_reference: self.gdf_reference,
            lossless_grid: self.lossless_grid.as_ref(),
            plane_sizes: [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| {
                self.workspace
                    .plane(plane)
                    .ok()
                    .map(splot_recon::CurrentFramePlane::storage_size)
            }),
            max_sample_fits: T::try_from_u16(self.bit_depth.max_sample()).is_ok(),
        }
    }
}

impl StripeChain<'_> {
    fn validate_filter_stripe(
        &self,
        plane: PlaneId,
        stripe: &crate::filters::source::StripePlane,
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
    fn needs_tx_skip_grid(&self, core: &splot_core::headers::frame::FrameHeaderCore) -> bool {
        let cdef_needs_skip_grid = core
            .cdef_params
            .as_ref()
            .is_some_and(|cdef| cdef.cdef_on_skip_txfm_frame_enable == Some(false));
        let luma_lr_needs_skip_grid = core.lr_params.as_ref().is_some_and(|lr| {
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
            .any(|block| block.plane == PlaneId::Y.index());
        cdef_needs_skip_grid || luma_lr_needs_skip_grid
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

fn publish_filter_stripe_to<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    stripe: &crate::filters::source::StripePlane,
) -> Result<()> {
    let error = || lr_pipeline_state_error();
    let end_y = stripe.end_y().ok_or_else(&error)?;
    let size = workspace.plane(plane).map_err(|_| error())?.storage_size();
    let mut frame = workspace.as_frame_mut();
    let view = frame.plane_mut(plane).ok_or_else(&error)?;
    let stride = view.stride_samples();
    if stripe.width() != size.width() || stripe.frame_height() != size.height() {
        return Err(error());
    }
    let samples = view.samples_mut();
    if stride == stripe.width()
        && let Some(destination) = T::u16_slice_mut(samples)
    {
        let start = stripe.origin_y().checked_mul(stride).ok_or_else(&error)?;
        let end = start
            .checked_add(stripe.samples().len())
            .ok_or_else(&error)?;
        destination
            .get_mut(start..end)
            .ok_or_else(&error)?
            .copy_from_slice(stripe.samples());
        return Ok(());
    }
    for y in stripe.origin_y()..end_y {
        let source = stripe.row(y).ok_or_else(&error)?;
        let start = y.checked_mul(stride).ok_or_else(&error)?;
        let destination = samples
            .get_mut(start..start.checked_add(source.len()).ok_or_else(&error)?)
            .ok_or_else(&error)?;
        if let Some(destination) = T::u16_slice_mut(destination) {
            destination.copy_from_slice(source);
        } else {
            for (destination, &source) in destination.iter_mut().zip(source) {
                *destination = T::try_from_u16(source).map_err(|_| error())?;
            }
        }
    }
    Ok(())
}

pub(crate) fn chroma_transform_deblock_block(
    plane_id: PlaneId,
    x: usize,
    y: usize,
    chroma_tx: usize,
    chroma_subsampling: (u32, u32),
    qindex: u32,
    lossless: bool,
) -> Option<(usize, crate::filters::deblock::DeblockBlock)> {
    let (log2_width, log2_height) = tx_size_log2(chroma_tx)?;
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
