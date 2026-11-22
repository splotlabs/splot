// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The symbols of the inter mode walks that the decoder shares with the
//! encoder (§ 5.20.7.6, § 5.20.7.11).

use splot_core::symbol::SymbolDecoder;

use crate::bitstream::tile_payload::{TileCdfSubset, read_row, row_at};

/// One method per syntax element of the shared walks, taking its contexts:
/// the decoder reads the element and the encoder answers it for a target.
pub(crate) trait InterModeSymbols {
    fn comp_ref0(&mut self, ctx: usize, ref_idx: usize) -> u8;
    fn comp_ref1(&mut self, ctx: usize, bit_type: usize, ref_idx: usize) -> u8;
    fn is_joint(&mut self, ctx: usize) -> u8;
    fn compound_mode_same_refs(&mut self, ctx: usize) -> u8;
    fn compound_mode_non_joint(&mut self, ctx: usize) -> u8;
    fn use_most_probable_precision(&mut self, ctx: usize) -> u8;
    fn pb_mv_precision(&mut self, ctx: usize, frame_ctx: usize) -> u8;
}

/// Reads each element from its row of the tile CDFs.
pub(crate) struct TileModeSymbols<'a, 'data> {
    pub(crate) cdfs: &'a mut TileCdfSubset,
    pub(crate) symbols: &'a mut SymbolDecoder<'data>,
}

impl InterModeSymbols for TileModeSymbols<'_, '_> {
    #[inline]
    fn comp_ref0(&mut self, ctx: usize, ref_idx: usize) -> u8 {
        let bank = row_at(self.cdfs.block_rows_mut().comp_ref0(), ctx);
        read_row(self.symbols, row_at(bank, ref_idx)) as u8
    }

    #[inline]
    fn comp_ref1(&mut self, ctx: usize, bit_type: usize, ref_idx: usize) -> u8 {
        let bank = row_at(self.cdfs.block_rows_mut().comp_ref1(), ctx);
        read_row(self.symbols, row_at(row_at(bank, bit_type), ref_idx)) as u8
    }

    #[inline]
    fn is_joint(&mut self, ctx: usize) -> u8 {
        let rows = self.cdfs.block_rows_mut().is_joint();
        read_row(self.symbols, row_at(rows, ctx)) as u8
    }

    #[inline]
    fn compound_mode_same_refs(&mut self, ctx: usize) -> u8 {
        let rows = self.cdfs.block_rows_mut().compound_mode_same_refs();
        read_row(self.symbols, row_at(rows, ctx)) as u8
    }

    #[inline]
    fn compound_mode_non_joint(&mut self, ctx: usize) -> u8 {
        let rows = self.cdfs.block_rows_mut().compound_mode_non_joint();
        read_row(self.symbols, row_at(rows, ctx)) as u8
    }

    #[inline]
    fn use_most_probable_precision(&mut self, ctx: usize) -> u8 {
        let rows = self.cdfs.block_rows_mut().use_most_probable_precision();
        read_row(self.symbols, row_at(rows, ctx)) as u8
    }

    #[inline]
    fn pb_mv_precision(&mut self, ctx: usize, frame_ctx: usize) -> u8 {
        let bank = row_at(self.cdfs.block_rows_mut().pb_mv_precision(), ctx);
        read_row(self.symbols, row_at(bank, frame_ctx)) as u8
    }
}
