// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 7.13.3.6 refine-MV search of one unit in one call: the § 7.13.3.18
//! bilinear search windows of both references under any § 7.13.3.5 clip
//! bounds, then the SAD search on them, the windows left for optical flow.

use super::search_window;
use super::*;

/// The samples of the largest search window, `(16 + 8) x (16 + 8)`.
pub const REFINEMV_SEARCH_AREA: usize = 24 * 24;

mod private {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
}

/// A search-window sample: `u8` for windows of eight-bit samples, `u16`
/// for windows of samples of any bit depth.
pub trait WindowSample: Copy + Default + private::Sealed + 'static {
    /// The samples as bytes when this type is `u8`.
    #[doc(hidden)]
    fn bytes(samples: &[Self]) -> Option<&[u8]>;
    /// The samples as words when this type is `u16`.
    #[doc(hidden)]
    fn words(samples: &[Self]) -> Option<&[u16]>;
    /// The mutable samples as bytes when this type is `u8`.
    #[doc(hidden)]
    fn bytes_mut(samples: &mut [Self]) -> Option<&mut [u8]>;
    /// The mutable samples as words when this type is `u16`.
    #[doc(hidden)]
    fn words_mut(samples: &mut [Self]) -> Option<&mut [u16]>;
}

impl WindowSample for u8 {
    fn bytes(samples: &[Self]) -> Option<&[u8]> {
        Some(samples)
    }
    fn words(_: &[Self]) -> Option<&[u16]> {
        None
    }
    fn bytes_mut(samples: &mut [Self]) -> Option<&mut [u8]> {
        Some(samples)
    }
    fn words_mut(_: &mut [Self]) -> Option<&mut [u16]> {
        None
    }
}

impl WindowSample for u16 {
    fn bytes(_: &[Self]) -> Option<&[u8]> {
        None
    }
    fn words(samples: &[Self]) -> Option<&[u16]> {
        Some(samples)
    }
    fn bytes_mut(_: &mut [Self]) -> Option<&mut [u8]> {
        None
    }
    fn words_mut(samples: &mut [Self]) -> Option<&mut [u16]> {
        Some(samples)
    }
}

/// Builds the bilinear § 7.13.3.6 search windows of one refine-MV unit of
/// both references and runs the SAD search on them.
///
/// `params[i]` is the `(unit_w + 8) x (unit_h + 8)` window of reference `i`
/// (`unit_w` and `unit_h` each 8 or 16) at its § 7.13.3.17 start with the
/// § 7.13.3.5 clip bounds of the unit; `windows[i]` receives it at stride
/// `unit_w + 8` (the § 7.13.3.18 prediction with the `isCompound = 0`
/// rounding of § 7.13.3.16, every tap clipped to the bounds on its axis).
/// Returns the search offset `(dx, dy)`: the centre when `allow_center` and
/// its biased SAD is below the threshold, else the strictly best of the 24
/// `Refinemv_Neighbors`, each SAD over every other row of the area two
/// samples in, shifted down by `BitDepth - 8`. Both windows are written but
/// for their last row, which neither the SAD (rows up to `h - 2`) nor the
/// optical flow of the unit at `(4 + dx, 4 + dy)` (rows up to `h - 3`)
/// reads; when the centre of `u16` windows ends the search only their
/// `(w - 4) x (h - 4)` centre at `(2, 2)` is written: all that the SAD and
/// the optical flow of a `(0, 0)` result read. An unclamped byte window
/// takes the tuned fixed-pattern kernel and every other window the planned
/// kernel, a specialisation chosen by the window's bounds, both tested
/// against the same § 7.13.3.18 oracle; the centre of `u16` windows takes
/// the general § 7.13.3.18 prediction, which no bound clamps there.
///
/// Returns `None`, with the windows unspecified, when the caller's contract
/// does not hold: a scaled or non-bilinear window, another window size,
/// inverted or negative bounds, byte bounds wider than the window's inner
/// columns, `P = u8` with a `u16` view or a bit depth above 8, or a read
/// outside the view's published rows, width or storage.
#[must_use]
pub fn refinemv_search_windows<T: ReconSample, P: WindowSample>(
    views: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    allow_center: bool,
    bit_depth: BitDepth,
    windows: [&mut [P; REFINEMV_SEARCH_AREA]; 2],
) -> Option<(i32, i32)> {
    refinemv_search_windows_with_runner(
        BaselineSubpel,
        views,
        params,
        allow_center,
        bit_depth,
        windows,
    )
}

/// [`refinemv_search_windows`] using caller-selected `backend`.
#[must_use]
pub fn refinemv_search_windows_with<T: ReconSample, P: WindowSample>(
    backend: SimdBackend,
    views: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    allow_center: bool,
    bit_depth: BitDepth,
    windows: [&mut [P; REFINEMV_SEARCH_AREA]; 2],
) -> Option<(i32, i32)> {
    refinemv_search_windows_with_runner(
        TypedSubpel(backend),
        views,
        params,
        allow_center,
        bit_depth,
        windows,
    )
}

fn refinemv_search_windows_with_runner<R: SubpelRunner, T: ReconSample, P: WindowSample>(
    runner: R,
    views: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    allow_center: bool,
    bit_depth: BitDepth,
    windows: [&mut [P; REFINEMV_SEARCH_AREA]; 2],
) -> Option<(i32, i32)> {
    let size = [params[0].w, params[0].h];
    let bytes = T::u8_slice(&[]).is_some();
    if !matches!(size, [16 | 24, 16 | 24])
        || [params[1].w, params[1].h] != size
        || (P::bytes(&[]).is_some() && (!bytes || bit_depth != BitDepth::Eight))
    {
        return None;
    }
    let lanes = if bytes { 16 } else { 8 };
    let [first, second] = windows;
    let geometry = [size[0], size[1] - 4];
    let shift = u32::from(bit_depth.bits().saturating_sub(8));
    if allow_center && P::words(&[]).is_some() {
        for ((view, params), window) in views
            .into_iter()
            .zip(params)
            .zip([&mut *first, &mut *second])
        {
            predict_centre(runner, view, params, size, window)?;
        }
        let words = [&first[..], &second[..]].map(|window| P::words(window).unwrap_or_default());
        if centre_ends_search(runner, words, geometry, shift)? {
            return Some((0, 0));
        }
    }
    for ((view, params), window) in views
        .into_iter()
        .zip(params)
        .zip([&mut *first, &mut *second])
    {
        if let Some(output) = P::bytes_mut(window)
            && search_window::subpel_predict_bilinear_search_window_u8(runner, view, params, output)
        {
            continue;
        }
        predict_window(runner, view, &plan(view, params, lanes)?, size, window);
    }
    if let (Some(first), Some(second)) = (P::bytes(&first[..]), P::bytes(&second[..])) {
        return Some(search_bytes(
            runner,
            [first, second],
            geometry,
            allow_center,
        ));
    }
    let (first, second) = (P::words(&first[..])?, P::words(&second[..])?);
    Some(search_words(
        runner,
        [first, second],
        geometry,
        allow_center,
        shift,
    ))
}

/// Predicts the `(w - 4) x (h - 4)` centre at `(2, 2)` of the `u16` window
/// `params` into `window` at stride `w`, after the checks of [`plan`].
fn predict_centre<R: SubpelRunner, T: ReconSample, P: WindowSample>(
    runner: R,
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    [w, h]: [usize; 2],
    window: &mut [P],
) -> Option<()> {
    checked(view, params)?;
    let inset = 2 << SCALE_SUBPEL_BITS;
    let centre = SubpelPredictParams {
        w: w - 4,
        h: h - 4,
        start_x: params.start_x + inset,
        start_y: params.start_y + inset,
        ..*params
    };
    let output = P::words_mut(window)?.get_mut(2 * w + 2..)?;
    subpel_predict_block_strided_into_with_runner(runner, view, &centre, output, w).ok()
}

/// Whether the biased centre SAD of two `u16` windows of `[stride, sad_h]`
/// keeps the centre, from their centre alone.
fn centre_ends_search<R: SubpelRunner>(
    runner: R,
    windows: [&[u16]; 2],
    geometry: [usize; 2],
    shift: u32,
) -> Option<bool> {
    let sad = runner.refinemv_centre_sad(windows, geometry)? >> shift;
    Some(sad - (sad >> 3) < (2 * (geometry[0] - 4) * geometry[1]) as u32)
}

#[expect(
    clippy::inline_always,
    reason = "keep the centre SAD in the caller's SIMD scope"
)]
#[inline(always)]
pub(super) fn centre_sad_generic<S: fearless_simd::Simd>(
    simd: S,
    windows: [&[u16]; 2],
    geometry: [usize; 2],
) -> Option<u32> {
    match geometry {
        [16, 12] => generic::centre_sad::<_, 16, 12, 256>(simd, windows),
        [16, _] => generic::centre_sad::<_, 16, 20, 384>(simd, windows),
        [_, 12] => generic::centre_sad::<_, 24, 12, 384>(simd, windows),
        _ => generic::centre_sad::<_, 24, 20, 576>(simd, windows),
    }
}

/// The SAD search of two word windows of `[stride, sad_h]`, each SAD
/// shifted down by `shift`.
pub(super) fn search_words<R: SubpelRunner>(
    runner: R,
    windows: [&[u16]; 2],
    geometry: [usize; 2],
    allow_center: bool,
    shift: u32,
) -> (i32, i32) {
    runner.refinemv_search_words(windows, geometry, allow_center, shift)
}

pub(super) fn search_words_generic<S: fearless_simd::Simd>(
    simd: S,
    windows: [&[u16]; 2],
    geometry: [usize; 2],
    allow_center: bool,
    shift: u32,
) -> (i32, i32) {
    match geometry {
        [16, 12] => {
            search_window::generic::search_u16::<_, 16, 12, 256>(simd, windows, allow_center, shift)
        }
        [16, _] => {
            search_window::generic::search_u16::<_, 16, 20, 384>(simd, windows, allow_center, shift)
        }
        [_, 12] => {
            search_window::generic::search_u16::<_, 24, 12, 384>(simd, windows, allow_center, shift)
        }
        _ => {
            search_window::generic::search_u16::<_, 24, 20, 576>(simd, windows, allow_center, shift)
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn search_bytes_avx2(
    t: fearless_simd::Avx2,
    windows: [&[u8]; 2],
    geometry: [usize; 2],
    allow_center: bool,
) -> (i32, i32) {
    search_window::avx2::search(t, windows, geometry, allow_center)
}

#[cfg(target_arch = "aarch64")]
fn search_bytes_neon(
    t: fearless_simd::Neon,
    windows: [&[u8]; 2],
    geometry: [usize; 2],
    allow_center: bool,
) -> (i32, i32) {
    search_window::neon::search(t, windows, geometry, allow_center)
}

/// The SAD search of two byte windows of `[stride, sad_h]`.
pub(super) fn search_bytes<R: SubpelRunner>(
    runner: R,
    windows: [&[u8]; 2],
    geometry: [usize; 2],
    allow_center: bool,
) -> (i32, i32) {
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = runner.avx2_token() {
        return search_bytes_avx2(t, windows, geometry, allow_center);
    }
    #[cfg(target_arch = "aarch64")]
    if let Some(t) = runner.neon_token() {
        return search_bytes_neon(t, windows, geometry, allow_center);
    }
    runner.refinemv_search_bytes_generic(windows, geometry, allow_center)
}

/// Where one window reads its reference: `[y0, first, last]` of its rows,
/// row `i` reading source row `clamp(y0 + i, first, last)` for `i` in
/// `0..=h`, the view stride, and per eight-column group and tap (left,
/// right) the load column and the `[a, lo, hi]` that map group column `j` to
/// lane `clamp(a + j, lo, hi)` of that load. In a byte view both taps of
/// columns `0..16` share one 16-byte load, and those of `16..24` another.
/// Every load of every row lies in the view's storage, which `u32` indexes.
#[derive(Clone, Copy, Debug)]
pub(super) struct Plan {
    pub(super) rows: [i32; 3],
    pub(super) stride: u32,
    pub(super) loads: [[i32; 2]; 3],
    pub(super) taps: [[[i8; 3]; 2]; 3],
    pub(super) phases: [u8; 2],
}

impl Plan {
    /// The storage offset of the source row of window row 0.
    #[inline]
    pub(super) fn first_row(&self) -> u32 {
        let [y0, first, last] = self.rows;
        y0.max(first).min(last) as u32 * self.stride
    }

    /// The storage offset of the source row of window row `i` from that of
    /// row `i - 1`: one row down inside the clip bounds, else the same row.
    #[expect(clippy::inline_always, reason = "per window row")]
    #[inline(always)]
    pub(super) fn next_row(&self, row: u32, i: usize) -> u32 {
        let [y0, first, last] = self.rows;
        let inside = ((i as i32 + y0 - first - 1) as u32) < (last - first) as u32;
        if inside { row + self.stride } else { row }
    }

    /// The storage offset of the load of `tap` of `group` in the source
    /// row at `row`.
    #[expect(clippy::inline_always, reason = "per window row")]
    #[inline(always)]
    pub(super) fn at(&self, row: u32, group: usize, tap: usize) -> u32 {
        row.wrapping_add_signed(self.loads[group][tap])
    }
}

/// The `N` samples of `source` from `at`, or zeros when they leave it: a
/// `u32` start ends one compare from the storage end.
#[expect(clippy::inline_always, reason = "per window row")]
#[inline(always)]
pub(super) fn lanes_at<T, const N: usize>(source: &[T], at: u32) -> Option<&[T; N]> {
    let at = at as usize;
    if at + N > source.len() {
        return None;
    }
    source.get(at..)?.first_chunk::<N>()
}

/// The checks of the window `params` in `view` that every kernel needs:
/// bilinear and unscaled, bounds in order, and every read inside the
/// view's width, published rows and `u32`-indexed storage.
#[inline]
fn checked<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Option<()> {
    let unscaled = 1 << SCALE_SUBPEL_BITS;
    let [x0, y0] = [params.start_x, params.start_y].map(|start| start >> SCALE_SUBPEL_BITS);
    let [first_x, first_y, last_x, last_y] = params.bounds();
    let (w, h) = (params.w as i32, params.h as i32);
    (params.interp == InterpolationFilter::Bilinear
        && (params.step_x, params.step_y) == (unscaled, unscaled)
        && first_x >= 0
        && first_y >= 0
        && first_x <= last_x
        && first_y <= last_y
        && ((x0 + w).max(first_x).min(last_x) as usize) < view.width
        && ((y0 + h).max(first_y).min(last_y) as usize) < view.readable_rows
        && u32::try_from(view.samples.len()).is_ok()
        && u32::try_from(view.stride).is_ok())
    .then_some(())
}

/// The [`Plan`] of the window `params` in `view` with loads of `lanes`
/// samples, or `None` when the window is outside the contract.
#[inline]
pub(super) fn plan<T: ReconSample>(
    view: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    lanes: i32,
) -> Option<Plan> {
    checked(view, params)?;
    let [x0, y0] = [params.start_x, params.start_y].map(|start| start >> SCALE_SUBPEL_BITS);
    let [first_x, first_y, last_x, last_y] = params.bounds();
    let (w, h) = (params.w as i32, params.h as i32);
    let column = |j: i32| (x0 + j).max(first_x).min(last_x);
    let row = |i: i32| i64::from((y0 + i).max(first_y).min(last_y)) * view.stride as i64;
    let (top, bottom) = (row(0), row(h));
    let room = view.samples.len() as i64 - i64::from(lanes) - bottom;
    let fit = |first: i32, last: i32| -> Option<i32> {
        let base = i64::from(first).min(room);
        (base > i64::from(last - lanes) && top + base >= 0).then_some(base as i32)
    };
    let tap = |group: i32, base: i32| {
        let near = |value: i32| (value - base).clamp(-64, 64) as i8;
        [near(x0 + 8 * group), near(first_x), near(last_x)]
    };
    let mut loads = [[0; 2]; 3];
    let mut taps = [[[0; 3]; 2]; 3];
    if lanes == 16 {
        let low = fit(column(0), column(w.min(16)))?;
        let [a, lo, hi] = tap(0, low);
        (loads[0][0], loads[1][0]) = (low, low);
        (taps[0][0], taps[1][0]) = ([a, lo, hi], [a + 8, lo, hi]);
        if w == 24 {
            let high = fit(column(16), column(24))?;
            (loads[2][0], taps[2][0]) = (high, tap(2, high));
        }
    } else {
        for group in 0..params.w / 8 {
            for k in 0..2 {
                let first = 8 * group as i32 + k as i32;
                let base = fit(column(first), column(first + 7))?;
                (loads[group][k], taps[group][k]) = (base, tap(group as i32, base));
            }
        }
    }
    Some(Plan {
        rows: [y0, first_y, last_y],
        stride: view.stride as u32,
        loads,
        taps,
        phases: search_window::phases(params),
    })
}

/// Predicts one planned window of `size` into `window`.
pub(super) fn predict_window<R: SubpelRunner, T: ReconSample, P: WindowSample>(
    runner: R,
    view: &ReferencePlaneView<'_, T>,
    plan: &Plan,
    size: [usize; 2],
    window: &mut [P],
) {
    #[cfg(target_arch = "x86_64")]
    if let Some(t) = runner.avx2_token() {
        if let (Some(source), Some(output)) = (T::u8_slice(view.samples), P::bytes_mut(window)) {
            avx2::window_u8(t, source, plan, size, output);
            return;
        }
        if let (Some(source), Some(output)) = (T::u16_slice(view.samples), P::words_mut(window)) {
            avx2::window_u16(t, source, plan, size, output);
            return;
        }
    }
    runner.refinemv_predict_window_generic(view.samples, plan, size, window);
}

/// The windows in generic `u16` lanes, eight columns per group.
pub(super) mod generic {
    use super::{Plan, ReconSample, WindowSample};
    use fearless_simd::{Simd, i8x16, prelude::*, u8x16, u16x8, u64x2};

    /// Group column `j` of each lane: the left taps then the right taps of
    /// eight columns.
    const BYTE_TAPS: [i8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 1, 2, 3, 4, 5, 6, 7, 8];

    /// Group column `j` of each byte of the left `u16` taps of eight columns.
    const WORD_TAPS: [i8; 16] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7];

    /// The lane indices `clamp(a + pattern, lo, hi)` of one tap.
    fn lanes<S: Simd>(simd: S, pattern: [i8; 16], [a, lo, hi]: [i8; 3]) -> i8x16<S> {
        (i8x16::load_array(simd, pattern) + a)
            .max(i8x16::splat(simd, lo))
            .min(i8x16::splat(simd, hi))
    }

    /// `Σ |a - b|` over every other row of the `(STRIDE - 4) x SAD_H` centres
    /// at `(2, 2)` of two `STRIDE`-wide windows of `AREA` samples, summed as
    /// the word search sums it, or `None` for a short window.
    #[expect(
        clippy::inline_always,
        reason = "keep the centre SAD in the caller's SIMD scope"
    )]
    #[inline(always)]
    pub(super) fn centre_sad<
        S: Simd,
        const STRIDE: usize,
        const SAD_H: usize,
        const AREA: usize,
    >(
        simd: S,
        [first, second]: [&[u16]; 2],
    ) -> Option<u32> {
        let (first, second) = (first.first_chunk::<AREA>()?, second.first_chunk::<AREA>()?);
        let zero = u16x8::splat(simd, 0);
        let high = u16x8::load_array(simd, [0, 0, 0, 0, u16::MAX, u16::MAX, u16::MAX, u16::MAX]);
        let lanes = |window: &[u16; AREA], at: usize| {
            window
                .get(at..)
                .and_then(<[u16]>::first_chunk::<8>)
                .map_or(zero, |lanes| u16x8::load_array_ref(simd, lanes))
        };
        let width = STRIDE - 4;
        let mut sums = [zero; 3];
        for row in (0..SAD_H).step_by(2) {
            let diff = |at: usize| {
                let at = (2 + row) * STRIDE + 2 + at;
                let (x, y) = (lanes(first, at), lanes(second, at));
                x.max(y) - x.min(y)
            };
            for (column, sum) in sums.iter_mut().enumerate().take(width / 8) {
                *sum += diff(8 * column);
            }
            sums[2] += diff(width - 8) & high;
        }
        let total = sums
            .iter()
            .fold(fearless_simd::u32x4::splat(simd, 0), |total, sum| {
                let (low, high) = sum.widen();
                total + low + high
            });
        Some(total.reduce_sum())
    }

    /// The `w x h` window of `plan` from `source` into `output` at stride `w`.
    #[inline(never)]
    pub(in crate::subpel_mc) fn window<S: Simd, T: ReconSample, P: WindowSample>(
        simd: S,
        source: &[T],
        plan: &Plan,
        [w, h]: [usize; 2],
        output: &mut [P],
    ) {
        let [h_phase, v_phase] = plan.phases.map(u16::from);
        let groups = w / 8;
        if let Some(source) = T::u8_slice(source) {
            let indices: [u8x16<S>; 3] =
                core::array::from_fn(|group| lanes(simd, BYTE_TAPS, plan.taps[group][0]).bitcast());
            let weights = u8x16::load_array(
                simd,
                core::array::from_fn(|lane| {
                    if lane < 8 {
                        16 - h_phase as u8
                    } else {
                        h_phase as u8
                    }
                }),
            );
            let horizontal = |row: u32| -> [u16x8<S>; 3] {
                core::array::from_fn(|group| {
                    if group >= groups {
                        return u16x8::splat(simd, 0);
                    }
                    let bytes = super::lanes_at::<_, 16>(source, plan.at(row, group, 0))
                        .map_or(u8x16::splat(simd, 0), |bytes| {
                            u8x16::load_array_ref(simd, bytes)
                        });
                    let (left, right) = simd
                        .swizzle_dyn_within_blocks_u8x16(bytes, indices[group])
                        .widening_mul(weights);
                    left + right
                })
            };
            let weights = (16 - v_phase, v_phase);
            let blend =
                |top: u16x8<S>, bottom: u16x8<S>| (top * weights.0 + bottom * weights.1 + 128) >> 8;
            rows(plan, [groups, h], horizontal, blend, output);
            return;
        }
        let Some(source) = T::u16_slice(source) else {
            return;
        };
        let indices: [[u8x16<S>; 2]; 3] = core::array::from_fn(|group| {
            core::array::from_fn(|tap| {
                let pairs = lanes(
                    simd,
                    WORD_TAPS.map(|j| j + tap as i8),
                    plan.taps[group][tap],
                );
                let odd = u8x16::load_array(simd, core::array::from_fn(|lane| (lane % 2) as u8));
                (pairs.bitcast::<u8x16<S>>() + pairs.bitcast::<u8x16<S>>()) + odd
            })
        });
        let load = |at: u32| {
            super::lanes_at::<_, 8>(source, at).map_or(u16x8::splat(simd, 0), |lanes| {
                u16x8::load_array_ref(simd, lanes)
            })
        };
        let horizontal = |row: u32| -> [u16x8<S>; 3] {
            core::array::from_fn(|group| {
                if group >= groups {
                    return u16x8::splat(simd, 0);
                }
                let [left, right] = [0, 1].map(|tap| {
                    let lanes = load(plan.at(row, group, tap)).bitcast::<u8x16<S>>();
                    simd.swizzle_dyn_within_blocks_u8x16(lanes, indices[group][tap])
                        .bitcast::<u16x8<S>>()
                });
                left * (16 - h_phase) + right * h_phase
            })
        };
        let blend = |top: u16x8<S>, bottom: u16x8<S>| {
            let (top_low, top_high) = top.widening_mul(u16x8::splat(simd, 16 - v_phase));
            let (bottom_low, bottom_high) = bottom.widening_mul(u16x8::splat(simd, v_phase));
            ((top_low + bottom_low + 128) >> 8).narrow((top_high + bottom_high + 128) >> 8)
        };
        rows(plan, [groups, h], horizontal, blend, output);
    }

    /// Blends the horizontal rows of `plan` vertically into the `h` output
    /// rows of `groups` eight-sample groups, filtering each distinct source
    /// row once.
    #[expect(clippy::inline_always, reason = "per refine-MV window")]
    #[inline(always)]
    fn rows<S: Simd, P: WindowSample>(
        plan: &Plan,
        [groups, h]: [usize; 2],
        horizontal: impl Fn(u32) -> [u16x8<S>; 3],
        blend: impl Fn(u16x8<S>, u16x8<S>) -> u16x8<S>,
        output: &mut [P],
    ) {
        let width = 8 * groups;
        let mut source = plan.first_row();
        let mut top = horizontal(source);
        for row in 0..h - 1 {
            let next = plan.next_row(source, row + 1);
            let bottom = if next == source {
                top
            } else {
                horizontal(next)
            };
            let lanes = [0, 1, 2].map(|group| blend(top[group], bottom[group]));
            let Some(line) = output.get_mut(row * width..(row + 1) * width) else {
                return;
            };
            if let Some(line) = P::bytes_mut(line) {
                let simd = lanes[0].simd;
                if let Some(slot) = line.first_chunk_mut::<16>() {
                    simd.narrow_u16x8(lanes[0], lanes[1]).store_array(slot);
                }
                if groups == 3
                    && let Some(slot) = line.last_chunk_mut::<8>()
                {
                    let tail = simd.narrow_u16x8(lanes[2], lanes[2]);
                    *slot = tail.bitcast::<u64x2<S>>()[0].to_le_bytes();
                }
            } else if let Some(line) = P::words_mut(line) {
                for (slot, lanes) in line.as_chunks_mut::<8>().0.iter_mut().zip(lanes) {
                    lanes.store_array(slot);
                }
            }
            (source, top) = (next, bottom);
        }
    }
}

/// The windows in AVX2, two eight-column groups per `ymm`: byte taps paired
/// by `pshufb` from one broadcast load per 16 columns and filtered by
/// `pmaddubsw`, word taps gathered by `pshufb` from a load per group and tap
/// and blended vertically by `pmaddwd`.
#[cfg(target_arch = "x86_64")]
mod avx2 {
    use super::Plan;
    use core::arch::x86_64::*;
    use fearless_simd::{Avx2, SimdBase, SimdFrom, u8x16, u16x8, u16x16};

    /// Group column `j` of each `(left, right)` byte pair of eight columns.
    const PAIRS: [u8; 16] = [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8];

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn window_u8(
            t: Avx2,
            source: &[u8],
            plan: &Plan,
            size: [usize; 2],
            output: &mut [u8],
        ) {
            match size {
                [16, 16] => rows::<16, 16, 256>(t, source, plan, output),
                [16, _] => rows::<16, 24, 384>(t, source, plan, output),
                [_, 16] => rows::<24, 16, 384>(t, source, plan, output),
                _ => rows::<24, 24, 576>(t, source, plan, output),
            };
        }
    );

    /// [`window_u8`] of a `W x H` window of `N` bytes.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn rows<const W: usize, const H: usize, const N: usize>(
        t: Avx2,
        source: &[u8],
        plan: &Plan,
        output: &mut [u8],
    ) -> Option<()> {
        let output = output.first_chunk_mut::<N>()?;
        let pairs: __m128i = u8x16::load_array(t, PAIRS).into();
        let index = |group: usize| {
            let [a, lo, hi] = plan.taps[group][0];
            let lanes = _mm_add_epi8(pairs, _mm_set1_epi8(a));
            _mm_min_epi8(_mm_max_epi8(lanes, _mm_set1_epi8(lo)), _mm_set1_epi8(hi))
        };
        let low = _mm256_set_m128i(index(1), index(0));
        let high = if W == 24 {
            _mm256_broadcastsi128_si256(index(2))
        } else {
            low
        };
        let [h_phase, v_phase] = plan.phases.map(i16::from);
        let weights = _mm256_set1_epi16((h_phase << 8) | (16 - h_phase));
        let load = |at: u32| -> __m128i {
            super::lanes_at::<_, 16>(source, at).map_or(_mm_setzero_si128(), |bytes| {
                u8x16::load_array_ref(t, bytes).into()
            })
        };
        let horizontal = |row: u32| -> [__m256i; 2] {
            let at = |group: usize| plan.at(row, group, 0);
            let bytes = _mm256_broadcastsi128_si256(load(at(0)));
            let first = _mm256_maddubs_epi16(_mm256_shuffle_epi8(bytes, low), weights);
            if W != 24 {
                return [first, first];
            }
            let bytes = _mm256_broadcastsi128_si256(load(at(2)));
            [
                first,
                _mm256_maddubs_epi16(_mm256_shuffle_epi8(bytes, high), weights),
            ]
        };
        let (top_weight, bottom_weight) =
            (_mm256_set1_epi16(16 - v_phase), _mm256_set1_epi16(v_phase));
        let bias = _mm256_set1_epi16(128);
        let blend = |top: __m256i, bottom: __m256i| {
            let sum = _mm256_add_epi16(
                _mm256_mullo_epi16(top, top_weight),
                _mm256_mullo_epi16(bottom, bottom_weight),
            );
            _mm256_srli_epi16::<8>(_mm256_add_epi16(sum, bias))
        };
        let lines = output.as_chunks_mut::<W>().0;
        let mut source_row = plan.first_row();
        let mut top = horizontal(source_row);
        for (row, line) in lines.iter_mut().enumerate().take(H - 1) {
            let next = plan.next_row(source_row, row + 1);
            let bottom = if next == source_row {
                top
            } else {
                horizontal(next)
            };
            let packed = _mm256_permute4x64_epi64::<0b1101_1000>(_mm256_packus_epi16(
                blend(top[0], bottom[0]),
                blend(top[1], bottom[1]),
            ));
            if let Some(bytes) = line.first_chunk_mut::<16>() {
                u8x16::<Avx2>::simd_from(t, _mm256_castsi256_si128(packed)).store_array(bytes);
            }
            if W == 24
                && let Some(bytes) = line.last_chunk_mut::<8>()
            {
                *bytes = _mm_cvtsi128_si64(_mm256_extracti128_si256::<1>(packed)).to_le_bytes();
            }
            (source_row, top) = (next, bottom);
        }
        Some(())
    }

    /// Group column `j` of each byte of the left `u16` taps of eight columns.
    const WORDS: [u8; 16] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7];

    fearless_simd::kernel!(
        #[inline]
        pub(super) fn window_u16(
            t: Avx2,
            source: &[u16],
            plan: &Plan,
            size: [usize; 2],
            output: &mut [u16],
        ) {
            match size {
                [16, 16] => words::<16, 16, 256>(t, source, plan, output),
                [16, _] => words::<16, 24, 384>(t, source, plan, output),
                [_, 16] => words::<24, 16, 384>(t, source, plan, output),
                _ => words::<24, 24, 576>(t, source, plan, output),
            };
        }
    );

    /// [`window_u16`] of a `W x H` window of `N` samples.
    #[inline]
    #[target_feature(enable = "avx2")]
    fn words<const W: usize, const H: usize, const N: usize>(
        t: Avx2,
        source: &[u16],
        plan: &Plan,
        output: &mut [u16],
    ) -> Option<()> {
        let output = output.first_chunk_mut::<N>()?;
        let pattern: __m128i = u8x16::load_array(t, WORDS).into();
        let odd = _mm_set1_epi16(0x0100);
        let index = |group: usize, tap: usize| {
            let [a, lo, hi] = plan.taps[group][tap];
            let lanes = _mm_add_epi8(pattern, _mm_set1_epi8(a + tap as i8));
            let lanes = _mm_min_epi8(_mm_max_epi8(lanes, _mm_set1_epi8(lo)), _mm_set1_epi8(hi));
            _mm_add_epi8(_mm_add_epi8(lanes, lanes), odd)
        };
        let low = [0, 1].map(|tap| _mm256_set_m128i(index(1, tap), index(0, tap)));
        let high = [0, 1].map(|tap| {
            if W == 24 {
                _mm256_broadcastsi128_si256(index(2, tap))
            } else {
                low[tap]
            }
        });
        let [h_phase, v_phase] = plan.phases.map(i16::from);
        let (left, right) = (_mm256_set1_epi16(16 - h_phase), _mm256_set1_epi16(h_phase));
        let load = |at: u32| -> __m128i {
            super::lanes_at::<_, 8>(source, at).map_or(_mm_setzero_si128(), |lanes| {
                u16x8::load_array_ref(t, lanes).into()
            })
        };
        let horizontal = |row: u32| -> [__m256i; 2] {
            let at = |group: usize, tap: usize| plan.at(row, group, tap);
            let filter = |taps: [__m256i; 2], indices: &[__m256i; 2]| {
                let [l, r] = [0, 1].map(|tap| _mm256_shuffle_epi8(taps[tap], indices[tap]));
                _mm256_add_epi16(_mm256_mullo_epi16(l, left), _mm256_mullo_epi16(r, right))
            };
            let first = filter(
                [0, 1].map(|tap| _mm256_set_m128i(load(at(1, tap)), load(at(0, tap)))),
                &low,
            );
            if W != 24 {
                return [first, first];
            }
            let taps = [0, 1].map(|tap| _mm256_broadcastsi128_si256(load(at(2, tap))));
            [first, filter(taps, &high)]
        };
        let weights = _mm256_set1_epi32((i32::from(v_phase) << 16) | i32::from(16 - v_phase));
        let bias = _mm256_set1_epi32(128);
        let blend = |top: __m256i, bottom: __m256i| {
            let half = |pairs: __m256i| {
                _mm256_srli_epi32::<8>(_mm256_add_epi32(_mm256_madd_epi16(pairs, weights), bias))
            };
            _mm256_packus_epi32(
                half(_mm256_unpacklo_epi16(top, bottom)),
                half(_mm256_unpackhi_epi16(top, bottom)),
            )
        };
        let lines = output.as_chunks_mut::<W>().0;
        let mut source_row = plan.first_row();
        let mut top = horizontal(source_row);
        for (row, line) in lines.iter_mut().enumerate().take(H - 1) {
            let next = plan.next_row(source_row, row + 1);
            let bottom = if next == source_row {
                top
            } else {
                horizontal(next)
            };
            if let Some(slot) = line.first_chunk_mut::<16>() {
                u16x16::<Avx2>::simd_from(t, blend(top[0], bottom[0])).store_array(slot);
            }
            if W == 24
                && let Some(slot) = line.last_chunk_mut::<8>()
            {
                let tail = _mm256_castsi256_si128(blend(top[1], bottom[1]));
                u16x8::<Avx2>::simd_from(t, tail).store_array(slot);
            }
            (source_row, top) = (next, bottom);
        }
        Some(())
    }
}
