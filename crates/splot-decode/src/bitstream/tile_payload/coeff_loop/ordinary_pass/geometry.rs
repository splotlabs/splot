// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{MODE_TO_ANGLE, MODE_TO_TXFM};

use super::super::super::cdf::TileCdfSubset;
use super::super::super::coeff_state::TileCoeffContextState;
use super::super::base_level_pass::CoeffBaseDerivedLevelPassConfig;
use super::super::branch::{NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::super::max_level::CoeffTransformClass;
use super::super::scan_walk::{CoeffScanOrderError, derive_coeff_scan_order};
use super::super::{
    AllZeroCoeffBlockInput, CoeffBranchInput, CoeffTxSizeTables as CoeffOrdinaryTxSizeTables,
    DEFAULT_TX_SIZE_TABLES, NonZeroCoeffEobContextInput,
};
use super::{
    CoeffOrdinaryBranch, CoeffOrdinaryBranchError, CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    CoeffOrdinaryBranchPlaneTypeNonZeroInput, CoeffOrdinaryPlaneTypeStateContextConfig,
    CoeffOrdinaryStateContextConfig, CoeffOrdinaryStateContextPassInput,
    NonZeroCoeffOrdinaryDerivedBasePass, apply_coeff_ordinary_branch_from_plane_type,
    apply_nonzero_coeff_ordinary_pass_with_state_context,
};

const DCT_DCT: usize = 0;
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

macro_rules! coeff_default_tx_tables_adapter {
    (
        $vis:vis fn $name:ident($input_ty:ty) -> $ok_ty:ty,
        $callee:path $(,)?
    ) => {
        $vis fn $name(
            state: &mut TileCoeffContextState,
            cdfs: &mut TileCdfSubset,
            symbols: &mut SymbolDecoder<'_>,
            input: $input_ty,
        ) -> Result<$ok_ty, CoeffOrdinaryBranchError> {
            $callee(state, cdfs, symbols, input, DEFAULT_TX_SIZE_TABLES)
        }
    };
}

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

pub(crate) type CoeffOrdinaryBranchTxSizeDimensionsInput = CoeffBranchInput<
    CoeffOrdinaryTxSizeGeometryConfig,
    CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput,
>;

pub(crate) struct CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) is_inter: bool,
    pub(crate) base_config: CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
    pub(crate) lossless: bool,
}

struct CoeffOrdinaryStagedTxSizeDimensionsInput {
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    start: NonZeroCoeffBlockStart,
    coeff_cdf_q_ctx: usize,
    base_config: CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
    lossless: bool,
}

pub(crate) type CoeffOrdinaryBranchModeToTxfmInput =
    CoeffBranchInput<CoeffOrdinaryTxSizeGeometryConfig, CoeffOrdinaryBranchModeToTxfmNonZeroInput>;

pub(crate) struct CoeffOrdinaryBranchModeToTxfmNonZeroInput {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) is_inter: bool,
    pub(crate) base_config: CoeffOrdinaryBranchModeToTxfmBaseConfig,
    pub(crate) lossless: bool,
}

pub(crate) type CoeffOrdinaryBranchTxSetInput =
    CoeffBranchInput<CoeffOrdinaryTxSizeGeometryConfig, CoeffOrdinaryBranchTxSetNonZeroInput>;

pub(crate) struct CoeffOrdinaryBranchTxSetNonZeroInput {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) is_inter: bool,
    pub(crate) base_config: CoeffOrdinaryBranchTxSetBaseConfig,
    pub(crate) lossless: bool,
}

pub(crate) type CoeffOrdinaryBranchLosslessInput =
    CoeffBranchInput<CoeffOrdinaryTxSizeGeometryConfig, CoeffOrdinaryBranchLosslessNonZeroInput>;

pub(crate) struct CoeffOrdinaryBranchLosslessNonZeroInput {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) is_inter: bool,
    pub(crate) base_config: CoeffOrdinaryBranchLosslessBaseConfig,
    pub(crate) lossless: bool,
}

pub(crate) struct CoeffOrdinaryStagedLosslessNonZeroInput {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) is_inter: bool,
    pub(crate) base_config: CoeffOrdinaryBranchLosslessBaseConfig,
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

pub(crate) type CoeffOrdinaryBranchLosslessBaseConfig = CoeffOrdinaryBranchTxSetBaseConfig;

impl CoeffOrdinaryBranchModeToTxfmBaseConfig {
    fn tx_size_base_config(
        self,
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        is_inter: bool,
        lossless: bool,
        tables: CoeffOrdinaryTxSizeTables<'_>,
    ) -> Result<CoeffOrdinaryBranchTxSizeDimensionsBaseConfig, CoeffOrdinaryBranchError> {
        Ok(CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
            plane_tx_type: mode_to_txfm_plane_tx_type(geometry, is_inter, lossless, self, tables)?,
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
        tables: CoeffOrdinaryTxSizeTables<'_>,
    ) -> Result<CoeffOrdinaryBranchModeToTxfmBaseConfig, CoeffOrdinaryBranchError> {
        Ok(CoeffOrdinaryBranchModeToTxfmBaseConfig {
            tx_set: tx_set(geometry, is_inter, self, tables)?,
            uv_mode: self.uv_mode,
            angle_delta_uv: self.angle_delta_uv,
            luma_tx_type: self.luma_tx_type,
            chroma_inter_tx_type: self.chroma_inter_tx_type,
            enable_chroma_dctonly: self.enable_chroma_dctonly,
            parity_hiding: self.parity_hiding,
            use_tcq: self.use_tcq,
        })
    }

    #[allow(clippy::unused_self)]
    const fn lossless_tx_size_base_config(self) -> CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
        CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
            plane_tx_type: DCT_DCT,
            parity_hiding: false,
            use_tcq: false,
        }
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
    tx_height_log2: u32,
}

struct CoeffOrdinaryDerivedTxSize {
    raw_dimensions: CoeffOrdinaryTxSizeDimensions,
    adjusted_dimensions: CoeffOrdinaryTxSizeDimensions,
    tx_size_ctx: usize,
    tx_class: CoeffTransformClass,
    scan: Vec<u16>,
}

impl CoeffOrdinaryDerivedTxSize {
    fn derive(
        tables: CoeffOrdinaryTxSizeTables<'_>,
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        plane_tx_type: usize,
    ) -> Result<Self, CoeffOrdinaryBranchError> {
        let raw_dimensions = tx_size_dimensions(tables, geometry.tx_size)?;
        let adjusted_dimensions = adjusted_tx_size_dimensions(tables, geometry.tx_size)?;
        let tx_size_ctx = tx_size_context(tables, geometry.tx_size)?;
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

    const fn eob_context(
        &self,
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        is_inter: bool,
        coeff_cdf_q_ctx: usize,
    ) -> NonZeroCoeffEobContextInput {
        NonZeroCoeffEobContextInput {
            plane: geometry.plane,
            is_inter,
            tx_width_log2: self.raw_dimensions.tx_width_log2 as usize,
            tx_height_log2: self.raw_dimensions.tx_height_log2 as usize,
            coeff_cdf_q_ctx,
        }
    }

    fn state_context_input(
        &self,
        input: CoeffOrdinaryStagedTxSizeDimensionsInput,
    ) -> CoeffOrdinaryStateContextPassInput<'_> {
        let block = input
            .geometry
            .coeffs_geometry(self.raw_dimensions)
            .block_input();
        CoeffOrdinaryStateContextPassInput {
            start: input.start,
            scan: &self.scan,
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
                x4: block.x4,
                y4: block.y4,
                w4: block.w4,
                h4: block.h4,
            },
            lossless: input.lossless,
        }
    }
}

impl CoeffOrdinaryTxSizeGeometryConfig {
    fn coeffs_geometry(
        self,
        dimensions: CoeffOrdinaryTxSizeDimensions,
    ) -> CoeffOrdinaryCoeffsGeometryConfig {
        CoeffOrdinaryCoeffsGeometryConfig {
            plane: self.plane,
            start_x: self.start_x,
            start_y: self.start_y,
            tx_width: dimensions.tx_width,
            tx_height: dimensions.tx_height,
        }
    }
}

impl CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
    const fn base_config(
        self,
        geometry: CoeffOrdinaryTxSizeGeometryConfig,
        tx_size_ctx: usize,
        adjusted_dimensions: CoeffOrdinaryTxSizeDimensions,
        coeff_cdf_q_ctx: usize,
    ) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
        CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
            coeff_cdf_q_ctx,
            tx_size_ctx,
            tx_width_log2: adjusted_dimensions.tx_width_log2,
            tx_width: adjusted_dimensions.tx_width,
            tx_height: adjusted_dimensions.tx_height,
            plane: geometry.plane,
            plane_tx_type: self.plane_tx_type,
            parity_hiding: self.parity_hiding,
            use_tcq: self.use_tcq,
        }
    }
}

pub(crate) type CoeffOrdinaryBranchCoeffsGeometryInput<'a> = CoeffBranchInput<
    CoeffOrdinaryCoeffsGeometryConfig,
    CoeffOrdinaryBranchCoeffsGeometryNonZeroInput<'a>,
>;

pub(crate) struct CoeffOrdinaryBranchCoeffsGeometryNonZeroInput<'a> {
    pub(crate) geometry: CoeffOrdinaryCoeffsGeometryConfig,
    pub(crate) eob: NonZeroCoeffEobContextInput,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    pub(crate) state_context: CoeffOrdinaryGeometryStateContextConfig,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryCoeffsGeometryConfig {
    pub(crate) plane: usize,
    pub(crate) start_x: usize,
    pub(crate) start_y: usize,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
}

impl CoeffOrdinaryCoeffsGeometryConfig {
    const fn block_input(self) -> AllZeroCoeffBlockInput {
        AllZeroCoeffBlockInput {
            plane: self.plane,
            x4: self.start_x >> 2,
            y4: self.start_y >> 2,
            w4: self.tx_width >> 2,
            h4: self.tx_height >> 2,
        }
    }
}

pub(crate) type CoeffOrdinaryBranchGeometryInput<'a> =
    CoeffBranchInput<AllZeroCoeffBlockInput, CoeffOrdinaryBranchGeometryNonZeroInput<'a>>;

pub(crate) struct CoeffOrdinaryBranchGeometryNonZeroInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStartInput,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    pub(crate) state_context: CoeffOrdinaryGeometryStateContextConfig,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryGeometryStateContextConfig {
    pub(crate) coeff_cdf_q_ctx: usize,
}

pub(crate) fn apply_coeff_ordinary_branch_from_geometry(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchGeometryInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = input.map_nonzero(|input| {
        let block = input.start.block;
        CoeffOrdinaryBranchPlaneTypeNonZeroInput {
            start: input.start,
            scan: input.scan,
            base_config: input.base_config,
            state_context: CoeffOrdinaryPlaneTypeStateContextConfig {
                coeff_cdf_q_ctx: input.state_context.coeff_cdf_q_ctx,
                x4: block.x4,
                y4: block.y4,
                w4: block.w4,
                h4: block.h4,
            },
            lossless: input.lossless,
        }
    });
    apply_coeff_ordinary_branch_from_plane_type(state, cdfs, symbols, input)
}

pub(crate) fn apply_coeff_ordinary_branch_from_coeffs_geometry(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchCoeffsGeometryInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = match input {
        CoeffOrdinaryBranchCoeffsGeometryInput::AllZero(geometry) => {
            CoeffOrdinaryBranchGeometryInput::AllZero(geometry.block_input())
        }
        CoeffOrdinaryBranchCoeffsGeometryInput::NonZero(input) => {
            CoeffOrdinaryBranchGeometryInput::NonZero(CoeffOrdinaryBranchGeometryNonZeroInput {
                start: NonZeroCoeffBlockStartInput {
                    block: input.geometry.block_input(),
                    eob: input.eob,
                },
                scan: input.scan,
                base_config: input.base_config,
                state_context: input.state_context,
                lossless: input.lossless,
            })
        }
    };
    apply_coeff_ordinary_branch_from_geometry(state, cdfs, symbols, input)
}

coeff_default_tx_tables_adapter!(
    pub(crate) fn apply_coeff_ordinary_branch_from_tx_size_dimensions(
        CoeffOrdinaryBranchTxSizeDimensionsInput
    ) -> CoeffOrdinaryBranch,
    apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables,
);

coeff_default_tx_tables_adapter!(
    pub(crate) fn apply_coeff_ordinary_branch_from_mode_to_txfm(
        CoeffOrdinaryBranchModeToTxfmInput
    ) -> CoeffOrdinaryBranch,
    apply_coeff_ordinary_branch_from_mode_to_txfm_with_tables,
);

coeff_default_tx_tables_adapter!(
    pub(crate) fn apply_coeff_ordinary_branch_from_tx_set(CoeffOrdinaryBranchTxSetInput)
        -> CoeffOrdinaryBranch,
    apply_coeff_ordinary_branch_from_tx_set_with_tables,
);

coeff_default_tx_tables_adapter!(
    pub(crate) fn apply_coeff_ordinary_branch_from_lossless(CoeffOrdinaryBranchLosslessInput)
        -> CoeffOrdinaryBranch,
    apply_coeff_ordinary_branch_from_lossless_with_tables,
);

coeff_default_tx_tables_adapter!(
    pub(crate) fn apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        CoeffOrdinaryStagedLosslessNonZeroInput
    ) -> NonZeroCoeffOrdinaryDerivedBasePass,
    apply_staged_nonzero_coeff_ordinary_branch_from_lossless_with_tables,
);

fn apply_coeff_ordinary_branch_from_tx_set_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchTxSetInput,
    tables: CoeffOrdinaryTxSizeTables<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = input.try_map_nonzero(|input| {
        let base_config =
            input
                .base_config
                .mode_to_txfm_base_config(input.geometry, input.is_inter, tables)?;
        Ok::<_, CoeffOrdinaryBranchError>(CoeffOrdinaryBranchModeToTxfmNonZeroInput {
            geometry: input.geometry,
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
            is_inter: input.is_inter,
            base_config,
            lossless: input.lossless,
        })
    })?;
    apply_coeff_ordinary_branch_from_mode_to_txfm_with_tables(state, cdfs, symbols, input, tables)
}

fn apply_coeff_ordinary_branch_from_lossless_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchLosslessInput,
    tables: CoeffOrdinaryTxSizeTables<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    match input {
        CoeffOrdinaryBranchLosslessInput::AllZero(geometry) => {
            apply_coeff_ordinary_branch_from_tx_set_with_tables(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchTxSetInput::AllZero(geometry),
                tables,
            )
        }
        CoeffOrdinaryBranchLosslessInput::NonZero(input) if input.lossless => {
            if input.is_inter {
                return Err(CoeffOrdinaryBranchError::UnsupportedLosslessSubset {
                    reason: "inter",
                });
            }
            apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchTxSizeDimensionsInput::NonZero(
                    CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
                        geometry: input.geometry,
                        coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                        is_inter: input.is_inter,
                        base_config: input.base_config.lossless_tx_size_base_config(),
                        lossless: input.lossless,
                    },
                ),
                tables,
            )
        }
        CoeffOrdinaryBranchLosslessInput::NonZero(input) => {
            apply_coeff_ordinary_branch_from_tx_set_with_tables(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchTxSetInput::NonZero(CoeffOrdinaryBranchTxSetNonZeroInput {
                    geometry: input.geometry,
                    coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                    is_inter: input.is_inter,
                    base_config: input.base_config,
                    lossless: input.lossless,
                }),
                tables,
            )
        }
    }
}

fn apply_staged_nonzero_coeff_ordinary_branch_from_lossless_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStagedLosslessNonZeroInput,
    tables: CoeffOrdinaryTxSizeTables<'_>,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryBranchError> {
    let CoeffOrdinaryStagedLosslessNonZeroInput {
        geometry,
        start,
        coeff_cdf_q_ctx,
        is_inter,
        base_config,
        lossless,
    } = input;
    let base_config = if lossless {
        if is_inter {
            return Err(CoeffOrdinaryBranchError::UnsupportedLosslessSubset { reason: "inter" });
        }
        base_config.lossless_tx_size_base_config()
    } else {
        let mode_to_txfm_base_config =
            base_config.mode_to_txfm_base_config(geometry, is_inter, tables)?;
        mode_to_txfm_base_config.tx_size_base_config(geometry, is_inter, lossless, tables)?
    };
    apply_staged_nonzero_coeff_ordinary_pass_from_tx_size_dimensions_with_tables(
        state,
        cdfs,
        symbols,
        CoeffOrdinaryStagedTxSizeDimensionsInput {
            geometry,
            start,
            coeff_cdf_q_ctx,
            base_config,
            lossless,
        },
        tables,
    )
}

fn apply_staged_nonzero_coeff_ordinary_pass_from_tx_size_dimensions_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStagedTxSizeDimensionsInput,
    tables: CoeffOrdinaryTxSizeTables<'_>,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryBranchError> {
    let derived = CoeffOrdinaryDerivedTxSize::derive(
        tables,
        input.geometry,
        input.base_config.plane_tx_type,
    )?;
    apply_nonzero_coeff_ordinary_pass_with_state_context(
        state,
        cdfs,
        symbols,
        derived.state_context_input(input),
    )
    .map_err(CoeffOrdinaryBranchError::from)
}

pub(crate) fn apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchTxSizeDimensionsInput,
    tables: CoeffOrdinaryTxSizeTables<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    match input {
        CoeffOrdinaryBranchTxSizeDimensionsInput::AllZero(geometry) => {
            let raw_dimensions = tx_size_dimensions(tables, geometry.tx_size)?;
            apply_coeff_ordinary_branch_from_coeffs_geometry(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchCoeffsGeometryInput::AllZero(
                    geometry.coeffs_geometry(raw_dimensions),
                ),
            )
        }
        CoeffOrdinaryBranchTxSizeDimensionsInput::NonZero(input) => {
            let derived = CoeffOrdinaryDerivedTxSize::derive(
                tables,
                input.geometry,
                input.base_config.plane_tx_type,
            )?;
            apply_coeff_ordinary_branch_from_coeffs_geometry(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchCoeffsGeometryInput::NonZero(
                    CoeffOrdinaryBranchCoeffsGeometryNonZeroInput {
                        geometry: input.geometry.coeffs_geometry(derived.raw_dimensions),
                        eob: derived.eob_context(
                            input.geometry,
                            input.is_inter,
                            input.coeff_cdf_q_ctx,
                        ),
                        scan: &derived.scan,
                        base_config: input.base_config.base_config(
                            input.geometry,
                            derived.tx_size_ctx,
                            derived.adjusted_dimensions,
                            input.coeff_cdf_q_ctx,
                        ),
                        state_context: CoeffOrdinaryGeometryStateContextConfig {
                            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                        },
                        lossless: input.lossless,
                    },
                ),
            )
        }
    }
}

fn apply_coeff_ordinary_branch_from_mode_to_txfm_with_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchModeToTxfmInput,
    tables: CoeffOrdinaryTxSizeTables<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = input.try_map_nonzero(|input| {
        let base_config = input.base_config.tx_size_base_config(
            input.geometry,
            input.is_inter,
            input.lossless,
            tables,
        )?;
        Ok::<_, CoeffOrdinaryBranchError>(CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
            geometry: input.geometry,
            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
            is_inter: input.is_inter,
            base_config,
            lossless: input.lossless,
        })
    })?;
    apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables(
        state, cdfs, symbols, input, tables,
    )
}

fn tx_set(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    config: CoeffOrdinaryBranchTxSetBaseConfig,
    tables: CoeffOrdinaryTxSizeTables<'_>,
) -> Result<usize, CoeffOrdinaryBranchError> {
    if config.reduced_tx_set > MAX_REDUCED_TX_SET {
        return Err(CoeffOrdinaryBranchError::InvalidReducedTxSet {
            reduced_tx_set: config.reduced_tx_set,
        });
    }

    let tx_size_sqr =
        tx_size_table_tx_size(tables, tables.tx_size_sqr, "Tx_Size_Sqr", geometry.tx_size)?;
    let tx_size_sqr_up = tx_size_table_tx_size(
        tables,
        tables.tx_size_sqr_up,
        "Tx_Size_Sqr_Up",
        geometry.tx_size,
    )?;
    let tx_width = tx_size_table_usize(tables.tx_width, "Tx_Width", geometry.tx_size)?;
    let tx_height = tx_size_table_usize(tables.tx_height, "Tx_Height", geometry.tx_size)?;

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
    mode_to_txfm_plane_tx_type(geometry, is_inter, lossless, config, DEFAULT_TX_SIZE_TABLES)
}

fn mode_to_txfm_plane_tx_type(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    is_inter: bool,
    lossless: bool,
    config: CoeffOrdinaryBranchModeToTxfmBaseConfig,
    tables: CoeffOrdinaryTxSizeTables<'_>,
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
        directional_uv_mode(geometry, config, tables)?
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
    tables: CoeffOrdinaryTxSizeTables<'_>,
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
    let tx_width = tx_size_table_usize(tables.tx_width, "Tx_Width", geometry.tx_size)?;
    let tx_height = tx_size_table_usize(tables.tx_height, "Tx_Height", geometry.tx_size)?;
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
    tables: CoeffOrdinaryTxSizeTables<'_>,
    tx_size: usize,
) -> Result<CoeffOrdinaryTxSizeDimensions, CoeffOrdinaryBranchError> {
    let adjusted_tx_size =
        tx_size_table_usize(tables.adjusted_tx_size, "Adjusted_Tx_Size", tx_size)?;
    tx_size_dimensions(tables, adjusted_tx_size)
}

fn tx_size_context(
    tables: CoeffOrdinaryTxSizeTables<'_>,
    tx_size: usize,
) -> Result<usize, CoeffOrdinaryBranchError> {
    let tx_size_sqr = tx_size_table_tx_size(tables, tables.tx_size_sqr, "Tx_Size_Sqr", tx_size)?;
    let tx_size_sqr_up =
        tx_size_table_tx_size(tables, tables.tx_size_sqr_up, "Tx_Size_Sqr_Up", tx_size)?;
    Ok((tx_size_sqr + tx_size_sqr_up + 1) >> 1)
}

fn tx_size_dimensions(
    tables: CoeffOrdinaryTxSizeTables<'_>,
    tx_size: usize,
) -> Result<CoeffOrdinaryTxSizeDimensions, CoeffOrdinaryBranchError> {
    let tx_width = tx_size_table_usize(tables.tx_width, "Tx_Width", tx_size)?;
    let tx_height = tx_size_table_usize(tables.tx_height, "Tx_Height", tx_size)?;
    let tx_width_log2 = tx_size_table_u32(tables.tx_width_log2, "Tx_Width_Log2", tx_size)?;
    let tx_height_log2 = tx_size_table_u32(tables.tx_height_log2, "Tx_Height_Log2", tx_size)?;
    Ok(CoeffOrdinaryTxSizeDimensions {
        tx_width,
        tx_height,
        tx_width_log2,
        tx_height_log2,
    })
}

fn tx_size_table_tx_size(
    tables: CoeffOrdinaryTxSizeTables<'_>,
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<usize, CoeffOrdinaryBranchError> {
    let value = tx_size_table_usize(table, table_name, tx_size)?;
    tx_size_table_value(tables.tx_width, value)?;
    Ok(value)
}

fn tx_size_scan(
    dimensions: CoeffOrdinaryTxSizeDimensions,
    tx_class: CoeffTransformClass,
) -> Result<Vec<u16>, CoeffOrdinaryBranchError> {
    derive_coeff_scan_order(dimensions.tx_width, dimensions.tx_height, tx_class).map_err(|error| {
        match error {
            CoeffScanOrderError::InvalidShape { width, height } => {
                CoeffOrdinaryBranchError::InvalidScanShape { width, height }
            }
            CoeffScanOrderError::Allocation(source) => {
                CoeffOrdinaryBranchError::ScanAllocation(source)
            }
        }
    })
}

#[cfg(test)]
pub(crate) fn tx_size_scan_for_test(
    tx_width: usize,
    tx_height: usize,
    plane_tx_type: usize,
) -> Result<Vec<u16>, CoeffOrdinaryBranchError> {
    tx_size_scan(
        CoeffOrdinaryTxSizeDimensions {
            tx_width,
            tx_height,
            tx_width_log2: 0,
            tx_height_log2: 0,
        },
        CoeffTransformClass::from_plane_tx_type(plane_tx_type),
    )
}

fn tx_size_table_usize(
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<usize, CoeffOrdinaryBranchError> {
    let value = tx_size_table_value(table, tx_size)?;
    usize::try_from(value).map_err(
        |_| CoeffOrdinaryBranchError::InvalidTransformSizeTableValue {
            table: table_name,
            tx_size,
            value,
        },
    )
}

fn tx_size_table_u32(
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<u32, CoeffOrdinaryBranchError> {
    tx_size_table_usize(table, table_name, tx_size).map(|value| value as u32)
}

fn tx_size_table_value(table: &[i32], tx_size: usize) -> Result<i32, CoeffOrdinaryBranchError> {
    table
        .get(tx_size)
        .copied()
        .ok_or(CoeffOrdinaryBranchError::InvalidTransformSize { tx_size })
}
