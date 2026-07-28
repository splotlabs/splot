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
//! Feature tracking: `RECON-CURRENT-FRAME-WORKSPACE`, `RECON-RESIDUAL-ADDITION`.

use super::{CurrentFrameWorkspace, block_rect};
use crate::reconstruct::add_block_residual_into_rows;
use crate::{BitDepth, IntraRectBlockSize, PlaneId, PlaneRect, ReconError, ReconSample, Result};

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
        let stride_samples = target.stride_samples;
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
}
