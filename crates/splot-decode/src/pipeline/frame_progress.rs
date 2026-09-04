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

#![allow(
    unsafe_code,
    reason = "disjoint filter stripes write directly into one canonical frame allocation"
)]

use core::cell::UnsafeCell;
use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::slice;
use std::sync::{Arc, OnceLock};

use splot_parallel::{CompletionCell, Condition, WatermarkCell};
use splot_recon::{CurrentFrameWorkspace, DecodedFrameInfo, PlaneId, PlaneRect, ReconSample};

use crate::error::{DecodeError, Result};
use crate::pipeline::unsupported;
use parking_lot::{Mutex, RwLock, RwLockReadGuard};

/// The stripe geometry one frame's filter phase publishes through.
struct ProgressLayout {
    /// Each stripe's exclusive luma row end, in stripe order.
    stripe_ends: Vec<usize>,
    /// Whether each stripe has landed, indexed as `stripe_ends`.
    landed: Vec<bool>,
    leased: Vec<bool>,
    /// Live holders of each stripe's direct lease: the lease itself plus one
    /// per plane target. The lease is released when this reaches zero, which is
    /// what a per-lease reference count used to track.
    direct_holds: Vec<u32>,
    direct_aligned: bool,
    freezing: bool,
    /// The next stripe the contiguous prefix is waiting for.
    prefix: usize,
}

#[derive(Clone, Copy)]
enum DirectPlaneSamples {
    U8(NonNull<u8>),
    U16(NonNull<u16>),
}

impl DirectPlaneSamples {
    fn offset(self, offset: usize) -> Option<Self> {
        match self {
            Self::U8(samples) => Some(Self::U8(NonNull::new(
                samples.as_ptr().wrapping_add(offset),
            )?)),
            Self::U16(samples) => Some(Self::U16(NonNull::new(
                samples.as_ptr().wrapping_add(offset),
            )?)),
        }
    }
}

#[derive(Clone, Copy)]
struct PlaneStorage<T> {
    samples: NonNull<T>,
    direct_samples: Option<DirectPlaneSamples>,
    len: usize,
    stride: usize,
    height: usize,
    visible: PlaneRect,
}

struct DirectWorkspace<T: ReconSample> {
    workspace: UnsafeCell<CurrentFrameWorkspace<T>>,
    info: DecodedFrameInfo,
    planes: [Option<PlaneStorage<T>>; 3],
}

// SAFETY: tracked disjoint lends exclude published prefixes and terminal freeze.
unsafe impl<T: ReconSample> Send for DirectWorkspace<T> {}
// SAFETY: shared access exposes only immutable geometry or published rows.
unsafe impl<T: ReconSample> Sync for DirectWorkspace<T> {}

impl<T: ReconSample> DirectWorkspace<T> {
    fn new(mut workspace: CurrentFrameWorkspace<T>) -> Self {
        let info = workspace.info();
        let mut planes = [None, None, None];
        {
            let mut frame = workspace.as_frame_mut();
            for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
                let Some(view) = frame.plane_mut(plane) else {
                    continue;
                };
                let stride = view.stride_samples();
                let visible = view.visible_rect();
                let samples = view.samples_mut();
                let len = samples.len();
                let (samples, direct_samples) = if let Some(samples) = T::u16_slice_mut(samples) {
                    let Some(samples) = NonNull::new(samples.as_mut_ptr()) else {
                        continue;
                    };
                    (samples.cast(), Some(DirectPlaneSamples::U16(samples)))
                } else if let Some(samples) = T::u8_slice_mut(samples) {
                    let Some(samples) = NonNull::new(samples.as_mut_ptr()) else {
                        continue;
                    };
                    (samples.cast(), Some(DirectPlaneSamples::U8(samples)))
                } else {
                    let Some(samples) = NonNull::new(samples.as_mut_ptr()) else {
                        continue;
                    };
                    (samples, None)
                };
                planes[plane.index()] = Some(PlaneStorage {
                    samples,
                    direct_samples,
                    len,
                    stride,
                    height: len / stride,
                    visible,
                });
            }
        }
        Self {
            workspace: UnsafeCell::new(workspace),
            info,
            planes,
        }
    }

    fn direct_region(&self, plane: PlaneId, start: usize, end: usize) -> Option<DirectPlaneRegion> {
        let storage = self.planes[plane.index()]?;
        if start >= end || end > storage.height {
            return None;
        }
        let start_sample = start.checked_mul(storage.stride)?;
        let len = (end - start).checked_mul(storage.stride)?;
        if start_sample.checked_add(len)? > storage.len {
            return None;
        }
        let samples = storage.direct_samples?.offset(start_sample)?;
        Some(DirectPlaneRegion {
            samples,
            len,
            width: storage.stride,
            frame_height: storage.height,
            origin_y: start,
        })
    }

    fn published_plane(&self, plane: PlaneId, rows: usize) -> Option<PublishedPlane<'_, T>> {
        let storage = self.planes[plane.index()]?;
        let rows = rows.min(storage.height);
        let len = rows.checked_mul(storage.stride)?.min(storage.len);
        let samples = unsafe {
            // SAFETY: release/acquire publishes this prefix before it is read.
            slice::from_raw_parts(storage.samples.as_ptr(), len)
        };
        Some(PublishedPlane {
            samples,
            stride: storage.stride,
            visible: storage.visible,
        })
    }

    fn into_workspace(self) -> CurrentFrameWorkspace<T> {
        self.workspace.into_inner()
    }
}

struct DirectPlaneRegion {
    samples: DirectPlaneSamples,
    len: usize,
    width: usize,
    frame_height: usize,
    origin_y: usize,
}

pub(crate) struct DirectPlaneTarget {
    region: DirectPlaneRegion,
    progress: Arc<dyn DirectLeaseRelease>,
    stripe: usize,
}

/// SAFETY: moving this unique disjoint-band capability transfers ownership.
unsafe impl Send for DirectPlaneTarget {}

impl DirectPlaneTarget {
    pub(crate) const fn width(&self) -> usize {
        self.region.width
    }

    pub(crate) const fn frame_height(&self) -> usize {
        self.region.frame_height
    }

    pub(crate) const fn origin_y(&self) -> usize {
        self.region.origin_y
    }

    pub(crate) const fn len(&self) -> usize {
        self.region.len
    }

    pub(crate) fn end_y(&self) -> Option<usize> {
        self.region
            .origin_y
            .checked_add(self.region.len.checked_div(self.region.width)?)
    }

    pub(crate) const fn is_u16(&self) -> bool {
        matches!(self.region.samples, DirectPlaneSamples::U16(_))
    }

    #[inline]
    /// The target uniquely owns this non-overlapping stripe region.
    pub(crate) fn u8_samples_mut(&mut self) -> Option<&mut [u8]> {
        let DirectPlaneSamples::U8(samples) = self.region.samples else {
            return None;
        }; // SAFETY: this target owns a checked non-overlapping stripe region.
        Some(unsafe { slice::from_raw_parts_mut(samples.as_ptr(), self.region.len) })
    }

    #[inline]
    pub(crate) fn u16_samples_mut(&mut self) -> Option<&mut [u16]> {
        let DirectPlaneSamples::U16(samples) = self.region.samples else {
            return None;
        };
        Some(unsafe {
            // SAFETY: the layout lends each non-overlapping stripe once.
            slice::from_raw_parts_mut(samples.as_ptr(), self.region.len)
        })
    }
}

impl Drop for DirectPlaneTarget {
    fn drop(&mut self) {
        self.progress.release_hold(self.stripe);
    }
}

impl DirectPlaneRegion {
    fn into_target(
        self,
        progress: Arc<dyn DirectLeaseRelease>,
        stripe: usize,
    ) -> DirectPlaneTarget {
        DirectPlaneTarget {
            region: self,
            progress,
            stripe,
        }
    }
}

pub(crate) struct DirectStripeTarget {
    planes: [Option<DirectPlaneTarget>; 3],
}

impl DirectStripeTarget {
    pub(crate) fn get(&self, plane: PlaneId) -> Option<&DirectPlaneTarget> {
        self.planes[plane.index()].as_ref()
    }

    pub(crate) fn take(&mut self, plane: PlaneId) -> Option<DirectPlaneTarget> {
        self.planes[plane.index()].take()
    }

    #[cfg(test)]
    pub(crate) fn shorten_for_test(&mut self, plane: PlaneId) {
        if let Some(target) = self.planes[plane.index()].as_mut() {
            target.region.len = target.region.len.saturating_sub(1);
        }
    }

    pub(crate) fn split(mut self, second: [bool; 3]) -> (Self, Self) {
        let mut first_planes = [None, None, None];
        let mut second_planes = [None, None, None];
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            let target = self.take(plane);
            if second[plane.index()] {
                second_planes[plane.index()] = target;
            } else {
                first_planes[plane.index()] = target;
            }
        }
        (
            Self {
                planes: first_planes,
            },
            Self {
                planes: second_planes,
            },
        )
    }
}

pub(crate) struct DirectStripeLease<T: ReconSample> {
    progress: Arc<FrameProgress<T>>,
    stripe: usize,
    target: Option<DirectStripeTarget>,
}

impl<T: ReconSample> Drop for DirectStripeLease<T> {
    fn drop(&mut self) {
        self.progress.release_direct_hold(self.stripe);
    }
}

impl<T: ReconSample> DirectStripeLease<T> {
    pub(crate) fn take_target(&mut self) -> Option<DirectStripeTarget> {
        self.target.take()
    }

    pub(crate) fn submit(mut self) -> bool {
        drop(self.target.take());
        if self.progress.direct_holds(self.stripe) != 1 {
            return false;
        }
        self.progress.publish_direct(self.stripe);
        true
    }
}

trait DirectLeaseRelease: Send + Sync {
    fn release_hold(&self, stripe: usize);
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PublishedPlane<'a, T> {
    pub(crate) samples: &'a [T],
    pub(crate) stride: usize,
    pub(crate) visible: PlaneRect,
}

#[derive(Clone, Copy)]
pub(crate) struct PublishedStorage<'a, T: ReconSample> {
    workspace: &'a DirectWorkspace<T>,
}

impl<T: ReconSample> core::fmt::Debug for PublishedStorage<'_, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PublishedStorage")
    }
}

impl<'a, T: ReconSample> PublishedStorage<'a, T> {
    pub(crate) const fn info(self) -> DecodedFrameInfo {
        self.workspace.info
    }

    pub(crate) fn plane(self, plane: PlaneId, rows: usize) -> Option<PublishedPlane<'a, T>> {
        self.workspace.published_plane(plane, rows)
    }
}

/// One pending frame's filtered workspace and its published-row watermark.
pub(crate) struct FrameProgress<T: ReconSample> {
    workspace: RwLock<Option<DirectWorkspace<T>>>,
    layout: OnceLock<Mutex<ProgressLayout>>,
    published_luma_rows: WatermarkCell,
    terminal_published: CompletionCell<()>,
    luma_height: usize,
    subsampling_y: usize,
}

impl<T: ReconSample> Drop for FrameProgress<T> {
    fn drop(&mut self) {
        let Some(layout) = self.layout.get_mut() else {
            return;
        };
        let layout = layout.get_mut();
        crate::support::buffer_pool::recycle(&mut layout.stripe_ends);
        crate::support::buffer_pool::recycle(&mut layout.landed);
        crate::support::buffer_pool::recycle(&mut layout.leased);
        crate::support::buffer_pool::recycle(&mut layout.direct_holds);
    }
}

impl<T: ReconSample> DirectLeaseRelease for FrameProgress<T> {
    fn release_hold(&self, stripe: usize) {
        self.release_direct_hold(stripe);
    }
}

impl<T: ReconSample> FrameProgress<T> {
    /// Opens the filtered workspace one pending frame's filter phase publishes
    /// into, before that phase is handed to a worker.
    ///
    /// # Errors
    ///
    /// Returns the workspace allocation's own diagnostic.
    pub(crate) fn new(info: DecodedFrameInfo) -> Result<Self> {
        let workspace = DirectWorkspace::new(CurrentFrameWorkspace::new_recycled(info)?); // every row is published by a filter stripe before any consumer may read past the watermark
        Ok(Self {
            workspace: RwLock::new(Some(workspace)),
            layout: OnceLock::new(),
            published_luma_rows: WatermarkCell::new(),
            terminal_published: CompletionCell::new(),
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
        let _ = self.terminal_published.set(());
    }

    pub(crate) fn wait_terminal(&self) {
        let () = self.terminal_published.wait_with_pool_assist();
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
        let mut stripe_ends = crate::support::buffer_pool::take::<usize>(ranges.len());
        stripe_ends.extend(ranges.iter().map(|&(_, end)| end));
        let mut landed = crate::support::buffer_pool::take::<bool>(ranges.len());
        landed.resize(ranges.len(), false);
        let mut leased = crate::support::buffer_pool::take::<bool>(ranges.len());
        leased.resize(ranges.len(), false);
        let mut direct_holds = crate::support::buffer_pool::take::<u32>(ranges.len());
        direct_holds.resize(ranges.len(), 0);
        let layout = ProgressLayout {
            stripe_ends,
            landed,
            leased,
            direct_holds,
            direct_aligned: ranges
                .iter()
                .take(ranges.len().saturating_sub(1))
                .all(|&(_, end)| end.is_multiple_of(1 << self.subsampling_y)),
            freezing: false,
            prefix: 0,
        };
        self.layout.set(Mutex::new(layout)).is_ok()
    }

    pub(crate) fn direct_stripe(
        self: &std::sync::Arc<Self>,
        stripe: usize,
    ) -> Option<DirectStripeLease<T>> {
        let layout = self.layout.get()?;
        let mut layout = layout.lock();
        if layout.freezing
            || !layout.direct_aligned
            || layout.landed.get(stripe).copied()?
            || *layout.leased.get(stripe)?
        {
            return None;
        }
        let start = if stripe == 0 {
            0
        } else {
            *layout.stripe_ends.get(stripe - 1)?
        };
        let end = *layout.stripe_ends.get(stripe)?;
        let workspace_guard = self.workspace.read();
        let workspace = workspace_guard.as_ref()?;
        let chroma_start = start >> self.subsampling_y;
        let chroma_end = end.div_ceil(1 << self.subsampling_y);
        let y = workspace.direct_region(PlaneId::Y, start, end)?;
        let u = workspace.direct_region(PlaneId::U, chroma_start, chroma_end);
        let v = workspace.direct_region(PlaneId::V, chroma_start, chroma_end);
        if u.is_some() != v.is_some() {
            return None;
        }
        layout.leased[stripe] = true;
        let holds = 2 + u32::from(u.is_some()) + u32::from(v.is_some());
        if let Some(slot) = layout.direct_holds.get_mut(stripe) {
            *slot = holds;
        }
        drop(workspace_guard);
        drop(layout);
        let progress: Arc<dyn DirectLeaseRelease> = self.clone();
        let y = y.into_target(Arc::clone(&progress), stripe);
        let u = u.map(|region| region.into_target(Arc::clone(&progress), stripe));
        let v = v.map(|region| region.into_target(progress, stripe));
        Some(DirectStripeLease {
            progress: Arc::clone(self),
            stripe,
            target: Some(DirectStripeTarget {
                planes: [Some(y), u, v],
            }),
        })
    }

    /// Drops one holder of a stripe's direct lease, releasing it at zero.
    fn release_direct_hold(&self, stripe: usize) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let mut layout = layout.lock();
        let released = match layout.direct_holds.get_mut(stripe) {
            Some(holds) => {
                *holds = holds.saturating_sub(1);
                *holds == 0
            }
            None => return,
        };
        if released && let Some(leased) = layout.leased.get_mut(stripe) {
            *leased = false;
        }
    }

    /// Holders left on a stripe's direct lease.
    fn direct_holds(&self, stripe: usize) -> u32 {
        self.layout.get().map_or(0, |layout| {
            layout.lock().direct_holds.get(stripe).copied().unwrap_or(0)
        })
    }

    /// Records that one stripe's samples have landed in the workspace and
    /// advances the watermark over the contiguous published prefix.
    #[cfg(test)]
    pub(crate) fn publish(&self, stripe: usize) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let mut layout = layout.lock();
        let rows = complete_stripe(&mut layout, stripe);
        drop(layout);
        self.published_luma_rows.publish(rows);
    }

    fn publish_direct(&self, stripe: usize) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let mut layout = layout.lock();
        let Some(leased) = layout.leased.get_mut(stripe) else {
            return;
        };
        *leased = false;
        let rows = complete_stripe(&mut layout, stripe);
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
        Condition::watermark(&self.published_luma_rows, rows)
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
        let workspace = self.workspace.read();
        if workspace.is_none() {
            return None;
        }
        let chroma_rows = if rows == self.luma_height {
            rows.div_ceil(1 << self.subsampling_y)
        } else {
            rows >> self.subsampling_y
        };
        Some(PublishedFrame {
            workspace,
            luma_rows,
            chroma_rows,
        })
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
        if let Some(layout) = self.layout.get() {
            let mut layout = layout.lock();
            if layout.leased.iter().any(|&leased| leased) {
                return Err(live_direct_lease());
            }
            layout.freezing = true;
        }
        let mut guard = self.workspace.write();
        let workspace = guard.take().ok_or_else(taken_workspace)?;
        Ok(publish(workspace.into_workspace().freeze()?))
    }
}

fn complete_stripe(layout: &mut ProgressLayout, stripe: usize) -> usize {
    let Some(landed) = layout.landed.get_mut(stripe) else {
        return layout
            .prefix
            .checked_sub(1)
            .and_then(|last| layout.stripe_ends.get(last).copied())
            .unwrap_or_default();
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
    layout
        .prefix
        .checked_sub(1)
        .and_then(|last| layout.stripe_ends.get(last).copied())
        .unwrap_or_default()
}

/// A shared borrow of one frame's published filtered prefix.
pub(crate) struct PublishedFrame<'a, T: ReconSample> {
    workspace: RwLockReadGuard<'a, Option<DirectWorkspace<T>>>,
    luma_rows: NonZeroUsize,
    chroma_rows: usize,
}

impl<T: ReconSample> PublishedFrame<'_, T> {
    #[cfg(test)]
    pub(crate) fn plane(&self, plane: PlaneId) -> Result<Option<PublishedPlane<'_, T>>> {
        let workspace = self.workspace.as_ref().ok_or_else(taken_workspace)?;
        let rows = if plane == PlaneId::Y {
            self.luma_rows()
        } else {
            self.chroma_rows()
        };
        Ok(workspace.published_plane(plane, rows))
    }

    pub(crate) fn storage(&self) -> Result<PublishedStorage<'_, T>> {
        self.workspace
            .as_ref()
            .map(|workspace| PublishedStorage { workspace })
            .ok_or_else(taken_workspace)
    }

    /// The number of final luma rows, which is never zero.
    pub(crate) const fn luma_rows(&self) -> usize {
        self.luma_rows.get()
    }

    /// The number of final chroma rows.
    ///
    /// A chroma row is final once every in-frame luma row it subsamples is, so
    /// interior prefixes truncate while a complete odd-height frame includes
    /// its terminal chroma row.
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

fn live_direct_lease() -> DecodeError {
    unsupported(
        "decoded_frame_progress_live_direct_lease",
        None,
        "internal invariant violation: a pending frame froze while a filter stripe still owned its direct destination",
    )
}

#[cfg(test)]
#[path = "frame_progress_tests.rs"]
mod tests;
