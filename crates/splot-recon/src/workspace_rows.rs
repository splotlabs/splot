// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Write-through mutable row access to one current-frame block rectangle.
//!
//! [`CurrentFrameWorkspace::with_rect_block_rows_mut`] hands a caller the
//! destination rows of a block rectangle so reconstruction lands in the frame
//! directly, instead of filling a block buffer, range-scanning it, and copying
//! it into the plane.
//!
//! Fail-atomicity survives the loss of the staging buffer. Every failure mode of
//! the copy-based [`CurrentFrameWorkspace::write_rect_block`] is either checked
//! before the view exists — plane presence, rectangle geometry, storage span,
//! source length — or impossible once it does: the view is handed out only for a
//! rectangle lying wholly inside plane storage, and its write helpers produce
//! § 4.8 `Clip1`-clamped values, so the per-sample range check the copy path runs
//! before writing cannot reject anything written through this view. A rectangle
//! that would be clipped at the frame edge yields `Ok(None)` and never a partial
//! write, leaving that block on the caller's buffered path.
//!
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`, `RECON-RESIDUAL-ADDITION`,
//! `INFRA-DECODE-PARALLEL-STAGES`.

use super::owned_rect::OwnedFrameRectRows;
use super::{
    CurrentFramePlane, CurrentFramePlaneRect, CurrentFrameRect, CurrentFrameWorkspace,
    IntraPredictionScratch, block_rect, chroma_plane_geometry, ensure_rect_in_storage,
    subsampled_rect,
};
use crate::reconstruct::add_block_residual_into_rows;
use crate::{
    BitDepth, DecodedFrameInfo, IntraRectBlockSize, PlaneId, PlaneRect, PlaneRefRows, PlaneSize,
    ReconError, ReconSample, Result,
};

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Creates a workspace over recycled plane storage, without initializing
    /// its samples.
    ///
    /// The pooled buffers keep whatever the previous frame left in them, so
    /// this is only for a stage that seals every row before it reads it.
    ///
    /// # Errors
    /// Returns [`ReconError`] if the sample type cannot represent the frame bit
    /// depth, geometry arithmetic overflows, or plane allocation fails.
    pub fn new_recycled(info: DecodedFrameInfo) -> Result<Self> {
        Self::with_fill(info, None)
    }

    /// Creates a workspace over the sample buffers a retired frame handed back,
    /// without initializing its samples.
    ///
    /// Buffers the geometry cannot use are dropped and replaced, so a stream
    /// that changes frame size costs one allocation rather than a wrong frame.
    ///
    /// # Errors
    /// Returns [`ReconError`] if the sample type cannot represent the frame bit
    /// depth, geometry arithmetic overflows, or plane allocation fails.
    pub fn new_recycled_from(
        info: DecodedFrameInfo,
        recycled: &mut crate::FramePlaneSamples<T>,
    ) -> Result<Self> {
        Self::with_planes(info, None, recycled)
    }

    /// Creates a band: each plane stores the rows of one `luma_rows` tall
    /// superblock row plus the one row above it, from row 0.
    ///
    /// Every row access outside the stored rows fails closed. The buffers of
    /// `spare`, an earlier band, are reused; a band never takes plane-pool
    /// buffers, which are frame sized.
    ///
    /// # Errors
    /// Returns [`ReconError`] if the sample type cannot represent the frame bit
    /// depth, geometry arithmetic overflows, or plane allocation fails.
    pub fn new_band(info: DecodedFrameInfo, luma_rows: usize, spare: Option<Self>) -> Result<Self> {
        crate::intra_dc_math::validate_sample_type::<T>(info.bit_depth())?;
        let (mut spare, intra_prediction_scratch) = match spare {
            Some(mut spare) => {
                let scratch = core::mem::take(&mut spare.intra_prediction_scratch);
                (spare.into_plane_samples(), scratch)
            }
            None => (
                crate::FramePlaneSamples::default(),
                IntraPredictionScratch::new(),
            ),
        };
        let luma_size = info.storage_luma_size();
        let luma_rect = info.visible_luma_rect();
        let y = CurrentFramePlane::band(
            PlaneId::Y,
            luma_size,
            luma_rect,
            luma_rows,
            spare.take(PlaneId::Y),
        )?;
        let chroma_rows = luma_rows.div_ceil(1 << info.pixel_format().subsampling_y());
        let (u, v) = match chroma_plane_geometry(info.pixel_format(), luma_size, luma_rect)? {
            None => (None, None),
            Some((size, rect)) => (
                Some(CurrentFramePlane::band(
                    PlaneId::U,
                    size,
                    rect,
                    chroma_rows,
                    spare.take(PlaneId::U),
                )?),
                Some(CurrentFramePlane::band(
                    PlaneId::V,
                    size,
                    rect,
                    chroma_rows,
                    spare.take(PlaneId::V),
                )?),
            ),
        };
        Ok(Self {
            info,
            y,
            u,
            v,
            intra_prediction_scratch,
        })
    }

    /// A band over full-width plane rows another owner lends: each plane's
    /// rows start at plane row `origin_y`, and the band keeps `scratch`.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the planes do not match `info`, or a
    /// plane's rows are not whole rows inside its storage.
    pub fn lend_band(
        info: DecodedFrameInfo,
        [y, u, v]: [Option<(crate::LentRows<T>, usize)>; 3],
        scratch: IntraPredictionScratch<T>,
    ) -> Result<Self> {
        crate::intra_dc_math::validate_sample_type::<T>(info.bit_depth())?;
        let lent = |plane, storage_size: PlaneSize, visible_rect, rows: Option<(_, usize)>| {
            let Some((samples, origin_y)) = rows else {
                return Err(ReconError::MissingChromaPlane { plane });
            };
            let samples: crate::LentRows<T> = samples;
            let stride = crate::plane_stride::<T>(storage_size.width()).ok_or(
                ReconError::ArithmeticOverflow {
                    context: "lent band stride",
                },
            )?;
            let end = origin_y.checked_add(samples.len() / stride);
            if samples.is_empty()
                || !samples.len().is_multiple_of(stride)
                || end.is_none_or(|end| end > storage_size.height())
            {
                return Err(ReconError::WorkspaceRectOutOfBounds {
                    plane,
                    storage: storage_size,
                    rect: visible_rect,
                });
            }
            Ok(CurrentFramePlane {
                plane,
                storage_size,
                visible_rect,
                origin_y,
                stride,
                samples: PlaneSamples::Lent(samples),
                pool: None,
            })
        };
        let luma_size = info.storage_luma_size();
        let y = lent(PlaneId::Y, luma_size, info.visible_luma_rect(), y)?;
        let (u, v) = match chroma_plane_geometry(
            info.pixel_format(),
            luma_size,
            info.visible_luma_rect(),
        )? {
            None if u.is_none() && v.is_none() => (None, None),
            None => return Err(ReconError::UnexpectedChromaPlane { plane: PlaneId::U }),
            Some((size, rect)) => (
                Some(lent(PlaneId::U, size, rect, u)?),
                Some(lent(PlaneId::V, size, rect, v)?),
            ),
        };
        Ok(Self {
            info,
            y,
            u,
            v,
            intra_prediction_scratch: scratch,
        })
    }

    /// The address of the first luma sample of a band [`Self::lend_band`]
    /// made, or `None` for a workspace that owns its samples.
    pub fn lent_luma_addr(&self) -> Option<usize> {
        match &self.y.samples {
            PlaneSamples::Lent(rows) => Some(rows.as_ptr().addr()),
            PlaneSamples::Owned(_) => None,
        }
    }
}

impl<'storage, T: ReconSample> CurrentFrameRect<'storage, T> {
    /// The rectangle over full-width plane bands `(samples, stride)` the
    /// caller holds exclusively, as [`CurrentFrameWorkspace::rect_surfaces`]
    /// would split them off.
    ///
    /// # Errors
    /// Returns [`ReconError`] when a band does not span exactly the rows of
    /// `luma_rect` or its chroma rectangle, or `luma_rect` is not full width.
    pub fn from_bands(
        info: DecodedFrameInfo,
        luma_rect: PlaneRect,
        [y, u, v]: [Option<(&'storage mut [T], usize)>; 3],
    ) -> Result<Self> {
        let luma = info.storage_luma_size();
        let chroma = chroma_plane_geometry(info.pixel_format(), luma, info.visible_luma_rect())?
            .map(|(size, _)| size);
        let chroma_rect = chroma
            .map(|size| {
                let format = info.pixel_format();
                subsampled_rect(
                    luma_rect,
                    format.subsampling_x(),
                    format.subsampling_y(),
                    size,
                )
            })
            .transpose()?;
        let band =
            |plane, storage_size: PlaneSize, rect: PlaneRect, band: (&'storage mut [T], _)| {
                let (samples, stride) = band;
                let fits = rect.x() == 0
                    && rect.width() == storage_size.width()
                    && stride >= rect.width()
                    && rect.height().checked_mul(stride) == Some(samples.len());
                ensure_rect_in_storage(plane, storage_size, rect)?;
                fits.then_some(CurrentFramePlaneRect {
                    plane,
                    storage_size,
                    rect,
                    stride,
                    samples,
                })
                .ok_or(ReconError::WorkspaceRectSurfaceNotABand {
                    plane,
                    storage: storage_size,
                    rect,
                    settled_rows: rect.y(),
                })
            };
        let missing = |plane| ReconError::WorkspaceRectOutOfBounds {
            plane,
            storage: luma,
            rect: luma_rect,
        };
        let y = band(
            PlaneId::Y,
            luma,
            luma_rect,
            y.ok_or_else(|| missing(PlaneId::Y))?,
        )?;
        let (u, v) = match (chroma.zip(chroma_rect), u, v) {
            (None, None, None) => (None, None),
            (Some((size, rect)), Some(u), Some(v)) => (
                Some(band(PlaneId::U, size, rect, u)?),
                Some(band(PlaneId::V, size, rect, v)?),
            ),
            _ => return Err(missing(PlaneId::U)),
        };
        Ok(Self { info, y, u, v })
    }
}

/// A plane's samples: its own buffer, or rows another owner lends it.
#[derive(Debug)]
pub(super) enum PlaneSamples<T: Copy + 'static> {
    Owned(crate::PlaneBuf<T>),
    Lent(crate::LentRows<T>),
}

impl<T: Copy> Default for PlaneSamples<T> {
    fn default() -> Self {
        Self::Owned(crate::PlaneBuf::default())
    }
}

impl<T: Copy> core::ops::Deref for PlaneSamples<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        match self {
            Self::Owned(samples) => samples,
            Self::Lent(samples) => samples,
        }
    }
}

impl<T: Copy> core::ops::DerefMut for PlaneSamples<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        match self {
            Self::Owned(samples) => samples,
            Self::Lent(samples) => samples,
        }
    }
}

impl<T: Copy> PlaneSamples<T> {
    pub(super) fn take(&mut self) -> crate::PlaneBuf<T> {
        match core::mem::take(self) {
            Self::Owned(samples) => samples,
            Self::Lent(_) => crate::PlaneBuf::default(),
        }
    }
}

/// Plane rows a band keeps above its superblock row.
///
/// AV2 § 7.11.2 reads at most one row above a superblock: `sbBoundary` forces
/// `aboveMrlIndex` to zero, and the CfL and MHCCP luma reads clamp to
/// `sbTop - 1`. Local IntraBC reads only the current superblock row (§ 5.20.2.1
/// resets `IBCBufferValid` per row); a global IntraBC frame takes a band that
/// stores every row.
const BAND_EDGE_ROWS: usize = 1;

impl<T: ReconSample> CurrentFramePlane<T> {
    fn band(
        plane: PlaneId,
        storage_size: PlaneSize,
        visible_rect: PlaneRect,
        rows: usize,
        mut samples: crate::PlaneBuf<T>,
    ) -> Result<Self> {
        let stride = crate::plane_stride::<T>(storage_size.width()).ok_or(
            ReconError::ArithmeticOverflow {
                context: "current-frame band stride",
            },
        )?;
        let len = rows
            .saturating_add(BAND_EDGE_ROWS)
            .min(storage_size.height())
            .checked_mul(stride)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "current-frame band sample count",
            })?;
        samples.truncate(len);
        let missing = len - samples.len();
        samples
            .try_reserve_exact(missing)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane,
                context: "band sample buffer",
            })?;
        samples.resize(len, T::default());
        Ok(Self {
            plane,
            storage_size,
            visible_rect,
            origin_y: 0,
            stride,
            samples: PlaneSamples::Owned(samples),
            pool: None,
        })
    }

    /// The first plane row this plane stores.
    pub const fn origin_y(&self) -> usize {
        self.origin_y
    }

    /// Plane rows `start..end`, which must all be stored.
    ///
    /// # Errors
    /// Returns [`ReconError::WorkspaceRectOutOfBounds`] for a row outside the
    /// stored rows.
    pub fn rows(&self, start: usize, end: usize) -> Result<&[T]> {
        let (stride, rows) = (self.stride_samples(), end.saturating_sub(start));
        let local = self.band_row(start, rows)?;
        Ok(&self.samples[local * stride..(local + rows) * stride])
    }
}

/// Iterator over checked workspace rectangle rows.
#[derive(Debug)]
pub enum WorkspaceRectRows<'a, T: ReconSample> {
    /// Rows backed by conventional stride-based plane storage.
    Strided(PlaneRefRows<'a, T>),
    /// Rows backed by tightly packed caller-owned rectangle storage.
    Owned(OwnedFrameRectRows<'a, T>),
}

macro_rules! delegate_rect_rows {
    ($rows:expr, $method:ident) => {
        match $rows {
            WorkspaceRectRows::Strided(rows) => rows.$method(),
            WorkspaceRectRows::Owned(rows) => rows.$method(),
        }
    };
}

impl<T: ReconSample> WorkspaceRectRows<'_, T> {
    fn remaining(&self) -> usize {
        delegate_rect_rows!(self, len)
    }
}

impl<'a, T: ReconSample> Iterator for WorkspaceRectRows<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        delegate_rect_rows!(self, next)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.remaining();
        (remaining, Some(remaining))
    }
}

impl<T: ReconSample> ExactSizeIterator for WorkspaceRectRows<'_, T> {}

/// Exclusive mutable rows of one wholly in-frame current-frame rectangle.
///
/// The rows are the reconstruction target itself rather than a staging buffer,
/// so every sample written through this view is immediately part of the current
/// frame and must already be clamped to the active bit depth.
/// [`Self::add_block_residual`] guarantees that by construction.
#[derive(Debug)]
pub struct CurrentFrameRectRowsMut<'a, T: ReconSample> {
    samples: &'a mut [T],
    stride_samples: usize,
    rect: PlaneRect,
    bit_depth: BitDepth,
}

impl<T: ReconSample> CurrentFrameRectRowsMut<'_, T> {
    /// Returns this view's rectangle in global plane coordinates.
    pub const fn rect(&self) -> PlaneRect {
        self.rect
    }

    /// The rows as one strided slice, `(samples, stride)`, for a predictor that
    /// writes the block in place; every sample it writes must already lie
    /// within the active bit depth.
    pub fn prediction_target(&mut self) -> (&mut [T], usize) {
        (&mut *self.samples, self.stride_samples)
    }

    /// Reconstructs the rectangle from a contiguous block prediction and its
    /// AV2 § 7.14.3 residual, writing `Clip1(prediction + residual)` straight
    /// into the frame.
    ///
    /// `prediction` and `residual` hold `rect().width() * rect().height()`
    /// samples in block raster order. Both are validated before the first
    /// destination sample changes, so a rejected block leaves the frame
    /// unchanged.
    ///
    /// # Errors
    /// Returns [`ReconError`] when `T` cannot represent the active bit depth,
    /// `prediction` or `residual` does not match the rectangle's sample count,
    /// or a prediction sample exceeds the active bit depth.
    pub fn add_block_residual(&mut self, prediction: &[T], residual: &[i32]) -> Result<()> {
        add_block_residual_into_rows(
            prediction,
            residual,
            self.bit_depth,
            self.samples,
            self.stride_samples,
            self.rect.width(),
            self.rect.height(),
        )
    }
}

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Runs `write` over the exclusive destination rows of one block rectangle.
    ///
    /// Returns `Ok(None)` without running `write` when the block overhangs the
    /// frame edge, because a write-through view cannot reproduce the in-frame
    /// clamp [`CurrentFrameWorkspace::write_rect_block`] applies to a block
    /// buffer; the caller keeps its buffered path for those blocks. The whole
    /// target geometry is resolved and bounds-checked before `write` runs, so
    /// the view it receives addresses only in-storage samples.
    ///
    /// # Errors
    /// Returns the caller's error type for an absent plane, a rectangle whose
    /// origin falls outside plane storage, or any failure `write` raises.
    pub fn with_rect_block_rows_mut<R, E>(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        write: impl FnOnce(&mut CurrentFrameRectRowsMut<'_, T>) -> core::result::Result<R, E>,
    ) -> core::result::Result<Option<R>, E>
    where
        E: From<ReconError>,
    {
        let rect = block_rect(x, y, size)?;
        let bit_depth = self.info().bit_depth();
        let target = self.plane_mut(plane)?;
        if target.clamp_rect_to_storage(rect)? != rect {
            return Ok(None);
        }
        let stride_samples = target.stride_samples();
        let first = target.row_range(rect.y(), rect.x(), rect.width())?;
        let end = (rect.height() - 1)
            .checked_mul(stride_samples)
            .and_then(|offset| first.start.checked_add(offset))
            .and_then(|start| start.checked_add(rect.width()))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "current-frame rectangle row span",
            })?;
        let available = target.samples.len();
        let samples =
            target
                .samples
                .get_mut(first.start..end)
                .ok_or(ReconError::BufferLengthMismatch {
                    expected: end,
                    actual: available,
                })?;
        let mut rows = CurrentFrameRectRowsMut {
            samples,
            stride_samples,
            rect,
            bit_depth,
        };
        write(&mut rows).map(Some)
    }
}

/// One block lying wholly inside plane storage, with the stored samples
/// around it: an intra predictor reads its § 7.13.2 edges straight from the
/// frame and writes its prediction in place.
///
/// Every sample written through [`Self::target`] must lie within the active
/// bit depth.
pub struct IntraBlockFrame<'a, T: ReconSample> {
    samples: &'a mut [T],
    origin: usize,
    /// Where the block's first row starts in `samples`.
    row_start: usize,
    stride: usize,
    /// Stored samples of the row above, from the block's first column.
    above_len: usize,
    /// Plane rows of the column to the left from the block's first row, stored
    /// or not; none in the plane's first column.
    left_len: usize,
    /// Plane rows from the block's first row to the bottom storage edge.
    rows_below: usize,
}

impl<T: ReconSample> IntraBlockFrame<'_, T> {
    /// The stored row above the block from its first column, up to the plane's
    /// right storage edge; empty in the plane's first row.
    #[inline]
    pub fn above_row(&self) -> &[T] {
        match self.origin.checked_sub(self.stride) {
            Some(start) if self.above_len > 0 => self
                .samples
                .get(start..start + self.above_len)
                .unwrap_or_default(),
            _ => &[],
        }
    }

    /// Copies the column left of the block from its first row into `out`, up
    /// to the plane's bottom storage edge, and returns how many samples it
    /// copied: none in the plane's first column.
    #[inline]
    pub fn left_column(&self, out: &mut [T]) -> usize {
        let stride = self.stride.max(1);
        let rows = self.samples.get(self.row_start..).unwrap_or_default();
        let len = out.len().min(self.left_len).min(rows.len() / stride);
        let column = (self.origin - self.row_start).wrapping_sub(1);
        if column >= stride {
            return 0;
        }
        for (slot, row) in out[..len].iter_mut().zip(rows.chunks_exact(stride)) {
            *slot = row[column];
        }
        len
    }

    /// Plane rows from the block's first row down to the bottom storage edge,
    /// stored or not.
    #[inline]
    pub const fn left_extent(&self) -> usize {
        self.rows_below
    }

    /// The sample above and left of the block, when both edges are stored.
    #[inline]
    pub fn top_left(&self) -> Option<T> {
        if self.above_len == 0 || self.left_len == 0 {
            return None;
        }
        self.samples.get(self.origin - self.stride - 1).copied()
    }

    /// [`Self::above_row`] beside [`Self::target`], for a prediction that
    /// reads its above edge straight from the frame while it writes.
    #[inline]
    pub fn above_row_and_target(&mut self) -> (&[T], &mut [T], usize) {
        let (before, target) = self
            .samples
            .split_at_mut_checked(self.origin)
            .unwrap_or_default();
        let above = match self.origin.checked_sub(self.stride) {
            Some(start) if self.above_len > 0 => before
                .get(start..start + self.above_len)
                .unwrap_or_default(),
            _ => &[],
        };
        (above, target, self.stride)
    }

    /// The block's first sample onwards and the row stride, for the
    /// prediction write.
    #[inline]
    pub fn target(&mut self) -> (&mut [T], usize) {
        (
            self.samples.get_mut(self.origin..).unwrap_or_default(),
            self.stride,
        )
    }

    /// Writes the § 7.13.2.10 DC prediction of the block in place, blended by
    /// § 7.13.2.12 IBP when `ibp`, from the stored left column and above row
    /// that `[left, above]` make available.
    ///
    /// # Errors
    /// Returns [`ReconError`] for a sample type that cannot hold `bit_depth`.
    #[inline]
    pub fn predict_dc(
        &mut self,
        size: IntraRectBlockSize,
        bit_depth: BitDepth,
        [left, above]: [bool; 2],
        ibp: bool,
    ) -> Result<()> {
        let rows = self.samples.len().saturating_sub(self.row_start) / self.stride.max(1);
        let block = crate::intra_ibp_dc::DcFrameBlock {
            size,
            bit_depth,
            origin: self.origin,
            stride: self.stride,
            left: left && self.left_len >= size.height() && rows >= size.height(),
            above: above && self.above_len >= size.width(),
            ibp,
        };
        crate::intra_ibp_dc::predict_dc_in_frame(self.samples, block)
    }
}

impl<T: ReconSample> CurrentFrameWorkspace<T> {
    /// Runs `predict` over one block and the stored samples around it.
    ///
    /// Returns `Ok(None)` without running `predict` when the block overhangs
    /// plane storage or its above row lies outside a band, so the caller keeps
    /// its buffered path for those blocks.
    ///
    /// # Errors
    /// Returns the caller's error type for an absent plane, a block origin
    /// outside plane storage, or any failure `predict` raises.
    #[inline]
    pub fn with_intra_block_frame<R, E>(
        &mut self,
        plane: PlaneId,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        predict: impl FnOnce(&mut IntraBlockFrame<'_, T>) -> core::result::Result<R, E>,
    ) -> core::result::Result<Option<R>, E>
    where
        E: From<ReconError>,
    {
        match IntraBlockFrame::of(self.plane_mut(plane)?, x, y, size)? {
            Some(mut frame) => predict(&mut frame).map(Some),
            None => Ok(None),
        }
    }

    /// [`Self::with_intra_block_frame`] over the U and V blocks at the same
    /// position at once.
    ///
    /// # Errors
    /// Returns the caller's error type for a monochrome workspace, a block
    /// origin outside plane storage, or any failure `predict` raises.
    #[inline]
    pub fn with_intra_chroma_frames<R, E>(
        &mut self,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
        predict: impl FnOnce(&mut [IntraBlockFrame<'_, T>; 2]) -> core::result::Result<R, E>,
    ) -> core::result::Result<Option<R>, E>
    where
        E: From<ReconError>,
    {
        let (Some(u), Some(v)) = (self.u.as_mut(), self.v.as_mut()) else {
            return Err(ReconError::MissingWorkspacePlane { plane: PlaneId::U }.into());
        };
        match (
            IntraBlockFrame::of(u, x, y, size)?,
            IntraBlockFrame::of(v, x, y, size)?,
        ) {
            (Some(u), Some(v)) => predict(&mut [u, v]).map(Some),
            _ => Ok(None),
        }
    }
}

impl<'a, T: ReconSample> IntraBlockFrame<'a, T> {
    /// The block of `target` at `(x, y)`, or `None` when it overhangs plane
    /// storage or its above row lies outside a band.
    #[allow(
        clippy::inline_always,
        reason = "measured: keeps the intra glue as lean as before"
    )]
    #[inline(always)]
    fn of(
        target: &'a mut CurrentFramePlane<T>,
        x: usize,
        y: usize,
        size: IntraRectBlockSize,
    ) -> Result<Option<Self>> {
        let storage = target.storage_size;
        let (width, height) = (size.width(), size.height());
        if x + width > storage.width() || y + height > storage.height() {
            return Ok(None);
        }
        let stride = target.stride_samples();
        let local_y = target.band_row(y, height)?;
        if y > 0 && local_y == 0 {
            return Ok(None);
        }
        Ok(Some(Self {
            origin: local_y * stride + x,
            row_start: local_y * stride,
            stride,
            above_len: if y > 0 { storage.width() - x } else { 0 },
            left_len: if x > 0 { storage.height() - y } else { 0 },
            rows_below: storage.height() - y,
            samples: &mut target.samples,
        }))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::reconstruct::reconstruct_add_residual;
    use crate::{DecodedFrameInfo, OutputIndex, PixelFormat, PlaneSize};

    fn workspace<T: ReconSample>(
        bit_depth: BitDepth,
        width: usize,
        height: usize,
        fill: T,
    ) -> CurrentFrameWorkspace<T> {
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            bit_depth,
            PixelFormat::Monochrome,
            PlaneSize::new(width, height).unwrap(),
            PlaneRect::new(0, 0, width, height).unwrap(),
        )
        .unwrap();
        CurrentFrameWorkspace::new(info, fill).unwrap()
    }

    fn block(log2_width: u8, log2_height: u8) -> IntraRectBlockSize {
        IntraRectBlockSize::new(log2_width, log2_height).unwrap()
    }

    fn assert_same_plane<T: ReconSample + PartialEq>(
        actual: &CurrentFrameWorkspace<T>,
        expected: &CurrentFrameWorkspace<T>,
        context: &str,
    ) {
        assert!(
            actual.plane(PlaneId::Y).unwrap().samples()
                == expected.plane(PlaneId::Y).unwrap().samples(),
            "{context}"
        );
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// The write-through path must reproduce the buffered reference exactly for
    /// every AV2 transform-block shape, at both sample widths, over randomized
    /// predictions and residuals including the `i32` extremes.
    #[test]
    fn write_through_matches_the_buffered_reference_for_every_shape() {
        let mut rng = Rng(0x2f6e_2b1c_9d4a_1357);
        for log2_width in 2..=6u8 {
            for log2_height in 2..=6u8 {
                let size = block(log2_width, log2_height);
                let count = size.sample_count();
                let residual: Vec<i32> = (0..count)
                    .map(|index| match index % 8 {
                        0 => i32::MAX,
                        1 => i32::MIN,
                        _ => (rng.next() as i32) >> (rng.next() % 20) as i32,
                    })
                    .collect();
                let (x, y) = (64, 128);
                let (frame_width, frame_height) = (x + size.width(), y + size.height());

                let wide: Vec<u16> = (0..count).map(|_| (rng.next() % 1024) as u16).collect();
                let mut expected = vec![0u16; count];
                reconstruct_add_residual(&wide, &residual, BitDepth::Ten, &mut expected).unwrap();
                let mut reference = workspace::<u16>(BitDepth::Ten, frame_width, frame_height, 0);
                reference
                    .write_rect_block(PlaneId::Y, x, y, size, &expected)
                    .unwrap();
                let mut actual = workspace::<u16>(BitDepth::Ten, frame_width, frame_height, 0);
                let written: Option<()> = actual
                    .with_rect_block_rows_mut(PlaneId::Y, x, y, size, |rows| {
                        rows.add_block_residual(&wide, &residual)
                    })
                    .unwrap();
                assert!(written.is_some(), "{size:?} 10-bit must write through");
                assert_same_plane(&actual, &reference, "10-bit write-through");

                let narrow: Vec<u8> = wide.iter().map(|&sample| sample as u8).collect();
                let mut expected = vec![0u8; count];
                reconstruct_add_residual(&narrow, &residual, BitDepth::Eight, &mut expected)
                    .unwrap();
                let mut reference = workspace::<u8>(BitDepth::Eight, frame_width, frame_height, 0);
                reference
                    .write_rect_block(PlaneId::Y, x, y, size, &expected)
                    .unwrap();
                let mut actual = workspace::<u8>(BitDepth::Eight, frame_width, frame_height, 0);
                let written: Option<()> = actual
                    .with_rect_block_rows_mut(PlaneId::Y, x, y, size, |rows| {
                        rows.add_block_residual(&narrow, &residual)
                    })
                    .unwrap();
                assert!(written.is_some(), "{size:?} 8-bit must write through");
                assert_same_plane(&actual, &reference, "8-bit write-through");
            }
        }
    }

    /// A block overhanging the frame edge must decline the write-through path
    /// before any sample changes, leaving it to the buffered write.
    #[test]
    fn frame_edge_overhang_declines_without_writing() {
        let mut ws = workspace::<u16>(BitDepth::Ten, 12, 12, 7);
        let untouched = workspace::<u16>(BitDepth::Ten, 12, 12, 7);
        let written: Option<()> = ws
            .with_rect_block_rows_mut(PlaneId::Y, 8, 8, block(3, 3), |rows| {
                rows.add_block_residual(&[0u16; 64], &[100; 64])
            })
            .unwrap();
        assert!(written.is_none(), "an overhanging block must decline");
        assert_same_plane(&ws, &untouched, "declined overhang");
    }

    /// Every rejected input must be raised before the first destination sample
    /// changes, so the frame is untouched on failure.
    #[test]
    fn rejected_inputs_leave_the_plane_unchanged() {
        let size = block(2, 2);
        let mut ws = workspace::<u16>(BitDepth::Ten, 16, 16, 5);
        let untouched = workspace::<u16>(BitDepth::Ten, 16, 16, 5);

        let outside = ws.with_rect_block_rows_mut(PlaneId::Y, 20, 20, size, |rows| {
            rows.add_block_residual(&[0u16; 16], &[1; 16])
        });
        assert!(matches!(
            outside,
            Err(ReconError::WorkspaceRectOutOfBounds { .. })
        ));
        assert_same_plane(&ws, &untouched, "out-of-frame origin");

        let short = ws.with_rect_block_rows_mut(PlaneId::Y, 4, 4, size, |rows| {
            rows.add_block_residual(&[0u16; 8], &[1; 8])
        });
        assert!(matches!(
            short,
            Err(ReconError::ReconstructLengthMismatch { .. })
        ));
        assert_same_plane(&ws, &untouched, "short prediction");

        let out_of_range = ws.with_rect_block_rows_mut(PlaneId::Y, 4, 4, size, |rows| {
            rows.add_block_residual(&[2000u16; 16], &[1; 16])
        });
        assert!(matches!(
            out_of_range,
            Err(ReconError::ReconstructPredictionOutOfRange { .. })
        ));
        assert_same_plane(&ws, &untouched, "out-of-range prediction");
    }

    /// The view reports the rectangle its geometry was resolved for.
    #[test]
    fn view_reports_its_rectangle() {
        let mut ws = workspace::<u16>(BitDepth::Ten, 32, 32, 0);
        let rect: Option<PlaneRect> = ws
            .with_rect_block_rows_mut(PlaneId::Y, 8, 16, block(3, 2), |rows| {
                Ok::<_, ReconError>(rows.rect())
            })
            .unwrap();
        assert_eq!(rect, Some(PlaneRect::new(8, 16, 8, 4).unwrap()));
    }

    #[test]
    fn intra_block_frame_reads_stored_edges_and_writes_in_place() {
        let mut ws = workspace(BitDepth::Eight, 16, 16, 0u8);
        for y in 0..16 {
            for x in 0..16 {
                ws.set_reconstructed_sample(PlaneId::Y, x, y, (x + 16 * y) as u8)
                    .unwrap();
            }
        }
        let above: Vec<u8> = (4..16).map(|x| x + 48).collect();
        let read = ws
            .with_intra_block_frame(PlaneId::Y, 4, 4, block(2, 2), |frame| {
                assert_eq!(frame.above_row(), &above[..]);
                let mut left = [0u8; 16];
                let copied = frame.left_column(&mut left);
                let (target, stride) = frame.target();
                target[stride + 1] = 7;
                Ok::<_, ReconError>((copied, left, frame.top_left(), frame.left_extent()))
            })
            .unwrap()
            .unwrap();
        assert_eq!(read.0, 12);
        assert_eq!(
            read.1[..12],
            core::array::from_fn::<u8, 12, _>(|i| (3 + 16 * (i + 4)) as u8)
        );
        assert_eq!((read.2, read.3), (Some(51), 12));
        assert_eq!(ws.reconstructed_sample(PlaneId::Y, 5, 5).unwrap(), 7);

        let corner = ws
            .with_intra_block_frame(PlaneId::Y, 0, 0, block(2, 2), |frame| {
                Ok::<_, ReconError>((
                    frame.above_row().len(),
                    frame.left_column(&mut [0; 4]),
                    frame.top_left(),
                ))
            })
            .unwrap();
        assert_eq!(corner, Some((0, 0, None)));
        let overhang = ws
            .with_intra_block_frame(PlaneId::Y, 14, 4, block(2, 2), |_| Ok::<_, ReconError>(()))
            .unwrap();
        assert_eq!(overhang, None);
    }

    fn dc_in_frame_matches_the_edge_kernels<T: ReconSample + PartialEq + core::fmt::Debug>(
        bit_depth: BitDepth,
    ) {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let span = u64::from(bit_depth.max_sample()) + 1;
        let mut ws = workspace(bit_depth, 136, 136, T::default());
        for py in 0..136 {
            for px in 0..136 {
                let value = T::try_from_u16((rng.next() % span) as u16).unwrap();
                ws.set_reconstructed_sample(PlaneId::Y, px, py, value)
                    .unwrap();
            }
        }
        for log2_width in 2..=6u8 {
            for log2_height in 2..=6u8 {
                let size = block(log2_width, log2_height);
                for (x, y) in [(0, 0), (0, 68), (68, 0), (68, 68)] {
                    for edges in [[false, false], [true, false], [false, true], [true, true]] {
                        for ibp in [false, true] {
                            let stored =
                                ws.intra_dc_edges_for_rect(PlaneId::Y, x, y, size).unwrap();
                            let dc_edges = crate::IntraDcEdges::new(
                                stored.left_samples().filter(|_| edges[0]),
                                stored.above_samples().filter(|_| edges[1]),
                            );
                            let mut pred = vec![T::default(); size.sample_count()];
                            if ibp {
                                crate::predict_intra_ibp_dc_rect_into(
                                    bit_depth,
                                    size,
                                    dc_edges,
                                    &mut pred,
                                    size.width(),
                                )
                                .unwrap();
                            } else {
                                pred.fill(
                                    crate::predict_intra_dc_rect_value(bit_depth, size, dc_edges)
                                        .unwrap(),
                                );
                            }
                            ws.with_intra_block_frame(PlaneId::Y, x, y, size, |frame| {
                                frame.predict_dc(size, bit_depth, edges, ibp)
                            })
                            .unwrap()
                            .unwrap();
                            for (i, &value) in pred.iter().enumerate() {
                                let (px, py) = (x + i % size.width(), y + i / size.width());
                                assert_eq!(
                                    ws.reconstructed_sample(PlaneId::Y, px, py).unwrap(),
                                    value,
                                    "{size:?} at ({x}, {y}) {edges:?} ibp {ibp}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dc_in_frame_matches_the_edge_kernels_at_both_depths() {
        dc_in_frame_matches_the_edge_kernels::<u8>(BitDepth::Eight);
        dc_in_frame_matches_the_edge_kernels::<u16>(BitDepth::Ten);
    }
}
