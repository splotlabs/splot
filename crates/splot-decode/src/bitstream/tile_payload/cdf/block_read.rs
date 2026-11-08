// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 block-symbol `S()` reads.

use splot_core::Error as CoreError;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder};

use super::{
    CoeffCdfRows, CoeffCdfSelector, EobPtSize, TileCdfError, TileCdfSelector, TileCdfSubset,
};

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

    /// Reads `all_zero` (§ 8.3.2); see [`Self::read_trusted_symbol`].
    #[allow(
        clippy::inline_always,
        reason = "measured: the selector folds into the row read"
    )]
    #[inline(always)]
    pub(crate) fn read_all_zero(
        &mut self,
        selector: TileCdfSelector,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<bool, BlockSymbolTraceReadError> {
        Ok(self.read_trusted_symbol(selector, symbols)? != 0)
    }

    /// Reads the `all_zero`, `eob_pt_*`, `eob_extra`, `dc_sign`, intra
    /// transform-type, `sec_tx_type`, `most_probable_stx_set` and
    /// `cctx_type` symbols through the trusted read of their decoder-owned
    /// rows; any other selector, or a context outside the rows, takes
    /// [`Self::read_block_symbol_trace`].
    #[allow(
        clippy::inline_always,
        reason = "measured: the selector folds into the row read"
    )]
    #[inline(always)]
    pub(crate) fn read_trusted_symbol(
        &mut self,
        selector: TileCdfSelector,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<u8, BlockSymbolTraceReadError> {
        let update = symbols.cdf_update_mode() == CdfUpdateMode::Enabled;
        let rows = &mut self.rows.block;
        macro_rules! read {
            ($row:expr) => {
                if let Some(row) = $row {
                    return Ok(read_trusted(symbols, row, update));
                }
            };
        }
        match selector {
            TileCdfSelector::TxbSkip {
                plane_type,
                tx_size,
                ctx,
            } => read!(
                rows.txb_skip
                    .get_mut(plane_type)
                    .and_then(|rows| rows.get_mut(tx_size))
                    .and_then(|rows| rows.get_mut(ctx))
            ),
            TileCdfSelector::VTxbSkip { ctx } => read!(rows.v_txb_skip.get_mut(ctx)),
            TileCdfSelector::EobExtra => read!(Some(&mut rows.eob_extra)),
            TileCdfSelector::EobPt { size, eob_ctx } => match size {
                EobPtSize::Pt16 => read!(rows.eob_pt_16.get_mut(eob_ctx)),
                EobPtSize::Pt32 => read!(rows.eob_pt_32.get_mut(eob_ctx)),
                EobPtSize::Pt64 => read!(rows.eob_pt_64.get_mut(eob_ctx)),
                EobPtSize::Pt128 => read!(rows.eob_pt_128.get_mut(eob_ctx)),
                EobPtSize::Pt256 => read!(rows.eob_pt_256.get_mut(eob_ctx)),
                EobPtSize::Pt512 => read!(rows.eob_pt_512.get_mut(eob_ctx)),
                EobPtSize::Pt1024 => read!(rows.eob_pt_1024.get_mut(eob_ctx)),
            },
            TileCdfSelector::DcSign {
                plane_type,
                group,
                ctx,
            } => read!(
                rows.dc_sign
                    .get_mut(plane_type)
                    .and_then(|rows| rows.get_mut(group))
                    .and_then(|rows| rows.get_mut(ctx))
            ),
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr } => {
                read!(rows.intra_tx_type_set1.get_mut(tx_size_sqr));
            }
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr } => {
                read!(rows.intra_tx_type_set2.get_mut(tx_size_sqr));
            }
            TileCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            } => read!(
                rows.sec_tx_type
                    .get_mut(is_inter)
                    .and_then(|rows| rows.get_mut(tx_size_sqr))
            ),
            TileCdfSelector::MostProbableStxSet => read!(Some(&mut rows.most_probable_stx_set)),
            TileCdfSelector::MostProbableStxSetAdst => {
                read!(Some(&mut rows.most_probable_stx_set_adst));
            }
            TileCdfSelector::CctxType => read!(Some(&mut rows.cctx_type)),
            _ => {}
        }
        Ok(self.read_block_symbol_trace(selector, symbols)?.get())
    }

    #[inline]
    pub(crate) fn coeff_rows_mut(&mut self) -> &mut CoeffCdfRows {
        &mut self.rows.block.coeff
    }
}

/// A trusted read of a decoder-owned row in the tile's update mode.
#[allow(clippy::inline_always, reason = "measured: per-block symbol read")]
#[inline(always)]
fn read_trusted<const L: usize>(
    symbols: &mut SymbolDecoder<'_>,
    row: &mut [u16; L],
    update: bool,
) -> u8 {
    (if update {
        symbols.read_trusted::<L, true>(row)
    } else {
        symbols.read_trusted::<L, false>(row)
    }) as u8
}

#[cfg(test)]
#[path = "block_read_tests.rs"]
mod tests;
