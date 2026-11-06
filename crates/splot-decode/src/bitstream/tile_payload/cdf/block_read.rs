// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 block-symbol `S()` reads.

use splot_core::Error as CoreError;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder};

use super::{CoeffCdfRows, CoeffCdfSelector, TileCdfError, TileCdfSelector, TileCdfSubset};

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
        Ok(symbol_decoder.read_symbol_u16(row)?)
    }

    /// Reads a coefficient symbol from the coefficient rows directly,
    /// skipping the generic tile-selector dispatch.
    #[allow(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn read_coeff_symbol(
        &mut self,
        selector: CoeffCdfSelector,
        symbol_decoder: &mut SymbolDecoder<'_>,
    ) -> Result<u8, BlockSymbolTraceReadError> {
        let row = self.rows.block.coeff.row_mut(selector)?;
        Ok(symbol_decoder.read_symbol_u16(row)?.get())
    }

    /// Reads `all_zero` (§ 8.3.2) through the trusted read of its
    /// decoder-owned row; any other selector takes
    /// [`Self::read_block_symbol_trace`].
    #[inline]
    pub(crate) fn read_all_zero(
        &mut self,
        selector: TileCdfSelector,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<bool, BlockSymbolTraceReadError> {
        let rows = &mut self.rows.block;
        let row = match selector {
            TileCdfSelector::TxbSkip {
                plane_type,
                tx_size,
                ctx,
            } => rows
                .txb_skip
                .get_mut(plane_type)
                .and_then(|rows| rows.get_mut(tx_size))
                .and_then(|rows| rows.get_mut(ctx)),
            TileCdfSelector::VTxbSkip { ctx } => rows.v_txb_skip.get_mut(ctx),
            _ => None,
        };
        if let Some(row) = row {
            let symbol = match symbols.cdf_update_mode() {
                CdfUpdateMode::Enabled => symbols.read_trusted::<3, true>(row),
                CdfUpdateMode::Disabled => symbols.read_trusted::<3, false>(row),
            };
            return Ok(symbol != 0);
        }
        Ok(self.read_block_symbol_trace(selector, symbols)?.get() != 0)
    }

    #[inline]
    pub(crate) fn coeff_rows_mut(&mut self) -> &mut CoeffCdfRows {
        &mut self.rows.block.coeff
    }
}

#[cfg(test)]
#[path = "block_read_tests.rs"]
mod tests;
