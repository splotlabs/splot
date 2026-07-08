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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBlockStartInput {
    pub(crate) block: AllZeroCoeffBlockInput,
    pub(crate) eob: NonZeroCoeffEobContextInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBlockStart {
    eob_read: NonZeroCoeffEobSymbolRead,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffBlockStart {
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }

    pub(crate) fn into_parts(self) -> (NonZeroCoeffEobSymbolRead, TransformCoeffBlockState) {
        (self.eob_read, self.block)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBlockEobBranchInput {
    AllZero(AllZeroCoeffBlockInput),
    NonZero(NonZeroCoeffBlockStartInput),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBlockEobBranch {
    AllZero(AllZeroCoeffBlock),
    NonZero(NonZeroCoeffBlockStart),
}

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
