// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base/level first pass with derived contexts.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::{CoeffBaseContext, CoeffBrContext, coeff_base_eob_ctx};
use super::super::cdf::{CoeffBaseRow, CoeffCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEob;
use super::base_symbol::{CoeffBaseSymbolReadError, read_coeff_symbol};
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::{
    COEFF_BASE_RANGE, CoeffTransformClass, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS,
    coeff_is_low_frequency,
};
use super::quant_state::next_tcq_state;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

const PHTHRESH: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseDerivedLevelPassConfig {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) tx_size_ctx: usize,
    pub(crate) tx_width_log2: u32,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
    pub(crate) plane: usize,
    pub(crate) tx_class: CoeffTransformClass,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CoeffBaseFirstPassSummary {
    sum_abs1: u32,
    num_nonzero: usize,
    is_hidden: bool,
    tcq_state: usize,
}

impl CoeffBaseFirstPassSummary {
    #[must_use]
    pub(crate) const fn sum_abs1(self) -> u32 {
        self.sum_abs1
    }

    #[must_use]
    pub(crate) const fn is_hidden(self) -> bool {
        self.is_hidden
    }

    fn update_after_level(
        &mut self,
        entry: CoeffScanEntry,
        level: u32,
        config: CoeffBaseDerivedLevelPassConfig,
    ) -> Result<(), CoeffBaseDerivedLevelPassError> {
        if config.use_tcq {
            let Some(tcq_state) = next_tcq_state(self.tcq_state, level) else {
                return Err(CoeffBaseDerivedLevelPassError::InvalidTcqState {
                    entry,
                    tcq_state: self.tcq_state,
                });
            };
            self.tcq_state = tcq_state;
        }
        if config.parity_hiding && entry.scan_index() > 0 {
            let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
            self.sum_abs1 ^= clipped & 1;
            if level != 0 {
                self.num_nonzero += 1;
                self.is_hidden = self.num_nonzero >= PHTHRESH;
            }
        }
        Ok(())
    }
}

/// One bit per scan index, enough for the 32x32 coefficient maximum.
const SCAN_MASK_WORDS: usize = 16;

/// Per-scan-index facts the base pass hands to the sign and quant pass, so
/// that pass visits only the nonzero levels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CoeffLevelMasks {
    nonzero: [u64; SCAN_MASK_WORDS],
    /// `(tcqState >> 1) & 1` before each scan index.
    tcq_q0: [u64; SCAN_MASK_WORDS],
}

impl CoeffLevelMasks {
    fn record(&mut self, scan_index: usize, level: u32, tcq_state: usize) {
        let word = (scan_index >> 6) & (SCAN_MASK_WORDS - 1);
        let shift = scan_index & 63;
        self.nonzero[word] |= u64::from(level != 0) << shift;
        self.tcq_q0[word] |= (((tcq_state >> 1) & 1) as u64) << shift;
    }

    pub(crate) fn tcq_q0(&self, scan_index: usize) -> bool {
        (self.tcq_q0[(scan_index >> 6) & (SCAN_MASK_WORDS - 1)] >> (scan_index & 63)) & 1 != 0
    }

    /// The nonzero scan indices below `eob` from high to low (the walk
    /// order), plus scan index 0 when `with_dc`.
    pub(crate) fn nonzero_scan_indices(
        &self,
        eob: usize,
        with_dc: bool,
    ) -> impl Iterator<Item = usize> + '_ {
        let words = eob.div_ceil(64).min(SCAN_MASK_WORDS);
        (0..words).rev().flat_map(move |word| {
            let mut bits = self.nonzero[word] | u64::from(with_dc && word == 0);
            core::iter::from_fn(move || {
                let top = 63_u32.checked_sub(bits.leading_zeros())?;
                bits &= !(1 << top);
                Some(word * 64 + top as usize)
            })
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBaseDerivedLevelPass {
    first_pass: CoeffBaseFirstPassSummary,
    masks: CoeffLevelMasks,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffBaseDerivedLevelPass {
    #[must_use]
    pub(crate) const fn first_pass(&self) -> CoeffBaseFirstPassSummary {
        self.first_pass
    }

    #[must_use]
    pub(crate) const fn block_and_masks_mut(
        &mut self,
    ) -> (&mut TransformCoeffBlockState, &CoeffLevelMasks) {
        (&mut self.block, &self.masks)
    }

    #[must_use]
    pub(crate) fn into_block(self) -> TransformCoeffBlockState {
        self.block
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseDerivedLevelPassError {
    #[error("coefficient base/level scan entries {entries} do not match eob {eob}")]
    ScanEntryCountMismatch { eob: usize, entries: usize },
    #[error("coefficient base/level scan has {entries} entries, above the 32x32 maximum")]
    ScanTooLong { entries: usize },
    #[error(
        "coefficient base/level config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
    )]
    BlockGeometryMismatch {
        block_width: usize,
        block_height: usize,
        config_width: usize,
        config_height: usize,
    },
    #[error("coefficient base/level tx_width_log2 {tx_width_log2} does not match width {tx_width}")]
    TxWidthLog2Mismatch { tx_width_log2: u32, tx_width: usize },
    #[error("coefficient base/level config cannot enable parity hiding and TCQ together")]
    InconsistentParityAndTcq,
    #[error("coefficient base/level entry {entry:?} used invalid tcqState {tcq_state}")]
    InvalidTcqState {
        entry: CoeffScanEntry,
        tcq_state: usize,
    },
    #[error("coefficient base/level base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
    #[error("coefficient base/level state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Kept out of line: inlined into the ordinary-branch function, the level
/// loop shares that function's registers and spills its context state.
#[inline(never)]
pub(crate) fn apply_nonzero_coeff_base_derived_level_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    start: NonZeroCoeffBlockStart,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<NonZeroCoeffBaseDerivedLevelPass, CoeffBaseDerivedLevelPassError> {
    let (eob_read, mut block) = start.into_parts();
    preflight_pass(eob_read, &block, walk, config)?;

    let mut first_pass = CoeffBaseFirstPassSummary::default();
    let mut masks = CoeffLevelMasks::default();
    let mut entries = walk.entries();
    if let Some(entry) = entries.next() {
        let is_lf = coeff_is_low_frequency(entry, config.plane, config.tx_class);
        let selector = base_eob_selector(entry, is_lf, config);
        let level = u32::from(read_coeff_symbol(cdfs, symbols, selector)?) + 1;
        finish_level(
            cdfs,
            symbols,
            entry,
            is_lf,
            level,
            &mut block,
            &mut first_pass,
            &mut masks,
            config,
        )?;
    }
    for entry in entries {
        let is_lf = coeff_is_low_frequency(entry, config.plane, config.tx_class);
        let level = read_base_symbol(cdfs, symbols, entry, is_lf, &block, first_pass, config)?;
        finish_level(
            cdfs,
            symbols,
            entry,
            is_lf,
            level,
            &mut block,
            &mut first_pass,
            &mut masks,
            config,
        )?;
    }

    Ok(NonZeroCoeffBaseDerivedLevelPass {
        first_pass,
        masks,
        block,
    })
}

/// Adds the base-range symbol when the base symbol saturates, then records
/// the level.
#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::inline_always,
    reason = "measured: keeps the typed symbol reads inside the level loop"
)]
#[inline(always)]
fn finish_level(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    entry: CoeffScanEntry,
    is_lf: bool,
    mut level: u32,
    block: &mut TransformCoeffBlockState,
    first_pass: &mut CoeffBaseFirstPassSummary,
    masks: &mut CoeffLevelMasks,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<(), CoeffBaseDerivedLevelPassError> {
    let base_levels = if is_lf {
        LF_NUM_BASE_LEVELS
    } else {
        NUM_BASE_LEVELS
    };
    if level > base_levels && !(is_lf && config.plane > 0) {
        level += read_br_symbol(cdfs, symbols, entry, is_lf, block, config)?;
    }
    masks.record(entry.scan_index(), level, first_pass.tcq_state);
    first_pass.update_after_level(entry, level, config)?;
    block.set_level(entry.row(), entry.col(), level)?;
    Ok(())
}

fn preflight_pass(
    eob_read: NonZeroCoeffEob,
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<(), CoeffBaseDerivedLevelPassError> {
    if config.parity_hiding && config.use_tcq {
        return Err(CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq);
    }
    if block.width() != config.tx_width || block.height() != config.tx_height {
        return Err(CoeffBaseDerivedLevelPassError::BlockGeometryMismatch {
            block_width: block.width(),
            block_height: block.height(),
            config_width: config.tx_width,
            config_height: config.tx_height,
        });
    }
    if 1usize.checked_shl(config.tx_width_log2) != Some(config.tx_width) {
        return Err(CoeffBaseDerivedLevelPassError::TxWidthLog2Mismatch {
            tx_width_log2: config.tx_width_log2,
            tx_width: config.tx_width,
        });
    }

    let eob = eob_read.eob();
    if walk.len() > SCAN_MASK_WORDS * 64 {
        return Err(CoeffBaseDerivedLevelPassError::ScanTooLong {
            entries: walk.len(),
        });
    }
    if eob != walk.len() {
        return Err(CoeffBaseDerivedLevelPassError::ScanEntryCountMismatch {
            eob,
            entries: walk.len(),
        });
    }
    Ok(())
}

fn base_eob_selector(
    entry: CoeffScanEntry,
    is_lf: bool,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffCdfSelector {
    let ctx = coeff_base_eob_ctx(entry.scan_index(), config.tx_width_log2, config.tx_height);
    if config.plane > 0 {
        if is_lf {
            CoeffCdfSelector::BaseLfEobUv {
                coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
                ctx,
            }
        } else {
            CoeffCdfSelector::BaseEobUv {
                coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
                ctx,
            }
        }
    } else if is_lf {
        CoeffCdfSelector::BaseLfEob {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
        }
    } else {
        CoeffCdfSelector::BaseEob {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::inline_always,
    reason = "measured: keeps the typed symbol reads inside the level loop"
)]
#[inline(always)]
fn read_base_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    entry: CoeffScanEntry,
    is_lf: bool,
    block: &TransformCoeffBlockState,
    first_pass: CoeffBaseFirstPassSummary,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<u32, CoeffBaseDerivedLevelPassError> {
    let selection = CoeffBaseContext {
        row: entry.row(),
        col: entry.col(),
        stride: block.level_stride(),
        plane: config.plane,
        is_lf,
        is_hidden: first_pass.is_hidden,
        c: entry.scan_index(),
        tx_class: tx_class_index(config.tx_class),
    }
    .select(block.level());
    let tcq_ctx = (first_pass.tcq_state >> 1) & 1;
    let row = cdfs
        .coeff_rows_mut()
        .base_row(
            selection,
            config.coeff_cdf_q_ctx,
            config.tx_size_ctx,
            tcq_ctx,
        )
        .map_err(read_error)?;
    let symbol = match row {
        CoeffBaseRow::Base(row) => symbols.read_symbol_u16_array(row),
        CoeffBaseRow::Lf(row) => symbols.read_symbol_u16_array(row),
    }
    .map_err(read_error)?;
    Ok(u32::from(symbol.get()))
}

#[allow(
    clippy::inline_always,
    reason = "measured: keeps the typed symbol reads inside the level loop"
)]
#[inline(always)]
fn read_br_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    entry: CoeffScanEntry,
    is_lf: bool,
    block: &TransformCoeffBlockState,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<u32, CoeffBaseDerivedLevelPassError> {
    let ctx = CoeffBrContext {
        row: entry.row(),
        col: entry.col(),
        stride: block.level_stride(),
        plane: config.plane,
        is_lf,
        tx_class: tx_class_index(config.tx_class),
    }
    .ctx(block.level());
    let row = cdfs
        .coeff_rows_mut()
        .br_row(config.coeff_cdf_q_ctx, config.plane, is_lf, ctx)
        .map_err(read_error)?;
    let symbol = symbols.read_symbol_u16_array(row).map_err(read_error)?;
    Ok(u32::from(symbol.get()))
}

fn read_error(error: impl Into<BlockSymbolTraceReadError>) -> CoeffBaseDerivedLevelPassError {
    CoeffBaseSymbolReadError::from(error.into()).into()
}

const fn tx_class_index(tx_class: CoeffTransformClass) -> usize {
    match tx_class {
        CoeffTransformClass::TwoD => 0,
        CoeffTransformClass::Horizontal => 1,
        CoeffTransformClass::Vertical => 2,
    }
}

#[cfg(test)]
#[path = "base_level_pass_tests.rs"]
mod tests;
