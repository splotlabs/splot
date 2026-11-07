// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    ADJUSTED_TX_SIZE, MODE_TO_ANGLE, MODE_TO_TXFM, TX_HEIGHT, TX_SIZE_SQR, TX_SIZE_SQR_UP,
    TX_WIDTH, TX_WIDTH_LOG2,
};

use crate::bitstream::tile_payload::general_intra_residual::wide_angle_mapping;

use super::super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::super::cdf::{TileCdfSelector, TileCdfSubset};
use super::super::super::coeff_state::TileCoeffContextState;
use super::super::base_level_pass::CoeffBaseDerivedLevelPassConfig;
use super::super::branch::NonZeroCoeffBlockStart;
use super::super::max_level::CoeffTransformClass;
use super::{
    CoeffOrdinaryBranchError, CoeffOrdinaryStateContextConfig, CoeffOrdinaryStateContextPassInput,
    apply_nonzero_coeff_ordinary_pass_with_state_context,
};

const TX_SIZES: usize = 25;
const DCT_DCT: usize = 0;
const IDTX: usize = 9;
const TX_4X4: usize = 0;
const V_PRED: usize = 1;
const D67_PRED: usize = 8;
const TX_TYPES: usize = 16;
const ANGLE_STEP: i32 = 3;

const TX_TYPE_IN_SET_INTRA: [[u8; 16]; 7] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0],
    [1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 1, 0],
    [1, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 0, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
];

const TX_TYPE_IN_SET_INTER: [[u8; 16]; 9] = [
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0],
    [1, 0, 1, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0],
    [1, 1, 0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 1, 0],
    [1, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 0, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0],
];

#[derive(Clone, Copy)]
pub(crate) struct CoeffOrdinaryStagedLosslessNonZeroInput {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) start: NonZeroCoeffBlockStart,
    /// The block's `PlaneTxType`, derived by the caller.
    pub(crate) plane_tx_type: usize,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryBranchModeToTxfmBaseConfig {
    pub(crate) tx_set: usize,
    pub(crate) uv_mode: usize,
    pub(crate) angle_delta_uv: i32,
    pub(crate) luma_tx_type: usize,
    pub(crate) chroma_inter_tx_type: usize,
    pub(crate) enable_chroma_dctonly: bool,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryBranchTxSetBaseConfig {
    pub(crate) reduced_tx_set: usize,
    pub(crate) enable_chroma_dctonly: bool,
    pub(crate) uv_mode: usize,
    pub(crate) angle_delta_uv: i32,
    pub(crate) luma_tx_type: usize,
    pub(crate) chroma_inter_tx_type: usize,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
}

pub(crate) fn read_lossless_inter_plane_tx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_size: usize,
) -> Result<usize, BlockSymbolTraceReadError> {
    if tx_size != TX_4X4 {
        return Ok(IDTX);
    }
    let lossless_inter_tx_type = cdfs
        .read_block_symbol_trace(TileCdfSelector::LosslessInterTxType, symbols)?
        .get();
    if lossless_inter_tx_type == 0 {
        Ok(DCT_DCT)
    } else {
        Ok(IDTX)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryTxSizeGeometryConfig {
    pub(crate) plane: usize,
    pub(crate) start_x: usize,
    pub(crate) start_y: usize,
    pub(crate) tx_size: usize,
}

/// The adjusted width, height and width log2, the transform-size context
/// and the 4x4 extent of one `TxSize`.
#[derive(Clone, Copy)]
struct TxGeometry {
    width: u8,
    height: u8,
    width_log2: u8,
    size_ctx: u8,
    w4: u8,
    h4: u8,
}

/// [`TxGeometry`] of every `TxSize`: `Adjusted_Tx_Size` for the
/// coefficient extent and `(Tx_Size_Sqr + Tx_Size_Sqr_Up + 1) >> 1` for the
/// context (§ 5.20.7.27).
static TX_GEOMETRY: [TxGeometry; TX_SIZES] = {
    let mut out = [TxGeometry {
        width: 0,
        height: 0,
        width_log2: 0,
        size_ctx: 0,
        w4: 0,
        h4: 0,
    }; TX_SIZES];
    let mut tx_size = 0;
    while tx_size < TX_SIZES {
        let adjusted = ADJUSTED_TX_SIZE[tx_size] as usize;
        out[tx_size] = TxGeometry {
            width: TX_WIDTH[adjusted] as u8,
            height: TX_HEIGHT[adjusted] as u8,
            width_log2: TX_WIDTH_LOG2[adjusted] as u8,
            size_ctx: ((TX_SIZE_SQR[tx_size] + TX_SIZE_SQR_UP[tx_size] + 1) >> 1) as u8,
            w4: (TX_WIDTH[tx_size] >> 2) as u8,
            h4: (TX_HEIGHT[tx_size] >> 2) as u8,
        };
        tx_size += 1;
    }
    out
};

pub(crate) fn apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStagedLosslessNonZeroInput,
    quant: &mut [i32],
) -> Result<usize, CoeffOrdinaryBranchError> {
    let geometry = input.geometry;
    let Some(tx) = TX_GEOMETRY.get(geometry.tx_size) else {
        return Err(CoeffOrdinaryBranchError::InvalidTransformSize {
            tx_size: geometry.tx_size,
        });
    };
    let lossy = !input.lossless;
    let pass_input = CoeffOrdinaryStateContextPassInput {
        start: input.start,
        base_config: CoeffBaseDerivedLevelPassConfig {
            tx_size_ctx: usize::from(tx.size_ctx),
            tx_width_log2: u32::from(tx.width_log2),
            tx_width: usize::from(tx.width),
            tx_height: usize::from(tx.height),
            plane: geometry.plane,
            tx_class: CoeffTransformClass::from_plane_tx_type(input.plane_tx_type),
            parity_hiding: lossy && input.parity_hiding,
            use_tcq: lossy && input.use_tcq,
        },
        state_context: CoeffOrdinaryStateContextConfig {
            x4: geometry.start_x >> 2,
            y4: geometry.start_y >> 2,
            w4: usize::from(tx.w4),
            h4: usize::from(tx.h4),
        },
        lossless: input.lossless,
    };
    apply_nonzero_coeff_ordinary_pass_with_state_context(state, cdfs, symbols, pass_input, quant)
        .map_err(CoeffOrdinaryBranchError::from)
}

pub(crate) fn lossless_plane_tx_type(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    base_config: CoeffOrdinaryBranchTxSetBaseConfig,
) -> usize {
    if is_inter && geometry.plane > 0 {
        return base_config.chroma_inter_tx_type;
    }
    if base_config.luma_tx_type == IDTX && (geometry.plane == 0 || !is_inter) {
        return IDTX;
    }
    DCT_DCT
}

pub(crate) fn resolve_mode_to_txfm_plane_tx_type(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    config: CoeffOrdinaryBranchModeToTxfmBaseConfig,
) -> Result<usize, CoeffOrdinaryBranchError> {
    if geometry.plane == 0 {
        return luma_tx_type(config.luma_tx_type);
    }
    if config.enable_chroma_dctonly {
        return Ok(DCT_DCT);
    }
    if is_inter {
        return chroma_inter_tx_type(config.tx_set, config.chroma_inter_tx_type);
    }

    let uv_mode = if is_directional_mode(config.uv_mode) {
        directional_uv_mode(geometry, config)?
    } else {
        config.uv_mode
    };
    let Some(&tx_type) = MODE_TO_TXFM.get(uv_mode) else {
        return Err(CoeffOrdinaryBranchError::InvalidUvMode { uv_mode });
    };
    let tx_type = usize::try_from(tx_type).map_err(|_| {
        CoeffOrdinaryBranchError::InvalidModeToTxfmTableValue {
            uv_mode,
            value: tx_type,
        }
    })?;
    let Some(set) = TX_TYPE_IN_SET_INTRA.get(config.tx_set) else {
        return Err(CoeffOrdinaryBranchError::InvalidIntraTransformSet {
            tx_set: config.tx_set,
        });
    };
    let Some(&in_set) = set.get(tx_type) else {
        return Err(CoeffOrdinaryBranchError::InvalidModeToTxfmTableValue {
            uv_mode,
            value: tx_type as i32,
        });
    };
    if in_set != 0 {
        Ok(tx_type)
    } else {
        Ok(DCT_DCT)
    }
}

fn luma_tx_type(tx_type: usize) -> Result<usize, CoeffOrdinaryBranchError> {
    if tx_type < TX_TYPES {
        Ok(tx_type)
    } else {
        Err(CoeffOrdinaryBranchError::InvalidLumaTxType { tx_type })
    }
}

fn chroma_inter_tx_type(tx_set: usize, tx_type: usize) -> Result<usize, CoeffOrdinaryBranchError> {
    if tx_type >= TX_TYPES {
        return Err(CoeffOrdinaryBranchError::InvalidChromaInterTxType { tx_type });
    }
    let Some(set) = TX_TYPE_IN_SET_INTER.get(tx_set) else {
        return Err(CoeffOrdinaryBranchError::InvalidInterTransformSet { tx_set });
    };
    let Some(&in_set) = set.get(tx_type) else {
        return Err(CoeffOrdinaryBranchError::InvalidChromaInterTxType { tx_type });
    };
    if in_set != 0 {
        Ok(tx_type)
    } else {
        Ok(DCT_DCT)
    }
}

const fn is_directional_mode(mode: usize) -> bool {
    mode >= V_PRED && mode <= D67_PRED
}

fn directional_uv_mode(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    config: CoeffOrdinaryBranchModeToTxfmBaseConfig,
) -> Result<usize, CoeffOrdinaryBranchError> {
    let Some(&mode_to_angle) = MODE_TO_ANGLE.get(config.uv_mode) else {
        return Err(CoeffOrdinaryBranchError::InvalidUvMode {
            uv_mode: config.uv_mode,
        });
    };
    let Some(p_angle) = config
        .angle_delta_uv
        .checked_mul(ANGLE_STEP)
        .and_then(|delta| mode_to_angle.checked_add(delta))
    else {
        return Err(CoeffOrdinaryBranchError::DirectionalAngleOverflow {
            uv_mode: config.uv_mode,
            angle_delta_uv: config.angle_delta_uv,
        });
    };
    let tx_width = canonical_tx_size_value(
        "Tx_Width",
        geometry.tx_size,
        TX_WIDTH.get(geometry.tx_size).copied(),
    )?;
    let tx_height = canonical_tx_size_value(
        "Tx_Height",
        geometry.tx_size,
        TX_HEIGHT.get(geometry.tx_size).copied(),
    )?;
    Ok(wide_angle_mapping(
        config.uv_mode,
        tx_width,
        tx_height,
        p_angle,
    ))
}

fn canonical_tx_size_value(
    table_name: &'static str,
    tx_size: usize,
    value: Option<i32>,
) -> Result<usize, CoeffOrdinaryBranchError> {
    let Some(value) = value else {
        return Err(CoeffOrdinaryBranchError::InvalidTransformSize { tx_size });
    };
    usize::try_from(value).map_err(
        |_| CoeffOrdinaryBranchError::InvalidTransformSizeTableValue {
            table: table_name,
            tx_size,
            value,
        },
    )
}
