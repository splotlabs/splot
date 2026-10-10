// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Flat-window detection for the § 7.20 loop-restoration filters.
//!
//! A § 7.20.3 or § 7.20.4 filter whose taps sum to zero leaves a sample
//! unchanged when every sample its taps read holds one value. A row group's
//! output splits into 32-column chunks, and each chunk is flat when its whole
//! tap reach in the group's padded source window holds one value. What one
//! group learns about a chunk carries to the next group, whose window
//! overlaps it: a flat chunk needs only its new rows checked, and a row that
//! shows two values keeps the chunk uneven while the window holds it.

use std::simd::{Simd, num::SimdUint};

use super::{
    LUMA_WINDOW_ROWS, LumaSimdOutput, LumaSimdSource, LumaSubclassLayout, PreparedLumaClass,
    WIENER_NS_LUMA_TAP_RADIUS, WienerNsLumaFilter, filter_luma_segment_simd, for_each_luma_segment,
    luma_segment_error,
};
use crate::Result;

/// Output columns per chunk.
const CHUNK: usize = 32;
/// Chunks a row may hold; later chunks are never flat.
const MAX_CHUNKS: usize = 32;

/// The flat chunks of consecutive row groups whose tap reach extends `R`
/// columns past each side of the output.
pub(crate) struct FlatChunks<const R: usize> {
    /// First source row of the last group's window.
    first: usize,
    /// Chunk `k` holds `values[k]` in source rows `first..flat_until[k]`.
    flat_until: [usize; MAX_CHUNKS],
    values: [u16; MAX_CHUNKS],
    /// Every window that holds source row `rough_until[k] - 1` shows two
    /// values in chunk `k`.
    rough_until: [usize; MAX_CHUNKS],
    /// Bit `k` marks chunk `k` flat in the last group.
    mask: u32,
}

impl<const R: usize> FlatChunks<R> {
    pub(crate) const fn new() -> Self {
        Self {
            first: 0,
            flat_until: [0; MAX_CHUNKS],
            values: [0; MAX_CHUNKS],
            rough_until: [0; MAX_CHUNKS],
            mask: 0,
        }
    }

    /// Whether the last row group has a flat chunk.
    pub(crate) const fn any(&self) -> bool {
        self.mask != 0
    }

    /// Finds the flat chunks of the `width`-column output whose padded source
    /// `window` starts at source row `first`.
    pub(crate) fn update<T: LumaSimdSource>(
        &mut self,
        window: &[&[T]],
        first: usize,
        width: usize,
    ) {
        if first < self.first {
            *self = Self::new();
        }
        self.first = first;
        self.mask = 0;
        let full = (width / CHUNK).min(MAX_CHUNKS);
        for chunk in 0..full {
            self.update_chunk(window, first, chunk, CHUNK + 2 * R);
        }
        if full < MAX_CHUNKS && !width.is_multiple_of(CHUNK) {
            self.update_chunk(window, first, full, width % CHUNK + 2 * R);
        }
    }

    #[allow(
        clippy::inline_always,
        reason = "a constant reach unrolls the row check"
    )]
    #[inline(always)]
    fn update_chunk<T: LumaSimdSource>(
        &mut self,
        window: &[&[T]],
        first: usize,
        chunk: usize,
        reach: usize,
    ) {
        if self.rough_until[chunk] > first {
            return;
        }
        let known = self.flat_until[chunk].saturating_sub(first);
        self.flat_until[chunk] = 0;
        match flat_value(window, CHUNK * chunk, reach, known, self.values[chunk]) {
            Ok(value) => {
                self.mask |= 1 << chunk;
                self.values[chunk] = value;
                self.flat_until[chunk] = first + window.len();
            }
            Err(row) => self.rough_until[chunk] = first + row + 1,
        }
    }

    /// Splits output columns `col..end` before the first flat chunk: the
    /// columns to filter end at the returned column, and the flat run that
    /// follows, if any, has the returned value and end.
    pub(crate) fn split(&self, col: usize, end: usize) -> (usize, Option<(u16, usize)>) {
        let ahead = self.mask.checked_shr((col / CHUNK) as u32).unwrap_or(0);
        let chunk = col / CHUNK + ahead.trailing_zeros() as usize;
        let start = (CHUNK * chunk).max(col);
        match self.values.get(chunk) {
            Some(&value) if ahead != 0 && start < end => {
                (start, Some((value, end.min(CHUNK * chunk + CHUNK))))
            }
            _ => (end, None),
        }
    }
}

/// A row group of [`super::filter_padded_luma_rows_simd`] with a flat chunk: the
/// padded source `window` of output rows from `rows.0`, written from output
/// row `rows.1`.
pub(super) struct LumaFlatGroup<'f, 's, 'a, T> {
    pub(super) window: &'s [&'a [T]],
    pub(super) rows: (usize, usize),
    pub(super) output_stride: usize,
    pub(super) max_sample: u16,
    pub(super) flat: &'f FlatChunks<WIENER_NS_LUMA_TAP_RADIUS>,
}

impl<T: LumaSimdSource> LumaFlatGroup<'_, '_, '_, T> {
    /// Filters the group, writing each flat chunk's value instead of
    /// filtering it.
    #[inline(never)]
    pub(super) fn filter<O: LumaSimdOutput>(
        &self,
        output: &mut [O],
        params: &WienerNsLumaFilter<'_>,
        prepared_classes: &[PreparedLumaClass],
        subclasses: LumaSubclassLayout<'_>,
    ) -> Result<()> {
        let error = || luma_segment_error(params.width);
        let group_rows = self.window.len() - 2 * WIENER_NS_LUMA_TAP_RADIUS;
        for_each_luma_segment(
            self.rows.0,
            params.width,
            subclasses,
            |start, len, subclass| {
                let class = prepared_classes.get(subclass).ok_or_else(error)?;
                let mut col = start;
                while col < start + len {
                    let (filter_end, run) = self.flat.split(col, start + len);
                    for row in 0..group_rows {
                        let row_start = (self.rows.1 + row) * self.output_stride;
                        if filter_end > col {
                            let (Some(window), Some(filtered)) = (
                                self.window[row..].first_chunk::<LUMA_WINDOW_ROWS>(),
                                output.get_mut(row_start + col..row_start + filter_end),
                            ) else {
                                return Err(error());
                            };
                            filter_luma_segment_simd::<_, _, 1>(
                                filtered,
                                window,
                                col,
                                class,
                                self.max_sample,
                            );
                        }
                        if let Some((value, run_end)) = run {
                            output
                                .get_mut(row_start + filter_end..row_start + run_end)
                                .ok_or_else(error)?
                                .fill(O::from_u16(value.min(self.max_sample)));
                        }
                    }
                    col = run.map_or(start + len, |(_, run_end)| run_end);
                }
                Ok(())
            },
        )
    }
}

/// The one value the `width` columns from `start` of every row of `window`
/// hold, given that the first `known` rows hold `value`; otherwise the
/// window row that every window holding it shows two values in. Rows go
/// bottom up, so that row stays in the next windows longest.
#[allow(
    clippy::inline_always,
    reason = "a constant width unrolls the row check"
)]
#[inline(always)]
fn flat_value<T: LumaSimdSource>(
    window: &[&[T]],
    start: usize,
    width: usize,
    known: usize,
    value: u16,
) -> core::result::Result<u16, usize> {
    let mut reference = (known > 0).then(|| (value, known - 1));
    for (index, row) in window.iter().enumerate().skip(known).rev() {
        let row = &row[start..start + width];
        let sample = T::scalar(row, 0);
        let splat = Simd::<u16, 8>::splat(sample);
        let mut diff = T::load::<8>(row, width - 8) ^ splat;
        let mut x = 0;
        while x + 8 < width {
            diff |= T::load::<8>(row, x) ^ splat;
            x += 8;
        }
        if diff.reduce_max() != 0 {
            return Err(index);
        }
        let (value, value_row) = *reference.get_or_insert((sample, index));
        if value != sample {
            return Err(value_row.min(index));
        }
    }
    reference.map(|(value, _)| value).ok_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::{
        WIENER_NS_LUMA_COEFFS, WienerNsLumaPaddedSource, WienerNsLumaScratch,
        wiener_ns_filter_luma_block, wiener_ns_filter_luma_block_padded_cells_into,
        wiener_ns_filter_luma_block_padded_u8_into,
    };
    use super::*;
    use crate::BitDepth;

    const R: usize = WIENER_NS_LUMA_TAP_RADIUS;

    /// Flat areas with uneven patches, single spikes and gradients filter
    /// exactly like the per-sample callback reference, for both output types
    /// and subclass layouts.
    #[test]
    fn flat_chunks_match_the_callback_reference() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move |limit: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % limit as u64) as usize
        };
        let coeffs: Vec<[i16; WIENER_NS_LUMA_COEFFS]> = (0..2)
            .map(|class| core::array::from_fn(|tap| (tap as i16 * 5 + class * 7) % 23 - 11))
            .collect();
        for (width, height) in [(96, 16), (67, 13), (128, 9), (40, 64)] {
            let stride = width + 2 * R;
            let mut padded = vec![300u16; (height + 2 * R) * stride];
            for _ in 0..6 {
                let (x, y) = (next(stride - 6), next(height + 2 * R - 3));
                for row in y..y + 3 {
                    padded[row * stride + x..row * stride + x + 6].fill(next(1024) as u16);
                }
                let spike = next(padded.len());
                padded[spike] = 301;
            }
            for row in 0..height + 2 * R {
                padded[row * stride + stride - 20..(row + 1) * stride].fill(row as u16);
            }
            let cells: Vec<u8> = (0..width.div_ceil(4) * height.div_ceil(4))
                .map(|cell| (cell / 3 % 2) as u8)
                .collect();
            let samples: Vec<usize> = (0..width * height)
                .map(|i| usize::from(cells[(i / width / 4) * width.div_ceil(4) + i % width / 4]))
                .collect();
            let mut scratch = WienerNsLumaScratch::default();
            for bit_depth in [BitDepth::Ten, BitDepth::Eight] {
                let max = bit_depth.max_sample();
                let padded: Vec<u16> = padded.iter().map(|&value| value % (max + 1)).collect();
                let at = |x: isize, y: isize| {
                    padded[(y + R as isize) as usize * stride + (x + R as isize) as usize]
                };
                let source = WienerNsLumaPaddedSource::new(&padded, stride, width, height).unwrap();
                let params = WienerNsLumaFilter {
                    width,
                    height,
                    output_stride: width,
                    bit_depth,
                    coeffs_by_class: &coeffs,
                    subclasses: Some(&samples),
                };
                let mut reference = vec![0u16; width * height];
                wiener_ns_filter_luma_block(&mut reference, &params, at).unwrap();
                let params = WienerNsLumaFilter {
                    subclasses: None,
                    ..params
                };
                let mut actual = vec![0u16; width * height];
                wiener_ns_filter_luma_block_padded_cells_into(
                    &mut actual,
                    &params,
                    &source,
                    &cells,
                    &mut scratch,
                )
                .unwrap();
                assert_eq!(actual, reference, "{width}x{height} {bit_depth:?} cells");
                if bit_depth == BitDepth::Eight {
                    let params = WienerNsLumaFilter {
                        coeffs_by_class: &coeffs[..1],
                        ..params
                    };
                    wiener_ns_filter_luma_block(&mut reference, &params, at).unwrap();
                    let mut narrow = vec![0u8; width * height];
                    wiener_ns_filter_luma_block_padded_u8_into(
                        &mut narrow,
                        &params,
                        &source,
                        &mut scratch,
                    )
                    .unwrap();
                    let narrow: Vec<u16> = narrow.into_iter().map(u16::from).collect();
                    assert_eq!(narrow, reference, "{width}x{height} u8");
                }
            }
        }
    }

    fn window(rows: &[Vec<u16>]) -> Vec<&[u16]> {
        rows.iter().map(Vec::as_slice).collect()
    }

    /// A chunk is flat only when its whole tap reach holds one value: a
    /// differing sample anywhere in the reach makes it uneven, and one past
    /// the reach does not.
    #[test]
    fn a_differing_sample_anywhere_in_the_reach_breaks_a_chunk() {
        let (width, rows) = (64, 12);
        for row in 0..rows {
            for col in 0..=CHUNK + 2 * R {
                let mut samples = vec![vec![7u16; width + 2 * R]; rows];
                samples[row][col] = 8;
                let mut flat = FlatChunks::<R>::new();
                flat.update(&window(&samples), 0, width);
                assert_eq!(
                    flat.mask & 1 == 0,
                    col < CHUNK + 2 * R,
                    "row {row} col {col}"
                );
            }
        }
    }

    /// What one row group learns carries to the next: a flat chunk turns
    /// uneven when a new row differs, and flat again once the window has
    /// moved past the uneven row.
    #[test]
    fn chunk_state_follows_the_moving_window() {
        let (width, rows) = (32, 12);
        for odd in [(17, 3, 6), (20, 0, 9)] {
            let mut source = vec![vec![5u16; width + 2 * R]; 40];
            if odd.1 == 0 {
                source[odd.0].fill(odd.2);
            } else {
                source[odd.0][odd.1] = odd.2;
            }
            let mut flat = FlatChunks::<R>::new();
            for first in (0..source.len() - rows).step_by(4) {
                flat.update(&window(&source[first..first + rows]), first, width);
                let uneven = (first..first + rows).contains(&odd.0);
                assert_eq!(flat.any(), !uneven, "{odd:?} first {first}");
            }
        }
        let source = vec![vec![5u16; 64 + 2 * R]; rows];
        let mut flat = FlatChunks::<R>::new();
        flat.update(&window(&source), 0, 64);
        assert_eq!(flat.split(0, 40), (0, Some((5, 32))));
        assert_eq!(flat.split(32, 40), (32, Some((5, 40))));
    }
}
