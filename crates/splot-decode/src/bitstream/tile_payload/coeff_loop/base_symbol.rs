// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base symbol reads.
//!
//! Feature tracking: `DECODE-COEFF-BASE-SYMBOL-READ`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseSymbolSource {
    BaseEob { selector: CoeffCdfSelector },
    Base { selector: CoeffCdfSelector },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffBaseRangeRead {
    Disabled,
    Enabled { selector: CoeffCdfSelector },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseSymbolReadInput {
    pub(crate) base: CoeffBaseSymbolSource,
    pub(crate) base_levels: u32,
    pub(crate) base_range: CoeffBaseRangeRead,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseSymbolReadError {
    #[error("coefficient base symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
}

pub(crate) fn read_coeff_base_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffBaseSymbolReadInput,
) -> Result<u32, CoeffBaseSymbolReadError> {
    let (base_selector, base_bias) = match input.base {
        CoeffBaseSymbolSource::BaseEob { selector } => (selector, 1),
        CoeffBaseSymbolSource::Base { selector } => (selector, 0),
    };
    let base_symbol = read_coeff_symbol(cdfs, symbols, base_selector)?;
    let mut level = u32::from(base_symbol) + base_bias;
    match input.base_range {
        CoeffBaseRangeRead::Enabled { selector } if level > input.base_levels => {
            let symbol = read_coeff_symbol(cdfs, symbols, selector)?;
            level += u32::from(symbol);
        }
        CoeffBaseRangeRead::Enabled { .. } | CoeffBaseRangeRead::Disabled => {}
    }
    Ok(level)
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
