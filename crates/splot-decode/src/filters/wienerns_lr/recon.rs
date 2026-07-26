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
use std::sync::Mutex;

use crate::Result;

use super::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use splot_core::span::ByteOffset;

const MI_SIZE: usize = 4;

/// Where one frame's filtered stripes are published.
///
/// A pipelined frame publishes into the [`FrameProgress`] its slot already
/// shares, so a consumer can read the published prefix before the freeze; every
/// other path keeps its filtered workspace local to the filter chain.
///
/// [`FrameProgress`]: crate::pipeline::frame_progress::FrameProgress
enum FilteredFrameSink<'a, T: ReconSample> {
    /// The filter chain owns the output until the freeze.
    Local(Box<Mutex<CurrentFrameWorkspace<T>>>),
    /// The frame's slot shares the output, stripe by stripe.
    Published(&'a crate::pipeline::frame_progress::FrameProgress<T>),
}

impl<'a, T: ReconSample> FilteredFrameSink<'a, T> {
    fn open(
        progress: Option<&'a crate::pipeline::frame_progress::FrameProgress<T>>,
        info: splot_recon::DecodedFrameInfo,
        ranges: &[(usize, usize)],
    ) -> Result<Self> {
        match progress {
            Some(progress) if progress.begin(ranges) => Ok(Self::Published(progress)),
            _ => Ok(Self::Local(Box::new(Mutex::new(
                CurrentFrameWorkspace::new(info, T::default())?,
            )))),
        }
    }

    fn with_workspace_mut(
        &self,
        publish: impl FnOnce(&mut CurrentFrameWorkspace<T>) -> Result<()>,
    ) -> Result<()> {
        match self {
            Self::Local(workspace) => publish(
                &mut workspace
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
            Self::Published(progress) => progress.with_workspace_mut(publish),
        }
    }

    fn publish(&self, stripe: usize) {
        if let Self::Published(progress) = self {
            progress.publish(stripe);
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
            Self::Published(progress) => progress.freeze_workspace(publish),
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

    pub(crate) fn into_filtered_frame<R>(
        mut self,
        core: &splot_core::headers::frame::FrameHeaderCore,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        progress: Option<&crate::pipeline::frame_progress::FrameProgress<T>>,
        offset: ByteOffset,
        publish: impl FnOnce(DecodedFrame<T>) -> R,
    ) -> Result<(R, super::FrameFilterRecords)> {
        if std::env::var_os("SPLOT_DECODE_SKIP_FILTERS").is_some() {
            return Ok((publish(self.workspace.freeze()?), self.filter_records));
        }
        let mi_rows = self.luma_height.div_ceil(MI_SIZE);
        let mi_cols = self.luma_width.div_ceil(MI_SIZE);
        if core
            .lossless_info
            .as_ref()
            .is_some_and(|lossless| lossless.has_lossless_segment)
        {
            self.lossless_grid = Some(
                crate::filters::lossless::LosslessBlockGrid::from_deblock_blocks(
                    mi_rows,
                    mi_cols,
                    &self.filter_records.deblock_blocks,
                    [
                        &self.filter_records.chroma_deblock_blocks[0],
                        &self.filter_records.chroma_deblock_blocks[1],
                    ],
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_lossless_grid",
                    )
                })?,
            );
        }
        if self.needs_tx_skip_grid(core) {
            self.ensure_tx_skip_grid(mi_rows, mi_cols, offset)?;
        }
        let deblock_timer = crate::timing::start();
        if let Some(filter) = core.deblocking_filter_params
            && filter.apply_deblocking_filter != [false; 4]
        {
            crate::filters::deblock::deblock_general_intra_frame(
                &mut self.workspace,
                &self.filter_records.deblock_blocks,
                [
                    &self.filter_records.chroma_deblock_blocks[0],
                    &self.filter_records.chroma_deblock_blocks[1],
                ],
                mi_rows,
                mi_cols,
                filter,
                core.tile_info.as_ref(),
                disable_loopfilters_across_tiles,
                deblock_quant_deltas,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_deblock_filter",
                )
            })?;
        }
        crate::timing::report("filter_deblock", deblock_timer);
        let cdef_skip_grid = self.cdef_skip_grid(core, mi_rows, mi_cols, offset)?;
        let cdef_strengths = crate::filters::cdef::cdef_frame_strengths(core);
        let lr_source_blocks = core::mem::take(&mut self.filter_records.lr_source_blocks);
        let lr_unit_filters = core::mem::take(&mut self.filter_records.lr_unit_filters);
        let (lr_source_blocks, plane_ends) =
            final_filters::coalesced_lr_source_rows_all(lr_source_blocks);
        let [y_end, u_end] = plane_ends;
        let y_runs = &lr_source_blocks[..y_end];
        let u_runs = &lr_source_blocks[y_end..u_end];
        let v_runs = &lr_source_blocks[u_end..];
        let ranges = crate::filters::gdf::stripe_ranges(core, self.luma_height, offset)?;
        let pixel_format = self.workspace.info().pixel_format();
        let subsampling = (
            usize::from(pixel_format.subsampling_x()),
            usize::from(pixel_format.subsampling_y()),
        );
        let ccso_config = self
            .ccso_grid
            .as_ref()
            .map(|grid| crate::filters::ccso::prepare_ccso(core, grid, self.bit_depth, subsampling))
            .transpose()
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_ccso_filter",
                )
            })?;
        let filter_timer = crate::timing::start();
        let run_stripe = |&(start, end): &(usize, usize)| -> Result<final_filters::FilteredStripe> {
            let cdef_timer = crate::timing::start();
            let mut cdef = crate::filters::cdef::cdef_stripe(
                &self.workspace,
                cdef_strengths.as_deref(),
                self.cdef_grid.as_ref(),
                cdef_skip_grid.as_ref(),
                self.lossless_grid.as_ref(),
                (mi_rows, mi_cols),
                self.bit_depth,
                start,
                end,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_cdef_filter",
                )
            })?;
            crate::timing::report("filter_cdef_stripe", cdef_timer);
            if let Some((grid, config)) = self.ccso_grid.as_ref().zip(ccso_config.as_ref()) {
                let ccso_timer = crate::timing::start();
                crate::filters::ccso::ccso_stripe(
                    &mut cdef,
                    grid,
                    config,
                    self.lossless_grid.as_ref(),
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_ccso_filter",
                    )
                })?;
                crate::timing::report("filter_ccso_stripe", ccso_timer);
            }
            let lr_timer = crate::timing::start();
            let mut frame = self.apply_lr_stripe(
                core,
                offset,
                cdef,
                [y_runs, u_runs, v_runs],
                &lr_unit_filters,
            )?;
            crate::timing::report("filter_lr_stripe", lr_timer);
            let (separate_cdef_luma, output_luma) =
                if let Some(post_lr_y) = frame.post_lr_y.as_mut() {
                    (Some(&frame.cdef_y), post_lr_y)
                } else {
                    (None, &mut frame.cdef_y)
                };
            let gdf_timer = crate::timing::start();
            crate::filters::gdf::apply_stripe(
                core,
                frame.deblocked_y,
                separate_cdef_luma,
                output_luma,
                self.gdf_grid.as_ref(),
                self.lossless_grid.as_ref(),
                self.bit_depth,
                disable_loopfilters_across_tiles,
                self.gdf_reference,
                offset,
            )?;
            crate::timing::report("filter_gdf_stripe", gdf_timer);
            Ok(frame.into_filtered())
        };
        let sink = FilteredFrameSink::open(progress, self.workspace.info(), &ranges)?;
        let run_stripe_and_publish = |stripe: usize, range: &(usize, usize)| {
            let frame = run_stripe(range)?;
            self.validate_filter_stripe(PlaneId::Y, &frame.y, offset)?;
            if let Some(plane) = frame.u.as_ref() {
                self.validate_filter_stripe(PlaneId::U, plane, offset)?;
            }
            if let Some(plane) = frame.v.as_ref() {
                self.validate_filter_stripe(PlaneId::V, plane, offset)?;
            }
            sink.with_workspace_mut(|output| {
                Self::publish_filter_stripe_to(output, PlaneId::Y, &frame.y, offset)?;
                if let Some(plane) = frame.u.as_ref() {
                    Self::publish_filter_stripe_to(output, PlaneId::U, plane, offset)?;
                }
                if let Some(plane) = frame.v.as_ref() {
                    Self::publish_filter_stripe_to(output, PlaneId::V, plane, offset)?;
                }
                Ok(())
            })?;
            sink.publish(stripe);
            Ok(())
        };
        if ranges.len() > 1 && splot_parallel::on_multiworker_pool() {
            let mut slots: Vec<Option<Result<()>>> = (0..ranges.len()).map(|_| None).collect();
            let scheduled = splot_parallel::ready_task_scope(|scope| {
                for ((stripe, range), slot) in ranges.iter().enumerate().zip(&mut slots) {
                    let run_stripe_and_publish = &run_stripe_and_publish;
                    scope.spawn(move |_| {
                        *slot = Some(run_stripe_and_publish(stripe, range));
                    });
                }
            });
            if scheduled.is_ok() {
                let missing = || {
                    wienerns_lr_selectable_transform_record_error_reason(
                        offset,
                        "unsupported_wienerns_lr_selectable_transform_records_filter_stripe_publish",
                    )
                };
                for slot in slots {
                    slot.unwrap_or_else(|| Err(missing()))?;
                }
            } else {
                for (stripe, range) in ranges.iter().enumerate() {
                    run_stripe_and_publish(stripe, range)?;
                }
            }
        } else {
            for (stripe, range) in ranges.iter().enumerate() {
                run_stripe_and_publish(stripe, range)?;
            }
        }
        crate::timing::report("filter_stripes", filter_timer);
        self.filter_records.lr_source_blocks = lr_source_blocks;
        self.filter_records.lr_unit_filters = lr_unit_filters;
        let frame = sink.freeze(publish)?;
        self.workspace.recycle_planes();
        Ok((frame, self.filter_records))
    }

    fn validate_filter_stripe(
        &self,
        plane: PlaneId,
        stripe: &crate::filters::source::StripePlane,
        offset: ByteOffset,
    ) -> Result<()> {
        let error = || {
            wienerns_lr_selectable_transform_record_error_reason(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_filter_stripe_publish",
            )
        };
        let size = self
            .workspace
            .plane(plane)
            .map_err(|_| error())?
            .storage_size();
        let end_y = stripe.end_y().ok_or_else(&error)?;
        let max_sample = self.bit_depth.max_sample();
        if stripe.width() != size.width()
            || stripe.frame_height() != size.height()
            || stripe.origin_y() > end_y
            || end_y > size.height()
            || T::try_from_u16(max_sample).is_err()
        {
            return Err(error());
        }
        Ok(())
    }

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

    fn ensure_tx_skip_grid(
        &mut self,
        mi_rows: usize,
        mi_cols: usize,
        offset: ByteOffset,
    ) -> Result<()> {
        if self.tx_skip_grid.is_some() {
            return Ok(());
        }
        let grid = super::derive_wienerns_lr_tx_skip_grid_retention(
            mi_rows,
            mi_cols,
            &self.filter_records.tx_skip_records,
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_tx_skip_grid",
            )
        })?;
        self.tx_skip_grid = Some(grid);
        Ok(())
    }

    fn publish_filter_stripe_to(
        workspace: &mut CurrentFrameWorkspace<T>,
        plane: PlaneId,
        stripe: &crate::filters::source::StripePlane,
        offset: ByteOffset,
    ) -> Result<()> {
        let error = || {
            wienerns_lr_selectable_transform_record_error_reason(
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_filter_stripe_publish",
            )
        };
        let end_y = stripe.end_y().ok_or_else(&error)?;
        let size = workspace.plane(plane).map_err(|_| error())?.storage_size();
        let mut frame = workspace.as_frame_mut();
        let view = frame.plane_mut(plane).ok_or_else(&error)?;
        let stride = view.stride_samples();
        if stripe.width() != size.width() || stripe.frame_height() != size.height() {
            return Err(error());
        }
        let samples = view.samples_mut();
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
