// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.2 symbol decoder foundation.
//!
//! This module implements the generic symbol-decoder primitives from AV2 v1.0.0
//! § 8.2 over a caller-provided tile payload byte slice
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`). It deliberately stops
//! before § 8.3 syntax-element CDF selection, tile CDF bank ownership,
//! `decode_tile()`, reconstruction, and encoder range writing.

#[cfg(target_arch = "x86_64")]
use fearless_simd::Avx2;
#[cfg(target_arch = "aarch64")]
use fearless_simd::Neon;
#[cfg(target_arch = "x86_64")]
use fearless_simd::SimdMask;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use fearless_simd::{
    Bytes, Level, Select, Simd, SimdBase, SimdInt, SimdInto, i16x8, mask16x8, u16x8, u64x2,
};

/// The token of the lane reads: AVX2 on x86-64, NEON on AArch64.
#[cfg(target_arch = "x86_64")]
type LaneToken = Avx2;
/// The token of the lane reads: AVX2 on x86-64, NEON on AArch64.
#[cfg(target_arch = "aarch64")]
type LaneToken = Neon;

use crate::bitio::be_window;
use crate::error::{Error, Result, SymbolCdfErrorKind, SymbolDecoderErrorKind};
use crate::span::{BitOffset, ByteOffset};
use crate::tables::conversion::PARA_ADJUSTMENT_LIST;

pub(crate) const CDF_PROB_SCALE: u32 = 1 << 15;
pub(crate) const CDF_PROB_MAX: i32 = (1 << 15) - 1;
pub(crate) const EC_PROB_SHIFT: u32 = 7;
pub(crate) const SYMBOL_RANGE_INIT: u32 = 1 << 15;
pub(crate) const MIN_SYMBOLS: usize = 2;
pub(crate) const MAX_SYMBOLS: usize = 8;
pub(crate) const MAX_LITERAL_BITS: u32 = 32;
pub(crate) const MAX_CDF_COUNT: i32 = 32;
/// A const copy of the static table, so per-arity reads fold to immediates
/// in the crates that instantiate the generic decode.
const PROB_INC_U32: [[u32; 8]; 7] = [
    [8, 0, 0, 0, 0, 0, 0, 0],
    [10, 5, 0, 0, 0, 0, 0, 0],
    [12, 8, 4, 0, 0, 0, 0, 0],
    [12, 9, 6, 3, 0, 0, 0, 0],
    [13, 10, 8, 5, 2, 0, 0, 0],
    [13, 11, 9, 6, 4, 2, 0, 0],
    [14, 12, 10, 8, 6, 4, 2, 0],
];

pub(crate) trait CdfStorage: Copy {
    fn to_i32(self) -> i32;
    fn to_u32(self) -> u32 {
        self.to_i32() as u32
    }
    fn from_i32(value: i32) -> Self;

    /// Adapts the `n - 1` probability entries of a row (§ 8.2.6).
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn adapt_probabilities(cdf: &mut [Self], n: usize, rate: u32, symbol: usize) {
        adapt_probabilities_scalar(cdf, 0..n - 1, rate, symbol);
    }
}

impl CdfStorage for i32 {
    fn to_i32(self) -> i32 {
        self
    }

    fn from_i32(value: i32) -> Self {
        value
    }
}

impl CdfStorage for u16 {
    fn to_i32(self) -> i32 {
        i32::from(self)
    }

    fn to_u32(self) -> u32 {
        u32::from(self)
    }

    fn from_i32(value: i32) -> Self {
        debug_assert!((0..=i32::from(u16::MAX)).contains(&value));
        value as u16
    }

    /// Adapts whole rows in vector lanes; `n` is a constant at every inlined
    /// read, so the arity match folds away.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn adapt_probabilities(cdf: &mut [u16], n: usize, rate: u32, symbol: usize) {
        match n {
            2 => {
                if let Some(entry) = cdf.first_mut() {
                    let value = i32::from(*entry);
                    let grown = value + (((CDF_PROB_SCALE as i32) - value) >> rate);
                    let shrunk = value - (value >> rate);
                    let keep = -i32::from(symbol == 0);
                    *entry = (shrunk ^ ((shrunk ^ grown) & keep)) as u16;
                }
            }
            3..=5 => adapt_probability_lanes::<4>(cdf, 0, n - 1, rate, symbol),
            6 => {
                adapt_probabilities_scalar(cdf, 0..1, rate, symbol);
                adapt_probability_lanes::<4>(cdf, 1, n - 1, rate, symbol);
            }
            7 | 8 => adapt_probability_lanes::<8>(cdf, 0, n - 1, rate, symbol),
            _ => adapt_probabilities_scalar(cdf, 0..n.saturating_sub(1), rate, symbol),
        }
    }
}

/// The two-sided § 8.2.6 step on `range`. The grow branch uses wrapping
/// arithmetic: identical for in-range entries, and panic-free under overflow
/// checks for trusted rows with hostile entries.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn adapt_probabilities_scalar<T: CdfStorage>(
    cdf: &mut [T],
    range: core::ops::Range<usize>,
    rate: u32,
    symbol: usize,
) {
    let start = range.start;
    let Some(entries) = cdf.get_mut(range) else {
        return;
    };
    for (offset, entry) in entries.iter_mut().enumerate() {
        let value = entry.to_i32();
        if start + offset < symbol {
            *entry = T::from_i32(value - (value >> rate));
        } else {
            let gap = (CDF_PROB_SCALE as i32).wrapping_sub(value);
            *entry = T::from_i32(value.wrapping_add(gap >> rate));
        }
    }
}

/// The § 8.2.6 step on the `L` entries from `first`, of which those below
/// `probabilities` are probabilities. Equal to the scalar step for entries up
/// to `CDF_PROB_SCALE`.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn adapt_probability_lanes<const L: usize>(
    cdf: &mut [u16],
    first: usize,
    probabilities: usize,
    rate: u32,
    symbol: usize,
) {
    if cdf.len() < first + L {
        return;
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    if let Some(token) = lane_token() {
        let lanes = &mut cdf[first..first + L];
        return adapt_row_lanes(token, lanes, first, probabilities, rate, symbol);
    }
    adapt_probabilities_scalar(cdf, first..probabilities.min(first + L), rate, symbol);
}

/// Four, five or eight row entries in one register without a stack round trip.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn load_row_lanes(token: LaneToken, row: &[u16], len: usize) -> u16x8<LaneToken> {
    if len >= 8 {
        return u16x8::from_slice(token, &row[..8]);
    }
    let word = u64::from(row[0])
        | u64::from(row[1]) << 16
        | u64::from(row[2]) << 32
        | u64::from(row[3]) << 48;
    let fifth = if len == 5 { u64::from(row[4]) } else { 0 };
    u64x2::load_array(token, [word, fifth]).bitcast()
}

/// [`adapt_probability_lanes`] on the four or eight entries of `lanes`.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn adapt_row_lanes(
    token: LaneToken,
    lanes: &mut [u16],
    first: usize,
    probabilities: usize,
    rate: u32,
    symbol: usize,
) {
    let value = load_row_lanes(token, lanes, lanes.len());
    let index = i16x8::load_array(token, [0, 1, 2, 3, 4, 5, 6, 7]) + first as i16;
    let shrunk = value - (value >> rate);
    let grown = value + ((u16x8::splat(token, CDF_PROB_SCALE as u16) - value) >> rate);
    let adapted = index
        .simd_lt(i16x8::splat(token, symbol as i16))
        .select(shrunk, grown);
    let adapted = index
        .simd_lt(i16x8::splat(token, probabilities as i16))
        .select(adapted, value);
    if lanes.len() >= 8 {
        adapted.store_slice(&mut lanes[..8]);
    } else {
        let word = adapted.bitcast::<u64x2<LaneToken>>().as_array()[0];
        for (lane, entry) in lanes.iter_mut().take(4).enumerate() {
            *entry = (word >> (16 * lane)) as u16;
        }
    }
}

/// The lane token of this target, bound directly so the hot reads inline.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn lane_token() -> Option<LaneToken> {
    Level::baseline().as_avx2()
}

/// The lane token of this target, bound directly so the hot reads inline.
#[cfg(target_arch = "aarch64")]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn lane_token() -> Option<LaneToken> {
    Level::baseline().as_neon()
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// The signed high half of each lane product (`pmulhw`).
    #[inline]
    fn mul_high_avx2(token: Avx2, a: i16x8<Avx2>, b: i16x8<Avx2>) -> i16x8<Avx2> {
        core::arch::x86_64::_mm_mulhi_epi16(a.into(), b.into()).simd_into(token)
    }
);

/// `(range7 * b) >> 16` in each lane, the signed high half of the product.
#[cfg(target_arch = "x86_64")]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn mul_high(token: LaneToken, range7: u16, b: i16x8<LaneToken>) -> i16x8<LaneToken> {
    mul_high_avx2(token, i16x8::splat(token, range7 as i16), b)
}

#[cfg(target_arch = "aarch64")]
fearless_simd::kernel!(
    /// `(range7 * b) >> 16` in each lane, as `sqdmulh` of `range7 / 2`:
    /// `range7` is even and below `2^15`, so the doubled product is exact
    /// and never saturates.
    #[inline]
    fn mul_high(token: Neon, range7: u16, b: i16x8<Neon>) -> i16x8<Neon> {
        use core::arch::aarch64::{vdupq_n_s16, vqdmulhq_s16};
        debug_assert!(range7.is_multiple_of(2) && range7 < 1 << 15);
        vqdmulhq_s16(vdupq_n_s16((range7 >> 1) as i16), b.into()).simd_into(token)
    }
);

/// The § 8.2.6 adaptation rate `3 + timeInterval + n_term +
/// Para_Adjustment_List[ index ][ timeInterval ]`, `n_term` being
/// `Min(FloorLog2(N), 2)`, for every rate index and use count below 64, so a
/// read takes its whole rate with one load. The rows past the list repeat
/// row 0, so a masked index is always in bounds.
const RATE_ROWS: usize = PARA_ADJUSTMENT_LIST.len().next_power_of_two();
const fn adaptation_rates(n_term: i32) -> [[u8; 2 * MAX_CDF_COUNT as usize]; RATE_ROWS] {
    let mut rates = [[0; 2 * MAX_CDF_COUNT as usize]; RATE_ROWS];
    let mut index = 0;
    while index < RATE_ROWS {
        let list = if index < PARA_ADJUSTMENT_LIST.len() {
            index
        } else {
            0
        };
        let mut count = 0;
        while count < 2 * MAX_CDF_COUNT as usize {
            let time_interval = if count > 31 {
                2
            } else if count > 15 {
                1
            } else {
                0
            };
            rates[index][count] =
                (3 + time_interval as i32 + n_term + PARA_ADJUSTMENT_LIST[list][time_interval])
                    as u8;
            count += 1;
        }
        index += 1;
    }
    rates
}

/// The [`adaptation_rates`] of an `N`-ary row, one constant per `n_term` so
/// each crate inlining a read addresses its own copy directly instead of
/// through the global offset table.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn arity_rates<const N: usize>() -> &'static [[u8; 2 * MAX_CDF_COUNT as usize]; RATE_ROWS] {
    if N <= 3 {
        &const { adaptation_rates(1) }
    } else {
        &const { adaptation_rates(2) }
    }
}

/// `CDF_PROB_SCALE` in the lanes past the `N - 1` probabilities of a row.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const fn past_row_lanes<const N: usize>() -> [u16; 8] {
    let mut lanes = [0u16; 8];
    let mut i = N - 1;
    while i < 8 {
        lanes[i] = CDF_PROB_SCALE as u16;
        i += 1;
    }
    lanes
}

/// `Prob_Inc << 2` of an `N`-ary row in vector lanes.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const fn prob_inc_lanes<const N: usize>() -> [u16; 8] {
    let mut lanes = [0u16; 8];
    let mut i = 0;
    while i < 8 {
        lanes[i] = (PROB_INC_U32[N - 2][i] << 2) as u16;
        i += 1;
    }
    lanes
}

/// The § 8.2.6 boundary search of an `N`-ary row, stopping at the first
/// boundary at or below `SymbolValue`: a predicted branch per boundary
/// keeps the symbol off the range's dependency chain. Takes and returns
/// the range and boundaries shifted left by 16, see [`SymbolDecoder`].
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn early_exit_search<T: CdfStorage, const N: usize>(
    cdf: &[T],
    dif: u64,
    range_hi: u32,
) -> (usize, u32, u32) {
    let r8 = range_hi >> 24;
    let mut prev = range_hi;
    for symbol in 0..N - 1 {
        let f = CDF_PROB_SCALE.saturating_sub(cdf[symbol].to_u32());
        let pp = ((f >> EC_PROB_SHIFT) << 4) + PROB_INC_U32[N - 2][symbol];
        let cur = ((r8 * pp) >> 7) << 19;
        if dif >= u64::from(cur) << 32 {
            return (symbol, cur, prev);
        }
        prev = cur;
    }
    (N - 1, 0, prev)
}

/// The lane search of [`early_exit_search`] in one register: the signed
/// high-half multiply and the compare. The boundaries do not increase from
/// lane to lane, so the symbol's two are the ones beside the last lane above
/// the value, read back from memory. For rows of at most four symbols the
/// lanes fit one word each, so the count of lanes above (a prefix) and the
/// two boundaries are word shifts instead of an across-lane add and a
/// memory round trip. Returns the number of lanes above and
/// the lanes above, which for such rows are the lanes below the symbol.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn search_lanes<const N: usize>(
    token: LaneToken,
    row: u16x8<LaneToken>,
    past: [u16; 8],
    increments: [u16; 8],
    range7: u16,
    threshold: i16,
    range: u16,
) -> (u32, u16, u16, mask16x8<LaneToken>) {
    let probabilities = row | u16x8::load_array(token, past);
    let f = u16x8::splat(token, CDF_PROB_SCALE as u16).saturating_sub(probabilities);
    let pp4 = ((f >> EC_PROB_SHIFT) << 6) + u16x8::load_array(token, increments);
    let scaled = mul_high(token, range7, pp4.bitcast());
    let boundaries: u16x8<LaneToken> = (scaled << 3).bitcast();
    let below = scaled.simd_gt(i16x8::splat(token, threshold));
    if N <= 4 {
        let word = |lanes: i16x8<LaneToken>| lanes.bitcast::<u64x2<LaneToken>>().as_array()[0];
        let mask = word(token.to_vector_mask16x8(below));
        let above = (u64::BITS - mask.leading_zeros()) >> 4;
        let low = word(boundaries.bitcast());
        let cur = low.wrapping_shr(16 * above) as u16;
        let prev = ((low << 16) | u64::from(range)).wrapping_shr(16 * above) as u16;
        return (above, cur, prev, below);
    }
    #[cfg(target_arch = "x86_64")]
    let above = below.to_bitmask().count_ones();
    // NEON has no `movemask`: one across-lane add of the `-1` lanes counts them.
    #[cfg(target_arch = "aarch64")]
    let above = u32::from(
        token
            .reduce_sum_i16x8(token.to_vector_mask16x8(below))
            .unsigned_abs(),
    );
    let mut lanes = [range; 9];
    boundaries.store_slice(&mut lanes[1..]);
    let symbol = above as usize & 7;
    (above, lanes[symbol + 1], lanes[symbol], below)
}

/// The lane count [`load_row_lanes`] reads for an `N`-ary row.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const fn row_lanes<const N: usize>() -> usize {
    if N >= 7 {
        8
    } else if N == 6 {
        5
    } else {
        4
    }
}

/// The search of an `N`-ary row (`N > 2`) in vector lanes: the symbol, its
/// boundaries, the loaded row and the lanes above the value. On rows within
/// the § 8.2.6 invariants those lanes are the entries below the symbol, so
/// they select [`adapt_lanes`] directly; lanes past the row's `N - 1`
/// boundaries never count, so the symbol is at most `N - 1` without a clamp.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn row_search_lanes<const N: usize>(
    token: LaneToken,
    cdf: &[u16],
    symbol_value: u32,
    range: u32,
) -> (u32, u32, u32, u16x8<LaneToken>, mask16x8<LaneToken>) {
    let value = load_row_lanes(token, cdf, row_lanes::<N>());
    let (above, cur, prev, below) = search_lanes::<N>(
        token,
        value,
        const { past_row_lanes::<N>() },
        const { prob_inc_lanes::<N>() },
        ((range >> 1) & 0x7f80) as u16,
        (symbol_value >> 3) as i16,
        range as u16,
    );
    (above & 7, u32::from(cur), u32::from(prev), value, below)
}

/// The § 8.2.6 step on the `N - 1` probabilities of the row `value` was
/// loaded from, shrinking the lanes `below` the symbol.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn adapt_lanes<const N: usize>(
    token: LaneToken,
    cdf: &mut [u16],
    value: u16x8<LaneToken>,
    below: mask16x8<LaneToken>,
    rate: u32,
) {
    let shrunk = value - (value >> rate);
    let grown = value + ((u16x8::splat(token, CDF_PROB_SCALE as u16) - value) >> rate);
    let index = i16x8::load_array(token, [0, 1, 2, 3, 4, 5, 6, 7]);
    let adapted = index
        .simd_lt(i16x8::splat(token, N as i16 - 1))
        .select(below.select(shrunk, grown), value);
    if row_lanes::<N>() >= 8 {
        adapted.store_slice(&mut cdf[..8]);
    } else {
        let words = *adapted.bitcast::<u64x2<LaneToken>>().as_array();
        for (lane, entry) in cdf.iter_mut().take(4).enumerate() {
            *entry = (words[0] >> (16 * lane)) as u16;
        }
        if row_lanes::<N>() == 5 {
            cdf[4] = words[1] as u16;
        }
    }
}

/// The adaptation rate of a trusted `N`-ary row (§ 8.2.6), with its rate
/// index and count masked into the table.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn trusted_rate<const N: usize>(cdf: &[u16]) -> u32 {
    let rates = &arity_rates::<N>()[usize::from(cdf[N - 1]) & (RATE_ROWS - 1)];
    u32::from(rates[usize::from(cdf[N]) & (2 * MAX_CDF_COUNT as usize - 1)])
}

/// One in each 16-bit lane of a row word.
const LANES: u64 = 0x0001_0001_0001_0001;

/// `0xffff >> rate` in each probability lane of an `N`-ary row word, by any
/// `u8` rate: the bits of a lane shifted right by `rate` that came from the
/// lane itself.
const fn keep_lanes<const N: usize>() -> [u64; 256] {
    let mut keep = [0; 256];
    let mut rate = 0;
    while rate < 16 {
        keep[rate] = ((0xffff >> rate) * LANES) & probability_lanes::<N>();
        rate += 1;
    }
    keep
}

/// The row-word lanes below a symbol, at most four.
const LOWER_LANES: [u64; 8] = {
    let mut lanes = [u64::MAX; 8];
    let mut symbol = 0;
    while symbol < 4 {
        lanes[symbol] = (1 << (16 * symbol)) - 1;
        symbol += 1;
    }
    lanes
};

/// The entries of an `N`-ary row that its word holds, entry `i` in lane `i`:
/// the whole row up to four entries.
const fn word_entries<const N: usize>() -> usize {
    if N < 4 { N + 1 } else { 4 }
}

/// The lanes of a row word that hold probabilities.
const fn probability_lanes<const N: usize>() -> u64 {
    if N > 4 {
        u64::MAX
    } else {
        (1 << (16 * (N - 1))) - 1
    }
}

/// The [`word_entries`] of a row in one word.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn row_word<const N: usize>(cdf: &[u16]) -> u64 {
    cdf.iter()
        .take(word_entries::<N>())
        .enumerate()
        .fold(0, |word, (i, &entry)| word | u64::from(entry) << (16 * i))
}

/// Writes a row word back to the row.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn store_row_word<const N: usize>(cdf: &mut [u16], word: u64) {
    for (i, entry) in cdf.iter_mut().take(word_entries::<N>()).enumerate() {
        *entry = (word >> (16 * i)) as u16;
    }
}

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

/// Controls how much of a caller-supplied CDF row `read_symbol` validates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdfValidationMode {
    /// Validate the full row shape on every read: length, per-entry
    /// probability range and monotonicity, adaptation-rate index, and count.
    Validated,
    /// Validate only what indexing safety requires (length and
    /// adaptation-rate index). For callers whose rows come exclusively from
    /// decoder-owned banks — spec-default tables adapted by `update_cdf`,
    /// which preserves the full-row invariants — the two modes are
    /// observably identical. A malformed row still decodes panic-free (the
    /// arithmetic loop saturates the probability gap, bounding every
    /// downstream product) but may yield an arithmetic-state error or an
    /// arbitrary in-range symbol instead of a CDF shape error.
    Trusted,
}

/// Configuration for [`SymbolDecoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolDecoderConfig {
    cdf_update: CdfUpdateMode,
    cdf_validation: CdfValidationMode,
}

impl SymbolDecoderConfig {
    /// Creates the default symbol-decoder configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cdf_update: CdfUpdateMode::Enabled,
            cdf_validation: CdfValidationMode::Validated,
        }
    }

    /// Returns the configured CDF update mode.
    #[must_use]
    pub const fn cdf_update_mode(self) -> CdfUpdateMode {
        self.cdf_update
    }

    /// Returns a copy of this configuration with a different CDF validation
    /// mode.
    #[must_use]
    pub const fn with_cdf_validation_mode(mut self, mode: CdfValidationMode) -> Self {
        self.cdf_validation = mode;
        self
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
    /// Number of frame symbols counted by `read_literal`, `read_unary`, and
    /// `read_symbol`; see [`SymbolDecoder::symbol_count`].
    pub symbol_count: u64,
    /// Relative `trailingBitPosition` inside the tile payload.
    pub trailing_bit_position: SymbolBitPosition,
    /// Relative `paddingEndPosition` inside the tile payload.
    pub padding_end_position: SymbolBitPosition,
}

/// A run of bypass reads ([`SymbolDecoder::bypass_run`]) decoded 32 bits
/// at a time.
///
/// A bypass bit (§ 8.2.3) is one step of the binary long division of
/// `SymbolValue` and the bits below it by `SymbolRange`, which a bypass read
/// leaves unchanged: the bit is 1 when the step does not subtract. A chunk of
/// `n` bits is therefore one quotient of the `16 + n` leading `dif` bits, and
/// any prefix of `m` bits is that quotient shifted right by `n - m`.
#[derive(Debug, Clone, Copy)]
pub struct BypassRun {
    quotient: u64,
    left: u32,
    len: u32,
}

/// The bits a [`BypassRun`] decodes per division; at most the 32 bits a
/// refill guarantees below the value window.
const BYPASS_RUN_CHUNK_BITS: u32 = 32;

/// Snapshot of the AV2 § 8.2 arithmetic decoder state at a syntax boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolDecoderCheckpoint {
    /// Number of bits consumed from the tile payload.
    pub consumed_bits: SymbolBitPosition,
    /// Number of frame symbols counted by `read_literal`, `read_unary`, and
    /// `read_symbol`; see [`SymbolDecoder::symbol_count`].
    pub symbol_count: u64,
    /// Current signed `SymbolMaxBits` value.
    pub symbol_max_bits: i64,
    /// Current `SymbolValue` arithmetic decoder register.
    pub symbol_value: u32,
    /// Current `SymbolRange` arithmetic decoder register.
    pub symbol_range: u32,
}

/// Bit position of the active `SymbolValue` window inside [`SymbolDecoder::dif`].
const SV_SHIFT: u32 = 48;
/// Buffered future bits kept below the `SymbolValue` window after a refill.
const DIF_BUFFER_BITS: i32 = 48;

/// Bounded AV2 § 8.2 symbol decoder over one tile payload byte slice.
///
/// The arithmetic state lives in `dif`: bits `[63:48]` hold the AV2
/// `SymbolValue` register and the `buffered` bits directly below hold
/// upcoming payload bits, pre-inverted the way every § 8.2 read consumes
/// them (bits past the payload end buffer as ones, the inverted zero
/// padding). Renormalization therefore only shifts `dif`; the payload is
/// touched once per refill instead of once per symbol.
///
/// `range_hi` holds `SymbolRange << 16`, so renormalization shifts by its
/// leading zeros directly and boundaries compare against `dif` unshifted.
#[derive(Debug)]
pub struct SymbolDecoder<'a> {
    data: &'a [u8],
    base: ByteOffset,
    dif: u64,
    buffered: i32,
    fed_bits: u64,
    range_hi: u32,
    /// `8 * sz`; `SymbolMaxBits` is derived from it (see
    /// [`Self::symbol_max_bits`]) instead of being updated on every read.
    payload_bits: i64,
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
    /// The shift into the window seeds the 15-bit `SymbolValue` register
    /// (behind its leading zero bit) and leaves the next 48 payload bits
    /// buffered below it.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] if the payload is too large for
    /// the signed `SymbolMaxBits` state.
    pub fn with_base_and_config(
        tile_payload: &'a [u8],
        base: ByteOffset,
        config: SymbolDecoderConfig,
    ) -> Result<Self> {
        let payload_bits = symbol_max_bits_for_len(tile_payload.len(), base)? + 15;
        let dif = !be_window(tile_payload, 0) >> 1;
        Ok(Self {
            data: tile_payload,
            base,
            dif,
            buffered: DIF_BUFFER_BITS,
            fed_bits: 63,
            range_hi: SYMBOL_RANGE_INIT << 16,
            payload_bits,
            frame_symbol_count: 0,
            config,
        })
    }

    /// Tops the buffered window back up to [`DIF_BUFFER_BITS`] future bits,
    /// reading payload bits past the end as ones (inverted zero padding).
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn refill(&mut self) {
        let byte_index = usize::try_from(self.fed_bits / 8).unwrap_or(usize::MAX);
        let bit_offset = (self.fed_bits & 7) as u32;
        let bytes = byte_index
            .checked_add(8)
            .and_then(|end| self.data.get(byte_index..end))
            .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok());
        let raw = match bytes {
            Some(bytes) => u64::from_be_bytes(bytes),
            None => tail_window(self.data, byte_index),
        };
        let window = !(raw << bit_offset);
        let buffered = self.buffered.max(0) as u32;
        self.dif |= window >> (16 + buffered);
        self.fed_bits += u64::from(DIF_BUFFER_BITS as u32 - buffered);
        self.buffered = DIF_BUFFER_BITS;
    }

    /// Guarantees at least `bits` buffered future bits below the value window.
    #[inline]
    fn ensure_buffered(&mut self, bits: u32) {
        if self.buffered < bits as i32 {
            self.refill();
        }
    }

    /// Returns the current signed `SymbolMaxBits` value.
    ///
    /// Every read shifts `dif` by exactly the bits it subtracts from
    /// `SymbolMaxBits` (`8 * sz - 15` at init), and a refill moves bits from
    /// `fed_bits` into `buffered` one for one, so `SymbolMaxBits` is
    /// `8 * sz - fed_bits + buffered`.
    #[must_use]
    #[inline]
    pub const fn symbol_max_bits(&self) -> i64 {
        self.payload_bits - self.fed_bits as i64 + self.buffered as i64
    }

    /// Returns the number of counted frame symbols so far. Only builds with
    /// debug assertions or the `symbol-count` feature count; the hot decode
    /// loops skip the counter, so other builds return 0.
    #[must_use]
    pub const fn symbol_count(&self) -> u64 {
        self.frame_symbol_count
    }

    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    const fn count_symbols(&mut self, n: u64) {
        if cfg!(any(debug_assertions, feature = "symbol-count")) {
            self.frame_symbol_count = self.frame_symbol_count.wrapping_add(n);
        }
    }

    /// Copies the decoder state by value, for hot loops that keep the
    /// arithmetic registers (`dif`, range, buffered bits) in machine
    /// registers instead of behind a pointer. [`Self::restore`] writes the
    /// state back.
    #[must_use]
    pub const fn snapshot(&self) -> Self {
        Self {
            data: self.data,
            base: self.base,
            dif: self.dif,
            buffered: self.buffered,
            fed_bits: self.fed_bits,
            range_hi: self.range_hi,
            payload_bits: self.payload_bits,
            frame_symbol_count: self.frame_symbol_count,
            config: self.config,
        }
    }

    /// Replaces the decoder state with [`Self::snapshot`] output.
    pub const fn restore(&mut self, snapshot: Self) {
        *self = snapshot;
    }

    /// Returns the current relative bit position in the tile payload.
    ///
    /// `SymbolMaxBits` starts at `8 * sz - 15` and every read consumes
    /// exactly the bits it subtracts until the payload is exhausted, after
    /// which reads consume nothing, so the historical bounded-reader
    /// position is exactly `8 * sz - max(SymbolMaxBits, 0)`.
    #[must_use]
    #[inline]
    pub fn consumed_bits(&self) -> SymbolBitPosition {
        let total = total_bits(self.data.len());
        let remaining = self.symbol_max_bits().max(0) as u64;
        SymbolBitPosition::new(total.saturating_sub(remaining))
    }

    /// Returns the current AV2 `SymbolValue` register.
    const fn symbol_value(&self) -> u32 {
        (self.dif >> SV_SHIFT) as u32
    }

    /// Returns the current AV2 `SymbolRange` register.
    const fn range(&self) -> u32 {
        self.range_hi >> 16
    }

    /// Returns a lossless checkpoint of the current arithmetic decoder state.
    #[must_use]
    pub fn checkpoint(&self) -> SymbolDecoderCheckpoint {
        SymbolDecoderCheckpoint {
            consumed_bits: self.consumed_bits(),
            symbol_count: self.frame_symbol_count,
            symbol_max_bits: self.symbol_max_bits(),
            symbol_value: self.symbol_value(),
            symbol_range: self.range(),
        }
    }

    /// Decodes a pseudo-raw bit with equal probability, per AV2 § 8.2.3.
    ///
    /// # Errors
    /// Returns [`Error::UnexpectedEof`] if the bounded tile payload unexpectedly
    /// cannot supply a required coded bit.
    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_bypass_bits(1) != 0)
    }

    /// Decodes an AV2 § 8.2.5 `read_literal(n)` value, returned MSB-first.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] if `n > 32`, or propagates
    /// [`Error::UnexpectedEof`] from the bounded bit reader.
    #[inline]
    pub fn read_literal(&mut self, n: u32) -> Result<u32> {
        if n > MAX_LITERAL_BITS {
            return Err(
                self.state_error(SymbolDecoderErrorKind::LiteralWidthTooLarge {
                    requested: n,
                    max: MAX_LITERAL_BITS,
                }),
            );
        }

        self.count_symbols(u64::from(n));
        Ok(self.read_bypass_bits(n))
    }

    /// Decodes an AVM-style truncated unary bypass value.
    ///
    /// The result is the number of zero bits before the terminating one bit,
    /// capped at `max_bits`. When the cap is reached, exactly `max_bits` zero
    /// bits are consumed and no terminator is read. The bypass run is normalized
    /// once, matching AVM's `avm_read_unary`.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] if `max_bits > 32`, or
    /// propagates [`Error::UnexpectedEof`] from the bounded bit reader.
    pub fn read_unary(&mut self, max_bits: u32) -> Result<u32> {
        if max_bits > MAX_LITERAL_BITS {
            return Err(
                self.state_error(SymbolDecoderErrorKind::LiteralWidthTooLarge {
                    requested: max_bits,
                    max: MAX_LITERAL_BITS,
                }),
            );
        }
        if max_bits == 0 {
            return Ok(0);
        }

        let mut split = u64::from(self.range()) << max_bits;
        let mut value = 0u32;
        let mut consumed = 0u32;
        let mut scaled_value = self.scaled_bypass_value(max_bits);

        for _ in 0..max_bits {
            split >>= 1;
            consumed += 1;
            if scaled_value >= split {
                scaled_value -= split;
                value += 1;
            } else {
                break;
            }
        }

        let normalized = scaled_value >> (max_bits - consumed);
        let symbol_value = u32::try_from(normalized)
            .map_err(|_| self.state_error(SymbolDecoderErrorKind::InvalidArithmeticRange))?;
        self.advance_bypass(consumed, symbol_value);
        self.count_symbols(u64::from(consumed));

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
        self.read_symbol_row(cdf)
    }

    /// Decodes one AV2 § 8.2.6 symbol from a compact `u16` CDF row.
    ///
    /// The row layout and errors are identical to [`Self::read_symbol`].
    ///
    /// # Errors
    /// Returns the same errors as [`Self::read_symbol`].
    #[inline]
    pub fn read_symbol_u16(&mut self, cdf: &mut [u16]) -> Result<Symbol> {
        self.read_symbol_row(cdf)
    }

    /// Decodes one AV2 § 8.2.6 symbol from a fixed-length compact CDF row.
    ///
    /// The arity is a compile-time constant, so the read inlines into hot
    /// loops without the length dispatch of [`Self::read_symbol_u16`].
    ///
    /// # Errors
    /// Returns the same errors as [`Self::read_symbol`].
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    pub fn read_symbol_u16_array<const L: usize>(&mut self, cdf: &mut [u16; L]) -> Result<Symbol> {
        match L {
            3 => self.read_symbol_arity::<u16, 2>(cdf),
            4 => self.read_symbol_arity::<u16, 3>(cdf),
            5 => self.read_symbol_arity::<u16, 4>(cdf),
            6 => self.read_symbol_arity::<u16, 5>(cdf),
            7 => self.read_symbol_arity::<u16, 6>(cdf),
            8 => self.read_symbol_arity::<u16, 7>(cdf),
            9 => self.read_symbol_arity::<u16, 8>(cdf),
            _ => self.read_symbol_row(cdf),
        }
    }

    /// Dispatches on the row length so each arity's search loop and
    /// adaptation are unrolled with constant bounds.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_symbol_row<T: CdfStorage>(&mut self, cdf: &mut [T]) -> Result<Symbol> {
        match cdf.len() {
            3 => self.read_symbol_arity::<T, 2>(cdf),
            4 => self.read_symbol_arity::<T, 3>(cdf),
            5 => self.read_symbol_arity::<T, 4>(cdf),
            6 => self.read_symbol_arity::<T, 5>(cdf),
            7 => self.read_symbol_arity::<T, 6>(cdf),
            8 => self.read_symbol_arity::<T, 7>(cdf),
            9 => self.read_symbol_arity::<T, 8>(cdf),
            len => Err(self.cdf_error(SymbolCdfErrorKind::UnsupportedLength { len })),
        }
    }

    /// Decodes one symbol of a row with `N + 1` entries. The last symbol's
    /// `next_cur` is 0 (`f = 0` and `Prob_Inc` 0), so a search that passes
    /// every other boundary ends there. Both validation modes return
    /// `n = N`; restating it keeps `n` constant in the adaptation step.
    /// The validation branch stays out of the hot body: a conditional call
    /// there spills every loop register even when the mode is trusted.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_symbol_arity<T: CdfStorage, const N: usize>(
        &mut self,
        cdf: &mut [T],
    ) -> Result<Symbol> {
        if self.config.cdf_validation == CdfValidationMode::Validated {
            validate_cdf_shape(cdf).map_err(|kind| self.cdf_error(kind))?;
            return self.read_symbol_arity_hot::<T, N>(cdf);
        }
        self.read_symbol_arity_hot::<T, N>(cdf)
    }

    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_symbol_arity_hot<T: CdfStorage, const N: usize>(
        &mut self,
        cdf: &mut [T],
    ) -> Result<Symbol> {
        let rate_index = cdf[N - 1].to_i32();
        let Some(rates) = usize::try_from(rate_index)
            .ok()
            .filter(|&index| index < PARA_ADJUSTMENT_LIST.len())
            .and_then(|index| arity_rates::<N>().get(index))
        else {
            return Err(
                self.cdf_error(SymbolCdfErrorKind::AdaptationRateOutOfRange {
                    index: N - 1,
                    value: rate_index,
                }),
            );
        };
        let count = cdf[N].to_i32();
        self.ensure_buffered(15);
        let (symbol, cur, prev) = early_exit_search::<T, N>(cdf, self.dif, self.range_hi);

        if prev.saturating_sub(cur) == 0 {
            return Err(self.state_error(SymbolDecoderErrorKind::InvalidArithmeticRange));
        }
        self.consume(cur, prev);

        if self.config.cdf_update == CdfUpdateMode::Enabled {
            let rate = u32::from(rates[count.clamp(0, MAX_CDF_COUNT * 2 - 1) as usize]);
            T::adapt_probabilities(cdf, N, rate, symbol);
            if count < MAX_CDF_COUNT {
                cdf[N] = T::from_i32(count + 1);
            }
        }

        Ok(Symbol::new(symbol as u8))
    }

    /// The configured CDF update mode, the `UPDATE` of [`Self::read_trusted`].
    #[must_use]
    pub const fn cdf_update_mode(&self) -> CdfUpdateMode {
        self.config.cdf_update
    }

    /// Decodes one symbol of an `L`-entry row the caller owns and keeps
    /// within the § 8.2.6 invariants (spec defaults adapted only by reads),
    /// with `UPDATE` equal to [`Self::cdf_update_mode`] being enabled. It
    /// skips every per-read check in either validation mode; a row outside
    /// the invariants decodes panic-free to an arbitrary symbol.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    pub fn read_trusted<const L: usize, const UPDATE: bool>(&mut self, cdf: &mut [u16; L]) -> u32 {
        self.read_trusted_len(cdf, UPDATE).unwrap_or(0)
    }

    /// [`Self::read_trusted`] of a row of three to five symbols, from its
    /// first entries as `carried`, the word the previous read of the same
    /// row returned, or from memory when `None`. Returns the symbol and the
    /// row's word after the read: a loop that reads one row repeatedly takes
    /// it from a register instead of waiting for its own store.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    pub fn read_trusted_carried<const L: usize, const UPDATE: bool>(
        &mut self,
        cdf: &mut [u16; L],
        carried: Option<u64>,
    ) -> (u32, u64) {
        match L {
            4 => self.read_carried::<3>(cdf, carried, UPDATE),
            5 => self.read_carried::<4>(cdf, carried, UPDATE),
            6 => self.read_carried::<5>(cdf, carried, UPDATE),
            _ => (self.read_trusted_len(cdf, UPDATE).unwrap_or(0), 0),
        }
    }

    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_carried<const N: usize>(
        &mut self,
        cdf: &mut [u16],
        carried: Option<u64>,
        update: bool,
    ) -> (u32, u64) {
        if cdf.len() <= N {
            return (0, 0);
        }
        debug_assert!(carried.is_none_or(|word| word == row_word::<N>(cdf)));
        self.ensure_buffered(15);
        match carried {
            Some(word) => self.read_trusted_word::<N>(cdf, word, update),
            None => self.read_trusted_word::<N>(cdf, row_word::<N>(cdf), update),
        }
    }

    /// [`Self::read_trusted`] of a slice row in the configured update mode.
    /// `None`, for the checked read, under [`CdfValidationMode::Validated`]
    /// or for a length no arity has.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    pub fn read_trusted_row(&mut self, cdf: &mut [u16]) -> Option<u32> {
        if self.config.cdf_validation == CdfValidationMode::Validated {
            return None;
        }
        self.read_trusted_len(cdf, self.config.cdf_update == CdfUpdateMode::Enabled)
    }

    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_trusted_len(&mut self, cdf: &mut [u16], update: bool) -> Option<u32> {
        Some(match cdf.len() {
            3 => self.read_trusted_arity::<2>(cdf, update),
            4 => self.read_trusted_arity::<3>(cdf, update),
            5 => self.read_trusted_arity::<4>(cdf, update),
            6 => self.read_trusted_arity::<5>(cdf, update),
            7 => self.read_trusted_arity::<6>(cdf, update),
            8 => self.read_trusted_arity::<7>(cdf, update),
            9 => self.read_trusted_arity::<8>(cdf, update),
            _ => return None,
        })
    }

    /// [`Self::read_bool`] without its unreachable error path, for hot loops.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    pub fn read_bool_trusted(&mut self) -> bool {
        let scaled = self.scaled_bypass_value(1);
        let split = u64::from(self.range());
        let one = scaled < split;
        let rest = if one { scaled } else { scaled - split };
        self.advance_bypass(1, rest as u32);
        self.count_symbols(1);
        one
    }

    /// Starts a run of [`Self::read_bool_in`] reads; [`Self::end_bypass_run`]
    /// applies the bits it read before any other read.
    #[must_use]
    pub const fn bypass_run(&self) -> BypassRun {
        BypassRun {
            quotient: 0,
            left: 0,
            len: 0,
        }
    }

    /// Consumes the bits a [`Self::bypass_run`] has read: the remainder of
    /// their quotient becomes `SymbolValue`.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    pub const fn end_bypass_run(&mut self, run: BypassRun) {
        let bits = run.len.saturating_sub(run.left);
        let bits = if bits < BYPASS_RUN_CHUNK_BITS {
            bits
        } else {
            BYPASS_RUN_CHUNK_BITS
        };
        let dividend = self.dif >> (SV_SHIFT - bits);
        let quotient = run.quotient >> (run.left & 63);
        let rest = dividend.wrapping_sub(quotient.wrapping_mul(self.range() as u64));
        self.dif = (rest << SV_SHIFT) | ((self.dif << bits) & ((1 << SV_SHIFT) - 1));
        self.buffered -= bits as i32;
        self.count_symbols(bits as u64);
    }

    /// [`Self::read_bool_trusted`] on a [`Self::bypass_run`].
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    pub fn read_bool_in(&mut self, run: &mut BypassRun) -> bool {
        if run.left == 0 {
            *run = self.refill_run(*run);
        }
        run.left -= 1;
        (run.quotient >> run.left) & 1 == 0
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: out of line it saves and reloads the decoder"
    )]
    #[inline(always)]
    fn refill_run(&mut self, run: BypassRun) -> BypassRun {
        self.end_bypass_run(run);
        self.ensure_buffered(BYPASS_RUN_CHUNK_BITS);
        let dividend = self.dif >> (SV_SHIFT - BYPASS_RUN_CHUNK_BITS);
        BypassRun {
            quotient: dividend / u64::from(self.range().max(1)),
            left: BYPASS_RUN_CHUNK_BITS,
            len: BYPASS_RUN_CHUNK_BITS,
        }
    }

    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_trusted_arity<const N: usize>(&mut self, cdf: &mut [u16], update: bool) -> u32 {
        if cdf.len() <= N {
            return 0;
        }
        self.ensure_buffered(15);
        if (3..=6).contains(&N) {
            return self
                .read_trusted_word::<N>(cdf, row_word::<N>(cdf), update)
                .0;
        }
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        if N > 2
            && let Some(token) = lane_token()
        {
            let symbol_value = (self.dif >> SV_SHIFT) as u32;
            return self.read_trusted_lanes::<N>(token, cdf, symbol_value, update);
        }
        let (symbol, cur, prev) = early_exit_search::<u16, N>(cdf, self.dif, self.range_hi);
        self.consume(cur, prev);
        if update {
            let count = cdf[N];
            let rate = trusted_rate::<N>(cdf);
            u16::adapt_probabilities(cdf, N, rate, symbol);
            cdf[N] = count + u16::from(count < MAX_CDF_COUNT as u16);
        }
        symbol as u32
    }

    /// [`Self::read_trusted_arity`] for rows of three to six symbols, all in
    /// general registers (the fifth probability of a six-symbol row on its
    /// own): the boundaries are independent products of
    /// the range, so the symbol and its interval are selects instead of a
    /// vector round trip, and the § 8.2.6 step adapts the row as one word.
    /// Each lane holds `p` and `gap = CDF_PROB_SCALE - p`, neither borrowing
    /// from the next lane for entries up to `CDF_PROB_SCALE`; a boundary
    /// `((r8 * pp) >> 7) << 19` is one product and a mask. Both steps,
    /// `p >> rate` and `gap >> rate`, are ready before the symbol, which only
    /// selects them through [`LOWER_LANES`]: the lanes below the symbol
    /// shrink, the others grow; lanes without a probability stay.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_trusted_word<const N: usize>(
        &mut self,
        cdf: &mut [u16],
        word: u64,
        update: bool,
    ) -> (u32, u64) {
        let gap = (u64::from(CDF_PROB_SCALE) * LANES).wrapping_sub(word);
        let r12 = (self.range_hi >> 24) << 12;
        let mut boundaries = [0u32; 6];
        for (i, boundary) in boundaries.iter_mut().enumerate().take((N - 1).min(4)) {
            let pp = ((gap >> (16 * i + 3)) as u32 & 0x1ff0) | PROB_INC_U32[N - 2][i];
            *boundary = r12.wrapping_mul(pp) & 0xfff8_0000;
        }
        let fifth = if N == 6 { u32::from(cdf[4]) } else { 0 };
        let fifth_gap = CDF_PROB_SCALE.wrapping_sub(fifth);
        if N == 6 {
            let pp = ((fifth_gap >> (EC_PROB_SHIFT - 4)) & 0x1ff0) | PROB_INC_U32[N - 2][4];
            boundaries[4] = r12.wrapping_mul(pp) & 0xfff8_0000;
        }
        let value = (self.dif >> 32) as u32;
        let (mut cur, mut prev, mut symbol) = (boundaries[0], self.range_hi, 0);
        for i in 0..N - 1 {
            let above = value < boundaries[i];
            cur = if above { boundaries[i + 1] } else { cur };
            prev = if above { boundaries[i] } else { prev };
            symbol += u32::from(above);
        }
        self.consume(cur, prev);
        if update {
            let below = LOWER_LANES[symbol as usize & 7];
            let (index, count) = if N < 4 {
                ((word >> (16 * (N - 1))) as u16, (word >> (16 * N)) as u16)
            } else {
                (cdf[N - 1], cdf[N])
            };
            let rate = arity_rates::<N>()[usize::from(index) & (RATE_ROWS - 1)]
                [usize::from(count) & (2 * MAX_CDF_COUNT as usize - 1)];
            let keep = const { keep_lanes::<N>() }[usize::from(rate)];
            let rate = u32::from(rate);
            let (shrink, grow) = ((word >> rate) & keep, (gap >> rate) & keep);
            let adapted = word
                .wrapping_add(grow & !below)
                .wrapping_sub(shrink & below);
            let grown = u16::from(count < MAX_CDF_COUNT as u16);
            if N < 4 {
                let adapted = adapted + (u64::from(grown) << (16 * N));
                store_row_word::<N>(cdf, adapted);
                return (symbol, adapted);
            }
            store_row_word::<N>(cdf, adapted);
            if N == 6 {
                let shrinks = symbol > 4;
                let step = (if shrinks { fifth } else { fifth_gap }) >> rate;
                cdf[4] = if shrinks {
                    fifth - step
                } else {
                    fifth.wrapping_add(step)
                } as u16;
            }
            cdf[N] = count + grown;
            return (symbol, adapted);
        }
        (symbol, word)
    }

    /// [`Self::read_trusted_arity`] for `N > 6` in vector lanes, see
    /// [`row_search_lanes`]. Branch-free on purpose: coefficient symbols
    /// mispredict too often for [`early_exit_search`] on some streams.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn read_trusted_lanes<const N: usize>(
        &mut self,
        token: LaneToken,
        cdf: &mut [u16],
        symbol_value: u32,
        update: bool,
    ) -> u32 {
        let (symbol, cur, prev, value, below) =
            row_search_lanes::<N>(token, cdf, symbol_value, self.range());
        self.consume(cur << 16, prev << 16);
        if update {
            let count = cdf[N];
            adapt_lanes::<N>(token, cdf, value, below, trusted_rate::<N>(cdf));
            cdf[N] = count + u16::from(count < MAX_CDF_COUNT as u16);
        }
        symbol
    }

    /// Narrows the interval to `cur..prev` (§ 8.2.6, both shifted left by
    /// 16) and renormalizes. The interval is never empty for rows within the
    /// § 8.2.6 invariants; an empty one leaves an arbitrary state.
    #[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
    #[inline(always)]
    fn consume(&mut self, cur: u32, prev: u32) {
        let new_range = prev.wrapping_sub(cur);
        let bits = new_range.leading_zeros();
        self.range_hi = new_range.wrapping_shl(bits);
        self.dif = self.dif.wrapping_sub(u64::from(cur) << 32) << bits;
        self.buffered -= bits as i32;
        self.count_symbols(1);
    }

    /// Validates AV2 § 8.2.4 `exit_symbol()` and returns the final decoder summary.
    ///
    /// # Errors
    /// Returns [`Error::InvalidSymbolDecoderState`] when `SymbolMaxBits < -14`, the
    /// computed trailing bit is missing or not `1`, or any padding bit before
    /// `paddingEndPosition` is nonzero.
    pub fn exit_symbol(self) -> Result<SymbolDecoderSummary> {
        let symbol_max_bits = self.symbol_max_bits();
        if symbol_max_bits < -14 {
            return Err(
                self.state_error(SymbolDecoderErrorKind::SymbolMaxBitsTooSmall { symbol_max_bits })
            );
        }

        let current = self.consumed_bits().get();
        let rewind = u64::try_from((symbol_max_bits + 15).min(15)).map_err(|_| {
            self.state_error(SymbolDecoderErrorKind::SymbolMaxBitsTooSmall { symbol_max_bits })
        })?;
        let trailing_bit_position = current.checked_sub(rewind).ok_or_else(|| {
            self.state_error(SymbolDecoderErrorKind::TrailingBitOutOfRange { bit_position: 0 })
        })?;
        let skip = if symbol_max_bits > 0 {
            symbol_max_bits as u64
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

    /// Reads `bits` (at most [`MAX_LITERAL_BITS`]) bypass bits in one
    /// division, see [`BypassRun`].
    #[inline]
    fn read_bypass_bits(&mut self, bits: u32) -> u32 {
        let dividend = self.scaled_bypass_value(bits);
        let range = u64::from(self.range().max(1));
        let quotient = dividend / range;
        self.advance_bypass(bits, (dividend - quotient * range) as u32);
        (!quotient & ((1 << bits) - 1)) as u32
    }

    #[inline]
    fn scaled_bypass_value(&mut self, bits: u32) -> u64 {
        self.ensure_buffered(bits);
        self.dif >> (SV_SHIFT - bits)
    }

    #[inline]
    fn advance_bypass(&mut self, bits: u32, symbol_value: u32) {
        self.dif =
            (u64::from(symbol_value) << SV_SHIFT) | ((self.dif << bits) & ((1 << SV_SHIFT) - 1));
        self.buffered -= bits as i32;
    }

    fn bit_at(&self, bit_position: u64) -> Option<u8> {
        let byte_index = usize::try_from(bit_position / 8).ok()?;
        let bit_offset = (bit_position % 8) as u8;
        let byte = *self.data.get(byte_index)?;
        Some((byte >> (7 - bit_offset)) & 1)
    }

    #[inline]
    fn cdf_error(&self, kind: SymbolCdfErrorKind) -> Error {
        cdf_error_at(self.base, self.consumed_bits().get(), kind)
    }

    #[inline]
    fn state_error(&self, kind: SymbolDecoderErrorKind) -> Error {
        state_error_at(self.base, self.consumed_bits().get(), kind)
    }

    fn state_error_at_bit(&self, bit_position: u64, kind: SymbolDecoderErrorKind) -> Error {
        state_error_at(self.base, bit_position, kind)
    }
}

/// The payload tail, zero-padded. Inlined: a call in a hot loop forces the
/// loop's decoder state out of caller-saved registers.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn tail_window(data: &[u8], byte_index: usize) -> u64 {
    be_window(data, byte_index)
}

/// Error constructors are cold and take the position by value, so an inlined
/// read keeps only a call on its failure path.
#[cold]
fn cdf_error_at(base: ByteOffset, bit_position: u64, kind: SymbolCdfErrorKind) -> Error {
    let (offset, bit_offset) = offset_for_bit(base, bit_position);
    Error::InvalidSymbolCdf {
        offset,
        bit_offset,
        kind,
    }
}

#[cold]
fn state_error_at(base: ByteOffset, bit_position: u64, kind: SymbolDecoderErrorKind) -> Error {
    let (offset, bit_offset) = offset_for_bit(base, bit_position);
    Error::InvalidSymbolDecoderState {
        offset,
        bit_offset,
        kind,
    }
}

fn offset_for_bit(base: ByteOffset, bit_position: u64) -> (ByteOffset, BitOffset) {
    (
        base.saturating_add(bit_position / 8),
        BitOffset::from_bits((bit_position % 8) as u8),
    )
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CdfShape {
    pub(crate) n: usize,
    pub(crate) rate_index: usize,
    pub(crate) count: i32,
}

/// Extracts the row shape with only the checks indexing safety requires
/// (see [`CdfValidationMode::Trusted`]): the length bound that keeps every
/// row and `PROB_INC` access in range, and the adaptation-rate bound that
/// keeps the `PARA_ADJUSTMENT_LIST` lookup in range.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
pub(crate) fn trusted_cdf_shape<T: CdfStorage>(
    cdf: &[T],
) -> core::result::Result<CdfShape, SymbolCdfErrorKind> {
    let len = cdf.len();
    let n = len.saturating_sub(1);
    if !(MIN_SYMBOLS..=MAX_SYMBOLS).contains(&n) {
        return Err(SymbolCdfErrorKind::UnsupportedLength { len });
    }
    let rate_index = cdf[n - 1].to_i32();
    if !(0..PARA_ADJUSTMENT_LIST.len() as i32).contains(&rate_index) {
        return Err(SymbolCdfErrorKind::AdaptationRateOutOfRange {
            index: n - 1,
            value: rate_index,
        });
    }
    Ok(CdfShape {
        n,
        rate_index: rate_index as usize,
        count: cdf[n].to_i32(),
    })
}

pub(crate) fn validate_cdf_shape<T: CdfStorage>(
    cdf: &[T],
) -> core::result::Result<CdfShape, SymbolCdfErrorKind> {
    let len = cdf.len();
    let n = len.saturating_sub(1);
    if !(MIN_SYMBOLS..=MAX_SYMBOLS).contains(&n) {
        return Err(SymbolCdfErrorKind::UnsupportedLength { len });
    }

    for index in 0..n - 1 {
        let value = cdf[index].to_i32();
        if !(1..=CDF_PROB_MAX).contains(&value) {
            return Err(SymbolCdfErrorKind::ProbabilityOutOfRange { index, value });
        }
        if index > 0 && value < cdf[index - 1].to_i32() {
            return Err(SymbolCdfErrorKind::DecreasingCumulative {
                previous_index: index - 1,
                index,
            });
        }
    }

    let rate_index = cdf[n - 1].to_i32();
    if !(0..PARA_ADJUSTMENT_LIST.len() as i32).contains(&rate_index) {
        return Err(SymbolCdfErrorKind::AdaptationRateOutOfRange {
            index: n - 1,
            value: rate_index,
        });
    }

    let count = cdf[n].to_i32();
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

#[inline]
fn total_bits(len: usize) -> u64 {
    match u64::try_from(len) {
        Ok(bytes) => bytes.saturating_mul(8),
        Err(_) => u64::MAX,
    }
}

#[cfg(test)]
fn mask_for_bits(bits: u32) -> u64 {
    if bits == 0 { 0 } else { (1u64 << bits) - 1 }
}

#[inline]
pub(crate) fn floor_log2(value: u32) -> u32 {
    u32::BITS - 1 - value.leading_zeros()
}

/// Applies the AV2 § 8.2.6 adaptation step.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
pub(crate) fn update_cdf<T: CdfStorage>(cdf: &mut [T], shape: CdfShape, symbol: usize) {
    update_cdf_arity::<T>(cdf, shape, shape.n, symbol);
}

/// Applies the AV2 § 8.2.6 adaptation step with compile-time or explicit arity `n`.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
pub(crate) fn update_cdf_arity<T: CdfStorage>(
    cdf: &mut [T],
    shape: CdfShape,
    n: usize,
    symbol: usize,
) {
    update_cdf_with_rates(
        cdf,
        &PARA_ADJUSTMENT_LIST[shape.rate_index],
        shape.count,
        n,
        symbol,
    );
}

/// [`update_cdf_arity`] with the row's `Para_Adjustment_List` entry and
/// use count already read.
#[allow(clippy::inline_always, reason = "measured symbol-decode hot path")]
#[inline(always)]
fn update_cdf_with_rates<T: CdfStorage>(
    cdf: &mut [T],
    rates: &[i32; 3],
    count: i32,
    n: usize,
    symbol: usize,
) {
    let time_interval = if count > 31 {
        2usize
    } else {
        usize::from(count > 15)
    };
    let n_term = match n {
        2 | 3 => 1,
        _ => 2,
    };
    let rate = 3 + time_interval as i32 + n_term + rates[time_interval];
    T::adapt_probabilities(cdf, n, rate as u32, symbol);
    if count < MAX_CDF_COUNT {
        cdf[n] = T::from_i32(count + 1);
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
