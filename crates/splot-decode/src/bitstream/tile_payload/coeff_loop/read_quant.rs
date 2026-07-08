// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient `read_quant` syntax.
//!
//! Feature tracking: `DECODE-COEFF-READ-QUANT-SYNTAX`.

use std::collections::TryReserveError;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::quant_state::CoeffQuantReadInput;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

const MIN_M: u32 = 1;
const MAX_M: u32 = 6;
const MAX_COEFF_REM_BITS: u32 = 32;
const MAX_EXP_GOLOMB_PREFIX_BITS: u32 = 21;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuantConfig {
    pub(crate) is_hidden: bool,
    pub(crate) allow_tcq: bool,
    pub(crate) hr_level_avg: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuantInput {
    pub(crate) entry: CoeffScanEntry,
    pub(crate) level: u32,
    pub(crate) max_level: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffReadQuantPath {
    BelowThreshold,
    Extended {
        m: u32,
        k: u32,
        c_max: u32,
        q: u32,
        length: u32,
        x_base: u32,
        coeff_rem: u32,
        x: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuant {
    quant: CoeffQuantReadInput,
    path: CoeffReadQuantPath,
}

impl CoeffReadQuant {
    #[must_use]
    pub(crate) const fn quant_input(self) -> CoeffQuantReadInput {
        self.quant
    }

    #[must_use]
    pub(crate) const fn path(self) -> CoeffReadQuantPath {
        self.path
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffReadQuantError {
    #[error("coefficient read_quant input count {inputs} does not match scan entries {entries}")]
    InputCountMismatch { inputs: usize, entries: usize },
    #[error(
        "coefficient read_quant input {index} targets {actual:?}, expected checked scan entry {expected:?}"
    )]
    ScanEntryMismatch {
        index: usize,
        expected: CoeffScanEntry,
        actual: CoeffScanEntry,
    },
    #[error("coefficient read_quant input {index} has invalid maxLevel {max_level}")]
    InvalidMaxLevel {
        index: usize,
        max_level: u32,
        allow_tcq: bool,
    },
    #[error("coefficient read_quant input {index} literal read failed for {syntax}: {source}")]
    LiteralRead {
        index: usize,
        syntax: &'static str,
        #[source]
        source: CoreError,
    },
    #[error("coefficient read_quant input {index} overflowed during {operation}")]
    QuantOverflow {
        index: usize,
        operation: &'static str,
    },
    #[error("coefficient read_quant allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
}

pub(crate) fn read_nonzero_coeff_quants(
    symbols: &mut SymbolDecoder<'_>,
    walk: &NonZeroCoeffScanWalk,
    inputs: &[CoeffReadQuantInput],
    config: CoeffReadQuantConfig,
) -> Result<Vec<CoeffReadQuant>, CoeffReadQuantError> {
    let entries = walk.entries();
    if inputs.len() != entries.len() {
        return Err(CoeffReadQuantError::InputCountMismatch {
            inputs: inputs.len(),
            entries: entries.len(),
        });
    }
    for (index, (entry, input)) in entries
        .iter()
        .copied()
        .zip(inputs.iter().copied())
        .enumerate()
    {
        if input.entry != entry {
            return Err(CoeffReadQuantError::ScanEntryMismatch {
                index,
                expected: entry,
                actual: input.entry,
            });
        }
        quant_threshold(index, input.max_level, config.allow_tcq)?;
    }

    let mut state = CoeffReadQuantState::new(config);
    let mut reads = Vec::new();
    reads.try_reserve(entries.len())?;
    for (index, input) in inputs.iter().copied().enumerate() {
        reads.push(state.read_one(symbols, index, input)?);
    }
    Ok(reads)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffReadQuantState {
    is_hidden: bool,
    allow_tcq: bool,
    hr_level_avg: u32,
}

impl CoeffReadQuantState {
    pub(crate) const fn new(config: CoeffReadQuantConfig) -> Self {
        Self {
            is_hidden: config.is_hidden,
            allow_tcq: config.allow_tcq,
            hr_level_avg: config.hr_level_avg,
        }
    }

    pub(crate) fn read_one(
        &mut self,
        symbols: &mut SymbolDecoder<'_>,
        index: usize,
        input: CoeffReadQuantInput,
    ) -> Result<CoeffReadQuant, CoeffReadQuantError> {
        let threshold = quant_threshold(index, input.max_level, self.allow_tcq)?;
        if input.level < threshold {
            return Ok(CoeffReadQuant {
                quant: CoeffQuantReadInput {
                    entry: input.entry,
                    quant: input.level,
                    hr_level_avg: self.hr_level_avg,
                },
                path: CoeffReadQuantPath::BelowThreshold,
            });
        }

        let lvl_shift = u32::from(input.entry.pos() == 0 && self.is_hidden);
        let pred_level = self.hr_level_avg >> lvl_shift;
        let m = get_msb(pred_level).clamp(MIN_M, MAX_M);
        let k = m + 1;
        let c_max = (m + 4).min(6);

        let q = read_bypass_symbol(symbols, index, c_max, "q_length", BypassSyntax::Unary)?;

        let (length, x_base) = if q == c_max {
            let prefix = read_bypass_symbol(
                symbols,
                index,
                MAX_EXP_GOLOMB_PREFIX_BITS,
                "golomb_length",
                BypassSyntax::Unary,
            )?;
            if prefix >= MAX_EXP_GOLOMB_PREFIX_BITS || prefix > MAX_COEFF_REM_BITS.saturating_sub(k)
            {
                return Err(quant_overflow(index, "coeff_rem literal width"));
            }
            let length = checked_add(index, prefix, k, "golomb length + k")?;
            let q_base = checked_shl_u64(index, u64::from(q), m, "q << m")?;
            let length_base = checked_shl_u64(index, 1, length, "1 << length")?;
            let k_base = checked_shl_u64(index, 1, k, "1 << k")?;
            (
                length,
                checked_u32(
                    index,
                    checked_add_u64(
                        index,
                        q_base,
                        length_base
                            .checked_sub(k_base)
                            .ok_or(quant_overflow(index, "1 << length - 1 << k"))?,
                        "extended xBase",
                    )?,
                    "u64 xBase to u32",
                )?,
            )
        } else {
            (
                m,
                checked_u32(
                    index,
                    checked_shl_u64(index, u64::from(q), m, "q << m")?,
                    "u64 xBase to u32",
                )?,
            )
        };

        if length > MAX_COEFF_REM_BITS {
            return Err(quant_overflow(index, "coeff_rem literal width"));
        }
        let coeff_rem =
            read_bypass_symbol(symbols, index, length, "coeff_rem", BypassSyntax::Literal)?;
        let x = checked_u32(
            index,
            checked_add_u64(
                index,
                u64::from(x_base),
                u64::from(coeff_rem),
                "xBase + coeff_rem",
            )?,
            "u64 x to u32",
        )?;

        let shifted_x = checked_shl_u64(index, u64::from(x), lvl_shift, "x << lvlShift")?;
        let next_hr = checked_u32(
            index,
            checked_add_u64(
                index,
                shifted_x,
                u64::from(self.hr_level_avg),
                "x << lvlShift + hrLevelAvg",
            )? >> 1,
            "u64 hrLevelAvg to u32",
        )?;
        let quant_add = checked_u32(
            index,
            checked_shl_u64(
                index,
                u64::from(x),
                u32::from(self.allow_tcq),
                "x << allowTcq",
            )?,
            "u64 quant extension to u32",
        )?;
        let quant = input
            .level
            .checked_add(quant_add)
            .ok_or(quant_overflow(index, "quant + x << allowTcq"))?;
        self.hr_level_avg = next_hr;

        Ok(CoeffReadQuant {
            quant: CoeffQuantReadInput {
                entry: input.entry,
                quant,
                hr_level_avg: next_hr,
            },
            path: CoeffReadQuantPath::Extended {
                m,
                k,
                c_max,
                q,
                length,
                x_base,
                coeff_rem,
                x,
            },
        })
    }
}

const fn get_msb(value: u32) -> u32 {
    if value == 0 {
        0
    } else {
        u32::BITS - 1 - value.leading_zeros()
    }
}

fn quant_threshold(
    index: usize,
    max_level: u32,
    allow_tcq: bool,
) -> Result<u32, CoeffReadQuantError> {
    max_level
        .checked_sub(u32::from(allow_tcq))
        .ok_or(CoeffReadQuantError::InvalidMaxLevel {
            index,
            max_level,
            allow_tcq,
        })
}

#[derive(Clone, Copy)]
enum BypassSyntax {
    Literal,
    Unary,
}

impl BypassSyntax {
    fn read(self, symbols: &mut SymbolDecoder<'_>, bits: u32) -> Result<u32, CoreError> {
        match self {
            Self::Literal => symbols.read_literal(bits),
            Self::Unary => symbols.read_unary(bits),
        }
    }
}

fn read_bypass_symbol(
    symbols: &mut SymbolDecoder<'_>,
    index: usize,
    bits: u32,
    syntax: &'static str,
    kind: BypassSyntax,
) -> Result<u32, CoeffReadQuantError> {
    let value = kind
        .read(symbols, bits)
        .map_err(|source| CoeffReadQuantError::LiteralRead {
            index,
            syntax,
            source,
        })?;
    Ok(value)
}

fn checked_add(
    index: usize,
    lhs: u32,
    rhs: u32,
    operation: &'static str,
) -> Result<u32, CoeffReadQuantError> {
    lhs.checked_add(rhs).ok_or(quant_overflow(index, operation))
}

fn checked_shl_u64(
    index: usize,
    value: u64,
    shift: u32,
    operation: &'static str,
) -> Result<u64, CoeffReadQuantError> {
    value
        .checked_shl(shift)
        .ok_or(quant_overflow(index, operation))
}

fn checked_add_u64(
    index: usize,
    lhs: u64,
    rhs: u64,
    operation: &'static str,
) -> Result<u64, CoeffReadQuantError> {
    lhs.checked_add(rhs).ok_or(quant_overflow(index, operation))
}

fn checked_u32(
    index: usize,
    value: u64,
    operation: &'static str,
) -> Result<u32, CoeffReadQuantError> {
    u32::try_from(value).map_err(|_| quant_overflow(index, operation))
}

fn quant_overflow(index: usize, operation: &'static str) -> CoeffReadQuantError {
    CoeffReadQuantError::QuantOverflow { index, operation }
}

#[cfg(test)]
#[path = "read_quant_tests.rs"]
mod tests;
