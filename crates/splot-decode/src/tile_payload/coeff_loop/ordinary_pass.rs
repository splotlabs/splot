// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient pass composition.
//!
//! Feature tracking: `DECODE-COEFF-ORDINARY-PASS-COMPOSE`,
//! `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS`,
//! `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS`,
//! `DECODE-COEFF-NONZERO-CONTEXT-COMMIT`,
//! `DECODE-COEFF-STATE-CONTEXT-HANDOFF`,
//! `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF`,
//! `DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF`,
//! `DECODE-COEFF-ORDINARY-BRANCH-PLANE-TYPE-HANDOFF`,
//! `DECODE-COEFF-ORDINARY-BRANCH-GEOMETRY-HANDOFF`,
//! `DECODE-COEFF-ORDINARY-BRANCH-COEFFS-GEOMETRY-HANDOFF`,
//! `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS`,
//! `DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE`,
//! `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT`,
//! `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER`,
//! `DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF`.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{
    CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError,
    NonZeroCoeffBaseDerivedLevelPass, apply_nonzero_coeff_base_derived_level_pass,
};
use super::base_symbol::{
    CoeffBaseSymbolRead, CoeffBaseSymbolReadError, CoeffBaseSymbolReadInput,
    read_nonzero_coeff_base_symbols,
};
use super::branch::{
    CoeffBlockEobBranch, CoeffBlockEobBranchInput, NonZeroCoeffBlockStart,
    NonZeroCoeffBlockStartInput, read_coeff_block_eob_branch,
};
use super::level_state::{CoeffLevelStateWriteError, apply_nonzero_coeff_base_levels};
use super::max_level::{CoeffMaxLevelConfig, CoeffTransformClass, derive_nonzero_coeff_max_levels};
use super::quant_pass::{
    CoeffQuantPassConfig, CoeffQuantPassError, CoeffQuantPassMaxLevelConfig, NonZeroCoeffQuantPass,
    validate_coeff_quant_pass_config,
};
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, CoeffQuantStateWriteError,
    NonZeroCoeffQuantState, apply_nonzero_coeff_quant_state_step,
};
use super::read_quant::{CoeffReadQuantConfig, CoeffReadQuantInput, CoeffReadQuantState};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::sign_symbol::{
    CoeffSignRead, CoeffSignReadError, CoeffSignReadInput, CoeffSignReadSymbol,
    CoeffSignSourceDeriveConfig, CoeffSignSourceDeriveError, derive_nonzero_coeff_sign_inputs,
    preflight_nonzero_coeff_signs, read_preflighted_nonzero_coeff_sign,
};
use super::{
    AllZeroCoeffBlock, AllZeroCoeffBlockInput, CoeffLoopContextError, NonZeroCoeffEobSymbolRead,
};

pub(crate) mod geometry;

/// Caller-resolved facts for the loaded ordinary non-FSC coefficient pass.
pub(crate) struct CoeffOrdinaryPassInput<'a> {
    /// Decoded nonzero EOB and zeroed local coefficient state.
    pub(crate) start: NonZeroCoeffBlockStart,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved base/base-range inputs, one per checked scan entry.
    ///
    /// Runtime derivation from evolving `Level[]` neighbours and TCQ state is
    /// intentionally deferred; this loaded boundary only consumes supplied rows.
    pub(crate) base_inputs: &'a [CoeffBaseSymbolReadInput],
    /// Caller-resolved sign inputs, one per checked scan entry.
    ///
    /// Runtime selection of skipped zero-level signs versus sign syntax is
    /// intentionally deferred until the real `coeffs()` integration.
    pub(crate) sign_inputs: &'a [CoeffSignReadInput],
    /// Caller-resolved plane and transform-class facts for `maxLevel`.
    pub(crate) max_level_config: CoeffQuantPassMaxLevelConfig,
    /// Caller-resolved hidden, sumAbs1, TCQ, and lossless facts.
    ///
    /// This wrapper resets `hrLevelAvg` to `0` at the coefficient-block entry
    /// as required by AV2 § 5.20.7.27 before calling `read_quant`.
    pub(crate) quant_config: CoeffQuantPassConfig,
}

/// Caller-resolved facts for the derived-base ordinary non-FSC coefficient pass.
pub(crate) struct CoeffOrdinaryDerivedBasePassInput<'a> {
    /// Decoded nonzero EOB and zeroed local coefficient state.
    pub(crate) start: NonZeroCoeffBlockStart,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors and first-pass state.
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    /// Caller-resolved facts for deriving post-first-pass sign sources.
    pub(crate) sign_config: CoeffOrdinaryDerivedSignPassConfig<'a>,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-resolved facts for the state-backed ordinary non-FSC coefficient pass.
pub(crate) struct CoeffOrdinaryStateContextPassInput<'a> {
    /// Decoded nonzero EOB and zeroed local coefficient state.
    pub(crate) start: NonZeroCoeffBlockStart,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors and first-pass state.
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    /// Caller-resolved facts for reading and committing tile context lines.
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-selected ordinary coefficient branch after `all_zero`.
pub(crate) enum CoeffOrdinaryBranchInput<'a> {
    /// Decoded `all_zero == 1`.
    AllZero(AllZeroCoeffBlockInput),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchNonZeroInput<'a>),
}

/// Caller-resolved facts for the ordinary nonzero branch.
pub(crate) struct CoeffOrdinaryBranchNonZeroInput<'a> {
    /// Caller-resolved facts for nonzero EOB start.
    pub(crate) start: NonZeroCoeffBlockStartInput,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors and first-pass state.
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    /// Caller-resolved facts for reading and committing tile context lines.
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-selected ordinary coefficient branch with nonzero `PlaneTxType`.
pub(crate) enum CoeffOrdinaryBranchPlaneTxTypeInput<'a> {
    /// Decoded `all_zero == 1`.
    AllZero(AllZeroCoeffBlockInput),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchPlaneTxTypeNonZeroInput<'a>),
}

/// Caller-resolved facts for the ordinary nonzero branch before `txClass`.
pub(crate) struct CoeffOrdinaryBranchPlaneTxTypeNonZeroInput<'a> {
    /// Caller-resolved facts for nonzero EOB start.
    pub(crate) start: NonZeroCoeffBlockStartInput,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors plus `PlaneTxType`.
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    /// Caller-resolved facts for reading and committing tile context lines.
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-selected ordinary coefficient branch before nonzero `plane_type`.
pub(crate) enum CoeffOrdinaryBranchPlaneTypeInput<'a> {
    /// Decoded `all_zero == 1`.
    AllZero(AllZeroCoeffBlockInput),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffOrdinaryBranchPlaneTypeNonZeroInput<'a>),
}

/// Caller-resolved facts for the ordinary nonzero branch before `plane_type`.
pub(crate) struct CoeffOrdinaryBranchPlaneTypeNonZeroInput<'a> {
    /// Caller-resolved facts for nonzero EOB start.
    pub(crate) start: NonZeroCoeffBlockStartInput,
    /// Caller-resolved `scan = get_scan(txSz, txClass)` raster positions.
    pub(crate) scan: &'a [u16],
    /// Caller-resolved facts for deriving base selectors plus `PlaneTxType`.
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    /// Caller-resolved facts for reading and committing tile context lines, before `ptype`.
    pub(crate) state_context: CoeffOrdinaryPlaneTypeStateContextConfig,
    /// Caller-resolved lossless flag for the quantized-state update.
    pub(crate) lossless: bool,
}

/// Caller-resolved base-derivation facts with `PlaneTxType`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Transform-size context (`txSzCtx`) for luma coefficient rows.
    pub(crate) tx_size_ctx: usize,
    /// `Tx_Width_Log2[adjTxSz]`, resolved by the caller.
    pub(crate) tx_width_log2: u32,
    /// Adjusted transform width in coefficients.
    pub(crate) tx_width: usize,
    /// Adjusted transform height in coefficients.
    pub(crate) tx_height: usize,
    /// Plane index, 0 for luma and greater than 0 for chroma.
    pub(crate) plane: usize,
    /// Caller-resolved `PlaneTxType` from AV2 § 5.20.7.29 `compute_tx_type`.
    pub(crate) plane_tx_type: usize,
    /// Whether hidden parity is active for this transform block.
    pub(crate) parity_hiding: bool,
    /// Whether TCQ is active for this transform block.
    pub(crate) use_tcq: bool,
}

impl CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    const fn base_config(self) -> CoeffBaseDerivedLevelPassConfig {
        CoeffBaseDerivedLevelPassConfig {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            tx_size_ctx: self.tx_size_ctx,
            tx_width_log2: self.tx_width_log2,
            tx_width: self.tx_width,
            tx_height: self.tx_height,
            plane: self.plane,
            tx_class: CoeffTransformClass::from_plane_tx_type(self.plane_tx_type),
            parity_hiding: self.parity_hiding,
            use_tcq: self.use_tcq,
        }
    }
}

/// Caller-resolved facts used by the derived-base pass to derive sign sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryDerivedSignPassConfig<'a> {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Plane type context used by `TileDcSignCdf`.
    pub(crate) plane_type: usize,
    /// `AboveDcContext[plane]`.
    pub(crate) above_dc: &'a [u8],
    /// `LeftDcContext[plane]`.
    pub(crate) left_dc: &'a [u8],
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

/// Caller-resolved state-context facts for a state-backed ordinary pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryStateContextConfig {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Plane type context used by `TileDcSignCdf`.
    pub(crate) plane_type: usize,
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

/// Caller-resolved state-context facts before AV2 `ptype` derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryPlaneTypeStateContextConfig {
    /// Coefficient-CDF quantization context.
    pub(crate) coeff_cdf_q_ctx: usize,
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

impl CoeffOrdinaryPlaneTypeStateContextConfig {
    const fn state_context(self, plane: usize) -> CoeffOrdinaryStateContextConfig {
        CoeffOrdinaryStateContextConfig {
            coeff_cdf_q_ctx: self.coeff_cdf_q_ctx,
            plane_type: if plane > 0 { 1 } else { 0 },
            x4: self.x4,
            y4: self.y4,
            w4: self.w4,
            h4: self.h4,
        }
    }
}

/// Caller-resolved facts for committing the end-of-`coeffs()` context lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryContextCommitConfig {
    /// Plane index, 0 for luma and 1/2 for chroma.
    pub(crate) plane: usize,
    /// Transform-block x coordinate in 4x4 units.
    pub(crate) x4: usize,
    /// Transform-block y coordinate in 4x4 units.
    pub(crate) y4: usize,
    /// Transform-block width in 4x4 units.
    pub(crate) w4: usize,
    /// Transform-block height in 4x4 units.
    pub(crate) h4: usize,
}

/// Result of the loaded ordinary non-FSC coefficient pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffOrdinaryPass {
    eob_read: NonZeroCoeffEobSymbolRead,
    walk: NonZeroCoeffScanWalk,
    base_reads: Vec<CoeffBaseSymbolRead>,
    sign_reads: Vec<CoeffSignRead>,
    quant_pass: NonZeroCoeffQuantPass,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffOrdinaryPass {
    /// Decoded nonzero EOB syntax carried from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    /// Checked scan walk used by every composed phase.
    #[must_use]
    pub(crate) const fn walk(&self) -> &NonZeroCoeffScanWalk {
        &self.walk
    }

    /// Decoded base/base-range summaries in scan-walk order.
    #[must_use]
    pub(crate) fn base_reads(&self) -> &[CoeffBaseSymbolRead] {
        &self.base_reads
    }

    /// Decoded sign summaries in scan-walk order.
    #[must_use]
    pub(crate) fn sign_reads(&self) -> &[CoeffSignRead] {
        &self.sign_reads
    }

    /// Composed `read_quant` and signed `Quant[]` state summary.
    #[must_use]
    pub(crate) const fn quant_pass(&self) -> &NonZeroCoeffQuantPass {
        &self.quant_pass
    }

    /// Final local coefficient state after `Level[]` and signed `Quant[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Result of the loaded ordinary non-FSC coefficient pass with derived base state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffOrdinaryDerivedBasePass {
    base_level_pass: NonZeroCoeffBaseDerivedLevelPass,
    sign_inputs: Vec<CoeffSignReadInput>,
    sign_reads: Vec<CoeffSignRead>,
    quant_pass: NonZeroCoeffQuantPass,
    block: TransformCoeffBlockState,
}

/// Result of the ordinary coefficient branch handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "crate-private coefficient handoff avoids boxing the hot-path nonzero pass result"
)]
pub(crate) enum CoeffOrdinaryBranch {
    /// All-zero coefficient state was applied.
    AllZero(AllZeroCoeffBlock),
    /// Nonzero ordinary coefficient pass completed and committed context state.
    NonZero(NonZeroCoeffOrdinaryDerivedBasePass),
}

impl NonZeroCoeffOrdinaryDerivedBasePass {
    /// Decoded nonzero EOB syntax carried from block start.
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.base_level_pass.eob_read()
    }

    /// Checked scan walk used by every composed phase.
    #[must_use]
    pub(crate) const fn walk(&self) -> &NonZeroCoeffScanWalk {
        self.base_level_pass.walk()
    }

    /// First-pass base/level derivation result.
    #[must_use]
    pub(crate) const fn base_level_pass(&self) -> &NonZeroCoeffBaseDerivedLevelPass {
        &self.base_level_pass
    }

    /// Derived base/base-range selector inputs in scan-walk order.
    #[must_use]
    pub(crate) fn derived_base_inputs(&self) -> &[CoeffBaseSymbolReadInput] {
        self.base_level_pass.derived_inputs()
    }

    /// Derived sign inputs in scan-walk order.
    #[must_use]
    pub(crate) fn derived_sign_inputs(&self) -> &[CoeffSignReadInput] {
        &self.sign_inputs
    }

    /// Decoded base/base-range summaries in scan-walk order.
    #[must_use]
    pub(crate) fn base_reads(&self) -> &[CoeffBaseSymbolRead] {
        self.base_level_pass.base_reads()
    }

    /// Decoded sign summaries in scan-walk order.
    #[must_use]
    pub(crate) fn sign_reads(&self) -> &[CoeffSignRead] {
        &self.sign_reads
    }

    /// Composed `read_quant` and signed `Quant[]` state summary.
    #[must_use]
    pub(crate) const fn quant_pass(&self) -> &NonZeroCoeffQuantPass {
        &self.quant_pass
    }

    /// Final local coefficient state after `Level[]` and signed `Quant[]` writes.
    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

/// Error returned by the ordinary non-FSC pass composition boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffOrdinaryPassError {
    /// Checked scan walking failed.
    #[error("ordinary coefficient pass scan walk failed: {0}")]
    Scan(#[from] CoeffLoopContextError),
    /// Base/base-range symbol reading failed.
    #[error("ordinary coefficient pass base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
    /// Derived base/level first-pass composition failed.
    #[error("ordinary coefficient pass derived base/level first pass failed: {0}")]
    BaseDerived(#[from] CoeffBaseDerivedLevelPassError),
    /// Derived sign-source composition failed.
    #[error("ordinary coefficient pass derived sign-source pass failed: {0}")]
    SignSource(#[from] CoeffSignSourceDeriveError),
    /// Local `Level[]` state writes failed.
    #[error("ordinary coefficient pass level state write failed: {0}")]
    Level(#[from] CoeffLevelStateWriteError),
    /// Sign syntax reading failed.
    #[error("ordinary coefficient pass sign read failed: {0}")]
    Sign(#[from] CoeffSignReadError),
    /// `read_quant` plus signed `Quant[]` writes failed.
    #[error("ordinary coefficient pass quant pass failed: {0}")]
    Quant(#[from] CoeffQuantPassError),
    /// End-of-`coeffs()` tile context-line update failed.
    #[error("ordinary coefficient pass context update failed: {0}")]
    ContextUpdate(#[from] TileCoeffStateError),
}

/// Error returned by the ordinary coefficient branch handoff.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffOrdinaryBranchError {
    /// `txSz` did not index the generated transform-size conversion tables.
    #[error("ordinary coefficient branch invalid transform size index {tx_size}")]
    InvalidTransformSize {
        /// Caller-provided `txSz` index.
        tx_size: usize,
    },
    /// A generated transform-size conversion table held an invalid dimension.
    #[error(
        "ordinary coefficient branch invalid {table}[{tx_size}] transform-size table value {value}"
    )]
    InvalidTransformSizeTableValue {
        /// AV2 conversion table name.
        table: &'static str,
        /// Caller-provided `txSz` index.
        tx_size: usize,
        /// Generated table value.
        value: i32,
    },
    /// `get_scan(txSz, txClass)` received an unsupported scan extent.
    #[error("ordinary coefficient branch invalid scan shape {width}x{height}")]
    InvalidScanShape {
        /// Scan width after `Min(Tx_Width[txSz], 32)`.
        width: usize,
        /// Scan height after `Min(Tx_Height[txSz], 32)`.
        height: usize,
    },
    /// The `Mode_To_Txfm` subset handoff received a branch it intentionally does not cover.
    #[error("ordinary coefficient branch Mode_To_Txfm handoff does not support {reason}")]
    UnsupportedModeToTxfmSubset {
        /// Unsupported subset reason.
        reason: &'static str,
    },
    /// The lossless subset handoff received a branch it intentionally does not cover.
    #[error("ordinary coefficient branch lossless handoff does not support {reason}")]
    UnsupportedLosslessSubset {
        /// Unsupported subset reason.
        reason: &'static str,
    },
    /// The `Mode_To_Txfm` subset handoff received a `UVMode` outside the table domain.
    #[error("ordinary coefficient branch invalid UVMode {uv_mode} for Mode_To_Txfm")]
    InvalidUvMode {
        /// Caller-provided `UVMode`.
        uv_mode: usize,
    },
    /// The `Mode_To_Txfm` subset handoff received an invalid intra transform set index.
    #[error("ordinary coefficient branch invalid intra transform set {tx_set}")]
    InvalidIntraTransformSet {
        /// Caller-provided `txSet`.
        tx_set: usize,
    },
    /// The chroma-inter `TxTypes` subset handoff received an invalid inter transform set index.
    #[error("ordinary coefficient branch invalid inter transform set {tx_set}")]
    InvalidInterTransformSet {
        /// Caller-provided `txSet`.
        tx_set: usize,
    },
    /// The `get_tx_set` handoff received a caller-resolved reduced set outside f(2).
    #[error("ordinary coefficient branch invalid reduced_tx_set value {reduced_tx_set}")]
    InvalidReducedTxSet {
        /// Caller-provided `reduced_tx_set`.
        reduced_tx_set: usize,
    },
    /// Generated `Mode_To_Txfm` held a value outside the `TX_TYPES` domain.
    #[error("ordinary coefficient branch invalid Mode_To_Txfm[{uv_mode}] table value {value}")]
    InvalidModeToTxfmTableValue {
        /// Caller-provided `UVMode`.
        uv_mode: usize,
        /// Generated table value.
        value: i32,
    },
    /// Caller-resolved luma `TxTypes` value is outside the AV2 `TX_TYPES` domain.
    #[error("ordinary coefficient branch luma TxTypes value {tx_type} is out of range")]
    InvalidLumaTxType {
        /// Caller-resolved luma `TxTypes` value.
        tx_type: usize,
    },
    /// Caller-resolved chroma-inter `TxTypes` value is outside the AV2 `TX_TYPES` domain.
    #[error("ordinary coefficient branch chroma-inter TxTypes value {tx_type} is out of range")]
    InvalidChromaInterTxType {
        /// Caller-resolved chroma-inter `TxTypes` value.
        tx_type: usize,
    },
    /// Directional `UVMode` angle derivation overflowed before `wide_angle_mapping`.
    #[error(
        "ordinary coefficient branch directional UVMode {uv_mode} angle_delta_uv {angle_delta_uv} overflowed"
    )]
    DirectionalAngleOverflow {
        /// Caller-provided `UVMode`.
        uv_mode: usize,
        /// Caller-provided `AngleDeltaUV`.
        angle_delta_uv: i32,
    },
    /// Allocation for a derived scan order failed.
    #[error("ordinary coefficient branch scan allocation failed: {0}")]
    ScanAllocation(#[from] TryReserveError),
    /// EOB branch handoff failed.
    #[error("ordinary coefficient branch handoff failed: {0}")]
    Branch(#[from] CoeffLoopContextError),
    /// Nonzero ordinary coefficient pass failed.
    #[error("ordinary coefficient branch nonzero pass failed: {0}")]
    Ordinary(#[from] CoeffOrdinaryPassError),
    /// Internal branch routing returned a different branch than requested.
    #[error(
        "ordinary coefficient branch returned unexpected {actual} arm while expecting {expected}"
    )]
    UnexpectedBranch {
        /// Expected branch arm.
        expected: &'static str,
        /// Actual branch arm.
        actual: &'static str,
    },
}

/// Dispatches the ordinary coefficient branch after caller-decoded `all_zero`.
///
/// This mirrors the AV2 § 5.20.7.27 `coeffs()` all-zero branch
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). The all-zero
/// arm preserves the existing context-state application. The nonzero arm first
/// initializes and reads the EOB start, then runs the state-backed ordinary
/// non-FSC pass. Runtime transform-block syntax fact derivation,
/// dequantization, inverse transform, residual add, and reconstruction remain
/// out of scope.
pub(crate) fn apply_coeff_ordinary_branch(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    match input {
        CoeffOrdinaryBranchInput::AllZero(input) => {
            match read_coeff_block_eob_branch(
                state,
                cdfs,
                symbols,
                CoeffBlockEobBranchInput::AllZero(input),
            )? {
                CoeffBlockEobBranch::AllZero(block) => Ok(CoeffOrdinaryBranch::AllZero(block)),
                CoeffBlockEobBranch::NonZero(_) => {
                    Err(CoeffOrdinaryBranchError::UnexpectedBranch {
                        expected: "all-zero",
                        actual: "nonzero",
                    })
                }
            }
        }
        CoeffOrdinaryBranchInput::NonZero(input) => {
            let start = match read_coeff_block_eob_branch(
                state,
                cdfs,
                symbols,
                CoeffBlockEobBranchInput::NonZero(input.start),
            )? {
                CoeffBlockEobBranch::NonZero(start) => start,
                CoeffBlockEobBranch::AllZero(_) => {
                    return Err(CoeffOrdinaryBranchError::UnexpectedBranch {
                        expected: "nonzero",
                        actual: "all-zero",
                    });
                }
            };
            apply_nonzero_coeff_ordinary_pass_with_state_context(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryStateContextPassInput {
                    start,
                    scan: input.scan,
                    base_config: input.base_config,
                    state_context: input.state_context,
                    lossless: input.lossless,
                },
            )
            .map(CoeffOrdinaryBranch::NonZero)
            .map_err(CoeffOrdinaryBranchError::from)
        }
    }
}

/// Dispatches the ordinary branch after deriving `txClass` from `PlaneTxType`.
///
/// This is the decode-local handoff for AV2 § 5.20.7.27
/// `txClass = get_tx_class(PlaneTxType)` using the AV2 § 8.3.2 mapping
/// (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`). It does not implement
/// `compute_tx_type`, derive scan order, wire runtime `coeffs()`, dequantize,
/// inverse transform, residual add, or reconstruct.
pub(crate) fn apply_coeff_ordinary_branch_from_plane_tx_type(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchPlaneTxTypeInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = match input {
        CoeffOrdinaryBranchPlaneTxTypeInput::AllZero(input) => {
            CoeffOrdinaryBranchInput::AllZero(input)
        }
        CoeffOrdinaryBranchPlaneTxTypeInput::NonZero(input) => {
            CoeffOrdinaryBranchInput::NonZero(CoeffOrdinaryBranchNonZeroInput {
                start: input.start,
                scan: input.scan,
                base_config: input.base_config.base_config(),
                state_context: input.state_context,
                lossless: input.lossless,
            })
        }
    };
    apply_coeff_ordinary_branch(state, cdfs, symbols, input)
}

/// Dispatches the ordinary branch after deriving `plane_type` from `plane`.
///
/// This models the AV2 § 5.20.7.27 `coeffs()` assignment `ptype = plane > 0`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`) before
/// delegating to the `PlaneTxType` handoff. It does not implement
/// `compute_tx_type`, derive scan order, wire runtime `coeffs()`, dequantize,
/// inverse transform, residual add, or reconstruct.
pub(crate) fn apply_coeff_ordinary_branch_from_plane_type(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchPlaneTypeInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = match input {
        CoeffOrdinaryBranchPlaneTypeInput::AllZero(input) => {
            CoeffOrdinaryBranchPlaneTxTypeInput::AllZero(input)
        }
        CoeffOrdinaryBranchPlaneTypeInput::NonZero(input) => {
            CoeffOrdinaryBranchPlaneTxTypeInput::NonZero(
                CoeffOrdinaryBranchPlaneTxTypeNonZeroInput {
                    start: input.start,
                    scan: input.scan,
                    base_config: input.base_config,
                    state_context: input.state_context.state_context(input.base_config.plane),
                    lossless: input.lossless,
                },
            )
        }
    };
    apply_coeff_ordinary_branch_from_plane_tx_type(state, cdfs, symbols, input)
}

/// Runs the loaded ordinary non-FSC coefficient pass from nonzero EOB start.
///
/// This composes the existing AV2 § 5.20.7.27 boundaries for checked scan walk,
/// base/base-range reads, local `Level[]` writes, and the interleaved sign,
/// `maxLevel`, § 5.20.7.28 `read_quant`, and signed `Quant[pos]` steps
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27` and
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28`). The caller still
/// owns runtime scan, evolving CDF selector, post-level sign-source,
/// transform-class, hidden parity, sumAbs1, TCQ, and lossless derivation. Tile
/// context writes, dequantization, inverse transform, residual add, and
/// reconstruction remain out of scope.
pub(crate) fn apply_nonzero_coeff_ordinary_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryPassInput<'_>,
) -> Result<NonZeroCoeffOrdinaryPass, CoeffOrdinaryPassError> {
    let walk = walk_nonzero_coeff_scan(&input.start, input.scan)?;
    let base_reads = read_nonzero_coeff_base_symbols(cdfs, symbols, &walk, input.base_inputs)?;
    let level_state = apply_nonzero_coeff_base_levels(input.start, &walk, &base_reads)?;
    let (eob_read, mut block) = level_state.into_parts();

    let sign_levels = preflight_nonzero_coeff_signs(&block, &walk, input.sign_inputs)?;
    let quant_config = CoeffQuantPassConfig {
        hr_level_avg: 0,
        ..input.quant_config
    };
    let quant_pass = apply_interleaved_sign_and_quant_pass(
        cdfs,
        symbols,
        &mut block,
        &walk,
        InterleavedSignQuantInput {
            sign_inputs: input.sign_inputs,
            sign_levels: &sign_levels,
            max_level_config: input.max_level_config,
            config: quant_config,
        },
    )?;
    let sign_reads = quant_pass.0;
    let quant_pass = quant_pass.1;

    Ok(NonZeroCoeffOrdinaryPass {
        eob_read,
        walk,
        base_reads,
        sign_reads,
        quant_pass,
        block,
    })
}

/// Runs the loaded ordinary non-FSC coefficient pass with derived base selectors.
///
/// This composes the AV2 § 5.20.7.27 state-derived first pass with the existing
/// interleaved sign, `maxLevel`, § 5.20.7.28 `read_quant`, and signed
/// `Quant[pos]` steps
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27` and
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28`). The first pass
/// supplies base/base-range reads, local `Level[]`, hidden parity, and `sumAbs1`
/// facts. This wrapper derives sign sources from that first-pass state and
/// caller-provided DC context-line facts. The caller still owns runtime scan,
/// geometry, plane, transform-class, parity, TCQ, and lossless derivation.
/// Runtime `coeffs()` wiring,
/// tile context writes, dequantization, inverse transform, residual add, and
/// reconstruction remain out of scope.
pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_derived_base(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryDerivedBasePassInput<'_>,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryPassError> {
    let base_config = input.base_config;
    let sign_config = input.sign_config;
    let lossless = input.lossless;
    let walk = walk_nonzero_coeff_scan(&input.start, input.scan)?;
    let base_level_pass =
        apply_nonzero_coeff_base_derived_level_pass(cdfs, symbols, input.start, walk, base_config)?;
    let first_pass = base_level_pass.first_pass();
    let sign_inputs = derive_nonzero_coeff_sign_inputs(
        base_level_pass.block(),
        base_level_pass.walk(),
        CoeffSignSourceDeriveConfig {
            coeff_cdf_q_ctx: sign_config.coeff_cdf_q_ctx,
            plane: base_config.plane,
            plane_type: sign_config.plane_type,
            tx_class: base_config.tx_class,
            is_hidden: first_pass.is_hidden(),
            sum_abs1: first_pass.sum_abs1(),
            above_dc: sign_config.above_dc,
            left_dc: sign_config.left_dc,
            x4: sign_config.x4,
            y4: sign_config.y4,
            w4: sign_config.w4,
            h4: sign_config.h4,
        },
    )?;
    let sign_levels = preflight_nonzero_coeff_signs(
        base_level_pass.block(),
        base_level_pass.walk(),
        &sign_inputs,
    )?;
    let quant_config = CoeffQuantPassConfig {
        is_hidden: first_pass.is_hidden(),
        sum_abs1: first_pass.sum_abs1(),
        use_tcq: base_config.use_tcq,
        lossless,
        hr_level_avg: 0,
    };
    let mut block = base_level_pass.block().clone();
    let quant_pass = apply_interleaved_sign_and_quant_pass(
        cdfs,
        symbols,
        &mut block,
        base_level_pass.walk(),
        InterleavedSignQuantInput {
            sign_inputs: &sign_inputs,
            sign_levels: &sign_levels,
            max_level_config: CoeffQuantPassMaxLevelConfig {
                plane: base_config.plane,
                tx_class: base_config.tx_class,
            },
            config: quant_config,
        },
    )?;
    let sign_reads = quant_pass.0;
    let quant_pass = quant_pass.1;

    Ok(NonZeroCoeffOrdinaryDerivedBasePass {
        base_level_pass,
        sign_inputs,
        sign_reads,
        quant_pass,
        block,
    })
}

/// Runs the derived ordinary non-FSC pass and commits tile context lines.
///
/// This wraps [`apply_nonzero_coeff_ordinary_pass_with_derived_base`] with the
/// AV2 § 5.20.7.27 end-of-`coeffs()` context update
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). The final
/// `culLevel` and `dcCategory` come from the signed `Quant[]` state summary
/// produced after § 5.20.7.28 `read_quant`; the caller still resolves scan,
/// transform, plane, geometry, parity, TCQ, lossless, and DC context facts.
/// Runtime `coeffs()` wiring, dequantization, inverse transform, residual add,
/// and reconstruction remain out of scope.
pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_context_commit(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryDerivedBasePassInput<'_>,
    context: CoeffOrdinaryContextCommitConfig,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryPassError> {
    let pass = apply_nonzero_coeff_ordinary_pass_with_derived_base(cdfs, symbols, input)?;
    let quant_state = pass.quant_pass().quant_state();
    state.update_after_coeffs(CoeffContextUpdate {
        plane: context.plane,
        x4: context.x4,
        y4: context.y4,
        w4: context.w4,
        h4: context.h4,
        cul_level: quant_state.cul_level(),
        dc_category: quant_state.dc_category(),
    })?;
    Ok(pass)
}

/// Runs the derived ordinary non-FSC pass with state-backed DC context handoff.
///
/// This reads `AboveDcContext[plane]` and `LeftDcContext[plane]` from
/// [`TileCoeffContextState`] before sign-source derivation, then reuses the
/// AV2 § 5.20.7.27 end-of-`coeffs()` context update
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`) through
/// [`apply_nonzero_coeff_ordinary_pass_with_context_commit`]. Runtime
/// `coeffs()` wiring, dequantization, inverse transform, residual add, and
/// reconstruction remain out of scope.
pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_state_context(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStateContextPassInput<'_>,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryPassError> {
    let CoeffOrdinaryStateContextPassInput {
        start,
        scan,
        base_config,
        state_context,
        lossless,
    } = input;
    let plane = base_config.plane;
    let above_dc = clone_dc_context_line(state.above_dc(plane)?)?;
    let left_dc = clone_dc_context_line(state.left_dc(plane)?)?;
    apply_nonzero_coeff_ordinary_pass_with_context_commit(
        state,
        cdfs,
        symbols,
        CoeffOrdinaryDerivedBasePassInput {
            start,
            scan,
            base_config,
            sign_config: CoeffOrdinaryDerivedSignPassConfig {
                coeff_cdf_q_ctx: state_context.coeff_cdf_q_ctx,
                plane_type: state_context.plane_type,
                above_dc: &above_dc,
                left_dc: &left_dc,
                x4: state_context.x4,
                y4: state_context.y4,
                w4: state_context.w4,
                h4: state_context.h4,
            },
            lossless,
        },
        CoeffOrdinaryContextCommitConfig {
            plane,
            x4: state_context.x4,
            y4: state_context.y4,
            w4: state_context.w4,
            h4: state_context.h4,
        },
    )
}

fn clone_dc_context_line(values: &[u8]) -> Result<Vec<u8>, CoeffOrdinaryPassError> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(values.len())
        .map_err(CoeffSignSourceDeriveError::from)?;
    copy.extend_from_slice(values);
    Ok(copy)
}

struct InterleavedSignQuantInput<'a> {
    sign_inputs: &'a [CoeffSignReadInput],
    sign_levels: &'a [u32],
    max_level_config: CoeffQuantPassMaxLevelConfig,
    config: CoeffQuantPassConfig,
}

fn apply_interleaved_sign_and_quant_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    block: &mut TransformCoeffBlockState,
    walk: &NonZeroCoeffScanWalk,
    input: InterleavedSignQuantInput<'_>,
) -> Result<(Vec<CoeffSignRead>, NonZeroCoeffQuantPass), CoeffOrdinaryPassError> {
    let InterleavedSignQuantInput {
        sign_inputs,
        sign_levels,
        max_level_config,
        config,
    } = input;

    validate_coeff_quant_pass_config(config)?;

    let entries = walk.entries();
    let max_levels = derive_nonzero_coeff_max_levels(
        walk,
        CoeffMaxLevelConfig {
            plane: max_level_config.plane,
            tx_class: max_level_config.tx_class,
            is_hidden: config.is_hidden,
        },
    )
    .map_err(CoeffQuantPassError::from)?;
    for (index, (entry, max_level)) in entries.iter().copied().zip(max_levels.iter()).enumerate() {
        debug_assert_eq!(max_level.entry, entry);
        max_level
            .max_level
            .checked_sub(u32::from(config.use_tcq))
            .ok_or(CoeffQuantPassError::InvalidMaxLevel {
                index,
                max_level: max_level.max_level,
                use_tcq: config.use_tcq,
            })?;
        block
            .quant_at(entry.pos())
            .map_err(CoeffQuantPassError::from)?;
    }

    let mut sign_reads = Vec::new();
    let mut read_quants = Vec::new();
    let mut quant_writes = Vec::new();
    sign_reads
        .try_reserve(entries.len())
        .map_err(CoeffSignReadError::from)?;
    read_quants
        .try_reserve(entries.len())
        .map_err(CoeffQuantPassError::from)?;
    quant_writes
        .try_reserve(entries.len())
        .map_err(CoeffQuantStateWriteError::from)
        .map_err(CoeffQuantPassError::from)?;

    let mut read_quant_state = CoeffReadQuantState::new(CoeffReadQuantConfig {
        is_hidden: config.is_hidden,
        allow_tcq: config.use_tcq,
        hr_level_avg: config.hr_level_avg,
    });
    let mut quant_state = CoeffQuantStateAccumulator::new(CoeffQuantStateConfig {
        is_hidden: config.is_hidden,
        sum_abs1: config.sum_abs1,
        use_tcq: config.use_tcq,
        lossless: config.lossless,
    });

    for (index, (((entry, sign_input), level), max_level)) in entries
        .iter()
        .copied()
        .zip(sign_inputs.iter().copied())
        .zip(sign_levels.iter().copied())
        .zip(max_levels.iter().copied())
        .enumerate()
    {
        let sign = read_preflighted_nonzero_coeff_sign(cdfs, symbols, sign_input, level)?;
        if config.is_hidden
            && config.sum_abs1 > 0
            && entry.scan_index() == 0
            && sign.symbol() == CoeffSignReadSymbol::None
        {
            return Err(CoeffQuantPassError::HiddenParityMissingSign { index, entry }.into());
        }
        let read_quant = read_quant_state
            .read_one(
                symbols,
                index,
                CoeffReadQuantInput {
                    entry,
                    level,
                    max_level: max_level.max_level,
                },
            )
            .map_err(CoeffQuantPassError::from)?;
        let write = apply_nonzero_coeff_quant_state_step(
            block,
            &mut quant_state,
            index,
            entry,
            sign,
            read_quant.quant_input(),
        )
        .map_err(CoeffQuantPassError::from)?;

        sign_reads.push(sign);
        read_quants.push(read_quant);
        quant_writes.push(write);
    }

    let quant_state = NonZeroCoeffQuantState::from_interleaved_parts(quant_writes, quant_state);
    Ok((
        sign_reads,
        NonZeroCoeffQuantPass::from_interleaved_parts(read_quants, quant_state),
    ))
}
