// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Admission-scheduled reconstruction state for one parsed tile.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`,
//! `INFRA-DECODE-FRAME-PIPELINING`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use splot_core::headers::frame::RefIdxBuf;
use splot_parallel::Condition;

use super::*;
use crate::prediction::inter::block::row_gate::RowReferenceGate;
use crate::prediction::inter::find_mv_stack::TemporalBandPlan;

struct ScheduledCommit<T: ReconSample> {
    next: usize,
    handed_rows: usize,
    ordered: deferred_recon::InterReconScratch<T>,
    workspace: CurrentFrameWorkspace<T>,
    block_decoded: TileBlockDecodedState,
    current_block_decoded_superblock: Option<[usize; 2]>,
    decoded_any: bool,
    surfaces: Vec<splot_recon::OwnedFrameRect<T>>,
}

/// The § 7.17 frontier's own storage, advanced by one ordered chain per frame.
///
/// The chain runs beside the commit spine, so it owns the pixels it filters:
/// either the sealed copy the spine hands it one superblock row at a time, or
/// the canonical row bands a banded frame reconstructs directly into. A frame
/// with no active deblock plan has nothing for the chain to advance, so sealing
/// would only add a copy; it receives the spine's whole workspace once
/// reconstruction is complete.
struct ScheduledFrontier<T: ReconSample> {
    sealed: Option<CurrentFrameWorkspace<T>>,
    sealed_rows: usize,
    terminal_workspace: Option<CurrentFrameWorkspace<T>>,
    bands: Option<splot_recon::OwnedFrameBands<T>>,
    deblock: Option<crate::filters::deblock::FrameDeblock<'static>>,
    filter: Option<Arc<crate::filters::wienerns_lr::recon::OwnedFilterSetup<'static, 'static, T>>>,
    next_filter_stripe: usize,
}

#[allow(clippy::large_enum_variant)]
enum PreparedBatch<T: ReconSample> {
    Legacy(Vec<ReadyReconRow<'static, T>>),
    Banded {
        rows: Vec<ReconRow>,
        band: splot_recon::OwnedFrameRowBand<T>,
    },
}

struct ScheduledResolve {
    next: usize,
    submitted_batches: usize,
    grid: NeighbourMvGrid,
    state: TileResolveState,
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
    .ok_or(DecodeHeaderStateError::InvalidInterTileSchedulingState.into())
}

#[allow(clippy::large_enum_variant)]
enum ScheduledRowState<T: ReconSample> {
    Unresolved {
        row: ReconRow,
        surface: Option<ReadyReconSurface<'static, T>>,
    },
    Ready(ReadyReconRow<'static, T>),
    Taken,
}

/// Filter jobs made ready by one ordered frontier link.
pub(in crate::prediction::inter::block) struct ScheduledTileProgress<T: ReconSample> {
    pub(in crate::prediction::inter::block) filters:
        Vec<crate::filters::wienerns_lr::recon::OwnedFilterJob<T>>,
    pub(in crate::prediction::inter::block) output: Option<ScheduledTileOutput<T>>,
}

/// Canonical rows one ordered commit handed to the frontier chain.
pub(crate) struct ScheduledCommitProgress {
    /// Frontier links whose rows are now sealed, in chain order.
    pub(crate) frontier_rows: core::ops::Range<usize>,
    /// Whether this commit completed the frame's reconstruction.
    pub(crate) recon_complete: bool,
}

/// Completed scheduled tile reconstruction.
pub(in crate::prediction::inter::block) struct ScheduledTileOutput<T: ReconSample> {
    pub(in crate::prediction::inter::block) filter:
        crate::filters::wienerns_lr::recon::OwnedFilterFinish<T>,
}

/// Owned state shared by one admission job per parsed reconstruction unit.
pub(in crate::prediction::inter::block) struct ScheduledTileRecon<T: ReconSample> {
    rows: Mutex<Vec<ScheduledRowState<T>>>,
    prepared: Mutex<Vec<Option<PreparedBatch<T>>>>,
    unit_count: usize,
    units_per_row: usize,
    batches: Vec<core::ops::Range<usize>>,
    owned_bands: bool,
    filter_count: usize,
    frontier_rows: usize,
    commit: Mutex<Option<ScheduledCommit<T>>>,
    scratch: Mutex<Option<TileDecodeScratch<T>>>,
    frontier: Mutex<ScheduledFrontier<T>>,
    resolve: Mutex<ScheduledResolve>,
    workers: InterReconScratchPool<T>,
    prepass_block_decoded: TileBlockDecodedState,
    motion: MotionFieldUnits,
    info: splot_recon::DecodedFrameInfo,
    params: TileWalkParams,
    quantizer: FrameQuantizerSnapshot,
    temporal: Arc<TemporalMvContext>,
    temporal_plan: TemporalBandPlan,
    reference: Arc<InterReferenceState<T>>,
    ref_frame_idx: RefIdxBuf,
    sequence: Arc<SequenceHeader>,
    core: Arc<FrameHeaderCore>,
    tile_offset: ByteOffset,
    finished: AtomicBool,
}

impl<T: ReconSample> ScheduledTileRecon<T> {
    /// Number of independently admitted reconstruction units.
    pub(in crate::prediction::inter::block) const fn len(&self) -> usize {
        self.batches.len()
    }

    /// Number of final-filter stripes this frame owes.
    pub(in crate::prediction::inter::block) const fn filter_count(&self) -> usize {
        self.filter_count
    }

    pub(in crate::prediction::inter::block) fn resolve_len(&self) -> usize {
        self.temporal_plan.len()
    }

    pub(in crate::prediction::inter::block) fn resolve_conditions(
        &self,
        index: usize,
    ) -> Vec<Condition<'_>> {
        self.temporal_plan
            .requirements(index)
            .into_iter()
            .filter_map(|(slot, band)| {
                self.reference
                    .ref_motion_fields
                    .get(slot)
                    .and_then(Option::as_ref)
                    .and_then(|field| field.band_condition(band))
            })
            .collect()
    }

    pub(in crate::prediction::inter::block) fn resolve(
        &self,
        index: usize,
    ) -> Result<core::ops::Range<usize>> {
        project_temporal_band(
            &self.temporal_plan,
            &self.temporal,
            &self.reference.ref_motion_fields,
            index,
        )?;
        let projected_rows = self.temporal_plan.rows8(index);
        let final_band = index.saturating_add(1) == self.temporal_plan.len();
        let context = self.params.context(
            &self.sequence,
            &self.core,
            &self.reference,
            self.ref_frame_idx.as_slice(),
        );
        let row_gate = RowReferenceGate::new(
            &self.reference,
            &self.core,
            self.ref_frame_idx.as_slice(),
            self.info,
            &self.temporal,
        );
        let mut resolve = self.resolve.lock().unwrap_or_else(PoisonError::into_inner);
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let next = resolve.next;
            let Some(ScheduledRowState::Unresolved { row, .. }) = rows.get(next) else {
                break;
            };
            let row_end8 = row
                .superblocks
                .iter()
                .filter_map(|superblock| superblock.origin[0].checked_add(self.params.sb_h4))
                .map(|end4| end4.div_ceil(2))
                .max()
                .unwrap_or(projected_rows.end);
            if !final_band && row_end8 > projected_rows.end {
                break;
            }
            let Some(slot) = rows.get_mut(next) else {
                break;
            };
            let ScheduledRowState::Unresolved { mut row, surface } =
                core::mem::replace(slot, ScheduledRowState::Taken)
            else {
                break;
            };
            resolve.grid.replay_flag_log(&row.flag_log);
            if let Some(error) = row.terminal.take() {
                return Err(error);
            }
            {
                let ScheduledResolve { grid, state, .. } = &mut *resolve;
                state.resolve_unit(grid, &context, &self.temporal, &mut row, self.tile_offset)
            }?;
            if let Some(error) = row.terminal.take() {
                return Err(error);
            }
            let bounds = row_gate.bounds_for_row(&row);
            if let Some(slot) = rows.get_mut(next) {
                *slot = ScheduledRowState::Ready(ReadyReconRow {
                    row,
                    surface,
                    bounds,
                });
            }
            resolve.next = resolve.next.saturating_add(1);
        }
        let ready_batches = self
            .batches
            .partition_point(|batch| batch.end <= resolve.next);
        let ready = resolve.submitted_batches..ready_batches;
        resolve.submitted_batches = ready_batches;
        Ok(ready)
    }

    pub(in crate::prediction::inter::block) fn fail_temporal(&self) {
        TemporalBandPlan::fail(&self.temporal);
    }

    fn batch_range(&self, index: usize) -> Option<core::ops::Range<usize>> {
        self.batches.get(index).cloned()
    }

    /// Conditions that replace the parsed unit's former cross-frame wait.
    pub(in crate::prediction::inter::block) fn conditions(
        &self,
        index: usize,
    ) -> Vec<Condition<'_>> {
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut bounds = row_gate::RowReferenceBounds::default();
        for ready in self
            .batch_range(index)
            .and_then(|range| rows.get(range))
            .into_iter()
            .flatten()
            .filter_map(|row| match row {
                ScheduledRowState::Ready(ready) => Some(ready),
                ScheduledRowState::Unresolved { .. } | ScheduledRowState::Taken => None,
            })
        {
            bounds.merge(ready.bounds);
        }
        RowReferenceGate::new(
            &self.reference,
            &self.core,
            self.ref_frame_idx.as_slice(),
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
        let range = self
            .batch_range(index)
            .ok_or_else(invalid_inter_tile_scheduling_state)?;
        {
            let prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(slot) = prepared.get(index) else {
                self.finished.store(true, Ordering::Release);
                return Err(invalid_inter_tile_scheduling_state());
            };
            if slot.is_some() {
                self.finished.store(true, Ordering::Release);
                return Err(invalid_inter_tile_scheduling_state());
            }
        }
        let ready = {
            let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(rows) = rows.get_mut(range.clone()) else {
                return Err(invalid_inter_tile_scheduling_state());
            };
            if rows
                .iter()
                .any(|row| !matches!(row, ScheduledRowState::Ready(_)))
            {
                return Err(invalid_inter_tile_scheduling_state());
            }
            let mut ready = Vec::with_capacity(rows.len());
            for row in rows {
                if let ScheduledRowState::Ready(row) =
                    core::mem::replace(row, ScheduledRowState::Taken)
                {
                    ready.push(row);
                }
            }
            ready
        };
        let batch = self
            .workers
            .with_scratch(|scratch| -> Result<PreparedBatch<T>> {
                let _quantizer_scopes = self.quantizer.install_frame();
                let ready = ready;
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
                if self.owned_bands {
                    let rows8 = self.temporal_plan.rows8(index);
                    let coded_height = self.info.coded_luma_size().height();
                    let start = rows8.start.saturating_mul(8).min(coded_height);
                    let end = rows8.end.saturating_mul(8).min(coded_height);
                    let mut band =
                        splot_recon::OwnedFrameRowBand::new(self.info, start..end, T::default())?;
                    let mut surface = band.surface_mut();
                    let mut rows = Vec::with_capacity(ready.len());
                    for mut ready in ready {
                        let mut row = scratch.with_installed(|scratch| {
                            if ready.row.terminal.is_none() && !ready.row.motion_derived {
                                mvres::derive_unit_motion_on_surface(
                                    &mut ready.row,
                                    &surface,
                                    scratch,
                                    &self.motion,
                                    &shared,
                                );
                            }
                            precompute_recon_row_on_surface(
                                ready.row,
                                &mut surface,
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
                        });
                        let first_remaining = row
                            .entries
                            .iter()
                            .position(|entry| entry.command().is_some());
                        record_banded_command_error(&mut row.precompute_error, first_remaining);
                        rows.push(row);
                    }
                    return Ok(PreparedBatch::Banded { rows, band });
                }
                Ok(PreparedBatch::Legacy(
                    ready
                        .into_iter()
                        .map(|mut ready| {
                            scratch.with_installed(|scratch| {
                                if ready.row.terminal.is_none() && !ready.row.motion_derived {
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
                        .collect::<Vec<_>>(),
                ))
            })?;
        let mut prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(slot) = prepared.get_mut(index) else {
            self.finished.store(true, Ordering::Release);
            return Err(invalid_inter_tile_scheduling_state());
        };
        if slot.is_some() {
            self.finished.store(true, Ordering::Release);
            return Err(invalid_inter_tile_scheduling_state());
        }
        *slot = Some(batch);
        Ok(())
    }

    /// Seals every canonical superblock row this commit completed into the
    /// frame's filter copy.
    ///
    /// Frames that reconstruct into the shared workspace keep their raw pixels
    /// there, while the § 7.17 frontier and the filter windows it releases read
    /// the sealed copy, so deblock never writes rows a later current-frame
    /// prediction can still read. The copy needs no fill: this seals whole
    /// rows upward from row zero, and the frontier reads only below them.
    fn seal_committed_rows(&self, commit: &ScheduledCommit<T>, rows: usize) -> Result<()> {
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
            .saturating_mul(self.params.sb_h4.saturating_mul(4).max(1))
            .min(self.info.coded_luma_size().height());
        if end > *sealed_rows {
            commit.workspace.copy_rows_into(sealed, *sealed_rows..end)?;
            *sealed_rows = end;
        }
        Ok(())
    }

    /// Number of ordered links in this frame's § 7.17 frontier chain.
    pub(in crate::prediction::inter::block) const fn frontier_len(&self) -> usize {
        self.frontier_rows
    }

    /// Advances the § 7.17 deblock frontier over the rows sealed by superblock
    /// row `row` and takes the filter stripes that frontier releases.
    ///
    /// The one caller that runs the final link receives the completed tile.
    pub(in crate::prediction::inter::block) fn frontier(
        &self,
        row: usize,
    ) -> Result<ScheduledTileProgress<T>> {
        let terminal = row.saturating_add(1) >= self.frontier_rows;
        let committed_units = row
            .saturating_add(1)
            .saturating_mul(self.units_per_row)
            .min(self.unit_count);
        let mut frontier = self.frontier.lock().unwrap_or_else(PoisonError::into_inner);
        let ScheduledFrontier {
            sealed,
            sealed_rows,
            terminal_workspace,
            bands,
            deblock,
            filter,
            next_filter_stripe,
            ..
        } = &mut *frontier;
        let sealed_rows = sealed.as_ref().map(|_| *sealed_rows);
        let filtered = sealed.as_mut().or(terminal_workspace.as_mut());
        let mut filters = Vec::new();
        if let Some(deblock) = deblock.as_mut()
            && let Some(safe_mi_end) = safe_deblock_mi_end(
                committed_units,
                self.units_per_row,
                self.params.sb_h4,
                self.params.mi_rows,
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
            match (bands.as_mut(), filtered) {
                (Some(bands), _) => {
                    deblock.advance_bands(bands, safe_mi_end.get(), self.params.bit_depth)
                }
                (None, Some(filtered)) => {
                    deblock.advance(filtered, safe_mi_end.get(), self.params.bit_depth)
                }
                (None, None) => Err(crate::filters::deblock::DeblockError::Workspace),
            }
            .map_err(|error| crate::filters::wienerns_lr::recon::deblock_prepare_error(&error))?;
            while *next_filter_stripe < filter.stripe_ranges().len() {
                let stripe = *next_filter_stripe;
                let window = match (
                    bands.as_ref(),
                    sealed.as_ref().or(terminal_workspace.as_ref()),
                ) {
                    (Some(bands), _) => filter.extract_ready_band_window(stripe, deblock, bands)?,
                    (None, Some(filtered)) => {
                        filter.extract_ready_window(stripe, deblock, filtered)?
                    }
                    (None, None) => None,
                };
                let Some(window) = window else {
                    break;
                };
                filters
                    .try_reserve(1)
                    .map_err(|_| inter_allocation!("inter admission filter jobs"))?;
                filters.push(filter.owned_job(stripe, window));
                *next_filter_stripe += 1;
            }
        }
        if !terminal {
            return Ok(ScheduledTileProgress {
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
    ) -> Result<ScheduledTileProgress<T>> {
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
            let window = match (
                frontier.bands.as_ref(),
                frontier
                    .sealed
                    .as_ref()
                    .or(frontier.terminal_workspace.as_ref()),
            ) {
                (Some(bands), _) => filter.extract_terminal_band_window(stripe, bands)?,
                (None, Some(filtered)) => filter.extract_terminal_window(stripe, filtered)?,
                (None, None) => {
                    return Err(crate::filters::wienerns_lr::recon::lr_pipeline_state_error());
                }
            };
            filters
                .try_reserve(1)
                .map_err(|_| inter_allocation!("inter admission filter jobs"))?;
            filters.push(filter.owned_job(stripe, window));
            frontier.next_filter_stripe += 1;
        }
        for workspace in [frontier.sealed.take(), frontier.terminal_workspace.take()]
            .into_iter()
            .flatten()
        {
            workspace.recycle_planes();
        }
        Ok(ScheduledTileProgress {
            filters,
            output: Some(ScheduledTileOutput {
                filter: filter.owned_finish(),
            }),
        })
    }

    pub(in crate::prediction::inter::block) fn take_scheduled_scratch(
        &self,
    ) -> Result<TileDecodeScratch<T>> {
        self.scratch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or(inter_internal!(
                "inter_admission_recon_scratch_owner",
                self.tile_offset
            ))
    }

    /// Commits one precomputed unit after its predecessor has completed.
    ///
    /// The one caller that commits the final unit receives the completed tile.
    pub(in crate::prediction::inter::block) fn commit(
        &self,
        index: usize,
    ) -> Result<ScheduledCommitProgress> {
        if self.finished.load(Ordering::Acquire) {
            return Ok(ScheduledCommitProgress {
                frontier_rows: 0..0,
                recon_complete: false,
            });
        }
        let batch = {
            let mut prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(prepared) = prepared.get_mut(index) else {
                return Err(inter_internal!(
                    "inter_admission_prepared_range_missing",
                    self.tile_offset
                ));
            };
            prepared.take().ok_or(inter_internal!(
                "inter_admission_prepared_rows_missing",
                self.tile_offset
            ))?
        };
        let (ready, completed_band) = match batch {
            PreparedBatch::Legacy(ready) => (ready, None),
            PreparedBatch::Banded { rows, band } => (
                rows.into_iter()
                    .map(|row| ReadyReconRow {
                        row,
                        surface: None,
                        bounds: row_gate::RowReferenceBounds::default(),
                    })
                    .collect(),
                Some(band),
            ),
        };
        let mut holder = self.commit.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(commit) = holder.as_mut() else {
            return Ok(ScheduledCommitProgress {
                frontier_rows: 0..0,
                recon_complete: false,
            });
        };
        for mut ready in ready {
            if commit.next != ready.row.ordinal {
                self.finished.store(true, Ordering::Release);
                return Err(inter_internal!(
                    "inter_admission_commit_order",
                    self.tile_offset
                ));
            }
            if let Some(surface) = ready.surface.take() {
                surface.publish_into(&mut commit.workspace)?;
                if let ReadyReconSurface::Owned(surface) = surface {
                    commit.surfaces.push(surface);
                }
            }
            let buffers = commit.ordered.with_installed(|scratch| {
                pixel_commit::replay_recon_row(
                    ready.row,
                    &mut commit.next,
                    &mut commit.decoded_any,
                    &self.quantizer,
                    scratch,
                    &mut commit.workspace,
                    &mut commit.block_decoded,
                    &mut commit.current_block_decoded_superblock,
                    &self.motion,
                    &mut crate::filters::wienerns_lr::FrameFilterRecords::default(),
                    &self.temporal,
                    &self.reference,
                    self.ref_frame_idx.as_slice(),
                    &self.sequence,
                    &self.core,
                    self.params.mi_rows,
                    self.params.mi_cols,
                    self.params.current_order_hint,
                    self.params.luma_use_tcq,
                    self.params.residual_use_ddt,
                    self.params.bit_depth,
                )
            })?;
            recycle_retained_recon_row_buffers(buffers);
        }
        if let Some(band) = completed_band {
            let mut frontier = self.frontier.lock().unwrap_or_else(PoisonError::into_inner);
            frontier
                .bands
                .as_mut()
                .ok_or(inter_internal!(
                    "inter_admission_band_owner",
                    self.tile_offset
                ))?
                .push(band)?;
        }
        let terminal = commit.next == self.unit_count;
        let closed_rows = closed_frontier_rows(
            commit.next,
            self.unit_count,
            self.units_per_row,
            self.frontier_rows,
        );
        if closed_rows > commit.handed_rows {
            self.seal_committed_rows(commit, closed_rows)?;
        }
        let frontier_rows = commit.handed_rows..closed_rows;
        commit.handed_rows = closed_rows;
        if !terminal {
            return Ok(ScheduledCommitProgress {
                frontier_rows,
                recon_complete: false,
            });
        }
        if !commit.decoded_any {
            self.finished.store(true, Ordering::Release);
            return Err(no_decoded_block_error());
        }
        self.finished.store(true, Ordering::Release);
        let mut commit = holder.take().ok_or(inter_internal!(
            "inter_admission_commit_state",
            self.tile_offset
        ))?;
        commit.surfaces.reverse();
        let scratch =
            TileDecodeScratch::from_scheduled(commit.ordered, &self.workers, commit.surfaces);
        *self.scratch.lock().unwrap_or_else(PoisonError::into_inner) = Some(scratch);
        {
            let mut frontier = self.frontier.lock().unwrap_or_else(PoisonError::into_inner);
            if frontier.sealed.is_some() || frontier.bands.is_some() {
                commit.workspace.recycle_planes();
            } else {
                frontier.terminal_workspace = Some(commit.workspace);
            }
        }
        Ok(ScheduledCommitProgress {
            frontier_rows,
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
/// The frontier deblocks the sealed copy or the frame's own canonical bands, so
/// its only bound is the sealed rows it may read. Current-frame readers —
/// ordinary intra, and local or global IntraBC — keep reading the spine's raw
/// workspace, which no deblock pass writes, so an IntraBC source's liveness
/// places no constraint here.
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
/// banded frame already owns its canonical rows, and a frame with no active
/// deblock plan has nothing to advance, so neither seals.
const fn seals_filter_copy(owned_bands: bool, has_deblock: bool, whole_frame: bool) -> bool {
    !owned_bands && has_deblock && whole_frame
}

/// Whether one parsed tile owns every mode-info position in the frame, which
/// is what makes superblock row `r` the frame's own row band `r`.
fn covers_whole_frame(parsed: &ParsedTile, params: &TileWalkParams) -> bool {
    parsed.mi_rows.start == 0
        && parsed.mi_rows.end.min(params.mi_rows) == params.mi_rows
        && parsed.mi_cols.start == 0
        && parsed.mi_cols.end.min(params.mi_cols) == params.mi_cols
}

fn supports_owned_bands(
    parsed: &ParsedTile,
    params: &TileWalkParams,
    info: splot_recon::DecodedFrameInfo,
) -> core::result::Result<(), &'static str> {
    if std::env::var_os("SPLOT_DECODE_SKIP_FILTERS").is_some() {
        return Err("filters_disabled");
    }
    if !covers_whole_frame(parsed, params) {
        return Err("partial_tile");
    }
    for row in &parsed.rows {
        if row.terminal.is_some() {
            return Err("terminal");
        }
        for superblock in &row.superblocks {
            if superblock.dependency == ReconDependency::GlobalIntrabcFence {
                return Err("intrabc");
            }
            if superblock.dependency == ReconDependency::CurrentFrame {
                return Err("current_frame");
            }
            let entries = row
                .entries
                .get(superblock.entries.clone())
                .ok_or("entry_range")?;
            for entry in entries {
                match entry.command() {
                    Some(ReconCommand::Inter(command))
                        if !command.reads_current_frame()
                            && command.prepass_write_is_contained(
                                superblock.origin,
                                params.sb_h4,
                                info,
                                &row.residual_blocks,
                            ) => {}
                    Some(ReconCommand::Inter(command)) if command.reads_current_frame() => {
                        return Err("current_frame_inter");
                    }
                    Some(ReconCommand::Inter(_)) => return Err("unbounded_inter"),
                    Some(ReconCommand::GeneralIntra(_)) => return Err("general_intra"),
                    Some(ReconCommand::Intrabc(_)) => return Err("intrabc"),
                    None if entry.resolve_record().is_some_and(|record| {
                        record.prepass_write_is_contained(
                            superblock.origin,
                            params.sb_h4,
                            info,
                            &row.residual_blocks,
                        )
                    }) => {}
                    None => return Err("unbounded_resolve_record"),
                }
            }
        }
    }
    Ok(())
}

/// Reconstruction units one legacy precompute batch prepares.
const LEGACY_BATCH_UNITS: usize = 4;

fn record_banded_command_error(
    error: &mut Option<(usize, crate::DecodeError)>,
    first_remaining: Option<usize>,
) {
    if let Some(index) = first_remaining {
        let _ = error.get_or_insert_with(|| (index, invalid_inter_tile_scheduling_state()));
    }
}

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

/// Splits the units into one batch per owned canonical row band.
///
/// The banded path's batch index is its superblock row, and the last batch
/// absorbs any trailing rows the temporal plan does not name.
fn band_batches(
    unit_count: usize,
    units_per_row: usize,
    band_count: usize,
) -> Vec<core::ops::Range<usize>> {
    (0..band_count)
        .map(|index| {
            let start = index.saturating_mul(units_per_row).min(unit_count);
            let end = if index.saturating_add(1) == band_count {
                unit_count
            } else {
                start.saturating_add(units_per_row).min(unit_count)
            };
            start..end
        })
        .collect()
}

fn prepare_scheduled_motion(
    mi_rows: core::ops::Range<usize>,
    mi_cols: core::ops::Range<usize>,
    motion_field: TemporalMotionField,
    units: usize,
    units_per_row: usize,
    motion_handle: MotionFieldHandle,
    started: Option<std::time::Instant>,
) -> Result<(NeighbourMvGrid, MotionFieldUnits)> {
    let grid = NeighbourMvGrid::new_for_tile(mi_rows, mi_cols)
        .map_err(|error| inter_tile_grid_error(&error, "inter admission MV grid"))?;
    let motion =
        MotionFieldUnits::publishing(motion_field, units, units_per_row, motion_handle, started);
    Ok((grid, motion))
}

/// Resolves a parsed tile and turns each unit into owned admission state.
#[allow(clippy::large_types_passed_by_value, clippy::too_many_arguments)]
pub(in crate::prediction::inter::block) fn prepare_scheduled_tile<T: ReconSample>(
    mut scratch: TileDecodeScratch<T>,
    mut parsed: ParsedTile,
    params: TileWalkParams,
    sequence: Arc<SequenceHeader>,
    core: Arc<FrameHeaderCore>,
    temporal: Arc<TemporalMvContext>,
    reference: Arc<InterReferenceState<T>>,
    ref_frame_idx: RefIdxBuf,
    workspace: CurrentFrameWorkspace<T>,
    filter_setup: crate::filters::wienerns_lr::recon::OwnedFilterSetup<'static, 'static, T>,
    deblock_records: Option<crate::filters::deblock::OwnedDeblockRecords>,
    deblock_quant_deltas: crate::filters::deblock::DeblockQuantDeltas,
    motion_field: TemporalMotionField,
    motion_handle: MotionFieldHandle,
    temporal_plan: TemporalBandPlan,
) -> Result<ScheduledTileRecon<T>> {
    scratch.workers.ensure_workers(
        splot_parallel::current_pool_width()
            .saturating_sub(1)
            .max(1),
    );
    let info = workspace.info();
    let tile_offset = parsed.tile_offset;
    let quantizer = FrameQuantizerSnapshot::capture();
    let units_per_row = parsed
        .mi_cols
        .end
        .min(params.mi_cols)
        .saturating_sub(parsed.mi_cols.start)
        .div_ceil(params.sb_h4);
    let candidate_batch_count = parsed
        .rows
        .iter()
        .filter(|row| !row.superblocks.is_empty())
        .count()
        .div_ceil(units_per_row.max(1));
    let band_eligibility = if candidate_batch_count != temporal_plan.len() {
        Err("band_count")
    } else {
        supports_owned_bands(&parsed, &params, info)
    };
    let owned_bands = band_eligibility.is_ok();
    let storage_started = crate::timing::start();
    if storage_started.is_some() {
        crate::timing::report_detail(
            "inter_admission_storage",
            storage_started,
            if owned_bands {
                "mode=owned_bands"
            } else {
                match band_eligibility {
                    Ok(()) => "mode=legacy_rects reason=unknown",
                    Err(reason) => reason,
                }
            },
        );
    }
    let (resolve_grid, motion) = prepare_scheduled_motion(
        parsed.mi_rows.clone(),
        parsed.mi_cols.clone(),
        motion_field,
        parsed
            .rows
            .iter()
            .filter(|row| !row.superblocks.is_empty())
            .count(),
        units_per_row,
        motion_handle,
        crate::timing::start(),
    )?;
    let surfaces = if owned_bands {
        Vec::new()
    } else {
        let rects =
            superblock_luma_rects(&parsed.mi_rows, &parsed.mi_cols, &workspace, params.sb_h4)?;
        scratch
            .surfaces
            .retain(|surface| surface.info() == info && rects.contains(&surface.luma_rect()));
        rects
            .into_iter()
            .map(|rect| {
                scratch
                    .take_surface(info, rect)
                    .map(ReadyReconSurface::Owned)
            })
            .collect::<splot_recon::Result<Vec<_>>>()?
    };
    let mut surfaces = surfaces.into_iter();
    let rows = core::mem::take(&mut parsed.rows)
        .into_iter()
        .map(|row| {
            let surface = if owned_bands || row.superblocks.is_empty() {
                None
            } else {
                surfaces.next()
            };
            ScheduledRowState::Unresolved { row, surface }
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err(inter_internal!(
            "inter_admission_no_resolved_rows",
            tile_offset
        ));
    }
    let prepass_block_decoded = parsed.block_decoded.clone();
    let block_decoded = parsed.block_decoded.clone();
    let unit_count = rows.len();
    let batches = if owned_bands {
        band_batches(unit_count, units_per_row.max(1), temporal_plan.len())
    } else {
        superblock_row_batches(unit_count, units_per_row.max(1), LEGACY_BATCH_UNITS)
    };
    let batch_count = batches.len();
    let mut prepared = Vec::new();
    prepared
        .try_reserve_exact(batch_count)
        .map_err(|_| inter_allocation!("inter admission prepared batches"))?;
    prepared.resize_with(batch_count, || None);
    let deblock = deblock_records
        .zip(core.deblocking_filter_params)
        .map(|(deblock_records, filter)| {
            crate::filters::deblock::FrameDeblock::prepare_owned(
                deblock_records,
                params.mi_rows,
                params.mi_cols,
                filter,
                Arc::clone(&core),
                sequence
                    .filter
                    .is_some_and(|filter| filter.disable_loopfilters_across_tiles),
                deblock_quant_deltas,
            )
            .map_err(|error| crate::filters::wienerns_lr::recon::deblock_prepare_error(&error))
        })
        .transpose()?
        .flatten();
    let seals_filter_copy = seals_filter_copy(
        owned_bands,
        deblock.is_some(),
        covers_whole_frame(&parsed, &params),
    );
    let TileDecodeScratch {
        ordered,
        workers,
        surfaces,
    } = scratch;
    let filter_count = filter_setup.stripe_ranges().len();
    Ok(ScheduledTileRecon {
        rows: Mutex::new(rows),
        prepared: Mutex::new(prepared),
        unit_count,
        units_per_row,
        batches,
        owned_bands,
        filter_count,
        frontier_rows: unit_count.div_ceil(units_per_row.max(1)).max(1),
        commit: Mutex::new(Some(ScheduledCommit {
            next: 0,
            handed_rows: 0,
            ordered,
            workspace,
            block_decoded,
            current_block_decoded_superblock: None,
            decoded_any: false,
            surfaces,
        })),
        scratch: Mutex::new(None),
        frontier: Mutex::new(ScheduledFrontier {
            sealed: seals_filter_copy
                .then(|| CurrentFrameWorkspace::new_recycled(info))
                .transpose()?,
            sealed_rows: 0,
            terminal_workspace: None,
            bands: owned_bands.then(|| splot_recon::OwnedFrameBands::new(info)),
            deblock,
            filter: Some(Arc::new(filter_setup)),
            next_filter_stripe: 0,
        }),
        resolve: Mutex::new(ScheduledResolve {
            next: 0,
            submitted_batches: 0,
            grid: resolve_grid,
            state: TileResolveState::new(&sequence),
        }),
        workers,
        prepass_block_decoded,
        motion,
        info,
        params,
        quantizer,
        temporal,
        temporal_plan,
        reference,
        ref_frame_idx,
        sequence,
        core,
        tile_offset,
        finished: AtomicBool::new(false),
    })
}

#[cfg(test)]
mod tests {
    use super::{project_temporal_band, record_banded_command_error, safe_deblock_mi_end};
    use crate::prediction::inter::MotionFieldLayout;

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
    fn banded_command_error_preserves_the_first_precompute_failure() {
        let mut error = Some((
            3,
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: splot_recon::PlaneId::Y,
                context: "original banded precompute",
            }
            .into(),
        ));

        record_banded_command_error(&mut error, Some(7));

        assert!(matches!(
            error,
            Some((
                3,
                crate::DecodeError::Reconstruction {
                    source: splot_recon::ReconError::WorkspaceAllocationFailed {
                        plane: splot_recon::PlaneId::Y,
                        context: "original banded precompute"
                    }
                }
            ))
        ));
    }

    #[test]
    fn banded_command_error_records_the_first_remaining_command() {
        let mut error = None;

        record_banded_command_error(&mut error, Some(5));

        assert!(matches!(
            &error,
            Some((
                5,
                crate::DecodeError::HeaderState {
                    source: crate::DecodeHeaderStateError::InvalidInterTileSchedulingState
                }
            ))
        ));
        assert!(
            error
                .as_ref()
                .and_then(|(_, error)| crate::DecodeDiagnosticReport::from_decode_error(error))
                .is_none()
        );
    }

    #[test]
    fn banded_command_error_leaves_complete_rows_clean() {
        let mut error = None;

        record_banded_command_error(&mut error, None);

        assert!(error.is_none());
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
    fn admission_grid_failure_precedes_empty_motion_band_publication() -> Result<(), &'static str> {
        let field = super::TemporalMotionField::new(8, 8).ok_or("valid motion field")?;
        let handle = super::MotionFieldHandle::pending_with_layout(field.layout());
        handle.publish_metadata(field.metadata());
        let observer = handle.clone();

        let result = super::prepare_scheduled_motion(1..1, 0..1, field, 0, 1, handle, None);

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

        let _state = super::prepare_scheduled_motion(0..8, 0..8, field, 0, 1, handle, None)
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
        assert!(super::seals_filter_copy(false, true, true));
        assert!(!super::seals_filter_copy(true, true, true));
        assert!(!super::seals_filter_copy(false, false, true));
        assert!(!super::seals_filter_copy(false, true, false));
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

    #[test]
    fn band_batches_keep_one_batch_per_superblock_row() {
        assert_eq!(super::band_batches(9, 3, 3), vec![0..3, 3..6, 6..9]);
        assert_eq!(super::band_batches(11, 3, 3), vec![0..3, 3..6, 6..11]);
    }

    #[test]
    fn band_batches_partition_every_unit_once() {
        for units_per_row in 1..8 {
            for unit_count in 1..32usize {
                let band_count = unit_count.div_ceil(units_per_row);
                let batches = super::band_batches(unit_count, units_per_row, band_count);
                let mut next = 0;
                for batch in batches {
                    assert_eq!(batch.start, next);
                    assert!(batch.end > batch.start);
                    assert!(batch.end <= unit_count);
                    next = batch.end;
                }
                assert_eq!(next, unit_count);
            }
        }
    }
}
