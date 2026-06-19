// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient quant pass.
//!
//! Feature tracking: `DECODE-COEFF-FSC-QUANT-PASS`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEobSymbolRead;
use super::fsc_level_pass::{CoeffFscLevelPassConfig, CoeffFscLevelRead, NonZeroCoeffFscLevelPass};
use super::fsc_sign_pass::{
    CoeffFscSignPassError, CoeffFscSignRead, CoeffFscSignReadInput, derive_fsc_sign_input,
    fsc_sign_entries, preflight_pass, quant_sign_value, read_fsc_sign_symbol,
};
use super::max_level::{COEFF_BASE_RANGE, NUM_BASE_LEVELS};
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, CoeffQuantStateWrite,
    CoeffQuantStateWriteError, NonZeroCoeffQuantState,
};
use super::read_quant::{
    CoeffReadQuant, CoeffReadQuantConfig, CoeffReadQuantError, CoeffReadQuantInput,
    CoeffReadQuantState,
};
use super::scan_walk::{CoeffScanEntry, FscCoeffScanWalk};

const FSC_MAX_LEVEL: u32 = NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1;

/// Result of the FSC/IDTX quant pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffFscQuantPass {
    eob_read: NonZeroCoeffEobSymbolRead,
    level_walk: FscCoeffScanWalk,
    level_reads: Vec<CoeffFscLevelRead>,
    sign_entries: Vec<CoeffScanEntry>,
    sign_inputs: Vec<CoeffFscSignReadInput>,
    sign_reads: Vec<CoeffFscSignRead>,
    read_quants: Vec<CoeffReadQuant>,
    quant_state: NonZeroCoeffQuantState,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffFscQuantPass {
    /// Decoded nonzero EOB syntax carried from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    /// Checked `bob..segEob` walk used by the level first pass.
    #[must_use]
    pub(crate) const fn level_walk(&self) -> &FscCoeffScanWalk {
        &self.level_walk
    }

    /// Decoded level reads in forward `bob..segEob` order.
    #[must_use]
    pub(crate) fn level_reads(&self) -> &[CoeffFscLevelRead] {
        &self.level_reads
    }

    /// Checked sign and quant entries in forward `0..segEob` order.
    #[must_use]
    pub(crate) fn sign_entries(&self) -> &[CoeffScanEntry] {
        &self.sign_entries
    }

    /// Derived sign inputs in forward `0..segEob` order.
    #[must_use]
    pub(crate) fn sign_inputs(&self) -> &[CoeffFscSignReadInput] {
        &self.sign_inputs
    }

    /// Decoded sign reads in forward `0..segEob` order.
    #[must_use]
    pub(crate) fn sign_reads(&self) -> &[CoeffFscSignRead] {
        &self.sign_reads
    }

    /// Raw `read_quant` results in forward `0..segEob` order.
    #[must_use]
    pub(crate) fn read_quants(&self) -> &[CoeffReadQuant] {
        &self.read_quants
    }

    /// Final quant-state summary after signed `Quant[]` writes.
    #[must_use]
    pub(crate) const fn quant_state(&self) -> &NonZeroCoeffQuantState {
        &self.quant_state
    }

    /// Local coefficient state after FSC `Quant[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Error returned by the FSC/IDTX quant-pass boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscQuantPassError {
    /// Allocation for read or write summaries failed.
    #[error("coefficient FSC quant allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// The FSC/IDTX sign step failed.
    #[error("coefficient FSC quant sign step failed: {0}")]
    Sign(#[from] CoeffFscSignPassError),
    /// The local transform-block state rejected a checked coordinate or position.
    #[error("coefficient FSC quant state error: {0}")]
    State(#[from] TileCoeffStateError),
    /// The `read_quant` parser failed.
    #[error("coefficient FSC quant read_quant failed: {0}")]
    ReadQuant(#[from] CoeffReadQuantError),
    /// Applying signed quant state failed.
    #[error("coefficient FSC quant write failed: {0}")]
    QuantState(#[from] CoeffQuantStateWriteError),
}

/// Runs the FSC/IDTX §5.20.7.27 sign/quant loop over `c = 0 .. segEob`.
///
/// The helper follows the second `useFsc` loop in spec order. For each checked
/// entry, it reads `idtx_sign` when the local level is nonzero, immediately
/// writes `QuantSign[]` for later sign contexts, calls §5.20.7.28
/// `read_quant(level, pos, 0, NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1,
/// hrLevelAvg, 0)`, computes signed `Quant[pos]`, and derives final `culLevel`
/// and `dcCategory`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`;
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28`). It does not
/// commit tile context lines, dequantize, or reconstruct pixels.
pub(crate) fn apply_nonzero_coeff_fsc_quant_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    level_pass: NonZeroCoeffFscLevelPass,
    scan: &[u16],
    config: CoeffFscLevelPassConfig,
) -> Result<NonZeroCoeffFscQuantPass, CoeffFscQuantPassError> {
    let (eob_read, level_walk, _level_inputs, level_reads, mut block) = level_pass.into_parts();
    preflight_pass(&block, &level_walk, scan, config)?;

    let sign_entries = fsc_sign_entries(&block, level_walk.seg_eob(), scan)?;
    let mut interleaved = FscInterleavedQuantPass::new(sign_entries.len())?;
    interleaved.run(cdfs, symbols, &mut block, &sign_entries, config)?;
    let (sign_inputs, sign_reads, read_quants, quant_state) = interleaved.finish(&mut block)?;

    Ok(NonZeroCoeffFscQuantPass {
        eob_read,
        level_walk,
        level_reads,
        sign_entries,
        sign_inputs,
        sign_reads,
        read_quants,
        quant_state,
        block,
    })
}

struct FscInterleavedQuantPass {
    sign_inputs: Vec<CoeffFscSignReadInput>,
    sign_reads: Vec<CoeffFscSignRead>,
    read_quants: Vec<CoeffReadQuant>,
    quant_writes: Vec<CoeffQuantStateWrite>,
    read_quant_state: CoeffReadQuantState,
    quant_state: CoeffQuantStateAccumulator,
}

type FscInterleavedQuantPassOutput = (
    Vec<CoeffFscSignReadInput>,
    Vec<CoeffFscSignRead>,
    Vec<CoeffReadQuant>,
    NonZeroCoeffQuantState,
);

impl FscInterleavedQuantPass {
    fn new(entry_count: usize) -> Result<Self, CoeffFscQuantPassError> {
        let mut sign_inputs = Vec::new();
        let mut sign_reads = Vec::new();
        let mut read_quants = Vec::new();
        let mut quant_writes = Vec::new();
        sign_inputs.try_reserve(entry_count)?;
        sign_reads.try_reserve(entry_count)?;
        read_quants.try_reserve(entry_count)?;
        quant_writes.try_reserve(entry_count)?;
        Ok(Self {
            sign_inputs,
            sign_reads,
            read_quants,
            quant_writes,
            read_quant_state: CoeffReadQuantState::new(CoeffReadQuantConfig {
                is_hidden: false,
                allow_tcq: false,
                hr_level_avg: 0,
            }),
            quant_state: CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
                is_hidden: false,
                sum_abs1: 0,
                use_tcq: false,
                lossless: false,
            }),
        })
    }

    fn run(
        &mut self,
        cdfs: &mut TileCdfSubset,
        symbols: &mut SymbolDecoder<'_>,
        block: &mut TransformCoeffBlockState,
        sign_entries: &[CoeffScanEntry],
        config: CoeffFscLevelPassConfig,
    ) -> Result<(), CoeffFscQuantPassError> {
        for (index, entry) in sign_entries.iter().copied().enumerate() {
            self.step(cdfs, symbols, block, index, entry, config)?;
        }
        Ok(())
    }

    fn step(
        &mut self,
        cdfs: &mut TileCdfSubset,
        symbols: &mut SymbolDecoder<'_>,
        block: &mut TransformCoeffBlockState,
        index: usize,
        entry: CoeffScanEntry,
        config: CoeffFscLevelPassConfig,
    ) -> Result<(), CoeffFscQuantPassError> {
        let sign_input = derive_fsc_sign_input(entry, block, config)?;
        let sign = read_fsc_sign_symbol(cdfs, symbols, sign_input)?;
        if sign_input.level != 0 {
            block.set_quant_sign(entry.row(), entry.col(), quant_sign_value(sign.sign()))?;
        }
        let read_quant = self.read_quant_state.read_one(
            symbols,
            index,
            CoeffReadQuantInput {
                entry,
                level: sign_input.level,
                max_level: FSC_MAX_LEVEL,
            },
        )?;
        let quant_write = self.quant_state.apply_entry(
            index,
            entry,
            sign_input.level,
            sign.sign(),
            read_quant.quant_input(),
        )?;
        self.sign_inputs.push(sign_input);
        self.sign_reads.push(sign);
        self.read_quants.push(read_quant);
        self.quant_writes.push(quant_write);
        Ok(())
    }

    fn finish(
        self,
        block: &mut TransformCoeffBlockState,
    ) -> Result<FscInterleavedQuantPassOutput, CoeffFscQuantPassError> {
        let Self {
            sign_inputs,
            sign_reads,
            read_quants,
            quant_writes,
            read_quant_state: _,
            quant_state,
        } = self;
        for write in &quant_writes {
            block.set_quant(write.entry().pos(), write.quant())?;
        }
        Ok((
            sign_inputs,
            sign_reads,
            read_quants,
            NonZeroCoeffQuantState::from_interleaved_parts(quant_writes, quant_state),
        ))
    }
}
