// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared frame-plane storage and filter-record capacity hints for one decode.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::filters::wienerns_lr::FrameFilterRecordCapacities;
use crate::prediction::inter::{TemporalMotionBlock, TemporalMvContext};

/// The TIP output walk's temporal context and motion records.
pub(crate) type TipTemporal = (TemporalMvContext, Vec<TemporalMotionBlock>);

/// The storage one decode's finished work leaves for the work behind it.
#[derive(Default)]
pub(crate) struct DecodeBuffers {
    planes: Arc<splot_recon::PlanePool>,
    tile_records: Mutex<FrameFilterRecordCapacities>,
    /// Per superblock unit, the row-list capacities its spent buffers reached.
    row_capacities: Mutex<Vec<RowCapacities>>,
    /// One TIP walk's temporal state for the whole decode: TIP output frames
    /// are rare, so a copy per reconstruction lane is memory nobody reads.
    tip_temporal: Mutex<Option<TipTemporal>>,
}

/// The capacities of one superblock unit's growable row lists.
pub(crate) type RowCapacities = [usize; 6];

impl DecodeBuffers {
    /// Opens the storage for one decode.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The frame-sized plane storage, which reconstruction workspaces name so
    /// their buffers come home whichever holder releases them last.
    pub(crate) fn planes(&self) -> &Arc<splot_recon::PlanePool> {
        &self.planes
    }

    /// The record capacities a spent tile reached, for the next tile's set.
    pub(crate) fn tile_record_capacities(&self) -> FrameFilterRecordCapacities {
        *self.tile_records.lock()
    }

    /// Notes the record capacities one spent tile reached.
    pub(crate) fn note_tile_record_capacities(&self, reached: FrameFilterRecordCapacities) {
        self.tile_records.lock().cover(reached);
    }

    /// The row-list capacities any frame's buffers reached for `unit`.
    pub(crate) fn row_capacities(&self, unit: usize) -> RowCapacities {
        self.row_capacities
            .lock()
            .get(unit)
            .copied()
            .unwrap_or_default()
    }

    /// Notes the row-list capacities one spent unit reached, so every frame
    /// in flight sizes that unit's lists once instead of growing them apart.
    pub(crate) fn note_row_capacities(&self, unit: usize, reached: RowCapacities) {
        let mut table = self.row_capacities.lock();
        if table.len() <= unit {
            table.resize(unit + 1, RowCapacities::default());
        }
        for (held, reached) in table[unit].iter_mut().zip(reached) {
            *held = (*held).max(reached);
        }
    }

    /// Takes the decode's TIP temporal state, or a new one if a walk holds it.
    pub(crate) fn take_tip_temporal(&self) -> TipTemporal {
        self.tip_temporal
            .lock()
            .take()
            .unwrap_or_else(|| (TemporalMvContext::empty(), Vec::new()))
    }

    /// Gives the TIP temporal state back for the next TIP output frame.
    pub(crate) fn park_tip_temporal(&self, state: TipTemporal) {
        *self.tip_temporal.lock() = Some(state);
    }
}
