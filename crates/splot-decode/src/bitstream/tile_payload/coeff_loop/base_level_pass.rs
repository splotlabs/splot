// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base/level first pass with derived contexts.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::coeff_context::{
    CoeffBaseContext, CoeffBaseSelection, CoeffBrContext, coeff_base_eob_ctx,
};
use super::super::cdf::{CoeffCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEobSymbolRead;
use super::base_symbol::{
    CoeffBaseRangeRead, CoeffBaseSymbolReadError, CoeffBaseSymbolReadInput, CoeffBaseSymbolSource,
    read_coeff_base_symbol,
};
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
            self.tcq_state = next_tcq_state(self.tcq_state, level).ok_or(
                CoeffBaseDerivedLevelPassError::InvalidTcqState {
                    entry,
                    tcq_state: self.tcq_state,
                },
            )?;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBaseDerivedLevelPass {
    first_pass: CoeffBaseFirstPassSummary,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffBaseDerivedLevelPass {
    #[must_use]
    pub(crate) const fn first_pass(&self) -> CoeffBaseFirstPassSummary {
        self.first_pass
    }

    #[must_use]
    pub(crate) const fn block_mut(&mut self) -> &mut TransformCoeffBlockState {
        &mut self.block
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
    for (index, entry) in walk.entries().enumerate() {
        let input = derive_base_symbol_input(index, entry, &block, first_pass, config);
        let level = read_coeff_base_symbol(cdfs, symbols, input)?;
        first_pass.update_after_level(entry, level, config)?;
        block.set_level(entry.row(), entry.col(), level)?;
    }

    Ok(NonZeroCoeffBaseDerivedLevelPass { first_pass, block })
}

fn preflight_pass(
    eob_read: NonZeroCoeffEobSymbolRead,
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

    let eob = eob_read.eob().eob();
    if eob != walk.len() {
        return Err(CoeffBaseDerivedLevelPassError::ScanEntryCountMismatch {
            eob,
            entries: walk.len(),
        });
    }
    Ok(())
}

fn derive_base_symbol_input(
    index: usize,
    entry: CoeffScanEntry,
    block: &TransformCoeffBlockState,
    first_pass: CoeffBaseFirstPassSummary,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffBaseSymbolReadInput {
    let is_lf = coeff_is_low_frequency(entry, config.plane, config.tx_class);
    let base_levels = if is_lf {
        LF_NUM_BASE_LEVELS
    } else {
        NUM_BASE_LEVELS
    };
    let base = if index == 0 {
        CoeffBaseSymbolSource::BaseEob {
            selector: base_eob_selector(entry, is_lf, config),
        }
    } else {
        CoeffBaseSymbolSource::Base {
            selector: base_selector(entry, is_lf, block, first_pass, config),
        }
    };
    let base_range = if is_lf && config.plane > 0 {
        CoeffBaseRangeRead::Disabled
    } else {
        CoeffBaseRangeRead::Enabled {
            selector: base_range_selector(entry, is_lf, block, config),
        }
    };

    CoeffBaseSymbolReadInput {
        base,
        base_levels,
        base_range,
    }
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

fn base_selector(
    entry: CoeffScanEntry,
    is_lf: bool,
    block: &TransformCoeffBlockState,
    first_pass: CoeffBaseFirstPassSummary,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffCdfSelector {
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
    map_base_selection(selection, first_pass, config)
}

fn map_base_selection(
    selection: CoeffBaseSelection,
    first_pass: CoeffBaseFirstPassSummary,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffCdfSelector {
    let tcq_ctx = (first_pass.tcq_state >> 1) & 1;
    match selection {
        CoeffBaseSelection::Ph { ctx } => CoeffCdfSelector::BasePh {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        },
        CoeffBaseSelection::LfUv { ctx } => CoeffCdfSelector::BaseLfUv {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        },
        CoeffBaseSelection::Uv { ctx } => CoeffCdfSelector::BaseUv {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        },
        CoeffBaseSelection::Lf { ctx } => CoeffCdfSelector::BaseLf {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
            tcq_ctx,
        },
        CoeffBaseSelection::Hf { ctx } => CoeffCdfSelector::Base {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
            tcq_ctx,
        },
    }
}

fn base_range_selector(
    entry: CoeffScanEntry,
    is_lf: bool,
    block: &TransformCoeffBlockState,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffCdfSelector {
    let ctx = CoeffBrContext {
        row: entry.row(),
        col: entry.col(),
        stride: block.level_stride(),
        plane: config.plane,
        is_lf,
        tx_class: tx_class_index(config.tx_class),
    }
    .ctx(block.level());
    if config.plane > 0 {
        CoeffCdfSelector::BrUv {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }
    } else if is_lf {
        CoeffCdfSelector::BrLf {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }
    } else {
        CoeffCdfSelector::Br {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }
    }
}

const fn tx_class_index(tx_class: CoeffTransformClass) -> usize {
    match tx_class {
        CoeffTransformClass::TwoD => 0,
        CoeffTransformClass::Horizontal => 1,
        CoeffTransformClass::Vertical => 2,
    }
}
