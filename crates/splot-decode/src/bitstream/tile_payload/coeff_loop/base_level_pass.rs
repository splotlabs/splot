// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base/level first pass with derived contexts.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfArray::{
    CoeffBase, CoeffBaseLf, CoeffBaseLfUv, CoeffBasePh, CoeffBaseUv, CoeffBr, CoeffBrLf, CoeffBrUv,
};
use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::{
    CoeffBaseContext, CoeffBaseSelection, CoeffBrContext, coeff_base_eob_ctx,
};
use super::super::cdf::{
    COEFF_BR_ROW_LEN, CoeffBaseLevelRows, CoeffBaseLfLevelRows, CoeffBaseLfUvCdfRows,
    CoeffBasePhCdfRows, CoeffBaseRow, CoeffBaseUvCdfRows, CoeffBrCdfRows, CoeffBrLfCdfRows,
    CoeffBrUvCdfRows, CoeffCdfSelector, CoeffLevelRows, TileCdfError, TileCdfSubset, row2,
};
use super::super::coeff_state::{
    LEVEL_GRID_LEN, LEVEL_STRIDE, LevelGrid, TileCoeffStateError, TransformCoeffBlockState,
};
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
    pub(crate) tx_size_ctx: usize,
    pub(crate) tx_width_log2: u32,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
    pub(crate) plane: usize,
    pub(crate) tx_class: CoeffTransformClass,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
}

/// The base pass's cross-coefficient state packed into one register-sized
/// value: bits 0-2 `tcqState`, bit 3 the `sumAbs1` parity, bits 4-7 the
/// nonzero count capped below `is_hidden`'s threshold decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CoeffBaseFirstPassSummary {
    packed: u32,
}

const SUMMARY_TCQ_MASK: u32 = 0b111;
const SUMMARY_PARITY_SHIFT: u32 = 3;
const SUMMARY_COUNT_SHIFT: u32 = 4;
const SUMMARY_COUNT_MASK: u32 = 0b1111;

impl CoeffBaseFirstPassSummary {
    #[must_use]
    pub(crate) const fn sum_abs1(self) -> u32 {
        (self.packed >> SUMMARY_PARITY_SHIFT) & 1
    }

    #[must_use]
    pub(crate) const fn is_hidden(self) -> bool {
        ((self.packed >> SUMMARY_COUNT_SHIFT) & SUMMARY_COUNT_MASK) >= PHTHRESH as u32
    }

    const fn tcq_state(self) -> usize {
        (self.packed & SUMMARY_TCQ_MASK) as usize
    }

    #[allow(clippy::inline_always, reason = "measured: per-coefficient update")]
    #[inline(always)]
    pub(crate) fn update_after_level<const TCQ: bool, const PH: bool>(
        &mut self,
        entry: CoeffScanEntry,
        level: u32,
    ) {
        if TCQ {
            let tcq_state = next_tcq_state(self.tcq_state(), level);
            self.packed = (self.packed & !SUMMARY_TCQ_MASK) | tcq_state as u32;
        }
        if PH && level != 0 && entry.scan_index() > 0 {
            let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
            let parity = ((self.packed >> SUMMARY_PARITY_SHIFT) & 1) ^ (clipped & 1);
            let count = ((self.packed >> SUMMARY_COUNT_SHIFT) & SUMMARY_COUNT_MASK)
                .min(SUMMARY_COUNT_MASK - 1);
            self.packed = (self.packed
                & !(SUMMARY_COUNT_MASK << SUMMARY_COUNT_SHIFT)
                & !(1 << SUMMARY_PARITY_SHIFT))
                | (parity << SUMMARY_PARITY_SHIFT)
                | ((count + 1) << SUMMARY_COUNT_SHIFT);
        }
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
    #[inline]
    fn record(&mut self, scan_index: usize, level: u32, tcq_state: usize) {
        let word = (scan_index >> 6) & (SCAN_MASK_WORDS - 1);
        let shift = scan_index & 63;
        self.nonzero[word] |= u64::from(level != 0) << shift;
        self.tcq_q0[word] |= (((tcq_state >> 1) & 1) as u64) << shift;
    }

    #[inline]
    pub(crate) fn is_nonzero(&self, scan_index: usize) -> bool {
        (self.nonzero[(scan_index >> 6) & (SCAN_MASK_WORDS - 1)] >> (scan_index & 63)) & 1 != 0
    }

    #[inline]
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
    #[error("coefficient base/level block has no level grid")]
    LevelGridUnavailable,
    #[error("coefficient base/level config cannot enable parity hiding and TCQ together")]
    InconsistentParityAndTcq,
    #[error("coefficient base/level base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
    #[error("coefficient base/level state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Kept out of line: inlined into the ordinary-branch function, the level
/// loop shares that function's registers and spills its context state.
///
#[inline(never)]
pub(crate) fn apply_nonzero_coeff_base_derived_level_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    start: NonZeroCoeffBlockStart,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    block: &mut TransformCoeffBlockState,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let (eob_read, owned_block) = start.into_parts();
    *block = owned_block;
    preflight_pass(eob_read, block, walk, config)?;
    match (config.use_tcq, config.parity_hiding) {
        (false, false) => {
            dispatch_level_pass::<false, false>(cdfs, symbols, block, walk, config, masks)
        }
        (false, true) => {
            dispatch_level_pass::<false, true>(cdfs, symbols, block, walk, config, masks)
        }
        (true, _) => dispatch_level_pass::<true, false>(cdfs, symbols, block, walk, config, masks),
    }
}

/// Splits the class and plane dispatch so each pass instantiation folds the
/// context derivation and the summary update to one straight-line body.
#[inline(always)]
fn dispatch_level_pass<const TCQ: bool, const PH: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    use CoeffTransformClass::{Horizontal, TwoD, Vertical};
    match (config.tx_class, config.plane == 0) {
        (TwoD, true) => level_pass::<0, true, TCQ, PH>(cdfs, symbols, block, walk, config, masks),
        (TwoD, false) => level_pass::<0, false, TCQ, PH>(cdfs, symbols, block, walk, config, masks),
        (Horizontal, true) => {
            level_pass::<1, true, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
        (Horizontal, false) => {
            level_pass::<1, false, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
        (Vertical, true) => {
            level_pass::<2, true, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
        (Vertical, false) => {
            level_pass::<2, false, TCQ, PH>(cdfs, symbols, block, walk, config, masks)
        }
    }
}

/// The level loop for one transform class and plane type, so every class
/// and plane branch of the context derivation folds away.
#[allow(
    clippy::inline_always,
    reason = "measured: one specialised loop per class and plane type"
)]
#[inline(always)]
fn level_pass<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let mut owned = symbols.snapshot();
    let result = level_pass_on_snapshot::<CLASS, LUMA, TCQ, PH>(
        cdfs, &mut owned, block, walk, config, masks,
    );
    symbols.restore(owned);
    result
}

/// The loop body over an owned decoder, so the arithmetic state stays in
/// registers instead of behind a pointer.
#[allow(
    clippy::inline_always,
    reason = "measured: merges the owned decoder into the specialised loop"
)]
#[inline(always)]
fn level_pass_on_snapshot<const CLASS: usize, const LUMA: bool, const TCQ: bool, const PH: bool>(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    config: CoeffBaseDerivedLevelPassConfig,
    masks: &mut CoeffLevelMasks,
) -> Result<CoeffBaseFirstPassSummary, CoeffBaseDerivedLevelPassError> {
    let config = CoeffBaseDerivedLevelPassConfig {
        tx_class: [
            CoeffTransformClass::TwoD,
            CoeffTransformClass::Horizontal,
            CoeffTransformClass::Vertical,
        ][CLASS],
        plane: if LUMA { 0 } else { config.plane.max(1) },
        ..config
    };
    let mut first_pass = CoeffBaseFirstPassSummary::default();
    *masks = CoeffLevelMasks::default();
    let mut entries = walk.entries();
    let Some(eob_entry) = entries.next() else {
        return Ok(first_pass);
    };
    let eob_is_lf = coeff_is_low_frequency(eob_entry, config.plane, config.tx_class);
    let selector = base_eob_selector(eob_entry, eob_is_lf, config);
    let eob_level = u32::from(read_coeff_symbol(cdfs, symbols, selector)?) + 1;

    let Some(grid) = block.level_grid_mut() else {
        return Err(CoeffBaseDerivedLevelPassError::LevelGridUnavailable);
    };
    let CoeffLevelRows {
        base,
        base_lf,
        base_ph,
        base_uv,
        base_lf_uv,
        br,
        br_lf,
        br_uv,
    } = cdfs
        .coeff_rows_mut()
        .level_rows(config.tx_size_ctx)
        .map_err(read_error)?;
    let mut pass = LevelPassState {
        grid,
        first_pass: &mut first_pass,
        masks,
        config,
        base,
        base_lf,
        base_ph,
        base_uv,
        base_lf_uv,
        br,
        br_lf,
        br_uv,
    };
    pass.finish_level::<TCQ, PH>(symbols, eob_entry, eob_is_lf, eob_level)?;
    let dc = entries.next_back();
    for entry in entries {
        let is_lf = coeff_is_low_frequency(entry, config.plane, config.tx_class);
        let level = pass.read_base_symbol::<false>(symbols, entry, is_lf)?;
        pass.finish_level::<TCQ, PH>(symbols, entry, is_lf, level)?;
    }
    if let Some(entry) = dc {
        let is_lf = coeff_is_low_frequency(entry, config.plane, config.tx_class);
        let level = pass.read_base_symbol::<true>(symbols, entry, is_lf)?;
        pass.finish_level::<TCQ, PH>(symbols, entry, is_lf, level)?;
    }

    Ok(first_pass)
}

/// What the level loop reads and writes besides the symbol decoder: the
/// level grid, the pass summary, the masks, and the CDF row arrays it
/// selects rows from, each a register-resident pointer.
struct LevelPassState<'a> {
    grid: &'a mut LevelGrid,
    first_pass: &'a mut CoeffBaseFirstPassSummary,
    masks: &'a mut CoeffLevelMasks,
    config: CoeffBaseDerivedLevelPassConfig,
    base: &'a mut CoeffBaseLevelRows,
    base_lf: &'a mut CoeffBaseLfLevelRows,
    base_ph: &'a mut CoeffBasePhCdfRows,
    base_uv: &'a mut CoeffBaseUvCdfRows,
    base_lf_uv: &'a mut CoeffBaseLfUvCdfRows,
    br: &'a mut CoeffBrCdfRows,
    br_lf: &'a mut CoeffBrLfCdfRows,
    br_uv: &'a mut CoeffBrUvCdfRows,
}

impl LevelPassState<'_> {
    /// Reads the base symbol of a non-EOB coefficient; `DC` is whether it
    /// is scan index 0, the only one whose context the hidden parity
    /// changes.
    #[allow(
        clippy::inline_always,
        reason = "measured: keeps the typed symbol reads inside the level loop"
    )]
    #[inline(always)]
    fn read_base_symbol<const DC: bool>(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        entry: CoeffScanEntry,
        is_lf: bool,
    ) -> Result<u32, CoeffBaseDerivedLevelPassError> {
        let selection = CoeffBaseContext {
            row: entry.row(),
            col: entry.col(),
            stride: LEVEL_STRIDE,
            plane: self.config.plane,
            is_lf,
            is_hidden: DC && self.first_pass.is_hidden(),
            c: usize::from(!DC),
            tx_class: tx_class_index(self.config.tx_class),
        }
        .select(self.grid);
        let tcq_ctx = (self.first_pass.tcq_state() >> 1) & 1;
        let row = self
            .base_selection_row(selection, tcq_ctx)
            .map_err(read_error)?;
        Ok(u32::from(
            match row {
                CoeffBaseRow::Base(row) => symbols.read_symbol_u16_array(row),
                CoeffBaseRow::Lf(row) => symbols.read_symbol_u16_array(row),
            }
            .map_err(read_error)?
            .get(),
        ))
    }

    /// The base row of a derived selection, over the register-resident row
    /// pointers instead of a moved row-set struct.
    #[allow(
        clippy::inline_always,
        reason = "measured: per-coefficient row fetch in the level loop"
    )]
    #[inline(always)]
    fn base_selection_row(
        &mut self,
        selection: CoeffBaseSelection,
        tcq_ctx: usize,
    ) -> Result<CoeffBaseRow<'_>, TileCdfError> {
        Ok(match selection {
            CoeffBaseSelection::Hf { ctx } => {
                CoeffBaseRow::Base(&mut row2(self.base, CoeffBase, ctx)?[tcq_ctx])
            }
            CoeffBaseSelection::Ph { ctx } => {
                CoeffBaseRow::Base(row2(self.base_ph, CoeffBasePh, ctx)?)
            }
            CoeffBaseSelection::Uv { ctx } => {
                CoeffBaseRow::Base(row2(self.base_uv, CoeffBaseUv, ctx)?)
            }
            CoeffBaseSelection::Lf { ctx } => {
                CoeffBaseRow::Lf(&mut row2(self.base_lf, CoeffBaseLf, ctx)?[tcq_ctx])
            }
            CoeffBaseSelection::LfUv { ctx } => {
                CoeffBaseRow::Lf(row2(self.base_lf_uv, CoeffBaseLfUv, ctx)?)
            }
        })
    }

    /// Adds the base-range symbol when the base symbol saturates, then
    /// records the level.
    #[allow(
        clippy::inline_always,
        reason = "measured: keeps the typed symbol reads inside the level loop"
    )]
    #[inline(always)]
    fn finish_level<const TCQ: bool, const PH: bool>(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        entry: CoeffScanEntry,
        is_lf: bool,
        mut level: u32,
    ) -> Result<(), CoeffBaseDerivedLevelPassError> {
        let config = self.config;
        if level != 0 {
            let base_levels = if is_lf {
                LF_NUM_BASE_LEVELS
            } else {
                NUM_BASE_LEVELS
            };
            if level > base_levels && !(is_lf && config.plane > 0) {
                let ctx = CoeffBrContext {
                    row: entry.row(),
                    col: entry.col(),
                    stride: LEVEL_STRIDE,
                    plane: config.plane,
                    is_lf,
                    tx_class: tx_class_index(config.tx_class),
                }
                .ctx(self.grid);
                let row = self.br_row(config.plane, is_lf, ctx).map_err(read_error)?;
                level += u32::from(
                    symbols
                        .read_symbol_u16_array(row)
                        .map_err(read_error)?
                        .get(),
                );
            }
            self.masks
                .record(entry.scan_index(), level, self.first_pass.tcq_state());
            let index = entry.row() * LEVEL_STRIDE + entry.col();
            self.grid[index & (LEVEL_GRID_LEN - 1)] = level as u8;
        }
        self.first_pass.update_after_level::<TCQ, PH>(entry, level);
        Ok(())
    }

    /// The base-range row for `plane` and `is_lf`, over the
    /// register-resident row pointers.
    fn br_row(
        &mut self,
        plane: usize,
        is_lf: bool,
        ctx: usize,
    ) -> Result<&mut [u16; COEFF_BR_ROW_LEN], TileCdfError> {
        if plane > 0 {
            row2(self.br_uv, CoeffBrUv, ctx)
        } else if is_lf {
            row2(self.br_lf, CoeffBrLf, ctx)
        } else {
            row2(self.br, CoeffBr, ctx)
        }
    }
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
            CoeffCdfSelector::BaseLfEobUv { ctx }
        } else {
            CoeffCdfSelector::BaseEobUv { ctx }
        }
    } else if is_lf {
        CoeffCdfSelector::BaseLfEob {
            tx_size: config.tx_size_ctx,
            ctx,
        }
    } else {
        CoeffCdfSelector::BaseEob {
            tx_size: config.tx_size_ctx,
            ctx,
        }
    }
}

fn read_error(error: impl Into<BlockSymbolTraceReadError>) -> CoeffBaseDerivedLevelPassError {
    CoeffBaseSymbolReadError::from(error.into()).into()
}

pub(crate) const fn tx_class_index(tx_class: CoeffTransformClass) -> usize {
    match tx_class {
        CoeffTransformClass::TwoD => 0,
        CoeffTransformClass::Horizontal => 1,
        CoeffTransformClass::Vertical => 2,
    }
}

#[cfg(test)]
#[path = "base_level_pass_tests.rs"]
mod tests;
