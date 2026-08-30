// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.18 block inter prediction (sub-pel motion compensation) kernel.
//!
//! Implements source-backed separable interpolation with § 7.13.3.16 rounding
//! and § 4.8 clipping. Callers provide § 7.13.3.17 scaling and clipping bounds.
//! The § 9 `Subpel_Filters` table is transcribed from the AV2 specification and
//! must never be regenerated from AVM.
//! Feature tracking: `RECON-SUBPEL-MC`.

use crate::error::{ReconError, Result};
use crate::format::{BitDepth, ReconSample};
use crate::math::round2_i32;
use std::simd::{Simd, cmp::SimdOrd, num::SimdInt, num::SimdUint};
macro_rules! finish_fused_compound_2d {
    ($lanes:ident, $params:expr, $cwp_weight:expr, $intermediate:expr, $output:expr, $stride:expr) => {{
        let vertical = $params.map(|params| {
            let filter = params.interp.pass_index(params.h as u32) as usize;
            let phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
            let (start, end) = ACTIVE_TAP_SPANS[filter][phase];
            (&SUBPEL_FILTERS[filter][phase][start..end], start)
        });
        let forward = i32::from($cwp_weight);
        let backward = 16 - forward;
        for row in 0..$params[0].h {
            let mut predictors = [Simd::<i32, $lanes>::splat(0); 2];
            for reference in 0..2 {
                let (taps, tap_start) = vertical[reference];
                for (offset, &tap) in taps.iter().enumerate() {
                    let start = (row + tap_start + offset) * $lanes;
                    predictors[reference] = tap_mac(
                        predictors[reference],
                        Simd::from_slice(&$intermediate[reference][start..]),
                        tap,
                    );
                }
                predictors[reference] =
                    round2_simd(predictors[reference], INTER_ROUND1_COMPOUND);
            }
            let blended = round2_simd(
                predictors[0] * Simd::splat(forward)
                    + predictors[1] * Simd::splat(backward),
                4 + compound_inter_post_round(),
            )
            .simd_clamp(
                Simd::splat(0),
                Simd::splat(i32::from($params[0].bit_depth.max_sample())),
            )
            .cast::<u16>();
            $output[row * $stride..][..$lanes].copy_from_slice(&blended.to_array()); // splot-copy-ok: publish fused two-axis compound lanes
        }
    }};
}
mod clipped_compound;
mod clipped_edges;
mod copy;
mod fullpel_u8;
mod output;
mod slide;
mod tip_overlap;
pub use copy::{
    blend_compound_average_equal, blend_compound_average_weighted,
    blend_compound_average_weighted_sample,
};
use copy::{
    compound_inter_post_round, subpel_copy_block_into, subpel_copy_block_u16_into,
    subpel_copy_compound_average_u16_into, subpel_direct_copy_x, subpel_horizontal_only_into,
    subpel_horizontal_window_x, subpel_vertical_only_into,
};
pub use fullpel_u8::{
    subpel_predict_block_compound_average_fullpel_strided_into_u8,
    subpel_predict_block_strided_into_u8,
};
use output::*;
use slide::SlideLanes;
pub use tip_overlap::subpel_predict_16x16_bilinear_horizontal_overlap_into;
/// AV2 § 3 `SCALE_SUBPEL_BITS`: number of fractional bits in the 1/1024-sample
/// reference coordinates (`startX` / `startY` / `stepX` / `stepY` units).
const SCALE_SUBPEL_BITS: u32 = 10;
const MIN_SCALE_STEP: i32 = 1 << (SCALE_SUBPEL_BITS - 4);
const MAX_SCALE_STEP: i32 = 1 << (SCALE_SUBPEL_BITS + 1);

/// AV2 § 3 `SUBPEL_BITS`: number of fractional bits when choosing a filter tap.
const SUBPEL_BITS: u32 = 4;

/// AV2 § 3 `SUBPEL_MASK = (1 << SUBPEL_BITS) - 1`: the 16-phase sub-pel mask used
/// to index the inner `Subpel_Filters[...][phase]` dimension.
const SUBPEL_MASK: i32 = (1 << SUBPEL_BITS) - 1;

/// AV2 § 7.13.3.16 `InterRound0`: the down-shift after the horizontal filter
/// pass. Fixed at 3.
const INTER_ROUND0: u32 = 3;

/// AV2 § 7.13.3.18 `FILTER_BITS`: every `Subpel_Filters` row sums to
/// `1 << FILTER_BITS`.
const FILTER_BITS: u32 = 7;

/// AV2 § 7.13.3.16 `InterRound1`: the down-shift after the vertical filter pass
/// for the non-compound (`isCompound == 0`) prediction this kernel produces.
const INTER_ROUND1_NON_COMPOUND: u32 = 11;

/// AV2 § 7.13.3.16 `InterRound1`: the down-shift after the vertical filter pass
/// for compound (`isCompound == 1`) predictors, before § 7.13.3.16 blending.
const INTER_ROUND1_COMPOUND: u32 = 7;

/// AV2 § 6 `EIGHTTAP` interpolation filter index.
const EIGHTTAP: u8 = 0;
/// AV2 § 6 `EIGHTTAP_SMOOTH` interpolation filter index.
const EIGHTTAP_SMOOTH: u8 = 1;
/// AV2 § 6 `EIGHTTAP_SHARP` interpolation filter index.
const EIGHTTAP_SHARP: u8 = 2;
/// AV2 § 6 `BILINEAR` interpolation filter index. The 2-tap bilinear filter has no
/// 4-tap small-block substitution (it is already short), so it keeps index 3 for
/// every block size.
const BILINEAR: u8 = 3;

/// `Subpel_Filters` index for the 4-tap version of the `EIGHTTAP` filter
/// (selected for small blocks, AV2 § 7.13.3.18).
const SMALL_BLOCK_EIGHTTAP: u8 = 4;
/// `Subpel_Filters` index for the 4-tap version of the `EIGHTTAP_SMOOTH` filter
/// (selected for small blocks, AV2 § 7.13.3.18).
const SMALL_BLOCK_EIGHTTAP_SMOOTH: u8 = 5;

/// The number of filter taps in each `Subpel_Filters` row.
const NUM_TAPS: usize = 8;

/// Extra readable samples one reference row needs past its § 7.13.3.18 window
/// before the sliding window shape may replace the eight overlapping tap loads
/// with two loads and seven lane slides. Nine covers the widest window set
/// (sixteen lanes); rows that cannot spare them keep the overlapping loads.
const SLIDE_RESERVE: usize = 9;

/// The number of sub-pel phases (the 16 rows of each filter type).
const NUM_PHASES: usize = 16;

/// The number of `Subpel_Filters` filter types.
const NUM_FILTER_TYPES: usize = 6;

/// The block-size threshold (in samples) at or below which the 4-tap small-block
/// filter substitution applies (AV2 § 7.13.3.18: `w <= 4` / `h <= 4`).
const SMALL_BLOCK_DIM: u32 = 4;

/// AV2 § 7.13.3.18 `Subpel_Filters[6][16][8]` interpolation-filter coefficients,
/// transcribed verbatim from
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-18`.
///
/// Index `[filter][phase][tap]`: `filter` is the § 6 `interp_filter`
/// (`EIGHTTAP` / `EIGHTTAP_SMOOTH` / `EIGHTTAP_SHARP`, with indices 4 and 5 the
/// 4-tap small-block versions of `EIGHTTAP` and `EIGHTTAP_SMOOTH`), `phase` is
/// the 16-way sub-pel position `(p >> 6) & SUBPEL_MASK`, and `tap` is the 8-tap
/// FIR coefficient. All coefficients are even and each row sums to 128
/// (`1 << FILTER_BITS`).
pub const SUBPEL_FILTERS: [[[i32; NUM_TAPS]; NUM_PHASES]; NUM_FILTER_TYPES] = [
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [0, 2, -6, 126, 8, -2, 0, 0],
        [0, 2, -10, 122, 18, -4, 0, 0],
        [0, 2, -12, 116, 28, -8, 2, 0],
        [0, 2, -14, 110, 38, -10, 2, 0],
        [0, 2, -14, 102, 48, -12, 2, 0],
        [0, 2, -16, 94, 58, -12, 2, 0],
        [0, 2, -14, 84, 66, -12, 2, 0],
        [0, 2, -14, 76, 76, -14, 2, 0],
        [0, 2, -12, 66, 84, -14, 2, 0],
        [0, 2, -12, 58, 94, -16, 2, 0],
        [0, 2, -12, 48, 102, -14, 2, 0],
        [0, 2, -10, 38, 110, -14, 2, 0],
        [0, 2, -8, 28, 116, -12, 2, 0],
        [0, 0, -4, 18, 122, -10, 2, 0],
        [0, 0, -2, 8, 126, -6, 2, 0],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [0, 2, 28, 62, 34, 2, 0, 0],
        [0, 0, 26, 62, 36, 4, 0, 0],
        [0, 0, 22, 62, 40, 4, 0, 0],
        [0, 0, 20, 60, 42, 6, 0, 0],
        [0, 0, 18, 58, 44, 8, 0, 0],
        [0, 0, 16, 56, 46, 10, 0, 0],
        [0, -2, 16, 54, 48, 12, 0, 0],
        [0, -2, 14, 52, 52, 14, -2, 0],
        [0, 0, 12, 48, 54, 16, -2, 0],
        [0, 0, 10, 46, 56, 16, 0, 0],
        [0, 0, 8, 44, 58, 18, 0, 0],
        [0, 0, 6, 42, 60, 20, 0, 0],
        [0, 0, 4, 40, 62, 22, 0, 0],
        [0, 0, 4, 36, 62, 26, 0, 0],
        [0, 0, 2, 34, 62, 28, 2, 0],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [-2, 2, -6, 126, 8, -2, 2, 0],
        [-2, 6, -12, 124, 16, -6, 4, -2],
        [-2, 8, -18, 120, 26, -10, 6, -2],
        [-4, 10, -22, 116, 38, -14, 6, -2],
        [-4, 10, -22, 108, 48, -18, 8, -2],
        [-4, 10, -24, 100, 60, -20, 8, -2],
        [-4, 10, -24, 90, 70, -22, 10, -2],
        [-4, 12, -24, 80, 80, -24, 12, -4],
        [-2, 10, -22, 70, 90, -24, 10, -4],
        [-2, 8, -20, 60, 100, -24, 10, -4],
        [-2, 8, -18, 48, 108, -22, 10, -4],
        [-2, 6, -14, 38, 116, -22, 10, -4],
        [-2, 6, -10, 26, 120, -18, 8, -2],
        [-2, 4, -6, 16, 124, -12, 6, -2],
        [0, 2, -2, 8, 126, -6, 2, -2],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [0, 0, 0, 120, 8, 0, 0, 0],
        [0, 0, 0, 112, 16, 0, 0, 0],
        [0, 0, 0, 104, 24, 0, 0, 0],
        [0, 0, 0, 96, 32, 0, 0, 0],
        [0, 0, 0, 88, 40, 0, 0, 0],
        [0, 0, 0, 80, 48, 0, 0, 0],
        [0, 0, 0, 72, 56, 0, 0, 0],
        [0, 0, 0, 64, 64, 0, 0, 0],
        [0, 0, 0, 56, 72, 0, 0, 0],
        [0, 0, 0, 48, 80, 0, 0, 0],
        [0, 0, 0, 40, 88, 0, 0, 0],
        [0, 0, 0, 32, 96, 0, 0, 0],
        [0, 0, 0, 24, 104, 0, 0, 0],
        [0, 0, 0, 16, 112, 0, 0, 0],
        [0, 0, 0, 8, 120, 0, 0, 0],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [0, 0, -4, 126, 8, -2, 0, 0],
        [0, 0, -8, 122, 18, -4, 0, 0],
        [0, 0, -10, 116, 28, -6, 0, 0],
        [0, 0, -12, 110, 38, -8, 0, 0],
        [0, 0, -12, 102, 48, -10, 0, 0],
        [0, 0, -14, 94, 58, -10, 0, 0],
        [0, 0, -12, 84, 66, -10, 0, 0],
        [0, 0, -12, 76, 76, -12, 0, 0],
        [0, 0, -10, 66, 84, -12, 0, 0],
        [0, 0, -10, 58, 94, -14, 0, 0],
        [0, 0, -10, 48, 102, -12, 0, 0],
        [0, 0, -8, 38, 110, -12, 0, 0],
        [0, 0, -6, 28, 116, -10, 0, 0],
        [0, 0, -4, 18, 122, -8, 0, 0],
        [0, 0, -2, 8, 126, -4, 0, 0],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [0, 0, 30, 62, 34, 2, 0, 0],
        [0, 0, 26, 62, 36, 4, 0, 0],
        [0, 0, 22, 62, 40, 4, 0, 0],
        [0, 0, 20, 60, 42, 6, 0, 0],
        [0, 0, 18, 58, 44, 8, 0, 0],
        [0, 0, 16, 56, 46, 10, 0, 0],
        [0, 0, 14, 54, 48, 12, 0, 0],
        [0, 0, 12, 52, 52, 12, 0, 0],
        [0, 0, 12, 48, 54, 14, 0, 0],
        [0, 0, 10, 46, 56, 16, 0, 0],
        [0, 0, 8, 44, 58, 18, 0, 0],
        [0, 0, 6, 42, 60, 20, 0, 0],
        [0, 0, 4, 40, 62, 22, 0, 0],
        [0, 0, 4, 36, 62, 26, 0, 0],
        [0, 0, 2, 34, 62, 30, 0, 0],
    ],
];

const fn active_tap_spans() -> [[(usize, usize); NUM_PHASES]; NUM_FILTER_TYPES] {
    let mut spans = [[(0, 0); NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 0;
        while phase < NUM_PHASES {
            let taps = &SUBPEL_FILTERS[filter][phase];
            let mut start = 0;
            while start < NUM_TAPS && taps[start] == 0 {
                start += 1;
            }
            let mut end = NUM_TAPS;
            while end > start && taps[end - 1] == 0 {
                end -= 1;
            }
            spans[filter][phase] = (start, end);
            phase += 1;
        }
        filter += 1;
    }
    spans
}

const ACTIVE_TAP_SPANS: [[(usize, usize); NUM_PHASES]; NUM_FILTER_TYPES] = active_tap_spans();

/// AV2 § 6 interpolation filter for the inter prediction (`interp_filter`).
///
/// Only the three frame-level filter types are exposed; the 4-tap small-block
/// substitutions (indices 4 and 5) are an internal § 7.13.3.18 step, never a
/// caller-selectable filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterpolationFilter {
    /// `EIGHTTAP` (`interp_filter == 0`).
    EightTap,
    /// `EIGHTTAP_SMOOTH` (`interp_filter == 1`).
    EightTapSmooth,
    /// `EIGHTTAP_SHARP` (`interp_filter == 2`).
    EightTapSharp,
    /// `BILINEAR` (`interp_filter == 3`). The 2-tap bilinear filter has no 4-tap
    /// small-block substitution — it keeps index 3 for every block size.
    Bilinear,
}

impl InterpolationFilter {
    /// The base `Subpel_Filters` index for this filter (`EIGHTTAP == 0`,
    /// `EIGHTTAP_SMOOTH == 1`, `EIGHTTAP_SHARP == 2`, `BILINEAR == 3`).
    const fn base_index(self) -> u8 {
        match self {
            Self::EightTap => EIGHTTAP,
            Self::EightTapSmooth => EIGHTTAP_SMOOTH,
            Self::EightTapSharp => EIGHTTAP_SHARP,
            Self::Bilinear => BILINEAR,
        }
    }

    /// Applies the AV2 § 7.13.3.18 small-block (`dim <= 4`) substitution: an
    /// `EIGHTTAP` / `EIGHTTAP_SHARP` filter maps to the 4-tap index 4,
    /// `EIGHTTAP_SMOOTH` maps to the 4-tap index 5, and `BILINEAR` (already a
    /// 2-tap filter) keeps index 3.
    const fn pass_index(self, dim: u32) -> u8 {
        if dim <= SMALL_BLOCK_DIM {
            match self {
                Self::EightTap | Self::EightTapSharp => SMALL_BLOCK_EIGHTTAP,
                Self::EightTapSmooth => SMALL_BLOCK_EIGHTTAP_SMOOTH,
                Self::Bilinear => BILINEAR,
            }
        } else {
            self.base_index()
        }
    }
}

/// A reference-plane sample view for the AV2 § 7.13.3.18 convolution.
///
/// The plane is a row-major sample buffer of `height` rows spaced `stride`
/// samples apart; the kernel reads
/// `ref[Clip3(firstY, lastY, refY)][Clip3(firstX, lastX, refX)]`, so the
/// clipping bounds (a § 7.13.3.18 input) implement the reference-border
/// extension without the caller copying a padded plane.
#[derive(Clone, Copy, Debug)]
pub struct ReferencePlaneView<'a, T: ReconSample = u16> {
    samples: &'a [T],
    stride: usize,
    width: usize,
    height: usize,
    readable_rows: usize,
}

impl<'a, T: ReconSample> ReferencePlaneView<'a, T> {
    /// Builds a reference-plane view over a contiguous row-major
    /// `width * height` sample buffer.
    ///
    /// # Errors
    ///
    /// Returns [`ReconError::SubpelReferencePlaneMismatch`] when `samples.len()`
    /// is not exactly `width * height`, or [`ReconError::ZeroDimension`] when a
    /// dimension is zero.
    pub fn new(samples: &'a [T], width: usize, height: usize) -> Result<Self> {
        let expected = width
            .checked_mul(height)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "subpel reference plane size",
            })?;
        if samples.len() != expected {
            return Err(ReconError::SubpelReferencePlaneMismatch {
                expected,
                actual: samples.len(),
            });
        }
        Self::from_strided(samples, width, width, height)
    }

    /// Builds a reference-plane view over `height` rows of `width` samples
    /// spaced `stride` samples apart, borrowing the caller's plane storage
    /// directly.
    ///
    /// # Errors
    ///
    /// Returns [`ReconError::ZeroDimension`] when a dimension is zero, or
    /// [`ReconError::SubpelReferencePlaneMismatch`] when `stride < width` or
    /// `samples` cannot cover the final row.
    pub fn from_strided(
        samples: &'a [T],
        stride: usize,
        width: usize,
        height: usize,
    ) -> Result<Self> {
        Self::from_strided_rows(samples, stride, width, height, height)
    }

    /// Builds a full-geometry view backed by an already-published row prefix.
    ///
    /// # Errors
    ///
    /// Returns [`ReconError::ZeroDimension`] when a dimension is zero, or
    /// [`ReconError::SubpelReferencePlaneMismatch`] when the geometry or
    /// published prefix cannot be represented by `samples`.
    pub fn from_published_strided(
        samples: &'a [T],
        stride: usize,
        width: usize,
        height: usize,
        readable_rows: usize,
    ) -> Result<Self> {
        Self::from_strided_rows(samples, stride, width, height, readable_rows)
    }

    fn from_strided_rows(
        samples: &'a [T],
        stride: usize,
        width: usize,
        height: usize,
        readable_rows: usize,
    ) -> Result<Self> {
        if width == 0 {
            return Err(ReconError::ZeroDimension {
                field: "subpel reference plane width",
            });
        }
        if height == 0 {
            return Err(ReconError::ZeroDimension {
                field: "subpel reference plane height",
            });
        }
        if readable_rows == 0 || readable_rows > height {
            return Err(ReconError::SubpelReferencePlaneMismatch {
                expected: width,
                actual: 0,
            });
        }
        let required = readable_rows
            .checked_sub(1)
            .and_then(|rows| rows.checked_mul(stride))
            .and_then(|prefix| prefix.checked_add(width))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "subpel reference plane size",
            })?;
        if stride < width || samples.len() < required {
            return Err(ReconError::SubpelReferencePlaneMismatch {
                expected: required,
                actual: samples.len(),
            });
        }
        Ok(Self {
            samples,
            stride,
            width,
            height,
            readable_rows,
        })
    }

    /// Returns the view width in samples.
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Returns the view height in samples.
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Reads `ref[plane][row][col]`. The convolution clips the requested indices
    /// to the caller-supplied `[firstX, lastX] x [firstY, lastY]` region; this
    /// read additionally clamps to the view's own width and readable row prefix
    /// so it is total even if a caller passes a clipping region wider than the
    /// available samples (defense in depth — it never indexes out of bounds).
    pub fn sample(&self, row: usize, col: usize) -> i32 {
        let row = row.min(self.readable_rows - 1);
        let col = col.min(self.width - 1);
        i32::from(self.samples[row * self.stride + col].to_u16())
    }

    pub(crate) fn row(&self, row: usize) -> &[T] {
        let row = row.min(self.readable_rows - 1);
        let start = row * self.stride;
        &self.samples[start..start + self.width]
    }
}

/// AV2 § 7.13.3.18 block inter prediction parameters for the single-reference
/// (non-compound) sub-pel convolution.
#[derive(Clone, Copy, Debug)]
pub struct SubpelPredictParams {
    /// The § 6 interpolation filter (`interp`).
    pub interp: InterpolationFilter,
    /// The block width in samples (`w`), `1..=128`.
    pub w: usize,
    /// The block height in samples (`h`), `1..=128`.
    pub h: usize,
    /// The § 7.13.3.17 `startX`: the reference block left edge in 1/1024-sample
    /// units (the `x` input to § 7.13.3.18).
    pub start_x: i32,
    /// The § 7.13.3.17 `startY`: the reference block top edge in 1/1024-sample
    /// units (the `y` input to § 7.13.3.18).
    pub start_y: i32,
    /// The § 7.13.3.17 `stepX`: the horizontal step in 1/1024-sample units
    /// (the `xStep` input to § 7.13.3.18).
    pub step_x: i32,
    /// The § 7.13.3.17 `stepY`: the vertical step in 1/1024-sample units
    /// (the `yStep` input to § 7.13.3.18).
    pub step_y: i32,
    /// The § 7.13.3.18 reference-clipping region left bound (`firstX`).
    pub first_x: i32,
    /// The § 7.13.3.18 reference-clipping region top bound (`firstY`).
    pub first_y: i32,
    /// The § 7.13.3.18 reference-clipping region right bound (`lastX`).
    pub last_x: i32,
    /// The § 7.13.3.18 reference-clipping region bottom bound (`lastY`).
    pub last_y: i32,
    /// The active bit depth, used by the final § 4.8 `Clip1`.
    pub bit_depth: BitDepth,
}

/// The maximum supported block dimension (AV2 super-block transform block side).
const MAX_BLOCK_DIM: usize = 128;

/// Runs the AV2 § 7.13.3.18 separable interpolation-filter convolution for a
/// single-reference (non-compound) inter block and returns the row-major
/// `w * h` predicted samples after the final § 4.8 `Clip1` (the value the
/// § 7.13.3 write stores into `CurrFrame`).
///
/// The horizontal pass builds an `intermediateHeight * w` array with
/// `Round2(s, InterRound0)`; the vertical pass produces the `h * w` output with
/// `Clip1(Round2(s, InterRound1))`. Filter taps are selected from
/// [`SUBPEL_FILTERS`] by the § 6 `interp` (with the § 7.13.3.18 small-block
/// 4-tap substitution applied per pass) and the sub-pel phase
/// `(p >> 6) & SUBPEL_MASK`. Reference reads are clipped to
/// `[firstX, lastX] x [firstY, lastY]` (the reference-border extension).
///
/// # Errors
///
/// Returns [`ReconError::ZeroDimension`] for a zero dimension,
/// [`ReconError::SubpelBlockDimensionUnsupported`] for `w`/`h` above the
/// 128-sample super-block side, [`ReconError::SubpelNegativeStep`] for a negative
/// step, and [`ReconError::ArithmeticOverflow`] if the intermediate height cannot
/// be derived.
pub fn subpel_predict_block<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Result<Vec<u16>> {
    let max_sample = i32::from(params.bit_depth.max_sample());
    subpel_predict_block_internal(reference, params, INTER_ROUND1_NON_COMPOUND, |pred| {
        pred.clamp(0, max_sample) as u16
    })
}

/// Writes one AV2 § 7.13.3.18 single-reference prediction into caller-owned
/// contiguous `u16` storage after the final § 4.8 `Clip1`.
///
/// The function writes the first `params.w * params.h` samples and leaves any
/// trailing storage unchanged.
///
/// # Errors
///
/// Returns the same errors as [`subpel_predict_block`] and
/// [`ReconError::BufferLengthMismatch`] when `output` is too short.
pub fn subpel_predict_block_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> Result<()> {
    subpel_predict_block_strided_into(reference, params, output, params.w)
}

/// Writes one single-reference prediction into row-strided `u16` storage.
///
/// `output` starts at the prediction's top-left sample. The function writes
/// `params.w` samples in each of `params.h` rows and leaves row padding and
/// trailing storage unchanged.
///
/// # Errors
///
/// Returns the same errors as [`subpel_predict_block_into`] and
/// [`ReconError::StrideTooSmall`] when `output_stride < params.w`.
pub fn subpel_predict_block_strided_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
    output_stride: usize,
) -> Result<()> {
    let intermediate_height = validate_subpel_params(params)?;
    if params.step_x == 1 << SCALE_SUBPEL_BITS
        && params.step_y == 1 << SCALE_SUBPEL_BITS
        && (params.start_x >> 6) & SUBPEL_MASK == 0
        && (params.start_y >> 6) & SUBPEL_MASK == 0
    {
        return subpel_copy_block_u16_into(reference, params, output, output_stride);
    }
    if params.interp == InterpolationFilter::Bilinear
        && params.step_x == 1 << SCALE_SUBPEL_BITS
        && params.step_y == 1 << SCALE_SUBPEL_BITS
    {
        let h_phase = (params.start_x >> 6) & SUBPEL_MASK;
        let v_phase = (params.start_y >> 6) & SUBPEL_MASK;
        match (h_phase == 0, v_phase == 0) {
            (false, true) => {
                return subpel_bilinear_horizontal_into(reference, params, output, output_stride);
            }
            (true, false) => {
                return subpel_bilinear_vertical_into(reference, params, output, output_stride);
            }
            (false, false) => {
                return subpel_bilinear_2d_into(reference, params, output, output_stride);
            }
            (true, true) => {}
        }
    }
    subpel_predict_block_internal_into_validated(
        reference,
        params,
        INTER_ROUND1_NON_COMPOUND,
        intermediate_height,
        None,
        output,
        output_stride,
        ClippedU16SubpelOutput {
            max_sample: i32::from(params.bit_depth.max_sample()),
        },
    )
}

fn subpel_output_len(params: &SubpelPredictParams, output_stride: usize) -> Result<usize> {
    if output_stride < params.w {
        return Err(ReconError::StrideTooSmall {
            stride_samples: output_stride,
            storage_width: params.w,
        });
    }
    (params.h - 1)
        .checked_mul(output_stride)
        .and_then(|len| len.checked_add(params.w))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided subpel output sample count",
        })
}

fn bilinear_sample(left: u16, right: u16, phase: i32) -> u16 {
    let left = i32::from(left);
    let right = i32::from(right);
    round2_i32((16 - phase) * left + phase * right, SUBPEL_BITS) as u16
}

fn fixed_16x16_window_in_bounds<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    x0: i32,
    y0: i32,
) -> bool {
    x0 + 1 >= 0
        && x0 + 15 < reference.width as i32
        && y0 + 1 >= 0
        && y0 + 15 < reference.readable_rows as i32
}

/// One AV2 § 7.13.3.18 `BILINEAR` tap pair, entirely in 16-bit lanes.
///
/// `(16 - phase) * left + phase * right` is at most `16 * 1023` at the § 6
/// Table 6.3 maximum `BitDepth`, so the weighted sum and its `Round2(s, 4)`
/// both stay exact in `u16`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn bilinear_u16<const LANES: usize>(
    left: Simd<u16, LANES>,
    right: Simd<u16, LANES>,
    phase: i32,
) -> Simd<u16, LANES> {
    let phase = phase as u16;
    (left * Simd::splat(16 - phase)
        + right * Simd::splat(phase)
        + Simd::splat(1 << (SUBPEL_BITS - 1)))
        >> SUBPEL_BITS as u16
}

/// Reads `LANES` consecutive reference samples as `u16` lanes for either
/// storage width, so the bilinear kernels keep their vector shape on the
/// eight-bit plane the § 7.13.3.18 `u8` entry point serves.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn reference_lanes<const LANES: usize, T: ReconSample>(
    source: &[T],
    start: usize,
) -> Simd<u16, LANES> {
    if let Some(source) = T::u16_slice(source) {
        return Simd::from_slice(&source[start..]);
    }
    if let Some(source) = T::u8_slice(source) {
        return Simd::<u8, LANES>::from_slice(&source[start..]).cast();
    }
    Simd::from_array(core::array::from_fn(|lane| source[start + lane].to_u16()))
}

fn subpel_bilinear_horizontal_into<T: ReconSample, O: BilinearOutput>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [O],
    output_stride: usize,
) -> Result<()> {
    let output_len = subpel_output_len(params, output_stride)?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let phase = (params.start_x >> 6) & SUBPEL_MASK;
    if params.w == 16
        && params.h == 16
        && params.first_x == x0 + 1
        && params.last_x == x0 + 15
        && params.first_y == y0 + 1
        && params.last_y == y0 + 15
        && fixed_16x16_window_in_bounds(reference, x0, y0)
        && let Some(samples) = T::u16_slice(reference.samples)
    {
        let first = (x0 + 1) as usize;
        let second = (x0 + 8) as usize;
        for row in 0..params.h {
            let source_row = (y0 + (row as i32).clamp(1, 15)) as usize;
            let source = &samples[source_row * reference.stride..];
            let destination = &mut output[row * output_stride..][..params.w];
            let low = bilinear_u16(
                Simd::<u16, 8>::from_slice(&source[first..]),
                Simd::<u16, 8>::from_slice(&source[first + 1..]),
                phase,
            );
            O::store(low, &mut destination[1..9]);
            let high = tip_overlap::overlap_bilinear_u16x8(source, source, second, None, phase, 0);
            O::store(high, &mut destination[8..]);
            destination[0] = O::from_sample(source[first]);
        }
        return Ok(());
    }
    let direct_x = usize::try_from(x0).ok().filter(|&x| {
        x >= params.first_x.max(0) as usize
            && x.checked_add(params.w).is_some_and(|last| {
                last < reference.width
                    && i32::try_from(last).is_ok_and(|last| last <= params.last_x)
            })
    });
    let mut clipped_x = [0usize; MAX_BLOCK_DIM + 1];
    if direct_x.is_none() {
        for (c, col) in clipped_x[..=params.w].iter_mut().enumerate() {
            *col = (x0 + c as i32)
                .clamp(params.first_x, params.last_x)
                .clamp(0, reference.width as i32 - 1) as usize;
        }
    }
    for r in 0..params.h {
        let row = (y0 + r as i32)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.readable_rows as i32 - 1) as usize;
        let source = reference.row(row);
        let destination = &mut output[r * output_stride..][..params.w];
        if let Some(x) = direct_x {
            let vector_width8 = params.w - params.w % 8;
            for c in (0..vector_width8).step_by(8) {
                let left = reference_lanes::<8, T>(source, x + c);
                let right = reference_lanes::<8, T>(source, x + c + 1);
                O::store(bilinear_u16(left, right, phase), &mut destination[c..]);
            }
            let vector_width4 = params.w - params.w % 4;
            for c in (vector_width8..vector_width4).step_by(4) {
                let left = reference_lanes::<4, T>(source, x + c);
                let right = reference_lanes::<4, T>(source, x + c + 1);
                O::store(bilinear_u16(left, right, phase), &mut destination[c..]);
            }
            for c in vector_width4..params.w {
                destination[c] = O::from_sample(bilinear_sample(
                    source[x + c].to_u16(),
                    source[x + c + 1].to_u16(),
                    phase,
                ));
            }
        } else {
            for (c, out) in destination.iter_mut().enumerate() {
                *out = O::from_sample(bilinear_sample(
                    source[clipped_x[c]].to_u16(),
                    source[clipped_x[c + 1]].to_u16(),
                    phase,
                ));
            }
        }
    }
    Ok(())
}

fn subpel_bilinear_vertical_into<T: ReconSample, O: BilinearOutput>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [O],
    output_stride: usize,
) -> Result<()> {
    let output_len = subpel_output_len(params, output_stride)?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let phase = (params.start_y >> 6) & SUBPEL_MASK;
    if params.w == 16
        && params.h == 16
        && params.first_x == x0 + 1
        && params.last_x == x0 + 15
        && params.first_y == y0 + 1
        && params.last_y == y0 + 15
        && fixed_16x16_window_in_bounds(reference, x0, y0)
        && let Some(samples) = T::u16_slice(reference.samples)
    {
        let first = (x0 + 1) as usize;
        let second = (x0 + 8) as usize;
        for row in 0..params.h {
            let top = (y0 + (row as i32).clamp(1, 15)) as usize;
            let bottom = (y0 + (row as i32 + 1).clamp(1, 15)) as usize;
            let top = &samples[top * reference.stride..];
            let bottom = &samples[bottom * reference.stride..];
            let destination = &mut output[row * output_stride..][..params.w];
            let low = bilinear_u16(
                Simd::<u16, 8>::from_slice(&top[first..]),
                Simd::<u16, 8>::from_slice(&bottom[first..]),
                phase,
            );
            O::store(low, &mut destination[1..9]);
            let high = bilinear_u16(
                Simd::<u16, 8>::from_slice(&top[second..]),
                Simd::<u16, 8>::from_slice(&bottom[second..]),
                phase,
            );
            O::store(high, &mut destination[8..]);
            destination[0] = O::from_sample(bilinear_sample(top[first], bottom[first], phase));
        }
        return Ok(());
    }
    let direct_x = subpel_direct_copy_x(reference, params);
    for r in 0..params.h {
        let top = (y0 + r as i32)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.readable_rows as i32 - 1) as usize;
        let bottom = (y0 + r as i32 + 1)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.readable_rows as i32 - 1) as usize;
        let top = reference.row(top);
        let bottom = reference.row(bottom);
        let destination = &mut output[r * output_stride..][..params.w];
        if let Some(x) = direct_x {
            let vector_width8 = params.w - params.w % 8;
            for c in (0..vector_width8).step_by(8) {
                let top = reference_lanes::<8, T>(top, x + c);
                let bottom = reference_lanes::<8, T>(bottom, x + c);
                O::store(bilinear_u16(top, bottom, phase), &mut destination[c..]);
            }
            let vector_width4 = params.w - params.w % 4;
            for c in (vector_width8..vector_width4).step_by(4) {
                let top = reference_lanes::<4, T>(top, x + c);
                let bottom = reference_lanes::<4, T>(bottom, x + c);
                O::store(bilinear_u16(top, bottom, phase), &mut destination[c..]);
            }
            for c in vector_width4..params.w {
                destination[c] = O::from_sample(bilinear_sample(
                    top[x + c].to_u16(),
                    bottom[x + c].to_u16(),
                    phase,
                ));
            }
        } else {
            for (c, out) in destination.iter_mut().enumerate() {
                let col = (x0 + c as i32)
                    .clamp(params.first_x, params.last_x)
                    .clamp(0, reference.width as i32 - 1) as usize;
                *out = O::from_sample(bilinear_sample(
                    top[col].to_u16(),
                    bottom[col].to_u16(),
                    phase,
                ));
            }
        }
    }
    Ok(())
}

fn subpel_bilinear_2d_into<T: ReconSample, O: BilinearOutput>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [O],
    output_stride: usize,
) -> Result<()> {
    let output_len = subpel_output_len(params, output_stride)?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }

    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let h_phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
    let v_phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
    let [h0, h1] = [
        SUBPEL_FILTERS[BILINEAR as usize][h_phase][3],
        SUBPEL_FILTERS[BILINEAR as usize][h_phase][4],
    ];
    let [v0, v1] = [
        SUBPEL_FILTERS[BILINEAR as usize][v_phase][3],
        SUBPEL_FILTERS[BILINEAR as usize][v_phase][4],
    ];
    if params.w == 16
        && params.h == 16
        && params.first_x == x0 + 1
        && params.last_x == x0 + 15
        && params.first_y == y0 + 1
        && params.last_y == y0 + 15
        && fixed_16x16_window_in_bounds(reference, x0, y0)
        && let Some(samples) = T::u16_slice(reference.samples)
    {
        let first = (x0 + 1) as usize;
        let second = (x0 + 8) as usize;
        let max_sample = params.bit_depth.max_sample();
        for row in 0..params.h {
            let top = (y0 + (row as i32).clamp(1, 15)) as usize;
            let bottom = (y0 + (row as i32 + 1).clamp(1, 15)) as usize;
            let top = &samples[top * reference.stride..];
            let bottom = &samples[bottom * reference.stride..];
            let destination = &mut output[row * output_stride..][..params.w];
            let low = tip_overlap::overlap_bilinear_u16x8(
                top,
                bottom,
                first,
                Some(first + 1),
                h_phase as i32,
                v_phase as i32,
            )
            .simd_min(Simd::splat(max_sample));
            O::store(low, &mut destination[1..9]);
            let high = tip_overlap::overlap_bilinear_u16x8(
                top,
                bottom,
                second,
                None,
                h_phase as i32,
                v_phase as i32,
            )
            .simd_min(Simd::splat(max_sample));
            O::store(high, &mut destination[8..]);
            destination[0] = O::from_sample(
                bilinear_sample(top[first], bottom[first], v_phase as i32).min(max_sample),
            );
        }
        return Ok(());
    }
    let direct_x = usize::try_from(x0).ok().filter(|&x| {
        x0 >= params.first_x
            && x.checked_add(params.w).is_some_and(|last| {
                last < reference.width
                    && i32::try_from(last).is_ok_and(|last| last <= params.last_x)
            })
    });
    if let (Some(x), Some(samples)) = (direct_x, T::u16_slice(reference.samples)) {
        let max_sample = params.bit_depth.max_sample();
        for r in 0..params.h {
            let top_row = (y0 + r as i32)
                .clamp(params.first_y, params.last_y)
                .clamp(0, reference.readable_rows as i32 - 1) as usize;
            let bottom_row = (y0 + r as i32 + 1)
                .clamp(params.first_y, params.last_y)
                .clamp(0, reference.readable_rows as i32 - 1) as usize;
            let top = &samples[top_row * reference.stride..][..reference.width];
            let bottom = &samples[bottom_row * reference.stride..][..reference.width];
            let destination = &mut output[r * output_stride..][..params.w];
            let vector_width = params.w - params.w % 8;
            for c in (0..vector_width).step_by(8) {
                let filtered = tip_overlap::overlap_bilinear_u16x8(
                    top,
                    bottom,
                    x + c,
                    Some(x + c + 1),
                    h_phase as i32,
                    v_phase as i32,
                )
                .simd_min(Simd::splat(max_sample));
                O::store(filtered, &mut destination[c..]);
            }
            for c in vector_width..params.w {
                let top_value = (16 - h_phase as i32) * i32::from(top[x + c])
                    + h_phase as i32 * i32::from(top[x + c + 1]);
                let bottom_value = (16 - h_phase as i32) * i32::from(bottom[x + c])
                    + h_phase as i32 * i32::from(bottom[x + c + 1]);
                destination[c] = O::from_sample(
                    (round2_i32(
                        (16 - v_phase as i32) * top_value + v_phase as i32 * bottom_value,
                        8,
                    ) as u16)
                        .min(max_sample),
                );
            }
        }
        return Ok(());
    }
    let mut clipped_x = [0usize; MAX_BLOCK_DIM + 1];
    if direct_x.is_none() {
        for (c, col) in clipped_x[..=params.w].iter_mut().enumerate() {
            *col = (x0 + c as i32)
                .clamp(params.first_x, params.last_x)
                .clamp(0, reference.width as i32 - 1) as usize;
        }
    }
    let horizontal_row = |row: i32, destination: &mut [i32]| {
        let row = row
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.readable_rows as i32 - 1) as usize;
        let source = reference.row(row);
        if let Some(x) = direct_x {
            for (out, pair) in destination
                .iter_mut()
                .zip(source[x..=x + params.w].windows(2))
            {
                *out = round2_i32(
                    h0 * i32::from(pair[0].to_u16()) + h1 * i32::from(pair[1].to_u16()),
                    INTER_ROUND0,
                );
            }
        } else {
            for (c, out) in destination.iter_mut().enumerate() {
                *out = round2_i32(
                    h0 * i32::from(source[clipped_x[c]].to_u16())
                        + h1 * i32::from(source[clipped_x[c + 1]].to_u16()),
                    INTER_ROUND0,
                );
            }
        }
    };

    let max_sample = i32::from(params.bit_depth.max_sample());
    let mut storage = [0i32; 2 * MAX_BLOCK_DIM];
    let intermediate = &mut storage[..2 * params.w];
    horizontal_row(y0, &mut intermediate[..params.w]);
    let mut top_is_first = true;
    for r in 0..params.h {
        let (first, second) = intermediate.split_at_mut(params.w);
        let (top, bottom) = if top_is_first {
            (first, second)
        } else {
            (second, first)
        };
        horizontal_row(y0 + r as i32 + 1, bottom);
        for (out, (&top, &bottom)) in output[r * output_stride..][..params.w]
            .iter_mut()
            .zip(top.iter().zip(bottom.iter()))
        {
            *out = O::from_sample(
                round2_i32(v0 * top + v1 * bottom, INTER_ROUND1_NON_COMPOUND).clamp(0, max_sample)
                    as u16,
            );
        }
        top_is_first = !top_is_first;
    }
    Ok(())
}

/// Runs the AV2 § 7.13.3.18 separable interpolation-filter convolution for one
/// reference list of a compound inter block and returns the row-major `w * h`
/// intermediate `Preds[refList]` values after `Round2(s, InterRound1)` but
/// before any § 7.13.3.16 compound averaging, masking, or final `Clip1`.
///
/// The supported source-backed subset currently consumes this for
/// COMPOUND_AVERAGE / CWP_EQUAL blocks. Keeping it unclipped is intentional:
/// § 7.13.3.18 only clips the single-reference write path; compound predictors
/// are clipped after the § 7.13.3.16 blend.
///
/// # Errors
///
/// Returns the same errors as [`subpel_predict_block`]:
/// [`ReconError::ZeroDimension`], [`ReconError::SubpelBlockDimensionUnsupported`],
/// [`ReconError::SubpelNegativeStep`], and [`ReconError::ArithmeticOverflow`].
pub fn subpel_predict_block_compound_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Result<Vec<i32>> {
    subpel_predict_block_internal(reference, params, INTER_ROUND1_COMPOUND, |pred| pred)
}

/// Writes one AV2 § 7.13.3.18 compound intermediate predictor into caller-owned
/// strided storage.
///
/// `output` starts at the prediction's top-left sample. The function writes
/// `params.w` samples in each of `params.h` rows and leaves row padding and any
/// trailing storage unchanged. `scratch` optionally provides the horizontal-pass
/// intermediate storage; when absent or too small the convolution falls back to
/// its internal storage.
///
/// # Errors
///
/// Returns the same errors as [`subpel_predict_block`],
/// [`ReconError::StrideTooSmall`] when `output_stride < params.w`, and
/// [`ReconError::BufferLengthMismatch`] when `output` cannot hold the strided
/// prediction rectangle.
pub fn subpel_predict_block_compound_intermediate_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    scratch: Option<&mut [i16]>,
    output: &mut [i32],
    output_stride: usize,
) -> Result<()> {
    let intermediate_height = validate_subpel_params(params)?;
    subpel_predict_block_internal_into_validated(
        reference,
        params,
        INTER_ROUND1_COMPOUND,
        intermediate_height,
        scratch,
        output,
        output_stride,
        ScalarSubpelOutput(|pred: i32| pred),
    )
}

/// Produces one compound intermediate predictor and blends it directly with
/// the caller-owned first predictor using a uniform § 7.13.3.16 weight.
///
/// Both `pred0` and `output` are contiguous row-major `params.w * params.h`
/// blocks. The second intermediate is passed directly from the convolution to
/// the average and is never materialized.
///
/// # Errors
///
/// Returns the same errors as [`subpel_predict_block`] and
/// [`ReconError::BufferLengthMismatch`] when either block has the wrong length.
pub fn subpel_predict_block_compound_average_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    pred0: &[i32],
    cwp_weight: i16,
    output: &mut [u16],
) -> Result<()> {
    if output.len() != pred0.len() {
        return Err(ReconError::BufferLengthMismatch {
            expected: pred0.len(),
            actual: output.len(),
        });
    }
    subpel_predict_block_compound_average_strided_into(
        reference, params, pred0, cwp_weight, None, output, params.w,
    )
}

/// Produces and blends the second compound predictor into caller-owned strided
/// output. `pred0` remains a contiguous `w * h` intermediate block. `scratch`
/// optionally provides the horizontal-pass intermediate storage; when absent or
/// too small the convolution falls back to its internal storage.
///
/// # Errors
///
/// Returns the same errors as [`subpel_predict_block_compound_average_into`],
/// plus [`ReconError::StrideTooSmall`] when `output_stride < params.w`.
pub fn subpel_predict_block_compound_average_strided_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    pred0: &[i32],
    cwp_weight: i16,
    scratch: Option<&mut [i16]>,
    output: &mut [u16],
    output_stride: usize,
) -> Result<()> {
    let intermediate_height = validate_subpel_params(params)?;
    let sample_count = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "compound prediction sample count",
        })?;
    if pred0.len() != sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len(),
        });
    }
    if subpel_copy_compound_average_u16_into(
        reference,
        params,
        pred0,
        cwp_weight,
        output,
        output_stride,
    )? {
        return Ok(());
    }
    let forward = i32::from(cwp_weight);
    subpel_predict_block_internal_into_validated(
        reference,
        params,
        INTER_ROUND1_COMPOUND,
        intermediate_height,
        scratch,
        output,
        output_stride,
        CompoundAverageSubpelOutput {
            pred0,
            index: 0,
            forward,
            backward: 16 - forward,
            max_sample: i32::from(params.bit_depth.max_sample()),
        },
    )
}

/// Eight-bit-output variant of [`subpel_predict_block_compound_average_strided_into`].
///
/// # Errors
/// Returns the same validation and output-layout errors as the `u16` variant.
pub fn subpel_predict_block_compound_average_strided_into_u8<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    pred0: &[i32],
    cwp_weight: i16,
    scratch: Option<&mut [i16]>,
    output: &mut [u8],
    output_stride: usize,
) -> Result<()> {
    crate::intra_dc_math::validate_sample_type::<u8>(params.bit_depth)?;
    let intermediate_height = validate_subpel_params(params)?;
    let sample_count = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "compound prediction sample count",
        })?;
    if pred0.len() != sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len(),
        });
    }
    let forward = i32::from(cwp_weight);
    subpel_predict_block_internal_into_validated(
        reference,
        params,
        INTER_ROUND1_COMPOUND,
        intermediate_height,
        scratch,
        output,
        output_stride,
        CompoundAverageSubpelOutputU8 {
            pred0,
            index: 0,
            forward,
            backward: 16 - forward,
        },
    )
}

fn subpel_predict_block_compound_average_fullpel_validated<T: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [u16],
    output_stride: usize,
) -> bool {
    let x0 = [
        params0.start_x >> SCALE_SUBPEL_BITS,
        params1.start_x >> SCALE_SUBPEL_BITS,
    ];
    let y0 = [
        params0.start_y >> SCALE_SUBPEL_BITS,
        params1.start_y >> SCALE_SUBPEL_BITS,
    ];
    let direct_x = [
        subpel_direct_copy_x(reference0, params0),
        subpel_direct_copy_x(reference1, params1),
    ];
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    let max_sample = i32::from(params0.bit_depth.max_sample());
    for row in 0..params0.h {
        let source_row = [
            (y0[0] + row as i32).clamp(params0.first_y, params0.last_y) as usize,
            (y0[1] + row as i32).clamp(params1.first_y, params1.last_y) as usize,
        ];
        let destination = &mut output[row * output_stride..][..params0.w];
        if let [Some(x0), Some(x1)] = direct_x {
            let left = &reference0.row(source_row[0])[x0..x0 + params0.w];
            let right = &reference1.row(source_row[1])[x1..x1 + params0.w];
            if let (Some(left), Some(right)) = (T::u16_slice(left), T::u16_slice(right)) {
                blend_fullpel_u16_row(
                    left,
                    right,
                    cwp_weight,
                    forward,
                    backward,
                    max_sample,
                    destination,
                );
            } else {
                for (slot, (&left, &right)) in destination.iter_mut().zip(left.iter().zip(right)) {
                    *slot = if forward == 8 {
                        (left.to_u16() + right.to_u16() + 1) >> 1
                    } else {
                        let weighted = forward * i32::from(left.to_u16())
                            + backward * i32::from(right.to_u16());
                        round2_i32(weighted, 4).clamp(0, max_sample) as u16
                    };
                }
            }
        } else {
            for (col, slot) in destination.iter_mut().enumerate() {
                let source_col = [
                    (x0[0] + col as i32).clamp(params0.first_x, params0.last_x) as usize,
                    (x0[1] + col as i32).clamp(params1.first_x, params1.last_x) as usize,
                ];
                let left = reference0.sample(source_row[0], source_col[0]);
                let right = reference1.sample(source_row[1], source_col[1]);
                *slot = if forward == 8 {
                    ((left + right + 1) >> 1).clamp(0, max_sample) as u16
                } else {
                    round2_i32(forward * left + backward * right, 4).clamp(0, max_sample) as u16
                };
            }
        }
    }
    true
}

#[inline(always)]
fn blend_fullpel_u16_row(
    left: &[u16],
    right: &[u16],
    cwp_weight: i16,
    forward: i32,
    backward: i32,
    max_sample: i32,
    destination: &mut [u16],
) {
    const LANES: usize = 8;
    if cwp_weight == 8 {
        let mut copied = 0;
        for ((output, left), right) in destination
            .chunks_exact_mut(LANES)
            .zip(left.chunks_exact(LANES))
            .zip(right.chunks_exact(LANES))
        {
            let left = Simd::<u16, LANES>::from_slice(left);
            let right = Simd::<u16, LANES>::from_slice(right);
            output.copy_from_slice(&((left + right + Simd::splat(1)) >> 1).to_array()); // splot-copy-ok: publish equal-weight SIMD fullpel lanes
            copied += LANES;
        }
        for ((slot, &left), &right) in destination[copied..]
            .iter_mut()
            .zip(&left[copied..])
            .zip(&right[copied..])
        {
            *slot = (left + right + 1) >> 1;
        }
        return;
    }
    let mut copied = 0;
    for ((output, left), right) in destination
        .chunks_exact_mut(LANES)
        .zip(left.chunks_exact(LANES))
        .zip(right.chunks_exact(LANES))
    {
        output.copy_from_slice(&blend_fullpel_8(left, right, forward, backward, max_sample)); // splot-copy-ok: publish weighted SIMD fullpel lanes
        copied += LANES;
    }
    for ((slot, &left), &right) in destination[copied..]
        .iter_mut()
        .zip(&left[copied..])
        .zip(&right[copied..])
    {
        let weighted = forward * i32::from(left) + backward * i32::from(right);
        *slot = round2_i32(weighted, 4).clamp(0, max_sample) as u16;
    }
}

#[allow(
    clippy::inline_always,
    reason = "measured weighted fullpel SIMD hot path"
)]
#[inline(always)]
fn blend_fullpel_8(
    left: &[u16],
    right: &[u16],
    forward: i32,
    backward: i32,
    max_sample: i32,
) -> [u16; 8] {
    let left = Simd::<u16, 8>::from_slice(left);
    let right = Simd::<u16, 8>::from_slice(right);
    round2_simd(
        left.cast::<i32>() * Simd::splat(forward) + right.cast::<i32>() * Simd::splat(backward),
        4,
    )
    .simd_max(Simd::splat(0))
    .simd_min(Simd::splat(max_sample))
    .cast::<u16>()
    .to_array()
}

#[inline]
fn validate_compound_output<O>(
    params: &SubpelPredictParams,
    output: &[O],
    output_stride: usize,
) -> Result<()> {
    if output_stride < params.w {
        return Err(ReconError::StrideTooSmall {
            stride_samples: output_stride,
            storage_width: params.w,
        });
    }
    let output_len = (params.h - 1)
        .checked_mul(output_stride)
        .and_then(|len| len.checked_add(params.w))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided compound prediction sample count",
        })?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    Ok(())
}

/// Internal fast dispatch for caller-constructed, already validated parameters.
///
/// # Errors
/// Returns [`ReconError::StrideTooSmall`] or [`ReconError::BufferLengthMismatch`]
/// when the output rectangle does not fit the supplied storage.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "measured TIP compound hot path")]
#[inline(always)]
pub fn subpel_predict_block_compound_average_fast_validated_strided_into<T: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [u16],
    output_stride: usize,
) -> Result<bool> {
    debug_assert!(validate_subpel_params(params0).is_ok());
    debug_assert!(validate_subpel_params(params1).is_ok());
    debug_assert_eq!(
        (params0.w, params0.h, params0.bit_depth),
        (params1.w, params1.h, params1.bit_depth)
    );
    debug_assert!([params0, params1].iter().all(|params| {
        params.step_x == 1 << SCALE_SUBPEL_BITS && params.step_y == 1 << SCALE_SUBPEL_BITS
    }));
    validate_compound_output(params0, output, output_stride)?;
    Ok(subpel_predict_block_compound_average_fast_dispatch(
        reference0,
        params0,
        reference1,
        params1,
        cwp_weight,
        scratch,
        output,
        output_stride,
    ))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::inline_always, reason = "measured TIP compound hot path")]
#[inline(always)]
fn subpel_predict_block_compound_average_fast_dispatch<T: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [u16],
    output_stride: usize,
) -> bool {
    let phases = [params0, params1].map(|params| {
        (
            (params.start_x >> 6) & SUBPEL_MASK,
            (params.start_y >> 6) & SUBPEL_MASK,
        )
    });
    match phases {
        [(0, 0), (0, 0)] => subpel_predict_block_compound_average_fullpel_validated(
            reference0,
            params0,
            reference1,
            params1,
            cwp_weight,
            output,
            output_stride,
        ),
        [(x0, 0), (x1, 0)] if x0 != 0 && x1 != 0 => {
            subpel_predict_block_compound_average_horizontal_validated(
                reference0,
                params0,
                reference1,
                params1,
                cwp_weight,
                output,
                output_stride,
            )
        }
        [(x0, y0), (x1, y1)]
            if x0 != 0
                && y0 != 0
                && x1 != 0
                && y1 != 0
                && matches!(params0.w, 4 | 8)
                && params0.h <= 8 =>
        {
            subpel_predict_block_compound_average_2d_validated(
                reference0,
                params0,
                reference1,
                params1,
                cwp_weight,
                scratch,
                output,
                output_stride,
            )
        }
        _ => false,
    }
}

fn subpel_predict_block_compound_average_horizontal_validated<T: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [u16],
    output_stride: usize,
) -> bool {
    let window_x0 = subpel_horizontal_window_x(reference0, params0);
    let window_x1 = subpel_horizontal_window_x(reference1, params1);
    let Some(source0) = T::u16_slice(reference0.samples) else {
        return false;
    };
    let Some(source1) = T::u16_slice(reference1.samples) else {
        return false;
    };
    let (Some(window_x0), Some(window_x1)) = (window_x0, window_x1) else {
        clipped_compound::horizontal(
            [reference0, reference1],
            [params0, params1],
            [source0, source1],
            cwp_weight,
            output,
            output_stride,
        );
        return true;
    };

    let filters = [params0, params1].map(|params| {
        let filter = params.interp.pass_index(params.w as u32) as usize;
        let phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
        let (start, end) = ACTIVE_TAP_SPANS[filter][phase];
        let full = &SUBPEL_FILTERS[filter][phase];
        (
            &full[start..end],
            start,
            (start == 0 && end == NUM_TAPS).then_some(full),
        )
    });
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    let max_sample = i32::from(params0.bit_depth.max_sample());
    let y0 = [
        params0.start_y >> SCALE_SUBPEL_BITS,
        params1.start_y >> SCALE_SUBPEL_BITS,
    ];
    for row in 0..params0.h {
        let source_rows = [
            (y0[0] + row as i32)
                .clamp(params0.first_y, params0.last_y)
                .clamp(0, reference0.height as i32 - 1) as usize,
            (y0[1] + row as i32)
                .clamp(params1.first_y, params1.last_y)
                .clamp(0, reference1.height as i32 - 1) as usize,
        ];
        let windows = [
            &source0[source_rows[0] * reference0.stride + window_x0..],
            &source1[source_rows[1] * reference1.stride + window_x1..],
        ];
        let destination = &mut output[row * output_stride..][..params0.w];
        let vector_width8 = params0.w - params0.w % 8;
        for col in (0..vector_width8).step_by(8) {
            let mut predictors = [Simd::<i32, 8>::splat(0); 2];
            for reference in 0..2 {
                let (taps, tap_start, full_taps) = filters[reference];
                let window = windows[reference];
                predictors[reference] = match full_taps {
                    Some(full_taps) if Simd::<i32, 8>::admits(window.len(), col) => {
                        Simd::<i32, 8>::slid_tap_sum(window, col, full_taps)
                    }
                    _ => {
                        let mut sum = predictors[reference];
                        for (tap_offset, &tap) in taps.iter().enumerate() {
                            sum = tap_mac(
                                sum,
                                Simd::<u16, 8>::from_slice(&window[col + tap_start + tap_offset..])
                                    .cast(),
                                tap,
                            );
                        }
                        sum
                    }
                };
                predictors[reference] = round2_simd(predictors[reference], INTER_ROUND0);
            }
            let blended = round2_simd(
                predictors[0] * Simd::splat(forward) + predictors[1] * Simd::splat(backward),
                4 + compound_inter_post_round(),
            )
            .simd_clamp(Simd::splat(0), Simd::splat(max_sample))
            .cast::<u16>();
            destination[col..col + 8].copy_from_slice(&blended.to_array()); // splot-copy-ok: publish fused horizontal compound lanes
        }
        let vector_width4 = params0.w - params0.w % 4;
        for col in (vector_width8..vector_width4).step_by(4) {
            let mut predictors = [Simd::<i32, 4>::splat(0); 2];
            for reference in 0..2 {
                let (taps, tap_start, full_taps) = filters[reference];
                let window = windows[reference];
                predictors[reference] = match full_taps {
                    Some(full_taps) if Simd::<i32, 4>::admits(window.len(), col) => {
                        Simd::<i32, 4>::slid_tap_sum(window, col, full_taps)
                    }
                    _ => {
                        let mut sum = predictors[reference];
                        for (tap_offset, &tap) in taps.iter().enumerate() {
                            sum = tap_mac(
                                sum,
                                Simd::<u16, 4>::from_slice(&window[col + tap_start + tap_offset..])
                                    .cast(),
                                tap,
                            );
                        }
                        sum
                    }
                };
                predictors[reference] = round2_simd(predictors[reference], INTER_ROUND0);
            }
            let blended = round2_simd(
                predictors[0] * Simd::splat(forward) + predictors[1] * Simd::splat(backward),
                4 + compound_inter_post_round(),
            )
            .simd_clamp(Simd::splat(0), Simd::splat(max_sample))
            .cast::<u16>();
            destination[col..col + 4].copy_from_slice(&blended.to_array()); // splot-copy-ok: publish fused horizontal compound lanes
        }
        for (col, destination) in destination[vector_width4..].iter_mut().enumerate() {
            let col = vector_width4 + col;
            let mut predictors = [0i32; 2];
            for reference in 0..2 {
                let (taps, tap_start, _) = filters[reference];
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    predictors[reference] +=
                        tap * i32::from(windows[reference][col + tap_start + tap_offset]);
                }
                predictors[reference] = round2_i32(predictors[reference], INTER_ROUND0);
            }
            *destination = round2_i32(
                forward * predictors[0] + backward * predictors[1],
                4 + compound_inter_post_round(),
            )
            .clamp(0, max_sample) as u16;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn subpel_predict_block_compound_average_2d_validated<T: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [u16],
    output_stride: usize,
) -> bool {
    const SCRATCH_LEN: usize = 2 * (8 + NUM_TAPS - 1) * 8;
    let params = [params0, params1];
    let window_x0 = subpel_horizontal_window_x(reference0, params0);
    let window_x1 = subpel_horizontal_window_x(reference1, params1);
    let Some(source0) = T::u16_slice(reference0.samples) else {
        return false;
    };
    let Some(source1) = T::u16_slice(reference1.samples) else {
        return false;
    };
    let references = [reference0, reference1];
    let sources = [source0, source1];
    let Some(scratch) = scratch.get_mut(..SCRATCH_LEN) else {
        return false;
    };
    let (Some(window_x0), Some(window_x1)) = (window_x0, window_x1) else {
        clipped_compound::two_axis(
            references,
            params,
            sources,
            cwp_weight,
            scratch,
            output,
            output_stride,
        );
        return true;
    };
    let windows = [window_x0, window_x1];
    match params0.w {
        4 => fused_compound_average_2d::<4>(
            references,
            params,
            sources,
            windows,
            cwp_weight,
            scratch,
            output,
            output_stride,
        ),
        8 => fused_compound_average_2d::<8>(
            references,
            params,
            sources,
            windows,
            cwp_weight,
            scratch,
            output,
            output_stride,
        ),
        _ => return false,
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn fused_compound_average_2d<const LANES: usize>(
    references: [&ReferencePlaneView<'_, impl ReconSample>; 2],
    params: [&SubpelPredictParams; 2],
    sources: [&[u16]; 2],
    windows: [usize; 2],
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [u16],
    output_stride: usize,
) where
    Simd<i32, LANES>: SlideLanes,
{
    const MAX_INTERMEDIATE: usize = (8 + NUM_TAPS - 1) * 8;
    let (first, second) = scratch.split_at_mut(MAX_INTERMEDIATE);
    let intermediate = [first, second];
    let horizontal = params.map(|params| {
        let filter = params.interp.pass_index(params.w as u32) as usize;
        let phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
        let (start, end) = ACTIVE_TAP_SPANS[filter][phase];
        let full = &SUBPEL_FILTERS[filter][phase];
        (
            &full[start..end],
            start,
            (start == 0 && end == NUM_TAPS).then_some(full),
        )
    });
    for reference in 0..2 {
        for row in 0..params[reference].h + NUM_TAPS - 1 {
            let source_row = ((params[reference].start_y >> SCALE_SUBPEL_BITS) + row as i32 - 3)
                .clamp(params[reference].first_y, params[reference].last_y)
                as usize;
            let source = &sources[reference][source_row.min(references[reference].height - 1)
                * references[reference].stride
                + windows[reference]..];
            let (taps, tap_start, full_taps) = horizontal[reference];
            let sum = match full_taps {
                Some(full_taps) if Simd::<i32, LANES>::admits(source.len(), 0) => {
                    Simd::<i32, LANES>::slid_tap_sum(source, 0, full_taps)
                }
                _ => {
                    let mut sum = Simd::<i32, LANES>::splat(0);
                    for (offset, &tap) in taps.iter().enumerate() {
                        sum = tap_mac(
                            sum,
                            Simd::<u16, LANES>::from_slice(&source[tap_start + offset..]).cast(),
                            tap,
                        );
                    }
                    sum
                }
            };
            let lanes = round2_simd(sum, INTER_ROUND0).cast::<i16>().to_array();
            intermediate[reference][row * LANES..(row + 1) * LANES].copy_from_slice(&lanes); // splot-copy-ok: store horizontal SIMD lanes in caller scratch
        }
    }
    finish_fused_compound_2d!(
        LANES,
        params,
        cwp_weight,
        intermediate,
        output,
        output_stride
    );
}

/// Accumulates one AV2 § 7.13.3.18 filter tap into a 32-bit convolution sum.
///
/// Both factors are 16-bit: § 6 Table 6.3 caps `BitDepth` at 10, so reference
/// samples and `SUBPEL_INTERMEDIATE` values alike fit `i16`, and every
/// `Subpel_Filters` tap lies in `-24..=128`. Sign-extending both sides lets the
/// target fold the widening into the multiply-accumulate.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn tap_mac<const LANES: usize>(
    accumulator: Simd<i32, LANES>,
    samples: Simd<i16, LANES>,
    tap: i32,
) -> Simd<i32, LANES> {
    accumulator + samples.cast::<i32>() * Simd::<i16, LANES>::splat(tap as i16).cast::<i32>()
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn round2_simd<const LANES: usize>(value: Simd<i32, LANES>, shift: u32) -> Simd<i32, LANES> {
    (value + Simd::splat(1 << (shift - 1))) >> shift as i32
}

#[inline]
fn subpel_params_are_valid_fullpel(params: &SubpelPredictParams) -> bool {
    params.w > 0
        && params.h > 0
        && params.w <= MAX_BLOCK_DIM
        && params.h <= MAX_BLOCK_DIM
        && params.step_x == 1 << SCALE_SUBPEL_BITS
        && params.step_y == 1 << SCALE_SUBPEL_BITS
        && (params.start_x >> 6) & SUBPEL_MASK == 0
        && (params.start_y >> 6) & SUBPEL_MASK == 0
        && params
            .start_x
            .checked_add((params.w as i32 - 1) << SCALE_SUBPEL_BITS)
            .is_some()
}

std::thread_local! {
    /// Retained storage for the AV2 § 7.13.3.18 `intermediate[r][c]` horizontal
    /// pass.
    ///
    /// `i16` holds every legal value exactly. § 7.13.3.16 fixes `InterRound0`
    /// at 3 and notes the horizontal filter output always fits within 16 bits;
    /// § 6 Table 6.3 admits only `BitDepth` 8 and 10, and across the
    /// § 7.13.3.18 `Subpel_Filters` rows the positive taps sum to at most 184
    /// and the negative taps to at most -56 (filter 2, phase 8), so at
    /// `BitDepth == 10` the pass spans
    /// `Round2(-56 * 1023, 3) ..= Round2(184 * 1023, 3)`, that is
    /// `-7161..=23529`. The zero-phase shortcut writes `sample << 4 <= 16368`.
    static SUBPEL_INTERMEDIATE: std::cell::Cell<Option<Vec<i16>>> =
        const { std::cell::Cell::new(None) };
}

fn with_subpel_intermediate<R>(len: usize, f: impl FnOnce(&mut [i16]) -> R) -> R {
    SUBPEL_INTERMEDIATE.with(|slot| {
        let mut intermediate = slot.take().unwrap_or_default();
        if intermediate.len() < len {
            intermediate.resize(len, 0);
        }
        let result = f(&mut intermediate[..len]);
        slot.set(Some(intermediate));
        result
    })
}

/// Two-pass § 7.13.3.18 convolution core. With an unscaled horizontal step
/// the sub-pel phase is column-invariant, and when every clipped column read
/// is the identity a row's whole tap window is one contiguous `w + 7` slice
/// read directly; the vertical pass accumulates its eight consecutive
/// `w`-sample tap rows one row at a time. Both shapes perform the same
/// per-sample additions in the same ascending-tap order as the general
/// per-column fallback, which remains for scaled or clipped blocks. A zero
/// phase on either axis uses that filter row's pure center tap directly.
fn subpel_predict_block_internal<T: ReconSample, O: Clone + Default>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    inter_round1: u32,
    finish: impl FnMut(i32) -> O,
) -> Result<Vec<O>> {
    let intermediate_height = validate_subpel_params(params)?;
    let output_len = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "subpel output sample count",
        })?;
    let mut output = vec![O::default(); output_len];
    subpel_predict_block_internal_into_validated(
        reference,
        params,
        inter_round1,
        intermediate_height,
        None,
        &mut output,
        params.w,
        ScalarSubpelOutput(finish),
    )?;
    Ok(output)
}

#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn validate_subpel_params(params: &SubpelPredictParams) -> Result<usize> {
    let SubpelPredictParams {
        w,
        h,
        start_x,
        step_x,
        step_y,
        ..
    } = *params;

    if w == 0 {
        return Err(ReconError::ZeroDimension {
            field: "subpel block width",
        });
    }
    if h == 0 {
        return Err(ReconError::ZeroDimension {
            field: "subpel block height",
        });
    }
    if w > MAX_BLOCK_DIM || h > MAX_BLOCK_DIM {
        return Err(ReconError::SubpelBlockDimensionUnsupported { w, h });
    }
    if step_x < 0 || step_y < 0 {
        return Err(ReconError::SubpelNegativeStep { step_x, step_y });
    }
    if !(MIN_SCALE_STEP..=MAX_SCALE_STEP).contains(&step_x)
        || !(MIN_SCALE_STEP..=MAX_SCALE_STEP).contains(&step_y)
    {
        return Err(ReconError::ArithmeticOverflow {
            context: "subpel step outside AV2 reference-scaling range",
        });
    }

    let intermediate_height =
        (((h as i32 - 1) * step_y + (1 << SCALE_SUBPEL_BITS) - 1) >> SCALE_SUBPEL_BITS) + 8;
    start_x
        .checked_add((w as i32 - 1) * step_x)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "subpel horizontal coordinate",
        })?;

    Ok(intermediate_height as usize)
}

#[allow(clippy::too_many_arguments)]
fn subpel_predict_block_internal_into_validated<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    inter_round1: u32,
    intermediate_height: usize,
    scratch: Option<&mut [i16]>,
    output: &mut [O],
    output_stride: usize,
    mut finish: impl SubpelOutput<O>,
) -> Result<()> {
    if output_stride < params.w {
        return Err(ReconError::StrideTooSmall {
            stride_samples: output_stride,
            storage_width: params.w,
        });
    }
    let output_len = (params.h - 1)
        .checked_mul(output_stride)
        .and_then(|len| len.checked_add(params.w))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided subpel output sample count",
        })?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }

    let SubpelPredictParams {
        interp,
        w,
        h,
        start_x,
        start_y,
        step_x,
        step_y,
        first_x,
        first_y,
        last_x,
        last_y,
        bit_depth: _,
    } = *params;

    if step_x == 1 << SCALE_SUBPEL_BITS && step_y == 1 << SCALE_SUBPEL_BITS {
        let h_phase = (start_x >> 6) & SUBPEL_MASK;
        let v_phase = (start_y >> 6) & SUBPEL_MASK;
        match (h_phase == 0, v_phase == 0) {
            (true, true) => subpel_copy_block_into(
                reference,
                params,
                2 * FILTER_BITS - (INTER_ROUND0 + inter_round1),
                output,
                output_stride,
                &mut finish,
            ),
            (false, true) => subpel_horizontal_only_into(
                reference,
                params,
                inter_round1,
                output,
                output_stride,
                &mut finish,
            ),
            (true, false) => subpel_vertical_only_into(
                reference,
                params,
                inter_round1,
                output,
                output_stride,
                &mut finish,
            ),
            (false, false) => {}
        }
        if h_phase == 0 || v_phase == 0 {
            return Ok(());
        }
    }

    let h_filter = interp.pass_index(w as u32);
    let h_filter_rows = &SUBPEL_FILTERS[h_filter as usize];

    let x_window_start = if step_x == 1 << SCALE_SUBPEL_BITS {
        subpel_horizontal_window_x(reference, params)
    } else {
        None
    };

    let v_filter = interp.pass_index(h as u32);
    let mut read_lo = intermediate_height;
    let mut read_hi = 0usize;
    for r in 0..h {
        let p = scaled_position(start_y & 1023, step_y, r);
        let base = (p >> SCALE_SUBPEL_BITS) as usize;
        if base
            .checked_add(NUM_TAPS)
            .is_none_or(|end| end > intermediate_height)
        {
            return Err(ReconError::SubpelIntermediateOutOfRange {
                base,
                intermediate_height,
            });
        }
        let phase = ((p >> 6) & SUBPEL_MASK) as usize;
        let (lo, hi) = if phase == 0 {
            (base + 3, base + 4)
        } else {
            let (tap_start, tap_end) = ACTIVE_TAP_SPANS[v_filter as usize][phase];
            (base + tap_start, base + tap_end)
        };
        read_lo = read_lo.min(lo);
        read_hi = read_hi.max(hi);
    }
    read_hi = read_hi.min(intermediate_height);
    read_lo = read_lo.min(read_hi);

    let intermediate_len =
        intermediate_height
            .checked_mul(w)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "subpel intermediate sample count",
            })?;
    let mut run = |intermediate: &mut [i16]| {
        for r in read_lo..read_hi {
            let ref_row = ((start_y >> SCALE_SUBPEL_BITS) + r as i32 - 3).clamp(first_y, last_y);
            let ref_row = (ref_row as usize).min(reference.readable_rows - 1);
            let row_out = &mut intermediate[r * w..(r + 1) * w];
            let window = x_window_start.and_then(|window_start| {
                let row_base = ref_row * reference.stride + window_start;
                let taps_end = row_base + w + NUM_TAPS - 1;
                reference
                    .samples
                    .get(row_base..taps_end + SLIDE_RESERVE)
                    .or_else(|| reference.samples.get(row_base..taps_end))
            });
            if let Some(window) = window {
                let phase = ((start_x >> 6) & SUBPEL_MASK) as usize;
                if phase == 0 {
                    for (out, sample) in row_out.iter_mut().zip(&window[3..3 + w]) {
                        *out = (sample.to_u16() << (FILTER_BITS - INTER_ROUND0)) as i16;
                    }
                    continue;
                }
                let full_taps = &h_filter_rows[phase];
                let (tap_start, tap_end) = ACTIVE_TAP_SPANS[h_filter as usize][phase];
                let taps = &full_taps[tap_start..tap_end];
                if let Some(window) = T::u16_slice(window) {
                    let full_span = tap_start == 0 && tap_end == NUM_TAPS;
                    let available = window.len();
                    let vector_width16 = w - w % 16;
                    for c in (0..vector_width16).step_by(16) {
                        let sum = if full_span && Simd::<i32, 16>::admits(available, c) {
                            Simd::<i32, 16>::slid_tap_sum(window, c, full_taps)
                        } else {
                            let mut sum = Simd::<i32, 16>::splat(0);
                            for (tap_offset, &tap) in taps.iter().enumerate() {
                                sum = tap_mac(
                                    sum,
                                    Simd::<u16, 16>::from_slice(
                                        &window[c + tap_start + tap_offset..],
                                    )
                                    .cast(),
                                    tap,
                                );
                            }
                            sum
                        };
                        let filtered = round2_simd(sum, INTER_ROUND0).cast::<i16>().to_array();
                        row_out[c..c + 16].copy_from_slice(&filtered); // splot-copy-ok: publish sixteen SIMD convolution outputs
                    }
                    let vector_width8 = w - w % 8;
                    for c in (vector_width16..vector_width8).step_by(8) {
                        let sum = if full_span && Simd::<i32, 8>::admits(available, c) {
                            Simd::<i32, 8>::slid_tap_sum(window, c, full_taps)
                        } else {
                            let mut sum = Simd::<i32, 8>::splat(0);
                            for (tap_offset, &tap) in taps.iter().enumerate() {
                                sum = tap_mac(
                                    sum,
                                    Simd::<u16, 8>::from_slice(
                                        &window[c + tap_start + tap_offset..],
                                    )
                                    .cast(),
                                    tap,
                                );
                            }
                            sum
                        };
                        let filtered = round2_simd(sum, INTER_ROUND0).cast::<i16>().to_array();
                        row_out[c..c + 8].copy_from_slice(&filtered); // splot-copy-ok: publish eight SIMD convolution outputs
                    }
                    let vector_width4 = w - w % 4;
                    for c in (vector_width8..vector_width4).step_by(4) {
                        let sum = if full_span && Simd::<i32, 4>::admits(available, c) {
                            Simd::<i32, 4>::slid_tap_sum(window, c, full_taps)
                        } else {
                            let mut sum = Simd::<i32, 4>::splat(0);
                            for (tap_offset, &tap) in taps.iter().enumerate() {
                                sum = tap_mac(
                                    sum,
                                    Simd::<u16, 4>::from_slice(
                                        &window[c + tap_start + tap_offset..],
                                    )
                                    .cast(),
                                    tap,
                                );
                            }
                            sum
                        };
                        let filtered = round2_simd(sum, INTER_ROUND0).cast::<i16>().to_array();
                        row_out[c..c + 4].copy_from_slice(&filtered); // splot-copy-ok: publish four SIMD convolution lanes into row scratch
                    }
                    for c in vector_width4..w {
                        let mut sum = 0i32;
                        for (tap_offset, &tap) in taps.iter().enumerate() {
                            sum += tap * i32::from(window[c + tap_start + tap_offset]);
                        }
                        row_out[c] = round2_i32(sum, INTER_ROUND0) as i16;
                    }
                    continue;
                }
                for (out, win) in row_out.iter_mut().zip(window.windows(NUM_TAPS)) {
                    let mut s = 0i32;
                    let samples = &win[tap_start..tap_start + taps.len()];
                    for (&tap, &sample) in taps.iter().zip(samples) {
                        s += tap * i32::from(sample.to_u16());
                    }
                    *out = round2_i32(s, INTER_ROUND0) as i16;
                }
                continue;
            }
            if clipped_edges::horizontal_intermediate(
                reference,
                params,
                ref_row,
                h_filter as usize,
                row_out,
            ) {
                continue;
            }
            for c in 0..w {
                let p = scaled_position(start_x, step_x, c);
                let phase = ((p >> 6) & SUBPEL_MASK) as usize;
                if phase == 0 {
                    let ref_col = (p >> SCALE_SUBPEL_BITS).clamp(first_x, last_x);
                    intermediate[r * w + c] = (reference.sample(ref_row, ref_col as usize)
                        << (FILTER_BITS - INTER_ROUND0))
                        as i16;
                    continue;
                }
                let taps = &h_filter_rows[phase];
                let (tap_start, tap_end) = ACTIVE_TAP_SPANS[h_filter as usize][phase];
                let taps = &taps[tap_start..tap_end];
                let mut s = 0i32;
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    let ref_col = ((p >> SCALE_SUBPEL_BITS) + t as i32 - 3).clamp(first_x, last_x);
                    s += tap * reference.sample(ref_row, ref_col as usize);
                }
                intermediate[r * w + c] = round2_i32(s, INTER_ROUND0) as i16;
            }
        }

        let v_filter_rows = &SUBPEL_FILTERS[v_filter as usize];

        for r in 0..h {
            let p = scaled_position(start_y & 1023, step_y, r);
            let phase = ((p >> 6) & SUBPEL_MASK) as usize;
            let taps = &v_filter_rows[phase];
            let base = (p >> SCALE_SUBPEL_BITS) as usize;
            let output = &mut output[r * output_stride..][..w];
            if phase == 0 {
                let center = &intermediate[(base + 3) * w..(base + 4) * w];
                for (out, &value) in output.iter_mut().zip(center) {
                    *out = finish.one(round2_i32(i32::from(value) << FILTER_BITS, inter_round1));
                }
                continue;
            }
            let rows = &intermediate[base * w..(base + NUM_TAPS) * w];
            let (tap_start, tap_end) = ACTIVE_TAP_SPANS[v_filter as usize][phase];
            let taps = &taps[tap_start..tap_end];
            let vector_width16 = w - w % 16;
            for c in (0..vector_width16).step_by(16) {
                let mut sum = Simd::<i32, 16>::splat(0);
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    sum = tap_mac(sum, Simd::from_slice(&rows[t * w + c..]), tap);
                }
                finish.sixteen(round2_simd(sum, inter_round1), &mut output[c..c + 16]);
            }
            let vector_width8 = w - w % 8;
            for c in (vector_width16..vector_width8).step_by(8) {
                let mut sum = Simd::<i32, 8>::splat(0);
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    sum = tap_mac(sum, Simd::from_slice(&rows[t * w + c..]), tap);
                }
                finish.eight(round2_simd(sum, inter_round1), &mut output[c..c + 8]);
            }
            let vector_width4 = w - w % 4;
            for c in (vector_width8..vector_width4).step_by(4) {
                let mut sum = Simd::<i32, 4>::splat(0);
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    sum = tap_mac(sum, Simd::from_slice(&rows[t * w + c..]), tap);
                }
                finish.four(round2_simd(sum, inter_round1), &mut output[c..c + 4]);
            }
            for (c, out) in output[vector_width4..].iter_mut().enumerate() {
                let c = vector_width4 + c;
                let mut sum = 0i32;
                for (tap_offset, &tap) in taps.iter().enumerate() {
                    let t = tap_start + tap_offset;
                    sum += tap * i32::from(rows[t * w + c]);
                }
                *out = finish.one(round2_i32(sum, inter_round1));
            }
        }

        Ok(())
    };
    match scratch.and_then(|scratch| scratch.get_mut(..intermediate_len)) {
        Some(intermediate) => run(intermediate),
        None => with_subpel_intermediate(intermediate_len, run),
    }
}

fn scaled_position(start: i32, step: i32, index: usize) -> i32 {
    start + step * index as i32
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
