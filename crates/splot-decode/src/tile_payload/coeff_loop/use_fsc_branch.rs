// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient branch selector for caller-resolved `useFsc`.
//!
//! Feature tracking: `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::TileCoeffContextState;
use super::fsc_quant_pass::{
    CoeffFscBranch, CoeffFscBranchError, CoeffFscBranchTxSizeInput,
    CoeffFscBranchTxSizeNonZeroInput, apply_coeff_fsc_branch_from_tx_size,
};
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessInput, CoeffOrdinaryBranchLosslessNonZeroInput,
    CoeffOrdinaryTxSizeGeometryConfig, apply_coeff_ordinary_branch_from_lossless,
};
use super::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};

/// Caller-selected coefficient branch before the AV2 `useFsc` split.
pub(crate) enum CoeffUseFscBranchInput {
    /// Decoded `all_zero == 1`; AV2 handles this before deriving `useFsc`.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffUseFscBranchNonZeroInput),
}

/// Caller-resolved facts for the nonzero `useFsc` branch selector.
pub(crate) struct CoeffUseFscBranchNonZeroInput {
    /// Caller-resolved AV2 § 5.20.7.27 `useFsc` branch condition.
    pub(crate) use_fsc: bool,
    /// Lower-boundary input for the ordinary non-FSC branch.
    pub(crate) ordinary: CoeffOrdinaryBranchLosslessNonZeroInput,
    /// Lower-boundary input for the FSC/IDTX branch.
    pub(crate) fsc: CoeffFscBranchTxSizeNonZeroInput,
}

/// Result of the loaded `useFsc` branch selector.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "crate-private selector preserves existing branch result types without boxing"
)]
pub(crate) enum CoeffUseFscBranch {
    /// Ordinary non-FSC branch result.
    Ordinary(CoeffOrdinaryBranch),
    /// FSC/IDTX branch result.
    Fsc(CoeffFscBranch),
}

/// Error returned by the loaded `useFsc` branch selector.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffUseFscBranchError {
    /// The ordinary branch rejected the selected input.
    #[error("coefficient useFsc ordinary branch failed: {0}")]
    Ordinary(#[from] CoeffOrdinaryBranchError),
    /// The FSC/IDTX branch rejected the selected input.
    #[error("coefficient useFsc FSC branch failed: {0}")]
    Fsc(#[from] CoeffFscBranchError),
}

/// Dispatches the coefficient branch after caller-resolved `useFsc`.
///
/// AV2 § 5.20.7.27 handles `all_zero` before deriving and testing `useFsc`.
/// This loaded-but-unwired selector preserves that ordering: all-zero inputs
/// always go through the ordinary all-zero branch, while nonzero inputs dispatch
/// to either the ordinary lossless handoff or the FSC tx-size handoff based on
/// caller-resolved `use_fsc`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). Runtime
/// `useFsc` derivation, full `compute_tx_type`, dequantization, inverse
/// transform, residual add, and reconstruction remain out of scope.
pub(crate) fn apply_coeff_use_fsc_branch(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffUseFscBranchInput,
) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError> {
    match input {
        CoeffUseFscBranchInput::AllZero(input) => {
            let branch = apply_coeff_ordinary_branch_from_lossless(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchLosslessInput::AllZero(input),
            )?;
            Ok(CoeffUseFscBranch::Ordinary(branch))
        }
        CoeffUseFscBranchInput::NonZero(input) if input.use_fsc => {
            let branch = apply_coeff_fsc_branch_from_tx_size(
                state,
                cdfs,
                symbols,
                CoeffFscBranchTxSizeInput::NonZero(input.fsc),
            )?;
            Ok(CoeffUseFscBranch::Fsc(branch))
        }
        CoeffUseFscBranchInput::NonZero(input) => {
            let branch = apply_coeff_ordinary_branch_from_lossless(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchLosslessInput::NonZero(input.ordinary),
            )?;
            Ok(CoeffUseFscBranch::Ordinary(branch))
        }
    }
}
