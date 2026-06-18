// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient pass composition.
//!
//! Feature tracking: `DECODE-COEFF-ORDINARY-PASS-COMPOSE`,
//! `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS`.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::TransformCoeffBlockState;
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError,
    NonZeroCoeffBaseDerivedLevelPass, apply_nonzero_coeff_base_derived_level_pass,
};
use super::base_symbol::{
    CoeffBaseSymbolRead, CoeffBaseSymbolReadError, CoeffBaseSymbolReadInput,
    read_nonzero_coeff_base_symbols,
};
use super::branch::NonZeroCoeffBlockStart;
use super::level_state::{CoeffLevelStateWriteError, apply_nonzero_coeff_base_levels};
use super::max_level::{CoeffMaxLevelConfig, derive_nonzero_coeff_max_levels};
use super::quant_pass::{
    CoeffQuantPassConfig, CoeffQuantPassError, CoeffQuantPassMaxLevelConfig, NonZeroCoeffQuantPass,
    validate_coeff_quant_pass_config,
};
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, CoeffQuantStateWriteError,
    NonZeroCoeffQuantState, apply_nonzero_coeff_quant_state_step,
};
use super::read_quant::{CoeffReadQuantConfig, CoeffReadQuantInput, CoeffReadQuantState};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::sign_symbol::{
    CoeffSignRead, CoeffSignReadError, CoeffSignReadInput, CoeffSignReadSymbol,
    preflight_nonzero_coeff_signs, read_preflighted_nonzero_coeff_sign,
};
use super::{CoeffLoopContextError, NonZeroCoeffEobSymbolRead};

/// Caller-resolved facts for the loaded ordinary non-FSC coefficient pass.
pub(crate) struct CoeffOrdinaryPassInput<'a> {
    /// Decoded nonzero EOB and zeroed local coefficient state.
    pub(crate) start: NonZeroCoeffBlockStart,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved base/base-range inputs, one per checked scan entry.
    ///
    /// Runtime derivation from evolving `Level[]` neighbours and TCQ state is
    /// intentionally deferred; this loaded boundary only consumes supplied rows.
    pub(crate) base_inputs: &'a [CoeffBaseSymbolReadInput],
    /// Caller-resolved sign inputs, one per checked scan entry.
    ///
    /// Runtime selection of skipped zero-level signs versus sign syntax is
    /// intentionally deferred until the real `coeffs()` integration.
    pub(crate) sign_inputs: &'a [CoeffSignReadInput],
    /// Caller-resolved plane and transform-class facts for `maxLevel`.
    pub(crate) max_level_config: CoeffQuantPassMaxLevelConfig,
    /// Caller-resolved hidden, sumAbs1, TCQ, and lossless facts.
    ///
    /// This wrapper resets `hrLevelAvg` to `0` at the coefficient-block entry
    /// as required by AV2 § 5.20.7.27 before calling `read_quant`.
    pub(crate) quant_config: CoeffQuantPassConfig,
}

/// Caller-resolved facts for the derived-base ordinary non-FSC coefficient pass.
pub(crate) struct CoeffOrdinaryDerivedBasePassInput<'a> {
    /// Decoded nonzero EOB and zeroed local coefficient state.
    pub(crate) start: NonZeroCoeffBlockStart,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors and first-pass state.
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    /// Caller-resolved sign inputs, one per checked scan entry.
    ///
    /// Runtime selection of skipped zero-level signs versus sign syntax is
    /// intentionally deferred until the real `coeffs()` integration.
    pub(crate) sign_inputs: &'a [CoeffSignReadInput],
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Result of the loaded ordinary non-FSC coefficient pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffOrdinaryPass {
    eob_read: NonZeroCoeffEobSymbolRead,
    walk: NonZeroCoeffScanWalk,
    base_reads: Vec<CoeffBaseSymbolRead>,
    sign_reads: Vec<CoeffSignRead>,
    quant_pass: NonZeroCoeffQuantPass,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffOrdinaryPass {
    /// Decoded nonzero EOB syntax carried from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    /// Checked scan walk used by every composed phase.
    #[must_use]
    pub(crate) const fn walk(&self) -> &NonZeroCoeffScanWalk {
        &self.walk
    }

    /// Decoded base/base-range summaries in scan-walk order.
    #[must_use]
    pub(crate) fn base_reads(&self) -> &[CoeffBaseSymbolRead] {
        &self.base_reads
    }

    /// Decoded sign summaries in scan-walk order.
    #[must_use]
    pub(crate) fn sign_reads(&self) -> &[CoeffSignRead] {
        &self.sign_reads
    }

    /// Composed `read_quant` and signed `Quant[]` state summary.
    #[must_use]
    pub(crate) const fn quant_pass(&self) -> &NonZeroCoeffQuantPass {
        &self.quant_pass
    }

    /// Final local coefficient state after `Level[]` and signed `Quant[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Result of the loaded ordinary non-FSC coefficient pass with derived base state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffOrdinaryDerivedBasePass {
    base_level_pass: NonZeroCoeffBaseDerivedLevelPass,
    sign_reads: Vec<CoeffSignRead>,
    quant_pass: NonZeroCoeffQuantPass,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffOrdinaryDerivedBasePass {
    /// Decoded nonzero EOB syntax carried from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.base_level_pass.eob_read()
    }

    /// Checked scan walk used by every composed phase.
    #[must_use]
    pub(crate) const fn walk(&self) -> &NonZeroCoeffScanWalk {
        self.base_level_pass.walk()
    }

    /// First-pass base/level derivation result.
    #[must_use]
    pub(crate) const fn base_level_pass(&self) -> &NonZeroCoeffBaseDerivedLevelPass {
        &self.base_level_pass
    }

    /// Derived base/base-range selector inputs in scan-walk order.
    #[must_use]
    pub(crate) fn derived_base_inputs(&self) -> &[CoeffBaseSymbolReadInput] {
        self.base_level_pass.derived_inputs()
    }

    /// Decoded base/base-range summaries in scan-walk order.
    #[must_use]
    pub(crate) fn base_reads(&self) -> &[CoeffBaseSymbolRead] {
        self.base_level_pass.base_reads()
    }

    /// Decoded sign summaries in scan-walk order.
    #[must_use]
    pub(crate) fn sign_reads(&self) -> &[CoeffSignRead] {
        &self.sign_reads
    }

    /// Composed `read_quant` and signed `Quant[]` state summary.
    #[must_use]
    pub(crate) const fn quant_pass(&self) -> &NonZeroCoeffQuantPass {
        &self.quant_pass
    }

    /// Final local coefficient state after `Level[]` and signed `Quant[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Error returned by the ordinary non-FSC pass composition boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffOrdinaryPassError {
    /// Checked scan walking failed.
    #[error("ordinary coefficient pass scan walk failed: {0}")]
    Scan(#[from] CoeffLoopContextError),
    /// Base/base-range symbol reading failed.
    #[error("ordinary coefficient pass base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
    /// Derived base/level first-pass composition failed.
    #[error("ordinary coefficient pass derived base/level first pass failed: {0}")]
    BaseDerived(#[from] CoeffBaseDerivedLevelPassError),
    /// Local `Level[]` state writes failed.
    #[error("ordinary coefficient pass level state write failed: {0}")]
    Level(#[from] CoeffLevelStateWriteError),
    /// Sign syntax reading failed.
    #[error("ordinary coefficient pass sign read failed: {0}")]
    Sign(#[from] CoeffSignReadError),
    /// `read_quant` plus signed `Quant[]` writes failed.
    #[error("ordinary coefficient pass quant pass failed: {0}")]
    Quant(#[from] CoeffQuantPassError),
}

/// Runs the loaded ordinary non-FSC coefficient pass from nonzero EOB start.
///
/// This composes the existing AV2 § 5.20.7.27 boundaries for checked scan walk,
/// base/base-range reads, local `Level[]` writes, and the interleaved sign,
/// `maxLevel`, § 5.20.7.28 `read_quant`, and signed `Quant[pos]` steps
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27` and
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28`). The caller still
/// owns runtime scan, evolving CDF selector, post-level sign-source,
/// transform-class, hidden parity, sumAbs1, TCQ, and lossless derivation. Tile
/// context writes, dequantization, inverse transform, residual add, and
/// reconstruction remain out of scope.
pub(crate) fn apply_nonzero_coeff_ordinary_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryPassInput<'_>,
) -> Result<NonZeroCoeffOrdinaryPass, CoeffOrdinaryPassError> {
    let walk = walk_nonzero_coeff_scan(&input.start, input.scan)?;
    let base_reads = read_nonzero_coeff_base_symbols(cdfs, symbols, &walk, input.base_inputs)?;
    let level_state = apply_nonzero_coeff_base_levels(input.start, &walk, &base_reads)?;
    let (eob_read, mut block) = level_state.into_parts();

    let sign_levels = preflight_nonzero_coeff_signs(&block, &walk, input.sign_inputs)?;
    let quant_config = CoeffQuantPassConfig {
        hr_level_avg: 0,
        ..input.quant_config
    };
    let quant_pass = apply_interleaved_sign_and_quant_pass(
        cdfs,
        symbols,
        &mut block,
        &walk,
        InterleavedSignQuantInput {
            sign_inputs: input.sign_inputs,
            sign_levels: &sign_levels,
            max_level_config: input.max_level_config,
            config: quant_config,
        },
    )?;
    let sign_reads = quant_pass.0;
    let quant_pass = quant_pass.1;

    Ok(NonZeroCoeffOrdinaryPass {
        eob_read,
        walk,
        base_reads,
        sign_reads,
        quant_pass,
        block,
    })
}

/// Runs the loaded ordinary non-FSC coefficient pass with derived base selectors.
///
/// This composes the AV2 § 5.20.7.27 state-derived first pass with the existing
/// interleaved sign, `maxLevel`, § 5.20.7.28 `read_quant`, and signed
/// `Quant[pos]` steps
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27` and
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28`). The first pass
/// supplies base/base-range reads, local `Level[]`, hidden parity, and `sumAbs1`
/// facts. The caller still owns runtime scan, geometry, plane, transform-class,
/// parity, TCQ, lossless, and sign-source derivation. Runtime `coeffs()` wiring,
/// tile context writes, dequantization, inverse transform, residual add, and
/// reconstruction remain out of scope.
pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_derived_base(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryDerivedBasePassInput<'_>,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryPassError> {
    let base_config = input.base_config;
    let lossless = input.lossless;
    let walk = walk_nonzero_coeff_scan(&input.start, input.scan)?;
    let base_level_pass =
        apply_nonzero_coeff_base_derived_level_pass(cdfs, symbols, input.start, walk, base_config)?;
    let first_pass = base_level_pass.first_pass();
    let sign_levels = preflight_nonzero_coeff_signs(
        base_level_pass.block(),
        base_level_pass.walk(),
        input.sign_inputs,
    )?;
    let quant_config = CoeffQuantPassConfig {
        is_hidden: first_pass.is_hidden(),
        sum_abs1: first_pass.sum_abs1(),
        use_tcq: base_config.use_tcq,
        lossless,
        hr_level_avg: 0,
    };
    let mut block = base_level_pass.block().clone();
    let quant_pass = apply_interleaved_sign_and_quant_pass(
        cdfs,
        symbols,
        &mut block,
        base_level_pass.walk(),
        InterleavedSignQuantInput {
            sign_inputs: input.sign_inputs,
            sign_levels: &sign_levels,
            max_level_config: CoeffQuantPassMaxLevelConfig {
                plane: base_config.plane,
                tx_class: base_config.tx_class,
            },
            config: quant_config,
        },
    )?;
    let sign_reads = quant_pass.0;
    let quant_pass = quant_pass.1;

    Ok(NonZeroCoeffOrdinaryDerivedBasePass {
        base_level_pass,
        sign_reads,
        quant_pass,
        block,
    })
}

struct InterleavedSignQuantInput<'a> {
    sign_inputs: &'a [CoeffSignReadInput],
    sign_levels: &'a [u32],
    max_level_config: CoeffQuantPassMaxLevelConfig,
    config: CoeffQuantPassConfig,
}

fn apply_interleaved_sign_and_quant_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    input: InterleavedSignQuantInput<'_>,
) -> Result<(Vec<CoeffSignRead>, NonZeroCoeffQuantPass), CoeffOrdinaryPassError> {
    let InterleavedSignQuantInput {
        sign_inputs,
        sign_levels,
        max_level_config,
        config,
    } = input;

    validate_coeff_quant_pass_config(config)?;

    let entries = walk.entries();
    let max_levels = derive_nonzero_coeff_max_levels(
        walk,
        CoeffMaxLevelConfig {
            plane: max_level_config.plane,
            tx_class: max_level_config.tx_class,
            is_hidden: config.is_hidden,
        },
    )
    .map_err(CoeffQuantPassError::from)?;
    for (index, (entry, max_level)) in entries.iter().copied().zip(max_levels.iter()).enumerate() {
        debug_assert_eq!(max_level.entry, entry);
        max_level
            .max_level
            .checked_sub(u32::from(config.use_tcq))
            .ok_or(CoeffQuantPassError::InvalidMaxLevel {
                index,
                max_level: max_level.max_level,
                use_tcq: config.use_tcq,
            })?;
        block
            .quant_at(entry.pos())
            .map_err(CoeffQuantPassError::from)?;
    }

    let mut sign_reads = Vec::new();
    let mut read_quants = Vec::new();
    let mut quant_writes = Vec::new();
    sign_reads
        .try_reserve(entries.len())
        .map_err(CoeffSignReadError::from)?;
    read_quants
        .try_reserve(entries.len())
        .map_err(CoeffQuantPassError::from)?;
    quant_writes
        .try_reserve(entries.len())
        .map_err(CoeffQuantStateWriteError::from)
        .map_err(CoeffQuantPassError::from)?;

    let mut read_quant_state = CoeffReadQuantState::new(CoeffReadQuantConfig {
        is_hidden: config.is_hidden,
        allow_tcq: config.use_tcq,
        hr_level_avg: config.hr_level_avg,
    });
    let mut quant_state = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: config.is_hidden,
        sum_abs1: config.sum_abs1,
        use_tcq: config.use_tcq,
        lossless: config.lossless,
    });

    for (index, (((entry, sign_input), level), max_level)) in entries
        .iter()
        .copied()
        .zip(sign_inputs.iter().copied())
        .zip(sign_levels.iter().copied())
        .zip(max_levels.iter().copied())
        .enumerate()
    {
        let sign = read_preflighted_nonzero_coeff_sign(cdfs, symbols, sign_input, level)?;
        if config.is_hidden
            && config.sum_abs1 > 0
            && entry.scan_index() == 0
            && sign.symbol() == CoeffSignReadSymbol::None
        {
            return Err(CoeffQuantPassError::HiddenParityMissingSign { index, entry }.into());
        }
        let read_quant = read_quant_state
            .read_one(
                symbols,
                index,
                CoeffReadQuantInput {
                    entry,
                    level,
                    max_level: max_level.max_level,
                },
            )
            .map_err(CoeffQuantPassError::from)?;
        let write = apply_nonzero_coeff_quant_state_step(
            block,
            &mut quant_state,
            index,
            entry,
            sign,
            read_quant.quant_input(),
        )
        .map_err(CoeffQuantPassError::from)?;

        sign_reads.push(sign);
        read_quants.push(read_quant);
        quant_writes.push(write);
    }

    let quant_state = NonZeroCoeffQuantState::from_interleaved_parts(quant_writes, quant_state);
    Ok((
        sign_reads,
        NonZeroCoeffQuantPass::from_interleaved_parts(read_quants, quant_state),
    ))
}
