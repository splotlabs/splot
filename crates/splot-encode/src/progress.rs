// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! How far a running encode has come, read from any thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The count of one whole frame.
const FRAME: u64 = 1 << 20;

/// Reads the progress of an [`Encoder`](crate::Encoder) from any thread;
/// clones share one counter.
///
/// [`Encoder::progress`](crate::Encoder::progress) returns the encoder's
/// handle. The encoder only writes the counter, so reading it changes neither
/// the output nor the statistics.
#[derive(Debug, Clone, Default)]
pub struct Progress(Arc<AtomicU64>);

impl Progress {
    /// Frames coded so far, in coding order. A frame being coded counts by the
    /// fraction of its superblocks searched, so the value rises while
    /// [`Encoder::receive_packet`](crate::Encoder::receive_packet) runs; the
    /// lookahead and the loop filters of a frame do not move it. It is a whole
    /// number whenever no frame is being coded, and steps back by at most one
    /// frame when rate control codes a frame again.
    #[must_use]
    pub fn frames(&self) -> f64 {
        self.0.load(Ordering::Relaxed) as f64 / FRAME as f64
    }

    /// Counts one searched superblock of a frame of `superblocks`.
    pub(crate) fn superblock(&self, superblocks: usize) {
        self.0
            .fetch_add(FRAME / superblocks.max(1) as u64, Ordering::Relaxed);
    }

    /// Sets the count to `frames` whole frames, once no frame is being coded.
    pub(crate) fn set_frames(&self, frames: u64) {
        self.0.store(frames * FRAME, Ordering::Relaxed);
    }

    /// Takes back the frame that is coded again after `committed` frames.
    pub(crate) fn rewind_frame(&self, committed: u64) {
        let _ = self
            .0
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                Some(count.saturating_sub(FRAME).max(committed * FRAME))
            });
    }
}
