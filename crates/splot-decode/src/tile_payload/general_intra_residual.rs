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
use splot_recon::{
    BitDepth, DequantBlockParams, InverseTransform2dOuter, PlaneId, QuantizerDeltas, ReconError,
    ac_quantizer, dc_quantizer, reconstruct_transform_block_residual,
};

use super::DecodeTileWorkUnit;
use super::cdf::TileCdfSelector;
use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::coeff_loop::ordinary_pass::CoeffOrdinaryBranch;
use super::coeff_loop::ordinary_pass::geometry::CoeffOrdinaryTxSizeGeometryConfig;
use super::coeff_loop::use_fsc_branch::{
    CoeffUseFscBranch, CoeffUseFscBranchError, CoeffUseFscFrameBlockFacts,
    CoeffUseFscFrameFactsInput, CoeffUseFscFrameFactsNonZeroInput, CoeffUseFscFrameOrdinaryFacts,
    apply_coeff_use_fsc_branch_from_frame_facts, coeff_cdf_q_ctx_from_base_q_idx,
};
use super::coeff_state::CoeffContextUpdate;
use super::coeff_state::{TileCoeffContextState, TileCoeffStateError};

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
    /// The decoded `Quant[]` length does not match the adjusted 32x32 transform
    /// block the reconstruction expects.
    #[error("general intra luma reconstruction expected {expected} quant entries, got {actual}")]
    QuantLength {
        /// Expected adjusted-block length.
        expected: usize,
        /// Actual decoded `Quant[]` length.
        actual: usize,
    },
    /// The supplied per-sample prediction buffer length does not match the
    /// original transform-block sample count.
    #[error("general intra reconstruction expected {expected} prediction samples, got {actual}")]
    PredictionLength {
        /// Expected original-block sample count (`orig_side * orig_side`).
        expected: usize,
        /// Actual prediction buffer length.
        actual: usize,
    },
    /// The `splot-recon` dequant / inverse-transform / residual reconstruction
    /// rejected the luma block.
    #[error("general intra luma reconstruction failed: {source}")]
    Reconstruct {
        /// Source reconstruction error.
        source: ReconError,
    },
    /// A § 7.13.2.8 middle-angle directional block requested edges with a real
    /// reconstructed ABOVE neighbour (`haveAbove == 1`). That path needs the real
    /// § 7.13.2.1 corner sample `CurrFrame[plane][y-1][x-1]` (D135 reads the corner
    /// on its main diagonal `column == row`, where `above_base == -1`), which the
    /// current edge builder does not reconstruct. It is gated out (row>0 directional
    /// is deferred) and reached only if that gate is relaxed without first modelling
    /// the corner.
    #[error(
        "general intra directional prediction over a real above-neighbour edge is not yet supported"
    )]
    UnsupportedDirectionalAboveEdge,
    /// A § 7.13.2.8 cardinal directional block (`V_PRED` pAngle 90 / `H_PRED`
    /// pAngle 180) was reached without its required reconstructed neighbour edge:
    /// `V_PRED` needs the real § 7.13.2.1 above row (`haveAbove == 1`), `H_PRED`
    /// needs the real left column (`haveLeft == 1`). The admission gate only
    /// admits these when the edge is present, so this is reached only if that gate
    /// is relaxed without supplying the edge.
    #[error("general intra cardinal directional prediction is missing its required neighbour edge")]
    MissingCardinalEdge,
    /// A cardinal `V_PRED` / `H_PRED` mode reached the § 7.13.2.8 middle-angle
    /// (`90 < pAngle < 180`) mapping, which only covers `D135`. The dispatch routes
    /// cardinal modes to the dedicated copy predictor
    /// (`reconstruct_general_intra_cardinal_neighbour_block_into`), so this is
    /// unreachable in correct operation; it is a defensive guard for a dispatch
    /// regression (returned rather than panicking, per the no-panic policy).
    #[error(
        "general intra cardinal (V_PRED/H_PRED) mode reached the middle-angle path; it must be dispatched to the cardinal copy reconstruction"
    )]
    CardinalModeInMiddleAnglePath,
}

/// OR-reduces a `u32` context line over `[start, start + len)` (clamped to the
/// available range), the AV2 § 8.3.2 above/left level-context reduction.
fn or_u32(line: &[u32], start: usize, len: usize) -> u32 {
    line.iter().skip(start).take(len).fold(0, |acc, &v| acc | v)
}

/// OR-reduces a `u8` DC-context line over `[start, start + len)` (clamped).
fn or_u8(line: &[u8], start: usize, len: usize) -> u8 {
    line.iter().skip(start).take(len).fold(0, |acc, &v| acc | v)
}

fn coeff_ctx_err(source: TileCoeffStateError) -> GeneralIntraResidualError {
    GeneralIntraResidualError::CoeffContextState { source }
}

/// Decodes the AV2 § 5.20.7.27 `coeffs()` syntax for one square transform block
/// of any plane in the general intra multi-block walk.
///
/// The § 8.3.2 `txb_skip` (`all_zero`) context is derived from the persistent
/// neighbour context lines (`context`): luma uses
/// `TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]` with `ctx` from the
/// above/left level OR-reductions; U adds the `+6` chroma offset; V uses
/// `TileVTxbSkipCdf[ctx]` (with the `EobU` term). When `all_zero == 1` the zero
/// context write is committed here; otherwise the nonzero coefficient pass reads
/// `dc_sign` from `context` at `start_x`/`start_y` and commits its own context
/// update internally. `start_x`/`start_y` are the block's plane-sample position.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_general_intra_plane_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    plane: usize,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    eob_u_nonzero: bool,
    uv_mode: usize,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let x4 = start_x >> 2;
    let y4 = start_y >> 2;
    // Square transform: w4 == h4 == Tx_Width[txSz] >> 2 == 1 << tx_size.
    let span4 = 1usize << tx_size;
    let frame_facts = work_unit.coeff_frame_facts();
    let coeff_cdf_q_ctx = coeff_cdf_q_ctx_from_base_q_idx(frame_facts.base_q_idx());
    let tx_size_ctx = txb_skip_tx_size_ctx(tx_size);

    let above_level_or = or_u32(
        context.above_level(plane).map_err(coeff_ctx_err)?,
        x4,
        span4,
    );
    let left_level_or = or_u32(context.left_level(plane).map_err(coeff_ctx_err)?, y4, span4);
    let selector = match plane {
        2 => {
            let above_nz = above_level_or != 0
                || or_u8(context.above_dc(plane).map_err(coeff_ctx_err)?, x4, span4) != 0;
            let left_nz = left_level_or != 0
                || or_u8(context.left_dc(plane).map_err(coeff_ctx_err)?, y4, span4) != 0;
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx: v_txb_skip_ctx(above_nz, left_nz, false, eob_u_nonzero),
            }
        }
        1 => {
            let above_nz = above_level_or != 0
                || or_u8(context.above_dc(plane).map_err(coeff_ctx_err)?, x4, span4) != 0;
            let left_nz = left_level_or != 0
                || or_u8(context.left_dc(plane).map_err(coeff_ctx_err)?, y4, span4) != 0;
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: 0,
                tx_size: tx_size_ctx,
                ctx: usize::from(above_nz) + usize::from(left_nz) + 6,
            }
        }
        _ => TileCdfSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: 0,
            tx_size: tx_size_ctx,
            ctx: txb_skip_ctx_luma(above_level_or, left_level_or, true, false),
        },
    };

    let all_zero = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .read_block_symbol_trace(selector, symbols)
        .map_err(|source| GeneralIntraResidualError::AllZeroRead { source })?
        .get()
        != 0;

    if all_zero {
        context
            .update_after_coeffs(CoeffContextUpdate {
                plane,
                x4,
                y4,
                w4: span4,
                h4: span4,
                cul_level: 0,
                dc_category: 0,
            })
            .map_err(coeff_ctx_err)?;
        return Ok(LumaCoeffBlock {
            all_zero: true,
            eob: 0,
            quant: Vec::new(),
        });
    }

    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane,
        start_x,
        start_y,
        tx_size,
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
            uv_mode,
            angle_delta_uv: 0,
            luma_tx_type: DCT_DCT,
            chroma_inter_tx_type: DCT_DCT,
        },
    });
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let branch = apply_coeff_use_fsc_branch_from_frame_facts(context, cdfs, symbols, input)
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

/// Reconstructs one square intra plane block from the decoded `Quant[]` of its
/// single DC_PRED transform block over a flat DC prediction `dc_sample`.
///
/// This composes the § 7.14.4 dequantization, § 7.15.4 inverse transform, and
/// § 7.14.3 residual addition (`reconstruct_transform_block_residual`) over the
/// flat § 7.13.2 DC prediction (`dc_sample`, derived from the partially-built
/// frame's neighbours, or `128` when none). `qindex == base_q_idx` for this
/// minimal-tool frame (no segmentation or delta-Q), and the transform is
/// `DCT_DCT` over the original `log2_side` (adjusted, capped at 32) dimensions.
/// `use_tcq` adds the § 7.14.4 TCQ `dqDenom` term (luma only).
pub(crate) fn reconstruct_general_intra_block(
    quant: &[i32],
    dc_sample: u8,
    qindex: u32,
    plane_id: PlaneId,
    log2_side: u32,
    use_tcq: bool,
) -> Result<Vec<u8>, GeneralIntraResidualError> {
    let orig_side = 1usize << log2_side;
    let prediction = vec![dc_sample; orig_side * orig_side];
    reconstruct_general_intra_block_with_prediction(
        quant,
        &prediction,
        qindex,
        plane_id,
        log2_side,
        use_tcq,
    )
}

/// Reconstructs one square intra plane block from the decoded `Quant[]` of its
/// single transform block over an arbitrary per-sample `prediction` (§ 7.13.2),
/// composing § 7.14.4 dequantization, § 7.15.4 inverse transform, and § 7.14.3
/// residual addition. `prediction` is the predicted block in raster order over
/// the original (unadjusted) `log2_side` dimensions. The flat DC path is the
/// special case where every prediction sample is the DC value (see
/// [`reconstruct_general_intra_block`]); the non-DC § 7.13.2.13 smooth path
/// supplies a per-sample predicted block.
pub(crate) fn reconstruct_general_intra_block_with_prediction(
    quant: &[i32],
    prediction: &[u8],
    qindex: u32,
    plane_id: PlaneId,
    log2_side: u32,
    use_tcq: bool,
) -> Result<Vec<u8>, GeneralIntraResidualError> {
    let orig_side = 1usize << log2_side;
    let adj_log2 = log2_side.min(5);
    let adj_side = 1usize << adj_log2;
    let adjusted = adj_side * adj_side;
    if quant.len() != adjusted {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: adjusted,
            actual: quant.len(),
        });
    }
    let samples = orig_side * orig_side;
    if prediction.len() != samples {
        return Err(GeneralIntraResidualError::PredictionLength {
            expected: samples,
            actual: prediction.len(),
        });
    }
    let deltas = QuantizerDeltas {
        y_dc: 0,
        u_dc: 0,
        v_dc: 0,
        u_ac: 0,
        v_ac: 0,
    };
    // AV2 §7.14.4: dqDenom = 1 << shift, shift = (pels > 256) + (pels > 1024)
    // over the ORIGINAL (unadjusted) dimensions, plus 1 when TCQ applies (luma
    // DCT_DCT non-lossless non-FSC with allow_tcq; chroma never).
    let pels = (orig_side * orig_side) as u32;
    let dq_shift = u32::from(pels > 256) + u32::from(pels > 1024) + u32::from(use_tcq);
    let dq_denom = 1u32 << dq_shift;
    let params = DequantBlockParams {
        dc_quant: dc_quantizer(plane_id, qindex, deltas, BitDepth::Eight),
        ac_quant: ac_quantizer(plane_id, qindex, deltas, BitDepth::Eight),
        tx_width: adj_side,
        tx_height: adj_side,
        dq_denom,
        bit_depth: BitDepth::Eight,
    };
    let transform = InverseTransform2dOuter::resolve(
        DCT_DCT,
        log2_side,
        log2_side,
        false,
        false,
        BitDepth::Eight,
        None,
    )
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;

    let mut dequant_scratch = vec![0i32; adjusted];
    let mut residual_scratch = vec![0i32; samples];
    let mut out = vec![0u8; samples];
    reconstruct_transform_block_residual(
        prediction,
        quant,
        &params,
        &transform,
        &mut dequant_scratch,
        &mut residual_scratch,
        &mut out,
    )
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;
    Ok(out)
}

/// AV2 § 5.20.7.27 `txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1`
/// over the generated § 9.2 conversion tables, the `txb_skip` CDF transform-size
/// context axis.
fn txb_skip_tx_size_ctx(tx_size: usize) -> usize {
    let sqr = TX_SIZE_SQR.get(tx_size).copied().unwrap_or(0);
    let sqr_up = TX_SIZE_SQR_UP.get(tx_size).copied().unwrap_or(0);
    (((sqr + sqr_up + 1) >> 1).max(0)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstruct_with_prediction_rejects_wrong_prediction_length() {
        // A 4x4 block needs 16 prediction samples; a short buffer is rejected
        // with a structured error (no panic) before reconstruction.
        let quant = vec![0i32; 16];
        let prediction = vec![128u8; 8];
        let result = reconstruct_general_intra_block_with_prediction(
            &quant,
            &prediction,
            64,
            PlaneId::Y,
            2,
            false,
        );
        assert!(matches!(
            result,
            Err(GeneralIntraResidualError::PredictionLength {
                expected: 16,
                actual: 8
            })
        ));
    }

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
