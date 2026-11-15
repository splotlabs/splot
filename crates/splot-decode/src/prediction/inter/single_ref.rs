// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;

use crate::bitstream::tile_payload::{TileCdfSubset, read_row, row_at};

#[allow(clippy::inline_always, reason = "measured: per-leaf reference read")]
#[inline(always)]
pub(crate) fn read_single_ref(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    num_total_refs: usize,
    ctx: impl Fn(usize) -> usize,
) -> usize {
    let decisions = num_total_refs.saturating_sub(1);
    for ref_idx in 0..decisions {
        let bank = row_at(&mut cdfs.block_rows_mut().single_ref, ctx(ref_idx));
        if read_row(symbols, row_at(bank, ref_idx)) != 0 {
            return ref_idx;
        }
    }
    decisions
}

#[cfg(test)]
mod tests;
