// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra transform-block coefficient decode.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-TRANSFORM-UNIT-PREDICTION`.

use splot_core::Error as CoreError;
use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MAX_TX_SIZE_RECT, MD_IDX_TO_TYPE, MODE_TO_ANGLE, NUM_4X4_BLOCKS_HIGH, NUM_4X4_BLOCKS_WIDE,
    SIZE_CLASS, SIZE_TO_TX_PART_GROUP_LOOKUP, SIZE_TO_TX_TYPE_GROUP_VERT_AND_HORZ,
    SIZE_TO_TX_TYPE_GROUP_VERT_OR_HORZ, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR, TX_SIZE_SQR_UP,
    TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::{
    BitDepth, DpcmDirection, PlaneId, QM_OFFSET, QmDequant, QmFrameLevels, QmUserPlane,
    QuantizerDeltas, ReconError, ReconSample, SecondaryInverseTransform,
    reconstruct_transform_block_residual_with_secondary, tx_size_index,
};

use super::cdf::block_context::IntraYMode;
use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::cdf::{TileCdfSelector, coeff_cdf_q_ctx_from_base_q_idx};
use super::coeff_loop::fsc_quant_pass::{
    CoeffFscBranchError, CoeffFscStagedTxSizeNonZeroInput,
    apply_staged_nonzero_coeff_fsc_branch_from_tx_size,
};
use super::coeff_loop::max_level::CoeffTransformClass;
use super::coeff_loop::ordinary_pass::CoeffOrdinaryBranchError;
use super::coeff_loop::ordinary_pass::geometry::{
    CoeffOrdinaryBranchModeToTxfmBaseConfig, CoeffOrdinaryBranchTxSetBaseConfig,
    CoeffOrdinaryStagedLosslessNonZeroInput, CoeffOrdinaryTxSizeGeometryConfig,
    apply_staged_nonzero_coeff_ordinary_branch_from_lossless, lossless_plane_tx_type,
    read_lossless_inter_plane_tx_type, resolve_mode_to_txfm_plane_tx_type,
};
use super::coeff_loop::{
    AllZeroCoeffBlockInput, CoeffLoopContextError, NonZeroCoeffBlockStartInput,
    NonZeroCoeffEobContextInput, read_nonzero_coeff_block_start,
};
use super::coeff_state::{CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError};
use super::{DecodeTileWorkUnit, TileCdfSubset, TileCoeffFrameFacts};

mod cctx;
mod luma_transform_partition;
mod reconstruct;

#[cfg(test)]
use cctx::apply_cross_chroma_transform;
pub(crate) use cctx::{
    reconstruct_general_intra_chroma_cctx_pair_into,
    reconstruct_general_intra_chroma_cctx_pair_with_predictions,
};
#[cfg(test)]
use luma_transform_partition::MAX_LUMA_TRANSFORM_PARTITION_UNITS;
pub(crate) use luma_transform_partition::{
    LumaTransformPartitionContext, LumaTransformPartitionUnits,
};
use reconstruct::reconstruct_block_setup;
pub(crate) use reconstruct::{
    reconstruct_general_intra_coeff_block_rect_into_frame,
    reconstruct_general_intra_coeff_block_rect_with_prediction_into,
    reconstruct_inter_coeff_block_residual_rect_into,
};

const TX_4X4: usize = 0;
#[cfg(test)]
const TX_64X64: usize = 4;
const TX_8X8: usize = 1;
#[cfg(test)]
const TX_16X16: usize = 2;
const TX_32X32: usize = 3;
#[cfg(test)]
const TX_8X4: usize = 6;
#[cfg(test)]
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
const ZERO_QUANTIZER_DELTAS: QuantizerDeltas = QuantizerDeltas {
    y_dc: 0,
    u_dc: 0,
    v_dc: 0,
    u_ac: 0,
    v_ac: 0,
};
const NUM_CUSTOM_QMS: usize = 15;
use std::{cell::RefCell, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct FrameUserQmLevel {
    pub(crate) transforms: [[Option<QmUserPlane>; 3]; 3],
}

pub(crate) type FrameUserQmLevels = Arc<[Option<FrameUserQmLevel>; NUM_CUSTOM_QMS]>;

thread_local! {
    static FRAME_QUANTIZER_DELTAS: core::cell::Cell<QuantizerDeltas> =
        const { core::cell::Cell::new(ZERO_QUANTIZER_DELTAS) };
    static FRAME_QM: core::cell::Cell<Option<QmFrameLevels>> = const { core::cell::Cell::new(None) };
    static FRAME_USER_QM: RefCell<Option<FrameUserQmLevels>> = const { RefCell::new(None) };
    static FRAME_QM_SEGMENT_ID: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

macro_rules! frame_cell_scope {
    ($name:ident, $cell:ident, $value:ty) => {
        pub(crate) struct $name($value);

        impl $name {
            pub(crate) fn install(value: $value) -> Self {
                Self($cell.with(|cell| cell.replace(value)))
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                $cell.with(|cell| cell.set(self.0));
            }
        }
    };
}

frame_cell_scope!(
    FrameQuantizerDeltasScope,
    FRAME_QUANTIZER_DELTAS,
    QuantizerDeltas
);
frame_cell_scope!(FrameQmScope, FRAME_QM, Option<QmFrameLevels>);
frame_cell_scope!(FrameQmSegmentScope, FRAME_QM_SEGMENT_ID, usize);

pub(crate) struct FrameUserQmScope(Option<FrameUserQmLevels>);

impl FrameUserQmScope {
    pub(crate) fn install(levels: Option<FrameUserQmLevels>) -> Self {
        Self(FRAME_USER_QM.with(|cell| cell.replace(levels)))
    }
}

impl Drop for FrameUserQmScope {
    fn drop(&mut self) {
        let _ = FRAME_USER_QM.with(|cell| cell.replace(self.0.take()));
    }
}

pub(crate) fn current_frame_qm_segment_id() -> usize {
    FRAME_QM_SEGMENT_ID.with(core::cell::Cell::get)
}

#[derive(Clone, Debug)]
pub(crate) struct FrameQuantizerSnapshot {
    deltas: QuantizerDeltas,
    qm: Option<QmFrameLevels>,
    user_qm: Option<FrameUserQmLevels>,
}

impl FrameQuantizerSnapshot {
    pub(crate) fn capture() -> Self {
        Self {
            deltas: FRAME_QUANTIZER_DELTAS.with(core::cell::Cell::get),
            qm: FRAME_QM.with(core::cell::Cell::get),
            user_qm: FRAME_USER_QM.with(|cell| cell.borrow().clone()),
        }
    }

    pub(crate) fn install_frame(
        &self,
    ) -> (FrameQuantizerDeltasScope, FrameQmScope, FrameUserQmScope) {
        (
            FrameQuantizerDeltasScope::install(self.deltas),
            FrameQmScope::install(self.qm),
            FrameUserQmScope::install(self.user_qm.clone()),
        )
    }
}

fn current_quantizer_deltas() -> QuantizerDeltas {
    FRAME_QUANTIZER_DELTAS.with(core::cell::Cell::get)
}

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
        let segment_id = current_frame_qm_segment_id();
        levels
            .levels_le8
            .get(segment_id)
            .map_or(NUM_CUSTOM_QMS as u8, |level| level[plane_idx])
    });
    if seg_level >= NUM_CUSTOM_QMS {
        return None;
    }
    let tx_sz = tx_size_index(log2_width, log2_height).ok()?;
    let qm_offset = usize::try_from(*QM_OFFSET.get(tx_sz)?).ok()?;
    let transform = match tw.cmp(&th) {
        core::cmp::Ordering::Less => 2,
        core::cmp::Ordering::Greater => 1,
        core::cmp::Ordering::Equal => 0,
    };
    let user = if tw <= 8 && th <= 8 {
        FRAME_USER_QM.with(|cell| {
            let levels = cell.borrow();
            levels.as_ref()?[seg_level].as_ref()?.transforms[transform][plane_idx].clone()
        })
    } else {
        None
    };
    Some(QmDequant {
        seg_level,
        plane_is_chroma: plane_idx != 0,
        qm_offset,
        user,
    })
}
const H_PRED: usize = 2;
const D45_PRED: usize = 3;
const D157_PRED: usize = 6;
const D203_PRED: usize = 7;
const D67_PRED: usize = 8;
const SMOOTH_H_PRED: usize = 11;
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
/// AV2 § 6.19.6.3 Table 6.23 `TX_PARTITION_*`, in `txPartition` value order;
/// read per § 5.20.6.3.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LumaTxPartition {
    None,
    Split,
    Horz,
    Vert,
    Horz4,
    Vert4,
    Horz5,
    Vert5,
}

impl LumaTxPartition {
    /// Maps the partition-type symbol, which codes every type except `None`.
    ///
    /// The `TxPartitionType` rows are `TX_PARTITION_TYPE_ROW_LEN` wide, so the
    /// symbol decoder cannot return above `Vert5`.
    const fn from_partition_type_symbol(symbol: u8) -> Self {
        match symbol {
            0 => Self::Split,
            1 => Self::Horz,
            2 => Self::Vert,
            3 => Self::Horz4,
            4 => Self::Vert4,
            5 => Self::Horz5,
            _ => Self::Vert5,
        }
    }
}
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
    dpcm: Option<DpcmDirection>,
}

impl LumaTransformTypeContext {
    #[must_use]
    pub(crate) const fn new(y_mode: IntraYMode, angle_delta_y: i8) -> Self {
        Self::with_mrl_indices(y_mode, angle_delta_y, 0, None, None)
    }

    #[must_use]
    pub(crate) const fn with_mrl_indices(
        y_mode: IntraYMode,
        angle_delta_y: i8,
        mrl_index: u8,
        mrl_sec_index: Option<u8>,
        dpcm: Option<DpcmDirection>,
    ) -> Self {
        Self {
            y_mode,
            angle_delta_y,
            mrl_index,
            mrl_sec_index,
            dpcm,
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

/// The § 5.20.6.1 luma transform-type context a residual read hands to the
/// transform-tool path, which every sequence takes: the intra transform set has
/// more than one type whatever the § 5.4.8 tool flags say.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TransformToolResidualPolicy {
    pub(crate) luma: Option<LumaTransformTypeContext>,
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
    pub(crate) cctx_type: Option<usize>,
    pub(crate) plane_tx_type: usize,
    pub(crate) use_tcq: bool,
    pub(crate) lossless: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PositionedLumaCoeffBlock {
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) tx_size: usize,
    pub(crate) middle: bool,
    pub(crate) coeffs: LumaCoeffBlock,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraResidualError {
    #[error("general intra luma all_zero symbol read failed: {source}")]
    AllZeroRead { source: BlockSymbolTraceReadError },
    #[error("general intra luma coefficient context state failed: {source}")]
    CoeffContextState { source: TileCoeffStateError },
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
}

fn or_u8(line: &[u8], start: usize, len: usize) -> u8 {
    line.iter().skip(start).take(len).fold(0, |acc, &v| acc | v)
}

fn coeff_ctx_err(source: TileCoeffStateError) -> GeneralIntraResidualError {
    GeneralIntraResidualError::CoeffContextState { source }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub(crate) fn decode_general_intra_luma_partition_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    frame_width: usize,
    frame_height: usize,
    tx_fills_block: bool,
    luma_tx_partition: LumaTransformPartitionContext,
    uv_mode: usize,
    angle_delta_uv: i32,
    fsc_mode: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
) -> Result<LumaTransformPartitionUnits<PositionedLumaCoeffBlock>, GeneralIntraResidualError> {
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
    let mut blocks = LumaTransformPartitionUnits::new();
    let record_count = records.len();
    for record in records
        .into_iter()
        .filter(|record| luma_transform_record_starts_in_frame(record, frame_width, frame_height))
    {
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
            false,
            uv_mode,
            angle_delta_uv,
            DCT_DCT,
            false,
            fsc_mode,
            fsc_mode,
            false,
            transform_tool_residual_policy,
        )?;
        blocks.push(PositionedLumaCoeffBlock {
            x: record.x,
            y: record.y,
            tx_size: record.tx_size,
            middle: record.middle,
            coeffs,
        })?;
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

fn luma_transform_record_starts_in_frame(
    record: &LumaTransformPartitionRecord,
    frame_width: usize,
    frame_height: usize,
) -> bool {
    record.x < frame_width && record.y < frame_height
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
) -> Result<LumaTransformPartitionUnits<LumaTransformPartitionRecord>, GeneralIntraResidualError> {
    if context.mi_size == BLOCK_4X4 {
        let mut records = LumaTransformPartitionUnits::new();
        records.push(luma_transform_record(start_x, start_y, tx_size))?;
        return Ok(records);
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
            * MI_SIZE;
    let block_height =
        block_size_table_usize(&NUM_4X4_BLOCKS_HIGH, "Num_4x4_Blocks_High", context.mi_size)?
            * MI_SIZE;
    if (block_width >> 6) > 1 || (block_height >> 6) > 1 {
        let mut records = LumaTransformPartitionUnits::new();
        records.push(luma_transform_record(start_x, start_y, tx_size))?;
        return Ok(records);
    }

    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    let allow_horz = tx_size_from_dimensions(tx_width, tx_height >> 1).is_some();
    let allow_vert = tx_size_from_dimensions(tx_width >> 1, tx_height).is_some();
    if !allow_horz && !allow_vert {
        let mut records = LumaTransformPartitionUnits::new();
        records.push(luma_transform_record(start_x, start_y, tx_size))?;
        return Ok(records);
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
) -> Result<LumaTxPartition, GeneralIntraResidualError> {
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
        return Ok(LumaTxPartition::None);
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
        return Ok(LumaTxPartition::from_partition_type_symbol(symbol));
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
                LumaTxPartition::Horz4
            } else {
                LumaTxPartition::Horz
            }
        } else if tx_2or3 != 0 {
            LumaTxPartition::Vert4
        } else {
            LumaTxPartition::Vert
        });
    }
    Ok(if allow_horz {
        LumaTxPartition::Horz
    } else {
        LumaTxPartition::Vert
    })
}

fn luma_transform_records_for_partition(
    start_x: usize,
    start_y: usize,
    tx_size: usize,
    tx_partition: LumaTxPartition,
) -> Result<LumaTransformPartitionUnits<LumaTransformPartitionRecord>, GeneralIntraResidualError> {
    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    let mut w4 = tx_width / MI_SIZE;
    let mut h4 = tx_height / MI_SIZE;
    let col4 = start_x / MI_SIZE;
    let row4 = start_y / MI_SIZE;
    let mut records = LumaTransformPartitionUnits::new();
    match tx_partition {
        LumaTxPartition::None => {
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
        }
        LumaTxPartition::Horz => {
            h4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4, w4, false)?;
        }
        LumaTxPartition::Vert => {
            w4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4, w4, false)?;
        }
        LumaTxPartition::Horz4 => {
            h4 >>= 2;
            for part in 0..4 {
                push_luma_transform_record(&mut records, row4 + part * h4, col4, h4, w4, false)?;
            }
        }
        LumaTxPartition::Vert4 => {
            w4 >>= 2;
            for part in 0..4 {
                push_luma_transform_record(&mut records, row4, col4 + part * w4, h4, w4, false)?;
            }
        }
        LumaTxPartition::Horz5 => {
            h4 >>= 2;
            w4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4 << 1, w4 << 1, true)?;
            push_luma_transform_record(&mut records, row4 + h4 * 3, col4, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4 + h4 * 3, col4 + w4, h4, w4, true)?;
        }
        LumaTxPartition::Vert5 => {
            h4 >>= 1;
            w4 >>= 2;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4 << 1, w4 << 1, true)?;
            push_luma_transform_record(&mut records, row4, col4 + w4 * 3, h4, w4, true)?;
            push_luma_transform_record(&mut records, row4 + h4, col4 + w4 * 3, h4, w4, true)?;
        }
        LumaTxPartition::Split => {
            w4 >>= 1;
            h4 >>= 1;
            push_luma_transform_record(&mut records, row4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4, col4 + w4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4, h4, w4, false)?;
            push_luma_transform_record(&mut records, row4 + h4, col4 + w4, h4, w4, false)?;
        }
    }
    Ok(records)
}

fn push_luma_transform_record(
    records: &mut LumaTransformPartitionUnits<LumaTransformPartitionRecord>,
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
    let width = w4 * MI_SIZE;
    let height = h4 * MI_SIZE;
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
    })
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
    eob_u_nonzero: bool,
    uv_mode: usize,
    angle_delta_uv: i32,
    chroma_inter_tx_type: usize,
    is_inter: bool,
    fsc_mode: bool,
    txb_skip_fsc_mode: bool,
    cctx_allowed: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let frame_facts = work_unit.coeff_frame_facts();
    let lossless = frame_facts
        .lossless_for_segment(current_frame_qm_segment_id())
        .unwrap_or(false);
    let tx_size = if lossless && !is_inter && !fsc_mode {
        TX_4X4
    } else {
        tx_size
    };
    let x4 = start_x >> 2;
    let y4 = start_y >> 2;
    let w4 = usize::try_from(TX_WIDTH.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let h4 = usize::try_from(TX_HEIGHT.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let coeff_cdf_q_ctx = coeff_cdf_q_ctx_from_base_q_idx(frame_facts.base_q_idx());
    let tx_size_ctx = txb_skip_tx_size_ctx(tx_size);

    let local_x4 = context.local_x4(plane, x4).map_err(coeff_ctx_err)?;
    let local_y4 = context.local_y4(plane, y4).map_err(coeff_ctx_err)?;
    let above_level_or = u32::from(or_u8(
        context.above_level(plane).map_err(coeff_ctx_err)?,
        local_x4,
        w4,
    ));
    let left_level_or = u32::from(or_u8(
        context.left_level(plane).map_err(coeff_ctx_err)?,
        local_y4,
        h4,
    ));
    let txb_skip_intra_inter = usize::from(is_inter || txb_skip_fsc_mode);
    let selector = match plane {
        1 | 2 => {
            let above_nz = above_level_or != 0
                || or_u8(
                    context.above_dc(plane).map_err(coeff_ctx_err)?,
                    local_x4,
                    w4,
                ) != 0;
            let left_nz = left_level_or != 0
                || or_u8(context.left_dc(plane).map_err(coeff_ctx_err)?, local_y4, h4) != 0;
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
            cctx_type: None,
            plane_tx_type: DCT_DCT,
            use_tcq: false,
            lossless: frame_facts
                .lossless_for_segment(current_frame_qm_segment_id())
                .unwrap_or(false),
        });
    }

    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane,
        start_x,
        start_y,
        tx_size,
    };
    let TransformToolResidualPolicy { luma } = transform_tool_residual_policy;
    decode_staged_transform_tool_nonzero_coeffs(
        work_unit,
        symbols,
        context,
        frame_facts,
        geometry,
        coeff_cdf_q_ctx,
        uv_mode,
        angle_delta_uv,
        chroma_inter_tx_type,
        is_inter,
        fsc_mode,
        txb_skip_fsc_mode,
        cctx_allowed,
        luma,
    )
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn decode_staged_transform_tool_nonzero_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    frame_facts: TileCoeffFrameFacts,
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    coeff_cdf_q_ctx: usize,
    uv_mode: usize,
    angle_delta_uv: i32,
    chroma_inter_tx_type: usize,
    is_inter: bool,
    fsc_mode: bool,
    txb_skip_fsc_mode: bool,
    cctx_allowed: bool,
    luma_transform_type_context: Option<LumaTransformTypeContext>,
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
    let start = read_nonzero_coeff_block_start(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        NonZeroCoeffBlockStartInput {
            block,
            eob: NonZeroCoeffEobContextInput {
                plane: geometry.plane,
                is_inter,
                tx_width_log2,
                tx_height_log2,
                coeff_cdf_q_ctx,
            },
        },
    )
    .map_err(|source| GeneralIntraResidualError::NonZeroStart { source })?;
    let eob = start.eob_read().eob().eob();
    let segment_id = current_frame_qm_segment_id();
    let lossless = frame_facts.lossless_for_segment(segment_id).ok_or(
        unsupported_transform_tool_residual_error("unsupported_dctonly_residual_segment_id"),
    )?;
    let metadata = ensure_transform_tool_residual_handoff(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        TransformToolResidualInput {
            frame_facts,
            plane: geometry.plane,
            tx_size: geometry.tx_size,
            is_inter,
            lossless,
            fsc_mode: fsc_mode || (geometry.plane > 0 && txb_skip_fsc_mode),
            eob,
            cctx_allowed,
            luma_transform_type_context,
        },
    )?;
    let use_fsc = frame_facts.enable_fsc()
        && metadata.luma_tx_type == IDTX
        && geometry.plane == 0
        && (fsc_mode || is_inter);
    let mut base_config = staged_transform_tool_lossless_base_config(
        frame_facts,
        geometry.plane,
        uv_mode,
        angle_delta_uv,
        chroma_inter_tx_type,
        lossless,
        metadata,
    );
    if use_fsc {
        base_config.use_tcq = false;
    }
    let plane_tx_type =
        staged_transform_tool_plane_tx_type(geometry, is_inter, lossless, base_config)?;
    if use_fsc {
        let block = apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
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
            eob,
            quant: block.into_quant(),
            intra_ist: metadata.intra_ist,
            cctx_type: metadata.cctx_type,
            plane_tx_type,
            use_tcq: false,
            lossless,
        });
    }
    let block = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
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
        quant: block.into_quant(),
        intra_ist: metadata.intra_ist,
        cctx_type: metadata.cctx_type,
        plane_tx_type,
        use_tcq: base_config.use_tcq,
        lossless,
    })
}

fn staged_transform_tool_lossless_base_config(
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    uv_mode: usize,
    angle_delta_uv: i32,
    chroma_inter_tx_type: usize,
    lossless: bool,
    metadata: TransformToolResidualMetadata,
) -> CoeffOrdinaryBranchTxSetBaseConfig {
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
    CoeffOrdinaryBranchTxSetBaseConfig {
        reduced_tx_set: frame_facts.reduced_tx_set(),
        enable_chroma_dctonly: frame_facts.enable_chroma_dctonly(),
        uv_mode,
        angle_delta_uv,
        luma_tx_type: metadata.luma_tx_type,
        chroma_inter_tx_type,
        parity_hiding,
        use_tcq,
    }
}

fn staged_transform_tool_plane_tx_type(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    lossless: bool,
    base_config: CoeffOrdinaryBranchTxSetBaseConfig,
) -> Result<usize, GeneralIntraResidualError> {
    if lossless {
        return Ok(lossless_plane_tx_type(geometry, is_inter, base_config));
    }
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
    lossless: bool,
    fsc_mode: bool,
    eob: usize,
    cctx_allowed: bool,
    luma_transform_type_context: Option<LumaTransformTypeContext>,
}

/// § 5.20.7.27 `is_cctx_allowed` block-geometry clause: cross-chroma transforms
/// stay available for 4:2:0 and for chroma plane residual blocks under 32
/// samples in either dimension.
pub(crate) const fn is_cctx_geometry_allowed(
    is_420: bool,
    plane_width: usize,
    plane_height: usize,
) -> bool {
    is_420 || plane_width < 32 || plane_height < 32
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
    let lossless = input.lossless;
    let eob = input.eob;
    let mut metadata = TransformToolResidualMetadata {
        luma_tx_type: DCT_DCT,
        ..TransformToolResidualMetadata::default()
    };
    if plane == 1
        && frame_facts.enable_cctx()
        && !lossless
        && input.cctx_allowed
        && (is_inter || eob != 1)
    {
        let cctx_type = read_transform_symbol(cdfs, symbols, TileCdfSelector::CctxType)?;
        metadata.cctx_type = Some(cctx_type);
    }
    if lossless {
        if !is_inter && input.fsc_mode {
            metadata.luma_tx_type = IDTX;
        } else if is_inter && plane == 0 {
            metadata.luma_tx_type = read_lossless_inter_plane_tx_type(cdfs, symbols, tx_size)
                .map_err(|source| GeneralIntraResidualError::TransformTypeRead { source })?;
        }
        return Ok(metadata);
    }
    let tx_set = transform_set(frame_facts, plane, tx_size, is_inter)?;
    let dct_forced = (!is_inter && plane == 0 && eob == 1)
        || (plane > 0 && frame_facts.enable_chroma_dctonly())
        || tx_set == TX_SET_DCTONLY
        || (!is_inter && plane == 0 && frame_facts.reduced_tx_set() == 2);
    if !is_inter && plane == 0 && input.fsc_mode {
        metadata.luma_tx_type = IDTX;
    } else if !(dct_forced || plane > 0) {
        if !is_inter && plane == 0 {
            let luma_tx_type = read_active_luma_transform_type(
                cdfs,
                symbols,
                input.luma_transform_type_context,
                tx_size,
                tx_set,
            )?;
            metadata.luma_tx_type = luma_tx_type;
        } else {
            let luma_tx_type =
                read_active_inter_transform_type(cdfs, symbols, tx_size, tx_set, eob)?;
            metadata.luma_tx_type = luma_tx_type;
        }
    }
    if is_inter
        && plane == 0
        && !lossless
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
        && !lossless
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
            metadata.luma_tx_type,
        )?;
    }
    Ok(metadata)
}

fn read_intra_ist_sec_tx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    luma_context: Option<LumaTransformTypeContext>,
    tx_size: usize,
    luma_tx_type: usize,
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
            intra_ist_most_probable_stx_selector(tx_size, luma_tx_type)?,
        )?)
    };
    let syntax = IntraIstSyntax {
        sec_tx_type,
        most_probable_stx_set,
    };
    Ok(Some(syntax))
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

fn intra_ist_most_probable_stx_selector(
    tx_size: usize,
    luma_tx_type: usize,
) -> Result<TileCdfSelector, GeneralIntraResidualError> {
    let (tx_width, tx_height) = tx_size_dimensions(tx_size)?;
    if luma_tx_type == ADST_ADST && tx_width >= 8 && tx_height >= 8 {
        Ok(TileCdfSelector::MostProbableStxSetAdst)
    } else {
        Ok(TileCdfSelector::MostProbableStxSet)
    }
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

/// Reads the signalled intra luma transform type for `tx_set`.
///
/// The caller's `dct_forced` test has already excluded `TX_SET_DCTONLY` and the
/// `reduced_tx_set == 2` intra case, so the trailing arm is `TX_SET_INTRA_1`.
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
            let intra_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr },
            )?;
            md_idx_luma_tx_type(tx_size, luma_context, intra_tx_type)?
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
    let shape = long_tx_set_shape(tx_set);
    let is_long_side_dct = read_long_side_dct_symbol(cdfs, symbols, shape, 0)?;
    let intra_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::IntraTxTypeLong { tx_size_sqr },
    )?;
    Ok(long_tx_type_from_index(
        shape,
        is_long_side_dct,
        intra_tx_type,
    ))
}

/// Reads the signalled inter transform type for `tx_set`.
///
/// The caller's `dct_forced` test has already excluded `TX_SET_DCTONLY`, so the
/// trailing arm is `TX_SET_DCT_IDTX_IDDCT`.
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
        _ => {
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
    }
}

fn read_active_inter_long_tx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_set: usize,
    tx_size_sqr: usize,
    ctx: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let shape = long_tx_set_shape(tx_set);
    let is_long_side_dct = read_long_side_dct_symbol(cdfs, symbols, shape, 1)?;
    let inter_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::InterTxTypeLong { ctx, tx_size_sqr },
    )?;
    Ok(long_tx_type_from_index(
        shape,
        is_long_side_dct,
        inter_tx_type,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LongTxSetShape {
    wide_or_high: usize,
    long_side_dct_is_forced: bool,
}

/// Splits a long transform set into its two independent bits.
///
/// Only the four long sets reach here, and for those both bits are predicates
/// over `tx_set`, so neither needs a fallible lookup.
const fn long_tx_set_shape(tx_set: usize) -> LongTxSetShape {
    LongTxSetShape {
        wide_or_high: matches!(tx_set, TX_SET_HIGH_64 | TX_SET_HIGH_32) as usize,
        long_side_dct_is_forced: matches!(tx_set, TX_SET_WIDE_64 | TX_SET_HIGH_64),
    }
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

/// Inverts a long transform set's `(is_long_side_dct, shape, tx_type)` triple.
///
/// `is_long_side_dct` and `wide_or_high` are binary, and the `IntraTxTypeLong`
/// and `InterTxTypeLong` rows are five wide, so `tx_type` is 0..=3. Every index
/// is therefore a literal and the lookup is total.
const fn long_tx_type_from_index(
    shape: LongTxSetShape,
    is_long_side_dct: usize,
    tx_type: usize,
) -> usize {
    let long_side = if is_long_side_dct == 0 {
        TX_TYPE_INV_LONG[0]
    } else {
        TX_TYPE_INV_LONG[1]
    };
    let row = if shape.wide_or_high == 0 {
        long_side[0]
    } else {
        long_side[1]
    };
    match tx_type {
        0 => row[0],
        1 => row[1],
        2 => row[2],
        _ => row[3],
    }
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

/// § 8.3.2 `InterTxTypeLong` context from the last coefficient position.
///
/// `eob` is the non-zero-block end of block, which `nonzero_coeff_eob` builds
/// from an `eob_pt` of 1..=11: below 3 it is the point itself, and above it
/// starts at `EOB_GROUP_START[3]` = 3. It is never 0, so the last-position
/// conversion is total.
fn inter_tx_type_long_ctx(tx_size: usize, eob: usize) -> Result<usize, GeneralIntraResidualError> {
    let eob = eob.saturating_sub(1);
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

/// Resolves the § 9 `Md_Idx_To_Type` transform type for a signalled intra index.
///
/// Every `Md_Idx_To_Type` entry is a transform type in 0..=14, so widening the
/// table's `i32` to `usize` is total.
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
    Ok(tx_type.unsigned_abs() as usize)
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
fn reconstruct_general_intra_block_rect_with_prediction_core<T: ReconSample>(
    quant: &[i32],
    prediction: &[T],
    out: &mut Vec<T>,
    qindex: u32,
    plane_id: PlaneId,
    log2_width: u32,
    log2_height: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    use_ddt: bool,
    lossless: bool,
    secondary: Option<&SecondaryInverseTransform>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Result<(), GeneralIntraResidualError> {
    let setup = reconstruct_block_setup(
        prediction.len(),
        qindex,
        plane_id,
        log2_width,
        log2_height,
        plane_tx_type,
        use_tcq,
        use_ddt,
        lossless,
        dpcm,
        bit_depth,
    )?;
    if quant.len() != setup.adjusted {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: setup.adjusted,
            actual: quant.len(),
        });
    }
    out.clear();
    out.resize(setup.samples, T::default());
    with_residual_scratch(|scratch| {
        let dequant_scratch = &mut scratch.dequant[..setup.adjusted];
        let residual_scratch = &mut scratch.residual[..setup.samples];
        reconstruct_transform_block_residual_with_secondary(
            prediction,
            quant,
            &setup.params,
            &setup.transform,
            secondary,
            dequant_scratch,
            residual_scratch,
            out.as_mut_slice(),
        )
    })
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;
    Ok(())
}

const MAX_ADJUSTED_COEFFS: usize = 32 * 32;

const MAX_ORIGINAL_SAMPLES: usize = 64 * 64;

struct ResidualScratch {
    dequant: [i32; MAX_ADJUSTED_COEFFS],
    dequant_pair: [i32; MAX_ADJUSTED_COEFFS],
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
                dequant_pair: [0; MAX_ADJUSTED_COEFFS],
                residual: [0; MAX_ORIGINAL_SAMPLES],
            })
        });
        let result = f(&mut scratch);
        cell.set(Some(scratch));
        result
    })
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
