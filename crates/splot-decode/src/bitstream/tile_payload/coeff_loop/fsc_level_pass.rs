// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient level first pass.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::{
    coeff_base_bob_ctx, coeff_base_idtx_ctx, coeff_br_idtx_ctx,
};
use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEobSymbolRead;
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::NUM_BASE_LEVELS;
use super::scan_walk::{CoeffScanEntry, FscCoeffScanWalk};

const TX_16X16_CONTEXT: usize = 2;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscLevelPassConfig {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) tx_size_ctx: usize,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
}

impl CoeffFscLevelPassConfig {
    pub(crate) fn fsc_tx_size_ctx(self) -> usize {
        self.tx_size_ctx.min(TX_16X16_CONTEXT)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffFscLevelSymbolSource {
    BaseBob { selector: CoeffCdfSelector },
    BaseIdtx { selector: CoeffCdfSelector },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscLevelReadInput {
    pub(crate) base: CoeffFscLevelSymbolSource,
    pub(crate) base_range: CoeffCdfSelector,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffFscLevelPass {
    walk: FscCoeffScanWalk,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffFscLevelPass {
    pub(crate) fn into_parts(self) -> (FscCoeffScanWalk, TransformCoeffBlockState) {
        (self.walk, self.block)
    }
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscLevelPassError {
    #[error("coefficient FSC level entries {entries} do not match decoded eob {eob}")]
    ScanEntryCountMismatch { eob: usize, entries: usize },
    #[error(
        "coefficient FSC level config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
    )]
    BlockGeometryMismatch {
        block_width: usize,
        block_height: usize,
        config_width: usize,
        config_height: usize,
    },
    #[error(
        "coefficient FSC level scan entry {entry:?} maps to position {expected_pos}, not {actual_pos}"
    )]
    ScanEntryPositionMismatch {
        entry: CoeffScanEntry,
        expected_pos: usize,
        actual_pos: usize,
    },
    #[error("coefficient FSC level symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
    #[error("coefficient FSC level state error: {0}")]
    State(#[from] TileCoeffStateError),
}

pub(crate) fn apply_nonzero_coeff_fsc_level_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    start: NonZeroCoeffBlockStart,
    walk: FscCoeffScanWalk,
    config: CoeffFscLevelPassConfig,
) -> Result<NonZeroCoeffFscLevelPass, CoeffFscLevelPassError> {
    let (eob_read, mut block) = start.into_parts();
    preflight_pass(eob_read, &block, &walk, config)?;
    block.ensure_quant_sign()?;

    for (index, entry) in walk.entries().enumerate() {
        let input = derive_fsc_level_input(index, entry, &walk, &block, config);
        let level = read_fsc_level_symbol(cdfs, symbols, input)?;
        block.set_level(entry.row(), entry.col(), level)?;
    }

    Ok(NonZeroCoeffFscLevelPass { walk, block })
}

fn preflight_pass(
    eob_read: NonZeroCoeffEobSymbolRead,
    block: &TransformCoeffBlockState,
    walk: &FscCoeffScanWalk,
    config: CoeffFscLevelPassConfig,
) -> Result<(), CoeffFscLevelPassError> {
    if block.width() != config.tx_width || block.height() != config.tx_height {
        return Err(CoeffFscLevelPassError::BlockGeometryMismatch {
            block_width: block.width(),
            block_height: block.height(),
            config_width: config.tx_width,
            config_height: config.tx_height,
        });
    }
    let eob = eob_read.eob().eob();
    if eob != walk.len() {
        return Err(CoeffFscLevelPassError::ScanEntryCountMismatch {
            eob,
            entries: walk.len(),
        });
    }
    for entry in walk.entries() {
        block.level_at(entry.row(), entry.col())?;
        block.quant_at(entry.pos())?;
        let expected_pos = expected_fsc_entry_pos(block, entry)?;
        if expected_pos != entry.pos() {
            return Err(CoeffFscLevelPassError::ScanEntryPositionMismatch {
                entry,
                expected_pos,
                actual_pos: entry.pos(),
            });
        }
    }
    Ok(())
}

pub(crate) fn expected_fsc_entry_pos(
    block: &TransformCoeffBlockState,
    entry: CoeffScanEntry,
) -> Result<usize, TileCoeffStateError> {
    entry
        .row()
        .checked_mul(block.width())
        .and_then(|base| base.checked_add(entry.col()))
        .ok_or(TileCoeffStateError::ArithmeticOverflow {
            operation: "row * width + col",
            left: entry.row(),
            right: block.width(),
        })
}

fn derive_fsc_level_input(
    index: usize,
    entry: CoeffScanEntry,
    walk: &FscCoeffScanWalk,
    block: &TransformCoeffBlockState,
    config: CoeffFscLevelPassConfig,
) -> CoeffFscLevelReadInput {
    let tx_size_ctx = config.fsc_tx_size_ctx();
    let base = if index == 0 {
        CoeffFscLevelSymbolSource::BaseBob {
            selector: CoeffCdfSelector::BaseBob {
                coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
                tx_size_ctx,
                ctx: coeff_base_bob_ctx(walk.bob(), walk.seg_eob()),
            },
        }
    } else {
        CoeffFscLevelSymbolSource::BaseIdtx {
            selector: CoeffCdfSelector::BaseIdtx {
                coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
                tx_size_ctx,
                ctx: coeff_base_idtx_ctx(
                    block.level(),
                    entry.row(),
                    entry.col(),
                    block.level_stride(),
                ),
            },
        }
    };
    let br_ctx = coeff_br_idtx_ctx(
        block.level(),
        entry.row(),
        entry.col(),
        block.level_stride(),
    );
    CoeffFscLevelReadInput {
        base,
        base_range: CoeffCdfSelector::BrIdtx {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size_ctx,
            ctx: br_ctx,
        },
    }
}

fn read_fsc_level_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscLevelReadInput,
) -> Result<u32, CoeffFscLevelPassError> {
    let mut read_symbol = |selector| -> Result<u8, CoeffFscLevelPassError> {
        Ok(cdfs
            .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
            .get())
    };
    let (selector, base_offset) = match input.base {
        CoeffFscLevelSymbolSource::BaseBob { selector } => (selector, 1),
        CoeffFscLevelSymbolSource::BaseIdtx { selector } => (selector, 0),
    };
    let base_symbol = read_symbol(selector)?;
    let mut level = u32::from(base_symbol) + base_offset;
    if level > NUM_BASE_LEVELS {
        let symbol = read_symbol(input.base_range)?;
        level += u32::from(symbol);
    }
    Ok(level)
}
