// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient sign pass.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::idtx_sign_ctx;
use super::super::cdf::{CoeffCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::fsc_level_pass::CoeffFscLevelPassConfig;
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
    #[error("coefficient FSC sign scan walk was built for another block geometry")]
    WalkBlockMismatch,
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
    if !level_walk.matches_block(block) {
        return Err(CoeffFscSignPassError::WalkBlockMismatch);
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
) -> Result<bool, CoeffFscSignPassError> {
    let sign = match input.source {
        CoeffFscSignReadSource::IdtxSign { selector } => {
            cdfs.read_coeff_symbol(selector, symbols)? != 0
        }
        CoeffFscSignReadSource::None => false,
    };
    Ok(sign)
}

pub(crate) const fn quant_sign_value(sign: bool) -> i8 {
    if sign { -1 } else { 1 }
}
