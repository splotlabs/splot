// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(
    unsafe_code,
    reason = "stripe buffers retain their unique allocation owner"
)]

#[cfg(test)]
use splot_recon::CurrentFrameWorkspace;
use splot_recon::{PlaneId, PlaneRect, ReconSample};

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

/// Staging buffers one worker's filter stage keeps between stripes.
///
/// dav2d gives each worker a task context whose scratch is sized once and
/// reused for the life of the decode. A stripe that cannot be filtered straight
/// into the published frame stages its rows here instead, and hands the buffer
/// back when the stripe's planes drop. The slot count is the most a single
/// stripe holds at once -- three planes for the stripe itself and three for the
/// fringe rows the GDF read radius needs -- so it is a fixed context, not a
/// store that grows with the stream.
const STRIPE_STAGING_SLOTS: usize = 6;

type StagingSlots = (
    [Vec<u8>; STRIPE_STAGING_SLOTS],
    [Vec<u16>; STRIPE_STAGING_SLOTS],
);

std::thread_local! {
    static STRIPE_STAGING: core::cell::RefCell<StagingSlots> = const {
        core::cell::RefCell::new((
            [const { Vec::new() }; STRIPE_STAGING_SLOTS],
            [const { Vec::new() }; STRIPE_STAGING_SLOTS],
        ))
    };
}

/// This worker's staging slots of sample type `T`, when they are free.
fn with_staging_slots<T: ReconSample, R>(
    f: impl FnOnce(&mut dyn Iterator<Item = &mut Vec<T>>) -> R,
) -> Option<R> {
    STRIPE_STAGING.with(|slots| {
        let mut slots = slots.try_borrow_mut().ok()?;
        let (eight, ten) = &mut *slots;
        let mut slots = eight
            .iter_mut()
            .zip(ten.iter_mut())
            .map(|(eight, ten)| T::depth_vec(eight, ten));
        Some(f(&mut slots))
    })
}

/// Takes this worker's largest free staging buffer, or a fresh one, holding
/// `sample_count` samples of whatever it last held, for a caller that
/// overwrites every one before reading it.
fn take_stripe_sample_buffer<T: ReconSample>(
    sample_count: usize,
) -> Result<Vec<T>, StripeCopyError> {
    let mut buffer = with_staging_slots(|slots| {
        slots
            .max_by_key(|slot| slot.capacity())
            .map(core::mem::take)
    })
    .flatten()
    .unwrap_or_default();
    buffer.truncate(sample_count);
    buffer
        .try_reserve_exact(sample_count - buffer.len())
        .map_err(|_| StripeCopyError::Allocation(PlaneId::Y))?;
    buffer.resize(sample_count, T::default());
    Ok(buffer)
}

/// Returns a spent staging buffer to this worker's context.
fn give_stripe_sample_buffer<T: ReconSample>(buffer: Vec<T>) {
    if buffer.capacity() == 0 {
        return;
    }
    with_staging_slots(|slots| {
        if let Some(slot) = slots
            .min_by_key(|slot| slot.capacity())
            .filter(|slot| slot.capacity() < buffer.capacity())
        {
            *slot = buffer;
        }
    });
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

    /// Copies rows `origin_y..end_y` into `destination`, rows `stride` apart.
    fn copy_rows(
        self,
        origin_y: usize,
        end_y: usize,
        destination: &mut [T],
        stride: usize,
    ) -> Option<()> {
        let Some(last) = end_y.checked_sub(origin_y)?.checked_sub(1) else {
            return Some(());
        };
        let span = last.checked_mul(stride)?.checked_add(self.width)?;
        let destination = destination.get_mut(..span)?;
        if stride == self.width
            && let Some(source) = self.contiguous_rows(origin_y, end_y)
        {
            destination.copy_from_slice(source);
            return Some(());
        }
        for (y, output) in (origin_y..end_y).zip(destination.chunks_mut(stride)) {
            output.get_mut(..self.width)?.copy_from_slice(self.row(y)?);
        }
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

/// One filter stripe's private copy of its deblocked input rows.
///
/// A stripe writes its output into the frame the deblock ran on, so it reads
/// its input from this copy; the rows above it come from the carry, because the
/// stripe above may already have overwritten them in the frame.
#[derive(Default)]
pub(crate) struct DeblockedWindow<T> {
    planes: [splot_recon::PlaneBuf<T>; 3],
    rows: [Option<(usize, usize)>; 3],
    sizes: [(usize, usize); 3],
}

impl<T: ReconSample> DeblockedWindow<T> {
    pub(crate) fn from_samples(mut samples: splot_recon::FramePlaneSamples<T>) -> Self {
        Self {
            planes: [PlaneId::Y, PlaneId::U, PlaneId::V].map(|plane| samples.take(plane)),
            ..Self::default()
        }
    }

    pub(crate) fn into_samples(self) -> splot_recon::FramePlaneSamples<T> {
        let [y, u, v] = self.planes;
        splot_recon::FramePlaneSamples::new(y, Some(u), Some(v))
    }

    /// Forgets the rows a previous frame left in this carry.
    pub(crate) fn clear(&mut self) {
        self.rows = [None; 3];
    }

    #[cfg(test)]
    pub(crate) fn fill(
        &mut self,
        frame: &mut crate::pipeline::frame_progress::FrontierRows<T>,
        carry: &mut Self,
        luma: (usize, usize),
        margin: usize,
    ) -> crate::Result<()> {
        self.copy_window(frame, Some(carry), luma, margin, [true; 3], [false; 3])
    }

    /// Copies stripe `luma` and `margin` rows around it: rows the carry holds
    /// from it, the rest from the frame. The carry then keeps this window's
    /// rows the next stripe's window shares. Without a carry the window must
    /// hold the stripe before this one, which has run: the rows both share
    /// move down in place, so the window is its own carry.
    ///
    /// A plane that `read` clears gets no rows, so each read of it is refused.
    /// Every stripe of a frame must pass the same `read`. A plane that `skip`
    /// sets gets only the rows the next stripe's window shares.
    pub(crate) fn copy_window(
        &mut self,
        frame: &mut crate::pipeline::frame_progress::FrontierRows<T>,
        mut carry: Option<&mut Self>,
        luma: (usize, usize),
        margin: usize,
        read: [bool; 3],
        skip: [bool; 3],
    ) -> crate::Result<()> {
        let state = || crate::DecodeHeaderStateError::InvalidLoopRestorationFilterState;
        let sub_y = usize::from(frame.info().pixel_format().subsampling_y());
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            let index = plane.index();
            let held = self.rows[index].take();
            let Some((width, height)) = frame.plane_size(plane) else {
                if let Some(carry) = carry.as_deref_mut() {
                    carry.rows[index] = None;
                }
                continue;
            };
            let shift = usize::from(plane != PlaneId::Y) * sub_y;
            let (start, end) = window_bounds(luma, shift, margin, height).map_err(|_| state())?;
            let alloc = |_| splot_recon::ReconError::WorkspaceAllocationFailed {
                plane,
                context: "filter stripe window",
            };
            let held = held.filter(|_| self.sizes[index] == (width, height));
            let samples = &mut self.planes[index];
            if !read[index] {
                samples.clear();
                if let Some(carry) = carry.as_deref_mut() {
                    carry.rows[index] = None;
                }
                self.rows[index] = Some((start, start));
                self.sizes[index] = (width, height);
                continue;
            }
            let tail = (luma.1 >> shift).saturating_sub(margin).clamp(start, end);
            let start = if skip[index] { tail } else { start };
            let shared = |(held_start, held_end): (usize, usize)| {
                (held_start <= start && start < held_end).then(|| {
                    let next = held_end.min(end);
                    (
                        next,
                        (start - held_start) * width..(next - held_start) * width,
                    )
                })
            };
            let mut next = start;
            match carry.as_deref() {
                Some(carry) => {
                    samples.clear();
                    samples
                        .try_reserve_exact((end - start) * width)
                        .map_err(alloc)?;
                    if let Some((kept, rows)) = carry.rows[index].and_then(shared) {
                        next = kept;
                        samples.append_samples(carry.planes[index].get(rows).ok_or_else(state)?);
                    }
                }
                None => match held.and_then(shared) {
                    Some((kept, rows)) => {
                        let count = samples.get(rows.clone()).ok_or_else(state)?.len();
                        samples.copy_within(rows, 0);
                        samples.truncate(count);
                        next = kept;
                    }
                    None => samples.clear(),
                },
            }
            samples
                .try_reserve_exact((end - next) * width)
                .map_err(alloc)?;
            if next < end {
                frame
                    .append_rows(plane, next, end, samples)
                    .ok_or_else(state)?;
            }
            if let Some(carry) = carry.as_deref_mut() {
                let kept = samples.get((tail - start) * width..).ok_or_else(state)?;
                let carried = &mut carry.planes[index];
                carried.clear();
                carried.try_reserve_exact(kept.len()).map_err(alloc)?;
                carried.append_samples(kept);
                carry.rows[index] = Some((tail, end));
            }
            self.rows[index] = Some((start, end));
            self.sizes[index] = (width, height);
        }
        frame
            .release_rows(luma.1)
            .then_some(())
            .ok_or_else(|| state().into())
    }

    pub(crate) fn planes(&self) -> Option<DeblockedPlanes<'_, T>> {
        let plane = |index: usize| {
            let (start, end) = self.rows[index]?;
            let (width, height) = self.sizes[index];
            FramePlane::window(&self.planes[index], width, height, start, end - start)
        };
        Some(DeblockedPlanes {
            y: plane(PlaneId::Y.index())?,
            u: plane(PlaneId::U.index()),
            v: plane(PlaneId::V.index()),
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

enum StripeOwner<T> {
    Owned(Vec<T>),
    Direct(
        #[allow(dead_code, reason = "keeps the leased stripe region alive")] DirectPlaneTarget<T>,
    ),
}

struct StripeSamples<T: ReconSample> {
    samples: core::ptr::NonNull<[T]>,
    owner: StripeOwner<T>,
}

// SAFETY: the `Send` owner keeps the cached pointer's allocation alive.
unsafe impl<T: ReconSample> Send for StripeSamples<T> {}

impl<T: ReconSample> Drop for StripeSamples<T> {
    fn drop(&mut self) {
        if let StripeOwner::Owned(buffer) = &mut self.owner {
            give_stripe_sample_buffer(core::mem::take(buffer));
        }
    }
}

impl<T: ReconSample> StripeSamples<T> {
    fn owned(mut samples: Vec<T>) -> Self {
        let pointer = core::ptr::NonNull::from(samples.as_mut_slice());
        Self {
            samples: pointer,
            owner: StripeOwner::Owned(samples),
        }
    }

    fn direct(mut target: DirectPlaneTarget<T>) -> Self {
        let pointer = core::ptr::NonNull::from(target.samples_mut());
        Self {
            samples: pointer,
            owner: StripeOwner::Direct(target),
        }
    }

    #[inline]
    fn as_slice(&self) -> &[T] {
        unsafe {
            // SAFETY: the owner stays alive and cannot reallocate.
            self.samples.as_ref()
        }
    }

    #[inline]
    fn as_mut_slice(&mut self) -> &mut [T] {
        unsafe {
            // SAFETY: the owner and this borrow are both exclusive.
            self.samples.as_mut()
        }
    }
}

type DirectPlaneTarget<T> = crate::pipeline::frame_progress::DirectPlaneTarget<T>;

/// One plane's rows of a filter stripe: a pooled private buffer, or the
/// stripe's own region of the published frame.
pub(crate) struct StripePlane<T: ReconSample> {
    width: usize,
    stride: usize,
    frame_height: usize,
    origin_y: usize,
    samples: StripeSamples<T>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StripeInitialization {
    CopyAll,
    /// The caller proves every destination sample is written, or already holds
    /// its source sample, before publication.
    FullyOverwritten,
}

impl<T: ReconSample> StripePlane<T> {
    pub(crate) fn from_samples(
        width: usize,
        frame_height: usize,
        origin_y: usize,
        samples: Vec<T>,
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
            stride: width,
            frame_height,
            origin_y,
            samples: StripeSamples::owned(samples),
        })
    }

    /// The target as a stripe plane, for a stripe whose target already holds
    /// the rows it starts from.
    pub(crate) fn from_target(target: DirectPlaneTarget<T>) -> Self {
        Self {
            width: target.width(),
            stride: target.stride(),
            frame_height: target.frame_height(),
            origin_y: target.origin_y(),
            samples: StripeSamples::direct(target),
        }
    }

    #[cfg(test)]
    pub(crate) fn copy_from(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
    ) -> Result<Self, StripeCopyError> {
        Self::copy_from_into(source, origin_y, end_y, None)
    }

    #[cfg(test)]
    pub(crate) fn copy_from_into(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
        target: Option<DirectPlaneTarget<T>>,
    ) -> Result<Self, StripeCopyError> {
        Self::copy_from_into_mode(
            source,
            origin_y,
            end_y,
            target,
            StripeInitialization::CopyAll,
        )
    }

    pub(crate) fn preflight_copy_from_into(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
        target: Option<&DirectPlaneTarget<T>>,
        initialization: StripeInitialization,
    ) -> Result<(), StripeCopyError> {
        let geometry = StripeCopyError::Geometry;
        if origin_y > end_y || end_y > source.frame_height() {
            return Err(geometry);
        }
        match target {
            Some(target) => (target.width() == source.width()
                && target.frame_height() == source.frame_height()
                && target.origin_y() == origin_y
                && target.end_y() == Some(end_y))
            .then_some(())
            .ok_or(geometry),
            None if initialization == StripeInitialization::FullyOverwritten => Err(geometry),
            None => Ok(()),
        }
    }

    /// Rows `origin_y..end_y` of `source` in `target`, or in a pooled buffer
    /// without one; a `FullyOverwritten` target is not initialized.
    pub(crate) fn copy_from_into_mode(
        source: FramePlane<'_, T>,
        origin_y: usize,
        end_y: usize,
        target: Option<DirectPlaneTarget<T>>,
        initialization: StripeInitialization,
    ) -> Result<Self, StripeCopyError> {
        Self::preflight_copy_from_into(source, origin_y, end_y, target.as_ref(), initialization)?;
        let mut output = if let Some(target) = target {
            Self::from_target(target)
        } else {
            let sample_count = source
                .width()
                .checked_mul(end_y - origin_y)
                .ok_or(StripeCopyError::Geometry)?;
            Self::from_samples(
                source.width(),
                source.frame_height(),
                origin_y,
                take_stripe_sample_buffer(sample_count)?,
            )
            .ok_or(StripeCopyError::Geometry)?
        };
        if initialization == StripeInitialization::CopyAll {
            let stride = output.stride;
            source
                .copy_rows(origin_y, end_y, output.samples_mut(), stride)
                .ok_or(StripeCopyError::Geometry)?;
        }
        Ok(output)
    }

    #[cfg(test)]
    pub(crate) fn copy_rows_into(
        &self,
        origin_y: usize,
        end_y: usize,
        target: Option<DirectPlaneTarget<T>>,
    ) -> Result<Self, StripeCopyError> {
        self.copy_rows_into_mode(origin_y, end_y, target, StripeInitialization::CopyAll)
    }

    pub(crate) fn preflight_copy_rows_into(
        &self,
        origin_y: usize,
        end_y: usize,
        target: Option<&DirectPlaneTarget<T>>,
        initialization: StripeInitialization,
    ) -> Result<(), StripeCopyError> {
        if origin_y < self.origin_y || origin_y > end_y || Some(end_y) > self.end_y() {
            return Err(StripeCopyError::Geometry);
        }
        Self::preflight_copy_from_into(self.view()?, origin_y, end_y, target, initialization)
    }

    pub(crate) fn copy_rows_into_mode(
        &self,
        origin_y: usize,
        end_y: usize,
        target: Option<DirectPlaneTarget<T>>,
        initialization: StripeInitialization,
    ) -> Result<Self, StripeCopyError> {
        self.preflight_copy_rows_into(origin_y, end_y, target.as_ref(), initialization)?;
        Self::copy_from_into_mode(self.view()?, origin_y, end_y, target, initialization)
    }

    /// These rows as a frame-plane view.
    pub(crate) fn view(&self) -> Result<FramePlane<'_, T>, StripeCopyError> {
        let end_y = self.end_y().ok_or(StripeCopyError::Geometry)?;
        Ok(FramePlane {
            width: self.width,
            height: self.frame_height,
            stride: self.stride,
            storage_origin_y: self.origin_y,
            storage_rows: end_y - self.origin_y,
            samples: self.samples(),
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

    pub(crate) const fn stride(&self) -> usize {
        self.stride
    }

    pub(crate) fn end_y(&self) -> Option<usize> {
        self.origin_y
            .checked_add(self.samples().len().checked_div(self.stride)?)
    }

    #[inline]
    pub(crate) fn samples(&self) -> &[T] {
        self.samples.as_slice()
    }

    #[inline]
    pub(crate) fn samples_mut(&mut self) -> &mut [T] {
        self.samples.as_mut_slice()
    }

    #[inline]
    pub(crate) fn row(&self, y: usize) -> Option<&[T]> {
        let row = y.checked_sub(self.origin_y)?;
        let start = row.checked_mul(self.stride)?;
        self.samples().get(start..start.checked_add(self.width)?)
    }

    #[inline]
    pub(crate) fn row_mut(&mut self, y: usize) -> Option<&mut [T]> {
        let row = y.checked_sub(self.origin_y)?;
        let width = self.width;
        let start = row.checked_mul(self.stride)?;
        self.samples_mut().get_mut(start..start.checked_add(width)?)
    }

    #[inline]
    pub(crate) fn rect_mut(&mut self, rect: PlaneRect) -> Option<(&mut [T], usize)> {
        if rect.x().checked_add(rect.width())? > self.width {
            return None;
        }
        let (row, stride) = (rect.y().checked_sub(self.origin_y)?, self.stride);
        let start = row.checked_mul(stride)?.checked_add(rect.x())?;
        let end = rect
            .height()
            .checked_sub(1)?
            .checked_mul(stride)?
            .checked_add(start)?
            .checked_add(rect.width())?;
        Some((self.samples_mut().get_mut(start..end)?, stride))
    }

    pub(crate) const fn is_direct(&self) -> bool {
        matches!(self.samples.owner, StripeOwner::Direct(_))
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
