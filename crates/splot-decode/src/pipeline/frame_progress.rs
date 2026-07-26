// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Row-granular publication of one pending frame's filtered samples.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.
//!
//! A pipelined frame's § 7.2 filter phase writes its output stripe by stripe
//! into one filtered workspace, then freezes that workspace into the decoded
//! frame its slot publishes. Until the freeze, the workspace already holds the
//! final samples of every stripe that has landed, and
//! [`CurrentFramePlane::freeze`](splot_recon::CurrentFramePlane) moves that same
//! storage into the frozen plane unchanged, so a consumer that reads only rows
//! a published stripe covers reads exactly the bytes the frozen frame will
//! report.
//!
//! [`FrameProgress`] owns that workspace for the pipelined path and tracks how
//! many luma rows from the top are final. Stripes complete out of order, so the
//! watermark advances over the contiguous published prefix only. Reads take the
//! shared lock and are refused past the watermark, so a consumer can never
//! observe a row the filter chain has not written.

use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, RwLock, RwLockReadGuard};

use splot_recon::{CurrentFrameWorkspace, DecodedFrameInfo, ReconSample};

use crate::error::{DecodeError, Result};
use crate::pipeline::unsupported;

/// The stripe geometry one frame's filter phase publishes through.
struct ProgressLayout {
    /// Each stripe's exclusive luma row end, in stripe order.
    stripe_ends: Vec<usize>,
    /// Whether each stripe has landed, indexed as `stripe_ends`.
    landed: Vec<bool>,
    /// The next stripe the contiguous prefix is waiting for.
    prefix: usize,
}

/// One pending frame's filtered workspace and its published-row watermark.
pub(crate) struct FrameProgress<T: ReconSample> {
    workspace: RwLock<Option<CurrentFrameWorkspace<T>>>,
    layout: OnceLock<Mutex<ProgressLayout>>,
    published_luma_rows: AtomicUsize,
    subsampling_y: usize,
}

impl<T: ReconSample> FrameProgress<T> {
    /// Opens the filtered workspace one pending frame's filter phase publishes
    /// into, before that phase is handed to a worker.
    ///
    /// # Errors
    ///
    /// Returns the workspace allocation's own diagnostic.
    pub(crate) fn new(info: DecodedFrameInfo) -> Result<Self> {
        let workspace = CurrentFrameWorkspace::new(info, T::default())?;
        Ok(Self {
            workspace: RwLock::new(Some(workspace)),
            layout: OnceLock::new(),
            published_luma_rows: AtomicUsize::new(0),
            subsampling_y: usize::from(info.pixel_format().subsampling_y()),
        })
    }

    /// Installs the stripe geometry the filter phase will publish through.
    ///
    /// The ranges must ascend, be contiguous, and start at the frame top, since
    /// the watermark is the end of the contiguous published prefix. A geometry
    /// that does not satisfy that leaves the frame unpublished rather than
    /// letting a consumer read an unwritten row.
    pub(crate) fn begin(&self, ranges: &[(usize, usize)]) -> bool {
        let mut next = 0usize;
        for &(start, end) in ranges {
            if start != next || end <= start {
                return false;
            }
            next = end;
        }
        let layout = ProgressLayout {
            stripe_ends: ranges.iter().map(|&(_, end)| end).collect(),
            landed: vec![false; ranges.len()],
            prefix: 0,
        };
        self.layout.set(Mutex::new(layout)).is_ok()
    }

    /// Records that one stripe's samples have landed in the workspace and
    /// advances the watermark over the contiguous published prefix.
    pub(crate) fn publish(&self, stripe: usize) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let mut layout = layout.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(landed) = layout.landed.get_mut(stripe) else {
            return;
        };
        *landed = true;
        while layout
            .landed
            .get(layout.prefix)
            .copied()
            .unwrap_or_default()
        {
            layout.prefix += 1;
        }
        let rows = layout
            .prefix
            .checked_sub(1)
            .and_then(|last| layout.stripe_ends.get(last).copied())
            .unwrap_or_default();
        self.published_luma_rows.store(rows, Ordering::Release);
    }

    /// The number of luma rows from the frame top whose samples are final.
    pub(crate) fn published_luma_rows(&self) -> usize {
        self.published_luma_rows.load(Ordering::Acquire)
    }

    /// Borrows the published prefix of the frame's filtered samples.
    ///
    /// Returns `None` once the filter phase has taken the workspace to freeze
    /// it; the caller then waits for the slot, which is about to settle.
    pub(crate) fn read(&self) -> Option<PublishedFrame<'_, T>> {
        let rows = self.published_luma_rows();
        let luma_rows = NonZeroUsize::new(rows)?;
        let workspace = self
            .workspace
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        if workspace.is_none() {
            return None;
        }
        Some(PublishedFrame {
            workspace,
            luma_rows,
            chroma_rows: rows >> self.subsampling_y,
        })
    }

    /// Runs `publish` against the filtered workspace under the exclusive lock.
    ///
    /// # Errors
    ///
    /// Returns `publish`'s own diagnostic, or an internal diagnostic when the
    /// workspace has already been taken.
    pub(crate) fn with_workspace_mut<R>(
        &self,
        publish: impl FnOnce(&mut CurrentFrameWorkspace<T>) -> Result<R>,
    ) -> Result<R> {
        let mut workspace = self
            .workspace
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let workspace = workspace.as_mut().ok_or_else(taken_workspace)?;
        publish(workspace)
    }

    /// Freezes the filtered workspace and publishes the frozen frame, both
    /// under the exclusive lock.
    ///
    /// The freeze is the one moment the published prefix stops being readable,
    /// so `publish` — which settles the frame's slot — runs before the lock is
    /// released. A reader that arrives during the freeze waits for the lock and
    /// then finds the slot settled, instead of finding neither storage.
    ///
    /// # Errors
    ///
    /// Returns the freeze's own diagnostic, or an internal diagnostic when the
    /// workspace has already been taken.
    pub(crate) fn freeze_workspace<R>(
        &self,
        publish: impl FnOnce(splot_recon::DecodedFrame<T>) -> R,
    ) -> Result<R> {
        let mut guard = self
            .workspace
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let workspace = guard.take().ok_or_else(taken_workspace)?;
        Ok(publish(workspace.freeze()?))
    }
}

/// A shared borrow of one frame's published filtered prefix.
pub(crate) struct PublishedFrame<'a, T: ReconSample> {
    workspace: RwLockReadGuard<'a, Option<CurrentFrameWorkspace<T>>>,
    luma_rows: NonZeroUsize,
    chroma_rows: usize,
}

impl<T: ReconSample> PublishedFrame<'_, T> {
    /// Borrows the workspace the published rows live in.
    ///
    /// # Errors
    ///
    /// Returns an internal diagnostic when the workspace has been taken, which
    /// [`FrameProgress::read`] already rules out for a live borrow.
    pub(crate) fn workspace(&self) -> Result<&CurrentFrameWorkspace<T>> {
        self.workspace.as_ref().ok_or_else(taken_workspace)
    }

    /// The number of final luma rows, which is never zero.
    pub(crate) const fn luma_rows(&self) -> usize {
        self.luma_rows.get()
    }

    /// The number of final chroma rows.
    ///
    /// A chroma row is final once every luma row it subsamples is, so the count
    /// truncates rather than rounds: § 6.4.1 pairs chroma row `r` with luma rows
    /// `r << subsampling_y ..= (r << subsampling_y) + subsampling_y`.
    pub(crate) const fn chroma_rows(&self) -> usize {
        self.chroma_rows
    }
}

fn taken_workspace() -> DecodeError {
    unsupported(
        "decoded_frame_progress_taken",
        None,
        "internal invariant violation: a pending frame's filtered workspace was read after the freeze took it",
    )
}

#[cfg(test)]
#[path = "frame_progress_tests.rs"]
mod tests;
