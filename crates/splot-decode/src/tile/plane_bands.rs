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

pub(crate) type RectRun<T> = (PlaneRect, Vec<T>, usize);

type RectRunBand<'a, T> = (&'a mut [T], usize, &'a [RectRun<T>]);

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

    if splot_parallel::on_multiworker_pool() {
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
    } else {
        bands
            .into_iter()
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
}

#[cfg(test)]
#[path = "plane_bands_tests.rs"]
mod tests;
