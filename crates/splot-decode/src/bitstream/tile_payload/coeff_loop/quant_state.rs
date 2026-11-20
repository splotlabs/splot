// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient quantized-state writes.
//!
//! Feature tracking: `DECODE-COEFF-QUANT-STATE-WRITE`.

use super::super::coeff_state::TileCoeffStateError;
use super::scan_walk::CoeffScanEntry;

const TCQ_STATES: usize = 8;
pub(crate) const MAX_CONFORMING_QUANT_MAGNITUDE: u32 = 1 << 20;
/// `Tcq_Next_State[ tcqState ][ parity & 1 ]` (§ 5.20.7.27) in bit form:
/// the state shifts right by one and bit 2 becomes `s0 ^ s2 ^ parity`.
#[inline]
pub(crate) const fn next_tcq_state(tcq_state: usize, parity: u32) -> usize {
    let state = tcq_state & (TCQ_STATES - 1);
    (state >> 1) | ((((state >> 2) ^ state ^ parity as usize) & 1) << 2)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantStateConfig {
    pub(crate) is_hidden: bool,
    pub(crate) sum_abs1: u32,
    pub(crate) use_tcq: bool,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantReadInput {
    pub(crate) quant: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantStateWrite {
    entry: CoeffScanEntry,
    quant: i32,
}

impl CoeffQuantStateWrite {
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    #[must_use]
    pub(crate) const fn quant(self) -> i32 {
        self.quant
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffQuantState {
    cul_level: u8,
    dc_category: u8,
}

impl NonZeroCoeffQuantState {
    pub(crate) const fn from_accumulator(state: CoeffQuantStateAccumulator) -> Self {
        Self {
            cul_level: state.cul_level,
            dc_category: state.dc_category,
        }
    }

    #[must_use]
    pub(crate) const fn cul_level(&self) -> u8 {
        self.cul_level
    }

    #[must_use]
    pub(crate) const fn dc_category(&self) -> u8 {
        self.dc_category
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffQuantStateWriteError {
    #[error("coefficient quant input {index} overflowed during {operation}")]
    QuantOverflow {
        index: usize,
        operation: &'static str,
    },
    #[error("coefficient quant state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error(
        "coefficient quant input {index} has nonconforming magnitude {magnitude}; expected less than 1 << 20"
    )]
    QuantMagnitudeOutOfRange { index: usize, magnitude: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantStateAccumulator {
    is_hidden: bool,
    sum_abs1: u32,
    use_tcq: bool,
    lossless: bool,
    cul_level: u8,
    dc_category: u8,
    tcq_state: usize,
}

impl CoeffQuantStateAccumulator {
    pub(crate) const fn new(config: CoeffQuantStateConfig) -> Self {
        Self {
            is_hidden: config.is_hidden,
            sum_abs1: config.sum_abs1,
            use_tcq: config.use_tcq,
            lossless: config.lossless,
            cul_level: 0,
            dc_category: 0,
            tcq_state: 0,
        }
    }

    /// Sets a TCQ state whose `(tcqState >> 1) & 1` is `q0`, for a pass
    /// that skips zero levels and takes each entry's state from the base
    /// pass. [`Self::apply_entry`] reads only that bit before the next call
    /// sets it again.
    /// The accumulator with `culLevel` taken from the sum of the levels so
    /// far: `Min(4, culLevel + quant)` per level is `Min(4, sum)`.
    #[must_use]
    pub(crate) fn with_cul_level(self, level_sum: u32) -> Self {
        Self {
            cul_level: level_sum.min(4) as u8,
            ..self
        }
    }

    /// The `dcCategory` of a nonzero DC coefficient with `sign`.
    #[inline]
    pub(crate) fn set_dc_sign(&mut self, sign: bool) {
        self.dc_category = if sign { 1 } else { 2 };
    }

    #[inline]
    pub(crate) fn set_tcq_q0(&mut self, q0: bool) {
        self.tcq_state = usize::from(q0) << 1;
    }

    /// [`Self::apply_entry`] for an entry above scan index 0, which is
    /// neither the DC nor parity-hidden; returns its magnitude.
    /// [`Self::ac_magnitude`] of a level below `maxLevel - allowTcq`, which
    /// read no remainder and so cannot leave the conforming range.
    #[allow(clippy::inline_always, reason = "measured: per-coefficient quant step")]
    #[inline(always)]
    pub(crate) fn ac_magnitude(
        &mut self,
        index: usize,
        quant: u32,
        tcq_q0: bool,
    ) -> Result<u32, CoeffQuantStateWriteError> {
        self.cul_level = 4.min(u32::from(self.cul_level).saturating_add(quant)) as u8;
        let mut quant = quant;
        if !self.lossless && self.use_tcq && quant > 0 {
            quant = checked_mul_sub(index, quant, 2, u32::from(tcq_q0), "quant * 2 - q0")?;
        }
        if quant >= MAX_CONFORMING_QUANT_MAGNITUDE {
            return Err(CoeffQuantStateWriteError::QuantMagnitudeOutOfRange {
                index,
                magnitude: quant,
            });
        }
        Ok(quant)
    }

    #[inline]
    pub(crate) fn apply_entry(
        &mut self,
        index: usize,
        entry: CoeffScanEntry,
        sign: bool,
        input: CoeffQuantReadInput,
    ) -> Result<CoeffQuantStateWrite, CoeffQuantStateWriteError> {
        let mut quant = input.quant;

        if self.is_hidden && entry.scan_index() == 0 {
            quant = checked_mul_add(index, quant, 2, self.sum_abs1, "2 * quant + sumAbs1")?;
        }
        if entry.pos() == 0 && quant > 0 {
            self.dc_category = if sign { 1 } else { 2 };
        }
        self.cul_level = 4.min(u32::from(self.cul_level).saturating_add(quant)) as u8;

        if !self.lossless && self.use_tcq {
            let q0 = ((self.tcq_state >> 1) & 1) as u32;
            self.tcq_state = next_tcq_state(self.tcq_state, quant);
            if quant > 0 {
                quant = checked_mul_sub(index, quant, 2, q0, "quant * 2 - q0")?;
            }
        }

        if quant >= MAX_CONFORMING_QUANT_MAGNITUDE {
            return Err(CoeffQuantStateWriteError::QuantMagnitudeOutOfRange {
                index,
                magnitude: quant,
            });
        }

        let magnitude = quant as i32;
        Ok(CoeffQuantStateWrite {
            entry,
            quant: if sign { -magnitude } else { magnitude },
        })
    }
}

fn checked_mul_add(
    index: usize,
    value: u32,
    mul: u32,
    add: u32,
    operation: &'static str,
) -> Result<u32, CoeffQuantStateWriteError> {
    value
        .checked_mul(mul)
        .and_then(|value| value.checked_add(add))
        .ok_or(CoeffQuantStateWriteError::QuantOverflow { index, operation })
}

fn checked_mul_sub(
    index: usize,
    value: u32,
    mul: u32,
    sub: u32,
    operation: &'static str,
) -> Result<u32, CoeffQuantStateWriteError> {
    value
        .checked_mul(mul)
        .and_then(|value| value.checked_sub(sub))
        .ok_or(CoeffQuantStateWriteError::QuantOverflow { index, operation })
}

#[cfg(test)]
#[path = "quant_state_tests.rs"]
mod tests;
