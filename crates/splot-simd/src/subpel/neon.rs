// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AArch64 NEON § 7.13.3.18 filter passes.
//!
//! The module is gated on `target_feature = "neon"` being in the build
//! configuration, so every intrinsic's required feature is statically present.
//! The kernels are therefore plain functions rather than
//! `#[target_feature(enable = "neon")]` ones: the attribute would bar the
//! kernel from being inlined into any caller that does not carry it, which
//! costs a call per filtered row. What the attribute would have provided —
//! permission to name the intrinsics — this crate has instead as the `unsafe`
//! its quarantine exists for.

use super::{NUM_TAPS, round2};
use core::arch::aarch64::{
    int16x8_t, int32x4_t, vcombine_s16, vdupq_n_s16, vdupq_n_s32, vextq_s16, vget_low_s16,
    vld1q_u16, vmlal_high_s16, vmlal_s16, vmovn_s32, vreinterpretq_s16_u16, vrshrq_n_s32, vst1_s16,
    vst1q_s16,
};

/// AV2 § 7.13.3.16 `InterRound0`, the only horizontal-pass down-shift this
/// kernel serves. § 7.13.3.16 fixes it at 3, and the rounding shift needs a
/// constant, so any other value is refused back to the caller.
const HORIZONTAL_ROUND: u32 = 3;

/// Samples the two loads of one eight-lane window group read from its origin.
///
/// The windows use only `NUM_TAPS - 1 + 8` of them; the rest of the second load
/// is discarded but still has to be readable, which is what [`source_reach`]
/// accounts for.
const SPAN: usize = 16;

/// Samples one horizontal row needs past `window`'s origin for the load shape.
///
/// `None` when the arithmetic overflows, which refuses the shape rather than
/// wrapping.
#[allow(
    clippy::inline_always,
    reason = "one call per filtered row on the measured subpel hot path"
)]
#[inline(always)]
fn source_reach(width: usize, tap_start: usize, tap_end: usize) -> Option<usize> {
    let mut reach = width.checked_add(tap_end)?.checked_sub(1)?;
    let vector8 = width - width % 8;
    if vector8 >= 8 {
        reach = reach.max((vector8 - 8).checked_add(tap_start)?.checked_add(SPAN)?);
    }
    if width % 8 >= 4 {
        reach = reach.max(vector8.checked_add(tap_start)?.checked_add(SPAN)?);
    }
    Some(reach)
}

#[allow(
    clippy::inline_always,
    reason = "one call per filtered row on the measured subpel hot path"
)]
#[inline(always)]
pub(super) fn horizontal_8tap_row_u16(
    window: &[u16],
    taps: &[i32; NUM_TAPS],
    tap_start: usize,
    tap_end: usize,
    round0: u32,
    row_out: &mut [i16],
) -> bool {
    let span = tap_end.saturating_sub(tap_start);
    if round0 != HORIZONTAL_ROUND || tap_end > NUM_TAPS || !(2..=NUM_TAPS).contains(&span) {
        return false;
    }
    if row_out.is_empty() {
        return true;
    }
    let Some(reach) = source_reach(row_out.len(), tap_start, tap_end) else {
        return false;
    };
    if window.len() < reach {
        return false;
    }
    let Some(narrow) = narrow_taps(taps) else {
        return false;
    };
    match span {
        2 => horizontal_row::<2>(window, &narrow, tap_start, row_out),
        3 => horizontal_row::<3>(window, &narrow, tap_start, row_out),
        4 => horizontal_row::<4>(window, &narrow, tap_start, row_out),
        5 => horizontal_row::<5>(window, &narrow, tap_start, row_out),
        6 => horizontal_row::<6>(window, &narrow, tap_start, row_out),
        7 => horizontal_row::<7>(window, &narrow, tap_start, row_out),
        _ => horizontal_row::<8>(window, &narrow, tap_start, row_out),
    }
    true
}

/// Narrows a `Subpel_Filters` row to the 16-bit lanes the widening
/// multiply-accumulate takes, refusing any row that does not fit.
#[allow(
    clippy::inline_always,
    reason = "one call per filtered row on the measured subpel hot path"
)]
#[inline(always)]
fn narrow_taps(taps: &[i32; NUM_TAPS]) -> Option<[i16; NUM_TAPS]> {
    let mut narrow = [0i16; NUM_TAPS];
    for (slot, &tap) in narrow.iter_mut().zip(taps) {
        *slot = i16::try_from(tap).ok()?;
    }
    Some(narrow)
}

/// Broadcasts the active taps into vector lanes, leaving unused slots zero.
#[allow(
    clippy::inline_always,
    reason = "one call per filtered row on the measured subpel hot path"
)]
#[inline(always)]
fn splat_taps<const K: usize>(taps: &[i16; NUM_TAPS], tap_start: usize) -> [int16x8_t; NUM_TAPS] {
    // SAFETY: NEON, per the module gate; no memory is touched.
    unsafe {
        let mut splat = [vdupq_n_s16(0); NUM_TAPS];
        for (lane, slot) in splat.iter_mut().enumerate().take(K) {
            *slot = vdupq_n_s16(taps[tap_start + lane]);
        }
        splat
    }
}

/// Accumulates `K` taps of one eight-lane group from the group's two loads.
///
/// Every `ext` lane offset is a literal because `tap_start` moved into the load
/// address, which is the whole point of the hand-scheduled path.
#[allow(
    clippy::inline_always,
    reason = "one call per filtered row on the measured subpel hot path"
)]
#[inline(always)]
fn accumulate<const K: usize>(
    lo: int16x8_t,
    hi: int16x8_t,
    splat: &[int16x8_t; NUM_TAPS],
) -> (int32x4_t, int32x4_t) {
    // SAFETY: NEON, per the module gate; no memory is touched.
    unsafe {
        let mut low = vdupq_n_s32(0);
        let mut high = vdupq_n_s32(0);
        macro_rules! tap {
            ($index:literal, $window:expr) => {
                if K > $index {
                    let window = $window;
                    low = vmlal_s16(low, vget_low_s16(window), vget_low_s16(splat[$index]));
                    high = vmlal_high_s16(high, window, splat[$index]);
                }
            };
        }
        tap!(0, lo);
        tap!(1, vextq_s16::<1>(lo, hi));
        tap!(2, vextq_s16::<2>(lo, hi));
        tap!(3, vextq_s16::<3>(lo, hi));
        tap!(4, vextq_s16::<4>(lo, hi));
        tap!(5, vextq_s16::<5>(lo, hi));
        tap!(6, vextq_s16::<6>(lo, hi));
        tap!(7, vextq_s16::<7>(lo, hi));
        (low, high)
    }
}

#[allow(
    clippy::inline_always,
    reason = "one call per filtered row on the measured subpel hot path"
)]
#[inline(always)]
fn horizontal_row<const K: usize>(
    window: &[u16],
    taps: &[i16; NUM_TAPS],
    tap_start: usize,
    row_out: &mut [i16],
) {
    let width = row_out.len();
    let source = window.as_ptr();
    let destination = row_out.as_mut_ptr();
    let splat = splat_taps::<K>(taps, tap_start);

    let mut column = 0;
    let vector8 = width - width % 8;
    while column < vector8 {
        // SAFETY: NEON, per the module gate; `source_reach` bounds this group's
        // loads by `column + tap_start + SPAN <= window.len()` and its store by
        // `column + 8 <= vector8 <= row_out.len()`.
        unsafe {
            let base = source.add(column + tap_start);
            let lo = vreinterpretq_s16_u16(vld1q_u16(base));
            let hi = vreinterpretq_s16_u16(vld1q_u16(base.add(8)));
            let (low, high) = accumulate::<K>(lo, hi, &splat);
            vst1q_s16(
                destination.add(column),
                vcombine_s16(
                    vmovn_s32(vrshrq_n_s32::<{ HORIZONTAL_ROUND as i32 }>(low)),
                    vmovn_s32(vrshrq_n_s32::<{ HORIZONTAL_ROUND as i32 }>(high)),
                ),
            );
        }
        column += 8;
    }

    if width % 8 >= 4 {
        // SAFETY: as above, with `column + 4 <= row_out.len()`.
        unsafe {
            let base = source.add(column + tap_start);
            let lo = vreinterpretq_s16_u16(vld1q_u16(base));
            let hi = vreinterpretq_s16_u16(vld1q_u16(base.add(8)));
            let (low, _) = accumulate::<K>(lo, hi, &splat);
            vst1_s16(
                destination.add(column),
                vmovn_s32(vrshrq_n_s32::<{ HORIZONTAL_ROUND as i32 }>(low)),
            );
        }
        column += 4;
    }

    for column in column..width {
        let mut sum = 0i32;
        for tap in 0..K {
            sum += i32::from(taps[tap_start + tap]) * i32::from(window[column + tap_start + tap]);
        }
        row_out[column] = round2(sum, HORIZONTAL_ROUND) as i16;
    }
}
