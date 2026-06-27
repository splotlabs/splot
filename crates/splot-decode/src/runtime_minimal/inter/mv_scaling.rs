// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.17 motion-vector scaling + § 7.13.3.18 reference-clipping bounds
//! for the verified identity-scale (same-size reference) single-reference inter
//! block.
//!
//! Derives the `startX` / `startY` / `stepX` / `stepY` reference-block location
//! (in 1/1024-sample units) and the `firstX` / `firstY` / `lastX` / `lastY`
//! clipping region that [`splot_recon::subpel_predict_block`] consumes, from the
//! decoded motion vector (eighth-pel units, AV2 § 7.11), the block's plane
//! position, and the (unscaled) reference plane geometry.
//!
//! The reference is the same size as the current frame (the caller rejects any
//! scaled reference), so `xScale == yScale == 1 << REF_SCALE_SHIFT` and
//! `stepX == stepY == 1 << SCALE_SUBPEL_BITS == 1024` (one current-frame sample
//! per reference sample). The MV's fractional eighth-pel part survives into the
//! sub-pel phase of `startX` / `startY`.

use splot_recon::math::{clip3, round2_signed};

/// AV2 § 3 `REF_SCALE_SHIFT`.
const REF_SCALE_SHIFT: u32 = 14;
/// AV2 § 3 `SUBPEL_BITS`.
const SUBPEL_BITS: u32 = 4;
/// AV2 § 3 `SCALE_SUBPEL_BITS`.
const SCALE_SUBPEL_BITS: u32 = 10;
/// AV2 § 3 `MI_SIZE`.
const MI_SIZE: i64 = 4;

/// The § 7.13.3.17 / § 7.13.3.18 scaling + clipping result for one plane: the
/// inputs to [`splot_recon::SubpelPredictParams`] for the identity-scale block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct PlaneScaling {
    /// `startX` (1/1024-sample units).
    pub(in crate::runtime_minimal) start_x: i64,
    /// `startY` (1/1024-sample units).
    pub(in crate::runtime_minimal) start_y: i64,
    /// `stepX` (1/1024-sample units); `1024` for the identity scale.
    pub(in crate::runtime_minimal) step_x: i64,
    /// `stepY` (1/1024-sample units); `1024` for the identity scale.
    pub(in crate::runtime_minimal) step_y: i64,
    /// `firstX` clip bound.
    pub(in crate::runtime_minimal) first_x: i64,
    /// `firstY` clip bound.
    pub(in crate::runtime_minimal) first_y: i64,
    /// `lastX` clip bound.
    pub(in crate::runtime_minimal) last_x: i64,
    /// `lastY` clip bound.
    pub(in crate::runtime_minimal) last_y: i64,
}

/// Derives the § 7.13.3.17 scaling + § 7.13.3.18 clip bounds for a plane block at
/// luma position `(x, y)` (in luma samples) with motion vector `mv` (eighth-pel,
/// `mv = (row, col)`), over an unscaled reference of `ref_mi_cols` x `ref_mi_rows`
/// mode-info units. `sub_x` / `sub_y` are the plane subsampling (0 for luma).
///
/// The block's plane-space top-left is `(x >> sub_x, y >> sub_y)`. The committed
/// inter callers pass ordinary reference-frame motion at their current block
/// position; the ac0ej3 IntrABC frontier also reuses this identity-scale math for
/// `refIdx == -1` current-frame prediction at arbitrary luma positions.
#[allow(clippy::too_many_arguments)]
pub(in crate::runtime_minimal) fn derive_plane_scaling(
    plane_x: i64,
    plane_y: i64,
    mv_row: i64,
    mv_col: i64,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
    block_w: i64,
    block_h: i64,
) -> PlaneScaling {
    // §7.13.3.17 identity scale: xScale == yScale == 1 << REF_SCALE_SHIFT.
    let scale: i64 = 1 << REF_SCALE_SHIFT;
    let half_sample: i64 = 1 << (SUBPEL_BITS - 1); // 8 (1/16-sample units)

    // origX/origY: prescaled == 0, so mv is in 1/8-sample units. The current-plane
    // position is already in plane samples (plane_x/plane_y).
    let orig_x = (plane_x << SUBPEL_BITS) + ((2 * mv_col) >> sub_x) + half_sample;
    let orig_y = (plane_y << SUBPEL_BITS) + ((2 * mv_row) >> sub_y) + half_sample;

    let base_x = orig_x * scale - (half_sample << REF_SCALE_SHIFT);
    let base_y = orig_y * scale - (half_sample << REF_SCALE_SHIFT);

    // off = (1 << (SCALE_SUBPEL_BITS - SUBPEL_BITS)) / 2.
    let off: i64 = (1 << (SCALE_SUBPEL_BITS - SUBPEL_BITS)) / 2;
    let round_shift = REF_SCALE_SHIFT + SUBPEL_BITS - SCALE_SUBPEL_BITS; // 8
    let start_x = round2_signed(base_x, round_shift) + off;
    let start_y = round2_signed(base_y, round_shift) + off;

    let step_x = round2_signed(scale, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS); // 1024
    let step_y = round2_signed(scale, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS); // 1024

    // §7.13.3.18 clip bounds (useRefArea == 0; also used by IntrABC refIdx == -1):
    // lastX = ((RefMiCols * MI_SIZE) >> subX) - 1; firstX = firstY = 0.
    let last_x = ((ref_mi_cols * MI_SIZE) >> sub_x) - 1;
    let last_y = ((ref_mi_rows * MI_SIZE) >> sub_y) - 1;

    // The §7.13.3.18 convolution reads ref[Clip3(firstY, lastY, ...)][Clip3(firstX,
    // lastX, ...)], so firstX/firstY are 0 and the block-w/h only matter via the
    // kernel's own coordinate walk. Kept as explicit inputs for clarity / future
    // useRefArea support.
    let _ = (block_w, block_h);
    PlaneScaling {
        start_x,
        start_y,
        step_x,
        step_y,
        first_x: 0,
        first_y: 0,
        last_x: clip3(0, last_x, last_x),
        last_y: clip3(0, last_y, last_y),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn zero_mv_luma_is_full_pel_origin() {
        // Zero MV, luma (sub == 0), 64x64 (16 MI) reference: startX/startY land on a
        // full-pel sample with zero fractional phase, step == 1024.
        let s = derive_plane_scaling(0, 0, 0, 0, 0, 0, 16, 16, 64, 64);
        assert_eq!(s.step_x, 1024);
        assert_eq!(s.step_y, 1024);
        assert_eq!(s.last_x, 63);
        assert_eq!(s.last_y, 63);
        // A zero-MV start must have a zero sub-pel phase: (startX >> 6) & 15 == 0.
        assert_eq!((s.start_x >> 6) & 15, 0);
        assert_eq!((s.start_y >> 6) & 15, 0);
    }

    #[test]
    fn fractional_mv_produces_subpel_phase() {
        // A horizontal +4 eighth-pel (== half a luma sample) MV produces a non-zero
        // horizontal sub-pel phase but a zero vertical phase.
        let s = derive_plane_scaling(0, 0, 0, 4, 0, 0, 16, 16, 64, 64);
        assert_ne!((s.start_x >> 6) & 15, 0, "horizontal sub-pel phase set");
        assert_eq!((s.start_y >> 6) & 15, 0, "vertical phase zero");
    }

    #[test]
    fn chroma_420_halves_dimensions() {
        // 4:2:0 chroma over a 64x64 luma reference: lastX/lastY == 31.
        let s = derive_plane_scaling(0, 0, 0, 0, 1, 1, 16, 16, 32, 32);
        assert_eq!(s.last_x, 31);
        assert_eq!(s.last_y, 31);
    }
}
