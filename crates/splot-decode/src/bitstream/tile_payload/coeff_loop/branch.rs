// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop EOB branch handoff helpers.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::{
    AllZeroCoeffBlockInput, CoeffLoopContextError, NonZeroCoeffEob, NonZeroCoeffEobContextInput,
    adjusted_coeff_extent, read_nonzero_coeff_eob_from_context,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBlockStartInput {
    pub(crate) block: AllZeroCoeffBlockInput,
    pub(crate) eob: NonZeroCoeffEobContextInput,
}

/// The EOB of a nonzero block with its validated coefficient extent; the
/// block state is created by the pass that fills it, so this stays small.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBlockStart {
    eob_read: NonZeroCoeffEob,
    width: usize,
    height: usize,
}

impl NonZeroCoeffBlockStart {
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEob {
        self.eob_read
    }

    #[must_use]
    pub(crate) const fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub(crate) const fn coeff_count(&self) -> usize {
        self.width * self.height
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: an out-of-line return spills the state through memory"
    )]
    #[inline(always)]
    pub(crate) fn into_parts(
        self,
    ) -> Result<(NonZeroCoeffEob, TransformCoeffBlockState), TileCoeffStateError> {
        Ok((
            self.eob_read,
            TransformCoeffBlockState::new(self.width, self.height)?,
        ))
    }
}

pub(crate) fn read_nonzero_coeff_block_start(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffBlockStartInput,
) -> Result<NonZeroCoeffBlockStart, CoeffLoopContextError> {
    let width = adjusted_coeff_extent(input.block.w4);
    let height = adjusted_coeff_extent(input.block.h4);
    TransformCoeffBlockState::allocation(width, height)?;
    let eob_read = read_nonzero_coeff_eob_from_context(cdfs, symbols, input.eob)?;

    Ok(NonZeroCoeffBlockStart {
        eob_read,
        width,
        height,
    })
}
