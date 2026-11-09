// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared frame-plane storage and filter-record capacity hints for one decode.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::filters::wienerns_lr::FrameFilterRecordCapacities;
use crate::prediction::inter::{ReconRowBuffers, TemporalMotionBlock, TemporalMvContext};

/// A frame walk's temporal context and motion records.
pub(crate) type WalkTemporal = (TemporalMvContext, Vec<TemporalMotionBlock>);

/// The storage one decode's finished work leaves for the work behind it.
#[derive(Default)]
pub(crate) struct DecodeBuffers {
    planes: Arc<splot_recon::PlanePool>,
    tile_records: Mutex<FrameFilterRecordCapacities>,
    /// Sixteen times the running mean of each large row list's length.
    unit_lengths: Mutex<UnitLengths>,
    /// Spent superblock units' row buffers, lent to the next unit parsed.
    units: Mutex<Vec<ReconRowBuffers>>,
    /// One walk's temporal state for the whole decode: the inter and TIP output
    /// walks take turns, so a copy per walk or lane is memory nobody reads.
    walk_temporal: Mutex<Option<WalkTemporal>>,
}

/// The lengths of a superblock unit's large row lists.
pub(crate) type UnitLengths = [usize; 11];

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

    /// A spent unit's row buffers, if any is free.
    pub(crate) fn take_unit_buffers(&self) -> Option<ReconRowBuffers> {
        self.units.lock().pop()
    }

    /// Takes back a committed unit's row buffers.
    pub(crate) fn give_unit_buffers(&self, buffers: ReconRowBuffers) {
        self.units.lock().push(buffers);
    }

    /// The running mean of each large row list's length.
    pub(crate) fn unit_lengths(&self) -> UnitLengths {
        self.unit_lengths.lock().map(|mean| mean / 16)
    }

    /// Folds one spent unit's list lengths into the decode's running mean,
    /// over about the last sixteen units, and returns that mean.
    pub(crate) fn note_unit_lengths(&self, used: UnitLengths) -> UnitLengths {
        let mut means = self.unit_lengths.lock();
        for (mean, used) in means.iter_mut().zip(used) {
            *mean = (*mean - *mean / 16).saturating_add(used);
        }
        means.map(|mean| mean / 16)
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

/// Sizes an emptied unit list for the next unit: to twice the decode's
/// typical length for it when its capacity is under the typical length, or
/// over three times it and 8 KiB. Pooled buffers otherwise each climb to the
/// largest unit any of them has held. The list is empty, so a fresh
/// allocation replaces it without a copy.
pub(crate) fn fit_list<E>(list: &mut Vec<E>, typical: usize) {
    debug_assert!(list.is_empty());
    let keep = typical.saturating_mul(2);
    let floor = (8 << 10) / core::mem::size_of::<E>().max(1);
    if list.capacity() < typical || list.capacity() > keep.saturating_add(keep / 2).max(floor) {
        *list = Vec::with_capacity(keep);
    }
}
