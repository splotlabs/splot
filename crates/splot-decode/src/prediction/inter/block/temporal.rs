// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::sync::atomic::{AtomicUsize, Ordering};

use splot_recon::ReconSample;

use super::super::find_mv_stack::{TemporalMotionBlock, TemporalMotionField};
use super::super::{InterReferenceState, MotionFieldHandle, Mv};
use parking_lot::Mutex;

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
    TemporalMotionBlock::new(
        mi_row,
        mi_col,
        n4w,
        n4h,
        mi_rows,
        mi_cols,
        current_order_hint,
        [
            temporal_ref_order_hint(reference, ref_frame_idx, ref_frame0),
            ref_frame1.and_then(|ref_frame1| {
                temporal_ref_order_hint(reference, ref_frame_idx, ref_frame1)
            }),
        ],
        [mv0, mv1],
        warp_params,
    )
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
    units: usize,
    bands: Vec<MotionBandUnits>,
    units_per_row: usize,
    handle: Option<MotionFieldHandle>,
}

struct MotionBandUnits {
    owed: AtomicUsize,
}

impl MotionFieldUnits {
    /// Collects one frame's units and leaves the field for the caller to take.
    pub(super) fn new(field: TemporalMotionField) -> Self {
        Self {
            field: Mutex::new(Some(field)),
            owed: AtomicUsize::new(0),
            units: 0,
            bands: Vec::new(),
            units_per_row: 0,
            handle: None,
        }
    }

    /// Collects `units` units and publishes the field once the last one lands.
    ///
    pub(super) fn publishing(
        field: TemporalMotionField,
        units: usize,
        units_per_row: usize,
        handle: MotionFieldHandle,
    ) -> crate::Result<Self> {
        let mut this = Self::new(TemporalMotionField::empty());
        this.reset_publishing(field, units, units_per_row, handle)?;
        Ok(this)
    }

    pub(super) fn retire(&mut self) {
        self.handle.take();
        self.field.get_mut().take();
    }

    pub(super) fn reset_publishing(
        &mut self,
        field: TemporalMotionField,
        units: usize,
        units_per_row: usize,
        handle: MotionFieldHandle,
    ) -> crate::Result<()> {
        if field.layout() != handle.layout() {
            return Err(crate::DecodeHeaderStateError::InvalidInterTemporalMotionState.into());
        }
        let metadata = field.metadata();
        drop(field);
        handle.begin_bands(&metadata)?;
        self.bands
            .resize_with(handle.layout().band_count(), || MotionBandUnits {
                owed: AtomicUsize::new(0),
            });
        for (index, band) in self.bands.iter_mut().enumerate() {
            let start = index.saturating_mul(units_per_row).min(units);
            let end = start.saturating_add(units_per_row).min(units);
            *band.owed.get_mut() = end.saturating_sub(start);
        }
        self.field.get_mut().take();
        *self.owed.get_mut() = units;
        self.units = units;
        self.units_per_row = units_per_row;
        self.handle = Some(handle);
        self.publish_empty_bands();
        Ok(())
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

    /// Folds one source unit into its exclusive full-width row-band owner.
    pub(super) fn fold_unit(
        &self,
        ordinal: usize,
        records: &[TemporalMotionBlock],
    ) -> crate::Result<()> {
        if self.bands.is_empty() {
            self.fold(records);
            return Ok(());
        }
        if records.is_empty() || self.units_per_row == 0 {
            return Ok(());
        }
        if let Some(handle) = self.handle.as_ref() {
            handle.fold_band(ordinal / self.units_per_row, records)?;
        }
        Ok(())
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

    /// Settles one unit and publishes its source superblock-row band when the
    /// last horizontal unit in that row lands.
    pub(super) fn unit_landed_for(&self, ordinal: usize) {
        if self.bands.is_empty() {
            self.unit_landed();
            return;
        }
        if ordinal >= self.units {
            if let Some(handle) = self.handle.as_ref() {
                handle.fail();
            }
            return;
        }
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        if self.units_per_row == 0 {
            handle.fail();
            return;
        }
        let band_index = ordinal / self.units_per_row;
        let Some(band) = self.bands.get(band_index) else {
            handle.fail();
            return;
        };
        if band.owed.fetch_sub(1, Ordering::AcqRel) == 1 {
            handle.publish_builder_band(band_index);
        }
        if self.owed.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        handle.publish_whole_from_bands();
    }

    /// Takes the field, or an empty one once it has been published.
    pub(super) fn into_field(self) -> TemporalMotionField {
        self.field
            .into_inner()
            .unwrap_or_else(TemporalMotionField::empty)
    }

    fn locked(&self) -> parking_lot::MutexGuard<'_, Option<TemporalMotionField>> {
        self.field.lock()
    }

    fn publish_empty_bands(&self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        for (index, band) in self.bands.iter().enumerate() {
            if band.owed.load(Ordering::Acquire) == 0 {
                handle.publish_builder_band(index);
            }
        }
        if self.owed.load(Ordering::Acquire) == 0 {
            handle.publish_whole_from_bands();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn block(order_hint: u32, mv: Mv) -> TemporalMotionBlock {
        TemporalMotionBlock::new(
            0,
            0,
            2,
            2,
            2,
            2,
            0,
            [Some(order_hint), None],
            [mv, Mv::ZERO],
            [None, None],
        )
    }

    #[test]
    fn retired_motion_units_reuse_counters_and_release_publication() {
        let mut field = TemporalMotionField::new(8, 8).expect("field");
        field.set_reference_metadata(true, (32, 32), &[Some(0)]);
        let layout = field.layout();
        let mut handle = MotionFieldHandle::pending_with_layout(layout);
        let mut units = MotionFieldUnits::publishing(field, 1, 1, handle.clone()).expect("units");
        let counters = units.bands.as_ptr();
        for _ in 0..1200 {
            units.unit_landed_for(0);
            assert!(handle.field().is_some());
            units.retire();
            assert!(handle.try_retire());
            handle.reset_layout(layout).expect("reset layout");
            let field = TemporalMotionField::metadata_only(layout, true, (32, 32), &[Some(0)]);
            units
                .reset_publishing(field, 1, 1, handle.clone())
                .expect("reset units");
            assert_eq!(units.bands.as_ptr(), counters);
            assert!(handle.field().is_none());
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

    #[test]
    fn non_inter_clear_record_resets_a_previously_stored_cell() {
        let clear = TemporalMotionBlock::new(
            0,
            0,
            2,
            2,
            2,
            2,
            0,
            [None, None],
            [Mv::ZERO; 2],
            [None, None],
        );
        let mut cleared = TemporalMotionField::new(2, 2).expect("cleared field");
        cleared.set_reference_metadata(true, (8, 8), &[Some(1), Some(2)]);
        cleared.record_block(block(1, Mv { row: 8, col: 16 }));
        cleared.record_block(clear);

        let mut untouched = TemporalMotionField::new(2, 2).expect("untouched field");
        untouched.set_reference_metadata(true, (8, 8), &[Some(1), Some(2)]);
        assert_eq!(cleared, untouched);
    }

    #[test]
    fn out_of_range_unit_settles_the_publication_as_failed() {
        let mut field = TemporalMotionField::new(2, 2).expect("motion field");
        field.set_reference_metadata(true, (8, 8), &[Some(1)]);
        let handle = MotionFieldHandle::pending_with_layout(field.layout());
        let units =
            MotionFieldUnits::publishing(field, 1, 1, handle.clone()).expect("motion units");

        units.unit_landed_for(1);

        assert!(handle.field().is_none());
        assert!(handle.band_publication(0).is_some_and(Option::is_none));
    }
}
