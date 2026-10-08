// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared frame-plane storage and filter-record capacity hints for one decode.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::filters::wienerns_lr::FrameFilterRecordCapacities;
use crate::prediction::inter::{TemporalMotionBlock, TemporalMvContext};

/// A frame walk's temporal context and motion records.
pub(crate) type WalkTemporal = (TemporalMvContext, Vec<TemporalMotionBlock>);

/// The storage one decode's finished work leaves for the work behind it.
#[derive(Default)]
pub(crate) struct DecodeBuffers {
    planes: Arc<splot_recon::PlanePool>,
    tile_records: Mutex<FrameFilterRecordCapacities>,
    /// The small row-list capacities any spent superblock unit reached.
    row_capacities: Mutex<RowCapacities>,
    /// One walk's temporal state for the whole decode: the inter and TIP output
    /// walks take turns, so a copy per walk or lane is memory nobody reads.
    walk_temporal: Mutex<Option<WalkTemporal>>,
}

/// The capacities of a superblock unit's small growable row lists.
pub(crate) type RowCapacities = [usize; 3];

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

    /// The small row-list capacities any unit's buffers reached.
    pub(crate) fn row_capacities(&self) -> RowCapacities {
        *self.row_capacities.lock()
    }

    /// Notes the small row-list capacities one spent unit reached, so every
    /// unit of every frame in flight sizes its lists once instead of growing
    /// them apart. These lists are a few kilobytes, so one size fits all.
    pub(crate) fn note_row_capacities(&self, reached: RowCapacities) {
        for (held, reached) in self.row_capacities.lock().iter_mut().zip(reached) {
            *held = (*held).max(reached);
        }
    }

    /// Lends the decode's walk temporal state to one walk, or a new one when
    /// another walk holds it or there are no decode buffers.
    pub(crate) fn lend_temporal(buffers: Option<&Self>) -> TemporalLease<'_> {
        let (temporal, records) = buffers
            .and_then(|buffers| buffers.walk_temporal.lock().take())
            .unwrap_or_else(|| (TemporalMvContext::empty(), Vec::new()));
        TemporalLease {
            buffers,
            temporal,
            records,
        }
    }
}

/// The temporal state lent to one walk, given back however the walk ends.
pub(crate) struct TemporalLease<'a> {
    buffers: Option<&'a DecodeBuffers>,
    pub(crate) temporal: TemporalMvContext,
    pub(crate) records: Vec<TemporalMotionBlock>,
}

impl Drop for TemporalLease<'_> {
    fn drop(&mut self) {
        if let Some(buffers) = self.buffers {
            let temporal = core::mem::replace(&mut self.temporal, TemporalMvContext::empty());
            *buffers.walk_temporal.lock() = Some((temporal, core::mem::take(&mut self.records)));
        }
    }
}
