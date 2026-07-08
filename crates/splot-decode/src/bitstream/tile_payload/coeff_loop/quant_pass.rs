// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient quant pass composition.
//!
//! Feature tracking: `DECODE-COEFF-QUANT-PASS-COMPOSE`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::max_level::{
    CoeffMaxLevelConfig, CoeffMaxLevelError, CoeffTransformClass, derive_nonzero_coeff_max_levels,
    max_levels_to_quant_pass_inputs,
};
use super::quant_state::{
    CoeffQuantStateConfig, CoeffQuantStateWriteError, NonZeroCoeffQuantState,
    apply_nonzero_coeff_quant_state,
};
use super::read_quant::{
    CoeffReadQuant, CoeffReadQuantConfig, CoeffReadQuantError, CoeffReadQuantInput,
    read_nonzero_coeff_quants,
};
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};
use super::sign_symbol::{CoeffSignRead, CoeffSignReadSymbol};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantPassConfig {
    pub(crate) is_hidden: bool,
    pub(crate) sum_abs1: u32,
    pub(crate) use_tcq: bool,
    pub(crate) lossless: bool,
    pub(crate) hr_level_avg: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantPassMaxLevelConfig {
    pub(crate) plane: usize,
    pub(crate) tx_class: CoeffTransformClass,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantPassInput {
    pub(crate) entry: CoeffScanEntry,
    pub(crate) max_level: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffQuantPass {
    read_quants: Vec<CoeffReadQuant>,
    quant_state: NonZeroCoeffQuantState,
}

impl NonZeroCoeffQuantPass {
    pub(crate) fn from_interleaved_parts(
        read_quants: Vec<CoeffReadQuant>,
        quant_state: NonZeroCoeffQuantState,
    ) -> Self {
        Self {
            read_quants,
            quant_state,
        }
    }

    #[must_use]
    pub(crate) fn read_quants(&self) -> &[CoeffReadQuant] {
        &self.read_quants
    }

    #[must_use]
    pub(crate) const fn quant_state(&self) -> &NonZeroCoeffQuantState {
        &self.quant_state
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffQuantPassError {
    #[error("coefficient quant pass enabled hidden parity with TCQ or lossless facts")]
    InconsistentHiddenParityConfig { use_tcq: bool, lossless: bool },
    #[error("coefficient quant pass enabled TCQ for a lossless block")]
    InconsistentTcqConfig,
    #[error("coefficient quant pass sign count {signs} does not match scan entries {entries}")]
    SignCountMismatch { signs: usize, entries: usize },
    #[error(
        "coefficient quant pass max-level input count {inputs} does not match scan entries {entries}"
    )]
    InputCountMismatch { inputs: usize, entries: usize },
    #[error(
        "coefficient quant pass sign {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    SignEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error(
        "coefficient quant pass input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    InputEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error(
        "coefficient quant pass sign {index} carried level {actual}, expected local level {expected}"
    )]
    SignLevelMismatch {
        index: usize,
        expected: u32,
        actual: u32,
    },
    #[error("coefficient quant pass input {index} skipped required hidden-parity sign")]
    HiddenParityMissingSign { index: usize, entry: CoeffScanEntry },
    #[error("coefficient quant pass input {index} has invalid maxLevel {max_level}")]
    InvalidMaxLevel {
        index: usize,
        max_level: u32,
        use_tcq: bool,
    },
    #[error("coefficient quant pass state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient quant pass allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("coefficient quant pass maxLevel derivation failed: {0}")]
    MaxLevel(#[from] CoeffMaxLevelError),
    #[error("coefficient quant pass read_quant failed: {0}")]
    ReadQuant(#[from] CoeffReadQuantError),
    #[error("coefficient quant pass write failed: {0}")]
    QuantState(#[from] CoeffQuantStateWriteError),
}

pub(crate) fn apply_nonzero_coeff_quant_pass(
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantPassInput],
    config: CoeffQuantPassConfig,
) -> Result<NonZeroCoeffQuantPass, CoeffQuantPassError> {
    validate_coeff_quant_pass_config(config)?;

    let read_inputs = preflight_quant_pass(block, walk.entries(), signs, inputs, config)?;
    let read_quants = read_nonzero_coeff_quants(
        symbols,
        walk,
        &read_inputs,
        CoeffReadQuantConfig {
            is_hidden: config.is_hidden,
            allow_tcq: config.use_tcq,
            hr_level_avg: config.hr_level_avg,
        },
    )?;
    let mut quant_inputs = Vec::new();
    quant_inputs.try_reserve(read_quants.len())?;
    quant_inputs.extend(read_quants.iter().map(|read| read.quant_input()));
    let quant_state = apply_nonzero_coeff_quant_state(
        block,
        walk,
        signs,
        &quant_inputs,
        CoeffQuantStateConfig {
            is_hidden: config.is_hidden,
            sum_abs1: config.sum_abs1,
            use_tcq: config.use_tcq,
            lossless: config.lossless,
        },
    )?;

    Ok(NonZeroCoeffQuantPass {
        read_quants,
        quant_state,
    })
}

pub(crate) fn validate_coeff_quant_pass_config(
    config: CoeffQuantPassConfig,
) -> Result<(), CoeffQuantPassError> {
    if config.is_hidden && (config.use_tcq || config.lossless) {
        return Err(CoeffQuantPassError::InconsistentHiddenParityConfig {
            use_tcq: config.use_tcq,
            lossless: config.lossless,
        });
    }
    if config.lossless && config.use_tcq {
        return Err(CoeffQuantPassError::InconsistentTcqConfig);
    }
    Ok(())
}

pub(crate) fn apply_nonzero_coeff_quant_pass_with_derived_max_levels(
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    signs: &[CoeffSignRead],
    max_level_config: CoeffQuantPassMaxLevelConfig,
    config: CoeffQuantPassConfig,
) -> Result<NonZeroCoeffQuantPass, CoeffQuantPassError> {
    let levels = derive_nonzero_coeff_max_levels(
        walk,
        CoeffMaxLevelConfig {
            plane: max_level_config.plane,
            tx_class: max_level_config.tx_class,
            is_hidden: config.is_hidden,
        },
    )?;
    let inputs = max_levels_to_quant_pass_inputs(&levels)?;
    apply_nonzero_coeff_quant_pass(symbols, block, walk, signs, &inputs, config)
}

fn preflight_quant_pass(
    block: &TransformCoeffBlockState,
    entries: &[CoeffScanEntry],
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantPassInput],
    config: CoeffQuantPassConfig,
) -> Result<Vec<CoeffReadQuantInput>, CoeffQuantPassError> {
    if signs.len() != entries.len() {
        return Err(CoeffQuantPassError::SignCountMismatch {
            signs: signs.len(),
            entries: entries.len(),
        });
    }
    if inputs.len() != entries.len() {
        return Err(CoeffQuantPassError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }

    let mut read_inputs = Vec::new();
    read_inputs.try_reserve(entries.len())?;
    for (index, ((entry, sign), input)) in entries
        .iter()
        .copied()
        .zip(signs.iter().copied())
        .zip(inputs.iter().copied())
        .enumerate()
    {
        if sign.entry() != entry {
            return Err(CoeffQuantPassError::SignEntryMismatch {
                index,
                expected: entry,
                actual: sign.entry(),
            });
        }
        if input.entry != entry {
            return Err(CoeffQuantPassError::InputEntryMismatch {
                index,
                expected: entry,
                actual: input.entry,
            });
        }

        let level = block.level_at(entry.row(), entry.col())?;
        if sign.level() != level {
            return Err(CoeffQuantPassError::SignLevelMismatch {
                index,
                expected: level,
                actual: sign.level(),
            });
        }
        if config.is_hidden
            && config.sum_abs1 > 0
            && entry.scan_index() == 0
            && sign.symbol() == CoeffSignReadSymbol::None
        {
            return Err(CoeffQuantPassError::HiddenParityMissingSign { index, entry });
        }
        input
            .max_level
            .checked_sub(u32::from(config.use_tcq))
            .ok_or(CoeffQuantPassError::InvalidMaxLevel {
                index,
                max_level: input.max_level,
                use_tcq: config.use_tcq,
            })?;
        block.quant_at(entry.pos())?;

        read_inputs.push(CoeffReadQuantInput {
            entry,
            level,
            max_level: input.max_level,
        });
    }
    Ok(read_inputs)
}

#[cfg(test)]
#[path = "quant_pass_tests.rs"]
mod tests;
