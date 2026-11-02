// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient level first pass.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::{
    coeff_base_bob_ctx, coeff_base_idtx_ctx, coeff_br_idtx_ctx,
};
use super::super::cdf::{CoeffCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEob;
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::NUM_BASE_LEVELS;
use super::scan_walk::{CoeffScanEntry, FscCoeffScanWalk};

const TX_16X16_CONTEXT: usize = 2;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub(crate) struct CoeffFscLevelPassConfig {
    pub(crate) tx_size_ctx: usize,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
}

impl CoeffFscLevelPassConfig {
    pub(crate) fn fsc_tx_size_ctx(self) -> usize {
        self.tx_size_ctx.min(TX_16X16_CONTEXT)
    }
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
    #[error("coefficient FSC level scan walk was built for another block geometry")]
    WalkBlockMismatch,
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

    let tx_size_ctx = config.fsc_tx_size_ctx();
    let mut entries = walk.entries();
    if let Some(entry) = entries.next() {
        let selector = CoeffCdfSelector::BaseBob {
            tx_size_ctx,
            ctx: coeff_base_bob_ctx(walk.bob(), walk.seg_eob()),
        };
        let level = u32::from(cdfs.read_coeff_symbol(selector, symbols)?) + 1;
        finish_fsc_level(cdfs, symbols, entry, level, tx_size_ctx, &mut block)?;
    }
    for entry in entries {
        let selector = CoeffCdfSelector::BaseIdtx {
            tx_size_ctx,
            ctx: coeff_base_idtx_ctx(
                block.level(),
                entry.row(),
                entry.col(),
                block.level_stride(),
            ),
        };
        let level = u32::from(cdfs.read_coeff_symbol(selector, symbols)?);
        if level != 0 {
            finish_fsc_level(cdfs, symbols, entry, level, tx_size_ctx, &mut block)?;
        }
    }

    Ok(NonZeroCoeffFscLevelPass { walk, block })
}

#[inline]
fn finish_fsc_level(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    entry: CoeffScanEntry,
    mut level: u32,
    tx_size_ctx: usize,
    block: &mut TransformCoeffBlockState,
) -> Result<(), CoeffFscLevelPassError> {
    if level > NUM_BASE_LEVELS {
        let ctx = coeff_br_idtx_ctx(
            block.level(),
            entry.row(),
            entry.col(),
            block.level_stride(),
        );
        let selector = CoeffCdfSelector::BrIdtx { tx_size_ctx, ctx };
        level += u32::from(cdfs.read_coeff_symbol(selector, symbols)?);
    }
    block.set_level(entry.row(), entry.col(), level)?;
    Ok(())
}

fn preflight_pass(
    eob_read: NonZeroCoeffEob,
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
    let eob = eob_read.eob();
    if eob != walk.len() {
        return Err(CoeffFscLevelPassError::ScanEntryCountMismatch {
            eob,
            entries: walk.len(),
        });
    }
    if !walk.matches_block(block) {
        return Err(CoeffFscLevelPassError::WalkBlockMismatch);
    }
    Ok(())
}
