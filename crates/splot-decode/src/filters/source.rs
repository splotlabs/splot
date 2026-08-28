// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(
    unsafe_code,
    reason = "stripe buffers and final deblocked-row leases retain their unique allocation owner"
)]

use splot_recon::{CurrentFrameWorkspace, OwnedFrameBands, PlaneId, PlaneRect, ReconSample};
use std::any::Any;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ptr::NonNull;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Copy)]
struct DeblockedPlaneStorage<T> {
    samples: NonNull<T>,
    len: usize,
    stride: usize,
    width: usize,
    height: usize,
}

/// One contiguous reconstructed workspace whose final deblocked prefix may be
/// read by filter jobs while deblock continues below that prefix.
pub(crate) struct DeblockedSource<T: ReconSample> {
    storage: Arc<DeblockedStorage<T>>,
    final_luma_rows: usize,
}

struct DeblockedStorage<T: ReconSample> {
    workspace: ManuallyDrop<CurrentFrameWorkspace<T>>,
    info: splot_recon::DecodedFrameInfo,
    planes: [Option<DeblockedPlaneStorage<T>>; 3],
    #[cfg(test)]
    recycled: Option<Arc<AtomicBool>>,
}

/// Safety: `DeblockedSource` is the sole mutable writer, admits writes only below
/// immutable leases, and keeps this storage alive for every view.
unsafe impl<T: ReconSample> Send for DeblockedStorage<T> {}
/// Safety: shared storage access creates immutable views only.
unsafe impl<T: ReconSample> Sync for DeblockedStorage<T> {}

impl<T: ReconSample> DeblockedSource<T> {
    pub(crate) fn new(mut workspace: CurrentFrameWorkspace<T>) -> Self {
        let info = workspace.info();
        let geometry = [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| {
            workspace
                .plane(plane)
                .ok()
                .map(splot_recon::CurrentFramePlane::storage_size)
        });
        let mut planes = [None, None, None];
        let (y, u, v) = workspace.as_frame_mut().into_planes();
        for (plane, view) in [Some(y), u, v].into_iter().enumerate() {
            let (Some(mut view), Some(size)) = (view, geometry[plane]) else {
                continue;
            };
            let stride = view.stride_samples();
            let samples = view.samples_mut();
            let len = samples.len();
            let Some(samples) = NonNull::new(samples.as_mut_ptr()) else {
                continue;
            };
            planes[plane] = Some(DeblockedPlaneStorage {
                samples,
                len,
                stride,
                width: size.width(),
                height: size.height(),
            });
        }
        Self {
            storage: Arc::new(DeblockedStorage {
                workspace: ManuallyDrop::new(workspace),
                info,
                planes,
                #[cfg(test)]
                recycled: None,
            }),
            final_luma_rows: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_recycle_probe(
        workspace: CurrentFrameWorkspace<T>,
        recycled: Arc<AtomicBool>,
    ) -> Self {
        let mut source = Self::new(workspace);
        if let Some(storage) = Arc::get_mut(&mut source.storage) {
            storage.recycled = Some(recycled);
        }
        source
    }

    pub(crate) fn info(&self) -> splot_recon::DecodedFrameInfo {
        self.storage.info
    }

    pub(crate) fn plane_size(&self, plane: PlaneId) -> Option<(usize, usize)> {
        self.storage.planes[plane.index()].map(|plane| (plane.width, plane.height))
    }

    /// The mutable receiver guarantees this ascending copy cannot enter a lease.
    pub(crate) fn copy_rows_from(
        &mut self,
        source: &CurrentFrameWorkspace<T>,
        luma_rows: core::ops::Range<usize>,
    ) -> splot_recon::Result<()> {
        if source.info() != self.storage.info || luma_rows.start > luma_rows.end {
            return Err(splot_recon::ReconError::ArithmeticOverflow {
                context: "deblocked source row geometry",
            });
        }
        let sub_y = usize::from(self.storage.info.pixel_format().subsampling_y());
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            let Some(storage) = self.storage.planes[plane.index()] else {
                continue;
            };
            let source = source.plane(plane)?;
            let shift = usize::from(plane != PlaneId::Y) * sub_y;
            let start = luma_rows.start >> shift;
            let end = luma_rows.end.div_ceil(1 << shift);
            if start > end
                || end > storage.height
                || source.stride_samples() != storage.stride
                || source.storage_size().width() != storage.width
                || (start << shift) < self.final_luma_rows
            {
                return Err(splot_recon::ReconError::ArithmeticOverflow {
                    context: "deblocked source row geometry",
                });
            }
            let sample_start = start * storage.stride;
            let sample_end = end * storage.stride;
            let source = source.samples().get(sample_start..sample_end).ok_or(
                splot_recon::ReconError::ArithmeticOverflow {
                    context: "deblocked source row geometry",
                },
            )?;
            if sample_end > storage.len {
                return Err(splot_recon::ReconError::ArithmeticOverflow {
                    context: "deblocked source row geometry",
                });
            } // SAFETY: the mutable owner lends only unpublished rows in bounds.
            unsafe {
                core::slice::from_raw_parts_mut(
                    storage.samples.as_ptr().add(sample_start),
                    sample_end - sample_start,
                )
            }
            .copy_from_slice(source);
        }
        Ok(())
    }

    /// Lends only a checked band below the immutable final-row frontier.
    pub(crate) fn with_plane_rows_mut<R>(
        &mut self,
        plane: PlaneId,
        start: usize,
        end: usize,
        f: impl FnOnce(&mut [T], usize, usize, usize, usize) -> R,
    ) -> Option<R> {
        let storage = self.storage.planes[plane.index()]?;
        let shift = usize::from(plane != PlaneId::Y)
            * usize::from(self.storage.info.pixel_format().subsampling_y());
        if start > end || end > storage.height || (start << shift) < self.final_luma_rows {
            return None;
        }
        let sample_start = start.checked_mul(storage.stride)?;
        let sample_end = end.checked_mul(storage.stride)?;
        if sample_end > storage.len {
            return None;
        } // SAFETY: the mutable owner lends only unpublished rows in bounds.
        let samples = unsafe {
            core::slice::from_raw_parts_mut(
                storage.samples.as_ptr().add(sample_start),
                sample_end - sample_start,
            )
        };
        Some(f(
            samples,
            storage.stride,
            storage.width,
            storage.height,
            start,
        ))
    }

    pub(crate) fn publish_final_rows(&mut self, rows: usize) -> bool {
        let rows = rows.min(self.storage.info.coded_luma_size().height());
        if rows < self.final_luma_rows {
            return false;
        }
        self.final_luma_rows = rows;
        true
    }

    pub(crate) fn lease(
        &self,
        luma_start: usize,
        luma_end: usize,
        margin: usize,
    ) -> Option<DeblockedReadLease<T>> {
        let ranges = self.lease_ranges(luma_start, luma_end, margin)?;
        Some(DeblockedReadLease {
            source: Arc::clone(&self.storage),
            ranges,
        })
    }

    /// Reuses one serial reader's storage owner for another checked stripe.
    pub(crate) fn retarget_lease(
        &self,
        lease: &mut DeblockedReadLease<T>,
        luma_start: usize,
        luma_end: usize,
        margin: usize,
    ) -> bool {
        if !Arc::ptr_eq(&self.storage, &lease.source) {
            return false;
        }
        let Some(ranges) = self.lease_ranges(luma_start, luma_end, margin) else {
            return false;
        };
        lease.ranges = ranges;
        true
    }

    fn lease_ranges(
        &self,
        luma_start: usize,
        luma_end: usize,
        margin: usize,
    ) -> Option<[Option<(usize, usize)>; 3]> {
        let luma_height = self.storage.info.coded_luma_size().height();
        let needed = luma_end
            .checked_add(margin << usize::from(self.storage.info.pixel_format().subsampling_y()))?
            .min(luma_height);
        if luma_start >= luma_end || luma_end > luma_height || needed > self.final_luma_rows {
            return None;
        }
        let sub_y = usize::from(self.storage.info.pixel_format().subsampling_y());
        let mut ranges = [None; 3];
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            let Some(storage) = self.storage.planes[plane.index()] else {
                continue;
            };
            let shift = usize::from(plane != PlaneId::Y) * sub_y;
            ranges[plane.index()] =
                Some(window_bounds((luma_start, luma_end), shift, margin, storage.height).ok()?);
        }
        Some(ranges)
    }
}

impl<T: ReconSample> DeblockedStorage<T> {
    /// Builds an immutable view only for rows released through `lease`.
    fn plane(&self, plane: PlaneId, range: (usize, usize)) -> Option<FramePlane<'_, T>> {
        let storage = self.planes[plane.index()]?;
        let (start, end) = range;
        let sample_start = start.checked_mul(storage.stride)?;
        let sample_end = end.checked_mul(storage.stride)?;
        if start >= end || end > storage.height || sample_end > storage.len {
            return None;
        } // SAFETY: leases expose only checked immutable finalized rows.
        let samples = unsafe {
            core::slice::from_raw_parts(
                storage.samples.as_ptr().add(sample_start),
                sample_end - sample_start,
            )
        };
        Some(FramePlane {
            width: storage.width,
            height: storage.height,
            stride: storage.stride,
            origin_y: start,
            storage_origin_y: start,
            storage_rows: end - start,
            samples,
            secondary: &[],
        })
    }
}

/// Recycles the workspace after the final owning `Arc` is gone.
impl<T: ReconSample> Drop for DeblockedStorage<T> {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::take(&mut self.workspace).recycle_planes();
        } // SAFETY: the final owning Arc has exclusive workspace access.
        #[cfg(test)]
        if let Some(recycled) = &self.recycled {
            recycled.store(true, Ordering::SeqCst);
        }
    }
}

pub(crate) struct DeblockedReadLease<T: ReconSample> {
    source: Arc<DeblockedStorage<T>>,
    ranges: [Option<(usize, usize)>; 3],
}

impl<T: ReconSample> DeblockedReadLease<T> {
    pub(crate) fn planes(&self) -> Option<DeblockedPlanes<'_, T>> {
        Some(DeblockedPlanes {
            y: self
                .source
                .plane(PlaneId::Y, self.ranges[PlaneId::Y.index()]?)?,
            u: self.ranges[PlaneId::U.index()]
                .and_then(|range| self.source.plane(PlaneId::U, range)),
            v: self.ranges[PlaneId::V.index()]
                .and_then(|range| self.source.plane(PlaneId::V, range)),
        })
    }
}

pub(crate) enum DeblockedStripe<T: ReconSample> {
    Window(DeblockedWindow<T>),
    Lease(DeblockedReadLease<T>),
}

impl<T: ReconSample> DeblockedStripe<T> {
    pub(crate) fn planes(&self) -> Option<DeblockedPlanes<'_, T>> {
        match self {
            Self::Window(window) => window.planes(),
            Self::Lease(lease) => lease.planes(),
        }
    }
}

/// Fewest stripe sample buffers retained, and the floor the pool-width bound
/// never drops below.
const MIN_RETAINED_STRIPE_BUFFERS: usize = 128;
/// Stripe sample buffers retained per worker: a wide pool has that many more
/// stripe chains in flight, each holding its own copy.
const RETAINED_STRIPE_BUFFERS_PER_WORKER: usize = 16;

/// Retains one worker's share per worker, with [`MIN_RETAINED_STRIPE_BUFFERS`]
/// as the floor.
/// Scales per worker only on a pool thread; off-pool callers get the floor.
fn max_retained_stripe_buffers() -> usize {
    splot_parallel::current_pool_width()
        .saturating_mul(RETAINED_STRIPE_BUFFERS_PER_WORKER)
        .max(MIN_RETAINED_STRIPE_BUFFERS)
}
static STRIPE_SAMPLE_BUFFERS: Mutex<Vec<Vec<u16>>> = Mutex::new(Vec::new());

fn lock_stripe_sample_buffers() -> MutexGuard<'static, Vec<Vec<u16>>> {
    STRIPE_SAMPLE_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn select_buffer_index<I>(capacities: I, sample_count: usize, pool_is_full: bool) -> Option<usize>
where
    I: IntoIterator<Item = (usize, usize)>,
{
    let mut fitting: Option<(usize, usize)> = None;
    let mut fallback: Option<(usize, usize)> = None;
    for (index, capacity) in capacities {
        if capacity >= sample_count {
            if fitting.is_none_or(|(_, best_capacity)| capacity < best_capacity) {
                fitting = Some((index, capacity));
            }
        } else if pool_is_full && fallback.is_none_or(|(_, best_capacity)| capacity > best_capacity)
        {
            fallback = Some((index, capacity));
        }
    }
    fitting.or(fallback).map(|(index, _)| index)
}

/// Why a stripe/window copy failed: inconsistent geometry, or storage that
/// could not be reserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StripeCopyError {
    Geometry,
    Allocation(PlaneId),
}

impl StripeCopyError {
    pub(crate) const fn for_plane(self, plane: PlaneId) -> Self {
        match self {
            Self::Allocation(_) => Self::Allocation(plane),
            Self::Geometry => Self::Geometry,
        }
    }
}

fn take_stripe_sample_buffer(sample_count: usize) -> Result<Vec<u16>, StripeCopyError> {
    let mut buffers = lock_stripe_sample_buffers();
    let mut buffer = take_stripe_sample_buffer_from_pool(&mut buffers, sample_count);
    drop(buffers);
    buffer.clear();
    buffer
        .try_reserve_exact(sample_count)
        .map_err(|_| StripeCopyError::Allocation(PlaneId::Y))?;
    Ok(buffer)
}

fn take_stripe_sample_buffer_from_pool(
    buffers: &mut Vec<Vec<u16>>,
    sample_count: usize,
) -> Vec<u16> {
    let pool_is_full = buffers.len() >= max_retained_stripe_buffers();
    let index = select_buffer_index(
        buffers
            .iter()
            .enumerate()
            .map(|(index, buffer)| (index, buffer.capacity())),
        sample_count,
        pool_is_full,
    );
    index
        .map(|index| buffers.swap_remove(index))
        .unwrap_or_default()
}

fn recycle_stripe_sample_buffer(mut buffer: Vec<u16>) {
    if buffer.capacity() == 0 {
        return;
    }
    buffer.clear();
    let mut buffers = lock_stripe_sample_buffers();
    recycle_stripe_sample_buffer_into_pool(&mut buffers, buffer);
}

fn recycle_stripe_sample_buffer_into_pool(buffers: &mut Vec<Vec<u16>>, buffer: Vec<u16>) {
    if buffers.len() < max_retained_stripe_buffers() && buffers.try_reserve(1).is_ok() {
        buffers.push(buffer);
    }
}

/// A frame plane backed by one or two packed row spans.
///
/// `height` is always the plane's frame height, so callers keep reasoning in
/// frame coordinates; `origin_y` and `rows` name the window the view actually
/// carries; accesses outside that logical range fail closed.
#[derive(Clone, Copy)]
pub(crate) struct FramePlane<'a, T> {
    width: usize,
    height: usize,
    stride: usize,
    origin_y: usize,
    storage_origin_y: usize,
    storage_rows: usize,
    samples: &'a [T],
    secondary: &'a [T],
}

#[derive(Clone, Copy)]
pub(crate) struct PackedPlane<'a, T> {
    width: usize,
    height: usize,
    stride: usize,
    origin_y: usize,
    rows: usize,
    samples: &'a [T],
}

impl<'a, T: ReconSample> FramePlane<'a, T> {
    #[cfg(test)]
    pub(crate) fn new(workspace: &'a CurrentFrameWorkspace<T>, plane: PlaneId) -> Option<Self> {
        let source = workspace.plane(plane).ok()?;
        let size = source.storage_size();
        Some(Self {
            width: size.width(),
            height: size.height(),
            stride: source.stride_samples(),
            origin_y: 0,
            storage_origin_y: 0,
            storage_rows: size.height(),
            samples: source.samples(),
            secondary: &[],
        })
    }

    /// Views `samples` as the plane rows `origin_y..origin_y + rows` of a plane
    /// `width` wide and `height` tall, packed at `width` samples per row.
    #[cfg(test)]
    pub(crate) fn window(
        samples: &'a [T],
        width: usize,
        height: usize,
        origin_y: usize,
        rows: usize,
    ) -> Option<Self> {
        if width == 0 || origin_y.checked_add(rows)? > height || samples.len() < width * rows {
            return None;
        }
        Some(Self {
            width,
            height,
            stride: width,
            origin_y,
            storage_origin_y: origin_y,
            storage_rows: rows,
            samples,
            secondary: &[],
        })
    }

    pub(crate) const fn width(self) -> usize {
        self.width
    }

    pub(crate) const fn frame_height(self) -> usize {
        self.height
    }

    pub(crate) const fn stride(self) -> usize {
        self.stride
    }

    /// The first plane row this view carries.
    pub(crate) const fn origin_y(self) -> usize {
        self.origin_y
    }

    /// The exclusive last plane row this view carries.
    pub(crate) const fn end_y(self) -> usize {
        self.storage_origin_y + self.storage_rows
    }

    pub(crate) fn contiguous_rows(self, origin_y: usize, end_y: usize) -> Option<&'a [T]> {
        if self.stride == self.width && origin_y <= end_y {
            let offsets = origin_y
                .checked_sub(self.storage_origin_y)
                .and_then(|start| start.checked_mul(self.stride))
                .zip(
                    end_y
                        .checked_sub(self.storage_origin_y)
                        .and_then(|end| end.checked_mul(self.stride)),
                );
            if let Some((start, end)) = offsets
                && end_y <= self.storage_origin_y + self.storage_rows
            {
                return self.samples.get(start..end);
            }
        }
        let secondary_rows = self.secondary.len().checked_div(self.width)?;
        if self.secondary.is_empty()
            || origin_y < self.origin_y
            || end_y < origin_y
            || end_y > self.origin_y + secondary_rows
        {
            return None;
        }
        let start = (origin_y - self.origin_y).checked_mul(self.width)?;
        let end = (end_y - self.origin_y).checked_mul(self.width)?;
        self.secondary.get(start..end)
    }

    #[inline]
    pub(crate) fn packed_storage(self, origin_y: usize, end_y: usize) -> Option<(&'a [T], usize)> {
        if self.stride == self.width
            && origin_y <= end_y
            && origin_y >= self.storage_origin_y
            && end_y <= self.storage_origin_y + self.storage_rows
        {
            return Some((self.samples, self.storage_origin_y));
        }
        let secondary_rows = self.secondary.len().checked_div(self.width)?;
        (!self.secondary.is_empty()
            && origin_y >= self.origin_y
            && end_y <= self.origin_y + secondary_rows)
            .then_some((self.secondary, self.origin_y))
    }

    pub(crate) fn packed_plane(self, origin_y: usize, end_y: usize) -> Option<PackedPlane<'a, T>> {
        let (samples, storage_origin_y) = self.packed_storage(origin_y, end_y)?;
        let (stride, rows) = if storage_origin_y == self.storage_origin_y {
            (self.stride, self.storage_rows)
        } else {
            (self.width, self.secondary.len().checked_div(self.width)?)
        };
        Some(PackedPlane {
            width: self.width,
            height: self.height,
            stride,
            origin_y: storage_origin_y,
            rows,
            samples,
        })
    }

    pub(crate) fn whole_packed(self) -> Option<PackedPlane<'a, T>> {
        self.secondary.is_empty().then_some(PackedPlane {
            width: self.width,
            height: self.height,
            stride: self.stride,
            origin_y: self.storage_origin_y,
            rows: self.storage_rows,
            samples: self.samples,
        })
    }

    fn u16_row_spans(self, origin_y: usize, end_y: usize) -> Option<(&'a [u16], &'a [u16])> {
        let expected = end_y.checked_sub(origin_y)?.checked_mul(self.width)?;
        if let Some(source) = self.contiguous_rows(origin_y, end_y).and_then(T::u16_slice) {
            return (source.len() == expected).then_some((source, &[]));
        }
        if self.secondary.is_empty() {
            return None;
        }
        let upper_end = end_y.min(self.storage_origin_y);
        let upper_rows = upper_end.saturating_sub(origin_y);
        let upper = if upper_rows == 0 {
            &[]
        } else {
            let start = origin_y
                .checked_sub(self.origin_y)?
                .checked_mul(self.width)?;
            let end = (upper_end - self.origin_y).checked_mul(self.width)?;
            T::u16_slice(self.secondary.get(start..end)?)?
        };
        let lower_start = origin_y.max(self.storage_origin_y);
        let lower = if lower_start < end_y {
            self.contiguous_rows(lower_start, end_y)
                .and_then(T::u16_slice)?
        } else {
            &[]
        };
        (upper.len().checked_add(lower.len())? == expected).then_some((upper, lower))
    }

    fn copy_u16_rows(self, origin_y: usize, end_y: usize, destination: &mut [u16]) -> Option<()> {
        let (upper, lower) = self.u16_row_spans(origin_y, end_y)?;
        if destination.len() != upper.len().checked_add(lower.len())? {
            return None;
        }
        let (upper_destination, lower_destination) = destination.split_at_mut(upper.len());
        upper_destination.copy_from_slice(upper);
        lower_destination.copy_from_slice(lower);
        Some(())
    }

    fn append_u16_rows(
        self,
        origin_y: usize,
        end_y: usize,
        destination: &mut Vec<u16>,
    ) -> Option<()> {
        let (upper, lower) = self.u16_row_spans(origin_y, end_y)?;
        destination.extend_from_slice(upper);
        destination.extend_from_slice(lower);
        Some(())
    }

    pub(crate) fn row(self, y: usize) -> Option<&'a [T]> {
        if let Some(row) = y.checked_sub(self.storage_origin_y)
            && row < self.storage_rows
        {
            let start = row.checked_mul(self.stride)?;
            return self.samples.get(start..start.checked_add(self.width)?);
        }
        if self.secondary.is_empty() {
            return None;
        }
        let row = y.checked_sub(self.origin_y)?;
        let start = row.checked_mul(self.width)?;
        self.secondary.get(start..start.checked_add(self.width)?)
    }

    #[cfg(test)]
    pub(crate) fn get(self, x: isize, y: isize) -> Option<i32> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        if x >= self.width || y >= self.height {
            return None;
        }
        self.row(y)
            .and_then(|row| row.get(x))
            .map(|value| i32::from(value.to_u16()))
    }
}

impl<'a, T: ReconSample> PackedPlane<'a, T> {
    pub(crate) const fn width(self) -> usize {
        self.width
    }

    pub(crate) const fn frame_height(self) -> usize {
        self.height
    }

    pub(crate) const fn stride(self) -> usize {
        self.stride
    }

    pub(crate) const fn origin_y(self) -> usize {
        self.origin_y
    }

    pub(crate) const fn end_y(self) -> usize {
        self.origin_y + self.rows
    }

    pub(crate) const fn samples(self) -> &'a [T] {
        self.samples
    }

    pub(crate) fn row(self, y: usize) -> Option<&'a [T]> {
        let row = y.checked_sub(self.origin_y)?;
        if row >= self.rows {
            return None;
        }
        let start = row.checked_mul(self.stride)?;
        self.samples.get(start..start.checked_add(self.width)?)
    }

    #[cfg(test)]
    pub(crate) fn get(self, x: isize, y: isize) -> Option<i32> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        if x >= self.width || y >= self.height {
            return None;
        }
        self.row(y)
            .and_then(|row| row.get(x))
            .map(|value| i32::from(value.to_u16()))
    }
}

/// The deblocked planes one filter stripe reads.
///
/// The chain reads whole frame coordinates, so a stripe that reads from a
/// window of the deblocked frame reads exactly the same rows as one reading the
/// frame; a row outside the window is refused rather than aliased onto another.
#[derive(Clone, Copy)]
pub(crate) struct DeblockedPlanes<'a, T> {
    pub(crate) y: FramePlane<'a, T>,
    pub(crate) u: Option<FramePlane<'a, T>>,
    pub(crate) v: Option<FramePlane<'a, T>>,
}

#[cfg(test)]
impl<'a, T: ReconSample> DeblockedPlanes<'a, T> {
    /// Borrows a whole deblocked frame.
    pub(crate) fn frame(workspace: &'a CurrentFrameWorkspace<T>) -> Option<Self> {
        let has_chroma = !workspace.info().pixel_format().is_monochrome();
        Some(Self {
            y: FramePlane::new(workspace, PlaneId::Y)?,
            u: has_chroma
                .then(|| FramePlane::new(workspace, PlaneId::U))
                .flatten(),
            v: has_chroma
                .then(|| FramePlane::new(workspace, PlaneId::V))
                .flatten(),
        })
    }
}

/// Fewest deblocked-window buffers retained, and the floor the pool-width bound
/// never drops below.
const MIN_RETAINED_WINDOW_BUFFERS: usize = 64;
/// Window buffers retained per worker: one window per plane per filter chain in
/// flight, and a wide pool runs that many more chains at once.
const RETAINED_WINDOW_BUFFERS_PER_WORKER: usize = 8;

/// Retains one worker's share per worker, with [`MIN_RETAINED_WINDOW_BUFFERS`]
/// as the floor.
/// Scales per worker only on a pool thread; off-pool callers get the floor.
fn max_retained_window_buffers() -> usize {
    splot_parallel::current_pool_width()
        .saturating_mul(RETAINED_WINDOW_BUFFERS_PER_WORKER)
        .max(MIN_RETAINED_WINDOW_BUFFERS)
}
static WINDOW_SAMPLE_BUFFERS: Mutex<Vec<Box<dyn Any + Send>>> = Mutex::new(Vec::new());

fn take_window_buffer<T: ReconSample>(sample_count: usize) -> Result<Vec<T>, StripeCopyError> {
    let mut buffer = {
        let mut buffers = WINDOW_SAMPLE_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pool_is_full = buffers.len() >= max_retained_window_buffers();
        let index = select_buffer_index(
            buffers.iter().enumerate().filter_map(|(index, buffer)| {
                buffer
                    .downcast_ref::<Vec<T>>()
                    .map(|buffer| (index, buffer.capacity()))
            }),
            sample_count,
            pool_is_full,
        );
        index
            .map(|index| buffers.swap_remove(index))
            .and_then(|buffer| buffer.downcast::<Vec<T>>().ok())
            .map_or_else(Vec::new, |buffer| *buffer)
    };
    buffer.clear();
    buffer
        .try_reserve_exact(sample_count)
        .map_err(|_| StripeCopyError::Allocation(PlaneId::Y))?;
    Ok(buffer)
}

fn recycle_window_buffer<T: ReconSample>(mut buffer: Vec<T>) {
    if buffer.capacity() == 0 {
        return;
    }
    buffer.clear();
    let mut buffers = WINDOW_SAMPLE_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if buffers.len() < max_retained_window_buffers() && buffers.try_reserve(1).is_ok() {
        buffers.push(Box::new(buffer));
    } else {
        drop(buffer);
    }
}

/// One owned copy of the deblocked rows a run of filter stripes reads.
///
/// The stripe chain runs while the deblock is still filtering rows further down
/// the frame, so it cannot borrow the frame the deblock is writing. It owns its
/// rows instead, which is what lets the two overlap without either waiting for
/// the other.
pub(crate) struct DeblockedWindow<T: ReconSample> {
    planes: [Option<WindowPlane<T>>; 3],
}

struct WindowPlane<T: ReconSample> {
    parts: [Option<WindowPart<T>>; 2],
    width: usize,
    height: usize,
    origin_y: usize,
    rows: usize,
}

#[derive(Clone)]
struct WindowPart<T: ReconSample> {
    storage: Arc<WindowRows<T>>,
    origin_y: usize,
    rows: usize,
}

struct WindowRows<T: ReconSample> {
    samples: Vec<T>,
    width: usize,
    origin_y: usize,
    rows: usize,
}

pub(crate) struct DeblockedWindowSequence<T: ReconSample> {
    next_stripe: usize,
    boundaries: [Option<WindowPart<T>>; 3],
    #[cfg(test)]
    copied_samples: usize,
}

impl<T: ReconSample> Default for DeblockedWindowSequence<T> {
    fn default() -> Self {
        Self {
            next_stripe: 0,
            boundaries: std::array::from_fn(|_| None),
            #[cfg(test)]
            copied_samples: 0,
        }
    }
}

impl<T: ReconSample> DeblockedWindow<T> {
    /// Copies the deblocked rows `luma_start..luma_end`, widened by `margin`
    /// rows of each plane on both sides, out of the frame being deblocked.
    #[cfg(test)]
    pub(crate) fn extract(
        workspace: &CurrentFrameWorkspace<T>,
        luma_start: usize,
        luma_end: usize,
        margin: usize,
    ) -> Result<Self, StripeCopyError> {
        let format = workspace.info().pixel_format();
        let subsampling_y = usize::from(format.subsampling_y());
        let has_chroma = !format.is_monochrome();
        let mut planes = [None, None, None];
        for (index, plane) in [PlaneId::Y, PlaneId::U, PlaneId::V].into_iter().enumerate() {
            if index > 0 && !has_chroma {
                break;
            }
            let source = FramePlane::new(workspace, plane).ok_or(StripeCopyError::Geometry)?;
            let shift = if index == 0 { 0 } else { subsampling_y };
            let start = (luma_start >> shift).saturating_sub(margin);
            let end = (luma_end.div_ceil(1 << shift) + margin).min(source.frame_height());
            let storage = Arc::new(
                WindowRows::copy(source, start, end).map_err(|error| error.for_plane(plane))?,
            );
            let part = WindowPart::new(storage, start, end).ok_or(StripeCopyError::Geometry)?;
            planes[index] = Some(WindowPlane::single(
                part,
                source.width(),
                source.frame_height(),
                start,
                end,
            )?);
        }
        Ok(Self { planes })
    }

    /// Borrows the window as the deblocked planes a stripe reads.
    pub(crate) fn planes(&self) -> Option<DeblockedPlanes<'_, T>> {
        Some(DeblockedPlanes {
            y: self.planes[PlaneId::Y.index()].as_ref()?.plane()?,
            u: self.planes[PlaneId::U.index()]
                .as_ref()
                .and_then(WindowPlane::plane),
            v: self.planes[PlaneId::V.index()]
                .as_ref()
                .and_then(WindowPlane::plane),
        })
    }
}

impl<T: ReconSample> WindowPlane<T> {
    #[cfg(test)]
    fn single(
        part: WindowPart<T>,
        width: usize,
        height: usize,
        start: usize,
        end: usize,
    ) -> Result<Self, StripeCopyError> {
        Ok(Self {
            parts: [Some(part), None],
            width,
            height,
            origin_y: start,
            rows: end.checked_sub(start).ok_or(StripeCopyError::Geometry)?,
        })
    }

    fn plane(&self) -> Option<FramePlane<'_, T>> {
        let primary = self.parts[0].as_ref()?;
        let secondary = self.parts[1].as_ref();
        if primary.storage.width != self.width
            || secondary.is_some_and(|part| {
                part.storage.width != self.width
                    || part.origin_y != self.origin_y
                    || part.end_y() < primary.origin_y
            })
        {
            return None;
        }
        (self.width != 0 && self.origin_y.checked_add(self.rows)? <= self.height).then_some(
            FramePlane {
                width: self.width,
                height: self.height,
                stride: self.width,
                origin_y: self.origin_y,
                storage_origin_y: primary.origin_y,
                storage_rows: primary.rows,
                samples: primary.samples(),
                secondary: secondary.map_or(&[], WindowPart::samples),
            },
        )
    }
}

impl<T: ReconSample> WindowPart<T> {
    fn new(storage: Arc<WindowRows<T>>, start: usize, end: usize) -> Option<Self> {
        if start < storage.origin_y
            || end < start
            || end > storage.origin_y.checked_add(storage.rows)?
        {
            return None;
        }
        Some(Self {
            storage,
            origin_y: start,
            rows: end - start,
        })
    }

    fn end_y(&self) -> usize {
        self.origin_y + self.rows
    }

    fn samples(&self) -> &[T] {
        let start = (self.origin_y - self.storage.origin_y) * self.storage.width;
        let end = start + self.rows * self.storage.width;
        &self.storage.samples[start..end]
    }
}

impl<T: ReconSample> WindowRows<T> {
    #[cfg(test)]
    fn copy(source: FramePlane<'_, T>, start: usize, end: usize) -> Result<Self, StripeCopyError> {
        let geometry = StripeCopyError::Geometry;
        let rows = end.checked_sub(start).ok_or(geometry)?;
        let mut samples =
            take_window_buffer::<T>(rows.checked_mul(source.width()).ok_or(geometry)?)?;
        if let Some(source) = source.contiguous_rows(start, end) {
            samples.extend_from_slice(source);
        } else {
            for y in start..end {
                samples.extend_from_slice(source.row(y).ok_or(geometry)?);
            }
        }
        Ok(Self {
            samples,
            width: source.width(),
            origin_y: start,
            rows,
        })
    }

    fn copy_bands(
        frame: &OwnedFrameBands<T>,
        plane: PlaneId,
        start: usize,
        end: usize,
    ) -> Result<Self, StripeCopyError> {
        let geometry = StripeCopyError::Geometry;
        let bands = frame.plane_bands(plane).map_err(|_| geometry)?;
        let first = bands.first().ok_or(geometry)?;
        let storage = first.storage_size();
        if start > end || end > storage.height() {
            return Err(geometry);
        }
        let rows = end - start;
        let width = storage.width();
        let mut samples = take_window_buffer::<T>(rows.checked_mul(width).ok_or(geometry)?)?;
        let mut band_index = 0;
        for y in start..end {
            while bands
                .get(band_index)
                .is_some_and(|band| band.rect().y() + band.rect().height() <= y)
            {
                band_index += 1;
            }
            let band = bands.get(band_index).ok_or(geometry)?;
            let rect = band.rect();
            let local_y = y.checked_sub(rect.y()).ok_or(geometry)?;
            if local_y >= rect.height() || rect.width() != width {
                return Err(geometry);
            }
            let row_start = local_y.checked_mul(width).ok_or(geometry)?;
            samples.extend_from_slice(
                band.samples()
                    .get(row_start..row_start.checked_add(width).ok_or(geometry)?)
                    .ok_or(geometry)?,
            );
        }
        Ok(Self {
            samples,
            width,
            origin_y: start,
            rows,
        })
    }
}

impl<T: ReconSample> Drop for WindowRows<T> {
    fn drop(&mut self) {
        recycle_window_buffer(core::mem::take(&mut self.samples));
    }
}

impl<T: ReconSample> DeblockedWindowSequence<T> {
    #[cfg(test)]
    pub(crate) fn extract(
        &mut self,
        workspace: &CurrentFrameWorkspace<T>,
        ranges: &[(usize, usize)],
        stripe: usize,
        margin: usize,
    ) -> Result<DeblockedWindow<T>, StripeCopyError> {
        let format = workspace.info().pixel_format();
        self.extract_with(
            ranges,
            stripe,
            margin,
            format,
            |plane| {
                let source = FramePlane::new(workspace, plane)?;
                Some((source.width(), source.frame_height()))
            },
            |plane, start, end| {
                let source = FramePlane::new(workspace, plane).ok_or(StripeCopyError::Geometry)?;
                WindowRows::copy(source, start, end)
            },
        )
    }

    pub(crate) fn extract_bands(
        &mut self,
        frame: &OwnedFrameBands<T>,
        ranges: &[(usize, usize)],
        stripe: usize,
        margin: usize,
    ) -> Result<DeblockedWindow<T>, StripeCopyError> {
        let format = frame.info().pixel_format();
        self.extract_with(
            ranges,
            stripe,
            margin,
            format,
            |plane| {
                let size = frame.plane_bands(plane).ok()?.first()?.storage_size();
                Some((size.width(), size.height()))
            },
            |plane, start, end| WindowRows::copy_bands(frame, plane, start, end),
        )
    }

    fn extract_with(
        &mut self,
        ranges: &[(usize, usize)],
        stripe: usize,
        margin: usize,
        format: splot_recon::PixelFormat,
        mut dimensions: impl FnMut(PlaneId) -> Option<(usize, usize)>,
        mut copy: impl FnMut(PlaneId, usize, usize) -> Result<WindowRows<T>, StripeCopyError>,
    ) -> Result<DeblockedWindow<T>, StripeCopyError> {
        if stripe != self.next_stripe {
            return Err(StripeCopyError::Geometry);
        }
        let range = ranges
            .get(stripe)
            .copied()
            .ok_or(StripeCopyError::Geometry)?;
        if range.0 >= range.1
            || stripe
                .checked_sub(1)
                .and_then(|previous| ranges.get(previous))
                .is_some_and(|previous| previous.1 != range.0)
            || ranges.get(stripe + 1).is_some_and(|next| next.0 != range.1)
        {
            return Err(StripeCopyError::Geometry);
        }
        let subsampling_y = usize::from(format.subsampling_y());
        let has_chroma = !format.is_monochrome();
        let mut planes = std::array::from_fn(|_| None);
        let mut next_boundaries = std::array::from_fn(|_| None);
        #[cfg(test)]
        let mut copied_samples = 0usize;
        for (index, plane) in [PlaneId::Y, PlaneId::U, PlaneId::V].into_iter().enumerate() {
            if index > 0 && !has_chroma {
                break;
            }
            let (width, height) = dimensions(plane).ok_or(StripeCopyError::Geometry)?;
            let shift = if index == 0 { 0 } else { subsampling_y };
            let current = window_bounds(range, shift, margin, height)?;
            let top_bounds = stripe
                .checked_sub(1)
                .and_then(|previous| ranges.get(previous).copied())
                .map(|previous| window_bounds(previous, shift, margin, height))
                .transpose()?
                .and_then(|previous| intersect_rows(previous, current));
            let bottom_bounds = ranges
                .get(stripe + 1)
                .copied()
                .map(|next| window_bounds(next, shift, margin, height))
                .transpose()?
                .and_then(|next| intersect_rows(current, next));
            let top = match (top_bounds, self.boundaries[index].as_ref()) {
                (None, None) => None,
                (Some((start, end)), Some(part))
                    if part.origin_y == start && part.end_y() == end =>
                {
                    Some(part.clone())
                }
                _ => return Err(StripeCopyError::Geometry),
            };
            let top_end = top_bounds.map_or(current.0, |(_, end)| end);
            let reuse_top = top.is_some()
                && bottom_bounds.is_none_or(|(bottom_start, _)| bottom_start >= top_end);
            let top = reuse_top.then_some(top).flatten();
            let fresh_start = top.as_ref().map_or(current.0, WindowPart::end_y);
            let fresh = if fresh_start < current.1 {
                let rows = Arc::new(
                    copy(plane, fresh_start, current.1).map_err(|error| error.for_plane(plane))?,
                );
                #[cfg(test)]
                {
                    copied_samples += rows.samples.len();
                }
                Some(
                    WindowPart::new(Arc::clone(&rows), fresh_start, current.1)
                        .ok_or(StripeCopyError::Geometry)?,
                )
            } else {
                None
            };
            let bottom = bottom_bounds
                .map(|(start, end)| {
                    let storage = fresh
                        .as_ref()
                        .map(|part| Arc::clone(&part.storage))
                        .ok_or(StripeCopyError::Geometry)?;
                    WindowPart::new(storage, start, end).ok_or(StripeCopyError::Geometry)
                })
                .transpose()?;
            next_boundaries[index] = bottom;
            let (primary, secondary) = match (fresh, top) {
                (Some(fresh), top) => (fresh, top),
                (None, Some(top)) => (top, None),
                (None, None) => return Err(StripeCopyError::Geometry),
            };
            planes[index] = Some(WindowPlane {
                parts: [Some(primary), secondary],
                width,
                height,
                origin_y: current.0,
                rows: current.1 - current.0,
            });
        }
        self.boundaries = next_boundaries;
        self.next_stripe += 1;
        #[cfg(test)]
        {
            self.copied_samples += copied_samples;
        }
        Ok(DeblockedWindow { planes })
    }

    #[cfg(test)]
    pub(crate) const fn copied_samples(&self) -> usize {
        self.copied_samples
    }
}

fn window_bounds(
    range: (usize, usize),
    shift: usize,
    margin: usize,
    height: usize,
) -> Result<(usize, usize), StripeCopyError> {
    let scale = 1usize
        .checked_shl(u32::try_from(shift).map_err(|_| StripeCopyError::Geometry)?)
        .ok_or(StripeCopyError::Geometry)?;
    let start = (range.0 >> shift).saturating_sub(margin);
    let end = range
        .1
        .div_ceil(scale)
        .checked_add(margin)
        .ok_or(StripeCopyError::Geometry)?
        .min(height);
    (start < end && end <= height)
        .then_some((start, end))
        .ok_or(StripeCopyError::Geometry)
}

fn intersect_rows(left: (usize, usize), right: (usize, usize)) -> Option<(usize, usize)> {
    let start = left.0.max(right.0);
    let end = left.1.min(right.1);
    (start < end).then_some((start, end))
}

enum StripeOwner {
    Owned(Vec<u16>),
    DirectU16 {
        _target: crate::pipeline::frame_progress::DirectPlaneTarget,
    },
    DirectU8 {
        target: crate::pipeline::frame_progress::DirectPlaneTarget,
        staging: Vec<u16>,
    },
}

struct StripeSamples {
    samples: core::ptr::NonNull<[u16]>,
    owner: StripeOwner,
}

// SAFETY: the `Send` owner keeps the cached pointer's allocation alive.
unsafe impl Send for StripeSamples {}

impl StripeSamples {
    fn owned(mut samples: Vec<u16>) -> Self {
        let pointer = core::ptr::NonNull::from(samples.as_mut_slice());
        Self {
            samples: pointer,
            owner: StripeOwner::Owned(samples),
        }
    }

    fn direct_u16(
        mut target: crate::pipeline::frame_progress::DirectPlaneTarget,
    ) -> Result<Self, StripeCopyError> {
        let samples = target.u16_samples_mut().ok_or(StripeCopyError::Geometry)?;
        let pointer = core::ptr::NonNull::from(samples);
        Ok(Self {
            samples: pointer,
            owner: StripeOwner::DirectU16 { _target: target },
        })
    }

    fn direct_u8_from_frame<T: ReconSample>(
        mut target: crate::pipeline::frame_progress::DirectPlaneTarget,
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
    ) -> Result<Self, StripeCopyError> {
        if target.u8_samples_mut().is_none() {
            return Err(StripeCopyError::Geometry);
        }
        let sample_count = end_y
            .checked_sub(origin_y)
            .and_then(|rows| rows.checked_mul(source.width()))
            .ok_or(StripeCopyError::Geometry)?;
        if target.len() != sample_count {
            return Err(StripeCopyError::Geometry);
        }
        let mut staging = take_stripe_sample_buffer(sample_count)?;
        let initialized = (|| {
            let destination = staging
                .spare_capacity_mut()
                .get_mut(..sample_count)
                .ok_or(StripeCopyError::Geometry)?;
            if let Some((upper, lower)) = source.u16_row_spans(origin_y, end_y) {
                let split = upper.len();
                write_uninit_u16(
                    destination
                        .get_mut(..split)
                        .ok_or(StripeCopyError::Geometry)?,
                    upper,
                )?;
                write_uninit_u16(
                    destination
                        .get_mut(split..)
                        .ok_or(StripeCopyError::Geometry)?,
                    lower,
                )?;
                return Ok(());
            }
            let mut written = 0usize;
            for y in origin_y..end_y {
                let row = source.row(y).ok_or(StripeCopyError::Geometry)?;
                let end = written
                    .checked_add(row.len())
                    .ok_or(StripeCopyError::Geometry)?;
                let output = destination
                    .get_mut(written..end)
                    .ok_or(StripeCopyError::Geometry)?;
                for (output, &sample) in output.iter_mut().zip(row) {
                    output.write(sample.to_u16());
                }
                written = end;
            }
            (written == sample_count)
                .then_some(())
                .ok_or(StripeCopyError::Geometry)
        })();
        if let Err(error) = initialized {
            recycle_stripe_sample_buffer(staging);
            return Err(error);
        }
        unsafe { staging.set_len(sample_count) }; // SAFETY: the live pooled allocation has length zero and sufficient capacity; every in-bounds spare slot is initialized without a `&mut [u16]` before this non-panicking call, errors recycle and panics drop length-zero storage, and the owner prevents reallocation until the fully initialized vector is cleared and recycled.
        let pointer = core::ptr::NonNull::from(staging.as_mut_slice());
        Ok(Self {
            samples: pointer,
            owner: StripeOwner::DirectU8 { target, staging },
        })
    }

    fn direct_u8_from_u16_slice(
        mut target: crate::pipeline::frame_progress::DirectPlaneTarget,
        source: &[u16],
    ) -> Result<Self, StripeCopyError> {
        if target.u8_samples_mut().is_none() || target.len() != source.len() {
            return Err(StripeCopyError::Geometry);
        }
        let mut staging = take_stripe_sample_buffer(source.len())?;
        let initialized = staging
            .spare_capacity_mut()
            .get_mut(..source.len())
            .ok_or(StripeCopyError::Geometry)
            .and_then(|destination| write_uninit_u16(destination, source));
        if let Err(error) = initialized {
            recycle_stripe_sample_buffer(staging);
            return Err(error);
        }
        unsafe { staging.set_len(source.len()) }; // SAFETY: the live pooled allocation has length zero and sufficient capacity; the equal-length helper initialized every in-bounds spare slot without a `&mut [u16]` before this non-panicking call, errors recycle and panics drop length-zero storage, and the owner prevents reallocation until the fully initialized vector is cleared and recycled.
        let pointer = core::ptr::NonNull::from(staging.as_mut_slice());
        Ok(Self {
            samples: pointer,
            owner: StripeOwner::DirectU8 { target, staging },
        })
    }

    #[inline]
    fn as_slice(&self) -> &[u16] {
        unsafe {
            // SAFETY: the owner stays alive and cannot reallocate.
            self.samples.as_ref()
        }
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [u16] {
        unsafe {
            // SAFETY: the owner and this borrow are both exclusive.
            self.samples.as_mut()
        }
    }

    fn is_direct(&self) -> bool {
        !matches!(self.owner, StripeOwner::Owned(_))
    }

    fn finish_direct(&mut self) -> Result<(), StripeCopyError> {
        let StripeOwner::DirectU8 { target, staging } = &mut self.owner else {
            return Ok(());
        };
        let unpublished_destination = target.u8_samples_mut().ok_or(StripeCopyError::Geometry)?;
        if unpublished_destination.len() != staging.len() {
            return Err(StripeCopyError::Geometry);
        }
        let mut discarded_high_bits = 0u16;
        for (destination, &sample) in unpublished_destination.iter_mut().zip(staging.iter()) {
            discarded_high_bits |= sample & !u16::from(u8::MAX);
            *destination = sample as u8;
        }
        (discarded_high_bits == 0)
            .then_some(())
            .ok_or(StripeCopyError::Geometry)
    }
}

fn write_uninit_u16(
    destination: &mut [MaybeUninit<u16>],
    source: &[u16],
) -> Result<(), StripeCopyError> {
    if destination.len() != source.len() {
        return Err(StripeCopyError::Geometry);
    }
    for (destination, &source) in destination.iter_mut().zip(source) {
        destination.write(source);
    }
    Ok(())
}

impl Drop for StripeSamples {
    fn drop(&mut self) {
        match &mut self.owner {
            StripeOwner::Owned(samples) => {
                recycle_stripe_sample_buffer(core::mem::take(samples));
            }
            StripeOwner::DirectU8 { staging, .. } => {
                recycle_stripe_sample_buffer(core::mem::take(staging));
            }
            StripeOwner::DirectU16 { .. } => {}
        }
    }
}

pub(crate) struct StripePlane {
    width: usize,
    frame_height: usize,
    origin_y: usize,
    samples: StripeSamples,
}

pub(crate) enum StripeOutputPlane {
    U16(StripePlane),
    DirectU8(crate::pipeline::frame_progress::DirectPlaneTarget),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StripeInitialization {
    CopyAll,
    /// The caller proves every destination sample is written before publication.
    FullyOverwritten,
}

impl StripePlane {
    #[cfg(test)]
    pub(crate) fn from_samples(
        width: usize,
        frame_height: usize,
        origin_y: usize,
        samples: Vec<u16>,
    ) -> Option<Self> {
        if width == 0 || !samples.len().is_multiple_of(width) {
            return None;
        }
        let height = samples.len() / width;
        if origin_y.checked_add(height)? > frame_height {
            return None;
        }
        Some(Self {
            width,
            frame_height,
            origin_y,
            samples: StripeSamples::owned(samples),
        })
    }

    fn from_target(
        target: crate::pipeline::frame_progress::DirectPlaneTarget,
    ) -> Result<Self, StripeCopyError> {
        Ok(Self {
            width: target.width(),
            frame_height: target.frame_height(),
            origin_y: target.origin_y(),
            samples: StripeSamples::direct_u16(target)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn copy_from<T: ReconSample>(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
    ) -> Result<Self, StripeCopyError> {
        Self::copy_from_into(source, origin_y, end_y, None)
    }

    #[cfg(test)]
    pub(crate) fn copy_from_into<T: ReconSample>(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
        target: Option<crate::pipeline::frame_progress::DirectPlaneTarget>,
    ) -> Result<Self, StripeCopyError> {
        Self::copy_from_into_mode(
            source,
            origin_y,
            end_y,
            target,
            StripeInitialization::CopyAll,
        )
    }

    pub(crate) fn preflight_copy_from_into<T: ReconSample>(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
        target: Option<&crate::pipeline::frame_progress::DirectPlaneTarget>,
        initialization: StripeInitialization,
    ) -> Result<(), StripeCopyError> {
        let geometry = StripeCopyError::Geometry;
        if origin_y > end_y || end_y > source.frame_height() {
            return Err(geometry);
        }
        let sample_count = source
            .width()
            .checked_mul(end_y - origin_y)
            .ok_or(geometry)?;
        match target {
            Some(target) => (target.width() == source.width()
                && target.frame_height() == source.frame_height()
                && target.origin_y() == origin_y
                && target.len() == sample_count
                && (initialization == StripeInitialization::CopyAll || target.is_u16()))
            .then_some(())
            .ok_or(geometry),
            None if initialization == StripeInitialization::FullyOverwritten => Err(geometry),
            None => Ok(()),
        }
    }

    pub(crate) fn copy_from_into_mode<T: ReconSample>(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
        target: Option<crate::pipeline::frame_progress::DirectPlaneTarget>,
        initialization: StripeInitialization,
    ) -> Result<Self, StripeCopyError> {
        Self::preflight_copy_from_into(source, origin_y, end_y, target.as_ref(), initialization)?;
        let geometry = StripeCopyError::Geometry;
        let sample_count = source
            .width()
            .checked_mul(end_y - origin_y)
            .ok_or(geometry)?;
        let mut output = match target {
            Some(target)
                if target.width() == source.width()
                    && target.frame_height() == source.frame_height()
                    && target.origin_y() == origin_y
                    && target.len() == sample_count =>
            {
                if target.is_u16() {
                    Self::from_target(target)?
                } else {
                    if initialization != StripeInitialization::CopyAll {
                        return Err(geometry);
                    }
                    return Ok(Self {
                        width: source.width(),
                        frame_height: source.frame_height(),
                        origin_y,
                        samples: StripeSamples::direct_u8_from_frame(
                            target, source, origin_y, end_y,
                        )?,
                    });
                }
            }
            Some(_) => return Err(geometry),
            None => {
                let mut samples = take_stripe_sample_buffer(sample_count)?;
                if source
                    .append_u16_rows(origin_y, end_y, &mut samples)
                    .is_none()
                {
                    for y in origin_y..end_y {
                        let Some(row) = source.row(y) else {
                            recycle_stripe_sample_buffer(samples);
                            return Err(geometry);
                        };
                        if let Some(row) = T::u16_slice(row) {
                            samples.extend_from_slice(row);
                        } else {
                            samples.extend(row.iter().map(|sample| sample.to_u16()));
                        }
                    }
                }
                if samples.len() != sample_count {
                    recycle_stripe_sample_buffer(samples);
                    return Err(geometry);
                }
                return Ok(Self {
                    width: source.width(),
                    frame_height: source.frame_height(),
                    origin_y,
                    samples: StripeSamples::owned(samples),
                });
            }
        };
        if initialization == StripeInitialization::FullyOverwritten {
            return Ok(output);
        }
        let samples = output.samples_mut();
        if source.copy_u16_rows(origin_y, end_y, samples).is_none() {
            let mut written = 0usize;
            for y in origin_y..end_y {
                let row = source.row(y).ok_or(geometry)?;
                let destination = samples
                    .get_mut(written..written.checked_add(row.len()).ok_or(geometry)?)
                    .ok_or(geometry)?;
                if let Some(row) = T::u16_slice(row) {
                    destination.copy_from_slice(row);
                } else {
                    for (destination, source) in destination.iter_mut().zip(row) {
                        *destination = source.to_u16();
                    }
                }
                written += row.len();
            }
            if written != sample_count {
                return Err(geometry);
            }
        }
        Ok(output)
    }

    pub(crate) fn copy_rows_into(
        &self,
        origin_y: usize,
        end_y: usize,
        target: Option<crate::pipeline::frame_progress::DirectPlaneTarget>,
    ) -> Result<Self, StripeCopyError> {
        let geometry = StripeCopyError::Geometry;
        let start = origin_y
            .checked_sub(self.origin_y)
            .ok_or(geometry)?
            .checked_mul(self.width)
            .ok_or(geometry)?;
        let end = end_y
            .checked_sub(self.origin_y)
            .ok_or(geometry)?
            .checked_mul(self.width)
            .ok_or(geometry)?;
        let source = self.samples().get(start..end).ok_or(geometry)?;
        let mut output = match target {
            Some(target)
                if target.width() == self.width
                    && target.frame_height() == self.frame_height
                    && target.origin_y() == origin_y
                    && target.len() == source.len() =>
            {
                if target.is_u16() {
                    Self::from_target(target)?
                } else {
                    return Ok(Self {
                        width: self.width,
                        frame_height: self.frame_height,
                        origin_y,
                        samples: StripeSamples::direct_u8_from_u16_slice(target, source)?,
                    });
                }
            }
            Some(_) => return Err(geometry),
            None => {
                let mut samples = take_stripe_sample_buffer(source.len())?;
                samples.extend_from_slice(source);
                return Ok(Self {
                    width: self.width,
                    frame_height: self.frame_height,
                    origin_y,
                    samples: StripeSamples::owned(samples),
                });
            }
        };
        output.samples_mut().copy_from_slice(source);
        Ok(output)
    }

    pub(crate) const fn width(&self) -> usize {
        self.width
    }

    pub(crate) const fn frame_height(&self) -> usize {
        self.frame_height
    }

    pub(crate) const fn origin_y(&self) -> usize {
        self.origin_y
    }

    pub(crate) fn end_y(&self) -> Option<usize> {
        self.origin_y
            .checked_add(self.samples().len().checked_div(self.width)?)
    }

    #[inline]
    pub(crate) fn samples(&self) -> &[u16] {
        self.samples.as_slice()
    }

    #[inline]
    pub(crate) fn samples_mut(&mut self) -> &mut [u16] {
        self.samples.as_mut_slice()
    }

    #[inline]
    pub(crate) fn row(&self, y: usize) -> Option<&[u16]> {
        let row = y.checked_sub(self.origin_y)?;
        let start = row.checked_mul(self.width)?;
        self.samples().get(start..start.checked_add(self.width)?)
    }

    #[inline]
    pub(crate) fn row_mut(&mut self, y: usize) -> Option<&mut [u16]> {
        let row = y.checked_sub(self.origin_y)?;
        let width = self.width;
        let start = row.checked_mul(width)?;
        self.samples_mut().get_mut(start..start.checked_add(width)?)
    }

    pub(crate) fn write_rect(
        &mut self,
        rect: PlaneRect,
        samples: &[u16],
        stride: usize,
    ) -> Option<()> {
        for row in 0..rect.height() {
            let src_start = row.checked_mul(stride)?;
            let src = samples.get(src_start..src_start.checked_add(rect.width())?)?;
            let dst = self.row_mut(rect.y().checked_add(row)?)?;
            dst.get_mut(rect.x()..rect.x().checked_add(rect.width())?)?
                .copy_from_slice(src);
        }
        Some(())
    }

    #[inline]
    pub(crate) fn rect_mut(&mut self, rect: PlaneRect) -> Option<(&mut [u16], usize)> {
        if rect.x().checked_add(rect.width())? > self.width {
            return None;
        }
        let row = rect.y().checked_sub(self.origin_y)?;
        let start = row.checked_mul(self.width)?.checked_add(rect.x())?;
        let end = rect
            .height()
            .checked_sub(1)?
            .checked_mul(self.width)?
            .checked_add(start)?
            .checked_add(rect.width())?;
        let width = self.width;
        Some((self.samples_mut().get_mut(start..end)?, width))
    }

    pub(crate) fn is_direct(&self) -> bool {
        self.samples.is_direct()
    }

    pub(crate) fn finish_direct(&mut self) -> Result<(), StripeCopyError> {
        self.samples.finish_direct()
    }
}

impl StripeOutputPlane {
    pub(crate) const fn u16(plane: StripePlane) -> Self {
        Self::U16(plane)
    }

    pub(crate) fn direct_u8(
        mut target: crate::pipeline::frame_progress::DirectPlaneTarget,
        source: &StripePlane,
    ) -> Result<Self, StripeCopyError> {
        let end_y = source.end_y().ok_or(StripeCopyError::Geometry)?;
        if target.is_u16()
            || target.width() != source.width()
            || target.frame_height() != source.frame_height()
            || target.origin_y() != source.origin_y()
            || target.len() != source.samples().len()
            || target.u8_samples_mut().is_none()
            || end_y > source.frame_height()
        {
            return Err(StripeCopyError::Geometry);
        }
        Ok(Self::DirectU8(target))
    }

    pub(crate) const fn width(&self) -> usize {
        match self {
            Self::U16(plane) => plane.width(),
            Self::DirectU8(target) => target.width(),
        }
    }

    pub(crate) const fn frame_height(&self) -> usize {
        match self {
            Self::U16(plane) => plane.frame_height(),
            Self::DirectU8(target) => target.frame_height(),
        }
    }

    pub(crate) const fn origin_y(&self) -> usize {
        match self {
            Self::U16(plane) => plane.origin_y(),
            Self::DirectU8(target) => target.origin_y(),
        }
    }

    pub(crate) fn end_y(&self) -> Option<usize> {
        match self {
            Self::U16(plane) => plane.end_y(),
            Self::DirectU8(target) => target.end_y(),
        }
    }

    pub(crate) const fn as_u16(&self) -> Option<&StripePlane> {
        match self {
            Self::U16(plane) => Some(plane),
            Self::DirectU8(_) => None,
        }
    }

    pub(crate) const fn as_u16_mut(&mut self) -> Option<&mut StripePlane> {
        match self {
            Self::U16(plane) => Some(plane),
            Self::DirectU8(_) => None,
        }
    }

    pub(crate) fn u8_rect_mut(&mut self, rect: PlaneRect) -> Option<(&mut [u8], usize)> {
        let Self::DirectU8(target) = self else {
            return None;
        };
        let width = target.width();
        if rect.x().checked_add(rect.width())? > width {
            return None;
        }
        let row = rect.y().checked_sub(target.origin_y())?;
        let start = row.checked_mul(width)?.checked_add(rect.x())?;
        let end = rect
            .height()
            .checked_sub(1)?
            .checked_mul(width)?
            .checked_add(start)?
            .checked_add(rect.width())?;
        Some((target.u8_samples_mut()?.get_mut(start..end)?, width))
    }

    pub(crate) fn is_direct(&self) -> bool {
        match self {
            Self::U16(plane) => plane.is_direct(),
            Self::DirectU8(_) => true,
        }
    }

    pub(crate) fn finish_direct(&mut self) -> Result<(), StripeCopyError> {
        match self {
            Self::U16(plane) => plane.finish_direct(),
            Self::DirectU8(_) => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
