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

/// Base symbol row for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseSymbolSource {
    BaseEob { selector: CoeffCdfSelector },
    Base { selector: CoeffCdfSelector },
}

/// Base-range read policy for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseRangeRead {
    Disabled,
    Enabled { selector: CoeffCdfSelector },
}

/// Read facts for one checked scan-walk entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseSymbolReadInput {
    pub(crate) entry: CoeffScanEntry,
    pub(crate) base: CoeffBaseSymbolSource,
    pub(crate) base_levels: u32,
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
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    #[must_use]
    pub(crate) const fn base_symbol(self) -> u8 {
        self.base_symbol
    }

    #[must_use]
    pub(crate) const fn base_range_symbol(self) -> Option<u8> {
        self.base_range_symbol
    }

    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }
}

/// Error returned by the coefficient base symbol-read boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseSymbolReadError {
    #[error("coefficient base symbol input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch { inputs: usize, entries: usize },
    #[error(
        "coefficient base symbol input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error("coefficient base symbol read allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("coefficient base symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
}

/// Reads ordinary non-FSC §5.20.7.27 coefficient base symbols over a checked scan walk.
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

/// Reads one ordinary non-FSC §5.20.7.27 coefficient base/base-range symbol pair.
pub(crate) fn read_coeff_base_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffBaseSymbolReadInput,
) -> Result<CoeffBaseSymbolRead, CoeffBaseSymbolReadError> {
    let (base_selector, base_bias) = match input.base {
        CoeffBaseSymbolSource::BaseEob { selector } => (selector, 1),
        CoeffBaseSymbolSource::Base { selector } => (selector, 0),
    };
    let base_symbol = read_coeff_symbol(cdfs, symbols, base_selector)?;
    let mut level = u32::from(base_symbol) + base_bias;
    let base_range_symbol = match input.base_range {
        CoeffBaseRangeRead::Enabled { selector } if level > input.base_levels => {
            let symbol = read_coeff_symbol(cdfs, symbols, selector)?;
            level += u32::from(symbol);
            Some(symbol)
        }
        CoeffBaseRangeRead::Enabled { .. } | CoeffBaseRangeRead::Disabled => None,
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
