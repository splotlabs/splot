// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! General intra luma transform-block coefficient decode for the AVM-oracle
//! general intra path.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-LUMA-COEFFS`.
//!
//! This decodes the AV2 § 5.20.7.27 `coeffs()` syntax for the single luma
//! transform block of a minimal-tool intra key frame: it reads the `all_zero`
//! (`txb_skip`) symbol with the spec-derived § 8.3.2 context, and when
//! `all_zero == 0` routes the nonzero coefficient pass through the existing
//! coefficient-loop machinery to produce the decoded `Quant[]` and end-of-block.
//!
//! Scope: the single non-partitioned 64x64 luma transform block at the tile
//! origin (`tx_size == TX_64X64`, `PlaneTxType == DCT_DCT`). The chroma
//! transform blocks, dequantization, inverse transform, residual addition,
//! reconstruction, output, the general transform-block partition walk, and tile
//! context-line persistence remain future increments. The decoded coefficients
//! are returned to the caller (which currently still reports the reconstruction
//! step unsupported) rather than committed to runtime decode state.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{
    MD_IDX_TO_TYPE, MODE_TO_ANGLE, SIZE_CLASS, TX_HEIGHT, TX_HEIGHT_LOG2, TX_SIZE_SQR,
    TX_SIZE_SQR_UP, TX_WIDTH, TX_WIDTH_LOG2,
};
use splot_recon::{
    BitDepth, DequantBlockParams, InverseTransform2dOuter, PlaneId, QuantizerDeltas, ReconError,
    ReconSample, ac_quantizer, dc_quantizer, reconstruct_transform_block_residual,
};

use super::cdf::TileCdfSelector;
use super::cdf::block_context::IntraYMode;
use super::cdf::block_context::{txb_skip_ctx_luma, v_txb_skip_ctx};
use super::cdf::block_read::BlockSymbolTraceReadError;
use super::coeff_loop::fsc_quant_pass::{
    CoeffFscBranchError, CoeffFscStagedTxSizeNonZeroInput,
    apply_staged_nonzero_coeff_fsc_branch_from_tx_size,
};
use super::coeff_loop::max_level::CoeffTransformClass;
use super::coeff_loop::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryStagedLosslessNonZeroInput,
    CoeffOrdinaryTxSizeGeometryConfig, apply_staged_nonzero_coeff_ordinary_branch_from_lossless,
};
use super::coeff_loop::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};
use super::coeff_loop::use_fsc_branch::{
    CoeffUseFscBranch, CoeffUseFscBranchError, CoeffUseFscFrameBlockFacts,
    CoeffUseFscFrameFactsInput, CoeffUseFscFrameFactsNonZeroInput, CoeffUseFscFrameOrdinaryFacts,
    apply_coeff_use_fsc_branch_from_frame_facts, coeff_cdf_q_ctx_from_base_q_idx,
};
use super::coeff_loop::{
    AllZeroCoeffBlockInput, CoeffBlockEobBranch, CoeffBlockEobBranchInput, CoeffLoopContextError,
    NonZeroCoeffBlockStartInput, NonZeroCoeffEobContextInput, read_coeff_block_eob_branch,
};
use super::coeff_state::CoeffContextUpdate;
use super::coeff_state::{TileCoeffContextState, TileCoeffStateError};
use super::{DecodeTileWorkUnit, TileCdfSubset, TileCoeffFrameFacts};

/// AV2 § 9.2 `TX_SIZES_ALL` index of `TX_64X64` (the single non-partitioned
/// luma transform size for a 64x64 intra block with transform partitioning
/// disabled).
const TX_64X64: usize = 4;
/// AV2 § 9.2 `TX_SIZES_ALL` index of `TX_8X8`.
const TX_8X8: usize = 1;
/// AV2 § 9.2 `TX_SIZES_ALL` index of `TX_16X16`.
const TX_16X16: usize = 2;
/// AV2 § 9.2 square-transform ordinal for `TX_32X32`, used by
/// § 5.20.8.3 `get_tx_set`.
const TX_32X32: usize = 3;
/// AV2 § 9.2 `TX_SIZES_ALL` index of `TX_8X16`.
const TX_8X16: usize = 7;
/// AV2 § 3 `IST_4X4_HEIGHT`.
const IST_4X4_HEIGHT: usize = 8;
/// AV2 § 3 `IST_8X8_HEIGHT_RED`.
const IST_8X8_HEIGHT_RED: usize = 20;
/// AV2 § 3 `IST_8X8_HEIGHT`.
const IST_8X8_HEIGHT: usize = 32;
/// AV2 § 3 `ANGLE_STEP`.
const ANGLE_STEP: i32 = 3;
/// AV2 § 7.13.2.3 `Mrl_Index_To_Delta[MrlIndex]`.
const MRL_INDEX_TO_DELTA: [i32; 4] = [0, 1, -1, 0];
/// `DCT_DCT` `PlaneTxType` (AV2 § 5.20.7.29 implies `DCT_DCT` for this
/// minimal-tool intra block; no `intra_tx_type` symbol is coded).
const DCT_DCT: usize = 0;
/// AV2 § 3 `ADST_DCT`.
const ADST_DCT: usize = 1;
/// AV2 § 3 `DCT_ADST`.
const DCT_ADST: usize = 2;
/// AV2 § 3 `ADST_ADST`.
const ADST_ADST: usize = 3;
/// AV2 § 3 `FLIPADST_DCT`.
const FLIPADST_DCT: usize = 4;
/// AV2 § 3 `DCT_FLIPADST`.
const DCT_FLIPADST: usize = 5;
/// AV2 § 3 `FLIPADST_FLIPADST`.
const FLIPADST_FLIPADST: usize = 6;
/// AV2 § 3 `ADST_FLIPADST`.
const ADST_FLIPADST: usize = 7;
/// AV2 § 3 `FLIPADST_ADST`.
const FLIPADST_ADST: usize = 8;
/// AV2 § 3 `IDTX`.
const IDTX: usize = 9;
/// AV2 § 3 `V_DCT`.
const V_DCT: usize = 10;
/// AV2 § 3 `H_DCT`.
const H_DCT: usize = 11;
/// AV2 § 3 `V_ADST`.
const V_ADST: usize = 12;
/// AV2 § 3 `H_ADST`.
const H_ADST: usize = 13;
/// AV2 § 3 `V_FLIPADST`.
const V_FLIPADST: usize = 14;
/// AV2 § 3 `H_FLIPADST`.
const H_FLIPADST: usize = 15;
/// AV2 § 9.2 `D45_PRED`.
const D45_PRED: usize = 3;
/// AV2 § 9.2 `D203_PRED`.
const D203_PRED: usize = 7;
/// The single default segment id for the minimal-tool intra block.
const SEGMENT_ID: usize = 0;
/// AV2 § 5.20.8.3 `TX_SET_DCTONLY`.
const TX_SET_DCTONLY: usize = 0;
/// AV2 § 5.20.8.3 `TX_SET_WIDE_64`.
const TX_SET_WIDE_64: usize = 1;
/// AV2 § 5.20.8.3 `TX_SET_HIGH_64`.
const TX_SET_HIGH_64: usize = 2;
/// AV2 § 5.20.8.3 `TX_SET_WIDE_32`.
const TX_SET_WIDE_32: usize = 3;
/// AV2 § 5.20.8.3 `TX_SET_HIGH_32`.
const TX_SET_HIGH_32: usize = 4;
/// AV2 § 5.20.8.3 `TX_SET_INTRA_1`.
const TX_SET_INTRA_1: usize = 5;
/// AV2 § 5.20.8.3 `TX_SET_INTRA_2`.
const TX_SET_INTRA_2: usize = 6;
/// AV2 § 5.20.8.3 `TX_SET_INTER_1`.
const TX_SET_INTER_1: usize = 5;
/// AV2 § 5.20.8.3 `TX_SET_INTER_2`.
const TX_SET_INTER_2: usize = 6;
/// AV2 § 5.20.8.3 `TX_SET_DCT_IDTX`.
const TX_SET_DCT_IDTX: usize = 7;
/// AV2 § 5.20.8.3 `TX_SET_DCT_IDTX_IDDCT`.
const TX_SET_DCT_IDTX_IDDCT: usize = 8;
/// AV2 § 5.20.7.29 `wide_angle_mapping` threshold.
const WAIP_WH_RATIO_2_THRES: i32 = 61;
/// AV2 § 5.20.7.29 `wide_angle_mapping` threshold.
const WAIP_WH_RATIO_4_THRES: i32 = 73;
/// AV2 § 5.20.7.29 `wide_angle_mapping` threshold.
const WAIP_WH_RATIO_8_THRES: i32 = 82;
/// AV2 § 5.20.7.29 `wide_angle_mapping` threshold.
const WAIP_WH_RATIO_16_THRES: i32 = 86;
/// AV2 § 5.20.8.2 `Tx_Type_Inv_Long[is_long_side_dct][wide_or_high][symbol]`.
const TX_TYPE_INV_LONG: [[[usize; 4]; 2]; 2] = [
    [
        [V_DCT, V_ADST, V_FLIPADST, IDTX],
        [H_DCT, H_ADST, H_FLIPADST, IDTX],
    ],
    [
        [DCT_DCT, ADST_DCT, FLIPADST_DCT, H_DCT],
        [DCT_DCT, DCT_ADST, DCT_FLIPADST, V_DCT],
    ],
];

/// AV2 § 5.20.8.2 `Tx_Type_Inter_Inv_Set1[16]` (the `TX_SET_INTER_1` inversion,
/// indexed by `inter_tx_type * 8 + inter_tx_type_offset`).
const TX_TYPE_INTER_INV_SET1: [usize; 16] = [
    IDTX,
    V_DCT,
    H_DCT,
    V_ADST,
    H_ADST,
    V_FLIPADST,
    H_FLIPADST,
    DCT_DCT,
    ADST_DCT,
    DCT_ADST,
    FLIPADST_DCT,
    DCT_FLIPADST,
    ADST_ADST,
    FLIPADST_FLIPADST,
    ADST_FLIPADST,
    FLIPADST_ADST,
];

/// AV2 § 5.20.8.2 `Tx_Type_Inter_Inv_Set2[12]` (the `TX_SET_INTER_2` inversion,
/// indexed by `inter_tx_type * 8 + inter_tx_type_offset`).
const TX_TYPE_INTER_INV_SET2: [usize; 12] = [
    IDTX,
    V_DCT,
    H_DCT,
    DCT_DCT,
    ADST_DCT,
    DCT_ADST,
    FLIPADST_DCT,
    DCT_FLIPADST,
    ADST_ADST,
    FLIPADST_FLIPADST,
    ADST_FLIPADST,
    FLIPADST_ADST,
];

/// AV2 § 5.20.8.2 `Tx_Type_Inter_Inv_Set3[2]` (the `TX_SET_DCT_IDTX` inversion,
/// indexed by `inter_tx_type`).
const TX_TYPE_INTER_INV_SET3: [usize; 2] = [IDTX, DCT_DCT];

/// AV2 § 5.20.8.2 `Tx_Type_Inter_Inv_Set4[4]` (the `TX_SET_DCT_IDTX_IDDCT`
/// inversion, indexed by `inter_tx_type`).
const TX_TYPE_INTER_INV_SET4: [usize; 4] = [DCT_DCT, V_DCT, H_DCT, IDTX];

/// AV2 § 5.20.8.2 split point between `inter_tx_type == 0` (the index symbol) and
/// `inter_tx_type == 1` (the offset symbol) for `TX_SET_INTER_1/2`.
const INTER_TX_TYPE_INDEX_COUNT: usize = 8;

/// Already-decoded luma mode facts needed by AV2 § 5.20.8.2 `transform_type()`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LumaTransformTypeContext {
    y_mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
}

impl LumaTransformTypeContext {
    /// Creates luma transform-type context from § 5.20.5.3 mode-info facts for
    /// the common `MrlIndex == 0` case.
    #[must_use]
    pub(crate) const fn new(y_mode: IntraYMode, angle_delta_y: i8) -> Self {
        Self {
            y_mode,
            angle_delta_y,
            mrl_index: 0,
        }
    }

    /// Creates luma transform-type context with the active § 5.20.5.3 `MrlIndex`
    /// retained for § 5.20.8.2 `transform_type()` directional remapping.
    #[must_use]
    pub(crate) const fn with_mrl_index(
        y_mode: IntraYMode,
        angle_delta_y: i8,
        mrl_index: u8,
    ) -> Self {
        Self {
            y_mode,
            angle_delta_y,
            mrl_index,
        }
    }

    /// The leaf's § 5.20.5.5 `MrlIndex` (the multi-reference-line distance, `0` for
    /// the immediate edge). The ac0ej3 recon sink reads this to DEFER a cardinal
    /// `H_PRED` / `V_PRED` leaf whose `mrl_index > 0` (its primitive copies the
    /// immediate edge, not the selected multi-reference line).
    #[must_use]
    pub(crate) const fn mrl_index(self) -> u8 {
        self.mrl_index
    }

    /// The leaf's § 5.20.5.3 `AngleDeltaY` (the signed angle-delta count). The
    /// ac0ej3 recon sink combines it with the §9.2 `Mode_To_Angle[y_mode]` and
    /// `Mrl_Index_To_Delta[mrl_index]` to recover the §7.13.2.8 `pAngle` for the
    /// one-sided angular admission.
    #[must_use]
    pub(crate) const fn angle_delta_y(self) -> i8 {
        self.angle_delta_y
    }
}

/// Caller-selected policy for nonzero residuals when transform tools are active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TransformToolResidualPolicy {
    /// Decode the nonzero coefficient branch without an additional transform-tool
    /// guard. Existing fixed-size minimal paths use this only after they have
    /// already proven their syntax subset.
    Allow,
    /// Consume `all_zero`, then admit a nonzero residual only through the
    /// implemented transform-tool subset. Reconstruction-safe callers still
    /// require `DCT_DCT`; the LR tx-skip record handoff may carry parsed
    /// transform metadata without claiming reconstructed coefficients.
    AdmitTransformToolSubset {
        /// Luma mode context for active plane-0 `intra_tx_type`; `None` for chroma.
        luma: Option<LumaTransformTypeContext>,
        /// Whether active intra IST is admissible for a syntax-only handoff.
        active_intra_ist: ActiveIntraIstResidualPolicy,
        /// Whether chroma transform/CCTX syntax is admissible for a syntax-only handoff.
        active_chroma: ActiveChromaResidualPolicy,
    },
}

/// Caller-selected handling for active intra IST secondary-transform syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveIntraIstResidualPolicy {
    /// Reject active IST after consuming required syntax.
    Reject,
    /// Admit active IST metadata for LR tx-skip record derivation only.
    LrTxSkipRecordHandoff,
}

/// Caller-selected handling for active chroma transform-tool syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveChromaResidualPolicy {
    /// Reject active chroma CCTX/non-DCT transform syntax before reconstruction.
    Reject,
    /// Admit CCTX metadata and chroma transform syntax for LR tx-skip record derivation only.
    LrTxSkipRecordHandoff,
}

/// Parsed AV2 § 5.20.7.29 intra IST secondary-transform syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraIstSyntax {
    /// Decoded `sec_tx_type` symbol.
    pub(crate) sec_tx_type: usize,
    /// Decoded `most_probable_stx_set`, present only when `sec_tx_type != 0`.
    pub(crate) most_probable_stx_set: Option<usize>,
}

/// The decoded luma transform-block coefficient facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LumaCoeffBlock {
    /// `all_zero` (`txb_skip`) decision: `true` when the block is skipped.
    pub(crate) all_zero: bool,
    /// Decoded end-of-block (number of coded coefficients); `0` when skipped.
    pub(crate) eob: usize,
    /// Decoded row-major `Quant[]` for the adjusted transform block; empty when
    /// skipped.
    pub(crate) quant: Vec<i32>,
    /// Parsed intra IST syntax, when AV2 § 5.20.7.29 read that branch.
    pub(crate) intra_ist: Option<IntraIstSyntax>,
    /// The retained § 3 `PlaneTxType` index (`0..TX_TYPES`, `DCT_DCT == 0`) of
    /// this luma transform block: the already-decoded `metadata.luma_tx_type`,
    /// carried so the § 7.15.4 primary inverse transform resolves the ACTUAL
    /// `Transform_1d_Type[PlaneTxType]` kernels instead of assuming `DCT_DCT`.
    /// The simple non-staged path forces `DCT_DCT` (its eob==1/DCTONLY subset),
    /// and an `all_zero` skip block carries `DCT_DCT` (no residual is inverted).
    pub(crate) plane_tx_type: usize,
}

/// Error returned while decoding the general intra luma coefficient block.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraResidualError {
    /// The `all_zero` (`txb_skip`) symbol read failed.
    #[error("general intra luma all_zero symbol read failed: {source}")]
    AllZeroRead {
        /// Source CDF selection or symbol-decoder error.
        source: BlockSymbolTraceReadError,
    },
    /// The tile coefficient context state could not be allocated.
    #[error("general intra luma coefficient context state failed: {source}")]
    CoeffContextState {
        /// Source coefficient context state error.
        source: TileCoeffStateError,
    },
    /// The nonzero coefficient pass failed.
    #[error("general intra luma nonzero coefficient pass failed: {source}")]
    NonZeroPass {
        /// Source coefficient branch error.
        source: CoeffUseFscBranchError,
    },
    /// The staged nonzero EOB branch failed before transform-tool admission could
    /// decide whether post-EOB syntax is active.
    #[error("general intra luma staged nonzero EOB read failed: {source}")]
    NonZeroStart {
        /// Source coefficient-loop branch error.
        source: CoeffLoopContextError,
    },
    /// The staged nonzero coefficient pass failed after transform-tool admission.
    #[error("general intra luma staged nonzero coefficient pass failed: {source}")]
    StagedNonZeroPass {
        /// Source ordinary coefficient branch error.
        source: CoeffOrdinaryBranchError,
    },
    /// The staged FSC/IDTX coefficient pass failed after transform-tool admission.
    #[error("general intra luma staged FSC coefficient pass failed: {source}")]
    StagedFscPass {
        /// Source FSC coefficient branch error.
        source: CoeffFscBranchError,
    },
    /// An active § 5.20.8.2 luma `intra_tx_type` symbol read failed.
    #[error("general intra luma transform_type symbol read failed: {source}")]
    TransformTypeRead {
        /// Source CDF selection or symbol-decoder error.
        source: BlockSymbolTraceReadError,
    },
    /// A nonzero residual appeared while the caller is only prepared to consume a
    /// narrower transform-tool subset than the block requires.
    #[error("general intra residual requires unsupported active transform-tool syntax: {reason}")]
    UnsupportedTransformToolResidual {
        /// Fail-closed reason for the unsupported transform-tool branch.
        reason: &'static str,
    },
    /// The nonzero coefficient pass returned an unexpected branch result (FSC or
    /// all-zero) for the ordinary luma block.
    #[error("general intra luma nonzero coefficient pass produced an unexpected branch result")]
    UnexpectedBranch,
    /// The decoded `Quant[]` length does not match the adjusted 32x32 transform
    /// block the reconstruction expects.
    #[error("general intra luma reconstruction expected {expected} quant entries, got {actual}")]
    QuantLength {
        /// Expected adjusted-block length.
        expected: usize,
        /// Actual decoded `Quant[]` length.
        actual: usize,
    },
    /// The supplied per-sample prediction buffer length does not match the
    /// original transform-block sample count.
    #[error("general intra reconstruction expected {expected} prediction samples, got {actual}")]
    PredictionLength {
        /// Expected original-block sample count (`orig_side * orig_side`).
        expected: usize,
        /// Actual prediction buffer length.
        actual: usize,
    },
    /// The `splot-recon` dequant / inverse-transform / residual reconstruction
    /// rejected the luma block.
    #[error("general intra luma reconstruction failed: {source}")]
    Reconstruct {
        /// Source reconstruction error.
        #[from]
        source: ReconError,
    },
    /// A § 7.13.2.8 middle-angle directional block hit the `haveLeft && haveAbove`
    /// edge-builder arm without its real § 7.13.2.1 corner sample
    /// `CurrFrame[plane][y-1][x-1]` (D135 reads the corner on its main diagonal
    /// `column == row`, where `above_base == -1`). The neighbour reconstruction path
    /// always supplies that corner via `reconstructed_sample` before calling the
    /// builder, so this is a defensive guard reached only if a future caller invokes
    /// the builder for the `haveLeft && haveAbove` arm without the corner (returned
    /// rather than panicking, per the no-panic policy).
    #[error(
        "general intra directional prediction over a real above-neighbour edge is missing its §7.13.2.1 corner sample"
    )]
    UnsupportedDirectionalAboveEdge,
    /// A § 7.13.2.8 cardinal directional block (`V_PRED` pAngle 90 / `H_PRED`
    /// pAngle 180) was reached without its required reconstructed neighbour edge:
    /// `V_PRED` needs the real § 7.13.2.1 above row (`haveAbove == 1`), `H_PRED`
    /// needs the real left column (`haveLeft == 1`). The admission gate only
    /// admits these when the edge is present, so this is reached only if that gate
    /// is relaxed without supplying the edge.
    #[error("general intra cardinal directional prediction is missing its required neighbour edge")]
    MissingCardinalEdge,
    /// A cardinal `V_PRED` / `H_PRED` mode reached the § 7.13.2.8 middle-angle
    /// (`90 < pAngle < 180`) mapping, which only covers `D135`. The dispatch routes
    /// cardinal modes to the dedicated copy predictor
    /// (`reconstruct_general_intra_cardinal_neighbour_block_into`), so this is
    /// unreachable in correct operation; it is a defensive guard for a dispatch
    /// regression (returned rather than panicking, per the no-panic policy).
    #[error(
        "general intra cardinal (V_PRED/H_PRED) mode reached the middle-angle path; it must be dispatched to the cardinal copy reconstruction"
    )]
    CardinalModeInMiddleAnglePath,
}

/// OR-reduces a `u32` context line over `[start, start + len)` (clamped to the
/// available range), the AV2 § 8.3.2 above/left level-context reduction.
fn or_u32(line: &[u32], start: usize, len: usize) -> u32 {
    line.iter().skip(start).take(len).fold(0, |acc, &v| acc | v)
}

/// OR-reduces a `u8` DC-context line over `[start, start + len)` (clamped).
fn or_u8(line: &[u8], start: usize, len: usize) -> u8 {
    line.iter().skip(start).take(len).fold(0, |acc, &v| acc | v)
}

fn coeff_ctx_err(source: TileCoeffStateError) -> GeneralIntraResidualError {
    GeneralIntraResidualError::CoeffContextState { source }
}

/// Decodes the AV2 § 5.20.7.27 `coeffs()` syntax for one transform block of any
/// plane in the general intra multi-block walk. `tx_size` is the full § 9.2
/// `TX_SIZES_ALL` index, so square (e.g. `TX_64X64`) and rectangular (e.g.
/// `TX_64X32`) transforms are both handled: the above context span is
/// `Tx_Width[txSz] >> 2` and the left context span is `Tx_Height[txSz] >> 2`,
/// and the nonzero coefficient geometry (scan, eob class, dequant, transform)
/// already reads width and height independently from the conversion tables.
///
/// The § 8.3.2 `txb_skip` (`all_zero`) context is derived from the persistent
/// neighbour context lines (`context`): luma uses
/// `TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]` with `ctx` from the
/// above/left level OR-reductions; U adds the `+6` chroma offset; V uses
/// `TileVTxbSkipCdf[ctx]` (with the `EobU` term). When `all_zero == 1` the zero
/// context write is committed here; otherwise the nonzero coefficient pass reads
/// `dc_sign` from `context` at `start_x`/`start_y` and commits its own context
/// update internally. `start_x`/`start_y` are the block's plane-sample position.
/// `tx_fills_block` is the caller-resolved § 8.3.2 fact `bw == w && bh == h`
/// for luma `all_zero` context derivation; for V it drives the complementary
/// `bw > w || bh > h` context term.
/// `transform_tool_residual_policy` preserves the AV2 ordering: the helper
/// always consumes the `all_zero` decision first, admits skipped transform
/// blocks, and applies any transform-tool guard only before the nonzero
/// coefficient branch.
// Each bool is a distinct AV2 coefficient-branch syntax flag; bundling them would obscure the spec mapping.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub(crate) fn decode_general_intra_plane_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    plane: usize,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    tx_fills_block: bool,
    eob_u_nonzero: bool,
    uv_mode: usize,
    is_inter: bool,
    fsc_mode: bool,
    transform_tool_residual_policy: TransformToolResidualPolicy,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let x4 = start_x >> 2;
    let y4 = start_y >> 2;
    // AV2 § 5.20.7.27: the above (`txw4`) and left (`txh4`) context spans are the
    // transform block's width / height in 4x4 units, `Tx_Width[txSz] >> 2` and
    // `Tx_Height[txSz] >> 2`, read from the generated § 9.2 conversion tables. For
    // a square transform `w4 == h4 == 1 << tx_size`; for a rectangular transform
    // (e.g. TX_64X32) they differ, so the above context line is OR-reduced over the
    // width span and the left context line over the height span.
    let w4 = usize::try_from(TX_WIDTH.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let h4 = usize::try_from(TX_HEIGHT.get(tx_size).copied().unwrap_or(0)).unwrap_or(0) >> 2;
    let frame_facts = work_unit.coeff_frame_facts();
    let coeff_cdf_q_ctx = coeff_cdf_q_ctx_from_base_q_idx(frame_facts.base_q_idx());
    let tx_size_ctx = txb_skip_tx_size_ctx(tx_size);

    let above_level_or = or_u32(context.above_level(plane).map_err(coeff_ctx_err)?, x4, w4);
    let left_level_or = or_u32(context.left_level(plane).map_err(coeff_ctx_err)?, y4, h4);
    // AV2 § 8.3.2 `all_zero` (txb_skip): for plane 0/1 the cdf is
    // `TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]`. The V plane uses the
    // separate `TileVTxbSkipCdf[ctx]`, which carries no inter/intra split.
    let txb_skip_intra_inter = usize::from(is_inter || fsc_mode);
    let selector = match plane {
        2 => {
            let above_nz = above_level_or != 0
                || or_u8(context.above_dc(plane).map_err(coeff_ctx_err)?, x4, w4) != 0;
            let left_nz = left_level_or != 0
                || or_u8(context.left_dc(plane).map_err(coeff_ctx_err)?, y4, h4) != 0;
            TileCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx: v_txb_skip_ctx(above_nz, left_nz, !tx_fills_block, eob_u_nonzero),
            }
        }
        1 => {
            let above_nz = above_level_or != 0
                || or_u8(context.above_dc(plane).map_err(coeff_ctx_err)?, x4, w4) != 0;
            let left_nz = left_level_or != 0
                || or_u8(context.left_dc(plane).map_err(coeff_ctx_err)?, y4, h4) != 0;
            TileCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: txb_skip_intra_inter,
                tx_size: tx_size_ctx,
                ctx: usize::from(above_nz) + usize::from(left_nz) + 6,
            }
        }
        _ => TileCdfSelector::TxbSkip {
            coeff_cdf_q_ctx,
            plane_type: txb_skip_intra_inter,
            tx_size: tx_size_ctx,
            ctx: txb_skip_ctx_luma(
                above_level_or,
                left_level_or,
                tx_fills_block,
                fsc_mode && frame_facts.enable_fsc(),
            ),
        },
    };

    let all_zero = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .read_block_symbol_trace(selector, symbols)
        .map_err(|source| GeneralIntraResidualError::AllZeroRead { source })?
        .get()
        != 0;

    if all_zero {
        context
            .update_after_coeffs(CoeffContextUpdate {
                plane,
                x4,
                y4,
                w4,
                h4,
                cul_level: 0,
                dc_category: 0,
            })
            .map_err(coeff_ctx_err)?;
        return Ok(LumaCoeffBlock {
            all_zero: true,
            eob: 0,
            quant: Vec::new(),
            intra_ist: None,
            // A skipped block has no residual to inverse-transform; the type is
            // irrelevant, so carry the canonical `DCT_DCT`.
            plane_tx_type: DCT_DCT,
        });
    }

    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane,
        start_x,
        start_y,
        tx_size,
    };
    if let TransformToolResidualPolicy::AdmitTransformToolSubset {
        luma,
        active_intra_ist,
        active_chroma,
    } = transform_tool_residual_policy
    {
        return decode_staged_transform_tool_nonzero_coeffs(
            work_unit,
            symbols,
            context,
            frame_facts,
            geometry,
            coeff_cdf_q_ctx,
            uv_mode,
            is_inter,
            fsc_mode,
            luma,
            active_intra_ist,
            active_chroma,
        );
    }

    let input = CoeffUseFscFrameFactsInput::NonZero(CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts,
        block: CoeffUseFscFrameBlockFacts {
            geometry,
            plane_tx_type: DCT_DCT,
            fsc_mode,
            is_inter,
            segment_id: SEGMENT_ID,
        },
        ordinary: CoeffUseFscFrameOrdinaryFacts {
            uv_mode,
            angle_delta_uv: 0,
            luma_tx_type: DCT_DCT,
            chroma_inter_tx_type: DCT_DCT,
        },
    });
    let cdfs = work_unit.cdf_mut().tile_cdfs_mut();
    let branch = apply_coeff_use_fsc_branch_from_frame_facts(context, cdfs, symbols, input)
        .map_err(|source| GeneralIntraResidualError::NonZeroPass { source })?;
    let CoeffUseFscBranch::Ordinary(CoeffOrdinaryBranch::NonZero(pass)) = branch else {
        return Err(GeneralIntraResidualError::UnexpectedBranch);
    };
    Ok(LumaCoeffBlock {
        all_zero: false,
        eob: pass.eob_read().eob().eob(),
        quant: pass.block().quant().to_vec(),
        intra_ist: None,
        // The simple non-staged path is the `DCTONLY`-forced subset (no
        // transform-tool syntax was read), so the luma tx-type is `DCT_DCT`.
        plane_tx_type: DCT_DCT,
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_staged_transform_tool_nonzero_coeffs(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    context: &mut TileCoeffContextState,
    frame_facts: TileCoeffFrameFacts,
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    coeff_cdf_q_ctx: usize,
    uv_mode: usize,
    is_inter: bool,
    fsc_mode: bool,
    luma_transform_type_context: Option<LumaTransformTypeContext>,
    active_intra_ist_policy: ActiveIntraIstResidualPolicy,
    active_chroma_policy: ActiveChromaResidualPolicy,
) -> Result<LumaCoeffBlock, GeneralIntraResidualError> {
    let tx_width_log2 = tx_size_table_usize(&TX_WIDTH_LOG2, "Tx_Width_Log2", geometry.tx_size)?;
    let tx_height_log2 = tx_size_table_usize(&TX_HEIGHT_LOG2, "Tx_Height_Log2", geometry.tx_size)?;
    let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", geometry.tx_size)?;
    let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", geometry.tx_size)?;
    let block = AllZeroCoeffBlockInput {
        plane: geometry.plane,
        x4: geometry.start_x >> 2,
        y4: geometry.start_y >> 2,
        w4: tx_width >> 2,
        h4: tx_height >> 2,
    };
    let start = match read_coeff_block_eob_branch(
        context,
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        CoeffBlockEobBranchInput::NonZero(NonZeroCoeffBlockStartInput {
            block,
            eob: NonZeroCoeffEobContextInput {
                plane: geometry.plane,
                is_inter,
                tx_width_log2,
                tx_height_log2,
                coeff_cdf_q_ctx,
            },
        }),
    )
    .map_err(|source| GeneralIntraResidualError::NonZeroStart { source })?
    {
        CoeffBlockEobBranch::NonZero(start) => start,
        CoeffBlockEobBranch::AllZero(_) => {
            return Err(GeneralIntraResidualError::UnexpectedBranch);
        }
    };
    let eob = start.eob_read().eob().eob();
    let metadata = ensure_transform_tool_residual_handoff(
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        TransformToolResidualInput {
            frame_facts,
            plane: geometry.plane,
            tx_size: geometry.tx_size,
            is_inter,
            fsc_mode,
            eob,
            luma_transform_type_context,
            active_intra_ist_policy,
            active_chroma_policy,
        },
    )?;
    let lossless = frame_facts.lossless_for_segment(SEGMENT_ID).ok_or(
        GeneralIntraResidualError::UnsupportedTransformToolResidual {
            reason: "unsupported_dctonly_residual_segment_id",
        },
    )?;
    let base_config = staged_transform_tool_lossless_base_config(
        frame_facts,
        geometry.plane,
        uv_mode,
        lossless,
        metadata,
    );
    let use_fsc = frame_facts.enable_fsc()
        && metadata.luma_tx_type == IDTX
        && geometry.plane == 0
        && (fsc_mode || is_inter);
    if use_fsc {
        let pass = apply_staged_nonzero_coeff_fsc_branch_from_tx_size(
            context,
            work_unit.cdf_mut().tile_cdfs_mut(),
            symbols,
            CoeffFscStagedTxSizeNonZeroInput {
                block,
                start,
                tx_size: geometry.tx_size,
                plane_tx_type: metadata.luma_tx_type,
                coeff_cdf_q_ctx,
            },
        )
        .map_err(|source| GeneralIntraResidualError::StagedFscPass { source })?;
        return Ok(LumaCoeffBlock {
            all_zero: false,
            eob: pass.eob_read().eob().eob(),
            quant: pass.block().quant().to_vec(),
            intra_ist: metadata.intra_ist,
            plane_tx_type: metadata.luma_tx_type,
        });
    }
    let pass = apply_staged_nonzero_coeff_ordinary_branch_from_lossless(
        context,
        work_unit.cdf_mut().tile_cdfs_mut(),
        symbols,
        CoeffOrdinaryStagedLosslessNonZeroInput {
            geometry,
            start,
            coeff_cdf_q_ctx,
            is_inter,
            base_config,
            lossless,
        },
    )
    .map_err(|source| GeneralIntraResidualError::StagedNonZeroPass { source })?;
    Ok(LumaCoeffBlock {
        all_zero: false,
        eob,
        quant: pass.block().quant().to_vec(),
        intra_ist: metadata.intra_ist,
        plane_tx_type: metadata.luma_tx_type,
    })
}

fn staged_transform_tool_lossless_base_config(
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    uv_mode: usize,
    lossless: bool,
    metadata: TransformToolResidualMetadata,
) -> CoeffOrdinaryBranchLosslessBaseConfig {
    let luma_tx_class = CoeffTransformClass::from_plane_tx_type(metadata.luma_tx_type);
    let luma_transform_block = plane == 0;
    let parity_hiding = frame_facts.allow_parity_hiding()
        && !lossless
        && luma_transform_block
        && metadata.luma_tx_type != IDTX;
    let use_tcq = frame_facts.allow_tcq()
        && !lossless
        && luma_transform_block
        && luma_tx_class == CoeffTransformClass::TwoD;
    CoeffOrdinaryBranchLosslessBaseConfig {
        reduced_tx_set: frame_facts.reduced_tx_set(),
        enable_chroma_dctonly: frame_facts.enable_chroma_dctonly(),
        uv_mode,
        angle_delta_uv: 0,
        luma_tx_type: metadata.luma_tx_type,
        chroma_inter_tx_type: DCT_DCT,
        parity_hiding,
        use_tcq,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TransformToolResidualMetadata {
    intra_ist: Option<IntraIstSyntax>,
    luma_tx_type: usize,
    // Plane-1 CCTX syntax is intentionally retained only for bitstream sync here;
    // luma tx-skip record derivation drops it until chroma records are consumed.
    // TODO(spec: DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS): hand this
    // syntax metadata to the next transform-record residual parser frontier.
    cctx_type: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TransformToolResidualInput {
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    fsc_mode: bool,
    eob: usize,
    luma_transform_type_context: Option<LumaTransformTypeContext>,
    active_intra_ist_policy: ActiveIntraIstResidualPolicy,
    active_chroma_policy: ActiveChromaResidualPolicy,
}

fn ensure_transform_tool_residual_handoff(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    input: TransformToolResidualInput,
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    let frame_facts = input.frame_facts;
    let plane = input.plane;
    let tx_size = input.tx_size;
    let is_inter = input.is_inter;
    let eob = input.eob;
    let mut metadata = TransformToolResidualMetadata {
        luma_tx_type: DCT_DCT,
        ..TransformToolResidualMetadata::default()
    };
    if frame_facts.lossless_for_segment(SEGMENT_ID) != Some(false) {
        return unsupported_transform_tool_residual("unsupported_dctonly_residual_lossless");
    }
    if !is_inter && plane == 1 && frame_facts.enable_cctx() && eob != 1 {
        metadata.cctx_type = Some(read_chroma_cctx_type(
            cdfs,
            symbols,
            input.active_chroma_policy,
        )?);
    }
    let tx_set = transform_set(frame_facts, plane, tx_size, is_inter)?;
    let dct_forced = (!is_inter && plane == 0 && eob == 1)
        || (plane > 0 && frame_facts.enable_chroma_dctonly())
        || tx_set == TX_SET_DCTONLY
        || (!is_inter && plane == 0 && frame_facts.reduced_tx_set() == 2);
    if !is_inter && plane == 0 && input.fsc_mode {
        metadata.luma_tx_type = IDTX;
    } else if !dct_forced {
        if !is_inter
            && plane > 0
            && input.active_chroma_policy == ActiveChromaResidualPolicy::LrTxSkipRecordHandoff
        {
            // Syntax-only handoff: the later coefficient branch derives the
            // chroma PlaneTxType from UVMode and txSet, but no reconstruction
            // or output consumes the Quant values in this LR-record path.
        } else if !is_inter && plane == 0 {
            let luma_tx_type = read_active_luma_transform_type(
                cdfs,
                symbols,
                input.luma_transform_type_context,
                tx_size,
                tx_set,
            )?;
            if luma_tx_type != DCT_DCT
                && input.active_intra_ist_policy
                    != ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff
            {
                return unsupported_transform_tool_residual(
                    "unsupported_dctonly_residual_luma_tx_type",
                );
            }
            metadata.luma_tx_type = luma_tx_type;
        } else if is_inter && plane == 0 {
            // AV2 § 5.20.7.29 inter luma `transform_type`: the §7.13.3.18 IntrABC
            // leaf is `is_inter == 1`, so its primary transform type is read on the
            // inter path. Only the long-side sets (`TX_SET_WIDE_32/64`,
            // `TX_SET_HIGH_32/64`) are proven here — the `inter_tx_type` symbol uses
            // `TileInterTxTypeLongCdf[ctx][Tx_Size_Sqr[txSz]]` (§8.3.2 Table 8.3).
            // The other inter sets (`TX_SET_INTER_1/2`, `TX_SET_DCT_IDTX*`) need the
            // `inter_tx_type_offset` reads this brick has not proven, so they defer.
            let luma_tx_type =
                read_active_inter_transform_type(cdfs, symbols, tx_size, tx_set, eob)?;
            // A non-`DCT_DCT` inter luma transform type carries the same §5.20.7.27
            // coefficient entropy as the DCT_DCT path — the coefficient loop is
            // transform-type-agnostic (it parameterizes scan/class/parity/TCQ from
            // `metadata.luma_tx_type`); only the §7.13.3 inverse transform differs.
            // The §7.13.3.18 IntrABC leaf (`is_inter == 1`) already DEFERS its sample
            // write at the reconstruction sink, so a syntax-only handoff caller can
            // admit the non-DCT type, decode the residual to advance the entropy
            // state, and defer the unsupported inverse transform. A reconstruction-safe
            // caller still requires `DCT_DCT`: its sink would inverse-transform with the
            // wrong (DCT_DCT) kernel, so it stays fail-closed.
            if luma_tx_type != DCT_DCT
                && input.active_intra_ist_policy
                    != ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff
            {
                return unsupported_transform_tool_residual(
                    "unsupported_dctonly_residual_inter_tx_type",
                );
            }
            metadata.luma_tx_type = luma_tx_type;
        } else {
            return unsupported_transform_tool_residual("unsupported_dctonly_residual_tx_set");
        }
    }
    if is_inter
        && plane == 0
        && frame_facts.enable_inter_ist()
        && eob > 3
        && metadata.luma_tx_type == DCT_DCT
        && inter_ist_can_read_sec_tx(tx_size, eob)?
    {
        // AV2 § 5.20.7.29 inter IST: read `sec_tx_type` (CDF
        // `TileSecTxTypeCdf[is_inter == 1][Tx_Size_Sqr[txSz]]`). For inter,
        // `most_probable_stx_set` is NOT read (the spec gates that on `!is_inter`),
        // so the parse stays synced regardless of the value.
        let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
        let sec_tx_type = read_transform_symbol(
            cdfs,
            symbols,
            TileCdfSelector::SecTxType {
                is_inter: 1,
                tx_size_sqr,
            },
        )?;
        metadata.intra_ist = Some(IntraIstSyntax {
            sec_tx_type,
            most_probable_stx_set: None,
        });
    }
    if !is_inter
        && plane == 0
        && frame_facts.enable_intra_ist()
        && eob != 1
        && intra_ist_can_read_sec_tx_type(metadata.luma_tx_type)
        && eob <= ist_eob_limit(tx_size, metadata.luma_tx_type)?
    {
        metadata.intra_ist = read_intra_ist_sec_tx(
            cdfs,
            symbols,
            input.luma_transform_type_context,
            tx_size,
            input.active_intra_ist_policy,
        )?;
    }
    Ok(metadata)
}

fn read_chroma_cctx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    policy: ActiveChromaResidualPolicy,
) -> Result<usize, GeneralIntraResidualError> {
    if policy != ActiveChromaResidualPolicy::LrTxSkipRecordHandoff {
        return unsupported_transform_tool_residual("unsupported_dctonly_residual_cctx");
    }
    read_transform_symbol(cdfs, symbols, TileCdfSelector::CctxType)
}

fn read_intra_ist_sec_tx(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    luma_context: Option<LumaTransformTypeContext>,
    tx_size: usize,
    policy: ActiveIntraIstResidualPolicy,
) -> Result<Option<IntraIstSyntax>, GeneralIntraResidualError> {
    let Some(luma_context) = luma_context else {
        return unsupported_transform_tool_residual(
            "unsupported_dctonly_residual_intra_ist_context",
        );
    };
    if luma_context.y_mode.is_paeth() {
        return Ok(None);
    }

    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let sec_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::SecTxType {
            is_inter: 0,
            tx_size_sqr,
        },
    )?;
    let most_probable_stx_set = if sec_tx_type == 0 {
        None
    } else {
        Some(read_transform_symbol(
            cdfs,
            symbols,
            TileCdfSelector::MostProbableStxSet,
        )?)
    };
    let syntax = IntraIstSyntax {
        sec_tx_type,
        most_probable_stx_set,
    };
    ensure_supported_intra_ist_sec_tx_type(syntax, policy)?;
    Ok(Some(syntax))
}

fn ensure_supported_intra_ist_sec_tx_type(
    syntax: IntraIstSyntax,
    policy: ActiveIntraIstResidualPolicy,
) -> Result<(), GeneralIntraResidualError> {
    if syntax.sec_tx_type == 0 || policy == ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff {
        Ok(())
    } else {
        unsupported_transform_tool_residual("unsupported_dctonly_residual_intra_sec_tx_type")
    }
}

fn inter_ist_can_read_sec_tx(
    tx_size: usize,
    eob: usize,
) -> Result<bool, GeneralIntraResidualError> {
    let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?;
    let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?;
    Ok(tx_width >= 16 && tx_height >= 16 && eob <= ist_eob_limit(tx_size, DCT_DCT)?)
}

const fn intra_ist_can_read_sec_tx_type(luma_tx_type: usize) -> bool {
    matches!(luma_tx_type, DCT_DCT | ADST_ADST)
}

fn ist_eob_limit(tx_size: usize, tx_type: usize) -> Result<usize, GeneralIntraResidualError> {
    let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?;
    let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?;
    if tx_width < 8 || tx_height < 8 {
        Ok(IST_4X4_HEIGHT)
    } else if tx_size == TX_8X8 || tx_type == ADST_ADST {
        Ok(IST_8X8_HEIGHT_RED)
    } else {
        Ok(IST_8X8_HEIGHT)
    }
}

fn read_active_luma_transform_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    luma_context: Option<LumaTransformTypeContext>,
    tx_size: usize,
    tx_set: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let Some(luma_context) = luma_context else {
        return unsupported_transform_tool_residual(
            "unsupported_dctonly_residual_luma_transform_context",
        );
    };
    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let tx_type = match tx_set {
        TX_SET_INTRA_1 => {
            let intra_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr },
            )?;
            md_idx_luma_tx_type(tx_size, luma_context, intra_tx_type)?
        }
        TX_SET_INTRA_2 => {
            let intra_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr },
            )?;
            md_idx_luma_tx_type(tx_size, luma_context, intra_tx_type)?
        }
        TX_SET_WIDE_64 | TX_SET_HIGH_64 | TX_SET_WIDE_32 | TX_SET_HIGH_32 => {
            read_active_luma_long_tx_type(cdfs, symbols, tx_set, tx_size_sqr)?
        }
        _ => {
            return unsupported_transform_tool_residual("unsupported_dctonly_residual_luma_tx_set");
        }
    };
    Ok(tx_type)
}

fn read_active_luma_long_tx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_set: usize,
    tx_size_sqr: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let is_long_side_dct = match tx_set {
        TX_SET_WIDE_32 | TX_SET_HIGH_32 => read_transform_symbol(
            cdfs,
            symbols,
            TileCdfSelector::IsLongSideDct { is_inter: 0 },
        )?,
        TX_SET_WIDE_64 | TX_SET_HIGH_64 => 1,
        _ => {
            return unsupported_transform_tool_residual(
                "unsupported_dctonly_residual_luma_tx_set_long",
            );
        }
    };
    let intra_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::IntraTxTypeLong { tx_size_sqr },
    )?;
    let wide_or_high = match tx_set {
        TX_SET_WIDE_64 | TX_SET_WIDE_32 => 0,
        TX_SET_HIGH_64 | TX_SET_HIGH_32 => 1,
        _ => {
            return unsupported_transform_tool_residual(
                "unsupported_dctonly_residual_luma_tx_set_long",
            );
        }
    };
    TX_TYPE_INV_LONG
        .get(is_long_side_dct)
        .and_then(|long_side| long_side.get(wide_or_high))
        .and_then(|row| row.get(intra_tx_type))
        .copied()
        .ok_or(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "unsupported_dctonly_residual_invalid_luma_tx_type",
            },
        )
}

/// Reads the AV2 § 5.20.7.29 / § 5.20.8.2 inter luma primary `transform_type`.
///
/// Dispatches by §5.20.8.3 transform set: the long-side sets (`TX_SET_WIDE_32/64`,
/// `TX_SET_HIGH_32/64`) read `is_long_side_dct` plus the long `inter_tx_type`
/// (`TileInterTxTypeLongCdf`); the small sets (`TX_SET_INTER_1/2`,
/// `TX_SET_DCT_IDTX`, `TX_SET_DCT_IDTX_IDDCT`) read the `inter_tx_type` selector
/// (and, for `INTER_1/2`, the follow-up `inter_tx_type_offset`) per §8.3.2 Table
/// 8.3. The coefficient entropy is transform-type-agnostic, so any returned type
/// only changes the §7.13.3 inverse transform (deferred at the reconstruction
/// sink), not the residual parse.
fn read_active_inter_transform_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_size: usize,
    tx_set: usize,
    eob: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let ctx = inter_tx_type_long_ctx(tx_size, eob)?;
    match tx_set {
        TX_SET_WIDE_64 | TX_SET_WIDE_32 | TX_SET_HIGH_64 | TX_SET_HIGH_32 => {
            read_active_inter_long_tx_type(cdfs, symbols, tx_set, tx_size_sqr, ctx)
        }
        TX_SET_INTER_1 => read_inter_tx_type_signaling_set(
            cdfs,
            symbols,
            InterTxTypeSignalingSet::Inter1,
            tx_size_sqr,
            ctx,
        ),
        TX_SET_INTER_2 => read_inter_tx_type_signaling_set(
            cdfs,
            symbols,
            InterTxTypeSignalingSet::Inter2,
            tx_size_sqr,
            ctx,
        ),
        TX_SET_DCT_IDTX => {
            let inter_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::InterTxTypeSet3 { ctx, tx_size_sqr },
            )?;
            TX_TYPE_INTER_INV_SET3
                .get(inter_tx_type)
                .copied()
                .ok_or(invalid_inter_tx_type())
        }
        TX_SET_DCT_IDTX_IDDCT => {
            let inter_tx_type = read_transform_symbol(
                cdfs,
                symbols,
                TileCdfSelector::InterTxTypeSet4 { ctx, tx_size_sqr },
            )?;
            TX_TYPE_INTER_INV_SET4
                .get(inter_tx_type)
                .copied()
                .ok_or(invalid_inter_tx_type())
        }
        _ => unsupported_transform_tool_residual("unsupported_dctonly_residual_inter_tx_set"),
    }
}

/// Reads the inter long-side `transform_type` (`TX_SET_WIDE_32/64`,
/// `TX_SET_HIGH_32/64`) per §5.20.8.2: `is_long_side_dct` (32 sets only) plus the
/// long `inter_tx_type` symbol, inverted through `Tx_Type_Inv_Long`.
fn read_active_inter_long_tx_type(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    tx_set: usize,
    tx_size_sqr: usize,
    ctx: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let wide_or_high = match tx_set {
        TX_SET_WIDE_64 | TX_SET_WIDE_32 => 0,
        _ => 1,
    };
    // §5.20.7.29 `is_long_side_dct` is read (CDF `TileIsLongSideDctCdf[is_inter]`)
    // only for the 32 sets; the 64 sets force `is_long_side_dct = 1`.
    let is_long_side_dct = match tx_set {
        TX_SET_WIDE_32 | TX_SET_HIGH_32 => read_transform_symbol(
            cdfs,
            symbols,
            TileCdfSelector::IsLongSideDct { is_inter: 1 },
        )?,
        _ => 1,
    };
    let inter_tx_type = read_transform_symbol(
        cdfs,
        symbols,
        TileCdfSelector::InterTxTypeLong { ctx, tx_size_sqr },
    )?;
    TX_TYPE_INV_LONG
        .get(is_long_side_dct)
        .and_then(|long_side| long_side.get(wide_or_high))
        .and_then(|row| row.get(inter_tx_type))
        .copied()
        .ok_or(invalid_inter_tx_type())
}

/// Distinguishes the §5.20.8.2 `TX_SET_INTER_1` / `TX_SET_INTER_2` signaling sets,
/// which share the two-stage `inter_tx_type` + `inter_tx_type_offset` read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterTxTypeSignalingSet {
    /// `TX_SET_INTER_1` (`Tx_Type_Inter_Inv_Set1`).
    Inter1,
    /// `TX_SET_INTER_2` (`Tx_Type_Inter_Inv_Set2`).
    Inter2,
}

/// Reads §5.20.8.2 `inter_tx_type` then `inter_tx_type_offset` for the
/// `TX_SET_INTER_1` / `TX_SET_INTER_2` signaling sets, returning the inverted
/// `TxType`. `inter_tx_type` is a 2-symbol selector; the follow-up index/offset
/// symbol picks `inter_tx_type * 8 + inter_tx_type_offset` into the §5.20.8.2
/// inversion table (AVM `av2_read_tx_type`, decodemv.c §`inter_block`).
fn read_inter_tx_type_signaling_set(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    set: InterTxTypeSignalingSet,
    tx_size_sqr: usize,
    ctx: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let (set_selector, index_selector, offset_selector, inversion): (
        TileCdfSelector,
        TileCdfSelector,
        TileCdfSelector,
        &[usize],
    ) = match set {
        InterTxTypeSignalingSet::Inter1 => (
            TileCdfSelector::InterTxTypeSet1 { ctx, tx_size_sqr },
            TileCdfSelector::InterTxTypeIndexSet1 { ctx },
            TileCdfSelector::InterTxTypeOffsetSet1 { ctx },
            &TX_TYPE_INTER_INV_SET1,
        ),
        InterTxTypeSignalingSet::Inter2 => (
            TileCdfSelector::InterTxTypeSet2 { ctx },
            TileCdfSelector::InterTxTypeIndexSet2 { ctx },
            TileCdfSelector::InterTxTypeOffsetSet2 { ctx },
            &TX_TYPE_INTER_INV_SET2,
        ),
    };
    let inter_tx_type = read_transform_symbol(cdfs, symbols, set_selector)?;
    let inter_tx_type_offset = if inter_tx_type == 0 {
        read_transform_symbol(cdfs, symbols, index_selector)?
    } else {
        read_transform_symbol(cdfs, symbols, offset_selector)?
    };
    let tx_type_idx = inter_tx_type * INTER_TX_TYPE_INDEX_COUNT + inter_tx_type_offset;
    inversion
        .get(tx_type_idx)
        .copied()
        .ok_or(invalid_inter_tx_type())
}

/// The fail-closed error for an out-of-range inverted inter `TxType`.
const fn invalid_inter_tx_type() -> GeneralIntraResidualError {
    GeneralIntraResidualError::UnsupportedTransformToolResidual {
        reason: "unsupported_dctonly_residual_invalid_inter_tx_type",
    }
}

/// AV2 § 8.3.2 `inter_tx_type` context: from the `eob` diagonal position relative
/// to the (32-capped) transform extent.
fn inter_tx_type_long_ctx(tx_size: usize, eob: usize) -> Result<usize, GeneralIntraResidualError> {
    let eob = eob.checked_sub(1).ok_or(
        GeneralIntraResidualError::UnsupportedTransformToolResidual {
            reason: "unsupported_dctonly_residual_inter_tx_type_eob",
        },
    )?;
    let tx_width_log2 = tx_size_table_usize(&TX_WIDTH_LOG2, "Tx_Width_Log2", tx_size)?;
    let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?;
    let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?;
    let bwl = tx_width_log2.min(5);
    let eoby = eob >> bwl;
    let eobx = eob - (eoby << bwl);
    let diag = eobx + eoby;
    let max_diag = tx_width.min(32) + tx_height.min(32) - 4;
    Ok(if diag < 2 {
        1
    } else if diag > max_diag {
        2
    } else {
        0
    })
}

fn read_transform_symbol(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    selector: TileCdfSelector,
) -> Result<usize, GeneralIntraResidualError> {
    Ok(usize::from(
        cdfs.read_block_symbol_trace(selector, symbols)
            .map_err(|source| GeneralIntraResidualError::TransformTypeRead { source })?
            .get(),
    ))
}

fn md_idx_luma_tx_type(
    tx_size: usize,
    luma_context: LumaTransformTypeContext,
    intra_tx_type: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let size_info = tx_size_table_usize(&SIZE_CLASS, "Size_Class", tx_size)?;
    let intra_dir = luma_transform_intra_dir(tx_size, luma_context)?;
    let mode_row = MD_IDX_TO_TYPE
        .get(size_info)
        .and_then(|size| size.get(intra_dir))
        .ok_or(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "unsupported_dctonly_residual_invalid_intra_mode",
            },
        )?;
    let tx_type = mode_row.get(intra_tx_type).copied().ok_or(
        GeneralIntraResidualError::UnsupportedTransformToolResidual {
            reason: "unsupported_dctonly_residual_invalid_intra_tx_type",
        },
    )?;
    usize::try_from(tx_type).map_err(|_| {
        GeneralIntraResidualError::UnsupportedTransformToolResidual {
            reason: "unsupported_dctonly_residual_invalid_luma_tx_type",
        }
    })
}

fn luma_transform_intra_dir(
    tx_size: usize,
    luma_context: LumaTransformTypeContext,
) -> Result<usize, GeneralIntraResidualError> {
    let intra_dir = luma_context.y_mode.value();
    if !luma_context.y_mode.is_directional() {
        return Ok(intra_dir);
    }
    let mode_to_angle = MODE_TO_ANGLE.get(intra_dir).copied().ok_or(
        GeneralIntraResidualError::UnsupportedTransformToolResidual {
            reason: "unsupported_dctonly_residual_invalid_intra_mode",
        },
    )?;
    let mrl_delta = MRL_INDEX_TO_DELTA
        .get(usize::from(luma_context.mrl_index))
        .copied()
        .ok_or(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "unsupported_dctonly_residual_invalid_mrl_index",
            },
        )?;
    let p_angle = mode_to_angle
        .checked_add(i32::from(luma_context.angle_delta_y) * ANGLE_STEP)
        .and_then(|angle| angle.checked_add(mrl_delta))
        .ok_or(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "unsupported_dctonly_residual_luma_angle_overflow",
            },
        )?;
    let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?;
    let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?;
    Ok(wide_angle_mapping(intra_dir, tx_width, tx_height, p_angle))
}

/// AV2 § 5.20.7.29 `wide_angle_mapping`.
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

fn transform_set(
    frame_facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
) -> Result<usize, GeneralIntraResidualError> {
    let tx_size_sqr = tx_size_table_usize(&TX_SIZE_SQR, "Tx_Size_Sqr", tx_size)?;
    let tx_size_sqr_up = tx_size_table_usize(&TX_SIZE_SQR_UP, "Tx_Size_Sqr_Up", tx_size)?;
    if tx_size_sqr_up > TX_32X32 {
        if tx_size_sqr >= TX_32X32 {
            return Ok(TX_SET_DCTONLY);
        }
        let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?;
        let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?;
        return if tx_width > tx_height {
            Ok(TX_SET_WIDE_64)
        } else {
            Ok(TX_SET_HIGH_64)
        };
    }
    if tx_size_sqr_up == TX_32X32 && tx_size_sqr != TX_32X32 {
        let tx_width = tx_size_table_usize(&TX_WIDTH, "Tx_Width", tx_size)?;
        let tx_height = tx_size_table_usize(&TX_HEIGHT, "Tx_Height", tx_size)?;
        return if tx_width > tx_height {
            Ok(TX_SET_WIDE_32)
        } else {
            Ok(TX_SET_HIGH_32)
        };
    }
    if !is_inter && tx_size_sqr_up == TX_32X32 {
        return Ok(TX_SET_DCTONLY);
    }
    let reduced_tx_set = if plane == 0 {
        frame_facts.reduced_tx_set()
    } else {
        usize::from(frame_facts.enable_chroma_dctonly())
    };
    if reduced_tx_set > 3 {
        return unsupported_transform_tool_residual("unsupported_dctonly_residual_reduced_tx_set");
    }
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
        return if tx_size_sqr == 2 {
            Ok(TX_SET_INTER_2)
        } else {
            Ok(TX_SET_INTER_1)
        };
    }
    Ok(TX_SET_INTRA_1)
}

fn tx_size_table_usize(
    table: &[i32],
    table_name: &'static str,
    tx_size: usize,
) -> Result<usize, GeneralIntraResidualError> {
    let value = table
        .get(tx_size)
        .copied()
        .ok_or("unsupported_dctonly_residual_invalid_tx_size")
        .map_err(|reason| GeneralIntraResidualError::UnsupportedTransformToolResidual { reason })?;
    usize::try_from(value).map_err(|_| {
        let reason = match table_name {
            "Tx_Size_Sqr" => "unsupported_dctonly_residual_invalid_tx_size_sqr",
            _ => "unsupported_dctonly_residual_invalid_tx_size_sqr_up",
        };
        GeneralIntraResidualError::UnsupportedTransformToolResidual { reason }
    })
}

fn unsupported_transform_tool_residual<T>(
    reason: &'static str,
) -> Result<T, GeneralIntraResidualError> {
    Err(GeneralIntraResidualError::UnsupportedTransformToolResidual { reason })
}

/// Reconstructs one square intra plane block from the decoded `Quant[]` of its
/// single DC_PRED transform block over a flat DC prediction `dc_sample`.
///
/// This composes the § 7.14.4 dequantization, § 7.15.4 inverse transform, and
/// § 7.14.3 residual addition (`reconstruct_transform_block_residual`) over the
/// flat § 7.13.2 DC prediction (`dc_sample`, derived from the partially-built
/// frame's neighbours, or `128` when none). `qindex == base_q_idx` for this
/// minimal-tool frame (no segmentation or delta-Q), and the § 7.15.4 primary
/// inverse transform uses the supplied `plane_tx_type` (a § 3 `PlaneTxType`
/// index, `DCT_DCT == 0`) over the original `log2_side` (adjusted, capped at 32)
/// dimensions. `use_tcq` adds the § 7.14.4 TCQ `dqDenom` term (luma only). `bit_depth` is the
/// active sequence sample depth (§ 6.4.1); the sample storage type `T` matches it
/// (`u8` for 8-bit, `u16` for 10-bit) and bounds the § 7.14.3 Clip1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block<T: ReconSample>(
    quant: &[i32],
    dc_sample: T,
    qindex: u32,
    plane_id: PlaneId,
    log2_side: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    let orig_side = 1usize << log2_side;
    let prediction = vec![dc_sample; orig_side * orig_side];
    reconstruct_general_intra_block_with_prediction(
        quant,
        &prediction,
        qindex,
        plane_id,
        log2_side,
        plane_tx_type,
        use_tcq,
        bit_depth,
    )
}

/// Reconstructs one square intra plane block from the decoded `Quant[]` of its
/// single transform block over an arbitrary per-sample `prediction` (§ 7.13.2),
/// composing § 7.14.4 dequantization, § 7.15.4 inverse transform, and § 7.14.3
/// residual addition. `prediction` is the predicted block in raster order over
/// the original (unadjusted) `log2_side` dimensions. The flat DC path is the
/// special case where every prediction sample is the DC value (see
/// [`reconstruct_general_intra_block`]); the non-DC § 7.13.2.13 smooth path
/// supplies a per-sample predicted block. `bit_depth` is the active sequence
/// sample depth (§ 6.4.1); the sample storage type `T` matches it and bounds the
/// § 7.14.3 Clip1.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_with_prediction<T: ReconSample>(
    quant: &[i32],
    prediction: &[T],
    qindex: u32,
    plane_id: PlaneId,
    log2_side: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    // A square block is the rectangular case with equal log2 dimensions; the
    // §7.15.4 outer process collapses to the no-adjustment, no-√2-rescale,
    // no-duplication path for `log2_width == log2_height <= 5`.
    reconstruct_general_intra_block_rect_with_prediction(
        quant,
        prediction,
        qindex,
        plane_id,
        log2_side,
        log2_side,
        plane_tx_type,
        use_tcq,
        bit_depth,
    )
}

/// Reconstructs one **rectangular** intra plane block from the decoded `Quant[]`
/// over an arbitrary per-sample `prediction` (§7.13.2), the rectangular
/// generalisation of [`reconstruct_general_intra_block_with_prediction`].
///
/// `prediction` is the predicted block in raster order over the *original*
/// (unadjusted) `1<<log2_width` x `1<<log2_height` dimensions. The flat DC path is
/// the special case where every prediction sample is the §7.13.2.10 DC value; the
/// §7.13.2.12 IBP DC path supplies a per-sample predicted block whose edge
/// rows/columns the IBP modifier has already blended toward the reconstructed
/// neighbours. This composes the §7.14.4
/// dequantization over the adjusted `Min(1<<log2_w, 32) x Min(1<<log2_h, 32)`
/// coefficient grid, the §7.15.4 / §7.15.4.1 inverse transform (resolving the
/// `Transform_1d_Type[PlaneTxType]` row/col kernels for the supplied
/// `plane_tx_type`, plus the `Abs(log2_w - log2_h)` odd-ratio √2 rescale and the
/// over-32 sample duplication), and the §7.14.3 residual addition. Intra passes
/// use `use_ddt == false`. Chroma never uses the §7.14.4 TCQ `dqDenom` term (only
/// the luma 2-D `DCT_DCT` class does), so `use_tcq` is `false` for chroma callers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_rect_with_prediction<T: ReconSample>(
    quant: &[i32],
    prediction: &[T],
    qindex: u32,
    plane_id: PlaneId,
    log2_width: u32,
    log2_height: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    bit_depth: BitDepth,
) -> Result<Vec<T>, GeneralIntraResidualError> {
    let orig_w = 1usize << log2_width;
    let orig_h = 1usize << log2_height;

    let adj_w = 1usize << log2_width.min(5);
    let adj_h = 1usize << log2_height.min(5);
    let adjusted = adj_w * adj_h;
    if quant.len() != adjusted {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: adjusted,
            actual: quant.len(),
        });
    }
    let samples = orig_w * orig_h;
    if prediction.len() != samples {
        return Err(GeneralIntraResidualError::PredictionLength {
            expected: samples,
            actual: prediction.len(),
        });
    }
    let deltas = QuantizerDeltas {
        y_dc: 0,
        u_dc: 0,
        v_dc: 0,
        u_ac: 0,
        v_ac: 0,
    };
    // AV2 §7.14.4: dqDenom = 1 << shift, shift = (pels > 256) + (pels > 1024) over
    // the ORIGINAL (unadjusted) dimensions, plus 1 when TCQ applies (luma DCT_DCT
    // non-lossless non-FSC with allow_tcq; chroma never).
    let pels = (orig_w * orig_h) as u32;
    let dq_shift = u32::from(pels > 256) + u32::from(pels > 1024) + u32::from(use_tcq);
    let dq_denom = 1u32 << dq_shift;
    let params = DequantBlockParams {
        dc_quant: dc_quantizer(plane_id, qindex, deltas, bit_depth),
        ac_quant: ac_quantizer(plane_id, qindex, deltas, bit_depth),
        tx_width: adj_w,
        tx_height: adj_h,
        dq_denom,
        bit_depth,
    };
    // §7.15.4 / §7.15.4.1: the primary inverse transform resolves the per-pass
    // `Transform_1d_Type[PlaneTxType]` kernels for the ACTUAL retained luma
    // tx-type. Intra passes use `use_ddt == false`, so the inter-only DDT/DDTX
    // substitution (transform_params.rs) never applies here.
    let transform = InverseTransform2dOuter::resolve(
        plane_tx_type,
        log2_width,
        log2_height,
        false,
        false,
        bit_depth,
        None,
    )
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;

    let mut dequant_scratch = vec![0i32; adjusted];
    let mut residual_scratch = vec![0i32; samples];
    let mut out = vec![T::default(); samples];
    reconstruct_transform_block_residual(
        prediction,
        quant,
        &params,
        &transform,
        &mut dequant_scratch,
        &mut residual_scratch,
        &mut out,
    )
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;
    Ok(out)
}

/// AV2 § 5.20.7.27 `txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1`
/// over the generated § 9.2 conversion tables, the `txb_skip` CDF transform-size
/// context axis.
fn txb_skip_tx_size_ctx(tx_size: usize) -> usize {
    let sqr = TX_SIZE_SQR.get(tx_size).copied().unwrap_or(0);
    let sqr_up = TX_SIZE_SQR_UP.get(tx_size).copied().unwrap_or(0);
    (((sqr + sqr_up + 1) >> 1).max(0)) as usize
}

#[cfg(test)]
mod tests;
