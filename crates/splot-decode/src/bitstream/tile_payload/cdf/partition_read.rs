// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.2 partition-entry `S()` symbol reads.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY`.

use splot_core::Error as CoreError;
use splot_core::symbol::{Symbol, SymbolDecoder};

use super::{TileCdfError, TileCdfSelector, TileCdfSubset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum PartitionEntrySymbolReadError {
    #[error("partition-entry CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("partition-entry symbol read failed: {0}")]
    Symbol(#[from] CoreError),
}

impl TileCdfSubset {
    pub(crate) fn read_partition_entry_symbol(
        &mut self,
        selector: TileCdfSelector,
        symbol_decoder: &mut SymbolDecoder<'_>,
    ) -> Result<Symbol, PartitionEntrySymbolReadError> {
        let symbol = self
            .with_row_mut(selector, |row| symbol_decoder.read_symbol_u16(row))
            .map_err(PartitionEntrySymbolReadError::Cdf)?
            .map_err(PartitionEntrySymbolReadError::Symbol)?;
        Ok(symbol)
    }
}

#[cfg(test)]
#[path = "partition_read_tests.rs"]
mod tests;
