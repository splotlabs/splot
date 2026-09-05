// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The reusable frame- and row-sized storage of one decode.
//!
//! Both kinds outlive the owner that retires them: a workspace's last holder is
//! whichever filter job finishes last, and a row buffer set is parsed on one
//! worker and replayed on another. Neither can be handed straight to its
//! successor, so both are kept here, and both are kept *per decode* rather than
//! per process so concurrent decodes never draw on each other's storage.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::prediction::inter::ReconRowBuffers;

/// Row buffer sets one decode keeps between units and frames.
const MAX_RETAINED_ROW_BUFFERS: usize = 256;

/// The storage one decode's finished work leaves for the work behind it.
#[derive(Default)]
pub(crate) struct DecodeBuffers {
    planes: Arc<splot_recon::PlanePool>,
    rows: Mutex<Vec<ReconRowBuffers>>,
}

impl core::fmt::Debug for DecodeBuffers {
    /// Names the storage without locking it or printing a frame of samples.
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("DecodeBuffers")
    }
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

    /// Takes a retained row buffer set, or a fresh one.
    pub(crate) fn take_rows(&self) -> ReconRowBuffers {
        self.rows.lock().pop().unwrap_or_default()
    }

    /// Returns a spent row buffer set, whose vectors are already cleared.
    pub(crate) fn retain_rows(&self, buffers: ReconRowBuffers) {
        let mut rows = self.rows.lock();
        if rows.len() < MAX_RETAINED_ROW_BUFFERS {
            rows.push(buffers);
        }
    }
}
