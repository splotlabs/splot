// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient sign reads.
//!
//! Feature tracking: `DECODE-COEFF-SIGN-SYMBOL-READ`,
//! `DECODE-COEFF-SIGN-SOURCE-DERIVE`.

use std::collections::TryReserveError;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::dc_sign_ctx;
use super::super::cdf::{TileCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::max_level::CoeffTransformClass;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffSignCdfSyntax {
    DcSign,
    DcSignHorzVert,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffDcSignSelector {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) plane_type: usize,
    pub(crate) group: usize,
    pub(crate) ctx: usize,
}

impl CoeffDcSignSelector {
    fn tile_selector(self) -> TileCdfSelector {
        TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            plane_type: self.plane_type,
            group: self.group,
            ctx: self.ctx,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffSignReadSource {
    None,
    Cdf {
        syntax: CoeffSignCdfSyntax,
        selector: CoeffDcSignSelector,
    },
    SignBit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffSignReadInput {
    pub(crate) entry: CoeffScanEntry,
    pub(crate) source: CoeffSignReadSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffSignSourceDeriveConfig<'a> {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) plane: usize,
    pub(crate) plane_type: usize,
    pub(crate) tx_class: CoeffTransformClass,
    pub(crate) is_hidden: bool,
    pub(crate) sum_abs1: u32,
    pub(crate) above_dc: &'a [u8],
    pub(crate) left_dc: &'a [u8],
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
}

impl CoeffSignSourceDeriveConfig<'_> {
    fn dc_selector(self, ctx: usize) -> CoeffDcSignSelector {
        CoeffDcSignSelector {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            plane_type: self.plane_type,
            group: usize::from(self.is_hidden),
            ctx,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffSignReadSymbol {
    None,
    Cdf {
        syntax: CoeffSignCdfSyntax,
        symbol: u8,
    },
    SignBit {
        bit: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffSignRead {
    entry: CoeffScanEntry,
    level: u32,
    symbol: CoeffSignReadSymbol,
    sign: bool,
}

impl CoeffSignRead {
    #[cfg(test)]
    pub(crate) const fn for_test(
        entry: CoeffScanEntry,
        level: u32,
        symbol: CoeffSignReadSymbol,
        sign: bool,
    ) -> Self {
        Self {
            entry,
            level,
            symbol,
            sign,
        }
    }

    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }

    #[must_use]
    pub(crate) const fn symbol(self) -> CoeffSignReadSymbol {
        self.symbol
    }

    #[must_use]
    pub(crate) const fn sign(self) -> bool {
        self.sign
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffSignReadError {
    #[error("coefficient sign input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch { inputs: usize, entries: usize },
    #[error(
        "coefficient sign input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error("coefficient sign input {index} disabled sign read for nonzero level {level}")]
    MissingRequiredSign {
        index: usize,
        entry: CoeffScanEntry,
        level: u32,
    },
    #[error("coefficient sign read state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient sign read allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("coefficient sign symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
    #[error("coefficient sign literal read failed: {source}")]
    LiteralRead {
        #[source]
        source: CoreError,
    },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffSignSourceDeriveError {
    #[error("coefficient sign-source derivation state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient sign-source derivation allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
}

pub(crate) fn derive_nonzero_coeff_sign_inputs(
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    config: CoeffSignSourceDeriveConfig<'_>,
) -> Result<Vec<CoeffSignReadInput>, CoeffSignSourceDeriveError> {
    let entries = walk.entries();
    let mut inputs = Vec::new();
    inputs.try_reserve(entries.len())?;
    for entry in entries.iter().copied() {
        let level = block.level_at(entry.row(), entry.col())?;
        inputs.push(CoeffSignReadInput {
            entry,
            source: derive_coeff_sign_source(entry, level, config),
        });
    }
    Ok(inputs)
}

fn derive_coeff_sign_source(
    entry: CoeffScanEntry,
    level: u32,
    config: CoeffSignSourceDeriveConfig<'_>,
) -> CoeffSignReadSource {
    if level == 0 && !(config.is_hidden && entry.scan_index() == 0 && config.sum_abs1 > 0) {
        return CoeffSignReadSource::None;
    }

    if entry.row() == 0 && entry.col() == 0 && config.plane == 0 {
        let ctx = dc_sign_ctx(
            config.above_dc,
            config.left_dc,
            config.x4,
            config.y4,
            config.w4,
            config.h4,
        );
        return CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSign,
            selector: config.dc_selector(ctx),
        };
    }

    let uses_axis_cdf = config.plane == 0
        && match config.tx_class {
            CoeffTransformClass::Horizontal => entry.col() == 0,
            CoeffTransformClass::Vertical => entry.row() == 0,
            CoeffTransformClass::TwoD => false,
        };
    if uses_axis_cdf {
        return CoeffSignReadSource::Cdf {
            syntax: CoeffSignCdfSyntax::DcSignHorzVert,
            selector: config.dc_selector(0),
        };
    }

    CoeffSignReadSource::SignBit
}

pub(crate) fn read_nonzero_coeff_signs(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    inputs: &[CoeffSignReadInput],
) -> Result<Vec<CoeffSignRead>, CoeffSignReadError> {
    let entries = walk.entries();
    if inputs.len() != entries.len() {
        return Err(CoeffSignReadError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }

    let levels = preflight_nonzero_coeff_signs(block, walk, inputs)?;
    let mut reads = Vec::new();
    reads.try_reserve(entries.len())?;
    for (input, level) in inputs.iter().copied().zip(levels) {
        reads.push(read_preflighted_nonzero_coeff_sign(
            cdfs, symbols, input, level,
        )?);
    }
    Ok(reads)
}

pub(crate) fn preflight_nonzero_coeff_signs(
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    inputs: &[CoeffSignReadInput],
) -> Result<Vec<u32>, CoeffSignReadError> {
    let entries = walk.entries();
    if inputs.len() != entries.len() {
        return Err(CoeffSignReadError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }

    let mut levels = Vec::new();
    levels.try_reserve(entries.len())?;
    for (index, (entry, input)) in entries
        .iter()
        .copied()
        .zip(inputs.iter().copied())
        .enumerate()
    {
        if input.entry != entry {
            return Err(CoeffSignReadError::ScanEntryMismatch {
                index,
                expected: entry,
                actual: input.entry,
            });
        }
        let level = block.level_at(entry.row(), entry.col())?;
        if level != 0 && input.source == CoeffSignReadSource::None {
            return Err(CoeffSignReadError::MissingRequiredSign {
                index,
                entry,
                level,
            });
        }
        levels.push(level);
    }
    Ok(levels)
}

pub(crate) fn read_preflighted_nonzero_coeff_sign(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffSignReadInput,
    level: u32,
) -> Result<CoeffSignRead, CoeffSignReadError> {
    let (symbol, sign) = match input.source {
        CoeffSignReadSource::None => (CoeffSignReadSymbol::None, false),
        CoeffSignReadSource::Cdf { syntax, selector } => {
            let symbol = cdfs
                .read_block_symbol_trace(selector.tile_selector(), symbols)?
                .get();
            (CoeffSignReadSymbol::Cdf { syntax, symbol }, symbol != 0)
        }
        CoeffSignReadSource::SignBit => {
            let bit = symbols
                .read_literal(1)
                .map_err(|source| CoeffSignReadError::LiteralRead { source })?
                != 0;
            (CoeffSignReadSymbol::SignBit { bit }, bit)
        }
    };
    Ok(CoeffSignRead {
        entry: input.entry,
        level,
        symbol,
        sign,
    })
}

#[cfg(test)]
#[path = "sign_symbol_tests.rs"]
mod tests;
