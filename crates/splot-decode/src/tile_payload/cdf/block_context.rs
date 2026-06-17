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
//! Scope: currently the `y_mode_index` context only, for the single-block
//! tile-origin case where both `get_joint_mode` neighbours are out of frame. The
//! `uv_mode`, `txb_skip`, and other block-symbol contexts, and the in-frame
//! `get_joint_mode` neighbour lookup, are derived by future increments.

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

#[cfg(test)]
mod tests {
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
}
