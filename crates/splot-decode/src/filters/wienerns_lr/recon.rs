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
use std::sync::Arc;

use crate::Result;
use crate::pipeline::inflight::FrameSlotWriter;
use parking_lot::Mutex;

const MI_SIZE: usize = 4;

/// Where one frame's filtered stripes are published.
///
/// A pipelined frame publishes into the [`FrameProgress`] its slot already
/// shares, so a consumer can read the published prefix before the freeze. An
/// inline frame wraps its reconstructed workspace in the same sink privately,
/// keeping one in-place filter path.
///
/// [`FrameProgress`]: crate::pipeline::frame_progress::FrameProgress
struct FilteredFrameSink<T: ReconSample> {
    progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
}

impl<T: ReconSample> FilteredFrameSink<T> {
    fn open(
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
        workspace: Option<CurrentFrameWorkspace<T>>,
        ranges: &[(usize, usize)],
    ) -> Result<Self> {
        let progress = match (progress, workspace) {
            (Some(progress), None) => progress,
            (None, Some(workspace)) => Arc::new(
                crate::pipeline::frame_progress::FrameProgress::from_workspace(workspace, None)?,
            ),
            _ => return Err(lr_pipeline_state_error()),
        };
        progress.begin(ranges)?;
        Ok(Self { progress })
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
        samples: final_filters::FilteredStripe<T>,
        direct: crate::pipeline::frame_progress::DirectStripeLease<T>,
    ) -> Result<()> {
        if !samples.y.is_direct()
            || samples.u.as_ref().is_some_and(|plane| !plane.is_direct())
            || samples.v.as_ref().is_some_and(|plane| !plane.is_direct())
        {
            return Err(lr_pipeline_state_error());
        }
        drop(samples);
        if !direct.submit() {
            return Err(lr_pipeline_state_error());
        }
        Ok(())
    }

    /// Freezes the filtered output into the paired slot `writer` settles, or
    /// returns the frozen frame of a private sink.
    fn freeze(mut self, writer: Option<FrameSlotWriter<T>>) -> Result<Option<DecodedFrame<T>>> {
        match writer {
            Some(writer) => self.progress.freeze_workspace(writer).map(|()| None),
            None => Arc::get_mut(&mut self.progress)
                .ok_or_else(lr_pipeline_state_error)?
                .freeze()
                .map(Some),
        }
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
    ccso_grid: Option<std::sync::Arc<crate::filters::ccso::CcsoUnitGrid>>,
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
    tx_skip_grid: Option<super::WienerNsLrTxSkipGrid>,
    gdf_reference: Option<crate::filters::gdf::GdfReferenceContext>,
    lossless_grid: Option<crate::filters::lossless::LosslessBlockGrid>,
}

/// One frame's owned final-filter state after reconstruction and deblock have
/// separated their sample ownership.
///
/// Every immutable grid and filter record lives here, so a stripe job with its
/// private input window can run on any worker.
/// The output sink and pending stripe list stay frame-owned until the single
/// terminal freeze consumes this value.
pub(crate) struct OwnedFilterSetup<T: ReconSample> {
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
    ccso_grid: Option<std::sync::Arc<crate::filters::ccso::CcsoUnitGrid>>,
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
    sink: FilteredFrameSink<T>,
    stripe_state: Mutex<Vec<StripeLifecycle>>,
    /// Each stripe's filter outcome, borrowed from the records with the rest.
    stripe_outcomes: Vec<Option<crate::Result<()>>>,
    deblock_records: Mutex<Option<crate::filters::deblock::OwnedDeblockRecords>>,
    /// Spent stripe windows of a frame that has no decode-wide pool.
    windows: Mutex<Vec<splot_recon::RetiredFramePlanes>>,
}

pub(crate) type OwnedFilterShell<T> = Arc<Option<OwnedFilterSetup<T>>>;

/// A stripe window's planes, and the rows the last CDEF block row reads of
/// each plane that holds only the rows of its first and last block rows.
type WindowPlanes<'a, T> = (
    crate::filters::source::DeblockedPlanes<'a, T>,
    [Option<crate::filters::source::FramePlane<'a, T>>; 3],
);

/// The terminal freeze's result: the frame of a private sink, or `None` once a
/// slot writer settled it, plus the records handed back to the scratch.
pub(crate) type FrozenFrame<T> = Result<(Option<DecodedFrame<T>>, super::FrameFilterRecords)>;

/// One completed stripe whose index and samples move together into publication.
pub(crate) struct OwnedFilteredStripe<T: ReconSample> {
    stripe: usize,
    frame: final_filters::FilteredStripe<T>,
    direct: crate::pipeline::frame_progress::DirectStripeLease<T>,
}

/// One scheduled stripe with its private deblocked input window.
pub(crate) struct OwnedFilterJob<T: ReconSample> {
    setup: OwnedFilterShell<T>,
    stripe: usize,
    window: crate::filters::source::DeblockedWindow<T>,
}

/// The sole setup owner after every scheduled stripe has settled.
pub(crate) struct OwnedFilterFinish<T: ReconSample> {
    setup: OwnedFilterShell<T>,
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(crate) enum StripeLifecycle {
    #[default]
    Pending,
    Claimed,
    Submitted,
}

pub(crate) fn intra_edge_filter_strength(w: u32, h: u32, filter_type: u8, delta: i32) -> u8 {
    let d = delta.unsigned_abs();
    let blk_wh = w + h;
    let mut strength = 0u8;
    if filter_type == 0 {
        if blk_wh <= 8 {
            if d >= 56 {
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

    /// Builds the sink for a scheduled frame whose workspace remains with
    /// reconstruction until its finalized rows are published for filtering.
    pub(crate) fn for_deferred_filtering(
        info: splot_recon::DecodedFrameInfo,
        plane_sizes: [Option<splot_recon::PlaneSize>; 3],
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
    ) -> Self {
        let (luma_width, luma_height) = plane_sizes[PlaneId::Y.index()]
            .map_or((luma_width, luma_height), |size| {
                (size.width(), size.height())
            });
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

    pub(crate) fn take_workspace(&mut self) -> Option<CurrentFrameWorkspace<T>> {
        self.workspace.take()
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

    pub(crate) fn set_ccso_grid(
        &mut self,
        grid: Option<std::sync::Arc<crate::filters::ccso::CcsoUnitGrid>>,
    ) {
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

    /// Builds a filter setup whose progressive output handle is owned by
    /// scheduled row tasks.
    pub(crate) fn into_owned_filter_setup_published(
        self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        progress: Arc<crate::pipeline::frame_progress::FrameProgress<T>>,
    ) -> Result<OwnedFilterSetup<T>> {
        self.into_owned_filter_setup(core, disable_loopfilters_across_tiles, Some(progress))
    }

    pub(crate) fn into_owned_filter_setup(
        mut self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        progress: Option<Arc<crate::pipeline::frame_progress::FrameProgress<T>>>,
    ) -> Result<OwnedFilterSetup<T>> {
        let mi_rows = self.luma_height.div_ceil(MI_SIZE);
        let mi_cols = self.luma_width.div_ceil(MI_SIZE);
        self.lossless_grid = lossless_grid(
            &core,
            (mi_rows, mi_cols),
            &self.filter_records.deblock_blocks,
            &self.filter_records.chroma_deblock_blocks,
        )?;
        if self.needs_lr_tx_skip_grid(&core) {
            self.ensure_tx_skip_grid(mi_rows, mi_cols)?;
        }
        let cdef_skip_grid = self.cdef_skip_grid(&core, mi_rows, mi_cols)?;
        let cdef_strengths = crate::filters::cdef::cdef_frame_strengths(
            &core,
            &mut self.filter_records.cdef_strengths,
        )
        .map(|()| core::mem::take(&mut self.filter_records.cdef_strengths));
        let lr_source_blocks = core::mem::take(&mut self.filter_records.lr_source_blocks);
        let lr_unit_filters = core::mem::take(&mut self.filter_records.lr_unit_filters);
        let (lr_source_blocks, lr_plane_ends) =
            final_filters::lr_source_blocks_by_plane(lr_source_blocks);
        let mut stripes = core::mem::take(&mut self.filter_records.stripes);
        let mut ranges = core::mem::take(&mut stripes.ranges);
        crate::filters::gdf::stripe_ranges_into(&core, self.luma_height, &mut ranges)?;
        let mut stripe_lifecycles = core::mem::take(&mut stripes.lifecycles);
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
                crate::filters::ccso::prepare_ccso(
                    &core,
                    grid,
                    self.bit_depth,
                    subsampling,
                    &mut self.filter_records.ccso_offset_luts,
                )
            })
            .transpose()
            .map_err(|error| ccso_filter_error(&error))?;
        let sink = FilteredFrameSink::open(progress, self.workspace.take(), &ranges)?;
        let stripe_count = ranges.len();
        let plane_sizes = self.plane_sizes;

        let Self {
            workspace: _,
            info: _,
            plane_sizes: _,
            bit_depth,
            cfl_ds_filter_index,
            luma_width,
            luma_height,
            mut filter_records,
            cdef_grid,
            ccso_grid,
            gdf_grid,
            tx_skip_grid,
            gdf_reference,
            lossless_grid,
        } = self;
        let windows = Mutex::new(core::mem::take(&mut filter_records.filter_windows));
        Ok(OwnedFilterSetup {
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
            stripe_state: Mutex::new({
                stripe_lifecycles.clear();
                stripe_lifecycles.resize(stripe_count, StripeLifecycle::Pending);
                stripe_lifecycles
            }),
            stripe_outcomes: core::mem::take(&mut stripes.outcomes),
            deblock_records: Mutex::new(None),
            windows,
        })
    }

    pub(crate) fn into_filtered_frame(
        self,
        core: Arc<splot_core::headers::frame::FrameHeaderCore>,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        writer: Option<FrameSlotWriter<T>>,
    ) -> FrozenFrame<T> {
        let progress = writer.as_ref().map(FrameSlotWriter::progress_handle);
        let setup =
            self.into_owned_filter_setup(core, disable_loopfilters_across_tiles, progress)?;
        let mut frontier = OwnedFilterFrontier::start(setup, deblock_quant_deltas, None)?;
        while let Some(job) = frontier.advance()? {
            job.run()?;
        }
        let (frame, _shell) = frontier.finish()?.finish(writer);
        frame
    }
}

/// Deblocks a reconstructed frame stripe by stripe and hands each stripe's
/// filter job out as soon as its input window is final, so a scheduler runs
/// the stripes concurrently while the deblock continues below them.
pub(crate) struct OwnedFilterFrontier<T: ReconSample> {
    setup: OwnedFilterShell<T>,
    rows: crate::pipeline::frame_progress::FrontierRows<T>,
    deblock: Option<crate::filters::deblock::FrameDeblock>,
    carry: crate::filters::source::DeblockedWindow<T>,
    next: usize,
}

impl<T: ReconSample> OwnedFilterFrontier<T> {
    /// Opens the frontier over the frame's rows, with the deblock owning the
    /// records it reads.
    ///
    /// # Errors
    /// Returns the deblock preparation's own diagnostic, or the state error
    /// of a frame whose frontier is already claimed.
    pub(crate) fn start(
        mut setup: OwnedFilterSetup<T>,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        shell: Option<OwnedFilterShell<T>>,
    ) -> Result<Self> {
        let mut rows = setup
            .sink
            .progress
            .frontier_rows()
            .ok_or_else(lr_pipeline_state_error)?;
        let deblock = match setup.core.deblocking_filter_params {
            Some(filter) if filter.apply_deblocking_filter == [false; 4] => None,
            Some(filter) => crate::filters::deblock::FrameDeblock::prepare_owned(
                setup.detach_deblock_records(),
                setup.mi_rows,
                setup.mi_cols,
                filter,
                setup.core.tile_info.clone(),
                setup.disable_loopfilters_across_tiles,
                deblock_quant_deltas,
                setup.subsampling,
            )
            .map_err(|error| deblock_prepare_error(&error))?,
            None => None,
        };
        if deblock.is_none() && !rows.publish_final_rows(setup.luma_height) {
            return Err(lr_pipeline_state_error());
        }
        let carry = setup.take_window();
        let setup = if let Some(mut shell) = shell
            && let Some(slot) = Arc::get_mut(&mut shell)
        {
            *slot = Some(setup);
            shell
        } else {
            Arc::new(Some(setup))
        };
        Ok(Self {
            setup,
            rows,
            deblock,
            carry,
            next: 0,
        })
    }

    /// The stripes the frame owes.
    pub(crate) fn stripes(&self) -> usize {
        self.setup
            .as_ref()
            .as_ref()
            .map_or(0, |setup| setup.ranges.len())
    }

    /// Deblocks up to the next stripe's window and returns that stripe's job,
    /// or `None` once every stripe is out.
    ///
    /// # Errors
    /// Returns the deblock's or the window copy's own diagnostic.
    pub(crate) fn advance(&mut self) -> Result<Option<OwnedFilterJob<T>>> {
        let setup = self
            .setup
            .as_ref()
            .as_ref()
            .ok_or_else(lr_pipeline_state_error)?;
        let Some(range) = setup.ranges.get(self.next) else {
            return Ok(None);
        };
        advance_deblock_for_stripe(
            self.deblock.as_mut(),
            &mut self.rows,
            range,
            setup.window_margin(),
            setup.subsampling.1,
            setup.bit_depth,
        )?;
        let job =
            OwnedFilterSetup::window_job(&self.setup, self.next, &mut self.rows, &mut self.carry)?
                .ok_or_else(lr_pipeline_state_error)?;
        self.next += 1;
        Ok(Some(job))
    }

    /// Hands the deblock records back and transfers the setup to the freeze,
    /// once every stripe job ran.
    ///
    /// # Errors
    /// Returns the state error of a deblock that did not finish.
    pub(crate) fn finish(self) -> Result<OwnedFilterFinish<T>> {
        let Self {
            setup,
            rows,
            deblock,
            carry,
            next: _,
        } = self;
        let owner = setup
            .as_ref()
            .as_ref()
            .ok_or_else(lr_pipeline_state_error)?;
        owner.give_window(carry);
        if let Some(deblock) = deblock {
            owner.restore_deblock_records(deblock.finish())?;
        }
        drop(rows);
        Ok(OwnedFilterSetup::owned_finish(setup))
    }
}

impl<T: ReconSample> OwnedFilterSetup<T> {
    /// Transfers deblock geometry to the incremental deblock owner after every
    /// setup grid that reads it has been derived.
    pub(crate) fn detach_deblock_records(
        &mut self,
    ) -> crate::filters::deblock::OwnedDeblockRecords {
        crate::filters::deblock::OwnedDeblockRecords {
            grids: core::mem::take(&mut self.filter_records.deblock_grids),
            blocks: core::mem::take(&mut self.filter_records.deblock_blocks),
            chroma: core::mem::take(&mut self.filter_records.chroma_deblock_blocks),
            bru_modes: self.filter_records.bru_modes.clone(),
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
            ccso_grid: self.ccso_grid.as_deref(),
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

    fn one_tile_row(&self) -> bool {
        self.core
            .tile_info
            .as_ref()
            .is_none_or(|tile| tile.mi_row_starts.len() <= 2)
    }

    /// The plane rows a stripe's window holds past each end of the stripe.
    fn window_margin(&self) -> usize {
        STRIPE_WINDOW_MARGIN + usize::from(!self.one_tile_row()) * TILE_ROW_FRINGE_MARGIN
    }

    /// The planes whose deblocked window rows a filter of this frame may read.
    ///
    /// A plane is left out only when CDEF is off, no `cdef_overlap_planes`
    /// fringe is built, and the stripe target already holds the
    /// deblocked rows, so CDEF initializes it without a read. Then GDF and
    /// CCSO read only luma, and loop restoration of any plane reads luma and
    /// its own plane.
    fn window_reads(&self) -> Result<[bool; 3]> {
        if !self.one_tile_row() || (self.cdef_strengths.is_some() && self.cdef_grid.is_some()) {
            return Ok([true; 3]);
        }
        let [y_end, u_end] = self.lr_plane_ends;
        let lr = [
            y_end > 0,
            u_end > y_end,
            self.lr_source_blocks.len() > u_end,
        ];
        let luma = lr.contains(&true)
            || (self.ccso_grid.is_some() && self.ccso_config.is_some())
            || crate::filters::gdf::is_active(
                &self.core,
                self.gdf_grid.as_ref(),
                self.bit_depth,
                self.gdf_reference,
            )?;
        Ok([luma, lr[1], lr[2]])
    }

    fn ready_stripe(&self, stripe: usize, final_rows: usize) -> Result<Option<(usize, usize)>> {
        let (start, end) = self.stripe_bounds(stripe)?;
        let needed = end
            .checked_add(self.window_margin() << self.subsampling.1)
            .ok_or_else(lr_pipeline_state_error)?
            .min(self.luma_height);
        Ok((final_rows.min(self.luma_height) >= needed).then_some((start, end)))
    }

    fn spare_windows(&self) -> &Mutex<Vec<splot_recon::RetiredFramePlanes>> {
        self.filter_records
            .buffers
            .as_deref()
            .map_or(&self.windows, |buffers| &buffers.filter_windows)
    }

    fn take_window(&self) -> crate::filters::source::DeblockedWindow<T> {
        self.spare_windows()
            .lock()
            .pop()
            .map(|mut spent| {
                crate::filters::source::DeblockedWindow::from_samples(T::reclaim_planes(&mut spent))
            })
            .unwrap_or_default()
    }

    fn give_window(&self, window: crate::filters::source::DeblockedWindow<T>) {
        let mut windows = self.spare_windows().lock();
        if windows.try_reserve(1).is_ok() {
            windows.push(T::retire_planes(window.into_samples()));
        }
    }

    /// Copies one final stripe's input window out of the frame.
    fn fill_window(
        &self,
        stripe: usize,
        frame: &mut crate::pipeline::frame_progress::FrontierRows<T>,
        carry: &mut crate::filters::source::DeblockedWindow<T>,
    ) -> Result<crate::filters::source::DeblockedWindow<T>> {
        let range = self
            .ready_stripe(stripe, frame.final_luma_rows())?
            .ok_or_else(lr_pipeline_state_error)?;
        let mut window = self.take_window();
        let filled = self.window_reads().and_then(|read| {
            let chain = self.chain();
            let [y_end, u_end] = self.lr_plane_ends;
            let (y_runs, rest) = self.lr_source_blocks.split_at(y_end);
            let (u_runs, v_runs) = rest.split_at(u_end - y_end);
            let active_lr = chain.active_lr_planes(range.0, range.1, [y_runs, u_runs, v_runs]);
            let gdf_active = crate::filters::gdf::is_active(
                &self.core,
                chain.gdf_grid,
                self.bit_depth,
                chain.gdf_reference,
            )?;
            let [all, chroma, luma] = self.stripe_in_place(&chain, range, active_lr, gdf_active);
            let skip = [all || luma, chroma, chroma];
            let edges = self.edge_planes(range, active_lr, skip, [y_runs, u_runs, v_runs], frame);
            let margin = self.window_margin();
            window.copy_window(frame, Some(carry), range, margin, read, skip, edges)
        });
        if let Err(error) = filled {
            self.give_window(window);
            return Err(error);
        }
        Ok(window)
    }

    /// The planes whose window holds only the rows the first and the last
    /// CDEF block row of the stripe read: one tile row, and loop restoration
    /// writes the whole stripe of the plane, so CDEF reads the deblocked rows
    /// inside the stripe from its target before loop restoration writes it.
    /// CDEF reads U and V at one offset, so both chroma planes or neither. A
    /// frame with a lossless segment keeps the full window: loop restoration
    /// copies its lossless samples from the deblocked rows inside the stripe.
    fn edge_planes(
        &self,
        (start, end): (usize, usize),
        active_lr: [bool; 3],
        in_place: [bool; 3],
        blocks: [&[crate::bitstream::tile_payload::WienerNsLrSourceBlock]; 3],
        frame: &crate::pipeline::frame_progress::FrontierRows<T>,
    ) -> [bool; 3] {
        let [luma, u, v] = core::array::from_fn(|index| {
            let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][index];
            let shift = usize::from(index > 0) * self.subsampling.1;
            let params = self
                .core
                .lr_params
                .as_ref()
                .and_then(|lr| lr.planes.get(index));
            self.one_tile_row()
                && !self
                    .core
                    .lossless_info
                    .as_ref()
                    .is_some_and(|lossless| lossless.has_lossless_segment)
                && active_lr[index]
                && !in_place[index]
                && frame
                    .plane_size(plane)
                    .zip(params)
                    .is_some_and(|((width, height), params)| {
                        final_filters::lr_plane_fully_overwritten(
                            blocks[index],
                            plane,
                            params.restoration_type,
                            width,
                            height,
                            start >> shift,
                            end.div_ceil(1 << shift).min(height),
                        )
                    })
        });
        [luma, u && v, u && v]
    }

    /// Claims, filters and publishes one stripe, then keeps its window.
    fn run_window(
        &self,
        stripe: usize,
        window: crate::filters::source::DeblockedWindow<T>,
    ) -> Result<()> {
        let result = self.run_filled(stripe, &window);
        self.give_window(window);
        result
    }

    /// Claims, filters and publishes one stripe from its filled window.
    fn run_filled(
        &self,
        stripe: usize,
        window: &crate::filters::source::DeblockedWindow<T>,
    ) -> Result<()> {
        self.claim(stripe)
            .and_then(|range| {
                let planes = window.planes().ok_or_else(lr_pipeline_state_error)?;
                let lower = [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| window.lower(plane));
                self.run_claimed_planes(stripe, range, &(planes, lower))
            })
            .and_then(|filtered| self.publish(filtered))
    }

    fn stripe_bounds(&self, stripe: usize) -> Result<(usize, usize)> {
        self.ranges
            .get(stripe)
            .copied()
            .ok_or_else(lr_pipeline_state_error)
    }

    /// Restores the moved deblock vectors exactly once before terminal freeze.
    /// Rebuilds the lossless grid from the deblock records of a frame whose
    /// frontier took them before this setup was built.
    pub(crate) fn set_lossless_grid(
        &mut self,
        blocks: &[crate::filters::deblock::DeblockBlock],
        chroma: &crate::filters::deblock::ChromaDeblockRecords,
    ) -> Result<()> {
        self.lossless_grid =
            lossless_grid(&self.core, (self.mi_rows, self.mi_cols), blocks, chroma)?;
        Ok(())
    }

    pub(crate) fn restore_deblock_records(
        &self,
        records: crate::filters::deblock::OwnedDeblockRecords,
    ) -> Result<()> {
        let mut slot = self.deblock_records.lock();
        if slot.is_some() {
            return Err(lr_pipeline_state_error());
        }
        *slot = Some(records);
        Ok(())
    }

    fn run_claimed_planes(
        &self,
        stripe: usize,
        range: &(usize, usize),
        deblocked: &WindowPlanes<'_, T>,
    ) -> Result<OwnedFilteredStripe<T>> {
        let mut direct = self.sink.direct_stripe(stripe)?;
        let target = direct.take_target().ok_or_else(lr_pipeline_state_error)?;
        Ok(OwnedFilteredStripe {
            stripe,
            frame: self.run_planes(range, deblocked, target)?,
            direct,
        })
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
            let mut state = self.stripe_state.lock();
            let Some(lifecycle) = state.get_mut(stripe.stripe) else {
                return Err(lr_pipeline_state_error());
            };
            if *lifecycle != StripeLifecycle::Claimed {
                return Err(lr_pipeline_state_error());
            }
            *lifecycle = StripeLifecycle::Submitted;
        }
        FilteredFrameSink::<T>::publish_stripe(stripe.frame, stripe.direct)
    }

    fn claim(&self, stripe: usize) -> Result<&(usize, usize)> {
        let range = self
            .ranges
            .get(stripe)
            .ok_or_else(lr_pipeline_state_error)?;
        let mut state = self.stripe_state.lock();
        let lifecycle = state.get_mut(stripe).ok_or_else(lr_pipeline_state_error)?;
        if *lifecycle != StripeLifecycle::Pending {
            return Err(lr_pipeline_state_error());
        }
        *lifecycle = StripeLifecycle::Claimed;
        Ok(range)
    }

    /// Tile MI starts when § 7.5's in-loop filters must stay inside their tile.
    fn tile_starts(&self) -> Option<(&[u32], &[u32])> {
        if !self.disable_loopfilters_across_tiles {
            return None;
        }
        self.core
            .tile_info
            .as_ref()
            .map(|tile| (tile.mi_row_starts.as_ref(), tile.mi_col_starts.as_ref()))
    }

    fn cdef_ccso_range<'d>(
        &self,
        deblocked: crate::filters::source::DeblockedPlanes<'d, T>,
        chain: &StripeChain<'_>,
        start: usize,
        end: usize,
        target: Option<crate::pipeline::frame_progress::DirectStripeTarget<T>>,
        luma_output: bool,
    ) -> Result<crate::filters::cdef::CdefFrame<'d, T>> {
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
            luma_output,
        )
        .map_err(|error| cdef_filter_error(&error))?;
        if let Some((grid, config)) = chain.ccso_grid.zip(self.ccso_config.as_ref()) {
            crate::filters::ccso::ccso_stripe(
                &mut cdef,
                grid,
                config,
                chain.lossless_grid,
                self.tile_starts(),
                self.filter_records.bru_modes.as_deref(),
            )
            .map_err(|error| ccso_filter_error(&error))?;
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
    ) -> Result<final_filters::CdefOverlap<T>> {
        const CDEF_START_ALIGN: usize = 8;
        let mut overlap = final_filters::CdefOverlap::default();
        if self.one_tile_row() {
            return Ok(overlap);
        }
        let frame_height = self.ranges.last().map_or(0, |&(_, last_end)| last_end);
        let push_range = |overlap: &mut final_filters::CdefOverlap<T>,
                          range_start: usize,
                          range_end: usize|
         -> Result<()> {
            let fringe =
                self.cdef_ccso_range(deblocked, chain, range_start, range_end, None, true)?;
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

    /// Whether no filter changes stripe `(start, end)`, whether none changes
    /// its chroma planes, so their targets keep the deblocked rows, and
    /// whether none changes its luma and the chroma filters read no luma past
    /// the stripe, so the luma target serves as the deblocked luma.
    fn stripe_in_place(
        &self,
        chain: &StripeChain<'_>,
        (start, end): (usize, usize),
        active_lr: [bool; 3],
        gdf_active: bool,
    ) -> [bool; 3] {
        let [luma, chroma] = crate::filters::cdef::cdef_stripe_identity(
            self.cdef_strengths.as_deref(),
            chain.cdef_grid,
            self.cdef_skip_grid.as_ref(),
            (self.mi_rows, self.mi_cols),
            (start, end),
        );
        let ccso = chain.ccso_grid.and(self.ccso_config.as_ref());
        let unfiltered = |plane: PlaneId| {
            !active_lr[plane.index()] && !ccso.is_some_and(|config| config.filters_plane(plane))
        };
        let chroma = chroma && unfiltered(PlaneId::U) && unfiltered(PlaneId::V);
        let luma = luma && !gdf_active && unfiltered(PlaneId::Y);
        let chroma_reads_luma = active_lr[PlaneId::U.index()]
            || active_lr[PlaneId::V.index()]
            || ccso.is_some_and(|config| {
                config.filters_plane(PlaneId::U) || config.filters_plane(PlaneId::V)
            });
        [
            chroma && luma,
            chroma,
            luma && !chroma && !chroma_reads_luma,
        ]
    }

    fn run_planes(
        &self,
        &(start, end): &(usize, usize),
        &(deblocked, lower): &WindowPlanes<'_, T>,
        mut target: crate::pipeline::frame_progress::DirectStripeTarget<T>,
    ) -> Result<final_filters::FilteredStripe<T>> {
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
        let [all_in_place, chroma_in_place, luma_in_place] =
            self.stripe_in_place(&chain, (start, end), active_lr, gdf_active);
        let mut in_place = |plane: PlaneId| {
            target
                .take(plane)
                .map(|target| {
                    target
                        .holds_deblocked()
                        .then(|| crate::filters::source::StripePlane::from_target(target))
                        .ok_or_else(lr_pipeline_state_error)
                })
                .transpose()
        };
        if all_in_place {
            return Ok(final_filters::FilteredStripe {
                y: in_place(PlaneId::Y)?.ok_or_else(lr_pipeline_state_error)?,
                u: in_place(PlaneId::U)?,
                v: in_place(PlaneId::V)?,
            });
        }
        let chroma = if chroma_in_place {
            Some([in_place(PlaneId::U)?, in_place(PlaneId::V)?])
        } else {
            None
        };
        if luma_in_place {
            let luma = in_place(PlaneId::Y)?.ok_or_else(lr_pipeline_state_error)?;
            let deblocked = crate::filters::source::DeblockedPlanes {
                y: luma.view().map_err(|_| lr_pipeline_state_error())?,
                ..deblocked
            };
            let crate::filters::cdef::CdefFrame {
                filtered_u: u,
                filtered_v: v,
                ..
            } = self.cdef_ccso_range(deblocked, &chain, start, end, Some(target), false)?;
            return Ok(final_filters::FilteredStripe { y: luma, u, v });
        }
        let deblocked = if chroma_in_place {
            crate::filters::source::DeblockedPlanes {
                u: None,
                v: None,
                ..deblocked
            }
        } else {
            deblocked
        };
        let lr_initializations =
            final_filters::lr_initializations(&self.core, active_lr, plane_blocks, &target);
        let edges = lower.map(|plane| plane.is_some());
        let lr_copies = core::array::from_fn(|index| {
            edges[index]
                || active_lr[index]
                    && !target
                        .get([PlaneId::Y, PlaneId::U, PlaneId::V][index])
                        .is_some_and(
                            crate::pipeline::frame_progress::DirectPlaneTarget::holds_deblocked,
                        )
        });
        let (cdef_target, lr_target) = target.split(lr_copies);
        let planes = [Some(deblocked.y), deblocked.u, deblocked.v];
        let mut edged = [None; 3];
        let mut cores = [None; 3];
        for (index, plane) in [PlaneId::Y, PlaneId::U, PlaneId::V].into_iter().enumerate() {
            let Some(lower) = lower[index].as_ref() else {
                continue;
            };
            let core = lr_target
                .get(plane)
                .filter(|target| target.holds_deblocked())
                .and_then(crate::pipeline::frame_progress::DirectPlaneTarget::rows)
                .filter(|_| {
                    lr_initializations[index]
                        == crate::filters::source::StripeInitialization::FullyOverwritten
                });
            edged[index] = planes[index].map(|plane| plane.over(lower));
            cores[index] = Some(core.ok_or_else(lr_pipeline_state_error)?);
        }
        let full: [_; 3] = core::array::from_fn(|index| {
            cores[index]
                .zip(edged[index].as_ref())
                .map(|(core, edged)| core.over(edged))
        });
        let cdef = self.cdef_ccso_range(
            crate::filters::source::DeblockedPlanes {
                y: full[0].or(planes[0]).ok_or_else(lr_pipeline_state_error)?,
                u: full[1].or(planes[1]),
                v: full[2].or(planes[2]),
            },
            &chain,
            start,
            end,
            Some(cdef_target),
            true,
        )?;
        let cdef = crate::filters::cdef::CdefFrame {
            deblocked_y: edged[0].or(planes[0]).ok_or_else(lr_pipeline_state_error)?,
            deblocked_u: edged[1].or(planes[1]),
            deblocked_v: edged[2].or(planes[2]),
            filtered_y: cdef.filtered_y,
            filtered_u: cdef.filtered_u,
            filtered_v: cdef.filtered_v,
        };
        let cdef_overlap = self.cdef_overlap_planes(deblocked, &chain, start, end)?;
        let mut frame = chain.apply_lr_stripe(
            &self.core,
            cdef,
            &cdef_overlap,
            plane_blocks,
            &self.lr_unit_filters,
            final_filters::LrStripeOutput {
                active_planes: active_lr,
                initializations: lr_initializations,
                target: lr_target,
            },
        )?;
        if gdf_active {
            let (separate_cdef_luma, output_luma) =
                if let Some(post_lr_y) = frame.post_lr_y.as_mut() {
                    (Some(&frame.cdef_y), post_lr_y)
                } else {
                    (None, &mut frame.cdef_y)
                };
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
        }
        let mut filtered = frame.into_filtered();
        if let Some([u, v]) = chroma {
            (filtered.u, filtered.v) = (u, v);
        }
        Ok(filtered)
    }

    /// Publishes every completed stripe, then consumes the sole output owner to
    /// freeze exactly once.
    pub(crate) fn finish(mut self, writer: Option<FrameSlotWriter<T>>) -> FrozenFrame<T> {
        let complete = self
            .stripe_state
            .get_mut()
            .iter()
            .all(|lifecycle| *lifecycle == StripeLifecycle::Submitted);
        if !complete {
            return Err(lr_pipeline_state_error());
        }
        self.filter_records.lr_source_blocks = self.lr_source_blocks;
        self.filter_records.lr_unit_filters = self.lr_unit_filters;
        self.filter_records.stripes.ranges = core::mem::take(&mut self.ranges);
        self.filter_records.stripes.lifecycles = core::mem::take(self.stripe_state.get_mut());
        self.filter_records.stripes.outcomes = core::mem::take(&mut self.stripe_outcomes);
        self.filter_records.filter_windows = core::mem::take(self.windows.get_mut());
        let has_restored_deblock = self.deblock_records.get_mut().is_some();
        if has_restored_deblock
            && (!self.filter_records.deblock_blocks.is_empty()
                || !self.filter_records.chroma_deblock_blocks.is_empty())
        {
            return Err(lr_pipeline_state_error());
        }
        if let Some(records) = self.deblock_records.into_inner() {
            self.filter_records.deblock_grids = records.grids;
            self.filter_records.deblock_blocks = records.blocks;
            self.filter_records.chroma_deblock_blocks = records.chroma;
        }
        if let Some(grid) = self.cdef_grid.take() {
            self.filter_records.cdef_grid_values = grid.into_values();
        }
        if let Some(strengths) = self.cdef_strengths.take() {
            self.filter_records.cdef_strengths = strengths;
        }
        if let Some(grid) = self.tx_skip_grid.take() {
            self.filter_records.tx_skip_grid_values = grid.into_values();
        }
        if let Some(config) = self.ccso_config.take() {
            config.return_offset_luts(&mut self.filter_records.ccso_offset_luts);
        }
        let frame = self.sink.freeze(writer)?;
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
        self.setup
            .as_ref()
            .as_ref()
            .ok_or_else(lr_pipeline_state_error)?
            .run_window(self.stripe, self.window)
    }
}

impl<T: ReconSample> OwnedFilterFinish<T> {
    /// Freezes only when this is the last live setup owner.
    pub(crate) fn finish(
        self,
        writer: Option<FrameSlotWriter<T>>,
    ) -> (FrozenFrame<T>, OwnedFilterShell<T>) {
        let mut setup = self.setup;
        let result = Arc::get_mut(&mut setup)
            .ok_or_else(lr_pipeline_state_error)
            .and_then(|setup| setup.take().ok_or_else(lr_pipeline_state_error))
            .and_then(|setup| setup.finish(writer));
        (result, setup)
    }
}

impl<T: ReconSample> OwnedFilterSetup<T> {
    /// Copies one stripe's input window out of the frame once its rows are
    /// final, and wraps it in the stripe's job; `None` while they are not.
    pub(crate) fn window_job(
        setup: &OwnedFilterShell<T>,
        stripe: usize,
        frame: &mut crate::pipeline::frame_progress::FrontierRows<T>,
        carry: &mut crate::filters::source::DeblockedWindow<T>,
    ) -> Result<Option<OwnedFilterJob<T>>> {
        let owner = setup
            .as_ref()
            .as_ref()
            .ok_or_else(lr_pipeline_state_error)?;
        if owner
            .ready_stripe(stripe, frame.final_luma_rows())?
            .is_none()
        {
            return Ok(None);
        }
        Ok(Some(OwnedFilterJob {
            setup: Arc::clone(setup),
            stripe,
            window: owner.fill_window(stripe, frame, carry)?,
        }))
    }

    /// Transfers terminal ownership to the exactly-once freeze job.
    pub(crate) fn owned_finish(setup: OwnedFilterShell<T>) -> OwnedFilterFinish<T> {
        OwnedFilterFinish { setup }
    }
}

/// The lossless grid of a frame with a lossless segment, laid out from its
/// deblock records.
fn lossless_grid(
    core: &splot_core::headers::frame::FrameHeaderCore,
    (mi_rows, mi_cols): (usize, usize),
    blocks: &[crate::filters::deblock::DeblockBlock],
    chroma: &crate::filters::deblock::ChromaDeblockRecords,
) -> Result<Option<crate::filters::lossless::LosslessBlockGrid>> {
    if !core
        .lossless_info
        .as_ref()
        .is_some_and(|lossless| lossless.has_lossless_segment)
    {
        return Ok(None);
    }
    crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
        mi_rows, mi_cols, blocks, chroma,
    )
    .map(Some)
    .map_err(|error| match error {
        crate::filters::lossless::LosslessGridError::Allocation(plane) => {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane,
                context: "lossless block grid",
            }
            .into()
        }
        crate::filters::lossless::LosslessGridError::Geometry => lr_pipeline_state_error(),
    })
}

/// How many plane rows past each end of a stripe the § 7.2 chain reads.
///
/// CDEF reaches two samples past the stripe and § 7.17 loop restoration clamps
/// its own reads to the stripe it is filtering plus one row. Every read outside
/// the window is refused rather than served from another stripe's rows.
const STRIPE_WINDOW_MARGIN: usize = 2;

/// The extra rows the post-CCSO fringes of a frame with more than one tile row
/// read past each stripe end (`cdef_overlap_planes`).
const TILE_ROW_FRINGE_MARGIN: usize = 8;

/// Deblocks far enough to copy one stripe's finalized input window.
///
/// The private window lets the stripe chain run while deblock continues in
/// place below it.
fn advance_deblock_for_stripe<T: ReconSample>(
    sections: Option<&mut crate::filters::deblock::FrameDeblock>,
    source: &mut crate::pipeline::frame_progress::FrontierRows<T>,
    range: &(usize, usize),
    margin: usize,
    subsampling_y: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    let needed = range.1 + (margin << subsampling_y);
    if let Some(sections) = sections {
        let reach = crate::filters::deblock::DEBLOCK_PASS_1_REACH << subsampling_y;
        sections
            .advance_source(
                source,
                needed.saturating_add(reach).div_ceil(MI_SIZE),
                bit_depth,
            )
            .map_err(|_| lr_pipeline_state_error())?;
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

pub(crate) fn lr_pipeline_state_error() -> crate::error::DecodeError {
    crate::error::DecodeHeaderStateError::InvalidLoopRestorationFilterState.into()
}

/// Immutable setup state one filter stripe reads alongside its source lease.
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
            ccso_grid: self.ccso_grid.as_deref(),
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
    fn validate_filter_stripe<T: ReconSample>(
        &self,
        plane: PlaneId,
        stripe: &crate::filters::source::StripePlane<T>,
    ) -> Result<()> {
        let error = || lr_pipeline_state_error();
        let size = self.plane_sizes[plane.index()].ok_or_else(error)?;
        let end_y = stripe.end_y().ok_or_else(error)?;
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
        let grid = super::derive_wienerns_lr_tx_skip_grid_reusing(
            mi_rows,
            mi_cols,
            &self.filter_records.tx_skip_records,
            core::mem::take(&mut self.filter_records.tx_skip_grid_values),
        )
        .map_err(|_| lr_pipeline_state_error())?;
        self.tx_skip_grid = Some(grid);
        Ok(())
    }
}

pub(crate) fn chroma_transform_deblock_block(
    x: usize,
    y: usize,
    chroma_tx: usize,
    log2_dimensions: Option<(u32, u32)>,
    chroma_subsampling: (u32, u32),
    qindex: u16,
    lossless: bool,
) -> Option<crate::filters::deblock::DeblockBlock> {
    let [r, c, n4w, n4h] =
        chroma_transform_cells(x, y, chroma_tx, log2_dimensions, chroma_subsampling)?;
    Some(crate::filters::deblock::DeblockBlock {
        r: r as u16,
        c: c as u16,
        n4w: n4w as u8,
        n4h: n4h as u8,
        tx: chroma_tx as u8,
        skip: false,
        lossless,
        qindex,
        prediction: crate::filters::deblock::DeblockPredictionUnit::new(r, c, chroma_tx),
        sub_pu_size: None,
    })
}

/// The luma mode-info row, column, width and height of the chroma transform
/// block `chroma_tx` (of `log2_dimensions` when given) at plane sample
/// (`x`, `y`), which its deblock record covers.
fn chroma_transform_cells(
    x: usize,
    y: usize,
    chroma_tx: usize,
    log2_dimensions: Option<(u32, u32)>,
    (sub_x, sub_y): (u32, u32),
) -> Option<[usize; 4]> {
    let (log2_width, log2_height) = log2_dimensions.or_else(|| tx_size_log2(chroma_tx))?;
    let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
    let scale_x = 1usize.checked_shl(sub_x)?;
    let scale_y = 1usize.checked_shl(sub_y)?;
    Some([
        (y / MI_SIZE).saturating_mul(scale_y),
        (x / MI_SIZE).saturating_mul(scale_x),
        mi_w.saturating_mul(scale_x),
        mi_h.saturating_mul(scale_y),
    ])
}

fn tx_size_log2(tx_size: usize) -> Option<(u32, u32)> {
    Some((
        *TX_WIDTH_LOG2.get(tx_size)? as u32,
        *TX_HEIGHT_LOG2.get(tx_size)? as u32,
    ))
}

fn mi_extent(log2_width: u32, log2_height: u32) -> (usize, usize) {
    let mi_w = (1usize << log2_width >> 2).max(1);
    let mi_h = (1usize << log2_height >> 2).max(1);
    (mi_w, mi_h)
}

mod final_filters;
pub(crate) mod full_recon;
