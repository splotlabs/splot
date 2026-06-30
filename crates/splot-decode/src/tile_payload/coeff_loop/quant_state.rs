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

/// Returns the next ordinary coefficient `tcqState` for one decoded parity.
///
/// The state table is AV2 §5.20.7.27 local decode state. Invalid caller state
/// returns `None` instead of indexing the table.
pub(crate) fn next_tcq_state(tcq_state: usize, parity: u32) -> Option<usize> {
    TCQ_NEXT_STATE
        .get(tcq_state)
        .map(|row| row[(parity & 1) as usize])
}

/// Caller-resolved block-level facts for applying ordinary non-FSC quant state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantStateConfig {
    /// Whether parity hiding is active for this transform block.
    pub(crate) is_hidden: bool,
    /// Caller-maintained `sumAbs1` parity accumulator.
    pub(crate) sum_abs1: u32,
    /// Whether TCQ is active for this transform block.
    pub(crate) use_tcq: bool,
    /// Whether the block is lossless.
    pub(crate) lossless: bool,
}

/// Caller-provided output of §5.20.7.28 `read_quant` for one scan entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantReadInput {
    /// Checked scan entry this quant result belongs to.
    pub(crate) entry: CoeffScanEntry,
    /// Unsigned `quant` returned by `read_quant` before hidden, TCQ, and sign effects.
    pub(crate) quant: u32,
    /// Updated `hrLevelAvg` returned by `read_quant`.
    pub(crate) hr_level_avg: u32,
}

/// Applied quantized-coefficient state for one ordinary non-FSC coefficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffQuantStateWrite {
    entry: CoeffScanEntry,
    level: u32,
    sign: bool,
    read_quant: u32,
    quant: i32,
    cul_level: u32,
    dc_category: u8,
    tcq_state: usize,
    hr_level_avg: u32,
}

impl CoeffQuantStateWrite {
    /// Checked scan entry associated with this write.
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    /// Local `Level[row][col]` value used by this write.
    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }

    /// Boolean sign applied to this quantized coefficient.
    #[must_use]
    pub(crate) const fn sign(self) -> bool {
        self.sign
    }

    /// Raw caller-provided `read_quant` result before later state effects.
    #[must_use]
    pub(crate) const fn read_quant(self) -> u32 {
        self.read_quant
    }

    /// Signed value written to `Quant[pos]`.
    #[must_use]
    pub(crate) const fn quant(self) -> i32 {
        self.quant
    }

    /// `culLevel` after this write.
    #[must_use]
    pub(crate) const fn cul_level(self) -> u32 {
        self.cul_level
    }

    /// `dcCategory` after this write.
    #[must_use]
    pub(crate) const fn dc_category(self) -> u8 {
        self.dc_category
    }

    /// `tcqState` after this write.
    #[must_use]
    pub(crate) const fn tcq_state(self) -> usize {
        self.tcq_state
    }

    /// `hrLevelAvg` returned by this entry's caller-provided `read_quant`.
    #[must_use]
    pub(crate) const fn hr_level_avg(self) -> u32 {
        self.hr_level_avg
    }
}

/// Final quant-state facts after applying one ordinary non-FSC block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffQuantState {
    writes: Vec<CoeffQuantStateWrite>,
    cul_level: u32,
    dc_category: u8,
    tcq_state: usize,
    hr_level_avg: u32,
}

impl NonZeroCoeffQuantState {
    /// Builds the final summary from interleaved per-entry writes.
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

    /// Per-entry quant-state writes in scan-walk order.
    #[must_use]
    pub(crate) fn writes(&self) -> &[CoeffQuantStateWrite] {
        &self.writes
    }

    /// Final clamped `culLevel`.
    #[must_use]
    pub(crate) const fn cul_level(&self) -> u32 {
        self.cul_level
    }

    /// Final `dcCategory`.
    #[must_use]
    pub(crate) const fn dc_category(&self) -> u8 {
        self.dc_category
    }

    /// Final `tcqState`.
    #[must_use]
    pub(crate) const fn tcq_state(&self) -> usize {
        self.tcq_state
    }

    /// Final caller-provided `hrLevelAvg`.
    #[must_use]
    pub(crate) const fn hr_level_avg(&self) -> u32 {
        self.hr_level_avg
    }
}

/// Error returned by the coefficient quant-state boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffQuantStateWriteError {
    /// The number of sign records did not match the checked scan walk.
    #[error("coefficient quant sign count {signs} does not match scan entries {entries}")]
    SignCountMismatch {
        /// Decoded sign record count.
        signs: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// The number of quant records did not match the checked scan walk.
    #[error("coefficient quant input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch {
        /// Caller-provided quant input count.
        inputs: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// One sign record was not paired with the matching checked scan entry.
    #[error(
        "coefficient quant sign {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    SignEntryMismatch {
        /// Input index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Actual sign-read entry.
        actual: CoeffScanEntry,
    },
    /// One quant input was not paired with the matching checked scan entry.
    #[error(
        "coefficient quant input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    InputEntryMismatch {
        /// Input index.
        index: usize,
        /// Expected checked scan entry.
        expected: CoeffScanEntry,
        /// Caller-provided quant entry.
        actual: CoeffScanEntry,
    },
    /// A sign record was decoded against a different local level.
    #[error(
        "coefficient quant sign {index} carried level {actual}, expected local level {expected}"
    )]
    SignLevelMismatch {
        /// Input index.
        index: usize,
        /// Level read from local state.
        expected: u32,
        /// Level carried by the sign record.
        actual: u32,
    },
    /// Hidden parity required sign syntax for the final scan entry.
    #[error("coefficient quant input {index} skipped required hidden-parity sign")]
    HiddenParityMissingSign {
        /// Input index.
        index: usize,
        /// Checked scan entry.
        entry: CoeffScanEntry,
    },
    /// A quantized coefficient arithmetic operation overflowed the local type.
    #[error("coefficient quant input {index} overflowed during {operation}")]
    QuantOverflow {
        /// Input index.
        index: usize,
        /// Operation name.
        operation: &'static str,
    },
    /// Allocation for decoded coefficient quant records failed.
    #[error("coefficient quant state allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// The local transform-block state rejected a checked coordinate or position.
    #[error("coefficient quant state error: {0}")]
    State(#[from] TileCoeffStateError),
    /// The local TCQ state was outside the AV2 state table.
    #[error("coefficient quant input {index} used invalid tcqState {tcq_state}")]
    InvalidTcqState {
        /// Input index.
        index: usize,
        /// Invalid `tcqState`.
        tcq_state: usize,
    },
}

/// Applies ordinary non-FSC §5.20.7.27 quantized coefficient state effects.
///
/// The caller owns §5.20.7.28 `read_quant` parsing and all block facts such as
/// parity hiding, `sumAbs1`, TCQ enablement, and lossless mode. This helper
/// validates the checked scan walk, sign summaries, local `Level[]`, and
/// `Quant[pos]` positions, computes all writes, then mutates `Quant[pos]`. The
/// ordinary non-FSC branch resets `tcqState` to `0` immediately before this pass.
/// The helper returns the derived `culLevel`, `dcCategory`, `tcqState`, and
/// `hrLevelAvg` facts. It does not mutate `QuantSign[]`, update tile context
/// lines, run dequantization, or invoke reconstruction.
pub(crate) fn apply_nonzero_coeff_quant_state(
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantReadInput],
    config: CoeffQuantStateConfig,
) -> Result<NonZeroCoeffQuantState, CoeffQuantStateWriteError> {
    let entries = walk.entries();
    if signs.len() != entries.len() {
        return Err(CoeffQuantStateWriteError::SignCountMismatch {
            signs: signs.len(),
            entries: entries.len(),
        });
    }
    if inputs.len() != entries.len() {
        return Err(CoeffQuantStateWriteError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }
    let levels = preflight_quant_writes(block, entries, signs, inputs, config)?;
    let mut state = CoeffQuantStateAccumulator::new(config);
    let mut writes = Vec::new();
    writes.try_reserve(entries.len())?;

    for (index, ((entry, sign), input)) in entries
        .iter()
        .copied()
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
    entries: &[CoeffScanEntry],
    signs: &[CoeffSignRead],
    inputs: &[CoeffQuantReadInput],
    config: CoeffQuantStateConfig,
) -> Result<Vec<u32>, CoeffQuantStateWriteError> {
    let mut levels = Vec::new();
    levels.try_reserve(entries.len())?;
    for (index, ((entry, sign), input)) in entries
        .iter()
        .copied()
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
    cul_level: u32,
    dc_category: u8,
    tcq_state: usize,
    hr_level_avg: u32,
}

impl CoeffQuantStateAccumulator {
    /// Creates a coefficient quant-state accumulator from caller-resolved facts.
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

    /// Applies one checked coefficient to the accumulator without mutating block storage.
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
        self.cul_level = 4.min(self.cul_level.saturating_add(quant));

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

/// Applies one pre-read ordinary non-FSC quant-state step and writes `Quant[pos]`.
pub(crate) fn apply_nonzero_coeff_quant_state_step(
    block: &mut TransformCoeffBlockState,
    state: &mut CoeffQuantStateAccumulator,
    index: usize,
    entry: CoeffScanEntry,
    sign: CoeffSignRead,
    input: CoeffQuantReadInput,
) -> Result<CoeffQuantStateWrite, CoeffQuantStateWriteError> {
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
    Ok(write)
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use splot_core::span::ByteOffset;
    use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};

    use super::super::super::cdf::FrameCdfSubset;
    use super::super::super::coeff_state::{TileCoeffContextState, TransformCoeffBlockState};
    use super::super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStartInput};
    use super::super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
    use super::super::sign_symbol::{
        CoeffSignRead, CoeffSignReadInput, CoeffSignReadSource, read_nonzero_coeff_signs,
    };
    use super::super::*;
    use super::*;

    const EOB_SCAN: [u16; 4] = [0, 8, 1, 9];
    const ALT_SCAN: [u16; 4] = [0, 8, 9, 1];
    const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
        [0x00, 0x00, 0x80],
        [0xff, 0x00, 0x80],
        [0x55, 0xaa, 0x80],
        [0xff, 0xff, 0x80],
    ];

    fn symbol_decoder(payload: &[u8], mode: CdfUpdateMode) -> SymbolDecoder<'_> {
        SymbolDecoder::with_base_and_config(
            payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(mode),
        )
        .unwrap()
    }

    fn branch_nonzero(
        branch: CoeffBlockEobBranch,
    ) -> Option<super::super::branch::NonZeroCoeffBlockStart> {
        match branch {
            CoeffBlockEobBranch::AllZero(_) => None,
            CoeffBlockEobBranch::NonZero(start) => Some(start),
        }
    }

    fn setup_walk(payload: &[u8], scan: &[u16]) -> Option<NonZeroCoeffScanWalk> {
        let frame = FrameCdfSubset::from_defaults();
        let mut tile = frame.tile_copy();
        let mut symbols = symbol_decoder(payload, CdfUpdateMode::Enabled);
        let mut state = TileCoeffContextState::new(4, 4).ok()?;
        let branch = read_coeff_block_eob_branch(
            &mut state,
            &mut tile,
            &mut symbols,
            CoeffBlockEobBranchInput::NonZero(NonZeroCoeffBlockStartInput {
                block: AllZeroCoeffBlockInput {
                    plane: 0,
                    x4: 0,
                    y4: 0,
                    w4: 2,
                    h4: 2,
                },
                eob: NonZeroCoeffEobContextInput {
                    plane: 0,
                    is_inter: false,
                    tx_width_log2: 3,
                    tx_height_log2: 3,
                    coeff_cdf_q_ctx: 0,
                },
            }),
        )
        .ok()?;
        let start = branch_nonzero(branch)?;
        if start.eob_read().eob().eob() != scan.len() {
            return None;
        }
        walk_nonzero_coeff_scan(&start, scan).ok()
    }

    fn find_eob_payload() -> [u8; 5] {
        for first in u8::MIN..=u8::MAX {
            for second in u8::MIN..=u8::MAX {
                for suffix in PAYLOAD_SUFFIXES {
                    let payload = [first, second, suffix[0], suffix[1], suffix[2]];
                    if setup_walk(&payload, &EOB_SCAN).is_some() {
                        return payload;
                    }
                }
            }
        }
        panic!("no coefficient quant EOB payload found");
    }

    fn block_for(walk: &NonZeroCoeffScanWalk) -> TransformCoeffBlockState {
        let mut block = TransformCoeffBlockState::new(8, 8).unwrap();
        for (index, entry) in walk.entries().iter().copied().enumerate() {
            let level = match index {
                0 => 3,
                1 => 2,
                2 => 0,
                _ => 1,
            };
            block.set_level(entry.row(), entry.col(), level).unwrap();
            block
                .set_quant_sign(
                    entry.row(),
                    entry.col(),
                    if index % 2 == 0 { 7 } else { -7 },
                )
                .unwrap();
        }
        block
    }

    fn signs_for(
        block: &TransformCoeffBlockState,
        walk: &NonZeroCoeffScanWalk,
    ) -> Vec<CoeffSignRead> {
        let inputs: Vec<_> = walk
            .entries()
            .iter()
            .copied()
            .map(|entry| CoeffSignReadInput {
                entry,
                source: if block.level_at(entry.row(), entry.col()).unwrap() == 0 {
                    CoeffSignReadSource::None
                } else {
                    CoeffSignReadSource::SignBit
                },
            })
            .collect();
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&[0xff, 0xff, 0x80], CdfUpdateMode::Enabled);
        read_nonzero_coeff_signs(&mut tile, &mut symbols, block, walk, &inputs).unwrap()
    }

    fn quant_inputs_for(walk: &NonZeroCoeffScanWalk, quants: &[u32]) -> Vec<CoeffQuantReadInput> {
        walk.entries()
            .iter()
            .copied()
            .zip(quants.iter().copied())
            .enumerate()
            .map(|(index, (entry, quant))| CoeffQuantReadInput {
                entry,
                quant,
                hr_level_avg: (index as u32 + 1) * 10,
            })
            .collect()
    }

    fn config() -> CoeffQuantStateConfig {
        CoeffQuantStateConfig {
            is_hidden: false,
            sum_abs1: 0,
            use_tcq: false,
            lossless: false,
        }
    }

    #[test]
    fn coefficient_quant_state_writes_signed_quant_and_summary_state() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let mut block = block_for(&walk);
        let quant_sign_before = block.quant_sign().to_vec();
        let signs = signs_for(&block, &walk);
        let inputs = quant_inputs_for(&walk, &[2, 1, 0, 3]);

        let state =
            apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, config()).unwrap();

        assert_eq!(state.writes().len(), walk.entries().len());
        for ((write, sign), input) in state.writes().iter().zip(&signs).zip(&inputs) {
            let expected = if sign.sign() {
                -(input.quant as i32)
            } else {
                input.quant as i32
            };
            assert_eq!(write.entry(), input.entry);
            assert_eq!(write.level(), sign.level());
            assert_eq!(write.sign(), sign.sign());
            assert_eq!(write.read_quant(), input.quant);
            assert_eq!(write.quant(), expected);
            assert!(write.cul_level() <= 4);
            assert!(write.dc_category() <= 2);
            assert_eq!(write.tcq_state(), 0);
            assert_eq!(write.hr_level_avg(), input.hr_level_avg);
            assert_eq!(block.quant_at(input.entry.pos()).unwrap(), expected);
        }
        let dc_entry_index = walk
            .entries()
            .iter()
            .position(|entry| entry.pos() == 0)
            .unwrap();
        let expected_dc = if signs[dc_entry_index].sign() { 1 } else { 2 };
        assert_eq!(state.cul_level(), 4);
        assert_eq!(state.dc_category(), expected_dc);
        assert_eq!(state.hr_level_avg(), 40);
        assert_eq!(block.quant_sign(), quant_sign_before);
    }

    #[test]
    fn coefficient_quant_state_applies_hidden_parity_and_tcq() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let mut block = block_for(&walk);
        let signs = signs_for(&block, &walk);
        let inputs = quant_inputs_for(&walk, &[0, 0, 0, 1]);
        let hidden_tcq = CoeffQuantStateConfig {
            is_hidden: true,
            sum_abs1: 1,
            use_tcq: true,
            lossless: false,
        };

        let state = apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, hidden_tcq)
            .unwrap();

        let dc_write = state
            .writes()
            .iter()
            .find(|write| write.entry().scan_index() == 0)
            .unwrap();
        assert_eq!(dc_write.read_quant(), 1);
        assert_eq!(dc_write.quant().unsigned_abs(), 6);
        assert_eq!(
            block.quant_at(dc_write.entry().pos()).unwrap(),
            dc_write.quant()
        );
        assert_eq!(state.cul_level(), 3);
        assert_eq!(state.tcq_state(), 4);
        assert_eq!(state.hr_level_avg(), 40);
    }

    #[test]
    fn coefficient_quant_state_preserves_quant_sign() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let mut block = block_for(&walk);
        let quant_sign_before = block.quant_sign().to_vec();
        let signs = signs_for(&block, &walk);
        let inputs = quant_inputs_for(&walk, &[5, 4, 0, 2]);

        apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, config()).unwrap();

        assert_eq!(block.quant_sign(), quant_sign_before);
    }

    #[test]
    fn coefficient_quant_state_rejects_mismatches_before_mutation() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let alt_walk = setup_walk(&payload, &ALT_SCAN).unwrap();
        let block = block_for(&walk);
        let signs = signs_for(&block, &walk);
        let inputs = quant_inputs_for(&walk, &[2, 1, 0, 3]);

        let mut count_block = block.clone();
        let count_before = count_block.clone();
        let mut short_inputs = inputs.clone();
        short_inputs.pop();
        let err = apply_nonzero_coeff_quant_state(
            &mut count_block,
            &walk,
            &signs,
            &short_inputs,
            config(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CoeffQuantStateWriteError::InputCountMismatch {
                inputs: 3,
                entries: 4
            }
        ));
        assert_eq!(count_block, count_before);

        let mut sign_block = block.clone();
        let sign_before = sign_block.clone();
        let err =
            apply_nonzero_coeff_quant_state(&mut sign_block, &alt_walk, &signs, &inputs, config())
                .unwrap_err();
        assert!(matches!(
            err,
            CoeffQuantStateWriteError::SignEntryMismatch { index: 0, .. }
        ));
        assert_eq!(sign_block, sign_before);
    }

    #[test]
    fn coefficient_quant_state_requires_hidden_parity_sign() {
        let payload = find_eob_payload();
        let walk = setup_walk(&payload, &EOB_SCAN).unwrap();
        let mut block = block_for(&walk);
        let hidden_entry = walk
            .entries()
            .iter()
            .copied()
            .find(|entry| entry.scan_index() == 0)
            .unwrap();
        block
            .set_level(hidden_entry.row(), hidden_entry.col(), 0)
            .unwrap();
        let before = block.clone();
        let signs = signs_for(&block, &walk);
        let inputs = quant_inputs_for(&walk, &[0, 0, 0, 1]);
        let hidden = CoeffQuantStateConfig {
            is_hidden: true,
            sum_abs1: 1,
            use_tcq: false,
            lossless: false,
        };

        let err = apply_nonzero_coeff_quant_state(&mut block, &walk, &signs, &inputs, hidden)
            .unwrap_err();

        assert!(matches!(
            err,
            CoeffQuantStateWriteError::HiddenParityMissingSign { entry, .. }
                if entry == hidden_entry
        ));
        assert_eq!(block, before);
    }
}
