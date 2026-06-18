// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient base/level first pass with derived contexts.
//!
//! Feature tracking: `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::coeff_context::{
    CoeffBaseContext, CoeffBaseSelection, CoeffBrContext, coeff_base_eob_ctx,
};
use super::super::cdf::{CoeffCdfSelector, TileCdfSubset};
use super::super::coeff_state::{TileCoeffStateError, TransformCoeffBlockState};
use super::NonZeroCoeffEobSymbolRead;
use super::base_symbol::{
    CoeffBaseRangeRead, CoeffBaseSymbolRead, CoeffBaseSymbolReadError, CoeffBaseSymbolReadInput,
    CoeffBaseSymbolSource, read_coeff_base_symbol,
};
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::{
    COEFF_BASE_RANGE, CoeffTransformClass, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS,
    coeff_is_low_frequency,
};
use super::quant_state::next_tcq_state;
use super::scan_walk::{CoeffScanEntry, NonZeroCoeffScanWalk};

const PHTHRESH: usize = 4;

/// Caller-resolved facts for deriving ordinary non-FSC base/level selectors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffBaseDerivedLevelPassConfig {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Transform-size context (`txSzCtx`) for luma coefficient rows.
    pub(crate) tx_size_ctx: usize,
    /// `Tx_Width_Log2[adjTxSz]`, resolved by the caller.
    pub(crate) tx_width_log2: u32,
    /// Adjusted transform width in coefficients.
    pub(crate) tx_width: usize,
    /// Adjusted transform height in coefficients.
    pub(crate) tx_height: usize,
    /// Plane index, 0 for luma and greater than 0 for chroma.
    pub(crate) plane: usize,
    /// Caller-resolved `get_tx_class(PlaneTxType)` result.
    pub(crate) tx_class: CoeffTransformClass,
    /// Whether hidden parity is active for this transform block.
    pub(crate) parity_hiding: bool,
    /// Whether TCQ is active for this transform block.
    pub(crate) use_tcq: bool,
}

/// First-pass hidden-parity and TCQ state after base/level reads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CoeffBaseFirstPassSummary {
    sum_abs1: u32,
    num_nonzero: usize,
    is_hidden: bool,
    tcq_state: usize,
}

impl CoeffBaseFirstPassSummary {
    /// Caller-visible `sumAbs1` parity accumulator after the first pass.
    #[must_use]
    pub(crate) const fn sum_abs1(self) -> u32 {
        self.sum_abs1
    }

    /// Number of nonzero `c > 0` coefficients contributing to parity hiding.
    #[must_use]
    pub(crate) const fn num_nonzero(self) -> usize {
        self.num_nonzero
    }

    /// Whether hidden parity is active for the final DC coefficient.
    #[must_use]
    pub(crate) const fn is_hidden(self) -> bool {
        self.is_hidden
    }

    /// `tcqState` after the first pass.
    #[must_use]
    pub(crate) const fn tcq_state(self) -> usize {
        self.tcq_state
    }

    fn update_after_level(
        &mut self,
        entry: CoeffScanEntry,
        level: u32,
        config: CoeffBaseDerivedLevelPassConfig,
    ) -> Result<(), CoeffBaseDerivedLevelPassError> {
        if config.use_tcq {
            self.tcq_state = next_tcq_state(self.tcq_state, level).ok_or(
                CoeffBaseDerivedLevelPassError::InvalidTcqState {
                    entry,
                    tcq_state: self.tcq_state,
                },
            )?;
        }
        if config.parity_hiding && entry.scan_index() > 0 {
            let clipped = level.min(NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1);
            self.sum_abs1 ^= clipped & 1;
            if level != 0 {
                self.num_nonzero += 1;
                self.is_hidden = self.num_nonzero >= PHTHRESH;
            }
        }
        Ok(())
    }
}

/// Result of the derived ordinary non-FSC base/level first pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffBaseDerivedLevelPass {
    eob_read: NonZeroCoeffEobSymbolRead,
    walk: NonZeroCoeffScanWalk,
    derived_inputs: Vec<CoeffBaseSymbolReadInput>,
    base_reads: Vec<CoeffBaseSymbolRead>,
    first_pass: CoeffBaseFirstPassSummary,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffBaseDerivedLevelPass {
    /// Decoded nonzero EOB syntax carried from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    /// Checked scan walk used by the first pass.
    #[must_use]
    pub(crate) const fn walk(&self) -> &NonZeroCoeffScanWalk {
        &self.walk
    }

    /// Derived base/base-range selector inputs in scan-walk order.
    #[must_use]
    pub(crate) fn derived_inputs(&self) -> &[CoeffBaseSymbolReadInput] {
        &self.derived_inputs
    }

    /// Decoded base/base-range summaries in scan-walk order.
    #[must_use]
    pub(crate) fn base_reads(&self) -> &[CoeffBaseSymbolRead] {
        &self.base_reads
    }

    /// First-pass parity and TCQ state summary.
    #[must_use]
    pub(crate) const fn first_pass(&self) -> CoeffBaseFirstPassSummary {
        self.first_pass
    }

    /// Local coefficient state after first-pass `Level[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Error returned by the derived ordinary base/level first-pass boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffBaseDerivedLevelPassError {
    /// The checked scan walk cardinality did not match the decoded EOB value.
    #[error("coefficient base/level scan entries {entries} do not match eob {eob}")]
    ScanEntryCountMismatch {
        /// Decoded `eob`.
        eob: usize,
        /// Checked scan-walk entry count.
        entries: usize,
    },
    /// The caller supplied geometry that does not match the initialized block.
    #[error(
        "coefficient base/level config geometry {config_width}x{config_height} does not match block {block_width}x{block_height}"
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
    /// The caller supplied a width log2 inconsistent with the adjusted width.
    #[error("coefficient base/level tx_width_log2 {tx_width_log2} does not match width {tx_width}")]
    TxWidthLog2Mismatch {
        /// Caller-resolved width log2.
        tx_width_log2: u32,
        /// Caller-resolved width.
        tx_width: usize,
    },
    /// A checked scan entry did not match the local row-major block geometry.
    #[error(
        "coefficient base/level scan entry {entry:?} maps to position {expected_pos}, not {actual_pos}"
    )]
    ScanEntryPositionMismatch {
        /// Checked scan entry.
        entry: CoeffScanEntry,
        /// Row-major position derived from `row`, `col`, and block width.
        expected_pos: usize,
        /// Entry position.
        actual_pos: usize,
    },
    /// The caller supplied mutually exclusive block facts.
    #[error("coefficient base/level config cannot enable parity hiding and TCQ together")]
    InconsistentParityAndTcq,
    /// Hidden-parity `TileCoeffBasePhCdf` rows are not loaded yet.
    #[error("coefficient base/level parity-hidden base selector ctx {ctx} is not supported")]
    UnsupportedParityHiddenBaseSelector {
        /// Checked scan entry that requested the parity-hidden row.
        entry: CoeffScanEntry,
        /// Parity-hidden base context.
        ctx: usize,
    },
    /// The local TCQ state was outside the AV2 state table.
    #[error("coefficient base/level entry {entry:?} used invalid tcqState {tcq_state}")]
    InvalidTcqState {
        /// Checked scan entry.
        entry: CoeffScanEntry,
        /// Invalid `tcqState`.
        tcq_state: usize,
    },
    /// Allocation for derived selector or decoded-read records failed.
    #[error("coefficient base/level allocation failed: {0}")]
    Allocation(#[from] TryReserveError),
    /// Base/base-range symbol reading failed.
    #[error("coefficient base/level base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
    /// The local transform-block state rejected a checked coordinate or position.
    #[error("coefficient base/level state error: {0}")]
    State(#[from] TileCoeffStateError),
}

/// Runs the ordinary non-FSC base/level first pass with runtime selector derivation.
///
/// The helper implements the first loop over `c = eob - 1 .. 0` from AV2
/// §5.20.7.27 for non-FSC blocks: it derives `coeff_base_eob`/`coeff_base` and
/// conditional `coeff_br` selectors from the current `Level[]`, reads the
/// symbols, updates first-pass `tcqState`, `sumAbs1`, `numNz`, and `isHidden`,
/// then writes `Level[row][col]` before the next selector derivation. It does not
/// read signs or quant residuals, update tile context lines, dequantize, or run
/// reconstruction.
pub(crate) fn apply_nonzero_coeff_base_derived_level_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    start: NonZeroCoeffBlockStart,
    walk: NonZeroCoeffScanWalk,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<NonZeroCoeffBaseDerivedLevelPass, CoeffBaseDerivedLevelPassError> {
    let (eob_read, mut block) = start.into_parts();
    preflight_pass(eob_read, &block, &walk, config)?;

    let entries = walk.entries();
    let mut derived_inputs = Vec::new();
    let mut base_reads = Vec::new();
    derived_inputs.try_reserve(entries.len())?;
    base_reads.try_reserve(entries.len())?;

    let mut first_pass = CoeffBaseFirstPassSummary::default();
    for (index, entry) in entries.iter().copied().enumerate() {
        let input = derive_base_symbol_input(index, entry, &block, first_pass, config)?;
        let read = read_coeff_base_symbol(cdfs, symbols, input)?;
        first_pass.update_after_level(entry, read.level(), config)?;
        block.set_level(entry.row(), entry.col(), read.level())?;
        derived_inputs.push(input);
        base_reads.push(read);
    }

    Ok(NonZeroCoeffBaseDerivedLevelPass {
        eob_read,
        walk,
        derived_inputs,
        base_reads,
        first_pass,
        block,
    })
}

fn preflight_pass(
    eob_read: NonZeroCoeffEobSymbolRead,
    block: &TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<(), CoeffBaseDerivedLevelPassError> {
    if config.parity_hiding && config.use_tcq {
        return Err(CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq);
    }
    if block.width() != config.tx_width || block.height() != config.tx_height {
        return Err(CoeffBaseDerivedLevelPassError::BlockGeometryMismatch {
            block_width: block.width(),
            block_height: block.height(),
            config_width: config.tx_width,
            config_height: config.tx_height,
        });
    }
    if 1usize.checked_shl(config.tx_width_log2) != Some(config.tx_width) {
        return Err(CoeffBaseDerivedLevelPassError::TxWidthLog2Mismatch {
            tx_width_log2: config.tx_width_log2,
            tx_width: config.tx_width,
        });
    }

    let eob = eob_read.eob().eob();
    let entries = walk.entries();
    if eob != entries.len() {
        return Err(CoeffBaseDerivedLevelPassError::ScanEntryCountMismatch {
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
            return Err(CoeffBaseDerivedLevelPassError::ScanEntryPositionMismatch {
                entry,
                expected_pos,
                actual_pos: entry.pos(),
            });
        }
    }
    Ok(())
}

fn derive_base_symbol_input(
    index: usize,
    entry: CoeffScanEntry,
    block: &TransformCoeffBlockState,
    first_pass: CoeffBaseFirstPassSummary,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<CoeffBaseSymbolReadInput, CoeffBaseDerivedLevelPassError> {
    let is_lf = coeff_is_low_frequency(entry, config.plane, config.tx_class);
    let base_levels = if is_lf {
        LF_NUM_BASE_LEVELS
    } else {
        NUM_BASE_LEVELS
    };
    let base = if index == 0 {
        CoeffBaseSymbolSource::BaseEob {
            selector: base_eob_selector(entry, is_lf, config),
        }
    } else {
        CoeffBaseSymbolSource::Base {
            selector: base_selector(entry, is_lf, block, first_pass, config)?,
        }
    };
    let base_range = if is_lf && config.plane > 0 {
        CoeffBaseRangeRead::Disabled
    } else {
        CoeffBaseRangeRead::Enabled {
            selector: base_range_selector(entry, is_lf, block, config),
        }
    };

    Ok(CoeffBaseSymbolReadInput {
        entry,
        base,
        base_levels,
        base_range,
    })
}

fn base_eob_selector(
    entry: CoeffScanEntry,
    is_lf: bool,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffCdfSelector {
    let ctx = coeff_base_eob_ctx(entry.scan_index(), config.tx_width_log2, config.tx_height);
    if config.plane > 0 {
        if is_lf {
            CoeffCdfSelector::BaseLfEobUv {
                coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
                ctx,
            }
        } else {
            CoeffCdfSelector::BaseEobUv {
                coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
                ctx,
            }
        }
    } else if is_lf {
        CoeffCdfSelector::BaseLfEob {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
        }
    } else {
        CoeffCdfSelector::BaseEob {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
        }
    }
}

fn base_selector(
    entry: CoeffScanEntry,
    is_lf: bool,
    block: &TransformCoeffBlockState,
    first_pass: CoeffBaseFirstPassSummary,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<CoeffCdfSelector, CoeffBaseDerivedLevelPassError> {
    let selection = CoeffBaseContext {
        pos: entry.pos(),
        bwl: config.tx_width_log2,
        txw: config.tx_width,
        txh: config.tx_height,
        plane: config.plane,
        is_lf,
        is_hidden: first_pass.is_hidden,
        c: entry.scan_index(),
        tx_class: tx_class_index(config.tx_class),
    }
    .select(block.level());
    map_base_selection(entry, selection, first_pass, config)
}

fn map_base_selection(
    entry: CoeffScanEntry,
    selection: CoeffBaseSelection,
    first_pass: CoeffBaseFirstPassSummary,
    config: CoeffBaseDerivedLevelPassConfig,
) -> Result<CoeffCdfSelector, CoeffBaseDerivedLevelPassError> {
    let tcq_ctx = (first_pass.tcq_state >> 1) & 1;
    match selection {
        CoeffBaseSelection::Ph { ctx } => {
            Err(CoeffBaseDerivedLevelPassError::UnsupportedParityHiddenBaseSelector { entry, ctx })
        }
        CoeffBaseSelection::LfUv { ctx } => Ok(CoeffCdfSelector::BaseLfUv {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }),
        CoeffBaseSelection::Uv { ctx } => Ok(CoeffCdfSelector::BaseUv {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }),
        CoeffBaseSelection::Lf { ctx } => Ok(CoeffCdfSelector::BaseLf {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
            tcq_ctx,
        }),
        CoeffBaseSelection::Hf { ctx } => Ok(CoeffCdfSelector::Base {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            tx_size: config.tx_size_ctx,
            ctx,
            tcq_ctx,
        }),
    }
}

fn base_range_selector(
    entry: CoeffScanEntry,
    is_lf: bool,
    block: &TransformCoeffBlockState,
    config: CoeffBaseDerivedLevelPassConfig,
) -> CoeffCdfSelector {
    let ctx = CoeffBrContext {
        pos: entry.pos(),
        bwl: config.tx_width_log2,
        txw: config.tx_width,
        txh: config.tx_height,
        plane: config.plane,
        is_lf,
        tx_class: tx_class_index(config.tx_class),
    }
    .ctx(block.level());
    if config.plane > 0 {
        CoeffCdfSelector::BrUv {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }
    } else if is_lf {
        CoeffCdfSelector::BrLf {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }
    } else {
        CoeffCdfSelector::Br {
            coeff_cdf_q_ctx: config.coeff_cdf_q_ctx,
            ctx,
        }
    }
}

const fn tx_class_index(tx_class: CoeffTransformClass) -> usize {
    match tx_class {
        CoeffTransformClass::TwoD => 0,
        CoeffTransformClass::Horizontal => 1,
        CoeffTransformClass::Vertical => 2,
    }
}
