// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient quant pass.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    ADJUSTED_TX_SIZE, TX_HEIGHT, TX_SIZE_SQR, TX_SIZE_SQR_UP, TX_WIDTH,
};

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{
    TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};
use super::branch::NonZeroCoeffBlockStart;
use super::fsc_level_pass::{
    CoeffFscLevelPassConfig, CoeffFscLevelPassError, apply_nonzero_coeff_fsc_level_pass,
};
use super::fsc_sign_pass::{
    CoeffFscSignPassError, checked_fsc_sign_entries, derive_fsc_sign_input, quant_sign_value,
    read_fsc_sign_symbol,
};
use super::max_level::{COEFF_BASE_RANGE, CoeffTransformClass, NUM_BASE_LEVELS};
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, CoeffQuantStateWrite,
    CoeffQuantStateWriteError, NonZeroCoeffQuantState,
};
use super::read_quant::{
    CoeffReadQuantConfig, CoeffReadQuantError, CoeffReadQuantInput, CoeffReadQuantState,
};
use super::scan_walk::{
    CoeffScanEntry, CoeffScanOrderError, derive_coeff_scan_order, walk_fsc_coeff_scan,
};
use super::{AllZeroCoeffBlockInput, CoeffLoopContextError, commit_nonzero_coeff_context};

const FSC_MAX_LEVEL: u32 = NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1;

struct CoeffFscBranchTxSizeFacts {
    scan: Vec<u16>,
    level_config: CoeffFscLevelPassConfig,
    context: AllZeroCoeffBlockInput,
}

pub(crate) struct CoeffFscStagedTxSizeNonZeroInput {
    pub(crate) block: AllZeroCoeffBlockInput,
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) tx_size: usize,
    pub(crate) plane_tx_type: usize,
    pub(crate) coeff_cdf_q_ctx: usize,
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscQuantPassError {
    #[error("coefficient FSC quant allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("coefficient FSC quant sign step failed: {0}")]
    Sign(#[from] CoeffFscSignPassError),
    #[error("coefficient FSC quant state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient FSC quant read_quant failed: {0}")]
    ReadQuant(#[from] CoeffReadQuantError),
    #[error("coefficient FSC quant write failed: {0}")]
    QuantState(#[from] CoeffQuantStateWriteError),
    #[error("coefficient FSC quant context update failed: {0}")]
    ContextUpdate(TileCoeffStateError),
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscBranchError {
    #[error("coefficient FSC branch invalid transform size index {tx_size}")]
    InvalidTransformSize { tx_size: usize },
    #[error("coefficient FSC branch invalid {table}[{tx_size}] transform-size table value {value}")]
    InvalidTransformSizeTableValue {
        table: &'static str,
        tx_size: usize,
        value: i32,
    },
    #[error("coefficient FSC branch invalid {table}[{tx_size}] transform-size table index {value}")]
    InvalidTransformSizeTableIndex {
        table: &'static str,
        tx_size: usize,
        value: usize,
    },
    #[error("coefficient FSC branch transform-size context overflow for txSz {tx_size}")]
    TransformSizeContextOverflow { tx_size: usize },
    #[error(
        "coefficient FSC branch block geometry {actual_w4}x{actual_h4} does not match txSz {tx_size} geometry {expected_w4}x{expected_h4}"
    )]
    BlockGeometryMismatch {
        tx_size: usize,
        actual_w4: usize,
        actual_h4: usize,
        expected_w4: usize,
        expected_h4: usize,
    },
    #[error("coefficient FSC branch scan-order derivation failed: {0}")]
    ScanOrder(#[from] CoeffScanOrderError),
    #[error("coefficient FSC branch requires luma plane, got plane {plane}")]
    NonLumaPlane { plane: usize },
    #[error("coefficient FSC branch handoff failed: {0}")]
    Branch(#[from] CoeffLoopContextError),
    #[error("coefficient FSC branch level pass failed: {0}")]
    Level(#[from] CoeffFscLevelPassError),
    #[error("coefficient FSC branch quant pass failed: {0}")]
    Quant(#[from] CoeffFscQuantPassError),
}

pub(crate) fn apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscStagedTxSizeNonZeroInput,
) -> Result<TransformCoeffBlockState, CoeffFscBranchError> {
    if input.block.plane != 0 {
        return Err(CoeffFscBranchError::NonLumaPlane {
            plane: input.block.plane,
        });
    }

    let facts = fsc_branch_tx_size_facts(
        input.block,
        input.tx_size,
        input.plane_tx_type,
        input.coeff_cdf_q_ctx,
    )?;
    let walk = walk_fsc_coeff_scan(&input.start, facts.scan.len(), &facts.scan)?;
    let level_pass =
        apply_nonzero_coeff_fsc_level_pass(cdfs, symbols, input.start, walk, facts.level_config)?;
    let (level_walk, mut block) = level_pass.into_parts();
    let sign_entries =
        checked_fsc_sign_entries(&block, &level_walk, &facts.scan, facts.level_config)
            .map_err(CoeffFscQuantPassError::from)?;
    let mut interleaved = FscInterleavedQuantPass::new(sign_entries.len())?;
    interleaved.run(cdfs, symbols, &mut block, &sign_entries, facts.level_config)?;
    let quant_state = interleaved.finish(&mut block)?;
    commit_nonzero_coeff_context(state, facts.context, &quant_state)
        .map_err(CoeffFscQuantPassError::ContextUpdate)?;
    Ok(block)
}

fn fsc_branch_tx_size_facts(
    block: AllZeroCoeffBlockInput,
    tx_size: usize,
    plane_tx_type: usize,
    coeff_cdf_q_ctx: usize,
) -> Result<CoeffFscBranchTxSizeFacts, CoeffFscBranchError> {
    let raw_tx_width = TX_WIDTH
        .get(tx_size)
        .copied()
        .ok_or(CoeffFscBranchError::InvalidTransformSize { tx_size })?;
    let tx_width = usize::try_from(raw_tx_width).map_err(|_| {
        CoeffFscBranchError::InvalidTransformSizeTableValue {
            table: "Tx_Width",
            tx_size,
            value: raw_tx_width,
        }
    })?;
    let raw_tx_height = TX_HEIGHT
        .get(tx_size)
        .copied()
        .ok_or(CoeffFscBranchError::InvalidTransformSize { tx_size })?;
    let tx_height = usize::try_from(raw_tx_height).map_err(|_| {
        CoeffFscBranchError::InvalidTransformSizeTableValue {
            table: "Tx_Height",
            tx_size,
            value: raw_tx_height,
        }
    })?;
    validate_fsc_block_geometry(block, tx_size, tx_width, tx_height)?;

    let raw_adjusted_tx_size = ADJUSTED_TX_SIZE[tx_size];
    let adjusted_tx_size = usize::try_from(raw_adjusted_tx_size).map_err(|_| {
        CoeffFscBranchError::InvalidTransformSizeTableValue {
            table: "Adjusted_Tx_Size",
            tx_size,
            value: raw_adjusted_tx_size,
        }
    })?;
    if TX_WIDTH.get(adjusted_tx_size).is_none() {
        return Err(CoeffFscBranchError::InvalidTransformSizeTableIndex {
            table: "Adjusted_Tx_Size",
            tx_size,
            value: adjusted_tx_size,
        });
    }
    let raw_adjusted_tx_width = TX_WIDTH[adjusted_tx_size];
    let adjusted_tx_width = usize::try_from(raw_adjusted_tx_width).map_err(|_| {
        CoeffFscBranchError::InvalidTransformSizeTableValue {
            table: "Tx_Width",
            tx_size: adjusted_tx_size,
            value: raw_adjusted_tx_width,
        }
    })?;
    let raw_adjusted_tx_height = TX_HEIGHT[adjusted_tx_size];
    let adjusted_tx_height = usize::try_from(raw_adjusted_tx_height).map_err(|_| {
        CoeffFscBranchError::InvalidTransformSizeTableValue {
            table: "Tx_Height",
            tx_size: adjusted_tx_size,
            value: raw_adjusted_tx_height,
        }
    })?;

    let raw_tx_size_sqr = TX_SIZE_SQR[tx_size];
    let tx_size_sqr = usize::try_from(raw_tx_size_sqr).map_err(|_| {
        CoeffFscBranchError::InvalidTransformSizeTableValue {
            table: "Tx_Size_Sqr",
            tx_size,
            value: raw_tx_size_sqr,
        }
    })?;
    if TX_WIDTH.get(tx_size_sqr).is_none() {
        return Err(CoeffFscBranchError::InvalidTransformSizeTableIndex {
            table: "Tx_Size_Sqr",
            tx_size,
            value: tx_size_sqr,
        });
    }
    let raw_tx_size_sqr_up = TX_SIZE_SQR_UP[tx_size];
    let tx_size_sqr_up = usize::try_from(raw_tx_size_sqr_up).map_err(|_| {
        CoeffFscBranchError::InvalidTransformSizeTableValue {
            table: "Tx_Size_Sqr_Up",
            tx_size,
            value: raw_tx_size_sqr_up,
        }
    })?;
    if TX_WIDTH.get(tx_size_sqr_up).is_none() {
        return Err(CoeffFscBranchError::InvalidTransformSizeTableIndex {
            table: "Tx_Size_Sqr_Up",
            tx_size,
            value: tx_size_sqr_up,
        });
    }
    let tx_size_ctx = tx_size_sqr
        .checked_add(tx_size_sqr_up)
        .and_then(|sum| sum.checked_add(1))
        .map(|sum| sum >> 1)
        .ok_or(CoeffFscBranchError::TransformSizeContextOverflow { tx_size })?;

    Ok(CoeffFscBranchTxSizeFacts {
        scan: derive_coeff_scan_order(
            tx_width,
            tx_height,
            CoeffTransformClass::from_plane_tx_type(plane_tx_type),
        )?,
        level_config: CoeffFscLevelPassConfig {
            coeff_cdf_q_ctx,
            tx_size_ctx,
            tx_width: adjusted_tx_width,
            tx_height: adjusted_tx_height,
        },
        context: AllZeroCoeffBlockInput {
            plane: block.plane,
            x4: block.x4,
            y4: block.y4,
            w4: block.w4,
            h4: block.h4,
        },
    })
}

fn validate_fsc_block_geometry(
    block: AllZeroCoeffBlockInput,
    tx_size: usize,
    tx_width: usize,
    tx_height: usize,
) -> Result<(), CoeffFscBranchError> {
    let expected_w4 = tx_width >> 2;
    let expected_h4 = tx_height >> 2;
    if block.w4 != expected_w4 || block.h4 != expected_h4 {
        return Err(CoeffFscBranchError::BlockGeometryMismatch {
            tx_size,
            actual_w4: block.w4,
            actual_h4: block.h4,
            expected_w4,
            expected_h4,
        });
    }
    Ok(())
}

struct FscInterleavedQuantPass {
    quant_writes: Vec<CoeffQuantStateWrite>,
    read_quant_state: CoeffReadQuantState,
    quant_state: CoeffQuantStateAccumulator,
}

impl FscInterleavedQuantPass {
    fn new(entry_count: usize) -> Result<Self, CoeffFscQuantPassError> {
        let mut quant_writes = Vec::new();
        quant_writes.try_reserve(entry_count)?;
        Ok(Self {
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
        let quant_write = self
            .quant_state
            .apply_entry(index, entry, sign.sign(), read_quant)?;
        self.quant_writes.push(quant_write);
        Ok(())
    }

    fn finish(
        self,
        block: &mut TransformCoeffBlockState,
    ) -> Result<NonZeroCoeffQuantState, CoeffFscQuantPassError> {
        let Self {
            quant_writes,
            read_quant_state: _,
            quant_state,
        } = self;
        for write in &quant_writes {
            block.set_quant(write.entry().pos(), write.quant())?;
        }
        Ok(NonZeroCoeffQuantState::from_accumulator(quant_state))
    }
}

#[cfg(test)]
#[path = "fsc_quant_pass_tests.rs"]
mod tests;
