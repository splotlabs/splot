// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.2 symbol decoder foundation.
//!
//! This module implements the generic symbol-decoder primitives from AV2 v1.0.0
//! § 8.2 over a caller-provided tile payload byte slice
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`). It deliberately stops
//! before § 8.3 syntax-element CDF selection, tile CDF bank ownership,
//! `decode_tile()`, reconstruction, and encoder range writing.

use crate::bitio::BitReader;
use crate::error::{Error, Result, SymbolCdfErrorKind, SymbolDecoderErrorKind};
use crate::span::{BitOffset, ByteOffset};
use crate::tables::conversion::{PARA_ADJUSTMENT_LIST, PROB_INC};

pub(crate) const CDF_PROB_SCALE: u32 = 1 << 15;
pub(crate) const CDF_PROB_MAX: i32 = (1 << 15) - 1;
pub(crate) const EC_PROB_SHIFT: u32 = 7;
pub(crate) const SYMBOL_RANGE_INIT: u32 = 1 << 15;
pub(crate) const MIN_SYMBOLS: usize = 2;
pub(crate) const MAX_SYMBOLS: usize = 8;
pub(crate) const MAX_LITERAL_BITS: u32 = 32;
pub(crate) const MAX_CDF_COUNT: i32 = 32;

/// Relative bit position inside the tile payload consumed by a symbol decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolBitPosition(u64);

impl SymbolBitPosition {
    /// Creates a relative symbol-decoder bit position.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw relative bit position.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A decoded AV2 § 8.2 symbol value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Symbol(u8);

impl Symbol {
    /// Creates a symbol from a value already bounded by the CDF arity.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    /// Returns the raw decoded symbol value.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Controls whether `read_symbol(cdf)` mutates caller-supplied CDF rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdfUpdateMode {
    /// Apply the AV2 § 8.2.6 CDF adaptation step.
    Enabled,
    /// Decode symbols without mutating the CDF row.
    Disabled,
}

/// Configuration for [`SymbolDecoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolDecoderConfig {
    cdf_update: CdfUpdateMode,
}

impl SymbolDecoderConfig {
    /// Creates the default symbol-decoder configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cdf_update: CdfUpdateMode::Enabled,
        }
    }

    /// Returns the configured CDF update mode.
    #[must_use]
    pub const fn cdf_update_mode(self) -> CdfUpdateMode {
        self.cdf_update
    }

    /// Returns a copy of this configuration with a different CDF update mode.
    #[must_use]
    pub const fn with_cdf_update_mode(mut self, mode: CdfUpdateMode) -> Self {
        self.cdf_update = mode;
        self
    }
}

impl Default for SymbolDecoderConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary returned after a successful AV2 § 8.2.4 `exit_symbol()` validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolDecoderSummary {
    /// Number of bits consumed after advancing to `paddingEndPosition`.
    pub consumed_bits: SymbolBitPosition,
    /// Number of frame symbols counted by `read_literal` and `read_symbol`.
    pub symbol_count: u64,
    /// Relative `trailingBitPosition` inside the tile payload.
    pub trailing_bit_position: SymbolBitPosition,
    /// Relative `paddingEndPosition` inside the tile payload.
    pub padding_end_position: SymbolBitPosition,
}

/// Snapshot of the AV2 § 8.2 arithmetic decoder state at a syntax boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolDecoderCheckpoint {
    /// Number of bits consumed from the tile payload.
    pub consumed_bits: SymbolBitPosition,
    /// Number of frame symbols counted by `read_literal` and `read_symbol`.
    pub symbol_count: u64,
    /// Current signed `SymbolMaxBits` value.
    pub symbol_max_bits: i64,
    /// Current `SymbolValue` arithmetic decoder register.
    pub symbol_value: u32,
    /// Current `SymbolRange` arithmetic decoder register.
    pub symbol_range: u32,
}

/// Bounded AV2 § 8.2 symbol decoder over one tile payload byte slice.
#[derive(Debug)]
pub struct SymbolDecoder<'a> {
    data: &'a [u8],
    base: ByteOffset,
    reader: BitReader<'a>,
    symbol_value: u32,
    symbol_range: u32,
    symbol_max_bits: i64,
    frame_symbol_count: u64,
    config: SymbolDecoderConfig,
}

impl<'a> SymbolDecoder<'a> {
    /// Initializes the decoder over `tile_payload`, with offsets relative to byte 0.
    ///
    /// This implements AV2 § 8.2.2 `init_symbol(sz)` using `sz = tile_payload.len()`.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] if the payload is too large for
    /// the signed `SymbolMaxBits` state.
    pub fn new(tile_payload: &'a [u8]) -> Result<Self> {
        Self::with_config(tile_payload, SymbolDecoderConfig::new())
    }

    /// Initializes the decoder over `tile_payload` using `config`.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] if the payload is too large for
    /// the signed `SymbolMaxBits` state.
    pub fn with_config(tile_payload: &'a [u8], config: SymbolDecoderConfig) -> Result<Self> {
        Self::with_base_and_config(tile_payload, ByteOffset::new(0), config)
    }

    /// Initializes the decoder over `tile_payload`, reporting errors relative to `base`.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] if the payload is too large for
    /// the signed `SymbolMaxBits` state.
    pub fn with_base_and_config(
        tile_payload: &'a [u8],
        base: ByteOffset,
        config: SymbolDecoderConfig,
    ) -> Result<Self> {
        let symbol_max_bits = symbol_max_bits_for_len(tile_payload.len(), base)?;
        let mut reader = BitReader::new(tile_payload, base);
        let num_bits = tile_payload.len().saturating_mul(8).min(15) as u32;
        let buf = reader.read_bits(num_bits)?;
        let padded_buf = buf << (15 - num_bits);

        Ok(Self {
            data: tile_payload,
            base,
            reader,
            symbol_value: (SYMBOL_RANGE_INIT - 1) ^ padded_buf,
            symbol_range: SYMBOL_RANGE_INIT,
            symbol_max_bits,
            frame_symbol_count: 0,
            config,
        })
    }

    /// Returns the current signed `SymbolMaxBits` value.
    #[must_use]
    pub const fn symbol_max_bits(&self) -> i64 {
        self.symbol_max_bits
    }

    /// Returns the number of counted frame symbols so far.
    #[must_use]
    pub const fn symbol_count(&self) -> u64 {
        self.frame_symbol_count
    }

    /// Returns the current relative bit position in the tile payload.
    #[must_use]
    pub fn consumed_bits(&self) -> SymbolBitPosition {
        SymbolBitPosition::new(self.reader.consumed_bits())
    }

    /// Returns a lossless checkpoint of the current arithmetic decoder state.
    #[must_use]
    pub fn checkpoint(&self) -> SymbolDecoderCheckpoint {
        SymbolDecoderCheckpoint {
            consumed_bits: self.consumed_bits(),
            symbol_count: self.frame_symbol_count,
            symbol_max_bits: self.symbol_max_bits,
            symbol_value: self.symbol_value,
            symbol_range: self.symbol_range,
        }
    }

    /// Decodes a pseudo-raw bit with equal probability, per AV2 § 8.2.3.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] if the bounded tile payload unexpectedly
    /// cannot supply a required coded bit.
    pub fn read_bool(&mut self) -> Result<bool> {
        let cur = self.symbol_range >> 1;
        let symbol = self.symbol_value < cur;
        if !symbol {
            self.symbol_value -= cur;
        }

        let num_bits = self.num_bits_to_read(1);
        let new_data = self.reader.read_bits(num_bits)?;
        self.symbol_value = (self.symbol_value << 1) | (new_data ^ 1);
        self.symbol_max_bits -= 1;
        Ok(symbol)
    }

    /// Decodes an AV2 § 8.2.5 `read_literal(n)` value, returned MSB-first.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] if `n > 32`, or propagates
    /// [`Error::UnexpectedEof`] from the bounded bit reader.
    pub fn read_literal(&mut self, n: u32) -> Result<u32> {
        if n > MAX_LITERAL_BITS {
            return Err(
                self.state_error(SymbolDecoderErrorKind::LiteralWidthTooLarge {
                    requested: n,
                    max: MAX_LITERAL_BITS,
                }),
            );
        }

        self.frame_symbol_count = self.frame_symbol_count.saturating_add(u64::from(n));
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | u32::from(self.read_bool()?);
        }
        Ok(value)
    }

    /// Decodes one AV2 § 8.2.6 symbol from a caller-supplied mutable CDF row.
    ///
    /// The row layout is `N + 1` entries: `N - 1` cumulative probability entries,
    /// `cdf[N - 1]` adaptation-rate index, and `cdf[N]` capped use count. This
    /// primitive validates that shape before indexing generated § 9 tables.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolCdf`] for malformed CDF rows,
    /// [`Error::InvalidSymbolDecoderState`] for impossible arithmetic state, or
    /// [`Error::UnexpectedEof`] if the bounded tile payload unexpectedly cannot
    /// supply a required coded bit.
    pub fn read_symbol(&mut self, cdf: &mut [i32]) -> Result<Symbol> {
        let shape = self.validate_cdf(cdf)?;

        let mut cur = self.symbol_range;
        let mut symbol = 0usize;

        let (prev, cur) = loop {
            let prev = cur;
            let f = if symbol == shape.n - 1 {
                0
            } else {
                CDF_PROB_SCALE - cdf[symbol] as u32
            };
            let prob_inc = PROB_INC[shape.n - 2][symbol] as u32;
            let pp = ((f >> EC_PROB_SHIFT) << 4) + prob_inc;
            let next_cur = (((self.symbol_range >> 8) * pp) >> 7) << 3;

            if self.symbol_value >= next_cur {
                break (prev, next_cur);
            }

            cur = next_cur;
            symbol += 1;
            if symbol >= shape.n {
                return Err(self.state_error(SymbolDecoderErrorKind::InvalidArithmeticRange));
            }
        };

        let new_range = prev.saturating_sub(cur);
        if new_range == 0 {
            return Err(self.state_error(SymbolDecoderErrorKind::InvalidArithmeticRange));
        }
        let new_value = self.symbol_value - cur;
        let bits = 15 - floor_log2(new_range);
        self.symbol_range = new_range << bits;
        let num_bits = self.num_bits_to_read(bits);
        let new_data = self.reader.read_bits(num_bits)?;
        let padded_data = new_data << (bits - num_bits);
        let mask = (1u32 << bits) - 1;
        self.symbol_value = (new_value << bits) | (padded_data ^ mask);
        self.symbol_max_bits -= i64::from(bits);
        self.frame_symbol_count = self.frame_symbol_count.saturating_add(1);

        if self.config.cdf_update == CdfUpdateMode::Enabled {
            update_cdf(cdf, shape, symbol);
        }

        Ok(Symbol::new(symbol as u8))
    }

    /// Validates AV2 § 8.2.4 `exit_symbol()` and returns the final decoder summary.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] when `SymbolMaxBits < -14`, the
    /// computed trailing bit is missing or not `1`, or any padding bit before
    /// `paddingEndPosition` is nonzero.
    pub fn exit_symbol(self) -> Result<SymbolDecoderSummary> {
        if self.symbol_max_bits < -14 {
            return Err(
                self.state_error(SymbolDecoderErrorKind::SymbolMaxBitsTooSmall {
                    symbol_max_bits: self.symbol_max_bits,
                }),
            );
        }

        let current = self.reader.consumed_bits();
        let rewind = u64::try_from((self.symbol_max_bits + 15).min(15)).map_err(|_| {
            self.state_error(SymbolDecoderErrorKind::SymbolMaxBitsTooSmall {
                symbol_max_bits: self.symbol_max_bits,
            })
        })?;
        let trailing_bit_position = current.checked_sub(rewind).ok_or_else(|| {
            self.state_error(SymbolDecoderErrorKind::TrailingBitOutOfRange { bit_position: 0 })
        })?;
        let skip = if self.symbol_max_bits > 0 {
            self.symbol_max_bits as u64
        } else {
            0
        };
        let padding_end_position = current.checked_add(skip).ok_or_else(|| {
            self.state_error(SymbolDecoderErrorKind::PaddingEndOutOfRange {
                bit_position: u64::MAX,
            })
        })?;
        let total_bits = total_bits(self.data.len());

        if trailing_bit_position >= total_bits {
            return Err(self.state_error_at_bit(
                trailing_bit_position,
                SymbolDecoderErrorKind::TrailingBitOutOfRange {
                    bit_position: trailing_bit_position,
                },
            ));
        }
        if padding_end_position > total_bits {
            return Err(self.state_error_at_bit(
                total_bits,
                SymbolDecoderErrorKind::PaddingEndOutOfRange {
                    bit_position: padding_end_position,
                },
            ));
        }
        if padding_end_position % 8 != 0 {
            return Err(self.state_error_at_bit(
                padding_end_position,
                SymbolDecoderErrorKind::PaddingEndNotByteAligned {
                    bit_position: padding_end_position,
                },
            ));
        }
        if self.bit_at(trailing_bit_position) != Some(1) {
            return Err(self.state_error_at_bit(
                trailing_bit_position,
                SymbolDecoderErrorKind::MissingTrailingOneBit,
            ));
        }

        for bit_position in trailing_bit_position + 1..padding_end_position {
            if self.bit_at(bit_position) != Some(0) {
                return Err(self
                    .state_error_at_bit(bit_position, SymbolDecoderErrorKind::NonZeroPaddingBit));
            }
        }

        Ok(SymbolDecoderSummary {
            consumed_bits: SymbolBitPosition::new(padding_end_position),
            symbol_count: self.frame_symbol_count,
            trailing_bit_position: SymbolBitPosition::new(trailing_bit_position),
            padding_end_position: SymbolBitPosition::new(padding_end_position),
        })
    }

    /// Alias for [`Self::exit_symbol`].
    ///
    /// # Errors
    /// Returns the same errors as [`Self::exit_symbol`].
    pub fn finish(self) -> Result<SymbolDecoderSummary> {
        self.exit_symbol()
    }

    fn validate_cdf(&self, cdf: &[i32]) -> Result<CdfShape> {
        validate_cdf_shape(cdf).map_err(|kind| self.cdf_error(kind))
    }

    fn num_bits_to_read(&self, bits: u32) -> u32 {
        if self.symbol_max_bits <= 0 {
            0
        } else if self.symbol_max_bits >= i64::from(bits) {
            bits
        } else {
            self.symbol_max_bits as u32
        }
    }

    fn bit_at(&self, bit_position: u64) -> Option<u8> {
        let byte_index = usize::try_from(bit_position / 8).ok()?;
        let bit_offset = (bit_position % 8) as u8;
        let byte = *self.data.get(byte_index)?;
        Some((byte >> (7 - bit_offset)) & 1)
    }

    fn cdf_error(&self, kind: SymbolCdfErrorKind) -> Error {
        Error::InvalidSymbolCdf {
            offset: self.reader.byte_offset(),
            bit_offset: self.reader.bit_offset(),
            kind,
        }
    }

    fn state_error(&self, kind: SymbolDecoderErrorKind) -> Error {
        Error::InvalidSymbolDecoderState {
            offset: self.reader.byte_offset(),
            bit_offset: self.reader.bit_offset(),
            kind,
        }
    }

    fn state_error_at_bit(&self, bit_position: u64, kind: SymbolDecoderErrorKind) -> Error {
        let (offset, bit_offset) = self.offset_for_bit(bit_position);
        Error::InvalidSymbolDecoderState {
            offset,
            bit_offset,
            kind,
        }
    }

    fn offset_for_bit(&self, bit_position: u64) -> (ByteOffset, BitOffset) {
        (
            self.base.saturating_add(bit_position / 8),
            BitOffset::from_bits((bit_position % 8) as u8),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CdfShape {
    pub(crate) n: usize,
    pub(crate) rate_index: usize,
    pub(crate) count: i32,
}

pub(crate) fn validate_cdf_shape(
    cdf: &[i32],
) -> core::result::Result<CdfShape, SymbolCdfErrorKind> {
    let len = cdf.len();
    let n = len.saturating_sub(1);
    if !(MIN_SYMBOLS..=MAX_SYMBOLS).contains(&n) {
        return Err(SymbolCdfErrorKind::UnsupportedLength { len });
    }

    for index in 0..n - 1 {
        let value = cdf[index];
        if !(1..=CDF_PROB_MAX).contains(&value) {
            return Err(SymbolCdfErrorKind::ProbabilityOutOfRange { index, value });
        }
        // AV2 § 8.2.6 (docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2) does
        // not require strictly increasing cumulative entries: the adaptation
        // step can drive adjacent entries equal, and `read_symbol` still
        // separates the affected symbols through `Prob_Inc`. Only a strict
        // decrease is rejected.
        if index > 0 && value < cdf[index - 1] {
            return Err(SymbolCdfErrorKind::DecreasingCumulative {
                previous_index: index - 1,
                index,
            });
        }
    }

    let rate_index = cdf[n - 1];
    if !(0..PARA_ADJUSTMENT_LIST.len() as i32).contains(&rate_index) {
        return Err(SymbolCdfErrorKind::AdaptationRateOutOfRange {
            index: n - 1,
            value: rate_index,
        });
    }

    let count = cdf[n];
    if !(0..=MAX_CDF_COUNT).contains(&count) {
        return Err(SymbolCdfErrorKind::CountOutOfRange {
            index: n,
            value: count,
        });
    }

    Ok(CdfShape {
        n,
        rate_index: rate_index as usize,
        count,
    })
}

fn symbol_max_bits_for_len(len: usize, base: ByteOffset) -> Result<i64> {
    let bytes = i64::try_from(len).map_err(|_| Error::InvalidSymbolDecoderState {
        offset: base,
        bit_offset: BitOffset::from_bits(0),
        kind: SymbolDecoderErrorKind::PayloadTooLarge { bytes: len },
    })?;
    bytes
        .checked_mul(8)
        .and_then(|bits| bits.checked_sub(15))
        .ok_or(Error::InvalidSymbolDecoderState {
            offset: base,
            bit_offset: BitOffset::from_bits(0),
            kind: SymbolDecoderErrorKind::PayloadTooLarge { bytes: len },
        })
}

fn total_bits(len: usize) -> u64 {
    match u64::try_from(len) {
        Ok(bytes) => bytes.saturating_mul(8),
        Err(_) => u64::MAX,
    }
}

pub(crate) fn floor_log2(value: u32) -> u32 {
    u32::BITS - 1 - value.leading_zeros()
}

pub(crate) fn update_cdf(cdf: &mut [i32], shape: CdfShape, symbol: usize) {
    let time_interval = if shape.count > 31 {
        2usize
    } else {
        usize::from(shape.count > 15)
    };
    let rate = 3
        + time_interval as i32
        + floor_log2(shape.n as u32).min(2) as i32
        + PARA_ADJUSTMENT_LIST[shape.rate_index][time_interval];
    let rate = rate as u32;

    for (index, entry) in cdf.iter_mut().take(shape.n - 1).enumerate() {
        if index < symbol {
            *entry -= *entry >> rate;
        } else {
            *entry += (CDF_PROB_SCALE as i32 - *entry) >> rate;
        }
    }
    if cdf[shape.n] < MAX_CDF_COUNT {
        cdf[shape.n] += 1;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "symbol_tests.rs"]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "symbol_proptests.rs"]
mod proptests;
