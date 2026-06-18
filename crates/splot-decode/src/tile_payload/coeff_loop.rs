// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop foundation helpers.
//!
//! Feature tracking: `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE`.

use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::coeff_state::{TileCoeffContextState, TileCoeffStateError};

const LUMA_PLANE: usize = 0;
const V_PLANE: usize = 2;

/// Caller-resolved facts for luma § 8.3.2 `all_zero` context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaAllZeroContextInput {
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
    /// Whether the transform fills its plane residual block (`bw == w && bh == h`).
    pub(crate) tx_fills_block: bool,
    /// Whether `fsc_mode && enable_fsc` selects the final luma context.
    pub(crate) fsc_active: bool,
}

/// Caller-resolved facts for V-plane § 8.3.2 `all_zero` context derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VAllZeroContextInput {
    /// Transform-block x coordinate in chroma 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in chroma 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in chroma 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in chroma 4x4 units.
    pub(crate) h4: usize,
    /// Whether the chroma residual block is larger than the transform.
    pub(crate) chroma_block_larger_than_tx: bool,
    /// Whether the previously decoded U-plane EOB is nonzero.
    pub(crate) eob_u_nonzero: bool,
}

/// Error returned by coefficient-loop context handoff helpers.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffLoopContextError {
    /// The underlying coefficient context state rejected a plane or allocation fact.
    #[error("coefficient context state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Derives the luma § 8.3.2 `all_zero` (`txb_skip`) context from tile state.
///
/// The context formula is defined in § 8.3.2
/// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). This helper only
/// resolves the `AboveLevelContext[0]` / `LeftLevelContext[0]` OR reductions
/// from owned tile state; transform geometry and FSC facts stay caller-resolved
/// until broader § 5.20 transform-block syntax is wired.
pub(crate) fn luma_all_zero_context(
    state: &TileCoeffContextState,
    input: LumaAllZeroContextInput,
) -> Result<usize, CoeffLoopContextError> {
    let above = bounded_or_u32(state.above_level(LUMA_PLANE)?, input.x4, input.w4);
    let left = bounded_or_u32(state.left_level(LUMA_PLANE)?, input.y4, input.h4);
    Ok(txb_skip_ctx_luma(
        above,
        left,
        input.tx_fills_block,
        input.fsc_active,
    ))
}

/// Derives the V-plane § 8.3.2 `all_zero` (`v_txb_skip`) context from tile state.
///
/// The context formula is defined in § 8.3.2
/// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). This helper resolves
/// the V-plane level/DC above and left nonzero facts from owned tile state; the
/// chroma geometry and `EobU` facts remain caller-resolved until broader
/// § 5.20 transform-block syntax is wired.
pub(crate) fn v_all_zero_context(
    state: &TileCoeffContextState,
    input: VAllZeroContextInput,
) -> Result<usize, CoeffLoopContextError> {
    let above = bounded_or_level_dc(
        state.above_level(V_PLANE)?,
        state.above_dc(V_PLANE)?,
        input.x4,
        input.w4,
    );
    let left = bounded_or_level_dc(
        state.left_level(V_PLANE)?,
        state.left_dc(V_PLANE)?,
        input.y4,
        input.h4,
    );
    Ok(v_txb_skip_ctx(
        above != 0,
        left != 0,
        input.chroma_block_larger_than_tx,
        input.eob_u_nonzero,
    ))
}

fn bounded_or_u32(values: &[u32], start: usize, count: usize) -> u32 {
    let mut value = 0;
    if let Some(tail) = values.get(start..) {
        for entry in tail.iter().take(count) {
            value |= *entry;
        }
    }
    value
}

fn bounded_or_u8(values: &[u8], start: usize, count: usize) -> u32 {
    let mut value = 0;
    if let Some(tail) = values.get(start..) {
        for entry in tail.iter().take(count) {
            value |= u32::from(*entry);
        }
    }
    value
}

fn bounded_or_level_dc(level: &[u32], dc: &[u8], start: usize, count: usize) -> u32 {
    bounded_or_u32(level, start, count) | bounded_or_u8(dc, start, count)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
    use super::*;

    fn update(plane: usize, x4: usize, y4: usize, w4: usize, h4: usize) -> CoeffContextUpdate {
        CoeffContextUpdate {
            plane,
            x4,
            y4,
            w4,
            h4,
            cul_level: 4,
            dc_category: 2,
        }
    }

    #[test]
    fn luma_all_zero_context_reads_zero_state_for_first_block() {
        let state = TileCoeffContextState::new(16, 16).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 0,
                y4: 0,
                w4: 16,
                h4: 16,
                tx_fills_block: true,
                fsc_active: false,
            },
        )
        .unwrap();

        assert_eq!(ctx, 0);
    }

    #[test]
    fn luma_all_zero_context_reduces_state_lines_when_not_filling() {
        let mut state = TileCoeffContextState::new(8, 8).unwrap();
        state.update_after_coeffs(update(0, 2, 3, 2, 2)).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 1,
                y4: 2,
                w4: 4,
                h4: 4,
                tx_fills_block: false,
                fsc_active: false,
            },
        )
        .unwrap();

        assert_eq!(ctx, 5);
    }

    #[test]
    fn luma_all_zero_context_fsc_overrides_state() {
        let mut state = TileCoeffContextState::new(4, 4).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 4, 4)).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 0,
                y4: 0,
                w4: 4,
                h4: 4,
                tx_fills_block: true,
                fsc_active: true,
            },
        )
        .unwrap();

        assert_eq!(ctx, 9);
    }

    #[test]
    fn v_all_zero_context_combines_level_dc_state_and_geometry() {
        let mut state = TileCoeffContextState::new(8, 8).unwrap();
        state.update_after_coeffs(update(2, 2, 5, 2, 1)).unwrap();
        let ctx = v_all_zero_context(
            &state,
            VAllZeroContextInput {
                x4: 1,
                y4: 4,
                w4: 4,
                h4: 3,
                chroma_block_larger_than_tx: true,
                eob_u_nonzero: true,
            },
        )
        .unwrap();

        assert_eq!(ctx, 11);
    }

    #[test]
    fn all_zero_context_reductions_bound_out_of_range_and_pathological_counts() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(2, 0, 0, 1, 1)).unwrap();

        let luma = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: usize::MAX,
                y4: usize::MAX,
                w4: usize::MAX,
                h4: usize::MAX,
                tx_fills_block: false,
                fsc_active: false,
            },
        )
        .unwrap();
        let v = v_all_zero_context(
            &state,
            VAllZeroContextInput {
                x4: usize::MAX,
                y4: usize::MAX,
                w4: usize::MAX,
                h4: usize::MAX,
                chroma_block_larger_than_tx: false,
                eob_u_nonzero: false,
            },
        )
        .unwrap();

        assert_eq!(luma, 1);
        assert_eq!(v, 0);
    }
}
