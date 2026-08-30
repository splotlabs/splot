// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(
    unsafe_code,
    reason = "stripe buffers and final deblocked-row leases retain their unique allocation owner"
)]

use splot_recon::{CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample};
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
            storage_origin_y: start,
            storage_rows: end - start,
            samples,
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

/// A frame plane backed by one contiguous row span.
///
/// `height` is always the plane's frame height, so callers keep reasoning in
/// frame coordinates; `origin_y` and `rows` name the window the view actually
/// carries; accesses outside that logical range fail closed.
#[derive(Clone, Copy)]
pub(crate) struct FramePlane<'a, T> {
    width: usize,
    height: usize,
    stride: usize,
    storage_origin_y: usize,
    storage_rows: usize,
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
            storage_origin_y: 0,
            storage_rows: size.height(),
            samples: source.samples(),
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
            storage_origin_y: origin_y,
            storage_rows: rows,
            samples,
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
        self.storage_origin_y
    }

    /// The exclusive last plane row this view carries.
    pub(crate) const fn end_y(self) -> usize {
        self.storage_origin_y + self.storage_rows
    }

    pub(crate) fn contiguous_rows(self, origin_y: usize, end_y: usize) -> Option<&'a [T]> {
        if self.stride != self.width || origin_y > end_y {
            return None;
        }
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
        None
    }

    fn u16_rows(self, origin_y: usize, end_y: usize) -> Option<&'a [u16]> {
        let expected = end_y.checked_sub(origin_y)?.checked_mul(self.width)?;
        let source = self
            .contiguous_rows(origin_y, end_y)
            .and_then(T::u16_slice)?;
        (source.len() == expected).then_some(source)
    }

    fn copy_u16_rows(self, origin_y: usize, end_y: usize, destination: &mut [u16]) -> Option<()> {
        let source = self.u16_rows(origin_y, end_y)?;
        if destination.len() != source.len() {
            return None;
        }
        destination.copy_from_slice(source);
        Some(())
    }

    fn append_u16_rows(
        self,
        origin_y: usize,
        end_y: usize,
        destination: &mut Vec<u16>,
    ) -> Option<()> {
        destination.extend_from_slice(self.u16_rows(origin_y, end_y)?);
        Some(())
    }

    pub(crate) fn row(self, y: usize) -> Option<&'a [T]> {
        if let Some(row) = y.checked_sub(self.storage_origin_y)
            && row < self.storage_rows
        {
            let start = row.checked_mul(self.stride)?;
            return self.samples.get(start..start.checked_add(self.width)?);
        }
        None
    }

    pub(crate) const fn samples(self) -> &'a [T] {
        self.samples
    }

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
            if let Some(source) = source.u16_rows(origin_y, end_y) {
                write_uninit_u16(destination, source)?;
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

    #[cfg(test)]
    pub(crate) fn copy_rows_into(
        &self,
        origin_y: usize,
        end_y: usize,
        target: Option<crate::pipeline::frame_progress::DirectPlaneTarget>,
    ) -> Result<Self, StripeCopyError> {
        self.copy_rows_into_mode(origin_y, end_y, target, StripeInitialization::CopyAll)
    }

    pub(crate) fn preflight_copy_rows_into(
        &self,
        origin_y: usize,
        end_y: usize,
        target: Option<&crate::pipeline::frame_progress::DirectPlaneTarget>,
        initialization: StripeInitialization,
    ) -> Result<(), StripeCopyError> {
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
        match target {
            Some(target) => (target.width() == self.width
                && target.frame_height() == self.frame_height
                && target.origin_y() == origin_y
                && target.len() == source.len()
                && (initialization == StripeInitialization::CopyAll || target.is_u16()))
            .then_some(())
            .ok_or(geometry),
            None if initialization == StripeInitialization::FullyOverwritten => Err(geometry),
            None => Ok(()),
        }
    }

    pub(crate) fn copy_rows_into_mode(
        &self,
        origin_y: usize,
        end_y: usize,
        target: Option<crate::pipeline::frame_progress::DirectPlaneTarget>,
        initialization: StripeInitialization,
    ) -> Result<Self, StripeCopyError> {
        self.preflight_copy_rows_into(origin_y, end_y, target.as_ref(), initialization)?;
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
        if initialization == StripeInitialization::FullyOverwritten {
            return Ok(output);
        }
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

    #[cfg(test)]
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
