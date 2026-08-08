// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient branch selector for derived or caller-resolved `useFsc`.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::TileCoeffContextState;
use super::fsc_quant_pass::{
    CoeffFscBranch, CoeffFscBranchError, CoeffFscBranchTxSizeInput,
    CoeffFscBranchTxSizeNonZeroInput, apply_coeff_fsc_branch_from_tx_size, tx_size_table_usize,
};
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryBranchLosslessInput,
    CoeffOrdinaryBranchLosslessNonZeroInput, CoeffOrdinaryTxSizeGeometryConfig,
    apply_coeff_ordinary_branch_from_lossless,
};
use super::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};
use super::{AllZeroCoeffBlockInput, CoeffBranchInput};

const IDTX: usize = 9;
pub(crate) type CoeffUseFscBranchInput =
    CoeffBranchInput<CoeffOrdinaryTxSizeGeometryConfig, CoeffUseFscBranchNonZeroInput>;
pub(crate) struct CoeffUseFscBranchNonZeroInput {
    pub(crate) use_fsc: bool,
    pub(crate) ordinary: CoeffOrdinaryBranchLosslessNonZeroInput,
    pub(crate) fsc: CoeffFscBranchTxSizeNonZeroInput,
}
pub(crate) type CoeffUseFscConditionInput =
    CoeffBranchInput<CoeffOrdinaryTxSizeGeometryConfig, CoeffUseFscConditionNonZeroInput>;
pub(crate) struct CoeffUseFscConditionNonZeroInput {
    pub(crate) condition: CoeffUseFscConditionFacts,
    pub(crate) ordinary: CoeffOrdinaryBranchLosslessNonZeroInput,
    pub(crate) fsc: CoeffFscBranchTxSizeNonZeroInput,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffUseFscConditionFacts {
    pub(crate) enable_fsc: bool,
    pub(crate) plane_tx_type: usize,
    pub(crate) plane: usize,
    pub(crate) fsc_mode: bool,
    pub(crate) is_inter: bool,
}

impl CoeffUseFscConditionFacts {
    const fn use_fsc(self) -> bool {
        self.enable_fsc
            && self.plane_tx_type == IDTX
            && self.plane == 0
            && (self.fsc_mode || self.is_inter)
    }
}
pub(crate) type CoeffUseFscSharedFactsInput =
    CoeffBranchInput<CoeffOrdinaryTxSizeGeometryConfig, CoeffUseFscSharedFactsNonZeroInput>;
pub(crate) struct CoeffUseFscSharedFactsNonZeroInput {
    pub(crate) facts: CoeffUseFscSharedFacts,
    pub(crate) ordinary_base_config: CoeffOrdinaryBranchLosslessBaseConfig,
    pub(crate) lossless: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffUseFscSharedFacts {
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    pub(crate) enable_fsc: bool,
    pub(crate) plane_tx_type: usize,
    pub(crate) fsc_mode: bool,
    pub(crate) is_inter: bool,
    pub(crate) coeff_cdf_q_ctx: usize,
}

impl CoeffUseFscSharedFacts {
    const fn condition(self) -> CoeffUseFscConditionFacts {
        CoeffUseFscConditionFacts {
            enable_fsc: self.enable_fsc,
            plane_tx_type: self.plane_tx_type,
            plane: self.geometry.plane,
            fsc_mode: self.fsc_mode,
            is_inter: self.is_inter,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "crate-private selector preserves existing branch result types without boxing"
)]
pub(crate) enum CoeffUseFscBranch {
    Ordinary(CoeffOrdinaryBranch),
    Fsc(CoeffFscBranch),
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffUseFscBranchError {
    #[error("coefficient useFsc ordinary branch failed: {0}")]
    Ordinary(#[from] CoeffOrdinaryBranchError),
    #[error("coefficient useFsc FSC branch failed: {0}")]
    Fsc(#[from] CoeffFscBranchError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoeffCdfQContext {
    Q0,
    Q1,
    Q2,
    Q3,
}

impl CoeffCdfQContext {
    pub(crate) const fn from_base_q_idx(base_q_idx: u32) -> Self {
        match base_q_idx {
            0..=90 => Self::Q0,
            91..=140 => Self::Q1,
            141..=190 => Self::Q2,
            _ => Self::Q3,
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Q0 => 0,
            Self::Q1 => 1,
            Self::Q2 => 2,
            Self::Q3 => 3,
        }
    }
}

pub(crate) const fn coeff_cdf_q_ctx_from_base_q_idx(base_q_idx: u32) -> usize {
    CoeffCdfQContext::from_base_q_idx(base_q_idx).index()
}

pub(crate) fn apply_coeff_use_fsc_branch(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffUseFscBranchInput,
) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError> {
    match input {
        CoeffUseFscBranchInput::AllZero(input) => Ok(CoeffUseFscBranch::Ordinary(
            apply_coeff_ordinary_branch_from_lossless(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchLosslessInput::AllZero(input),
            )?,
        )),
        CoeffUseFscBranchInput::NonZero(input) if input.use_fsc => {
            Ok(CoeffUseFscBranch::Fsc(apply_coeff_fsc_branch_from_tx_size(
                state,
                cdfs,
                symbols,
                CoeffFscBranchTxSizeInput::NonZero(input.fsc),
            )?))
        }
        CoeffUseFscBranchInput::NonZero(input) => Ok(CoeffUseFscBranch::Ordinary(
            apply_coeff_ordinary_branch_from_lossless(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchLosslessInput::NonZero(input.ordinary),
            )?,
        )),
    }
}

coeff_branch_map_adapter!(
    pub(crate) fn apply_coeff_use_fsc_branch_from_condition(
        CoeffUseFscConditionInput
    ) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError>,
    input,
    CoeffUseFscBranchNonZeroInput {
        use_fsc: input.condition.use_fsc(),
        ordinary: input.ordinary,
        fsc: input.fsc,
    },
    apply_coeff_use_fsc_branch,
);

pub(crate) fn apply_coeff_use_fsc_branch_from_shared_facts(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffUseFscSharedFactsInput,
) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError> {
    match input {
        CoeffUseFscSharedFactsInput::AllZero(input) => {
            apply_coeff_use_fsc_branch(state, cdfs, symbols, CoeffUseFscBranchInput::AllZero(input))
        }
        CoeffUseFscSharedFactsInput::NonZero(input) if input.facts.condition().use_fsc() => {
            let facts = input.facts;
            let block = fsc_block_from_tx_size_geometry(facts.geometry)?;
            Ok(CoeffUseFscBranch::Fsc(apply_coeff_fsc_branch_from_tx_size(
                state,
                cdfs,
                symbols,
                CoeffFscBranchTxSizeInput::NonZero(CoeffFscBranchTxSizeNonZeroInput {
                    block,
                    tx_size: facts.geometry.tx_size,
                    plane_tx_type: facts.plane_tx_type,
                    is_inter: facts.is_inter,
                    coeff_cdf_q_ctx: facts.coeff_cdf_q_ctx,
                }),
            )?))
        }
        CoeffUseFscSharedFactsInput::NonZero(input) => {
            let facts = input.facts;
            Ok(CoeffUseFscBranch::Ordinary(
                apply_coeff_ordinary_branch_from_lossless(
                    state,
                    cdfs,
                    symbols,
                    CoeffOrdinaryBranchLosslessInput::NonZero(
                        CoeffOrdinaryBranchLosslessNonZeroInput {
                            geometry: facts.geometry,
                            coeff_cdf_q_ctx: facts.coeff_cdf_q_ctx,
                            is_inter: facts.is_inter,
                            base_config: input.ordinary_base_config,
                            lossless: input.lossless,
                        },
                    ),
                )?,
            ))
        }
    }
}

fn fsc_block_from_tx_size_geometry(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
) -> Result<AllZeroCoeffBlockInput, CoeffFscBranchError> {
    let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", geometry.tx_size)?;
    let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", geometry.tx_size)?;
    Ok(AllZeroCoeffBlockInput {
        plane: geometry.plane,
        x4: geometry.start_x >> 2,
        y4: geometry.start_y >> 2,
        w4: tx_width >> 2,
        h4: tx_height >> 2,
    })
}
