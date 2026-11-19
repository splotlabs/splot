// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Four-row CDEF kernels for native byte and rebased wide samples.

use super::*;

/// [`CdefConstrain8::apply_diff`] on 32 lanes.
#[derive(Clone, Copy)]
struct CdefConstrainWide<S: SimdToken> {
    threshold: u8x32<S>,
    shift: u32,
}

impl<S: SimdToken> CdefConstrainWide<S> {
    fn new(simd: S, threshold: i32, adj: i32) -> Self {
        Self {
            threshold: u8x32::splat(simd, threshold as u8),
            shift: adj as u32,
        }
    }

    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply_diff<Q: QuadLanes<S>>(self, simd: S, quads: Q, diff: i8x32<S>) -> i8x32<S> {
        let magnitude = diff.abs().bitcast::<u8x32<S>>();
        let clip = self.threshold.saturating_sub(magnitude >> self.shift);
        quads.clamp(simd, diff, magnitude, clip)
    }

    /// `constrain(tap - center)` on `u8` samples as the tap clipped to
    /// `center -/+ clip` minus the centre: the clip is below 128, so the
    /// wrapping difference is exact.
    #[allow(clippy::inline_always, reason = "measured CDEF hot path")]
    #[inline(always)]
    fn apply(self, tap: u8x32<S>, center: u8x32<S>) -> i8x32<S> {
        let magnitude = tap.saturating_sub(center) | center.saturating_sub(tap);
        let clip = self.threshold.saturating_sub(magnitude >> self.shift);
        let clipped = tap
            .max(center.saturating_sub(clip))
            .min(center.saturating_add(clip));
        (clipped - center).bitcast()
    }
}

/// [`cdef_filter_rows_8bit`] for a segment's 8x8 luma block, four rows per
/// 32-lane vector: the window samples minus `base`, which fit `u8`, pack
/// into one vector per tap view. A primary pair, at most `2 * pri_str`, and a
/// secondary family, at most `4 * sec_str`, sum in `i8`; the rounded offset,
/// below 72 in magnitude, is clamped to the tap range in `i8` and added to
/// the centre samples.
#[inline(never)]
pub(super) fn cdef_filter_rows_u8_quad<
    S: SimdToken,
    Q: QuadLanes<S>,
    T: crate::ReconSample,
    const STRIDE: usize,
    const ROWS: usize,
    const SPAN: usize,
    const AREA: usize,
    const SIGNED: bool,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    quads: Q,
    (pad, plan, base): (&[u16; AREA], &CdefFilterPlan, u16),
    (out, out_stride): (&mut [T], usize),
) -> Option<()> {
    let filter = &plan.filter;
    let (center_view, pri_views, sec_views) = cdef_tap_views::<_, STRIDE, 2, SPAN, AREA, PRI, SEC>(
        pad,
        filter.dir,
        &CDEF_SEGMENT_TAP_STARTS,
    )?;
    let tap_row = ((filter.pri_str >> filter.coeff_shift) & 1) as usize;
    let pri_taps = CDEF_PRI_TAPS[tap_row].map(|tap| tap as i16);
    let pri = CdefConstrainWide::new(simd, filter.pri_str, plan.adj[0]);
    let sec = CdefConstrainWide::new(simd, filter.sec_str, plan.adj[1]);
    let base = u16x16::splat(simd, base);
    for_each_row_pair!([0, 4], ROWS, ROWS, |first| {
        let pairs = |view: &[u16; SPAN]| -> Option<[u16x16<S>; 2]> {
            Some([
                cdef_row_pair::<S, STRIDE, SPAN>(simd, view, first)?,
                cdef_row_pair::<S, STRIDE, SPAN>(simd, view, first + 2)?,
            ])
        };
        let quad = |view: &[u16; SPAN]| -> Option<u8x32<S>> {
            Some(quads.pack(simd, pairs(view)?.map(|pair| pair - base)))
        };
        let centers = pairs(center_view)?;
        let center = quads.pack(simd, centers.map(|pair| pair - base));
        let (mut min, mut max) = (center, center);
        macro_rules! tap {
            ($view:expr, $constrain:expr) => {{
                let tap = quad($view)?;
                if PRI && SEC {
                    min = min.min(tap);
                    max = max.max(tap);
                }
                if SIGNED {
                    $constrain.apply_diff(simd, quads, (tap - center).bitcast())
                } else {
                    $constrain.apply(tap, center)
                }
            }};
        }
        let mut sum = [i16x16::splat(simd, 0); 2];
        if PRI {
            let pair0 = quads.widen(simd, tap!(pri_views[0], pri) + tap!(pri_views[1], pri));
            let pair1 = quads.widen(simd, tap!(pri_views[2], pri) + tap!(pri_views[3], pri));
            sum = [0, 1].map(|half| pair0[half] * pri_taps[0] + pair1[half] * pri_taps[1]);
        }
        if SEC {
            let view = |k: usize| sec_views[k];
            let family0 =
                tap!(view(0), sec) + tap!(view(1), sec) + tap!(view(2), sec) + tap!(view(3), sec);
            let family1 =
                tap!(view(4), sec) + tap!(view(5), sec) + tap!(view(6), sec) + tap!(view(7), sec);
            let (family0, family1) = (quads.widen(simd, family0), quads.widen(simd, family1));
            sum = [0, 1].map(|half| sum[half] + family0[half] + family0[half] + family1[half]);
        }
        let mut offset = sum.map(cdef_round);
        if PRI && SEC {
            let below = (center - min)
                .min(u8x32::splat(simd, 127))
                .bitcast::<i8x32<S>>();
            let above = (max - center)
                .min(u8x32::splat(simd, 127))
                .bitcast::<i8x32<S>>();
            offset = quads.widen(simd, quads.narrow(simd, offset).max(-below).min(above));
        }
        for half in 0..2 {
            let filtered = (centers[half].bitcast::<i16x16<S>>() + offset[half]).bitcast();
            cdef_store_row_pair::<S, 8, _>(filtered, out, out_stride, first + 2 * half, ROWS)?;
        }
        Some(())
    });
    Some(())
}

/// Filters the byte scratch of a segment's
/// `ROWS`-row block, 8 lanes wide, four rows per 32-lane vector, for
/// filters [`cdef_wide8_fits`] admits, with `constrain` in the
/// [`cdef_window_mode`] `MODE`.
/// The constrained taps are summed in `i8` per family, the primary as
/// `2 * pair0 + pair1` for `Cdef_Pri_Taps` `[4, 2]` or `pair0 + pair1` for
/// `[3, 3]`, then widen once and take the remaining weight 2 or 3.
/// Packed chroma pairs keep the quad's lane order for deinterleaving.
///
/// Kept out of line: inlined into the kernel dispatch it ran more
/// instructions.
#[inline(never)]
pub(super) fn cdef_filter_rows_wide8<
    S: SimdToken,
    Q: QuadLanes<S>,
    T: crate::ReconSample,
    const STRIDE: usize,
    const CENTER: usize,
    const ROWS: usize,
    const SPAN8: usize,
    const AREA8: usize,
    const MODE: u8,
    const PRI: bool,
    const SEC: bool,
>(
    simd: S,
    quads: Q,
    (bytes, plan, starts): (&[u8; AREA8], &CdefFilterPlan, &CdefTapStarts),
    (out, out_stride): (&mut [T], usize),
) -> Option<()> {
    let filter = &plan.filter;
    let (center_bytes, pri_bytes, sec_bytes) =
        cdef_tap_views::<_, STRIDE, CENTER, SPAN8, AREA8, PRI, SEC>(bytes, filter.dir, starts)?;
    let packed_pair = STRIDE == LUMA_BYTE_STRIDE && ROWS == 4;
    let len = if packed_pair {
        CDEF_PAIR_OUTPUT
    } else {
        out_stride.checked_mul(ROWS - 1)?.checked_add(8)?
    };
    let out = out.get_mut(..len)?;
    let heavy_first = (filter.pri_str >> filter.coeff_shift) & 1 == 0;
    let pri_double = i8x32::splat(simd, if heavy_first { -1 } else { 0 });
    let pri_weight: i16 = if heavy_first { 2 } else { 3 };
    let pri = CdefConstrainWide::new(simd, filter.pri_str, plan.adj[0]);
    let sec = CdefConstrainWide::new(simd, filter.sec_str, plan.adj[1]);
    for_each_row_pair!([0, 4], ROWS, ROWS, |first| {
        let byte_quad = |view: &[u8; SPAN8]| -> Option<i8x32<S>> {
            if STRIDE == LUMA_BYTE_STRIDE {
                let pair = |row: usize| {
                    let at = row * STRIDE;
                    Some(u8x32::from_slice(simd, view.get(at..at + 32)?).bitcast())
                };
                return Some(quads.padded_rows(simd, pair(first)?, pair(first + 2)?));
            }
            let row = |index: usize| -> Option<u64x2<S>> {
                let at = (first + Q::ROWS[index]) * STRIDE;
                Some(u8x16::from_slice(simd, view.get(at..at + 16)?).bitcast())
            };
            let low = simd.zip_low_u64x2(row(0)?, row(1)?);
            let high = simd.zip_low_u64x2(row(2)?, row(3)?);
            Some(simd.combine_u64x2(low, high).bitcast())
        };
        let center_signed = byte_quad(center_bytes)?;
        let center = quads.widen(simd, center_signed).map(|center| center + 128);
        let (mut min, mut max) = (i8x32::splat(simd, 0), i8x32::splat(simd, 0));
        macro_rules! tap {
            ($view:expr, $constrain:expr) => {{
                let diff = byte_quad($view)?.saturating_sub(center_signed);
                if PRI && SEC {
                    min = min.min(diff);
                    max = max.max(diff);
                }
                match MODE {
                    CDEF_LINEAR => diff,
                    CDEF_CLAMPED => {
                        let strength = $constrain.threshold.bitcast::<i8x32<S>>();
                        diff.min(strength).max(-strength)
                    }
                    _ => $constrain.apply_diff(simd, quads, diff),
                }
            }};
        }
        let primary = if PRI {
            let pair0 = tap!(pri_bytes[0], pri) + tap!(pri_bytes[1], pri);
            let pair1 = tap!(pri_bytes[2], pri) + tap!(pri_bytes[3], pri);
            pair0 + pair1 + (pair0 & pri_double)
        } else {
            i8x32::splat(simd, 0)
        };
        let secondary = if SEC {
            let view = |k: usize| sec_bytes[k];
            let family0 =
                tap!(view(0), sec) + tap!(view(1), sec) + tap!(view(2), sec) + tap!(view(3), sec);
            let family1 =
                tap!(view(4), sec) + tap!(view(5), sec) + tap!(view(6), sec) + tap!(view(7), sec);
            family0 + family0 + family1
        } else {
            i8x32::splat(simd, 0)
        };
        let offset = if PRI && SEC {
            let p = quads.widen(simd, primary);
            let s = quads.widen(simd, secondary);
            let offset = core::array::from_fn(|half| cdef_round(p[half] * pri_weight + s[half]));
            quads.narrow(simd, offset).max(min).min(max)
        } else if PRI {
            quads.round_single(simd, primary, pri_weight)
        } else {
            quads.round_single(simd, secondary, 1)
        };
        if let Some(out) = T::u8_slice_mut(&mut *out) {
            let bias = u8x32::splat(simd, 0x80);
            let bytes = (center_signed + offset).bitcast::<u8x32<S>>() ^ bias;
            if packed_pair {
                bytes.store_slice(out);
            } else {
                for (lanes, &row) in bytes.to_array().chunks_exact(8).zip(&Q::ROWS) {
                    let at = (first + row) * out_stride;
                    out.get_mut(at..at + 8)?.copy_from_slice(lanes); // splot-copy-ok: publish the filtered byte rows into output
                }
            }
        } else {
            let offset = quads.widen(simd, offset);
            let out = T::u16_slice_mut(&mut *out)?;
            for half in 0..2 {
                let filtered = (center[half] + offset[half])
                    .bitcast::<u16x16<S>>()
                    .to_array();
                for (lanes, row) in filtered.chunks_exact(8).zip(first + 2 * half..) {
                    let at = row * out_stride;
                    out.get_mut(at..at + 8)?.copy_from_slice(lanes); // splot-copy-ok: publish the filtered rows into output
                }
            }
        }
        Some(())
    });
    Some(())
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// Rounds a single tap family's byte sum with its remaining weight.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    pub(super) fn round_single_avx2(t: Avx2, taps: i8x32<Avx2>, scale: i16) -> i8x32<Avx2> {
        use core::arch::x86_64::{
            _mm256_abs_epi8, _mm256_add_epi8, _mm256_and_si256, _mm256_mulhrs_epi16,
            _mm256_packus_epi16, _mm256_set1_epi8, _mm256_set1_epi16, _mm256_setzero_si256,
            _mm256_sign_epi8, _mm256_srli_epi16, _mm256_unpackhi_epi8, _mm256_unpacklo_epi8,
        };
        let taps = taps.into();
        let abs = _mm256_abs_epi8(taps);
        let offset = if scale == 1 {
            let rounded = _mm256_add_epi8(abs, _mm256_set1_epi8(8));
            _mm256_and_si256(_mm256_srli_epi16::<4>(rounded), _mm256_set1_epi8(15))
        } else {
            let zero = _mm256_setzero_si256();
            let factor = _mm256_set1_epi16(scale << 11);
            let a = _mm256_mulhrs_epi16(_mm256_unpacklo_epi8(abs, zero), factor);
            let b = _mm256_mulhrs_epi16(_mm256_unpackhi_epi8(abs, zero), factor);
            _mm256_packus_epi16(a, b)
        };
        _mm256_sign_epi8(offset, taps).simd_into(t)
    }
);
