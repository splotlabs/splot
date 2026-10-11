// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base symbol reads.
//!
//! Feature tracking: `DECODE-COEFF-BASE-SYMBOL-READ`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::{CoeffCdfSelector, TileCdfSubset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseSymbolReadError {
    #[error("coefficient base symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
}

pub(crate) fn read_coeff_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: CoeffCdfSelector,
) -> Result<u8, CoeffBaseSymbolReadError> {
    Ok(cdfs.read_coeff_symbol(selector, symbols)?)
}
