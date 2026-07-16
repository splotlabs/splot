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
pub(crate) struct WienerNsLrReconSink<T: ReconSample> {
    workspace: CurrentFrameWorkspace<T>,
    bit_depth: BitDepth,
    cfl_ds_filter_index: u8,
    luma_width: usize,
    luma_height: usize,
    deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    cdef_grid: Option<crate::filters::cdef::CdefUnitGrid>,
    ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    gdf_grid: Option<crate::filters::gdf::GdfBlockGrid>,
    tx_skip_grid: Option<super::WienerNsLrTxSkipGrid>,
    tx_skip_records: Vec<super::WienerNsLrTxSkipTransformRecord>,
    lr_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    lr_unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
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
            deblock_blocks: Vec::new(),
            chroma_deblock_blocks: [Vec::new(), Vec::new()],
            cdef_grid: None,
            ccso_grid: None,
            gdf_grid: None,
            tx_skip_grid: None,
            tx_skip_records: Vec::new(),
            lr_source_blocks: Vec::new(),
            lr_unit_filters: Vec::new(),
            gdf_reference: None,
            lossless_grid: None,
        }
    }

    pub(crate) fn set_deblock_blocks(
        &mut self,
        luma: Vec<crate::filters::deblock::DeblockBlock>,
        chroma: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    ) {
        self.deblock_blocks = luma;
        self.chroma_deblock_blocks = chroma;
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

    pub(crate) fn set_tx_skip_records(
        &mut self,
        records: Vec<super::WienerNsLrTxSkipTransformRecord>,
    ) {
        self.tx_skip_records = records;
    }

    pub(crate) fn set_lr_source_blocks(
        &mut self,
        blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    ) {
        self.lr_source_blocks = blocks;
    }

    pub(crate) fn set_lr_unit_filters(
        &mut self,
        filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    ) {
        self.lr_unit_filters = filters;
    }

    pub(crate) const fn set_gdf_reference_context(
        &mut self,
        context: Option<crate::filters::gdf::GdfReferenceContext>,
    ) {
        self.gdf_reference = context;
    }

    pub(crate) fn into_filtered_frame(
        mut self,
        core: &splot_core::headers::frame::FrameHeaderCore,
        disable_loopfilters_across_tiles: bool,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        offset: ByteOffset,
    ) -> Result<DecodedFrame<T>> {
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
                    &self.deblock_blocks,
                    [
                        &self.chroma_deblock_blocks[0],
                        &self.chroma_deblock_blocks[1],
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
                &self.deblock_blocks,
                [
                    &self.chroma_deblock_blocks[0],
                    &self.chroma_deblock_blocks[1],
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
        let lr_source_blocks = core::mem::take(&mut self.lr_source_blocks);
        let lr_unit_filters = core::mem::take(&mut self.lr_unit_filters);
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
            if let Some((grid, config)) = self.ccso_grid.as_ref().zip(ccso_config.as_ref()) {
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
            }
            let mut frame = self.apply_lr_stripe(
                core,
                offset,
                cdef,
                [y_runs, u_runs, v_runs],
                &lr_unit_filters,
            )?;
            let (separate_cdef_luma, output_luma) =
                if let Some(post_lr_y) = frame.post_lr_y.as_mut() {
                    (Some(&frame.cdef_y), post_lr_y)
                } else {
                    (None, &mut frame.cdef_y)
                };
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
            Ok(frame.into_filtered())
        };
        let filtered_workspace = Mutex::new(CurrentFrameWorkspace::new(
            self.workspace.info(),
            T::default(),
        )?);
        let run_stripe_and_publish = |range: &(usize, usize)| {
            let frame = run_stripe(range)?;
            self.validate_filter_stripe(PlaneId::Y, &frame.y, offset)?;
            if let Some(plane) = frame.u.as_ref() {
                self.validate_filter_stripe(PlaneId::U, plane, offset)?;
            }
            if let Some(plane) = frame.v.as_ref() {
                self.validate_filter_stripe(PlaneId::V, plane, offset)?;
            }
            let mut output = filtered_workspace
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Self::publish_filter_stripe_to(&mut output, PlaneId::Y, &frame.y, offset)?;
            if let Some(plane) = frame.u.as_ref() {
                Self::publish_filter_stripe_to(&mut output, PlaneId::U, plane, offset)?;
            }
            if let Some(plane) = frame.v.as_ref() {
                Self::publish_filter_stripe_to(&mut output, PlaneId::V, plane, offset)?;
            }
            Ok(())
        };
        if ranges.len() > 1 && splot_parallel::on_multiworker_pool() {
            let mut slots: Vec<Option<Result<()>>> = (0..ranges.len()).map(|_| None).collect();
            let scheduled = splot_parallel::ready_task_scope(|scope| {
                for (range, slot) in ranges.iter().zip(&mut slots) {
                    let run_stripe_and_publish = &run_stripe_and_publish;
                    scope.spawn(move |_| {
                        *slot = Some(run_stripe_and_publish(range));
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
                for range in &ranges {
                    run_stripe_and_publish(range)?;
                }
            }
        } else {
            for range in &ranges {
                run_stripe_and_publish(range)?;
            }
        }
        crate::timing::report("filter_stripes", filter_timer);
        let filtered_workspace = filtered_workspace
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(filtered_workspace.freeze()?)
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
            || stripe.samples().iter().any(|&sample| sample > max_sample)
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
            &self.tx_skip_records,
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
            for (destination, &source) in destination.iter_mut().zip(source) {
                *destination = T::try_from_u16(source).map_err(|_| error())?;
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
