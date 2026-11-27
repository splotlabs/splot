// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 block-symbol `S()` reads.

use splot_core::Error as CoreError;
use splot_core::symbol::{Symbol, SymbolDecoder};

use super::{BlockCdfRows, CoeffCdfRows, TileCdfError, TileCdfSelector, TileCdfSubset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum BlockSymbolTraceReadError {
    #[error("block-symbol trace CDF selection failed: {0}")]
    Cdf(#[from] TileCdfError),
    #[error("block-symbol trace read failed: {0}")]
    Symbol(#[from] CoreError),
}

impl TileCdfSubset {
    #[expect(
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
        #[cfg(feature = "block-evidence")]
        if symbol_decoder.observes_entropy() {
            annotate_symbol(symbol_decoder, selector);
        }
        match symbol_decoder.read_trusted_row(row) {
            Some(symbol) => Ok(Symbol::new(symbol as u8)),
            None => Ok(symbol_decoder.read_symbol_u16(row)?),
        }
    }

    /// [`Self::read_block_symbol_trace`] with the inter mode-info rows read
    /// through [`read_row`], so a constant selector reads a fixed-arity row.
    #[expect(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn read_block_symbol(
        &mut self,
        selector: TileCdfSelector,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<u8, BlockSymbolTraceReadError> {
        #[cfg(feature = "block-evidence")]
        if symbols.observes_entropy() {
            annotate_symbol(symbols, selector);
        }
        let tile = &mut self.rows;
        let rows = &mut tile.block;
        let symbol = match selector {
            TileCdfSelector::IsCfl { ctx } => read_row(symbols, row_at(rows.is_cfl(), ctx)),
            TileCdfSelector::UvModeCflNotAllowed { ctx } => {
                read_row(symbols, row_at(rows.uv_mode_cfl_not_allowed(), ctx))
            }
            TileCdfSelector::UseDip { ctx } => read_row(symbols, row_at(rows.use_dip(), ctx)),
            TileCdfSelector::DipMode => read_row(symbols, rows.dip_mode()),
            TileCdfSelector::CflIndex => read_row(symbols, rows.cfl_index()),
            TileCdfSelector::CflMhccp => read_row(symbols, rows.cfl_mhccp()),
            TileCdfSelector::Skip { ctx } => read_row(symbols, row_at(rows.skip(), ctx)),
            TileCdfSelector::FscMode { ctx, bsize_group } => {
                read_row(symbols, row_at(row_at(tile.fsc_mode(), ctx), bsize_group))
            }
            TileCdfSelector::MrlIndex { ctx } => read_row(symbols, row_at(tile.mrl_index(), ctx)),
            TileCdfSelector::MrlSecIndex { ctx } => {
                read_row(symbols, row_at(tile.mrl_sec_index(), ctx))
            }
            TileCdfSelector::Intrabc { ctx } => read_row(symbols, row_at(tile.intrabc(), ctx)),
            TileCdfSelector::CdefIndex0 { ctx } => {
                read_row(symbols, row_at(tile.cdef_index0(), ctx))
            }
            TileCdfSelector::UseAmvd { index, ctx } => {
                read_row(symbols, row_at(row_at(rows.use_amvd(), index), ctx))
            }
            TileCdfSelector::ExplicitBawp { ctx } => {
                read_row(symbols, row_at(rows.explicit_bawp(), ctx))
            }
            TileCdfSelector::ExplicitBawpScale => read_row(symbols, rows.explicit_bawp_scale()),
            TileCdfSelector::UseBawpChroma => read_row(symbols, rows.use_bawp_chroma()),
            TileCdfSelector::UseExtendWarp { ctx } => {
                read_row(symbols, row_at(rows.use_extend_warp(), ctx))
            }
            TileCdfSelector::UseLocalWarp { ctx } => {
                read_row(symbols, row_at(rows.use_local_warp(), ctx))
            }
            TileCdfSelector::SkipDrlMode { idx } => {
                read_row(symbols, row_at(rows.skip_drl_mode(), idx))
            }
            TileCdfSelector::TipDrlMode { idx } => {
                read_row(symbols, row_at(rows.tip_drl_mode(), idx))
            }
            selector => {
                return self
                    .read_block_symbol_trace(selector, symbols)
                    .map(Symbol::get);
            }
        };
        Ok(symbol as u8)
    }

    /// The luma palette colour index of context `ctx` (§ 8.3.2), read once
    /// per pixel of a palette block.
    pub(crate) fn read_palette_y_color(
        &mut self,
        palette_size: usize,
        ctx: usize,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<u8, BlockSymbolTraceReadError> {
        let rows = &mut self.rows.block;
        let symbol = match palette_size {
            2 => read_row(symbols, row_at(rows.palette_size_2_y_color(), ctx)),
            3 => read_row(symbols, row_at(rows.palette_size_3_y_color(), ctx)),
            4 => read_row(symbols, row_at(rows.palette_size_4_y_color(), ctx)),
            5 => read_row(symbols, row_at(rows.palette_size_5_y_color(), ctx)),
            6 => read_row(symbols, row_at(rows.palette_size_6_y_color(), ctx)),
            7 => read_row(symbols, row_at(rows.palette_size_7_y_color(), ctx)),
            8 => read_row(symbols, row_at(rows.palette_size_8_y_color(), ctx)),
            _ => {
                let selector = TileCdfSelector::PaletteYColorIndex { palette_size, ctx };
                return self
                    .read_block_symbol_trace(selector, symbols)
                    .map(Symbol::get);
            }
        };
        Ok(symbol as u8)
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

#[cfg(feature = "block-evidence")]
#[cold]
fn annotate_symbol(symbols: &SymbolDecoder<'_>, selector: TileCdfSelector) {
    use splot_core::symbol::EntropySyntax;

    let (key, context, values) = match selector {
        TileCdfSelector::Skip { ctx } => ("skip", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::Intrabc { ctx } => ("intrabc", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::UseDip { ctx } => ("use_dip", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::UseAmvd { index, ctx } => {
            ("use_amvd", "index,ctx", [index as u16, ctx as u16, 0])
        }
        TileCdfSelector::MrlIndex { ctx } => ("mrl_index", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::MrlSecIndex { ctx } => ("mrl_sec_index", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::IsCfl { ctx } => ("is_cfl", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::FscMode { ctx, bsize_group } => (
            "fsc_mode",
            "ctx,bsize_group",
            [ctx as u16, bsize_group as u16, 0],
        ),
        TileCdfSelector::UseExtendWarp { ctx } => ("use_extend_warp", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::UseLocalWarp { ctx } => ("use_local_warp", "ctx", [ctx as u16, 0, 0]),
        TileCdfSelector::UseDpcmY => ("use_dpcm_y", "", [0; 3]),
        TileCdfSelector::UseDpcmUv => ("use_dpcm_uv", "", [0; 3]),
        TileCdfSelector::SecTxType {
            is_inter,
            tx_size_sqr,
        } => (
            "sec_tx_type",
            "is_inter,tx_size_sqr",
            [is_inter as u16, tx_size_sqr as u16, 0],
        ),
        _ => return,
    };
    symbols.annotate_entropy(EntropySyntax::new(key, context, values));
}

/// A trusted read of a decoder-owned row in the tile's update mode.
#[expect(clippy::inline_always, reason = "measured: per-block symbol read")]
#[inline(always)]
pub(crate) fn read_row<const L: usize>(
    symbols: &mut SymbolDecoder<'_>,
    row: &mut [u16; L],
) -> usize {
    symbols.read_trusted(row) as usize
}

/// `rows[ctx]` with the context clamped into the array: every context the
/// decoder derives is in range, so the clamp only keeps the read panic-free.
#[expect(clippy::inline_always, reason = "measured: per-symbol row")]
#[inline(always)]
pub(crate) fn row_at<T, const C: usize>(rows: &mut [T; C], ctx: usize) -> &mut T {
    &mut rows[ctx.min(C - 1)]
}

#[cfg(test)]
#[path = "block_read_tests.rs"]
mod tests;
