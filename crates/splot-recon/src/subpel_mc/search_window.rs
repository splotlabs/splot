// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The eight-bit § 7.13.3.6 refine-MV search: the bilinear window of each
//! reference as bytes, and the SADs between two windows.

use super::*;

/// Predicts the `w x h` bilinear refine-MV search window of `params` (`w`
/// and `h` each 16 or 24) into `output` at stride `w` as bytes, or returns
/// `false` without writing for any other block. Its clip bounds are its
/// inner `w - 1` columns and `h - 1` rows, so the outer column and row read
/// their inner neighbour twice.
#[must_use]
pub fn subpel_predict_bilinear_search_window_u8<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u8],
) -> bool {
    let (w, h) = (params.w, params.h);
    let (x0, y0) = (
        params.start_x >> SCALE_SUBPEL_BITS,
        params.start_y >> SCALE_SUBPEL_BITS,
    );
    let Some(view) = view.as_u8() else {
        return false;
    };
    if params.interp != InterpolationFilter::Bilinear
        || (params.step_x, params.step_y) != (1 << SCALE_SUBPEL_BITS, 1 << SCALE_SUBPEL_BITS)
        || !matches!((w, h), (16 | 24, 16 | 24))
        || [params.first_x, params.last_x, params.first_y, params.last_y]
            != [x0 + 1, x0 + w as i32 - 1, y0 + 1, y0 + h as i32 - 1]
        || x0 + 1 < 0
        || y0 + 1 < 0
        || params.last_x >= view.width as i32
        || params.last_y >= view.readable_rows as i32
        || output.len() < w * h
    {
        return false;
    }
    let origin = (y0 + 1) as usize * view.stride + (x0 + 1) as usize;
    let last_load = origin + (h - 2) * view.stride + if w == 24 { 31 } else { 16 };
    if last_load > view.samples.len() || u32::try_from(last_load).is_err() {
        return false;
    }
    let phases = [
        ((params.start_x >> 6) & SUBPEL_MASK) as u8,
        ((params.start_y >> 6) & SUBPEL_MASK) as u8,
    ];
    baseline_simd!(simd => lanes::window(
        simd,
        view.samples,
        [origin, view.stride],
        [w, h],
        phases,
        output,
    ));
    true
}

/// Predicts a bilinear block of eight-bit storage, `w` a multiple of 8 and
/// `h` even, whose taps all lie inside its clip bounds and the plane into
/// `output` at `stride`, or returns `false` without writing for any other
/// block.
#[must_use]
pub fn subpel_predict_bilinear_u8<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
    stride: usize,
) -> bool {
    let (w, h) = (params.w, params.h);
    let (x0, y0) = (
        params.start_x >> SCALE_SUBPEL_BITS,
        params.start_y >> SCALE_SUBPEL_BITS,
    );
    let Some(view) = view.as_u8() else {
        return false;
    };
    if params.interp != InterpolationFilter::Bilinear
        || (params.step_x, params.step_y) != (1 << SCALE_SUBPEL_BITS, 1 << SCALE_SUBPEL_BITS)
        || w == 0
        || !w.is_multiple_of(8)
        || h == 0
        || !h.is_multiple_of(2)
        || stride < w
        || output.len() < (h - 1) * stride + w
        || x0 < params.first_x.max(0)
        || y0 < params.first_y.max(0)
        || x0 + w as i32 > params.last_x.min(view.width as i32 - 1)
        || y0 + h as i32 > params.last_y.min(view.readable_rows as i32 - 1)
    {
        return false;
    }
    let origin = y0 as usize * view.stride + x0 as usize;
    let last_load = origin + h * view.stride + w + 8;
    if last_load > view.samples.len() || u32::try_from(last_load).is_err() {
        return false;
    }
    let phases = [
        ((params.start_x >> 6) & SUBPEL_MASK) as u8,
        ((params.start_y >> 6) & SUBPEL_MASK) as u8,
    ];
    let geometry = ([origin, view.stride], [w, h]);
    baseline_simd!(simd => lanes::block(simd, view.samples, geometry, phases, output, stride));
    true
}

/// The § 7.13.3.6 search positions `(dy, dx)` in `Refinemv_Neighbors` order.
pub const REFINEMV_NEIGHBORS: [(i32, i32); 24] = {
    let mut out = [(0, 0); 24];
    let mut index = 0;
    let mut position = 0;
    while position < 25 {
        if position != 12 {
            out[index] = (position / 5 - 2, position % 5 - 2);
            index += 1;
        }
        position += 1;
    }
    out
};

/// The § 7.13.3.6 search offset `(dx, dy)` of two `stride`-wide eight-bit
/// search windows: the biased centre SAD when `allow_center` keeps it, else
/// the best of the 24 neighbours, each `Σ |w0 - w1|` over every other row of
/// the `(stride - 4) x sad_h` areas at `(2 + dy, 2 + dx)` in the first window
/// and at `(2 - dy, 2 - dx)` in the second. Returns `None` unless `stride` is
/// 16 or 24, `sad_h` is 12 or 20 and each window holds its
/// `stride x (sad_h + 4)` bytes.
#[must_use]
pub fn refinemv_search_u8(
    windows: [&[u8]; 2],
    stride: usize,
    sad_h: usize,
    allow_center: bool,
) -> Option<(i32, i32)> {
    if !matches!(stride, 16 | 24)
        || !matches!(sad_h, 12 | 20)
        || windows
            .iter()
            .any(|window| window.len() < stride * (sad_h + 4))
    {
        return None;
    }
    Some(baseline_simd!(simd => lanes::search(
        simd,
        windows,
        [stride, sad_h],
        allow_center,
    )))
}

/// The § 7.13.3.6 search over the biased `center` SAD, kept when below
/// `threshold`, and the SADs of the 24 neighbours in [`REFINEMV_NEIGHBORS`] order.
#[allow(clippy::inline_always, reason = "per refine-MV unit search")]
#[inline(always)]
fn best_offset(
    center: impl FnOnce() -> u32,
    neighbours: impl FnOnce() -> [u32; 24],
    threshold: u32,
    allow_center: bool,
) -> (i32, i32) {
    let mut best = (0, 0);
    let mut best_sad = u32::MAX;
    if allow_center {
        let center = center();
        best_sad = center - (center >> 3);
        if best_sad < threshold {
            return best;
        }
    }
    for ((dy, dx), sad) in REFINEMV_NEIGHBORS.into_iter().zip(neighbours()) {
        if sad < best_sad {
            best_sad = sad;
            best = (dx, dy);
        }
    }
    best
}

/// The bilinear windows and blocks in `u16` lanes, and the SADs in byte
/// lanes.
mod lanes {
    use fearless_simd::{Simd, prelude::*, u8x16, u8x32, u16x8, u16x16, u64x2};

    /// The source index, from `base`, of the left or right tap of output
    /// columns `first..first + 16` of an `m`-sample row, clamped to the row.
    const fn taps(first: usize, base: usize, m: usize, right: bool) -> [u8; 16] {
        let mut out = [0u8; 16];
        let mut lane = 0;
        while lane < 16 {
            let column = first + lane;
            let index = if right || column == 0 {
                column
            } else {
                column - 1
            };
            out[lane] = ((if index > m - 1 { m - 1 } else { index }) - base) as u8;
            lane += 1;
        }
        out
    }

    /// Left and right taps of columns `0..16` of a 16-wide window, then of
    /// columns `0..16` and `16..24` of a 24-wide one.
    const TAPS: [[[u8; 16]; 2]; 3] = [
        [taps(0, 0, 15, false), taps(0, 0, 15, true)],
        [taps(0, 0, 23, false), taps(0, 0, 23, true)],
        [taps(16, 15, 23, false), taps(16, 15, 23, true)],
    ];

    #[allow(clippy::inline_always, reason = "per refine-MV unit search")]
    #[inline(always)]
    fn load<S: Simd>(simd: S, bytes: &[u8], at: usize) -> u8x16<S> {
        bytes
            .get(at..)
            .and_then(<[u8]>::first_chunk::<16>)
            .map_or(u8x16::splat(simd, 0), |bytes| {
                u8x16::load_array_ref(simd, bytes)
            })
    }

    /// `Round2((16 - phase) * top + phase * bottom, 8)` of two horizontal rows.
    #[allow(clippy::inline_always, reason = "per refine-MV unit search")]
    #[inline(always)]
    fn blend<S: Simd>(simd: S, phase: u8, top: u16x8<S>, bottom: u16x8<S>) -> u16x8<S> {
        let weights = (
            u16x8::splat(simd, 16 - u16::from(phase)),
            u16x8::splat(simd, u16::from(phase)),
        );
        (top * weights.0 + bottom * weights.1 + 128) >> 8
    }

    /// The `w x h` search window, `w` and `h` each 16 or 24, of the rows of
    /// `source` from `origin`, into `output` at stride `w`.
    #[inline(never)]
    pub(super) fn window<S: Simd>(
        simd: S,
        source: &[u8],
        [origin, stride]: [usize; 2],
        [width, height]: [usize; 2],
        [h_phase, v_phase]: [u8; 2],
        output: &mut [u8],
    ) {
        let weights = (
            u8x16::splat(simd, 16 - h_phase),
            u8x16::splat(simd, h_phase),
        );
        let tables = TAPS.map(|pair| pair.map(|taps| u8x16::load_array(simd, taps)));
        if [h_phase, v_phase] == [0, 0] {
            for row in 0..height {
                let at = origin + (row.max(1) - 1) * stride;
                let Some(line) = output.get_mut(row * width..(row + 1) * width) else {
                    return;
                };
                let left = |at: usize, table: usize| {
                    simd.swizzle_dyn_within_blocks_u8x16(load(simd, source, at), tables[table][0])
                };
                if let Some(slot) = line.first_chunk_mut::<16>() {
                    left(at, usize::from(width == 24)).store_array(slot);
                }
                if width == 24
                    && let Some(slot) = line.last_chunk_mut::<8>()
                {
                    *slot = left(at + 15, 2).bitcast::<u64x2<S>>()[0].to_le_bytes();
                }
            }
            return;
        }
        let part = |bytes: u8x16<S>, [left, right]: [u8x16<S>; 2]| {
            let (l0, l1) = simd
                .swizzle_dyn_within_blocks_u8x16(bytes, left)
                .widening_mul(weights.0);
            let (r0, r1) = simd
                .swizzle_dyn_within_blocks_u8x16(bytes, right)
                .widening_mul(weights.1);
            (l0 + r0, l1 + r1)
        };
        let horizontal = |row: usize| -> [u16x8<S>; 3] {
            let at = origin + row * stride;
            if width == 24 {
                let (low, high) = part(load(simd, source, at), tables[1]);
                [low, high, part(load(simd, source, at + 15), tables[2]).0]
            } else {
                let (low, high) = part(load(simd, source, at), tables[0]);
                [low, high, low]
            }
        };
        let mut write = |row: usize, top: [u16x8<S>; 3], bottom: [u16x8<S>; 3]| {
            let [low, high, tail] =
                [0, 1, 2].map(|part| blend(simd, v_phase, top[part], bottom[part]));
            let Some(line) = output.get_mut(row * width..(row + 1) * width) else {
                return;
            };
            if let Some(slot) = line.first_chunk_mut::<16>() {
                simd.narrow_u16x8(low, high).store_array(slot);
            }
            if width == 24
                && let Some(slot) = line.last_chunk_mut::<8>()
            {
                *slot = simd.narrow_u16x8(tail, tail).bitcast::<u64x2<S>>()[0].to_le_bytes();
            }
        };
        let mut top = horizontal(0);
        write(0, top, top);
        for row in 1..height - 1 {
            let bottom = horizontal(row);
            write(row, top, bottom);
            top = bottom;
        }
        write(height - 1, top, top);
    }

    /// The bilinear `w x h` block, `w` a multiple of 8, of the rows of
    /// `source` from `origin`, into `output` at `stride`.
    #[inline(never)]
    pub(super) fn block<S: Simd>(
        simd: S,
        source: &[u8],
        ([origin, source_stride], [w, h]): ([usize; 2], [usize; 2]),
        [h_phase, v_phase]: [u8; 2],
        output: &mut [u16],
        stride: usize,
    ) {
        let weights = (
            u8x16::splat(simd, 16 - h_phase),
            u8x16::splat(simd, h_phase),
        );
        for column in (0..w).step_by(8) {
            let horizontal = |row: usize| {
                let bytes = load(simd, source, origin + column + row * source_stride);
                bytes.widening_mul(weights.0).0 + bytes.slide::<1>(bytes).widening_mul(weights.1).0
            };
            let mut top = horizontal(0);
            for row in 0..h {
                let bottom = horizontal(row + 1);
                let at = row * stride + column;
                if let Some(slot) = output.get_mut(at..).and_then(<[u16]>::first_chunk_mut::<8>) {
                    blend(simd, v_phase, top, bottom).store_array(slot);
                }
                top = bottom;
            }
        }
    }

    /// The [`super::refinemv_search_u8`] search of `[stride, sad_h]` windows.
    #[inline(never)]
    pub(super) fn search<S: Simd>(
        simd: S,
        windows: [&[u8]; 2],
        geometry: [usize; 2],
        allow_center: bool,
    ) -> (i32, i32) {
        match geometry {
            [16, 12] => search_for::<_, 16, 12, 176, 256>(simd, windows, allow_center),
            [16, _] => search_for::<_, 16, 20, 304, 384>(simd, windows, allow_center),
            [_, 12] => search_for::<_, 24, 12, 264, 384>(simd, windows, allow_center),
            _ => search_for::<_, 24, 20, 456, 576>(simd, windows, allow_center),
        }
    }

    /// [`search`] of `STRIDE`-wide windows of `AREA` bytes and `SAD_H`-row
    /// areas, each read as one `SPAN`-byte run of rows: the byte differences
    /// of each masked 12-byte row at stride 16 in 16 lanes, or 20-byte row at
    /// stride 24 in 32 lanes (bytes `0..16` and `8..24`), summed in pairs into
    /// `u16` lanes.
    #[allow(clippy::inline_always, reason = "per refine-MV unit search")]
    #[inline(always)]
    fn search_for<
        S: Simd,
        const STRIDE: usize,
        const SAD_H: usize,
        const SPAN: usize,
        const AREA: usize,
    >(
        simd: S,
        [first, second]: [&[u8]; 2],
        allow_center: bool,
    ) -> (i32, i32) {
        let (Some(first), Some(second)) =
            (first.first_chunk::<AREA>(), second.first_chunk::<AREA>())
        else {
            return (0, 0);
        };
        let narrow_mask =
            u8x16::load_array(simd, core::array::from_fn(|lane| u8::from(lane < 12) * 255));
        let wide_mask = u8x32::load_array(
            simd,
            core::array::from_fn(|lane| u8::from(lane < 16 || (24..28).contains(&lane)) * 255),
        );
        let half = |bytes: Option<&[u8; 16]>| {
            bytes.map_or(u8x16::splat(simd, 0), |bytes| {
                u8x16::load_array_ref(simd, bytes)
            })
        };
        let sad = |dy: i32, dx: i32| -> u32 {
            let start = |dy: i32, dx: i32| ((2 + dy) * STRIDE as i32 + 2 + dx) as usize;
            let (Some(left), Some(right)) = (
                first
                    .get(start(dy, dx)..)
                    .and_then(<[u8]>::first_chunk::<SPAN>),
                second
                    .get(start(-dy, -dx)..)
                    .and_then(<[u8]>::first_chunk::<SPAN>),
            ) else {
                return u32::MAX;
            };
            const { assert!(SPAN >= (SAD_H - 1) * STRIDE) };
            let (left, right) = (left.as_chunks::<STRIDE>().0, right.as_chunks::<STRIDE>().0);
            if STRIDE == 16 {
                let mut sum = u16x8::splat(simd, 0);
                for row in (0..SAD_H).step_by(2) {
                    let (x, y) = (
                        half(left[row].first_chunk()),
                        half(right[row].first_chunk()),
                    );
                    sum += ((x.max(y) - x.min(y)) & narrow_mask).pairwise_widening_add();
                }
                return u32::from(sum.reduce_sum());
            }
            let wide = |row: &[u8; STRIDE]| {
                simd.combine_u8x16(half(row.first_chunk()), half(row.last_chunk()))
            };
            let mut sum = u16x16::splat(simd, 0);
            for row in (0..SAD_H).step_by(2) {
                let (x, y) = (wide(&left[row]), wide(&right[row]));
                sum += ((x.max(y) - x.min(y)) & wide_mask).pairwise_widening_add();
            }
            u32::from(sum.reduce_sum())
        };
        super::best_offset(
            || sad(0, 0),
            || super::REFINEMV_NEIGHBORS.map(|(dy, dx)| sad(dy, dx)),
            (2 * (STRIDE - 4) * SAD_H) as u32,
            allow_center,
        )
    }
}
