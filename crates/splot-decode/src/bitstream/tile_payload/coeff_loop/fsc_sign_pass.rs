// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient sign pass.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::idtx_sign_ctx;
use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::fsc_level_pass::{CoeffFscLevelPassConfig, expected_fsc_entry_pos};
use super::scan_walk::{CoeffScanEntry, FscCoeffScanWalk};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffFscSignReadSource {
    None,
    IdtxSign { selector: CoeffCdfSelector },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscSignReadInput {
    pub(crate) level: u32,
    pub(crate) source: CoeffFscSignReadSource,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscSignRead {
    sign: bool,
}

impl CoeffFscSignRead {
    #[must_use]
    pub(crate) const fn sign(self) -> bool {
        self.sign
    }
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscSignPassError {
    #[error(
        "coefficient FSC sign config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
    )]
    BlockGeometryMismatch {
        block_width: usize,
        block_height: usize,
        config_width: usize,
        config_height: usize,
    },
    #[error(
        "coefficient FSC sign scan entry {entry:?} maps to position {expected_pos}, not {actual_pos}"
    )]
    ScanEntryPositionMismatch {
        entry: CoeffScanEntry,
        expected_pos: usize,
        actual_pos: usize,
    },
    #[error("coefficient FSC sign symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
    #[error("coefficient FSC sign state error: {0}")]
    State(#[from] TileCoeffStateError),
}

pub(crate) fn checked_fsc_sign_walk(
    block: &TransformCoeffBlockState,
    level_walk: &FscCoeffScanWalk,
    config: CoeffFscLevelPassConfig,
) -> Result<(), CoeffFscSignPassError> {
    if block.width() != config.tx_width || block.height() != config.tx_height {
        return Err(CoeffFscSignPassError::BlockGeometryMismatch {
            block_width: block.width(),
            block_height: block.height(),
            config_width: config.tx_width,
            config_height: config.tx_height,
        });
    }
    for entry in level_walk.entries() {
        preflight_entry(block, entry)?;
    }
    Ok(())
}

fn preflight_entry(
    block: &TransformCoeffBlockState,
    entry: CoeffScanEntry,
) -> Result<(), CoeffFscSignPassError> {
    block.level_at(entry.row(), entry.col())?;
    block.quant_sign_at(entry.row(), entry.col())?;
    block.quant_at(entry.pos())?;
    let expected_pos = expected_fsc_entry_pos(block, entry)?;
    if expected_pos != entry.pos() {
        return Err(CoeffFscSignPassError::ScanEntryPositionMismatch {
            entry,
            expected_pos,
            actual_pos: entry.pos(),
        });
    }
    Ok(())
}

pub(crate) fn derive_fsc_sign_input(
    entry: CoeffScanEntry,
    block: &TransformCoeffBlockState,
    config: CoeffFscLevelPassConfig,
) -> Result<CoeffFscSignReadInput, CoeffFscSignPassError> {
    let level = block.level_at(entry.row(), entry.col())?;
    let source = if level == 0 {
        CoeffFscSignReadSource::None
    } else {
        CoeffFscSignReadSource::IdtxSign {
            selector: CoeffCdfSelector::IdtxSign {
                coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
                tx_size_ctx: config.fsc_tx_size_ctx(),
                ctx: idtx_sign_ctx(
                    block.quant_sign(),
                    block.level(),
                    entry.row(),
                    entry.col(),
                    block.level_stride(),
                ),
            },
        }
    };
    Ok(CoeffFscSignReadInput { level, source })
}

pub(crate) fn read_fsc_sign_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscSignReadInput,
) -> Result<CoeffFscSignRead, CoeffFscSignPassError> {
    let sign = match input.source {
        CoeffFscSignReadSource::IdtxSign { selector } => {
            let symbol = cdfs
                .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
                .get();
            symbol != 0
        }
        CoeffFscSignReadSource::None => false,
    };
    Ok(CoeffFscSignRead { sign })
}

pub(crate) const fn quant_sign_value(sign: bool) -> i8 {
    if sign { -1 } else { 1 }
}
