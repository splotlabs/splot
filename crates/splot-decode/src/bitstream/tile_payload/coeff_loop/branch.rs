// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop EOB branch handoff helpers.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::TransformCoeffBlockState;
use super::{
    AllZeroCoeffBlockInput, CoeffLoopContextError, NonZeroCoeffEobContextInput,
    NonZeroCoeffEobSymbolRead, adjusted_coeff_extent, read_nonzero_coeff_eob_from_context,
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

pub(crate) fn read_nonzero_coeff_block_start(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffBlockStartInput,
) -> Result<NonZeroCoeffBlockStart, CoeffLoopContextError> {
    let width = adjusted_coeff_extent(input.block.w4);
    let height = adjusted_coeff_extent(input.block.h4);
    let block = TransformCoeffBlockState::new(width, height)?;
    finish_nonzero_coeff_block_start(cdfs, symbols, input, block)
}

pub(crate) fn read_nonzero_fsc_coeff_block_start(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffBlockStartInput,
) -> Result<NonZeroCoeffBlockStart, CoeffLoopContextError> {
    let width = adjusted_coeff_extent(input.block.w4);
    let height = adjusted_coeff_extent(input.block.h4);
    let mut block = TransformCoeffBlockState::new(width, height)?;
    block.ensure_quant_sign()?;
    finish_nonzero_coeff_block_start(cdfs, symbols, input, block)
}

fn finish_nonzero_coeff_block_start(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffBlockStartInput,
    block: TransformCoeffBlockState,
) -> Result<NonZeroCoeffBlockStart, CoeffLoopContextError> {
    let eob_read = read_nonzero_coeff_eob_from_context(cdfs, symbols, input.eob)?;

    Ok(NonZeroCoeffBlockStart { eob_read, block })
}
