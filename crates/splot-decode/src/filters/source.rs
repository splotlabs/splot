// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{CurrentFrameWorkspace, OwnedFrameBands, PlaneId, PlaneRect, ReconSample};
use std::any::Any;
use std::sync::{Mutex, MutexGuard};

const MAX_RETAINED_STRIPE_BUFFERS: usize = 128;
static STRIPE_SAMPLE_BUFFERS: Mutex<Vec<Vec<u16>>> = Mutex::new(Vec::new());

fn lock_stripe_sample_buffers() -> MutexGuard<'static, Vec<Vec<u16>>> {
    STRIPE_SAMPLE_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    let pool_is_full = buffers.len() >= MAX_RETAINED_STRIPE_BUFFERS;
    let mut fitting = None;
    let mut fallback = None;
    for (index, buffer) in buffers.iter().enumerate() {
        let capacity = buffer.capacity();
        if capacity >= sample_count {
            if fitting.is_none_or(|(_, best_capacity)| capacity < best_capacity) {
                fitting = Some((index, capacity));
            }
        } else if pool_is_full && fallback.is_none_or(|(_, best_capacity)| capacity > best_capacity)
        {
            fallback = Some((index, capacity));
        }
    }
    let index = fitting.or(fallback).map(|(index, _)| index);
    let mut buffer = index
        .map(|index| buffers.swap_remove(index))
        .unwrap_or_default();
    drop(buffers);
    buffer.clear();
    buffer
        .try_reserve_exact(sample_count)
        .map_err(|_| StripeCopyError::Allocation(PlaneId::Y))?;
    Ok(buffer)
}

fn recycle_stripe_sample_buffer(mut buffer: Vec<u16>) {
    if buffer.capacity() == 0 {
        return;
    }
    buffer.clear();
    let mut buffers = lock_stripe_sample_buffers();
    if buffers.len() < MAX_RETAINED_STRIPE_BUFFERS && buffers.try_reserve(1).is_ok() {
        buffers.push(buffer);
    }
}

/// A read view of one frame plane, or of a contiguous row window of it.
///
/// `height` is always the plane's frame height, so callers keep reasoning in
/// frame coordinates; `origin_y` and `rows` name the window the view actually
/// carries. A whole-plane view covers `0..height`, and a window view refuses
/// every row outside its own range, which is what makes a windowed source fail
/// closed instead of reading a neighbour's samples.
#[derive(Clone, Copy)]
pub(crate) struct FramePlane<'a, T> {
    width: usize,
    height: usize,
    stride: usize,
    origin_y: usize,
    rows: usize,
    samples: &'a [T],
}

impl<'a, T: ReconSample> FramePlane<'a, T> {
    pub(crate) fn new(workspace: &'a CurrentFrameWorkspace<T>, plane: PlaneId) -> Option<Self> {
        let source = workspace.plane(plane).ok()?;
        let size = source.storage_size();
        Some(Self {
            width: size.width(),
            height: size.height(),
            stride: source.stride_samples(),
            origin_y: 0,
            rows: size.height(),
            samples: source.samples(),
        })
    }

    /// Views `samples` as the plane rows `origin_y..origin_y + rows` of a plane
    /// `width` wide and `height` tall, packed at `width` samples per row.
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
            rows,
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
        self.origin_y
    }

    /// The exclusive last plane row this view carries.
    pub(crate) const fn end_y(self) -> usize {
        self.origin_y + self.rows
    }

    pub(crate) const fn samples(self) -> &'a [T] {
        self.samples
    }

    fn contiguous_rows(self, origin_y: usize, end_y: usize) -> Option<&'a [T]> {
        if self.stride != self.width || origin_y > end_y {
            return None;
        }
        let start = origin_y
            .checked_sub(self.origin_y)?
            .checked_mul(self.stride)?;
        let end = end_y.checked_sub(self.origin_y)?.checked_mul(self.stride)?;
        if end_y > self.end_y() {
            return None;
        }
        self.samples.get(start..end)
    }

    pub(crate) fn row(self, y: usize) -> Option<&'a [T]> {
        let row = y.checked_sub(self.origin_y)?;
        if row >= self.rows {
            return None;
        }
        let start = row.checked_mul(self.stride)?;
        self.samples.get(start..start.checked_add(self.width)?)
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

const MAX_RETAINED_WINDOW_BUFFERS: usize = 64;
static WINDOW_SAMPLE_BUFFERS: Mutex<Vec<Box<dyn Any + Send>>> = Mutex::new(Vec::new());

fn take_window_buffer<T: ReconSample>(sample_count: usize) -> Result<Vec<T>, StripeCopyError> {
    let mut buffer = {
        let mut buffers = WINDOW_SAMPLE_BUFFERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pool_is_full = buffers.len() >= MAX_RETAINED_WINDOW_BUFFERS;
        let mut fitting = None;
        let mut fallback = None;
        for (index, buffer) in buffers.iter().enumerate() {
            let Some(buffer) = buffer.downcast_ref::<Vec<T>>() else {
                continue;
            };
            let capacity = buffer.capacity();
            if capacity >= sample_count {
                if fitting.is_none_or(|(_, best_capacity)| capacity < best_capacity) {
                    fitting = Some((index, capacity));
                }
            } else if pool_is_full
                && fallback.is_none_or(|(_, best_capacity)| capacity > best_capacity)
            {
                fallback = Some((index, capacity));
            }
        }
        let index = fitting.or(fallback).map(|(index, _)| index);
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
    if buffers.len() < MAX_RETAINED_WINDOW_BUFFERS && buffers.try_reserve(1).is_ok() {
        buffers.push(Box::new(buffer));
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
    samples: Vec<T>,
    width: usize,
    height: usize,
    origin_y: usize,
    rows: usize,
}

impl<T: ReconSample> DeblockedWindow<T> {
    /// Copies the deblocked rows `luma_start..luma_end`, widened by `margin`
    /// rows of each plane on both sides, out of the frame being deblocked.
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
            planes[index] = Some(
                WindowPlane::copy(source, start, end).map_err(|error| error.for_plane(plane))?,
            );
        }
        Ok(Self { planes })
    }

    /// Copies a deblocked stripe window directly from segmented canonical row
    /// bands without assembling a raw frame.
    pub(crate) fn extract_bands(
        frame: &OwnedFrameBands<T>,
        luma_start: usize,
        luma_end: usize,
        margin: usize,
    ) -> Result<Self, StripeCopyError> {
        let info = frame.info();
        let format = info.pixel_format();
        let luma = info.coded_luma_size();
        let chroma = format
            .chroma_size(luma)
            .map_err(|_| StripeCopyError::Geometry)?;
        let subsampling_y = usize::from(format.subsampling_y());
        let has_chroma = !format.is_monochrome();
        let mut planes = [None, None, None];
        for (index, plane) in [PlaneId::Y, PlaneId::U, PlaneId::V].into_iter().enumerate() {
            if index > 0 && !has_chroma {
                break;
            }
            let size = if index == 0 {
                luma
            } else {
                chroma.ok_or(StripeCopyError::Geometry)?
            };
            let shift = if index == 0 { 0 } else { subsampling_y };
            let start = (luma_start >> shift).saturating_sub(margin);
            let end = (luma_end.div_ceil(1 << shift) + margin).min(size.height());
            planes[index] = Some(
                WindowPlane::copy_bands(frame, plane, start, end)
                    .map_err(|error| error.for_plane(plane))?,
            );
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
            height: source.frame_height(),
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
            height: storage.height(),
            origin_y: start,
            rows,
        })
    }

    fn plane(&self) -> Option<FramePlane<'_, T>> {
        FramePlane::window(
            &self.samples,
            self.width,
            self.height,
            self.origin_y,
            self.rows,
        )
    }
}

impl<T: ReconSample> Drop for WindowPlane<T> {
    fn drop(&mut self) {
        recycle_window_buffer(core::mem::take(&mut self.samples));
    }
}

pub(crate) struct StripePlane {
    width: usize,
    frame_height: usize,
    origin_y: usize,
    samples: Vec<u16>,
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
            samples,
        })
    }

    pub(crate) fn copy_from<T: ReconSample>(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
    ) -> Result<Self, StripeCopyError> {
        let geometry = StripeCopyError::Geometry;
        if origin_y > end_y || end_y > source.frame_height() {
            return Err(geometry);
        }
        let sample_count = source
            .width()
            .checked_mul(end_y - origin_y)
            .ok_or(geometry)?;
        let mut samples = take_stripe_sample_buffer(sample_count)?;
        if let Some(source_rows) = source.contiguous_rows(origin_y, end_y)
            && let Some(source_samples) = T::u16_slice(source_rows)
        {
            samples.extend_from_slice(source_samples);
        } else if let Some(source_samples) = T::u16_slice(source.samples()) {
            for y in origin_y..end_y {
                let start = y
                    .checked_sub(source.origin_y())
                    .filter(|row| *row < source.end_y() - source.origin_y())
                    .ok_or(geometry)?
                    .checked_mul(source.stride())
                    .ok_or(geometry)?;
                let row = source_samples
                    .get(start..start.checked_add(source.width()).ok_or(geometry)?)
                    .ok_or(geometry)?;
                samples.extend_from_slice(row);
            }
        } else {
            for y in origin_y..end_y {
                samples.extend(
                    source
                        .row(y)
                        .ok_or(geometry)?
                        .iter()
                        .map(|value| value.to_u16()),
                );
            }
        }
        Ok(Self {
            width: source.width(),
            frame_height: source.frame_height(),
            origin_y,
            samples,
        })
    }

    pub(crate) fn copy_rows(&self, origin_y: usize, end_y: usize) -> Result<Self, StripeCopyError> {
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
        let source = self.samples.get(start..end).ok_or(geometry)?;
        let mut samples = take_stripe_sample_buffer(source.len())?;
        samples.extend_from_slice(source);
        Ok(Self {
            width: self.width,
            frame_height: self.frame_height,
            origin_y,
            samples,
        })
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
            .checked_add(self.samples.len().checked_div(self.width)?)
    }

    pub(crate) fn samples(&self) -> &[u16] {
        &self.samples
    }

    pub(crate) fn samples_mut(&mut self) -> &mut [u16] {
        &mut self.samples
    }

    pub(crate) fn row(&self, y: usize) -> Option<&[u16]> {
        let row = y.checked_sub(self.origin_y)?;
        let start = row.checked_mul(self.width)?;
        self.samples.get(start..start.checked_add(self.width)?)
    }

    pub(crate) fn row_mut(&mut self, y: usize) -> Option<&mut [u16]> {
        let row = y.checked_sub(self.origin_y)?;
        let start = row.checked_mul(self.width)?;
        self.samples.get_mut(start..start.checked_add(self.width)?)
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
        Some((self.samples.get_mut(start..end)?, self.width))
    }
}

impl Drop for StripePlane {
    fn drop(&mut self) {
        recycle_stripe_sample_buffer(core::mem::take(&mut self.samples));
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
