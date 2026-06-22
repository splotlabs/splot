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
use crate::format::BitDepth;

/// AV2 § 3 `SCALE_SUBPEL_BITS`: number of fractional bits in the 1/1024-sample
/// reference coordinates (`startX` / `startY` / `stepX` / `stepY` units).
const SCALE_SUBPEL_BITS: u32 = 10;

/// AV2 § 3 `SUBPEL_BITS`: number of fractional bits when choosing a filter tap.
const SUBPEL_BITS: u32 = 4;

/// AV2 § 3 `SUBPEL_MASK = (1 << SUBPEL_BITS) - 1`: the 16-phase sub-pel mask used
/// to index the inner `Subpel_Filters[...][phase]` dimension.
const SUBPEL_MASK: i64 = (1 << SUBPEL_BITS) - 1;

/// AV2 § 7.13.3.16 `InterRound0`: the down-shift after the horizontal filter
/// pass. Fixed at 3.
const INTER_ROUND0: u32 = 3;

/// AV2 § 7.13.3.16 `InterRound1`: the down-shift after the vertical filter pass
/// for the non-compound (`isCompound == 0`) prediction this kernel produces.
const INTER_ROUND1_NON_COMPOUND: u32 = 11;

/// AV2 § 6 `EIGHTTAP` interpolation filter index.
const EIGHTTAP: u8 = 0;
/// AV2 § 6 `EIGHTTAP_SMOOTH` interpolation filter index.
const EIGHTTAP_SMOOTH: u8 = 1;
/// AV2 § 6 `EIGHTTAP_SHARP` interpolation filter index.
const EIGHTTAP_SHARP: u8 = 2;

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
    // 0: EIGHTTAP
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
    // 1: EIGHTTAP_SMOOTH
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
    // 2: EIGHTTAP_SHARP
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
    // 3: bilinear (the spec's index-3 filter; two non-zero taps at the centre)
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
    // 4: 4-tap EIGHTTAP (small-block substitution)
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
    // 5: 4-tap EIGHTTAP_SMOOTH (small-block substitution)
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterpolationFilter {
    /// `EIGHTTAP` (`interp_filter == 0`).
    EightTap,
    /// `EIGHTTAP_SMOOTH` (`interp_filter == 1`).
    EightTapSmooth,
    /// `EIGHTTAP_SHARP` (`interp_filter == 2`).
    EightTapSharp,
}

impl InterpolationFilter {
    /// The base `Subpel_Filters` index for this filter (`EIGHTTAP == 0`,
    /// `EIGHTTAP_SMOOTH == 1`, `EIGHTTAP_SHARP == 2`).
    const fn base_index(self) -> u8 {
        match self {
            Self::EightTap => EIGHTTAP,
            Self::EightTapSmooth => EIGHTTAP_SMOOTH,
            Self::EightTapSharp => EIGHTTAP_SHARP,
        }
    }

    /// Applies the AV2 § 7.13.3.18 small-block (`dim <= 4`) substitution: an
    /// `EIGHTTAP` / `EIGHTTAP_SHARP` filter maps to the 4-tap index 4, and
    /// `EIGHTTAP_SMOOTH` maps to the 4-tap index 5.
    const fn pass_index(self, dim: u32) -> u8 {
        if dim <= SMALL_BLOCK_DIM {
            match self {
                Self::EightTap | Self::EightTapSharp => SMALL_BLOCK_EIGHTTAP,
                Self::EightTapSmooth => SMALL_BLOCK_EIGHTTAP_SMOOTH,
            }
        } else {
            self.base_index()
        }
    }
}

/// A reference-plane sample view for the AV2 § 7.13.3.18 convolution.
///
/// The plane is a row-major sample buffer of `width * height` samples; the
/// kernel reads `ref[Clip3(firstY, lastY, refY)][Clip3(firstX, lastX, refX)]`,
/// so the clipping bounds (a § 7.13.3.18 input) implement the reference-border
/// extension without the caller copying a padded plane.
#[derive(Clone, Copy, Debug)]
pub struct ReferencePlaneView<'a> {
    samples: &'a [u16],
    width: usize,
    height: usize,
}

impl<'a> ReferencePlaneView<'a> {
    /// Builds a reference-plane view over a row-major `width * height` sample
    /// buffer.
    ///
    /// Returns [`ReconError::SubpelReferencePlaneMismatch`] when `samples.len()`
    /// is not exactly `width * height`, or [`ReconError::ZeroDimension`] when a
    /// dimension is zero.
    pub fn new(samples: &'a [u16], width: usize, height: usize) -> Result<Self> {
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
        Ok(Self {
            samples,
            width,
            height,
        })
    }

    /// Reads `ref[plane][row][col]`. The convolution clips the requested indices
    /// to the caller-supplied `[firstX, lastX] x [firstY, lastY]` region; this
    /// read additionally clamps to the view's own `width`/`height` so it is total
    /// even if a caller passes a clipping region wider than the actual plane
    /// (defense in depth — the function never indexes out of bounds).
    fn sample(&self, row: usize, col: usize) -> i64 {
        let row = row.min(self.height - 1);
        let col = col.min(self.width - 1);
        // `width` and `height` are non-zero (validated in `new`) and
        // `samples.len() == width * height`, so this index is always in bounds.
        i64::from(self.samples[row * self.width + col])
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
    pub start_x: i64,
    /// The § 7.13.3.17 `startY`: the reference block top edge in 1/1024-sample
    /// units (the `y` input to § 7.13.3.18).
    pub start_y: i64,
    /// The § 7.13.3.17 `stepX`: the horizontal step in 1/1024-sample units
    /// (the `xStep` input to § 7.13.3.18).
    pub step_x: i64,
    /// The § 7.13.3.17 `stepY`: the vertical step in 1/1024-sample units
    /// (the `yStep` input to § 7.13.3.18).
    pub step_y: i64,
    /// The § 7.13.3.18 reference-clipping region left bound (`firstX`).
    pub first_x: i64,
    /// The § 7.13.3.18 reference-clipping region top bound (`firstY`).
    pub first_y: i64,
    /// The § 7.13.3.18 reference-clipping region right bound (`lastX`).
    pub last_x: i64,
    /// The § 7.13.3.18 reference-clipping region bottom bound (`lastY`).
    pub last_y: i64,
    /// The active bit depth, used by the final § 4.8 `Clip1`.
    pub bit_depth: BitDepth,
}

/// The maximum supported block dimension (AV2 super-block transform block side).
const MAX_BLOCK_DIM: usize = 128;

/// AV2 § 4.8 `Round2(x, n)`, computed in `i64`. `Round2(x, 0) == x`; otherwise
/// `(x + (1 << (n - 1))) >> n` with an arithmetic right shift (the filter sum can
/// be negative).
const fn round2(x: i64, n: u32) -> i64 {
    if n == 0 { x } else { (x + (1 << (n - 1))) >> n }
}

/// AV2 § 4.8 `Clip3(low, high, value)`.
const fn clip3(low: i64, high: i64, value: i64) -> i64 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

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
/// Returns [`ReconError::ZeroDimension`] for a zero dimension,
/// [`ReconError::SubpelBlockDimensionUnsupported`] for `w`/`h` above the
/// 128-sample super-block side, [`ReconError::SubpelNegativeStep`] for a negative
/// step, and [`ReconError::ArithmeticOverflow`] if the intermediate height cannot
/// be derived.
pub fn subpel_predict_block(
    reference: &ReferencePlaneView<'_>,
    params: &SubpelPredictParams,
) -> Result<Vec<u16>> {
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
        bit_depth,
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
    // The two-pass convolution and the intermediateHeight derivation assume
    // non-negative steps (the §7.13.3.17 scaling factors are non-negative). A
    // negative step would make the vertical-pass `base` index negative; reject it
    // rather than risk an out-of-bounds intermediate read.
    if step_x < 0 || step_y < 0 {
        return Err(ReconError::SubpelNegativeStep { step_x, step_y });
    }

    // §7.13.3.18 intermediateHeight =
    //   (((h - 1) * yStep + (1 << SCALE_SUBPEL_BITS) - 1) >> SCALE_SUBPEL_BITS) + 8
    let h_i64 = h as i64;
    let intermediate_height_i64 =
        (((h_i64 - 1) * step_y + (1 << SCALE_SUBPEL_BITS) - 1) >> SCALE_SUBPEL_BITS) + 8;
    if intermediate_height_i64 <= 0 {
        return Err(ReconError::ArithmeticOverflow {
            context: "subpel intermediate height",
        });
    }
    let intermediate_height =
        usize::try_from(intermediate_height_i64).map_err(|_| ReconError::ArithmeticOverflow {
            context: "subpel intermediate height",
        })?;

    let max_sample = i64::from(bit_depth.max_sample());

    // Horizontal pass: §7.13.3.18 small-block substitution keys on w.
    let h_filter = interp.pass_index(w as u32);
    let h_filter_rows = &SUBPEL_FILTERS[h_filter as usize];

    let mut intermediate = vec![0i32; intermediate_height * w];
    for r in 0..intermediate_height {
        let ref_row = clip3(
            first_y,
            last_y,
            (start_y >> SCALE_SUBPEL_BITS) + r as i64 - 3,
        );
        let ref_row = ref_row as usize;
        for c in 0..w {
            let p = start_x + step_x * c as i64;
            let phase = ((p >> 6) & SUBPEL_MASK) as usize;
            let taps = &h_filter_rows[phase];
            let mut s: i64 = 0;
            for (t, &tap) in taps.iter().enumerate() {
                let ref_col = clip3(first_x, last_x, (p >> SCALE_SUBPEL_BITS) + t as i64 - 3);
                s += i64::from(tap) * reference.sample(ref_row, ref_col as usize);
            }
            // The §7.13.3.18 NOTE: the InterRound0 shift keeps this in 16 bits.
            intermediate[r * w + c] = round2(s, INTER_ROUND0) as i32;
        }
    }

    // Vertical pass: §7.13.3.18 small-block substitution keys on h.
    let v_filter = interp.pass_index(h as u32);
    let v_filter_rows = &SUBPEL_FILTERS[v_filter as usize];

    let mut output = vec![0u16; w * h];
    for r in 0..h {
        let p = (start_y & 1023) + step_y * r as i64;
        let phase = ((p >> 6) & SUBPEL_MASK) as usize;
        let taps = &v_filter_rows[phase];
        // `p >= 0` (step_y >= 0 and `start_y & 1023` in 0..=1023), so base >= 0.
        // The §7.13.3.18 intermediateHeight derivation guarantees
        // `base + NUM_TAPS - 1 < intermediate_height`; the explicit check keeps the
        // function panic-free for any caller step/start combination.
        let base = (p >> SCALE_SUBPEL_BITS) as usize;
        if base + NUM_TAPS > intermediate_height {
            return Err(ReconError::SubpelIntermediateOutOfRange {
                base,
                intermediate_height,
            });
        }
        for c in 0..w {
            let mut s: i64 = 0;
            for (t, &tap) in taps.iter().enumerate() {
                // §7.13.3.18 vertical pass reads intermediate[(p >> 10) + t][c].
                let row = base + t;
                s += i64::from(tap) * i64::from(intermediate[row * w + c]);
            }
            let pred = round2(s, INTER_ROUND1_NON_COMPOUND);
            // §7.13.3 single-reference write: CurrFrame = Clip1(Preds[0][i][j]).
            output[r * w + c] = clip3(0, max_sample, pred) as u16;
        }
    }

    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
