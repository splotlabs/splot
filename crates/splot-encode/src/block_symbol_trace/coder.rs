// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The unified `BlockSymbolToken` plus the § 8.2 coder driver
//! (`encode_block_symbol_trace`, `roundtrip_block_symbol_trace`, and the
//! `BlockSymbolTraceRoundtrip` proof). Split out of `block_symbol_trace` to keep
//! each file under the 1000-line source budget.

use super::*;

/// One symbol of the ordered block-symbol trace, spanning the intra-mode and
/// coefficient token kinds that a coded tile body interleaves through one coder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockSymbolToken {
    /// An AV2 § 5.20.3.2 partition token (here, the root `do_split`).
    Partition(PartitionToken),
    /// An AV2 § 5.20.5 mode-info token (`y_mode_set` / `y_mode_index` / `uv_mode`).
    Mode(IntraModeToken),
    /// An AV2 § 5.20.7 coefficient token (here, the luma `txb_skip` all-zero).
    Coeff(CoefficientEntropyToken),
    /// An AV2 § 8.2.5 bypass literal of `width` bits carrying `value` (MSB-first).
    ///
    /// Unlike `Mode`/`Coeff` (CDF-coded `S()` symbols) a bypass literal is an
    /// `L(n)` read with no CDF — e.g. the `sign_bit` of a chroma or ordinary
    /// non-axis luma coefficient (§ 5.20.7.27 codes the luma DC sign as `dc_sign`
    /// and the directional luma axis signs as `dc_sign_horz_vert`, both CDF; every
    /// other sign is `sign_bit`) or the `read_quant` golomb tail (§ 5.20.7.28).
    Bypass {
        /// Number of literal bits (`n` in `L(n)`).
        width: u32,
        /// The literal value, written/read most-significant-bit first.
        value: u32,
    },
}

impl BlockSymbolToken {
    /// Constructs a bypass literal of `width` bits carrying `value`.
    pub(crate) const fn bypass(width: u32, value: u32) -> Self {
        Self::Bypass { width, value }
    }

    /// Returns the raw symbol/value view of the token (the CDF symbol for
    /// `Mode`/`Coeff`, or the literal value for `Bypass`).
    pub(crate) const fn symbol(self) -> u8 {
        match self {
            Self::Partition(token) => token.symbol(),
            Self::Mode(token) => token.symbol(),
            Self::Coeff(token) => token.symbol(),
            Self::Bypass { value, .. } => value as u8,
        }
    }
}

/// Result of proving a block-symbol trace through AV2 § 8.2 symbol bytes.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct BlockSymbolTraceRoundtrip {
    bytes: Vec<u8>,
    decoded_symbols: Vec<u8>,
    symbol_count: u64,
}

impl BlockSymbolTraceRoundtrip {
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

/// Encodes an ordered block-symbol trace into AV2 § 8.2 entropy-coded bytes — the
/// encoder's production entropy-coding entry point. Each token is written to its
/// scoped default CDF row (a bypass literal writes its raw bits); `finish()`
/// terminates the § 8.2 stream (§ 8.2.4 padding) and yields the coded bytes that a
/// § 5.20.1 `tile_group_payload()` carries as a single tile's data.
///
/// This drives the same § 8.2 encode path as [`roundtrip_block_symbol_trace`] (which
/// calls it), but returns the coded bytes for downstream tile-group assembly rather
/// than re-decoding them. It does not assemble a tile-group payload, OBU, frame, or
/// packet — those are later bricks.
pub(crate) fn encode_block_symbol_trace(trace: &[BlockSymbolToken]) -> Result<Vec<u8>> {
    let mut encode_cdfs = BlockSymbolTraceCdfRows::from_defaults();
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
            BlockSymbolToken::Bypass { width, value } => {
                encoder
                    .write_literal(*value, *width)
                    .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?;
            }
            _ => {
                encoder
                    .write_symbol(
                        encode_cdfs.row_mut(*token, index)?,
                        Symbol::new(token.symbol()),
                    )
                    .map_err(|source| Error::BlockSymbolTraceSymbolWrite { index, source })?;
            }
        }
    }
    let output = encoder
        .finish()
        .map_err(|source| Error::BlockSymbolTraceSymbolEncodeFinish { source })?;
    Ok(output.into_bytes())
}

/// Proves a block-symbol trace through one § 8.2 coder: encodes it via
/// [`encode_block_symbol_trace`], then decodes the bytes back through one symbol
/// decoder with the same shared CDF state, verifying every token reproduces. Returns
/// the coded bytes and the decoded symbols for assertions.
pub(crate) fn roundtrip_block_symbol_trace(
    trace: &[BlockSymbolToken],
) -> Result<BlockSymbolTraceRoundtrip> {
    let bytes = encode_block_symbol_trace(trace)?;

    let mut decode_cdfs = BlockSymbolTraceCdfRows::from_defaults();
    let mut decoder = SymbolDecoder::with_config(&bytes, SymbolDecoderConfig::new())
        .map_err(|source| Error::BlockSymbolTraceSymbolDecodeInit { source })?;
    let mut decoded_symbols = Vec::new();
    decoded_symbols
        .try_reserve_exact(trace.len())
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "roundtrip decoded symbols",
        })?;
    for (index, token) in trace.iter().enumerate() {
        let decoded = if let BlockSymbolToken::Bypass { width, value } = token {
            let actual = decoder
                .read_literal(*width)
                .map_err(|source| Error::BlockSymbolTraceSymbolRead { index, source })?;
            if actual != *value {
                return Err(Error::BlockSymbolTraceLiteralMismatch {
                    index,
                    width: *width,
                    expected: *value,
                    actual,
                });
            }
            actual as u8
        } else {
            let decoded = decoder
                .read_symbol(decode_cdfs.row_mut(*token, index)?)
                .map_err(|source| Error::BlockSymbolTraceSymbolRead { index, source })?
                .get();
            if decoded != token.symbol() {
                return Err(Error::BlockSymbolTraceSymbolMismatch {
                    index,
                    expected: token.symbol(),
                    actual: decoded,
                });
            }
            decoded
        };
        decoded_symbols.push(decoded);
    }
    let summary = decoder
        .finish()
        .map_err(|source| Error::BlockSymbolTraceSymbolDecodeFinish { source })?;

    Ok(BlockSymbolTraceRoundtrip {
        bytes,
        decoded_symbols,
        symbol_count: summary.symbol_count,
    })
}
