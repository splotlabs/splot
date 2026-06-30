// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra transform-block coefficient decode.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MD_IDX_TO_TYPE, MODE_TO_ANGLE, SIZE_CLASS, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR,
    TX_SIZE_SQR_UP, TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::{
    BitDepth, DequantBlockParams, InverseTransform2dOuter, PlaneId, QuantizerDeltas, ReconError,
    ReconSample, ac_quantizer, dc_quantizer, reconstruct_transform_block_residual,
};

use super::cdf::TileCdfSelector;
use super::cdf::block_context::IntraYMode;
use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::coeff_loop::fsc_quant_pass::{
    CoeffFscBranchError, CoeffFscStagedTxSizeNonZeroInput,
    apply_staged_nonzero_coeff_fsc_branch_from_tx_size,
};
use super::coeff_loop::max_level::CoeffTransformClass;
use super::coeff_loop::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryStagedLosslessNonZeroInput,
    CoeffOrdinaryTxSizeGeometryConfig, apply_staged_nonzero_coeff_ordinary_branch_from_lossless,
};
use super::coeff_loop::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};
use super::coeff_loop::use_fsc_branch::{
    CoeffUseFscBranch, CoeffUseFscBranchError, CoeffUseFscFrameBlockFacts,
    CoeffUseFscFrameFactsInput, CoeffUseFscFrameFactsNonZeroInput, CoeffUseFscFrameOrdinaryFacts,
    apply_coeff_use_fsc_branch_from_frame_facts, coeff_cdf_q_ctx_from_base_q_idx,
};
use super::coeff_loop::{
    AllZeroCoeffBlockInput, CoeffBlockEobBranch, CoeffBlockEobBranchInput, CoeffLoopContextError,
    NonZeroCoeffBlockStartInput, NonZeroCoeffEobContextInput, read_coeff_block_eob_branch,
};
use super::coeff_state::CoeffContextUpdate;
use super::coeff_state::{TileCoeffContextState, TileCoeffStateError};
use super::{DecodeTileWorkUnit, TileCdfSubset, TileCoeffFrameFacts};

const TX_64X64: usize = 4;
const TX_8X8: usize = 1;
const TX_16X16: usize = 2;
const TX_32X32: usize = 3;
const TX_8X16: usize = 7;
const IST_4X4_HEIGHT: usize = 8;
const IST_8X8_HEIGHT_RED: usize = 20;
const IST_8X8_HEIGHT: usize = 32;
const ANGLE_STEP: i32 = 3;
const MRL_INDEX_TO_DELTA: [i32; 4] = [0, 1, -1, 0];
const DCT_DCT: usize = 0;
const ADST_DCT: usize = 1;
const DCT_ADST: usize = 2;
const ADST_ADST: usize = 3;
const FLIPADST_DCT: usize = 4;
const DCT_FLIPADST: usize = 5;
const FLIPADST_FLIPADST: usize = 6;
const ADST_FLIPADST: usize = 7;
const FLIPADST_ADST: usize = 8;
const IDTX: usize = 9;
const V_DCT: usize = 10;
const H_DCT: usize = 11;
const V_ADST: usize = 12;
const H_ADST: usize = 13;
const V_FLIPADST: usize = 14;
const H_FLIPADST: usize = 15;
const D45_PRED: usize = 3;
const D203_PRED: usize = 7;
const SEGMENT_ID: usize = 0;
const TX_SET_DCTONLY: usize = 0;
const TX_SET_WIDE_64: usize = 1;
const TX_SET_HIGH_64: usize = 2;
const TX_SET_WIDE_32: usize = 3;
const TX_SET_HIGH_32: usize = 4;
const TX_SET_INTRA_1: usize = 5;
const TX_SET_INTRA_2: usize = 6;
const TX_SET_INTER_1: usize = 5;
const TX_SET_INTER_2: usize = 6;
const TX_SET_DCT_IDTX: usize = 7;
const TX_SET_DCT_IDTX_IDDCT: usize = 8;
const WAIP_WH_RATIO_THRESHOLDS: [(usize, i32); 4] = [(2, 61), (4, 73), (8, 82), (16, 86)];
const TX_TYPE_INV_LONG: [[[usize; 4]; 2]; 2] = [
    [
        [V_DCT, V_ADST, V_FLIPADST, IDTX],
        [H_DCT, H_ADST, H_FLIPADST, IDTX],
    ],
    [
        [DCT_DCT, ADST_DCT, FLIPADST_DCT, H_DCT],
        [DCT_DCT, DCT_ADST, DCT_FLIPADST, V_DCT],
    ],
];

const TX_TYPE_INTER_INV_SET1: [usize; 16] = [
    IDTX,
    V_DCT,
    H_DCT,
    V_ADST,
    H_ADST,
    V_FLIPADST,
    H_FLIPADST,
    DCT_DCT,
    ADST_DCT,
    DCT_ADST,
    FLIPADST_DCT,
    DCT_FLIPADST,
    ADST_ADST,
    FLIPADST_FLIPADST,
    ADST_FLIPADST,
    FLIPADST_ADST,
];

const TX_TYPE_INTER_INV_SET2: [usize; 12] = [
    IDTX,
    V_DCT,
    H_DCT,
    DCT_DCT,
    ADST_DCT,
    DCT_ADST,
    FLIPADST_DCT,
    DCT_FLIPADST,
    ADST_ADST,
    FLIPADST_FLIPADST,
    ADST_FLIPADST,
    FLIPADST_ADST,
];

const TX_TYPE_INTER_INV_SET3: [usize; 2] = [IDTX, DCT_DCT];
const TX_TYPE_INTER_INV_SET4: [usize; 4] = [DCT_DCT, V_DCT, H_DCT, IDTX];
const INTER_TX_TYPE_INDEX_COUNT: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaTransformTypeContext {
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
}

impl LumaTransformTypeContext {
    #[must_use]
    pub(crate) const fn new(y_mode: IntraYMode, angle_delta_y: i8) -> Self {
        Self {
            y_mode,
            angle_delta_y,
            mrl_index: 0,
        }
    }

    #[must_use]
    pub(crate) const fn with_mrl_index(
        y_mode: IntraYMode,
        angle_delta_y: i8,
        mrl_index: u8,
    ) -> Self {
        Self {
            y_mode,
            angle_delta_y,
            mrl_index,
        }
    }

    #[must_use]
    pub(crate) const fn mrl_index(self) -> u8 {
        self.mrl_index
    }

    #[must_use]
    pub(crate) const fn angle_delta_y(self) -> i8 {
        self.angle_delta_y
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransformToolResidualPolicy {
    Allow,
    AdmitTransformToolSubset {
        luma: Option<LumaTransformTypeContext>,
        active_intra_ist: ActiveIntraIstResidualPolicy,
        active_chroma: ActiveChromaResidualPolicy,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveIntraIstResidualPolicy {
    Reject,
    LrTxSkipRecordHandoff,
}

impl ActiveIntraIstResidualPolicy {
    const fn allows_record_handoff(self) -> bool {
        matches!(self, Self::LrTxSkipRecordHandoff)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveChromaResidualPolicy {
    Reject,
    LrTxSkipRecordHandoff,
}

impl ActiveChromaResidualPolicy {
    const fn allows_record_handoff(self) -> bool {
        matches!(self, Self::LrTxSkipRecordHandoff)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraIstSyntax {
    pub(crate) sec_tx_type: usize,
    pub(crate) most_probable_stx_set: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LumaCoeffBlock {
    pub(crate) all_zero: bool,
    pub(crate) eob: usize,
    pub(crate) quant: Vec<i32>,
    pub(crate) intra_ist: Option<IntraIstSyntax>,
    pub(crate) plane_tx_type: usize,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraResidualError {
    #[error("general intra luma all_zero symbol read failed: {source}")]
    AllZeroRead { source: BlockSymbolTraceReadError },
    #[error("general intra luma coefficient context state failed: {source}")]
    CoeffContextState { source: TileCoeffStateError },
    #[error("general intra luma nonzero coefficient pass failed: {source}")]
    NonZeroPass { source: CoeffUseFscBranchError },
    #[error("general intra luma staged nonzero EOB read failed: {source}")]
    NonZeroStart { source: CoeffLoopContextError },
    #[error("general intra luma staged nonzero coefficient pass failed: {source}")]
    StagedNonZeroPass { source: CoeffOrdinaryBranchError },
    #[error("general intra luma staged FSC coefficient pass failed: {source}")]
    StagedFscPass { source: CoeffFscBranchError },
    #[error("general intra luma transform_type symbol read failed: {source}")]
    TransformTypeRead { source: BlockSymbolTraceReadError },
    #[error("general intra residual requires unsupported active transform-tool syntax: {reason}")]
    UnsupportedTransformToolResidual { reason: &'static str },
    #[error("general intra luma nonzero coefficient pass produced an unexpected branch result")]
    UnexpectedBranch,
    #[error("general intra luma reconstruction expected {expected} quant entries, got {actual}")]
    QuantLength { expected: usize, actual: usize },
    #[error("general intra reconstruction expected {expected} prediction samples, got {actual}")]
    PredictionLength { expected: usize, actual: usize },
    #[error("general intra luma reconstruction failed: {source}")]
    Reconstruct {
        #[from]
        source: ReconError,
    },
    #[error(
        "general intra directional prediction over a real above-neighbour edge is missing its §7.13.2.1 corner sample"
    )]
    UnsupportedDirectionalAboveEdge,
    #[error("general intra cardinal directional prediction is missing its required neighbour edge")]
    MissingCardinalEdge,
    #[error(
        "general intra cardinal (V_PRED/H_PRED) mode reached the middle-angle path; it must be dispatched to the cardinal copy reconstruction"
    )]
    CardinalModeInMiddleAnglePath,
}

fn or_u32(line: &[u32], start: usize, len: usize) -> u32 {
    line.iter().skip(start).take(len).fold(0, |acc, &v| acc | v)
}

fn or_u8(line: &[u8], start: usize, len: usize) -> u8 {
    line.iter().skip(start).take(len).fold(0, |acc, &v| acc | v)
}

fn coeff_ctx_err(source: TileCoeffStateError) -> GeneralIntraResidualError {
    GeneralIntraResidualError::CoeffContextState { source }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub(crate) fn decode_general_intra_plane_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    plane: usize,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    tx_fills_block: bool,
    eob_u_nonzero: bool,
    uv_mode: usize,
    is_inter: bool,
    fsc_mode: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let x4 = start_x >> 2;
    let y4 = start_y >> 2;
    let w4 = usize::try_from(TX_WIDTH.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let h4 = usize::try_from(TX_HEIGHT.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let frame_facts = work_unit.coeff_frame_facts();
    let coeff_cdf_q_ctx = coeff_cdf_q_ctx_from_base_q_idx(frame_facts.base_q_idx());
    let tx_size_ctx = txb_skip_tx_size_ctx(tx_size);

    let above_level_or = or_u32(context.above_level(plane).map_err(coeff_ctx_err)?, x4, w4);
    let left_level_or = or_u32(context.left_level(plane).map_err(coeff_ctx_err)?, y4, h4);
    let txb_skip_intra_inter = usize::from(is_inter || fsc_mode);
    let selector = match plane {
        1 | 2 => {
            let above_nz = above_level_or != 0
                || or_u8(context.above_dc(plane).map_err(coeff_ctx_err)?, x4, w4) != 0;
            let left_nz = left_level_or != 0
                || or_u8(context.left_dc(plane).map_err(coeff_ctx_err)?, y4, h4) != 0;
            if plane == 2 {
                TileCdfSelector::VTxbSkip {
                    coeff_cdf_q_ctx,
                    ctx: v_txb_skip_ctx(above_nz, left_nz, !tx_fills_block, eob_u_nonzero),
                }
            } else {
                TileCdfSelector::TxbSkip {
                    coeff_cdf_q_ctx,
                    plane_type: txb_skip_intra_inter,
                    tx_size: tx_size_ctx,
                    ctx: usize::from(above_nz) + usize::from(left_nz) + 6,
                }
            }
        }
        _ => TileCdfSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: txb_skip_intra_inter,
            tx_size: tx_size_ctx,
            ctx: txb_skip_ctx_luma(
                above_level_or,
                left_level_or,
                tx_fills_block,
                fsc_mode && frame_facts.enable_fsc(),
            ),
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
                w4,
                h4,
                cul_level: 0,
                dc_category: 0,
            })
            .map_err(coeff_ctx_err)?;
        return Ok(LumaCoeffBlock {
            all_zero: true,
            eob: 0,
            quant: Vec::new(),
            intra_ist: None,
            plane_tx_type: DCT_DCT,
        });
    }

    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane,
        start_x,
        start_y,
        tx_size,
    };
    if let TransformToolResidualPolicy::AdmitTransformToolSubset {
        luma,
        active_intra_ist,
        active_chroma,
    } = transform_tool_residual_policy
    {
        return decode_staged_transform_tool_nonzero_coeffs(
            work_unit,
            symbols,
            context,
            frame_facts,
            geometry,
            coeff_cdf_q_ctx,
            uv_mode,
            is_inter,
            fsc_mode,
            luma,
            active_intra_ist,
            active_chroma,
        );
    }

    let input = CoeffUseFscFrameFactsInput::NonZero(CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts,
        block: CoeffUseFscFrameBlockFacts {
            geometry,
            plane_tx_type: DCT_DCT,
            fsc_mode,
            is_inter,
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
        intra_ist: None,
        plane_tx_type: DCT_DCT,
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_staged_transform_tool_nonzero_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    frame_facts: TileCoeffFrameFacts,
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    coeff_cdf_q_ctx: usize,
    uv_mode: usize,
    is_inter: bool,
    fsc_mode: bool,
    luma_transform_type_context: Option<LumaTransformTypeContext>,
    active_intra_ist_policy: ActiveIntraIstResidualPolicy,
    active_chroma_policy: ActiveChromaResidualPolicy,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let tx_width_log2 = tx_size_table_usize(&TX_WIDTH_LOG2, "Tx_Width_Log2", geometry.tx_size)?;
    let tx_height_log2 = tx_size_table_usize(&TX_HEIGHT_LOG2, "Tx_Height_Log2", geometry.tx_size)?;
    let (tx_width, tx_height) = tx_size_dimensions(geometry.tx_size)?;
    let block = AllZeroCoeffBlockInput {
        plane: geometry.plane,
        x4: geometry.start_x >> 2,
        y4: geometry.start_y >> 2,
        w4: tx_width >> 2,
        h4: tx_height >> 2,
    };
    let start = match read_coeff_block_eob_branch(
        context,
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        CoeffBlockEobBranchInput::NonZero(NonZeroCoeffBlockStartInput {
            block,
            eob: NonZeroCoeffEobContextInput {
                plane: geometry.plane,
                is_inter,
                tx_width_log2,
                tx_height_log2,
                coeff_cdf_q_ctx,
            },
        }),
    )
    .map_err(|source| GeneralIntraResidualError::NonZeroStart { source })?
    {
        CoeffBlockEobBranch::NonZero(start) => start,
        CoeffBlockEobBranch::AllZero(_) => {
            return Err(GeneralIntraResidualError::UnexpectedBranch);
        }
    };
    let eob = start.eob_read().eob().eob();
    let metadata = ensure_transform_tool_residual_handoff(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        TransformToolResidualInput {
            frame_facts,
            plane: geometry.plane,
            tx_size: geometry.tx_size,
            is_inter,
            fsc_mode,
            eob,
            luma_transform_type_context,
            active_intra_ist_policy,
            active_chroma_policy,
        },
    )?;
    let lossless = frame_facts.lossless_for_segment(SEGMENT_ID).ok_or(
        unsupported_transform_tool_residual_error("unsupported_dctonly_residual_segment_id"),
    )?;
    let base_config = staged_transform_tool_lossless_base_config(
        frame_facts,
        geometry.plane,
        uv_mode,
        lossless,
        metadata,
    );
    let use_fsc = frame_facts.enable_fsc()
        && metadata.luma_tx_type == IDTX
        && geometry.plane == 0
        && (fsc_mode || is_inter);
    if use_fsc {
        let pass = apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
            context,
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            CoeffFscStagedTxSizeNonZeroInput {
                block,
                start,
                tx_size: geometry.tx_size,
                plane_tx_type: metadata.luma_tx_type,
                coeff_cdf_q_ctx,
            },
        )
        .map_err(|source| GeneralIntraResidualError::StagedFscPass { source })?;
        return Ok(LumaCoeffBlock {
            all_zero: false,
            eob: pass.eob_read().eob().eob(),
            quant: pass.block().quant().to_vec(),
            intra_ist: metadata.intra_ist,
            plane_tx_type: metadata.luma_tx_type,
        });
    }
    let pass = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        context,
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        CoeffOrdinaryStagedLosslessNonZeroInput {
            geometry,
            start,
            coeff_cdf_q_ctx,
            is_inter,
            base_config,
            lossless,
        },
    )
    .map_err(|source| GeneralIntraResidualError::StagedNonZeroPass { source })?;
    Ok(LumaCoeffBlock {
        all_zero: false,
        eob,
        quant: pass.block().quant().to_vec(),
        intra_ist: metadata.intra_ist,
        plane_tx_type: metadata.luma_tx_type,
    })
}

fn staged_transform_tool_lossless_base_config(
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    uv_mode: usize,
    lossless: bool,
    metadata: TransformToolResidualMetadata,
) -> CoeffOrdinaryBranchLosslessBaseConfig {
    let luma_tx_class = CoeffTransformClass::from_plane_tx_type(metadata.luma_tx_type);
    let luma_transform_block = plane == 0;
    let parity_hiding = frame_facts.allow_parity_hiding()
        && !lossless
        && luma_transform_block
        && metadata.luma_tx_type != IDTX;
    let use_tcq = frame_facts.allow_tcq()
        && !lossless
        && luma_transform_block
        && luma_tx_class == CoeffTransformClass::TwoD;
    CoeffOrdinaryBranchLosslessBaseConfig {
        reduced_tx_set: frame_facts.reduced_tx_set(),
        enable_chroma_dctonly: frame_facts.enable_chroma_dctonly(),
        uv_mode,
        angle_delta_uv: 0,
        luma_tx_type: metadata.luma_tx_type,
        chroma_inter_tx_type: DCT_DCT,
        parity_hiding,
        use_tcq,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TransformToolResidualMetadata {
    intra_ist: Option<IntraIstSyntax>,
    luma_tx_type: usize,
    cctx_type: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransformToolResidualInput {
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    fsc_mode: bool,
    eob: usize,
    luma_transform_type_context: Option<LumaTransformTypeContext>,
    active_intra_ist_policy: ActiveIntraIstResidualPolicy,
    active_chroma_policy: ActiveChromaResidualPolicy,
}

fn ensure_transform_tool_residual_handoff(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: TransformToolResidualInput,
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    let frame_facts = input.frame_facts;
    let plane = input.plane;
    let tx_size = input.tx_size;
    let is_inter = input.is_inter;
    let eob = input.eob;
    let mut metadata = TransformToolResidualMetadata {
        luma_tx_type: DCT_DCT,
        ..TransformToolResidualMetadata::default()
    };
    if frame_facts.lossless_for_segment(SEGMENT_ID) != Some(false) {
        return unsupported_transform_tool_residual("unsupported_dctonly_residual_lossless");
    }
    if !is_inter && plane == 1 && frame_facts.enable_cctx() && eob != 1 {
        metadata.cctx_type = Some(read_chroma_cctx_type(
            cdfs,
            symbols,
            input.active_chroma_policy,
        )?);
    }
    let tx_set = transform_set(frame_facts, plane, tx_size, is_inter)?;
    let dct_forced = (!is_inter && plane == 0 && eob == 1)
        || (plane > 0 && frame_facts.enable_chroma_dctonly())
        || tx_set == TX_SET_DCTONLY
        || (!is_inter && plane == 0 && frame_facts.reduced_tx_set() == 2);
    if !is_inter && plane == 0 && input.fsc_mode {
        metadata.luma_tx_type = IDTX;
    } else if !dct_forced {
        if !is_inter && plane > 0 && input.active_chroma_policy.allows_record_handoff() {
        } else if !is_inter && plane == 0 {
            let luma_tx_type = read_active_luma_transform_type(
                cdfs,
                symbols,
                input.luma_transform_type_context,
                tx_size,
                tx_set,
            )?;
            if luma_tx_type != DCT_DCT && !input.active_intra_ist_policy.allows_record_handoff() {
                return unsupported_transform_tool_residual(
                    "unsupported_dctonly_residual_luma_tx_type",
                );
            }
            metadata.luma_tx_type = luma_tx_type;
        } else if is_inter && plane == 0 {
            let luma_tx_type =
                read_active_inter_transform_type(cdfs, symbols, tx_size, tx_set, eob)?;
            if luma_tx_type != DCT_DCT && !input.active_intra_ist_policy.allows_record_handoff() {
                return unsupported_transform_tool_residual(
                    "unsupported_dctonly_residual_inter_tx_type",
                );
            }
            metadata.luma_tx_type = luma_tx_type;
        } else {
            return unsupported_transform_tool_residual("unsupported_dctonly_residual_tx_set");
        }
    }
    if is_inter
        && plane == 0
        && frame_facts.enable_inter_ist()
        && eob > 3
        && metadata.luma_tx_type == DCT_DCT
        && inter_ist_can_read_sec_tx(tx_size, eob)?
    {
        let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
        let sec_tx_type = read_transform_symbol(
            cdfs,
            symbols,
            TileCdfSelector::SecTxType {
                is_inter: 1,
                tx_size_sqr,
            },
        )?;
        metadata.intra_ist = Some(IntraIstSyntax {
            sec_tx_type,
            most_probable_stx_set: None,
        });
    }
    if !is_inter
        && plane == 0
        && frame_facts.enable_intra_ist()
        && eob != 1
        && intra_ist_can_read_sec_tx_type(metadata.luma_tx_type)
        && eob <= ist_eob_limit(tx_size, metadata.luma_tx_type)?
    {
        metadata.intra_ist = read_intra_ist_sec_tx(
            cdfs,
            symbols,
            input.luma_transform_type_context,
            tx_size,
            input.active_intra_ist_policy,
        )?;
    }
    Ok(metadata)
}

fn read_chroma_cctx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    policy: ActiveChromaResidualPolicy,
) -> Result<usize, GeneralIntraResidualError> {
    if !policy.allows_record_handoff() {
        return unsupported_transform_tool_residual("unsupported_dctonly_residual_cctx");
    }
    read_transform_symbol(cdfs, symbols, TileCdfSelector::CctxType)
}

fn read_intra_ist_sec_tx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    luma_context: Option<LumaTransformTypeContext>,
    tx_size: usize,
    policy: ActiveIntraIstResidualPolicy,
) -> Result<Option<IntraIstSyntax>, GeneralIntraResidualError> {
    let luma_context = require_luma_context(
        luma_context,
        "unsupported_dctonly_residual_intra_ist_context",
    )?;
    if luma_context.y_mode.is_paeth() {
        return Ok(None);
    }

    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let sec_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::SecTxType {
            is_inter: 0,
            tx_size_sqr,
        },
    )?;
    let most_probable_stx_set = if sec_tx_type == 0 {
        None
    } else {
        Some(read_transform_symbol(
            cdfs,
            symbols,
            TileCdfSelector::MostProbableStxSet,
        )?)
    };
    let syntax = IntraIstSyntax {
        sec_tx_type,
        most_probable_stx_set,
    };
    ensure_supported_intra_ist_sec_tx_type(syntax, policy)?;
    Ok(Some(syntax))
}

fn ensure_supported_intra_ist_sec_tx_type(
    syntax: IntraIstSyntax,
    policy: ActiveIntraIstResidualPolicy,
) -> Result<(), GeneralIntraResidualError> {
    if syntax.sec_tx_type == 0 || policy.allows_record_handoff() {
        Ok(())
    } else {
        unsupported_transform_tool_residual("unsupported_dctonly_residual_intra_sec_tx_type")
    }
}

fn inter_ist_can_read_sec_tx(
    tx_size: usize,
    eob: usize,
) -> Result<bool, GeneralIntraResidualError> {
    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    Ok(tx_width >= 16 && tx_height >= 16 && eob <= ist_eob_limit(tx_size, DCT_DCT)?)
}

const fn intra_ist_can_read_sec_tx_type(luma_tx_type: usize) -> bool {
    matches!(luma_tx_type, DCT_DCT | ADST_ADST)
}

fn ist_eob_limit(tx_size: usize, tx_type: usize) -> Result<usize, GeneralIntraResidualError> {
    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    if tx_width < 8 || tx_height < 8 {
        Ok(IST_4X4_HEIGHT)
    } else if tx_size == TX_8X8 || tx_type == ADST_ADST {
        Ok(IST_8X8_HEIGHT_RED)
    } else {
        Ok(IST_8X8_HEIGHT)
    }
}

fn read_active_luma_transform_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    luma_context: Option<LumaTransformTypeContext>,
    tx_size: usize,
    tx_set: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let luma_context = require_luma_context(
        luma_context,
        "unsupported_dctonly_residual_luma_transform_context",
    )?;
    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let tx_type = match tx_set {
        TX_SET_INTRA_1 => {
            let intra_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr },
            )?;
            md_idx_luma_tx_type(tx_size, luma_context, intra_tx_type)?
        }
        TX_SET_INTRA_2 => {
            let intra_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr },
            )?;
            md_idx_luma_tx_type(tx_size, luma_context, intra_tx_type)?
        }
        TX_SET_WIDE_64 | TX_SET_HIGH_64 | TX_SET_WIDE_32 | TX_SET_HIGH_32 => {
            read_active_luma_long_tx_type(cdfs, symbols, tx_set, tx_size_sqr)?
        }
        _ => {
            return unsupported_transform_tool_residual("unsupported_dctonly_residual_luma_tx_set");
        }
    };
    Ok(tx_type)
}

fn read_active_luma_long_tx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_set: usize,
    tx_size_sqr: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let shape = long_tx_set_shape(tx_set, "unsupported_dctonly_residual_luma_tx_set_long")?;
    let is_long_side_dct = read_long_side_dct_symbol(cdfs, symbols, shape, 0)?;
    let intra_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::IntraTxTypeLong { tx_size_sqr },
    )?;
    long_tx_type_from_index(
        shape,
        is_long_side_dct,
        intra_tx_type,
        "unsupported_dctonly_residual_invalid_luma_tx_type",
    )
}

fn read_active_inter_transform_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_size: usize,
    tx_set: usize,
    eob: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let ctx = inter_tx_type_long_ctx(tx_size, eob)?;
    match tx_set {
        TX_SET_WIDE_64 | TX_SET_WIDE_32 | TX_SET_HIGH_64 | TX_SET_HIGH_32 => {
            read_active_inter_long_tx_type(cdfs, symbols, tx_set, tx_size_sqr, ctx)
        }
        TX_SET_INTER_1 => read_inter_tx_type_signaling_set(
            cdfs,
            symbols,
            InterTxTypeSignalingSet::Inter1,
            tx_size_sqr,
            ctx,
        ),
        TX_SET_INTER_2 => read_inter_tx_type_signaling_set(
            cdfs,
            symbols,
            InterTxTypeSignalingSet::Inter2,
            tx_size_sqr,
            ctx,
        ),
        TX_SET_DCT_IDTX => {
            let inter_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::InterTxTypeSet3 { ctx, tx_size_sqr },
            )?;
            TX_TYPE_INTER_INV_SET3
                .get(inter_tx_type)
                .copied()
                .ok_or(invalid_inter_tx_type())
        }
        TX_SET_DCT_IDTX_IDDCT => {
            let inter_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::InterTxTypeSet4 { ctx, tx_size_sqr },
            )?;
            TX_TYPE_INTER_INV_SET4
                .get(inter_tx_type)
                .copied()
                .ok_or(invalid_inter_tx_type())
        }
        _ => unsupported_transform_tool_residual("unsupported_dctonly_residual_inter_tx_set"),
    }
}

fn read_active_inter_long_tx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_set: usize,
    tx_size_sqr: usize,
    ctx: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let shape = long_tx_set_shape(tx_set, "unsupported_dctonly_residual_inter_tx_set")?;
    let is_long_side_dct = read_long_side_dct_symbol(cdfs, symbols, shape, 1)?;
    let inter_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::InterTxTypeLong { ctx, tx_size_sqr },
    )?;
    long_tx_type_from_index(
        shape,
        is_long_side_dct,
        inter_tx_type,
        "unsupported_dctonly_residual_invalid_inter_tx_type",
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LongTxSetShape {
    wide_or_high: usize,
    long_side_dct_is_forced: bool,
}

fn long_tx_set_shape(
    tx_set: usize,
    invalid_reason: &'static str,
) -> Result<LongTxSetShape, GeneralIntraResidualError> {
    let wide_or_high = match tx_set {
        TX_SET_WIDE_64 | TX_SET_WIDE_32 => 0,
        TX_SET_HIGH_64 | TX_SET_HIGH_32 => 1,
        _ => return unsupported_transform_tool_residual(invalid_reason),
    };
    Ok(LongTxSetShape {
        wide_or_high,
        long_side_dct_is_forced: matches!(tx_set, TX_SET_WIDE_64 | TX_SET_HIGH_64),
    })
}

fn read_long_side_dct_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    shape: LongTxSetShape,
    is_inter: usize,
) -> Result<usize, GeneralIntraResidualError> {
    if shape.long_side_dct_is_forced {
        Ok(1)
    } else {
        read_transform_symbol(cdfs, symbols, TileCdfSelector::IsLongSideDct { is_inter })
    }
}

fn long_tx_type_from_index(
    shape: LongTxSetShape,
    is_long_side_dct: usize,
    tx_type: usize,
    invalid_reason: &'static str,
) -> Result<usize, GeneralIntraResidualError> {
    TX_TYPE_INV_LONG
        .get(is_long_side_dct)
        .and_then(|long_side| long_side.get(shape.wide_or_high))
        .and_then(|row| row.get(tx_type))
        .copied()
        .ok_or(unsupported_transform_tool_residual_error(invalid_reason))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterTxTypeSignalingSet {
    Inter1,
    Inter2,
}

fn read_inter_tx_type_signaling_set(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    set: InterTxTypeSignalingSet,
    tx_size_sqr: usize,
    ctx: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let (set_selector, index_selector, offset_selector, inversion): (
        TileCdfSelector,
        TileCdfSelector,
        TileCdfSelector,
        &[usize],
    ) = match set {
        InterTxTypeSignalingSet::Inter1 => (
            TileCdfSelector::InterTxTypeSet1 { ctx, tx_size_sqr },
            TileCdfSelector::InterTxTypeIndexSet1 { ctx },
            TileCdfSelector::InterTxTypeOffsetSet1 { ctx },
            &TX_TYPE_INTER_INV_SET1,
        ),
        InterTxTypeSignalingSet::Inter2 => (
            TileCdfSelector::InterTxTypeSet2 { ctx },
            TileCdfSelector::InterTxTypeIndexSet2 { ctx },
            TileCdfSelector::InterTxTypeOffsetSet2 { ctx },
            &TX_TYPE_INTER_INV_SET2,
        ),
    };
    let inter_tx_type = read_transform_symbol(cdfs, symbols, set_selector)?;
    let inter_tx_type_offset = if inter_tx_type == 0 {
        read_transform_symbol(cdfs, symbols, index_selector)?
    } else {
        read_transform_symbol(cdfs, symbols, offset_selector)?
    };
    let tx_type_idx = inter_tx_type * INTER_TX_TYPE_INDEX_COUNT + inter_tx_type_offset;
    inversion
        .get(tx_type_idx)
        .copied()
        .ok_or(invalid_inter_tx_type())
}

const fn invalid_inter_tx_type() -> GeneralIntraResidualError {
    unsupported_transform_tool_residual_error("unsupported_dctonly_residual_invalid_inter_tx_type")
}

fn inter_tx_type_long_ctx(tx_size: usize, eob: usize) -> Result<usize, GeneralIntraResidualError> {
    let eob = eob
        .checked_sub(1)
        .ok_or(unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_inter_tx_type_eob",
        ))?;
    let tx_width_log2 = tx_size_table_usize(&TX_WIDTH_LOG2, "Tx_Width_Log2", tx_size)?;
    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    let bwl = tx_width_log2.min(5);
    let eoby = eob >> bwl;
    let eobx = eob - (eoby << bwl);
    let diag = eobx + eoby;
    let max_diag = tx_width.min(32) + tx_height.min(32) - 4;
    Ok(if diag < 2 {
        1
    } else if diag > max_diag {
        2
    } else {
        0
    })
}

fn read_transform_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
) -> Result<usize, GeneralIntraResidualError> {
    Ok(usize::from(
        cdfs.read_block_symbol_trace(selector, symbols)
            .map_err(|source| GeneralIntraResidualError::TransformTypeRead { source })?
            .get(),
    ))
}

fn md_idx_luma_tx_type(
    tx_size: usize,
    luma_context: LumaTransformTypeContext,
    intra_tx_type: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let size_info = tx_size_table_usize(&SIZE_CLASS, "Size_Class", tx_size)?;
    let intra_dir = luma_transform_intra_dir(tx_size, luma_context)?;
    let mode_row = MD_IDX_TO_TYPE
        .get(size_info)
        .and_then(|size| size.get(intra_dir))
        .ok_or(unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_invalid_intra_mode",
        ))?;
    let tx_type =
        mode_row
            .get(intra_tx_type)
            .copied()
            .ok_or(unsupported_transform_tool_residual_error(
                "unsupported_dctonly_residual_invalid_intra_tx_type",
            ))?;
    usize::try_from(tx_type).map_err(|_| {
        unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_invalid_luma_tx_type",
        )
    })
}

fn luma_transform_intra_dir(
    tx_size: usize,
    luma_context: LumaTransformTypeContext,
) -> Result<usize, GeneralIntraResidualError> {
    let intra_dir = luma_context.y_mode.value();
    if !luma_context.y_mode.is_directional() {
        return Ok(intra_dir);
    }
    let mode_to_angle =
        MODE_TO_ANGLE
            .get(intra_dir)
            .copied()
            .ok_or(unsupported_transform_tool_residual_error(
                "unsupported_dctonly_residual_invalid_intra_mode",
            ))?;
    let mrl_delta = MRL_INDEX_TO_DELTA
        .get(usize::from(luma_context.mrl_index))
        .copied()
        .ok_or(unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_invalid_mrl_index",
        ))?;
    let p_angle = mode_to_angle
        .checked_add(i32::from(luma_context.angle_delta_y) * ANGLE_STEP)
        .and_then(|angle| angle.checked_add(mrl_delta))
        .ok_or(unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_luma_angle_overflow",
        ))?;
    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    Ok(wide_angle_mapping(intra_dir, tx_width, tx_height, p_angle))
}

fn wide_angle_mapping(mode: usize, width: usize, height: usize, p_angle: i32) -> usize {
    if WAIP_WH_RATIO_THRESHOLDS
        .iter()
        .any(|&(scale, threshold)| is_scaled(height, width, scale) && p_angle < threshold)
    {
        D203_PRED
    } else if WAIP_WH_RATIO_THRESHOLDS
        .iter()
        .any(|&(scale, threshold)| is_scaled(width, height, scale) && p_angle > 270 - threshold)
    {
        D45_PRED
    } else {
        mode
    }
}

const fn is_scaled(value: usize, base: usize, factor: usize) -> bool {
    matches!(base.checked_mul(factor), Some(scaled) if scaled == value)
}

fn transform_set(
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
) -> Result<usize, GeneralIntraResidualError> {
    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let tx_size_sqr_up = tx_size_table_usize(&TX_SIZE_SQR_UP, "Tx_Size_Sqr_Up", tx_size)?;
    if tx_size_sqr_up > TX_32X32 {
        if tx_size_sqr >= TX_32X32 {
            return Ok(TX_SET_DCTONLY);
        }
        let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
        return if tx_width > tx_height {
            Ok(TX_SET_WIDE_64)
        } else {
            Ok(TX_SET_HIGH_64)
        };
    }
    if tx_size_sqr_up == TX_32X32 && tx_size_sqr != TX_32X32 {
        let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
        return if tx_width > tx_height {
            Ok(TX_SET_WIDE_32)
        } else {
            Ok(TX_SET_HIGH_32)
        };
    }
    if !is_inter && tx_size_sqr_up == TX_32X32 {
        return Ok(TX_SET_DCTONLY);
    }
    let reduced_tx_set = if plane == 0 {
        frame_facts.reduced_tx_set()
    } else {
        usize::from(frame_facts.enable_chroma_dctonly())
    };
    if reduced_tx_set > 3 {
        return unsupported_transform_tool_residual("unsupported_dctonly_residual_reduced_tx_set");
    }
    if tx_size_sqr_up == TX_32X32 || reduced_tx_set == 1 {
        return if is_inter {
            Ok(TX_SET_DCT_IDTX)
        } else {
            Ok(TX_SET_INTRA_2)
        };
    } else if reduced_tx_set == 2 {
        return Ok(TX_SET_DCT_IDTX);
    } else if reduced_tx_set == 3 {
        return if is_inter {
            Ok(TX_SET_DCT_IDTX_IDDCT)
        } else {
            Ok(TX_SET_INTRA_2)
        };
    }
    if is_inter {
        return if tx_size_sqr == 2 {
            Ok(TX_SET_INTER_2)
        } else {
            Ok(TX_SET_INTER_1)
        };
    }
    Ok(TX_SET_INTRA_1)
}

fn tx_size_table_usize(
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let value = table
        .get(tx_size)
        .copied()
        .ok_or(unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_invalid_tx_size",
        ))?;
    usize::try_from(value).map_err(|_| {
        let reason = match table_name {
            "Tx_Size_Sqr" => "unsupported_dctonly_residual_invalid_tx_size_sqr",
            _ => "unsupported_dctonly_residual_invalid_tx_size_sqr_up",
        };
        unsupported_transform_tool_residual_error(reason)
    })
}

fn tx_size_dimensions(tx_size: usize) -> Result<(usize, usize), GeneralIntraResidualError> {
    Ok((
        tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?,
        tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?,
    ))
}

fn require_luma_context(
    luma_context: Option<LumaTransformTypeContext>,
    reason: &'static str,
) -> Result<LumaTransformTypeContext, GeneralIntraResidualError> {
    luma_context.ok_or(unsupported_transform_tool_residual_error(reason))
}

const fn unsupported_transform_tool_residual_error(
    reason: &'static str,
) -> GeneralIntraResidualError {
    GeneralIntraResidualError::UnsupportedTransformToolResidual { reason }
}

fn unsupported_transform_tool_residual<T>(
    reason: &'static str,
) -> Result<T, GeneralIntraResidualError> {
    Err(unsupported_transform_tool_residual_error(reason))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block<T: ReconSample>(
    quant: &[i32],
    dc_sample: T,
    qindex: u32,
    plane_id: PlaneId,
    log2_side: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    let orig_side = 1usize << log2_side;
    let prediction = vec![dc_sample; orig_side * orig_side];
    reconstruct_general_intra_block_with_prediction(
        quant,
        &prediction,
        qindex,
        plane_id,
        log2_side,
        plane_tx_type,
        use_tcq,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_with_prediction<T: ReconSample>(
    quant: &[i32],
    prediction: &[T],
    qindex: u32,
    plane_id: PlaneId,
    log2_side: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    reconstruct_general_intra_block_rect_with_prediction(
        quant,
        prediction,
        qindex,
        plane_id,
        log2_side,
        log2_side,
        plane_tx_type,
        use_tcq,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_rect_with_prediction<T: ReconSample>(
    quant: &[i32],
    prediction: &[T],
    qindex: u32,
    plane_id: PlaneId,
    log2_width: u32,
    log2_height: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    let orig_w = 1usize << log2_width;
    let orig_h = 1usize << log2_height;

    let adj_w = 1usize << log2_width.min(5);
    let adj_h = 1usize << log2_height.min(5);
    let adjusted = adj_w * adj_h;
    if quant.len() != adjusted {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: adjusted,
            actual: quant.len(),
        });
    }
    let samples = orig_w * orig_h;
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
    let pels = (orig_w * orig_h) as u32;
    let dq_shift = u32::from(pels > 256) + u32::from(pels > 1024) + u32::from(use_tcq);
    let dq_denom = 1u32 << dq_shift;
    let params = DequantBlockParams {
        dc_quant: dc_quantizer(plane_id, qindex, deltas, bit_depth),
        ac_quant: ac_quantizer(plane_id, qindex, deltas, bit_depth),
        tx_width: adj_w,
        tx_height: adj_h,
        dq_denom,
        bit_depth,
    };
    let transform = InverseTransform2dOuter::resolve(
        plane_tx_type,
        log2_width,
        log2_height,
        false,
        false,
        bit_depth,
        None,
    )
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;

    let mut dequant_scratch = vec![0i32; adjusted];
    let mut residual_scratch = vec![0i32; samples];
    let mut out = vec![T::default(); samples];
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

fn txb_skip_tx_size_ctx(tx_size: usize) -> usize {
    let sqr = TX_SIZE_SQR.get(tx_size).copied().unwrap_or(0);
    let sqr_up = TX_SIZE_SQR_UP.get(tx_size).copied().unwrap_or(0);
    (((sqr + sqr_up + 1) >> 1).max(0)) as usize
}

#[cfg(test)]
mod tests;
