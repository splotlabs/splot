// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient-loop foundation helpers.

use std::collections::TryReserveError;
use std::env;

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;

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
pub(crate) mod base_level_pass;
pub(crate) mod base_symbol;
mod branch;
pub(crate) use branch::{
    CoeffBlockEobBranch, CoeffBlockEobBranchInput, NonZeroCoeffBlockStartInput,
    read_coeff_block_eob_branch,
};
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
    cul_level: u32,
    dc_category: u8,
    block: TransformCoeffBlockState,
}

impl AllZeroCoeffBlock {
    #[must_use]
    pub(crate) const fn eob(&self) -> usize {
        self.eob
    }
    #[must_use]
    pub(crate) const fn cul_level(&self) -> u32 {
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
    let above = bounded_or(state.above_level(LUMA_PLANE)?, input.x4, input.w4);
    let left = bounded_or(state.left_level(LUMA_PLANE)?, input.y4, input.h4);
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
        input.x4,
        input.w4,
    );
    let left = bounded_or_level_dc(
        state.left_level(V_PLANE)?,
        state.left_dc(V_PLANE)?,
        input.y4,
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
    let trace = std::env::var_os("SPLOT_TRACE_COEFF_EOB").is_some();
    let before = trace.then(|| symbols.checkpoint());
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
    let eob_pt = resolved_eob_pt(eob_pt_symbol, eob_pt_extra_width, eob_pt_extra);

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
    if let Some(before) = before {
        eprintln!(
            "coeff eob size={:?} qctx={} eob_ctx={} eob_pt_symbol={} eob_pt_extra_width={} eob_pt_extra={} eob_pt={} eob_extra={} eob_extra_bits={} eob={} checkpoint_before={:?} checkpoint_after={:?}",
            input.size,
            input.coeff_cdf_q_ctx,
            input.eob_ctx,
            eob_pt_symbol,
            eob_pt_extra_width,
            eob_pt_extra,
            eob_pt,
            eob_extra,
            eob_extra_bits,
            eob.eob(),
            before,
            symbols.checkpoint(),
        );
    }
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
    let trace = env::var_os("SPLOT_TRACE_RAW_LITERALS").is_some();
    let before = trace.then(|| symbols.checkpoint());
    let value = symbols
        .read_literal(width)
        .map_err(|source| CoeffLoopContextError::EobLiteralRead { syntax, source })?;
    if let Some(before) = before {
        eprintln!(
            "raw_literal kind=eob syntax={syntax} width={width} value={value} checkpoint_before={before:?} checkpoint_after={:?}",
            symbols.checkpoint(),
        );
    }
    Ok(value)
}

fn bounded_or<T: Copy + Into<u32>>(values: &[T], start: usize, count: usize) -> u32 {
    values.get(start..).map_or(0, |tail| {
        tail.iter()
            .take(count)
            .fold(0, |value, entry| value | (*entry).into())
    })
}

fn bounded_or_level_dc(level: &[u32], dc: &[u8], start: usize, count: usize) -> u32 {
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
mod test_support;
#[cfg(test)]
mod use_fsc_branch_tests;
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
    use super::*;

    fn update(plane: usize, x4: usize, y4: usize, w4: usize, h4: usize) -> CoeffContextUpdate {
        CoeffContextUpdate {
            plane,
            x4,
            y4,
            w4,
            h4,
            cul_level: 4,
            dc_category: 2,
        }
    }

    #[test]
    fn luma_all_zero_context_reads_zero_state_for_first_block() {
        let state = TileCoeffContextState::new(16, 16).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 0,
                y4: 0,
                w4: 16,
                h4: 16,
                tx_fills_block: true,
                fsc_active: false,
            },
        )
        .unwrap();

        assert_eq!(ctx, 0);
    }

    #[test]
    fn luma_all_zero_context_reduces_state_lines_when_not_filling() {
        let mut state = TileCoeffContextState::new(8, 8).unwrap();
        state.update_after_coeffs(update(0, 2, 3, 2, 2)).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 1,
                y4: 2,
                w4: 4,
                h4: 4,
                tx_fills_block: false,
                fsc_active: false,
            },
        )
        .unwrap();

        assert_eq!(ctx, 5);
    }

    #[test]
    fn luma_all_zero_context_fsc_overrides_state() {
        let mut state = TileCoeffContextState::new(4, 4).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 4, 4)).unwrap();
        let ctx = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: 0,
                y4: 0,
                w4: 4,
                h4: 4,
                tx_fills_block: true,
                fsc_active: true,
            },
        )
        .unwrap();

        assert_eq!(ctx, 9);
    }

    #[test]
    fn v_all_zero_context_combines_level_dc_state_and_geometry() {
        let mut state = TileCoeffContextState::new(8, 8).unwrap();
        state.update_after_coeffs(update(2, 2, 5, 2, 1)).unwrap();
        let ctx = v_all_zero_context(
            &state,
            VAllZeroContextInput {
                x4: 1,
                y4: 4,
                w4: 4,
                h4: 3,
                chroma_block_larger_than_tx: true,
                eob_u_nonzero: true,
            },
        )
        .unwrap();

        assert_eq!(ctx, 11);
    }

    #[test]
    fn all_zero_context_reductions_bound_out_of_range_and_pathological_counts() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(2, 0, 0, 1, 1)).unwrap();

        let luma = luma_all_zero_context(
            &state,
            LumaAllZeroContextInput {
                x4: usize::MAX,
                y4: usize::MAX,
                w4: usize::MAX,
                h4: usize::MAX,
                tx_fills_block: false,
                fsc_active: false,
            },
        )
        .unwrap();
        let v = v_all_zero_context(
            &state,
            VAllZeroContextInput {
                x4: usize::MAX,
                y4: usize::MAX,
                w4: usize::MAX,
                h4: usize::MAX,
                chroma_block_larger_than_tx: false,
                eob_u_nonzero: false,
            },
        )
        .unwrap();

        assert_eq!(luma, 1);
        assert_eq!(v, 0);
    }

    #[test]
    fn all_zero_coeff_block_applies_zero_state_and_context_writes() {
        let mut state = TileCoeffContextState::new(6, 6).unwrap();
        state.update_after_coeffs(update(0, 1, 2, 3, 2)).unwrap();

        let applied = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 1,
                y4: 2,
                w4: 3,
                h4: 2,
            },
        )
        .unwrap();

        assert_eq!(applied.eob(), 0);
        assert_eq!(applied.cul_level(), 0);
        assert_eq!(applied.dc_category(), 0);
        assert_eq!(applied.block().width(), 12);
        assert_eq!(applied.block().height(), 8);
        assert!(applied.block().level().iter().all(|level| *level == 0));
        assert!(applied.block().quant_sign().iter().all(|sign| *sign == 0));
        assert!(applied.block().quant().iter().all(|quant| *quant == 0));
        assert_eq!(state.above_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.above_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.left_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
        assert_eq!(state.left_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn all_zero_coeff_block_rejects_bad_ranges_without_mutation() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
        let before = state.clone();

        let err = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 2,
                y4: 0,
                w4: 1,
                h4: 1,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::State(TileCoeffStateError::ContextRangeOutOfBounds {
                context: "above",
                start: 2,
                end: 3,
                len: 2
            })
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn all_zero_coeff_block_rejects_zero_transform_extent_before_mutation() {
        let mut state = TileCoeffContextState::new(2, 2).unwrap();
        state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
        let before = state.clone();

        let err = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 0,
                x4: 0,
                y4: 0,
                w4: 0,
                h4: 1,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::State(TileCoeffStateError::InvalidAdjustedTransformExtent {
                axis: "width",
                value: 0
            })
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn all_zero_coeff_block_saturates_adjusted_extent_to_spec_cap() {
        let mut state = TileCoeffContextState::new(16, 16).unwrap();

        let applied = apply_all_zero_coeff_block(
            &mut state,
            AllZeroCoeffBlockInput {
                plane: 2,
                x4: 0,
                y4: 0,
                w4: 16,
                h4: 16,
            },
        )
        .unwrap();

        assert_eq!(applied.block().width(), 32);
        assert_eq!(applied.block().height(), 32);
        assert_eq!(applied.block().quant().len(), 1024);
    }

    #[test]
    fn nonzero_coeff_eob_maps_small_points_without_refinements() {
        let eob_one = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 1,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap();
        let eob_two = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 2,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap();

        assert_eq!(eob_one.eob_pt(), 1);
        assert_eq!(eob_one.eob(), 1);
        assert_eq!(eob_two.eob_pt(), 2);
        assert_eq!(eob_two.eob(), 2);
    }

    #[test]
    fn nonzero_coeff_eob_applies_eob_extra_and_refinement_bits() {
        let eob = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 6,
            eob_extra: true,
            eob_extra_bits: 0b110,
        })
        .unwrap();

        assert_eq!(eob.eob(), 31);
    }

    #[test]
    fn nonzero_coeff_eob_reaches_max_av2_eob() {
        let eob = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 11,
            eob_extra: true,
            eob_extra_bits: 0xFF,
        })
        .unwrap();

        assert_eq!(eob.eob(), 1024);
    }

    #[test]
    fn nonzero_coeff_eob_rejects_invalid_eob_points() {
        let zero = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 0,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap_err();
        let oversized = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 12,
            eob_extra: false,
            eob_extra_bits: 0,
        })
        .unwrap_err();

        assert!(matches!(
            zero,
            CoeffLoopContextError::InvalidEobPoint { eob_pt: 0 }
        ));
        assert!(matches!(
            oversized,
            CoeffLoopContextError::InvalidEobPoint { eob_pt: 12 }
        ));
    }

    #[test]
    fn nonzero_coeff_eob_rejects_refinements_for_small_points() {
        let err = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 1,
            eob_extra: true,
            eob_extra_bits: 0,
        })
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::UnexpectedEobRefinement {
                eob_pt: 1,
                eob_extra: true,
                eob_extra_bits: 0
            }
        ));
    }

    #[test]
    fn nonzero_coeff_eob_rejects_out_of_range_refinement_bits() {
        let err = nonzero_coeff_eob(NonZeroCoeffEobInput {
            eob_pt: 4,
            eob_extra: false,
            eob_extra_bits: 0b10,
        })
        .unwrap_err();

        assert!(matches!(
            err,
            CoeffLoopContextError::EobExtraBitsOutOfRange {
                eob_pt: 4,
                eob_extra_bits: 2,
                max_eob_extra_bits: 1
            }
        ));
    }
}
