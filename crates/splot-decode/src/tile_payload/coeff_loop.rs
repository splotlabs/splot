// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop foundation helpers.
//!
//! Feature tracking: `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE`,
//! `DECODE-COEFF-ALL-ZERO-BLOCK-STATE`.

use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::coeff_state::{
    CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};

const LUMA_PLANE: usize = 0;
const V_PLANE: usize = 2;
const COEFFS_PER_4X4: usize = 4;
const MAX_ADJUSTED_COEFF_EXTENT: usize = 32;

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

/// Caller-resolved facts for applying the § 5.20.7.27 all-zero coefficient path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllZeroCoeffBlockInput {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

/// Summary of a § 5.20.7.27 all-zero coefficient block state application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AllZeroCoeffBlock {
    eob: usize,
    cul_level: u32,
    dc_category: u8,
    block: TransformCoeffBlockState,
}

impl AllZeroCoeffBlock {
    /// End-of-block value returned by `coeffs()`.
    #[must_use]
    pub(crate) const fn eob(&self) -> usize {
        self.eob
    }

    /// `culLevel` written to level context lines.
    #[must_use]
    pub(crate) const fn cul_level(&self) -> u32 {
        self.cul_level
    }

    /// `dcCategory` written to DC context lines.
    #[must_use]
    pub(crate) const fn dc_category(&self) -> u8 {
        self.dc_category
    }

    /// Zero-initialized local transform coefficient state.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
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

/// Applies the AV2 § 5.20.7.27 `all_zero == 1` coefficient-block state effects.
///
/// The syntax initializes `Quant[]`, `QuantSign[]`, `Level[]`, sets `eob`,
/// `culLevel`, and `dcCategory` to zero, then writes those zero context values to
/// `AboveLevelContext` / `LeftLevelContext` and `AboveDcContext` /
/// `LeftDcContext` at the end of `coeffs()`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). This helper
/// models only that all-zero branch. Transform size, transform type, scan order,
/// nonzero EOB, `read_quant`, dequantization, and reconstruction stay deferred.
pub(crate) fn apply_all_zero_coeff_block(
    state: &mut TileCoeffContextState,
    input: AllZeroCoeffBlockInput,
) -> Result<AllZeroCoeffBlock, CoeffLoopContextError> {
    // TODO(spec: DECODE-COEFF-ALL-ZERO-BLOCK-STATE): Model the plane-0
    // `TxTypes[y4+j][x4+i] = DCT_DCT` writes and plane-1 `EobU` / `cctx_type`
    // reset when broader transform-block state is wired.
    let width = adjusted_coeff_extent(input.w4);
    let height = adjusted_coeff_extent(input.h4);
    let block = TransformCoeffBlockState::new(width, height)?;
    let cul_level = 0;
    let dc_category = 0;
    state.update_after_coeffs(CoeffContextUpdate {
        plane: input.plane,
        x4: input.x4,
        y4: input.y4,
        w4: input.w4,
        h4: input.h4,
        cul_level,
        dc_category,
    })?;
    Ok(AllZeroCoeffBlock {
        eob: 0,
        cul_level,
        dc_category,
        block,
    })
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

fn adjusted_coeff_extent(size4: usize) -> usize {
    size4
        .saturating_mul(COEFFS_PER_4X4)
        .min(MAX_ADJUSTED_COEFF_EXTENT)
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

    #[test]
    fn all_zero_coeff_block_applies_zero_state_and_context_writes() {
        let mut state = TileCoeffContextState::new(6, 6).unwrap();
        state.update_after_coeffs(update(0, 1, 2, 3, 2)).unwrap();

        let applied = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 1,
                y4: 2,
                w4: 3,
                h4: 2,
            },
        )
        .unwrap();

        assert_eq!(applied.eob(), 0);
        assert_eq!(applied.cul_level(), 0);
        assert_eq!(applied.dc_category(), 0);
        assert_eq!(applied.block().width(), 12);
        assert_eq!(applied.block().height(), 8);
        assert!(applied.block().level().iter().all(|level| *level == 0));
        assert!(applied.block().quant_sign().iter().all(|sign| *sign == 0));
        assert!(applied.block().quant().iter().all(|quant| *quant == 0));
        assert_eq!(state.above_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.above_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.left_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.left_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn all_zero_coeff_block_rejects_bad_ranges_without_mutation() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
        let before = state.clone();

        let err = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 1,
                y4: 0,
                w4: 2,
                h4: 1,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::State(TileCoeffStateError::ContextRangeOutOfBounds {
                context: "above",
                start: 1,
                end: 3,
                len: 2
            })
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn all_zero_coeff_block_rejects_zero_transform_extent_before_mutation() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
        let before = state.clone();

        let err = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 0,
                y4: 0,
                w4: 0,
                h4: 1,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::State(TileCoeffStateError::InvalidAdjustedTransformExtent {
                axis: "width",
                value: 0
            })
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn all_zero_coeff_block_saturates_adjusted_extent_to_spec_cap() {
        let mut state = TileCoeffContextState::new(16, 16).unwrap();

        let applied = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 2,
                x4: 0,
                y4: 0,
                w4: 16,
                h4: 16,
            },
        )
        .unwrap();

        assert_eq!(applied.block().width(), 32);
        assert_eq!(applied.block().height(), 32);
        assert_eq!(applied.block().quant().len(), 1024);
    }
}
