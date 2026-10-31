// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rate-distortion framework: symbol rates from CDF rows, the Lagrange
//! multiplier, and the recursive partition search.
//!
//! Every quantity is an integer, so a decision never depends on float rounding.

pub(crate) mod coeff_rate;
pub(crate) mod partition;
pub(crate) mod stages;

use core::ops::{Add, AddAssign};

use splot_core::symbol_encoder::symbol_split_q12;
use splot_decode::encoder_syntax::dct_recon_params;
use splot_recon::{BitDepth, PlaneId};

use crate::tile_writer::Result;

/// A rate in units of `1 / 256` bit. Sums saturate, so [`Bits::IMPOSSIBLE`] stays impossible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Bits(pub(crate) u64);

impl Bits {
    /// Fractional bits of the fixed-point unit.
    pub(crate) const FRAC_BITS: u32 = 8;
    /// The rate of a symbol the row cannot code.
    pub(crate) const IMPOSSIBLE: Self = Self(u64::MAX);

    /// `n` whole bits.
    pub(crate) const fn whole(n: u32) -> Self {
        Self((n as u64) << Self::FRAC_BITS)
    }
}

impl Add for Bits {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

impl AddAssign for Bits {
    fn add_assign(&mut self, other: Self) {
        *self = *self + other;
    }
}

impl core::iter::Sum for Bits {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

/// The § 8.2.6 interval scale: a symbol of interval `d` costs `log2(4096 / d)` bits.
const INTERVAL_LOG2: u32 = 12;

/// `floor(1024 * log2(x))` for `x >= 1`, by repeated squaring of the mantissa.
const fn log2_q10(x: u32) -> u32 {
    let int = x.ilog2();
    let mut mantissa = (x as u64) << (30 - int);
    let mut frac = 0;
    let mut bit = 0;
    while bit < 10 {
        mantissa = (mantissa * mantissa) >> 30;
        frac <<= 1;
        if mantissa >= 1 << 31 {
            mantissa >>= 1;
            frac |= 1;
        }
        bit += 1;
    }
    (int << 10) | frac
}

/// `256 * log2(x)` in Q8, for `x >= 1` (0 for `x = 0`).
pub(crate) fn log2_q8(x: u32) -> u32 {
    LOG2_Q8
        .get(x as usize)
        .map_or_else(|| (log2_q10(x) + 2) >> 2, |&value| u32::from(value))
}

/// `LOG2_Q8[x]`: [`log2_q8`] of the small arguments motion and filter rates use.
static LOG2_Q8: [u16; 4096] = {
    let mut table = [0u16; 4096];
    let mut x = 1;
    while x < table.len() {
        table[x] = ((log2_q10(x as u32) + 2) >> 2) as u16;
        x += 1;
    }
    table
};

/// `COST_Q8[d]`: `round(256 * log2(4096 / d))`, the cost of an interval of `d / 4096`.
static COST_Q8: [u16; 4097] = {
    let mut table = [0u16; 4097];
    let mut d = 1;
    while d < table.len() {
        table[d] = (((INTERVAL_LOG2 << 10) - log2_q10(d as u32) + 2) >> 2) as u16;
        d += 1;
    }
    table
};

/// The cost of coding `symbol` with the CDF `row` (layout of
/// `SymbolEncoder::write_symbol_u16`) before the row adapts.
///
/// The symbol takes `pp(s - 1) - pp(s)` of the § 8.2.6 4096 scale
/// ([`symbol_split_q12`]). A malformed row or an out-of-range symbol costs
/// [`Bits::IMPOSSIBLE`].
pub(crate) fn symbol_cost(row: &[u16], symbol: usize) -> Bits {
    let Some(low) = symbol_split_q12(row, symbol) else {
        return Bits::IMPOSSIBLE;
    };
    let high = match symbol.checked_sub(1) {
        Some(previous) => symbol_split_q12(row, previous).unwrap_or(0),
        None => 1 << INTERVAL_LOG2,
    };
    interval_cost(high, low)
}

/// [`symbol_cost`] of every symbol of `row` in symbol order, in one pass.
pub(crate) fn row_costs(row: &[u16]) -> impl Iterator<Item = Bits> + '_ {
    (0..)
        .map_while(|s| symbol_split_q12(row, s))
        .scan(1u32 << INTERVAL_LOG2, |high, low| {
            let cost = interval_cost(*high, low);
            *high = low;
            Some(cost)
        })
}

/// The cost of the interval `high - low` of a 4096 scale.
fn interval_cost(high: u32, low: u32) -> Bits {
    let width = high.saturating_sub(low);
    match COST_Q8.get(width as usize) {
        Some(&cost) if width > 0 => Bits(u64::from(cost)),
        _ => Bits::IMPOSSIBLE,
    }
}

/// The cost of `SymbolEncoder::write_unary(value, max_bits)`.
pub(crate) const fn unary_cost(value: u32, max_bits: u32) -> Bits {
    Bits::whole(if value < max_bits {
        value + 1
    } else {
        max_bits
    })
}

/// The Lagrange multiplier of `J = D + lambda * R`, with `D` a sum of squared
/// sample errors and `R` in bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Lambda {
    /// `lambda / step^2` in Q16; presets may tune it.
    pub(crate) scale_q16: u32,
    /// The squared luma AC step in sample units, in Q12 (`ac_quant^2 / 4096`).
    step_sq_q12: u64,
}

impl Lambda {
    /// `2 * ln(2) / 12 = 0.1155` in Q16: the high-rate slope `dD/dR` of a uniform
    /// quantizer with step `q`, whose distortion `q^2 / 12` halves per half bit.
    pub(crate) const DEFAULT_SCALE_Q16: u32 = 7_570;

    /// The multiplier of `qindex` at `bit_depth`. The luma AC quantizer of the
    /// decoder's dequantization is 64 times the step in sample units, at every
    /// bit depth, so `lambda = scale * (ac_quant / 64)^2`.
    ///
    /// # Errors
    /// Returns an error when the decoder rejects `qindex`.
    pub(crate) fn from_qindex(qindex: u32, bit_depth: BitDepth) -> Result<Self> {
        let (params, _) = dct_recon_params(qindex, PlaneId::Y, 3, 3, bit_depth)?;
        let ac = u64::from(params.ac_quant);
        Ok(Self {
            scale_q16: Self::DEFAULT_SCALE_Q16,
            step_sq_q12: ac * ac,
        })
    }

    /// The multiplier times `2^(log2_q8 / 256)`, with the fraction from
    /// `1 + 0.6565 f + 0.3435 f^2`, exact at both ends of `[0, 1)`.
    #[must_use]
    pub(crate) fn scaled(self, log2_q8: i32) -> Self {
        let frac = u64::from(log2_q8.rem_euclid(256).unsigned_abs());
        let mantissa_q16 = 65_536 + ((frac * 43_025) >> 8) + ((frac * frac * 22_511) >> 16);
        let scaled = (u128::from(self.step_sq_q12) * u128::from(mantissa_q16)) >> 16;
        let octaves = log2_q8.div_euclid(256);
        let scaled = if octaves >= 0 {
            scaled << octaves.unsigned_abs().min(32)
        } else {
            scaled >> octaves.unsigned_abs().min(127)
        };
        Self {
            step_sq_q12: u64::try_from(scaled).unwrap_or(u64::MAX).max(1),
            ..self
        }
    }

    /// The RDOQ multiplier of a transform with `dq_denom`: `lambda` per 1/256
    /// bit in the squared error of its coefficients, Q16. Coefficient energy is
    /// `64 / dq_denom^2` times sample energy.
    #[must_use]
    pub(crate) const fn coefficient_q8(self, dq_denom: u32) -> u64 {
        self.sse_per_bit_q16() / (4 * dq_denom as u64 * dq_denom as u64)
    }

    /// `lambda` in squared-error units per bit, Q16.
    #[must_use]
    pub(crate) const fn sse_per_bit_q16(self) -> u64 {
        (self.scale_q16 as u64 * self.step_sq_q12) >> 12
    }
}

/// Distortion and rate of a coding choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RdCost {
    /// Sum of squared sample errors.
    pub(crate) distortion: u64,
    /// Coded rate.
    pub(crate) rate: Bits,
}

impl RdCost {
    /// `J = D + lambda * R`, scaled by `2^24` to stay an exact integer.
    #[must_use]
    pub(crate) fn cost(self, lambda: Lambda) -> u128 {
        (u128::from(self.distortion) << (16 + Bits::FRAC_BITS))
            + u128::from(lambda.sse_per_bit_q16()) * u128::from(self.rate.0)
    }
}

impl Add for RdCost {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            distortion: self.distortion.saturating_add(other.distortion),
            rate: self.rate + other.rate,
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
