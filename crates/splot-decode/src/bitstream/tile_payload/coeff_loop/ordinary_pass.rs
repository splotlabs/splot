// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient pass composition.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{TileCoeffContextState, TileCoeffStateError};
use super::base_level_pass::{CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError};
use super::branch::NonZeroCoeffBlockStart;
use super::quant_pass::CoeffQuantPassError;
use super::sign_symbol::CoeffSignReadError;
use super::transform_block::{CoeffTransformBlockInput, decode_ordinary_transform_block};
use super::{AllZeroCoeffBlockInput, CoeffLoopContextError};

pub(crate) mod geometry;

#[derive(Clone, Copy)]
pub(crate) struct CoeffOrdinaryStateContextPassInput {
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryStateContextConfig {
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffOrdinaryPassError {
    #[error("ordinary coefficient pass scan walk failed: {0}")]
    Scan(#[from] CoeffLoopContextError),
    #[error("ordinary coefficient pass derived base/level first pass failed: {0}")]
    BaseDerived(#[from] CoeffBaseDerivedLevelPassError),
    #[error("ordinary coefficient pass sign read failed: {0}")]
    Sign(#[from] CoeffSignReadError),
    #[error("ordinary coefficient pass quant pass failed: {0}")]
    Quant(#[from] CoeffQuantPassError),
    #[error("ordinary coefficient pass context update failed: {0}")]
    ContextUpdate(#[from] TileCoeffStateError),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffOrdinaryBranchError {
    #[error("ordinary coefficient branch invalid transform size index {tx_size}")]
    InvalidTransformSize { tx_size: usize },
    #[error(
        "ordinary coefficient branch invalid {table}[{tx_size}] transform-size table value {value}"
    )]
    InvalidTransformSizeTableValue {
        table: &'static str,
        tx_size: usize,
        value: i32,
    },
    #[error("ordinary coefficient branch invalid UVMode {uv_mode} for Mode_To_Txfm")]
    InvalidUvMode { uv_mode: usize },
    #[error("ordinary coefficient branch invalid intra transform set {tx_set}")]
    InvalidIntraTransformSet { tx_set: usize },
    #[error("ordinary coefficient branch invalid inter transform set {tx_set}")]
    InvalidInterTransformSet { tx_set: usize },
    #[error("ordinary coefficient branch invalid Mode_To_Txfm[{uv_mode}] table value {value}")]
    InvalidModeToTxfmTableValue { uv_mode: usize, value: i32 },
    #[error("ordinary coefficient branch luma TxTypes value {tx_type} is out of range")]
    InvalidLumaTxType { tx_type: usize },
    #[error("ordinary coefficient branch chroma-inter TxTypes value {tx_type} is out of range")]
    InvalidChromaInterTxType { tx_type: usize },
    #[error(
        "ordinary coefficient branch directional UVMode {uv_mode} angle_delta_uv {angle_delta_uv} overflowed"
    )]
    DirectionalAngleOverflow { uv_mode: usize, angle_delta_uv: i32 },
    #[error("ordinary coefficient branch nonzero pass failed: {0}")]
    Ordinary(#[from] CoeffOrdinaryPassError),
}

/// Decodes an ordinary block's coefficients into `quant` and commits its
/// contexts; returns one past its last nonzero `quant` entry.
pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_state_context(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStateContextPassInput,
    quant: &mut [i32],
) -> Result<usize, CoeffOrdinaryPassError> {
    let context = input.state_context;
    decode_ordinary_transform_block(
        state,
        cdfs,
        symbols,
        CoeffTransformBlockInput {
            start: input.start,
            config: input.base_config,
            context: AllZeroCoeffBlockInput {
                plane: input.base_config.plane,
                x4: context.x4,
                y4: context.y4,
                w4: context.w4,
                h4: context.h4,
            },
            lossless: input.lossless,
        },
        quant,
    )
}

#[cfg(test)]
#[path = "ordinary_pass_tests.rs"]
mod tests;
