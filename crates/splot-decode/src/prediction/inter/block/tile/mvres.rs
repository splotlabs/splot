// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One frame's AV2 § 7.12 resolve pass and the motion pass that trails it.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.
//!
//! The motion pass derives every inter entry's refinement grid and § 7.22
//! temporal records and folds them into the frame's motion field, without
//! predicting a single sample. It writes nothing the walk order constrains, so
//! a unit's motion runs the moment its own resolve is done — beside the serial
//! resolve of the units behind it — and the field publishes as soon as the last
//! one lands, a whole reconstruction pass earlier than the ordered pixel commit
//! could publish it.
//!
//! Nothing here waits, neither task nor driver: motion reads reference pixels,
//! so a unit runs only once the row gate admits the rows it reads, and a unit
//! whose references are still short is simply left to the reconstruction pass,
//! which derives its motion exactly as it did before this pass existed.

use super::*;

/// One parsed unit between its resolve and its motion derivation.
struct MotionUnit<'a, 'surface, T: ReconSample> {
    row: &'a mut ReconRow,
    surface: Option<&'a mut splot_recon::CurrentFrameRect<'surface, T>>,
    bounds: row_gate::RowReferenceBounds,
    attempted: bool,
}

/// Runs one parsed tile's § 7.12 resolve pass and derives every resolved unit's
/// motion, returning the surfaces the reconstruction pass reuses.
///
/// The resolve pass replays each unit's flag plane before resolving it, so the
/// § 7.12 probes see the same published-but-unresolved frontier the fused walk
/// saw. A unit that fails ends the unit stream exactly where the fused walk's
/// would have: the units behind it were never resolved there, so they are
/// dropped here.
///
/// The grids the units carry out are the only ones the reconstruction pass may
/// predict through: a unit whose motion fails carries none, and the
/// reconstruction pass derives its own, with the diagnostic still surfacing on
/// the ordered commit where the walk order puts it.
#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_and_derive_motion<'surface, T: ReconSample>(
    parsed: &mut ParsedTile,
    mut surfaces: Vec<splot_recon::CurrentFrameRect<'surface, T>>,
    context: &TileDecodeContext<'_, T>,
    temporal_context: &TemporalMvContext,
    quantizer: &FrameQuantizerSnapshot,
    row_gate: &row_gate::RowReferenceGate<'_, T>,
    workers: &InterReconScratchPool<T>,
    motion: &MotionFieldUnits,
) -> Result<Vec<splot_recon::CurrentFrameRect<'surface, T>>> {
    let started = crate::timing::start();
    let tile_offset = parsed.tile_offset;
    let mut grid = NeighbourMvGrid::new_for_tile(parsed.mi_rows.clone(), parsed.mi_cols.clone())
        .ok_or_else(|| {
            inter_cap!(
                "inter_parsed_mv_grid",
                tile_offset,
                "inter.mv_grid",
                SPEC_MODE_INFO
            )
        })?;
    let shared = deferred_recon::ReconShared {
        reference: context.reference,
        ref_frame_idx: context.ref_frame_idx,
        temporal_context,
        sequence: context.sequence,
        core: context.core,
        luma_use_tcq: context.luma_use_tcq,
        residual_use_ddt: context.residual_use_ddt,
        bit_depth: context.bit_depth,
        mi_rows: context.mi_rows,
        mi_cols: context.mi_cols,
        current_order_hint: context.current_order_hint,
    };
    let mut resolve_state = TileResolveState::new(context.sequence);
    let mut resolved = 0usize;
    let mut deferred: Vec<MotionUnit<'_, 'surface, T>> = Vec::new();
    let tally = crate::timing::WorkerTally::new();
    let rows = &mut parsed.rows;
    let deferred_units = &mut deferred;
    let resolved_units = &mut resolved;
    let batch_size = rows
        .len()
        .div_ceil(
            splot_parallel::current_pool_width()
                .saturating_mul(4)
                .max(1),
        )
        .max(1);
    splot_parallel::ready_task_scope(|scope| {
        let mut free_surfaces = surfaces.iter_mut();
        let mut batch = Vec::new();
        for row in rows.iter_mut() {
            grid.replay_flag_log(&row.flag_log);
            *resolved_units += 1;
            if let Err(error) =
                resolve_state.resolve_unit(&mut grid, context, temporal_context, row, tile_offset)
            {
                row.terminal = Some(error);
            }
            let stop = row.terminal.is_some();
            let surface = if row.superblocks.is_empty() {
                None
            } else {
                free_surfaces.next()
            };
            let bounds = row_gate.bounds_for_row(row);
            if stop || !row_gate.admits(&bounds) {
                deferred_units.push(MotionUnit {
                    row,
                    surface,
                    bounds,
                    attempted: stop,
                });
                if stop {
                    break;
                }
                continue;
            }
            batch.push((row, surface));
            if batch.len() >= batch_size {
                spawn_motion_batch(
                    scope,
                    core::mem::take(&mut batch),
                    quantizer,
                    workers,
                    motion,
                    &shared,
                    &tally,
                );
            }
        }
        spawn_motion_batch(scope, batch, quantizer, workers, motion, &shared, &tally);
    })
    .map_err(|_| {
        inter_cap!(
            "inter_motion_sweep_scope",
            tile_offset,
            "inter.row.task_scope",
            SPEC_MODE_INFO
        )
    })?;
    drain_deferred_motion(
        &mut deferred,
        tile_offset,
        quantizer,
        row_gate,
        workers,
        motion,
        &shared,
        &tally,
    )?;
    drop(deferred);
    if started.is_some() {
        crate::timing::report_detail(
            "pass2_resolve",
            started,
            &format!(
                "units={resolved} threads={} workers_used={}",
                splot_parallel::current_pool_width(),
                tally.workers_used(),
            ),
        );
    }
    parsed.rows.truncate(resolved);
    Ok(surfaces)
}

/// Spawns one task for a run of resolved units, so the resolve pass pays one
/// dispatch and one scratch handover per run instead of per unit.
fn spawn_motion_batch<'scope, T: ReconSample>(
    scope: &splot_parallel::TaskScope<'_, 'scope>,
    batch: Vec<(
        &'scope mut ReconRow,
        Option<&'scope mut splot_recon::CurrentFrameRect<'_, T>>,
    )>,
    quantizer: &'scope FrameQuantizerSnapshot,
    workers: &'scope InterReconScratchPool<T>,
    motion: &'scope MotionFieldUnits,
    shared: &'scope deferred_recon::ReconShared<'_, T>,
    tally: &'scope crate::timing::WorkerTally,
) {
    if batch.is_empty() {
        return;
    }
    scope.spawn(move |_| {
        tally.note_worker();
        let _quantizer_scopes = quantizer.install_frame();
        workers.with_scratch(|scratch| {
            for (row, surface) in batch {
                derive_unit_motion(row, surface, scratch, motion, shared);
            }
        });
    });
}

/// Derives the motion of the units the resolve pass could not admit yet, for
/// whichever of them the gate admits by now.
///
/// The pass never waits on a reference: the driver would idle with no
/// prediction work to fill the pool, which costs more wall than an early motion
/// field buys. A unit left underived here is simply one the reconstruction pass
/// derives itself, exactly as it did before this pass existed.
#[allow(clippy::too_many_arguments)]
fn drain_deferred_motion<T: ReconSample>(
    deferred: &mut [MotionUnit<'_, '_, T>],
    tile_offset: ByteOffset,
    quantizer: &FrameQuantizerSnapshot,
    row_gate: &row_gate::RowReferenceGate<'_, T>,
    workers: &InterReconScratchPool<T>,
    motion: &MotionFieldUnits,
    shared: &deferred_recon::ReconShared<'_, T>,
    tally: &crate::timing::WorkerTally,
) -> Result<()> {
    splot_parallel::ready_task_scope(|scope| {
        for unit in deferred.iter_mut() {
            if unit.attempted || !row_gate.admits(&unit.bounds) {
                continue;
            }
            unit.attempted = true;
            let row = &mut *unit.row;
            let surface = unit.surface.as_deref_mut();
            scope.spawn(move |_| {
                tally.note_worker();
                let _quantizer_scopes = quantizer.install_frame();
                workers.with_scratch(|scratch| {
                    derive_unit_motion(row, surface, scratch, motion, shared);
                });
            });
        }
    })
    .map_err(|_| {
        inter_cap!(
            "inter_motion_drain_scope",
            tile_offset,
            "inter.row.task_scope",
            SPEC_MODE_INFO
        )
    })
}

/// Derives one unit's motion and folds its records into the frame's field.
///
/// A unit whose derivation fails is left exactly as the parse pass produced it
/// — no grid, no record, unlanded — so the reconstruction pass derives its own.
fn derive_unit_motion<T: ReconSample>(
    row: &mut ReconRow,
    surface: Option<&mut splot_recon::CurrentFrameRect<'_, T>>,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    motion: &MotionFieldUnits,
    shared: &deferred_recon::ReconShared<'_, T>,
) {
    if let Some(surface) = surface {
        let sink = mc::WorkspaceSink::Rect(surface);
        if !derive_row_motion(row, scratch, &sink, shared) {
            return;
        }
    }
    row.motion_derived = true;
    row.motion_folded = true;
    motion.fold(&row.temporal);
    motion.unit_landed(true);
}

/// Derives every inter entry's grid and records, reporting whether all landed.
fn derive_row_motion<T: ReconSample>(
    row: &mut ReconRow,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    sink: &mc::WorkspaceSink<'_, '_, T>,
    shared: &deferred_recon::ReconShared<'_, T>,
) -> bool {
    let capacity = row.entries.iter().fold(0usize, |capacity, entry| {
        capacity.saturating_add(
            entry
                .command
                .as_ref()
                .map_or(0, ReconCommand::temporal_record_capacity),
        )
    });
    let _ = row.temporal.try_reserve(capacity);
    let mut failure = None;
    for entry in &mut row.entries {
        let Some(ReconCommand::Inter(command)) = entry.command.as_ref() else {
            continue;
        };
        let start = row.temporal.len();
        match scratch.motion(command, sink, &mut row.temporal, shared) {
            Ok(grid) => {
                entry.motion = grid;
                entry.temporal = start..row.temporal.len();
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    let Some(error) = failure else {
        return true;
    };
    row.temporal.clear();
    for entry in &mut row.entries {
        entry.motion = None;
        entry.temporal = 0..0;
    }
    row.terminal.get_or_insert(error);
    false
}
