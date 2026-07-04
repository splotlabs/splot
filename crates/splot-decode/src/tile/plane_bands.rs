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
pub(crate) fn write_rect_into_band<T: ReconSample>(
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
pub(crate) type RectRun<T> = (PlaneRect, Vec<T>, usize);

/// One plane row band with its top row and the runs it owns.
type RectRunBand<'a, T> = (&'a mut [T], usize, &'a [RectRun<T>]);

/// Publishes computed output rectangles into one plane on the worker pool.
///
/// `runs` holds `(rect, samples, row_stride)` outputs in raster order (rows
/// non-decreasing). Consecutive runs are grouped into row-disjoint bands, the
/// plane splits at the band boundaries, and bands write concurrently; each
/// rectangle lands exactly where a serial in-order write would put it.
/// Returns `None` (without completing) when any rectangle falls outside the
/// plane, its group band, or its sample buffer, or when the caller is not on a
/// multi-worker pool — so the caller falls back to a serial in-order write.
/// A partial parallel write before a `None` is harmless: the caller's serial
/// fallback rewrites every run, and the outputs are disjoint, so the final
/// plane equals a pure serial write either way.
///
/// The parallel iterator is scoped by `DecodeContext`'s `WorkerPool::install`
/// (crates/splot-decode/src/context.rs) through the filter callers; the
/// `on_multiworker_pool` guard keeps a single-thread or direct caller on the
/// serial path rather than Rayon's global pool.
pub(crate) fn publish_rect_runs_parallel<T: ReconSample>(
    plane_samples: &mut [T],
    stride: usize,
    runs: &[RectRun<T>],
) -> Option<()> {
    if stride == 0 || !splot_parallel::on_multiworker_pool() {
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
    use splot_parallel::{ThreadCount, WorkerPool};

    fn serial_write(plane: &mut [u16], stride: usize, runs: &[RectRun<u16>]) {
        for (rect, samples, row_stride) in runs {
            for row in 0..rect.height() {
                let src = &samples[row * row_stride..row * row_stride + rect.width()];
                let dst_start = (rect.y() + row) * stride + rect.x();
                plane[dst_start..dst_start + rect.width()].copy_from_slice(src);
            }
        }
    }

    fn run(x: usize, y: usize, w: usize, h: usize, tag: u16) -> RectRun<u16> {
        let rect = PlaneRect::new(x, y, w, h).unwrap();
        let samples: Vec<u16> = (0..w * h).map(|i| tag.wrapping_add(i as u16)).collect();
        (rect, samples, w)
    }

    fn pool() -> WorkerPool {
        WorkerPool::new(ThreadCount::Fixed(4.try_into().unwrap())).unwrap()
    }

    #[test]
    fn parallel_banded_write_equals_serial_in_order() {
        let stride = 32usize;
        let height = 40usize;
        let runs = vec![
            run(0, 0, 8, 4, 100),
            run(8, 0, 8, 4, 200),
            run(16, 0, 16, 8, 300), // taller: extends group_bottom to 8
            run(0, 4, 8, 4, 400),   // y=4 < 8: same band
            run(0, 8, 32, 2, 500),  // y=8 >= 8: new band
            run(0, 10, 32, 30, 600),
        ];
        let mut par = vec![0u16; stride * height];
        let mut ser = vec![0u16; stride * height];
        let got = pool().install(|| publish_rect_runs_parallel(&mut par, stride, &runs));
        assert_eq!(got, Some(()), "valid raster-order input must publish");
        serial_write(&mut ser, stride, &runs);
        assert_eq!(par, ser, "parallel banded write must equal serial in-order");
    }

    #[test]
    fn none_on_single_worker_pool_leaves_plane_untouched() {
        let stride = 8usize;
        let runs = vec![run(0, 0, 4, 2, 7)];
        let mut plane = vec![0u16; stride * 4];
        let single = WorkerPool::new(ThreadCount::Fixed(1.try_into().unwrap())).unwrap();
        let got = single.install(|| publish_rect_runs_parallel(&mut plane, stride, &runs));
        assert_eq!(got, None);
        assert!(plane.iter().all(|&v| v == 0), "None path must not write");
    }

    #[test]
    fn partial_write_then_none_plus_serial_fallback_equals_serial() {
        let stride = 16usize;
        let height = 24usize;
        let mut runs = vec![run(0, 0, 8, 4, 111), run(0, 8, 8, 4, 222)];
        runs.push(run(0, height, 8, 4, 333));
        let mut par = vec![0u16; stride * height];
        let got = pool().install(|| publish_rect_runs_parallel(&mut par, stride, &runs[..]));
        let in_range = &runs[..2];
        let mut fallback = par.clone();
        if got.is_none() {
            serial_write(&mut fallback, stride, in_range);
        }
        let mut fresh = vec![0u16; stride * height];
        serial_write(&mut fresh, stride, in_range);
        assert_eq!(got, None, "out-of-plane rect must yield None");
        assert_eq!(
            fallback, fresh,
            "serial fallback after a partial parallel write must equal pure serial"
        );
    }

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
