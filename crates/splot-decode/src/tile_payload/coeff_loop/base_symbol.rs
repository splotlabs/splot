// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base symbol reads.
//!
//! Feature tracking: `DECODE-COEFF-BASE-SYMBOL-READ`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

/// Caller-selected base symbol row for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseSymbolSource {
    /// Read `coeff_base_eob`; the decoded level is `symbol + 1`.
    BaseEob {
        /// Caller-resolved CDF selector.
        selector: CoeffCdfSelector,
    },
    /// Read `coeff_base`; the decoded level is `symbol`.
    Base {
        /// Caller-resolved CDF selector.
        selector: CoeffCdfSelector,
    },
}

impl CoeffBaseSymbolSource {
    fn selector(self) -> CoeffCdfSelector {
        match self {
            Self::BaseEob { selector } | Self::Base { selector } => selector,
        }
    }

    fn level_from_symbol(self, symbol: u8) -> u32 {
        match self {
            Self::BaseEob { .. } => u32::from(symbol) + 1,
            Self::Base { .. } => u32::from(symbol),
        }
    }
}

/// Caller-selected base-range read policy for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseRangeRead {
    /// Do not read `coeff_br` even when the base level crosses the threshold.
    Disabled,
    /// Read the selected `coeff_br` row when `level > base_levels`.
    Enabled {
        /// Caller-resolved CDF selector.
        selector: CoeffCdfSelector,
    },
}

impl CoeffBaseRangeRead {
    fn selector(self) -> Option<CoeffCdfSelector> {
        match self {
            Self::Disabled => None,
            Self::Enabled { selector } => Some(selector),
        }
    }
}

/// Caller-resolved read facts for one checked scan-walk entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseSymbolReadInput {
    /// Checked scan entry this input belongs to.
    pub(crate) entry: CoeffScanEntry,
    /// Base or base-EOB CDF selector and level bias.
    pub(crate) base: CoeffBaseSymbolSource,
    /// Caller-resolved `baseLevels` threshold from AV2 §5.20.7.27.
    pub(crate) base_levels: u32,
    /// Optional base-range read selector.
    pub(crate) base_range: CoeffBaseRangeRead,
}

/// Decoded base/base-range symbols for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseSymbolRead {
    entry: CoeffScanEntry,
    base_symbol: u8,
    base_range_symbol: Option<u8>,
    level: u32,
}

impl CoeffBaseSymbolRead {
    /// Checked scan entry associated with this read.
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    /// Raw decoded `coeff_base_eob` or `coeff_base` symbol.
    #[must_use]
    pub(crate) const fn base_symbol(self) -> u8 {
        self.base_symbol
    }

    /// Raw decoded `coeff_br` symbol when the base-range branch was reached.
    #[must_use]
    pub(crate) const fn base_range_symbol(self) -> Option<u8> {
        self.base_range_symbol
    }

    /// Level after applying the base-EOB bias and any decoded base-range symbol.
    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }
}

/// Error returned by the coefficient base symbol-read boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseSymbolReadError {
    /// The number of caller inputs did not match the checked scan walk.
    #[error("coefficient base symbol input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch {
        /// Caller-provided input count.
        inputs: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// One caller input was not paired with the matching checked scan entry.
    #[error(
        "coefficient base symbol input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        /// Input index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Caller-provided scan entry.
        actual: CoeffScanEntry,
    },
    /// Allocation for decoded coefficient base symbol records failed.
    #[error("coefficient base symbol read allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// CDF row selection or AV2 §8.2 symbol decoding failed.
    #[error("coefficient base symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
}

/// Reads ordinary non-FSC §5.20.7.27 coefficient base symbols over a checked scan walk.
///
/// The caller owns all §8.3.2 context and selector derivation. This helper only
/// enforces the read order over the already checked scan entries:
/// `coeff_base_eob`/`coeff_base` first, then conditional `coeff_br` when the
/// decoded level is above the caller-provided `baseLevels` threshold
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27` and
/// `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It returns decoded
/// level-building symbols, but does not write `Level[]`, `QuantSign[]`,
/// `Quant[]`, tile context lines, or reconstruction state.
pub(crate) fn read_nonzero_coeff_base_symbols(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    walk: &NonZeroCoeffScanWalk,
    inputs: &[CoeffBaseSymbolReadInput],
) -> Result<Vec<CoeffBaseSymbolRead>, CoeffBaseSymbolReadError> {
    let entries = walk.entries();
    if inputs.len() != entries.len() {
        return Err(CoeffBaseSymbolReadError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }

    let mut reads = Vec::new();
    reads.try_reserve(entries.len())?;
    for (index, (entry, input)) in entries
        .iter()
        .copied()
        .zip(inputs.iter().copied())
        .enumerate()
    {
        if input.entry != entry {
            return Err(CoeffBaseSymbolReadError::ScanEntryMismatch {
                index,
                expected: entry,
                actual: input.entry,
            });
        }
        reads.push(read_coeff_base_symbol(cdfs, symbols, input)?);
    }
    Ok(reads)
}

fn read_coeff_base_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffBaseSymbolReadInput,
) -> Result<CoeffBaseSymbolRead, CoeffBaseSymbolReadError> {
    let base_symbol = read_coeff_symbol(cdfs, symbols, input.base.selector())?;
    let mut level = input.base.level_from_symbol(base_symbol);
    let base_range_symbol = if level > input.base_levels
        && let Some(selector) = input.base_range.selector()
    {
        let symbol = read_coeff_symbol(cdfs, symbols, selector)?;
        level += u32::from(symbol);
        Some(symbol)
    } else {
        None
    };
    Ok(CoeffBaseSymbolRead {
        entry: input.entry,
        base_symbol,
        base_range_symbol,
        level,
    })
}

fn read_coeff_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: CoeffCdfSelector,
) -> Result<u8, CoeffBaseSymbolReadError> {
    Ok(cdfs
        .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
        .get())
}
