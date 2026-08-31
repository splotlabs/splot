// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Admission-scheduled reconstruction state for one parsed tile.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`,
//! `INFRA-DECODE-FRAME-PIPELINING`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use splot_core::headers::frame::RefIdxBuf;
use splot_parallel::{AdmissionScheduler, CompletionCell, Condition};

use super::*;
use crate::prediction::inter::block::row_gate::RowReferenceGate;
use crate::prediction::inter::find_mv_stack::TemporalBandPlan;

pub(super) struct TileCommit<T: ReconSample> {
    next: usize,
    handed_rows: usize,
    ordered: deferred_recon::InterReconScratch<T>,
    workspace: CurrentFrameWorkspace<T>,
    block_decoded: TileBlockDecodedState,
    current_block_decoded_superblock: Option<[usize; 2]>,
    decoded_any: bool,
    surfaces: Vec<splot_recon::OwnedFrameRect<T>>,
    frame_filter_records: crate::filters::wienerns_lr::FrameFilterRecords,
}

impl<T: ReconSample> TileCommit<T> {
    pub(super) fn direct(
        ordered: deferred_recon::InterReconScratch<T>,
        workspace: CurrentFrameWorkspace<T>,
        block_decoded: TileBlockDecodedState,
        decoded_any: bool,
        surfaces: Vec<splot_recon::OwnedFrameRect<T>>,
        frame_filter_records: crate::filters::wienerns_lr::FrameFilterRecords,
    ) -> Self {
        Self {
            next: 0,
            handed_rows: 0,
            ordered,
            workspace,
            block_decoded,
            current_block_decoded_superblock: None,
            decoded_any,
            surfaces,
            frame_filter_records,
        }
    }

    pub(super) fn replay(
        &mut self,
        mut ready: ReadyReconRow<T>,
        quantizer: &FrameQuantizerSnapshot,
        motion: &MotionFieldUnits,
        temporal: &TemporalMvContext,
        context: &TileDecodeContext<'_, T>,
    ) -> Result<ReconRowBuffers> {
        ready.row.return_terminal_error()?;
        if ready.row.ordinal != self.next {
            return Err(invalid_inter_tile_scheduling_state());
        }
        if let Some(surface) = ready.surface.take() {
            surface.publish_into(&mut self.workspace)?;
            self.surfaces.push(surface);
        }
        self.ordered.with_installed(|scratch| {
            pixel_commit::replay_recon_row(
                ready.row,
                &mut self.next,
                &mut self.decoded_any,
                quantizer,
                scratch,
                &mut self.workspace,
                &mut self.block_decoded,
                &mut self.current_block_decoded_superblock,
                motion,
                &mut self.frame_filter_records,
                temporal,
                context.reference,
                context.ref_frame_idx,
                context.sequence,
                context.core,
                context.mi_rows,
                context.mi_cols,
                context.current_order_hint,
                context.luma_use_tcq,
                context.residual_use_ddt,
                context.bit_depth,
            )
        })
    }

    pub(super) fn finish_direct(
        self,
    ) -> (
        deferred_recon::InterReconScratch<T>,
        CurrentFrameWorkspace<T>,
        bool,
        Vec<splot_recon::OwnedFrameRect<T>>,
        crate::filters::wienerns_lr::FrameFilterRecords,
    ) {
        (
            self.ordered,
            self.workspace,
            self.decoded_any,
            self.surfaces,
            self.frame_filter_records,
        )
    }
}

struct CommittedBatch<T: ReconSample> {
    state: TileCommit<T>,
    frontier_rows: core::ops::Range<usize>,
    terminal: bool,
}

/// The § 7.17 frontier's own storage, advanced by one ordered chain per frame.
///
/// The chain runs beside the commit spine, so it owns the sealed copy the spine
/// hands it one superblock row at a time. A frame with no active deblock plan
/// has nothing for the chain to advance, so sealing would only add a copy; it
/// receives the spine's whole workspace once reconstruction is complete.
struct ScheduledFrontier<T: ReconSample> {
    sealed: Option<crate::filters::source::DeblockedSource<T>>,
    sealed_rows: usize,
    terminal_workspace: Option<crate::filters::source::DeblockedSource<T>>,
    deblock: Option<crate::filters::deblock::FrameDeblock<'static>>,
    filter: Option<Arc<crate::filters::wienerns_lr::recon::OwnedFilterSetup<'static, 'static, T>>>,
    next_filter_stripe: usize,
}

struct ScheduledResolve {
    next: usize,
    submitted_batches: usize,
    grid: NeighbourMvGrid,
    state: TileResolveState,
}

fn take_active_commit<T: ReconSample>(
    holder: &Mutex<Option<TileCommit<T>>>,
) -> Result<TileCommit<T>> {
    holder
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .ok_or_else(invalid_inter_tile_scheduling_state)
}

fn restore_active_commit<T: ReconSample>(
    holder: &Mutex<Option<TileCommit<T>>>,
    state: TileCommit<T>,
) -> Result<()> {
    let mut holder = holder.lock().unwrap_or_else(PoisonError::into_inner);
    if holder.is_some() {
        return Err(invalid_inter_tile_scheduling_state());
    }
    *holder = Some(state);
    Ok(())
}

fn project_temporal_band(
    plan: &TemporalBandPlan,
    temporal: &TemporalMvContext,
    reference_fields: &[Option<MotionFieldHandle>],
    index: usize,
) -> Result<()> {
    for (slot, band) in plan.requirements(index) {
        let publication = reference_fields
            .get(slot)
            .and_then(Option::as_ref)
            .and_then(|field| field.band_publication(band))
            .ok_or(DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        if publication.is_none() {
            return Err(
                DecodeReferenceStateError::MissingMotionFieldBandPublication { slot, band }.into(),
            );
        }
    }
    plan.project(temporal, index, |slot, band| {
        reference_fields.get(slot)?.as_ref()?.band(band).cloned()
    })
}

#[allow(clippy::large_enum_variant)]
enum TileReconRow<T: ReconSample> {
    Unresolved {
        row: ReconRow,
        surface: Option<splot_recon::OwnedFrameRect<T>>,
    },
    Ready(ReadyReconRow<T>),
    Taken,
}

/// Filter jobs made ready by one ordered frontier link.
pub(crate) struct ScheduledFrameProgress<T: ReconSample> {
    pub(crate) filters: Vec<crate::filters::wienerns_lr::recon::OwnedFilterJob<T>>,
    pub(crate) output: Option<crate::filters::wienerns_lr::recon::OwnedFilterFinish<T>>,
}

/// Canonical rows one ordered commit handed to the frontier chain.
pub(crate) struct ScheduledCommitProgress {
    /// Frontier links whose rows are now sealed, in chain order.
    pub(crate) frontier_rows: core::ops::Range<usize>,
    /// Whether this commit completed the frame's reconstruction.
    pub(crate) recon_complete: bool,
}

/// Resolved rows and the concrete reconstruction work they feed.
struct TileRecon<T: ReconSample> {
    rows: Mutex<Vec<TileReconRow<T>>>,
    prepared: Mutex<Vec<Option<Vec<ReadyReconRow<T>>>>>,
    unit_count: usize,
    units_per_row: usize,
    batches: Vec<core::ops::Range<usize>>,
    frontier_rows: usize,
    commit: Mutex<Option<TileCommit<T>>>,
    scratch: Mutex<Option<TileDecodeScratch<T>>>,
    workers: InterReconScratchPool<T>,
    prepass_block_decoded: TileBlockDecodedState,
    motion: MotionFieldUnits,
    params: TileWalkParams,
    quantizer: FrameQuantizerSnapshot,
    temporal: Arc<TemporalMvContext>,
    reference: Arc<InterReferenceState<T>>,
    ref_frame_idx: RefIdxBuf,
    sequence: Arc<SequenceHeader>,
    core: Arc<FrameHeaderCore>,
}

/// Owned state shared by one admission job per parsed reconstruction unit.
pub(in crate::prediction::inter) struct ScheduledTileRecon<T: ReconSample> {
    recon: TileRecon<T>,
    filter_count: usize,
    frontier: Mutex<ScheduledFrontier<T>>,
    resolve: Mutex<ScheduledResolve>,
    info: splot_recon::DecodedFrameInfo,
    temporal_plan: TemporalBandPlan,
    tile_offset: ByteOffset,
    whole_frame: bool,
    parse_progress: Arc<super::ParseProgress>,
    pending_surfaces: Mutex<std::vec::IntoIter<splot_recon::OwnedFrameRect<T>>>,
    finished: AtomicBool,
}

impl<T: ReconSample> TileRecon<T> {
    const fn len(&self) -> usize {
        self.batches.len()
    }

    fn batch_range(&self, index: usize) -> Option<core::ops::Range<usize>> {
        self.batches.get(index).cloned()
    }

    fn accept_resolved(
        rows: &mut [TileReconRow<T>],
        index: usize,
        ready: ReadyReconRow<T>,
    ) -> Result<()> {
        let slot = rows
            .get_mut(index)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        if !matches!(slot, TileReconRow::Taken) {
            return Err(invalid_inter_tile_scheduling_state());
        }
        *slot = TileReconRow::Ready(ready);
        Ok(())
    }

    fn conditions(&self, index: usize, info: splot_recon::DecodedFrameInfo) -> Vec<Condition<'_>> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut bounds = row_gate::RowReferenceBounds::default();
        for ready in self
            .batch_range(index)
            .and_then(|range| rows.get(range))
            .into_iter()
            .flatten()
            .filter_map(|row| match row {
                TileReconRow::Ready(ready) => Some(ready),
                TileReconRow::Unresolved { .. } | TileReconRow::Taken => None,
            })
        {
            bounds.merge(ready.bounds);
        }
        drop(rows);
        RowReferenceGate::new(
            &self.reference,
            &self.core,
            self.ref_frame_idx.as_slice(),
            info,
            &self.temporal,
        )
        .conditions(&bounds)
    }

    fn precompute(&self, index: usize) -> Result<()> {
        let range = self
            .batch_range(index)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        {
            let prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(slot) = prepared.get(index) else {
                return Err(invalid_inter_tile_scheduling_state());
            };
            if slot.is_some() {
                return Err(invalid_inter_tile_scheduling_state());
            }
        }
        let ready = {
            let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(rows) = rows.get_mut(range) else {
                return Err(invalid_inter_tile_scheduling_state());
            };
            if rows
                .iter()
                .any(|row| !matches!(row, TileReconRow::Ready(_)))
            {
                return Err(invalid_inter_tile_scheduling_state());
            }
            let mut ready = Vec::with_capacity(rows.len());
            for row in rows {
                if let TileReconRow::Ready(row) = core::mem::replace(row, TileReconRow::Taken) {
                    ready.push(row);
                }
            }
            ready
        };
        let batch = self
            .workers
            .with_scratch(|scratch| -> Result<Vec<ReadyReconRow<T>>> {
                let _quantizer_scopes = self.quantizer.install_frame();
                let shared = deferred_recon::ReconShared {
                    reference: &self.reference,
                    ref_frame_idx: self.ref_frame_idx.as_slice(),
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
                Ok(ready
                    .into_iter()
                    .map(|mut ready| {
                        scratch.with_installed(|scratch| {
                            if !ready.row.has_terminal_error() && !ready.row.motion_derived {
                                mvres::derive_unit_motion(
                                    &mut ready.row,
                                    ready.surface.as_mut(),
                                    scratch,
                                    &self.motion,
                                    &shared,
                                );
                            }
                            precompute_recon_row(
                                ready,
                                scratch,
                                &self.prepass_block_decoded,
                                &self.motion,
                                &self.quantizer,
                                &self.temporal,
                                &self.reference,
                                self.ref_frame_idx.as_slice(),
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
                    })
                    .collect())
            })?;
        let mut prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(slot) = prepared.get_mut(index) else {
            return Err(invalid_inter_tile_scheduling_state());
        };
        if slot.is_some() {
            return Err(invalid_inter_tile_scheduling_state());
        }
        *slot = Some(batch);
        Ok(())
    }

    fn take_scratch(&self) -> Result<TileDecodeScratch<T>> {
        self.scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(invalid_inter_tile_scheduling_state)
    }

    fn commit_batch(&self, index: usize) -> Result<CommittedBatch<T>> {
        let mut commit = take_active_commit(&self.commit)?;
        let batch = {
            let mut prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
            prepared
                .get_mut(index)
                .ok_or_else(invalid_inter_tile_scheduling_state)?
                .take()
                .ok_or_else(invalid_inter_tile_scheduling_state)?
        };
        let context = self.params.context(
            &self.sequence,
            &self.core,
            &self.reference,
            self.ref_frame_idx.as_slice(),
        );
        for ready in batch {
            let buffers = commit.replay(
                ready,
                &self.quantizer,
                &self.motion,
                &self.temporal,
                &context,
            )?;
            recycle_retained_recon_row_buffers(buffers);
        }
        let terminal = commit.next == self.unit_count;
        let closed_rows = closed_frontier_rows(
            commit.next,
            self.unit_count,
            self.units_per_row,
            self.frontier_rows,
        );
        let frontier_rows = commit.handed_rows..closed_rows;
        commit.handed_rows = closed_rows;
        if terminal && !commit.decoded_any {
            return Err(no_decoded_block_error());
        }
        Ok(CommittedBatch {
            state: commit,
            frontier_rows,
            terminal,
        })
    }

    fn restore_commit(&self, commit: TileCommit<T>) -> Result<()> {
        restore_active_commit(&self.commit, commit)
    }

    fn finish_commit(&self, mut commit: TileCommit<T>) -> CurrentFrameWorkspace<T> {
        commit.surfaces.reverse();
        let scratch =
            TileDecodeScratch::from_scheduled(commit.ordered, &self.workers, commit.surfaces);
        *self.scratch.lock().unwrap_or_else(PoisonError::into_inner) = Some(scratch);
        commit.workspace
    }
}

fn record_first_error(error: &Mutex<Option<crate::DecodeError>>, value: crate::DecodeError) {
    let mut error = error.lock().unwrap_or_else(PoisonError::into_inner);
    if error.is_none() {
        *error = Some(value);
    }
}

/// Runs the ordinary single-tile walk through the same concrete precompute and
/// ordered-commit operations as the scheduled frame path.
///
/// The driver owns entropy parsing and reference settlement. Pool jobs never
/// wait: each precompute carries its row-reference conditions and each commit
/// carries the preceding commit's completion.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_ordinary_tile<T: ReconSample>(
    parser: &mut TileParser<'_, '_>,
    resolve: &mut TileResolveState,
    tile_offset: ByteOffset,
    mut surfaces: std::vec::IntoIter<splot_recon::OwnedFrameRect<T>>,
    unit_count: usize,
    context: &TileDecodeContext<'_, T>,
    temporal: &TemporalMvContext,
    quantizer: &FrameQuantizerSnapshot,
    row_gate: &RowReferenceGate<'_, T>,
    row_buffers: &ReconRowBufferPool,
    workers: &InterReconScratchPool<T>,
    prepass_block_decoded: &TileBlockDecodedState,
    motion: &MotionFieldUnits,
    commit: TileCommit<T>,
) -> Result<TileCommit<T>> {
    let mut prepared = Vec::new();
    prepared
        .try_reserve_exact(unit_count)
        .map_err(|_| inter_allocation!("ordinary inter prepared rows"))?;
    prepared.resize_with(unit_count, || Mutex::new(None));
    let mut precomputed = Vec::new();
    precomputed
        .try_reserve_exact(unit_count)
        .map_err(|_| inter_allocation!("ordinary inter precompute completions"))?;
    precomputed.resize_with(unit_count, CompletionCell::new);
    let mut committed = Vec::new();
    committed
        .try_reserve_exact(unit_count)
        .map_err(|_| inter_allocation!("ordinary inter commit completions"))?;
    committed.resize_with(unit_count, CompletionCell::new);

    let commit = Mutex::new(Some(commit));
    let error = Mutex::new(None);
    let scheduler = AdmissionScheduler::new();
    let admission_window = splot_parallel::current_pool_width()
        .saturating_sub(1)
        .max(1)
        .saturating_mul(3);
    let mut references_settled = false;
    let mut submitted = 0usize;
    let mut reached_last = false;
    let parse_result = splot_parallel::ready_task_scope(|scope| {
        for index in 0..unit_count {
            let step = {
                let _quantizer_scopes = quantizer.install_frame();
                let step = parser.next_unit(
                    context,
                    ParserGranularity::Superblock,
                    Some(row_buffers.take()),
                );
                resolve_parser_step(step, |row| {
                    resolve.resolve_unit(&mut parser.mv_grid, context, temporal, row, tile_offset)
                })
            };
            let (row, last) = match step {
                ParserStep::More(row) => (row, false),
                ParserStep::Last(row) => (row, true),
            };
            let ready = ReadyReconRow {
                bounds: row_gate.bounds_for_row(&row),
                row,
                surface: surfaces.next(),
            };
            let conditions = row_gate.conditions(&ready.bounds);
            let prepared_slot = &prepared[index];
            let precomputed_cell = &precomputed[index];
            let precompute_error = &error;
            scheduler.submit(
                scope,
                (index as u64).saturating_mul(4).saturating_add(1),
                &conditions,
                Box::new(move |_| {
                    let enabled = precompute_error
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .is_none();
                    let ready = enabled.then(|| {
                        workers.with_scratch(|scratch| {
                            precompute_recon_row(
                                ready,
                                scratch,
                                prepass_block_decoded,
                                motion,
                                quantizer,
                                temporal,
                                context.reference,
                                context.ref_frame_idx,
                                context.sequence,
                                context.core,
                                context.sb_h4,
                                context.mi_rows,
                                context.mi_cols,
                                context.current_order_hint,
                                context.luma_use_tcq,
                                context.residual_use_ddt,
                                context.bit_depth,
                            )
                        })
                    });
                    *prepared_slot.lock().unwrap_or_else(PoisonError::into_inner) = ready;
                    let _ = precomputed_cell.set(());
                }),
            );

            let prepared_slot = &prepared[index];
            let completed = &committed[index];
            let commit = &commit;
            let commit_error = &error;
            let mut conditions = vec![Condition::completion(&precomputed[index])];
            if let Some(previous) = index.checked_sub(1) {
                conditions.push(Condition::completion(&committed[previous]));
            }
            scheduler.submit(
                scope,
                (index as u64).saturating_mul(4).saturating_add(2),
                &conditions,
                Box::new(move |_| {
                    let ready = prepared_slot
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take();
                    if commit_error
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .is_none()
                    {
                        let result = ready
                            .ok_or_else(invalid_inter_tile_scheduling_state)
                            .and_then(|ready| {
                                let mut state = take_active_commit(commit)?;
                                let result =
                                    state.replay(ready, quantizer, motion, temporal, context);
                                restore_active_commit(commit, state)?;
                                result
                            });
                        match result {
                            Ok(buffers) => row_buffers.recycle(buffers),
                            Err(value) => record_first_error(commit_error, value),
                        }
                    }
                    let _ = completed.set(());
                }),
            );
            submitted = index.saturating_add(1);
            if last {
                reached_last = true;
            }
            if let Some(target) = index.checked_sub(admission_window) {
                while !committed[target].is_set() {
                    let progress = splot_parallel::pool_progress_snapshot();
                    if !references_settled && row_gate.is_ready() {
                        references_settled = true;
                        if let Err(value) = row_gate.wait() {
                            record_first_error(&error, value);
                        }
                    }
                    scheduler.admit_ready(scope);
                    if committed[target].is_set() {
                        break;
                    }
                    if !references_settled {
                        if row_gate.is_ready() {
                            continue;
                        }
                        break;
                    }
                    splot_parallel::assist_pool_or_park(&progress);
                }
            }
            if last {
                break;
            }
        }
    });
    parse_result?;
    if !reached_last {
        record_first_error(&error, invalid_inter_tile_scheduling_state());
    }

    let terminal = submitted
        .checked_sub(1)
        .and_then(|last| committed.get(last));
    while terminal.is_some_and(|done| !done.is_set()) {
        let progress = splot_parallel::pool_progress_snapshot();
        if !references_settled && row_gate.is_ready() {
            references_settled = true;
            if let Err(value) = row_gate.wait() {
                record_first_error(&error, value);
            }
        }
        splot_parallel::ready_task_scope(|scope| {
            scheduler.admit_ready(scope);
        })?;
        if terminal.is_none_or(CompletionCell::is_set) {
            break;
        }
        if !references_settled && row_gate.is_ready() {
            continue;
        }
        if references_settled {
            break;
        }
        splot_parallel::assist_pool_or_park(&progress);
    }
    let scheduler_result = scheduler.finish();
    if let Some(error) = error.lock().unwrap_or_else(PoisonError::into_inner).take() {
        return Err(error);
    }
    scheduler_result?;
    take_active_commit(&commit)
}

impl<T: ReconSample> ScheduledTileRecon<T> {
    /// Number of independently admitted reconstruction units.
    pub(in crate::prediction::inter) const fn len(&self) -> usize {
        self.recon.len()
    }

    /// Number of final-filter stripes this frame owes.
    pub(in crate::prediction::inter) const fn filter_count(&self) -> usize {
        self.filter_count
    }

    pub(in crate::prediction::inter) fn resolve_len(&self) -> usize {
        self.temporal_plan.len()
    }

    pub(in crate::prediction::inter) fn resolve_conditions(
        &self,
        index: usize,
    ) -> Vec<Condition<'_>> {
        self.temporal_plan
            .requirements(index)
            .into_iter()
            .filter_map(|(slot, band)| {
                self.recon
                    .reference
                    .ref_motion_fields
                    .get(slot)
                    .and_then(Option::as_ref)
                    .and_then(|field| field.band_condition(band))
            })
            .collect()
    }

    /// Whether the § 8.2 pass failed.
    ///
    /// A failed pass drives the watermark past every threshold, so a stalled
    /// step that waited on it again would spin until the failure settles.
    fn parse_failed(&self) -> bool {
        self.parse_progress.cell().current() == splot_parallel::WatermarkCell::FAILED
    }

    /// The § 8.2 watermark a stalled resolve step waits on.
    pub(in crate::prediction::inter) fn parse_watermark(&self) -> &splot_parallel::WatermarkCell {
        self.parse_progress.cell()
    }

    pub(in crate::prediction::inter) fn resolve(
        &self,
        index: usize,
    ) -> Result<(core::ops::Range<usize>, Option<usize>)> {
        project_temporal_band(
            &self.temporal_plan,
            &self.recon.temporal,
            &self.recon.reference.ref_motion_fields,
            index,
        )?;
        let projected_rows = self.temporal_plan.rows8(index);
        let final_band = index.saturating_add(1) == self.temporal_plan.len();
        let context = self.recon.params.context(
            &self.recon.sequence,
            &self.recon.core,
            &self.recon.reference,
            self.recon.ref_frame_idx.as_slice(),
        );
        let row_gate = RowReferenceGate::new(
            &self.recon.reference,
            &self.recon.core,
            self.recon.ref_frame_idx.as_slice(),
            self.info,
            &self.recon.temporal,
        );
        self.materialize_rows(self.recon.unit_count);
        let mut awaiting = None;
        let mut resolve = self.resolve.lock().unwrap_or_else(PoisonError::into_inner);
        let mut rows = self
            .recon
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            let next = resolve.next;
            let Some(state) = rows.get(next) else {
                if next < self.recon.unit_count && !self.parse_failed() {
                    awaiting = Some(next.saturating_add(1));
                }
                break;
            };
            let TileReconRow::Unresolved { row, .. } = state else {
                break;
            };
            let row_end8 = row
                .superblocks
                .iter()
                .filter_map(|superblock| superblock.origin[0].checked_add(self.recon.params.sb_h4))
                .map(|end4| end4.div_ceil(2))
                .max()
                .unwrap_or(projected_rows.end);
            if !final_band && row_end8 > projected_rows.end {
                break;
            }
            let Some(slot) = rows.get_mut(next) else {
                break;
            };
            let TileReconRow::Unresolved { mut row, surface } =
                core::mem::replace(slot, TileReconRow::Taken)
            else {
                break;
            };
            resolve.grid.replay_flag_log(&row.flag_log);
            row.return_terminal_error()?;
            {
                let ScheduledResolve { grid, state, .. } = &mut *resolve;
                state.resolve_unit(
                    grid,
                    &context,
                    &self.recon.temporal,
                    &mut row,
                    self.tile_offset,
                )
            }?;
            row.return_terminal_error()?;
            let bounds = row_gate.bounds_for_row(&row);
            TileRecon::accept_resolved(
                &mut rows,
                next,
                ReadyReconRow {
                    row,
                    surface,
                    bounds,
                },
            )?;
            resolve.next = resolve.next.saturating_add(1);
        }
        let ready_batches = self
            .recon
            .batches
            .partition_point(|batch| batch.end <= resolve.next);
        let ready = resolve.submitted_batches..ready_batches;
        resolve.submitted_batches = ready_batches;
        Ok((ready, awaiting))
    }

    pub(in crate::prediction::inter) fn fail_temporal(&self) {
        TemporalBandPlan::fail(&self.recon.temporal);
    }

    /// Hands the frontier its § 7.17 deblock records and final-filter setup.
    ///
    /// Reconstruction never reads either, so they arrive once the § 8.2 pass
    /// has settled the grids they are built from, after the scheduler is
    /// already admitting precompute work.
    pub(in crate::prediction::inter::block) fn attach_filters(
        &self,
        filter_setup: crate::filters::wienerns_lr::recon::OwnedFilterSetup<'static, 'static, T>,
        deblock_records: Option<crate::filters::deblock::OwnedDeblockRecords>,
        deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
    ) -> Result<()> {
        let deblock = deblock_records
            .zip(self.recon.core.deblocking_filter_params)
            .map(|(deblock_records, filter)| {
                crate::filters::deblock::FrameDeblock::prepare_owned(
                    deblock_records,
                    self.recon.params.mi_rows,
                    self.recon.params.mi_cols,
                    filter,
                    Arc::clone(&self.recon.core),
                    self.recon
                        .sequence
                        .filter
                        .is_some_and(|filter| filter.disable_loopfilters_across_tiles),
                    deblock_quant_deltas,
                )
                .map_err(|error| crate::filters::wienerns_lr::recon::deblock_prepare_error(&error))
            })
            .transpose()?
            .flatten();
        let mut frontier = self.frontier.lock().unwrap_or_else(PoisonError::into_inner);
        frontier.sealed = seals_filter_copy(deblock.is_some(), self.whole_frame)
            .then(|| {
                CurrentFrameWorkspace::new_recycled(self.info)
                    .map(crate::filters::source::DeblockedSource::new)
            })
            .transpose()?;
        frontier.deblock = deblock;
        frontier.filter = Some(Arc::new(filter_setup));
        Ok(())
    }

    /// Pulls published units into the scheduled row list, up to `units`.
    ///
    /// The § 8.2 pass emits units in order, so a caller that has waited on
    /// the parse watermark for `units` finds exactly that prefix available,
    /// and a unit the pass has not reached simply stops the pull.
    fn materialize_rows(&self, units: usize) {
        let units = units.min(self.recon.unit_count);
        let mut rows = self
            .recon
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if rows.len() >= units {
            return;
        }
        let mut surfaces = self
            .pending_surfaces
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while rows.len() < units {
            let Some(row) = self.parse_progress.take_row(rows.len()) else {
                break;
            };
            let surface = if row.superblocks.is_empty() {
                None
            } else {
                surfaces.next()
            };
            rows.push(TileReconRow::Unresolved { row, surface });
        }
    }

    pub(in crate::prediction::inter) fn conditions(&self, index: usize) -> Vec<Condition<'_>> {
        let mut conditions = match self.recon.batch_range(index) {
            Some(range) => vec![Condition::watermark(self.parse_progress.cell(), range.end)],
            None => Vec::new(),
        };
        if let Some(range) = self.recon.batch_range(index) {
            self.materialize_rows(range.end);
        }
        conditions.extend(self.recon.conditions(index, self.info));
        conditions
    }

    /// Precomputes one admitted unit without entering the ordered commit spine.
    pub(in crate::prediction::inter) fn precompute(&self, index: usize) -> Result<()> {
        if self.finished.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(error) = self.recon.precompute(index) {
            self.finished.store(true, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    fn seal_committed_rows(&self, commit: &TileCommit<T>, rows: usize) -> Result<()> {
        let mut frontier = self.frontier.lock().unwrap_or_else(PoisonError::into_inner);
        let ScheduledFrontier {
            sealed,
            sealed_rows,
            ..
        } = &mut *frontier;
        let Some(sealed) = sealed.as_mut() else {
            return Ok(());
        };
        let end = rows
            .saturating_mul(self.recon.params.sb_h4.saturating_mul(4).max(1))
            .min(self.info.coded_luma_size().height());
        if end > *sealed_rows {
            sealed.copy_rows_from(&commit.workspace, *sealed_rows..end)?;
            *sealed_rows = end;
        }
        Ok(())
    }

    /// Number of ordered links in this frame's § 7.17 frontier chain.
    pub(in crate::prediction::inter) const fn frontier_len(&self) -> usize {
        self.recon.frontier_rows
    }

    /// Advances the § 7.17 deblock frontier over the rows sealed by superblock
    /// row `row` and takes the filter stripes that frontier releases.
    ///
    /// The one caller that runs the final link receives the completed tile.
    pub(in crate::prediction::inter) fn frontier(
        &self,
        row: usize,
    ) -> Result<ScheduledFrameProgress<T>> {
        let terminal = row.saturating_add(1) >= self.recon.frontier_rows;
        let committed_units = row
            .saturating_add(1)
            .saturating_mul(self.recon.units_per_row)
            .min(self.recon.unit_count);
        let mut frontier = self.frontier.lock().unwrap_or_else(PoisonError::into_inner);
        let ScheduledFrontier {
            sealed,
            sealed_rows,
            terminal_workspace,
            deblock,
            filter,
            next_filter_stripe,
            ..
        } = &mut *frontier;
        let sealed_rows = sealed.as_ref().map(|_| *sealed_rows);
        let filtered = terminal_workspace.as_mut();
        let mut filters = Vec::new();
        if let Some(deblock) = deblock.as_mut()
            && let Some(safe_mi_end) = safe_deblock_mi_end(
                committed_units,
                self.recon.units_per_row,
                self.recon.params.sb_h4,
                self.recon.params.mi_rows,
                terminal,
            )
        {
            let filter = filter
                .as_ref()
                .ok_or_else(crate::filters::wienerns_lr::recon::lr_pipeline_state_error)?;
            debug_assert!(
                sealed_rows.is_none_or(|rows| {
                    deblock
                        .data_reach_luma_rows(safe_mi_end.get())
                        .min(self.info.coded_luma_size().height())
                        <= rows
                }),
                "the frontier read a row the spine had not sealed"
            );
            let source = sealed.as_mut().or(filtered).ok_or_else(|| {
                crate::filters::wienerns_lr::recon::deblock_prepare_error(
                    &crate::filters::deblock::DeblockError::Workspace,
                )
            })?;
            deblock
                .advance_source(source, safe_mi_end.get(), self.recon.params.bit_depth)
                .map_err(|error| {
                    crate::filters::wienerns_lr::recon::deblock_prepare_error(&error)
                })?;
            while *next_filter_stripe < filter.stripe_ranges().len() {
                let stripe = *next_filter_stripe;
                let Some(source) = sealed.as_ref().or(terminal_workspace.as_ref()) else {
                    break;
                };
                let Some(source) = filter.lease_ready_rows(stripe, deblock, source)? else {
                    break;
                };
                filters
                    .try_reserve(1)
                    .map_err(|_| inter_allocation!("inter admission filter jobs"))?;
                filters.push(filter.source_job(stripe, source));
                *next_filter_stripe += 1;
            }
        }
        if !terminal {
            return Ok(ScheduledFrameProgress {
                filters,
                output: None,
            });
        }
        Self::finish_frontier(&mut frontier, filters)
    }

    /// Completes the frame's filter stripes after the final frontier link.
    fn finish_frontier(
        frontier: &mut ScheduledFrontier<T>,
        mut filters: Vec<crate::filters::wienerns_lr::recon::OwnedFilterJob<T>>,
    ) -> Result<ScheduledFrameProgress<T>> {
        let filter = frontier
            .filter
            .take()
            .ok_or_else(crate::filters::wienerns_lr::recon::lr_pipeline_state_error)?;
        if let Some(deblock) = frontier.deblock.take() {
            let records = deblock
                .finish()
                .ok_or_else(crate::filters::wienerns_lr::recon::lr_pipeline_state_error)?;
            filter.restore_deblock_records(records)?;
        }
        while frontier.next_filter_stripe < filter.stripe_ranges().len() {
            let stripe = frontier.next_filter_stripe;
            let source = frontier
                .sealed
                .as_ref()
                .or(frontier.terminal_workspace.as_ref())
                .ok_or_else(crate::filters::wienerns_lr::recon::lr_pipeline_state_error)?;
            let source = filter.lease_terminal_rows(stripe, source)?;
            filters
                .try_reserve(1)
                .map_err(|_| inter_allocation!("inter admission filter jobs"))?;
            filters.push(filter.source_job(stripe, source));
            frontier.next_filter_stripe += 1;
        }
        drop(frontier.sealed.take());
        drop(frontier.terminal_workspace.take());
        Ok(ScheduledFrameProgress {
            filters,
            output: Some(filter.owned_finish()),
        })
    }

    pub(in crate::prediction::inter) fn take_scheduled_scratch(
        &self,
    ) -> Result<TileDecodeScratch<T>> {
        self.recon.take_scratch()
    }

    /// Commits one precomputed unit after its predecessor has completed.
    ///
    /// The one caller that commits the final unit receives the completed tile.
    pub(in crate::prediction::inter) fn commit(
        &self,
        index: usize,
    ) -> Result<ScheduledCommitProgress> {
        let result = self.commit_active(index);
        if match &result {
            Ok(progress) => progress.recon_complete,
            Err(_) => true,
        } {
            self.finished.store(true, Ordering::Release);
        }
        result
    }

    fn commit_active(&self, index: usize) -> Result<ScheduledCommitProgress> {
        let committed = self.recon.commit_batch(index)?;
        if !committed.frontier_rows.is_empty() {
            self.seal_committed_rows(&committed.state, committed.frontier_rows.end)?;
        }
        if !committed.terminal {
            self.recon.restore_commit(committed.state)?;
            return Ok(ScheduledCommitProgress {
                frontier_rows: committed.frontier_rows,
                recon_complete: false,
            });
        }
        let workspace = self.recon.finish_commit(committed.state);
        {
            let mut frontier = self.frontier.lock().unwrap_or_else(PoisonError::into_inner);
            if frontier.sealed.is_some() {
                workspace.recycle_planes();
            } else {
                let mut source = crate::filters::source::DeblockedSource::new(workspace);
                if frontier.deblock.is_none()
                    && !source.publish_final_rows(self.info.coded_luma_size().height())
                {
                    return Err(invalid_inter_tile_scheduling_state());
                }
                frontier.terminal_workspace = Some(source);
            }
        }
        Ok(ScheduledCommitProgress {
            frontier_rows: committed.frontier_rows,
            recon_complete: true,
        })
    }
}

/// Frontier links whose superblock rows are complete after `committed` units.
///
/// The terminal commit closes every remaining link, so a frame whose last
/// parsed unit carries no superblock still hands its final row over exactly
/// once.
const fn closed_frontier_rows(
    committed: usize,
    unit_count: usize,
    units_per_row: usize,
    frontier_rows: usize,
) -> usize {
    if committed >= unit_count {
        return frontier_rows;
    }
    let closed = match units_per_row {
        0 => 0,
        units_per_row => committed / units_per_row,
    };
    if closed < frontier_rows {
        closed
    } else {
        frontier_rows
    }
}

/// Mode-info rows the § 7.17 frontier keeps below the reconstructed rows.
///
/// Ordinary current-frame prediction reads one row above a superblock row:
/// luma forces reference line 0 across that boundary, chroma has no multiple
/// reference line, and the cross-component references clamp to the
/// superblock's own first row. Reconstructed mode-info rows `F` therefore
/// leave luma row `4F - 1` and chroma row `2F - 1` readable, and no deblock
/// pass may write them. The vertical pass filters four plane rows per
/// mode-info row, which on a vertically subsampled plane spans two mode-info
/// rows, so its frontier must satisfy `2 * end_0 + 1 <= 2F - 2`; it also runs
/// four mode-info rows ahead of the horizontal one. The horizontal pass writes
/// at most four chroma rows past its edge, which is the weaker bound.
const RECON_READ_LEAD_MI_ROWS: usize = 6;

/// Returns the § 7.17 deblock frontier after `completed_rows` canonical rows.
///
/// The frontier deblocks the sealed copy, so its only bound is the sealed rows
/// it may read. Current-frame readers — ordinary intra, and local or global
/// IntraBC — keep reading the spine's raw workspace, which no deblock pass
/// writes, so an IntraBC source's liveness places no constraint here.
fn safe_deblock_mi_end(
    completed_units: usize,
    units_per_row: usize,
    sb_h4: usize,
    mi_rows: usize,
    terminal: bool,
) -> Option<core::num::NonZeroUsize> {
    if terminal {
        return core::num::NonZeroUsize::new(mi_rows);
    }
    let completed_rows = completed_units
        .checked_div(units_per_row)
        .unwrap_or_default();
    core::num::NonZeroUsize::new(
        completed_rows
            .saturating_mul(sb_h4)
            .min(mi_rows)
            .saturating_sub(RECON_READ_LEAD_MI_ROWS),
    )
}

/// Whether the frontier chain filters its own sealed copy of the frame.
///
/// Sealing is what decouples § 7.17 from the commit spine: deblock writes only
/// the copy, so every current-frame reader — ordinary intra, and local or
/// global IntraBC, whose source region is neither row-shaped nor bounded above
/// — still reads raw reconstructed samples however far the frontier has run. A
/// frame with no active deblock plan has nothing to advance, so it does not
/// seal.
const fn seals_filter_copy(has_deblock: bool, whole_frame: bool) -> bool {
    has_deblock && whole_frame
}

/// Whether one parsed tile owns every mode-info position in the frame.
fn covers_whole_frame(geometry: &TileGeometry, params: &TileWalkParams) -> bool {
    geometry.mi_rows.start == 0
        && geometry.mi_rows.end.min(params.mi_rows) == params.mi_rows
        && geometry.mi_cols.start == 0
        && geometry.mi_cols.end.min(params.mi_cols) == params.mi_cols
}

/// Reconstruction units one scheduled precompute batch prepares.
const SCHEDULED_BATCH_UNITS: usize = 4;

/// Splits the units into precompute batches that never cross a superblock row.
///
/// A batch's admission waits for the furthest reference row any of its units
/// reads, and a unit in superblock row `r` reads about `sb_h4 * 4 * (r + 1)`
/// reference rows. A batch that straddles the boundary therefore holds the
/// whole superblock row it completes behind the *next* row's reference bound,
/// which delays the § 7.17 frontier, the filter stripes that frontier releases,
/// and the dependent frame those stripes admit. Aligning the split keeps the
/// batch size and the commit order unchanged; only the grouping moves.
fn superblock_row_batches(
    unit_count: usize,
    units_per_row: usize,
    batch_units: usize,
) -> Vec<core::ops::Range<usize>> {
    let batch_units = batch_units.max(1);
    let units_per_row = units_per_row.max(1);
    let mut batches = Vec::new();
    let mut start = 0;
    while start < unit_count {
        let row_end = (start / units_per_row)
            .saturating_add(1)
            .saturating_mul(units_per_row)
            .min(unit_count);
        let end = start.saturating_add(batch_units).min(row_end);
        batches.push(start..end);
        start = end;
    }
    batches
}

fn prepare_scheduled_motion(
    mi_rows: core::ops::Range<usize>,
    mi_cols: core::ops::Range<usize>,
    motion_field: TemporalMotionField,
    units: usize,
    units_per_row: usize,
    motion_handle: MotionFieldHandle,
) -> Result<(NeighbourMvGrid, MotionFieldUnits)> {
    let grid = NeighbourMvGrid::new_for_tile(mi_rows, mi_cols)
        .map_err(|error| inter_tile_grid_error(&error, "inter admission MV grid"))?;
    let motion = MotionFieldUnits::publishing(motion_field, units, units_per_row, motion_handle);
    Ok((grid, motion))
}

/// Resolves a parsed tile and turns each unit into owned admission state.
#[allow(clippy::large_types_passed_by_value, clippy::too_many_arguments)]
pub(in crate::prediction::inter::block) fn prepare_scheduled_tile<T: ReconSample>(
    mut scratch: TileDecodeScratch<T>,
    params: TileWalkParams,
    sequence: Arc<SequenceHeader>,
    core: Arc<FrameHeaderCore>,
    temporal: Arc<TemporalMvContext>,
    reference: Arc<InterReferenceState<T>>,
    ref_frame_idx: RefIdxBuf,
    workspace: CurrentFrameWorkspace<T>,
    filter_count: usize,
    motion_field: TemporalMotionField,
    motion_handle: MotionFieldHandle,
    temporal_plan: TemporalBandPlan,
    parse_progress: Arc<super::ParseProgress>,
) -> Result<ScheduledTileRecon<T>> {
    scratch.workers.ensure_workers(
        splot_parallel::current_pool_width()
            .saturating_sub(1)
            .max(1),
    );
    let info = workspace.info();
    let geometry = parse_progress
        .geometry()
        .ok_or_else(invalid_inter_tile_scheduling_state)?;
    let tile_offset = geometry.tile_offset;
    let unit_count = geometry.unit_count;
    let quantizer = FrameQuantizerSnapshot::capture();
    let units_per_row = geometry
        .mi_cols
        .end
        .min(params.mi_cols)
        .saturating_sub(geometry.mi_cols.start)
        .div_ceil(params.sb_h4);
    let (resolve_grid, motion) = prepare_scheduled_motion(
        geometry.mi_rows.clone(),
        geometry.mi_cols.clone(),
        motion_field,
        unit_count.saturating_sub(1),
        units_per_row,
        motion_handle,
    )?;
    let rects = superblock_luma_rects(
        &geometry.mi_rows,
        &geometry.mi_cols,
        &workspace,
        params.sb_h4,
    )?;
    scratch.clear_incompatible_surface_layout(info, &rects);
    let surfaces = rects
        .into_iter()
        .map(|rect| scratch.take_surface(info, rect))
        .collect::<splot_recon::Result<Vec<_>>>()?;
    let surface_cursor = surfaces.into_iter();
    if unit_count == 0 {
        return Err(invalid_inter_tile_scheduling_state());
    }
    let prepass_block_decoded = geometry.block_decoded.clone();
    let block_decoded = geometry.block_decoded.clone();
    let batches = superblock_row_batches(unit_count, units_per_row.max(1), SCHEDULED_BATCH_UNITS);
    let batch_count = batches.len();
    let mut prepared = Vec::new();
    prepared
        .try_reserve_exact(batch_count)
        .map_err(|_| inter_allocation!("inter admission prepared batches"))?;
    prepared.resize_with(batch_count, || None);
    let whole_frame = covers_whole_frame(&geometry, &params);
    let TileDecodeScratch {
        ordered,
        workers,
        surfaces,
    } = scratch;
    let resolve_state = TileResolveState::new(&sequence);
    let tile = ScheduledTileRecon {
        recon: TileRecon {
            rows: Mutex::new(Vec::new()),
            prepared: Mutex::new(prepared),
            unit_count,
            units_per_row,
            batches,
            frontier_rows: unit_count.div_ceil(units_per_row.max(1)).max(1),
            commit: Mutex::new(Some(TileCommit {
                next: 0,
                handed_rows: 0,
                ordered,
                workspace,
                block_decoded,
                current_block_decoded_superblock: None,
                decoded_any: false,
                surfaces,
                frame_filter_records: crate::filters::wienerns_lr::FrameFilterRecords::default(),
            })),
            scratch: Mutex::new(None),
            workers,
            prepass_block_decoded,
            motion,
            params,
            quantizer,
            temporal,
            reference,
            ref_frame_idx,
            sequence,
            core,
        },
        filter_count,
        frontier: Mutex::new(ScheduledFrontier {
            sealed: None,
            sealed_rows: 0,
            terminal_workspace: None,
            deblock: None,
            filter: None,
            next_filter_stripe: 0,
        }),
        resolve: Mutex::new(ScheduledResolve {
            next: 0,
            submitted_batches: 0,
            grid: resolve_grid,
            state: resolve_state,
        }),
        info,
        temporal_plan,
        tile_offset,
        whole_frame,
        parse_progress,
        pending_surfaces: Mutex::new(surface_cursor),
        finished: AtomicBool::new(false),
    };
    Ok(tile)
}

#[cfg(test)]
mod tests {
    use super::{TileCommit, project_temporal_band, safe_deblock_mi_end, take_active_commit};
    use crate::prediction::inter::MotionFieldLayout;
    use std::sync::Mutex;

    struct TemporalProjectionCase {
        plan: super::TemporalBandPlan,
        temporal: super::TemporalMvContext,
        fields: Vec<Option<super::MotionFieldHandle>>,
        source: super::TemporalMotionField,
    }

    fn temporal_projection_case() -> Result<TemporalProjectionCase, &'static str> {
        let target_layout = MotionFieldLayout::new(16, 16, 16).ok_or("valid layout")?;
        let source =
            super::TemporalMotionField::new_with_metadata(16, 16, true, (64, 64), &[Some(3)])
                .ok_or("valid source field")?;
        let other =
            super::TemporalMotionField::new_with_metadata(16, 16, true, (64, 64), &[Some(1)])
                .ok_or("valid other field")?;
        let metadata = vec![Some(source.metadata()), Some(other.metadata())];
        let layouts = vec![Some(source.layout()), Some(other.layout())];
        let mut temporal = super::TemporalMvContext::empty();
        let plan = temporal
            .begin_banded_refresh(
                target_layout,
                2,
                super::TemporalProjectionConfig {
                    frame_size: (64, 64),
                    step: 1,
                    unit_size8: 8,
                    enable_tip: true,
                    enable_trajectory: false,
                    reduced: true,
                },
                &[0, 1],
                &[true, true],
                &[1, 3],
                &metadata,
                &layouts,
                None,
                false,
                false,
            )
            .ok_or("valid banded plan")?;
        if plan.requirements(0) != [(0, 0)] {
            return Err("expected one source-band requirement");
        }
        let handle = super::MotionFieldHandle::pending_with_layout(source.layout());
        handle.publish_metadata(source.metadata());
        Ok(TemporalProjectionCase {
            plan,
            temporal,
            fields: vec![Some(handle)],
            source,
        })
    }

    #[test]
    fn pending_reference_motion_band_is_a_scheduling_invariant() -> Result<(), &'static str> {
        let TemporalProjectionCase {
            plan,
            temporal,
            fields,
            ..
        } = temporal_projection_case()?;

        let Err(error) = project_temporal_band(&plan, &temporal, &fields, 0) else {
            return Err("pending publication must fail");
        };

        assert!(matches!(
            error,
            crate::DecodeError::HeaderState {
                source: crate::DecodeHeaderStateError::InvalidInterTileSchedulingState
            }
        ));
        assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
        Ok(())
    }

    #[test]
    fn failed_reference_motion_band_is_a_typed_dependency_error() -> Result<(), &'static str> {
        let TemporalProjectionCase {
            plan,
            temporal,
            fields,
            ..
        } = temporal_projection_case()?;
        let handle = fields[0].as_ref().ok_or("reference handle")?;
        handle.fail();

        let Err(error) = project_temporal_band(&plan, &temporal, &fields, 0) else {
            return Err("failed publication must fail");
        };

        assert!(matches!(
            error,
            crate::DecodeError::ReferenceState {
                source: crate::DecodeReferenceStateError::MissingMotionFieldBandPublication {
                    slot: 0,
                    band: 0
                }
            }
        ));
        assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
        Ok(())
    }

    #[test]
    fn published_reference_motion_band_survives_failure_and_projects() -> Result<(), &'static str> {
        let TemporalProjectionCase {
            plan,
            temporal,
            fields,
            source,
        } = temporal_projection_case()?;
        let handle = fields[0].as_ref().ok_or("reference handle")?;
        handle.publish(source);
        handle.fail();

        assert!(matches!(handle.band_publication(0), Some(Some(_))));
        project_temporal_band(&plan, &temporal, &fields, 0)
            .map_err(|_| "published band must project")?;
        Ok(())
    }

    #[test]
    fn missing_active_commit_is_a_typed_scheduling_invariant() -> Result<(), &'static str> {
        let holder = Mutex::new(None::<TileCommit<u8>>);

        let error = take_active_commit(&holder)
            .err()
            .ok_or("missing commit state must fail")?;

        assert!(matches!(
            error,
            crate::DecodeError::HeaderState {
                source: crate::DecodeHeaderStateError::InvalidInterTileSchedulingState
            }
        ));
        assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
        Ok(())
    }

    #[test]
    fn admission_grid_failure_precedes_empty_motion_band_publication() -> Result<(), &'static str> {
        let field = super::TemporalMotionField::new(8, 8).ok_or("valid motion field")?;
        let handle = super::MotionFieldHandle::pending_with_layout(field.layout());
        handle.publish_metadata(field.metadata());
        let observer = handle.clone();

        let result = super::prepare_scheduled_motion(1..1, 0..1, field, 0, 1, handle);

        assert!(matches!(
            result,
            Err(crate::DecodeError::HeaderState {
                source: crate::DecodeHeaderStateError::InvalidInterTileConstructionState
            })
        ));
        assert!(observer.band(0).is_none());
        assert!(observer.field().is_none());
        Ok(())
    }

    #[test]
    fn successful_admission_grid_construction_allows_empty_band_publication()
    -> Result<(), &'static str> {
        let field = super::TemporalMotionField::new(8, 8).ok_or("valid motion field")?;
        let handle = super::MotionFieldHandle::pending_with_layout(field.layout());
        handle.publish_metadata(field.metadata());
        let observer = handle.clone();

        let _state = super::prepare_scheduled_motion(0..8, 0..8, field, 0, 1, handle)
            .map_err(|_| "valid scheduled motion")?;

        assert!(observer.band(0).is_some());
        assert!(observer.field().is_some());
        Ok(())
    }

    fn frontier(
        completed_units: usize,
        units_per_row: usize,
        sb_h4: usize,
        mi_rows: usize,
        terminal: bool,
    ) -> Option<usize> {
        safe_deblock_mi_end(completed_units, units_per_row, sb_h4, mi_rows, terminal)
            .map(core::num::NonZeroUsize::get)
    }

    #[test]
    fn raw_safe_deblock_frontier_keeps_the_reconstruction_read_lead() {
        for (completed_units, expected) in
            [None, Some(10), Some(26), Some(42)].into_iter().enumerate()
        {
            assert_eq!(frontier(completed_units, 1, 16, 64, false), expected);
        }
        assert_eq!(frontier(4, 1, 16, 64, true), Some(64));
    }

    #[test]
    fn wide_frame_frontier_counts_only_complete_superblock_rows() {
        for completed_units in 0..3 {
            assert_eq!(frontier(completed_units, 3, 16, 64, false), None);
        }
        for completed_units in 3..6 {
            assert_eq!(frontier(completed_units, 3, 16, 64, false), Some(10));
        }
        assert_eq!(frontier(6, 3, 16, 64, false), Some(26));
        assert_eq!(frontier(9, 3, 16, 64, false), Some(42));
    }

    #[test]
    fn frontier_never_reaches_the_frame_bottom_before_terminal_commit() {
        assert_eq!(frontier(12, 3, 16, 64, false), Some(58));
        assert_eq!(frontier(15, 3, 16, 60, false), Some(54));
    }

    #[test]
    fn short_superblock_rows_keep_a_zero_frontier() {
        assert_eq!(frontier(1, 1, 4, 64, false), None);
        assert_eq!(frontier(2, 1, 4, 64, false), Some(2));
    }

    #[test]
    fn every_whole_frame_deblock_plan_seals_its_own_filter_copy() {
        assert!(super::seals_filter_copy(true, true));
        assert!(!super::seals_filter_copy(false, true));
        assert!(!super::seals_filter_copy(true, false));
    }

    #[test]
    fn precompute_batches_never_cross_a_superblock_row() {
        let batches = super::superblock_row_batches(35, 15, 4);
        assert_eq!(
            batches,
            vec![
                0..4,
                4..8,
                8..12,
                12..15,
                15..19,
                19..23,
                23..27,
                27..30,
                30..34,
                34..35,
            ]
        );
        for batch in &batches {
            assert_eq!(batch.start / 15, (batch.end - 1) / 15);
        }
    }

    #[test]
    fn precompute_batches_cover_every_unit_once_and_in_order() {
        for units_per_row in 1..8 {
            for unit_count in 0..24 {
                let batches = super::superblock_row_batches(unit_count, units_per_row, 4);
                let mut next = 0;
                for batch in &batches {
                    assert_eq!(batch.start, next);
                    assert!(batch.end > batch.start);
                    assert!(batch.len() <= 4);
                    next = batch.end;
                }
                assert_eq!(next, unit_count);
            }
        }
    }

    #[test]
    fn every_frontier_link_is_handed_over_exactly_once_in_order() {
        for (unit_count, units_per_row) in [(136usize, 15usize), (135, 15), (9, 3), (1, 1), (7, 4)]
        {
            let frontier_rows = unit_count.div_ceil(units_per_row).max(1);
            let mut handed = 0;
            for committed in 1..=unit_count {
                let closed = super::closed_frontier_rows(
                    committed,
                    unit_count,
                    units_per_row,
                    frontier_rows,
                );
                assert!(closed >= handed, "frontier rows went backwards");
                assert!(closed <= frontier_rows, "frontier rows overran the chain");
                handed = closed;
            }
            assert_eq!(handed, frontier_rows, "the terminal commit owes every link");
        }
    }

    #[test]
    fn a_trailing_empty_unit_closes_the_last_frontier_link_once() {
        assert_eq!(super::closed_frontier_rows(135, 136, 15, 10), 9);
        assert_eq!(super::closed_frontier_rows(136, 136, 15, 10), 10);
    }
}
