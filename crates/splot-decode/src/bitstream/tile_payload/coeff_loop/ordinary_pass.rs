// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient pass composition.

use std::collections::TryReserveError;

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::cdf::block_read::BlockSymbolTraceReadError;
use super::super::coeff_state::{
    TileCoeffContextState, TileCoeffStateError, TransformCoeffBlockState,
};
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError,
    NonZeroCoeffBaseDerivedLevelPass, apply_nonzero_coeff_base_derived_level_pass,
};
use super::base_symbol::{
    CoeffBaseSymbolReadError, CoeffBaseSymbolReadInput, read_nonzero_coeff_base_symbols,
};
use super::branch::{NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::level_state::{CoeffLevelStateWriteError, apply_nonzero_coeff_base_levels};
use super::max_level::{CoeffMaxLevelConfig, CoeffTransformClass, derive_coeff_max_level};
use super::quant_pass::{
    CoeffQuantPassConfig, CoeffQuantPassError, CoeffQuantPassMaxLevelConfig,
    validate_coeff_quant_pass_config,
};
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, NonZeroCoeffQuantState,
    apply_nonzero_coeff_quant_state_step,
};
use super::read_quant::{CoeffReadQuantConfig, CoeffReadQuantInput, CoeffReadQuantState};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::sign_symbol::{
    CoeffSignReadError, CoeffSignReadInput, CoeffSignReadSymbol, CoeffSignSourceDeriveConfig,
    CoeffSignSourceDeriveError, derive_nonzero_coeff_sign_input, preflight_nonzero_coeff_signs,
    read_preflighted_nonzero_coeff_sign,
};
use super::{
    AllZeroCoeffBlock, AllZeroCoeffBlockInput, CoeffBranchInput, CoeffLoopContextError,
    NonZeroCoeffEobSymbolRead, apply_all_zero_coeff_block, commit_nonzero_coeff_context,
    read_nonzero_coeff_block_start,
};

pub(crate) mod geometry;

pub(crate) struct CoeffOrdinaryPassInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) scan: &'a [u16],
    pub(crate) base_inputs: &'a [CoeffBaseSymbolReadInput],
    pub(crate) sign_inputs: &'a [CoeffSignReadInput],
    pub(crate) max_level_config: CoeffQuantPassMaxLevelConfig,
    pub(crate) quant_config: CoeffQuantPassConfig,
}

pub(crate) struct CoeffOrdinaryDerivedBasePassInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    pub(crate) sign_config: CoeffOrdinaryDerivedSignPassConfig<'a>,
    pub(crate) lossless: bool,
}

pub(crate) struct CoeffOrdinaryStateContextPassInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    pub(crate) lossless: bool,
}

pub(crate) type CoeffOrdinaryBranchInput<'a> =
    CoeffBranchInput<AllZeroCoeffBlockInput, CoeffOrdinaryBranchNonZeroInput<'a>>;

pub(crate) struct CoeffOrdinaryBranchNonZeroInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStartInput,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    pub(crate) lossless: bool,
}

pub(crate) type CoeffOrdinaryBranchPlaneTxTypeInput<'a> =
    CoeffBranchInput<AllZeroCoeffBlockInput, CoeffOrdinaryBranchPlaneTxTypeNonZeroInput<'a>>;

pub(crate) struct CoeffOrdinaryBranchPlaneTxTypeNonZeroInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStartInput,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    pub(crate) lossless: bool,
}

pub(crate) type CoeffOrdinaryBranchPlaneTypeInput<'a> =
    CoeffBranchInput<AllZeroCoeffBlockInput, CoeffOrdinaryBranchPlaneTypeNonZeroInput<'a>>;

pub(crate) struct CoeffOrdinaryBranchPlaneTypeNonZeroInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStartInput,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    pub(crate) state_context: CoeffOrdinaryPlaneTypeStateContextConfig,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) tx_size_ctx: usize,
    pub(crate) tx_width_log2: u32,
    pub(crate) tx_width: usize,
    pub(crate) tx_height: usize,
    pub(crate) plane: usize,
    pub(crate) plane_tx_type: usize,
    pub(crate) parity_hiding: bool,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryDerivedSignPassConfig<'a> {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) plane_type: usize,
    pub(crate) above_dc: &'a [u8],
    pub(crate) left_dc: &'a [u8],
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryStateContextConfig {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) plane_type: usize,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
    pub(crate) h4: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryPlaneTypeStateContextConfig {
    pub(crate) coeff_cdf_q_ctx: usize,
    pub(crate) x4: usize,
    pub(crate) y4: usize,
    pub(crate) w4: usize,
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

pub(crate) type CoeffOrdinaryContextCommitConfig = AllZeroCoeffBlockInput;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffOrdinaryPass {
    eob_read: NonZeroCoeffEobSymbolRead,
    walk: NonZeroCoeffScanWalk,
    quant_state: NonZeroCoeffQuantState,
    block: TransformCoeffBlockState,
}

impl NonZeroCoeffOrdinaryPass {
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.eob_read
    }

    #[must_use]
    pub(crate) const fn walk(&self) -> &NonZeroCoeffScanWalk {
        &self.walk
    }

    #[must_use]
    pub(crate) const fn quant_state(&self) -> &NonZeroCoeffQuantState {
        &self.quant_state
    }

    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        &self.block
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonZeroCoeffOrdinaryDerivedBasePass {
    base_level_pass: NonZeroCoeffBaseDerivedLevelPass,
    quant_state: NonZeroCoeffQuantState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "crate-private coefficient handoff avoids boxing the hot-path nonzero pass result"
)]
pub(crate) enum CoeffOrdinaryBranch {
    AllZero(AllZeroCoeffBlock),
    NonZero(NonZeroCoeffOrdinaryDerivedBasePass),
}

impl NonZeroCoeffOrdinaryDerivedBasePass {
    #[must_use]
    pub(crate) const fn eob_read(&self) -> NonZeroCoeffEobSymbolRead {
        self.base_level_pass.eob_read()
    }

    #[must_use]
    pub(crate) const fn walk(&self) -> &NonZeroCoeffScanWalk {
        self.base_level_pass.walk()
    }

    #[must_use]
    pub(crate) const fn base_level_pass(&self) -> &NonZeroCoeffBaseDerivedLevelPass {
        &self.base_level_pass
    }

    #[must_use]
    pub(crate) const fn quant_state(&self) -> &NonZeroCoeffQuantState {
        &self.quant_state
    }

    #[must_use]
    pub(crate) const fn block(&self) -> &TransformCoeffBlockState {
        self.base_level_pass.block()
    }

    #[must_use]
    pub(crate) fn into_block(self) -> TransformCoeffBlockState {
        self.base_level_pass.into_block()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffOrdinaryPassError {
    #[error("ordinary coefficient pass scan walk failed: {0}")]
    Scan(#[from] CoeffLoopContextError),
    #[error("ordinary coefficient pass base symbol read failed: {0}")]
    Base(#[from] CoeffBaseSymbolReadError),
    #[error("ordinary coefficient pass derived base/level first pass failed: {0}")]
    BaseDerived(#[from] CoeffBaseDerivedLevelPassError),
    #[error("ordinary coefficient pass derived sign-source pass failed: {0}")]
    SignSource(#[from] CoeffSignSourceDeriveError),
    #[error("ordinary coefficient pass level state write failed: {0}")]
    Level(#[from] CoeffLevelStateWriteError),
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
    #[error("ordinary coefficient branch invalid scan shape {width}x{height}")]
    InvalidScanShape { width: usize, height: usize },
    #[error("ordinary coefficient branch Mode_To_Txfm handoff does not support {reason}")]
    UnsupportedModeToTxfmSubset { reason: &'static str },
    #[error("ordinary coefficient branch invalid UVMode {uv_mode} for Mode_To_Txfm")]
    InvalidUvMode { uv_mode: usize },
    #[error("ordinary coefficient branch invalid intra transform set {tx_set}")]
    InvalidIntraTransformSet { tx_set: usize },
    #[error("ordinary coefficient branch invalid inter transform set {tx_set}")]
    InvalidInterTransformSet { tx_set: usize },
    #[error("ordinary coefficient branch invalid reduced_tx_set value {reduced_tx_set}")]
    InvalidReducedTxSet { reduced_tx_set: usize },
    #[error("ordinary coefficient branch invalid Mode_To_Txfm[{uv_mode}] table value {value}")]
    InvalidModeToTxfmTableValue { uv_mode: usize, value: i32 },
    #[error("ordinary coefficient branch luma TxTypes value {tx_type} is out of range")]
    InvalidLumaTxType { tx_type: usize },
    #[error("ordinary coefficient branch chroma-inter TxTypes value {tx_type} is out of range")]
    InvalidChromaInterTxType { tx_type: usize },
    #[error("ordinary coefficient branch lossless inter transform type read failed: {0}")]
    LosslessInterTxType(#[from] BlockSymbolTraceReadError),
    #[error(
        "ordinary coefficient branch directional UVMode {uv_mode} angle_delta_uv {angle_delta_uv} overflowed"
    )]
    DirectionalAngleOverflow { uv_mode: usize, angle_delta_uv: i32 },
    #[error("ordinary coefficient branch scan allocation failed: {0}")]
    ScanAllocation(#[from] TryReserveError),
    #[error("ordinary coefficient branch handoff failed: {0}")]
    Branch(#[from] CoeffLoopContextError),
    #[error("ordinary coefficient branch nonzero pass failed: {0}")]
    Ordinary(#[from] CoeffOrdinaryPassError),
}

pub(crate) fn apply_coeff_ordinary_branch(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    match input {
        CoeffOrdinaryBranchInput::AllZero(input) => Ok(CoeffOrdinaryBranch::AllZero(
            apply_all_zero_coeff_block(state, input)?,
        )),
        CoeffOrdinaryBranchInput::NonZero(input) => {
            let start = read_nonzero_coeff_block_start(cdfs, symbols, input.start)?;
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

coeff_branch_map_adapter!(
    pub(crate) fn apply_coeff_ordinary_branch_from_plane_tx_type(
        CoeffOrdinaryBranchPlaneTxTypeInput<'_>
    ) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError>,
    input,
    CoeffOrdinaryBranchNonZeroInput {
        start: input.start,
        scan: input.scan,
        base_config: input.base_config.base_config(),
        state_context: input.state_context,
        lossless: input.lossless,
    },
    apply_coeff_ordinary_branch,
);

pub(crate) fn apply_coeff_ordinary_branch_from_plane_type(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryBranchPlaneTypeInput<'_>,
) -> Result<CoeffOrdinaryBranch, CoeffOrdinaryBranchError> {
    let input = input.map_nonzero(|input| CoeffOrdinaryBranchPlaneTxTypeNonZeroInput {
        start: input.start,
        scan: input.scan,
        base_config: input.base_config,
        state_context: input.state_context.state_context(input.base_config.plane),
        lossless: input.lossless,
    });
    apply_coeff_ordinary_branch_from_plane_tx_type(state, cdfs, symbols, input)
}

pub(crate) fn apply_nonzero_coeff_ordinary_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryPassInput<'_>,
) -> Result<NonZeroCoeffOrdinaryPass, CoeffOrdinaryPassError> {
    let walk = walk_nonzero_coeff_scan(&input.start, input.scan)?;
    let base_reads = read_nonzero_coeff_base_symbols(cdfs, symbols, &walk, input.base_inputs)?;
    let level_state = apply_nonzero_coeff_base_levels(input.start, &walk, &base_reads)?;
    let (eob_read, mut block) = level_state.into_parts();

    preflight_nonzero_coeff_signs(&block, &walk, input.sign_inputs)?;
    let quant_config = CoeffQuantPassConfig {
        hr_level_avg: 0,
        ..input.quant_config
    };
    let quant_state = apply_interleaved_sign_and_quant_pass(
        cdfs,
        symbols,
        InterleavedSignQuantPassInput {
            block: &mut block,
            walk: &walk,
            sign_inputs: InterleavedSignInputs::Explicit(input.sign_inputs),
            max_level_config: input.max_level_config,
            config: quant_config,
        },
    )?;

    Ok(NonZeroCoeffOrdinaryPass {
        eob_read,
        walk,
        quant_state,
        block,
    })
}

pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_derived_base(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryDerivedBasePassInput<'_>,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryPassError> {
    let base_config = input.base_config;
    let sign_config = input.sign_config;
    let lossless = input.lossless;
    let walk = walk_nonzero_coeff_scan(&input.start, input.scan)?;
    let mut base_level_pass =
        apply_nonzero_coeff_base_derived_level_pass(cdfs, symbols, input.start, walk, base_config)?;
    let first_pass = base_level_pass.first_pass();
    let sign_derive_config = CoeffSignSourceDeriveConfig {
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
    };
    for entry in base_level_pass.walk().entries().iter().copied() {
        base_level_pass
            .block()
            .level_at(entry.row(), entry.col())
            .map_err(CoeffSignSourceDeriveError::from)?;
    }
    let quant_config = CoeffQuantPassConfig {
        is_hidden: first_pass.is_hidden(),
        sum_abs1: first_pass.sum_abs1(),
        use_tcq: base_config.use_tcq,
        lossless,
        hr_level_avg: 0,
    };
    let (walk, block) = base_level_pass.walk_and_block_mut();
    let quant_state = apply_interleaved_sign_and_quant_pass(
        cdfs,
        symbols,
        InterleavedSignQuantPassInput {
            block,
            walk,
            sign_inputs: InterleavedSignInputs::Derived(sign_derive_config),
            max_level_config: CoeffQuantPassMaxLevelConfig {
                plane: base_config.plane,
                tx_class: base_config.tx_class,
            },
            config: quant_config,
        },
    )?;
    Ok(NonZeroCoeffOrdinaryDerivedBasePass {
        base_level_pass,
        quant_state,
    })
}

pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_context_commit(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryDerivedBasePassInput<'_>,
    context: CoeffOrdinaryContextCommitConfig,
) -> Result<NonZeroCoeffOrdinaryDerivedBasePass, CoeffOrdinaryPassError> {
    let pass = apply_nonzero_coeff_ordinary_pass_with_derived_base(cdfs, symbols, input)?;
    commit_coeff_context(state, &pass, context)?;
    Ok(pass)
}

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
    let pass = apply_nonzero_coeff_ordinary_pass_with_derived_base(
        cdfs,
        symbols,
        CoeffOrdinaryDerivedBasePassInput {
            start,
            scan,
            base_config,
            sign_config: CoeffOrdinaryDerivedSignPassConfig {
                coeff_cdf_q_ctx: state_context.coeff_cdf_q_ctx,
                plane_type: state_context.plane_type,
                above_dc: state.above_dc(plane)?,
                left_dc: state.left_dc(plane)?,
                x4: state_context.x4,
                y4: state_context.y4,
                w4: state_context.w4,
                h4: state_context.h4,
            },
            lossless,
        },
    )?;
    commit_coeff_context(
        state,
        &pass,
        CoeffOrdinaryContextCommitConfig {
            plane,
            x4: state_context.x4,
            y4: state_context.y4,
            w4: state_context.w4,
            h4: state_context.h4,
        },
    )?;
    Ok(pass)
}

fn commit_coeff_context(
    state: &mut TileCoeffContextState,
    pass: &NonZeroCoeffOrdinaryDerivedBasePass,
    context: CoeffOrdinaryContextCommitConfig,
) -> Result<(), CoeffOrdinaryPassError> {
    commit_nonzero_coeff_context(state, context, pass.quant_state())?;
    Ok(())
}

struct InterleavedSignQuantPassInput<'a> {
    block: &'a mut TransformCoeffBlockState,
    walk: &'a NonZeroCoeffScanWalk,
    sign_inputs: InterleavedSignInputs<'a>,
    max_level_config: CoeffQuantPassMaxLevelConfig,
    config: CoeffQuantPassConfig,
}

#[derive(Clone, Copy)]
enum InterleavedSignInputs<'a> {
    Explicit(&'a [CoeffSignReadInput]),
    Derived(CoeffSignSourceDeriveConfig<'a>),
}

fn apply_interleaved_sign_and_quant_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: InterleavedSignQuantPassInput<'_>,
) -> Result<NonZeroCoeffQuantState, CoeffOrdinaryPassError> {
    let InterleavedSignQuantPassInput {
        block,
        walk,
        sign_inputs,
        max_level_config,
        config,
    } = input;
    validate_coeff_quant_pass_config(config)?;

    let entries = walk.entries();
    let max_level_config = CoeffMaxLevelConfig {
        plane: max_level_config.plane,
        tx_class: max_level_config.tx_class,
        is_hidden: config.is_hidden,
    };
    for (index, entry) in entries.iter().copied().enumerate() {
        let max_level = derive_coeff_max_level(entry, max_level_config);
        max_level
            .max_level
            .checked_sub(u32::from(config.use_tcq))
            .ok_or(CoeffQuantPassError::InvalidMaxLevel {
                index,
                max_level: max_level.max_level,
                use_tcq: config.use_tcq,
            })?;
    }

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

    for (index, entry) in entries.iter().copied().enumerate() {
        let level = block
            .level_at(entry.row(), entry.col())
            .map_err(CoeffSignReadError::from)?;
        let sign_input = match sign_inputs {
            InterleavedSignInputs::Explicit(inputs) => {
                inputs
                    .get(index)
                    .copied()
                    .ok_or(CoeffSignReadError::InputCountMismatch {
                        inputs: inputs.len(),
                        entries: entries.len(),
                    })?
            }
            InterleavedSignInputs::Derived(config) => {
                derive_nonzero_coeff_sign_input(entry, level, config)
            }
        };
        let max_level = derive_coeff_max_level(entry, max_level_config);
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
        apply_nonzero_coeff_quant_state_step(
            block,
            &mut quant_state,
            index,
            entry,
            sign,
            read_quant.quant_input(),
        )
        .map_err(CoeffQuantPassError::from)?;
    }

    Ok(NonZeroCoeffQuantState::from_interleaved_parts(
        Vec::new(),
        quant_state,
    ))
}
