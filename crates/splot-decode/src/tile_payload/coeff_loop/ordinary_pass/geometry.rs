// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary coefficient branch geometry handoff.

use splot_core::symbol::SymbolDecoder;

use super::super::super::cdf::TileCdfSubset;
use super::super::super::coeff_state::TileCoeffContextState;
use super::super::AllZeroCoeffBlockInput;
use super::super::branch::NonZeroCoeffBlockStartInput;
use super::{
    CoeffOrdinaryBranch, CoeffOrdinaryBranchError, CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    CoeffOrdinaryBranchPlaneTypeInput, CoeffOrdinaryBranchPlaneTypeNonZeroInput,
    CoeffOrdinaryPlaneTypeStateContextConfig, apply_coeff_ordinary_branch_from_plane_type,
};

/// Caller-selected ordinary coefficient branch before state-context geometry.
pub(crate) enum CoeffOrdinaryBranchGeometryInput<'a> {
    /// Decoded `all_zero == 1`.
    AllZero(AllZeroCoeffBlockInput),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchGeometryNonZeroInput<'a>),
}

/// Caller-resolved facts for the ordinary nonzero branch before geometry handoff.
pub(crate) struct CoeffOrdinaryBranchGeometryNonZeroInput<'a> {
    /// Caller-resolved facts for nonzero EOB start, including block geometry.
    pub(crate) start: NonZeroCoeffBlockStartInput,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors plus `PlaneTxType`.
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    /// Caller-resolved facts for state-backed sign/context handoff, before geometry.
    pub(crate) state_context: CoeffOrdinaryGeometryStateContextConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-resolved state-context facts before block-geometry handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryGeometryStateContextConfig {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
}

impl CoeffOrdinaryGeometryStateContextConfig {
    const fn state_context(
        self,
        block: AllZeroCoeffBlockInput,
    ) -> CoeffOrdinaryPlaneTypeStateContextConfig {
        CoeffOrdinaryPlaneTypeStateContextConfig {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            x4: block.x4,
            y4: block.y4,
            w4: block.w4,
            h4: block.h4,
        }
    }
}

/// Dispatches the ordinary branch after deriving context geometry from block geometry.
///
/// This adapts the staged branch boundary for AV2 § 5.20.7.27 `coeffs()`
/// geometry (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`) by
/// reusing the already caller-resolved block geometry in
/// `NonZeroCoeffBlockStartInput.block`. It does not derive raw `startX`,
/// `startY`, or `txSz`, implement `compute_tx_type`, derive scan order, wire
/// runtime `coeffs()`, dequantize, inverse transform, residual add, or
/// reconstruct.
pub(crate) fn apply_coeff_ordinary_branch_from_geometry(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchGeometryInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = match input {
        CoeffOrdinaryBranchGeometryInput::AllZero(input) => {
            CoeffOrdinaryBranchPlaneTypeInput::AllZero(input)
        }
        CoeffOrdinaryBranchGeometryInput::NonZero(input) => {
            CoeffOrdinaryBranchPlaneTypeInput::NonZero(CoeffOrdinaryBranchPlaneTypeNonZeroInput {
                start: input.start,
                scan: input.scan,
                base_config: input.base_config,
                state_context: input.state_context.state_context(input.start.block),
                lossless: input.lossless,
            })
        }
    };
    apply_coeff_ordinary_branch_from_plane_type(state, cdfs, symbols, input)
}
