// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Coefficient branch selector for derived or caller-resolved `useFsc`.
//!
//! Feature tracking: `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF` and
//! `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF` and
//! `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` and
//! `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` and
//! `DECODE-COEFF-FRAME-FACTS-HANDOFF` and
//! `DECODE-COEFF-PARITY-TCQ-HANDOFF`.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{TX_HEIGHT, TX_WIDTH};

use super::super::TileCoeffFrameFacts;
use super::super::cdf::TileCdfSubset;
use super::super::coeff_state::TileCoeffContextState;
use super::AllZeroCoeffBlockInput;
use super::fsc_quant_pass::{
    CoeffFscBranch, CoeffFscBranchError, CoeffFscBranchTxSizeInput,
    CoeffFscBranchTxSizeNonZeroInput, apply_coeff_fsc_branch_from_tx_size,
};
use super::max_level::CoeffTransformClass;
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryBranchLosslessInput,
    CoeffOrdinaryBranchLosslessNonZeroInput, CoeffOrdinaryTxSizeGeometryConfig,
    apply_coeff_ordinary_branch_from_lossless,
};
use super::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};

// AV2 § 3 `IDTX` transform type value.
const IDTX: usize = 9;

/// Caller-selected coefficient branch before the AV2 `useFsc` split.
pub(crate) enum CoeffUseFscBranchInput {
    /// Decoded `all_zero == 1`; AV2 handles this before deriving `useFsc`.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffUseFscBranchNonZeroInput),
}

/// Caller-resolved facts for the nonzero `useFsc` branch selector.
pub(crate) struct CoeffUseFscBranchNonZeroInput {
    /// Caller-resolved AV2 § 5.20.7.27 `useFsc` branch condition.
    pub(crate) use_fsc: bool,
    /// Lower-boundary input for the ordinary non-FSC branch.
    pub(crate) ordinary: CoeffOrdinaryBranchLosslessNonZeroInput,
    /// Lower-boundary input for the FSC/IDTX branch.
    pub(crate) fsc: CoeffFscBranchTxSizeNonZeroInput,
}

/// Caller-selected coefficient branch before deriving the AV2 `useFsc` condition.
pub(crate) enum CoeffUseFscConditionInput {
    /// Decoded `all_zero == 1`; AV2 handles this before deriving `useFsc`.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffUseFscConditionNonZeroInput),
}

/// Caller-resolved facts before deriving the nonzero `useFsc` condition.
pub(crate) struct CoeffUseFscConditionNonZeroInput {
    /// Caller-resolved facts named by the AV2 § 5.20.7.27 `useFsc` expression.
    pub(crate) condition: CoeffUseFscConditionFacts,
    /// Lower-boundary input for the ordinary non-FSC branch.
    pub(crate) ordinary: CoeffOrdinaryBranchLosslessNonZeroInput,
    /// Lower-boundary input for the FSC/IDTX branch.
    pub(crate) fsc: CoeffFscBranchTxSizeNonZeroInput,
}

/// Caller-resolved facts named by the AV2 § 5.20.7.27 `useFsc` expression.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffUseFscConditionFacts {
    /// Caller-resolved `enable_fsc` sequence/frame fact.
    pub(crate) enable_fsc: bool,
    /// Caller-resolved `PlaneTxType` from AV2 § 5.20.7.29 `compute_tx_type`.
    pub(crate) plane_tx_type: usize,
    /// Coefficient plane index; luma is 0.
    pub(crate) plane: usize,
    /// Caller-resolved `fsc_mode` fact.
    pub(crate) fsc_mode: bool,
    /// Caller-resolved `is_inter` fact.
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

/// Caller-selected coefficient branch before deriving `useFsc` from shared facts.
pub(crate) enum CoeffUseFscSharedFactsInput {
    /// Decoded `all_zero == 1`; AV2 handles this before deriving `useFsc`.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffUseFscSharedFactsNonZeroInput),
}

/// Shared caller-resolved facts for a nonzero coefficient block.
pub(crate) struct CoeffUseFscSharedFactsNonZeroInput {
    /// Facts shared by both nonzero branch targets.
    pub(crate) facts: CoeffUseFscSharedFacts,
    /// Ordinary-only facts used only when `useFsc == false`.
    pub(crate) ordinary_base_config: CoeffOrdinaryBranchLosslessBaseConfig,
    /// Caller-resolved AV2 § 5.20.7.29 `Lossless` flag for the ordinary path.
    pub(crate) lossless: bool,
}

/// Shared caller-resolved facts available before the `useFsc` branch split.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffUseFscSharedFacts {
    /// Caller-resolved AV2 § 5.20.7.27 `coeffs()` geometry.
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    /// Caller-resolved `enable_fsc` sequence/frame fact.
    pub(crate) enable_fsc: bool,
    /// Caller-resolved `PlaneTxType` from AV2 § 5.20.7.29 `compute_tx_type`.
    pub(crate) plane_tx_type: usize,
    /// Caller-resolved `fsc_mode` fact.
    pub(crate) fsc_mode: bool,
    /// Caller-resolved `is_inter` fact.
    pub(crate) is_inter: bool,
    /// Coefficient-CDF quantization context.
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

/// Caller-selected coefficient branch before deriving q-context from `base_q_idx`.
pub(crate) enum CoeffUseFscBaseQFactsInput {
    /// Decoded `all_zero == 1`; AV2 handles this before deriving q-context.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffUseFscBaseQFactsNonZeroInput),
}

/// Shared caller-resolved facts for a nonzero coefficient block carrying base q.
pub(crate) struct CoeffUseFscBaseQFactsNonZeroInput {
    /// Facts shared by both nonzero branch targets.
    pub(crate) facts: CoeffUseFscBaseQFacts,
    /// Ordinary-only facts used only when `useFsc == false`.
    pub(crate) ordinary_base_config: CoeffOrdinaryBranchLosslessBaseConfig,
    /// Caller-resolved AV2 § 5.20.7.29 `Lossless` flag for the ordinary path.
    pub(crate) lossless: bool,
}

/// Shared caller-resolved facts before q-context and `useFsc` derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffUseFscBaseQFacts {
    /// Caller-resolved AV2 § 5.20.7.27 `coeffs()` geometry.
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    /// Caller-resolved `enable_fsc` sequence/frame fact.
    pub(crate) enable_fsc: bool,
    /// Caller-resolved `PlaneTxType` from AV2 § 5.20.7.29 `compute_tx_type`.
    pub(crate) plane_tx_type: usize,
    /// Caller-resolved `fsc_mode` fact.
    pub(crate) fsc_mode: bool,
    /// Caller-resolved `is_inter` fact.
    pub(crate) is_inter: bool,
    /// Frame `base_q_idx` used by AV2 § 6.17.2 `init_coeff_cdfs()`.
    pub(crate) base_q_idx: u32,
}

impl CoeffUseFscBaseQFacts {
    const fn shared_facts(self) -> CoeffUseFscSharedFacts {
        CoeffUseFscSharedFacts {
            geometry: self.geometry,
            enable_fsc: self.enable_fsc,
            plane_tx_type: self.plane_tx_type,
            fsc_mode: self.fsc_mode,
            is_inter: self.is_inter,
            coeff_cdf_q_ctx: coeff_cdf_q_ctx_from_base_q_idx(self.base_q_idx),
        }
    }
}

/// Caller-selected coefficient branch before deriving frame-scoped facts.
pub(crate) enum CoeffUseFscFrameFactsInput {
    /// Decoded `all_zero == 1`; AV2 handles this before frame-fact branch setup.
    AllZero(CoeffOrdinaryTxSizeGeometryConfig),
    /// Decoded `all_zero == 0`.
    NonZero(CoeffUseFscFrameFactsNonZeroInput),
}

/// Caller-resolved block facts before frame-fact derivation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffUseFscFrameBlockFacts {
    /// Caller-resolved AV2 § 5.20.7.27 `coeffs()` geometry.
    pub(crate) geometry: CoeffOrdinaryTxSizeGeometryConfig,
    /// Caller-resolved `PlaneTxType` from AV2 § 5.20.7.29 `compute_tx_type`.
    pub(crate) plane_tx_type: usize,
    /// Caller-resolved `fsc_mode` fact.
    pub(crate) fsc_mode: bool,
    /// Caller-resolved `is_inter` fact.
    pub(crate) is_inter: bool,
    /// Caller-resolved `SegmentId` for the block.
    pub(crate) segment_id: usize,
}

/// Caller-resolved ordinary-only facts that are not frame scoped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CoeffUseFscFrameOrdinaryFacts {
    /// Caller-resolved `UVMode` from the chroma intra mode syntax.
    pub(crate) uv_mode: usize,
    /// Caller-resolved `AngleDeltaUV` from chroma intra mode syntax.
    pub(crate) angle_delta_uv: i32,
    /// Caller-resolved luma `TxTypes[blockY][blockX]`.
    pub(crate) luma_tx_type: usize,
    /// Caller-resolved chroma-inter `TxTypes[y4][x4]`.
    pub(crate) chroma_inter_tx_type: usize,
}

/// Shared caller-resolved facts plus parsed frame facts for a nonzero block.
pub(crate) struct CoeffUseFscFrameFactsNonZeroInput {
    /// Parsed frame/sequence coefficient facts.
    pub(crate) frame: TileCoeffFrameFacts,
    /// Caller-resolved block facts.
    pub(crate) block: CoeffUseFscFrameBlockFacts,
    /// Ordinary-only facts still resolved by the caller.
    pub(crate) ordinary: CoeffUseFscFrameOrdinaryFacts,
}

impl CoeffUseFscFrameFactsNonZeroInput {
    pub(crate) fn base_q_input(
        self,
    ) -> Result<CoeffUseFscBaseQFactsNonZeroInput, CoeffUseFscBranchError> {
        let lossless = self
            .frame
            .lossless_for_segment(self.block.segment_id)
            .ok_or(CoeffUseFscBranchError::InvalidSegmentId {
                segment_id: self.block.segment_id,
            })?;
        let use_fsc = CoeffUseFscConditionFacts {
            enable_fsc: self.frame.enable_fsc(),
            plane_tx_type: self.block.plane_tx_type,
            plane: self.block.geometry.plane,
            fsc_mode: self.block.fsc_mode,
            is_inter: self.block.is_inter,
        }
        .use_fsc();
        let parity_hiding = self.frame.allow_parity_hiding()
            && !lossless
            && self.block.geometry.plane == 0
            && self.block.plane_tx_type != IDTX;
        let use_tcq = self.frame.allow_tcq()
            && self.block.geometry.plane == 0
            && !lossless
            && CoeffTransformClass::from_plane_tx_type(self.block.plane_tx_type)
                == CoeffTransformClass::TwoD
            && !use_fsc;
        Ok(CoeffUseFscBaseQFactsNonZeroInput {
            facts: CoeffUseFscBaseQFacts {
                geometry: self.block.geometry,
                enable_fsc: self.frame.enable_fsc(),
                plane_tx_type: self.block.plane_tx_type,
                fsc_mode: self.block.fsc_mode,
                is_inter: self.block.is_inter,
                base_q_idx: self.frame.base_q_idx(),
            },
            ordinary_base_config: CoeffOrdinaryBranchLosslessBaseConfig {
                reduced_tx_set: self.frame.reduced_tx_set(),
                enable_chroma_dctonly: self.frame.enable_chroma_dctonly(),
                uv_mode: self.ordinary.uv_mode,
                angle_delta_uv: self.ordinary.angle_delta_uv,
                luma_tx_type: self.ordinary.luma_tx_type,
                chroma_inter_tx_type: self.ordinary.chroma_inter_tx_type,
                parity_hiding,
                use_tcq,
            },
            lossless,
        })
    }
}

/// Result of the loaded `useFsc` branch selector.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "crate-private selector preserves existing branch result types without boxing"
)]
pub(crate) enum CoeffUseFscBranch {
    /// Ordinary non-FSC branch result.
    Ordinary(CoeffOrdinaryBranch),
    /// FSC/IDTX branch result.
    Fsc(CoeffFscBranch),
}

/// Error returned by the loaded `useFsc` branch selector.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CoeffUseFscBranchError {
    /// The caller supplied a `SegmentId` outside `LosslessArray[]`.
    #[error("coefficient frame-facts handoff received invalid segment_id {segment_id}")]
    InvalidSegmentId {
        /// Caller-provided segment id.
        segment_id: usize,
    },
    /// The ordinary branch rejected the selected input.
    #[error("coefficient useFsc ordinary branch failed: {0}")]
    Ordinary(#[from] CoeffOrdinaryBranchError),
    /// The FSC/IDTX branch rejected the selected input.
    #[error("coefficient useFsc FSC branch failed: {0}")]
    Fsc(#[from] CoeffFscBranchError),
}

/// Derives AV2 § 6.17.2 `init_coeff_cdfs()` q-context from `base_q_idx`.
///
/// The spec defines four selectable coefficient CDF q-contexts
/// (`COEFF_CDF_Q_CTXS = 4` in § 3) and derives the active `idx` from
/// `base_q_idx` using thresholds 90, 140, and 190
/// (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2`;
/// `docs/spec/av2/1.0.0/03-symbols.md`). The final bucket is total for
/// staged caller-provided values beyond the syntax domain.
pub(crate) const fn coeff_cdf_q_ctx_from_base_q_idx(base_q_idx: u32) -> usize {
    if base_q_idx <= 90 {
        0
    } else if base_q_idx <= 140 {
        1
    } else if base_q_idx <= 190 {
        2
    } else {
        3
    }
}

/// Dispatches the coefficient branch after caller-resolved `useFsc`.
///
/// AV2 § 5.20.7.27 handles `all_zero` before deriving and testing `useFsc`.
/// This loaded-but-unwired selector preserves that ordering: all-zero inputs
/// always go through the ordinary all-zero branch, while nonzero inputs dispatch
/// to either the ordinary lossless handoff or the FSC tx-size handoff based on
/// caller-resolved `use_fsc`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). Runtime
/// `coeffs()` integration, full `compute_tx_type`, dequantization, inverse
/// transform, residual add, and reconstruction remain out of scope. This staged
/// boundary partitions the already-loaded branch targets; the full spec block
/// still has additional common work after the `if ( useFsc )` body.
pub(crate) fn apply_coeff_use_fsc_branch(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffUseFscBranchInput,
) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError> {
    match input {
        CoeffUseFscBranchInput::AllZero(input) => {
            let branch = apply_coeff_ordinary_branch_from_lossless(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchLosslessInput::AllZero(input),
            )?;
            Ok(CoeffUseFscBranch::Ordinary(branch))
        }
        CoeffUseFscBranchInput::NonZero(input) if input.use_fsc => {
            let branch = apply_coeff_fsc_branch_from_tx_size(
                state,
                cdfs,
                symbols,
                CoeffFscBranchTxSizeInput::NonZero(input.fsc),
            )?;
            Ok(CoeffUseFscBranch::Fsc(branch))
        }
        CoeffUseFscBranchInput::NonZero(input) => {
            let branch = apply_coeff_ordinary_branch_from_lossless(
                state,
                cdfs,
                symbols,
                CoeffOrdinaryBranchLosslessInput::NonZero(input.ordinary),
            )?;
            Ok(CoeffUseFscBranch::Ordinary(branch))
        }
    }
}

/// Dispatches the coefficient branch after deriving the AV2 `useFsc` condition.
///
/// AV2 § 5.20.7.27 derives
/// `useFsc = enable_fsc && PlaneTxType == IDTX && plane == 0 && (fsc_mode || is_inter)`
/// only in the decoded nonzero branch after `PlaneTxType` is available
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). This
/// loaded-but-unwired handoff preserves the preceding all-zero ordering and
/// delegates the derived boolean to `apply_coeff_use_fsc_branch`. Runtime
/// `coeffs()` integration, full `compute_tx_type`, and caller-to-runtime fact
/// derivation remain out of scope.
pub(crate) fn apply_coeff_use_fsc_branch_from_condition(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffUseFscConditionInput,
) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError> {
    let input = match input {
        CoeffUseFscConditionInput::AllZero(input) => CoeffUseFscBranchInput::AllZero(input),
        CoeffUseFscConditionInput::NonZero(input) => {
            CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
                use_fsc: input.condition.use_fsc(),
                ordinary: input.ordinary,
                fsc: input.fsc,
            })
        }
    };
    apply_coeff_use_fsc_branch(state, cdfs, symbols, input)
}

/// Dispatches the coefficient branch after deriving `useFsc` from shared facts.
///
/// AV2 § 5.20.7.27 computes `Tx_Width[txSz]` and `Tx_Height[txSz]`, handles
/// `all_zero`, then derives and tests
/// `useFsc = enable_fsc && PlaneTxType == IDTX && plane == 0 && (fsc_mode || is_inter)`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`;
/// `docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md`).
/// This loaded-but-unwired handoff accepts one shared nonzero fact packet and
/// constructs only the selected lower branch input. Runtime `coeffs()`
/// integration, full `compute_tx_type`, scan derivation, dequantization,
/// inverse transform, residual add, and reconstruction remain out of scope.
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
            let branch = apply_coeff_fsc_branch_from_tx_size(
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
            )?;
            Ok(CoeffUseFscBranch::Fsc(branch))
        }
        CoeffUseFscSharedFactsInput::NonZero(input) => {
            let facts = input.facts;
            let branch = apply_coeff_ordinary_branch_from_lossless(
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
            )?;
            Ok(CoeffUseFscBranch::Ordinary(branch))
        }
    }
}

/// Dispatches after deriving q-context from frame `base_q_idx`.
///
/// AV2 § 6.17.2 `init_coeff_cdfs()` derives the active coefficient CDF
/// q-context from `base_q_idx`, and AV2 § 5.20.7.27 then uses that context for
/// coefficient syntax CDF selection
/// (`docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2`;
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`). This
/// loaded-but-unwired handoff keeps all-zero inputs independent of frame q
/// facts, derives q-context only for nonzero inputs, and delegates to the
/// existing shared-facts wrapper. Runtime `coeffs()` integration, full CDF
/// lifecycle wiring, full `compute_tx_type`, dequantization, inverse transform,
/// residual add, and reconstruction remain out of scope.
pub(crate) fn apply_coeff_use_fsc_branch_from_base_q_facts(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffUseFscBaseQFactsInput,
) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError> {
    let input = match input {
        CoeffUseFscBaseQFactsInput::AllZero(input) => CoeffUseFscSharedFactsInput::AllZero(input),
        CoeffUseFscBaseQFactsInput::NonZero(input) => {
            CoeffUseFscSharedFactsInput::NonZero(CoeffUseFscSharedFactsNonZeroInput {
                facts: input.facts.shared_facts(),
                ordinary_base_config: input.ordinary_base_config,
                lossless: input.lossless,
            })
        }
    };
    apply_coeff_use_fsc_branch_from_shared_facts(state, cdfs, symbols, input)
}

/// Dispatches after deriving parsed frame and sequence facts.
///
/// AV2 § 5.4.8 / § 6.4.8 define `enable_fsc` and
/// `enable_chroma_dctonly`, AV2 § 5.18.2 derives `LosslessArray[segmentId]`
/// and reads `reduced_tx_set`, and AV2 § 6.17.2 derives the coefficient CDF
/// q-context from `base_q_idx`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-8`;
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-8`;
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`;
/// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2`).
/// This loaded-but-unwired handoff converts those parsed frame facts into the
/// existing base-q wrapper input while keeping all-zero inputs independent of
/// frame facts. Runtime `coeffs()` integration, full `compute_tx_type`,
/// block-syntax traversal, dequantization, inverse transform, residual add, and
/// reconstruction remain out of scope.
pub(crate) fn apply_coeff_use_fsc_branch_from_frame_facts(
    state: &mut TileCoeffContextState,
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: CoeffUseFscFrameFactsInput,
) -> Result<CoeffUseFscBranch, CoeffUseFscBranchError> {
    let input = match input {
        CoeffUseFscFrameFactsInput::AllZero(input) => CoeffUseFscBaseQFactsInput::AllZero(input),
        CoeffUseFscFrameFactsInput::NonZero(input) => {
            CoeffUseFscBaseQFactsInput::NonZero(input.base_q_input()?)
        }
    };
    apply_coeff_use_fsc_branch_from_base_q_facts(state, cdfs, symbols, input)
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

fn tx_size_table_usize(
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<usize, CoeffFscBranchError> {
    let value = table
        .get(tx_size)
        .copied()
        .ok_or(CoeffFscBranchError::InvalidTransformSize { tx_size })?;
    usize::try_from(value).map_err(|_| CoeffFscBranchError::InvalidTransformSizeTableValue {
        table: table_name,
        tx_size,
        value,
    })
}
