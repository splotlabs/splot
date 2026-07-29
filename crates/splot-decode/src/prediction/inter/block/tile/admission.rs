// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Admission-scheduled reconstruction state for one parsed tile.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`,
//! `INFRA-DECODE-FRAME-PIPELINING`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use splot_parallel::Condition;

use super::*;
use crate::prediction::inter::block::row_gate::RowReferenceGate;

struct ScheduledCommit<T: ReconSample> {
    next: usize,
    ordered: deferred_recon::InterReconScratch<T>,
    workspace: CurrentFrameWorkspace<T>,
    block_decoded: TileBlockDecodedState,
    current_block_decoded_superblock: Option<[usize; 2]>,
    records: crate::filters::wienerns_lr::FrameFilterRecords,
    decoded_any: bool,
}

/// Completed scheduled tile reconstruction.
pub(in crate::prediction::inter::block) struct ScheduledTileOutput<T: ReconSample> {
    pub(in crate::prediction::inter::block) workspace: CurrentFrameWorkspace<T>,
    pub(in crate::prediction::inter::block) records:
        crate::filters::wienerns_lr::FrameFilterRecords,
}

/// Owned state shared by one admission job per parsed reconstruction unit.
pub(in crate::prediction::inter::block) struct ScheduledTileRecon<T: ReconSample> {
    rows: Mutex<Vec<Option<ReadyReconRow<'static, T>>>>,
    prepared: Mutex<Vec<Option<ReadyReconRow<'static, T>>>>,
    unit_count: usize,
    batch_size: usize,
    batch_count: usize,
    commit: Mutex<Option<ScheduledCommit<T>>>,
    workers: InterReconScratchPool<T>,
    prepass_block_decoded: TileBlockDecodedState,
    row_buffers: ReconRowBufferPool,
    motion: MotionFieldUnits,
    info: splot_recon::DecodedFrameInfo,
    params: TileWalkParams,
    quantizer: FrameQuantizerSnapshot,
    temporal: Arc<TemporalMvContext>,
    reference: Arc<InterReferenceState<T>>,
    ref_frame_idx: Arc<[u32]>,
    sequence: Arc<SequenceHeader>,
    core: Arc<FrameHeaderCore>,
    tile_offset: ByteOffset,
    finished: AtomicBool,
}

impl<T: ReconSample> ScheduledTileRecon<T> {
    /// Number of independently admitted reconstruction units.
    pub(in crate::prediction::inter::block) const fn len(&self) -> usize {
        self.batch_count
    }

    fn batch_range(&self, index: usize) -> core::ops::Range<usize> {
        let start = index.saturating_mul(self.batch_size).min(self.unit_count);
        let end = start.saturating_add(self.batch_size).min(self.unit_count);
        start..end
    }

    /// Conditions that replace the parsed unit's former cross-frame wait.
    pub(in crate::prediction::inter::block) fn conditions(
        &self,
        index: usize,
    ) -> Vec<Condition<'_>> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut bounds = row_gate::RowReferenceBounds::default();
        for ready in rows
            .get(self.batch_range(index))
            .into_iter()
            .flatten()
            .filter_map(Option::as_ref)
        {
            bounds.merge(ready.bounds);
        }
        RowReferenceGate::new(
            &self.reference,
            &self.core,
            &self.ref_frame_idx,
            self.info,
            &self.temporal,
        )
        .conditions(&bounds)
    }

    /// Precomputes one admitted unit without entering the ordered commit spine.
    pub(in crate::prediction::inter::block) fn precompute(&self, index: usize) -> Result<()> {
        if self.finished.load(Ordering::Acquire) {
            return Ok(());
        }
        let range = self.batch_range(index);
        let ready = {
            let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(rows) = rows.get_mut(range.clone()) else {
                return Err(inter_cap!(
                    "inter_admission_unit_range_missing",
                    self.tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ));
            };
            rows.iter_mut()
                .map(Option::take)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    inter_cap!(
                        "inter_admission_unit_rows_missing",
                        self.tile_offset,
                        "inter.row.task_capacity",
                        SPEC_MODE_INFO
                    )
                })?
        };
        let ready = self.workers.with_scratch(|scratch| {
            let _quantizer_scopes = self.quantizer.install_frame();
            let mut ready = ready;
            let shared = deferred_recon::ReconShared {
                reference: &self.reference,
                ref_frame_idx: &self.ref_frame_idx,
                temporal_context: &self.temporal,
                sequence: &self.sequence,
                core: &self.core,
                luma_use_tcq: self.params.luma_use_tcq,
                residual_use_ddt: self.params.residual_use_ddt,
                bit_depth: self.params.bit_depth,
                mi_rows: self.params.mi_rows,
                mi_cols: self.params.mi_cols,
                current_order_hint: self.params.current_order_hint,
            };
            for ready in &mut ready {
                if ready.row.terminal.is_none() && !ready.row.motion_derived {
                    mvres::derive_unit_motion(
                        &mut ready.row,
                        ready.surface.as_mut(),
                        scratch,
                        &self.motion,
                        &shared,
                    );
                }
            }
            ready
                .into_iter()
                .map(|ready| {
                    precompute_recon_row(
                        ready,
                        scratch,
                        &self.prepass_block_decoded,
                        &self.motion,
                        &self.quantizer,
                        &self.temporal,
                        &self.reference,
                        &self.ref_frame_idx,
                        &self.sequence,
                        &self.core,
                        self.params.sb_h4,
                        self.params.mi_rows,
                        self.params.mi_cols,
                        self.params.current_order_hint,
                        self.params.luma_use_tcq,
                        self.params.residual_use_ddt,
                        self.params.bit_depth,
                    )
                })
                .collect::<Vec<_>>()
        });
        let mut prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
        for ready in ready {
            let ordinal = ready.row.ordinal;
            let Some(slot) = prepared.get_mut(ordinal) else {
                self.finished.store(true, Ordering::Release);
                return Err(inter_cap!(
                    "inter_admission_unit_ordinal",
                    self.tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ));
            };
            if slot.is_some() {
                self.finished.store(true, Ordering::Release);
                return Err(inter_cap!(
                    "inter_admission_unit_duplicate",
                    self.tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ));
            }
            *slot = Some(ready);
        }
        Ok(())
    }

    /// Commits one precomputed unit after its predecessor has completed.
    ///
    /// The one caller that commits the final unit receives the completed tile.
    pub(in crate::prediction::inter::block) fn commit(
        &self,
        index: usize,
    ) -> Result<Option<ScheduledTileOutput<T>>> {
        if self.finished.load(Ordering::Acquire) {
            return Ok(None);
        }
        let ready = {
            let mut prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(prepared) = prepared.get_mut(self.batch_range(index)) else {
                return Err(inter_cap!(
                    "inter_admission_prepared_range_missing",
                    self.tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ));
            };
            prepared
                .iter_mut()
                .map(Option::take)
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    inter_cap!(
                        "inter_admission_prepared_rows_missing",
                        self.tile_offset,
                        "inter.row.task_capacity",
                        SPEC_MODE_INFO
                    )
                })?
        };
        let mut holder = self.commit.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(commit) = holder.as_mut() else {
            return Ok(None);
        };
        for ready in ready {
            if commit.next != ready.row.ordinal {
                self.finished.store(true, Ordering::Release);
                return Err(inter_cap!(
                    "inter_admission_commit_order",
                    self.tile_offset,
                    "inter.row.task_capacity",
                    SPEC_MODE_INFO
                ));
            }
            if let Some(surface) = ready.surface.as_ref() {
                surface.publish_into(&mut commit.workspace)?;
            }
            let buffers = pixel_commit::replay_recon_row(
                ready.row,
                &mut commit.next,
                &mut commit.decoded_any,
                &self.quantizer,
                &mut commit.ordered,
                &mut commit.workspace,
                &mut commit.block_decoded,
                &mut commit.current_block_decoded_superblock,
                &self.motion,
                &mut commit.records,
                &self.temporal,
                &self.reference,
                &self.ref_frame_idx,
                &self.sequence,
                &self.core,
                self.params.mi_rows,
                self.params.mi_cols,
                self.params.current_order_hint,
                self.params.luma_use_tcq,
                self.params.residual_use_ddt,
                self.params.bit_depth,
                self.tile_offset,
            )?;
            self.row_buffers.recycle(buffers);
        }
        if commit.next != self.unit_count {
            return Ok(None);
        }
        if !commit.decoded_any {
            self.finished.store(true, Ordering::Release);
            return Err(inter_missing!(
                "inter_admission_commit_no_decoded_block",
                self.tile_offset,
                "inter.block",
                SPEC_MODE_INFO
            ));
        }
        self.finished.store(true, Ordering::Release);
        let commit = holder.take().ok_or_else(|| {
            inter_cap!(
                "inter_admission_commit_state",
                self.tile_offset,
                "inter.row.task_capacity",
                SPEC_MODE_INFO
            )
        })?;
        Ok(Some(ScheduledTileOutput {
            workspace: commit.workspace,
            records: commit.records,
        }))
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_for_admission<'surface, T: ReconSample>(
    parsed: &mut ParsedTile,
    surfaces: Vec<ReadyReconSurface<'surface, T>>,
    context: &TileDecodeContext<'_, T>,
    temporal: &TemporalMvContext,
    row_gate: &RowReferenceGate<'_, T>,
) -> Result<Vec<Option<ReadyReconRow<'surface, T>>>> {
    let started = crate::timing::start();
    let mut grid = NeighbourMvGrid::new_for_tile(parsed.mi_rows.clone(), parsed.mi_cols.clone())
        .ok_or_else(|| {
            inter_cap!(
                "inter_admission_mv_grid",
                parsed.tile_offset,
                "inter.mv_grid",
                SPEC_MODE_INFO
            )
        })?;
    let mut state = TileResolveState::new(context.sequence);
    let mut surfaces = surfaces.into_iter();
    let mut resolved = Vec::new();
    for mut row in core::mem::take(&mut parsed.rows) {
        grid.replay_flag_log(&row.flag_log);
        if let Err(error) =
            state.resolve_unit(&mut grid, context, temporal, &mut row, parsed.tile_offset)
        {
            row.terminal = Some(error);
        }
        let stop = row.terminal.is_some();
        let surface = if row.superblocks.is_empty() {
            None
        } else {
            surfaces.next()
        };
        let bounds = row_gate.bounds_for_row(&row);
        resolved.push(Some(ReadyReconRow {
            row,
            surface,
            bounds,
        }));
        if stop {
            break;
        }
    }
    crate::timing::report_detail(
        "pass2_resolve",
        started,
        &format!(
            "units={} threads={} workers_used=0",
            resolved.len(),
            splot_parallel::current_pool_width(),
        ),
    );
    Ok(resolved)
}

/// Resolves a parsed tile and turns each unit into owned admission state.
#[allow(clippy::large_types_passed_by_value, clippy::too_many_arguments)]
pub(in crate::prediction::inter::block) fn prepare_scheduled_tile<T: ReconSample>(
    mut scratch: TileDecodeScratch<T>,
    records: crate::filters::wienerns_lr::FrameFilterRecords,
    mut parsed: ParsedTile,
    params: TileWalkParams,
    sequence: Arc<SequenceHeader>,
    core: Arc<FrameHeaderCore>,
    temporal: Arc<TemporalMvContext>,
    reference: Arc<InterReferenceState<T>>,
    ref_frame_idx: Arc<[u32]>,
    workspace: CurrentFrameWorkspace<T>,
    motion_field: TemporalMotionField,
    motion_handle: MotionFieldHandle,
) -> Result<ScheduledTileRecon<T>> {
    scratch.workers.ensure_workers(
        splot_parallel::current_pool_width()
            .saturating_sub(1)
            .max(1),
    );
    let info = workspace.info();
    let context = params.context(&sequence, &core, &reference, &ref_frame_idx);
    let tile_offset = parsed.tile_offset;
    let row_gate = RowReferenceGate::new(
        &reference,
        &core,
        &ref_frame_idx,
        workspace.info(),
        &temporal,
    );
    let quantizer = FrameQuantizerSnapshot::capture();
    let rects = superblock_luma_rects(
        &parsed.mi_rows,
        &parsed.mi_cols,
        &workspace,
        params.sb_h4,
        tile_offset,
    )?;
    let motion = MotionFieldUnits::publishing(
        motion_field,
        parsed.rows.len(),
        motion_handle,
        crate::timing::start(),
    );
    let surfaces = rects
        .into_iter()
        .map(|rect| {
            splot_recon::OwnedFrameRect::new(workspace.info(), rect, T::default())
                .map(ReadyReconSurface::Owned)
        })
        .collect::<splot_recon::Result<Vec<_>>>()?;
    let rows = resolve_for_admission(&mut parsed, surfaces, &context, &temporal, &row_gate)?;
    if rows.is_empty() {
        return Err(inter_missing!(
            "inter_admission_no_resolved_rows",
            tile_offset,
            "inter.block",
            SPEC_MODE_INFO
        ));
    }
    let prepass_block_decoded = parsed.block_decoded.clone();
    let block_decoded = parsed.block_decoded.clone();
    let unit_count = rows.len();
    let batch_size = 4;
    let batch_count = unit_count.div_ceil(batch_size);
    let mut prepared = Vec::new();
    prepared.try_reserve_exact(unit_count).map_err(|_| {
        inter_cap!(
            "inter_admission_prepared_allocation",
            tile_offset,
            "inter.row.task_capacity",
            SPEC_MODE_INFO
        )
    })?;
    prepared.resize_with(unit_count, || None);
    let TileDecodeScratch { ordered, workers } = scratch;
    Ok(ScheduledTileRecon {
        rows: Mutex::new(rows),
        prepared: Mutex::new(prepared),
        unit_count,
        batch_size,
        batch_count,
        commit: Mutex::new(Some(ScheduledCommit {
            next: 0,
            ordered,
            workspace,
            block_decoded,
            current_block_decoded_superblock: None,
            records,
            decoded_any: false,
        })),
        workers,
        prepass_block_decoded,
        row_buffers: ReconRowBufferPool::new(0),
        motion,
        info,
        params,
        quantizer,
        temporal,
        reference,
        ref_frame_idx,
        sequence,
        core,
        tile_offset,
        finished: AtomicBool::new(false),
    })
}
