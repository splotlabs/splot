// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample};
use std::sync::{Mutex, MutexGuard};

const MAX_RETAINED_STRIPE_BUFFERS: usize = 128;
const MAX_RETAINED_STRIPE_SAMPLES: usize = 4096 * 128;
static STRIPE_SAMPLE_BUFFERS: Mutex<Vec<Vec<u16>>> = Mutex::new(Vec::new());

fn lock_stripe_sample_buffers() -> MutexGuard<'static, Vec<Vec<u16>>> {
    STRIPE_SAMPLE_BUFFERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn take_stripe_sample_buffer(sample_count: usize) -> Option<Vec<u16>> {
    let mut buffers = lock_stripe_sample_buffers();
    let index = buffers
        .iter()
        .enumerate()
        .filter(|(_, buffer)| buffer.capacity() >= sample_count)
        .min_by_key(|(_, buffer)| buffer.capacity())
        .or_else(|| {
            buffers
                .iter()
                .enumerate()
                .max_by_key(|(_, buffer)| buffer.capacity())
        })
        .map(|(index, _)| index);
    let mut buffer = index
        .map(|index| buffers.swap_remove(index))
        .unwrap_or_default();
    drop(buffers);
    buffer.clear();
    buffer.try_reserve_exact(sample_count).ok()?;
    Some(buffer)
}

fn recycle_stripe_sample_buffer(mut buffer: Vec<u16>) {
    if buffer.capacity() == 0 || buffer.capacity() > MAX_RETAINED_STRIPE_SAMPLES {
        return;
    }
    buffer.clear();
    let mut buffers = lock_stripe_sample_buffers();
    if buffers.len() < MAX_RETAINED_STRIPE_BUFFERS && buffers.try_reserve(1).is_ok() {
        buffers.push(buffer);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FramePlane<'a, T> {
    width: usize,
    height: usize,
    stride: usize,
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
            samples: source.samples(),
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

    pub(crate) const fn samples(self) -> &'a [T] {
        self.samples
    }

    pub(crate) fn row(self, y: usize) -> Option<&'a [T]> {
        let start = y.checked_mul(self.stride)?;
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
    ) -> Option<Self> {
        if origin_y > end_y || end_y > source.frame_height() {
            return None;
        }
        let sample_count = source.width().checked_mul(end_y - origin_y)?;
        let mut samples = take_stripe_sample_buffer(sample_count)?;
        for y in origin_y..end_y {
            samples.extend(source.row(y)?.iter().map(|value| value.to_u16()));
        }
        Some(Self {
            width: source.width(),
            frame_height: source.frame_height(),
            origin_y,
            samples,
        })
    }

    pub(crate) fn copy_rows(&self, origin_y: usize, end_y: usize) -> Option<Self> {
        let start = origin_y
            .checked_sub(self.origin_y)?
            .checked_mul(self.width)?;
        let end = end_y.checked_sub(self.origin_y)?.checked_mul(self.width)?;
        let source = self.samples.get(start..end)?;
        let mut samples = take_stripe_sample_buffer(source.len())?;
        samples.extend_from_slice(source);
        Some(Self {
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
}

impl Drop for StripePlane {
    fn drop(&mut self) {
        recycle_stripe_sample_buffer(core::mem::take(&mut self.samples));
    }
}
