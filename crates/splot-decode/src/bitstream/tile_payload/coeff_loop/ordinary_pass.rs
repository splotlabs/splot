// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Ordinary non-FSC coefficient pass composition.

use splot_core::symbol::SymbolDecoder;

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::{
    LEVEL_INDEX_MASK, LEVEL_STRIDE, TileCoeffContextState, TileCoeffStateError,
    TransformCoeffBlockState,
};
use super::base_level_pass::{
    CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError, CoeffLevelMasks,
    apply_nonzero_coeff_base_derived_level_pass,
};
use super::branch::NonZeroCoeffBlockStart;
use super::max_level::{CoeffMaxLevelConfig, CoeffTransformClass, derive_coeff_max_level};
use super::quant_pass::{CoeffQuantPassError, validate_coeff_quant_pass_config};
use super::quant_state::{
    CoeffQuantStateAccumulator, CoeffQuantStateConfig, NonZeroCoeffQuantState,
    apply_derived_nonzero_coeff_quant_state_step,
};
use super::read_quant::{CoeffReadQuantConfig, CoeffReadQuantInput, CoeffReadQuantState};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::sign_symbol::{
    CoeffSignReadError, CoeffSignReadSymbol, CoeffSignSourceDeriveConfig,
    derive_nonzero_coeff_sign_input, read_preflighted_nonzero_coeff_sign,
};
use super::{AllZeroCoeffBlockInput, CoeffLoopContextError, commit_nonzero_coeff_context};

pub(crate) mod geometry;

struct CoeffOrdinaryDerivedBasePassInput<'a> {
    start: NonZeroCoeffBlockStart,
    scan: &'a [u16],
    base_config: CoeffBaseDerivedLevelPassConfig,
    sign_config: CoeffOrdinaryDerivedSignPassConfig<'a>,
    lossless: bool,
}

pub(crate) struct CoeffOrdinaryStateContextPassInput<'a> {
    pub(crate) start: NonZeroCoeffBlockStart,
    pub(crate) scan: &'a [u16],
    pub(crate) base_config: CoeffBaseDerivedLevelPassConfig,
    pub(crate) state_context: CoeffOrdinaryStateContextConfig,
    pub(crate) lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CoeffOrdinaryDerivedSignPassConfig<'a> {
    plane_type: usize,
    above_dc: &'a [u8],
    left_dc: &'a [u8],
    x4: usize,
    y4: usize,
    w4: usize,
    h4: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffOrdinaryStateContextConfig {
    pub(crate) plane_type: usize,
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
    #[error("ordinary coefficient branch invalid scan shape {width}x{height}")]
    InvalidScanShape { width: usize, height: usize },
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
    #[error(
        "ordinary coefficient branch directional UVMode {uv_mode} angle_delta_uv {angle_delta_uv} overflowed"
    )]
    DirectionalAngleOverflow { uv_mode: usize, angle_delta_uv: i32 },
    #[error("ordinary coefficient branch scan derivation failed: {0}")]
    ScanOrder(#[source] splot_recon::ReconError),
    #[error("ordinary coefficient branch nonzero pass failed: {0}")]
    Ordinary(#[from] CoeffOrdinaryPassError),
}

fn apply_nonzero_coeff_ordinary_pass_with_derived_base(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryDerivedBasePassInput<'_>,
) -> Result<(TransformCoeffBlockState, NonZeroCoeffQuantState), CoeffOrdinaryPassError> {
    let base_config = input.base_config;
    let sign_config = input.sign_config;
    let walk = walk_nonzero_coeff_scan(&input.start, input.scan)?;
    let mut block = TransformCoeffBlockState::empty();
    let mut masks = CoeffLevelMasks::default();
    let first_pass = apply_nonzero_coeff_base_derived_level_pass(
        cdfs,
        symbols,
        input.start,
        &walk,
        base_config,
        &mut block,
        &mut masks,
    )?;
    let sign_derive_config = CoeffSignSourceDeriveConfig {
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
    let quant_config = CoeffQuantStateConfig {
        is_hidden: first_pass.is_hidden(),
        sum_abs1: first_pass.sum_abs1(),
        use_tcq: base_config.use_tcq,
        lossless: input.lossless,
    };
    let quant_state = apply_interleaved_sign_and_quant_pass(
        cdfs,
        symbols,
        InterleavedSignQuantPassInput {
            block: &mut block,
            walk: &walk,
            masks: &masks,
            sign_config: sign_derive_config,
            max_level_config: CoeffMaxLevelConfig {
                plane: base_config.plane,
                tx_class: base_config.tx_class,
                is_hidden: first_pass.is_hidden(),
            },
            config: quant_config,
        },
    )?;
    Ok((block, quant_state))
}

pub(crate) fn apply_nonzero_coeff_ordinary_pass_with_state_context(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffOrdinaryStateContextPassInput<'_>,
) -> Result<TransformCoeffBlockState, CoeffOrdinaryPassError> {
    let CoeffOrdinaryStateContextPassInput {
        start,
        scan,
        base_config,
        state_context,
        lossless,
    } = input;
    let plane = base_config.plane;
    let local_x4 = state.local_x4(plane, state_context.x4)?;
    let local_y4 = state.local_y4(plane, state_context.y4)?;
    let (block, quant_state) = apply_nonzero_coeff_ordinary_pass_with_derived_base(
        cdfs,
        symbols,
        CoeffOrdinaryDerivedBasePassInput {
            start,
            scan,
            base_config,
            sign_config: CoeffOrdinaryDerivedSignPassConfig {
                plane_type: state_context.plane_type,
                above_dc: state.above_dc(plane)?,
                left_dc: state.left_dc(plane)?,
                x4: local_x4,
                y4: local_y4,
                w4: state_context.w4,
                h4: state_context.h4,
            },
            lossless,
        },
    )?;
    commit_nonzero_coeff_context(
        state,
        AllZeroCoeffBlockInput {
            plane,
            x4: state_context.x4,
            y4: state_context.y4,
            w4: state_context.w4,
            h4: state_context.h4,
        },
        &quant_state,
    )?;
    Ok(block)
}

struct InterleavedSignQuantPassInput<'a> {
    block: &'a mut TransformCoeffBlockState,
    walk: &'a NonZeroCoeffScanWalk<'a>,
    masks: &'a CoeffLevelMasks,
    sign_config: CoeffSignSourceDeriveConfig<'a>,
    max_level_config: CoeffMaxLevelConfig,
    config: CoeffQuantStateConfig,
}

/// Reads each nonzero coefficient's sign and remainder in scan order. A
/// zero level (outside the hidden-parity DC) has no sign, no remainder and
/// a quant of 0, which the block already holds, so the pass skips it and
/// takes the TCQ state of each entry it visits from the base pass. Scan
/// index 0, the only one the hidden parity touches, is read last and apart
/// from the loop over the others, which is specialised per transform class
/// and plane type.
fn apply_interleaved_sign_and_quant_pass(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: InterleavedSignQuantPassInput<'_>,
) -> Result<NonZeroCoeffQuantState, CoeffOrdinaryPassError> {
    use CoeffTransformClass::{Horizontal, TwoD, Vertical};
    let config = input.config;
    validate_coeff_quant_pass_config(config)?;

    let mut pass = SignQuantPass {
        read_quant_state: CoeffReadQuantState::new(CoeffReadQuantConfig {
            is_hidden: config.is_hidden,
            allow_tcq: config.use_tcq,
            hr_level_avg: 0,
        }),
        quant_state: CoeffQuantStateAccumulator::new(config),
        input,
    };
    match (
        pass.input.max_level_config.tx_class,
        pass.input.max_level_config.plane == 0,
    ) {
        (TwoD, true) => pass.ac_entries::<0, true>(cdfs, symbols)?,
        (TwoD, false) => pass.ac_entries::<0, false>(cdfs, symbols)?,
        (Horizontal, true) => pass.ac_entries::<1, true>(cdfs, symbols)?,
        (Horizontal, false) => pass.ac_entries::<1, false>(cdfs, symbols)?,
        (Vertical, true) => pass.ac_entries::<2, true>(cdfs, symbols)?,
        (Vertical, false) => pass.ac_entries::<2, false>(cdfs, symbols)?,
    }
    if config.is_hidden || pass.input.masks.is_nonzero(0) {
        let (sign_config, max_level_config) = (pass.input.sign_config, pass.input.max_level_config);
        pass.entry(cdfs, symbols, 0, sign_config, max_level_config)?;
    }
    Ok(NonZeroCoeffQuantState::from_accumulator(pass.quant_state))
}

struct SignQuantPass<'a> {
    read_quant_state: CoeffReadQuantState,
    quant_state: CoeffQuantStateAccumulator,
    input: InterleavedSignQuantPassInput<'a>,
}

impl SignQuantPass<'_> {
    /// The nonzero entries above scan index 0, with the class and plane
    /// fixed so their context branches fold away.
    #[inline(never)]
    fn ac_entries<const CLASS: usize, const LUMA: bool>(
        &mut self,
        cdfs: &mut TileCdfSubset,
        symbols: &mut SymbolDecoder<'_>,
    ) -> Result<(), CoeffOrdinaryPassError> {
        let tx_class = [
            CoeffTransformClass::TwoD,
            CoeffTransformClass::Horizontal,
            CoeffTransformClass::Vertical,
        ][CLASS];
        let plane = if LUMA {
            0
        } else {
            self.input.max_level_config.plane.max(1)
        };
        let sign_config = CoeffSignSourceDeriveConfig {
            plane,
            tx_class,
            ..self.input.sign_config
        };
        let max_level_config = CoeffMaxLevelConfig {
            plane,
            tx_class,
            ..self.input.max_level_config
        };
        let masks = self.input.masks;
        let walk = self.input.walk;
        let eob = walk.len();
        let (grid, quant, quant_end) = self.input.block.level_and_quant_mut();
        for scan_index in masks.nonzero_scan_indices(eob, false) {
            let Some(index) = eob.checked_sub(scan_index + 1).filter(|_| scan_index > 0) else {
                break;
            };
            let entry = walk.entry(index);
            let level =
                u32::from(grid[(entry.row() * LEVEL_STRIDE + entry.col()) & LEVEL_INDEX_MASK]);
            let axis_cdf = LUMA
                && match CLASS {
                    1 => entry.col() == 0,
                    2 => entry.row() == 0,
                    _ => false,
                };
            let sign = if axis_cdf {
                let input = derive_nonzero_coeff_sign_input(entry, level, sign_config);
                read_preflighted_nonzero_coeff_sign(cdfs, symbols, input)?.sign()
            } else {
                symbols
                    .read_literal(1)
                    .map_err(|source| CoeffSignReadError::LiteralRead { source })?
                    != 0
            };
            let max_level = derive_coeff_max_level(entry, max_level_config);
            let read = self
                .read_quant_state
                .read_one(
                    symbols,
                    index,
                    CoeffReadQuantInput {
                        entry,
                        level,
                        max_level,
                    },
                )
                .map_err(CoeffQuantPassError::from)?;
            let magnitude = self
                .quant_state
                .ac_magnitude(index, read.quant, masks.tcq_q0(scan_index))
                .map_err(CoeffQuantPassError::from)? as i32;
            if let Some(slot) = quant.get_mut(entry.pos()) {
                *slot = if sign { -magnitude } else { magnitude };
                if magnitude != 0 {
                    *quant_end = (*quant_end).max(entry.pos() + 1);
                }
            }
        }
        Ok(())
    }

    #[allow(
        clippy::inline_always,
        reason = "measured: one copy per specialised loop"
    )]
    #[inline(always)]
    fn entry(
        &mut self,
        cdfs: &mut TileCdfSubset,
        symbols: &mut SymbolDecoder<'_>,
        scan_index: usize,
        sign_config: CoeffSignSourceDeriveConfig<'_>,
        max_level_config: CoeffMaxLevelConfig,
    ) -> Result<(), CoeffOrdinaryPassError> {
        let config = self.input.config;
        let walk = self.input.walk;
        let Some(index) = walk.len().checked_sub(scan_index + 1) else {
            return Err(CoeffLoopContextError::ScanWalkEobOutOfRange {
                eob: scan_index + 1,
                scan_len: walk.len(),
            }
            .into());
        };
        let entry = walk.entry(index);
        let level = self.input.block.level_at(entry.row(), entry.col())?;
        let sign_input = derive_nonzero_coeff_sign_input(entry, level, sign_config);
        let max_level = derive_coeff_max_level(entry, max_level_config);
        let sign = read_preflighted_nonzero_coeff_sign(cdfs, symbols, sign_input)?;
        if config.is_hidden
            && config.sum_abs1 > 0
            && entry.scan_index() == 0
            && sign.symbol() == CoeffSignReadSymbol::None
        {
            return Err(CoeffQuantPassError::HiddenParityMissingSign { index, entry }.into());
        }
        let read_quant = self
            .read_quant_state
            .read_one(
                symbols,
                index,
                CoeffReadQuantInput {
                    entry,
                    level,
                    max_level,
                },
            )
            .map_err(CoeffQuantPassError::from)?;
        self.quant_state
            .set_tcq_q0(self.input.masks.tcq_q0(scan_index));
        apply_derived_nonzero_coeff_quant_state_step(
            self.input.block,
            &mut self.quant_state,
            index,
            entry,
            sign,
            read_quant,
        )
        .map_err(CoeffQuantPassError::from)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "ordinary_pass_tests.rs"]
mod tests;
