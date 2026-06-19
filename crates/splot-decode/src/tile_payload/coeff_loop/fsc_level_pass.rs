// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! FSC/IDTX coefficient level first pass.
//!
//! Feature tracking: `DECODE-COEFF-FSC-LEVEL-PASS`.

use std::collections::TryReserveError;

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

/// Caller-resolved facts for the FSC/IDTX level first pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscLevelPassConfig {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Caller-resolved `txSzCtx`; selectors use `Min(TX_16X16, txSzCtx)`.
    pub(crate) tx_size_ctx: usize,
    /// Adjusted transform width in coefficients.
    pub(crate) tx_width: usize,
    /// Adjusted transform height in coefficients.
    pub(crate) tx_height: usize,
}

impl CoeffFscLevelPassConfig {
    pub(crate) const fn fsc_tx_size_ctx(self) -> usize {
        if self.tx_size_ctx < TX_16X16_CONTEXT {
            self.tx_size_ctx
        } else {
            TX_16X16_CONTEXT
        }
    }
}

/// Base symbol source selected for one FSC/IDTX level entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffFscLevelSymbolSource {
    /// Read `coeff_base_bob`; the decoded level is `symbol + 1`.
    BaseBob {
        /// Derived CDF selector.
        selector: CoeffCdfSelector,
    },
    /// Read `coeff_base_idtx`; the decoded level is `symbol`.
    BaseIdtx {
        /// Derived CDF selector.
        selector: CoeffCdfSelector,
    },
}

impl CoeffFscLevelSymbolSource {
    const fn selector(self) -> CoeffCdfSelector {
        match self {
            Self::BaseBob { selector } | Self::BaseIdtx { selector } => selector,
        }
    }

    const fn level_from_symbol(self, symbol: u8) -> u32 {
        match self {
            Self::BaseBob { .. } => symbol as u32 + 1,
            Self::BaseIdtx { .. } => symbol as u32,
        }
    }
}

/// Derived read facts for one checked FSC scan entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscLevelReadInput {
    /// Checked scan entry.
    pub(crate) entry: CoeffScanEntry,
    /// Base CDF selector and level bias.
    pub(crate) base: CoeffFscLevelSymbolSource,
    /// Conditional `coeff_br_idtx` selector.
    pub(crate) base_range: CoeffCdfSelector,
}

/// Decoded level symbols for one checked FSC scan entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffFscLevelRead {
    entry: CoeffScanEntry,
    base_symbol: u8,
    base_range_symbol: Option<u8>,
    level: u32,
}

impl CoeffFscLevelRead {
    /// Checked scan entry associated with this read.
    #[must_use]
    pub(crate) const fn entry(self) -> CoeffScanEntry {
        self.entry
    }

    /// Raw decoded `coeff_base_bob` or `coeff_base_idtx` symbol.
    #[must_use]
    pub(crate) const fn base_symbol(self) -> u8 {
        self.base_symbol
    }

    /// Raw decoded `coeff_br_idtx` symbol when the threshold branch was reached.
    #[must_use]
    pub(crate) const fn base_range_symbol(self) -> Option<u8> {
        self.base_range_symbol
    }

    /// Level after applying the BaseBob bias and any decoded BrIdtx symbol.
    #[must_use]
    pub(crate) const fn level(self) -> u32 {
        self.level
    }
}

/// Result of the FSC/IDTX level first pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffFscLevelPass {
    eob_read: NonZeroCoeffEobSymbolRead,
    walk: FscCoeffScanWalk,
    derived_inputs: Vec<CoeffFscLevelReadInput>,
    level_reads: Vec<CoeffFscLevelRead>,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffFscLevelPass {
    /// Decoded nonzero EOB syntax carried from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    /// Checked FSC scan walk used by the first pass.
    #[must_use]
    pub(crate) const fn walk(&self) -> &FscCoeffScanWalk {
        &self.walk
    }

    /// Derived read inputs in forward `bob..segEob` order.
    #[must_use]
    pub(crate) fn derived_inputs(&self) -> &[CoeffFscLevelReadInput] {
        &self.derived_inputs
    }

    /// Decoded level reads in forward `bob..segEob` order.
    #[must_use]
    pub(crate) fn level_reads(&self) -> &[CoeffFscLevelRead] {
        &self.level_reads
    }

    /// Local coefficient state after FSC first-pass `Level[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }

    /// Decomposes the completed first pass for later staged FSC/IDTX phases.
    pub(crate) fn into_parts(
        self,
    ) -> (
        NonZeroCoeffEobSymbolRead,
        FscCoeffScanWalk,
        Vec<CoeffFscLevelReadInput>,
        Vec<CoeffFscLevelRead>,
        TransformCoeffBlockState,
    ) {
        (
            self.eob_read,
            self.walk,
            self.derived_inputs,
            self.level_reads,
            self.block,
        )
    }
}

/// Error returned by the FSC/IDTX level first-pass boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffFscLevelPassError {
    /// The checked scan walk cardinality did not match the decoded EOB value.
    #[error("coefficient FSC level entries {entries} do not match decoded eob {eob}")]
    ScanEntryCountMismatch {
        /// Decoded EOB before FSC expands it to `segEob`.
        eob: usize,
        /// Checked walk entry count.
        entries: usize,
    },
    /// The caller supplied geometry that does not match the initialized block.
    #[error(
        "coefficient FSC level config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
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
    /// A checked scan entry did not match the local row-major block geometry.
    #[error(
        "coefficient FSC level scan entry {entry:?} maps to position {expected_pos}, not {actual_pos}"
    )]
    ScanEntryPositionMismatch {
        /// Checked scan entry.
        entry: CoeffScanEntry,
        /// Row-major position derived from `row`, `col`, and block width.
        expected_pos: usize,
        /// Entry position.
        actual_pos: usize,
    },
    /// Allocation for derived selector or decoded-read records failed.
    #[error("coefficient FSC level allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// CDF row selection or AV2 section 8.2 symbol decoding failed.
    #[error("coefficient FSC level symbol read failed: {0}")]
    SymbolRead(#[from] BlockSymbolTraceReadError),
    /// The local transform-block state rejected a checked coordinate or position.
    #[error("coefficient FSC level state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Runs the FSC/IDTX §5.20.7.27 level first pass over a checked scan window.
///
/// The helper implements the first `useFsc` loop over `c = bob .. segEob`:
/// `coeff_base_bob + 1` for `c == bob`, `coeff_base_idtx` for later entries,
/// conditional `coeff_br_idtx` when `level > NUM_BASE_LEVELS`, and
/// `Level[row][col] = level`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`;
/// selector contexts from
/// `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It does not read
/// `idtx_sign`, run `read_quant`, write `QuantSign[]` or `Quant[]`, commit tile
/// context lines, dequantize, or reconstruct pixels.
pub(crate) fn apply_nonzero_coeff_fsc_level_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    start: NonZeroCoeffBlockStart,
    walk: FscCoeffScanWalk,
    config: CoeffFscLevelPassConfig,
) -> Result<NonZeroCoeffFscLevelPass, CoeffFscLevelPassError> {
    let (eob_read, mut block) = start.into_parts();
    preflight_pass(eob_read, &block, &walk, config)?;

    let entries = walk.entries();
    let mut derived_inputs = Vec::new();
    let mut level_reads = Vec::new();
    derived_inputs.try_reserve(entries.len())?;
    level_reads.try_reserve(entries.len())?;

    for (index, entry) in entries.iter().copied().enumerate() {
        let input = derive_fsc_level_input(index, entry, &walk, &block, config);
        let read = read_fsc_level_symbol(cdfs, symbols, input)?;
        block.set_level(entry.row(), entry.col(), read.level())?;
        derived_inputs.push(input);
        level_reads.push(read);
    }

    Ok(NonZeroCoeffFscLevelPass {
        eob_read,
        walk,
        derived_inputs,
        level_reads,
        block,
    })
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
    let entries = walk.entries();
    if eob != entries.len() {
        return Err(CoeffFscLevelPassError::ScanEntryCountMismatch {
            eob,
            entries: entries.len(),
        });
    }
    for entry in entries.iter().copied() {
        block.level_at(entry.row(), entry.col())?;
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
            return Err(CoeffFscLevelPassError::ScanEntryPositionMismatch {
                entry,
                expected_pos,
                actual_pos: entry.pos(),
            });
        }
    }
    Ok(())
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
                ctx: coeff_base_idtx_ctx(block.level(), entry.row(), entry.col(), config.tx_width),
            },
        }
    };
    let br_ctx = coeff_br_idtx_ctx(block.level(), entry.row(), entry.col(), config.tx_width);
    CoeffFscLevelReadInput {
        entry,
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
) -> Result<CoeffFscLevelRead, CoeffFscLevelPassError> {
    let base_symbol = read_coeff_symbol(cdfs, symbols, input.base.selector())?;
    let mut level = input.base.level_from_symbol(base_symbol);
    let base_range_symbol = if level > NUM_BASE_LEVELS {
        let symbol = read_coeff_symbol(cdfs, symbols, input.base_range)?;
        level += u32::from(symbol);
        Some(symbol)
    } else {
        None
    };
    Ok(CoeffFscLevelRead {
        entry: input.entry,
        base_symbol,
        base_range_symbol,
        level,
    })
}

fn read_coeff_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: CoeffCdfSelector,
) -> Result<u8, CoeffFscLevelPassError> {
    Ok(cdfs
        .read_block_symbol_trace(TileCdfSelector::Coeff(selector), symbols)?
        .get())
}
