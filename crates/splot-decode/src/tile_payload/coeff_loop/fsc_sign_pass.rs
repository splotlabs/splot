// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient sign pass.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::idtx_sign_ctx;
use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEobSymbolRead;
use super::fsc_level_pass::{
    CoeffFscLevelPassConfig, CoeffFscLevelRead, NonZeroCoeffFscLevelPass, expected_fsc_entry_pos,
};
use super::scan_walk::{CoeffScanEntry, FscCoeffScanWalk};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffFscSignReadSource {
    None,
    IdtxSign { selector: CoeffCdfSelector },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscSignReadInput {
    pub(crate) entry: CoeffScanEntry,
    pub(crate) level: u32,
    pub(crate) source: CoeffFscSignReadSource,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffFscSignReadSymbol {
    None,
    IdtxSign { symbol: u8 },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscSignRead {
    entry: CoeffScanEntry,
    level: u32,
    symbol: CoeffFscSignReadSymbol,
    sign: bool,
}

impl CoeffFscSignRead {
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }
    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }
    #[must_use]
    pub(crate) const fn symbol(self) -> CoeffFscSignReadSymbol {
        self.symbol
    }
    #[must_use]
    pub(crate) const fn sign(self) -> bool {
        self.sign
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffFscSignPass {
    eob_read: NonZeroCoeffEobSymbolRead,
    level_walk: FscCoeffScanWalk,
    level_reads: Vec<CoeffFscLevelRead>,
    sign_entries: Vec<CoeffScanEntry>,
    sign_inputs: Vec<CoeffFscSignReadInput>,
    sign_reads: Vec<CoeffFscSignRead>,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffFscSignPass {
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
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
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
    #[error("coefficient FSC sign segEob {seg_eob} exceeds scan length {scan_len}")]
    ScanTooShort { seg_eob: usize, scan_len: usize },
    #[error(
        "coefficient FSC sign scan entry {scan_index} has position {pos}, outside coefficient count {coeff_count}"
    )]
    ScanPositionOutOfRange {
        scan_index: usize,
        pos: usize,
        coeff_count: usize,
    },
    #[error(
        "coefficient FSC sign scan entry {entry:?} maps to position {expected_pos}, not {actual_pos}"
    )]
    ScanEntryPositionMismatch {
        entry: CoeffScanEntry,
        expected_pos: usize,
        actual_pos: usize,
    },
    #[error("coefficient FSC sign allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    #[error("coefficient FSC sign symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
    #[error("coefficient FSC sign state error: {0}")]
    State(#[from] TileCoeffStateError),
}

pub(crate) fn apply_nonzero_coeff_fsc_sign_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    level_pass: NonZeroCoeffFscLevelPass,
    scan: &[u16],
    config: CoeffFscLevelPassConfig,
) -> Result<NonZeroCoeffFscSignPass, CoeffFscSignPassError> {
    let (eob_read, level_walk, _level_inputs, level_reads, mut block) = level_pass.into_parts();
    let sign_entries = checked_fsc_sign_entries(&block, &level_walk, scan, config)?;
    let mut sign_inputs = Vec::new();
    let mut sign_reads = Vec::new();
    sign_inputs.try_reserve(sign_entries.len())?;
    sign_reads.try_reserve(sign_entries.len())?;

    for entry in sign_entries.iter().copied() {
        let input = derive_fsc_sign_input(entry, &block, config)?;
        let read = read_fsc_sign_symbol(cdfs, symbols, input)?;
        if input.level != 0 {
            block.set_quant_sign(entry.row(), entry.col(), quant_sign_value(read.sign()))?;
        }
        sign_inputs.push(input);
        sign_reads.push(read);
    }

    Ok(NonZeroCoeffFscSignPass {
        eob_read,
        level_walk,
        level_reads,
        sign_entries,
        sign_inputs,
        sign_reads,
        block,
    })
}

pub(super) fn checked_fsc_sign_entries(
    block: &TransformCoeffBlockState,
    level_walk: &FscCoeffScanWalk,
    scan: &[u16],
    config: CoeffFscLevelPassConfig,
) -> Result<Vec<CoeffScanEntry>, CoeffFscSignPassError> {
    if block.width() != config.tx_width || block.height() != config.tx_height {
        return Err(CoeffFscSignPassError::BlockGeometryMismatch {
            block_width: block.width(),
            block_height: block.height(),
            config_width: config.tx_width,
            config_height: config.tx_height,
        });
    }
    let entries = fsc_sign_entries(block, level_walk.bob(), level_walk.seg_eob(), scan)?;
    for entry in entries.iter().copied() {
        preflight_entry(block, entry)?;
    }
    Ok(entries)
}

pub(super) fn fsc_sign_entries(
    block: &TransformCoeffBlockState,
    bob: usize,
    seg_eob: usize,
    scan: &[u16],
) -> Result<Vec<CoeffScanEntry>, CoeffFscSignPassError> {
    if seg_eob > scan.len() {
        return Err(CoeffFscSignPassError::ScanTooShort {
            seg_eob,
            scan_len: scan.len(),
        });
    }
    let width = block.width();
    let coeff_count = block.level().len();
    let mut entries = Vec::new();
    entries.try_reserve(seg_eob.saturating_sub(bob))?;
    for (scan_index, &scan_pos) in scan.iter().enumerate().take(seg_eob).skip(bob) {
        let pos = usize::from(scan_pos);
        if pos >= coeff_count {
            return Err(CoeffFscSignPassError::ScanPositionOutOfRange {
                scan_index,
                pos,
                coeff_count,
            });
        }
        entries.push(CoeffScanEntry::new(
            scan_index,
            pos,
            pos / width,
            pos % width,
        ));
    }
    Ok(entries)
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

pub(super) fn derive_fsc_sign_input(
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
                    config.tx_width,
                ),
            },
        }
    };
    Ok(CoeffFscSignReadInput {
        entry,
        level,
        source,
    })
}

pub(super) fn read_fsc_sign_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffFscSignReadInput,
) -> Result<CoeffFscSignRead, CoeffFscSignPassError> {
    let (symbol, sign) = match input.source {
        CoeffFscSignReadSource::IdtxSign { selector } => {
            let symbol = cdfs
                .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
                .get();
            (CoeffFscSignReadSymbol::IdtxSign { symbol }, symbol != 0)
        }
        CoeffFscSignReadSource::None => (CoeffFscSignReadSymbol::None, false),
    };
    Ok(CoeffFscSignRead {
        entry: input.entry,
        level: input.level,
        symbol,
        sign,
    })
}

pub(super) const fn quant_sign_value(sign: bool) -> i32 {
    if sign { -1 } else { 1 }
}
