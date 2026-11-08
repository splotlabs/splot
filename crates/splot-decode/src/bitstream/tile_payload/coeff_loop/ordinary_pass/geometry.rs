// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{MODE_TO_ANGLE, MODE_TO_TXFM, TX_HEIGHT, TX_WIDTH};

use crate::bitstream::tile_payload::general_intra_residual::wide_angle_mapping;

use super::super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::super::cdf::{TileCdfSelector, TileCdfSubset};
use super::super::super::coeff_state::{CoeffBlockLines, TileCoeffContextState};
use super::super::base_level_pass::CoeffBaseDerivedLevelPassConfig;
use super::super::branch::NonZeroCoeffBlockStart;
use super::super::max_level::CoeffTransformClass;
use super::super::transform_block::{CoeffTransformBlockInput, decode_ordinary_transform_block};
use super::CoeffOrdinaryBranchError;

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

pub(crate) struct CoeffOrdinaryStagedLosslessNonZeroInput<'l> {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) start: NonZeroCoeffBlockStart,
    /// The block's context-line spans, checked by the caller.
    pub(crate) lines: &'l CoeffBlockLines,
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

#[allow(clippy::inline_always, reason = "measured: once per nonzero block")]
#[inline(always)]
pub(crate) fn apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStagedLosslessNonZeroInput<'_>,
    quant: &mut [i32],
) -> Result<usize, CoeffOrdinaryBranchError> {
    let tx = input.start.tx();
    let lossy = !input.lossless;
    let block = CoeffTransformBlockInput {
        start: input.start,
        config: CoeffBaseDerivedLevelPassConfig {
            tx_size_ctx: usize::from(tx.size_ctx),
            tx_width_log2: u32::from(tx.width_log2),
            tx_width: usize::from(tx.width),
            tx_height: usize::from(tx.height),
            plane: input.geometry.plane,
            tx_class: CoeffTransformClass::from_plane_tx_type(input.plane_tx_type),
            parity_hiding: lossy && input.parity_hiding,
            use_tcq: lossy && input.use_tcq,
        },
        lines: input.lines,
        lossless: input.lossless,
    };
    decode_ordinary_transform_block(state, cdfs, symbols, block, quant)
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
