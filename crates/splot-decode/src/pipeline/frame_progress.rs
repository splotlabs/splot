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
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, RwLock, RwLockReadGuard, TryLockError};

use splot_parallel::{Condition, WatermarkCell};
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

/// One finished stripe's copy into the filtered workspace.
pub(crate) type StripeCopy<T> =
    Box<dyn FnOnce(&mut CurrentFrameWorkspace<T>) -> Result<()> + Send + 'static>;

/// Stripe copies that found the workspace busy, and the first one that failed.
struct PendingStripes<T: ReconSample> {
    queued: Vec<(usize, StripeCopy<T>)>,
    failed: Option<DecodeError>,
}

/// One pending frame's filtered workspace and its published-row watermark.
pub(crate) struct FrameProgress<T: ReconSample> {
    workspace: RwLock<Option<CurrentFrameWorkspace<T>>>,
    layout: OnceLock<Mutex<ProgressLayout>>,
    pending: Mutex<PendingStripes<T>>,
    has_pending: AtomicBool,
    published_luma_rows: WatermarkCell,
    luma_height: usize,
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
            pending: Mutex::new(PendingStripes {
                queued: Vec::new(),
                failed: None,
            }),
            has_pending: AtomicBool::new(false),
            published_luma_rows: WatermarkCell::new(),
            luma_height: info.coded_luma_size().height(),
            subsampling_y: usize::from(info.pixel_format().subsampling_y()),
        })
    }

    /// Publishes the terminal watermark of a filter phase that ended.
    ///
    /// `filtered` publishes the whole frame height, which every row threshold
    /// satisfies with rows that are genuinely final; a phase that failed
    /// publishes [`WatermarkCell::FAILED`] instead, so a consumer waiting on a
    /// row it will never get is released and fails closed on the settled slot.
    pub(crate) fn publish_terminal(&self, filtered: bool) {
        self.published_luma_rows.publish(if filtered {
            self.luma_height
        } else {
            WatermarkCell::FAILED
        });
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
        drop(layout);
        self.published_luma_rows.publish(rows);
    }

    /// The number of luma rows from the frame top whose samples are final.
    ///
    /// The watermark also carries the terminal values a finished or failed
    /// filter phase publishes. A failed phase publishes
    /// [`WatermarkCell::FAILED`], which admits every waiter but names no
    /// readable row, so it reports zero rather than clamping to the frame
    /// height: the rows a failed phase never wrote must fail closed, and the
    /// waiters it released are admitted by the slot settling as failed.
    pub(crate) fn published_luma_rows(&self) -> usize {
        let published = self.published_luma_rows.current();
        if published == WatermarkCell::FAILED {
            return 0;
        }
        published.min(self.luma_height)
    }

    /// Returns the scheduler condition that admits a reader once `rows` final
    /// luma rows have been published.
    pub(crate) fn row_condition(&self, rows: usize) -> Condition<'_> {
        Condition::Watermark(&self.published_luma_rows, rows)
    }

    /// Borrows the published prefix of the frame's filtered samples.
    ///
    /// Returns `None` once the filter phase has taken the workspace to freeze
    /// it; the caller then waits for the slot, which is about to settle. A
    /// phase that failed publishes no readable row, so it also returns `None`
    /// and the caller reads the settled failure instead of unfiltered samples.
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
            progress: self,
            workspace: Some(workspace),
            luma_rows,
            chroma_rows: rows >> self.subsampling_y,
        })
    }

    /// Queues one finished stripe's copy and runs whatever the workspace will
    /// take right now.
    ///
    /// A stripe never waits for the exclusive lock: the lock's other users are
    /// the blocks of the next frame reading this one's published prefix, so a
    /// waiting writer would both stall its own worker and, under a
    /// writer-preferring lock, hold up every reader that arrives behind it.
    /// [`Self::drain_pending`] is what keeps a queued stripe from waiting for
    /// the next one — every reader runs it as it releases the prefix, which is
    /// exactly when the lock a busy stripe lost becomes free again.
    ///
    /// # Errors
    ///
    /// Returns the first diagnostic a queued copy failed with, on whichever
    /// thread reaches it first.
    pub(crate) fn publish_stripe(&self, stripe: usize, copy: StripeCopy<T>) -> Result<()> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.queued.push((stripe, copy));
        self.has_pending.store(true, Ordering::Release);
        drop(pending);
        self.drain_pending();
        self.take_failure()
    }

    /// Copies every queued stripe into the workspace when its exclusive lock is
    /// free, then advances the watermark over what landed.
    ///
    /// Reading the prefix is what makes the lock busy, so a reader calls this as
    /// it releases its borrow. Every attempt is a `try_write`: a reader may hold
    /// a second pending frame's prefix while it runs, and queueing here would
    /// let two readers holding each other's prefixes deadlock.
    pub(crate) fn drain_pending(&self) {
        if !self.has_pending.load(Ordering::Acquire) {
            return;
        }
        let mut guard = match self.workspace.try_write() {
            Ok(workspace) => workspace,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        let landed = self.copy_queued(guard.as_mut());
        drop(guard);
        for stripe in landed {
            self.publish(stripe);
        }
    }

    /// Copies every queued stripe, blocking for the workspace.
    ///
    /// The filter phase drains this way once its stripes have all run, which is
    /// what makes every stripe's samples present before the freeze even when
    /// the workspace was busy each time a stripe finished. Blocking is safe
    /// only here: the phase is over, so no further stripe can queue behind this
    /// writer.
    ///
    /// # Errors
    ///
    /// Returns the first diagnostic a queued copy failed with.
    pub(crate) fn drain_pending_blocking(&self) -> Result<()> {
        let mut guard = self
            .workspace
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        let landed = self.copy_queued(guard.as_mut());
        drop(guard);
        for stripe in landed {
            self.publish(stripe);
        }
        self.take_failure()
    }

    /// Copies the queued batch into `workspace`, returning the stripes that
    /// landed whole.
    ///
    /// The queue is emptied under the exclusive lock the copies need, so a
    /// stripe is taken by exactly one drain and lands exactly once. A copy that
    /// fails abandons the rest of the batch and records its diagnostic, so no
    /// stripe behind a failure is ever reported as published.
    fn copy_queued(&self, mut workspace: Option<&mut CurrentFrameWorkspace<T>>) -> Vec<usize> {
        let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::FilterStripePublish);
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let batch = core::mem::take(&mut pending.queued);
        self.has_pending.store(false, Ordering::Release);
        drop(pending);
        let mut landed = Vec::with_capacity(batch.len());
        for (stripe, copy) in batch {
            let outcome = match workspace {
                Some(ref mut workspace) => copy(workspace),
                None => Err(taken_workspace()),
            };
            match outcome {
                Ok(()) => landed.push(stripe),
                Err(error) => {
                    self.record_failure(error);
                    break;
                }
            }
        }
        landed
    }

    fn record_failure(&self, error: DecodeError) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.failed.get_or_insert(error);
    }

    fn take_failure(&self) -> Result<()> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.failed.take().map_or(Ok(()), Err)
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
///
/// Dropping the borrow is what frees the exclusive lock a finished stripe needs,
/// so the drop runs [`FrameProgress::drain_pending`] once the borrow is gone: a
/// stripe that lost the lock to this reader is published by the reader that took
/// it rather than waiting for the next stripe to finish.
pub(crate) struct PublishedFrame<'a, T: ReconSample> {
    progress: &'a FrameProgress<T>,
    workspace: Option<RwLockReadGuard<'a, Option<CurrentFrameWorkspace<T>>>>,
    luma_rows: NonZeroUsize,
    chroma_rows: usize,
}

impl<T: ReconSample> Drop for PublishedFrame<'_, T> {
    fn drop(&mut self) {
        self.workspace = None;
        self.progress.drain_pending();
    }
}

impl<T: ReconSample> PublishedFrame<'_, T> {
    /// Borrows the workspace the published rows live in.
    ///
    /// # Errors
    ///
    /// Returns an internal diagnostic when the workspace has been taken, which
    /// [`FrameProgress::read`] already rules out for a live borrow.
    pub(crate) fn workspace(&self) -> Result<&CurrentFrameWorkspace<T>> {
        self.workspace
            .as_ref()
            .and_then(|workspace| workspace.as_ref())
            .ok_or_else(taken_workspace)
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
