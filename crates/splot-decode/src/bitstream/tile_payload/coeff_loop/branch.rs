// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop EOB branch handoff helpers.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::{
    CoeffLoopContextError, NonZeroCoeffEob, NonZeroCoeffEobSymbolInput, TxGeometry, eob_context,
    read_nonzero_coeff_eob,
};

/// The EOB of a nonzero block with its transform geometry; the block state
/// is created by the pass that fills it, so this stays small.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBlockStart {
    eob_read: NonZeroCoeffEob,
    tx: &'static TxGeometry,
}

impl NonZeroCoeffBlockStart {
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEob {
        self.eob_read
    }

    #[must_use]
    pub(crate) const fn tx(&self) -> &'static TxGeometry {
        self.tx
    }

    #[must_use]
    pub(crate) const fn width(&self) -> usize {
        self.tx.width as usize
    }

    #[must_use]
    pub(crate) const fn coeff_count(&self) -> usize {
        self.tx.width as usize * self.tx.height as usize
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
            TransformCoeffBlockState::new(self.width(), usize::from(self.tx.height))?,
        ))
    }
}

/// Reads the EOB of a nonzero block of plane `plane` with geometry `tx`.
pub(crate) fn read_nonzero_coeff_block_start(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx: &'static TxGeometry,
    plane: usize,
    is_inter: bool,
) -> Result<NonZeroCoeffBlockStart, CoeffLoopContextError> {
    let input = NonZeroCoeffEobSymbolInput {
        size: tx.eob_size,
        eob_ctx: eob_context(plane, is_inter),
    };
    let eob_read = read_nonzero_coeff_eob(cdfs, symbols, input)?;
    Ok(NonZeroCoeffBlockStart { eob_read, tx })
}
