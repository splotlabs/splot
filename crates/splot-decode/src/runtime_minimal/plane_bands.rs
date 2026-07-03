// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Disjoint plane row-band writes for pool-parallel filter stages.
//!
//! Filter stages that compute independent output rectangles publish them by
//! splitting each plane into disjoint mutable row bands and writing every
//! rectangle inside the band that owns its rows, so publication runs on the
//! worker pool without buffering or serial write-back loops.

use splot_parallel::prelude::*;
use splot_recon::{PlaneRect, ReconSample};

/// Writes one output rectangle into the row band that owns its rows.
///
/// `band` holds the plane rows starting at `band_top_row` with the plane's
/// full `stride`; `samples` holds `rect.height()` rows of `rect.width()`
/// samples at `row_stride`. Returns `None` when the rectangle does not fit
/// the band or the sample buffer, without writing.
pub(super) fn write_rect_into_band<T: ReconSample>(
    band: &mut [T],
    stride: usize,
    band_top_row: usize,
    rect: PlaneRect,
    samples: &[T],
    row_stride: usize,
) -> Option<()> {
    let width = rect.width();
    let local_y = rect.y().checked_sub(band_top_row)?;
    for row in 0..rect.height() {
        let src_start = row.checked_mul(row_stride)?;
        let src = samples.get(src_start..src_start.checked_add(width)?)?;
        let dst_start = local_y
            .checked_add(row)?
            .checked_mul(stride)?
            .checked_add(rect.x())?;
        let dst = band.get_mut(dst_start..dst_start.checked_add(width)?)?;
        dst.copy_from_slice(src);
    }
    Some(())
}

/// One published output rectangle: `(rect, samples, row stride)`.
pub(super) type RectRun<T> = (PlaneRect, Vec<T>, usize);

/// One plane row band with its top row and the runs it owns.
type RectRunBand<'a, T> = (&'a mut [T], usize, &'a [RectRun<T>]);

/// Publishes computed output rectangles into one plane on the worker pool.
///
/// `runs` holds `(rect, samples, row_stride)` outputs in raster order (rows
/// non-decreasing). Consecutive runs are grouped into row-disjoint bands, the
/// plane splits at the band boundaries, and bands write concurrently; each
/// rectangle lands exactly where a serial in-order write would put it.
/// Returns `None` (without completing) when any rectangle falls outside the
/// plane, its group band, or its sample buffer.
pub(super) fn publish_rect_runs_parallel<T: ReconSample>(
    plane_samples: &mut [T],
    stride: usize,
    runs: &[RectRun<T>],
) -> Option<()> {
    if stride == 0 {
        return None;
    }
    let mut groups: Vec<(usize, usize, usize)> = Vec::new();
    for (index, (rect, _, _)) in runs.iter().enumerate() {
        let bottom = rect.y().checked_add(rect.height())?;
        match groups.last_mut() {
            Some((_, end, group_bottom)) if rect.y() < *group_bottom => {
                *end = index + 1;
                *group_bottom = (*group_bottom).max(bottom);
            }
            _ => groups.push((index, index + 1, bottom)),
        }
    }

    let mut bands: Vec<RectRunBand<'_, T>> = Vec::new();
    let mut remaining = plane_samples;
    let mut split_row = 0usize;
    for &(start, end, group_bottom) in &groups {
        let rows = group_bottom.checked_sub(split_row)?;
        let split_at = rows.checked_mul(stride)?.min(remaining.len());
        let (band, rest) = remaining.split_at_mut(split_at);
        bands.push((band, split_row, runs.get(start..end)?));
        remaining = rest;
        split_row = group_bottom;
    }

    bands
        .into_par_iter()
        .try_for_each(|(band, band_top_row, band_runs)| {
            for (rect, samples, row_stride) in band_runs {
                write_rect_into_band(
                    &mut *band,
                    stride,
                    band_top_row,
                    *rect,
                    samples,
                    *row_stride,
                )?;
            }
            Some(())
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn writes_rect_at_band_origin_offset() {
        let mut band = vec![0u16; 4 * 8];
        let rect = PlaneRect::new(2, 5, 3, 2).unwrap();
        let samples = [1u16, 2, 3, 4, 5, 6];
        write_rect_into_band(&mut band, 8, 4, rect, &samples, 3).unwrap();
        assert_eq!(&band[8 + 2..8 + 5], &[1, 2, 3]);
        assert_eq!(&band[2 * 8 + 2..2 * 8 + 5], &[4, 5, 6]);
    }

    #[test]
    fn rejects_rect_above_band() {
        let mut band = vec![0u16; 8];
        let rect = PlaneRect::new(0, 1, 2, 1).unwrap();
        assert!(write_rect_into_band(&mut band, 8, 2, rect, &[1u16, 2], 2).is_none());
        assert!(band.iter().all(|&value| value == 0));
    }

    #[test]
    fn rejects_rect_below_band() {
        let mut band = vec![0u16; 8];
        let rect = PlaneRect::new(0, 3, 2, 1).unwrap();
        assert!(write_rect_into_band(&mut band, 8, 2, rect, &[1u16, 2], 2).is_none());
    }

    #[test]
    fn rejects_short_sample_buffer() {
        let mut band = vec![0u16; 16];
        let rect = PlaneRect::new(0, 0, 4, 2).unwrap();
        assert!(write_rect_into_band(&mut band, 8, 0, rect, &[1u16; 7], 4).is_none());
    }
}
