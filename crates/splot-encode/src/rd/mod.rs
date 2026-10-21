// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rate-distortion framework: symbol rates from CDF rows, the Lagrange
//! multiplier, search-state checkpoints, and the recursive partition search.
//!
//! Every quantity is an integer, so a decision never depends on float rounding.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "wired into intra_frame by the milestone-2 integration"
    )
)]

pub(crate) mod checkpoint;
pub(crate) mod coeff_rate;
pub(crate) mod partition;

use core::ops::{Add, AddAssign};

use splot_core::tables::conversion::PROB_INC;
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
/// It follows the § 8.2.6 interval split: `pp(s) = ((32768 - row[s]) >> 7 << 4) + PROB_INC`,
/// and the symbol takes `pp(s - 1) - pp(s)` of a 4096 scale. A malformed row or an
/// out-of-range symbol costs [`Bits::IMPOSSIBLE`].
pub(crate) fn symbol_cost(row: &[u16], symbol: usize) -> Bits {
    let n = row.len().wrapping_sub(1);
    let Some(inc) = PROB_INC.get(n.wrapping_sub(2)) else {
        return Bits::IMPOSSIBLE;
    };
    if symbol >= n {
        return Bits::IMPOSSIBLE;
    }
    let pp = |s: usize| -> u32 {
        if s == n - 1 {
            return 0;
        }
        let f = 32768u32.saturating_sub(u32::from(row[s]));
        ((f >> 7) << 4) + inc[s].unsigned_abs()
    };
    let high = if symbol == 0 {
        1 << INTERVAL_LOG2
    } else {
        pp(symbol - 1)
    };
    let width = high.saturating_sub(pp(symbol));
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
