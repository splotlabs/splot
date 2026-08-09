// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unified block tokens and their production entropy encoder.

use super::*;

/// One symbol of the ordered general-intra block trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockSymbolToken {
    /// The root `do_split` token.
    Partition(PartitionToken),
    /// A mode-info token.
    Mode(IntraModeToken),
    /// A coefficient token.
    Coeff(CoefficientEntropyToken),
    /// An AV2 § 8.2.5 bypass literal.
    Bypass { width: u32, value: u32 },
}

impl BlockSymbolToken {
    /// Constructs a bypass literal.
    pub(crate) const fn bypass(width: u32, value: u32) -> Self {
        Self::Bypass { width, value }
    }

    /// Returns the CDF symbol or bypass value for trace-order assertions.
    #[cfg(test)]
    pub(crate) const fn symbol(self) -> u8 {
        match self {
            Self::Partition(token) => token.symbol(),
            Self::Mode(token) => token.symbol(),
            Self::Coeff(token) => token.symbol(),
            Self::Bypass { value, .. } => value as u8,
        }
    }
}

/// Encodes an ordered block trace into AV2 § 8.2 tile-data bytes.
pub(crate) fn encode_block_symbol_trace(trace: &[BlockSymbolToken]) -> Result<Vec<u8>> {
    let mut cdfs = BlockSymbolTraceCdfRows::from_defaults();
    let trace_cost = trace
        .iter()
        .map(|token| match token {
            BlockSymbolToken::Bypass { width, .. } => *width as usize,
            _ => 1,
        })
        .sum::<usize>();
    let budget = trace_cost.saturating_add(BLOCK_SYMBOL_TRACE_BUDGET_HEADROOM);
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new()
            .with_max_output_bytes(budget)
            .with_max_operations(budget),
    );
    for (index, token) in trace.iter().enumerate() {
        match token {
            BlockSymbolToken::Bypass { width, value } => encoder
                .write_literal(*value, *width)
                .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?,
            BlockSymbolToken::Partition(token) => encoder
                .write_symbol_u16(cdfs.partition_row_mut(), Symbol::new(token.symbol()))
                .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?,
            BlockSymbolToken::Mode(token) => encoder
                .write_symbol_u16(cdfs.mode_row_mut(*token), Symbol::new(token.symbol()))
                .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?,
            BlockSymbolToken::Coeff(token) => encoder
                .write_symbol_u16(
                    cdfs.coefficient_row_mut(*token),
                    Symbol::new(token.symbol()),
                )
                .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?,
        }
    }
    let output = encoder
        .finish()
        .map_err(|source| Error::BlockSymbolTraceSymbolEncodeFinish { source })?;
    Ok(output.into_bytes())
}
