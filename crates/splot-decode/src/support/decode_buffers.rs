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
use crate::prediction::inter::ReconRowCapacities;

/// Row buffer sets one decode keeps between units and frames.
const MAX_RETAINED_ROW_BUFFERS: usize = 256;

/// The storage one decode's finished work leaves for the work behind it.
#[derive(Default)]
pub(crate) struct DecodeBuffers {
    planes: Arc<splot_recon::PlanePool>,
    rows: Mutex<RetainedRows>,
}

/// The row buffer sets a decode is holding, and the sizes they reached.
#[derive(Default)]
struct RetainedRows {
    spares: Vec<ReconRowBuffers>,
    reached: ReconRowCapacities,
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

    /// Takes a retained row buffer set, or a fresh one already sized for the
    /// rows this decode has seen.
    ///
    /// Every set outstanding at once is a set this decode is already paying
    /// for, so a miss is not worth retaining more of them against -- but a set
    /// built empty climbs the growth ladder for all nine of its lists, which
    /// the sizes a spent set left behind are enough to skip.
    pub(crate) fn take_rows(&self) -> ReconRowBuffers {
        let reached = {
            let mut rows = self.rows.lock();
            if let Some(buffers) = rows.spares.pop() {
                return buffers;
            }
            rows.reached
        };
        ReconRowBuffers::with_capacities(reached)
    }

    /// Returns a spent row buffer set, whose vectors are already cleared.
    pub(crate) fn retain_rows(&self, buffers: ReconRowBuffers) {
        let mut rows = self.rows.lock();
        rows.reached.cover(buffers.capacities());
        if rows.spares.len() < MAX_RETAINED_ROW_BUFFERS {
            rows.spares.push(buffers);
        }
    }
}
