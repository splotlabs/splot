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

use crate::Result;

use super::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use splot_core::span::ByteOffset;

/// AV2 §3 `MI_SIZE`: one mode-info unit spans four samples.
const MI_SIZE: usize = 4;
/// Wraps an already-reconstructed [`CurrentFrameWorkspace`] and the filter-state
/// inputs (deblock geometry, CDEF/CCSO/LR grids) so the shared §7.2 in-loop filter
/// chain can run over it. The workspace is filled by the unified decode engine; this
/// sink only carries the frozen samples plus the retained filter parameters.
pub(crate) struct WienerNsLrReconSink<T: ReconSample> {
    workspace: CurrentFrameWorkspace<T>,
    bit_depth: BitDepth,
    /// The §5.4.4 `cfl_ds_filter_index` sequence value used by §7.13.5 luma
    /// downsampling; value `3` aliases filter `0`.
    cfl_ds_filter_index: u8,
    luma_width: usize,
    luma_height: usize,
    deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    chroma_deblock_blocks: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    cdef_grid: Option<crate::filters::cdef::CdefUnitGrid>,
    ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    tx_skip_grid: Option<super::WienerNsLrTxSkipGrid>,
    tx_skip_records: Vec<super::WienerNsLrTxSkipTransformRecord>,
    lr_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    lr_unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
}

/// AV2 §7.13.2.17 intra edge filter strength selection process. Returns the
/// edge-filter strength `0..=3` for a `w` x `h` transform, `filter_type` (0 or 1,
/// from §7.13.2.15/16 — `1` when the relevant neighbour uses a smooth mode), and
/// `delta` (the §7.13.2.7 `angleAbove = pAngle - 90` / `angleLeft = pAngle - 180`).
/// Strength `0` means `av2_filter_intra_edge` is a no-op, so the §7.13.2.8
/// prediction over the UNFILTERED edge is bit-exact. Transcribed VERBATIM from the
/// committed spec mirror `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-17`.
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
    /// Wraps an already-reconstructed workspace so the caller can run the shared
    /// §7.2 final filter chain over it: feed the filter state via the `set_*`
    /// methods, then finish with [`Self::into_filtered_frame`].
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
            tx_skip_grid: None,
            tx_skip_records: Vec::new(),
            lr_source_blocks: Vec::new(),
            lr_unit_filters: Vec::new(),
        }
    }

    /// Hands over externally accumulated § 7.17 deblock geometry (luma list +
    /// per-plane chroma lists) for the final filter chain.
    pub(crate) fn set_deblock_blocks(
        &mut self,
        luma: Vec<crate::filters::deblock::DeblockBlock>,
        chroma: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    ) {
        self.deblock_blocks = luma;
        self.chroma_deblock_blocks = chroma;
    }

    /// Retains the parsed CDEF unit grid for the §7.2 final filter chain.
    pub(crate) fn set_cdef_grid(&mut self, grid: Option<crate::filters::cdef::CdefUnitGrid>) {
        self.cdef_grid = grid;
    }

    /// Retains the selectable walk's parsed CCSO block-enable grid for the final
    /// filter chain.
    pub(crate) fn set_ccso_grid(&mut self, grid: Option<crate::filters::ccso::CcsoUnitGrid>) {
        self.ccso_grid = grid;
    }

    /// Retains the sequence-level §5.4.4 `cfl_ds_filter_index` used by
    /// chroma Wiener NS LR luma companion reads.
    pub(crate) const fn set_cfl_ds_filter_index(&mut self, index: u8) {
        self.cfl_ds_filter_index = index;
    }

    /// Retains per-luma-transform skip/EOB facts for CDEF skip-grid and
    /// multi-class luma Wiener NS LR classification.
    pub(crate) fn set_tx_skip_records(
        &mut self,
        records: Vec<super::WienerNsLrTxSkipTransformRecord>,
    ) {
        self.tx_skip_records = records;
    }

    /// Retains active loop-restoration source blocks from the full selectable
    /// walk for final LR filtering.
    pub(crate) fn set_lr_source_blocks(
        &mut self,
        blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    ) {
        self.lr_source_blocks = blocks;
    }

    /// Retains entropy-coded per-unit Wiener NS filters from the full selectable
    /// walk for final LR filtering.
    pub(crate) fn set_lr_unit_filters(
        &mut self,
        filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
    ) {
        self.lr_unit_filters = filters;
    }

    /// Runs the §7.2 in-loop filter chain (deblock → CDEF → CCSO → LR) over
    /// the reconstructed workspace and freezes the filtered frame.
    pub(crate) fn into_filtered_frame(
        mut self,
        core: &splot_core::headers::frame::FrameHeaderCore,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
        offset: ByteOffset,
    ) -> Result<DecodedFrame<T>> {
        let mi_rows = self.luma_height.div_ceil(MI_SIZE);
        let mi_cols = self.luma_width.div_ceil(MI_SIZE);
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
        let snapshot_timer = crate::timing::start();
        let lr_plane_active = |plane_index: usize| {
            core.lr_params.as_ref().is_some_and(|lr| {
                lr.planes.get(plane_index).is_some_and(|plane| {
                    plane.restoration_type
                        == splot_core::headers::frame::FrameRestorationType::WienerNonsep
                })
            }) && self
                .lr_source_blocks
                .iter()
                .any(|block| block.plane == plane_index)
        };
        let luma_lr_active = lr_plane_active(PlaneId::Y.index());
        let u_lr_active = lr_plane_active(PlaneId::U.index());
        let v_lr_active = lr_plane_active(PlaneId::V.index());
        let any_lr_active = luma_lr_active || u_lr_active || v_lr_active;
        let deblocked_luma = if any_lr_active || self.ccso_grid.is_some() {
            self.plane_snapshot(
                PlaneId::Y,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_deblocked_luma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let deblocked_u = if u_lr_active {
            self.plane_snapshot(
                PlaneId::U,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_deblocked_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let deblocked_v = if v_lr_active {
            self.plane_snapshot(
                PlaneId::V,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_deblocked_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        crate::timing::report("filter_deblock_snapshots", snapshot_timer);
        let cdef_timer = crate::timing::start();
        let cdef_skip_grid = self.cdef_skip_grid(core, mi_rows, mi_cols, offset)?;
        if let (Some(grid), Some(strengths)) = (
            self.cdef_grid.as_ref(),
            crate::filters::cdef::cdef_frame_strengths(core),
        ) {
            crate::filters::cdef::cdef_general_intra_frame_indexed(
                &mut self.workspace,
                &strengths,
                grid,
                cdef_skip_grid.as_ref(),
                mi_rows,
                mi_cols,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_cdef_filter",
                )
            })?;
        }
        crate::timing::report("filter_cdef", cdef_timer);
        let ccso_timer = crate::timing::start();
        if let Some(grid) = self.ccso_grid.as_ref() {
            crate::filters::ccso::ccso_frame(
                &mut self.workspace,
                &deblocked_luma,
                core,
                grid,
                mi_rows,
                mi_cols,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_ccso_filter",
                )
            })?;
        }
        crate::timing::report("filter_ccso", ccso_timer);
        let lr_snapshot_timer = crate::timing::start();
        let cdef_luma = if any_lr_active {
            self.plane_snapshot(
                PlaneId::Y,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_luma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let cdef_u = if u_lr_active {
            self.plane_snapshot(
                PlaneId::U,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let cdef_v = if v_lr_active {
            self.plane_snapshot(
                PlaneId::V,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        crate::timing::report("filter_cdef_snapshots", lr_snapshot_timer);
        let lr_timer = crate::timing::start();
        let lr_source_blocks = core::mem::take(&mut self.lr_source_blocks);
        let lr_unit_filters = core::mem::take(&mut self.lr_unit_filters);
        let [y_runs, u_runs, v_runs] = if any_lr_active {
            final_filters::coalesced_lr_source_rows_all(&lr_source_blocks)
        } else {
            [Vec::new(), Vec::new(), Vec::new()]
        };
        self.apply_luma_lr_runs(
            core,
            offset,
            &y_runs,
            &lr_unit_filters,
            &deblocked_luma,
            &cdef_luma,
        )?;
        self.apply_chroma_lr_runs(
            core,
            offset,
            PlaneId::U,
            &u_runs,
            &lr_unit_filters,
            &deblocked_u,
            &cdef_u,
            &deblocked_luma,
            &cdef_luma,
        )?;
        self.apply_chroma_lr_runs(
            core,
            offset,
            PlaneId::V,
            &v_runs,
            &lr_unit_filters,
            &deblocked_v,
            &cdef_v,
            &deblocked_luma,
            &cdef_luma,
        )?;
        crate::timing::report("filter_lr", lr_timer);
        Ok(self.workspace.freeze()?)
    }

    fn needs_tx_skip_grid(&self, core: &splot_core::headers::frame::FrameHeaderCore) -> bool {
        let cdef_needs_skip_grid = core
            .cdef_params
            .as_ref()
            .is_some_and(|cdef| cdef.cdef_on_skip_txfm_frame_enable == Some(false));
        let luma_lr_needs_skip_grid = core.lr_params.as_ref().is_some_and(|lr| {
            lr.planes.get(PlaneId::Y.index()).is_some_and(|plane| {
                plane.restoration_type
                    == splot_core::headers::frame::FrameRestorationType::WienerNonsep
                    && plane.frame_filters_on
                    && plane.num_filter_classes.unwrap_or(1) > 1
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

    fn plane_snapshot(
        &self,
        plane: PlaneId,
        offset: ByteOffset,
        reason: &'static str,
    ) -> Result<Vec<u16>> {
        self.workspace
            .samples(plane)
            .map_err(|_| wienerns_lr_selectable_transform_record_error_reason(offset, reason))
            .map(|samples| samples.iter().map(|sample| sample.to_u16()).collect())
    }
}

/// § 7.17 chroma deblock geometry for one 4:2:0 chroma transform at
/// plane-sample (`x`, `y`): chroma MI cells map ×2 onto the luma MI grid.
/// Returns the chroma list index (U = 0, V = 1) with the record.
pub(crate) fn chroma_transform_deblock_block(
    plane_id: PlaneId,
    x: usize,
    y: usize,
    chroma_tx: usize,
    qindex: u32,
) -> Option<(usize, crate::filters::deblock::DeblockBlock)> {
    let (log2_width, log2_height) = tx_size_log2(chroma_tx)?;
    let plane_index = match plane_id {
        PlaneId::U => 0,
        PlaneId::V => 1,
        PlaneId::Y => return None,
    };
    let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
    Some((
        plane_index,
        crate::filters::deblock::DeblockBlock {
            r: (y / MI_SIZE).saturating_mul(2),
            c: (x / MI_SIZE).saturating_mul(2),
            block_r: (y / MI_SIZE).saturating_mul(2),
            block_c: (x / MI_SIZE).saturating_mul(2),
            chroma_base_r: (y / MI_SIZE).saturating_mul(2),
            chroma_base_c: (x / MI_SIZE).saturating_mul(2),
            n4w: mi_w.saturating_mul(2),
            n4h: mi_h.saturating_mul(2),
            luma_tx: chroma_tx,
            chroma_tx: Some(chroma_tx),
            qindex,
            skip: false,
        },
    ))
}

/// Maps a §5.20.6 `TxSize` index to its `(log2_width, log2_height)` sample
/// dimensions via the §9 `Tx_Width` / `Tx_Height` log2 tables, or `None` when the
/// index is outside the 19-entry table range.
fn tx_size_log2(tx_size: usize) -> Option<(u32, u32)> {
    let w = u32::try_from(*TX_WIDTH_LOG2.get(tx_size)?).ok()?;
    let h = u32::try_from(*TX_HEIGHT_LOG2.get(tx_size)?).ok()?;
    Some((w, h))
}

/// The MI-unit `(width, height)` of a transform with the given log2 sample
/// dimensions (one MI unit spans `MI_SIZE` samples; a transform is at least one MI
/// unit per axis).
fn mi_extent(log2_width: u32, log2_height: u32) -> (usize, usize) {
    let mi_w = (1usize << log2_width >> 2).max(1);
    let mi_h = (1usize << log2_height >> 2).max(1);
    (mi_w, mi_h)
}

mod final_filters;
pub(crate) mod full_recon;
