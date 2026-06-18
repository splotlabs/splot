// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary coefficient branch geometry handoff.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{TX_HEIGHT, TX_HEIGHT_LOG2, TX_WIDTH, TX_WIDTH_LOG2};

use super::super::super::cdf::TileCdfSubset;
use super::super::super::coeff_state::TileCoeffContextState;
use super::super::branch::NonZeroCoeffBlockStartInput;
use super::super::{AllZeroCoeffBlockInput, NonZeroCoeffEobContextInput};
use super::{
    CoeffOrdinaryBranch, CoeffOrdinaryBranchError, CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    CoeffOrdinaryBranchPlaneTypeInput, CoeffOrdinaryBranchPlaneTypeNonZeroInput,
    CoeffOrdinaryPlaneTypeStateContextConfig, apply_coeff_ordinary_branch_from_plane_type,
};

/// Caller-selected ordinary coefficient branch before transform-size dimensions.
pub(crate) enum CoeffOrdinaryBranchTxSizeDimensionsInput<'a> {
    /// Decoded `all_zero == 1`.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput<'a>),
}

/// Caller-resolved facts for the ordinary nonzero branch before dimensions.
pub(crate) struct CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput<'a> {
    /// Caller-resolved `coeffs()` geometry facts before table lookup.
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Caller-resolved inter/intra flag for EOB context derivation.
    pub(crate) is_inter: bool,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved base facts that still are not derived from `txSz`.
    pub(crate) base_config: CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-resolved base-derivation facts before transform-size dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
    /// Transform-size context (`txSzCtx`) for luma coefficient rows.
    pub(crate) tx_size_ctx: usize,
    /// Caller-resolved `PlaneTxType` from AV2 § 5.20.7.29 `compute_tx_type`.
    pub(crate) plane_tx_type: usize,
    /// Whether hidden parity is active for this transform block.
    pub(crate) parity_hiding: bool,
    /// Whether TCQ is active for this transform block.
    pub(crate) use_tcq: bool,
}

/// AV2 § 5.20.7.27 `coeffs()` geometry facts before table lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryTxSizeGeometryConfig {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// `startX` argument to `coeffs()`.
    pub(crate) start_x: usize,
    /// `startY` argument to `coeffs()`.
    pub(crate) start_y: usize,
    /// `txSz` argument to `coeffs()`.
    pub(crate) tx_size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoeffOrdinaryTxSizeDimensions {
    tx_width: usize,
    tx_height: usize,
    tx_width_log2: u32,
    tx_height_log2: u32,
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
        dimensions: CoeffOrdinaryTxSizeDimensions,
        coeff_cdf_q_ctx: usize,
    ) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
        CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
            coeff_cdf_q_ctx,
            tx_size_ctx: self.tx_size_ctx,
            tx_width_log2: dimensions.tx_width_log2,
            tx_width: dimensions.tx_width,
            tx_height: dimensions.tx_height,
            plane: geometry.plane,
            plane_tx_type: self.plane_tx_type,
            parity_hiding: self.parity_hiding,
            use_tcq: self.use_tcq,
        }
    }
}

/// Caller-selected ordinary coefficient branch before block geometry.
pub(crate) enum CoeffOrdinaryBranchCoeffsGeometryInput<'a> {
    /// Decoded `all_zero == 1`.
    AllZero(CoeffOrdinaryCoeffsGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchCoeffsGeometryNonZeroInput<'a>),
}

/// Caller-resolved facts for the ordinary nonzero branch before block geometry.
pub(crate) struct CoeffOrdinaryBranchCoeffsGeometryNonZeroInput<'a> {
    /// Caller-resolved `coeffs()` geometry facts.
    pub(crate) geometry: CoeffOrdinaryCoeffsGeometryConfig,
    /// Caller-resolved facts for reading the nonzero EOB syntax.
    pub(crate) eob: NonZeroCoeffEobContextInput,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors plus `PlaneTxType`.
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    /// Caller-resolved facts for state-backed sign/context handoff, before geometry.
    pub(crate) state_context: CoeffOrdinaryGeometryStateContextConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-resolved AV2 § 5.20.7.27 `coeffs()` geometry facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryCoeffsGeometryConfig {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// `startX` argument to `coeffs()`.
    pub(crate) start_x: usize,
    /// `startY` argument to `coeffs()`.
    pub(crate) start_y: usize,
    /// Caller-resolved `Tx_Width[txSz]`.
    pub(crate) tx_width: usize,
    /// Caller-resolved `Tx_Height[txSz]`.
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

/// Caller-selected ordinary coefficient branch before state-context geometry.
pub(crate) enum CoeffOrdinaryBranchGeometryInput<'a> {
    /// Decoded `all_zero == 1`.
    AllZero(AllZeroCoeffBlockInput),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchGeometryNonZeroInput<'a>),
}

/// Caller-resolved facts for the ordinary nonzero branch before geometry handoff.
pub(crate) struct CoeffOrdinaryBranchGeometryNonZeroInput<'a> {
    /// Caller-resolved facts for nonzero EOB start, including block geometry.
    pub(crate) start: NonZeroCoeffBlockStartInput,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors plus `PlaneTxType`.
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    /// Caller-resolved facts for state-backed sign/context handoff, before geometry.
    pub(crate) state_context: CoeffOrdinaryGeometryStateContextConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-resolved state-context facts before block-geometry handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryGeometryStateContextConfig {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
}

impl CoeffOrdinaryGeometryStateContextConfig {
    const fn state_context(
        self,
        block: AllZeroCoeffBlockInput,
    ) -> CoeffOrdinaryPlaneTypeStateContextConfig {
        CoeffOrdinaryPlaneTypeStateContextConfig {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            x4: block.x4,
            y4: block.y4,
            w4: block.w4,
            h4: block.h4,
        }
    }
}

/// Dispatches the ordinary branch after deriving context geometry from block geometry.
///
/// This adapts the staged branch boundary for AV2 § 5.20.7.27 `coeffs()`
/// geometry (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`) by
/// reusing the already caller-resolved block geometry in
/// `NonZeroCoeffBlockStartInput.block`. It does not derive raw `startX`,
/// `startY`, or `txSz`, implement `compute_tx_type`, derive scan order, wire
/// runtime `coeffs()`, dequantize, inverse transform, residual add, or
/// reconstruct.
pub(crate) fn apply_coeff_ordinary_branch_from_geometry(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchGeometryInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = match input {
        CoeffOrdinaryBranchGeometryInput::AllZero(input) => {
            CoeffOrdinaryBranchPlaneTypeInput::AllZero(input)
        }
        CoeffOrdinaryBranchGeometryInput::NonZero(input) => {
            CoeffOrdinaryBranchPlaneTypeInput::NonZero(CoeffOrdinaryBranchPlaneTypeNonZeroInput {
                start: input.start,
                scan: input.scan,
                base_config: input.base_config,
                state_context: input.state_context.state_context(input.start.block),
                lossless: input.lossless,
            })
        }
    };
    apply_coeff_ordinary_branch_from_plane_type(state, cdfs, symbols, input)
}

/// Dispatches the ordinary branch after deriving block geometry from `coeffs()` facts.
///
/// This adapts the staged branch boundary for AV2 § 5.20.7.27 `coeffs()`
/// geometry (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`) by
/// applying the spec assignments `x4 = startX >> 2`, `y4 = startY >> 2`,
/// `w4 = Tx_Width[txSz] >> 2`, and `h4 = Tx_Height[txSz] >> 2`. It does not
/// derive `Tx_Width[txSz]` or `Tx_Height[txSz]` from `txSz`, implement
/// `compute_tx_type`, derive scan order, wire runtime `coeffs()`, dequantize,
/// inverse transform, residual add, or reconstruct.
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

/// Dispatches the ordinary branch after deriving generated `txSz` dimensions.
///
/// This adapts the staged branch boundary for AV2 § 5.20.7.27 `coeffs()`
/// geometry (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`) by
/// deriving `Tx_Width[txSz]`, `Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`, and
/// `Tx_Height_Log2[txSz]` from generated AV2 § 9.2 conversion tables
/// (`docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md`).
/// It does not derive `Tx_Size_Sqr[txSz]`, `Tx_Size_Sqr_Up[txSz]`, `txSzCtx`,
/// `Adjusted_Tx_Size[txSz]`, implement `compute_tx_type`, derive scan order,
/// wire runtime `coeffs()`, dequantize, inverse transform, residual add, or
/// reconstruct.
pub(crate) fn apply_coeff_ordinary_branch_from_tx_size_dimensions(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchTxSizeDimensionsInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = match input {
        CoeffOrdinaryBranchTxSizeDimensionsInput::AllZero(geometry) => {
            let dimensions = tx_size_dimensions(geometry.tx_size)?;
            CoeffOrdinaryBranchCoeffsGeometryInput::AllZero(geometry.coeffs_geometry(dimensions))
        }
        CoeffOrdinaryBranchTxSizeDimensionsInput::NonZero(input) => {
            let dimensions = tx_size_dimensions(input.geometry.tx_size)?;
            CoeffOrdinaryBranchCoeffsGeometryInput::NonZero(
                CoeffOrdinaryBranchCoeffsGeometryNonZeroInput {
                    geometry: input.geometry.coeffs_geometry(dimensions),
                    eob: NonZeroCoeffEobContextInput {
                        plane: input.geometry.plane,
                        is_inter: input.is_inter,
                        tx_width_log2: dimensions.tx_width_log2 as usize,
                        tx_height_log2: dimensions.tx_height_log2 as usize,
                        coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                    },
                    scan: input.scan,
                    // TODO(spec: DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS):
                    // Use Tx_Width_Log2[Adjusted_Tx_Size[txSz]] for the base
                    // context once the adjusted-size table is generated and wired.
                    base_config: input.base_config.base_config(
                        input.geometry,
                        dimensions,
                        input.coeff_cdf_q_ctx,
                    ),
                    state_context: CoeffOrdinaryGeometryStateContextConfig {
                        coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                    },
                    lossless: input.lossless,
                },
            )
        }
    };
    apply_coeff_ordinary_branch_from_coeffs_geometry(state, cdfs, symbols, input)
}

fn tx_size_dimensions(
    tx_size: usize,
) -> Result<CoeffOrdinaryTxSizeDimensions, CoeffOrdinaryBranchError> {
    Ok(CoeffOrdinaryTxSizeDimensions {
        tx_width: tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?,
        tx_height: tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?,
        tx_width_log2: tx_size_table_u32(&TX_WIDTH_LOG2, "Tx_Width_Log2", tx_size)?,
        tx_height_log2: tx_size_table_u32(&TX_HEIGHT_LOG2, "Tx_Height_Log2", tx_size)?,
    })
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
    let value = tx_size_table_value(table, tx_size)?;
    u32::try_from(value).map_err(
        |_| CoeffOrdinaryBranchError::InvalidTransformSizeTableValue {
            table: table_name,
            tx_size,
            value,
        },
    )
}

fn tx_size_table_value(table: &[i32], tx_size: usize) -> Result<i32, CoeffOrdinaryBranchError> {
    table
        .get(tx_size)
        .copied()
        .ok_or(CoeffOrdinaryBranchError::InvalidTransformSize { tx_size })
}
