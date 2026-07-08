// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.2 symbol encoder foundation (`ENC-BITSTREAM-WRITER`).
//!
//! This module provides an I/O-free writer for the generic AV2 v1.0.0 § 8.2
//! symbol-decoder primitive (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`).
//! It deliberately stops before § 8.3 syntax-element CDF selection, tile CDF
//! lifecycle, coefficient syntax, and coded tile traversal.

use crate::symbol::{
    CDF_PROB_SCALE, CdfUpdateMode, EC_PROB_SHIFT, MAX_LITERAL_BITS, SYMBOL_RANGE_INIT, Symbol,
    floor_log2, update_cdf, validate_cdf_shape,
};
use crate::tables::conversion::PROB_INC;
use crate::write::{WriteError, WriteResult};

const DEFAULT_MAX_OUTPUT_BYTES: usize = 1 << 20;
const DEFAULT_MAX_OPERATIONS: usize = 1 << 20;
const SYMBOL_VALUE_BITS: u32 = 15;
const EXIT_Y_WINDOW: u32 = (1 << (SYMBOL_VALUE_BITS - 1)) - 1;
const INITIAL_CODE_BITS: u64 = SYMBOL_VALUE_BITS as u64;
const BYPASS_LITERAL_CHUNK_BITS: u32 = 8;

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

    /// Returns the configured maximum primitive operation count.
    #[must_use]
    pub const fn max_operations(self) -> usize {
        self.max_operations
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
#[derive(Debug, Clone)]
pub struct SymbolEncoder {
    config: SymbolEncoderConfig,
    range: u32,
    value_limit: u32,
    step_bits: u64,
    symbol_count: u64,
    steps: Vec<RangeStep>,
}

impl SymbolEncoder {
    /// Creates an empty symbol encoder with default configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            config: SymbolEncoderConfig::new(),
            range: SYMBOL_RANGE_INIT,
            value_limit: SYMBOL_RANGE_INIT,
            step_bits: 0,
            symbol_count: 0,
            steps: Vec::new(),
        }
    }

    /// Creates an empty symbol encoder with `config`.
    #[must_use]
    pub const fn with_config(config: SymbolEncoderConfig) -> Self {
        Self {
            config,
            range: SYMBOL_RANGE_INIT,
            value_limit: SYMBOL_RANGE_INIT,
            step_bits: 0,
            symbol_count: 0,
            steps: Vec::new(),
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
    pub fn operation_count(&self) -> usize {
        self.steps.len()
    }

    /// Writes a pseudo-raw bit with equal probability, inverse to AV2 § 8.2.3.
    ///
    /// # Errors
    /// Returns [`WriteError::SymbolOutputTooLarge`] if accepting the operation
    /// would exceed the configured output byte limit, or
    /// [`WriteError::SymbolOperationLimit`] if it would exceed the configured
    /// primitive operation count.
    pub fn write_bool(&mut self, value: bool) -> WriteResult<()> {
        self.ensure_projected_steps(1, 1)?;
        self.push_bypass_chunk(u32::from(value), 1);
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
        self.ensure_projected_steps(u64::from(n), chunk_count)?;
        let mut remaining = n;
        while remaining > 0 {
            let chunk_bits = remaining.min(BYPASS_LITERAL_CHUNK_BITS);
            remaining -= chunk_bits;
            let chunk_mask = (1u32 << chunk_bits) - 1;
            let chunk_value = (value >> remaining) & chunk_mask;
            self.push_bypass_chunk(chunk_value, chunk_bits);
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
        self.ensure_projected_steps(u64::from(bits), chunk_count)?;
        let has_terminator = value < max_bits;
        let mut remaining = bits;
        while remaining > 0 {
            let chunk_bits = remaining.min(BYPASS_LITERAL_CHUNK_BITS);
            remaining -= chunk_bits;
            let chunk_value = u32::from(has_terminator && remaining == 0);
            self.push_bypass_chunk(chunk_value, chunk_bits);
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
        let shape =
            validate_cdf_shape(cdf).map_err(|kind| WriteError::InvalidSymbolCdf { kind })?;
        let symbol = usize::from(symbol.get());
        if symbol >= shape.n {
            return Err(WriteError::SymbolOutOfRange {
                symbol: symbol as u8,
                symbols: shape.n,
            });
        }

        let step = self.symbol_step(cdf, shape.n, symbol)?;
        self.ensure_projected_steps(u64::from(step.bits), 1)?;
        self.range = step.range_after;
        self.value_limit = step.range_after;
        self.step_bits = self.step_bits.saturating_add(u64::from(step.bits));
        self.steps.push(RangeStep::Symbol {
            low: step.low,
            bits: step.bits,
            residual_limit: step.range_after >> step.bits,
        });
        self.symbol_count = self.symbol_count.saturating_add(1);

        if self.config.cdf_update == CdfUpdateMode::Enabled {
            update_cdf(cdf, shape, symbol);
        }

        Ok(())
    }

    /// Finalizes the symbol payload and returns owned tile-payload bytes.
    ///
    /// # Errors
    /// Returns [`WriteError::SymbolOutputTooLarge`] if even the finalized payload
    /// cannot fit in the configured output limit, or
    /// [`WriteError::SymbolFinalizationFailed`] if no valid `exit_symbol()` tail
    /// can be constructed for the committed operation stream.
    pub fn finish(self) -> WriteResult<SymbolEncoderOutput> {
        let requested = bytes_for_bits(
            INITIAL_CODE_BITS + self.step_bits,
            self.config.max_output_bytes,
        )?;
        if requested > self.config.max_output_bytes {
            return Err(WriteError::SymbolOutputTooLarge {
                requested,
                limit: self.config.max_output_bytes,
            });
        }

        let bytes = self.finalize_bytes()?;
        Ok(SymbolEncoderOutput {
            bytes,
            symbol_count: self.symbol_count,
            operation_count: self.steps.len(),
        })
    }

    fn push_bypass_chunk(&mut self, value: u32, bits: u32) {
        debug_assert!(bits <= BYPASS_LITERAL_CHUNK_BITS);

        let mut split = u64::from(self.range) << bits;
        let mut low = 0u64;
        let mut high = u64::from(self.value_limit) << bits;
        for bit in (0..bits).rev() {
            split >>= 1;
            if ((value >> bit) & 1) == 0 {
                low += split;
            } else {
                high = high.min(low + split);
            }
        }
        let residual_limit = high - low;
        self.steps.push(RangeStep::Bypass {
            low,
            bits,
            residual_limit,
        });
        self.value_limit = residual_limit as u32;
        self.step_bits = self.step_bits.saturating_add(u64::from(bits));
    }

    fn symbol_step(&self, cdf: &[i32], n: usize, target: usize) -> WriteResult<SymbolStep> {
        let mut cur = self.range;
        for symbol in 0..=target {
            let prev = cur;
            let f = if symbol == n - 1 {
                0
            } else {
                CDF_PROB_SCALE - cdf[symbol] as u32
            };
            let prob_inc = PROB_INC[n - 2][symbol] as u32;
            let pp = ((f >> EC_PROB_SHIFT) << 4) + prob_inc;
            cur = (((self.range >> 8) * pp) >> 7) << 3;

            if symbol == target {
                let width = prev
                    .checked_sub(cur)
                    .filter(|width| *width != 0)
                    .ok_or(WriteError::SymbolArithmeticRange)?;
                let bits = SYMBOL_VALUE_BITS - floor_log2(width);
                return Ok(SymbolStep {
                    low: cur,
                    bits,
                    range_after: width << bits,
                });
            }
        }

        Err(WriteError::SymbolArithmeticRange)
    }

    fn ensure_projected_steps(
        &mut self,
        additional_bits: u64,
        additional_steps: usize,
    ) -> WriteResult<()> {
        let requested_operations = self.steps.len().checked_add(additional_steps).ok_or(
            WriteError::SymbolOperationLimit {
                requested: usize::MAX,
                limit: self.config.max_operations,
            },
        )?;
        if requested_operations > self.config.max_operations {
            return Err(WriteError::SymbolOperationLimit {
                requested: requested_operations,
                limit: self.config.max_operations,
            });
        }

        let total_bits = INITIAL_CODE_BITS
            .checked_add(self.step_bits)
            .and_then(|bits| bits.checked_add(additional_bits))
            .ok_or(WriteError::SymbolOutputTooLarge {
                requested: usize::MAX,
                limit: self.config.max_output_bytes,
            })?;
        let requested = bytes_for_bits(total_bits, self.config.max_output_bytes)?;
        if requested > self.config.max_output_bytes {
            return Err(WriteError::SymbolOutputTooLarge {
                requested,
                limit: self.config.max_output_bytes,
            });
        }
        self.steps
            .try_reserve(additional_steps)
            .map_err(|_| WriteError::SymbolOutputTooLarge {
                requested,
                limit: self.config.max_output_bytes,
            })?;
        Ok(())
    }

    fn finalize_bytes(&self) -> WriteResult<Vec<u8>> {
        let mut suffixes = Vec::new();
        suffixes.try_reserve_exact(self.steps.len()).map_err(|_| {
            WriteError::SymbolOutputTooLarge {
                requested: self.config.max_output_bytes.saturating_add(1),
                limit: self.config.max_output_bytes,
            }
        })?;
        // TODO(spec: ENC-BITSTREAM-WRITER): Replace this correctness-first
        for candidate in 0..self.value_limit {
            suffixes.clear();
            let Some(initial) = self.backward_candidate(candidate, &mut suffixes) else {
                continue;
            };
            if !exit_window_matches(initial, &suffixes) {
                continue;
            }

            let requested = bytes_for_bits(
                INITIAL_CODE_BITS + self.step_bits,
                self.config.max_output_bytes,
            )?;
            let mut writer = PayloadWriter::with_capacity(requested, self.config.max_output_bytes)?;
            write_inverted_bits(&mut writer, initial, SYMBOL_VALUE_BITS)?;
            for chunk in suffixes.iter().rev() {
                write_inverted_bits(&mut writer, chunk.value, chunk.bits)?;
            }
            let bytes = writer.into_bytes()?;
            if bytes.len() > self.config.max_output_bytes {
                return Err(WriteError::SymbolOutputTooLarge {
                    requested: bytes.len(),
                    limit: self.config.max_output_bytes,
                });
            }
            return Ok(bytes);
        }

        Err(WriteError::SymbolFinalizationFailed)
    }

    fn backward_candidate(&self, candidate: u32, suffixes: &mut Vec<BitChunk>) -> Option<u32> {
        let mut next = candidate;
        for step in self.steps.iter().rev() {
            match *step {
                RangeStep::Symbol {
                    low,
                    bits,
                    residual_limit,
                } => {
                    let mask = mask_for_bits(bits);
                    suffixes.push(BitChunk {
                        value: next & mask,
                        bits,
                    });
                    let residual = next >> bits;
                    if residual >= residual_limit {
                        return None;
                    }
                    next = low + residual;
                }
                RangeStep::Bypass {
                    low,
                    bits,
                    residual_limit,
                } => {
                    if u64::from(next) >= residual_limit {
                        return None;
                    }
                    let scaled = low + u64::from(next);
                    let mask = u64::from(mask_for_bits(bits));
                    suffixes.push(BitChunk {
                        value: (scaled & mask) as u32,
                        bits,
                    });
                    next = u32::try_from(scaled >> bits).ok()?;
                }
            }
        }
        Some(next)
    }
}

impl Default for SymbolEncoder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
enum RangeStep {
    Symbol {
        low: u32,
        bits: u32,
        residual_limit: u32,
    },
    Bypass {
        low: u64,
        bits: u32,
        residual_limit: u64,
    },
}

#[derive(Debug, Clone, Copy)]
struct SymbolStep {
    low: u32,
    bits: u32,
    range_after: u32,
}

#[derive(Debug, Clone, Copy)]
struct BitChunk {
    value: u32,
    bits: u32,
}

struct PayloadWriter {
    bytes: Vec<u8>,
    current: u8,
    nbits: u8,
    limit: usize,
}

impl PayloadWriter {
    fn with_capacity(capacity: usize, limit: usize) -> WriteResult<Self> {
        if capacity > limit {
            return Err(WriteError::SymbolOutputTooLarge {
                requested: capacity,
                limit,
            });
        }

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| WriteError::SymbolOutputTooLarge {
                requested: capacity,
                limit,
            })?;
        Ok(Self {
            bytes,
            current: 0,
            nbits: 0,
            limit,
        })
    }

    fn write_bit(&mut self, bit: u8) -> WriteResult<()> {
        if bit > 1 {
            return Err(WriteError::ValueTooWide {
                value: u64::from(bit),
                width_bits: 1,
            });
        }

        self.current = (self.current << 1) | bit;
        self.nbits += 1;
        if self.nbits == 8 {
            self.push_byte(self.current)?;
            self.current = 0;
            self.nbits = 0;
        }
        Ok(())
    }

    fn into_bytes(mut self) -> WriteResult<Vec<u8>> {
        if self.nbits != 0 {
            let pad = 8 - u32::from(self.nbits);
            self.push_byte(self.current << pad)?;
        }
        Ok(self.bytes)
    }

    fn push_byte(&mut self, byte: u8) -> WriteResult<()> {
        if self.bytes.len() >= self.limit {
            return Err(WriteError::SymbolOutputTooLarge {
                requested: self.bytes.len().saturating_add(1),
                limit: self.limit,
            });
        }
        self.bytes.push(byte);
        Ok(())
    }
}

fn bytes_for_bits(bits: u64, limit: usize) -> WriteResult<usize> {
    let bytes = bits
        .checked_add(7)
        .map(|bits| bits / 8)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(WriteError::SymbolOutputTooLarge {
            requested: usize::MAX,
            limit,
        })?;
    Ok(bytes)
}

fn mask_for_bits(bits: u32) -> u32 {
    (1u32 << bits) - 1
}

fn exit_window_matches(initial: u32, suffixes: &[BitChunk]) -> bool {
    let mut tail = 0u32;
    append_tail_bits(&mut tail, initial, SYMBOL_VALUE_BITS);
    for chunk in suffixes.iter().rev() {
        append_tail_bits(&mut tail, chunk.value, chunk.bits);
    }
    tail == EXIT_Y_WINDOW
}

fn append_tail_bits(tail: &mut u32, value: u32, bits: u32) {
    for bit in (0..bits).rev() {
        *tail = ((*tail << 1) | ((value >> bit) & 1)) & ((1 << SYMBOL_VALUE_BITS) - 1);
    }
}

fn write_inverted_bits(writer: &mut PayloadWriter, value: u32, bits: u32) -> WriteResult<()> {
    for bit in (0..bits).rev() {
        let y = ((value >> bit) & 1) as u8;
        writer.write_bit(y ^ 1)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "symbol_encoder_tests.rs"]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "symbol_encoder_proptests.rs"]
mod proptests;
