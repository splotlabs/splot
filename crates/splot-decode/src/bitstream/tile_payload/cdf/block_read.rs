// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 block-symbol `S()` reads.

use splot_core::Error as CoreError;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder};

use super::{BlockCdfRows, CoeffCdfRows, TileCdfError, TileCdfSelector, TileCdfSubset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum BlockSymbolTraceReadError {
    #[error("block-symbol trace CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("block-symbol trace read failed: {0}")]
    Symbol(#[from] CoreError),
}

impl TileCdfSubset {
    #[allow(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn read_block_symbol_trace(
        &mut self,
        selector: TileCdfSelector,
        symbol_decoder: &mut SymbolDecoder<'_>,
    ) -> Result<Symbol, BlockSymbolTraceReadError> {
        let row = self.rows.row_mut(selector)?;
        match symbol_decoder.read_trusted_row(row) {
            Some(symbol) => Ok(Symbol::new(symbol as u8)),
            None => Ok(symbol_decoder.read_symbol_u16(row)?),
        }
    }

    #[inline]
    pub(crate) fn block_rows_mut(&mut self) -> &mut BlockCdfRows {
        &mut self.rows.block
    }

    #[inline]
    pub(crate) fn coeff_rows_mut(&mut self) -> &mut CoeffCdfRows {
        &mut self.rows.block.coeff
    }
}

/// A trusted read of a decoder-owned row in the tile's update mode.
#[allow(clippy::inline_always, reason = "measured: per-block symbol read")]
#[inline(always)]
pub(crate) fn read_row<const L: usize>(
    symbols: &mut SymbolDecoder<'_>,
    row: &mut [u16; L],
) -> usize {
    (if symbols.cdf_update_mode() == CdfUpdateMode::Enabled {
        symbols.read_trusted::<L, true>(row)
    } else {
        symbols.read_trusted::<L, false>(row)
    }) as usize
}

/// `rows[ctx]` with the context clamped into the array: every context the
/// decoder derives is in range, so the clamp only keeps the read panic-free.
#[allow(clippy::inline_always, reason = "measured: per-symbol row")]
#[inline(always)]
pub(crate) fn row_at<T, const C: usize>(rows: &mut [T; C], ctx: usize) -> &mut T {
    &mut rows[ctx.min(C - 1)]
}

#[cfg(test)]
#[path = "block_read_tests.rs"]
mod tests;
