// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 block-symbol CDF rows.

use splot_core::tables::cdf::{
    DEFAULT_CCTX_TYPE_CDF, DEFAULT_CFL_ALPHA_CDF, DEFAULT_CFL_INDEX_CDF, DEFAULT_CFL_MH_DIR_CDF,
    DEFAULT_CFL_MHCCP_CDF, DEFAULT_CFL_SIGN_CDF, DEFAULT_COMP_GROUP_IDX_CDF, DEFAULT_COMP_MODE_CDF,
    DEFAULT_COMP_REF0_CDF, DEFAULT_COMP_REF1_CDF, DEFAULT_COMPOUND_MODE_NON_JOINT_CDF,
    DEFAULT_CWP_IDX_CDF, DEFAULT_DC_SIGN_CDF, DEFAULT_DRL_MODE_CDF, DEFAULT_EOB_EXTRA_CDF,
    DEFAULT_EOB_PT_16_CDF, DEFAULT_EOB_PT_32_CDF, DEFAULT_EOB_PT_64_CDF, DEFAULT_EOB_PT_128_CDF,
    DEFAULT_EOB_PT_256_CDF, DEFAULT_EOB_PT_512_CDF, DEFAULT_EOB_PT_1024_CDF,
    DEFAULT_EXPLICIT_BAWP_CDF, DEFAULT_EXPLICIT_BAWP_SCALE_CDF, DEFAULT_IDENTITY_ROW_Y_CDF,
    DEFAULT_INTER_INTRA_MODE_CDF, DEFAULT_INTER_TX_TYPE_INDEX_SET1_CDF,
    DEFAULT_INTER_TX_TYPE_INDEX_SET2_CDF, DEFAULT_INTER_TX_TYPE_LONG_CDF,
    DEFAULT_INTER_TX_TYPE_OFFSET_SET1_CDF, DEFAULT_INTER_TX_TYPE_OFFSET_SET2_CDF,
    DEFAULT_INTER_TX_TYPE_SET1_CDF, DEFAULT_INTER_TX_TYPE_SET2_CDF, DEFAULT_INTER_TX_TYPE_SET3_CDF,
    DEFAULT_INTER_TX_TYPE_SET4_CDF, DEFAULT_INTERP_FILTER_CDF, DEFAULT_INTRA_TX_TYPE_LONG_CDF,
    DEFAULT_INTRA_TX_TYPE_SET1_CDF, DEFAULT_INTRA_TX_TYPE_SET2_CDF, DEFAULT_IS_CFL_CDF,
    DEFAULT_IS_INTER_CDF, DEFAULT_IS_JOINT_CDF, DEFAULT_IS_LONG_SIDE_DCT_CDF, DEFAULT_IS_WARP_CDF,
    DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF, DEFAULT_MOST_PROBABLE_STX_SET_CDF,
    DEFAULT_PALETTE_SIZE_2_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_3_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_4_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_5_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_6_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_7_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_8_Y_COLOR_CDF, DEFAULT_PALETTE_Y_MODE_CDF, DEFAULT_PALETTE_Y_SIZE_CDF,
    DEFAULT_SEC_TX_TYPE_CDF, DEFAULT_SINGLE_MODE_CDF, DEFAULT_SINGLE_REF_CDF, DEFAULT_SKIP_CDF,
    DEFAULT_TXB_SKIP_CDF, DEFAULT_USE_AMVD_CDF, DEFAULT_USE_BAWP_CDF, DEFAULT_USE_BAWP_CHROMA_CDF,
    DEFAULT_USE_WIENER_NS_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF, DEFAULT_V_TXB_SKIP_CDF,
    DEFAULT_WARP_DELTA_PARAM_HIGH_CDF, DEFAULT_WARP_DELTA_PARAM_LOW_CDF,
    DEFAULT_WARP_DELTA_PARAM_SIGN_CDF, DEFAULT_WARP_IDX_CDF, DEFAULT_WARP_INTER_INTRA_CDF,
    DEFAULT_WARP_MV_CDF, DEFAULT_WARP_PRECISION_CDF, DEFAULT_WARP_WITH_MVD_CDF,
    DEFAULT_WEDGE_ANGLE_CDF, DEFAULT_WEDGE_DIST1_CDF, DEFAULT_WEDGE_DIST2_CDF,
    DEFAULT_WEDGE_INTER_INTRA_CDF, DEFAULT_WEDGE_QUAD_CDF, DEFAULT_WIENER_NS_BASE_CDF,
    DEFAULT_WIENER_NS_LENGTH_CDF, DEFAULT_WIENER_NS_UV_SYM_CDF, DEFAULT_Y_MODE_INDEX_CDF,
    DEFAULT_Y_MODE_OFFSET_CDF, DEFAULT_Y_MODE_SET_CDF,
};

mod mv;

use self::mv::MvCdfRows;
pub(crate) use self::mv::MvCdfSelector;
use super::coeff_rows::{CoeffCdfRows, CoeffCdfSelector};
use super::{
    CDF_ROW_LEN, TileCdfArray, TileCdfError, avg_cdf_row, avg_cdf_rows, checked_context,
    scale_cdf_count, scale_cdf_rows,
};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const Y_MODE_INDEX_CONTEXTS: usize = 3;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const Y_MODE_OFFSET_CDF_ROW_LEN: usize = 7;
const Y_MODE_OFFSET_CONTEXTS: usize = 3;
const COEFF_CDF_Q_CONTEXTS: usize = 4;
const EOB_PLANE_CTXS: usize = 3;
const PLANE_TYPES: usize = 2;
const TX_SIZE_CONTEXTS: usize = 5;
const TXB_SKIP_CONTEXTS: usize = 10;
const UV_MODE_CONTEXTS: usize = 2;
const CFL_CONTEXTS: usize = 3;
const CFL_ALPHA_CONTEXTS: usize = 6;
const CFL_ALPHA_CDF_ROW_LEN: usize = 9;
const CFL_SIGN_CDF_ROW_LEN: usize = 9;
const CFL_MH_DIR_GROUPS: usize = 4;
const CFL_MH_DIR_CDF_ROW_LEN: usize = 4;
const V_TXB_SKIP_CONTEXTS: usize = 12;
const DC_SIGN_GROUPS: usize = 2;
const DC_SIGN_CONTEXTS: usize = 3;
const IS_INTER_CONTEXTS: usize = 4;
const SKIP_CONTEXTS: usize = 6;
const SINGLE_MODE_CONTEXTS: usize = 5;
const WARP_MODE_CONTEXTS: usize = 5;
const WARP_IDX_CONTEXTS: usize = 3;
const WARP_DELTA_PARAM_CONTEXTS: usize = 2;
const WARP_DELTA_PARAM_CDF_ROW_LEN: usize = 9;
const BLOCK_SIZE_CONTEXTS: usize = 29;
const BLOCK_SIZE_GROUPS: usize = 4;
const INTERINTRA_MODE_ROW_LEN: usize = 5;
const WEDGE_QUAD_ROW_LEN: usize = 5;
const WEDGE_ANGLE_CONTEXTS: usize = 4;
const WEDGE_ANGLE_ROW_LEN: usize = 6;
const WEDGE_DIST1_ROW_LEN: usize = 5;
const WEDGE_DIST2_ROW_LEN: usize = 4;
const DRL_MODE_IDX_BANKS: usize = 3;
const DRL_MODE_CONTEXTS: usize = 5;
const REF_CONTEXTS: usize = 3;
const REFS_PER_FRAME_MINUS_1: usize = 6;
const COMP_MODE_CONTEXTS: usize = 5;
const IS_JOINT_CONTEXTS: usize = 2;
const COMPOUND_MODE_CONTEXTS: usize = 5;
const COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN: usize = 6;
const COMP_GROUP_IDX_CONTEXTS: usize = 12;
const CWP_IDX_CONTEXTS: usize = 4;
const COMP_REF1_BIT_TYPES: usize = 2;
const INTERP_FILTER_CONTEXTS: usize = 16;
const AMVD_MODE_CONTEXTS: usize = 9;
const AMVD_CONTEXTS: usize = 3;
const BAWP_SCALES_CONTEXTS: usize = 3;
const WIENER_NS_LENGTH_CONTEXTS: usize = 2;
const WIENER_NS_BASE_CDF_ROW_LEN: usize = 5;
const INTRA_TX_TYPE_LONG_SIZE_CONTEXTS: usize = 4;
const INTRA_TX_TYPE_SIZE_CONTEXTS: usize = 3;
const INTRA_TX_TYPE_LONG_ROW_LEN: usize = 5;
const INTER_TX_TYPE_LONG_EOB_CONTEXTS: usize = 3;
const INTER_TX_TYPE_LONG_SIZE_CONTEXTS: usize = 4;
const INTER_TX_TYPE_LONG_ROW_LEN: usize = 5;
const INTER_TX_TYPE_EOB_CONTEXTS: usize = 3;
const INTER_TX_TYPE_SET1_SIZE_CONTEXTS: usize = 2;
const INTER_TX_TYPE_SET1_ROW_LEN: usize = 3;
const INTER_TX_TYPE_SET2_ROW_LEN: usize = 3;
const INTER_TX_TYPE_INDEX_ROW_LEN: usize = 9;
const INTER_TX_TYPE_OFFSET_SET1_ROW_LEN: usize = 9;
const INTER_TX_TYPE_OFFSET_SET2_ROW_LEN: usize = 5;
const INTER_TX_TYPE_SET34_SIZE_CONTEXTS: usize = 4;
const INTER_TX_TYPE_SET3_ROW_LEN: usize = 3;
const INTER_TX_TYPE_SET4_ROW_LEN: usize = 5;
const INTRA_TX_TYPE_SET1_ROW_LEN: usize = 8;
const INTRA_TX_TYPE_SET2_ROW_LEN: usize = 3;
const IS_LONG_SIDE_DCT_CONTEXTS: usize = 2;
const SEC_TX_TYPE_IS_INTER_CONTEXTS: usize = 2;
const SEC_TX_TYPE_TX_SIZE_CONTEXTS: usize = 5;
const SEC_TX_TYPE_ROW_LEN: usize = 5;
const MOST_PROBABLE_STX_SET_ROW_LEN: usize = 8;
const MOST_PROBABLE_STX_SET_ADST_ROW_LEN: usize = 5;
const CCTX_TYPE_CDF_ROW_LEN: usize = 8;
const PALETTE_ROW_FLAG_CONTEXTS: usize = 4;
const PALETTE_COLOR_CONTEXTS: usize = 5;

pub(crate) type YModeSetCdfRow = [i32; Y_MODE_SET_CDF_ROW_LEN];
pub(crate) type YModeIndexCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; Y_MODE_INDEX_CONTEXTS];
pub(crate) type YModeOffsetCdfRows = [[i32; Y_MODE_OFFSET_CDF_ROW_LEN]; Y_MODE_OFFSET_CONTEXTS];
pub(crate) type TxbSkipCdfRows = [[[[[i32; CDF_ROW_LEN]; TXB_SKIP_CONTEXTS]; TX_SIZE_CONTEXTS];
    PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type UvModeCflNotAllowedCdfRows = [[i32; INTRA_MODE_CDF_ROW_LEN]; UV_MODE_CONTEXTS];
pub(crate) type IsCflCdfRows = [[i32; CDF_ROW_LEN]; CFL_CONTEXTS];
pub(crate) type CflIndexCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type CflSignCdfRow = [i32; CFL_SIGN_CDF_ROW_LEN];
pub(crate) type CflAlphaCdfRows = [[i32; CFL_ALPHA_CDF_ROW_LEN]; CFL_ALPHA_CONTEXTS];
pub(crate) type CflMhccpCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type CflMhDirCdfRows = [[i32; CFL_MH_DIR_CDF_ROW_LEN]; CFL_MH_DIR_GROUPS];
pub(crate) type VTxbSkipCdfRows = [[[i32; CDF_ROW_LEN]; V_TXB_SKIP_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobExtraCdfRows = [[i32; CDF_ROW_LEN]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type DcSignCdfRows =
    [[[[[i32; CDF_ROW_LEN]; DC_SIGN_CONTEXTS]; DC_SIGN_GROUPS]; PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type IsInterCdfRows = [[i32; CDF_ROW_LEN]; IS_INTER_CONTEXTS];
pub(crate) type SkipCdfRows = [[i32; CDF_ROW_LEN]; SKIP_CONTEXTS];
pub(crate) type SingleModeCdfRows = [[i32; 4]; SINGLE_MODE_CONTEXTS];
pub(crate) type IsWarpCdfRows = [[i32; CDF_ROW_LEN]; WARP_MODE_CONTEXTS];
pub(crate) type WarpMvCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type WarpIdxCdfRows = [[i32; CDF_ROW_LEN]; WARP_IDX_CONTEXTS];
pub(crate) type WarpWithMvdCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type WarpPrecisionCdfRows = [[i32; CDF_ROW_LEN]; BLOCK_SIZE_CONTEXTS];
pub(crate) type WarpDeltaParamCdfRows =
    [[i32; WARP_DELTA_PARAM_CDF_ROW_LEN]; WARP_DELTA_PARAM_CONTEXTS];
pub(crate) type WarpDeltaParamSignCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type WarpInterIntraCdfRows = [[i32; CDF_ROW_LEN]; BLOCK_SIZE_GROUPS];
pub(crate) type InterIntraModeCdfRows = [[i32; INTERINTRA_MODE_ROW_LEN]; BLOCK_SIZE_GROUPS];
pub(crate) type WedgeInterIntraCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type WedgeQuadCdfRow = [i32; WEDGE_QUAD_ROW_LEN];
pub(crate) type WedgeAngleCdfRows = [[i32; WEDGE_ANGLE_ROW_LEN]; WEDGE_ANGLE_CONTEXTS];
pub(crate) type WedgeDist1CdfRow = [i32; WEDGE_DIST1_ROW_LEN];
pub(crate) type WedgeDist2CdfRow = [i32; WEDGE_DIST2_ROW_LEN];
pub(crate) type DrlModeCdfRows = [[[i32; CDF_ROW_LEN]; DRL_MODE_CONTEXTS]; DRL_MODE_IDX_BANKS];
pub(crate) type SingleRefCdfRows = [[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; REF_CONTEXTS];
pub(crate) type CompModeCdfRows = [[i32; CDF_ROW_LEN]; COMP_MODE_CONTEXTS];
pub(crate) type IsJointCdfRows = [[i32; CDF_ROW_LEN]; IS_JOINT_CONTEXTS];
pub(crate) type CompoundModeNonJointCdfRows =
    [[i32; COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN]; COMPOUND_MODE_CONTEXTS];
pub(crate) type CompGroupIdxCdfRows = [[i32; CDF_ROW_LEN]; COMP_GROUP_IDX_CONTEXTS];
pub(crate) type CwpIdxCdfRows = [[i32; CDF_ROW_LEN]; CWP_IDX_CONTEXTS];
pub(crate) type CompRef0CdfRows = [[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; REF_CONTEXTS];
pub(crate) type CompRef1CdfRows =
    [[[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; COMP_REF1_BIT_TYPES]; REF_CONTEXTS];
pub(crate) type UseAmvdCdfRows = [[[i32; CDF_ROW_LEN]; AMVD_CONTEXTS]; AMVD_MODE_CONTEXTS];
pub(crate) type UseBawpCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type ExplicitBawpCdfRows = [[i32; CDF_ROW_LEN]; BAWP_SCALES_CONTEXTS];
pub(crate) type ExplicitBawpScaleCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type UseWienerNsCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type WienerNsLengthCdfRows = [[i32; CDF_ROW_LEN]; WIENER_NS_LENGTH_CONTEXTS];
pub(crate) type WienerNsUvSymCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type WienerNsBaseCdfRow = [i32; WIENER_NS_BASE_CDF_ROW_LEN];
pub(crate) type IsLongSideDctCdfRows = [[i32; CDF_ROW_LEN]; IS_LONG_SIDE_DCT_CONTEXTS];
pub(crate) type IntraTxTypeLongCdfRows =
    [[i32; INTRA_TX_TYPE_LONG_ROW_LEN]; INTRA_TX_TYPE_LONG_SIZE_CONTEXTS];
pub(crate) type InterTxTypeLongCdfRows = [[[i32; INTER_TX_TYPE_LONG_ROW_LEN];
    INTER_TX_TYPE_LONG_SIZE_CONTEXTS];
    INTER_TX_TYPE_LONG_EOB_CONTEXTS];
pub(crate) type InterTxTypeSet1CdfRows = [[[i32; INTER_TX_TYPE_SET1_ROW_LEN];
    INTER_TX_TYPE_SET1_SIZE_CONTEXTS];
    INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type InterTxTypeSet2CdfRows =
    [[i32; INTER_TX_TYPE_SET2_ROW_LEN]; INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type InterTxTypeIndexSet1CdfRows =
    [[i32; INTER_TX_TYPE_INDEX_ROW_LEN]; INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type InterTxTypeIndexSet2CdfRows =
    [[i32; INTER_TX_TYPE_INDEX_ROW_LEN]; INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type InterTxTypeOffsetSet1CdfRows =
    [[i32; INTER_TX_TYPE_OFFSET_SET1_ROW_LEN]; INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type InterTxTypeOffsetSet2CdfRows =
    [[i32; INTER_TX_TYPE_OFFSET_SET2_ROW_LEN]; INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type InterTxTypeSet3CdfRows = [[[i32; INTER_TX_TYPE_SET3_ROW_LEN];
    INTER_TX_TYPE_SET34_SIZE_CONTEXTS];
    INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type InterTxTypeSet4CdfRows = [[[i32; INTER_TX_TYPE_SET4_ROW_LEN];
    INTER_TX_TYPE_SET34_SIZE_CONTEXTS];
    INTER_TX_TYPE_EOB_CONTEXTS];
pub(crate) type IntraTxTypeSet1CdfRows =
    [[i32; INTRA_TX_TYPE_SET1_ROW_LEN]; INTRA_TX_TYPE_SIZE_CONTEXTS];
pub(crate) type IntraTxTypeSet2CdfRows =
    [[i32; INTRA_TX_TYPE_SET2_ROW_LEN]; INTRA_TX_TYPE_SIZE_CONTEXTS];
pub(crate) type SecTxTypeCdfRows =
    [[[i32; SEC_TX_TYPE_ROW_LEN]; SEC_TX_TYPE_TX_SIZE_CONTEXTS]; SEC_TX_TYPE_IS_INTER_CONTEXTS];
pub(crate) type MostProbableStxSetCdfRow = [i32; MOST_PROBABLE_STX_SET_ROW_LEN];
pub(crate) type MostProbableStxSetAdstCdfRow = [i32; MOST_PROBABLE_STX_SET_ADST_ROW_LEN];
pub(crate) type CctxTypeCdfRow = [i32; CCTX_TYPE_CDF_ROW_LEN];
pub(crate) type PaletteYModeCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type PaletteYSizeCdfRow = [i32; 8];
pub(crate) type IdentityRowYCdfRows = [[i32; 4]; PALETTE_ROW_FLAG_CONTEXTS];
pub(crate) type PaletteSize2YColorCdfRows = [[i32; 3]; PALETTE_COLOR_CONTEXTS];
pub(crate) type PaletteSize3YColorCdfRows = [[i32; 4]; PALETTE_COLOR_CONTEXTS];
pub(crate) type PaletteSize4YColorCdfRows = [[i32; 5]; PALETTE_COLOR_CONTEXTS];
pub(crate) type PaletteSize5YColorCdfRows = [[i32; 6]; PALETTE_COLOR_CONTEXTS];
pub(crate) type PaletteSize6YColorCdfRows = [[i32; 7]; PALETTE_COLOR_CONTEXTS];
pub(crate) type PaletteSize7YColorCdfRows = [[i32; 8]; PALETTE_COLOR_CONTEXTS];
pub(crate) type PaletteSize8YColorCdfRows = [[i32; 9]; PALETTE_COLOR_CONTEXTS];
pub(crate) type InterpFilterCdfRows = [[i32; 4]; INTERP_FILTER_CONTEXTS];

pub(crate) type EobPt16CdfRows = [[[i32; 6]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt32CdfRows = [[[i32; 7]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt64CdfRows = [[[i32; 8]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt128CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt256CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt512CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobPt1024CdfRows = [[[i32; 9]; EOB_PLANE_CTXS]; COEFF_CDF_Q_CONTEXTS];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EobPtSize {
    Pt16,
    Pt32,
    Pt64,
    Pt128,
    Pt256,
    Pt512,
    Pt1024,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BlockCdfSelector {
    YModeSet,
    YModeIndex {
        ctx: usize,
    },
    YModeOffset {
        ctx: usize,
    },
    TxbSkip {
        coeff_cdf_q_ctx: usize,
        plane_type: usize,
        tx_size: usize,
        ctx: usize,
    },
    UvModeCflNotAllowed {
        ctx: usize,
    },
    IsCfl {
        ctx: usize,
    },
    CflIndex,
    CflSign,
    CflAlpha {
        ctx: usize,
    },
    CflMhccp,
    CflMhDir {
        size_group: usize,
    },
    VTxbSkip {
        coeff_cdf_q_ctx: usize,
        ctx: usize,
    },
    EobExtra {
        coeff_cdf_q_ctx: usize,
    },
    EobPt {
        size: EobPtSize,
        coeff_cdf_q_ctx: usize,
        eob_ctx: usize,
    },
    DcSign {
        coeff_cdf_q_ctx: usize,
        plane_type: usize,
        group: usize,
        ctx: usize,
    },
    IsInter {
        ctx: usize,
    },
    Skip {
        ctx: usize,
    },
    SingleMode {
        ctx: usize,
    },
    IsWarp {
        ctx: usize,
    },
    WarpMv,
    WarpIdx {
        ctx: usize,
    },
    WarpWithMvd,
    WarpPrecision {
        block_size: usize,
    },
    WarpDeltaParamLow {
        index_type: usize,
    },
    WarpDeltaParamHigh {
        index_type: usize,
    },
    WarpDeltaParamSign,
    WarpInterIntra {
        bsize_group: usize,
    },
    InterIntraMode {
        bsize_group: usize,
    },
    WedgeInterIntra,
    WedgeQuad,
    WedgeAngle {
        quad: usize,
    },
    WedgeDist1,
    WedgeDist2,
    DrlMode {
        idx: usize,
        ctx: usize,
    },
    SingleRef {
        ctx: usize,
        ref_idx: usize,
    },
    CompMode {
        ctx: usize,
    },
    IsJoint {
        ctx: usize,
    },
    CompoundModeNonJoint {
        ctx: usize,
    },
    CompGroupIdx {
        ctx: usize,
    },
    CwpIdx {
        idx: usize,
    },
    CompRef0 {
        ctx: usize,
        ref_idx: usize,
    },
    CompRef1 {
        ctx: usize,
        bit_type: usize,
        ref_idx: usize,
    },
    ReadMv(MvCdfSelector),
    InterpFilter {
        ctx: usize,
    },
    UseBawp,
    UseBawpChroma,
    UseAmvd {
        index: usize,
        ctx: usize,
    },
    ExplicitBawp {
        ctx: usize,
    },
    ExplicitBawpScale,
    UseWienerNs,
    WienerNsLength {
        plane_ctx: usize,
    },
    WienerNsUvSym,
    WienerNsBase,
    IsLongSideDct {
        is_inter: usize,
    },
    IntraTxTypeLong {
        tx_size_sqr: usize,
    },
    InterTxTypeLong {
        ctx: usize,
        tx_size_sqr: usize,
    },
    InterTxTypeSet1 {
        ctx: usize,
        tx_size_sqr: usize,
    },
    InterTxTypeSet2 {
        ctx: usize,
    },
    InterTxTypeIndexSet1 {
        ctx: usize,
    },
    InterTxTypeIndexSet2 {
        ctx: usize,
    },
    InterTxTypeOffsetSet1 {
        ctx: usize,
    },
    InterTxTypeOffsetSet2 {
        ctx: usize,
    },
    InterTxTypeSet3 {
        ctx: usize,
        tx_size_sqr: usize,
    },
    InterTxTypeSet4 {
        ctx: usize,
        tx_size_sqr: usize,
    },
    IntraTxTypeSet1 {
        tx_size_sqr: usize,
    },
    IntraTxTypeSet2 {
        tx_size_sqr: usize,
    },
    SecTxType {
        is_inter: usize,
        tx_size_sqr: usize,
    },
    MostProbableStxSet,
    MostProbableStxSetAdst,
    CctxType,
    PaletteYMode,
    PaletteYSize,
    IdentityRowY {
        ctx: usize,
    },
    PaletteYColorIndex {
        palette_size: usize,
        ctx: usize,
    },
    Coeff(CoeffCdfSelector),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockCdfRows {
    pub(super) y_mode_set: YModeSetCdfRow,
    pub(super) y_mode_index: YModeIndexCdfRows,
    pub(super) y_mode_offset: YModeOffsetCdfRows,
    pub(super) txb_skip: TxbSkipCdfRows,
    pub(super) uv_mode_cfl_not_allowed: UvModeCflNotAllowedCdfRows,
    pub(super) is_cfl: IsCflCdfRows,
    pub(super) cfl_index: CflIndexCdfRow,
    pub(super) cfl_sign: CflSignCdfRow,
    pub(super) cfl_alpha: CflAlphaCdfRows,
    pub(super) cfl_mhccp: CflMhccpCdfRow,
    pub(super) cfl_mh_dir: CflMhDirCdfRows,
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
    pub(super) is_warp: IsWarpCdfRows,
    pub(super) warp_mv: WarpMvCdfRow,
    pub(super) warp_idx: WarpIdxCdfRows,
    pub(super) warp_with_mvd: WarpWithMvdCdfRow,
    pub(super) warp_precision: WarpPrecisionCdfRows,
    pub(super) warp_delta_param_low: WarpDeltaParamCdfRows,
    pub(super) warp_delta_param_high: WarpDeltaParamCdfRows,
    pub(super) warp_delta_param_sign: WarpDeltaParamSignCdfRow,
    pub(super) warp_inter_intra: WarpInterIntraCdfRows,
    pub(super) inter_intra_mode: InterIntraModeCdfRows,
    pub(super) wedge_inter_intra: WedgeInterIntraCdfRow,
    pub(super) wedge_quad: WedgeQuadCdfRow,
    pub(super) wedge_angle: WedgeAngleCdfRows,
    pub(super) wedge_dist1: WedgeDist1CdfRow,
    pub(super) wedge_dist2: WedgeDist2CdfRow,
    pub(super) drl_mode: DrlModeCdfRows,
    pub(super) single_ref: SingleRefCdfRows,
    pub(super) comp_mode: CompModeCdfRows,
    pub(super) is_joint: IsJointCdfRows,
    pub(super) compound_mode_non_joint: CompoundModeNonJointCdfRows,
    pub(super) comp_group_idx: CompGroupIdxCdfRows,
    pub(super) cwp_idx: CwpIdxCdfRows,
    pub(super) comp_ref0: CompRef0CdfRows,
    pub(super) comp_ref1: CompRef1CdfRows,
    read_mv: MvCdfRows,
    pub(super) interp_filter: InterpFilterCdfRows,
    pub(super) use_amvd: UseAmvdCdfRows,
    pub(super) use_bawp: UseBawpCdfRow,
    pub(super) use_bawp_chroma: UseBawpCdfRow,
    pub(super) explicit_bawp: ExplicitBawpCdfRows,
    pub(super) explicit_bawp_scale: ExplicitBawpScaleCdfRow,
    pub(super) use_wiener_ns: UseWienerNsCdfRow,
    pub(super) wiener_ns_length: WienerNsLengthCdfRows,
    pub(super) wiener_ns_uv_sym: WienerNsUvSymCdfRow,
    pub(super) wiener_ns_base: WienerNsBaseCdfRow,
    pub(super) is_long_side_dct: IsLongSideDctCdfRows,
    pub(super) intra_tx_type_long: IntraTxTypeLongCdfRows,
    pub(super) inter_tx_type_long: InterTxTypeLongCdfRows,
    pub(super) inter_tx_type_set1: InterTxTypeSet1CdfRows,
    pub(super) inter_tx_type_set2: InterTxTypeSet2CdfRows,
    pub(super) inter_tx_type_index_set1: InterTxTypeIndexSet1CdfRows,
    pub(super) inter_tx_type_index_set2: InterTxTypeIndexSet2CdfRows,
    pub(super) inter_tx_type_offset_set1: InterTxTypeOffsetSet1CdfRows,
    pub(super) inter_tx_type_offset_set2: InterTxTypeOffsetSet2CdfRows,
    pub(super) inter_tx_type_set3: InterTxTypeSet3CdfRows,
    pub(super) inter_tx_type_set4: InterTxTypeSet4CdfRows,
    pub(super) intra_tx_type_set1: IntraTxTypeSet1CdfRows,
    pub(super) intra_tx_type_set2: IntraTxTypeSet2CdfRows,
    pub(super) sec_tx_type: SecTxTypeCdfRows,
    pub(super) most_probable_stx_set: MostProbableStxSetCdfRow,
    pub(super) most_probable_stx_set_adst: MostProbableStxSetAdstCdfRow,
    pub(super) cctx_type: CctxTypeCdfRow,
    pub(super) palette_y_mode: PaletteYModeCdfRow,
    pub(super) palette_y_size: PaletteYSizeCdfRow,
    pub(super) identity_row_y: IdentityRowYCdfRows,
    pub(super) palette_size_2_y_color: PaletteSize2YColorCdfRows,
    pub(super) palette_size_3_y_color: PaletteSize3YColorCdfRows,
    pub(super) palette_size_4_y_color: PaletteSize4YColorCdfRows,
    pub(super) palette_size_5_y_color: PaletteSize5YColorCdfRows,
    pub(super) palette_size_6_y_color: PaletteSize6YColorCdfRows,
    pub(super) palette_size_7_y_color: PaletteSize7YColorCdfRows,
    pub(super) palette_size_8_y_color: PaletteSize8YColorCdfRows,
    pub(super) coeff: CoeffCdfRows,
}

macro_rules! checked_block_row {
    ($rows:expr, $index:expr, $index_name:literal, $array:expr, $get:ident) => {{
        let max_exclusive = $rows.len();
        $rows.$get($index).ok_or(TileCdfError::SelectorOutOfRange {
            array: $array,
            index_name: $index_name,
            actual: $index,
            max_exclusive,
        })
    }};
}

macro_rules! block_row_slice {
    ($rows:expr, $index:expr, $index_name:literal, $array:expr, $get:ident, $as_slice:ident) => {{
        let row = checked_block_row!($rows, $index, $index_name, $array, $get)?;
        Ok(row.$as_slice())
    }};
}

macro_rules! block_cdf_row {
    ($self:ident, $selector:ident, $get:ident, $as_slice:ident, $delegate:ident) => {
        match $selector {
            BlockCdfSelector::YModeSet => Ok($self.y_mode_set.$as_slice()),
            BlockCdfSelector::YModeIndex { ctx } => block_row_slice!(
                $self.y_mode_index,
                ctx,
                "ctx",
                TileCdfArray::YModeIndex,
                $get,
                $as_slice
            ),
            BlockCdfSelector::YModeOffset { ctx } => block_row_slice!(
                $self.y_mode_offset,
                ctx,
                "ctx",
                TileCdfArray::YModeOffset,
                $get,
                $as_slice
            ),
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
                block_row_slice!(
                    $self.txb_skip[coeff_cdf_q_ctx][plane_type][tx_size],
                    ctx,
                    "ctx",
                    TileCdfArray::TxbSkip,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::UvModeCflNotAllowed { ctx } => block_row_slice!(
                $self.uv_mode_cfl_not_allowed,
                ctx,
                "ctx",
                TileCdfArray::UvModeCflNotAllowed,
                $get,
                $as_slice
            ),
            BlockCdfSelector::IsCfl { ctx } => block_row_slice!(
                $self.is_cfl,
                ctx,
                "ctx",
                TileCdfArray::IsCfl,
                $get,
                $as_slice
            ),
            BlockCdfSelector::CflIndex => Ok($self.cfl_index.$as_slice()),
            BlockCdfSelector::CflSign => Ok($self.cfl_sign.$as_slice()),
            BlockCdfSelector::CflAlpha { ctx } => block_row_slice!(
                $self.cfl_alpha,
                ctx,
                "ctx",
                TileCdfArray::CflAlpha,
                $get,
                $as_slice
            ),
            BlockCdfSelector::CflMhccp => Ok($self.cfl_mhccp.$as_slice()),
            BlockCdfSelector::CflMhDir { size_group } => block_row_slice!(
                $self.cfl_mh_dir,
                size_group,
                "size_group",
                TileCdfArray::CflMhDir,
                $get,
                $as_slice
            ),
            BlockCdfSelector::VTxbSkip {
                coeff_cdf_q_ctx,
                ctx,
            } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::VTxbSkip, coeff_cdf_q_ctx)?;
                block_row_slice!(
                    $self.v_txb_skip[coeff_cdf_q_ctx],
                    ctx,
                    "ctx",
                    TileCdfArray::VTxbSkip,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::EobExtra { coeff_cdf_q_ctx } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::EobExtra, coeff_cdf_q_ctx)?;
                Ok($self.eob_extra[coeff_cdf_q_ctx].$as_slice())
            }
            BlockCdfSelector::EobPt {
                size,
                coeff_cdf_q_ctx,
                eob_ctx,
            } => {
                let q = checked_coeff_cdf_q_context(TileCdfArray::EobPt, coeff_cdf_q_ctx)?;
                let c = checked_eob_plane_ctx(eob_ctx)?;
                Ok(match size {
                    EobPtSize::Pt16 => $self.eob_pt_16[q][c].$as_slice(),
                    EobPtSize::Pt32 => $self.eob_pt_32[q][c].$as_slice(),
                    EobPtSize::Pt64 => $self.eob_pt_64[q][c].$as_slice(),
                    EobPtSize::Pt128 => $self.eob_pt_128[q][c].$as_slice(),
                    EobPtSize::Pt256 => $self.eob_pt_256[q][c].$as_slice(),
                    EobPtSize::Pt512 => $self.eob_pt_512[q][c].$as_slice(),
                    EobPtSize::Pt1024 => $self.eob_pt_1024[q][c].$as_slice(),
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
                block_row_slice!(
                    $self.dc_sign[q][plane_type][group],
                    ctx,
                    "ctx",
                    TileCdfArray::DcSign,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::IsInter { ctx } => block_row_slice!(
                $self.is_inter,
                ctx,
                "ctx",
                TileCdfArray::IsInter,
                $get,
                $as_slice
            ),
            BlockCdfSelector::Skip { ctx } => {
                block_row_slice!($self.skip, ctx, "ctx", TileCdfArray::Skip, $get, $as_slice)
            }
            BlockCdfSelector::SingleMode { ctx } => block_row_slice!(
                $self.single_mode,
                ctx,
                "ctx",
                TileCdfArray::SingleMode,
                $get,
                $as_slice
            ),
            BlockCdfSelector::IsWarp { ctx } => block_row_slice!(
                $self.is_warp,
                ctx,
                "ctx",
                TileCdfArray::IsWarp,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WarpMv => Ok($self.warp_mv.$as_slice()),
            BlockCdfSelector::WarpIdx { ctx } => block_row_slice!(
                $self.warp_idx,
                ctx,
                "ctx",
                TileCdfArray::WarpIdx,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WarpWithMvd => Ok($self.warp_with_mvd.$as_slice()),
            BlockCdfSelector::WarpPrecision { block_size } => block_row_slice!(
                $self.warp_precision,
                block_size,
                "block_size",
                TileCdfArray::WarpPrecision,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WarpDeltaParamLow { index_type } => block_row_slice!(
                $self.warp_delta_param_low,
                index_type,
                "index_type",
                TileCdfArray::WarpDeltaParamLow,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WarpDeltaParamHigh { index_type } => block_row_slice!(
                $self.warp_delta_param_high,
                index_type,
                "index_type",
                TileCdfArray::WarpDeltaParamHigh,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WarpDeltaParamSign => Ok($self.warp_delta_param_sign.$as_slice()),
            BlockCdfSelector::WarpInterIntra { bsize_group } => block_row_slice!(
                $self.warp_inter_intra,
                bsize_group,
                "bsize_group",
                TileCdfArray::WarpInterIntra,
                $get,
                $as_slice
            ),
            BlockCdfSelector::InterIntraMode { bsize_group } => block_row_slice!(
                $self.inter_intra_mode,
                bsize_group,
                "bsize_group",
                TileCdfArray::InterIntraMode,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WedgeInterIntra => Ok($self.wedge_inter_intra.$as_slice()),
            BlockCdfSelector::WedgeQuad => Ok($self.wedge_quad.$as_slice()),
            BlockCdfSelector::WedgeAngle { quad } => block_row_slice!(
                $self.wedge_angle,
                quad,
                "quad",
                TileCdfArray::WedgeAngle,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WedgeDist1 => Ok($self.wedge_dist1.$as_slice()),
            BlockCdfSelector::WedgeDist2 => Ok($self.wedge_dist2.$as_slice()),
            BlockCdfSelector::DrlMode { idx, ctx } => {
                let bank =
                    checked_block_row!($self.drl_mode, idx, "idx", TileCdfArray::DrlMode, $get)?;
                block_row_slice!(bank, ctx, "ctx", TileCdfArray::DrlMode, $get, $as_slice)
            }
            BlockCdfSelector::SingleRef { ctx, ref_idx } => {
                let bank = checked_block_row!(
                    $self.single_ref,
                    ctx,
                    "ctx",
                    TileCdfArray::SingleRef,
                    $get
                )?;
                block_row_slice!(
                    bank,
                    ref_idx,
                    "ref",
                    TileCdfArray::SingleRef,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::CompMode { ctx } => block_row_slice!(
                $self.comp_mode,
                ctx,
                "ctx",
                TileCdfArray::CompMode,
                $get,
                $as_slice
            ),
            BlockCdfSelector::IsJoint { ctx } => block_row_slice!(
                $self.is_joint,
                ctx,
                "ctx",
                TileCdfArray::IsJoint,
                $get,
                $as_slice
            ),
            BlockCdfSelector::CompoundModeNonJoint { ctx } => block_row_slice!(
                $self.compound_mode_non_joint,
                ctx,
                "ctx",
                TileCdfArray::CompoundModeNonJoint,
                $get,
                $as_slice
            ),
            BlockCdfSelector::CompGroupIdx { ctx } => block_row_slice!(
                $self.comp_group_idx,
                ctx,
                "ctx",
                TileCdfArray::CompGroupIdx,
                $get,
                $as_slice
            ),
            BlockCdfSelector::CwpIdx { idx } => block_row_slice!(
                $self.cwp_idx,
                idx,
                "idx",
                TileCdfArray::CwpIdx,
                $get,
                $as_slice
            ),
            BlockCdfSelector::CompRef0 { ctx, ref_idx } => {
                let bank =
                    checked_block_row!($self.comp_ref0, ctx, "ctx", TileCdfArray::CompRef0, $get)?;
                block_row_slice!(
                    bank,
                    ref_idx,
                    "ref",
                    TileCdfArray::CompRef0,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => {
                let bank =
                    checked_block_row!($self.comp_ref1, ctx, "ctx", TileCdfArray::CompRef1, $get)?;
                let bit_bank =
                    checked_block_row!(bank, bit_type, "bit_type", TileCdfArray::CompRef1, $get)?;
                block_row_slice!(
                    bit_bank,
                    ref_idx,
                    "ref",
                    TileCdfArray::CompRef1,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::ReadMv(selector) => $self.read_mv.$delegate(selector),
            BlockCdfSelector::InterpFilter { ctx } => block_row_slice!(
                $self.interp_filter,
                ctx,
                "ctx",
                TileCdfArray::InterpFilter,
                $get,
                $as_slice
            ),
            BlockCdfSelector::UseBawp => Ok($self.use_bawp.$as_slice()),
            BlockCdfSelector::UseBawpChroma => Ok($self.use_bawp_chroma.$as_slice()),
            BlockCdfSelector::UseAmvd { index, ctx } => {
                let bank = checked_block_row!(
                    $self.use_amvd,
                    index,
                    "index",
                    TileCdfArray::UseAmvd,
                    $get
                )?;
                block_row_slice!(bank, ctx, "ctx", TileCdfArray::UseAmvd, $get, $as_slice)
            }
            BlockCdfSelector::ExplicitBawp { ctx } => block_row_slice!(
                $self.explicit_bawp,
                ctx,
                "ctx",
                TileCdfArray::ExplicitBawp,
                $get,
                $as_slice
            ),
            BlockCdfSelector::ExplicitBawpScale => Ok($self.explicit_bawp_scale.$as_slice()),
            BlockCdfSelector::UseWienerNs => Ok($self.use_wiener_ns.$as_slice()),
            BlockCdfSelector::WienerNsLength { plane_ctx } => block_row_slice!(
                $self.wiener_ns_length,
                plane_ctx,
                "plane_ctx",
                TileCdfArray::WienerNsLength,
                $get,
                $as_slice
            ),
            BlockCdfSelector::WienerNsUvSym => Ok($self.wiener_ns_uv_sym.$as_slice()),
            BlockCdfSelector::WienerNsBase => Ok($self.wiener_ns_base.$as_slice()),
            BlockCdfSelector::IsLongSideDct { is_inter } => block_row_slice!(
                $self.is_long_side_dct,
                is_inter,
                "is_inter",
                TileCdfArray::IsLongSideDct,
                $get,
                $as_slice
            ),
            BlockCdfSelector::IntraTxTypeLong { tx_size_sqr } => block_row_slice!(
                $self.intra_tx_type_long,
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeLong,
                $get,
                $as_slice
            ),
            BlockCdfSelector::InterTxTypeLong { ctx, tx_size_sqr } => {
                let eob_row = checked_block_row!(
                    $self.inter_tx_type_long,
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeLong,
                    $get
                )?;
                block_row_slice!(
                    eob_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeLong,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::InterTxTypeSet1 { ctx, tx_size_sqr } => {
                let ctx_row = checked_block_row!(
                    $self.inter_tx_type_set1,
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeSet1,
                    $get
                )?;
                block_row_slice!(
                    ctx_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeSet1,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::InterTxTypeSet2 { ctx } => block_row_slice!(
                $self.inter_tx_type_set2,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeSet2,
                $get,
                $as_slice
            ),
            BlockCdfSelector::InterTxTypeIndexSet1 { ctx } => block_row_slice!(
                $self.inter_tx_type_index_set1,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeIndexSet1,
                $get,
                $as_slice
            ),
            BlockCdfSelector::InterTxTypeIndexSet2 { ctx } => block_row_slice!(
                $self.inter_tx_type_index_set2,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeIndexSet2,
                $get,
                $as_slice
            ),
            BlockCdfSelector::InterTxTypeOffsetSet1 { ctx } => block_row_slice!(
                $self.inter_tx_type_offset_set1,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeOffsetSet1,
                $get,
                $as_slice
            ),
            BlockCdfSelector::InterTxTypeOffsetSet2 { ctx } => block_row_slice!(
                $self.inter_tx_type_offset_set2,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeOffsetSet2,
                $get,
                $as_slice
            ),
            BlockCdfSelector::InterTxTypeSet3 { ctx, tx_size_sqr } => {
                let ctx_row = checked_block_row!(
                    $self.inter_tx_type_set3,
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeSet3,
                    $get
                )?;
                block_row_slice!(
                    ctx_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeSet3,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::InterTxTypeSet4 { ctx, tx_size_sqr } => {
                let ctx_row = checked_block_row!(
                    $self.inter_tx_type_set4,
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeSet4,
                    $get
                )?;
                block_row_slice!(
                    ctx_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeSet4,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::IntraTxTypeSet1 { tx_size_sqr } => block_row_slice!(
                $self.intra_tx_type_set1,
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeSet1,
                $get,
                $as_slice
            ),
            BlockCdfSelector::IntraTxTypeSet2 { tx_size_sqr } => block_row_slice!(
                $self.intra_tx_type_set2,
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeSet2,
                $get,
                $as_slice
            ),
            BlockCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            } => {
                let is_inter = checked_sec_tx_is_inter(is_inter)?;
                block_row_slice!(
                    $self.sec_tx_type[is_inter],
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::SecTxType,
                    $get,
                    $as_slice
                )
            }
            BlockCdfSelector::MostProbableStxSet => Ok($self.most_probable_stx_set.$as_slice()),
            BlockCdfSelector::MostProbableStxSetAdst => {
                Ok($self.most_probable_stx_set_adst.$as_slice())
            }
            BlockCdfSelector::CctxType => Ok($self.cctx_type.$as_slice()),
            BlockCdfSelector::PaletteYMode => Ok($self.palette_y_mode.$as_slice()),
            BlockCdfSelector::PaletteYSize => Ok($self.palette_y_size.$as_slice()),
            BlockCdfSelector::IdentityRowY { ctx } => block_row_slice!(
                $self.identity_row_y,
                ctx,
                "ctx",
                TileCdfArray::IdentityRowY,
                $get,
                $as_slice
            ),
            BlockCdfSelector::PaletteYColorIndex { palette_size, ctx } => {
                let ctx = checked_context(
                    TileCdfArray::PaletteYColorIndex,
                    "ctx",
                    ctx,
                    PALETTE_COLOR_CONTEXTS,
                )?;
                match palette_size {
                    2 => Ok($self.palette_size_2_y_color[ctx].$as_slice()),
                    3 => Ok($self.palette_size_3_y_color[ctx].$as_slice()),
                    4 => Ok($self.palette_size_4_y_color[ctx].$as_slice()),
                    5 => Ok($self.palette_size_5_y_color[ctx].$as_slice()),
                    6 => Ok($self.palette_size_6_y_color[ctx].$as_slice()),
                    7 => Ok($self.palette_size_7_y_color[ctx].$as_slice()),
                    8 => Ok($self.palette_size_8_y_color[ctx].$as_slice()),
                    _ => Err(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::PaletteYColorIndex,
                        index_name: "palette_size",
                        actual: palette_size,
                        max_exclusive: 9,
                    }),
                }
            }
            BlockCdfSelector::Coeff(selector) => $self.coeff.$delegate(selector),
        }
    };
}

macro_rules! block_cdf_count_rows {
    ($row:ident, $rows:ident, $read_mv:block, $coeff:block) => {{
        $row!(y_mode_set);
        $rows!(y_mode_index);
        $rows!(txb_skip.flatten().flatten().flatten());
        $rows!(v_txb_skip.flatten());
        $rows!(eob_extra);
        $rows!(uv_mode_cfl_not_allowed);
        $rows!(is_cfl);
        $row!(cfl_index);
        $row!(cfl_sign);
        $rows!(cfl_alpha);
        $row!(cfl_mhccp);
        $rows!(cfl_mh_dir);
        $rows!(eob_pt_16.flatten());
        $rows!(eob_pt_32.flatten());
        $rows!(eob_pt_64.flatten());
        $rows!(eob_pt_128.flatten());
        $rows!(eob_pt_256.flatten());
        $rows!(eob_pt_512.flatten());
        $rows!(eob_pt_1024.flatten());
        $rows!(dc_sign.flatten().flatten().flatten());
        $rows!(is_inter);
        $rows!(skip);
        $rows!(single_mode);
        $rows!(is_warp);
        $row!(warp_mv);
        $rows!(warp_idx);
        $row!(warp_with_mvd);
        $rows!(warp_precision);
        $rows!(warp_delta_param_low);
        $rows!(warp_delta_param_high);
        $row!(warp_delta_param_sign);
        $rows!(warp_inter_intra);
        $rows!(inter_intra_mode);
        $row!(wedge_inter_intra);
        $row!(wedge_quad);
        $rows!(wedge_angle);
        $row!(wedge_dist1);
        $row!(wedge_dist2);
        $rows!(drl_mode.flatten());
        $rows!(single_ref.flatten());
        $rows!(comp_mode);
        $rows!(is_joint);
        $rows!(compound_mode_non_joint);
        $rows!(comp_group_idx);
        $rows!(cwp_idx);
        $rows!(comp_ref0.flatten());
        $rows!(comp_ref1.flatten().flatten());
        $read_mv
        $rows!(interp_filter);
        $rows!(use_amvd.flatten());
        $row!(use_bawp);
        $row!(use_bawp_chroma);
        $rows!(explicit_bawp);
        $row!(explicit_bawp_scale);
        $row!(use_wiener_ns);
        $rows!(wiener_ns_length);
        $row!(wiener_ns_uv_sym);
        $row!(wiener_ns_base);
        $rows!(is_long_side_dct);
        $rows!(intra_tx_type_long);
        $rows!(inter_tx_type_long.flatten());
        $rows!(inter_tx_type_set1.flatten());
        $rows!(inter_tx_type_set3.flatten());
        $rows!(inter_tx_type_set4.flatten());
        $rows!(inter_tx_type_set2);
        $rows!(inter_tx_type_index_set1);
        $rows!(inter_tx_type_index_set2);
        $rows!(inter_tx_type_offset_set1);
        $rows!(inter_tx_type_offset_set2);
        $rows!(intra_tx_type_set1);
        $rows!(intra_tx_type_set2);
        $rows!(sec_tx_type.flatten());
        $row!(most_probable_stx_set);
        $row!(most_probable_stx_set_adst);
        $row!(cctx_type);
        $row!(palette_y_mode);
        $row!(palette_y_size);
        $rows!(identity_row_y);
        $rows!(palette_size_2_y_color);
        $rows!(palette_size_3_y_color);
        $rows!(palette_size_4_y_color);
        $rows!(palette_size_5_y_color);
        $rows!(palette_size_6_y_color);
        $rows!(palette_size_7_y_color);
        $rows!(palette_size_8_y_color);
        $coeff
    }};
}

impl BlockCdfRows {
    pub(crate) fn from_defaults() -> Self {
        Self {
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index: DEFAULT_Y_MODE_INDEX_CDF,
            y_mode_offset: DEFAULT_Y_MODE_OFFSET_CDF,
            txb_skip: DEFAULT_TXB_SKIP_CDF,
            uv_mode_cfl_not_allowed: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
            is_cfl: DEFAULT_IS_CFL_CDF,
            cfl_index: DEFAULT_CFL_INDEX_CDF,
            cfl_sign: DEFAULT_CFL_SIGN_CDF,
            cfl_alpha: DEFAULT_CFL_ALPHA_CDF,
            cfl_mhccp: DEFAULT_CFL_MHCCP_CDF,
            cfl_mh_dir: DEFAULT_CFL_MH_DIR_CDF,
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
            is_warp: DEFAULT_IS_WARP_CDF,
            warp_mv: DEFAULT_WARP_MV_CDF,
            warp_idx: DEFAULT_WARP_IDX_CDF,
            warp_with_mvd: DEFAULT_WARP_WITH_MVD_CDF,
            warp_precision: DEFAULT_WARP_PRECISION_CDF,
            warp_delta_param_low: DEFAULT_WARP_DELTA_PARAM_LOW_CDF,
            warp_delta_param_high: DEFAULT_WARP_DELTA_PARAM_HIGH_CDF,
            warp_delta_param_sign: DEFAULT_WARP_DELTA_PARAM_SIGN_CDF,
            warp_inter_intra: DEFAULT_WARP_INTER_INTRA_CDF,
            inter_intra_mode: DEFAULT_INTER_INTRA_MODE_CDF,
            wedge_inter_intra: DEFAULT_WEDGE_INTER_INTRA_CDF,
            wedge_quad: DEFAULT_WEDGE_QUAD_CDF,
            wedge_angle: DEFAULT_WEDGE_ANGLE_CDF,
            wedge_dist1: DEFAULT_WEDGE_DIST1_CDF,
            wedge_dist2: DEFAULT_WEDGE_DIST2_CDF,
            drl_mode: DEFAULT_DRL_MODE_CDF,
            single_ref: DEFAULT_SINGLE_REF_CDF,
            comp_mode: DEFAULT_COMP_MODE_CDF,
            is_joint: DEFAULT_IS_JOINT_CDF,
            compound_mode_non_joint: DEFAULT_COMPOUND_MODE_NON_JOINT_CDF,
            comp_group_idx: DEFAULT_COMP_GROUP_IDX_CDF,
            cwp_idx: DEFAULT_CWP_IDX_CDF,
            comp_ref0: DEFAULT_COMP_REF0_CDF,
            comp_ref1: DEFAULT_COMP_REF1_CDF,
            read_mv: MvCdfRows::from_defaults(),
            interp_filter: DEFAULT_INTERP_FILTER_CDF,
            use_amvd: DEFAULT_USE_AMVD_CDF,
            use_bawp: DEFAULT_USE_BAWP_CDF,
            use_bawp_chroma: DEFAULT_USE_BAWP_CHROMA_CDF,
            explicit_bawp: DEFAULT_EXPLICIT_BAWP_CDF,
            explicit_bawp_scale: DEFAULT_EXPLICIT_BAWP_SCALE_CDF,
            use_wiener_ns: DEFAULT_USE_WIENER_NS_CDF,
            wiener_ns_length: DEFAULT_WIENER_NS_LENGTH_CDF,
            wiener_ns_uv_sym: DEFAULT_WIENER_NS_UV_SYM_CDF,
            wiener_ns_base: DEFAULT_WIENER_NS_BASE_CDF,
            is_long_side_dct: DEFAULT_IS_LONG_SIDE_DCT_CDF,
            intra_tx_type_long: DEFAULT_INTRA_TX_TYPE_LONG_CDF,
            inter_tx_type_long: DEFAULT_INTER_TX_TYPE_LONG_CDF,
            inter_tx_type_set1: DEFAULT_INTER_TX_TYPE_SET1_CDF,
            inter_tx_type_set2: DEFAULT_INTER_TX_TYPE_SET2_CDF,
            inter_tx_type_index_set1: DEFAULT_INTER_TX_TYPE_INDEX_SET1_CDF,
            inter_tx_type_index_set2: DEFAULT_INTER_TX_TYPE_INDEX_SET2_CDF,
            inter_tx_type_offset_set1: DEFAULT_INTER_TX_TYPE_OFFSET_SET1_CDF,
            inter_tx_type_offset_set2: DEFAULT_INTER_TX_TYPE_OFFSET_SET2_CDF,
            inter_tx_type_set3: DEFAULT_INTER_TX_TYPE_SET3_CDF,
            inter_tx_type_set4: DEFAULT_INTER_TX_TYPE_SET4_CDF,
            intra_tx_type_set1: DEFAULT_INTRA_TX_TYPE_SET1_CDF,
            intra_tx_type_set2: DEFAULT_INTRA_TX_TYPE_SET2_CDF,
            sec_tx_type: DEFAULT_SEC_TX_TYPE_CDF,
            most_probable_stx_set: DEFAULT_MOST_PROBABLE_STX_SET_CDF,
            most_probable_stx_set_adst: DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF,
            cctx_type: DEFAULT_CCTX_TYPE_CDF,
            palette_y_mode: DEFAULT_PALETTE_Y_MODE_CDF,
            palette_y_size: DEFAULT_PALETTE_Y_SIZE_CDF,
            identity_row_y: DEFAULT_IDENTITY_ROW_Y_CDF,
            palette_size_2_y_color: DEFAULT_PALETTE_SIZE_2_Y_COLOR_CDF,
            palette_size_3_y_color: DEFAULT_PALETTE_SIZE_3_Y_COLOR_CDF,
            palette_size_4_y_color: DEFAULT_PALETTE_SIZE_4_Y_COLOR_CDF,
            palette_size_5_y_color: DEFAULT_PALETTE_SIZE_5_Y_COLOR_CDF,
            palette_size_6_y_color: DEFAULT_PALETTE_SIZE_6_Y_COLOR_CDF,
            palette_size_7_y_color: DEFAULT_PALETTE_SIZE_7_Y_COLOR_CDF,
            palette_size_8_y_color: DEFAULT_PALETTE_SIZE_8_Y_COLOR_CDF,
            coeff: CoeffCdfRows::from_defaults(),
        }
    }

    pub(crate) fn replicate_coeff_q_context(
        &mut self,
        coeff_cdf_q_ctx: usize,
    ) -> Result<(), TileCdfError> {
        let q = checked_coeff_cdf_q_context(TileCdfArray::TxbSkip, coeff_cdf_q_ctx)?;
        self.txb_skip = [self.txb_skip[q]; COEFF_CDF_Q_CONTEXTS];
        self.v_txb_skip = [self.v_txb_skip[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_extra = [self.eob_extra[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_pt_16 = [self.eob_pt_16[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_pt_32 = [self.eob_pt_32[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_pt_64 = [self.eob_pt_64[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_pt_128 = [self.eob_pt_128[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_pt_256 = [self.eob_pt_256[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_pt_512 = [self.eob_pt_512[q]; COEFF_CDF_Q_CONTEXTS];
        self.eob_pt_1024 = [self.eob_pt_1024[q]; COEFF_CDF_Q_CONTEXTS];
        self.dc_sign = [self.dc_sign[q]; COEFF_CDF_Q_CONTEXTS];
        self.coeff.replicate_q_context(q)
    }

    pub(crate) fn row(&self, selector: BlockCdfSelector) -> Result<&[i32], TileCdfError> {
        block_cdf_row!(self, selector, get, as_slice, row)
    }

    pub(crate) fn row_mut(
        &mut self,
        selector: BlockCdfSelector,
    ) -> Result<&mut [i32], TileCdfError> {
        block_cdf_row!(self, selector, get_mut, as_mut_slice, row_mut)
    }

    pub(crate) fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        macro_rules! avg_row {
            ($field:ident) => {
                avg_cdf_row(&mut self.$field, &tile.$field, tile_num, num_log2);
            };
        }
        macro_rules! avg_rows {
            ($field:ident $(. $flatten:ident())*) => {
                avg_cdf_rows(
                    self.$field.iter_mut()$(.$flatten())*,
                    tile.$field.iter()$(.$flatten())*,
                    tile_num,
                    num_log2,
                );
            };
        }

        block_cdf_count_rows!(
            avg_row,
            avg_rows,
            {
                self.read_mv
                    .average_from_tile(&tile.read_mv, tile_num, num_log2);
            },
            {
                self.coeff.avg_from_tile(tile_num, &tile.coeff, num_log2);
            }
        );
    }

    pub(crate) fn scale_counts_for_frame_end_update(&mut self) {
        macro_rules! scale_row {
            ($field:ident) => {
                scale_cdf_count(&mut self.$field);
            };
        }
        macro_rules! scale_rows {
            ($field:ident $(. $flatten:ident())*) => {
                scale_cdf_rows(self.$field.iter_mut()$(.$flatten())*);
            };
        }

        block_cdf_count_rows!(
            scale_row,
            scale_rows,
            {
                self.read_mv.scale_counts();
            },
            {
                self.coeff.scale_counts_for_frame_end_update();
            }
        );
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
    pub(crate) const fn is_cfl(&self) -> &IsCflCdfRows {
        &self.is_cfl
    }

    #[cfg(test)]
    pub(crate) const fn cfl_index(&self) -> &CflIndexCdfRow {
        &self.cfl_index
    }

    #[cfg(test)]
    pub(crate) const fn cfl_sign(&self) -> &CflSignCdfRow {
        &self.cfl_sign
    }

    #[cfg(test)]
    pub(crate) const fn cfl_alpha(&self) -> &CflAlphaCdfRows {
        &self.cfl_alpha
    }

    #[cfg(test)]
    pub(crate) const fn cfl_mhccp(&self) -> &CflMhccpCdfRow {
        &self.cfl_mhccp
    }

    #[cfg(test)]
    pub(crate) const fn cfl_mh_dir(&self) -> &CflMhDirCdfRows {
        &self.cfl_mh_dir
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

    #[cfg(test)]
    pub(crate) const fn wiener_ns_length(&self) -> &WienerNsLengthCdfRows {
        &self.wiener_ns_length
    }

    #[cfg(test)]
    pub(crate) const fn wiener_ns_uv_sym(&self) -> &WienerNsUvSymCdfRow {
        &self.wiener_ns_uv_sym
    }

    #[cfg(test)]
    pub(crate) const fn wiener_ns_base(&self) -> &WienerNsBaseCdfRow {
        &self.wiener_ns_base
    }

    #[cfg(test)]
    pub(crate) const fn is_long_side_dct(&self) -> &IsLongSideDctCdfRows {
        &self.is_long_side_dct
    }

    #[cfg(test)]
    pub(crate) const fn intra_tx_type_long(&self) -> &IntraTxTypeLongCdfRows {
        &self.intra_tx_type_long
    }

    #[cfg(test)]
    pub(crate) const fn intra_tx_type_set1(&self) -> &IntraTxTypeSet1CdfRows {
        &self.intra_tx_type_set1
    }

    #[cfg(test)]
    pub(crate) const fn intra_tx_type_set2(&self) -> &IntraTxTypeSet2CdfRows {
        &self.intra_tx_type_set2
    }

    #[cfg(test)]
    pub(crate) const fn sec_tx_type(&self) -> &SecTxTypeCdfRows {
        &self.sec_tx_type
    }

    #[cfg(test)]
    pub(crate) const fn most_probable_stx_set(&self) -> &MostProbableStxSetCdfRow {
        &self.most_probable_stx_set
    }

    #[cfg(test)]
    pub(crate) const fn most_probable_stx_set_adst(&self) -> &MostProbableStxSetAdstCdfRow {
        &self.most_probable_stx_set_adst
    }

    #[cfg(test)]
    pub(crate) const fn cctx_type(&self) -> &CctxTypeCdfRow {
        &self.cctx_type
    }

    #[cfg(test)]
    pub(crate) const fn palette_y_mode(&self) -> &PaletteYModeCdfRow {
        &self.palette_y_mode
    }
}

fn checked_eob_plane_ctx(eob_ctx: usize) -> Result<usize, TileCdfError> {
    checked_context(TileCdfArray::EobPt, "eob_ctx", eob_ctx, EOB_PLANE_CTXS)
}

fn checked_sec_tx_is_inter(is_inter: usize) -> Result<usize, TileCdfError> {
    checked_context(
        TileCdfArray::SecTxType,
        "is_inter",
        is_inter,
        SEC_TX_TYPE_IS_INTER_CONTEXTS,
    )
}

fn checked_dc_sign_group(group: usize) -> Result<usize, TileCdfError> {
    checked_context(TileCdfArray::DcSign, "group", group, DC_SIGN_GROUPS)
}

fn checked_coeff_cdf_q_context(
    array: TileCdfArray,
    coeff_cdf_q_ctx: usize,
) -> Result<usize, TileCdfError> {
    checked_context(
        array,
        "coeff_cdf_q_ctx",
        coeff_cdf_q_ctx,
        COEFF_CDF_Q_CONTEXTS,
    )
}

fn checked_plane_type(array: TileCdfArray, plane_type: usize) -> Result<usize, TileCdfError> {
    checked_context(array, "plane_type", plane_type, PLANE_TYPES)
}

fn checked_tx_size(array: TileCdfArray, tx_size: usize) -> Result<usize, TileCdfError> {
    checked_context(array, "tx_size", tx_size, TX_SIZE_CONTEXTS)
}
