// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop EOB branch handoff helpers.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{TileCoeffContextState, TransformCoeffBlockState};
use super::{
    AllZeroCoeffBlock, AllZeroCoeffBlockInput, CoeffLoopContextError, NonZeroCoeffEobContextInput,
    NonZeroCoeffEobSymbolRead, adjusted_coeff_extent, apply_all_zero_coeff_block,
    read_nonzero_coeff_eob_from_context,
};

/// Caller-resolved facts for starting the nonzero § 5.20.7.27 coefficient path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBlockStartInput {
    /// Transform-block geometry for the local coefficient arrays.
    pub(crate) block: AllZeroCoeffBlockInput,
    /// Caller-resolved facts for reading the nonzero EOB syntax.
    pub(crate) eob: NonZeroCoeffEobContextInput,
}

/// Zeroed local block state plus the decoded nonzero EOB syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBlockStart {
    eob_read: NonZeroCoeffEobSymbolRead,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffBlockStart {
    /// Decoded nonzero EOB syntax result.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    /// Zero-initialized local transform coefficient state.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }

    /// Consumes the start state into its decoded EOB facts and local block.
    pub(crate) fn into_parts(self) -> (NonZeroCoeffEobSymbolRead, TransformCoeffBlockState) {
        (self.eob_read, self.block)
    }
}

/// Caller-selected § 5.20.7.27 coefficient EOB branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBlockEobBranchInput {
    /// Decoded `all_zero == 1`.
    AllZero(AllZeroCoeffBlockInput),
    /// Decoded `all_zero == 0`.
    NonZero(NonZeroCoeffBlockStartInput),
}

/// Result of the § 5.20.7.27 coefficient EOB branch handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBlockEobBranch {
    /// All-zero coefficient state was applied.
    AllZero(AllZeroCoeffBlock),
    /// Nonzero EOB syntax was read after local block state was initialized.
    NonZero(NonZeroCoeffBlockStart),
}

/// Dispatches the AV2 § 5.20.7.27 branch after caller-decoded `all_zero`.
pub(crate) fn read_coeff_block_eob_branch(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffBlockEobBranchInput,
) -> Result<CoeffBlockEobBranch, CoeffLoopContextError> {
    match input {
        CoeffBlockEobBranchInput::AllZero(input) => {
            apply_all_zero_coeff_block(state, input).map(CoeffBlockEobBranch::AllZero)
        }
        CoeffBlockEobBranchInput::NonZero(input) => {
            let width = adjusted_coeff_extent(input.block.w4);
            let height = adjusted_coeff_extent(input.block.h4);
            let block = TransformCoeffBlockState::new(width, height)?;
            let eob_read = read_nonzero_coeff_eob_from_context(cdfs, symbols, input.eob)?;

            Ok(CoeffBlockEobBranch::NonZero(NonZeroCoeffBlockStart {
                eob_read,
                block,
            }))
        }
    }
}
