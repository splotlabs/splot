// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 block-symbol CDF context derivation.
//!
//! This module derives the per-symbol `ctx` index that selects a block-symbol
//! CDF row in the § 8.3.2 Cdf selection process
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`), replacing the
//! hardcoded context literals in the minimal flat-intra block-symbol trace with
//! spec-grounded derivations.
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.
//!
//! Scope: the `y_mode_index` context (single-block tile-origin, out-of-frame
//! `get_joint_mode` neighbours), the `uv_mode` context (from the reconstructed
//! luma `YMode`, for the non-directional Y-mode subset), and the `all_zero`
//! (`txb_skip` / `v_txb_skip`) context formula (the luma and V-plane § 8.3.2
//! arithmetic over caller-supplied level context and transform-block geometry).
//! The in-frame `get_joint_mode` neighbour lookup, the directional / escape /
//! second-mode `YMode` reconstruction paths, the U-plane `txb_skip` branch, and
//! the actual level-context buffers and transform-block geometry that feed the
//! `all_zero` formula (which need the § 5.20 transform-block syntax) are derived
//! by future increments.

/// AV2 § 3 `NON_DIRECTIONAL_MODES_COUNT`: the number of non-directional intra
/// modes (intra modes `0..5`); a mode value at or above this is directional.
const NON_DIRECTIONAL_MODES_COUNT: usize = 5;

/// AV2 `DC_PRED` intra mode value (intra mode `0`); also the value
/// `get_joint_mode` returns for an out-of-frame neighbour (§ 5 `get_joint_mode`).
const DC_PRED: usize = 0;

/// AV2 § 8.3.2 `y_mode_index` (and `y_mode_offset`) CDF context derivation.
///
/// The context is
/// `ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) + (get_joint_mode(1)
/// >= NON_DIRECTIONAL_MODES_COUNT)` (§ 8.3.2), where `get_joint_mode(dir)` reads
/// the directional joint mode of the left (`dir == 0`) or above (`dir == 1`)
/// neighbour, or returns `DC_PRED` when that neighbour is out of frame (§ 5
/// `get_joint_mode`). The resulting context is in `0..=2`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeIndexContext {
    /// `get_joint_mode(0)` — the left neighbour's joint intra mode.
    left_joint_mode: usize,
    /// `get_joint_mode(1)` — the above neighbour's joint intra mode.
    above_joint_mode: usize,
}

impl YModeIndexContext {
    /// The single-block tile-origin case: the block at `MiRow == 0`,
    /// `MiCol == 0`, whose left (`MiCol - 1`) and above (`MiRow - 1`) joint-mode
    /// neighbours are both out of frame, so `get_joint_mode` returns `DC_PRED`
    /// for each (§ 5 `get_joint_mode` / § 8.3.2).
    //
    // TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): in-frame neighbours read
    // `IntraJointModes[mvRow][mvCol]`; the minimal flat-intra frontier tracks no
    // neighbour mode state yet, so only the out-of-frame `DC_PRED` branch is
    // modelled here.
    pub(crate) const fn tile_origin_block() -> Self {
        Self {
            left_joint_mode: DC_PRED,
            above_joint_mode: DC_PRED,
        }
    }

    /// The § 8.3.2 `y_mode_index` context, in `0..=2`.
    pub(crate) const fn ctx(self) -> usize {
        (self.left_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
            + (self.above_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
    }
}

/// AV2 intra luma prediction mode value, in the canonical `Mode_To_Txfm`
/// ordering (§ 9.2): `DC_PRED == 0`, the directional modes `V_PRED..=D67_PRED`
/// are `1..=8`, and the remaining non-directional modes (`SMOOTH_PRED`,
/// `SMOOTH_V_PRED`, `SMOOTH_H_PRED`, `PAETH_PRED`) are `9..=12`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraYMode(u8);

impl IntraYMode {
    /// `DC_PRED` (intra mode 0).
    pub(crate) const DC_PRED: Self = Self(0);

    /// First directional mode value (`V_PRED`), the lower `is_directional_mode`
    /// bound (§ 5 `is_directional_mode`).
    const V_PRED: u8 = 1;
    /// Last directional mode value (`D67_PRED`), the upper `is_directional_mode`
    /// bound (§ 5 `is_directional_mode`).
    const D67_PRED: u8 = 8;

    /// AV2 § 9.2 `SMOOTH_V_PRED` canonical luma mode value.
    const SMOOTH_V_PRED: u8 = 10;
    /// AV2 § 9.2 `SMOOTH_H_PRED` canonical luma mode value.
    const SMOOTH_H_PRED: u8 = 11;

    /// AV2 § 5 `is_directional_mode(mode)`: true when `V_PRED <= mode <= D67_PRED`.
    pub(crate) const fn is_directional(self) -> bool {
        self.0 >= Self::V_PRED && self.0 <= Self::D67_PRED
    }

    /// Maps this luma mode to the non-DC predictor the general intra decode
    /// currently reconstructs (§ 7.13.2.13 smooth prediction), or `None` for
    /// `DC_PRED` and the not-yet-supported non-DC modes (`SMOOTH_PRED`,
    /// `PAETH_PRED`, and the directional modes, which lack oracle fixtures).
    pub(crate) const fn supported_nondc(self) -> Option<SupportedNonDcLumaMode> {
        match self.0 {
            Self::SMOOTH_V_PRED => Some(SupportedNonDcLumaMode::SmoothVertical),
            Self::SMOOTH_H_PRED => Some(SupportedNonDcLumaMode::SmoothHorizontal),
            _ => None,
        }
    }
}

/// The non-DC non-directional luma intra modes the general intra decode can
/// reconstruct today — a strict subset of the § 9.2 modes, gated to those proven
/// bit-exact against the AVM/dav2d oracle. `SMOOTH_PRED` and `PAETH_PRED` remain
/// deferred until they have single-block oracle fixtures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedNonDcLumaMode {
    /// AV2 `SMOOTH_V_PRED` (§ 7.13.2.13 vertical smooth prediction).
    SmoothVertical,
    /// AV2 `SMOOTH_H_PRED` (§ 7.13.2.13 horizontal smooth prediction).
    SmoothHorizontal,
}

/// AV2 § 5 `Reordered_Y_Mode[0..NON_DIRECTIONAL_MODES_COUNT]`: the five
/// non-directional modes in reorder order — `DC_PRED`, `SMOOTH_PRED`,
/// `SMOOTH_V_PRED`, `SMOOTH_H_PRED`, `PAETH_PRED` (canonical values 0, 9, 10, 11,
/// 12).
const REORDERED_Y_MODE_NON_DIRECTIONAL: [IntraYMode; NON_DIRECTIONAL_MODES_COUNT] = [
    IntraYMode(0),
    IntraYMode(9),
    IntraYMode(10),
    IntraYMode(11),
    IntraYMode(12),
];

/// Reconstructs the typed luma `YMode` from the decoded `y_mode_set` and
/// `y_mode_index` for the supported minimal subset (§ 5 `intra_y_mode_info`,
/// `get_intra_y_mode_set`, and `Reordered_Y_Mode`).
///
/// Supported subset: `y_mode_set == 0` with a non-directional `y_mode_index`
/// (`0..NON_DIRECTIONAL_MODES_COUNT`). Then `modeIdx == y_mode_index` (the
/// `MODE_INDEX_COUNT - 1 == 7` escape never applies for these indices),
/// `get_intra_y_mode_set` passes `modeIdx` through unchanged (it is below
/// `NON_DIRECTIONAL_MODES_COUNT`), and `YMode == Reordered_Y_Mode[y_mode_index]`.
/// Returns `None` for inputs outside this subset.
//
// TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): the directional reordering
// (`modeDelta >= NON_DIRECTIONAL_MODES_COUNT`), the `y_mode_offset` escape at
// `MODE_INDEX_COUNT - 1`, and the `y_mode_set != 0` / `y_second_mode` path are
// not yet modelled.
pub(crate) fn reconstruct_minimal_y_mode(y_mode_set: u8, y_mode_index: u8) -> Option<IntraYMode> {
    if y_mode_set != 0 {
        return None;
    }
    REORDERED_Y_MODE_NON_DIRECTIONAL
        .get(usize::from(y_mode_index))
        .copied()
}

/// AV2 § 8.3.2 `uv_mode` (`TileUVModeCflNotAllowedCdf[ctx]`) context: `ctx`
/// equals `is_directional_mode(YMode)`, i.e. 1 when the reconstructed luma mode
/// is directional and 0 otherwise.
pub(crate) const fn uv_mode_ctx(y_mode: IntraYMode) -> usize {
    y_mode.is_directional() as usize
}

/// AV2 § 3 `TXB_SKIP_CONTEXTS`: the number of luma/U `all_zero` (txb_skip)
/// contexts; the `fsc_mode` luma branch selects the last one.
const TXB_SKIP_CONTEXTS: usize = 10;

/// AV2 § 8.3.2 luma `all_zero` (txb_skip) CDF context (`plane == 0`), selecting
/// `TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]`.
///
/// `above_level_or` / `left_level_or` are the OR-reductions of
/// `AboveLevelContext[0]` / `LeftLevelContext[0]` over the transform block's
/// in-frame 4x4 columns / rows (the caller's bounded reduction); this function
/// applies the § 8.3.2 `Min(·, 4)`. `tx_fills_block` is `bw == w && bh == h`
/// (the transform fills its plane residual block), and `fsc_active` is
/// `fsc_mode && enable_fsc`.
pub(crate) const fn txb_skip_ctx_luma(
    above_level_or: u32,
    left_level_or: u32,
    tx_fills_block: bool,
    fsc_active: bool,
) -> usize {
    if fsc_active {
        TXB_SKIP_CONTEXTS - 1
    } else if tx_fills_block {
        0
    } else {
        let top = if above_level_or < 4 {
            above_level_or
        } else {
            4
        };
        let left = if left_level_or < 4 { left_level_or } else { 4 };
        ((top + left + 3) >> 1) as usize
    }
}

/// AV2 § 8.3.2 V-plane `all_zero` (v_txb_skip) CDF context (`plane == 2`),
/// selecting `TileVTxbSkipCdf[ctx]`.
///
/// `above_nonzero` / `left_nonzero` are whether the OR of `AboveLevelContext[2]`
/// with `AboveDcContext[2]` (resp. the left arrays) over the in-frame 4x4
/// columns / rows is non-zero. `chroma_block_larger_than_tx` is `bw * bh > w * h`
/// and `eob_u_nonzero` is `EobU != 0`. (The `plane == 1` U-plane `+6` branch is
/// not modelled — the minimal trace has no U `all_zero` symbol.)
pub(crate) const fn v_txb_skip_ctx(
    above_nonzero: bool,
    left_nonzero: bool,
    chroma_block_larger_than_tx: bool,
    eob_u_nonzero: bool,
) -> usize {
    let mut ctx = above_nonzero as usize + left_nonzero as usize;
    if chroma_block_larger_than_tx {
        ctx += 3;
    }
    if eob_u_nonzero {
        ctx += 6;
    }
    ctx
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn tile_origin_block_is_dc_pred_context_zero() {
        // Both neighbours out of frame -> DC_PRED (0), which is non-directional
        // (0 < NON_DIRECTIONAL_MODES_COUNT), so each term is 0 and ctx == 0. This
        // matches the literal the minimal flat-intra trace previously hardcoded.
        assert_eq!(YModeIndexContext::tile_origin_block().ctx(), 0);
    }

    #[test]
    fn directional_neighbours_raise_the_context() {
        // A directional joint mode (>= NON_DIRECTIONAL_MODES_COUNT) on one side
        // gives ctx 1; on both sides ctx 2 (the § 8.3.2 sum of two indicators).
        let one = YModeIndexContext {
            left_joint_mode: NON_DIRECTIONAL_MODES_COUNT,
            above_joint_mode: DC_PRED,
        };
        assert_eq!(one.ctx(), 1);
        let both = YModeIndexContext {
            left_joint_mode: NON_DIRECTIONAL_MODES_COUNT,
            above_joint_mode: NON_DIRECTIONAL_MODES_COUNT + 7,
        };
        assert_eq!(both.ctx(), 2);
    }

    #[test]
    fn last_non_directional_mode_does_not_raise_the_context() {
        // The boundary: mode NON_DIRECTIONAL_MODES_COUNT - 1 is still
        // non-directional, so it contributes 0.
        let ctx = YModeIndexContext {
            left_joint_mode: NON_DIRECTIONAL_MODES_COUNT - 1,
            above_joint_mode: NON_DIRECTIONAL_MODES_COUNT - 1,
        };
        assert_eq!(ctx.ctx(), 0);
    }

    #[test]
    fn minimal_y_mode_reconstruction_maps_set0_index0_to_dc_pred() {
        // The minimal flat-intra trace decodes y_mode_set == 0, y_mode_index == 0,
        // so YMode == Reordered_Y_Mode[0] == DC_PRED.
        assert_eq!(reconstruct_minimal_y_mode(0, 0), Some(IntraYMode::DC_PRED));
    }

    #[test]
    fn minimal_y_mode_reconstruction_covers_the_non_directional_subset() {
        // y_mode_set == 0 with a non-directional index maps to the reorder prefix;
        // every reconstructed mode is non-directional.
        for index in 0..NON_DIRECTIONAL_MODES_COUNT {
            let mode = reconstruct_minimal_y_mode(0, index as u8)
                .expect("non-directional index is supported");
            assert!(
                !mode.is_directional(),
                "index {index} must be non-directional"
            );
        }
    }

    #[test]
    fn minimal_y_mode_reconstruction_rejects_unsupported_inputs() {
        // A non-zero set and a directional/escape index are outside the supported
        // subset and return None (deferred to a future increment).
        assert_eq!(reconstruct_minimal_y_mode(1, 0), None);
        assert_eq!(
            reconstruct_minimal_y_mode(0, NON_DIRECTIONAL_MODES_COUNT as u8),
            None
        );
    }

    #[test]
    fn uv_mode_ctx_is_zero_for_dc_pred_and_one_for_directional() {
        // is_directional_mode(DC_PRED) == false -> ctx 0 (matches the literal the
        // trace previously hardcoded); a directional mode -> ctx 1.
        assert_eq!(uv_mode_ctx(IntraYMode::DC_PRED), 0);
        assert_eq!(uv_mode_ctx(IntraYMode(IntraYMode::V_PRED)), 1);
        assert_eq!(uv_mode_ctx(IntraYMode(IntraYMode::D67_PRED)), 1);
        // A non-directional mode above the directional range (PAETH_PRED) -> 0.
        assert_eq!(uv_mode_ctx(IntraYMode(12)), 0);
    }

    #[test]
    fn luma_txb_skip_ctx_first_block_filling_transform_is_zero() {
        // The minimal trace's first luma transform block: zero level context
        // (first block, out-of-frame neighbours) and a transform that fills its
        // residual block -> the `bw == w && bh == h` branch -> ctx 0, matching
        // the value the conformant fixture forces.
        assert_eq!(txb_skip_ctx_luma(0, 0, true, false), 0);
    }

    #[test]
    fn luma_txb_skip_ctx_uses_min_clamped_level_sum_when_not_filling() {
        // When the transform does not fill the block, ctx = (Min(top,4) +
        // Min(left,4) + 3) >> 1. Zero context -> (0+0+3)>>1 = 1; saturated
        // context -> (4+4+3)>>1 = 5 (the Min(.,4) clamp caps each term).
        assert_eq!(txb_skip_ctx_luma(0, 0, false, false), 1);
        assert_eq!(txb_skip_ctx_luma(9, 9, false, false), 5);
        assert_eq!(txb_skip_ctx_luma(1, 2, false, false), 3);
    }

    #[test]
    fn luma_txb_skip_ctx_fsc_selects_last_context() {
        // fsc_mode && enable_fsc -> ctx = TXB_SKIP_CONTEXTS - 1, overriding the
        // fill/level branches.
        assert_eq!(txb_skip_ctx_luma(0, 0, true, true), TXB_SKIP_CONTEXTS - 1);
        assert_eq!(txb_skip_ctx_luma(3, 3, false, true), TXB_SKIP_CONTEXTS - 1);
    }

    #[test]
    fn v_txb_skip_ctx_first_block_larger_chroma_is_three() {
        // The minimal trace's V transform block: zero level/DC context (first
        // block), a chroma residual block larger than the transform (+3), and
        // EobU == 0 (the U plane decoded all-zero) -> ctx 3, matching the value
        // the conformant fixture forces.
        assert_eq!(v_txb_skip_ctx(false, false, true, false), 3);
    }

    #[test]
    fn v_txb_skip_ctx_adds_neighbour_chroma_and_eob_contributions() {
        // ctx = (above != 0) + (left != 0), then +3 if the chroma block exceeds
        // the transform and +6 if EobU != 0.
        assert_eq!(v_txb_skip_ctx(false, false, false, false), 0);
        assert_eq!(v_txb_skip_ctx(true, false, false, false), 1);
        assert_eq!(v_txb_skip_ctx(true, true, false, false), 2);
        assert_eq!(v_txb_skip_ctx(true, true, true, false), 5);
        assert_eq!(v_txb_skip_ctx(true, true, true, true), 11);
    }
}
