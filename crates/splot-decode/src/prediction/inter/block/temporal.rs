// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use splot_recon::ReconSample;

use super::super::find_mv_stack::{TemporalMotionBlock, TemporalMotionField};
use super::super::{InterReferenceState, MotionFieldHandle, Mv};

pub(super) fn block_ref_within_temporal_distance<T: ReconSample>(
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    current_order_hint: u32,
    ref_frame0: i8,
) -> bool {
    let Some(hint) = usize::try_from(ref_frame0)
        .ok()
        .and_then(|list_ref| ref_frame_idx.get(list_ref))
        .and_then(|&slot| reference.ref_order_hint.get(slot as usize))
    else {
        return false;
    };
    let dist = super::super::get_relative_dist(
        current_order_hint as i32,
        i32::try_from(*hint).unwrap_or(i32::MAX),
    );
    dist.abs() <= 2
}

fn temporal_ref_order_hint<T: ReconSample>(
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    ref_frame: i8,
) -> Option<u32> {
    usize::try_from(ref_frame)
        .ok()
        .and_then(|list_ref| ref_frame_idx.get(list_ref))
        .and_then(|&slot| reference.ref_order_hint.get(slot as usize))
        .copied()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn temporal_motion_block<T: ReconSample>(
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    mi_row: usize,
    mi_col: usize,
    n4w: usize,
    n4h: usize,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    ref_frame0: i8,
    ref_frame1: Option<i8>,
    mv0: Mv,
    mv1: Mv,
    warp_params: [Option<[i32; 6]>; 2],
) -> TemporalMotionBlock {
    TemporalMotionBlock {
        mi_row,
        mi_col,
        n4w,
        n4h,
        mi_rows,
        mi_cols,
        current_order_hint,
        ref_order_hints: [
            temporal_ref_order_hint(reference, ref_frame_idx, ref_frame0),
            ref_frame1.and_then(|ref_frame1| {
                temporal_ref_order_hint(reference, ref_frame_idx, ref_frame1)
            }),
        ],
        mvs: [mv0, mv1],
        warp_params,
    }
}

pub(super) fn commit_temporal_motion_blocks(
    motion_field: &mut TemporalMotionField,
    blocks: &[TemporalMotionBlock],
) {
    for &block in blocks {
        motion_field.record_block(block);
    }
}

/// One frame's AV2 § 7.9 motion field while its parse units are still landing.
///
/// A unit writes cells no other unit touches — units are superblock or tile
/// aligned and a field cell covers 8x8 luma — so a unit folds its records in as
/// soon as they are all derived, whatever order the units finish in. Only the
/// order *inside* a unit matters, and every caller keeps it.
///
/// A frame that names its unit count publishes the field through its handle the
/// moment the last unit lands. Units whose records the prepass derives in full
/// land there, so a frame's motion field can publish at the end of its prepass
/// instead of at the end of its ordered pixel commit, which is what lets the
/// next frame's § 7.9 prelude start while this one is still committing.
pub(super) struct MotionFieldUnits {
    field: Mutex<Option<TemporalMotionField>>,
    owed: AtomicUsize,
    handle: Option<MotionFieldHandle>,
}

impl MotionFieldUnits {
    /// Collects one frame's units and leaves the field for the caller to take.
    pub(super) fn new(field: TemporalMotionField) -> Self {
        Self {
            field: Mutex::new(Some(field)),
            owed: AtomicUsize::new(0),
            handle: None,
        }
    }

    /// Collects `units` units and publishes the field once the last one lands.
    pub(super) fn publishing(
        field: TemporalMotionField,
        units: usize,
        handle: MotionFieldHandle,
    ) -> Self {
        Self {
            field: Mutex::new(Some(field)),
            owed: AtomicUsize::new(units),
            handle: Some(handle),
        }
    }

    /// Folds one run of records into the field, in the caller's own order.
    pub(super) fn fold(&self, records: &[TemporalMotionBlock]) {
        if records.is_empty() {
            return;
        }
        if let Some(field) = self.locked().as_mut() {
            commit_temporal_motion_blocks(field, records);
        }
    }

    /// Reports that every record of one unit has been folded in.
    pub(super) fn unit_landed(&self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        if self.owed.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        if let Some(field) = self.locked().take() {
            handle.publish(field);
        }
    }

    /// Takes the field, or an empty one once it has been published.
    pub(super) fn into_field(self) -> TemporalMotionField {
        self.field
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or_else(TemporalMotionField::empty)
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, Option<TemporalMotionField>> {
        self.field
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn block(order_hint: u32, mv: Mv) -> TemporalMotionBlock {
        TemporalMotionBlock {
            mi_row: 0,
            mi_col: 0,
            n4w: 2,
            n4h: 2,
            mi_rows: 2,
            mi_cols: 2,
            current_order_hint: 0,
            ref_order_hints: [Some(order_hint), None],
            mvs: [mv, Mv::ZERO],
            warp_params: [None, None],
        }
    }

    #[test]
    fn ordered_log_commit_matches_direct_recording_and_preserves_last_write() {
        let first = block(1, Mv { row: 8, col: 16 });
        let second = block(2, Mv { row: 24, col: 32 });
        let mut direct = TemporalMotionField::new(2, 2).expect("direct field");
        direct.set_reference_metadata(true, (8, 8), &[Some(1), Some(2)]);
        direct.record_block(first);
        direct.record_block(second);

        let mut logged = TemporalMotionField::new(2, 2).expect("logged field");
        logged.set_reference_metadata(true, (8, 8), &[Some(1), Some(2)]);
        commit_temporal_motion_blocks(&mut logged, &[first, second]);
        assert_eq!(logged, direct);

        let mut reversed = TemporalMotionField::new(2, 2).expect("reversed field");
        reversed.set_reference_metadata(true, (8, 8), &[Some(1), Some(2)]);
        commit_temporal_motion_blocks(&mut reversed, &[second, first]);
        assert_ne!(reversed, direct);
    }
}
