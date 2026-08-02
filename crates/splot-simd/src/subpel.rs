// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Hand-scheduled AV2 § 7.13.3.18 separable 8-tap sub-pel convolution passes.
//!
//! The two-axis sub-pel predictor runs a horizontal pass that filters
//! `intermediateHeight` reference rows into a 16-bit intermediate, then a
//! vertical pass that filters eight intermediate rows into one output row.
//! Both passes are exposed here as one-row kernels over already-materialised
//! contiguous storage, so this crate never needs the caller's plane, block or
//! reference model.
//!
//! # The shape the hand-scheduled path buys
//!
//! A horizontal row applies `K = tap_end - tap_start` taps at `K` consecutive
//! offsets of one contiguous source span. Loading the span once into two
//! vectors and sliding it by lane leaves every window's values untouched, so
//! the convolution is unchanged and only the load shape moves: `K` overlapping
//! unaligned loads become two loads plus `K - 1` `ext`. The lane offsets are
//! `0..K`, always literals, because the § 7.13.3.18 active tap span's runtime
//! `tap_start` is folded into the load address instead of the lane index. That
//! last step is what the portable path cannot do: `simd_swizzle!` needs
//! constant indices, so a portable implementation can only take the shape when
//! `tap_start` is zero.

/// The number of taps in one AV2 § 7.13.3.18 `Subpel_Filters` row.
pub const NUM_TAPS: usize = 8;

/// AV2 § 4.7 `Round2`.
///
/// `n` is at least one at every call site; `Round2(x, 0)` is the identity and
/// the callers that need it never reach a filter pass.
#[inline]
fn round2(value: i32, n: u32) -> i32 {
    (value + (1 << (n - 1))) >> n
}

/// Runs the AV2 § 7.13.3.18 horizontal filter pass for one reference row.
///
/// `window` holds the row's source samples with `window[c + t]` the sample for
/// output column `c` and tap `t`; `taps` is the whole `Subpel_Filters` row and
/// `tap_start..tap_end` its active span (the leading and trailing zero taps the
/// § 7.13.3.18 filter rows carry). Each output is
/// `Round2(sum, round0)` truncated to `i16`, which is exact: § 6 Table 6.3 caps
/// `BitDepth` at 10, and across the § 7.13.3.18 filter rows the pass spans
/// `-7161..=23529`.
///
/// This is the portable reference. It is the implementation on targets with no
/// hand-scheduled kernel and the differential-test oracle everywhere.
///
/// # Panics
///
/// Panics if `window` is shorter than `row_out.len() + tap_end - 1`, or if
/// `tap_end` exceeds [`NUM_TAPS`].
pub fn horizontal_8tap_row_u16_reference(
    window: &[u16],
    taps: &[i32; NUM_TAPS],
    tap_start: usize,
    tap_end: usize,
    round0: u32,
    row_out: &mut [i16],
) {
    for (column, out) in row_out.iter_mut().enumerate() {
        let mut sum = 0i32;
        for tap in tap_start..tap_end {
            sum += taps[tap] * i32::from(window[column + tap]);
        }
        *out = round2(sum, round0) as i16;
    }
}

/// Runs the AV2 § 7.13.3.18 horizontal filter pass through the hand-scheduled
/// kernel when this build has one for the shape.
///
/// Returns `false` without touching `row_out` when it does not, so the caller
/// keeps its own portable path. The arguments and the produced values are
/// exactly [`horizontal_8tap_row_u16_reference`]'s.
#[allow(
    clippy::inline_always,
    reason = "one call per filtered row on the measured subpel hot path"
)]
#[inline(always)]
pub fn horizontal_8tap_row_u16(
    window: &[u16],
    taps: &[i32; NUM_TAPS],
    tap_start: usize,
    tap_end: usize,
    round0: u32,
    row_out: &mut [i16],
) -> bool {
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    {
        neon::horizontal_8tap_row_u16(window, taps, tap_start, tap_end, round0, row_out)
    }
    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    {
        let _ = (window, taps, tap_start, tap_end, round0, row_out);
        false
    }
}

#[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
mod neon;

#[cfg(test)]
mod tests;
