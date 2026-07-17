// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 8.2 entropy-token roundtrip proof: writes coefficient token records
//! through `splot-core`'s symbol encoder and decodes them back, proving the token
//! values survive the § 8.2 coder with the scoped default CDF rows. Split out of
//! `coefficient_tokenization` to keep the parent file under the 1000-line budget.

use splot_core::symbol::{Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

use super::CoefficientEntropyToken;
use super::cdf_rows::CoefficientTokenCdfRows;
use crate::error::{Error, Result};

/// Result of proving token values through AV2 § 8.2 symbol bytes.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CoefficientTokenRoundtrip {
    bytes: Vec<u8>,
    decoded_symbols: Vec<u8>,
    symbol_count: u64,
}

impl CoefficientTokenRoundtrip {
    /// Returns finalized symbol payload bytes.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns decoded token symbols in order.
    pub(crate) fn decoded_symbols(&self) -> &[u8] {
        &self.decoded_symbols
    }

    /// Returns the decoder's final symbol count.
    pub(crate) const fn symbol_count(&self) -> u64 {
        self.symbol_count
    }
}

/// Writes token records with the § 8.2 symbol encoder and decodes them back.
pub(crate) fn roundtrip_entropy_tokens(
    tokens: &[CoefficientEntropyToken],
) -> Result<CoefficientTokenRoundtrip> {
    let mut encode_cdfs = CoefficientTokenCdfRows::from_defaults();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new()
            .with_max_output_bytes(64)
            .with_max_operations(64),
    );
    for token in tokens.iter().copied() {
        encoder
            .write_symbol_u16(
                encode_cdfs.row_mut(token.selector())?,
                Symbol::new(token.symbol()),
            )
            .map_err(|source| Error::CoefficientTokenizationSymbolWrite {
                syntax: token.syntax_name(),
                source,
            })?;
    }
    let output = encoder
        .finish()
        .map_err(|source| Error::CoefficientTokenizationSymbolEncodeFinish { source })?;
    let bytes = output.into_bytes();

    let mut decode_cdfs = CoefficientTokenCdfRows::from_defaults();
    let mut decoder = SymbolDecoder::with_config(&bytes, SymbolDecoderConfig::new())
        .map_err(|source| Error::CoefficientTokenizationSymbolDecodeInit { source })?;
    let mut decoded_symbols = Vec::new();
    decoded_symbols
        .try_reserve_exact(tokens.len())
        .map_err(|_| Error::CoefficientTokenizationAllocationFailed {
            context: "roundtrip decoded symbols",
        })?;
    for token in tokens.iter().copied() {
        let decoded = decoder
            .read_symbol_u16(decode_cdfs.row_mut(token.selector())?)
            .map_err(|source| Error::CoefficientTokenizationSymbolRead {
                syntax: token.syntax_name(),
                source,
            })?
            .get();
        if decoded != token.symbol() {
            return Err(Error::CoefficientTokenizationSymbolMismatch {
                syntax: token.syntax_name(),
                expected: token.symbol(),
                actual: decoded,
            });
        }
        decoded_symbols.push(decoded);
    }
    let summary = decoder
        .finish()
        .map_err(|source| Error::CoefficientTokenizationSymbolDecodeFinish { source })?;

    Ok(CoefficientTokenRoundtrip {
        bytes,
        decoded_symbols,
        symbol_count: summary.symbol_count,
    })
}
