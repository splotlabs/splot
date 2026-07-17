// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop foundation helpers.

use std::collections::TryReserveError;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    ADJUSTED_TX_SIZE, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR, TX_SIZE_SQR_UP, TX_WIDTH,
    TX_WIDTH_LOG2,
};

use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{EobPtSize, TileCdfSelector, TileCdfSubset};
use super::coeff_state::{
    CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};

const LUMA_PLANE: usize = 0;
const V_PLANE: usize = 2;
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
macro_rules! coeff_branch_map_adapter {
    ($vis:vis fn $name:ident($input_ty:ty) -> $result_ty:ty, $nonzero:ident, $mapped:expr, $callee:path,) => {
        $vis fn $name(
            state: &mut TileCoeffContextState,
            cdfs: &mut TileCdfSubset,
            symbols: &mut SymbolDecoder<'_>,
            input: $input_ty,
        ) -> $result_ty {
            let input = input.map_nonzero(|$nonzero| $mapped);
            $callee(state, cdfs, symbols, input)
        }
    };
}
pub(crate) mod base_level_pass;
pub(crate) mod base_symbol;
mod branch;
pub(crate) use branch::{NonZeroCoeffBlockStartInput, read_nonzero_coeff_block_start};
pub(crate) mod fsc_level_pass;
pub(crate) mod fsc_quant_pass;
#[cfg(test)]
mod fsc_quant_pass_tests;
pub(crate) mod fsc_sign_pass;
#[cfg(test)]
mod fsc_sign_pass_tests;
pub(crate) mod level_state;
pub(crate) mod max_level;
pub(crate) mod ordinary_pass;
pub(crate) mod quant_pass;
pub(crate) mod quant_state;
pub(crate) mod read_quant;
mod scan_walk;
pub(crate) mod sign_symbol;
pub(crate) mod use_fsc_branch;
#[cfg(test)]
mod use_fsc_frame_facts_tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaAllZeroContextInput {
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
    pub(crate) tx_fills_block: bool,
    pub(crate) fsc_active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VAllZeroContextInput {
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
    pub(crate) chroma_block_larger_than_tx: bool,
    pub(crate) eob_u_nonzero: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AllZeroCoeffBlockInput {
    pub(crate) plane: usize,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
}

pub(crate) enum CoeffBranchInput<AllZero, NonZero> {
    AllZero(AllZero),
    NonZero(NonZero),
}

impl<AllZero, NonZero> CoeffBranchInput<AllZero, NonZero> {
    pub(crate) fn map_nonzero<NextNonZero>(
        self,
        map: impl FnOnce(NonZero) -> NextNonZero,
    ) -> CoeffBranchInput<AllZero, NextNonZero> {
        match self {
            Self::AllZero(input) => CoeffBranchInput::AllZero(input),
            Self::NonZero(input) => CoeffBranchInput::NonZero(map(input)),
        }
    }

    pub(crate) fn try_map_nonzero<NextNonZero, E>(
        self,
        map: impl FnOnce(NonZero) -> Result<NextNonZero, E>,
    ) -> Result<CoeffBranchInput<AllZero, NextNonZero>, E> {
        match self {
            Self::AllZero(input) => Ok(CoeffBranchInput::AllZero(input)),
            Self::NonZero(input) => map(input).map(CoeffBranchInput::NonZero),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CoeffTxSizeTables<'a> {
    pub(crate) adjusted_tx_size: &'a [i32],
    pub(crate) tx_size_sqr: &'a [i32],
    pub(crate) tx_size_sqr_up: &'a [i32],
    pub(crate) tx_width: &'a [i32],
    pub(crate) tx_height: &'a [i32],
    pub(crate) tx_width_log2: &'a [i32],
    pub(crate) tx_height_log2: &'a [i32],
}

pub(crate) const DEFAULT_TX_SIZE_TABLES: CoeffTxSizeTables<'static> = CoeffTxSizeTables {
    adjusted_tx_size: &ADJUSTED_TX_SIZE,
    tx_size_sqr: &TX_SIZE_SQR,
    tx_size_sqr_up: &TX_SIZE_SQR_UP,
    tx_width: &TX_WIDTH,
    tx_height: &TX_HEIGHT,
    tx_width_log2: &TX_WIDTH_LOG2,
    tx_height_log2: &TX_HEIGHT_LOG2,
};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AllZeroCoeffBlock {
    eob: usize,
    cul_level: u8,
    dc_category: u8,
    block: TransformCoeffBlockState,
}

impl AllZeroCoeffBlock {
    #[must_use]
    pub(crate) const fn eob(&self) -> usize {
        self.eob
    }
    #[must_use]
    pub(crate) const fn cul_level(&self) -> u8 {
        self.cul_level
    }
    #[must_use]
    pub(crate) const fn dc_category(&self) -> u8 {
        self.dc_category
    }
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEob {
    eob_pt: usize,
    eob: usize,
}

impl NonZeroCoeffEob {
    #[must_use]
    pub(crate) const fn eob_pt(self) -> usize {
        self.eob_pt
    }
    #[must_use]
    pub(crate) const fn eob(self) -> usize {
        self.eob
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffEobSymbolRead {
    eob: NonZeroCoeffEob,
    eob_pt_symbol: u8,
    eob_pt_extra: u32,
    eob_extra: bool,
    eob_extra_bits: u32,
}

impl NonZeroCoeffEobSymbolRead {
    #[must_use]
    pub(crate) const fn eob(self) -> NonZeroCoeffEob {
        self.eob
    }
    #[must_use]
    pub(crate) const fn eob_pt_symbol(self) -> u8 {
        self.eob_pt_symbol
    }
    #[must_use]
    pub(crate) const fn eob_pt_extra(self) -> u32 {
        self.eob_pt_extra
    }
    #[must_use]
    pub(crate) const fn eob_extra(self) -> bool {
        self.eob_extra
    }

    #[must_use]
    pub(crate) const fn eob_extra_bits(self) -> u32 {
        self.eob_extra_bits
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

pub(crate) fn luma_all_zero_context(
    state: &TileCoeffContextState,
    input: LumaAllZeroContextInput,
) -> Result<usize, CoeffLoopContextError> {
    let above = bounded_or(
        state.above_level(LUMA_PLANE)?,
        state.local_x4(LUMA_PLANE, input.x4)?,
        input.w4,
    );
    let left = bounded_or(
        state.left_level(LUMA_PLANE)?,
        state.local_y4(LUMA_PLANE, input.y4)?,
        input.h4,
    );
    Ok(txb_skip_ctx_luma(
        above,
        left,
        input.tx_fills_block,
        input.fsc_active,
    ))
}

pub(crate) fn v_all_zero_context(
    state: &TileCoeffContextState,
    input: VAllZeroContextInput,
) -> Result<usize, CoeffLoopContextError> {
    let above = bounded_or_level_dc(
        state.above_level(V_PLANE)?,
        state.above_dc(V_PLANE)?,
        state.local_x4(V_PLANE, input.x4)?,
        input.w4,
    );
    let left = bounded_or_level_dc(
        state.left_level(V_PLANE)?,
        state.left_dc(V_PLANE)?,
        state.local_y4(V_PLANE, input.y4)?,
        input.h4,
    );
    Ok(v_txb_skip_ctx(
        above != 0,
        left != 0,
        input.chroma_block_larger_than_tx,
        input.eob_u_nonzero,
    ))
}

pub(crate) fn apply_all_zero_coeff_block(
    state: &mut TileCoeffContextState,
    input: AllZeroCoeffBlockInput,
) -> Result<AllZeroCoeffBlock, CoeffLoopContextError> {
    let width = adjusted_coeff_extent(input.w4);
    let height = adjusted_coeff_extent(input.h4);
    let block = TransformCoeffBlockState::new(width, height)?;
    let cul_level = 0;
    let dc_category = 0;
    state.update_after_coeffs(CoeffContextUpdate {
        plane: input.plane,
        x4: input.x4,
        y4: input.y4,
        w4: input.w4,
        h4: input.h4,
        cul_level,
        dc_category,
    })?;
    Ok(AllZeroCoeffBlock {
        eob: 0,
        cul_level,
        dc_category,
        block,
    })
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
        return Ok(NonZeroCoeffEob {
            eob_pt,
            eob: eob_pt,
        });
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
        eob_pt,
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
    Ok(NonZeroCoeffEobSymbolRead {
        eob,
        eob_pt_symbol,
        eob_pt_extra,
        eob_extra,
        eob_extra_bits,
    })
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

fn bounded_or<T: Copy + Into<u32>>(values: &[T], start: usize, count: usize) -> u32 {
    values.get(start..).map_or(0, |tail| {
        tail.iter()
            .take(count)
            .fold(0, |value, entry| value | (*entry).into())
    })
}

fn bounded_or_level_dc(level: &[u8], dc: &[u8], start: usize, count: usize) -> u32 {
    bounded_or(level, start, count) | bounded_or(dc, start, count)
}

fn adjusted_coeff_extent(size4: usize) -> usize {
    size4
        .saturating_mul(COEFFS_PER_4X4)
        .min(MAX_ADJUSTED_COEFF_EXTENT)
}
#[cfg(test)]
mod base_level_pass_tests;
#[cfg(test)]
mod base_symbol_tests;
#[cfg(test)]
mod eob_symbol_tests;
#[cfg(test)]
mod fsc_level_pass_tests;
#[cfg(test)]
mod level_state_tests;
#[cfg(test)]
mod ordinary_branch_coeffs_geometry_tests;
#[cfg(test)]
mod ordinary_branch_lossless_tests;
#[cfg(test)]
mod ordinary_branch_mode_to_txfm_tests;
#[cfg(test)]
mod ordinary_branch_tx_set_tests;
#[cfg(test)]
mod ordinary_pass_tests;
#[cfg(test)]
mod ordinary_state_context_tests;
#[cfg(test)]
#[path = "coeff_loop/test_support_tests.rs"]
mod test_support;
#[cfg(test)]
#[path = "coeff_loop_tests.rs"]
mod tests;
#[cfg(test)]
mod use_fsc_branch_tests;
