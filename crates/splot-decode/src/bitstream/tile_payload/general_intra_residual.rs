// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra transform-block coefficient decode.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, MD_IDX_TO_TYPE, MODE_TO_ANGLE, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE,
    SIZE_CLASS, SIZE_TO_TX_PART_GROUP_LOOKUP, SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ,
    SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR, TX_SIZE_SQR_UP,
    TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::{
    BitDepth, DequantBlockParams, InverseTransform2dOuter, PlaneId, QM_OFFSET, QmDequant,
    QmFrameLevels, QuantizerDeltas, ReconError, ReconSample, SecondaryInverseTransform,
    ac_quantizer, dc_quantizer, reconstruct_transform_block_residual_with_secondary, tx_size_index,
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
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryBranchModeToTxfmBaseConfig,
    CoeffOrdinaryStagedLosslessNonZeroInput, CoeffOrdinaryTxSizeGeometryConfig,
    apply_staged_nonzero_coeff_ordinary_branch_from_lossless, resolve_mode_to_txfm_plane_tx_type,
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
/// AV2 § 3 `NUM_CUSTOM_QMS`: the number of built-in quantizer-matrix levels.
const NUM_CUSTOM_QMS: usize = 15;

thread_local! {
    /// Active frame's § 7.14.4 built-in quantization-matrix levels (installed by
    /// [`FrameQmScope`]). General-intra reconstruction is single-threaded per frame,
    /// so a thread-local frame context is sound.
    static FRAME_QM: core::cell::Cell<Option<QmFrameLevels>> = const { core::cell::Cell::new(None) };
}

/// RAII scope installing the frame's built-in quantization-matrix levels, restored
/// on drop so nothing leaks into a later frame. `None` is the flat dequant path.
pub(crate) struct FrameQmScope(Option<QmFrameLevels>);

impl FrameQmScope {
    pub(crate) fn install(levels: Option<QmFrameLevels>) -> Self {
        Self(FRAME_QM.with(|cell| cell.replace(levels)))
    }
}

impl Drop for FrameQmScope {
    fn drop(&mut self) {
        FRAME_QM.with(|cell| cell.set(self.0));
    }
}

/// § 7.14.4 built-in quantization-matrix selection for one transform block from the
/// active [`FrameQmScope`]: `Some` when `useQm` (`using_qmatrix`,
/// `PlaneTxType < IDTX`, `segLvl < NUM_CUSTOM_QMS`), else `None` (flat). `tw`/`th`
/// are the `Min(32, …)` dequant dims; `useUserQm` (§ 5.13) is not modelled.
fn resolve_block_qm(
    plane_id: PlaneId,
    plane_tx_type: usize,
    tw: usize,
    th: usize,
    log2_width: u32,
    log2_height: u32,
) -> Option<QmDequant> {
    let levels = FRAME_QM.with(core::cell::Cell::get)?;
    if plane_tx_type >= IDTX {
        return None;
    }
    let plane_idx = match plane_id {
        PlaneId::Y => 0,
        PlaneId::U => 1,
        PlaneId::V => 2,
    };
    let seg_level = usize::from(if tw > 8 || th > 8 {
        levels.levels_gt8[plane_idx]
    } else {
        levels.levels_le8[plane_idx]
    });
    if seg_level >= NUM_CUSTOM_QMS {
        return None;
    }
    let tx_sz = tx_size_index(log2_width, log2_height).ok()?;
    let qm_offset = usize::try_from(*QM_OFFSET.get(tx_sz)?).ok()?;
    Some(QmDequant {
        seg_level,
        plane_is_chroma: plane_idx != 0,
        qm_offset,
    })
}
const H_PRED: usize = 2;
const D45_PRED: usize = 3;
const D157_PRED: usize = 6;
const D203_PRED: usize = 7;
const D67_PRED: usize = 8;
const SMOOTH_H_PRED: usize = 11;
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
const BLOCK_4X4: usize = 0;
const MI_SIZE: usize = 4;
const TX_PARTITION_NONE: usize = 0;
const TX_PARTITION_SPLIT: usize = 1;
const TX_PARTITION_HORZ: usize = 2;
const TX_PARTITION_VERT: usize = 3;
const TX_PARTITION_HORZ4: usize = 4;
const TX_PARTITION_VERT4: usize = 5;
const TX_PARTITION_HORZ5: usize = 6;
const TX_PARTITION_VERT5: usize = 7;
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
const IST_DIR_SIZE: usize = 7;
const IST_REDUCE_SET_SIZE_ADST_ADST: usize = 4;

#[doc = "AV2 § 7.15.3 secondary transform process inverse IST maps."]
#[rustfmt::skip]
const INV_MOST_PROBABLE_STX_MAPPING: [[usize; IST_DIR_SIZE]; 12] = [
    [6, 1, 0, 5, 4, 3, 2],
    [1, 6, 0, 4, 2, 5, 3],
    [1, 6, 0, 4, 2, 5, 3],
    [2, 6, 0, 5, 1, 4, 3],
    [3, 4, 6, 1, 0, 2, 5],
    [4, 1, 3, 6, 0, 5, 2],
    [4, 1, 3, 6, 0, 5, 2],
    [5, 0, 6, 2, 1, 4, 3],
    [5, 0, 6, 2, 1, 4, 3],
    [6, 1, 0, 5, 4, 3, 2],
    [1, 6, 0, 4, 2, 5, 3],
    [1, 6, 0, 4, 2, 5, 3],
];

#[doc = "AV2 § 7.15.3 secondary transform process inverse IST maps."]
#[rustfmt::skip]
const INV_MOST_PROBABLE_STX_MAPPING_ADST: [[usize; IST_REDUCE_SET_SIZE_ADST_ADST]; 12] = [
    [3, 1, 0, 2],
    [1, 3, 0, 2],
    [1, 3, 0, 2],
    [1, 3, 0, 2],
    [0, 2, 3, 1],
    [2, 1, 0, 3],
    [2, 1, 0, 3],
    [1, 0, 3, 2],
    [1, 0, 3, 2],
    [3, 1, 0, 2],
    [1, 3, 0, 2],
    [1, 3, 0, 2],
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaTransformTypeContext {
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
    mrl_sec_index: Option<u8>,
}

impl LumaTransformTypeContext {
    #[must_use]
    pub(crate) const fn new(y_mode: IntraYMode, angle_delta_y: i8) -> Self {
        Self {
            y_mode,
            angle_delta_y,
            mrl_index: 0,
            mrl_sec_index: None,
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
            mrl_sec_index: None,
        }
    }

    #[must_use]
    pub(crate) const fn with_mrl_indices(
        y_mode: IntraYMode,
        angle_delta_y: i8,
        mrl_index: u8,
        mrl_sec_index: Option<u8>,
    ) -> Self {
        Self {
            y_mode,
            angle_delta_y,
            mrl_index,
            mrl_sec_index,
        }
    }

    #[must_use]
    pub(crate) const fn y_mode(self) -> IntraYMode {
        self.y_mode
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
pub(crate) struct LumaTransformPartitionContext {
    mi_size: usize,
}

impl LumaTransformPartitionContext {
    #[must_use]
    pub(crate) const fn new(mi_size: usize) -> Self {
        Self { mi_size }
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

impl TransformToolResidualPolicy {
    /// `Allow` unless the sequence enables a transform tool, in which case the
    /// caller-selected active-tool policies apply with no luma context.
    pub(crate) fn from_sequence_tools(
        sequence: &splot_core::headers::sequence::SequenceHeader,
        active_intra_ist: ActiveIntraIstResidualPolicy,
        active_chroma: ActiveChromaResidualPolicy,
    ) -> Self {
        sequence
            .transform_quant_entropy
            .as_ref()
            .map_or(Self::Allow, |tq| {
                if tq.enable_inter_ist
                    || tq.enable_intra_ist
                    || tq.enable_inter_ddt
                    || tq.enable_cctx
                    || tq.enable_fsc
                    || tq.enable_idtx_intra
                {
                    Self::AdmitTransformToolSubset {
                        luma: None,
                        active_intra_ist,
                        active_chroma,
                    }
                } else {
                    Self::Allow
                }
            })
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PositionedLumaCoeffBlock {
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) tx_size: usize,
    /// § 5.20.6.3 `LumaTxMiddle`: § 5.20.7.24 passes `allowCorners = 0` for
    /// this unit, so the top-right/bottom-left availability counts are zero.
    pub(crate) middle: bool,
    pub(crate) coeffs: LumaCoeffBlock,
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
    #[error("general intra luma transform partition symbol read failed: {source}")]
    TransformPartitionRead { source: BlockSymbolTraceReadError },
    #[error("general intra luma transform partition table {table} has invalid index {index}")]
    TransformPartitionGeometry { table: &'static str, index: usize },
    #[error("general intra luma transform partition syntax is unsupported: {reason}")]
    UnsupportedTransformPartition { reason: &'static str },
    #[error("general intra luma transform_type symbol read failed: {source}")]
    TransformTypeRead { source: BlockSymbolTraceReadError },
    #[error("general intra luma palette token symbol read failed: {source}")]
    PaletteSymbolRead { source: BlockSymbolTraceReadError },
    #[error("general intra luma palette token literal read failed for {reason}: {source}")]
    PaletteLiteral {
        reason: &'static str,
        source: CoreError,
    },
    #[error("general intra luma palette identity-row copy is invalid on the first row")]
    PaletteInvalidIdentityRow,
    #[error(
        "general intra luma palette color index {color_index} is outside palette size {palette_size}"
    )]
    PaletteColorIndex {
        color_index: usize,
        palette_size: usize,
    },
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

fn read_luma_transform_partition_prelude(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    context: LumaTransformPartitionContext,
    tx_size: usize,
    fsc_mode: bool,
    is_inter: bool,
) -> Result<(), GeneralIntraResidualError> {
    if context.mi_size == BLOCK_4X4 {
        return Ok(());
    }
    let max_tx_size =
        block_size_table_usize(&MAX_TX_SIZE_RECT, "Max_Tx_Size_Rect", context.mi_size)?;
    if tx_size != max_tx_size {
        return Err(unsupported_transform_partition(
            "unsupported_general_intra_tx_partition_non_max_tx_size",
        ));
    }
    let block_width =
        block_size_table_usize(&NUM_4X4_BLOCKS_WIDE, "Num_4x4_Blocks_Wide", context.mi_size)?
            .checked_mul(MI_SIZE)
            .ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Num_4x4_Blocks_Wide",
                index: context.mi_size,
            })?;
    let block_height =
        block_size_table_usize(&NUM_4X4_BLOCKS_HIGH, "Num_4x4_Blocks_High", context.mi_size)?
            .checked_mul(MI_SIZE)
            .ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Num_4x4_Blocks_High",
                index: context.mi_size,
            })?;
    if (block_width >> 6) > 1 || (block_height >> 6) > 1 {
        return Ok(());
    }

    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    let allow_horz = tx_size_from_dimensions(tx_width, tx_height >> 1).is_some();
    let allow_vert = tx_size_from_dimensions(tx_width >> 1, tx_height).is_some();
    if !allow_horz && !allow_vert {
        return Ok(());
    }

    let txfm_split_group = block_size_table_usize(
        &SIZE_TO_TX_PART_GROUP_LOOKUP,
        "Size_To_Tx_Part_Group_Lookup",
        context.mi_size,
    )?;
    let do_partition = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::TxDoPartition {
                fsc_mode: usize::from(fsc_mode),
                is_inter: usize::from(is_inter),
                txfm_split_group,
            },
            symbols,
        )
        .map_err(|source| GeneralIntraResidualError::TransformPartitionRead { source })?
        .get()
        != 0;
    if do_partition {
        return Err(unsupported_transform_partition(
            "unsupported_general_intra_tx_partition_split",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub(crate) fn decode_general_intra_luma_partition_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    tx_fills_block: bool,
    luma_tx_partition: LumaTransformPartitionContext,
    uv_mode: usize,
    angle_delta_uv: i32,
    fsc_mode: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
) -> Result<Vec<PositionedLumaCoeffBlock>, GeneralIntraResidualError> {
    let reduced_tx_set = work_unit.coeff_frame_facts().reduced_tx_set();
    let records = read_luma_transform_partition_records(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        luma_tx_partition,
        tx_size,
        start_x,
        start_y,
        fsc_mode,
        false,
        reduced_tx_set,
    )?;
    let mut blocks = Vec::new();
    blocks
        .try_reserve(records.len())
        .map_err(|_| unsupported_transform_partition("partition-record-allocation"))?;
    let record_count = records.len();
    for record in records {
        let record_fills_block = luma_partition_record_fills_block(
            tx_fills_block,
            record_count,
            record,
            start_x,
            start_y,
        );
        let coeffs = decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            context,
            0,
            record.tx_size,
            record.x,
            record.y,
            record_fills_block,
            None,
            false,
            uv_mode,
            angle_delta_uv,
            false,
            fsc_mode,
            fsc_mode,
            transform_tool_residual_policy,
        )?;
        blocks.push(PositionedLumaCoeffBlock {
            x: record.x,
            y: record.y,
            tx_size: record.tx_size,
            middle: record.middle,
            coeffs,
        });
    }
    Ok(blocks)
}

fn luma_partition_record_fills_block(
    block_fills_residual: bool,
    record_count: usize,
    record: LumaTransformPartitionRecord,
    start_x: usize,
    start_y: usize,
) -> bool {
    block_fills_residual && record_count == 1 && record.x == start_x && record.y == start_y
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LumaTransformPartitionRecord {
    x: usize,
    y: usize,
    tx_size: usize,
    middle: bool,
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn read_luma_transform_partition_records(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    context: LumaTransformPartitionContext,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    fsc_mode: bool,
    is_inter: bool,
    reduced_tx_set: usize,
) -> Result<Vec<LumaTransformPartitionRecord>, GeneralIntraResidualError> {
    if context.mi_size == BLOCK_4X4 {
        return Ok(vec![luma_transform_record(start_x, start_y, tx_size)]);
    }
    let max_tx_size =
        block_size_table_usize(&MAX_TX_SIZE_RECT, "Max_Tx_Size_Rect", context.mi_size)?;
    if tx_size != max_tx_size {
        return Err(unsupported_transform_partition(
            "unsupported_general_intra_tx_partition_non_max_tx_size",
        ));
    }
    let block_width =
        block_size_table_usize(&NUM_4X4_BLOCKS_WIDE, "Num_4x4_Blocks_Wide", context.mi_size)?
            .checked_mul(MI_SIZE)
            .ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Num_4x4_Blocks_Wide",
                index: context.mi_size,
            })?;
    let block_height =
        block_size_table_usize(&NUM_4X4_BLOCKS_HIGH, "Num_4x4_Blocks_High", context.mi_size)?
            .checked_mul(MI_SIZE)
            .ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Num_4x4_Blocks_High",
                index: context.mi_size,
            })?;
    if (block_width >> 6) > 1 || (block_height >> 6) > 1 {
        return Ok(vec![luma_transform_record(start_x, start_y, tx_size)]);
    }

    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    let allow_horz = tx_size_from_dimensions(tx_width, tx_height >> 1).is_some();
    let allow_vert = tx_size_from_dimensions(tx_width >> 1, tx_height).is_some();
    if !allow_horz && !allow_vert {
        return Ok(vec![luma_transform_record(start_x, start_y, tx_size)]);
    }

    let tx_partition = read_luma_tx_partition_type(
        cdfs,
        symbols,
        context.mi_size,
        fsc_mode,
        is_inter,
        allow_horz,
        allow_vert,
        reduced_tx_set,
    )?;
    luma_transform_records_for_partition(start_x, start_y, tx_size, tx_partition)
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn read_luma_tx_partition_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    mi_size: usize,
    fsc_mode: bool,
    is_inter: bool,
    allow_horz: bool,
    allow_vert: bool,
    reduced_tx_set: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let txfm_split_group = block_size_table_usize(
        &SIZE_TO_TX_PART_GROUP_LOOKUP,
        "Size_To_Tx_Part_Group_Lookup",
        mi_size,
    )?;
    let fsc_mode = usize::from(fsc_mode);
    let is_inter = usize::from(is_inter);
    let do_partition = cdfs
        .read_block_symbol_trace(
            TileCdfSelector::TxDoPartition {
                fsc_mode,
                is_inter,
                txfm_split_group,
            },
            symbols,
        )
        .map_err(|source| GeneralIntraResidualError::TransformPartitionRead { source })?
        .get()
        != 0;
    if !do_partition {
        return Ok(TX_PARTITION_NONE);
    }
    if allow_horz && allow_vert {
        let ctx = block_size_table_usize(
            &SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ,
            "Size_To_Tx_Type_Group_Vert_And_Horz",
            mi_size,
        )?;
        let symbol = cdfs
            .read_block_symbol_trace(
                TileCdfSelector::TxPartitionType {
                    fsc_mode,
                    is_inter,
                    ctx,
                    reduced: false,
                },
                symbols,
            )
            .map_err(|source| GeneralIntraResidualError::TransformPartitionRead { source })?
            .get();
        return Ok(usize::from(symbol).saturating_add(1));
    }
    let vert_or_horz_group = block_size_table_usize(
        &SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ,
        "Size_To_Tx_Type_Group_Vert_Or_Horz",
        mi_size,
    )?;
    if vert_or_horz_group > 0 {
        let tx_2or3 = if reduced_tx_set != 0 {
            0
        } else {
            cdfs.read_block_symbol_trace(
                TileCdfSelector::Tx2Or3PartitionType {
                    fsc_mode,
                    is_inter,
                    ctx: vert_or_horz_group.checked_sub(1).ok_or(
                        GeneralIntraResidualError::TransformPartitionGeometry {
                            table: "Size_To_Tx_Type_Group_Vert_Or_Horz",
                            index: mi_size,
                        },
                    )?,
                },
                symbols,
            )
            .map_err(|source| GeneralIntraResidualError::TransformPartitionRead { source })?
            .get()
        };
        return Ok(if allow_horz {
            if tx_2or3 != 0 {
                TX_PARTITION_HORZ4
            } else {
                TX_PARTITION_HORZ
            }
        } else if tx_2or3 != 0 {
            TX_PARTITION_VERT4
        } else {
            TX_PARTITION_VERT
        });
    }
    Ok(if allow_horz {
        TX_PARTITION_HORZ
    } else {
        TX_PARTITION_VERT
    })
}

fn luma_transform_records_for_partition(
    start_x: usize,
    start_y: usize,
    tx_size: usize,
    tx_partition: usize,
) -> Result<Vec<LumaTransformPartitionRecord>, GeneralIntraResidualError> {
    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    let mut w4 = tx_width / MI_SIZE;
    let mut h4 = tx_height / MI_SIZE;
    let col4 = start_x / MI_SIZE;
    let row4 = start_y / MI_SIZE;
    let mut records = Vec::new();
    match tx_partition {
        TX_PARTITION_NONE => push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?,
        TX_PARTITION_HORZ => {
            h4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4, w4, false)?;
        }
        TX_PARTITION_VERT => {
            w4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4, w4, false)?;
        }
        TX_PARTITION_HORZ4 => {
            h4 >>= 2;
            for part in 0..4 {
                push_luma_transform_record(&mut records, row4 + part * h4, col4, h4, w4, false)?;
            }
        }
        TX_PARTITION_VERT4 => {
            w4 >>= 2;
            for part in 0..4 {
                push_luma_transform_record(&mut records, row4, col4 + part * w4, h4, w4, false)?;
            }
        }
        TX_PARTITION_HORZ5 => {
            h4 >>= 2;
            w4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4 << 1, w4 << 1, true)?;
            push_luma_transform_record(&mut records, row4 + h4 * 3, col4, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4 + h4 * 3, col4 + w4, h4, w4, true)?;
        }
        TX_PARTITION_VERT5 => {
            h4 >>= 1;
            w4 >>= 2;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4 << 1, w4 << 1, true)?;
            push_luma_transform_record(&mut records, row4, col4 + w4 * 3, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4 + h4, col4 + w4 * 3, h4, w4, true)?;
        }
        TX_PARTITION_SPLIT => {
            w4 >>= 1;
            h4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4 + w4, h4, w4, false)?;
        }
        _ => {
            return Err(unsupported_transform_partition(
                "unsupported_general_intra_tx_partition_type",
            ));
        }
    }
    Ok(records)
}

fn push_luma_transform_record(
    records: &mut Vec<LumaTransformPartitionRecord>,
    row4: usize,
    col4: usize,
    h4: usize,
    w4: usize,
    middle: bool,
) -> Result<(), GeneralIntraResidualError> {
    if h4 == 0 || w4 == 0 {
        return Err(unsupported_transform_partition(
            "unsupported_general_intra_tx_partition_empty",
        ));
    }
    let width =
        w4.checked_mul(MI_SIZE)
            .ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Tx_Width",
                index: w4,
            })?;
    let height =
        h4.checked_mul(MI_SIZE)
            .ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Tx_Height",
                index: h4,
            })?;
    let tx_size = tx_size_from_dimensions(width, height).ok_or(
        GeneralIntraResidualError::TransformPartitionGeometry {
            table: "Tx_Size",
            index: width,
        },
    )?;
    records.push(LumaTransformPartitionRecord {
        x: col4 * MI_SIZE,
        y: row4 * MI_SIZE,
        tx_size,
        middle,
    });
    Ok(())
}

const fn luma_transform_record(x: usize, y: usize, tx_size: usize) -> LumaTransformPartitionRecord {
    LumaTransformPartitionRecord {
        x,
        y,
        tx_size,
        middle: false,
    }
}

fn block_size_table_usize(
    table: &[i32],
    table_name: &'static str,
    index: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let value =
        table
            .get(index)
            .copied()
            .ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
                table: table_name,
                index,
            })?;
    usize::try_from(value).map_err(|_| GeneralIntraResidualError::TransformPartitionGeometry {
        table: table_name,
        index,
    })
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
    luma_tx_partition: Option<LumaTransformPartitionContext>,
    eob_u_nonzero: bool,
    uv_mode: usize,
    angle_delta_uv: i32,
    is_inter: bool,
    fsc_mode: bool,
    txb_skip_fsc_mode: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let x4 = start_x >> 2;
    let y4 = start_y >> 2;
    let w4 = usize::try_from(TX_WIDTH.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let h4 = usize::try_from(TX_HEIGHT.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let frame_facts = work_unit.coeff_frame_facts();
    let coeff_cdf_q_ctx = coeff_cdf_q_ctx_from_base_q_idx(frame_facts.base_q_idx());
    let tx_size_ctx = txb_skip_tx_size_ctx(tx_size);

    if plane == 0
        && let Some(tx_partition) = luma_tx_partition
    {
        read_luma_transform_partition_prelude(
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            tx_partition,
            tx_size,
            fsc_mode,
            is_inter,
        )?;
    }

    let above_level_or = or_u32(context.above_level(plane).map_err(coeff_ctx_err)?, x4, w4);
    let left_level_or = or_u32(context.left_level(plane).map_err(coeff_ctx_err)?, y4, h4);
    let txb_skip_intra_inter = usize::from(is_inter || txb_skip_fsc_mode);
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

    if crate::trace_flags::trace_flag!("SPLOT_TRACE_TXB_SKIP") {
        eprintln!(
            "txb_skip read plane={plane} tx_size={tx_size} tx_size_ctx={tx_size_ctx} start=({start_x},{start_y}) x4={x4} y4={y4} w4={w4} h4={h4} fills={tx_fills_block} is_inter={is_inter} fsc={fsc_mode} txb_fsc={txb_skip_fsc_mode} above_or={above_level_or} left_or={left_level_or} eob_u={eob_u_nonzero} selector={selector:?} checkpoint={:?}",
            symbols.checkpoint(),
        );
    }
    let all_zero = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .read_block_symbol_trace(selector, symbols)
        .map_err(|source| GeneralIntraResidualError::AllZeroRead { source })?
        .get()
        != 0;
    if crate::trace_flags::trace_flag!("SPLOT_TRACE_TXB_SKIP") {
        eprintln!(
            "txb_skip done plane={plane} start=({start_x},{start_y}) all_zero={all_zero} checkpoint={:?}",
            symbols.checkpoint(),
        );
    }

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
            angle_delta_uv,
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
            angle_delta_uv,
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
        quant: pass.into_block().into_quant(),
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
    angle_delta_uv: i32,
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
        angle_delta_uv,
        lossless,
        metadata,
    );
    let plane_tx_type =
        staged_transform_tool_plane_tx_type(geometry, is_inter, lossless, base_config)?;
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
            plane_tx_type,
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
        quant: pass.into_block().into_quant(),
        intra_ist: metadata.intra_ist,
        plane_tx_type,
    })
}

fn staged_transform_tool_lossless_base_config(
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    uv_mode: usize,
    angle_delta_uv: i32,
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
        angle_delta_uv,
        luma_tx_type: metadata.luma_tx_type,
        chroma_inter_tx_type: DCT_DCT,
        parity_hiding,
        use_tcq,
    }
}

fn staged_transform_tool_plane_tx_type(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    lossless: bool,
    base_config: CoeffOrdinaryBranchLosslessBaseConfig,
) -> Result<usize, GeneralIntraResidualError> {
    let mode_to_txfm = CoeffOrdinaryBranchModeToTxfmBaseConfig {
        tx_set: transform_set_from_flags(
            base_config.reduced_tx_set,
            base_config.enable_chroma_dctonly,
            geometry.plane,
            geometry.tx_size,
            is_inter,
        )?,
        uv_mode: base_config.uv_mode,
        angle_delta_uv: base_config.angle_delta_uv,
        luma_tx_type: base_config.luma_tx_type,
        chroma_inter_tx_type: base_config.chroma_inter_tx_type,
        enable_chroma_dctonly: base_config.enable_chroma_dctonly,
        parity_hiding: base_config.parity_hiding,
        use_tcq: base_config.use_tcq,
    };
    resolve_mode_to_txfm_plane_tx_type(geometry, is_inter, lossless, mode_to_txfm)
        .map_err(|source| GeneralIntraResidualError::StagedNonZeroPass { source })
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
    // TODO(spec: DECODE-FIRST-INTER-FRAME-FRONTIER): 5.20.7.27 is_cctx_allowed also excludes non-4:2:0 >=32x32 chroma; exact for the admitted 4:2:0 subset.
    if plane == 1 && frame_facts.enable_cctx() && (is_inter || eob != 1) {
        let cctx_type = read_chroma_cctx_type(cdfs, symbols, input.active_chroma_policy)?;
        if is_inter && cctx_type != 0 {
            return unsupported_transform_tool_residual(
                "unsupported_inter_residual_cctx_transform",
            );
        }
        metadata.cctx_type = Some(cctx_type);
    }
    let tx_set = transform_set(frame_facts, plane, tx_size, is_inter)?;
    let dct_forced = (!is_inter && plane == 0 && eob == 1)
        || (plane > 0 && frame_facts.enable_chroma_dctonly())
        || tx_set == TX_SET_DCTONLY
        || (!is_inter && plane == 0 && frame_facts.reduced_tx_set() == 2);
    if !is_inter && plane == 0 && input.fsc_mode {
        metadata.luma_tx_type = IDTX;
    } else if !dct_forced {
        if plane > 0 && input.active_chroma_policy.allows_record_handoff() {
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
    transform_set_from_flags(
        frame_facts.reduced_tx_set(),
        frame_facts.enable_chroma_dctonly(),
        plane,
        tx_size,
        is_inter,
    )
}

fn transform_set_from_flags(
    reduced_tx_set: usize,
    enable_chroma_dctonly: bool,
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
        reduced_tx_set
    } else {
        usize::from(enable_chroma_dctonly)
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

fn tx_size_from_dimensions(width: usize, height: usize) -> Option<usize> {
    TX_WIDTH
        .iter()
        .zip(TX_HEIGHT.iter())
        .position(|(&tx_width, &tx_height)| {
            usize::try_from(tx_width) == Ok(width) && usize::try_from(tx_height) == Ok(height)
        })
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

const fn unsupported_transform_partition(reason: &'static str) -> GeneralIntraResidualError {
    GeneralIntraResidualError::UnsupportedTransformPartition { reason }
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
    reconstruct_general_intra_block_rect_with_prediction_and_ddt(
        quant,
        prediction,
        qindex,
        plane_id,
        log2_side,
        log2_side,
        plane_tx_type,
        use_tcq,
        false,
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
    reconstruct_general_intra_block_rect_with_prediction_and_ddt(
        quant,
        prediction,
        qindex,
        plane_id,
        log2_width,
        log2_height,
        plane_tx_type,
        use_tcq,
        false,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_rect_with_prediction_and_ddt<T: ReconSample>(
    quant: &[i32],
    prediction: &[T],
    qindex: u32,
    plane_id: PlaneId,
    log2_width: u32,
    log2_height: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    reconstruct_general_intra_block_rect_with_prediction_core(
        quant,
        prediction,
        qindex,
        plane_id,
        log2_width,
        log2_height,
        plane_tx_type,
        use_tcq,
        use_ddt,
        None,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_block_rect_with_prediction_and_ist<T: ReconSample>(
    block: &LumaCoeffBlock,
    prediction: &[T],
    qindex: u32,
    log2_width: u32,
    log2_height: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
    luma_context: LumaTransformTypeContext,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    let secondary =
        intra_secondary_inverse_transform(block, log2_width, log2_height, bit_depth, luma_context)?;
    reconstruct_general_intra_block_rect_with_prediction_core(
        &block.quant,
        prediction,
        qindex,
        PlaneId::Y,
        log2_width,
        log2_height,
        block.plane_tx_type,
        use_tcq,
        false,
        secondary.as_ref(),
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_block_rect_with_prediction_core<T: ReconSample>(
    quant: &[i32],
    prediction: &[T],
    qindex: u32,
    plane_id: PlaneId,
    log2_width: u32,
    log2_height: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    use_ddt: bool,
    secondary: Option<&SecondaryInverseTransform>,
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
    let tcq_two_d = use_tcq
        && CoeffTransformClass::from_plane_tx_type(plane_tx_type) == CoeffTransformClass::TwoD;
    let dq_shift = u32::from(pels > 256) + u32::from(pels > 1024) + u32::from(tcq_two_d);
    let dq_denom = 1u32 << dq_shift;
    let params = DequantBlockParams {
        dc_quant: dc_quantizer(plane_id, qindex, deltas, bit_depth),
        ac_quant: ac_quantizer(plane_id, qindex, deltas, bit_depth),
        tx_width: adj_w,
        tx_height: adj_h,
        dq_denom,
        bit_depth,
        qm: resolve_block_qm(
            plane_id,
            plane_tx_type,
            adj_w,
            adj_h,
            log2_width,
            log2_height,
        ),
    };
    let transform = InverseTransform2dOuter::resolve(
        plane_tx_type,
        log2_width,
        log2_height,
        use_ddt,
        false,
        bit_depth,
        None,
    )
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;

    let mut out = vec![T::default(); samples];
    with_residual_scratch(|scratch| {
        let dequant_scratch = &mut scratch.dequant[..adjusted.min(MAX_ADJUSTED_COEFFS)];
        let residual_scratch = &mut scratch.residual[..samples.min(MAX_ORIGINAL_SAMPLES)];
        reconstruct_transform_block_residual_with_secondary(
            prediction,
            quant,
            &params,
            &transform,
            secondary,
            dequant_scratch,
            residual_scratch,
            &mut out,
        )
    })
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;
    Ok(out)
}

/// Maximum adjusted coefficient count (§ 7.15.4 caps each adjusted side at 32).
const MAX_ADJUSTED_COEFFS: usize = 32 * 32;

/// Maximum original transform-block sample count (a 64x64 transform).
const MAX_ORIGINAL_SAMPLES: usize = 64 * 64;

/// Reusable per-thread working buffers for the § 7.14.4 → § 7.15.4 residual
/// chain; every used slot is fully overwritten by the chain before it is read.
/// `InverseTransform2dOuter::resolve` bounds `adjusted <= 1024` and
/// `samples <= 4096`, so the `min`-clamped slices are total and any
/// inconsistency is rejected by the chain's own buffer-length checks.
struct ResidualScratch {
    dequant: [i32; MAX_ADJUSTED_COEFFS],
    residual: [i32; MAX_ORIGINAL_SAMPLES],
}

thread_local! {
    static RESIDUAL_SCRATCH: std::cell::Cell<Option<Box<ResidualScratch>>> =
        const { std::cell::Cell::new(None) };
}

fn with_residual_scratch<R>(f: impl FnOnce(&mut ResidualScratch) -> R) -> R {
    RESIDUAL_SCRATCH.with(|cell| {
        let mut scratch = cell.take().unwrap_or_else(|| {
            Box::new(ResidualScratch {
                dequant: [0; MAX_ADJUSTED_COEFFS],
                residual: [0; MAX_ORIGINAL_SAMPLES],
            })
        });
        let result = f(&mut scratch);
        cell.set(Some(scratch));
        result
    })
}

fn intra_secondary_inverse_transform(
    block: &LumaCoeffBlock,
    log2_width: u32,
    log2_height: u32,
    bit_depth: BitDepth,
    luma_context: LumaTransformTypeContext,
) -> Result<Option<SecondaryInverseTransform>, GeneralIntraResidualError> {
    let Some(ist) = block.intra_ist else {
        return Ok(None);
    };
    if ist.sec_tx_type == 0 {
        return Ok(None);
    }
    let most_probable_stx_set =
        ist.most_probable_stx_set
            .ok_or(unsupported_transform_tool_residual_error(
                "unsupported_dctonly_residual_intra_ist_missing_most_probable_stx_set",
            ))?;
    let tx_width = transform_dimension(log2_width)?;
    let tx_height = transform_dimension(log2_height)?;
    let w = tx_width.min(32);
    let h = tx_height.min(32);
    let large = w >= 8 && h >= 8;
    let n = if !large {
        IST_4X4_HEIGHT
    } else if (tx_width == 8 && tx_height == 8) || block.plane_tx_type == ADST_ADST {
        IST_8X8_HEIGHT_RED
    } else {
        IST_8X8_HEIGHT
    };
    let mode = intra_secondary_transform_mode(luma_context, tx_width, tx_height)?;
    let kernel = intra_secondary_transform_kernel(
        mode,
        block.plane_tx_type,
        most_probable_stx_set,
        tx_width,
        tx_height,
    )?;
    let transpose = matches!(mode, H_PRED | D157_PRED | D67_PRED | SMOOTH_H_PRED);
    Ok(Some(SecondaryInverseTransform {
        w,
        h,
        n,
        kernel,
        sec_tx_type: ist.sec_tx_type,
        transpose,
        bit_depth,
    }))
}

fn transform_dimension(log2_dim: u32) -> Result<usize, GeneralIntraResidualError> {
    if !(2..=6).contains(&log2_dim) {
        return unsupported_transform_tool_residual(
            "unsupported_dctonly_residual_intra_ist_invalid_transform_shape",
        );
    }
    Ok(1usize << log2_dim)
}

fn intra_secondary_transform_mode(
    luma_context: LumaTransformTypeContext,
    tx_width: usize,
    tx_height: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let mode = luma_context.y_mode.value();
    if !luma_context.y_mode.is_directional() {
        return Ok(mode);
    }
    let mode_to_angle =
        MODE_TO_ANGLE
            .get(mode)
            .copied()
            .ok_or(unsupported_transform_tool_residual_error(
                "unsupported_dctonly_residual_intra_ist_invalid_intra_mode",
            ))?;
    let mrl_delta = MRL_INDEX_TO_DELTA
        .get(usize::from(luma_context.mrl_index))
        .copied()
        .ok_or(unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_intra_ist_invalid_mrl_index",
        ))?;
    let p_angle = mode_to_angle
        .checked_add(i32::from(luma_context.angle_delta_y) * ANGLE_STEP)
        .and_then(|angle| angle.checked_add(mrl_delta))
        .ok_or(unsupported_transform_tool_residual_error(
            "unsupported_dctonly_residual_intra_ist_angle_overflow",
        ))?;
    Ok(wide_angle_mapping(mode, tx_width, tx_height, p_angle))
}

fn intra_secondary_transform_kernel(
    mode: usize,
    plane_tx_type: usize,
    most_probable_stx_set: usize,
    tx_width: usize,
    tx_height: usize,
) -> Result<usize, GeneralIntraResidualError> {
    if mode >= INV_MOST_PROBABLE_STX_MAPPING.len() {
        return unsupported_transform_tool_residual(
            "unsupported_dctonly_residual_intra_ist_invalid_intra_mode",
        );
    }
    let base = if plane_tx_type == ADST_ADST && tx_width >= 8 && tx_height >= 8 {
        INV_MOST_PROBABLE_STX_MAPPING_ADST[mode]
            .get(most_probable_stx_set)
            .copied()
    } else {
        INV_MOST_PROBABLE_STX_MAPPING[mode]
            .get(most_probable_stx_set)
            .copied()
    }
    .ok_or(unsupported_transform_tool_residual_error(
        "unsupported_dctonly_residual_intra_ist_invalid_most_probable_stx_set",
    ))?;
    if plane_tx_type == ADST_ADST {
        base.checked_add(7)
            .ok_or(unsupported_transform_tool_residual_error(
                "unsupported_dctonly_residual_intra_ist_kernel_overflow",
            ))
    } else {
        Ok(base)
    }
}

fn txb_skip_tx_size_ctx(tx_size: usize) -> usize {
    let sqr = TX_SIZE_SQR.get(tx_size).copied().unwrap_or(0);
    let sqr_up = TX_SIZE_SQR_UP.get(tx_size).copied().unwrap_or(0);
    (((sqr + sqr_up + 1) >> 1).max(0)) as usize
}

#[cfg(test)]
mod tests;
