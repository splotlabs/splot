// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 block-symbol `S()` reads.

use splot_core::Error as CoreError;
use splot_core::symbol::{Symbol, SymbolDecoder};

use super::{TileCdfError, TileCdfSelector, TileCdfSubset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum BlockSymbolTraceReadError {
    #[error("block-symbol trace CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("block-symbol trace read failed: {0}")]
    Symbol(#[from] CoreError),
}

impl TileCdfSubset {
    pub(crate) fn read_block_symbol_trace(
        &mut self,
        selector: TileCdfSelector,
        symbol_decoder: &mut SymbolDecoder<'_>,
    ) -> Result<Symbol, BlockSymbolTraceReadError> {
        let symbol = self
            .with_row_mut(selector, |row| symbol_decoder.read_symbol(row))
            .map_err(BlockSymbolTraceReadError::Cdf)?
            .map_err(BlockSymbolTraceReadError::Symbol)?;
        Ok(symbol)
    }
}

#[cfg(test)]
#[path = "block_read_tests.rs"]
mod tests;
