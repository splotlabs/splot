// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 block-symbol CDF rows.

use splot_core::tables::cdf::{
    DEFAULT_CCTX_TYPE_CDF, DEFAULT_CFL_ALPHA_CDF, DEFAULT_CFL_INDEX_CDF, DEFAULT_CFL_MH_DIR_CDF,
    DEFAULT_CFL_MHCCP_CDF, DEFAULT_CFL_SIGN_CDF, DEFAULT_COMP_GROUP_IDX_CDF, DEFAULT_COMP_MODE_CDF,
    DEFAULT_COMP_REF0_CDF, DEFAULT_COMP_REF1_CDF, DEFAULT_COMPOUND_MODE_NON_JOINT_CDF,
    DEFAULT_COMPOUND_MODE_SAME_REFS_CDF, DEFAULT_COMPOUND_TYPE_CDF, DEFAULT_CWP_IDX_CDF,
    DEFAULT_DC_SIGN_CDF, DEFAULT_DIP_MODE_CDF, DEFAULT_DPCM_MODE_UV_CDF, DEFAULT_DPCM_MODE_Y_CDF,
    DEFAULT_DRL_MODE_CDF, DEFAULT_EOB_EXTRA_CDF, DEFAULT_EOB_PT_16_CDF, DEFAULT_EOB_PT_32_CDF,
    DEFAULT_EOB_PT_64_CDF, DEFAULT_EOB_PT_128_CDF, DEFAULT_EOB_PT_256_CDF, DEFAULT_EOB_PT_512_CDF,
    DEFAULT_EOB_PT_1024_CDF, DEFAULT_EXPLICIT_BAWP_CDF, DEFAULT_EXPLICIT_BAWP_SCALE_CDF,
    DEFAULT_IDENTITY_ROW_Y_CDF, DEFAULT_INTER_INTRA_CDF, DEFAULT_INTER_INTRA_MODE_CDF,
    DEFAULT_INTER_TX_TYPE_INDEX_SET1_CDF, DEFAULT_INTER_TX_TYPE_INDEX_SET2_CDF,
    DEFAULT_INTER_TX_TYPE_LONG_CDF, DEFAULT_INTER_TX_TYPE_OFFSET_SET1_CDF,
    DEFAULT_INTER_TX_TYPE_OFFSET_SET2_CDF, DEFAULT_INTER_TX_TYPE_SET1_CDF,
    DEFAULT_INTER_TX_TYPE_SET2_CDF, DEFAULT_INTER_TX_TYPE_SET3_CDF, DEFAULT_INTER_TX_TYPE_SET4_CDF,
    DEFAULT_INTERP_FILTER_CDF, DEFAULT_INTRA_TX_TYPE_LONG_CDF, DEFAULT_INTRA_TX_TYPE_SET1_CDF,
    DEFAULT_INTRA_TX_TYPE_SET2_CDF, DEFAULT_IS_CFL_CDF, DEFAULT_IS_INTER_CDF, DEFAULT_IS_JOINT_CDF,
    DEFAULT_IS_LONG_SIDE_DCT_CDF, DEFAULT_IS_WARP_CDF, DEFAULT_JMVD_ADAPTIVE_SCALE_MODE_CDF,
    DEFAULT_JMVD_SCALE_MODE_CDF, DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF,
    DEFAULT_MOST_PROBABLE_STX_SET_CDF, DEFAULT_PALETTE_SIZE_2_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_3_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_4_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_5_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_6_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_7_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_8_Y_COLOR_CDF,
    DEFAULT_PALETTE_Y_MODE_CDF, DEFAULT_PALETTE_Y_SIZE_CDF, DEFAULT_PB_MV_PRECISION_CDF,
    DEFAULT_SEC_TX_TYPE_CDF, DEFAULT_SINGLE_MODE_CDF, DEFAULT_SINGLE_REF_CDF, DEFAULT_SKIP_CDF,
    DEFAULT_SKIP_DRL_MODE_CDF, DEFAULT_SKIP_MODE_CDF, DEFAULT_TIP_DRL_MODE_CDF,
    DEFAULT_TIP_MODE_CDF, DEFAULT_TIP_PRED_MODE_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_USE_AMVD_CDF,
    DEFAULT_USE_BAWP_CDF, DEFAULT_USE_BAWP_CHROMA_CDF, DEFAULT_USE_DIP_CDF,
    DEFAULT_USE_DPCM_UV_CDF, DEFAULT_USE_DPCM_Y_CDF, DEFAULT_USE_EXTEND_WARP_CDF,
    DEFAULT_USE_LOCAL_WARP_CDF, DEFAULT_USE_MOST_PROBABLE_PRECISION_CDF, DEFAULT_USE_OPTFLOW_CDF,
    DEFAULT_USE_PC_WIENER_CDF, DEFAULT_USE_WIENER_NS_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
    DEFAULT_V_TXB_SKIP_CDF, DEFAULT_WARP_DELTA_PARAM_HIGH_CDF, DEFAULT_WARP_DELTA_PARAM_LOW_CDF,
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
use super::coeff_rows::CoeffCdfRows;
use super::{
    CDF_ROW_LEN, TileCdfArray, TileCdfError, TileCdfSelector, avg_cdf_row, avg_cdf_rows,
    blend_cdf_row, blend_cdf_rows, checked_context, scale_cdf_count, scale_cdf_rows,
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
const DIP_CONTEXTS: usize = 3;
const DIP_MODE_ROW_LEN: usize = 7;
const CFL_ALPHA_CONTEXTS: usize = 6;
const CFL_ALPHA_CDF_ROW_LEN: usize = 9;
const CFL_SIGN_CDF_ROW_LEN: usize = 9;
const CFL_MH_DIR_GROUPS: usize = 4;
const CFL_MH_DIR_CDF_ROW_LEN: usize = 4;
const V_TXB_SKIP_CONTEXTS: usize = 12;
const DC_SIGN_GROUPS: usize = 2;
const DC_SIGN_CONTEXTS: usize = 3;
const IS_INTER_CONTEXTS: usize = 4;
const SKIP_MODE_CONTEXTS: usize = 3;
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
const TIP_CONTEXTS: usize = 3;
const REF_CONTEXTS: usize = 3;
const REFS_PER_FRAME_MINUS_1: usize = 6;
const COMP_MODE_CONTEXTS: usize = 5;
const IS_JOINT_CONTEXTS: usize = 2;
const COMPOUND_MODE_CONTEXTS: usize = 5;
const COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN: usize = 6;
const COMPOUND_MODE_SAME_REFS_CDF_ROW_LEN: usize = 5;
const JMVD_SCALE_MODE_CDF_ROW_LEN: usize = 6;
const JMVD_ADAPTIVE_SCALE_MODE_CDF_ROW_LEN: usize = 4;
const COMP_GROUP_IDX_CONTEXTS: usize = 12;
const CWP_IDX_CONTEXTS: usize = 4;
const COMP_REF1_BIT_TYPES: usize = 2;
const INTERP_FILTER_CONTEXTS: usize = 16;
const AMVD_MODE_CONTEXTS: usize = 9;
const AMVD_CONTEXTS: usize = 3;
const USE_OPTFLOW_CONTEXTS: usize = 2;
const MOST_PROBABLE_PRECISION_CONTEXTS: usize = 3;
const USE_EXTEND_WARP_CONTEXTS: usize = 3;
const USE_LOCAL_WARP_CONTEXTS: usize = 4;
const PB_MV_PRECISION_CONTEXTS: usize = 2;
const PB_MV_PRECISION_FRAME_CONTEXTS: usize = 3;
const PB_MV_PRECISION_CDF_ROW_LEN: usize = 4;
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
pub(crate) type DpcmCdfRow = [i32; CDF_ROW_LEN];
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
pub(crate) type UseDipCdfRows = [[i32; CDF_ROW_LEN]; DIP_CONTEXTS];
pub(crate) type DipModeCdfRow = [i32; DIP_MODE_ROW_LEN];
pub(crate) type VTxbSkipCdfRows = [[[i32; CDF_ROW_LEN]; V_TXB_SKIP_CONTEXTS]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type EobExtraCdfRows = [[i32; CDF_ROW_LEN]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type DcSignCdfRows =
    [[[[[i32; CDF_ROW_LEN]; DC_SIGN_CONTEXTS]; DC_SIGN_GROUPS]; PLANE_TYPES]; COEFF_CDF_Q_CONTEXTS];
pub(crate) type IsInterCdfRows = [[i32; CDF_ROW_LEN]; IS_INTER_CONTEXTS];
pub(crate) type SkipModeCdfRows = [[i32; CDF_ROW_LEN]; SKIP_MODE_CONTEXTS];
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
pub(crate) type InterIntraCdfRows = [[i32; CDF_ROW_LEN]; BLOCK_SIZE_GROUPS];
pub(crate) type InterIntraModeCdfRows = [[i32; INTERINTRA_MODE_ROW_LEN]; BLOCK_SIZE_GROUPS];
pub(crate) type WedgeInterIntraCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type WedgeQuadCdfRow = [i32; WEDGE_QUAD_ROW_LEN];
pub(crate) type WedgeAngleCdfRows = [[i32; WEDGE_ANGLE_ROW_LEN]; WEDGE_ANGLE_CONTEXTS];
pub(crate) type WedgeDist1CdfRow = [i32; WEDGE_DIST1_ROW_LEN];
pub(crate) type WedgeDist2CdfRow = [i32; WEDGE_DIST2_ROW_LEN];
pub(crate) type DrlModeCdfRows = [[[i32; CDF_ROW_LEN]; DRL_MODE_CONTEXTS]; DRL_MODE_IDX_BANKS];
pub(crate) type SkipDrlModeCdfRows = [[i32; CDF_ROW_LEN]; DRL_MODE_IDX_BANKS];
pub(crate) type TipModeCdfRows = [[i32; CDF_ROW_LEN]; TIP_CONTEXTS];
pub(crate) type TipPredModeCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type TipDrlModeCdfRows = [[i32; CDF_ROW_LEN]; DRL_MODE_IDX_BANKS];
pub(crate) type SingleRefCdfRows = [[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; REF_CONTEXTS];
pub(crate) type CompModeCdfRows = [[i32; CDF_ROW_LEN]; COMP_MODE_CONTEXTS];
pub(crate) type IsJointCdfRows = [[i32; CDF_ROW_LEN]; IS_JOINT_CONTEXTS];
pub(crate) type JmvdScaleModeCdfRow = [i32; JMVD_SCALE_MODE_CDF_ROW_LEN];
pub(crate) type JmvdAdaptiveScaleModeCdfRow = [i32; JMVD_ADAPTIVE_SCALE_MODE_CDF_ROW_LEN];
pub(crate) type CompoundModeNonJointCdfRows =
    [[i32; COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN]; COMPOUND_MODE_CONTEXTS];
pub(crate) type CompoundModeSameRefsCdfRows =
    [[i32; COMPOUND_MODE_SAME_REFS_CDF_ROW_LEN]; COMPOUND_MODE_CONTEXTS];
pub(crate) type CompoundTypeCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type CompGroupIdxCdfRows = [[i32; CDF_ROW_LEN]; COMP_GROUP_IDX_CONTEXTS];
pub(crate) type CwpIdxCdfRows = [[i32; CDF_ROW_LEN]; CWP_IDX_CONTEXTS];
pub(crate) type CompRef0CdfRows = [[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; REF_CONTEXTS];
pub(crate) type CompRef1CdfRows =
    [[[[i32; CDF_ROW_LEN]; REFS_PER_FRAME_MINUS_1]; COMP_REF1_BIT_TYPES]; REF_CONTEXTS];
pub(crate) type UseAmvdCdfRows = [[[i32; CDF_ROW_LEN]; AMVD_CONTEXTS]; AMVD_MODE_CONTEXTS];
pub(crate) type UseOptflowCdfRows = [[i32; CDF_ROW_LEN]; USE_OPTFLOW_CONTEXTS];
pub(crate) type UseExtendWarpCdfRows = [[i32; CDF_ROW_LEN]; USE_EXTEND_WARP_CONTEXTS];
pub(crate) type UseLocalWarpCdfRows = [[i32; CDF_ROW_LEN]; USE_LOCAL_WARP_CONTEXTS];
pub(crate) type UseMostProbablePrecisionCdfRows =
    [[i32; CDF_ROW_LEN]; MOST_PROBABLE_PRECISION_CONTEXTS];
pub(crate) type PbMvPrecisionCdfRows = [[[i32; PB_MV_PRECISION_CDF_ROW_LEN];
    PB_MV_PRECISION_FRAME_CONTEXTS];
    PB_MV_PRECISION_CONTEXTS];
pub(crate) type UseBawpCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type ExplicitBawpCdfRows = [[i32; CDF_ROW_LEN]; BAWP_SCALES_CONTEXTS];
pub(crate) type ExplicitBawpScaleCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type UseWienerNsCdfRow = [i32; CDF_ROW_LEN];
pub(crate) type UsePcWienerCdfRow = [i32; CDF_ROW_LEN];
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockCdfRows {
    pub(crate) use_dpcm_y: DpcmCdfRow,
    pub(crate) dpcm_mode_y: DpcmCdfRow,
    pub(crate) use_dpcm_uv: DpcmCdfRow,
    pub(crate) dpcm_mode_uv: DpcmCdfRow,
    pub(crate) y_mode_set: YModeSetCdfRow,
    pub(crate) y_mode_index: YModeIndexCdfRows,
    pub(crate) y_mode_offset: YModeOffsetCdfRows,
    pub(crate) txb_skip: TxbSkipCdfRows,
    pub(crate) uv_mode_cfl_not_allowed: UvModeCflNotAllowedCdfRows,
    pub(crate) is_cfl: IsCflCdfRows,
    pub(crate) cfl_index: CflIndexCdfRow,
    pub(crate) cfl_sign: CflSignCdfRow,
    pub(crate) cfl_alpha: CflAlphaCdfRows,
    pub(crate) cfl_mhccp: CflMhccpCdfRow,
    pub(crate) cfl_mh_dir: CflMhDirCdfRows,
    pub(crate) use_dip: UseDipCdfRows,
    pub(crate) dip_mode: DipModeCdfRow,
    pub(crate) v_txb_skip: VTxbSkipCdfRows,
    pub(crate) eob_extra: EobExtraCdfRows,
    pub(crate) eob_pt_16: EobPt16CdfRows,
    pub(crate) eob_pt_32: EobPt32CdfRows,
    pub(crate) eob_pt_64: EobPt64CdfRows,
    pub(crate) eob_pt_128: EobPt128CdfRows,
    pub(crate) eob_pt_256: EobPt256CdfRows,
    pub(crate) eob_pt_512: EobPt512CdfRows,
    pub(crate) eob_pt_1024: EobPt1024CdfRows,
    pub(crate) dc_sign: DcSignCdfRows,
    pub(crate) is_inter: IsInterCdfRows,
    pub(crate) skip_mode: SkipModeCdfRows,
    pub(crate) skip: SkipCdfRows,
    pub(crate) single_mode: SingleModeCdfRows,
    pub(crate) is_warp: IsWarpCdfRows,
    pub(crate) warp_mv: WarpMvCdfRow,
    pub(crate) warp_idx: WarpIdxCdfRows,
    pub(crate) warp_with_mvd: WarpWithMvdCdfRow,
    pub(crate) warp_precision: WarpPrecisionCdfRows,
    pub(crate) warp_delta_param_low: WarpDeltaParamCdfRows,
    pub(crate) warp_delta_param_high: WarpDeltaParamCdfRows,
    pub(crate) warp_delta_param_sign: WarpDeltaParamSignCdfRow,
    pub(crate) warp_inter_intra: WarpInterIntraCdfRows,
    pub(crate) inter_intra: InterIntraCdfRows,
    pub(crate) inter_intra_mode: InterIntraModeCdfRows,
    pub(crate) wedge_inter_intra: WedgeInterIntraCdfRow,
    pub(crate) wedge_quad: WedgeQuadCdfRow,
    pub(crate) wedge_angle: WedgeAngleCdfRows,
    pub(crate) wedge_dist1: WedgeDist1CdfRow,
    pub(crate) wedge_dist2: WedgeDist2CdfRow,
    pub(crate) drl_mode: DrlModeCdfRows,
    pub(crate) skip_drl_mode: SkipDrlModeCdfRows,
    pub(crate) tip_mode: TipModeCdfRows,
    pub(crate) tip_pred_mode: TipPredModeCdfRow,
    pub(crate) tip_drl_mode: TipDrlModeCdfRows,
    pub(crate) single_ref: SingleRefCdfRows,
    pub(crate) comp_mode: CompModeCdfRows,
    pub(crate) is_joint: IsJointCdfRows,
    pub(crate) jmvd_scale_mode: JmvdScaleModeCdfRow,
    pub(crate) jmvd_adaptive_scale_mode: JmvdAdaptiveScaleModeCdfRow,
    pub(crate) compound_mode_non_joint: CompoundModeNonJointCdfRows,
    pub(crate) compound_mode_same_refs: CompoundModeSameRefsCdfRows,
    pub(crate) compound_type: CompoundTypeCdfRow,
    pub(crate) comp_group_idx: CompGroupIdxCdfRows,
    pub(crate) cwp_idx: CwpIdxCdfRows,
    pub(crate) comp_ref0: CompRef0CdfRows,
    pub(crate) comp_ref1: CompRef1CdfRows,
    read_mv: MvCdfRows,
    pub(crate) interp_filter: InterpFilterCdfRows,
    pub(crate) use_amvd: UseAmvdCdfRows,
    pub(crate) use_optflow: UseOptflowCdfRows,
    pub(crate) use_extend_warp: UseExtendWarpCdfRows,
    pub(crate) use_local_warp: UseLocalWarpCdfRows,
    pub(crate) use_most_probable_precision: UseMostProbablePrecisionCdfRows,
    pub(crate) pb_mv_precision: PbMvPrecisionCdfRows,
    pub(crate) use_bawp: UseBawpCdfRow,
    pub(crate) use_bawp_chroma: UseBawpCdfRow,
    pub(crate) explicit_bawp: ExplicitBawpCdfRows,
    pub(crate) explicit_bawp_scale: ExplicitBawpScaleCdfRow,
    pub(crate) use_wiener_ns: UseWienerNsCdfRow,
    pub(crate) use_pc_wiener: UsePcWienerCdfRow,
    pub(crate) wiener_ns_length: WienerNsLengthCdfRows,
    pub(crate) wiener_ns_uv_sym: WienerNsUvSymCdfRow,
    pub(crate) wiener_ns_base: WienerNsBaseCdfRow,
    pub(crate) is_long_side_dct: IsLongSideDctCdfRows,
    pub(crate) intra_tx_type_long: IntraTxTypeLongCdfRows,
    pub(crate) inter_tx_type_long: InterTxTypeLongCdfRows,
    pub(crate) inter_tx_type_set1: InterTxTypeSet1CdfRows,
    pub(crate) inter_tx_type_set2: InterTxTypeSet2CdfRows,
    pub(crate) inter_tx_type_index_set1: InterTxTypeIndexSet1CdfRows,
    pub(crate) inter_tx_type_index_set2: InterTxTypeIndexSet2CdfRows,
    pub(crate) inter_tx_type_offset_set1: InterTxTypeOffsetSet1CdfRows,
    pub(crate) inter_tx_type_offset_set2: InterTxTypeOffsetSet2CdfRows,
    pub(crate) inter_tx_type_set3: InterTxTypeSet3CdfRows,
    pub(crate) inter_tx_type_set4: InterTxTypeSet4CdfRows,
    pub(crate) intra_tx_type_set1: IntraTxTypeSet1CdfRows,
    pub(crate) intra_tx_type_set2: IntraTxTypeSet2CdfRows,
    pub(crate) sec_tx_type: SecTxTypeCdfRows,
    pub(crate) most_probable_stx_set: MostProbableStxSetCdfRow,
    pub(crate) most_probable_stx_set_adst: MostProbableStxSetAdstCdfRow,
    pub(crate) cctx_type: CctxTypeCdfRow,
    pub(crate) palette_y_mode: PaletteYModeCdfRow,
    pub(crate) palette_y_size: PaletteYSizeCdfRow,
    pub(crate) identity_row_y: IdentityRowYCdfRows,
    pub(crate) palette_size_2_y_color: PaletteSize2YColorCdfRows,
    pub(crate) palette_size_3_y_color: PaletteSize3YColorCdfRows,
    pub(crate) palette_size_4_y_color: PaletteSize4YColorCdfRows,
    pub(crate) palette_size_5_y_color: PaletteSize5YColorCdfRows,
    pub(crate) palette_size_6_y_color: PaletteSize6YColorCdfRows,
    pub(crate) palette_size_7_y_color: PaletteSize7YColorCdfRows,
    pub(crate) palette_size_8_y_color: PaletteSize8YColorCdfRows,
    pub(crate) coeff: CoeffCdfRows,
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
            TileCdfSelector::UseDpcmY => Ok($self.use_dpcm_y.$as_slice()),
            TileCdfSelector::DpcmModeY => Ok($self.dpcm_mode_y.$as_slice()),
            TileCdfSelector::UseDpcmUv => Ok($self.use_dpcm_uv.$as_slice()),
            TileCdfSelector::DpcmModeUv => Ok($self.dpcm_mode_uv.$as_slice()),
            TileCdfSelector::YModeSet => Ok($self.y_mode_set.$as_slice()),
            TileCdfSelector::YModeIndex { ctx } => block_row_slice!(
                $self.y_mode_index,
                ctx,
                "ctx",
                TileCdfArray::YModeIndex,
                $get,
                $as_slice
            ),
            TileCdfSelector::YModeOffset { ctx } => block_row_slice!(
                $self.y_mode_offset,
                ctx,
                "ctx",
                TileCdfArray::YModeOffset,
                $get,
                $as_slice
            ),
            TileCdfSelector::TxbSkip {
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
            TileCdfSelector::UvModeCflNotAllowed { ctx } => block_row_slice!(
                $self.uv_mode_cfl_not_allowed,
                ctx,
                "ctx",
                TileCdfArray::UvModeCflNotAllowed,
                $get,
                $as_slice
            ),
            TileCdfSelector::IsCfl { ctx } => block_row_slice!(
                $self.is_cfl,
                ctx,
                "ctx",
                TileCdfArray::IsCfl,
                $get,
                $as_slice
            ),
            TileCdfSelector::CflIndex => Ok($self.cfl_index.$as_slice()),
            TileCdfSelector::CflSign => Ok($self.cfl_sign.$as_slice()),
            TileCdfSelector::CflAlpha { ctx } => block_row_slice!(
                $self.cfl_alpha,
                ctx,
                "ctx",
                TileCdfArray::CflAlpha,
                $get,
                $as_slice
            ),
            TileCdfSelector::CflMhccp => Ok($self.cfl_mhccp.$as_slice()),
            TileCdfSelector::CflMhDir { size_group } => block_row_slice!(
                $self.cfl_mh_dir,
                size_group,
                "size_group",
                TileCdfArray::CflMhDir,
                $get,
                $as_slice
            ),
            TileCdfSelector::UseDip { ctx } => block_row_slice!(
                $self.use_dip,
                ctx,
                "ctx",
                TileCdfArray::UseDip,
                $get,
                $as_slice
            ),
            TileCdfSelector::DipMode => Ok($self.dip_mode.$as_slice()),
            TileCdfSelector::VTxbSkip {
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
            TileCdfSelector::EobExtra { coeff_cdf_q_ctx } => {
                let coeff_cdf_q_ctx =
                    checked_coeff_cdf_q_context(TileCdfArray::EobExtra, coeff_cdf_q_ctx)?;
                Ok($self.eob_extra[coeff_cdf_q_ctx].$as_slice())
            }
            TileCdfSelector::EobPt {
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
            TileCdfSelector::DcSign {
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
            TileCdfSelector::IsInter { ctx } => block_row_slice!(
                $self.is_inter,
                ctx,
                "ctx",
                TileCdfArray::IsInter,
                $get,
                $as_slice
            ),
            TileCdfSelector::SkipMode { ctx } => block_row_slice!(
                $self.skip_mode,
                ctx,
                "ctx",
                TileCdfArray::SkipMode,
                $get,
                $as_slice
            ),
            TileCdfSelector::Skip { ctx } => {
                block_row_slice!($self.skip, ctx, "ctx", TileCdfArray::Skip, $get, $as_slice)
            }
            TileCdfSelector::SingleMode { ctx } => block_row_slice!(
                $self.single_mode,
                ctx,
                "ctx",
                TileCdfArray::SingleMode,
                $get,
                $as_slice
            ),
            TileCdfSelector::IsWarp { ctx } => block_row_slice!(
                $self.is_warp,
                ctx,
                "ctx",
                TileCdfArray::IsWarp,
                $get,
                $as_slice
            ),
            TileCdfSelector::WarpMv => Ok($self.warp_mv.$as_slice()),
            TileCdfSelector::WarpIdx { ctx } => block_row_slice!(
                $self.warp_idx,
                ctx,
                "ctx",
                TileCdfArray::WarpIdx,
                $get,
                $as_slice
            ),
            TileCdfSelector::WarpWithMvd => Ok($self.warp_with_mvd.$as_slice()),
            TileCdfSelector::WarpPrecision { block_size } => block_row_slice!(
                $self.warp_precision,
                block_size,
                "block_size",
                TileCdfArray::WarpPrecision,
                $get,
                $as_slice
            ),
            TileCdfSelector::WarpDeltaParamLow { index_type } => block_row_slice!(
                $self.warp_delta_param_low,
                index_type,
                "index_type",
                TileCdfArray::WarpDeltaParamLow,
                $get,
                $as_slice
            ),
            TileCdfSelector::WarpDeltaParamHigh { index_type } => block_row_slice!(
                $self.warp_delta_param_high,
                index_type,
                "index_type",
                TileCdfArray::WarpDeltaParamHigh,
                $get,
                $as_slice
            ),
            TileCdfSelector::WarpDeltaParamSign => Ok($self.warp_delta_param_sign.$as_slice()),
            TileCdfSelector::WarpInterIntra { bsize_group } => block_row_slice!(
                $self.warp_inter_intra,
                bsize_group,
                "bsize_group",
                TileCdfArray::WarpInterIntra,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterIntra { bsize_group } => block_row_slice!(
                $self.inter_intra,
                bsize_group,
                "bsize_group",
                TileCdfArray::InterIntra,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterIntraMode { bsize_group } => block_row_slice!(
                $self.inter_intra_mode,
                bsize_group,
                "bsize_group",
                TileCdfArray::InterIntraMode,
                $get,
                $as_slice
            ),
            TileCdfSelector::WedgeInterIntra => Ok($self.wedge_inter_intra.$as_slice()),
            TileCdfSelector::WedgeQuad => Ok($self.wedge_quad.$as_slice()),
            TileCdfSelector::WedgeAngle { quad } => block_row_slice!(
                $self.wedge_angle,
                quad,
                "quad",
                TileCdfArray::WedgeAngle,
                $get,
                $as_slice
            ),
            TileCdfSelector::WedgeDist1 => Ok($self.wedge_dist1.$as_slice()),
            TileCdfSelector::WedgeDist2 => Ok($self.wedge_dist2.$as_slice()),
            TileCdfSelector::DrlMode { idx, ctx } => {
                let bank =
                    checked_block_row!($self.drl_mode, idx, "idx", TileCdfArray::DrlMode, $get)?;
                block_row_slice!(bank, ctx, "ctx", TileCdfArray::DrlMode, $get, $as_slice)
            }
            TileCdfSelector::SkipDrlMode { idx } => block_row_slice!(
                $self.skip_drl_mode,
                idx,
                "idx",
                TileCdfArray::SkipDrlMode,
                $get,
                $as_slice
            ),
            TileCdfSelector::TipMode { ctx } => block_row_slice!(
                $self.tip_mode,
                ctx,
                "ctx",
                TileCdfArray::TipMode,
                $get,
                $as_slice
            ),
            TileCdfSelector::TipPredMode => Ok($self.tip_pred_mode.$as_slice()),
            TileCdfSelector::TipDrlMode { idx } => block_row_slice!(
                $self.tip_drl_mode,
                idx,
                "idx",
                TileCdfArray::TipDrlMode,
                $get,
                $as_slice
            ),
            TileCdfSelector::SingleRef { ctx, ref_idx } => {
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
            TileCdfSelector::CompMode { ctx } => block_row_slice!(
                $self.comp_mode,
                ctx,
                "ctx",
                TileCdfArray::CompMode,
                $get,
                $as_slice
            ),
            TileCdfSelector::IsJoint { ctx } => block_row_slice!(
                $self.is_joint,
                ctx,
                "ctx",
                TileCdfArray::IsJoint,
                $get,
                $as_slice
            ),
            TileCdfSelector::JmvdScaleMode => Ok($self.jmvd_scale_mode.$as_slice()),
            TileCdfSelector::JmvdAdaptiveScaleMode => {
                Ok($self.jmvd_adaptive_scale_mode.$as_slice())
            }
            TileCdfSelector::CompoundModeNonJoint { ctx } => block_row_slice!(
                $self.compound_mode_non_joint,
                ctx,
                "ctx",
                TileCdfArray::CompoundModeNonJoint,
                $get,
                $as_slice
            ),
            TileCdfSelector::CompoundModeSameRefs { ctx } => block_row_slice!(
                $self.compound_mode_same_refs,
                ctx,
                "ctx",
                TileCdfArray::CompoundModeSameRefs,
                $get,
                $as_slice
            ),
            TileCdfSelector::CompoundType => Ok($self.compound_type.$as_slice()),
            TileCdfSelector::CompGroupIdx { ctx } => block_row_slice!(
                $self.comp_group_idx,
                ctx,
                "ctx",
                TileCdfArray::CompGroupIdx,
                $get,
                $as_slice
            ),
            TileCdfSelector::CwpIdx { idx } => block_row_slice!(
                $self.cwp_idx,
                idx,
                "idx",
                TileCdfArray::CwpIdx,
                $get,
                $as_slice
            ),
            TileCdfSelector::CompRef0 { ctx, ref_idx } => {
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
            TileCdfSelector::CompRef1 {
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
            TileCdfSelector::ReadMv(selector) => $self.read_mv.$delegate(selector),
            TileCdfSelector::InterpFilter { ctx } => block_row_slice!(
                $self.interp_filter,
                ctx,
                "ctx",
                TileCdfArray::InterpFilter,
                $get,
                $as_slice
            ),
            TileCdfSelector::UseBawp => Ok($self.use_bawp.$as_slice()),
            TileCdfSelector::UseBawpChroma => Ok($self.use_bawp_chroma.$as_slice()),
            TileCdfSelector::UseAmvd { index, ctx } => {
                let bank = checked_block_row!(
                    $self.use_amvd,
                    index,
                    "index",
                    TileCdfArray::UseAmvd,
                    $get
                )?;
                block_row_slice!(bank, ctx, "ctx", TileCdfArray::UseAmvd, $get, $as_slice)
            }
            TileCdfSelector::UseOptflow { ctx } => block_row_slice!(
                $self.use_optflow,
                ctx,
                "ctx",
                TileCdfArray::UseOptflow,
                $get,
                $as_slice
            ),
            TileCdfSelector::UseExtendWarp { ctx } => block_row_slice!(
                $self.use_extend_warp,
                ctx,
                "ctx",
                TileCdfArray::UseExtendWarp,
                $get,
                $as_slice
            ),
            TileCdfSelector::UseLocalWarp { ctx } => block_row_slice!(
                $self.use_local_warp,
                ctx,
                "ctx",
                TileCdfArray::UseLocalWarp,
                $get,
                $as_slice
            ),
            TileCdfSelector::UseMostProbablePrecision { ctx } => block_row_slice!(
                $self.use_most_probable_precision,
                ctx,
                "ctx",
                TileCdfArray::UseMostProbablePrecision,
                $get,
                $as_slice
            ),
            TileCdfSelector::PbMvPrecision { ctx, frame_ctx } => {
                let bank = checked_block_row!(
                    $self.pb_mv_precision,
                    ctx,
                    "ctx",
                    TileCdfArray::PbMvPrecision,
                    $get
                )?;
                block_row_slice!(
                    bank,
                    frame_ctx,
                    "frame_ctx",
                    TileCdfArray::PbMvPrecision,
                    $get,
                    $as_slice
                )
            }
            TileCdfSelector::ExplicitBawp { ctx } => block_row_slice!(
                $self.explicit_bawp,
                ctx,
                "ctx",
                TileCdfArray::ExplicitBawp,
                $get,
                $as_slice
            ),
            TileCdfSelector::ExplicitBawpScale => Ok($self.explicit_bawp_scale.$as_slice()),
            TileCdfSelector::UseWienerNs => Ok($self.use_wiener_ns.$as_slice()),
            TileCdfSelector::UsePcWiener => Ok($self.use_pc_wiener.$as_slice()),
            TileCdfSelector::WienerNsLength { plane_ctx } => block_row_slice!(
                $self.wiener_ns_length,
                plane_ctx,
                "plane_ctx",
                TileCdfArray::WienerNsLength,
                $get,
                $as_slice
            ),
            TileCdfSelector::WienerNsUvSym => Ok($self.wiener_ns_uv_sym.$as_slice()),
            TileCdfSelector::WienerNsBase => Ok($self.wiener_ns_base.$as_slice()),
            TileCdfSelector::IsLongSideDct { is_inter } => block_row_slice!(
                $self.is_long_side_dct,
                is_inter,
                "is_inter",
                TileCdfArray::IsLongSideDct,
                $get,
                $as_slice
            ),
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr } => block_row_slice!(
                $self.intra_tx_type_long,
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeLong,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterTxTypeLong { ctx, tx_size_sqr } => {
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
            TileCdfSelector::InterTxTypeSet1 { ctx, tx_size_sqr } => {
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
            TileCdfSelector::InterTxTypeSet2 { ctx } => block_row_slice!(
                $self.inter_tx_type_set2,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeSet2,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterTxTypeIndexSet1 { ctx } => block_row_slice!(
                $self.inter_tx_type_index_set1,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeIndexSet1,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterTxTypeIndexSet2 { ctx } => block_row_slice!(
                $self.inter_tx_type_index_set2,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeIndexSet2,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterTxTypeOffsetSet1 { ctx } => block_row_slice!(
                $self.inter_tx_type_offset_set1,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeOffsetSet1,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterTxTypeOffsetSet2 { ctx } => block_row_slice!(
                $self.inter_tx_type_offset_set2,
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeOffsetSet2,
                $get,
                $as_slice
            ),
            TileCdfSelector::InterTxTypeSet3 { ctx, tx_size_sqr } => {
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
            TileCdfSelector::InterTxTypeSet4 { ctx, tx_size_sqr } => {
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
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr } => block_row_slice!(
                $self.intra_tx_type_set1,
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeSet1,
                $get,
                $as_slice
            ),
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr } => block_row_slice!(
                $self.intra_tx_type_set2,
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeSet2,
                $get,
                $as_slice
            ),
            TileCdfSelector::SecTxType {
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
            TileCdfSelector::MostProbableStxSet => Ok($self.most_probable_stx_set.$as_slice()),
            TileCdfSelector::MostProbableStxSetAdst => {
                Ok($self.most_probable_stx_set_adst.$as_slice())
            }
            TileCdfSelector::CctxType => Ok($self.cctx_type.$as_slice()),
            TileCdfSelector::PaletteYMode => Ok($self.palette_y_mode.$as_slice()),
            TileCdfSelector::PaletteYSize => Ok($self.palette_y_size.$as_slice()),
            TileCdfSelector::IdentityRowY { ctx } => block_row_slice!(
                $self.identity_row_y,
                ctx,
                "ctx",
                TileCdfArray::IdentityRowY,
                $get,
                $as_slice
            ),
            TileCdfSelector::PaletteYColorIndex { palette_size, ctx } => {
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
            TileCdfSelector::Coeff(selector) => $self.coeff.$delegate(selector),
            _ => Err(TileCdfError::UnexpectedSelector),
        }
    };
}

macro_rules! block_cdf_count_rows {
    ($row:ident, $rows:ident, $read_mv:block, $coeff:block) => {{
        $row!(use_dpcm_y);
        $row!(dpcm_mode_y);
        $row!(use_dpcm_uv);
        $row!(dpcm_mode_uv);
        $row!(y_mode_set);
        $rows!(y_mode_index);
        $rows!(y_mode_offset);
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
        $rows!(use_dip);
        $row!(dip_mode);
        $rows!(eob_pt_16.flatten());
        $rows!(eob_pt_32.flatten());
        $rows!(eob_pt_64.flatten());
        $rows!(eob_pt_128.flatten());
        $rows!(eob_pt_256.flatten());
        $rows!(eob_pt_512.flatten());
        $rows!(eob_pt_1024.flatten());
        $rows!(dc_sign.flatten().flatten().flatten());
        $rows!(is_inter);
        $rows!(skip_mode);
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
        $rows!(inter_intra);
        $rows!(inter_intra_mode);
        $row!(wedge_inter_intra);
        $row!(wedge_quad);
        $rows!(wedge_angle);
        $row!(wedge_dist1);
        $row!(wedge_dist2);
        $rows!(drl_mode.flatten());
        $rows!(skip_drl_mode);
        $rows!(tip_mode);
        $row!(tip_pred_mode);
        $rows!(tip_drl_mode);
        $rows!(single_ref.flatten());
        $rows!(comp_mode);
        $rows!(is_joint);
        $row!(jmvd_scale_mode);
        $row!(jmvd_adaptive_scale_mode);
        $rows!(compound_mode_non_joint);
        $rows!(compound_mode_same_refs);
        $row!(compound_type);
        $rows!(comp_group_idx);
        $rows!(cwp_idx);
        $rows!(comp_ref0.flatten());
        $rows!(comp_ref1.flatten().flatten());
        $read_mv
        $rows!(interp_filter);
        $rows!(use_amvd.flatten());
        $rows!(use_optflow);
        $rows!(use_extend_warp);
        $rows!(use_local_warp);
        $rows!(use_most_probable_precision);
        $rows!(pb_mv_precision.flatten());
        $row!(use_bawp);
        $row!(use_bawp_chroma);
        $rows!(explicit_bawp);
        $row!(explicit_bawp_scale);
        $row!(use_wiener_ns);
        $row!(use_pc_wiener);
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
            use_dpcm_y: DEFAULT_USE_DPCM_Y_CDF,
            dpcm_mode_y: DEFAULT_DPCM_MODE_Y_CDF,
            use_dpcm_uv: DEFAULT_USE_DPCM_UV_CDF,
            dpcm_mode_uv: DEFAULT_DPCM_MODE_UV_CDF,
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
            use_dip: DEFAULT_USE_DIP_CDF,
            dip_mode: DEFAULT_DIP_MODE_CDF,
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
            skip_mode: DEFAULT_SKIP_MODE_CDF,
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
            inter_intra: DEFAULT_INTER_INTRA_CDF,
            inter_intra_mode: DEFAULT_INTER_INTRA_MODE_CDF,
            wedge_inter_intra: DEFAULT_WEDGE_INTER_INTRA_CDF,
            wedge_quad: DEFAULT_WEDGE_QUAD_CDF,
            wedge_angle: DEFAULT_WEDGE_ANGLE_CDF,
            wedge_dist1: DEFAULT_WEDGE_DIST1_CDF,
            wedge_dist2: DEFAULT_WEDGE_DIST2_CDF,
            drl_mode: DEFAULT_DRL_MODE_CDF,
            skip_drl_mode: DEFAULT_SKIP_DRL_MODE_CDF,
            tip_mode: DEFAULT_TIP_MODE_CDF,
            tip_pred_mode: DEFAULT_TIP_PRED_MODE_CDF,
            tip_drl_mode: DEFAULT_TIP_DRL_MODE_CDF,
            single_ref: DEFAULT_SINGLE_REF_CDF,
            comp_mode: DEFAULT_COMP_MODE_CDF,
            is_joint: DEFAULT_IS_JOINT_CDF,
            jmvd_scale_mode: DEFAULT_JMVD_SCALE_MODE_CDF,
            jmvd_adaptive_scale_mode: DEFAULT_JMVD_ADAPTIVE_SCALE_MODE_CDF,
            compound_mode_non_joint: DEFAULT_COMPOUND_MODE_NON_JOINT_CDF,
            compound_mode_same_refs: DEFAULT_COMPOUND_MODE_SAME_REFS_CDF,
            compound_type: DEFAULT_COMPOUND_TYPE_CDF,
            comp_group_idx: DEFAULT_COMP_GROUP_IDX_CDF,
            cwp_idx: DEFAULT_CWP_IDX_CDF,
            comp_ref0: DEFAULT_COMP_REF0_CDF,
            comp_ref1: DEFAULT_COMP_REF1_CDF,
            read_mv: MvCdfRows::from_defaults(),
            interp_filter: DEFAULT_INTERP_FILTER_CDF,
            use_amvd: DEFAULT_USE_AMVD_CDF,
            use_optflow: DEFAULT_USE_OPTFLOW_CDF,
            use_extend_warp: DEFAULT_USE_EXTEND_WARP_CDF,
            use_local_warp: DEFAULT_USE_LOCAL_WARP_CDF,
            use_most_probable_precision: DEFAULT_USE_MOST_PROBABLE_PRECISION_CDF,
            pb_mv_precision: DEFAULT_PB_MV_PRECISION_CDF,
            use_bawp: DEFAULT_USE_BAWP_CDF,
            use_bawp_chroma: DEFAULT_USE_BAWP_CHROMA_CDF,
            explicit_bawp: DEFAULT_EXPLICIT_BAWP_CDF,
            explicit_bawp_scale: DEFAULT_EXPLICIT_BAWP_SCALE_CDF,
            use_wiener_ns: DEFAULT_USE_WIENER_NS_CDF,
            use_pc_wiener: DEFAULT_USE_PC_WIENER_CDF,
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

    pub(crate) fn row(&self, selector: TileCdfSelector) -> Result<&[i32], TileCdfError> {
        block_cdf_row!(self, selector, get, as_slice, row)
    }

    pub(crate) fn row_mut(
        &mut self,
        selector: TileCdfSelector,
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

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        macro_rules! blend_row {
            ($field:ident) => {
                blend_cdf_row(&mut self.$field, &saved.$field);
            };
        }
        macro_rules! blend_rows {
            ($field:ident $(. $flatten:ident())*) => {
                blend_cdf_rows(
                    self.$field.iter_mut()$(.$flatten())*,
                    saved.$field.iter()$(.$flatten())*,
                );
            };
        }

        block_cdf_count_rows!(
            blend_row,
            blend_rows,
            {
                self.read_mv.blend_from_saved(&saved.read_mv);
            },
            {
                self.coeff.blend_from_saved(&saved.coeff);
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
