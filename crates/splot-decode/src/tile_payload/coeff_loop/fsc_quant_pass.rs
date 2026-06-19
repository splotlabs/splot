// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient quant pass.
//!
//! Feature tracking: `DECODE-COEFF-FSC-QUANT-PASS`,
//! `DECODE-COEFF-FSC-CONTEXT-COMMIT`,
//! `DECODE-COEFF-FSC-BRANCH-HANDOFF`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{
    CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};
use super::branch::{
    CoeffBlockEobBranch, CoeffBlockEobBranchInput, NonZeroCoeffBlockStartInput,
    read_coeff_block_eob_branch,
};
use super::fsc_level_pass::{
    CoeffFscLevelPassConfig, CoeffFscLevelPassError, CoeffFscLevelRead, NonZeroCoeffFscLevelPass,
    apply_nonzero_coeff_fsc_level_pass,
};
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
use super::scan_walk::{CoeffScanEntry, FscCoeffScanWalk, walk_fsc_coeff_scan};
use super::{AllZeroCoeffBlockInput, CoeffLoopContextError, NonZeroCoeffEobSymbolRead};

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

/// Caller-resolved facts for committing FSC end-of-`coeffs()` context lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscContextCommitConfig {
    /// Plane index; the FSC/IDTX branch is valid only for luma plane 0.
    pub(crate) plane: usize,
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

/// Caller-selected FSC/IDTX coefficient branch after `all_zero`.
pub(crate) enum CoeffFscBranchInput<'a> {
    /// Decoded `all_zero == 1`, invalid for the FSC-specific branch.
    AllZero(AllZeroCoeffBlockInput),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffFscBranchNonZeroInput<'a>),
}

/// Caller-resolved facts for the FSC/IDTX nonzero branch.
pub(crate) struct CoeffFscBranchNonZeroInput<'a> {
    /// Caller-resolved facts for nonzero EOB start.
    pub(crate) start: NonZeroCoeffBlockStartInput,
    /// Caller-resolved `segEob` for the FSC branch.
    pub(crate) seg_eob: usize,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved FSC level-pass facts.
    pub(crate) level_config: CoeffFscLevelPassConfig,
    /// Caller-resolved facts for committing tile context lines.
    pub(crate) context: CoeffFscContextCommitConfig,
}

/// Result of the loaded FSC/IDTX coefficient branch handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscBranch {
    pass: NonZeroCoeffFscQuantPass,
}

impl CoeffFscBranch {
    /// Completed FSC/IDTX quant pass.
    #[must_use]
    pub(crate) const fn pass(&self) -> &NonZeroCoeffFscQuantPass {
        &self.pass
    }
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
    /// The FSC/IDTX context commit was requested for a non-luma plane.
    #[error("coefficient FSC quant context commit requires luma plane, got plane {plane}")]
    NonLumaPlane {
        /// Rejected plane index.
        plane: usize,
    },
    /// Committing tile coefficient context lines failed.
    #[error("coefficient FSC quant context update failed: {0}")]
    ContextUpdate(TileCoeffStateError),
}

/// Error returned by the FSC/IDTX coefficient branch handoff.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscBranchError {
    /// The FSC-specific branch was asked to handle `all_zero == 1`.
    #[error("coefficient FSC branch does not support all-zero routing")]
    AllZero,
    /// The FSC-specific branch was requested for a non-luma plane.
    #[error("coefficient FSC branch requires luma plane, got plane {plane}")]
    NonLumaPlane {
        /// Rejected plane index.
        plane: usize,
    },
    /// EOB branch handoff or checked scan-walk derivation failed.
    #[error("coefficient FSC branch handoff failed: {0}")]
    Branch(#[from] CoeffLoopContextError),
    /// The FSC/IDTX first level pass failed.
    #[error("coefficient FSC branch level pass failed: {0}")]
    Level(#[from] CoeffFscLevelPassError),
    /// The FSC/IDTX quant and context-commit pass failed.
    #[error("coefficient FSC branch quant pass failed: {0}")]
    Quant(#[from] CoeffFscQuantPassError),
    /// Internal branch routing returned a different branch than requested.
    #[error("coefficient FSC branch returned unexpected {actual} arm while expecting {expected}")]
    UnexpectedBranch {
        /// Expected branch arm.
        expected: &'static str,
        /// Actual branch arm.
        actual: &'static str,
    },
}

/// Dispatches the FSC/IDTX nonzero coefficient branch.
///
/// This loaded-but-unwired helper models the AV2 §5.20.7.27 `useFsc` branch
/// after caller-decoded `all_zero == 0`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). It derives
/// `bob = segEob - eob`, walks the caller-resolved scan window, runs the FSC
/// level pass, then runs the FSC sign/quant pass and commits the final tile
/// context lines. Runtime `useFsc`, `segEob`, scan, transform, dequantization,
/// inverse transform, residual add, and reconstruction remain out of scope.
pub(crate) fn apply_coeff_fsc_branch(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchInput<'_>,
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    let input = match input {
        CoeffFscBranchInput::AllZero(input) => {
            let _ = input;
            return Err(CoeffFscBranchError::AllZero);
        }
        CoeffFscBranchInput::NonZero(input) => input,
    };
    if input.context.plane != 0 {
        return Err(CoeffFscBranchError::NonLumaPlane {
            plane: input.context.plane,
        });
    }

    let start = match read_coeff_block_eob_branch(
        state,
        cdfs,
        symbols,
        CoeffBlockEobBranchInput::NonZero(input.start),
    )? {
        CoeffBlockEobBranch::NonZero(start) => start,
        CoeffBlockEobBranch::AllZero(_) => {
            return Err(CoeffFscBranchError::UnexpectedBranch {
                expected: "nonzero",
                actual: "all-zero",
            });
        }
    };
    let walk = walk_fsc_coeff_scan(&start, input.seg_eob, input.scan)?;
    let level_pass =
        apply_nonzero_coeff_fsc_level_pass(cdfs, symbols, start, walk, input.level_config)?;
    let pass = apply_nonzero_coeff_fsc_quant_pass_with_context_commit(
        state,
        cdfs,
        symbols,
        level_pass,
        input.scan,
        input.level_config,
        input.context,
    )?;
    Ok(CoeffFscBranch { pass })
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

/// Runs the FSC/IDTX quant pass and commits tile context lines.
///
/// This wraps [`apply_nonzero_coeff_fsc_quant_pass`] with the AV2 §5.20.7.27
/// end-of-`coeffs()` context update
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). The final
/// `culLevel` and `dcCategory` come from the signed `Quant[]` state summary
/// produced after §5.20.7.28 `read_quant`; the caller still resolves `useFsc`,
/// `segEob`, scan, plane, and geometry facts. Runtime `coeffs()` wiring,
/// dequantization, inverse transform, residual add, and reconstruction remain
/// out of scope.
pub(crate) fn apply_nonzero_coeff_fsc_quant_pass_with_context_commit(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    level_pass: NonZeroCoeffFscLevelPass,
    scan: &[u16],
    config: CoeffFscLevelPassConfig,
    context: CoeffFscContextCommitConfig,
) -> Result<NonZeroCoeffFscQuantPass, CoeffFscQuantPassError> {
    if context.plane != 0 {
        return Err(CoeffFscQuantPassError::NonLumaPlane {
            plane: context.plane,
        });
    }

    let pass = apply_nonzero_coeff_fsc_quant_pass(cdfs, symbols, level_pass, scan, config)?;
    let quant_state = pass.quant_state();
    state
        .update_after_coeffs(CoeffContextUpdate {
            plane: context.plane,
            x4: context.x4,
            y4: context.y4,
            w4: context.w4,
            h4: context.h4,
            cul_level: quant_state.cul_level(),
            dc_category: quant_state.dc_category(),
        })
        .map_err(CoeffFscQuantPassError::ContextUpdate)?;
    Ok(pass)
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
