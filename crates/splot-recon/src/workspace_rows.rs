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
    CurrentFramePlane, CurrentFrameWorkspace, IntraPredictionScratch, block_rect,
    chroma_plane_geometry,
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

    /// Moves a band down so it stores the superblock row starting at luma row
    /// `luma_row`, keeping the row above it.
    ///
    /// A workspace that stores every row is left as it is.
    ///
    /// # Errors
    /// Returns [`ReconError`] when the band would move up, which would expose
    /// rows it no longer holds.
    pub fn move_band(&mut self, luma_row: usize) -> Result<()> {
        let subsampling_y = self.info.pixel_format().subsampling_y();
        self.y.move_band(luma_row.saturating_sub(BAND_EDGE_ROWS))?;
        for plane in [self.u.as_mut(), self.v.as_mut()].into_iter().flatten() {
            plane.move_band((luma_row >> subsampling_y).saturating_sub(BAND_EDGE_ROWS))?;
        }
        Ok(())
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
            samples,
            pool: None,
        })
    }

    fn move_band(&mut self, origin_y: usize) -> Result<()> {
        let stride = self.stride_samples();
        if self.samples.len() >= stride * self.storage_size.height() {
            return Ok(());
        }
        let Some(skip) = origin_y.checked_sub(self.origin_y) else {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: self.storage_size,
                rect: PlaneRect::new(0, origin_y, stride, 1)?,
            });
        };
        let skip = skip.saturating_mul(stride);
        if skip >= self.samples.len() {
            return Err(ReconError::WorkspaceRectOutOfBounds {
                plane: self.plane,
                storage: self.storage_size,
                rect: PlaneRect::new(0, origin_y, stride, 1)?,
            });
        }
        self.samples.copy_within(skip.., 0); // splot-copy-ok: keep the edge rows the next superblock row reads
        self.origin_y = origin_y;
        Ok(())
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

    /// The rectangle's 8-bit samples from its origin and their row stride,
    /// for a reconstruction added in place; `None` for wider samples.
    pub fn u8_samples_mut(&mut self) -> Option<(&mut [u8], usize)> {
        let stride = self.stride_samples;
        T::u8_slice_mut(self.samples).map(|samples| (samples, stride))
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
        let stored_rows = self.samples.len().saturating_sub(self.row_start) / self.stride.max(1);
        let len = out.len().min(self.left_len).min(stored_rows);
        let column = match (self.origin.checked_sub(1), len.checked_sub(1)) {
            (Some(first), Some(last)) => self.samples.get(first..first + last * self.stride + 1),
            _ => None,
        };
        let Some(column) = column else {
            return 0;
        };
        for (i, slot) in out[..len].iter_mut().enumerate() {
            *slot = column[i * self.stride];
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
        let target = self.plane_mut(plane)?;
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
        let mut frame = IntraBlockFrame {
            origin: local_y * stride + x,
            row_start: local_y * stride,
            stride,
            above_len: if y > 0 { storage.width() - x } else { 0 },
            left_len: if x > 0 { storage.height() - y } else { 0 },
            rows_below: storage.height() - y,
            samples: &mut target.samples,
        };
        predict(&mut frame).map(Some)
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

    /// A band stores one superblock row and the row above it; a moved band
    /// keeps that edge row, and every access outside its rows fails closed.
    #[test]
    fn band_keeps_the_edge_row_and_refuses_rows_it_does_not_store() {
        let frame = |height| {
            DecodedFrameInfo::new(
                OutputIndex::new(0),
                BitDepth::Ten,
                PixelFormat::Yuv420,
                PlaneSize::new(64, height).unwrap(),
                PlaneRect::new(0, 0, 64, height).unwrap(),
            )
            .unwrap()
        };
        let mut band = CurrentFrameWorkspace::<u16>::new_band(frame(256), 64, None).unwrap();
        band.set_reconstructed_sample(PlaneId::Y, 3, 63, 77)
            .unwrap();
        band.set_reconstructed_sample(PlaneId::U, 3, 31, 55)
            .unwrap();
        assert!(band.set_reconstructed_sample(PlaneId::Y, 3, 65, 1).is_err());
        assert!(band.samples(PlaneId::Y).is_err());

        band.move_band(64).unwrap();
        assert_eq!(band.reconstructed_sample(PlaneId::Y, 3, 63).unwrap(), 77);
        assert_eq!(band.reconstructed_sample(PlaneId::U, 3, 31).unwrap(), 55);
        assert!(band.reconstructed_sample(PlaneId::Y, 3, 62).is_err());
        assert!(band.reconstructed_sample(PlaneId::U, 3, 30).is_err());
        assert!(band.reconstructed_sample(PlaneId::Y, 3, 128).is_err());
        let rows = |y, height| PlaneRect::new(0, y, 8, height).unwrap();
        assert!(band.rect_rows(PlaneId::Y, rows(63, 65)).is_ok());
        assert!(band.rect_rows(PlaneId::Y, rows(62, 2)).is_err());
        assert!(band.fill_rect(PlaneId::Y, rows(120, 9), 1).is_err());
        assert!(band.plane(PlaneId::Y).unwrap().rows(64, 128).is_ok());
        assert!(band.plane(PlaneId::Y).unwrap().rows(60, 64).is_err());
        assert!(band.move_band(0).is_err());
        assert!(band.move_band(256).is_err());
        assert_eq!(band.reconstructed_sample(PlaneId::Y, 3, 63).unwrap(), 77);
        assert!(band.as_frame_ref().is_err());
        assert!(band.as_frame_mut().is_err());

        let mut odd = CurrentFrameWorkspace::<u16>::new_band(frame(64), 1, None).unwrap();
        odd.move_band(2).unwrap();
        odd.set_reconstructed_sample(PlaneId::U, 0, 1, 3).unwrap();

        let whole = CurrentFrameWorkspace::<u16>::new_band(frame(64), 64, Some(band)).unwrap();
        assert!(whole.samples(PlaneId::Y).is_ok());
        assert!(whole.as_frame_ref().is_ok());

        let mut bottom = CurrentFrameWorkspace::<u16>::new_band(frame(200), 64, None).unwrap();
        for row in [64, 128, 192] {
            bottom.move_band(row).unwrap();
        }
        assert!(bottom.plane(PlaneId::Y).unwrap().rows(191, 200).is_ok());
        assert!(bottom.plane(PlaneId::Y).unwrap().rows(200, 201).is_err());
        assert!(bottom.reconstructed_sample(PlaneId::Y, 0, 200).is_err());
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
}
