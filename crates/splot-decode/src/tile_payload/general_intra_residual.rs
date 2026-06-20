// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra luma transform-block coefficient decode for the AVM-oracle
//! general intra path.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-LUMA-COEFFS`.
//!
//! This decodes the AV2 § 5.20.7.27 `coeffs()` syntax for the single luma
//! transform block of a minimal-tool intra key frame: it reads the `all_zero`
//! (`txb_skip`) symbol with the spec-derived § 8.3.2 context, and when
//! `all_zero == 0` routes the nonzero coefficient pass through the existing
//! coefficient-loop machinery to produce the decoded `Quant[]` and end-of-block.
//!
//! Scope: the single non-partitioned 64x64 luma transform block at the tile
//! origin (`tx_size == TX_64X64`, `PlaneTxType == DCT_DCT`). The chroma
//! transform blocks, dequantization, inverse transform, residual addition,
//! reconstruction, output, the general transform-block partition walk, and tile
//! context-line persistence remain future increments. The decoded coefficients
//! are returned to the caller (which currently still reports the reconstruction
//! step unsupported) rather than committed to runtime decode state.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{TX_SIZE_SQR, TX_SIZE_SQR_UP};

use super::DecodeTileWorkUnit;
use super::cdf::TileCdfSelector;
use super::cdf::block_context::txb_skip_ctx_luma;
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::coeff_loop::ordinary_pass::CoeffOrdinaryBranch;
use super::coeff_loop::ordinary_pass::geometry::CoeffOrdinaryTxSizeGeometryConfig;
use super::coeff_loop::use_fsc_branch::{
    CoeffUseFscBranch, CoeffUseFscBranchError, CoeffUseFscFrameBlockFacts,
    CoeffUseFscFrameFactsInput, CoeffUseFscFrameFactsNonZeroInput, CoeffUseFscFrameOrdinaryFacts,
    apply_coeff_use_fsc_branch_from_frame_facts, coeff_cdf_q_ctx_from_base_q_idx,
};
use super::coeff_state::{TileCoeffContextState, TileCoeffStateError};
use super::general_intra_block::GeneralIntraBlockModes;

/// AV2 § 9.2 `TX_SIZES_ALL` index of `TX_64X64` (the single non-partitioned
/// luma transform size for a 64x64 intra block with transform partitioning
/// disabled).
const TX_64X64: usize = 4;
/// `DCT_DCT` `PlaneTxType` (AV2 § 5.20.7.29 implies `DCT_DCT` for this
/// minimal-tool intra block; no `intra_tx_type` symbol is coded).
const DCT_DCT: usize = 0;
/// The single default segment id for the minimal-tool intra block.
const SEGMENT_ID: usize = 0;

/// The decoded luma transform-block coefficient facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LumaCoeffBlock {
    /// `all_zero` (`txb_skip`) decision: `true` when the block is skipped.
    pub(crate) all_zero: bool,
    /// Decoded end-of-block (number of coded coefficients); `0` when skipped.
    pub(crate) eob: usize,
    /// Decoded row-major `Quant[]` for the adjusted transform block; empty when
    /// skipped.
    pub(crate) quant: Vec<i32>,
}

/// Error returned while decoding the general intra luma coefficient block.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraResidualError {
    /// The `all_zero` (`txb_skip`) symbol read failed.
    #[error("general intra luma all_zero symbol read failed: {source}")]
    AllZeroRead {
        /// Source CDF selection or symbol-decoder error.
        source: BlockSymbolTraceReadError,
    },
    /// The tile coefficient context state could not be allocated.
    #[error("general intra luma coefficient context state failed: {source}")]
    CoeffContextState {
        /// Source coefficient context state error.
        source: TileCoeffStateError,
    },
    /// The tile work-unit MI range is degenerate.
    #[error("general intra luma coefficient context {axis} range {start}..{end} is invalid")]
    InvalidContextRange {
        /// Range axis.
        axis: &'static str,
        /// Range start.
        start: u32,
        /// Range end.
        end: u32,
    },
    /// The nonzero coefficient pass failed.
    #[error("general intra luma nonzero coefficient pass failed: {source}")]
    NonZeroPass {
        /// Source coefficient branch error.
        source: CoeffUseFscBranchError,
    },
    /// The nonzero coefficient pass returned an unexpected branch result (FSC or
    /// all-zero) for the ordinary luma block.
    #[error("general intra luma nonzero coefficient pass produced an unexpected branch result")]
    UnexpectedBranch,
}

/// Decodes the AV2 § 5.20.7.27 `coeffs()` syntax for the single luma 64x64
/// transform block, returning the `all_zero` decision and, when coded, the
/// decoded `Quant[]` and end-of-block.
pub(crate) fn decode_general_intra_luma_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    modes: GeneralIntraBlockModes,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let frame_facts = work_unit.coeff_frame_facts();
    let coeff_cdf_q_ctx = coeff_cdf_q_ctx_from_base_q_idx(frame_facts.base_q_idx());
    let tx_size_ctx = txb_skip_tx_size_ctx(TX_64X64);
    // First transform block of the tile: the level context is zero and the
    // 64x64 transform fills its residual block, so § 8.3.2 `txb_skip` luma
    // context reduces to 0.
    let txb_skip_ctx = txb_skip_ctx_luma(0, 0, true, false);

    let mut context = minimal_tile_coeff_context(work_unit)?;
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();

    // all_zero (§ 5.20.7.27): `TileTxbSkipCdf[q][plane_type][txSzCtx][ctx]`.
    let all_zero = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: 0,
                tx_size: tx_size_ctx,
                ctx: txb_skip_ctx,
            },
            symbols,
        )
        .map_err(|source| GeneralIntraResidualError::AllZeroRead { source })?
        .get()
        != 0;

    if all_zero {
        return Ok(LumaCoeffBlock {
            all_zero: true,
            eob: 0,
            quant: Vec::new(),
        });
    }

    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane: 0,
        start_x: 0,
        start_y: 0,
        tx_size: TX_64X64,
    };
    let input = CoeffUseFscFrameFactsInput::NonZero(CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts,
        block: CoeffUseFscFrameBlockFacts {
            geometry,
            plane_tx_type: DCT_DCT,
            fsc_mode: false,
            is_inter: false,
            segment_id: SEGMENT_ID,
        },
        ordinary: CoeffUseFscFrameOrdinaryFacts {
            uv_mode: usize::from(modes.uv_mode),
            angle_delta_uv: 0,
            luma_tx_type: DCT_DCT,
            chroma_inter_tx_type: DCT_DCT,
        },
    });

    let branch = apply_coeff_use_fsc_branch_from_frame_facts(&mut context, cdfs, symbols, input)
        .map_err(|source| GeneralIntraResidualError::NonZeroPass { source })?;

    let CoeffUseFscBranch::Ordinary(CoeffOrdinaryBranch::NonZero(pass)) = branch else {
        return Err(GeneralIntraResidualError::UnexpectedBranch);
    };

    Ok(LumaCoeffBlock {
        all_zero: false,
        eob: pass.eob_read().eob().eob(),
        quant: pass.block().quant().to_vec(),
    })
}

/// AV2 § 5.20.7.27 `txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1`
/// over the generated § 9.2 conversion tables, the `txb_skip` CDF transform-size
/// context axis.
fn txb_skip_tx_size_ctx(tx_size: usize) -> usize {
    let sqr = TX_SIZE_SQR.get(tx_size).copied().unwrap_or(0);
    let sqr_up = TX_SIZE_SQR_UP.get(tx_size).copied().unwrap_or(0);
    (((sqr + sqr_up + 1) >> 1).max(0)) as usize
}

fn minimal_tile_coeff_context(
    work_unit: &DecodeTileWorkUnit<'_>,
) -> Result<TileCoeffContextState, GeneralIntraResidualError> {
    let mi_rows = range_len("rows", work_unit.mi_row_range())?;
    let mi_cols = range_len("columns", work_unit.mi_col_range())?;
    TileCoeffContextState::new(mi_rows, mi_cols)
        .map_err(|source| GeneralIntraResidualError::CoeffContextState { source })
}

fn range_len(
    axis: &'static str,
    range: core::ops::Range<u32>,
) -> Result<usize, GeneralIntraResidualError> {
    let length = range.end.checked_sub(range.start).ok_or(
        GeneralIntraResidualError::InvalidContextRange {
            axis,
            start: range.start,
            end: range.end,
        },
    )?;
    Ok(length as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn txb_skip_tx_size_ctx_matches_spec_formula_for_square_sizes() {
        // txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1.
        // TX_4X4 (0): (0 + 0 + 1) >> 1 == 0.
        assert_eq!(txb_skip_tx_size_ctx(0), 0);
        // TX_8X8 (1): (1 + 1 + 1) >> 1 == 1.
        assert_eq!(txb_skip_tx_size_ctx(1), 1);
        // TX_16X16 (2): (2 + 2 + 1) >> 1 == 2.
        assert_eq!(txb_skip_tx_size_ctx(2), 2);
        // TX_32X32 (3): (3 + 3 + 1) >> 1 == 3.
        assert_eq!(txb_skip_tx_size_ctx(3), 3);
        // TX_64X64 (4): (4 + 4 + 1) >> 1 == 4 (the q80 single-block luma size).
        assert_eq!(txb_skip_tx_size_ctx(TX_64X64), 4);
    }

    #[test]
    fn txb_skip_tx_size_ctx_is_total_for_out_of_range_tx_size() {
        // Out-of-range indices saturate to 0 rather than panicking.
        assert_eq!(txb_skip_tx_size_ctx(usize::MAX), 0);
        assert_eq!(txb_skip_tx_size_ctx(TX_SIZE_SQR.len()), 0);
    }
}
