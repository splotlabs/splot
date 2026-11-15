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

    /// [`Self::read_block_symbol_trace`] with the inter mode-info rows read
    /// through [`read_row`], so a constant selector reads a fixed-arity row.
    #[allow(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn read_block_symbol(
        &mut self,
        selector: TileCdfSelector,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<u8, BlockSymbolTraceReadError> {
        let tile = &mut self.rows;
        let rows = &mut tile.block;
        let symbol = match selector {
            TileCdfSelector::IsCfl { ctx } => read_row(symbols, row_at(&mut rows.is_cfl, ctx)),
            TileCdfSelector::UvModeCflNotAllowed { ctx } => {
                read_row(symbols, row_at(&mut rows.uv_mode_cfl_not_allowed, ctx))
            }
            TileCdfSelector::UseDip { ctx } => read_row(symbols, row_at(&mut rows.use_dip, ctx)),
            TileCdfSelector::DipMode => read_row(symbols, &mut rows.dip_mode),
            TileCdfSelector::CflIndex => read_row(symbols, &mut rows.cfl_index),
            TileCdfSelector::CflMhccp => read_row(symbols, &mut rows.cfl_mhccp),
            TileCdfSelector::Skip { ctx } => read_row(symbols, row_at(&mut rows.skip, ctx)),
            TileCdfSelector::FscMode { ctx, bsize_group } => read_row(
                symbols,
                row_at(row_at(&mut tile.fsc_mode, ctx), bsize_group),
            ),
            TileCdfSelector::MrlIndex { ctx } => {
                read_row(symbols, row_at(&mut tile.mrl_index, ctx))
            }
            TileCdfSelector::MrlSecIndex { ctx } => {
                read_row(symbols, row_at(&mut tile.mrl_sec_index, ctx))
            }
            TileCdfSelector::Intrabc { ctx } => read_row(symbols, row_at(&mut tile.intrabc, ctx)),
            TileCdfSelector::CdefIndex0 { ctx } => {
                read_row(symbols, row_at(&mut tile.cdef_index0, ctx))
            }
            TileCdfSelector::CompRef0 { ctx, ref_idx } => {
                read_row(symbols, row_at(row_at(&mut rows.comp_ref0, ctx), ref_idx))
            }
            TileCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => {
                let bank = row_at(row_at(&mut rows.comp_ref1, ctx), bit_type);
                read_row(symbols, row_at(bank, ref_idx))
            }
            TileCdfSelector::IsJoint { ctx } => read_row(symbols, row_at(&mut rows.is_joint, ctx)),
            TileCdfSelector::CompoundModeNonJoint { ctx } => {
                read_row(symbols, row_at(&mut rows.compound_mode_non_joint, ctx))
            }
            TileCdfSelector::CompoundModeSameRefs { ctx } => {
                read_row(symbols, row_at(&mut rows.compound_mode_same_refs, ctx))
            }
            TileCdfSelector::UseAmvd { index, ctx } => {
                read_row(symbols, row_at(row_at(&mut rows.use_amvd, index), ctx))
            }
            TileCdfSelector::UseMostProbablePrecision { ctx } => {
                read_row(symbols, row_at(&mut rows.use_most_probable_precision, ctx))
            }
            TileCdfSelector::PbMvPrecision { ctx, frame_ctx } => {
                let bank = row_at(&mut rows.pb_mv_precision, ctx);
                read_row(symbols, row_at(bank, frame_ctx))
            }
            TileCdfSelector::ExplicitBawp { ctx } => {
                read_row(symbols, row_at(&mut rows.explicit_bawp, ctx))
            }
            TileCdfSelector::ExplicitBawpScale => read_row(symbols, &mut rows.explicit_bawp_scale),
            TileCdfSelector::UseBawpChroma => read_row(symbols, &mut rows.use_bawp_chroma),
            TileCdfSelector::UseExtendWarp { ctx } => {
                read_row(symbols, row_at(&mut rows.use_extend_warp, ctx))
            }
            TileCdfSelector::UseLocalWarp { ctx } => {
                read_row(symbols, row_at(&mut rows.use_local_warp, ctx))
            }
            TileCdfSelector::SkipDrlMode { idx } => {
                read_row(symbols, row_at(&mut rows.skip_drl_mode, idx))
            }
            TileCdfSelector::TipDrlMode { idx } => {
                read_row(symbols, row_at(&mut rows.tip_drl_mode, idx))
            }
            selector => {
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
