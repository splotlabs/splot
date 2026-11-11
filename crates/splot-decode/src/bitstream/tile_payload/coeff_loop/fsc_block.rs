// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The coefficients of one FSC/IDTX transform block (§ 5.20.7.27, `useFsc`)
//! in one call, on the ordinary pass's level-word grid and trusted reads:
//! the forward level pass from `bob`, then the sign and remainder pass
//! writing `Quant` directly. The block sits one grid row and column down,
//! so the left, above and above-left neighbours of its first row and column
//! read zero padding.

use splot_core::symbol::{CdfUpdateMode, SymbolDecoder};

use super::super::cdf::TileCdfSubset;
use super::super::cdf::coeff_context::coeff_base_bob_ctx;
use super::super::coeff_state::{
    CoeffBlockLines, LEVEL_STRIDE, LEVEL_WORDS_MASK, TileCoeffContextState, TileCoeffStateError,
};
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::{COEFF_BASE_RANGE, CoeffTransformClass, NUM_BASE_LEVELS};
use super::ordinary_pass::CoeffOrdinaryPassError;
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, CoeffQuantStateWriteError,
};
use super::read_quant::{
    CoeffReadQuantConfig, CoeffReadQuantError, CoeffReadQuantInput, CoeffReadQuantState,
};
use super::scan_walk::CoeffScanEntry;
use super::transform_block::{LEVEL_WORD, entry_grid, grid_scan, row_at};

const FSC_MAX_LEVEL: u32 = NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1;
/// Grid-word fields counting the decoded positive and negative signs, above
/// the level bytes; a sum of three words never carries out of either field.
const POSITIVE_SHIFT: u32 = 26;
const NEGATIVE_SHIFT: u32 = 29;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscBranchError {
    #[error("coefficient FSC scan failed: {0}")]
    Scan(#[from] CoeffOrdinaryPassError),
    #[error("coefficient FSC state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient FSC read_quant failed: {0}")]
    ReadQuant(#[from] CoeffReadQuantError),
    #[error("coefficient FSC quant write failed: {0}")]
    QuantState(#[from] CoeffQuantStateWriteError),
}

/// Decodes the FSC block's coefficients into `quant` (zero on entry, one
/// entry per adjusted position), commits its above and left contexts, and
/// returns one past its last nonzero entry.
pub(crate) fn decode_fsc_transform_block(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    start: &NonZeroCoeffBlockStart,
    lines: &CoeffBlockLines,
    quant: &mut [i32],
) -> Result<usize, CoeffFscBranchError> {
    let tx = start.tx();
    let seg_eob = start.coeff_count();
    let scan = grid_scan(
        CoeffTransformClass::TwoD,
        usize::from(tx.width),
        usize::from(tx.height),
        seg_eob,
    )?;
    let eob = start.eob_read().eob();
    let bob = seg_eob.checked_sub(eob).ok_or_else(|| {
        CoeffOrdinaryPassError::from(super::CoeffLoopContextError::FscScanWalkEobOutOfRange {
            eob,
            seg_eob,
        })
    })?;
    let (_, _, levels, nonzero) = state.dc_lines_and_levels(0)?;
    let entries = &scan[bob..];
    let mut owned = symbols.snapshot();
    let block = Block {
        entries,
        bob,
        seg_eob,
        width_log2: u32::from(tx.width_log2),
        size_ctx: usize::from(tx.size_ctx).min(2),
    };
    let mut nonzero_len = 0;
    let grids = (&mut *levels, &mut *nonzero, &mut nonzero_len);
    let result = if owned.cdf_update_mode() == CdfUpdateMode::Enabled {
        block.decode::<true>(cdfs, &mut owned, grids, quant)
    } else {
        block.decode::<false>(cdfs, &mut owned, grids, quant)
    };
    for &index in nonzero.get(..nonzero_len).unwrap_or_default() {
        if let Some(&entry) = entries.get(index as usize) {
            levels[grid(entry)] = 0;
        }
    }
    symbols.restore(owned);
    let (quant_end, summary) = result?;
    state.commit_block(lines, summary.cul_level(), summary.dc_category());
    Ok(quant_end)
}

/// The grid word of a scan entry, one row and column past its ordinary
/// position.
#[allow(clippy::inline_always, reason = "per-coefficient index")]
#[inline(always)]
const fn grid(entry: u32) -> usize {
    (entry_grid(entry) + LEVEL_STRIDE + 1) & LEVEL_WORDS_MASK
}

/// The grid word `back` words before `at`.
#[allow(clippy::inline_always, reason = "per-coefficient index")]
#[inline(always)]
const fn before(at: usize, back: usize) -> usize {
    at.wrapping_sub(back) & LEVEL_WORDS_MASK
}

struct Block<'s> {
    /// The scan from `bob` to the end of the block.
    entries: &'s [u32],
    bob: usize,
    seg_eob: usize,
    width_log2: u32,
    size_ctx: usize,
}

impl Block<'_> {
    #[inline(never)]
    fn decode<const UPD: bool>(
        &self,
        cdfs: &mut TileCdfSubset,
        symbols: &mut SymbolDecoder<'_>,
        (levels, nonzero, nonzero_len): (
            &mut super::super::coeff_state::CoeffLevelWords,
            &mut super::super::coeff_state::CoeffNonzeroList,
            &mut usize,
        ),
        quant: &mut [i32],
    ) -> Result<(usize, super::quant_state::NonZeroCoeffQuantState), CoeffFscBranchError> {
        let rows = cdfs.coeff_rows_mut();
        let bob_row = row_at(row_at(&mut rows.coeff_base_bob, self.size_ctx), {
            coeff_base_bob_ctx(self.bob, self.seg_eob)
        });
        let first = symbols.read_trusted::<4, UPD>(bob_row) + 1;
        let base = row_at(&mut rows.coeff_base_idtx, self.size_ctx);
        let br = row_at(&mut rows.coeff_br_idtx, self.size_ctx);
        for (index, &entry) in self.entries.iter().enumerate() {
            let at = grid(entry);
            let near = levels[before(at, 1)] + levels[before(at, LEVEL_STRIDE)];
            let mut level = if index == 0 {
                first
            } else {
                symbols.read_trusted::<5, UPD>(row_at(base, (near & 0xff) as usize))
            };
            if level > NUM_BASE_LEVELS {
                let ctx = (((near >> 8) & 0xff) as usize).min(6);
                level += symbols.read_trusted::<5, UPD>(row_at(br, ctx));
            }
            levels[at] = LEVEL_WORD[level as usize & 15];
            nonzero[*nonzero_len & (nonzero.len() - 1)] = index as u32;
            *nonzero_len += usize::from(level != 0);
        }

        let signs = row_at(&mut rows.idtx_sign, self.size_ctx);
        let mut read_quant = CoeffReadQuantState::new(CoeffReadQuantConfig {
            is_hidden: false,
            allow_tcq: false,
            hr_level_avg: 0,
        });
        let mut summary = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
            is_hidden: false,
            sum_abs1: 0,
            use_tcq: false,
            lossless: false,
        });
        let mut end = 0;
        for &index in nonzero.get(..*nonzero_len).unwrap_or_default() {
            let index = index as usize;
            let Some(&entry) = self.entries.get(index) else {
                continue;
            };
            let at = grid(entry);
            let word = levels[at];
            let level = (word >> 16) & 0xff;
            let near = levels[before(at, 1)]
                + levels[before(at, LEVEL_STRIDE)]
                + levels[before(at, LEVEL_STRIDE + 1)];
            let signc =
                ((near >> POSITIVE_SHIFT) & 7) as i32 - ((near >> NEGATIVE_SHIFT) & 7) as i32;
            let ctx = match signc {
                3.. => 5,
                1..=2 => 1,
                0 => 0,
                -2..=-1 => 2,
                _ => 6,
            };
            let ctx = ctx
                + if level > COEFF_BASE_RANGE && ctx != 0 {
                    2
                } else {
                    0
                };
            let sign = symbols.read_trusted::<3, UPD>(row_at(signs, ctx)) != 0;
            levels[at] = word | 1 << if sign { NEGATIVE_SHIFT } else { POSITIVE_SHIFT };
            let pos = (entry >> 16) as usize;
            let value = if level < FSC_MAX_LEVEL {
                level
            } else {
                let column = (1 << self.width_log2) - 1;
                let input = CoeffReadQuantInput {
                    entry: CoeffScanEntry::new(
                        self.bob + index,
                        pos,
                        pos >> self.width_log2,
                        pos & column,
                    ),
                    level,
                    max_level: FSC_MAX_LEVEL,
                };
                read_quant.read_one(symbols, index, input)?.quant
            };
            if pos == 0 {
                summary.set_dc_sign(sign);
            }
            let magnitude = summary.ac_magnitude(index, value, false)? as i32;
            if let Some(slot) = quant.get_mut(pos) {
                *slot = if sign { -magnitude } else { magnitude };
                end = end.max(pos + 1);
            }
        }
        Ok((
            end,
            super::quant_state::NonZeroCoeffQuantState::from_accumulator(summary),
        ))
    }
}

#[cfg(test)]
#[path = "fsc_block_tests.rs"]
mod tests;
