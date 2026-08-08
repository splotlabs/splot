// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop foundation helpers.

use std::collections::TryReserveError;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{EobPtSize, TileCdfSelector, TileCdfSubset};
use super::coeff_state::{CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError};

const COEFFS_PER_4X4: usize = 4;
const MAX_ADJUSTED_COEFF_EXTENT: usize = 32;
const MIN_EOB_TX_LOG2: usize = 2;
const EOB_MULTISIZE_LOG2_CAP: usize = 5;
const EOB_MULTISIZE_OFFSET: usize = 4;
const MIN_NONZERO_EOB_PT: usize = 1;
const MAX_NONZERO_EOB_PT: usize = 11;
const EOB_GROUP_START: [usize; MAX_NONZERO_EOB_PT + 1] =
    [0, 1, 2, 3, 5, 9, 17, 33, 65, 129, 257, 513];
const EOB_OFFSET_BITS: [usize; MAX_NONZERO_EOB_PT + 1] = [0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
pub(crate) mod base_level_pass;
pub(crate) mod base_symbol;
mod branch;
pub(crate) use branch::{NonZeroCoeffBlockStartInput, read_nonzero_coeff_block_start};
pub(crate) mod fsc_level_pass;
pub(crate) mod fsc_quant_pass;
pub(crate) mod fsc_sign_pass;
pub(crate) mod max_level;
pub(crate) mod ordinary_pass;
pub(crate) mod quant_pass;
pub(crate) mod quant_state;
pub(crate) mod read_quant;
mod scan_walk;
pub(crate) mod sign_symbol;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllZeroCoeffBlockInput {
    pub(crate) plane: usize,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
}

pub(crate) fn commit_nonzero_coeff_context(
    state: &mut TileCoeffContextState,
    context: AllZeroCoeffBlockInput,
    quant_state: &quant_state::NonZeroCoeffQuantState,
) -> Result<(), TileCoeffStateError> {
    state.update_after_coeffs(CoeffContextUpdate {
        plane: context.plane,
        x4: context.x4,
        y4: context.y4,
        w4: context.w4,
        h4: context.h4,
        cul_level: quant_state.cul_level(),
        dc_category: quant_state.dc_category(),
    })
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobInput {
    pub(crate) eob_pt: usize,
    pub(crate) eob_extra: bool,
    pub(crate) eob_extra_bits: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobSymbolInput {
    pub(crate) size: EobPtSize,
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) eob_ctx: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobContextInput {
    pub(crate) plane: usize,
    pub(crate) is_inter: bool,
    pub(crate) tx_width_log2: usize,
    pub(crate) tx_height_log2: usize,
    pub(crate) coeff_cdf_q_ctx: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEob {
    eob: usize,
}

impl NonZeroCoeffEob {
    #[must_use]
    pub(crate) const fn eob(self) -> usize {
        self.eob
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobSymbolRead {
    eob: NonZeroCoeffEob,
}

impl NonZeroCoeffEobSymbolRead {
    #[must_use]
    pub(crate) const fn eob(self) -> NonZeroCoeffEob {
        self.eob
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffLoopContextError {
    #[error("coefficient context state error: {0}")]
    State(#[from] TileCoeffStateError),
    #[error("coefficient EOB symbol read failed: {0}")]
    EobSymbolRead(#[from] BlockSymbolTraceReadError),
    #[error("coefficient EOB literal read failed for {syntax}: {source}")]
    EobLiteralRead {
        syntax: &'static str,
        #[source]
        source: CoreError,
    },
    #[error("coefficient EOB point {eob_pt} is outside the supported AV2 range 1..=11")]
    InvalidEobPoint { eob_pt: usize },
    #[error(
        "coefficient EOB point {eob_pt} cannot carry eob_extra={eob_extra} or eob_extra_bits={eob_extra_bits}"
    )]
    UnexpectedEobRefinement {
        eob_pt: usize,
        eob_extra: bool,
        eob_extra_bits: usize,
    },
    #[error(
        "coefficient EOB point {eob_pt} allows eob_extra_bits <= {max_eob_extra_bits}, got {eob_extra_bits}"
    )]
    EobExtraBitsOutOfRange {
        eob_pt: usize,
        eob_extra_bits: usize,
        max_eob_extra_bits: usize,
    },
    #[error("coefficient EOB Pt512 extra value {eob_pt_extra} is reserved by AV2")]
    InvalidPt512EobExtra { eob_pt_extra: u32 },
    #[error(
        "coefficient EOB transform {axis} log2 value {value} is below the AV2 minimum {minimum}"
    )]
    InvalidEobTransformLog2 {
        axis: &'static str,
        value: usize,
        minimum: usize,
    },
    #[error("coefficient scan walk requires nonzero EOB, got {eob}")]
    InvalidScanWalkEob { eob: usize },
    #[error("coefficient scan walk EOB {eob} exceeds scan length {scan_len}")]
    ScanWalkEobOutOfRange { eob: usize, scan_len: usize },
    #[error("coefficient FSC scan walk EOB {eob} exceeds segment EOB {seg_eob}")]
    FscScanWalkEobOutOfRange { eob: usize, seg_eob: usize },
    #[error(
        "coefficient scan index {scan_index} points to position {pos}, outside coefficient count {coeff_count}"
    )]
    ScanWalkPositionOutOfRange {
        scan_index: usize,
        pos: usize,
        coeff_count: usize,
    },
    #[error("coefficient scan walk allocation failed: {0}")]
    ScanWalkAllocation(#[from] TryReserveError),
}

pub(crate) fn nonzero_coeff_eob(
    input: NonZeroCoeffEobInput,
) -> Result<NonZeroCoeffEob, CoeffLoopContextError> {
    let eob_pt = input.eob_pt;
    if !(MIN_NONZERO_EOB_PT..=MAX_NONZERO_EOB_PT).contains(&eob_pt) {
        return Err(CoeffLoopContextError::InvalidEobPoint { eob_pt });
    }
    if eob_pt < 3 {
        if input.eob_extra || input.eob_extra_bits != 0 {
            return Err(CoeffLoopContextError::UnexpectedEobRefinement {
                eob_pt,
                eob_extra: input.eob_extra,
                eob_extra_bits: input.eob_extra_bits,
            });
        }
        return Ok(NonZeroCoeffEob { eob: eob_pt });
    }

    let extra_bits_width = eob_extra_bits_width(eob_pt)?;
    let max_eob_extra_bits = (1usize << extra_bits_width) - 1;
    if input.eob_extra_bits > max_eob_extra_bits {
        return Err(CoeffLoopContextError::EobExtraBitsOutOfRange {
            eob_pt,
            eob_extra_bits: input.eob_extra_bits,
            max_eob_extra_bits,
        });
    }

    let base = EOB_GROUP_START[eob_pt];
    let extra = if input.eob_extra {
        1usize << extra_bits_width
    } else {
        0
    };
    Ok(NonZeroCoeffEob {
        eob: base + extra + input.eob_extra_bits,
    })
}

pub(crate) fn read_nonzero_coeff_eob(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffEobSymbolInput,
) -> Result<NonZeroCoeffEobSymbolRead, CoeffLoopContextError> {
    let eob_pt_symbol = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::EobPt {
                size: input.size,
                coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                eob_ctx: input.eob_ctx,
            },
            symbols,
        )?
        .get();
    let eob_pt_extra_width = eob_pt_extra_width(input.size, eob_pt_symbol);
    let eob_pt_extra = read_eob_literal(symbols, eob_pt_extra_width, "eob_pt_extra")?;
    let eob_pt =
        checked_resolved_eob_pt(input.size, eob_pt_symbol, eob_pt_extra_width, eob_pt_extra)?;

    let (eob_extra, eob_extra_bits) = if eob_pt >= 3 {
        let eob_extra = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::EobExtra {
                    coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                },
                symbols,
            )?
            .get()
            != 0;
        let width = eob_extra_bits_width(eob_pt)? as u32;
        (
            eob_extra,
            read_eob_literal(symbols, width, "eob_extra_bit")?,
        )
    } else {
        (false, 0)
    };

    let eob = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt,
        eob_extra,
        eob_extra_bits: eob_extra_bits as usize,
    })?;
    Ok(NonZeroCoeffEobSymbolRead { eob })
}

pub(crate) fn read_nonzero_coeff_eob_from_context(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: NonZeroCoeffEobContextInput,
) -> Result<NonZeroCoeffEobSymbolRead, CoeffLoopContextError> {
    let input = nonzero_coeff_eob_symbol_input(input)?;
    read_nonzero_coeff_eob(cdfs, symbols, input)
}

pub(crate) fn nonzero_coeff_eob_symbol_input(
    input: NonZeroCoeffEobContextInput,
) -> Result<NonZeroCoeffEobSymbolInput, CoeffLoopContextError> {
    Ok(NonZeroCoeffEobSymbolInput {
        size: eob_pt_size_from_tx_log2(input.tx_width_log2, input.tx_height_log2)?,
        coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
        eob_ctx: eob_context(input.plane, input.is_inter),
    })
}

fn eob_pt_size_from_tx_log2(
    tx_width_log2: usize,
    tx_height_log2: usize,
) -> Result<EobPtSize, CoeffLoopContextError> {
    checked_eob_tx_log2("width", tx_width_log2)?;
    checked_eob_tx_log2("height", tx_height_log2)?;

    let eob_multisize = tx_width_log2.min(EOB_MULTISIZE_LOG2_CAP)
        + tx_height_log2.min(EOB_MULTISIZE_LOG2_CAP)
        - EOB_MULTISIZE_OFFSET;
    Ok(match eob_multisize {
        0 => EobPtSize::Pt16,
        1 => EobPtSize::Pt32,
        2 => EobPtSize::Pt64,
        3 => EobPtSize::Pt128,
        4 => EobPtSize::Pt256,
        5 => EobPtSize::Pt512,
        _ => EobPtSize::Pt1024,
    })
}

fn checked_eob_tx_log2(axis: &'static str, value: usize) -> Result<(), CoeffLoopContextError> {
    if value < MIN_EOB_TX_LOG2 {
        return Err(CoeffLoopContextError::InvalidEobTransformLog2 {
            axis,
            value,
            minimum: MIN_EOB_TX_LOG2,
        });
    }
    Ok(())
}

fn eob_context(plane: usize, is_inter: bool) -> usize {
    if plane > 0 { 2 } else { usize::from(is_inter) }
}

fn eob_pt_extra_width(size: EobPtSize, eob_pt_symbol: u8) -> u32 {
    match (size, eob_pt_symbol) {
        (EobPtSize::Pt256, 7) => 1,
        (EobPtSize::Pt512 | EobPtSize::Pt1024, 7) => 2,
        _ => 0,
    }
}

fn resolved_eob_pt(eob_pt_symbol: u8, eob_pt_extra_width: u32, eob_pt_extra: u32) -> usize {
    if eob_pt_extra_width == 0 {
        usize::from(eob_pt_symbol) + 1
    } else {
        8 + eob_pt_extra as usize
    }
}

fn checked_resolved_eob_pt(
    size: EobPtSize,
    eob_pt_symbol: u8,
    eob_pt_extra_width: u32,
    eob_pt_extra: u32,
) -> Result<usize, CoeffLoopContextError> {
    if matches!(size, EobPtSize::Pt512) && eob_pt_extra_width != 0 && eob_pt_extra == 3 {
        return Err(CoeffLoopContextError::InvalidPt512EobExtra { eob_pt_extra });
    }
    Ok(resolved_eob_pt(
        eob_pt_symbol,
        eob_pt_extra_width,
        eob_pt_extra,
    ))
}

fn eob_extra_bits_width(eob_pt: usize) -> Result<usize, CoeffLoopContextError> {
    EOB_OFFSET_BITS
        .get(eob_pt)
        .copied()
        .map(|width| width.saturating_sub(1))
        .ok_or(CoeffLoopContextError::InvalidEobPoint { eob_pt })
}

fn read_eob_literal(
    symbols: &mut SymbolDecoder<'_>,
    width: u32,
    syntax: &'static str,
) -> Result<u32, CoeffLoopContextError> {
    if width == 0 {
        return Ok(0);
    }
    let value = symbols
        .read_literal(width)
        .map_err(|source| CoeffLoopContextError::EobLiteralRead { syntax, source })?;
    Ok(value)
}

fn adjusted_coeff_extent(size4: usize) -> usize {
    size4
        .saturating_mul(COEFFS_PER_4X4)
        .min(MAX_ADJUSTED_COEFF_EXTENT)
}
#[cfg(test)]
#[path = "coeff_loop_tests.rs"]
mod tests;
