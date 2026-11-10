// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop EOB branch handoff helpers.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
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
    pub(crate) const fn coeff_count(&self) -> usize {
        self.tx.width as usize * self.tx.height as usize
    }
}

/// Reads the EOB of a nonzero block of plane `plane` with geometry `tx`.
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "measured: the block start keeps the table entry, a copy costs 0.1%"
)]
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
