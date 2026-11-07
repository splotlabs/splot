// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.3.2 partition-entry `S()` symbol reads.
//!
//! Feature tracking: `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY`.

use splot_core::Error as CoreError;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder};

use super::block_read::BlockSymbolTraceReadError;
use super::{TileCdfError, TileCdfSelector, TileCdfSubset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum PartitionEntrySymbolReadError {
    #[error("partition-entry CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("partition-entry symbol read failed: {0}")]
    Symbol(#[from] CoreError),
}

impl TileCdfSubset {
    /// Reads a binary § 5.20.3.2 partition flag through the trusted read of
    /// its decoder-owned row; any other selector takes
    /// [`Self::read_partition_entry_symbol`].
    #[allow(clippy::inline_always, reason = "the selector folds into the row read")]
    #[inline(always)]
    pub(crate) fn read_partition_flag(
        &mut self,
        selector: TileCdfSelector,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<bool, PartitionEntrySymbolReadError> {
        let rows = &mut self.rows;
        let row = match selector {
            TileCdfSelector::DoSplit { plane_start, ctx } => rows
                .do_split
                .get_mut(plane_start)
                .and_then(|rows| rows.get_mut(ctx)),
            TileCdfSelector::DoSquareSplit { plane_start, ctx } => rows
                .do_square_split
                .get_mut(plane_start)
                .and_then(|rows| rows.get_mut(ctx)),
            TileCdfSelector::DoExtPartition { plane_start, ctx } => rows
                .do_ext_partition
                .get_mut(plane_start)
                .and_then(|rows| rows.get_mut(ctx)),
            TileCdfSelector::RectType { plane_start, ctx } => rows
                .rect_type
                .get_mut(plane_start)
                .and_then(|rows| rows.get_mut(ctx)),
            TileCdfSelector::DoUneven4WayPartition { plane_start, ctx } => rows
                .do_uneven_4way_partition
                .get_mut(plane_start)
                .and_then(|rows| rows.get_mut(ctx)),
            _ => None,
        };
        if let Some(row) = row {
            let symbol = match symbols.cdf_update_mode() {
                CdfUpdateMode::Enabled => symbols.read_trusted::<3, true>(row),
                CdfUpdateMode::Disabled => symbols.read_trusted::<3, false>(row),
            };
            return Ok(symbol != 0);
        }
        Ok(self.read_partition_entry_symbol(selector, symbols)?.get() != 0)
    }

    #[allow(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn read_partition_entry_symbol(
        &mut self,
        selector: TileCdfSelector,
        symbol_decoder: &mut SymbolDecoder<'_>,
    ) -> Result<Symbol, PartitionEntrySymbolReadError> {
        self.read_block_symbol_trace(selector, symbol_decoder)
            .map_err(|error| match error {
                BlockSymbolTraceReadError::Cdf(error) => PartitionEntrySymbolReadError::Cdf(error),
                BlockSymbolTraceReadError::Symbol(error) => {
                    PartitionEntrySymbolReadError::Symbol(error)
                }
            })
    }
}

#[cfg(test)]
#[path = "partition_read_tests.rs"]
mod tests;
