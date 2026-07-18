// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient quantized-state writes.
//!
//! Feature tracking: `DECODE-COEFF-QUANT-STATE-WRITE`.

use std::collections::TryReserveError;
use std::num::TryFromIntError;

use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};
use super::sign_symbol::{CoeffSignRead, CoeffSignReadSymbol};

const TCQ_STATES: usize = 8;
const TCQ_PARITIES: usize = 2;
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
    pub(crate) entry: CoeffScanEntry,
    pub(crate) quant: u32,
    pub(crate) hr_level_avg: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantStateWrite {
    entry: CoeffScanEntry,
    level: u32,
    sign: bool,
    read_quant: u32,
    quant: i32,
    cul_level: u8,
    dc_category: u8,
    tcq_state: usize,
    hr_level_avg: u32,
}

impl CoeffQuantStateWrite {
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }

    #[must_use]
    pub(crate) const fn sign(self) -> bool {
        self.sign
    }

    #[must_use]
    pub(crate) const fn read_quant(self) -> u32 {
        self.read_quant
    }

    #[must_use]
    pub(crate) const fn quant(self) -> i32 {
        self.quant
    }

    #[must_use]
    pub(crate) const fn cul_level(self) -> u8 {
        self.cul_level
    }

    #[must_use]
    pub(crate) const fn dc_category(self) -> u8 {
        self.dc_category
    }

    #[must_use]
    pub(crate) const fn tcq_state(self) -> usize {
        self.tcq_state
    }

    #[must_use]
    pub(crate) const fn hr_level_avg(self) -> u32 {
        self.hr_level_avg
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffQuantState {
    writes: Vec<CoeffQuantStateWrite>,
    cul_level: u8,
    dc_category: u8,
    tcq_state: usize,
    hr_level_avg: u32,
}

impl NonZeroCoeffQuantState {
    pub(crate) fn from_interleaved_parts(
        writes: Vec<CoeffQuantStateWrite>,
        state: CoeffQuantStateAccumulator,
    ) -> Self {
        Self {
            writes,
            cul_level: state.cul_level,
            dc_category: state.dc_category,
            tcq_state: state.tcq_state,
            hr_level_avg: state.hr_level_avg,
        }
    }

    #[must_use]
    pub(crate) fn writes(&self) -> &[CoeffQuantStateWrite] {
        &self.writes
    }

    #[must_use]
    pub(crate) const fn cul_level(&self) -> u8 {
        self.cul_level
    }

    #[must_use]
    pub(crate) const fn dc_category(&self) -> u8 {
        self.dc_category
    }

    #[must_use]
    pub(crate) const fn tcq_state(&self) -> usize {
        self.tcq_state
    }

    #[must_use]
    pub(crate) const fn hr_level_avg(&self) -> u32 {
        self.hr_level_avg
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffQuantStateWriteError {
    #[error("coefficient quant sign count {signs} does not match scan entries {entries}")]
    SignCountMismatch { signs: usize, entries: usize },
    #[error("coefficient quant input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch { inputs: usize, entries: usize },
    #[error(
        "coefficient quant sign {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    SignEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error(
        "coefficient quant input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    InputEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error(
        "coefficient quant sign {index} carried level {actual}, expected local level {expected}"
    )]
    SignLevelMismatch {
        index: usize,
        expected: u32,
        actual: u32,
    },
    #[error("coefficient quant input {index} skipped required hidden-parity sign")]
    HiddenParityMissingSign { index: usize, entry: CoeffScanEntry },
    #[error("coefficient quant input {index} overflowed during {operation}")]
    QuantOverflow {
        index: usize,
        operation: &'static str,
    },
    #[error("coefficient quant state allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("coefficient quant state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient quant input {index} used invalid tcqState {tcq_state}")]
    InvalidTcqState { index: usize, tcq_state: usize },
}

pub(crate) fn apply_nonzero_coeff_quant_state(
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantReadInput],
    config: CoeffQuantStateConfig,
) -> Result<NonZeroCoeffQuantState, CoeffQuantStateWriteError> {
    if signs.len() != walk.len() {
        return Err(CoeffQuantStateWriteError::SignCountMismatch {
            signs: signs.len(),
            entries: walk.len(),
        });
    }
    if inputs.len() != walk.len() {
        return Err(CoeffQuantStateWriteError::InputCountMismatch {
            inputs: inputs.len(),
            entries: walk.len(),
        });
    }
    let levels = preflight_quant_writes(block, walk, signs, inputs, config)?;
    let mut state = CoeffQuantStateAccumulator::new(config);
    let mut writes = Vec::new();
    writes.try_reserve(walk.len())?;

    for (index, ((entry, sign), input)) in walk
        .entries()
        .zip(signs.iter().copied())
        .zip(inputs.iter().copied())
        .enumerate()
    {
        let write = state.apply_entry(index, entry, levels[index], sign.sign(), input)?;
        writes.push(write);
    }

    for write in &writes {
        block.set_quant(write.entry().pos(), write.quant())?;
    }

    Ok(NonZeroCoeffQuantState::from_interleaved_parts(
        writes, state,
    ))
}

fn preflight_quant_writes(
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk<'_>,
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantReadInput],
    config: CoeffQuantStateConfig,
) -> Result<Vec<u32>, CoeffQuantStateWriteError> {
    let mut levels = Vec::new();
    levels.try_reserve(walk.len())?;
    for (index, ((entry, sign), input)) in walk
        .entries()
        .zip(signs.iter().copied())
        .zip(inputs.iter().copied())
        .enumerate()
    {
        let level = checked_quant_write_level(
            block,
            index,
            entry,
            sign,
            input,
            config.is_hidden,
            config.sum_abs1,
        )?;
        levels.push(level);
    }
    Ok(levels)
}

fn checked_quant_write_level(
    block: &TransformCoeffBlockState,
    index: usize,
    entry: CoeffScanEntry,
    sign: CoeffSignRead,
    input: CoeffQuantReadInput,
    is_hidden: bool,
    sum_abs1: u32,
) -> Result<u32, CoeffQuantStateWriteError> {
    if sign.entry() != entry {
        return Err(CoeffQuantStateWriteError::SignEntryMismatch {
            index,
            expected: entry,
            actual: sign.entry(),
        });
    }
    if input.entry != entry {
        return Err(CoeffQuantStateWriteError::InputEntryMismatch {
            index,
            expected: entry,
            actual: input.entry,
        });
    }
    let level = block.level_at(entry.row(), entry.col())?;
    if sign.level() != level {
        return Err(CoeffQuantStateWriteError::SignLevelMismatch {
            index,
            expected: level,
            actual: sign.level(),
        });
    }
    if is_hidden
        && sum_abs1 > 0
        && entry.scan_index() == 0
        && sign.symbol() == CoeffSignReadSymbol::None
    {
        return Err(CoeffQuantStateWriteError::HiddenParityMissingSign { index, entry });
    }
    block.quant_at(entry.pos())?;
    Ok(level)
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
    hr_level_avg: u32,
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
            hr_level_avg: 0,
        }
    }

    pub(crate) fn apply_entry(
        &mut self,
        index: usize,
        entry: CoeffScanEntry,
        level: u32,
        sign: bool,
        input: CoeffQuantReadInput,
    ) -> Result<CoeffQuantStateWrite, CoeffQuantStateWriteError> {
        let mut quant = input.quant;
        self.hr_level_avg = input.hr_level_avg;

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

        let signed_quant = signed_quant(index, quant, sign)?;
        Ok(CoeffQuantStateWrite {
            entry,
            level,
            sign,
            read_quant: input.quant,
            quant: signed_quant,
            cul_level: self.cul_level,
            dc_category: self.dc_category,
            tcq_state: self.tcq_state,
            hr_level_avg: self.hr_level_avg,
        })
    }
}

pub(crate) fn apply_nonzero_coeff_quant_state_step(
    block: &mut TransformCoeffBlockState,
    state: &mut CoeffQuantStateAccumulator,
    index: usize,
    entry: CoeffScanEntry,
    sign: CoeffSignRead,
    input: CoeffQuantReadInput,
) -> Result<(), CoeffQuantStateWriteError> {
    let level = checked_quant_write_level(
        block,
        index,
        entry,
        sign,
        input,
        state.is_hidden,
        state.sum_abs1,
    )?;
    let write = state.apply_entry(index, entry, level, sign.sign(), input)?;
    block.set_quant(write.entry().pos(), write.quant())?;
    Ok(())
}

/// Interleaved derived-path variant of
/// [`apply_nonzero_coeff_quant_state_step`]: the caller read `level` from
/// `block` at `entry`, derived the sign input from that same entry and level,
/// and ran the hidden-parity gate, so the cross-input validation in
/// [`checked_quant_write_level`] cannot fail and is skipped.
pub(crate) fn apply_derived_nonzero_coeff_quant_state_step(
    block: &mut TransformCoeffBlockState,
    state: &mut CoeffQuantStateAccumulator,
    index: usize,
    entry: CoeffScanEntry,
    level: u32,
    sign: CoeffSignRead,
    input: CoeffQuantReadInput,
) -> Result<(), CoeffQuantStateWriteError> {
    let write = state.apply_entry(index, entry, level, sign.sign(), input)?;
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
