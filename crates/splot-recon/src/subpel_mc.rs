// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.18 block inter prediction (sub-pel motion compensation) kernel.
//!
//! This is the separable interpolation-filter convolution that produces the
//! sub-pel motion-compensated prediction block from a reference plane and a
//! fractional motion vector. The full-pel (zero-fraction) case reduces to a
//! straight reference-sample copy; the sub-pel case applies a horizontal filter
//! pass into an intermediate array and then a vertical filter pass.
//!
//! The kernel is scheduler-free and source-backed: callers resolve the
//! § 7.13.3.17 motion-vector scaling (`startX` / `startY` / `stepX` / `stepY`),
//! the § 7.13.3.18 reference-clipping region (`firstX` / `firstY` / `lastX` /
//! `lastY`), the § 6 `interp_filter`, the block dimensions, and the reference
//! plane samples; this kernel runs the two-pass convolution with the
//! § 7.13.3.16 `InterRound0` / `InterRound1` rounding and the § 4.8 `Clip1`
//! final clamp used by the single-reference (non-compound) § 7.13.3 write
//! (`CurrFrame[plane][y + i][x + j] = Clip1(Preds[0][i][j])`).
//!
//! The § 9 `Subpel_Filters[6][16][8]` coefficient table is transcribed verbatim
//! from `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-18` and must never
//! be regenerated from AVM.
//!
//! Feature tracking: `RECON-SUBPEL-MC`.

use crate::error::{ReconError, Result};
use crate::format::{BitDepth, ReconSample};
use crate::math::round2_i32;

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
        let required = height
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
    /// read additionally clamps to the view's own `width`/`height` so it is total
    /// even if a caller passes a clipping region wider than the actual plane
    /// (defense in depth — the function never indexes out of bounds).
    pub fn sample(&self, row: usize, col: usize) -> i32 {
        let row = row.min(self.height - 1);
        let col = col.min(self.width - 1);
        i32::from(self.samples[row * self.stride + col].to_u16())
    }

    pub(crate) fn row(&self, row: usize) -> &[T] {
        let row = row.min(self.height - 1);
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
    subpel_predict_block_internal::<_, _, _, INTER_ROUND1_NON_COMPOUND>(reference, params, |pred| {
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
    let intermediate_height = validate_subpel_params(params)?;
    if params.interp == InterpolationFilter::Bilinear
        && params.step_x == 1 << SCALE_SUBPEL_BITS
        && params.step_y == 1 << SCALE_SUBPEL_BITS
    {
        let h_phase = (params.start_x >> 6) & SUBPEL_MASK;
        let v_phase = (params.start_y >> 6) & SUBPEL_MASK;
        match (h_phase == 0, v_phase == 0) {
            (false, true) => return subpel_bilinear_horizontal_into(reference, params, output),
            (true, false) => return subpel_bilinear_vertical_into(reference, params, output),
            (false, false) => return subpel_bilinear_2d_into(reference, params, output),
            (true, true) => {}
        }
    }
    let max_sample = i32::from(params.bit_depth.max_sample());
    subpel_predict_block_internal_into_validated::<_, _, _, INTER_ROUND1_NON_COMPOUND>(
        reference,
        params,
        intermediate_height,
        None,
        output,
        params.w,
        |pred| pred.clamp(0, max_sample) as u16,
    )
}

/// Reuses the seven stable columns shared by horizontally adjacent 16x16
/// zero-phase TIP refine-MV predictors and fills the other nine columns.
///
/// `output` must initially contain the prediction for the same reference,
/// motion vector, and vertical position eight samples left of `params.start_x`.
/// The leftmost column is regenerated because the refine-MV clipping window
/// moves with the 8x8 prediction unit. Returns `Ok(false)` when the current
/// predictor is not an eligible zero-phase, unscaled 16x16 block.
///
/// # Errors
///
/// Returns the same parameter and output-length errors as
/// [`subpel_predict_block_into`].
pub fn subpel_predict_16x16_fullpel_horizontal_overlap_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> Result<bool> {
    const SHIFT: usize = 8;
    if params.step_x != 1 << SCALE_SUBPEL_BITS
        || params.step_y != 1 << SCALE_SUBPEL_BITS
        || (params.start_x >> 6) & SUBPEL_MASK != 0
        || (params.start_y >> 6) & SUBPEL_MASK != 0
        || params.w != 16
        || params.h != 16
    {
        return Ok(false);
    }
    validate_subpel_params(params)?;
    let output_len = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "overlapping subpel prediction sample count",
        })?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }

    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let direct_x = subpel_direct_copy_x(reference, params);
    let retained = params.w - SHIFT;
    for row in 0..params.h {
        let source_row = (y0 + row as i32).clamp(params.first_y, params.last_y) as usize;
        let destination = &mut output[row * params.w..][..params.w];
        // splot-copy-ok: retain the seven horizontally overlapping TIP predictor columns
        destination.copy_within(SHIFT.., 0);
        if let Some(x) = direct_x {
            for (slot, sample) in destination[retained..]
                .iter_mut()
                .zip(&reference.row(source_row)[x + retained..x + retained + SHIFT])
            {
                *slot = sample.to_u16();
            }
        } else {
            for (col, slot) in destination[retained..].iter_mut().enumerate() {
                let source_col =
                    (x0 + (retained + col) as i32).clamp(params.first_x, params.last_x) as usize;
                *slot = reference.sample(source_row, source_col) as u16;
            }
        }
        let first_col = x0.clamp(params.first_x, params.last_x) as usize;
        destination[0] = reference.sample(source_row, first_col) as u16;
    }
    Ok(true)
}

fn bilinear_sample(left: u16, right: u16, phase: i32) -> u16 {
    let left = i32::from(left);
    let right = i32::from(right);
    round2_i32((16 - phase) * left + phase * right, SUBPEL_BITS) as u16
}

fn subpel_bilinear_horizontal_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> Result<()> {
    let output_len = params.w * params.h;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let phase = (params.start_x >> 6) & SUBPEL_MASK;
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
            .clamp(0, reference.height as i32 - 1) as usize;
        let source = reference.row(row);
        let destination = &mut output[r * params.w..][..params.w];
        if let Some(x) = direct_x {
            for (out, pair) in destination
                .iter_mut()
                .zip(source[x..=x + params.w].windows(2))
            {
                *out = bilinear_sample(pair[0].to_u16(), pair[1].to_u16(), phase);
            }
        } else {
            for (c, out) in destination.iter_mut().enumerate() {
                *out = bilinear_sample(
                    source[clipped_x[c]].to_u16(),
                    source[clipped_x[c + 1]].to_u16(),
                    phase,
                );
            }
        }
    }
    Ok(())
}

fn subpel_bilinear_vertical_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> Result<()> {
    let output_len = params.w * params.h;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let phase = (params.start_y >> 6) & SUBPEL_MASK;
    let direct_x = subpel_direct_copy_x(reference, params);
    for r in 0..params.h {
        let top = (y0 + r as i32)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.height as i32 - 1) as usize;
        let bottom = (y0 + r as i32 + 1)
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.height as i32 - 1) as usize;
        let top = reference.row(top);
        let bottom = reference.row(bottom);
        let destination = &mut output[r * params.w..][..params.w];
        if let Some(x) = direct_x {
            for (out, (&top, &bottom)) in destination
                .iter_mut()
                .zip(top[x..x + params.w].iter().zip(&bottom[x..x + params.w]))
            {
                *out = bilinear_sample(top.to_u16(), bottom.to_u16(), phase);
            }
        } else {
            for (c, out) in destination.iter_mut().enumerate() {
                let col = (x0 + c as i32)
                    .clamp(params.first_x, params.last_x)
                    .clamp(0, reference.width as i32 - 1) as usize;
                *out = bilinear_sample(top[col].to_u16(), bottom[col].to_u16(), phase);
            }
        }
    }
    Ok(())
}

fn subpel_bilinear_2d_into<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [u16],
) -> Result<()> {
    let output_len = params.w * params.h;
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
    let direct_x = usize::try_from(x0).ok().filter(|&x| {
        x0 >= params.first_x
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

    let horizontal_row = |row: i32, destination: &mut [i32]| {
        let row = row
            .clamp(params.first_y, params.last_y)
            .clamp(0, reference.height as i32 - 1) as usize;
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
        for (out, (&top, &bottom)) in output[r * params.w..][..params.w]
            .iter_mut()
            .zip(top.iter().zip(bottom.iter()))
        {
            *out = round2_i32(v0 * top + v1 * bottom, INTER_ROUND1_NON_COMPOUND)
                .clamp(0, max_sample) as u16;
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
    subpel_predict_block_internal::<_, _, _, INTER_ROUND1_COMPOUND>(reference, params, |pred| pred)
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
    scratch: Option<&mut [i32]>,
    output: &mut [i32],
    output_stride: usize,
) -> Result<()> {
    let intermediate_height = validate_subpel_params(params)?;
    subpel_predict_block_internal_into_validated::<_, _, _, INTER_ROUND1_COMPOUND>(
        reference,
        params,
        intermediate_height,
        scratch,
        output,
        output_stride,
        |pred| pred,
    )
}

/// Produces one compound intermediate predictor and blends it directly with
/// the caller-owned first predictor using a uniform § 7.13.3.16 weight.
///
/// Both `pred0` and `output` are contiguous row-major `params.w * params.h`
/// blocks. The second intermediate is convolved into scratch storage and
/// blended row-wise.
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
/// optionally provides the horizontal-pass intermediate and second-predictor
/// storage; when absent or too small the convolution falls back to its
/// internal storage.
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
    scratch: Option<&mut [i32]>,
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
    let intermediate_len =
        intermediate_height
            .checked_mul(params.w)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "subpel intermediate sample count",
            })?;
    let total_len =
        intermediate_len
            .checked_add(sample_count)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound prediction sample count",
            })?;
    let mut run = |buffer: &mut [i32]| -> Result<()> {
        let (conv_scratch, pred1) = buffer.split_at_mut(intermediate_len);
        subpel_predict_block_internal_into_validated::<_, _, _, INTER_ROUND1_COMPOUND>(
            reference,
            params,
            intermediate_height,
            Some(conv_scratch),
            pred1,
            params.w,
            |pred| pred,
        )?;
        for (destination, (pred0_row, pred1_row)) in output.chunks_mut(output_stride).zip(
            pred0
                .chunks_exact(params.w)
                .zip(pred1.chunks_exact(params.w)),
        ) {
            for (out, (&first, &second)) in destination[..params.w]
                .iter_mut()
                .zip(pred0_row.iter().zip(pred1_row))
            {
                *out = blend_compound_average_weighted_sample(
                    first,
                    second,
                    params.bit_depth,
                    cwp_weight,
                );
            }
        }
        Ok(())
    };
    match scratch.and_then(|scratch| scratch.get_mut(..total_len)) {
        Some(buffer) => run(buffer),
        None => with_subpel_intermediate(total_len, run),
    }
}

/// Produces and blends two zero-phase, unscaled compound predictors directly
/// into caller-owned strided output without materializing either intermediate.
///
/// Returns `Ok(false)` when either predictor is not zero-phase and unscaled, or
/// when their block geometry and bit depth differ.
///
/// # Errors
///
/// Returns the same validation and output-layout errors as
/// [`subpel_predict_block_compound_average_strided_into`].
#[allow(clippy::too_many_arguments)]
pub fn subpel_predict_block_compound_average_fullpel_strided_into<T: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [u16],
    output_stride: usize,
) -> Result<bool> {
    let fullpel = |params: &SubpelPredictParams| {
        params.step_x == 1 << SCALE_SUBPEL_BITS
            && params.step_y == 1 << SCALE_SUBPEL_BITS
            && (params.start_x >> 6) & SUBPEL_MASK == 0
            && (params.start_y >> 6) & SUBPEL_MASK == 0
    };
    if !fullpel(params0)
        || !fullpel(params1)
        || params0.w != params1.w
        || params0.h != params1.h
        || params0.bit_depth != params1.bit_depth
    {
        return Ok(false);
    }
    validate_subpel_params(params0)?;
    validate_subpel_params(params1)?;
    if output_stride < params0.w {
        return Err(ReconError::StrideTooSmall {
            stride_samples: output_stride,
            storage_width: params0.w,
        });
    }
    let output_len = (params0.h - 1)
        .checked_mul(output_stride)
        .and_then(|len| len.checked_add(params0.w))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "strided compound prediction sample count",
        })?;
    if output.len() < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: output.len(),
        });
    }

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
            for (slot, (&left, &right)) in destination.iter_mut().zip(
                reference0.row(source_row[0])[x0..x0 + params0.w]
                    .iter()
                    .zip(&reference1.row(source_row[1])[x1..x1 + params0.w]),
            ) {
                let weighted =
                    forward * i32::from(left.to_u16()) + backward * i32::from(right.to_u16());
                *slot = round2_i32(weighted, 4).clamp(0, max_sample) as u16;
            }
        } else {
            for (col, slot) in destination.iter_mut().enumerate() {
                let source_col = [
                    (x0[0] + col as i32).clamp(params0.first_x, params0.last_x) as usize,
                    (x0[1] + col as i32).clamp(params1.first_x, params1.last_x) as usize,
                ];
                let weighted = forward * reference0.sample(source_row[0], source_col[0])
                    + backward * reference1.sample(source_row[1], source_col[1]);
                *slot = round2_i32(weighted, 4).clamp(0, max_sample) as u16;
            }
        }
    }
    Ok(true)
}

/// Blends two § 7.13.3.18 compound intermediate predictors with § 7.13.3.16
/// COMPOUND_AVERAGE and the supplied `cwpWeight`, then applies the final § 4.8
/// `Clip1`.
///
/// # Errors
///
/// Returns [`ReconError::CompoundBlendLengthMismatch`] when `pred0` and `pred1`
/// have different lengths.
pub fn blend_compound_average_weighted(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: BitDepth,
    cwp_weight: i16,
) -> Result<Vec<u16>> {
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }

    Ok(pred0
        .iter()
        .zip(pred1.iter())
        .map(|(&left, &right)| {
            blend_compound_average_weighted_sample(left, right, bit_depth, cwp_weight)
        })
        .collect())
}

/// Blends one pair of § 7.13.3.18 compound intermediate samples with the
/// supplied § 7.13.3.16 `cwpWeight`, then applies the final § 4.8 `Clip1`.
#[inline]
pub fn blend_compound_average_weighted_sample(
    pred0: i32,
    pred1: i32,
    bit_depth: BitDepth,
    cwp_weight: i16,
) -> u16 {
    let forward = i32::from(cwp_weight);
    let backward = 16 - forward;
    let blended = round2_i32(
        forward * pred0 + backward * pred1,
        4 + compound_inter_post_round(),
    );
    blended.clamp(0, i32::from(bit_depth.max_sample())) as u16
}

/// Blends two § 7.13.3.18 compound intermediate predictors with § 7.13.3.16
/// COMPOUND_AVERAGE and `CWP_EQUAL` (`cwpWeight == 8`), then applies the final
/// § 4.8 `Clip1`.
///
/// With `InterRound0 == 3`, compound `InterRound1 == 7`, and `FILTER_BITS == 7`,
/// `InterPostRound == 4`; the equal-weight formula
/// `Round2(8 * p0 + 8 * p1, 4 + InterPostRound)` simplifies exactly to
/// `Round2(p0 + p1, 1 + InterPostRound)`.
///
/// # Errors
///
/// Returns [`ReconError::CompoundBlendLengthMismatch`] when `pred0` and `pred1`
/// have different lengths.
pub fn blend_compound_average_equal(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: BitDepth,
) -> Result<Vec<u16>> {
    blend_compound_average_weighted(pred0, pred1, bit_depth, 8)
}

const fn compound_inter_post_round() -> u32 {
    2 * FILTER_BITS - (INTER_ROUND0 + INTER_ROUND1_COMPOUND)
}

/// The zero-phase unscaled § 7.13.3.18 special case: with `stepX == stepY ==
/// (1 << SCALE_SUBPEL_BITS)` and both sub-pel phases zero, every filter row is
/// the pure `{ .., 128, .. }` tap, so the two-pass convolution is exactly the
/// clipped reference sample scaled by `1 << (2 * FILTER_BITS - (InterRound0 +
/// InterRound1))` — `Round2(128 * v, 3) == 16 * v` and `Round2(2048 * v, 11)
/// == v` / `Round2(2048 * v, 7) == 16 * v` hold exactly for every `v >= 0`
/// because each partial product is a multiple of the rounding divisor.
fn subpel_copy_block_into<T: ReconSample, O>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    shift_up: u32,
    output: &mut [O],
    output_stride: usize,
    mut finish: impl FnMut(i32) -> O,
) {
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let direct_x = subpel_direct_copy_x(reference, params);
    for r in 0..params.h {
        let row = (y0 + r as i32).clamp(params.first_y, params.last_y) as usize;
        let output = &mut output[r * output_stride..][..params.w];
        if let Some(x) = direct_x {
            let row = row.min(reference.height - 1);
            let start = row * reference.stride + x;
            for (out, sample) in output
                .iter_mut()
                .zip(&reference.samples[start..start + params.w])
            {
                *out = finish(i32::from(sample.to_u16()) << shift_up);
            }
        } else {
            for (c, out) in output.iter_mut().enumerate() {
                let col = (x0 + c as i32).clamp(params.first_x, params.last_x) as usize;
                *out = finish(reference.sample(row, col) << shift_up);
            }
        }
    }
}

fn subpel_direct_copy_x<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Option<usize> {
    usize::try_from(params.start_x >> SCALE_SUBPEL_BITS)
        .ok()
        .filter(|&x| {
            x >= usize::try_from(params.first_x.max(0)).unwrap_or(usize::MAX)
                && x.checked_add(params.w).is_some_and(|end| {
                    end <= reference.width
                        && i32::try_from(end - 1).is_ok_and(|last| last <= params.last_x)
                })
        })
}

fn subpel_horizontal_window_x<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Option<usize> {
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    (x0 - 3 >= params.first_x.max(0)
        && x0 + params.w as i32 + 3 <= params.last_x.min(reference.width as i32 - 1))
    .then(|| (x0 - 3) as usize)
}

fn subpel_horizontal_only_into<T: ReconSample, O, F: FnMut(i32) -> O, const INTER_ROUND1: u32>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [O],
    output_stride: usize,
    finish: &mut F,
) {
    let h_filter = params.interp.pass_index(params.w as u32) as usize;
    let phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
    let full_taps = &SUBPEL_FILTERS[h_filter][phase];
    let (tap_start, tap_end) = ACTIVE_TAP_SPANS[h_filter][phase];
    let taps = &full_taps[tap_start..tap_end];
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let x_window_start = subpel_horizontal_window_x(reference, params);

    for r in 0..params.h {
        let ref_row = ((params.start_y >> SCALE_SUBPEL_BITS) + r as i32)
            .clamp(params.first_y, params.last_y) as usize;
        let ref_row = ref_row.min(reference.height - 1);
        let row_out = &mut output[r * output_stride..][..params.w];
        if let Some(window_start) = x_window_start {
            let row_base = ref_row * reference.stride + window_start;
            let window = &reference.samples[row_base..row_base + params.w + NUM_TAPS - 1];
            for (out, win) in row_out.iter_mut().zip(window.windows(NUM_TAPS)) {
                let Some(win) = win.first_chunk::<NUM_TAPS>() else {
                    continue;
                };
                let mut sum = 0i32;
                for (&tap, &sample) in full_taps.iter().zip(win) {
                    sum += tap * i32::from(sample.to_u16());
                }
                let horizontal = round2_i32(sum, INTER_ROUND0);
                *out = finish(round2_i32(horizontal << FILTER_BITS, INTER_ROUND1));
            }
            continue;
        }
        for (c, out) in row_out.iter_mut().enumerate() {
            let mut sum = 0i32;
            for (tap_offset, &tap) in taps.iter().enumerate() {
                let t = tap_start + tap_offset;
                let ref_col =
                    (x0 + c as i32 + t as i32 - 3).clamp(params.first_x, params.last_x) as usize;
                sum += tap * reference.sample(ref_row, ref_col);
            }
            let horizontal = round2_i32(sum, INTER_ROUND0);
            *out = finish(round2_i32(horizontal << FILTER_BITS, INTER_ROUND1));
        }
    }
}

fn subpel_vertical_only_into<T: ReconSample, O, F: FnMut(i32) -> O, const INTER_ROUND1: u32>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    output: &mut [O],
    output_stride: usize,
    finish: &mut F,
) {
    let v_filter = params.interp.pass_index(params.h as u32) as usize;
    let phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
    let taps = &SUBPEL_FILTERS[v_filter][phase];
    let (tap_start, tap_end) = ACTIVE_TAP_SPANS[v_filter][phase];
    let taps = &taps[tap_start..tap_end];
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    let y0 = params.start_y >> SCALE_SUBPEL_BITS;
    let direct_x = subpel_direct_copy_x(reference, params);
    let mut acc = [0i32; MAX_BLOCK_DIM];

    for r in 0..params.h {
        let acc = &mut acc[..params.w];
        acc.fill(0);
        for (tap_offset, &tap) in taps.iter().enumerate() {
            let t = tap_start + tap_offset;
            let ref_row =
                (y0 + r as i32 + t as i32 - 3).clamp(params.first_y, params.last_y) as usize;
            let ref_row = ref_row.min(reference.height - 1);
            if let Some(x) = direct_x {
                let start = ref_row * reference.stride + x;
                for (sum, sample) in acc
                    .iter_mut()
                    .zip(&reference.samples[start..start + params.w])
                {
                    *sum += tap * i32::from(sample.to_u16());
                }
            } else {
                for (c, sum) in acc.iter_mut().enumerate() {
                    let ref_col = (x0 + c as i32).clamp(params.first_x, params.last_x) as usize;
                    *sum += tap * reference.sample(ref_row, ref_col);
                }
            }
        }
        let row_out = &mut output[r * output_stride..][..params.w];
        for (out, &sum) in row_out.iter_mut().zip(acc.iter()) {
            *out = finish(round2_i32(
                sum << (FILTER_BITS - INTER_ROUND0),
                INTER_ROUND1,
            ));
        }
    }
}

std::thread_local! {
    static SUBPEL_INTERMEDIATE: std::cell::Cell<Option<Vec<i32>>> =
        const { std::cell::Cell::new(None) };
}

fn with_subpel_intermediate<R>(len: usize, f: impl FnOnce(&mut [i32]) -> R) -> R {
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
fn subpel_predict_block_internal<
    T: ReconSample,
    O: Clone + Default,
    F: FnMut(i32) -> O,
    const INTER_ROUND1: u32,
>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    finish: F,
) -> Result<Vec<O>> {
    let intermediate_height = validate_subpel_params(params)?;
    let output_len = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "subpel output sample count",
        })?;
    let mut output = vec![O::default(); output_len];
    subpel_predict_block_internal_into_validated::<_, _, _, INTER_ROUND1>(
        reference,
        params,
        intermediate_height,
        None,
        &mut output,
        params.w,
        finish,
    )?;
    Ok(output)
}

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

    let intermediate_height_i32 = (h as i32 - 1)
        .checked_mul(step_y)
        .and_then(|p| p.checked_add((1 << SCALE_SUBPEL_BITS) - 1))
        .map(|p| (p >> SCALE_SUBPEL_BITS) + 8)
        .filter(|&v| v > 0)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "subpel intermediate height",
        })?;
    let intermediate_height =
        usize::try_from(intermediate_height_i32).map_err(|_| ReconError::ArithmeticOverflow {
            context: "subpel intermediate height",
        })?;
    (w as i32 - 1)
        .checked_mul(step_x)
        .and_then(|m| start_x.checked_add(m))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "subpel horizontal coordinate",
        })?;

    Ok(intermediate_height)
}

#[allow(clippy::too_many_arguments)]
fn subpel_predict_block_internal_into_validated<
    T: ReconSample,
    O,
    F: FnMut(i32) -> O,
    const INTER_ROUND1: u32,
>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
    intermediate_height: usize,
    scratch: Option<&mut [i32]>,
    output: &mut [O],
    output_stride: usize,
    mut finish: F,
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
                2 * FILTER_BITS - (INTER_ROUND0 + INTER_ROUND1),
                output,
                output_stride,
                &mut finish,
            ),
            (false, true) => subpel_horizontal_only_into::<_, _, _, INTER_ROUND1>(
                reference,
                params,
                output,
                output_stride,
                &mut finish,
            ),
            (true, false) => subpel_vertical_only_into::<_, _, _, INTER_ROUND1>(
                reference,
                params,
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
    let mut run = |intermediate: &mut [i32]| {
        for r in read_lo..read_hi {
            let ref_row = ((start_y >> SCALE_SUBPEL_BITS) + r as i32 - 3).clamp(first_y, last_y);
            let ref_row = (ref_row as usize).min(reference.height - 1);
            let window = x_window_start.and_then(|window_start| {
                let row_base = ref_row * reference.stride + window_start;
                reference.samples.get(row_base..row_base + w + NUM_TAPS - 1)
            });
            if let Some(window) = window {
                let phase = ((start_x >> 6) & SUBPEL_MASK) as usize;
                let row_out = &mut intermediate[r * w..(r + 1) * w];
                if phase == 0 {
                    for (out, sample) in row_out.iter_mut().zip(&window[3..3 + w]) {
                        *out = i32::from(sample.to_u16()) << (FILTER_BITS - INTER_ROUND0);
                    }
                    continue;
                }
                let taps = &h_filter_rows[phase];
                for (out, win) in row_out.iter_mut().zip(window.windows(NUM_TAPS)) {
                    let Some(win) = win.first_chunk::<NUM_TAPS>() else {
                        continue;
                    };
                    let mut s = 0i32;
                    for (&tap, &sample) in taps.iter().zip(win) {
                        s += tap * i32::from(sample.to_u16());
                    }
                    *out = round2_i32(s, INTER_ROUND0);
                }
                continue;
            }
            for c in 0..w {
                let p = scaled_position(start_x, step_x, c);
                let phase = ((p >> 6) & SUBPEL_MASK) as usize;
                if phase == 0 {
                    let ref_col = (p >> SCALE_SUBPEL_BITS).clamp(first_x, last_x);
                    intermediate[r * w + c] =
                        reference.sample(ref_row, ref_col as usize) << (FILTER_BITS - INTER_ROUND0);
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
                intermediate[r * w + c] = round2_i32(s, INTER_ROUND0);
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
                    *out = finish(round2_i32(value << FILTER_BITS, INTER_ROUND1));
                }
                continue;
            }
            let rows = &intermediate[base * w..(base + NUM_TAPS) * w];
            let row_slices: [&[i32]; NUM_TAPS] =
                core::array::from_fn(|t| &rows[t * w..(t + 1) * w]);
            for (x, out) in output.iter_mut().enumerate() {
                let mut s = 0i32;
                for (&tap, row) in taps.iter().zip(&row_slices) {
                    s += tap * row[x];
                }
                *out = finish(round2_i32(s, INTER_ROUND1));
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
