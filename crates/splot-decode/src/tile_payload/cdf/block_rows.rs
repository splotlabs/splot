// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Minimal block-symbol CDF rows for the traced runtime frontier.
//!
//! Feature tracking: `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER`.

use splot_core::tables::cdf::{
    DEFAULT_COL_MV_GREATER_CDF, DEFAULT_COL_MV_INDEX_CDF, DEFAULT_COMP_GROUP_IDX_CDF,
    DEFAULT_COMP_MODE_CDF, DEFAULT_COMP_REF0_CDF, DEFAULT_COMP_REF1_CDF,
    DEFAULT_COMPOUND_MODE_NON_JOINT_CDF, DEFAULT_CWP_IDX_CDF, DEFAULT_DC_SIGN_CDF,
    DEFAULT_DRL_MODE_CDF, DEFAULT_EOB_EXTRA_CDF, DEFAULT_EOB_PT_16_CDF, DEFAULT_EOB_PT_32_CDF,
    DEFAULT_EOB_PT_64_CDF, DEFAULT_EOB_PT_128_CDF, DEFAULT_EOB_PT_256_CDF, DEFAULT_EOB_PT_512_CDF,
    DEFAULT_EOB_PT_1024_CDF, DEFAULT_INTERP_FILTER_CDF, DEFAULT_IS_INTER_CDF, DEFAULT_IS_JOINT_CDF,
    DEFAULT_JOINT_SHELL_LAST_TWO_CLASSES_CDF, DEFAULT_JOINT_SHELL_SET_CDF,
    DEFAULT_JOINT_SHELL6_CLASS0_CDF, DEFAULT_JOINT_SHELL6_CLASS1_CDF,
    DEFAULT_SHELL_OFFSET_CLASS2_CDF, DEFAULT_SHELL_OFFSET_LOW_CLASS_CDF,
    DEFAULT_SHELL_OFFSET_OTHER_CLASS_CDF, DEFAULT_SINGLE_MODE_CDF, DEFAULT_SINGLE_REF_CDF,
    DEFAULT_SKIP_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_USE_WIENER_NS_CDF,
    DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF, DEFAULT_V_TXB_SKIP_CDF, DEFAULT_Y_MODE_INDEX_CDF,
    DEFAULT_Y_MODE_OFFSET_CDF, DEFAULT_Y_MODE_SET_CDF,
};

use super::coeff_rows::{CoeffCdfRows, CoeffCdfSelector};
use super::{CDF_ROW_LEN, TileCdfArray, TileCdfError, avg_cdf_row, scale_cdf_count};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const Y_MODE_INDEX_CONTEXTS: usize = 3;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
// `Default_Y_Mode_Offset_Cdf[Y_MODE_CONTEXTS][MODE_OFFSET_COUNT + 1]` is
// `[[i32; 7]; 3]`: 6 `y_mode_offset` symbols (MODE_OFFSET_COUNT) plus the count
// slot, sharing the `y_mode_index` 3-context axis.
const Y_MODE_OFFSET_CDF_ROW_LEN: usize = 7;
const Y_MODE_OFFSET_CONTEXTS: usize = 3;
const COEFF_CDF_Q_CONTEXTS: usize = 4;
const EOB_PLANE_CTXS: usize = 3;
const PLANE_TYPES: usize = 2;
const TX_SIZE_CONTEXTS: usize = 5;
const TXB_SKIP_CONTEXTS: usize = 10;
const UV_MODE_CONTEXTS: usize = 2;
const V_TXB_SKIP_CONTEXTS: usize = 12;
const DC_SIGN_GROUPS: usize = 2;
const DC_SIGN_CONTEXTS: usize = 3;
// §3 IS_INTER_CONTEXTS / SKIP_CONTEXTS / SINGLE_MODE_CONTEXTS: the §8.3.2 inter
// mode_info CDF banks consumed by the first-inter-frame decode.
const IS_INTER_CONTEXTS: usize = 4;
const SKIP_CONTEXTS: usize = 6;
const SINGLE_MODE_CONTEXTS: usize = 5;
// §9.3 `Default_Drl_Mode_Cdf[ 3 ][ DRL_MODE_CONTEXTS ][ 3 ]`: the first axis is the
// §5.20.7.8 `read_drl_idx` `Min(idx, 2)` index; the second is `NewMvContext`.
const DRL_MODE_IDX_BANKS: usize = 3;
const DRL_MODE_CONTEXTS: usize = 5;
// §9.3 `Default_Single_Ref_Cdf[ REF_CONTEXTS ][ REFS_PER_FRAME - 1 ][ 3 ]`: the
// §5.20.7.12 `read_single_ref` binary `single_ref` symbol's CDF banks. The first
// axis is the §8.3.2 neighbour-derived `ctx` (`av2_get_ref_pred_context`, the same
// derivation as `comp_ref`); the second is the `read_single_ref` loop counter `ref`
// (`0..NumTotalRefs - 1`).
const REF_CONTEXTS: usize = 3;
const REFS_PER_FRAME_MINUS_1: usize = 6;
// §9.3 compound mode_info banks used by §5.20.7.10 / §5.20.7.6 for the
// two-reference compound-average subset. `comp_mode` and `is_joint` are binary
// rows; `compound_mode_non_joint` has five symbols plus a count slot.
const COMP_MODE_CONTEXTS: usize = 5;
const IS_JOINT_CONTEXTS: usize = 2;
const COMPOUND_MODE_CONTEXTS: usize = 5;
const COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN: usize = 6;
const COMP_GROUP_IDX_CONTEXTS: usize = 12;
const CWP_IDX_CONTEXTS: usize = 4;
const COMP_REF1_BIT_TYPES: usize = 2;
// §9.3 / §8.3.2 SHELL-coded `read_mv` (§5.20.7.20) CDF banks for the verified
// EighthPel (`MvPrecision == MV_PRECISION_EIGHTH_PEL`, P == 6) subset. Only the
// P == 6 `shell_class` bank pair is wired; other precisions are rejected by the
// inter decode before any shell read. The MvCtx axis is single-context (MvCtx ==
// 0), matching the generated single-row §9.3 defaults.
//
// `shell_offset_low_class` is indexed by `shellClass` (0 or 1); `col_mv_greater`
// by the §5.20.7.20 loop counter `i` (0..MAX_COL_TRUNCATED_UNARY_VAL); `col_mv_index`
// by `Min(shellClass, NUM_CTX_COL_MV_INDEX - 1)`; `shell_offset_other_class` by `i`.
const SHELL_OFFSET_LOW_CLASS_BANKS: usize = 2;
const COL_MV_GREATER_BANKS: usize = 2;
const COL_MV_INDEX_BANKS: usize = 4;
const SHELL_OFFSET_OTHER_CLASS_BANKS: usize = 16;
// §8.3.2 `interp_filter`: `TileInterpFilterCdf[ctx]`, `[[i32; 4]; 16]` (3 filter
// symbols + count). The verified single-ref no-neighbour block uses ctx == 3.
const INTERP_FILTER_CONTEXTS: usize = 16;

pub(crate) type YModeSetCdfRow = [i32; Y_MODE_SET_CDF_ROW_LEN];
pub(crate) type YModeIndexCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; Y_MODE_INDEX_CONTEXTS];
pub(crate) type YModeOffsetCdfRows = [[i32; Y_MODE_OFFSET_CDF_ROW_LEN]; Y_MODE_OFFSET_CONTEXTS];
pub(crate) type TxbSkipCdfRows = [[[[[i32; CDF_ROW_LEN]; TXB_SKIP_CONTEXTS]; TX_SIZE_CONTEXTS];
    PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type UvModeCflNotAllowedCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; UV_MODE_CONTEXTS];
pub(crate) type VTxbSkipCdfRows = [[[i32; CDF_ROW_LEN]; V_TXB_SKIP_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
// `eob_extra` is a binary symbol, so its rows are width 3 — the same as the
// generic `CDF_ROW_LEN`. `DEFAULT_EOB_EXTRA_CDF` is `[[i32; 3]; 4]`, so this alias
// must keep an inner width of 3; if `CDF_ROW_LEN` ever changes, give `eob_extra`
// its own width constant rather than over-allocating from the generic one.
pub(crate) type EobExtraCdfRows = [[i32; CDF_ROW_LEN]; COEFF_CDF_Q_CONTEXTS];
// §9.3 `dc_sign`: `[coeff_cdf_q_ctx][plane_type][isHidden group][ctx][3]`. §8.3.2
// reads `TileDcSignCdf[ptype][isHidden][ctx]`; `ctx` (0/1/2) is derived from the
// Above/Left DC-context buffers (deferred with the coeffs() loop).
pub(crate) type DcSignCdfRows =
    [[[[[i32; CDF_ROW_LEN]; DC_SIGN_CONTEXTS]; DC_SIGN_GROUPS]; PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];
// §9.3 inter mode_info banks. `is_inter` / `skip` are binary (width 3 ==
// `CDF_ROW_LEN`); `single_mode` is the 3-ary (NEARMV/GLOBALMV/NEWMV) symbol whose
// `[SINGLE_MODE_CONTEXTS][3 + 1]` row keeps the §9.3 width (3 symbols + count).
pub(crate) type IsInterCdfRows = [[i32; CDF_ROW_LEN]; IS_INTER_CONTEXTS];
pub(crate) type SkipCdfRows = [[i32; CDF_ROW_LEN]; SKIP_CONTEXTS];
pub(crate) type SingleModeCdfRows = [[i32; 4]; SINGLE_MODE_CONTEXTS];
pub(crate) type DrlModeCdfRows = [[[i32; CDF_ROW_LEN]; DRL_MODE_CONTEXTS]; DRL_MODE_IDX_BANKS];
// §9.3 `Default_Single_Ref_Cdf[ REF_CONTEXTS ][ REFS_PER_FRAME - 1 ][ 3 ]`: the
// binary §5.20.7.12 `single_ref` symbol keeps the generic width 3 (`CDF_ROW_LEN`).
pub(crate) type SingleRefCdfRows = [[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; REF_CONTEXTS];
// §9.3 compound inter mode_info banks.
pub(crate) type CompModeCdfRows = [[i32; CDF_ROW_LEN]; COMP_MODE_CONTEXTS];
pub(crate) type IsJointCdfRows = [[i32; CDF_ROW_LEN]; IS_JOINT_CONTEXTS];
pub(crate) type CompoundModeNonJointCdfRows =
    [[i32; COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN]; COMPOUND_MODE_CONTEXTS];
pub(crate) type CompGroupIdxCdfRows = [[i32; CDF_ROW_LEN]; COMP_GROUP_IDX_CONTEXTS];
pub(crate) type CwpIdxCdfRows = [[i32; CDF_ROW_LEN]; CWP_IDX_CONTEXTS];
pub(crate) type CompRef0CdfRows = [[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; REF_CONTEXTS];
pub(crate) type CompRef1CdfRows =
    [[[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; COMP_REF1_BIT_TYPES]; REF_CONTEXTS];
pub(crate) type UseWienerNsCdfRow = [i32; CDF_ROW_LEN];
// §9.3 SHELL-coded `read_mv` banks. `shell_set` / `joint_shell_last_two_classes` /
// `shell_offset_class2` are binary (width 3 == `CDF_ROW_LEN`). The P == 6 EighthPel
// `shell_class` banks are width 9 (8 shell-class symbols + count). The offset / col
// banks keep their generated `[i32; 3]` binary widths.
pub(crate) type JointShellSetCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type JointShell6ClassCdfRow = [i32; 9];
pub(crate) type JointShellLastTwoCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type ShellOffsetLowClassCdfRows = [[i32; CDF_ROW_LEN]; SHELL_OFFSET_LOW_CLASS_BANKS];
pub(crate) type ShellOffsetClass2CdfRow = [i32; CDF_ROW_LEN];
pub(crate) type ShellOffsetOtherClassCdfRows = [[i32; CDF_ROW_LEN]; SHELL_OFFSET_OTHER_CLASS_BANKS];
pub(crate) type ColMvGreaterCdfRows = [[i32; CDF_ROW_LEN]; COL_MV_GREATER_BANKS];
pub(crate) type ColMvIndexCdfRows = [[i32; CDF_ROW_LEN]; COL_MV_INDEX_BANKS];
// §9.3 `interp_filter`: `[[i32; 4]; 16]` (3 symbols + count).
pub(crate) type InterpFilterCdfRows = [[i32; 4]; INTERP_FILTER_CONTEXTS];

// The §9.3 `eob_pt` CDF family: one bank per transform-size class, each
// `[coeff_cdf_q_ctx][eobCtx][N]` with a class-specific symbol width N. §8.3.2
// selects `TileEobPt<size>Cdf[eobCtx]` for the active q-context.
pub(crate) type EobPt16CdfRows = [[[i32; 6]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt32CdfRows = [[[i32; 7]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt64CdfRows = [[[i32; 8]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt128CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt256CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt512CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt1024CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];

/// The AV2 `eob_pt` transform-size class, selecting which `TileEobPt<size>Cdf`
/// family bank the §8.3.2 `eob_pt` symbol reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EobPtSize {
    /// `TileEobPt16Cdf`.
    Pt16,
    /// `TileEobPt32Cdf`.
    Pt32,
    /// `TileEobPt64Cdf`.
    Pt64,
    /// `TileEobPt128Cdf`.
    Pt128,
    /// `TileEobPt256Cdf`.
    Pt256,
    /// `TileEobPt512Cdf`.
    Pt512,
    /// `TileEobPt1024Cdf`.
    Pt1024,
}

/// Block-symbol CDF selectors handled by this focused row bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockCdfSelector {
    /// `TileYModeSetCdf`.
    YModeSet,
    /// `TileYModeIndexCdf[ctx]`.
    YModeIndex {
        /// Intra mode context index.
        ctx: usize,
    },
    /// `TileYModeOffsetCdf[ctx]`.
    YModeOffset {
        /// Intra mode context index (shares the `y_mode_index` § 8.3.2 context).
        ctx: usize,
    },
    /// `TileTxbSkipCdf[coeff_cdf_q_ctx][plane_type][tx_size][ctx]`.
    TxbSkip {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Plane type context.
        plane_type: usize,
        /// Transform-size context.
        tx_size: usize,
        /// Transform-skip context index.
        ctx: usize,
    },
    /// `TileUvModeCflNotAllowedCdf[ctx]`.
    UvModeCflNotAllowed {
        /// Chroma mode context index.
        ctx: usize,
    },
    /// `TileVTxbSkipCdf[coeff_cdf_q_ctx][ctx]`.
    VTxbSkip {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// V-plane transform-skip context index.
        ctx: usize,
    },
    /// `TileEobExtraCdf[coeff_cdf_q_ctx]` (AV2 § 8.3.2: the cdf is given by
    /// `TileEobExtraCdf` directly, with no per-symbol context).
    EobExtra {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
    },
    /// `TileEobPt<size>Cdf[coeff_cdf_q_ctx][eobCtx]` (AV2 § 8.3.2): the
    /// transform-size class selects the bank and `eobCtx` selects the row.
    EobPt {
        /// Transform-size class selecting the `eob_pt` family bank.
        size: EobPtSize,
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// `eobCtx = (plane > 0) ? 2 : is_inter` (`0..EOB_PLANE_CTXS`).
        eob_ctx: usize,
    },
    /// `TileDcSignCdf[coeff_cdf_q_ctx][plane_type][group][ctx]` (AV2 § 8.3.2):
    /// `group` is the spec `isHidden` flag; `ctx` (`0..DC_SIGN_CONTEXTS`) is the
    /// caller-resolved DC-sign context.
    DcSign {
        /// Coefficient-CDF quantization context.
        coeff_cdf_q_ctx: usize,
        /// Plane type context (luma vs chroma).
        plane_type: usize,
        /// `isHidden` group (`0..DC_SIGN_GROUPS`).
        group: usize,
        /// DC-sign context (`0..DC_SIGN_CONTEXTS`).
        ctx: usize,
    },
    /// `TileIsInterCdf[ctx]` (AV2 § 8.3.2): the `read_is_inter` decision.
    IsInter {
        /// `is_inter` context index (`0..IS_INTER_CONTEXTS`).
        ctx: usize,
    },
    /// `TileSkipCdf[ctx]` (AV2 § 8.3.2): the `read_skip` decision.
    Skip {
        /// `skip_flag` context index (`0..SKIP_CONTEXTS`).
        ctx: usize,
    },
    /// `TileSingleModeCdf[NewMvContext]` (AV2 § 8.3.2): the single-reference inter
    /// `single_mode` symbol (`YMode = NEARMV + single_mode`).
    SingleMode {
        /// `NewMvContext` (`0..SINGLE_MODE_CONTEXTS`).
        ctx: usize,
    },
    /// `TileDrlModeCdf[Min(idx, 2)][NewMvContext]` (AV2 § 8.3.2): the §5.20.7.8
    /// `read_drl_idx` `drl_mode` symbol for a non-skip-mode, non-TIP reference.
    DrlMode {
        /// `Min(idx, 2)` index bank (`0..DRL_MODE_IDX_BANKS`).
        idx: usize,
        /// `NewMvContext` (`0..DRL_MODE_CONTEXTS`).
        ctx: usize,
    },
    /// `TileSingleRefCdf[ctx][ref]` (AV2 § 8.3.2): the §5.20.7.12 `read_single_ref`
    /// binary `single_ref` symbol for a per-decision context/loop-index pair.
    SingleRef {
        /// §8.3.2 neighbour-derived single_ref context (`0..REF_CONTEXTS`).
        ctx: usize,
        /// The §5.20.7.12 loop counter `ref` (`0..REFS_PER_FRAME_MINUS_1`).
        ref_idx: usize,
    },
    /// `TileCompModeCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.10 `comp_mode` symbol.
    CompMode {
        /// §8.3.2 compound-reference mode context (`0..COMP_MODE_CONTEXTS`).
        ctx: usize,
    },
    /// `TileIsJointCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.6 `is_joint` symbol.
    IsJoint {
        /// §8.3.2 `is_joint` context (`0..IS_JOINT_CONTEXTS`).
        ctx: usize,
    },
    /// `TileCompoundModeNonJointCdf[NewMvContext]` (AV2 § 8.3.2): the
    /// §5.20.7.6 non-joint compound mode symbol.
    CompoundModeNonJoint {
        /// `NewMvContext` (`0..COMPOUND_MODE_CONTEXTS`).
        ctx: usize,
    },
    /// `TileCompGroupIdxCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.16
    /// `comp_group_idx` symbol.
    CompGroupIdx {
        /// `comp_group_idx` context (`0..COMP_GROUP_IDX_CONTEXTS`).
        ctx: usize,
    },
    /// `TileCwpIdxCdf[idx]` (AV2 § 8.3.2): the §5.20.7.6 `cwp_idx` symbol.
    CwpIdx {
        /// CWP truncated-unary index (`0..CWP_IDX_CONTEXTS`).
        idx: usize,
    },
    /// `TileCompRef0Cdf[ctx][ref]` (AV2 § 8.3.2): the §5.20.7.11 `comp_ref`
    /// symbol before any compound reference has been found.
    CompRef0 {
        /// §8.3.2 neighbour-derived comp_ref context (`0..REF_CONTEXTS`).
        ctx: usize,
        /// The §5.20.7.11 loop counter `ref` (`0..REFS_PER_FRAME_MINUS_1`).
        ref_idx: usize,
    },
    /// `TileCompRef1Cdf[ctx][bitType][ref]` (AV2 § 8.3.2): the §5.20.7.11
    /// `comp_ref` symbol after the first compound reference has been found.
    CompRef1 {
        /// §8.3.2 neighbour-derived comp_ref context (`0..REF_CONTEXTS`).
        ctx: usize,
        /// Same-side/opposite-side bit type (`0..COMP_REF1_BIT_TYPES`).
        bit_type: usize,
        /// The §5.20.7.11 loop counter `ref` (`0..REFS_PER_FRAME_MINUS_1`).
        ref_idx: usize,
    },
    /// `TileJointShellSetCdf[MvCtx]` (AV2 § 8.3.2): the §5.20.7.20 `shell_set`
    /// binary symbol (MvCtx == 0 — single-context).
    JointShellSet,
    /// `TileJointShell6ClassCdf[MvCtx][shell_set]` (AV2 § 8.3.2): the §5.20.7.20
    /// `shell_class` symbol for the verified EighthPel (P == 6) precision.
    /// `shell_set` selects between the `Class0` / `Class1` banks.
    JointShell6Class {
        /// `Q == shell_set` (`0..2`).
        shell_set: usize,
    },
    /// `TileJointShellLastTwoClassesCdf[MvCtx]` (AV2 § 8.3.2): the EighthPel
    /// `joint_shell_last_two_classes` binary symbol.
    JointShellLastTwo,
    /// `TileShellOffsetLowClassCdf[MvCtx][shellClass]` (AV2 § 8.3.2): the
    /// `shell_offset_low_class` symbol for `shellClass < 2`.
    ShellOffsetLowClass {
        /// `shellClass` (`0..SHELL_OFFSET_LOW_CLASS_BANKS`).
        shell_class: usize,
    },
    /// `TileShellOffsetClass2Cdf[MvCtx]` (AV2 § 8.3.2): the `shell_offset_class2`
    /// binary symbol for `shellClass == 2`.
    ShellOffsetClass2,
    /// `TileShellOffsetOtherClassCdf[MvCtx][i]` (AV2 § 8.3.2): the
    /// `shell_offset_other_class` symbol for `shellClass > 2`, bank `i`.
    ShellOffsetOtherClass {
        /// The §5.20.7.20 loop counter `i` (`0..SHELL_OFFSET_OTHER_CLASS_BANKS`).
        i: usize,
    },
    /// `TileColMvGreaterCdf[MvCtx][i]` (AV2 § 8.3.2): the `col_mv_greater` symbol,
    /// bank `i` (the truncated-unary loop counter).
    ColMvGreater {
        /// The §5.20.7.20 loop counter `i` (`0..COL_MV_GREATER_BANKS`).
        i: usize,
    },
    /// `TileColMvIndexCdf[MvCtx][Min(shellClass, NUM_CTX_COL_MV_INDEX - 1)]`
    /// (AV2 § 8.3.2): the `col_mv_index` symbol.
    ColMvIndex {
        /// `Min(shellClass, NUM_CTX_COL_MV_INDEX - 1)` (`0..COL_MV_INDEX_BANKS`).
        ctx: usize,
    },
    /// `TileInterpFilterCdf[ctx]` (AV2 § 8.3.2): the §5.20.7.6 `interp_filter`
    /// SWITCHABLE symbol.
    InterpFilter {
        /// The §8.3.2 interp-filter context (`0..INTERP_FILTER_CONTEXTS`).
        ctx: usize,
    },
    /// `TileUseWienerNsCdf` (AV2 § 8.3.2): the §5.20.10.5 `use_wiener_ns`
    /// binary symbol.
    UseWienerNs,
    /// Coefficient base/base-EOB/base-range and IDTX CDF rows.
    Coeff(CoeffCdfSelector),
}

/// Supported block-symbol CDF arrays for the minimal flat-intra trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockCdfRows {
    pub(super) y_mode_set: YModeSetCdfRow,
    pub(super) y_mode_index: YModeIndexCdfRows,
    pub(super) y_mode_offset: YModeOffsetCdfRows,
    pub(super) txb_skip: TxbSkipCdfRows,
    pub(super) uv_mode_cfl_not_allowed: UvModeCflNotAllowedCdfRows,
    pub(super) v_txb_skip: VTxbSkipCdfRows,
    pub(super) eob_extra: EobExtraCdfRows,
    pub(super) eob_pt_16: EobPt16CdfRows,
    pub(super) eob_pt_32: EobPt32CdfRows,
    pub(super) eob_pt_64: EobPt64CdfRows,
    pub(super) eob_pt_128: EobPt128CdfRows,
    pub(super) eob_pt_256: EobPt256CdfRows,
    pub(super) eob_pt_512: EobPt512CdfRows,
    pub(super) eob_pt_1024: EobPt1024CdfRows,
    pub(super) dc_sign: DcSignCdfRows,
    pub(super) is_inter: IsInterCdfRows,
    pub(super) skip: SkipCdfRows,
    pub(super) single_mode: SingleModeCdfRows,
    pub(super) drl_mode: DrlModeCdfRows,
    pub(super) single_ref: SingleRefCdfRows,
    pub(super) comp_mode: CompModeCdfRows,
    pub(super) is_joint: IsJointCdfRows,
    pub(super) compound_mode_non_joint: CompoundModeNonJointCdfRows,
    pub(super) comp_group_idx: CompGroupIdxCdfRows,
    pub(super) cwp_idx: CwpIdxCdfRows,
    pub(super) comp_ref0: CompRef0CdfRows,
    pub(super) comp_ref1: CompRef1CdfRows,
    pub(super) joint_shell_set: JointShellSetCdfRow,
    pub(super) joint_shell6_class0: JointShell6ClassCdfRow,
    pub(super) joint_shell6_class1: JointShell6ClassCdfRow,
    pub(super) joint_shell_last_two: JointShellLastTwoCdfRow,
    pub(super) shell_offset_low_class: ShellOffsetLowClassCdfRows,
    pub(super) shell_offset_class2: ShellOffsetClass2CdfRow,
    pub(super) shell_offset_other_class: ShellOffsetOtherClassCdfRows,
    pub(super) col_mv_greater: ColMvGreaterCdfRows,
    pub(super) col_mv_index: ColMvIndexCdfRows,
    pub(super) interp_filter: InterpFilterCdfRows,
    pub(super) use_wiener_ns: UseWienerNsCdfRow,
    pub(super) coeff: CoeffCdfRows,
}

impl BlockCdfRows {
    pub(crate) fn from_defaults() -> Self {
        Self {
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index: DEFAULT_Y_MODE_INDEX_CDF,
            y_mode_offset: DEFAULT_Y_MODE_OFFSET_CDF,
            txb_skip: DEFAULT_TXB_SKIP_CDF,
            uv_mode_cfl_not_allowed: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
            v_txb_skip: DEFAULT_V_TXB_SKIP_CDF,
            eob_extra: DEFAULT_EOB_EXTRA_CDF,
            eob_pt_16: DEFAULT_EOB_PT_16_CDF,
            eob_pt_32: DEFAULT_EOB_PT_32_CDF,
            eob_pt_64: DEFAULT_EOB_PT_64_CDF,
            eob_pt_128: DEFAULT_EOB_PT_128_CDF,
            eob_pt_256: DEFAULT_EOB_PT_256_CDF,
            eob_pt_512: DEFAULT_EOB_PT_512_CDF,
            eob_pt_1024: DEFAULT_EOB_PT_1024_CDF,
            dc_sign: DEFAULT_DC_SIGN_CDF,
            is_inter: DEFAULT_IS_INTER_CDF,
            skip: DEFAULT_SKIP_CDF,
            single_mode: DEFAULT_SINGLE_MODE_CDF,
            drl_mode: DEFAULT_DRL_MODE_CDF,
            single_ref: DEFAULT_SINGLE_REF_CDF,
            comp_mode: DEFAULT_COMP_MODE_CDF,
            is_joint: DEFAULT_IS_JOINT_CDF,
            compound_mode_non_joint: DEFAULT_COMPOUND_MODE_NON_JOINT_CDF,
            comp_group_idx: DEFAULT_COMP_GROUP_IDX_CDF,
            cwp_idx: DEFAULT_CWP_IDX_CDF,
            comp_ref0: DEFAULT_COMP_REF0_CDF,
            comp_ref1: DEFAULT_COMP_REF1_CDF,
            joint_shell_set: DEFAULT_JOINT_SHELL_SET_CDF,
            joint_shell6_class0: DEFAULT_JOINT_SHELL6_CLASS0_CDF,
            joint_shell6_class1: DEFAULT_JOINT_SHELL6_CLASS1_CDF,
            joint_shell_last_two: DEFAULT_JOINT_SHELL_LAST_TWO_CLASSES_CDF,
            shell_offset_low_class: DEFAULT_SHELL_OFFSET_LOW_CLASS_CDF,
            shell_offset_class2: DEFAULT_SHELL_OFFSET_CLASS2_CDF,
            shell_offset_other_class: DEFAULT_SHELL_OFFSET_OTHER_CLASS_CDF,
            col_mv_greater: DEFAULT_COL_MV_GREATER_CDF,
            col_mv_index: DEFAULT_COL_MV_INDEX_CDF,
            interp_filter: DEFAULT_INTERP_FILTER_CDF,
            use_wiener_ns: DEFAULT_USE_WIENER_NS_CDF,
            coeff: CoeffCdfRows::from_defaults(),
        }
    }

    pub(crate) fn row(&self, selector: BlockCdfSelector) -> Result<&[i32], TileCdfError> {
        match selector {
            BlockCdfSelector::YModeSet => Ok(self.y_mode_set.as_slice()),
            BlockCdfSelector::YModeIndex { ctx } => {
                let row = self
                    .y_mode_index
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::YModeIndex,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: Y_MODE_INDEX_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::YModeOffset { ctx } => {
                let row = self
                    .y_mode_offset
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::YModeOffset,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: Y_MODE_OFFSET_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::TxbSkip, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::TxbSkip, plane_type)?;
                let tx_size = checked_tx_size(TileCdfArray::TxbSkip, tx_size)?;
                let row = self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size]
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::TxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: TXB_SKIP_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::UvModeCflNotAllowed { ctx } => {
                let row = self.uv_mode_cfl_not_allowed.get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::UvModeCflNotAllowed,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: UV_MODE_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::VTxbSkip, coeff_cdf_q_ctx)?;
                let row = self.v_txb_skip[coeff_cdf_q_ctx].get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::VTxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: V_TXB_SKIP_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::EobExtra { coeff_cdf_q_ctx } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::EobExtra, coeff_cdf_q_ctx)?;
                Ok(self.eob_extra[coeff_cdf_q_ctx].as_slice())
            }
            BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::EobPt, coeff_cdf_q_ctx)?;
                let c = checked_eob_plane_ctx(eob_ctx)?;
                Ok(match size {
                    EobPtSize::Pt16 => self.eob_pt_16[q][c].as_slice(),
                    EobPtSize::Pt32 => self.eob_pt_32[q][c].as_slice(),
                    EobPtSize::Pt64 => self.eob_pt_64[q][c].as_slice(),
                    EobPtSize::Pt128 => self.eob_pt_128[q][c].as_slice(),
                    EobPtSize::Pt256 => self.eob_pt_256[q][c].as_slice(),
                    EobPtSize::Pt512 => self.eob_pt_512[q][c].as_slice(),
                    EobPtSize::Pt1024 => self.eob_pt_1024[q][c].as_slice(),
                })
            }
            BlockCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::DcSign, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::DcSign, plane_type)?;
                let group = checked_dc_sign_group(group)?;
                let row = self.dc_sign[q][plane_type][group].get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DcSign,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: DC_SIGN_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::IsInter { ctx } => {
                let row = self
                    .is_inter
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::IsInter,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: IS_INTER_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::Skip { ctx } => {
                let row = self.skip.get(ctx).ok_or(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::Skip,
                    index_name: "ctx",
                    actual: ctx,
                    max_exclusive: SKIP_CONTEXTS,
                })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::SingleMode { ctx } => {
                let row = self
                    .single_mode
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::SingleMode,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: SINGLE_MODE_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::DrlMode { idx, ctx } => {
                let bank = self
                    .drl_mode
                    .get(idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DrlMode,
                        index_name: "idx",
                        actual: idx,
                        max_exclusive: DRL_MODE_IDX_BANKS,
                    })?;
                let row = bank.get(ctx).ok_or(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::DrlMode,
                    index_name: "ctx",
                    actual: ctx,
                    max_exclusive: DRL_MODE_CONTEXTS,
                })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::SingleRef { ctx, ref_idx } => {
                let bank = self
                    .single_ref
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::SingleRef,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: REF_CONTEXTS,
                    })?;
                let row = bank.get(ref_idx).ok_or(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::SingleRef,
                    index_name: "ref",
                    actual: ref_idx,
                    max_exclusive: REFS_PER_FRAME_MINUS_1,
                })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::CompMode { ctx } => {
                let row = self
                    .comp_mode
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompMode,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: COMP_MODE_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::IsJoint { ctx } => {
                let row = self
                    .is_joint
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::IsJoint,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: IS_JOINT_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::CompoundModeNonJoint { ctx } => {
                let row = self.compound_mode_non_joint.get(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompoundModeNonJoint,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: COMPOUND_MODE_CONTEXTS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::CompGroupIdx { ctx } => {
                let row = self
                    .comp_group_idx
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompGroupIdx,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: COMP_GROUP_IDX_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::CwpIdx { idx } => {
                let row = self
                    .cwp_idx
                    .get(idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CwpIdx,
                        index_name: "idx",
                        actual: idx,
                        max_exclusive: CWP_IDX_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::CompRef0 { ctx, ref_idx } => {
                let bank = self
                    .comp_ref0
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef0,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: REF_CONTEXTS,
                    })?;
                let row = bank.get(ref_idx).ok_or(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::CompRef0,
                    index_name: "ref",
                    actual: ref_idx,
                    max_exclusive: REFS_PER_FRAME_MINUS_1,
                })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => {
                let bank = self
                    .comp_ref1
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef1,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: REF_CONTEXTS,
                    })?;
                let bit_bank = bank.get(bit_type).ok_or(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::CompRef1,
                    index_name: "bit_type",
                    actual: bit_type,
                    max_exclusive: COMP_REF1_BIT_TYPES,
                })?;
                let row = bit_bank
                    .get(ref_idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef1,
                        index_name: "ref",
                        actual: ref_idx,
                        max_exclusive: REFS_PER_FRAME_MINUS_1,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::JointShellSet => Ok(self.joint_shell_set.as_slice()),
            BlockCdfSelector::JointShell6Class { shell_set } => match shell_set {
                0 => Ok(self.joint_shell6_class0.as_slice()),
                1 => Ok(self.joint_shell6_class1.as_slice()),
                _ => Err(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::JointShell6Class,
                    index_name: "shell_set",
                    actual: shell_set,
                    max_exclusive: 2,
                }),
            },
            BlockCdfSelector::JointShellLastTwo => Ok(self.joint_shell_last_two.as_slice()),
            BlockCdfSelector::ShellOffsetLowClass { shell_class } => {
                let row = self.shell_offset_low_class.get(shell_class).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::ShellOffsetLowClass,
                        index_name: "shell_class",
                        actual: shell_class,
                        max_exclusive: SHELL_OFFSET_LOW_CLASS_BANKS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::ShellOffsetClass2 => Ok(self.shell_offset_class2.as_slice()),
            BlockCdfSelector::ShellOffsetOtherClass { i } => {
                let row = self.shell_offset_other_class.get(i).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::ShellOffsetOtherClass,
                        index_name: "i",
                        actual: i,
                        max_exclusive: SHELL_OFFSET_OTHER_CLASS_BANKS,
                    },
                )?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::ColMvGreater { i } => {
                let row = self
                    .col_mv_greater
                    .get(i)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::ColMvGreater,
                        index_name: "i",
                        actual: i,
                        max_exclusive: COL_MV_GREATER_BANKS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::ColMvIndex { ctx } => {
                let row = self
                    .col_mv_index
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::ColMvIndex,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: COL_MV_INDEX_BANKS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::InterpFilter { ctx } => {
                let row = self
                    .interp_filter
                    .get(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::InterpFilter,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: INTERP_FILTER_CONTEXTS,
                    })?;
                Ok(row.as_slice())
            }
            BlockCdfSelector::UseWienerNs => Ok(self.use_wiener_ns.as_slice()),
            BlockCdfSelector::Coeff(selector) => self.coeff.row(selector),
        }
    }

    pub(crate) fn row_mut(
        &mut self,
        selector: BlockCdfSelector,
    ) -> Result<&mut [i32], TileCdfError> {
        match selector {
            BlockCdfSelector::YModeSet => Ok(self.y_mode_set.as_mut_slice()),
            BlockCdfSelector::YModeIndex { ctx } => {
                let max_exclusive = self.y_mode_index.len();
                let row =
                    self.y_mode_index
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::YModeIndex,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::YModeOffset { ctx } => {
                let max_exclusive = self.y_mode_offset.len();
                let row =
                    self.y_mode_offset
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::YModeOffset,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type,
                tx_size,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::TxbSkip, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::TxbSkip, plane_type)?;
                let tx_size = checked_tx_size(TileCdfArray::TxbSkip, tx_size)?;
                let max_exclusive = self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size].len();
                let row = self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size]
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::TxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::UvModeCflNotAllowed { ctx } => {
                let max_exclusive = self.uv_mode_cfl_not_allowed.len();
                let row = self.uv_mode_cfl_not_allowed.get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::UvModeCflNotAllowed,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::VTxbSkip, coeff_cdf_q_ctx)?;
                let max_exclusive = self.v_txb_skip[coeff_cdf_q_ctx].len();
                let row = self.v_txb_skip[coeff_cdf_q_ctx].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::VTxbSkip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::EobExtra { coeff_cdf_q_ctx } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::EobExtra, coeff_cdf_q_ctx)?;
                Ok(self.eob_extra[coeff_cdf_q_ctx].as_mut_slice())
            }
            BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::EobPt, coeff_cdf_q_ctx)?;
                let c = checked_eob_plane_ctx(eob_ctx)?;
                Ok(match size {
                    EobPtSize::Pt16 => self.eob_pt_16[q][c].as_mut_slice(),
                    EobPtSize::Pt32 => self.eob_pt_32[q][c].as_mut_slice(),
                    EobPtSize::Pt64 => self.eob_pt_64[q][c].as_mut_slice(),
                    EobPtSize::Pt128 => self.eob_pt_128[q][c].as_mut_slice(),
                    EobPtSize::Pt256 => self.eob_pt_256[q][c].as_mut_slice(),
                    EobPtSize::Pt512 => self.eob_pt_512[q][c].as_mut_slice(),
                    EobPtSize::Pt1024 => self.eob_pt_1024[q][c].as_mut_slice(),
                })
            }
            BlockCdfSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type,
                group,
                ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::DcSign, coeff_cdf_q_ctx)?;
                let plane_type = checked_plane_type(TileCdfArray::DcSign, plane_type)?;
                let group = checked_dc_sign_group(group)?;
                let row = self.dc_sign[q][plane_type][group].get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DcSign,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: DC_SIGN_CONTEXTS,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::IsInter { ctx } => {
                let max_exclusive = self.is_inter.len();
                let row = self
                    .is_inter
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::IsInter,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::Skip { ctx } => {
                let max_exclusive = self.skip.len();
                let row = self
                    .skip
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::Skip,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::SingleMode { ctx } => {
                let max_exclusive = self.single_mode.len();
                let row =
                    self.single_mode
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::SingleMode,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::DrlMode { idx, ctx } => {
                let bank_len = self.drl_mode.len();
                let bank = self
                    .drl_mode
                    .get_mut(idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::DrlMode,
                        index_name: "idx",
                        actual: idx,
                        max_exclusive: bank_len,
                    })?;
                let ctx_len = bank.len();
                let row = bank.get_mut(ctx).ok_or(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::DrlMode,
                    index_name: "ctx",
                    actual: ctx,
                    max_exclusive: ctx_len,
                })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::SingleRef { ctx, ref_idx } => {
                let bank_len = self.single_ref.len();
                let bank =
                    self.single_ref
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::SingleRef,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive: bank_len,
                        })?;
                let ref_len = bank.len();
                let row = bank
                    .get_mut(ref_idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::SingleRef,
                        index_name: "ref",
                        actual: ref_idx,
                        max_exclusive: ref_len,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::CompMode { ctx } => {
                let max_exclusive = self.comp_mode.len();
                let row = self
                    .comp_mode
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompMode,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::IsJoint { ctx } => {
                let max_exclusive = self.is_joint.len();
                let row = self
                    .is_joint
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::IsJoint,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::CompoundModeNonJoint { ctx } => {
                let max_exclusive = self.compound_mode_non_joint.len();
                let row = self.compound_mode_non_joint.get_mut(ctx).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompoundModeNonJoint,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::CompGroupIdx { ctx } => {
                let max_exclusive = self.comp_group_idx.len();
                let row =
                    self.comp_group_idx
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::CompGroupIdx,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::CwpIdx { idx } => {
                let max_exclusive = self.cwp_idx.len();
                let row = self
                    .cwp_idx
                    .get_mut(idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CwpIdx,
                        index_name: "idx",
                        actual: idx,
                        max_exclusive,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::CompRef0 { ctx, ref_idx } => {
                let bank_len = self.comp_ref0.len();
                let bank = self
                    .comp_ref0
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef0,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: bank_len,
                    })?;
                let ref_len = bank.len();
                let row = bank
                    .get_mut(ref_idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef0,
                        index_name: "ref",
                        actual: ref_idx,
                        max_exclusive: ref_len,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => {
                let bank_len = self.comp_ref1.len();
                let bank = self
                    .comp_ref1
                    .get_mut(ctx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef1,
                        index_name: "ctx",
                        actual: ctx,
                        max_exclusive: bank_len,
                    })?;
                let bit_len = bank.len();
                let bit_bank = bank
                    .get_mut(bit_type)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef1,
                        index_name: "bit_type",
                        actual: bit_type,
                        max_exclusive: bit_len,
                    })?;
                let ref_len = bit_bank.len();
                let row = bit_bank
                    .get_mut(ref_idx)
                    .ok_or(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::CompRef1,
                        index_name: "ref",
                        actual: ref_idx,
                        max_exclusive: ref_len,
                    })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::JointShellSet => Ok(self.joint_shell_set.as_mut_slice()),
            BlockCdfSelector::JointShell6Class { shell_set } => match shell_set {
                0 => Ok(self.joint_shell6_class0.as_mut_slice()),
                1 => Ok(self.joint_shell6_class1.as_mut_slice()),
                _ => Err(TileCdfError::SelectorOutOfRange {
                    array: TileCdfArray::JointShell6Class,
                    index_name: "shell_set",
                    actual: shell_set,
                    max_exclusive: 2,
                }),
            },
            BlockCdfSelector::JointShellLastTwo => Ok(self.joint_shell_last_two.as_mut_slice()),
            BlockCdfSelector::ShellOffsetLowClass { shell_class } => {
                let max_exclusive = self.shell_offset_low_class.len();
                let row = self.shell_offset_low_class.get_mut(shell_class).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::ShellOffsetLowClass,
                        index_name: "shell_class",
                        actual: shell_class,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::ShellOffsetClass2 => Ok(self.shell_offset_class2.as_mut_slice()),
            BlockCdfSelector::ShellOffsetOtherClass { i } => {
                let max_exclusive = self.shell_offset_other_class.len();
                let row = self.shell_offset_other_class.get_mut(i).ok_or(
                    TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::ShellOffsetOtherClass,
                        index_name: "i",
                        actual: i,
                        max_exclusive,
                    },
                )?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::ColMvGreater { i } => {
                let max_exclusive = self.col_mv_greater.len();
                let row =
                    self.col_mv_greater
                        .get_mut(i)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::ColMvGreater,
                            index_name: "i",
                            actual: i,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::ColMvIndex { ctx } => {
                let max_exclusive = self.col_mv_index.len();
                let row =
                    self.col_mv_index
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::ColMvIndex,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::InterpFilter { ctx } => {
                let max_exclusive = self.interp_filter.len();
                let row =
                    self.interp_filter
                        .get_mut(ctx)
                        .ok_or(TileCdfError::SelectorOutOfRange {
                            array: TileCdfArray::InterpFilter,
                            index_name: "ctx",
                            actual: ctx,
                            max_exclusive,
                        })?;
                Ok(row.as_mut_slice())
            }
            BlockCdfSelector::UseWienerNs => Ok(self.use_wiener_ns.as_mut_slice()),
            BlockCdfSelector::Coeff(selector) => self.coeff.row_mut(selector),
        }
    }

    pub(crate) fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        avg_cdf_row(&mut self.y_mode_set, &tile.y_mode_set, tile_num, num_log2);
        for ctx in 0..Y_MODE_INDEX_CONTEXTS {
            avg_cdf_row(
                &mut self.y_mode_index[ctx],
                &tile.y_mode_index[ctx],
                tile_num,
                num_log2,
            );
        }
        for coeff_cdf_q_ctx in 0..COEFF_CDF_Q_CONTEXTS {
            for plane_type in 0..PLANE_TYPES {
                for tx_size in 0..TX_SIZE_CONTEXTS {
                    for ctx in 0..TXB_SKIP_CONTEXTS {
                        avg_cdf_row(
                            &mut self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size][ctx],
                            &tile.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size][ctx],
                            tile_num,
                            num_log2,
                        );
                    }
                }
            }
            for ctx in 0..V_TXB_SKIP_CONTEXTS {
                avg_cdf_row(
                    &mut self.v_txb_skip[coeff_cdf_q_ctx][ctx],
                    &tile.v_txb_skip[coeff_cdf_q_ctx][ctx],
                    tile_num,
                    num_log2,
                );
            }
            avg_cdf_row(
                &mut self.eob_extra[coeff_cdf_q_ctx],
                &tile.eob_extra[coeff_cdf_q_ctx],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..UV_MODE_CONTEXTS {
            avg_cdf_row(
                &mut self.uv_mode_cfl_not_allowed[ctx],
                &tile.uv_mode_cfl_not_allowed[ctx],
                tile_num,
                num_log2,
            );
        }
        avg_eob_pt_bank(&mut self.eob_pt_16, &tile.eob_pt_16, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_32, &tile.eob_pt_32, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_64, &tile.eob_pt_64, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_128, &tile.eob_pt_128, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_256, &tile.eob_pt_256, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_512, &tile.eob_pt_512, tile_num, num_log2);
        avg_eob_pt_bank(&mut self.eob_pt_1024, &tile.eob_pt_1024, tile_num, num_log2);
        for (frame_row, tile_row) in self
            .dc_sign
            .iter_mut()
            .flatten()
            .flatten()
            .flatten()
            .zip(tile.dc_sign.iter().flatten().flatten().flatten())
        {
            avg_cdf_row(frame_row, tile_row, tile_num, num_log2);
        }
        for ctx in 0..IS_INTER_CONTEXTS {
            avg_cdf_row(
                &mut self.is_inter[ctx],
                &tile.is_inter[ctx],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..SKIP_CONTEXTS {
            avg_cdf_row(&mut self.skip[ctx], &tile.skip[ctx], tile_num, num_log2);
        }
        for ctx in 0..SINGLE_MODE_CONTEXTS {
            avg_cdf_row(
                &mut self.single_mode[ctx],
                &tile.single_mode[ctx],
                tile_num,
                num_log2,
            );
        }
        for idx in 0..DRL_MODE_IDX_BANKS {
            for ctx in 0..DRL_MODE_CONTEXTS {
                avg_cdf_row(
                    &mut self.drl_mode[idx][ctx],
                    &tile.drl_mode[idx][ctx],
                    tile_num,
                    num_log2,
                );
            }
        }
        for ctx in 0..REF_CONTEXTS {
            for ref_idx in 0..REFS_PER_FRAME_MINUS_1 {
                avg_cdf_row(
                    &mut self.single_ref[ctx][ref_idx],
                    &tile.single_ref[ctx][ref_idx],
                    tile_num,
                    num_log2,
                );
            }
        }
        for ctx in 0..COMP_MODE_CONTEXTS {
            avg_cdf_row(
                &mut self.comp_mode[ctx],
                &tile.comp_mode[ctx],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..IS_JOINT_CONTEXTS {
            avg_cdf_row(
                &mut self.is_joint[ctx],
                &tile.is_joint[ctx],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..COMPOUND_MODE_CONTEXTS {
            avg_cdf_row(
                &mut self.compound_mode_non_joint[ctx],
                &tile.compound_mode_non_joint[ctx],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..COMP_GROUP_IDX_CONTEXTS {
            avg_cdf_row(
                &mut self.comp_group_idx[ctx],
                &tile.comp_group_idx[ctx],
                tile_num,
                num_log2,
            );
        }
        for idx in 0..CWP_IDX_CONTEXTS {
            avg_cdf_row(
                &mut self.cwp_idx[idx],
                &tile.cwp_idx[idx],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..REF_CONTEXTS {
            for ref_idx in 0..REFS_PER_FRAME_MINUS_1 {
                avg_cdf_row(
                    &mut self.comp_ref0[ctx][ref_idx],
                    &tile.comp_ref0[ctx][ref_idx],
                    tile_num,
                    num_log2,
                );
            }
        }
        for ctx in 0..REF_CONTEXTS {
            for bit_type in 0..COMP_REF1_BIT_TYPES {
                for ref_idx in 0..REFS_PER_FRAME_MINUS_1 {
                    avg_cdf_row(
                        &mut self.comp_ref1[ctx][bit_type][ref_idx],
                        &tile.comp_ref1[ctx][bit_type][ref_idx],
                        tile_num,
                        num_log2,
                    );
                }
            }
        }
        avg_cdf_row(
            &mut self.joint_shell_set,
            &tile.joint_shell_set,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.joint_shell6_class0,
            &tile.joint_shell6_class0,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.joint_shell6_class1,
            &tile.joint_shell6_class1,
            tile_num,
            num_log2,
        );
        avg_cdf_row(
            &mut self.joint_shell_last_two,
            &tile.joint_shell_last_two,
            tile_num,
            num_log2,
        );
        for bank in 0..SHELL_OFFSET_LOW_CLASS_BANKS {
            avg_cdf_row(
                &mut self.shell_offset_low_class[bank],
                &tile.shell_offset_low_class[bank],
                tile_num,
                num_log2,
            );
        }
        avg_cdf_row(
            &mut self.shell_offset_class2,
            &tile.shell_offset_class2,
            tile_num,
            num_log2,
        );
        for bank in 0..SHELL_OFFSET_OTHER_CLASS_BANKS {
            avg_cdf_row(
                &mut self.shell_offset_other_class[bank],
                &tile.shell_offset_other_class[bank],
                tile_num,
                num_log2,
            );
        }
        for bank in 0..COL_MV_GREATER_BANKS {
            avg_cdf_row(
                &mut self.col_mv_greater[bank],
                &tile.col_mv_greater[bank],
                tile_num,
                num_log2,
            );
        }
        for bank in 0..COL_MV_INDEX_BANKS {
            avg_cdf_row(
                &mut self.col_mv_index[bank],
                &tile.col_mv_index[bank],
                tile_num,
                num_log2,
            );
        }
        for ctx in 0..INTERP_FILTER_CONTEXTS {
            avg_cdf_row(
                &mut self.interp_filter[ctx],
                &tile.interp_filter[ctx],
                tile_num,
                num_log2,
            );
        }
        avg_cdf_row(
            &mut self.use_wiener_ns,
            &tile.use_wiener_ns,
            tile_num,
            num_log2,
        );
        self.coeff.avg_from_tile(tile_num, &tile.coeff, num_log2);
    }

    pub(crate) fn scale_counts_for_frame_end_update(&mut self) {
        scale_cdf_count(&mut self.y_mode_set);
        for ctx in 0..Y_MODE_INDEX_CONTEXTS {
            scale_cdf_count(&mut self.y_mode_index[ctx]);
        }
        for coeff_cdf_q_ctx in 0..COEFF_CDF_Q_CONTEXTS {
            for plane_type in 0..PLANE_TYPES {
                for tx_size in 0..TX_SIZE_CONTEXTS {
                    for ctx in 0..TXB_SKIP_CONTEXTS {
                        scale_cdf_count(
                            &mut self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size][ctx],
                        );
                    }
                }
            }
            for ctx in 0..V_TXB_SKIP_CONTEXTS {
                scale_cdf_count(&mut self.v_txb_skip[coeff_cdf_q_ctx][ctx]);
            }
            scale_cdf_count(&mut self.eob_extra[coeff_cdf_q_ctx]);
        }
        for ctx in 0..UV_MODE_CONTEXTS {
            scale_cdf_count(&mut self.uv_mode_cfl_not_allowed[ctx]);
        }
        scale_eob_pt_bank(&mut self.eob_pt_16);
        scale_eob_pt_bank(&mut self.eob_pt_32);
        scale_eob_pt_bank(&mut self.eob_pt_64);
        scale_eob_pt_bank(&mut self.eob_pt_128);
        scale_eob_pt_bank(&mut self.eob_pt_256);
        scale_eob_pt_bank(&mut self.eob_pt_512);
        scale_eob_pt_bank(&mut self.eob_pt_1024);
        for row in self.dc_sign.iter_mut().flatten().flatten().flatten() {
            scale_cdf_count(row);
        }
        for ctx in 0..IS_INTER_CONTEXTS {
            scale_cdf_count(&mut self.is_inter[ctx]);
        }
        for ctx in 0..SKIP_CONTEXTS {
            scale_cdf_count(&mut self.skip[ctx]);
        }
        for ctx in 0..SINGLE_MODE_CONTEXTS {
            scale_cdf_count(&mut self.single_mode[ctx]);
        }
        for idx in 0..DRL_MODE_IDX_BANKS {
            for ctx in 0..DRL_MODE_CONTEXTS {
                scale_cdf_count(&mut self.drl_mode[idx][ctx]);
            }
        }
        for ctx in 0..REF_CONTEXTS {
            for ref_idx in 0..REFS_PER_FRAME_MINUS_1 {
                scale_cdf_count(&mut self.single_ref[ctx][ref_idx]);
            }
        }
        for ctx in 0..COMP_MODE_CONTEXTS {
            scale_cdf_count(&mut self.comp_mode[ctx]);
        }
        for ctx in 0..IS_JOINT_CONTEXTS {
            scale_cdf_count(&mut self.is_joint[ctx]);
        }
        for ctx in 0..COMPOUND_MODE_CONTEXTS {
            scale_cdf_count(&mut self.compound_mode_non_joint[ctx]);
        }
        for ctx in 0..COMP_GROUP_IDX_CONTEXTS {
            scale_cdf_count(&mut self.comp_group_idx[ctx]);
        }
        for idx in 0..CWP_IDX_CONTEXTS {
            scale_cdf_count(&mut self.cwp_idx[idx]);
        }
        for row in self.comp_ref0.iter_mut().flatten() {
            scale_cdf_count(row);
        }
        for row in self.comp_ref1.iter_mut().flatten().flatten() {
            scale_cdf_count(row);
        }
        scale_cdf_count(&mut self.joint_shell_set);
        scale_cdf_count(&mut self.joint_shell6_class0);
        scale_cdf_count(&mut self.joint_shell6_class1);
        scale_cdf_count(&mut self.joint_shell_last_two);
        for bank in 0..SHELL_OFFSET_LOW_CLASS_BANKS {
            scale_cdf_count(&mut self.shell_offset_low_class[bank]);
        }
        scale_cdf_count(&mut self.shell_offset_class2);
        for bank in 0..SHELL_OFFSET_OTHER_CLASS_BANKS {
            scale_cdf_count(&mut self.shell_offset_other_class[bank]);
        }
        for bank in 0..COL_MV_GREATER_BANKS {
            scale_cdf_count(&mut self.col_mv_greater[bank]);
        }
        for bank in 0..COL_MV_INDEX_BANKS {
            scale_cdf_count(&mut self.col_mv_index[bank]);
        }
        for ctx in 0..INTERP_FILTER_CONTEXTS {
            scale_cdf_count(&mut self.interp_filter[ctx]);
        }
        scale_cdf_count(&mut self.use_wiener_ns);
        self.coeff.scale_counts_for_frame_end_update();
    }

    #[cfg(test)]
    pub(crate) const fn y_mode_set(&self) -> &YModeSetCdfRow {
        &self.y_mode_set
    }

    #[cfg(test)]
    pub(crate) const fn y_mode_index(&self) -> &YModeIndexCdfRows {
        &self.y_mode_index
    }

    #[cfg(test)]
    pub(crate) const fn txb_skip(&self) -> &TxbSkipCdfRows {
        &self.txb_skip
    }

    #[cfg(test)]
    pub(crate) const fn uv_mode_cfl_not_allowed(&self) -> &UvModeCflNotAllowedCdfRows {
        &self.uv_mode_cfl_not_allowed
    }

    #[cfg(test)]
    pub(crate) const fn v_txb_skip(&self) -> &VTxbSkipCdfRows {
        &self.v_txb_skip
    }

    #[cfg(test)]
    pub(crate) const fn eob_extra(&self) -> &EobExtraCdfRows {
        &self.eob_extra
    }

    #[cfg(test)]
    pub(crate) const fn comp_mode(&self) -> &CompModeCdfRows {
        &self.comp_mode
    }

    #[cfg(test)]
    pub(crate) const fn is_joint(&self) -> &IsJointCdfRows {
        &self.is_joint
    }

    #[cfg(test)]
    pub(crate) const fn compound_mode_non_joint(&self) -> &CompoundModeNonJointCdfRows {
        &self.compound_mode_non_joint
    }

    #[cfg(test)]
    pub(crate) const fn comp_group_idx(&self) -> &CompGroupIdxCdfRows {
        &self.comp_group_idx
    }

    #[cfg(test)]
    pub(crate) const fn cwp_idx(&self) -> &CwpIdxCdfRows {
        &self.cwp_idx
    }

    #[cfg(test)]
    pub(crate) const fn comp_ref0(&self) -> &CompRef0CdfRows {
        &self.comp_ref0
    }

    #[cfg(test)]
    pub(crate) const fn comp_ref1(&self) -> &CompRef1CdfRows {
        &self.comp_ref1
    }

    #[cfg(test)]
    pub(crate) const fn use_wiener_ns(&self) -> &UseWienerNsCdfRow {
        &self.use_wiener_ns
    }
}

/// Averages one `eob_pt` family bank (`[coeff_cdf_q_ctx][eobCtx][N]`) against the
/// completed tile's matching bank, for any class width `N`.
fn avg_eob_pt_bank<const N: usize>(
    frame: &mut [[[i32; N]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS],
    tile: &[[[i32; N]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS],
    tile_num: u32,
    num_log2: u8,
) {
    for (frame_q, tile_q) in frame.iter_mut().zip(tile.iter()) {
        for (frame_row, tile_row) in frame_q.iter_mut().zip(tile_q.iter()) {
            avg_cdf_row(frame_row, tile_row, tile_num, num_log2);
        }
    }
}

/// Scales the frame-end adaptation count of every row in one `eob_pt` family
/// bank, for any class width `N`.
fn scale_eob_pt_bank<const N: usize>(
    bank: &mut [[[i32; N]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS],
) {
    for q_rows in bank.iter_mut() {
        for row in q_rows.iter_mut() {
            scale_cdf_count(row);
        }
    }
}

fn checked_eob_plane_ctx(eob_ctx: usize) -> Result<usize, TileCdfError> {
    if eob_ctx >= EOB_PLANE_CTXS {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::EobPt,
            index_name: "eob_ctx",
            actual: eob_ctx,
            max_exclusive: EOB_PLANE_CTXS,
        });
    }
    Ok(eob_ctx)
}

fn checked_dc_sign_group(group: usize) -> Result<usize, TileCdfError> {
    if group >= DC_SIGN_GROUPS {
        return Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DcSign,
            index_name: "group",
            actual: group,
            max_exclusive: DC_SIGN_GROUPS,
        });
    }
    Ok(group)
}

fn checked_coeff_cdf_q_context(
    array: TileCdfArray,
    coeff_cdf_q_ctx: usize,
) -> Result<usize, TileCdfError> {
    if coeff_cdf_q_ctx >= COEFF_CDF_Q_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "coeff_cdf_q_ctx",
            actual: coeff_cdf_q_ctx,
            max_exclusive: COEFF_CDF_Q_CONTEXTS,
        });
    }
    Ok(coeff_cdf_q_ctx)
}

fn checked_plane_type(array: TileCdfArray, plane_type: usize) -> Result<usize, TileCdfError> {
    if plane_type >= PLANE_TYPES {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "plane_type",
            actual: plane_type,
            max_exclusive: PLANE_TYPES,
        });
    }
    Ok(plane_type)
}

fn checked_tx_size(array: TileCdfArray, tx_size: usize) -> Result<usize, TileCdfError> {
    if tx_size >= TX_SIZE_CONTEXTS {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "tx_size",
            actual: tx_size,
            max_exclusive: TX_SIZE_CONTEXTS,
        });
    }
    Ok(tx_size)
}
