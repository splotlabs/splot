// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient quantized-state writes.
//!
//! Feature tracking: `DECODE-COEFF-QUANT-STATE-WRITE`.

use std::num::TryFromIntError;

use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::scan_walk::CoeffScanEntry;
use super::sign_symbol::CoeffSignRead;

const TCQ_STATES: usize = 8;
const TCQ_PARITIES: usize = 2;
const MAX_CONFORMING_QUANT_MAGNITUDE: u32 = 1 << 20;
const TCQ_NEXT_STATE: [[usize; TCQ_PARITIES]; TCQ_STATES] = [
    [0, 4],
    [4, 0],
    [1, 5],
    [5, 1],
    [6, 2],
    [2, 6],
    [7, 3],
    [3, 7],
];

pub(crate) fn next_tcq_state(tcq_state: usize, parity: u32) -> Option<usize> {
    TCQ_NEXT_STATE
        .get(tcq_state)
        .map(|row| row[(parity & 1) as usize])
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
    #[error("coefficient quant input {index} used invalid tcqState {tcq_state}")]
    InvalidTcqState { index: usize, tcq_state: usize },
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
            self.tcq_state = next_tcq_state(self.tcq_state, quant).ok_or(
                CoeffQuantStateWriteError::InvalidTcqState {
                    index,
                    tcq_state: self.tcq_state,
                },
            )?;
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

        let signed_quant = signed_quant(index, quant, sign)?;
        Ok(CoeffQuantStateWrite {
            entry,
            quant: signed_quant,
        })
    }
}

pub(crate) fn apply_derived_nonzero_coeff_quant_state_step(
    block: &mut TransformCoeffBlockState,
    state: &mut CoeffQuantStateAccumulator,
    index: usize,
    entry: CoeffScanEntry,
    sign: CoeffSignRead,
    input: CoeffQuantReadInput,
) -> Result<(), CoeffQuantStateWriteError> {
    let write = state.apply_entry(index, entry, sign.sign(), input)?;
    block.set_quant(write.entry().pos(), write.quant())?;
    Ok(())
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

fn signed_quant(index: usize, quant: u32, sign: bool) -> Result<i32, CoeffQuantStateWriteError> {
    let magnitude = i32::try_from(quant).map_err(|_: TryFromIntError| {
        CoeffQuantStateWriteError::QuantOverflow {
            index,
            operation: "u32 to i32 quant conversion",
        }
    })?;
    if sign {
        magnitude
            .checked_neg()
            .ok_or(CoeffQuantStateWriteError::QuantOverflow {
                index,
                operation: "signed quant negation",
            })
    } else {
        Ok(magnitude)
    }
}

#[cfg(test)]
#[path = "quant_state_tests.rs"]
mod tests;
