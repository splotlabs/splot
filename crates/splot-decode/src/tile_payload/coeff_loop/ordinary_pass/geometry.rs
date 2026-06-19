// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary coefficient branch geometry handoff.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    ADJUSTED_TX_SIZE, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR, TX_SIZE_SQR_UP, TX_WIDTH,
    TX_WIDTH_LOG2,
};

use super::super::super::cdf::TileCdfSubset;
use super::super::super::coeff_state::TileCoeffContextState;
use super::super::branch::NonZeroCoeffBlockStartInput;
use super::super::max_level::CoeffTransformClass;
use super::super::{AllZeroCoeffBlockInput, NonZeroCoeffEobContextInput};
use super::{
    CoeffOrdinaryBranch, CoeffOrdinaryBranchError, CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    CoeffOrdinaryBranchPlaneTypeInput, CoeffOrdinaryBranchPlaneTypeNonZeroInput,
    CoeffOrdinaryPlaneTypeStateContextConfig, apply_coeff_ordinary_branch_from_plane_type,
};

/// Caller-selected ordinary coefficient branch before transform-size dimensions.
pub(crate) enum CoeffOrdinaryBranchTxSizeDimensionsInput {
    /// Decoded `all_zero == 1`.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput),
}

/// Caller-resolved facts for the ordinary nonzero branch before dimensions.
pub(crate) struct CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
    /// Caller-resolved `coeffs()` geometry facts before table lookup.
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Caller-resolved inter/intra flag for EOB context derivation.
    pub(crate) is_inter: bool,
    /// Caller-resolved base facts that still are not derived from `txSz`.
    pub(crate) base_config: CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-resolved base-derivation facts before transform-size dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
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

#[derive(Clone, Copy)]
struct CoeffOrdinaryTxSizeTables<'a> {
    adjusted_tx_size: &'a [i32],
    tx_size_sqr: &'a [i32],
    tx_size_sqr_up: &'a [i32],
    tx_width: &'a [i32],
    tx_height: &'a [i32],
    tx_width_log2: &'a [i32],
    tx_height_log2: &'a [i32],
}

#[cfg(test)]
pub(crate) struct CoeffOrdinaryTestDimensionTables<'a> {
    pub(crate) tx_width: &'a [i32],
    pub(crate) tx_height: &'a [i32],
    pub(crate) tx_width_log2: &'a [i32],
    pub(crate) tx_height_log2: &'a [i32],
}

const DEFAULT_TX_SIZE_TABLES: CoeffOrdinaryTxSizeTables<'static> = CoeffOrdinaryTxSizeTables {
    adjusted_tx_size: &ADJUSTED_TX_SIZE,
    tx_size_sqr: &TX_SIZE_SQR,
    tx_size_sqr_up: &TX_SIZE_SQR_UP,
    tx_width: &TX_WIDTH,
    tx_height: &TX_HEIGHT,
    tx_width_log2: &TX_WIDTH_LOG2,
    tx_height_log2: &TX_HEIGHT_LOG2,
};

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
/// For AV2 § 8.3.2 base contexts
/// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`), it separately
/// derives `Adjusted_Tx_Size[txSz]` and uses the adjusted width, height, and
/// width log2. It also derives the AV2 § 5.20.7.27 `txSzCtx` formula from
/// `Tx_Size_Sqr[txSz]` and `Tx_Size_Sqr_Up[txSz]`, then derives
/// `scan = get_scan(txSz, txClass)` per AV2 § 5.20.7.30
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30`). It does not
/// implement `compute_tx_type`, wire runtime `coeffs()`, dequantize, inverse
/// transform, residual add, or reconstruct.
pub(crate) fn apply_coeff_ordinary_branch_from_tx_size_dimensions(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchTxSizeDimensionsInput,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables(
        state,
        cdfs,
        symbols,
        input,
        DEFAULT_TX_SIZE_TABLES,
    )
}

#[cfg(test)]
pub(crate) fn apply_coeff_ordinary_branch_from_tx_size_dimensions_with_test_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchTxSizeDimensionsInput,
    adjusted_tx_size_table: &[i32],
    tx_size_sqr_table: &[i32],
    tx_size_sqr_up_table: &[i32],
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables(
        state,
        cdfs,
        symbols,
        input,
        CoeffOrdinaryTxSizeTables {
            adjusted_tx_size: adjusted_tx_size_table,
            tx_size_sqr: tx_size_sqr_table,
            tx_size_sqr_up: tx_size_sqr_up_table,
            ..DEFAULT_TX_SIZE_TABLES
        },
    )
}

#[cfg(test)]
pub(crate) fn apply_coeff_ordinary_branch_from_tx_size_dimensions_with_test_dimension_tables(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchTxSizeDimensionsInput,
    dimension_tables: CoeffOrdinaryTestDimensionTables<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables(
        state,
        cdfs,
        symbols,
        input,
        CoeffOrdinaryTxSizeTables {
            tx_width: dimension_tables.tx_width,
            tx_height: dimension_tables.tx_height,
            tx_width_log2: dimension_tables.tx_width_log2,
            tx_height_log2: dimension_tables.tx_height_log2,
            ..DEFAULT_TX_SIZE_TABLES
        },
    )
}

fn apply_coeff_ordinary_branch_from_tx_size_dimensions_with_tables(
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
            let raw_dimensions = tx_size_dimensions(tables, input.geometry.tx_size)?;
            let adjusted_dimensions = adjusted_tx_size_dimensions(tables, input.geometry.tx_size)?;
            let tx_size_ctx = tx_size_context(tables, input.geometry.tx_size)?;
            let tx_class = CoeffTransformClass::from_plane_tx_type(input.base_config.plane_tx_type);
            let scan = tx_size_scan(raw_dimensions, tx_class)?;
            apply_coeff_ordinary_branch_from_coeffs_geometry(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchCoeffsGeometryInput::NonZero(
                    CoeffOrdinaryBranchCoeffsGeometryNonZeroInput {
                        geometry: input.geometry.coeffs_geometry(raw_dimensions),
                        eob: NonZeroCoeffEobContextInput {
                            plane: input.geometry.plane,
                            is_inter: input.is_inter,
                            tx_width_log2: raw_dimensions.tx_width_log2 as usize,
                            tx_height_log2: raw_dimensions.tx_height_log2 as usize,
                            coeff_cdf_q_ctx: input.coeff_cdf_q_ctx,
                        },
                        scan: &scan,
                        base_config: input.base_config.base_config(
                            input.geometry,
                            tx_size_ctx,
                            adjusted_dimensions,
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
    Ok(CoeffOrdinaryTxSizeDimensions {
        tx_width: tx_size_table_usize(tables.tx_width, "Tx_Width", tx_size)?,
        tx_height: tx_size_table_usize(tables.tx_height, "Tx_Height", tx_size)?,
        tx_width_log2: tx_size_table_u32(tables.tx_width_log2, "Tx_Width_Log2", tx_size)?,
        tx_height_log2: tx_size_table_u32(tables.tx_height_log2, "Tx_Height_Log2", tx_size)?,
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
    let width = dimensions.tx_width.min(32);
    let height = dimensions.tx_height.min(32);
    if !matches!(width, 4 | 8 | 16 | 32) || !matches!(height, 4 | 8 | 16 | 32) {
        return Err(CoeffOrdinaryBranchError::InvalidScanShape { width, height });
    }
    let coeff_count = width * height;
    let mut out = Vec::new();
    out.try_reserve_exact(coeff_count)?;
    match tx_class {
        CoeffTransformClass::Vertical => {
            for y in 0..height {
                for x in 0..width {
                    out.push((y * width + x) as u16);
                }
            }
        }
        CoeffTransformClass::Horizontal => {
            for x in 0..width {
                for y in 0..height {
                    out.push((y * width + x) as u16);
                }
            }
        }
        CoeffTransformClass::TwoD => {
            let (wi, hi) = (width as i32, height as i32);
            let (mut x, mut y) = (0i32, 0i32);
            for _ in 0..coeff_count {
                out.push((y * wi + x) as u16);
                x += 1;
                y -= 1;
                if y < 0 || x >= wi {
                    x += 1;
                    let s = x.min(hi - 1 - y);
                    x -= s;
                    y += s;
                }
            }
        }
    }
    Ok(out)
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
