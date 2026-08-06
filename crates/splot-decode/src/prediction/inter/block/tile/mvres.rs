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

/// Derives one unit's motion and folds its records into the frame's field.
///
/// A unit whose derivation fails is left exactly as the parse pass produced it
/// — no grid, no record, unlanded — so the reconstruction pass derives its own.
pub(super) fn derive_unit_motion<T: ReconSample>(
    row: &mut ReconRow,
    surface: Option<&mut ReadyReconSurface<'_, T>>,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    motion: &MotionFieldUnits,
    shared: &deferred_recon::ReconShared<'_, T>,
) {
    if let Some(surface) = surface {
        let sink = surface.sink();
        if !derive_row_motion(row, scratch, &sink, shared) {
            return;
        }
    }
    row.motion_derived = true;
    row.motion_folded = true;
    motion.fold_unit(row.ordinal, &row.temporal);
    motion.unit_landed_for(row.ordinal, true);
}

pub(super) fn derive_unit_motion_on_surface<T: ReconSample>(
    row: &mut ReconRow,
    surface: &mc::WorkspaceSink<'_, '_, T>,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    motion: &MotionFieldUnits,
    shared: &deferred_recon::ReconShared<'_, T>,
) {
    if !derive_row_motion(row, scratch, surface, shared) {
        return;
    }
    row.motion_derived = true;
    row.motion_folded = true;
    motion.fold_unit(row.ordinal, &row.temporal);
    motion.unit_landed_for(row.ordinal, true);
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
                entry.store_motion(grid, &mut row.motion_grids);
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
    row.motion_grids.clear();
    for entry in &mut row.entries {
        entry.motion = None;
        entry.temporal = 0..0;
    }
    row.terminal.get_or_insert(error);
    false
}
