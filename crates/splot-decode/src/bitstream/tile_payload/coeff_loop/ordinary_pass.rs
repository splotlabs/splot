// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient pass composition.

use super::super::coeff_state::TileCoeffStateError;
use super::CoeffLoopContextError;
use super::base_level_pass::CoeffBaseDerivedLevelPassError;
use super::quant_pass::CoeffQuantPassError;
use super::sign_symbol::CoeffSignReadError;

pub(crate) mod geometry;

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

#[cfg(test)]
#[path = "ordinary_pass_tests.rs"]
mod tests;
