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
mod block;
mod cell;
mod copy;
mod eight_bit;
mod fullpel_u8;
mod portable;
mod search_window;
mod sixteen_bit;
mod tip_overlap;
use block::Sink;
pub use cell::CompoundCells;
pub use copy::{
    blend_compound_average_equal, blend_compound_average_weighted,
    blend_compound_average_weighted_sample,
};
pub use fullpel_u8::{
    UnscaledPut, subpel_predict_block_compound_average_fullpel_strided_into_u8,
    subpel_predict_block_strided_into_u8, subpel_put_u8_block, subpel_put_u8_in_place,
    subpel_put_u8_pair_in_place,
};
pub use search_window::{
    refinemv_window_sads_u8, subpel_predict_bilinear_8x8_u8,
    subpel_predict_bilinear_search_window_u8,
};
pub use tip_overlap::{BilinearOverlapCarry, subpel_predict_12x12_bilinear_overlap_into};
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

/// AV2 § 6 interpolation filter for the inter prediction (`interp_filter`).
///
/// Only the three frame-level filter types are exposed; the 4-tap small-block
/// substitutions (indices 4 and 5) are an internal § 7.13.3.18 step, never a
/// caller-selectable filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
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

    /// The view of every row of a settled plane, `None` for a vacant one. A
    /// [`crate::Plane`] holds `stride * height` samples with `stride >= width`.
    #[inline]
    pub fn of_plane(plane: &'a crate::Plane<T>) -> Option<Self> {
        let size = plane.storage_size();
        (size.width() != 0 && size.height() != 0).then(|| Self {
            samples: plane.samples(),
            stride: plane.stride_samples(),
            width: size.width(),
            height: size.height(),
            readable_rows: size.height(),
        })
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

    /// The samples from row `row` on, rows `stride` apart, and the stride.
    pub(crate) fn rows_from(&self, row: usize) -> Option<(&'a [T], usize)> {
        Some((self.samples.get(row * self.stride..)?, self.stride))
    }

    /// This view of eight-bit storage as `u8` samples.
    pub(crate) fn as_u8(&self) -> Option<ReferencePlaneView<'a, u8>> {
        Some(ReferencePlaneView {
            samples: T::u8_slice(self.samples)?,
            stride: self.stride,
            width: self.width,
            height: self.height,
            readable_rows: self.readable_rows,
        })
    }

    /// This view of 16-bit storage as `u16` samples.
    pub(crate) fn as_u16(&self) -> Option<ReferencePlaneView<'a, u16>> {
        Some(ReferencePlaneView {
            samples: T::u16_slice(self.samples)?,
            stride: self.stride,
            width: self.width,
            height: self.height,
            readable_rows: self.readable_rows,
        })
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
    /// The block width in samples (`w`), `1..=256`.
    pub w: usize,
    /// The block height in samples (`h`), `1..=256`.
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
    /// The active bit depth, used by the final § 4.8 `Clip1`. Reference
    /// samples above its maximum give unspecified predictions, which can
    /// also be above the maximum.
    pub bit_depth: BitDepth,
}

impl SubpelPredictParams {
    /// `[firstX, firstY, lastX, lastY]`.
    const fn bounds(&self) -> [i32; 4] {
        [self.first_x, self.first_y, self.last_x, self.last_y]
    }
}

/// An unscaled block's § 7.13.3.18 1/1024-sample start `[x, y]` and clip
/// bounds `[firstX, firstY, lastX, lastY]`.
pub type CellPlacement = ([i32; 2], [i32; 4]);

/// The maximum supported block dimension (the AV2 256x256 super-block side).
const MAX_BLOCK_DIM: usize = 256;

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
/// 256-sample super-block side, [`ReconError::SubpelNegativeStep`] for a negative
/// step, [`ReconError::SubpelReferenceBoundsInvalid`] for an inverted clipping
/// range, and [`ReconError::ArithmeticOverflow`] if the intermediate height
/// cannot be derived.
pub fn subpel_predict_block<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Result<Vec<u16>> {
    validate_subpel_params(params)?;
    let mut output = vec![0; params.w * params.h];
    subpel_predict_block_strided_into(reference, params, &mut output, params.w)?;
    Ok(output)
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
    validate_subpel_params(params)?;
    validate_output_len(params, output.len(), output_stride)?;
    let sink = Sink::PutWide {
        output,
        stride: output_stride,
        max: params.bit_depth.max_sample(),
    };
    block::predict(reference, params, sink, None);
    Ok(())
}

/// Rejects a strided output shorter than the `w x h` prediction rectangle.
fn validate_output_len(
    params: &SubpelPredictParams,
    len: usize,
    output_stride: usize,
) -> Result<()> {
    let output_len = subpel_output_len(params, output_stride)?;
    if len < output_len {
        return Err(ReconError::BufferLengthMismatch {
            expected: output_len,
            actual: len,
        });
    }
    Ok(())
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
/// [`ReconError::SubpelNegativeStep`], [`ReconError::SubpelReferenceBoundsInvalid`],
/// and [`ReconError::ArithmeticOverflow`].
pub fn subpel_predict_block_compound_intermediate<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> Result<Vec<i32>> {
    validate_subpel_params(params)?;
    let mut output = vec![0; params.w * params.h];
    subpel_predict_block_compound_intermediate_into(
        reference,
        params,
        None,
        &mut output,
        params.w,
    )?;
    Ok(output)
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
    validate_subpel_params(params)?;
    validate_output_len(params, output.len(), output_stride)?;
    let sink = Sink::Prep {
        output,
        stride: output_stride,
    };
    block::predict(reference, params, sink, scratch);
    Ok(())
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

/// Validates the parameters, the first predictor and the strided output of
/// a compound-average prediction.
fn validate_compound_average(
    params: &SubpelPredictParams,
    pred0: &[i32],
    output_len: usize,
    output_stride: usize,
) -> Result<()> {
    validate_subpel_params(params)?;
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
    validate_output_len(params, output_len, output_stride)
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
    validate_compound_average(params, pred0, output.len(), output_stride)?;
    let sink = Sink::AverageWide {
        pred0,
        forward: i32::from(cwp_weight),
        output,
        stride: output_stride,
        max: params.bit_depth.max_sample(),
    };
    block::predict(reference, params, sink, scratch);
    Ok(())
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
    validate_compound_average(params, pred0, output.len(), output_stride)?;
    let sink = Sink::Average {
        pred0,
        forward: i32::from(cwp_weight),
        output,
        stride: output_stride,
    };
    block::predict(reference, params, sink, scratch);
    Ok(())
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

/// Validating entry to the compound-average fast dispatch for unscaled blocks.
///
/// Returns `Ok(false)` without writing when either reference is scaled, the
/// blocks differ in size or bit depth, or a clipping range is inverted; the
/// caller then uses the two-call path.
///
/// # Errors
/// Returns the parameter errors of [`subpel_predict_block`], a sample-type
/// error when `O` cannot hold the bit depth, and the output-layout errors of
/// [`subpel_predict_block_compound_average_strided_into`].
pub fn subpel_predict_block_compound_average_unscaled_strided_into<
    T: ReconSample,
    O: ReconSample,
>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [O],
    output_stride: usize,
) -> Result<bool> {
    crate::intra_dc_math::validate_sample_type::<O>(params0.bit_depth)?;
    validate_subpel_params(params0)?;
    validate_subpel_params(params1)?;
    subpel_predict_block_compound_average_fast_validated_strided_into(
        reference0,
        params0,
        reference1,
        params1,
        cwp_weight,
        &mut [],
        output,
        output_stride,
    )
}

/// Fast compound dispatch for unscaled blocks of one shape. Returns
/// `Ok(false)` without writing when a step is not unscaled, the two blocks
/// differ in size or bit depth, a size is outside `1..=MAX_BLOCK_DIM`, a
/// clipping range is inverted, the last column overflows, or `O` is neither
/// 8- nor 16-bit storage; the checked
/// two-call path then predicts it or reports the parameter error.
///
/// # Errors
/// Returns [`ReconError::SampleTypeUnsupportedBitDepth`] when `O` cannot hold
/// the bit depth, and [`ReconError::StrideTooSmall`] or
/// [`ReconError::BufferLengthMismatch`] when the output rectangle does not fit
/// the supplied storage.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn subpel_predict_block_compound_average_fast_validated_strided_into<
    T: ReconSample,
    O: ReconSample,
>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [O],
    output_stride: usize,
) -> Result<bool> {
    crate::intra_dc_math::validate_sample_type::<O>(params0.bit_depth)?;
    if !fast_admitted(params0, params1) {
        return Ok(false);
    }
    validate_compound_output(params0, output, output_stride)?;
    Ok(compound_average(
        [reference0, reference1],
        [params0, params1],
        cwp_weight,
        scratch,
        output,
        output_stride,
    ))
}

/// Predicts the first predictor of an admitted pair into retained storage,
/// then blends the second into `output`; `false`, having written nothing,
/// when `O` is neither 8- nor 16-bit storage.
fn compound_average<T: ReconSample, O: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    params: [&SubpelPredictParams; 2],
    cwp_weight: i16,
    scratch: &mut [i16],
    output: &mut [O],
    output_stride: usize,
) -> bool {
    let (w, h) = (params[0].w, params[0].h);
    let forward = i32::from(cwp_weight);
    if let (Some(first), Some(second)) = (references[0].as_u8(), references[1].as_u8())
        && let Some(output) = O::u8_slice_mut(output)
        && cell::compound([&first, &second], params, forward, output, output_stride)
    {
        return true;
    }
    if block::small_compound(references, params, forward, scratch, output, output_stride) {
        return true;
    }
    if O::u8_slice(&[]).is_none() && O::u16_slice(&[]).is_none() {
        return false;
    }
    with_compound_pred0(w * h, |pred0| {
        let sink = Sink::Prep {
            output: pred0,
            stride: w,
        };
        block::predict(references[0], params[0], sink, Some(&mut *scratch));
        let pred0 = &*pred0;
        let sink = if let Some(output) = O::u8_slice_mut(output) {
            Sink::Average {
                pred0,
                forward,
                output,
                stride: output_stride,
            }
        } else if let Some(output) = O::u16_slice_mut(output) {
            Sink::AverageWide {
                pred0,
                forward,
                output,
                stride: output_stride,
                max: params[0].bit_depth.max_sample(),
            }
        } else {
            return;
        };
        block::predict(references[1], params[1], sink, Some(scratch));
    });
    true
}

std::thread_local! {
    /// Retained storage of the first compound predictor of a fused pair.
    static COMPOUND_PRED0: std::cell::Cell<Vec<i32>> = const { std::cell::Cell::new(Vec::new()) };
}

fn with_compound_pred0<R>(len: usize, f: impl FnOnce(&mut [i32]) -> R) -> R {
    COMPOUND_PRED0.with(|slot| {
        let mut pred0 = slot.take();
        if pred0.len() < len {
            pred0.resize(len, 0);
        }
        let result = f(&mut pred0[..len]);
        slot.set(pred0);
        result
    })
}

/// The fast compound dispatch of
/// [`subpel_predict_block_compound_average_fast_validated_strided_into`] for
/// a cell of two planes that share its parameters (U and V): it writes the
/// cell of `output`, then of `second`, and returns the number of planes
/// written. It writes none when that function would decline, when the
/// `second` views differ from the first in width or readable rows, or when
/// `O` is neither 8- nor 16-bit storage; the caller predicts the rest.
///
/// # Errors
/// Returns the errors of
/// [`subpel_predict_block_compound_average_fast_validated_strided_into`] for
/// either output.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn subpel_predict_block_compound_average_fast_pair_into<T: ReconSample, O: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [O],
    output_stride: usize,
    (second, second_output, second_stride): ([&ReferencePlaneView<'_, T>; 2], &mut [O], usize),
) -> Result<usize> {
    crate::intra_dc_math::validate_sample_type::<O>(params0.bit_depth)?;
    let same = |a: &ReferencePlaneView<'_, T>, b: &ReferencePlaneView<'_, T>| {
        (a.width, a.readable_rows) == (b.width, b.readable_rows)
    };
    if !fast_admitted(params0, params1)
        || !same(reference0, second[0])
        || !same(reference1, second[1])
    {
        return Ok(0);
    }
    validate_compound_output(params0, output, output_stride)?;
    validate_compound_output(params0, second_output, second_stride)?;
    let params = [params0, params1];
    let first = compound_average(
        [reference0, reference1],
        params,
        cwp_weight,
        &mut [],
        output,
        output_stride,
    );
    let second = compound_average(
        second,
        params,
        cwp_weight,
        &mut [],
        second_output,
        second_stride,
    );
    Ok(usize::from(first) + usize::from(first && second))
}

/// The eight-bit COMPOUND_AVERAGE of one unscaled `size`-square cell, 4 or
/// 8, from each reference's [`CellPlacement`] into `output` rows `stride`
/// apart, as [`subpel_predict_block_compound_average_fast_validated_strided_into`]
/// stores it; `false` without writing for a cell outside the register-cell
/// subset or an output that cannot hold it.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn subpel_compound_cell_u8<T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    interp: InterpolationFilter,
    size: usize,
    placements: [CellPlacement; 2],
    cwp_weight: i16,
    output: &mut [u8],
    stride: usize,
) -> bool {
    let [Some(first), Some(second)] = references.map(ReferencePlaneView::as_u8) else {
        return false;
    };
    stride >= size
        && size
            .checked_sub(1)
            .is_some_and(|rows| rows * stride + size <= output.len())
        && cell::compound_at(
            [&first, &second],
            interp,
            size,
            placements,
            i32::from(cwp_weight),
            output,
            stride,
        )
}

/// Runs a [`CompoundCells`] program of eight-bit `size`-square cells into
/// `output` rows `stride` apart. Each cell that [`subpel_compound_cell_u8`]
/// would store from its position and placements is stored the same way and
/// sets its bit in the returned mask; every other cell is left unwritten for
/// the caller to predict.
#[doc(hidden)]
#[inline]
pub fn subpel_compound_cells_u8<T: ReconSample>(
    references: [&ReferencePlaneView<'_, T>; 2],
    interp: InterpolationFilter,
    size: usize,
    cells: &CompoundCells,
    cwp_weight: i16,
    output: &mut [u8],
    stride: usize,
) -> u64 {
    let [Some(first), Some(second)] = references.map(ReferencePlaneView::as_u8) else {
        return 0;
    };
    cell::compound_program(
        [&first, &second],
        interp,
        size,
        cells,
        i32::from(cwp_weight),
        output,
        stride,
    )
}

/// Whether both blocks have one unscaled shape in `1..=MAX_BLOCK_DIM`, one
/// bit depth, uninverted clipping ranges and a last column that fits.
#[allow(clippy::inline_always, reason = "measured TIP compound hot path")]
#[inline(always)]
fn fast_admitted(params0: &SubpelPredictParams, params1: &SubpelPredictParams) -> bool {
    let unscaled = |params: &SubpelPredictParams| {
        params.step_x == 1 << SCALE_SUBPEL_BITS
            && params.step_y == 1 << SCALE_SUBPEL_BITS
            && params
                .start_x
                .checked_add((params.w as i32 - 1) << SCALE_SUBPEL_BITS)
                .is_some()
    };
    let span = |params: &SubpelPredictParams| {
        (i64::from(params.last_x) - i64::from(params.first_x))
            | (i64::from(params.last_y) - i64::from(params.first_y))
    };
    span(params0) | span(params1) >= 0
        && (1..=MAX_BLOCK_DIM).contains(&params0.w)
        && (1..=MAX_BLOCK_DIM).contains(&params0.h)
        && (params0.w, params0.h, params0.bit_depth) == (params1.w, params1.h, params1.bit_depth)
        && unscaled(params0)
        && unscaled(params1)
}

/// Clamps the § 7.13.3.18 clipping bounds into the plane's readable area.
///
/// `Clip3(firstX, lastX, x)` followed by the plane clamp equals one clamp to
/// the bounded range, so the prediction does not change, and the kernels can
/// convert the clipped coordinates to indices directly. A view has at least
/// one column and one readable row, so `max` then `min` is that clamp
/// without the panic path of `i32::clamp`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn plane_bounded<T: ReconSample>(
    reference: &ReferencePlaneView<'_, T>,
    params: &SubpelPredictParams,
) -> SubpelPredictParams {
    let (last_column, last_row) = (
        reference.width as i32 - 1,
        reference.readable_rows as i32 - 1,
    );
    SubpelPredictParams {
        first_x: params.first_x.max(0).min(last_column),
        first_y: params.first_y.max(0).min(last_row),
        last_x: params.last_x.max(0).min(last_column),
        last_y: params.last_y.max(0).min(last_row),
        ..*params
    }
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

#[allow(clippy::inline_always, reason = "measured sub-pel hot path")]
#[inline(always)]
fn validate_subpel_params(params: &SubpelPredictParams) -> Result<usize> {
    let SubpelPredictParams {
        w,
        h,
        start_x,
        step_x,
        step_y,
        first_x,
        first_y,
        last_x,
        last_y,
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
    if (i64::from(last_x) - i64::from(first_x)) | (i64::from(last_y) - i64::from(first_y)) < 0 {
        return Err(ReconError::SubpelReferenceBoundsInvalid {
            first_x,
            first_y,
            last_x,
            last_y,
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

fn scaled_position(start: i32, step: i32, index: usize) -> i32 {
    start + step * index as i32
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
