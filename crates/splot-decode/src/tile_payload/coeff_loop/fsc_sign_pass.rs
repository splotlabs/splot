// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient sign pass.
//!
//! Feature tracking: `DECODE-COEFF-FSC-SIGN-PASS`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::cdf::coeff_context::idtx_sign_ctx;
use super::super::cdf::{CoeffCdfSelector, TileCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEobSymbolRead;
use super::fsc_level_pass::{CoeffFscLevelPassConfig, CoeffFscLevelRead, NonZeroCoeffFscLevelPass};
use super::scan_walk::{CoeffScanEntry, FscCoeffScanWalk};

/// Sign source selected for one FSC/IDTX second-pass entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffFscSignReadSource {
    /// The local level is zero, so no sign syntax is read.
    None,
    /// Read `idtx_sign` from the derived CDF selector.
    IdtxSign {
        /// Derived CDF selector.
        selector: CoeffCdfSelector,
    },
}

impl CoeffFscSignReadSource {
    const fn selector(self) -> Option<CoeffCdfSelector> {
        match self {
            Self::None => None,
            Self::IdtxSign { selector } => Some(selector),
        }
    }
}

/// Derived read facts for one checked FSC sign entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscSignReadInput {
    /// Checked scan entry.
    pub(crate) entry: CoeffScanEntry,
    /// Local `Level[row][col]` before sign syntax.
    pub(crate) level: u32,
    /// Selected sign source.
    pub(crate) source: CoeffFscSignReadSource,
}

/// Raw FSC/IDTX sign syntax consumed for one entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffFscSignReadSymbol {
    /// No sign syntax was read.
    None,
    /// CDF-backed `idtx_sign` was read.
    IdtxSign {
        /// Raw decoded symbol.
        symbol: u8,
    },
}

/// Decoded FSC/IDTX sign summary for one checked entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscSignRead {
    entry: CoeffScanEntry,
    level: u32,
    symbol: CoeffFscSignReadSymbol,
    sign: bool,
}

impl CoeffFscSignRead {
    /// Checked scan entry associated with this read.
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    /// Local `Level[row][col]` read before sign syntax.
    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }

    /// Raw sign syntax consumed for this entry.
    #[must_use]
    pub(crate) const fn symbol(self) -> CoeffFscSignReadSymbol {
        self.symbol
    }

    /// Boolean sign value used by the later quantization pass.
    #[must_use]
    pub(crate) const fn sign(self) -> bool {
        self.sign
    }
}

/// Result of the FSC/IDTX sign pass.
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

    /// Checked sign entries in forward `0..segEob` order.
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

    /// Local coefficient state after FSC `QuantSign[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Error returned by the FSC/IDTX sign-pass boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscSignPassError {
    /// The caller supplied geometry that does not match the initialized block.
    #[error(
        "coefficient FSC sign config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
    )]
    BlockGeometryMismatch {
        /// Block width.
        block_width: usize,
        /// Block height.
        block_height: usize,
        /// Caller-resolved width.
        config_width: usize,
        /// Caller-resolved height.
        config_height: usize,
    },
    /// The caller supplied a scan table that does not cover `0..segEob`.
    #[error("coefficient FSC sign segEob {seg_eob} exceeds scan length {scan_len}")]
    ScanTooShort {
        /// FSC segment EOB after the branch assigns `eob = segEob`.
        seg_eob: usize,
        /// Caller-provided scan length.
        scan_len: usize,
    },
    /// A scan entry did not fit inside the local coefficient block.
    #[error(
        "coefficient FSC sign scan entry {scan_index} has position {pos}, outside coefficient count {coeff_count}"
    )]
    ScanPositionOutOfRange {
        /// Scan index.
        scan_index: usize,
        /// Raster coefficient position.
        pos: usize,
        /// Local coefficient count.
        coeff_count: usize,
    },
    /// A checked scan entry did not match the local row-major block geometry.
    #[error(
        "coefficient FSC sign scan entry {entry:?} maps to position {expected_pos}, not {actual_pos}"
    )]
    ScanEntryPositionMismatch {
        /// Checked scan entry.
        entry: CoeffScanEntry,
        /// Row-major position derived from `row`, `col`, and block width.
        expected_pos: usize,
        /// Entry position.
        actual_pos: usize,
    },
    /// Allocation for sign entries, inputs, or decoded reads failed.
    #[error("coefficient FSC sign allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// CDF row selection or AV2 section 8.2 symbol decoding failed.
    #[error("coefficient FSC sign symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
    /// The local transform-block state rejected a checked coordinate or position.
    #[error("coefficient FSC sign state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Runs the FSC/IDTX §5.20.7.27 sign pass over `c = 0 .. segEob`.
///
/// The helper follows the second `useFsc` loop through its `idtx_sign` step:
/// for each checked scan entry, it reads `Level[row][col]`, reads `idtx_sign`
/// only when the level is nonzero, and writes local `QuantSign[row][col]` to
/// `-1` or `1` so later `idtx_sign` contexts see prior signs
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`;
/// selector contexts from
/// `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It does not run
/// `read_quant`, write `Quant[]`, update `culLevel` or `dcCategory`, commit tile
/// context lines, dequantize, or reconstruct pixels.
pub(crate) fn apply_nonzero_coeff_fsc_sign_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    level_pass: NonZeroCoeffFscLevelPass,
    scan: &[u16],
    config: CoeffFscLevelPassConfig,
) -> Result<NonZeroCoeffFscSignPass, CoeffFscSignPassError> {
    let (eob_read, level_walk, _level_inputs, level_reads, mut block) = level_pass.into_parts();
    preflight_pass(&block, &level_walk, scan, config)?;

    let sign_entries = fsc_sign_entries(&block, level_walk.seg_eob(), scan)?;
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

pub(super) fn preflight_pass(
    block: &TransformCoeffBlockState,
    level_walk: &FscCoeffScanWalk,
    scan: &[u16],
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
    if level_walk.seg_eob() > scan.len() {
        return Err(CoeffFscSignPassError::ScanTooShort {
            seg_eob: level_walk.seg_eob(),
            scan_len: scan.len(),
        });
    }
    for entry in fsc_sign_entries(block, level_walk.seg_eob(), scan)? {
        preflight_entry(block, entry)?;
    }
    Ok(())
}

pub(super) fn fsc_sign_entries(
    block: &TransformCoeffBlockState,
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
    entries.try_reserve(seg_eob)?;
    for (scan_index, &scan_pos) in scan.iter().enumerate().take(seg_eob) {
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
    let expected_pos = entry
        .row()
        .checked_mul(block.width())
        .and_then(|base| base.checked_add(entry.col()))
        .ok_or(TileCoeffStateError::ArithmeticOverflow {
            operation: "row * width + col",
            left: entry.row(),
            right: block.width(),
        })?;
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
    let (symbol, sign) = match input.source.selector() {
        Some(selector) => {
            let symbol = cdfs
                .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
                .get();
            (CoeffFscSignReadSymbol::IdtxSign { symbol }, symbol != 0)
        }
        None => (CoeffFscSignReadSymbol::None, false),
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
