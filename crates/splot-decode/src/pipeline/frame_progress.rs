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
//! watermark advances over the contiguous published prefix only. Reads take no
//! lock: a [`PublishedFrame`] borrows, through its slot, only rows below the
//! watermark it loaded, which no writer touches again and whose allocation the
//! slot keeps alive across the freeze.

#![allow(
    unsafe_code,
    reason = "disjoint filter stripes write directly into one canonical frame allocation"
)]

use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::slice;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use splot_parallel::{CompletionCell, WatermarkCell};
use splot_recon::{CurrentFrameWorkspace, DecodedFrameInfo, PlaneId, PlaneSize, ReconSample};

use super::inflight::{FrameSlotWriter, RefFrameSlot};
use crate::error::{DecodeError, Result};
use crate::pipeline::unsupported;
use parking_lot::Mutex;

/// One stripe's publication state.
#[derive(Clone, Copy, Default)]
struct StripeProgress {
    /// The stripe's exclusive luma row end.
    end: usize,
    /// Whether the stripe has landed.
    landed: bool,
    leased: bool,
    /// Live holders of the stripe's direct lease: the lease itself plus one per
    /// plane target. The lease is released when this reaches zero, which is
    /// what a per-lease reference count used to track.
    holds: u32,
}

struct ProgressLayout {
    /// Each stripe's publication state, in stripe order.
    stripes: Vec<StripeProgress>,
    direct_aligned: bool,
    freezing: bool,
    /// The next stripe the contiguous prefix is waiting for.
    prefix: usize,
}

#[derive(Clone, Copy)]
struct PlaneStorage<T> {
    samples: NonNull<T>,
    len: usize,
    stride: usize,
    height: usize,
    size: PlaneSize,
}

/// SAFETY: a plane table is dereferenced only through the row ownership rules
/// of `FrameProgress`: published prefixes, leased stripes, the frontier's rows.
unsafe impl<T: ReconSample> Send for PlaneStorage<T> {}
/// SAFETY: as for `Send`; a shared table copy creates no sample access itself.
unsafe impl<T: ReconSample> Sync for PlaneStorage<T> {}

/// The raw table of `workspace`'s planes, which stays valid while its sample
/// buffers live: neither the freeze nor a plane take moves their allocation.
fn plane_table<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
) -> crate::Result<[Option<PlaneStorage<T>>; 3]> {
    let mut planes = [None, None, None];
    let sizes = crate::filters::wienerns_lr::recon::plane_storage_sizes(workspace);
    let mut frame = workspace.as_frame_mut()?;
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let Some(view) = frame.plane_mut(plane) else {
            continue;
        };
        let stride = view.stride_samples();
        let Some(size) = sizes[plane.index()] else {
            continue;
        };
        let samples = view.samples_mut();
        let len = samples.len();
        let Some(samples) = NonNull::new(samples.as_mut_ptr()) else {
            continue;
        };
        planes[plane.index()] = Some(PlaneStorage {
            samples,
            len,
            stride,
            height: len / stride,
            size,
        });
    }
    Ok(planes)
}

struct DirectPlaneRegion<T> {
    samples: NonNull<T>,
    len: usize,
    width: usize,
    stride: usize,
    frame_height: usize,
    origin_y: usize,
}

pub(crate) struct DirectPlaneTarget<T> {
    region: DirectPlaneRegion<T>,
    progress: Arc<dyn DirectLeaseRelease>,
    stripe: usize,
    deblocked: bool,
}

/// SAFETY: moving this unique disjoint-band capability transfers ownership.
unsafe impl<T: Send> Send for DirectPlaneTarget<T> {}

impl<T> DirectPlaneTarget<T> {
    pub(crate) const fn width(&self) -> usize {
        self.region.width
    }

    pub(crate) const fn stride(&self) -> usize {
        self.region.stride
    }

    pub(crate) const fn frame_height(&self) -> usize {
        self.region.frame_height
    }

    pub(crate) const fn origin_y(&self) -> usize {
        self.region.origin_y
    }

    pub(crate) fn end_y(&self) -> Option<usize> {
        self.region
            .origin_y
            .checked_add(self.region.len.checked_div(self.region.stride)?)
    }

    /// The region still holds the deblocked rows the frontier released to
    /// this stripe's window.
    pub(crate) const fn holds_deblocked(&self) -> bool {
        self.deblocked
    }

    #[inline]
    /// The target uniquely owns this non-overlapping stripe region.
    pub(crate) fn samples_mut(&mut self) -> &mut [T] {
        // SAFETY: disjoint checked stripes are lent once each; the whole workspace only with none leased.
        unsafe { slice::from_raw_parts_mut(self.region.samples.as_ptr(), self.region.len) }
    }
}

impl<T: ReconSample> DirectPlaneTarget<T> {
    /// The region's rows as they are, viewed while no `samples_mut` borrow lives.
    pub(crate) fn rows(&self) -> Option<crate::filters::source::FramePlane<'_, T>> {
        // SAFETY: the lease gives this target the region alone; `&self` excludes its `samples_mut`.
        let span = unsafe { slice::from_raw_parts(self.region.samples.as_ptr(), self.region.len) };
        let region = &self.region;
        let size = (region.width, region.frame_height);
        let rows = region.len.checked_div(region.stride)?;
        crate::filters::source::FramePlane::strided(
            span,
            size,
            region.stride,
            region.origin_y,
            rows,
        )
    }
}

#[cfg(test)]
impl<T: ReconSample> DirectPlaneTarget<T> {
    pub(crate) fn u8_samples_mut(&mut self) -> Option<&mut [u8]> {
        T::u8_slice_mut(self.samples_mut())
    }
}

impl<T> Drop for DirectPlaneTarget<T> {
    fn drop(&mut self) {
        self.progress.release_hold(self.stripe);
    }
}

impl<T> DirectPlaneRegion<T> {
    fn into_target(
        self,
        progress: Arc<dyn DirectLeaseRelease>,
        stripe: usize,
        deblocked: bool,
    ) -> DirectPlaneTarget<T> {
        DirectPlaneTarget {
            region: self,
            progress,
            stripe,
            deblocked,
        }
    }
}

pub(crate) struct DirectStripeTarget<T> {
    planes: [Option<DirectPlaneTarget<T>>; 3],
}

impl<T> DirectStripeTarget<T> {
    pub(crate) fn get(&self, plane: PlaneId) -> Option<&DirectPlaneTarget<T>> {
        self.planes[plane.index()].as_ref()
    }

    pub(crate) fn take(&mut self, plane: PlaneId) -> Option<DirectPlaneTarget<T>> {
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
    target: Option<DirectStripeTarget<T>>,
}

impl<T: ReconSample> Drop for DirectStripeLease<T> {
    fn drop(&mut self) {
        self.progress.release_direct_hold(self.stripe);
    }
}

impl<T: ReconSample> DirectStripeLease<T> {
    pub(crate) fn take_target(&mut self) -> Option<DirectStripeTarget<T>> {
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
    pub(crate) size: PlaneSize,
}

/// One pending frame's filtered workspace and its published-row watermark.
pub(crate) struct FrameProgress<T: ReconSample> {
    /// The decode's reusable storage, carried so every frame can hand it back
    /// to the scratch it is scheduled with.
    buffers: Option<std::sync::Arc<crate::support::decode_buffers::DecodeBuffers>>,
    /// The filtered workspace while it owns the frame's sample buffers. Only
    /// `begin`, the freeze and an unpublished pass lock it; readers and
    /// stripes reach the samples through `planes`.
    workspace: Mutex<Option<CurrentFrameWorkspace<T>>>,
    layout: OnceLock<Mutex<ProgressLayout>>,
    spare_stripes: Mutex<Vec<StripeProgress>>,
    published_luma_rows: WatermarkCell,
    terminal_published: CompletionCell<()>,
    luma_height: usize,
    subsampling_y: usize,
    info: DecodedFrameInfo,
    /// The workspace's plane table, fixed from `from_workspace` or `reset`
    /// until the next `reset`.
    planes: [Option<PlaneStorage<T>>; 3],
    /// [`UNCLAIMED`], [`RELEASED`], or the luma row end of the last stripe the
    /// live [`FrontierRows`] handed to a filter job.
    frontier: AtomicUsize,
}

const UNCLAIMED: usize = usize::MAX;
const RELEASED: usize = usize::MAX - 1;

/// The terminal watermark of a frame whose samples are in its settled slot.
/// It satisfies every row threshold like the frame height, but tells a
/// reader to read the slot instead of the plane table.
const FROZEN: usize = WatermarkCell::FAILED - 1;

impl<T: ReconSample> DirectLeaseRelease for FrameProgress<T> {
    fn release_hold(&self, stripe: usize) {
        self.release_direct_hold(stripe);
    }
}

impl<T: ReconSample> FrameProgress<T> {
    /// The decode's reusable storage, which every frame reads back from its
    /// progress rather than inheriting down a chain of defaultable scratches.
    pub(crate) fn buffers(
        &self,
    ) -> Option<&std::sync::Arc<crate::support::decode_buffers::DecodeBuffers>> {
        self.buffers.as_ref()
    }

    /// Opens the workspace one pending frame's filter phase filters in place
    /// and publishes, before that phase is handed to a worker.
    pub(crate) fn from_workspace(
        mut workspace: CurrentFrameWorkspace<T>,
        buffers: Option<&std::sync::Arc<crate::support::decode_buffers::DecodeBuffers>>,
    ) -> crate::Result<Self> {
        let buffers = buffers.cloned();
        let info = workspace.info();
        let planes = plane_table(&mut workspace)?;
        Ok(Self {
            buffers,
            info,
            planes,
            frontier: AtomicUsize::new(UNCLAIMED),
            workspace: Mutex::new(Some(workspace)),
            layout: OnceLock::new(),
            spare_stripes: Mutex::new(Vec::new()),
            published_luma_rows: WatermarkCell::new(),
            terminal_published: CompletionCell::new(),
            luma_height: info.storage_luma_size().height(),
            subsampling_y: usize::from(info.pixel_format().subsampling_y()),
        })
    }

    pub(crate) fn reset(&mut self, mut workspace: CurrentFrameWorkspace<T>) -> crate::Result<()> {
        let info = workspace.info();
        let planes = plane_table(&mut workspace)?;
        if let Some(layout) = self.layout.take() {
            *self.spare_stripes.get_mut() = layout.into_inner().stripes;
        }
        self.spare_stripes.get_mut().clear();
        self.info = info;
        self.planes = planes;
        *self.frontier.get_mut() = UNCLAIMED;
        *self.workspace.get_mut() = Some(workspace);
        self.published_luma_rows.reset();
        self.terminal_published.reset();
        self.luma_height = info.storage_luma_size().height();
        self.subsampling_y = usize::from(info.pixel_format().subsampling_y());
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn new(info: DecodedFrameInfo) -> Result<Self> {
        Self::from_workspace(
            CurrentFrameWorkspace::new_recycled_from(
                info,
                &mut splot_recon::FramePlaneSamples::default(),
            )?,
            None,
        )
    }

    /// Publishes the terminal watermark of a filter phase that ended.
    ///
    /// `filtered` publishes [`FROZEN`] after the slot settled with the frame,
    /// which every row threshold satisfies with rows that are genuinely final;
    /// a phase that failed publishes [`WatermarkCell::FAILED`] instead, so a
    /// consumer waiting on a row it will never get is released and fails
    /// closed on the settled slot.
    pub(crate) fn publish_terminal(&self, filtered: bool) {
        self.published_luma_rows.publish(if filtered {
            FROZEN
        } else {
            WatermarkCell::FAILED
        });
        let _ = self.terminal_published.set(());
    }

    pub(crate) fn wait_terminal(&self) {
        let () = self.terminal_published.wait();
    }

    /// Installs the stripe geometry the filter phase will publish through.
    ///
    /// The ranges must ascend, be contiguous, and start at the frame top, since
    /// the watermark is the end of the contiguous published prefix. A geometry
    /// that does not satisfy that leaves the frame unpublished rather than
    /// letting a consumer read an unwritten row. A frame whose planes were
    /// taken has no workspace left to publish.
    pub(crate) fn begin(&self, ranges: &[(usize, usize)]) -> Result<()> {
        let invalid = crate::filters::wienerns_lr::recon::lr_pipeline_state_error;
        let mut next = 0usize;
        for &(start, end) in ranges {
            if start != next || end <= start {
                return Err(invalid());
            }
            next = end;
        }
        let workspace = self.workspace.lock();
        if workspace.is_none() || self.layout.get().is_some() {
            return Err(invalid());
        }
        let mut stripes = core::mem::take(&mut *self.spare_stripes.lock());
        stripes.try_reserve_exact(ranges.len()).map_err(|_| {
            splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: PlaneId::Y,
                context: "filter stripe layout",
            }
        })?;
        stripes.extend(ranges.iter().map(|&(_, end)| StripeProgress {
            end,
            ..StripeProgress::default()
        }));
        let layout = ProgressLayout {
            stripes,
            direct_aligned: ranges
                .iter()
                .take(ranges.len().saturating_sub(1))
                .all(|&(_, end)| end.is_multiple_of(1 << self.subsampling_y)),
            freezing: false,
            prefix: 0,
        };
        self.layout.set(Mutex::new(layout)).map_err(|_| invalid())
    }

    pub(crate) fn direct_stripe(
        self: &std::sync::Arc<Self>,
        stripe: usize,
    ) -> Option<DirectStripeLease<T>> {
        let layout = self.layout.get()?;
        let mut layout = layout.lock();
        if layout.freezing
            || !layout.direct_aligned
            || layout.stripes.get(stripe)?.landed
            || layout.stripes.get(stripe)?.leased
        {
            return None;
        }
        let start = if stripe == 0 {
            0
        } else {
            layout.stripes.get(stripe - 1)?.end
        };
        let end = layout.stripes.get(stripe)?.end;
        let frontier = self.frontier.load(Ordering::Acquire);
        if end > frontier {
            return None;
        }
        let deblocked = frontier != UNCLAIMED;
        let chroma_start = start >> self.subsampling_y;
        let chroma_end = end.div_ceil(1 << self.subsampling_y);
        let y = self.direct_region(PlaneId::Y, start, end)?;
        let u = self.direct_region(PlaneId::U, chroma_start, chroma_end);
        let v = self.direct_region(PlaneId::V, chroma_start, chroma_end);
        if u.is_some() != v.is_some() {
            return None;
        }
        layout.stripes[stripe].leased = true;
        let holds = 2 + u32::from(u.is_some()) + u32::from(v.is_some());
        if let Some(slot) = layout
            .stripes
            .get_mut(stripe)
            .map(|stripe| &mut stripe.holds)
        {
            *slot = holds;
        }
        drop(layout);
        let progress: Arc<dyn DirectLeaseRelease> = self.clone();
        let y = y.into_target(Arc::clone(&progress), stripe, deblocked);
        let u = u.map(|region| region.into_target(Arc::clone(&progress), stripe, deblocked));
        let v = v.map(|region| region.into_target(progress, stripe, deblocked));
        Some(DirectStripeLease {
            progress: Arc::clone(self),
            stripe,
            target: Some(DirectStripeTarget {
                planes: [Some(y), u, v],
            }),
        })
    }

    fn direct_region(
        &self,
        plane: PlaneId,
        start: usize,
        end: usize,
    ) -> Option<DirectPlaneRegion<T>> {
        let storage = self.planes[plane.index()]?;
        if start >= end || end > storage.height {
            return None;
        }
        let start_sample = start.checked_mul(storage.stride)?;
        let len = (end - start).checked_mul(storage.stride)?;
        if start_sample.checked_add(len)? > storage.len {
            return None;
        }
        let samples = NonNull::new(storage.samples.as_ptr().wrapping_add(start_sample))?;
        Some(DirectPlaneRegion {
            samples,
            len,
            width: storage.size.width(),
            stride: storage.stride,
            frame_height: storage.height,
            origin_y: start,
        })
    }

    /// The rows of `plane` below the watermark published now: none after a
    /// failed phase, every row after the freeze.
    fn readable_rows(&self, plane: PlaneId) -> usize {
        let rows = self.published_luma_rows();
        if plane == PlaneId::Y {
            rows
        } else if rows == self.luma_height {
            rows.div_ceil(1 << self.subsampling_y)
        } else {
            rows >> self.subsampling_y
        }
    }

    /// Borrows at most the first `rows` rows of one plane that are below the
    /// watermark, for as long as the slot the caller reads through lives; see
    /// [`PublishedFrame::of_slot`].
    ///
    /// Safety argument of the read: rows <= W from an Acquire load
    /// synchronizes with every stripe write below W. No writer touches rows
    /// below W. The allocation outlives 'a because 'a is bounded by a
    /// &RefFrameSlot borrow whose cell (after the freeze) or progress (before
    /// it) owns the allocation, and both can be released only through &mut or
    /// Arc::get_mut. The freeze moves no allocation and writes no sample, and
    /// on error or unwind it neither frees nor recycles.
    fn published_plane(&self, plane: PlaneId, rows: usize) -> Option<PublishedPlane<'_, T>> {
        let storage = self.planes[plane.index()]?;
        let rows = rows.min(self.readable_rows(plane)).min(storage.height);
        let len = rows.checked_mul(storage.stride)?.min(storage.len);
        let samples = unsafe {
            // SAFETY: rows below W of a slot-bounded allocation; see the doc above.
            slice::from_raw_parts(storage.samples.as_ptr(), len)
        };
        Some(PublishedPlane {
            samples,
            stride: storage.stride,
            size: storage.size,
        })
    }

    /// Drops one holder of a stripe's direct lease, releasing it at zero.
    fn release_direct_hold(&self, stripe: usize) {
        let Some(layout) = self.layout.get() else {
            return;
        };
        let mut layout = layout.lock();
        let Some(stripe) = layout.stripes.get_mut(stripe) else {
            return;
        };
        stripe.holds = stripe.holds.saturating_sub(1);
        if stripe.holds == 0 {
            stripe.leased = false;
        }
    }

    /// Holders left on a stripe's direct lease.
    fn direct_holds(&self, stripe: usize) -> u32 {
        self.layout.get().map_or(0, |layout| {
            layout
                .lock()
                .stripes
                .get(stripe)
                .map_or(0, |stripe| stripe.holds)
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
        let Some(leased) = layout
            .stripes
            .get_mut(stripe)
            .map(|stripe| &mut stripe.leased)
        else {
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

    /// Lends the whole workspace to a pass of a frame that publishes no row
    /// before its freeze. The layout lock it holds lets no stripe lease
    /// start, and no lease, frontier or published row is live.
    pub(crate) fn with_unpublished_workspace<R>(
        &self,
        f: impl FnOnce(&mut CurrentFrameWorkspace<T>) -> R,
    ) -> Result<R> {
        let layout = self.layout.get().map(|layout| layout.lock());
        let released = self.frontier.compare_exchange(
            UNCLAIMED,
            RELEASED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        if layout
            .as_ref()
            .is_some_and(|layout| layout.stripes.iter().any(|stripe| stripe.leased))
            || released.is_err_and(|frontier| frontier != RELEASED)
            || self.published_luma_rows.current() != 0
        {
            return Err(live_direct_lease());
        }
        let mut workspace = self.workspace.lock();
        Ok(f(workspace.as_mut().ok_or_else(taken_workspace)?))
    }

    /// Freezes the filtered workspace into the paired slot.
    ///
    /// Each plane's sample buffer moves into the frozen frame with its
    /// allocation unchanged, so a reader that loaded the watermark before the
    /// freeze keeps reading the same immutable rows, now owned by the slot.
    /// The slot settles before [`FROZEN`] is published, so a reader that sees
    /// [`FROZEN`] finds the frame in the slot. A failed freeze leaves the
    /// workspace and its buffers in place.
    ///
    /// # Errors
    ///
    /// Returns the freeze's own diagnostic, or an internal diagnostic when the
    /// writer belongs to another frame, a stripe or the frontier is still live,
    /// or the workspace has already been taken.
    pub(crate) fn freeze_workspace(&self, writer: FrameSlotWriter<T>) -> Result<()> {
        if !core::ptr::eq(Arc::as_ptr(&writer.progress), self) {
            return Err(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into());
        }
        if self.frontier_live() {
            return Err(live_direct_lease());
        }
        if let Some(layout) = self.layout.get() {
            let mut layout = layout.lock();
            if layout.stripes.iter().any(|stripe| stripe.leased) {
                return Err(live_direct_lease());
            }
            layout.freezing = true;
        }
        let mut workspace = self.workspace.lock();
        let window = AbortOnUnwind;
        let frame = workspace
            .as_mut()
            .ok_or_else(taken_workspace)?
            .freeze_in_place_clamped()?;
        *workspace = None;
        drop(workspace);
        writer.complete(frame);
        drop(window);
        Ok(())
    }

    /// Freezes the workspace of a progress no slot shares, which no reader or
    /// stripe can hold since the caller has it exclusively.
    ///
    /// # Errors
    ///
    /// As [`Self::freeze_workspace`].
    pub(crate) fn freeze(&mut self) -> Result<splot_recon::DecodedFrame<T>> {
        let mut workspace = self
            .workspace
            .get_mut()
            .take()
            .ok_or_else(taken_workspace)?;
        Ok(workspace.freeze_in_place_clamped()?)
    }
}

/// Aborts the process when the freeze window unwinds: readers may hold rows of
/// the buffers it moves, so dropping them on the way out would free memory
/// still in use.
struct AbortOnUnwind;

impl Drop for AbortOnUnwind {
    fn drop(&mut self) {
        if std::thread::panicking() {
            std::process::abort();
        }
    }
}

impl<T: ReconSample> FrameProgress<T> {
    /// Claims the frame's one frontier handle, which seals, deblocks and copies
    /// the rows no stripe has been handed yet.
    pub(crate) fn frontier_rows(self: &Arc<Self>) -> Option<FrontierRows<T>> {
        self.frontier
            .compare_exchange(UNCLAIMED, 0, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(FrontierRows {
            progress: Arc::clone(self),
            final_luma_rows: 0,
            released_luma_rows: 0,
            commit_start: [usize::MAX; 3],
            commit_lease: None,
        })
    }

    fn frontier_live(&self) -> bool {
        !matches!(self.frontier.load(Ordering::Acquire), UNCLAIMED | RELEASED)
    }
}

/// The § 7.17 frontier's exclusive handle on the frame rows at or past the
/// last stripe it handed to a filter job.
///
/// Rows below `released_luma_rows` belong to stripe jobs and readers; rows at
/// or past `final_luma_rows` are still sealed and deblocked in place; the rows
/// between are final and are only copied into stripe windows. The plane rows
/// from `commit_start` are lent to the commit spine, which reconstructs them.
pub(crate) struct FrontierRows<T: ReconSample> {
    progress: Arc<FrameProgress<T>>,
    final_luma_rows: usize,
    released_luma_rows: usize,
    commit_start: [usize; 3],
    /// The first luma sample address of the live commit band, which keeps
    /// the frontier claimed when this handle drops, so the frame cannot freeze.
    commit_lease: Option<usize>,
}

impl<T: ReconSample> Drop for FrontierRows<T> {
    fn drop(&mut self) {
        if self.commit_lease.is_none() {
            self.progress.frontier.store(RELEASED, Ordering::Release);
        }
    }
}

impl<T: ReconSample> FrontierRows<T> {
    pub(crate) fn info(&self) -> DecodedFrameInfo {
        self.progress.info
    }

    pub(crate) fn plane_size(&self, plane: PlaneId) -> Option<(usize, usize)> {
        let storage = self.progress.planes[plane.index()]?;
        Some((storage.size.width(), storage.height))
    }

    pub(crate) const fn final_luma_rows(&self) -> usize {
        self.final_luma_rows
    }

    fn shift(&self, plane: PlaneId) -> usize {
        usize::from(plane != PlaneId::Y) * self.progress.subsampling_y
    }

    /// The checked start pointer and length of plane rows `start..end`.
    fn span(&self, plane: PlaneId, start: usize, end: usize) -> Option<(*mut T, usize)> {
        let storage = self.progress.planes[plane.index()]?;
        let sample_end = end.checked_mul(storage.stride)?;
        if start > end || end > storage.height || sample_end > storage.len {
            return None;
        }
        let sample_start = start * storage.stride;
        Some((
            storage.samples.as_ptr().wrapping_add(sample_start),
            sample_end - sample_start,
        ))
    }

    /// Lends plane rows `start..end` that no deblock pass has finalized.
    pub(crate) fn with_plane_rows_mut<R>(
        &mut self,
        plane: PlaneId,
        start: usize,
        end: usize,
        f: impl FnOnce(&mut [T], usize, usize, usize, usize) -> R,
    ) -> Option<R> {
        if (start << self.shift(plane)) < self.final_luma_rows
            || end > self.commit_start[plane.index()]
        {
            return None;
        }
        let (width, height) = self.plane_size(plane)?;
        let stride = self.progress.planes[plane.index()]?.stride;
        let (samples, len) = self.span(plane, start, end)?; // SAFETY: rows from `final_luma_rows` (never below `released_luma_rows`) up to `commit_start` are this unique frontier's alone: `direct_stripe` refuses a stripe ending past the release and the published prefix is made of such stripes; the rows from `commit_start` belong to the commit band, which this call refuses and which only this handle's `lend_commit_rows(&mut self)` moves down; freeze and plane take refuse while this handle lives, so the storage stays allocated.
        let samples = unsafe { slice::from_raw_parts_mut(samples, len) };
        Some(f(samples, stride, width, height, start))
    }

    /// Seals reconstructed luma rows and their chroma rows into the frame.
    pub(crate) fn copy_rows_from(
        &mut self,
        source: &CurrentFrameWorkspace<T>,
        luma_rows: core::ops::Range<usize>,
    ) -> splot_recon::Result<()> {
        let geometry = || splot_recon::ReconError::ArithmeticOverflow {
            context: "deblocked source row geometry",
        };
        if source.info() != self.progress.info || luma_rows.start > luma_rows.end {
            return Err(geometry());
        }
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            if self.progress.planes[plane.index()].is_none() {
                continue;
            }
            let source = source.plane(plane)?;
            let shift = self.shift(plane);
            let start = luma_rows.start >> shift;
            let end = luma_rows.end.div_ceil(1 << shift);
            let stride = source.stride_samples();
            let rows = source.rows(start, end)?;
            self.with_plane_rows_mut(plane, start, end, |target, target_stride, _, _, _| {
                (target_stride == stride).then(|| target.copy_from_slice(rows))
            })
            .flatten()
            .ok_or_else(geometry)?;
        }
        Ok(())
    }

    pub(crate) fn publish_final_rows(&mut self, rows: usize) -> bool {
        let rows = rows.min(self.progress.luma_height);
        if rows < self.final_luma_rows
            || [PlaneId::Y, PlaneId::U, PlaneId::V]
                .into_iter()
                .any(|plane| {
                    rows.div_ceil(1 << self.shift(plane)) > self.commit_start[plane.index()]
                })
        {
            return false;
        }
        self.final_luma_rows = rows;
        true
    }

    /// Appends final plane rows `start..end` that no stripe has been handed.
    pub(crate) fn append_rows(
        &self,
        plane: PlaneId,
        start: usize,
        end: usize,
        out: &mut splot_recon::PlaneBuf<T>,
    ) -> Option<()> {
        let scale = 1 << self.shift(plane);
        if start < self.released_luma_rows.div_ceil(scale)
            || end > self.final_luma_rows.div_ceil(scale)
        {
            return None;
        }
        let (samples, len) = self.span(plane, start, end)?; // SAFETY: final rows at or past `released_luma_rows` are, as in `with_plane_rows_mut`, viewed by no stripe or reader, and no deblock pass writes below `final_luma_rows`.
        let rows = unsafe { slice::from_raw_parts(samples, len) };
        let (width, stride) = (
            self.plane_size(plane)?.0,
            self.progress.planes[plane.index()]?.stride,
        );
        if width == stride {
            out.append_samples(rows);
        } else {
            for row in rows.chunks_exact(stride) {
                out.append_samples(row.get(..width)?);
            }
        }
        Some(())
    }

    /// Hands the rows below `luma_end` to stripe jobs for good.
    pub(crate) fn release_rows(&mut self, luma_end: usize) -> bool {
        if luma_end < self.released_luma_rows || luma_end > self.final_luma_rows {
            return false;
        }
        self.released_luma_rows = luma_end;
        self.progress.frontier.store(luma_end, Ordering::Release);
        true
    }

    /// Lends the commit spine the full-width rows of the superblock row from
    /// luma row `top`, `rows` tall, and the plane row above it, as a band
    /// that keeps `scratch`.
    ///
    /// # Errors
    /// Returns an internal diagnostic while another band lives, or when the
    /// rows reach a final row.
    pub(crate) fn lend_commit_rows(
        &mut self,
        top: usize,
        rows: usize,
        scratch: splot_recon::IntraPredictionScratch<T>,
    ) -> Result<CurrentFrameWorkspace<T>> {
        if self.commit_lease.is_some() {
            return Err(live_direct_lease());
        }
        let bottom = top.saturating_add(rows);
        let mut starts = [usize::MAX; 3];
        let mut spans = [None, None, None];
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            let Some(storage) = self.progress.planes[plane.index()] else {
                continue;
            };
            let shift = self.shift(plane);
            let start = (top >> shift).saturating_sub(1);
            let end = bottom.div_ceil(1 << shift).min(storage.height);
            if start < self.final_luma_rows.div_ceil(1 << shift) {
                return Err(live_direct_lease());
            }
            let (samples, len) = self.span(plane, start, end).ok_or_else(live_direct_lease)?;
            starts[plane.index()] = start;
            spans[plane.index()] = Some((
                NonNull::new(samples).ok_or_else(live_direct_lease)?,
                len,
                start,
            ));
        }
        let lender: Arc<dyn Send + Sync> = self.progress.clone();
        let planes = spans.map(|span| {
            span.map(|(samples, len, start)| {
                // SAFETY: these rows are at or past `final_luma_rows` and below no stripe, so only this frontier could reach them, and `with_plane_rows_mut` refuses them from `commit_start` on; no other band lives; `lender` keeps the frame allocated and a live band keeps the frontier claimed, so the frame cannot freeze or be reset under it.
                (
                    unsafe { splot_recon::LentRows::lend(samples, len, Arc::clone(&lender)) },
                    start,
                )
            })
        });
        self.commit_start = starts;
        let band = CurrentFrameWorkspace::lend_band(self.progress.info, planes, scratch)?;
        self.commit_lease = band.lent_luma_addr();
        Ok(band)
    }

    /// The most luma rows [`Self::publish_final_rows`] accepts while the
    /// commit band holds the rows from `commit_start`.
    pub(crate) fn unlent_luma_rows(&self) -> usize {
        [PlaneId::Y, PlaneId::U, PlaneId::V]
            .into_iter()
            .map(|plane| self.commit_start[plane.index()].saturating_mul(1 << self.shift(plane)))
            .min()
            .unwrap_or(usize::MAX)
    }

    /// Takes back the commit band, so every row is this frontier's again,
    /// and hands back its scratch.
    ///
    /// # Errors
    /// Returns an internal diagnostic when `band` is not the live band.
    pub(crate) fn return_commit_rows(
        &mut self,
        mut band: CurrentFrameWorkspace<T>,
    ) -> Result<splot_recon::IntraPredictionScratch<T>> {
        if self.commit_lease.is_none() || band.lent_luma_addr() != self.commit_lease {
            return Err(live_direct_lease());
        }
        let mut scratch = splot_recon::IntraPredictionScratch::new();
        band.swap_intra_prediction_scratch(&mut scratch);
        drop(band);
        self.commit_lease = None;
        self.commit_start = [usize::MAX; 3];
        Ok(scratch)
    }
}

fn complete_stripe(layout: &mut ProgressLayout, stripe: usize) -> usize {
    let Some(landed) = layout
        .stripes
        .get_mut(stripe)
        .map(|stripe| &mut stripe.landed)
    else {
        return layout
            .prefix
            .checked_sub(1)
            .and_then(|last| layout.stripes.get(last).map(|stripe| stripe.end))
            .unwrap_or_default();
    };
    *landed = true;
    while layout
        .stripes
        .get(layout.prefix)
        .is_some_and(|stripe| stripe.landed)
    {
        layout.prefix += 1;
    }
    layout
        .prefix
        .checked_sub(1)
        .and_then(|last| layout.stripes.get(last).map(|stripe| stripe.end))
        .unwrap_or_default()
}

/// A shared borrow of one frame's published filtered prefix.
#[derive(Clone, Copy)]
pub(crate) struct PublishedFrame<'a, T: ReconSample> {
    progress: &'a FrameProgress<T>,
    luma_rows: NonZeroUsize,
    chroma_rows: usize,
}

impl<T: ReconSample> core::fmt::Debug for PublishedFrame<'_, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PublishedFrame")
    }
}

impl<'a, T: ReconSample> PublishedFrame<'a, T> {
    /// Borrows the published prefix of a still-filtering slot's samples.
    ///
    /// Returns `None` before the first stripe lands, after a failed phase
    /// (whose rows must fail closed) and after the freeze, whose frame the
    /// caller then reads from the settled slot. A frame written whole
    /// publishes no row before its freeze, so its plane table is never read.
    pub(crate) fn of_slot(slot: &'a RefFrameSlot<T>) -> Option<Self> {
        Self::of_progress(slot.progress()?)
    }

    fn of_progress(progress: &'a FrameProgress<T>) -> Option<Self> {
        let rows = progress.published_luma_rows.current();
        if rows == FROZEN || rows == WatermarkCell::FAILED {
            return None;
        }
        let luma_rows = NonZeroUsize::new(rows.min(progress.luma_height))?;
        progress.layout.get()?;
        let chroma_rows = if luma_rows.get() == progress.luma_height {
            luma_rows.get().div_ceil(1 << progress.subsampling_y)
        } else {
            luma_rows.get() >> progress.subsampling_y
        };
        Some(Self {
            progress,
            luma_rows,
            chroma_rows,
        })
    }

    /// The frame's storage alone, which a reader carries beside these rows.
    pub(crate) const fn storage(self) -> PublishedStorage<'a, T> {
        PublishedStorage {
            progress: self.progress,
        }
    }

    /// Borrows one plane's published rows.
    #[cfg(test)]
    pub(crate) fn plane(self, plane: PlaneId) -> Option<PublishedPlane<'a, T>> {
        let rows = if plane == PlaneId::Y {
            self.luma_rows()
        } else {
            self.chroma_rows()
        };
        self.storage().plane(plane, rows)
    }

    /// The number of final luma rows, which is never zero.
    pub(crate) const fn luma_rows(self) -> usize {
        self.luma_rows.get()
    }

    /// The number of final chroma rows.
    ///
    /// A chroma row is final once every in-frame luma row it subsamples is, so
    /// interior prefixes truncate while a complete odd-height frame includes
    /// its terminal chroma row.
    pub(crate) const fn chroma_rows(self) -> usize {
        self.chroma_rows
    }
}

/// The storage of a [`PublishedFrame`], one pointer wide.
#[derive(Clone, Copy)]
pub(crate) struct PublishedStorage<'a, T: ReconSample> {
    progress: &'a FrameProgress<T>,
}

impl<T: ReconSample> core::fmt::Debug for PublishedStorage<'_, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PublishedStorage")
    }
}

impl<'a, T: ReconSample> PublishedStorage<'a, T> {
    pub(crate) const fn info(self) -> DecodedFrameInfo {
        self.progress.info
    }

    /// Borrows at most `rows` published rows of one plane.
    pub(crate) fn plane(self, plane: PlaneId, rows: usize) -> Option<PublishedPlane<'a, T>> {
        self.progress.published_plane(plane, rows)
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
