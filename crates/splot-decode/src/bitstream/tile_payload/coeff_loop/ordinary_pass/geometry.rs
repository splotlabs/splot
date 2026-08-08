// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    ADJUSTED_TX_SIZE, MODE_TO_ANGLE, MODE_TO_TXFM, TX_HEIGHT, TX_SIZE_SQR, TX_SIZE_SQR_UP,
    TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::{ReconError, TransformClass, coefficient_scan_slice};

use super::super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::super::cdf::{TileCdfSelector, TileCdfSubset};
use super::super::super::coeff_state::{TileCoeffContextState, TransformCoeffBlockState};
use super::super::base_level_pass::CoeffBaseDerivedLevelPassConfig;
use super::super::branch::NonZeroCoeffBlockStart;
use super::super::max_level::CoeffTransformClass;
use super::{
    CoeffOrdinaryBranchError, CoeffOrdinaryStateContextConfig, CoeffOrdinaryStateContextPassInput,
    apply_nonzero_coeff_ordinary_pass_with_state_context,
};

const DCT_DCT: usize = 0;
const IDTX: usize = 9;
const TX_4X4: usize = 0;
const V_PRED: usize = 1;
const D45_PRED: usize = 3;
const D203_PRED: usize = 7;
const D67_PRED: usize = 8;
const TX_TYPES: usize = 16;
const TX_16X16: usize = 2;
const TX_32X32: usize = 3;
const ANGLE_STEP: i32 = 3;
const WAIP_WH_RATIO_2_THRES: i32 = 61;
const WAIP_WH_RATIO_4_THRES: i32 = 73;
const WAIP_WH_RATIO_8_THRES: i32 = 82;
const WAIP_WH_RATIO_16_THRES: i32 = 86;
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
const MAX_REDUCED_TX_SET: usize = 3;
const MAX_SCAN_DIMENSION: usize = 32;

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

struct CoeffOrdinaryStagedTxSizeDimensionsInput {
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    start: NonZeroCoeffBlockStart,
    coeff_cdf_q_ctx: usize,
    base_config: CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
    lossless: bool,
}

pub(crate) struct CoeffOrdinaryStagedLosslessNonZeroInput {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) is_inter: bool,
    pub(crate) base_config: CoeffOrdinaryBranchTxSetBaseConfig,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
    pub(crate) plane_tx_type: usize,
    pub(crate) parity_hiding: bool,
    pub(crate) use_tcq: bool,
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

impl CoeffOrdinaryBranchModeToTxfmBaseConfig {
    fn tx_size_base_config(
        self,
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        is_inter: bool,
        lossless: bool,
    ) -> Result<CoeffOrdinaryBranchTxSizeDimensionsBaseConfig, CoeffOrdinaryBranchError> {
        Ok(CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
            plane_tx_type: mode_to_txfm_plane_tx_type(geometry, is_inter, lossless, self)?,
            parity_hiding: self.parity_hiding,
            use_tcq: self.use_tcq,
        })
    }
}

impl CoeffOrdinaryBranchTxSetBaseConfig {
    fn mode_to_txfm_base_config(
        self,
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        is_inter: bool,
    ) -> Result<CoeffOrdinaryBranchModeToTxfmBaseConfig, CoeffOrdinaryBranchError> {
        Ok(CoeffOrdinaryBranchModeToTxfmBaseConfig {
            tx_set: tx_set(geometry, is_inter, self)?,
            uv_mode: self.uv_mode,
            angle_delta_uv: self.angle_delta_uv,
            luma_tx_type: self.luma_tx_type,
            chroma_inter_tx_type: self.chroma_inter_tx_type,
            enable_chroma_dctonly: self.enable_chroma_dctonly,
            parity_hiding: self.parity_hiding,
            use_tcq: self.use_tcq,
        })
    }
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

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoeffOrdinaryTxSizeDimensions {
    tx_width: usize,
    tx_height: usize,
    tx_width_log2: u32,
}

struct CoeffOrdinaryDerivedTxSize {
    raw_dimensions: CoeffOrdinaryTxSizeDimensions,
    adjusted_dimensions: CoeffOrdinaryTxSizeDimensions,
    tx_size_ctx: usize,
    tx_class: CoeffTransformClass,
    scan: &'static [u16],
}

impl CoeffOrdinaryDerivedTxSize {
    fn derive(
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        plane_tx_type: usize,
    ) -> Result<Self, CoeffOrdinaryBranchError> {
        let raw_dimensions = tx_size_dimensions(geometry.tx_size)?;
        let adjusted_dimensions = adjusted_tx_size_dimensions(geometry.tx_size)?;
        let tx_size_ctx = tx_size_context(geometry.tx_size)?;
        let tx_class = CoeffTransformClass::from_plane_tx_type(plane_tx_type);
        let scan = tx_size_scan(raw_dimensions, tx_class)?;
        Ok(Self {
            raw_dimensions,
            adjusted_dimensions,
            tx_size_ctx,
            tx_class,
            scan,
        })
    }

    fn state_context_input(
        &self,
        input: CoeffOrdinaryStagedTxSizeDimensionsInput,
    ) -> CoeffOrdinaryStateContextPassInput<'_> {
        CoeffOrdinaryStateContextPassInput {
            start: input.start,
            scan: self.scan,
            base_config: CoeffBaseDerivedLevelPassConfig {
                coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                tx_size_ctx: self.tx_size_ctx,
                tx_width_log2: self.adjusted_dimensions.tx_width_log2,
                tx_width: self.adjusted_dimensions.tx_width,
                tx_height: self.adjusted_dimensions.tx_height,
                plane: input.geometry.plane,
                tx_class: self.tx_class,
                parity_hiding: input.base_config.parity_hiding,
                use_tcq: input.base_config.use_tcq,
            },
            state_context: CoeffOrdinaryStateContextConfig {
                coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                plane_type: usize::from(input.geometry.plane > 0),
                x4: input.geometry.start_x >> 2,
                y4: input.geometry.start_y >> 2,
                w4: self.raw_dimensions.tx_width >> 2,
                h4: self.raw_dimensions.tx_height >> 2,
            },
            lossless: input.lossless,
        }
    }
}

pub(crate) fn apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStagedLosslessNonZeroInput,
) -> Result<TransformCoeffBlockState, CoeffOrdinaryBranchError> {
    let CoeffOrdinaryStagedLosslessNonZeroInput {
        geometry,
        start,
        coeff_cdf_q_ctx,
        is_inter,
        base_config,
        lossless,
    } = input;
    let base_config = if lossless {
        staged_lossless_tx_size_base_config(geometry, is_inter, base_config)
    } else {
        let mode_to_txfm_base_config = base_config.mode_to_txfm_base_config(geometry, is_inter)?;
        mode_to_txfm_base_config.tx_size_base_config(geometry, is_inter, lossless)?
    };
    let input = CoeffOrdinaryStagedTxSizeDimensionsInput {
        geometry,
        start,
        coeff_cdf_q_ctx,
        base_config,
        lossless,
    };
    let derived = CoeffOrdinaryDerivedTxSize::derive(geometry, base_config.plane_tx_type)?;
    apply_nonzero_coeff_ordinary_pass_with_state_context(
        state,
        cdfs,
        symbols,
        derived.state_context_input(input),
    )
    .map_err(CoeffOrdinaryBranchError::from)
}

fn staged_lossless_tx_size_base_config(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    base_config: CoeffOrdinaryBranchTxSetBaseConfig,
) -> CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
    CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
        plane_tx_type: lossless_plane_tx_type(geometry, is_inter, base_config),
        parity_hiding: false,
        use_tcq: false,
    }
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

fn tx_set(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    config: CoeffOrdinaryBranchTxSetBaseConfig,
) -> Result<usize, CoeffOrdinaryBranchError> {
    if config.reduced_tx_set > MAX_REDUCED_TX_SET {
        return Err(CoeffOrdinaryBranchError::InvalidReducedTxSet {
            reduced_tx_set: config.reduced_tx_set,
        });
    }

    let tx_size_sqr = canonical_mapped_tx_size(
        "Tx_Size_Sqr",
        geometry.tx_size,
        TX_SIZE_SQR.get(geometry.tx_size).copied(),
    )?;
    let tx_size_sqr_up = canonical_mapped_tx_size(
        "Tx_Size_Sqr_Up",
        geometry.tx_size,
        TX_SIZE_SQR_UP.get(geometry.tx_size).copied(),
    )?;
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

    if tx_size_sqr_up > TX_32X32 {
        if tx_size_sqr >= TX_32X32 {
            return Ok(TX_SET_DCTONLY);
        }
        return if tx_width > tx_height {
            Ok(TX_SET_WIDE_64)
        } else {
            Ok(TX_SET_HIGH_64)
        };
    }
    if tx_size_sqr_up == TX_32X32 && tx_size_sqr != TX_32X32 {
        return if tx_width > tx_height {
            Ok(TX_SET_WIDE_32)
        } else {
            Ok(TX_SET_HIGH_32)
        };
    }
    if !is_inter && tx_size_sqr_up == TX_32X32 {
        return Ok(TX_SET_DCTONLY);
    }

    let reduced_tx_set = if geometry.plane == 0 {
        config.reduced_tx_set
    } else {
        usize::from(config.enable_chroma_dctonly)
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
        return if tx_size_sqr == TX_16X16 {
            Ok(TX_SET_INTER_2)
        } else {
            Ok(TX_SET_INTER_1)
        };
    }
    Ok(TX_SET_INTRA_1)
}

pub(crate) fn resolve_mode_to_txfm_plane_tx_type(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    lossless: bool,
    config: CoeffOrdinaryBranchModeToTxfmBaseConfig,
) -> Result<usize, CoeffOrdinaryBranchError> {
    mode_to_txfm_plane_tx_type(geometry, is_inter, lossless, config)
}

fn mode_to_txfm_plane_tx_type(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    lossless: bool,
    config: CoeffOrdinaryBranchModeToTxfmBaseConfig,
) -> Result<usize, CoeffOrdinaryBranchError> {
    if lossless {
        return Err(CoeffOrdinaryBranchError::UnsupportedModeToTxfmSubset { reason: "lossless" });
    }
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
    let tx_type = MODE_TO_TXFM
        .get(uv_mode)
        .copied()
        .ok_or(CoeffOrdinaryBranchError::InvalidUvMode { uv_mode })?;
    let tx_type = usize::try_from(tx_type).map_err(|_| {
        CoeffOrdinaryBranchError::InvalidModeToTxfmTableValue {
            uv_mode,
            value: tx_type,
        }
    })?;
    let set = TX_TYPE_IN_SET_INTRA.get(config.tx_set).ok_or(
        CoeffOrdinaryBranchError::InvalidIntraTransformSet {
            tx_set: config.tx_set,
        },
    )?;
    if set
        .get(tx_type)
        .copied()
        .ok_or(CoeffOrdinaryBranchError::InvalidModeToTxfmTableValue {
            uv_mode,
            value: tx_type as i32,
        })?
        != 0
    {
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
    let set = TX_TYPE_IN_SET_INTER
        .get(tx_set)
        .ok_or(CoeffOrdinaryBranchError::InvalidInterTransformSet { tx_set })?;
    if set
        .get(tx_type)
        .copied()
        .ok_or(CoeffOrdinaryBranchError::InvalidChromaInterTxType { tx_type })?
        != 0
    {
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
    let mode_to_angle = MODE_TO_ANGLE.get(config.uv_mode).copied().ok_or(
        CoeffOrdinaryBranchError::InvalidUvMode {
            uv_mode: config.uv_mode,
        },
    )?;
    let delta = config.angle_delta_uv.checked_mul(ANGLE_STEP).ok_or(
        CoeffOrdinaryBranchError::DirectionalAngleOverflow {
            uv_mode: config.uv_mode,
            angle_delta_uv: config.angle_delta_uv,
        },
    )?;
    let p_angle = mode_to_angle.checked_add(delta).ok_or(
        CoeffOrdinaryBranchError::DirectionalAngleOverflow {
            uv_mode: config.uv_mode,
            angle_delta_uv: config.angle_delta_uv,
        },
    )?;
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

fn wide_angle_mapping(mode: usize, width: usize, height: usize, p_angle: i32) -> usize {
    if is_scaled(height, width, 2) && p_angle < WAIP_WH_RATIO_2_THRES
        || is_scaled(height, width, 4) && p_angle < WAIP_WH_RATIO_4_THRES
        || is_scaled(height, width, 8) && p_angle < WAIP_WH_RATIO_8_THRES
        || is_scaled(height, width, 16) && p_angle < WAIP_WH_RATIO_16_THRES
    {
        D203_PRED
    } else if is_scaled(width, height, 2) && p_angle > 270 - WAIP_WH_RATIO_2_THRES
        || is_scaled(width, height, 4) && p_angle > 270 - WAIP_WH_RATIO_4_THRES
        || is_scaled(width, height, 8) && p_angle > 270 - WAIP_WH_RATIO_8_THRES
        || is_scaled(width, height, 16) && p_angle > 270 - WAIP_WH_RATIO_16_THRES
    {
        D45_PRED
    } else {
        mode
    }
}

const fn is_scaled(value: usize, base: usize, factor: usize) -> bool {
    matches!(base.checked_mul(factor), Some(scaled) if scaled == value)
}

fn adjusted_tx_size_dimensions(
    tx_size: usize,
) -> Result<CoeffOrdinaryTxSizeDimensions, CoeffOrdinaryBranchError> {
    let adjusted_tx_size = canonical_mapped_tx_size(
        "Adjusted_Tx_Size",
        tx_size,
        ADJUSTED_TX_SIZE.get(tx_size).copied(),
    )?;
    tx_size_dimensions(adjusted_tx_size)
}

fn tx_size_context(tx_size: usize) -> Result<usize, CoeffOrdinaryBranchError> {
    let tx_size_sqr =
        canonical_mapped_tx_size("Tx_Size_Sqr", tx_size, TX_SIZE_SQR.get(tx_size).copied())?;
    let tx_size_sqr_up = canonical_mapped_tx_size(
        "Tx_Size_Sqr_Up",
        tx_size,
        TX_SIZE_SQR_UP.get(tx_size).copied(),
    )?;
    Ok((tx_size_sqr + tx_size_sqr_up + 1) >> 1)
}

fn tx_size_dimensions(
    tx_size: usize,
) -> Result<CoeffOrdinaryTxSizeDimensions, CoeffOrdinaryBranchError> {
    Ok(CoeffOrdinaryTxSizeDimensions {
        tx_width: canonical_tx_size_value("Tx_Width", tx_size, TX_WIDTH.get(tx_size).copied())?,
        tx_height: canonical_tx_size_value("Tx_Height", tx_size, TX_HEIGHT.get(tx_size).copied())?,
        tx_width_log2: canonical_tx_size_value(
            "Tx_Width_Log2",
            tx_size,
            TX_WIDTH_LOG2.get(tx_size).copied(),
        )? as u32,
    })
}

fn canonical_mapped_tx_size(
    table_name: &'static str,
    tx_size: usize,
    value: Option<i32>,
) -> Result<usize, CoeffOrdinaryBranchError> {
    let value = canonical_tx_size_value(table_name, tx_size, value)?;
    if TX_WIDTH.get(value).is_none() {
        return Err(CoeffOrdinaryBranchError::InvalidTransformSize { tx_size: value });
    }
    Ok(value)
}

fn tx_size_scan(
    dimensions: CoeffOrdinaryTxSizeDimensions,
    tx_class: CoeffTransformClass,
) -> Result<&'static [u16], CoeffOrdinaryBranchError> {
    let width = dimensions.tx_width.min(MAX_SCAN_DIMENSION);
    let height = dimensions.tx_height.min(MAX_SCAN_DIMENSION);
    let class = match tx_class {
        CoeffTransformClass::TwoD => TransformClass::TwoD,
        CoeffTransformClass::Horizontal => TransformClass::Horizontal,
        CoeffTransformClass::Vertical => TransformClass::Vertical,
    };
    coefficient_scan_slice(width, height, class).map_err(|error| match error {
        ReconError::InvalidScanShape { w, h } => CoeffOrdinaryBranchError::InvalidScanShape {
            width: w,
            height: h,
        },
        source => CoeffOrdinaryBranchError::ScanOrder(source),
    })
}

fn canonical_tx_size_value(
    table_name: &'static str,
    tx_size: usize,
    value: Option<i32>,
) -> Result<usize, CoeffOrdinaryBranchError> {
    let value = value.ok_or(CoeffOrdinaryBranchError::InvalidTransformSize { tx_size })?;
    usize::try_from(value).map_err(
        |_| CoeffOrdinaryBranchError::InvalidTransformSizeTableValue {
            table: table_name,
            tx_size,
            value,
        },
    )
}
