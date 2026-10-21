// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.2 symbol encoder foundation (`ENC-BITSTREAM-WRITER`).
//!
//! This module provides an I/O-free writer for the generic AV2 v1.0.0 § 8.2
//! symbol-decoder primitive (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`).
//! It deliberately stops before § 8.3 syntax-element CDF selection, tile CDF
//! lifecycle, coefficient syntax, and coded tile traversal.

use crate::symbol::{
    CDF_PROB_SCALE, CdfStorage, CdfUpdateMode, EC_PROB_SHIFT, MAX_LITERAL_BITS, SYMBOL_RANGE_INIT,
    Symbol, floor_log2, update_cdf, validate_cdf_shape,
};
use crate::tables::conversion::PROB_INC;
use crate::write::{WriteError, WriteResult};

const DEFAULT_MAX_OUTPUT_BYTES: usize = 1 << 20;
const DEFAULT_MAX_OPERATIONS: usize = 1 << 20;
const SYMBOL_VALUE_BITS: u32 = 15;
/// Low 15 bits of the final interval base whose inversion is the § 8.2.4
/// trailing one bit followed by fourteen zero padding bits.
const EXIT_Y_WINDOW: u64 = (1 << (SYMBOL_VALUE_BITS - 1)) - 1;
const BYPASS_LITERAL_CHUNK_BITS: u32 = 8;
/// `low` settles whole bytes once it holds this many bits and keeps at least
/// 24, so a 16-bit range and its carry always stay inside the register.
const SETTLE_BITS: u32 = 32;

macro_rules! write_symbol_to_cdf {
    ($encoder:expr, $cdf:expr, $symbol:expr) => {{
        let encoder = $encoder;
        let cdf = $cdf;
        let shape =
            validate_cdf_shape(cdf).map_err(|kind| WriteError::InvalidSymbolCdf { kind })?;
        let symbol = usize::from($symbol.get());
        if symbol >= shape.n {
            return Err(WriteError::SymbolOutOfRange {
                symbol: symbol as u8,
                symbols: shape.n,
            });
        }

        let (cur, width) = encoder.symbol_interval(cdf, shape.n, symbol)?;
        let bits = SYMBOL_VALUE_BITS - floor_log2(width);
        encoder.reserve(u64::from(bits), 1)?;
        encoder.low = (encoder.low + u64::from(cur)) << bits;
        encoder.range = width << bits;
        encoder.advance(bits);
        encoder.symbol_count = encoder.symbol_count.saturating_add(1);

        if encoder.config.cdf_update == CdfUpdateMode::Enabled {
            update_cdf(cdf, shape, symbol);
        }

        Ok(())
    }};
}

/// Adapts `cdf` after `symbol` exactly as [`SymbolEncoder::write_symbol_u16`]
/// does with CDF updates enabled, without coding it; rate models price a
/// symbol with the row, then adapt it.
///
/// # Errors
/// Returns [`WriteError::InvalidSymbolCdf`] for a malformed row and
/// [`WriteError::SymbolOutOfRange`] for a symbol outside it.
pub fn adapt_cdf_u16(cdf: &mut [u16], symbol: Symbol) -> WriteResult<()> {
    let shape = validate_cdf_shape(cdf).map_err(|kind| WriteError::InvalidSymbolCdf { kind })?;
    let index = usize::from(symbol.get());
    if index >= shape.n {
        return Err(WriteError::SymbolOutOfRange {
            symbol: symbol.get(),
            symbols: shape.n,
        });
    }
    update_cdf(cdf, shape, index);
    Ok(())
}

/// Configuration for [`SymbolEncoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolEncoderConfig {
    cdf_update: CdfUpdateMode,
    max_output_bytes: usize,
    max_operations: usize,
}

impl SymbolEncoderConfig {
    /// Creates the default symbol-encoder configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cdf_update: CdfUpdateMode::Enabled,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_operations: DEFAULT_MAX_OPERATIONS,
        }
    }

    /// Returns the configured CDF update mode.
    #[must_use]
    pub const fn cdf_update_mode(self) -> CdfUpdateMode {
        self.cdf_update
    }

    /// Returns the configured maximum output payload size in bytes.
    #[must_use]
    pub const fn max_output_bytes(self) -> usize {
        self.max_output_bytes
    }

    /// Returns a copy of this configuration with a different CDF update mode.
    #[must_use]
    pub const fn with_cdf_update_mode(mut self, mode: CdfUpdateMode) -> Self {
        self.cdf_update = mode;
        self
    }

    /// Returns a copy of this configuration with a different output byte limit.
    #[must_use]
    pub const fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes;
        self
    }

    /// Returns a copy of this configuration with a different operation count limit.
    #[must_use]
    pub const fn with_max_operations(mut self, max_operations: usize) -> Self {
        self.max_operations = max_operations;
        self
    }
}

impl Default for SymbolEncoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary returned after successful AV2 § 8.2 symbol encoding finalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolEncoderOutput {
    bytes: Vec<u8>,
    symbol_count: u64,
    operation_count: usize,
}

impl SymbolEncoderOutput {
    /// Returns the finalized tile-payload bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the summary and returns the finalized tile-payload bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns the number of counted frame symbols.
    #[must_use]
    pub const fn symbol_count(&self) -> u64 {
        self.symbol_count
    }

    /// Returns the number of primitive operations committed before finalization.
    #[must_use]
    pub const fn operation_count(&self) -> usize {
        self.operation_count
    }
}

/// I/O-free AV2 § 8.2 symbol encoder.
///
/// The decoder keeps `SymbolValue = X - low`, where `X` is the inverted
/// payload read so far and `low` is the base of the coded interval. Each
/// write runs the matching decoder step backwards on `low` and `SymbolRange`,
/// and the high bytes of `low` settle into the output as soon as no carry
/// can reach past them, so each write costs O(1) amortized time.
#[derive(Debug, Clone)]
pub struct SymbolEncoder {
    config: SymbolEncoderConfig,
    range: u32,
    /// Unsettled low bits of the interval base, plus at most one carry bit.
    low: u64,
    /// Width of `low` in bits, without the carry bit.
    low_bits: u32,
    /// Settled high bytes of the interval base, before inversion.
    settled: Vec<u8>,
    /// Bits the decoder consumes after its 15-bit `init_symbol()` read.
    coded_bits: u64,
    symbol_count: u64,
    operation_count: usize,
}

impl SymbolEncoder {
    /// Creates an empty symbol encoder with default configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_config(SymbolEncoderConfig::new())
    }

    /// Creates an empty symbol encoder with `config`.
    #[must_use]
    pub const fn with_config(config: SymbolEncoderConfig) -> Self {
        Self {
            config,
            range: SYMBOL_RANGE_INIT,
            low: 0,
            low_bits: SYMBOL_VALUE_BITS,
            settled: Vec::new(),
            coded_bits: 0,
            symbol_count: 0,
            operation_count: 0,
        }
    }

    /// Returns this encoder's configuration.
    #[must_use]
    pub const fn config(&self) -> SymbolEncoderConfig {
        self.config
    }

    /// Returns the number of counted frame symbols so far.
    #[must_use]
    pub const fn symbol_count(&self) -> u64 {
        self.symbol_count
    }

    /// Returns the number of primitive range steps committed so far.
    #[must_use]
    pub const fn operation_count(&self) -> usize {
        self.operation_count
    }

    /// Writes a pseudo-raw bit with equal probability, inverse to AV2 § 8.2.3.
    ///
    /// # Errors
    /// Returns [`WriteError::SymbolOutputTooLarge`] if accepting the operation
    /// would exceed the configured output byte limit, or
    /// [`WriteError::SymbolOperationLimit`] if it would exceed the configured
    /// primitive operation count.
    pub fn write_bool(&mut self, value: bool) -> WriteResult<()> {
        self.reserve(1, 1)?;
        self.write_bypass(u32::from(value), 1);
        Ok(())
    }

    /// Writes AV2 § 8.2.5 `read_literal(n)` bits, MSB-first.
    ///
    /// # Errors
    /// Returns [`WriteError::BitWidthTooLarge`] when `n > 32`,
    /// [`WriteError::ValueTooWide`] when `value` does not fit in `n` bits,
    /// [`WriteError::SymbolOutputTooLarge`] if the output limit would be exceeded,
    /// or [`WriteError::SymbolOperationLimit`] if the operation limit would be exceeded.
    pub fn write_literal(&mut self, value: u32, n: u32) -> WriteResult<()> {
        if n > MAX_LITERAL_BITS {
            return Err(WriteError::BitWidthTooLarge {
                requested: n,
                max: MAX_LITERAL_BITS,
            });
        }
        if n < u32::BITS && value >= (1u32 << n) {
            return Err(WriteError::ValueTooWide {
                value: u64::from(value),
                width_bits: n,
            });
        }

        let chunk_count = n.div_ceil(BYPASS_LITERAL_CHUNK_BITS) as usize;
        self.reserve(u64::from(n), chunk_count)?;
        let mut remaining = n;
        while remaining > 0 {
            let chunk_bits = remaining.min(BYPASS_LITERAL_CHUNK_BITS);
            remaining -= chunk_bits;
            let chunk_mask = (1u32 << chunk_bits) - 1;
            let chunk_value = (value >> remaining) & chunk_mask;
            self.write_bypass(chunk_value, chunk_bits);
        }
        self.symbol_count = self.symbol_count.saturating_add(u64::from(n));
        Ok(())
    }

    /// Writes a truncated unary bypass value for [`crate::symbol::SymbolDecoder::read_unary`].
    ///
    /// Values smaller than `max_bits` are written as `value` zero bits followed
    /// by one terminating bit. A value equal to `max_bits` writes exactly
    /// `max_bits` zero bits and no terminator.
    ///
    /// # Errors
    /// Returns [`WriteError::BitWidthTooLarge`] when `max_bits > 32`,
    /// [`WriteError::ValueTooWide`] when `value > max_bits`,
    /// [`WriteError::SymbolOutputTooLarge`] if the output limit would be exceeded,
    /// or [`WriteError::SymbolOperationLimit`] if the operation limit would be exceeded.
    pub fn write_unary(&mut self, value: u32, max_bits: u32) -> WriteResult<()> {
        if max_bits > MAX_LITERAL_BITS {
            return Err(WriteError::BitWidthTooLarge {
                requested: max_bits,
                max: MAX_LITERAL_BITS,
            });
        }
        if value > max_bits {
            return Err(WriteError::ValueTooWide {
                value: u64::from(value),
                width_bits: max_bits,
            });
        }

        let bits = if value < max_bits {
            value.saturating_add(1)
        } else {
            max_bits
        };

        let chunk_count = bits.div_ceil(BYPASS_LITERAL_CHUNK_BITS) as usize;
        self.reserve(u64::from(bits), chunk_count)?;
        let has_terminator = value < max_bits;
        let mut remaining = bits;
        while remaining > 0 {
            let chunk_bits = remaining.min(BYPASS_LITERAL_CHUNK_BITS);
            remaining -= chunk_bits;
            let chunk_value = u32::from(has_terminator && remaining == 0);
            self.write_bypass(chunk_value, chunk_bits);
        }
        self.symbol_count = self.symbol_count.saturating_add(u64::from(bits));
        Ok(())
    }

    /// Writes one AV2 § 8.2.6 symbol from a caller-supplied CDF row.
    ///
    /// # Errors
    /// Returns [`WriteError::InvalidSymbolCdf`] for malformed CDF rows,
    /// [`WriteError::SymbolOutOfRange`] for symbols outside the row arity,
    /// [`WriteError::SymbolArithmeticRange`] for an impossible arithmetic state,
    /// [`WriteError::SymbolOutputTooLarge`] if the output limit would be exceeded,
    /// or [`WriteError::SymbolOperationLimit`] if the operation limit would be exceeded.
    pub fn write_symbol(&mut self, cdf: &mut [i32], symbol: Symbol) -> WriteResult<()> {
        write_symbol_to_cdf!(self, cdf, symbol)
    }

    /// Writes one AV2 § 8.2.6 symbol from a compact `u16` CDF row.
    ///
    /// The row layout and errors are identical to [`Self::write_symbol`].
    ///
    /// # Errors
    /// Returns the same errors as [`Self::write_symbol`].
    pub fn write_symbol_u16(&mut self, cdf: &mut [u16], symbol: Symbol) -> WriteResult<()> {
        write_symbol_to_cdf!(self, cdf, symbol)
    }

    /// Finalizes the symbol payload and returns owned tile-payload bytes.
    ///
    /// The payload is the shortest one § 8.2.4 `exit_symbol()` accepts: the
    /// coded bits, the trailing one bit, and zero bits up to the byte boundary.
    /// The final base is the smallest one in `[low, low + range)` that ends in
    /// the exit window; `range >= 2^15` makes it exist. One bits pad the base
    /// to whole bytes, and they invert to zero padding.
    ///
    /// # Errors
    /// Returns [`WriteError::SymbolOutputTooLarge`] if the finalized payload
    /// cannot fit in the configured output limit.
    pub fn finish(mut self) -> WriteResult<SymbolEncoderOutput> {
        let len = self.payload_len(self.coded_bits)?;
        if len > self.config.max_output_bytes {
            return Err(WriteError::SymbolOutputTooLarge {
                requested: len,
                limit: self.config.max_output_bytes,
            });
        }

        self.low += EXIT_Y_WINDOW.wrapping_sub(self.low) & ((1 << SYMBOL_VALUE_BITS) - 1);
        let pad = (8 - self.low_bits % 8) % 8;
        self.low = (self.low << pad) | ((1 << pad) - 1);
        self.low_bits += pad;
        self.settled
            .try_reserve_exact(self.low_bits as usize / 8)
            .map_err(|_| WriteError::SymbolOutputTooLarge {
                requested: len,
                limit: self.config.max_output_bytes,
            })?;
        while self.low_bits > 0 {
            self.settle_byte();
        }

        let mut bytes = self.settled;
        bytes.truncate(len);
        for byte in &mut bytes {
            *byte = !*byte;
        }
        Ok(SymbolEncoderOutput {
            bytes,
            symbol_count: self.symbol_count,
            operation_count: self.operation_count,
        })
    }

    /// Writes `bits <= 8` bypass bits, MSB-first. The decoder takes the upper
    /// half of the range for a zero bit (§ 8.2.3), so every zero bit adds the
    /// range at its scale to the base.
    fn write_bypass(&mut self, value: u32, bits: u32) {
        debug_assert!(bits <= BYPASS_LITERAL_CHUNK_BITS);
        let zeros = !value & ((1 << bits) - 1);
        self.low = (self.low << bits) + u64::from(self.range) * u64::from(zeros);
        self.advance(bits);
    }

    fn advance(&mut self, bits: u32) {
        self.low_bits += bits;
        self.coded_bits += u64::from(bits);
        while self.low_bits >= SETTLE_BITS {
            self.settle_byte();
        }
    }

    /// Moves the top byte of `low` into `settled`, carrying into the bytes
    /// already settled. The interval never leaves `[0, 2^(15 + K))`, so the
    /// carry always stops inside `settled`.
    fn settle_byte(&mut self) {
        let shift = self.low_bits - 8;
        let byte = self.low >> shift;
        if byte > 0xff {
            for settled in self.settled.iter_mut().rev() {
                let (sum, overflow) = settled.overflowing_add(1);
                *settled = sum;
                if !overflow {
                    break;
                }
            }
        }
        self.settled.push(byte as u8);
        self.low &= (1 << shift) - 1;
        self.low_bits = shift;
    }

    /// Returns the base offset `cur` and width of `target`'s interval, as the
    /// decoder search in § 8.2.6 derives them from the current range.
    fn symbol_interval<T: CdfStorage>(
        &self,
        cdf: &[T],
        n: usize,
        target: usize,
    ) -> WriteResult<(u32, u32)> {
        let mut prev = self.range;
        let mut cur = self.range;
        for (symbol, prob_inc) in PROB_INC[n - 2].iter().enumerate().take(target + 1) {
            let f = if symbol == n - 1 {
                0
            } else {
                CDF_PROB_SCALE - cdf[symbol].to_i32() as u32
            };
            let pp = ((f >> EC_PROB_SHIFT) << 4) + *prob_inc as u32;
            prev = cur;
            cur = (((self.range >> 8) * pp) >> 7) << 3;
        }
        let width = prev
            .checked_sub(cur)
            .filter(|width| *width != 0)
            .ok_or(WriteError::SymbolArithmeticRange)?;
        Ok((cur, width))
    }

    /// Checks the limits for one more write and reserves its settled bytes,
    /// so the write itself cannot fail. Settled bytes stay at least 24 bits
    /// behind the payload end, so the payload length bounds them.
    fn reserve(&mut self, additional_bits: u64, additional_operations: usize) -> WriteResult<()> {
        let limit = self.config.max_operations;
        let operations = self
            .operation_count
            .checked_add(additional_operations)
            .ok_or(WriteError::SymbolOperationLimit {
                requested: usize::MAX,
                limit,
            })?;
        if operations > limit {
            return Err(WriteError::SymbolOperationLimit {
                requested: operations,
                limit,
            });
        }

        let limit = self.config.max_output_bytes;
        let coded_bits = self.coded_bits.checked_add(additional_bits).ok_or(
            WriteError::SymbolOutputTooLarge {
                requested: usize::MAX,
                limit,
            },
        )?;
        let requested = self.payload_len(coded_bits)?;
        if requested > limit {
            return Err(WriteError::SymbolOutputTooLarge { requested, limit });
        }
        self.settled
            .try_reserve(requested.saturating_sub(self.settled.len()))
            .map_err(|_| WriteError::SymbolOutputTooLarge { requested, limit })?;
        self.operation_count = operations;
        Ok(())
    }

    /// Returns the payload length for `coded_bits` plus the trailing one bit.
    fn payload_len(&self, coded_bits: u64) -> WriteResult<usize> {
        coded_bits
            .checked_add(8)
            .and_then(|bits| usize::try_from(bits / 8).ok())
            .ok_or(WriteError::SymbolOutputTooLarge {
                requested: usize::MAX,
                limit: self.config.max_output_bytes,
            })
    }
}

impl Default for SymbolEncoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "symbol_encoder_tests.rs"]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "symbol_encoder_proptests.rs"]
mod proptests;
