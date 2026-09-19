// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared frame-plane storage and filter-record capacity hints for one decode.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::filters::wienerns_lr::FrameFilterRecordCapacities;
/// The storage one decode's finished work leaves for the work behind it.
#[derive(Default)]
pub(crate) struct DecodeBuffers {
    planes: Arc<splot_recon::PlanePool>,
    tile_records: Mutex<FrameFilterRecordCapacities>,
}

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
}
