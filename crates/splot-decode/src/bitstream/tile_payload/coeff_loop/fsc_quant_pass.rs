// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient quant pass.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    ADJUSTED_TX_SIZE, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR, TX_SIZE_SQR_UP, TX_WIDTH,
    TX_WIDTH_LOG2,
};

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{
    CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};
use super::branch::{
    CoeffBlockEobBranch, CoeffBlockEobBranchInput, NonZeroCoeffBlockStart,
    NonZeroCoeffBlockStartInput, read_coeff_block_eob_branch,
};
use super::fsc_level_pass::{
    CoeffFscLevelPassConfig, CoeffFscLevelPassError, CoeffFscLevelRead, NonZeroCoeffFscLevelPass,
    apply_nonzero_coeff_fsc_level_pass,
};
use super::fsc_sign_pass::{
    CoeffFscSignPassError, CoeffFscSignRead, CoeffFscSignReadInput, checked_fsc_sign_entries,
    derive_fsc_sign_input, quant_sign_value, read_fsc_sign_symbol,
};
use super::max_level::{COEFF_BASE_RANGE, CoeffTransformClass, NUM_BASE_LEVELS};
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, CoeffQuantStateWrite,
    CoeffQuantStateWriteError, NonZeroCoeffQuantState,
};
use super::read_quant::{
    CoeffReadQuant, CoeffReadQuantConfig, CoeffReadQuantError, CoeffReadQuantInput,
    CoeffReadQuantState,
};
use super::scan_walk::{
    CoeffScanEntry, CoeffScanOrderError, FscCoeffScanWalk, derive_coeff_scan_order,
    walk_fsc_coeff_scan,
};
use super::{
    AllZeroCoeffBlockInput, CoeffLoopContextError, NonZeroCoeffEobContextInput,
    NonZeroCoeffEobSymbolRead,
};

const FSC_MAX_LEVEL: u32 = NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1;

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoeffFscBranchTxSizeDimensions {
    tx_width: usize,
    tx_height: usize,
    tx_width_log2: usize,
    tx_height_log2: usize,
}

#[derive(Clone, Copy)]
struct CoeffFscBranchTxSizeTables<'a> {
    adjusted_tx_size: &'a [i32],
    tx_size_sqr: &'a [i32],
    tx_size_sqr_up: &'a [i32],
    tx_width: &'a [i32],
    tx_height: &'a [i32],
    tx_width_log2: &'a [i32],
    tx_height_log2: &'a [i32],
}

#[derive(Clone, Copy)]
struct CoeffFscBranchTxSizeFacts {
    raw_dimensions: CoeffFscBranchTxSizeDimensions,
    level_config: CoeffFscLevelPassConfig,
    context: CoeffFscContextCommitConfig,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct CoeffFscBranchTestDimensionTables<'a> {
    pub(crate) tx_width: &'a [i32],
    pub(crate) tx_height: &'a [i32],
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct CoeffFscBranchTestTxSizeTables<'a> {
    pub(crate) adjusted_tx_size: &'a [i32],
    pub(crate) tx_size_sqr: &'a [i32],
    pub(crate) tx_size_sqr_up: &'a [i32],
    pub(crate) tx_width: &'a [i32],
    pub(crate) tx_height: &'a [i32],
    pub(crate) tx_width_log2: &'a [i32],
    pub(crate) tx_height_log2: &'a [i32],
}

const DEFAULT_TX_SIZE_TABLES: CoeffFscBranchTxSizeTables<'static> = CoeffFscBranchTxSizeTables {
    adjusted_tx_size: &ADJUSTED_TX_SIZE,
    tx_size_sqr: &TX_SIZE_SQR,
    tx_size_sqr_up: &TX_SIZE_SQR_UP,
    tx_width: &TX_WIDTH,
    tx_height: &TX_HEIGHT,
    tx_width_log2: &TX_WIDTH_LOG2,
    tx_height_log2: &TX_HEIGHT_LOG2,
};
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscContextCommitConfig {
    pub(crate) plane: usize,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
}
pub(crate) enum CoeffFscBranchInput<'a> {
    AllZero(AllZeroCoeffBlockInput),
    NonZero(CoeffFscBranchNonZeroInput<'a>),
}
pub(crate) struct CoeffFscBranchNonZeroInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStartInput,
    pub(crate) seg_eob: usize,
    pub(crate) scan: &'a [u16],
    pub(crate) level_config: CoeffFscLevelPassConfig,
    pub(crate) context: CoeffFscContextCommitConfig,
}
pub(crate) enum CoeffFscBranchSegEobInput<'a> {
    AllZero(AllZeroCoeffBlockInput),
    NonZero(CoeffFscBranchSegEobNonZeroInput<'a>),
}
pub(crate) struct CoeffFscBranchSegEobNonZeroInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStartInput,
    pub(crate) scan: &'a [u16],
    pub(crate) level_config: CoeffFscLevelPassConfig,
    pub(crate) context: CoeffFscContextCommitConfig,
}
pub(crate) enum CoeffFscBranchTxSizeInput {
    AllZero(AllZeroCoeffBlockInput),
    NonZero(CoeffFscBranchTxSizeNonZeroInput),
}
pub(crate) struct CoeffFscBranchTxSizeNonZeroInput {
    pub(crate) block: AllZeroCoeffBlockInput,
    pub(crate) tx_size: usize,
    pub(crate) plane_tx_type: usize,
    pub(crate) is_inter: bool,
    pub(crate) coeff_cdf_q_ctx: usize,
}
pub(crate) struct CoeffFscStagedTxSizeNonZeroInput {
    pub(crate) block: AllZeroCoeffBlockInput,
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) tx_size: usize,
    pub(crate) plane_tx_type: usize,
    pub(crate) coeff_cdf_q_ctx: usize,
}
pub(crate) enum CoeffFscBranchScanOrderInput {
    AllZero(AllZeroCoeffBlockInput),
    NonZero(CoeffFscBranchScanOrderNonZeroInput),
}
pub(crate) struct CoeffFscBranchScanOrderNonZeroInput {
    pub(crate) start: NonZeroCoeffBlockStartInput,
    pub(crate) tx_size: usize,
    pub(crate) plane_tx_type: usize,
    pub(crate) level_config: CoeffFscLevelPassConfig,
    pub(crate) context: CoeffFscContextCommitConfig,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscBranch {
    pass: NonZeroCoeffFscQuantPass,
}

impl CoeffFscBranch {
    #[must_use]
    pub(crate) const fn pass(&self) -> &NonZeroCoeffFscQuantPass {
        &self.pass
    }
}

impl NonZeroCoeffFscQuantPass {
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }
    #[must_use]
    pub(crate) const fn level_walk(&self) -> &FscCoeffScanWalk {
        &self.level_walk
    }
    #[must_use]
    pub(crate) fn level_reads(&self) -> &[CoeffFscLevelRead] {
        &self.level_reads
    }
    #[must_use]
    pub(crate) fn sign_entries(&self) -> &[CoeffScanEntry] {
        &self.sign_entries
    }
    #[must_use]
    pub(crate) fn sign_inputs(&self) -> &[CoeffFscSignReadInput] {
        &self.sign_inputs
    }
    #[must_use]
    pub(crate) fn sign_reads(&self) -> &[CoeffFscSignRead] {
        &self.sign_reads
    }
    #[must_use]
    pub(crate) fn read_quants(&self) -> &[CoeffReadQuant] {
        &self.read_quants
    }
    #[must_use]
    pub(crate) const fn quant_state(&self) -> &NonZeroCoeffQuantState {
        &self.quant_state
    }
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
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
    #[error("coefficient FSC quant context commit requires luma plane, got plane {plane}")]
    NonLumaPlane { plane: usize },
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
    #[error("coefficient FSC branch does not support all-zero routing")]
    AllZero,
    #[error("coefficient FSC branch requires luma plane, got plane {plane}")]
    NonLumaPlane { plane: usize },
    #[error("coefficient FSC branch handoff failed: {0}")]
    Branch(#[from] CoeffLoopContextError),
    #[error("coefficient FSC branch level pass failed: {0}")]
    Level(#[from] CoeffFscLevelPassError),
    #[error("coefficient FSC branch quant pass failed: {0}")]
    Quant(#[from] CoeffFscQuantPassError),
    #[error("coefficient FSC branch returned unexpected {actual} arm while expecting {expected}")]
    UnexpectedBranch {
        expected: &'static str,
        actual: &'static str,
    },
}

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

pub(crate) fn apply_coeff_fsc_branch_from_scan_extent(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchSegEobInput<'_>,
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    let input = match input {
        CoeffFscBranchSegEobInput::AllZero(input) => CoeffFscBranchInput::AllZero(input),
        CoeffFscBranchSegEobInput::NonZero(input) => {
            CoeffFscBranchInput::NonZero(CoeffFscBranchNonZeroInput {
                start: input.start,
                seg_eob: input.scan.len(),
                scan: input.scan,
                level_config: input.level_config,
                context: input.context,
            })
        }
    };
    apply_coeff_fsc_branch(state, cdfs, symbols, input)
}

pub(crate) fn apply_coeff_fsc_branch_from_tx_size(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchTxSizeInput,
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    apply_coeff_fsc_branch_from_tx_size_with_tables(
        state,
        cdfs,
        symbols,
        input,
        DEFAULT_TX_SIZE_TABLES,
    )
}

pub(crate) fn apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscStagedTxSizeNonZeroInput,
) -> Result<NonZeroCoeffFscQuantPass, CoeffFscBranchError> {
    if input.block.plane != 0 {
        return Err(CoeffFscBranchError::NonLumaPlane {
            plane: input.block.plane,
        });
    }

    let facts = fsc_branch_tx_size_facts(
        DEFAULT_TX_SIZE_TABLES,
        input.block,
        input.tx_size,
        input.coeff_cdf_q_ctx,
    )?;
    let scan = fsc_branch_scan_order(&TX_WIDTH, &TX_HEIGHT, input.tx_size, input.plane_tx_type)?;
    let walk = walk_fsc_coeff_scan(&input.start, scan.len(), &scan)?;
    let level_pass =
        apply_nonzero_coeff_fsc_level_pass(cdfs, symbols, input.start, walk, facts.level_config)?;
    apply_nonzero_coeff_fsc_quant_pass_with_context_commit(
        state,
        cdfs,
        symbols,
        level_pass,
        &scan,
        facts.level_config,
        facts.context,
    )
    .map_err(CoeffFscBranchError::from)
}

#[cfg(test)]
pub(crate) fn apply_coeff_fsc_branch_from_tx_size_with_test_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchTxSizeInput,
    tables: CoeffFscBranchTestTxSizeTables<'_>,
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    apply_coeff_fsc_branch_from_tx_size_with_tables(
        state,
        cdfs,
        symbols,
        input,
        CoeffFscBranchTxSizeTables {
            adjusted_tx_size: tables.adjusted_tx_size,
            tx_size_sqr: tables.tx_size_sqr,
            tx_size_sqr_up: tables.tx_size_sqr_up,
            tx_width: tables.tx_width,
            tx_height: tables.tx_height,
            tx_width_log2: tables.tx_width_log2,
            tx_height_log2: tables.tx_height_log2,
        },
    )
}

fn apply_coeff_fsc_branch_from_tx_size_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchTxSizeInput,
    tables: CoeffFscBranchTxSizeTables<'_>,
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    match input {
        CoeffFscBranchTxSizeInput::AllZero(input) => apply_coeff_fsc_branch_from_scan_order(
            state,
            cdfs,
            symbols,
            CoeffFscBranchScanOrderInput::AllZero(input),
        ),
        CoeffFscBranchTxSizeInput::NonZero(input) => {
            if input.block.plane != 0 {
                return Err(CoeffFscBranchError::NonLumaPlane {
                    plane: input.block.plane,
                });
            }

            let facts = fsc_branch_tx_size_facts(
                tables,
                input.block,
                input.tx_size,
                input.coeff_cdf_q_ctx,
            )?;
            apply_coeff_fsc_branch_from_scan_order(
                state,
                cdfs,
                symbols,
                CoeffFscBranchScanOrderInput::NonZero(CoeffFscBranchScanOrderNonZeroInput {
                    start: NonZeroCoeffBlockStartInput {
                        block: input.block,
                        eob: NonZeroCoeffEobContextInput {
                            plane: input.block.plane,
                            is_inter: input.is_inter,
                            tx_width_log2: facts.raw_dimensions.tx_width_log2,
                            tx_height_log2: facts.raw_dimensions.tx_height_log2,
                            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                        },
                    },
                    tx_size: input.tx_size,
                    plane_tx_type: input.plane_tx_type,
                    level_config: facts.level_config,
                    context: facts.context,
                }),
            )
        }
    }
}

pub(crate) fn apply_coeff_fsc_branch_from_scan_order(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchScanOrderInput,
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    apply_coeff_fsc_branch_from_scan_order_with_tables(
        state, cdfs, symbols, input, &TX_WIDTH, &TX_HEIGHT,
    )
}

#[cfg(test)]
pub(crate) fn apply_coeff_fsc_branch_from_scan_order_with_test_dimension_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchScanOrderInput,
    tables: CoeffFscBranchTestDimensionTables<'_>,
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    apply_coeff_fsc_branch_from_scan_order_with_tables(
        state,
        cdfs,
        symbols,
        input,
        tables.tx_width,
        tables.tx_height,
    )
}

fn apply_coeff_fsc_branch_from_scan_order_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscBranchScanOrderInput,
    tx_width_table: &[i32],
    tx_height_table: &[i32],
) -> Result<CoeffFscBranch, CoeffFscBranchError> {
    match input {
        CoeffFscBranchScanOrderInput::AllZero(input) => apply_coeff_fsc_branch_from_scan_extent(
            state,
            cdfs,
            symbols,
            CoeffFscBranchSegEobInput::AllZero(input),
        ),
        CoeffFscBranchScanOrderInput::NonZero(input) => {
            let scan = fsc_branch_scan_order(
                tx_width_table,
                tx_height_table,
                input.tx_size,
                input.plane_tx_type,
            )?;
            apply_coeff_fsc_branch_from_scan_extent(
                state,
                cdfs,
                symbols,
                CoeffFscBranchSegEobInput::NonZero(CoeffFscBranchSegEobNonZeroInput {
                    start: input.start,
                    scan: &scan,
                    level_config: input.level_config,
                    context: input.context,
                }),
            )
        }
    }
}

fn fsc_branch_scan_order(
    tx_width_table: &[i32],
    tx_height_table: &[i32],
    tx_size: usize,
    plane_tx_type: usize,
) -> Result<Vec<u16>, CoeffFscBranchError> {
    let tx_width = tx_size_table_usize(tx_width_table, "Tx_Width", tx_size)?;
    let tx_height = tx_size_table_usize(tx_height_table, "Tx_Height", tx_size)?;
    Ok(derive_coeff_scan_order(
        tx_width,
        tx_height,
        CoeffTransformClass::from_plane_tx_type(plane_tx_type),
    )?)
}

fn fsc_branch_tx_size_facts(
    tables: CoeffFscBranchTxSizeTables<'_>,
    block: AllZeroCoeffBlockInput,
    tx_size: usize,
    coeff_cdf_q_ctx: usize,
) -> Result<CoeffFscBranchTxSizeFacts, CoeffFscBranchError> {
    let raw_dimensions = fsc_tx_size_dimensions(tables, tx_size)?;
    validate_fsc_block_geometry(block, tx_size, raw_dimensions)?;
    let adjusted_dimensions = fsc_adjusted_tx_size_dimensions(tables, tx_size)?;
    Ok(CoeffFscBranchTxSizeFacts {
        raw_dimensions,
        level_config: CoeffFscLevelPassConfig {
            coeff_cdf_q_ctx,
            tx_size_ctx: fsc_tx_size_context(tables, tx_size)?,
            tx_width: adjusted_dimensions.tx_width,
            tx_height: adjusted_dimensions.tx_height,
        },
        context: CoeffFscContextCommitConfig {
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
    raw_dimensions: CoeffFscBranchTxSizeDimensions,
) -> Result<(), CoeffFscBranchError> {
    let expected_w4 = raw_dimensions.tx_width >> 2;
    let expected_h4 = raw_dimensions.tx_height >> 2;
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

fn fsc_adjusted_tx_size_dimensions(
    tables: CoeffFscBranchTxSizeTables<'_>,
    tx_size: usize,
) -> Result<CoeffFscBranchTxSizeDimensions, CoeffFscBranchError> {
    let adjusted_tx_size =
        tx_size_table_tx_size(tables, tables.adjusted_tx_size, "Adjusted_Tx_Size", tx_size)?;
    fsc_tx_size_dimensions(tables, adjusted_tx_size)
}

fn fsc_tx_size_context(
    tables: CoeffFscBranchTxSizeTables<'_>,
    tx_size: usize,
) -> Result<usize, CoeffFscBranchError> {
    let tx_size_sqr = tx_size_table_tx_size(tables, tables.tx_size_sqr, "Tx_Size_Sqr", tx_size)?;
    let tx_size_sqr_up =
        tx_size_table_tx_size(tables, tables.tx_size_sqr_up, "Tx_Size_Sqr_Up", tx_size)?;
    tx_size_sqr
        .checked_add(tx_size_sqr_up)
        .and_then(|sum| sum.checked_add(1))
        .map(|sum| sum >> 1)
        .ok_or(CoeffFscBranchError::TransformSizeContextOverflow { tx_size })
}

fn fsc_tx_size_dimensions(
    tables: CoeffFscBranchTxSizeTables<'_>,
    tx_size: usize,
) -> Result<CoeffFscBranchTxSizeDimensions, CoeffFscBranchError> {
    Ok(CoeffFscBranchTxSizeDimensions {
        tx_width: tx_size_table_usize(tables.tx_width, "Tx_Width", tx_size)?,
        tx_height: tx_size_table_usize(tables.tx_height, "Tx_Height", tx_size)?,
        tx_width_log2: tx_size_table_usize(tables.tx_width_log2, "Tx_Width_Log2", tx_size)?,
        tx_height_log2: tx_size_table_usize(tables.tx_height_log2, "Tx_Height_Log2", tx_size)?,
    })
}

fn tx_size_table_tx_size(
    tables: CoeffFscBranchTxSizeTables<'_>,
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<usize, CoeffFscBranchError> {
    let value = tx_size_table_usize(table, table_name, tx_size)?;
    if tables.tx_width.get(value).is_none() {
        return Err(CoeffFscBranchError::InvalidTransformSizeTableIndex {
            table: table_name,
            tx_size,
            value,
        });
    }
    Ok(value)
}

fn tx_size_table_usize(
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<usize, CoeffFscBranchError> {
    let value = table
        .get(tx_size)
        .copied()
        .ok_or(CoeffFscBranchError::InvalidTransformSize { tx_size })?;
    usize::try_from(value).map_err(|_| CoeffFscBranchError::InvalidTransformSizeTableValue {
        table: table_name,
        tx_size,
        value,
    })
}

pub(crate) fn apply_nonzero_coeff_fsc_quant_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    level_pass: NonZeroCoeffFscLevelPass,
    scan: &[u16],
    config: CoeffFscLevelPassConfig,
) -> Result<NonZeroCoeffFscQuantPass, CoeffFscQuantPassError> {
    let (eob_read, level_walk, _level_inputs, level_reads, mut block) = level_pass.into_parts();
    let sign_entries = checked_fsc_sign_entries(&block, &level_walk, scan, config)?;
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

    let trace = trace_fsc_block_enabled(context);
    let before = trace.then(|| symbols.checkpoint());
    let pass = apply_nonzero_coeff_fsc_quant_pass(cdfs, symbols, level_pass, scan, config)?;
    if trace {
        trace_fsc_block(context, before, symbols.checkpoint(), &pass);
    }
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

fn trace_fsc_block_enabled(context: CoeffFscContextCommitConfig) -> bool {
    let Some(value) = crate::trace_flags::trace_value!("SPLOT_TRACE_FSC_BLOCK") else {
        return false;
    };
    if value.is_empty() || value == "1" {
        return true;
    }
    let Some((y4, x4)) = value.split_once(',') else {
        return true;
    };
    let Ok(y4) = y4.parse::<usize>() else {
        return false;
    };
    let Ok(x4) = x4.parse::<usize>() else {
        return false;
    };
    context.y4 == y4 && context.x4 == x4
}

fn trace_fsc_block(
    context: CoeffFscContextCommitConfig,
    before: Option<splot_core::symbol::SymbolDecoderCheckpoint>,
    after: splot_core::symbol::SymbolDecoderCheckpoint,
    pass: &NonZeroCoeffFscQuantPass,
) {
    eprintln!(
        "fsc block y4={} x4={} w4={} h4={} before={:?} after={:?} eob={} bob={} seg_eob={} level_reads={} sign_entries={} read_quants={} cul_level={} dc_category={}",
        context.y4,
        context.x4,
        context.w4,
        context.h4,
        before,
        after,
        pass.eob_read().eob().eob(),
        pass.level_walk().bob(),
        pass.level_walk().seg_eob(),
        pass.level_reads().len(),
        pass.sign_entries().len(),
        pass.read_quants().len(),
        pass.quant_state().cul_level(),
        pass.quant_state().dc_category(),
    );
    for (index, ((entry, input), read_quant)) in pass
        .sign_entries()
        .iter()
        .copied()
        .zip(pass.sign_inputs().iter().copied())
        .zip(pass.read_quants().iter().copied())
        .enumerate()
    {
        let sign = pass.sign_reads()[index];
        let write = pass.quant_state().writes()[index];
        eprintln!(
            "fsc entry index={} scan={} pos={} row={} col={} level={} source={:?} sign_symbol={:?} sign={} read_quant={:?} quant={} checkpoint_after_entry_unavailable",
            index,
            entry.scan_index(),
            entry.pos(),
            entry.row(),
            entry.col(),
            input.level,
            input.source,
            sign.symbol(),
            sign.sign(),
            read_quant.path(),
            write.quant(),
        );
    }
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
