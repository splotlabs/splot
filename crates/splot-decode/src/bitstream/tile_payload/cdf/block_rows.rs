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
    DEFAULT_FLEX_RESTORATION_TYPE_CDF, DEFAULT_IDENTITY_ROW_Y_CDF, DEFAULT_INTER_INTRA_CDF,
    DEFAULT_INTER_INTRA_MODE_CDF, DEFAULT_INTER_TX_TYPE_INDEX_SET1_CDF,
    DEFAULT_INTER_TX_TYPE_INDEX_SET2_CDF, DEFAULT_INTER_TX_TYPE_LONG_CDF,
    DEFAULT_INTER_TX_TYPE_OFFSET_SET1_CDF, DEFAULT_INTER_TX_TYPE_OFFSET_SET2_CDF,
    DEFAULT_INTER_TX_TYPE_SET1_CDF, DEFAULT_INTER_TX_TYPE_SET2_CDF, DEFAULT_INTER_TX_TYPE_SET3_CDF,
    DEFAULT_INTER_TX_TYPE_SET4_CDF, DEFAULT_INTERP_FILTER_CDF, DEFAULT_INTRA_TX_TYPE_LONG_CDF,
    DEFAULT_INTRA_TX_TYPE_SET1_CDF, DEFAULT_INTRA_TX_TYPE_SET2_CDF, DEFAULT_IS_CFL_CDF,
    DEFAULT_IS_INTER_CDF, DEFAULT_IS_JOINT_CDF, DEFAULT_IS_LONG_SIDE_DCT_CDF, DEFAULT_IS_WARP_CDF,
    DEFAULT_JMVD_ADAPTIVE_SCALE_MODE_CDF, DEFAULT_JMVD_SCALE_MODE_CDF,
    DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF, DEFAULT_MOST_PROBABLE_STX_SET_CDF,
    DEFAULT_PALETTE_SIZE_2_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_3_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_4_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_5_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_6_Y_COLOR_CDF, DEFAULT_PALETTE_SIZE_7_Y_COLOR_CDF,
    DEFAULT_PALETTE_SIZE_8_Y_COLOR_CDF, DEFAULT_PALETTE_Y_MODE_CDF, DEFAULT_PALETTE_Y_SIZE_CDF,
    DEFAULT_PB_MV_PRECISION_CDF, DEFAULT_SEC_TX_TYPE_CDF, DEFAULT_SINGLE_MODE_CDF,
    DEFAULT_SINGLE_REF_CDF, DEFAULT_SKIP_CDF, DEFAULT_SKIP_DRL_MODE_CDF, DEFAULT_SKIP_MODE_CDF,
    DEFAULT_TIP_DRL_MODE_CDF, DEFAULT_TIP_MODE_CDF, DEFAULT_TIP_PRED_MODE_CDF,
    DEFAULT_TXB_SKIP_CDF, DEFAULT_USE_AMVD_CDF, DEFAULT_USE_BAWP_CDF, DEFAULT_USE_BAWP_CHROMA_CDF,
    DEFAULT_USE_DIP_CDF, DEFAULT_USE_DPCM_UV_CDF, DEFAULT_USE_DPCM_Y_CDF,
    DEFAULT_USE_EXTEND_WARP_CDF, DEFAULT_USE_LOCAL_WARP_CDF,
    DEFAULT_USE_MOST_PROBABLE_PRECISION_CDF, DEFAULT_USE_OPTFLOW_CDF, DEFAULT_USE_PC_WIENER_CDF,
    DEFAULT_USE_REFINEMV_CDF, DEFAULT_USE_WIENER_NS_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
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
pub use self::mv::MvCdfSelector;
#[cfg(any(debug_assertions, fuzzing, test))]
use super::cdf_store_valid;
use super::coeff_rows::CoeffCdfRows;
use super::{
    CDF_ROW_LEN, CoeffCdfQContext, TileCdfArray, TileCdfError, TileCdfSelector, avg_cdf_store,
    blend_cdf_store, checked_context, scale_cdf_store,
};

const Y_MODE_SET_CDF_ROW_LEN: usize = 5;
const Y_MODE_INDEX_CONTEXTS: usize = 3;
const INTRA_MODE_CDF_ROW_LEN: usize = 9;
const Y_MODE_OFFSET_CDF_ROW_LEN: usize = 7;
const Y_MODE_OFFSET_CONTEXTS: usize = 3;
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
pub(crate) const COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN: usize = 6;
pub(crate) const COMPOUND_MODE_SAME_REFS_CDF_ROW_LEN: usize = 5;
const JMVD_SCALE_MODE_CDF_ROW_LEN: usize = 6;
const JMVD_ADAPTIVE_SCALE_MODE_CDF_ROW_LEN: usize = 4;
const COMP_GROUP_IDX_CONTEXTS: usize = 12;
const CWP_IDX_CONTEXTS: usize = 4;
const COMP_REF1_BIT_TYPES: usize = 2;
const INTERP_FILTER_CONTEXTS: usize = 16;
const AMVD_MODE_CONTEXTS: usize = 9;
const AMVD_CONTEXTS: usize = 3;
const USE_OPTFLOW_CONTEXTS: usize = 2;
const USE_REFINEMV_CONTEXTS: usize = 24;
const MOST_PROBABLE_PRECISION_CONTEXTS: usize = 3;
const USE_EXTEND_WARP_CONTEXTS: usize = 3;
const USE_LOCAL_WARP_CONTEXTS: usize = 4;
const PB_MV_PRECISION_CONTEXTS: usize = 2;
const PB_MV_PRECISION_FRAME_CONTEXTS: usize = 3;
pub(crate) const PB_MV_PRECISION_CDF_ROW_LEN: usize = 4;
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

cdf_rows! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct BlockCdfRows[BlockCdfElement](q: CoeffCdfQContext) {
        use_dpcm_y: CDF_ROW_LEN = DEFAULT_USE_DPCM_Y_CDF,
        dpcm_mode_y: CDF_ROW_LEN = DEFAULT_DPCM_MODE_Y_CDF,
        use_dpcm_uv: CDF_ROW_LEN = DEFAULT_USE_DPCM_UV_CDF,
        dpcm_mode_uv: CDF_ROW_LEN = DEFAULT_DPCM_MODE_UV_CDF,
        y_mode_set: Y_MODE_SET_CDF_ROW_LEN = DEFAULT_Y_MODE_SET_CDF,
        y_mode_index[Y_MODE_INDEX_CONTEXTS]: INTRA_MODE_CDF_ROW_LEN = DEFAULT_Y_MODE_INDEX_CDF,
        y_mode_offset[Y_MODE_OFFSET_CONTEXTS]: Y_MODE_OFFSET_CDF_ROW_LEN = DEFAULT_Y_MODE_OFFSET_CDF,
        txb_skip[PLANE_TYPES][TX_SIZE_CONTEXTS][TXB_SKIP_CONTEXTS]: CDF_ROW_LEN = DEFAULT_TXB_SKIP_CDF[q.index()],
        uv_mode_cfl_not_allowed[UV_MODE_CONTEXTS]: INTRA_MODE_CDF_ROW_LEN = DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
        is_cfl[CFL_CONTEXTS]: CDF_ROW_LEN = DEFAULT_IS_CFL_CDF,
        cfl_index: CDF_ROW_LEN = DEFAULT_CFL_INDEX_CDF,
        cfl_sign: CFL_SIGN_CDF_ROW_LEN = DEFAULT_CFL_SIGN_CDF,
        cfl_alpha[CFL_ALPHA_CONTEXTS]: CFL_ALPHA_CDF_ROW_LEN = DEFAULT_CFL_ALPHA_CDF,
        cfl_mhccp: CDF_ROW_LEN = DEFAULT_CFL_MHCCP_CDF,
        cfl_mh_dir[CFL_MH_DIR_GROUPS]: CFL_MH_DIR_CDF_ROW_LEN = DEFAULT_CFL_MH_DIR_CDF,
        use_dip[DIP_CONTEXTS]: CDF_ROW_LEN = DEFAULT_USE_DIP_CDF,
        dip_mode: DIP_MODE_ROW_LEN = DEFAULT_DIP_MODE_CDF,
        v_txb_skip[V_TXB_SKIP_CONTEXTS]: CDF_ROW_LEN = DEFAULT_V_TXB_SKIP_CDF[q.index()],
        eob_extra: CDF_ROW_LEN = DEFAULT_EOB_EXTRA_CDF[q.index()],
        eob_pt_16[EOB_PLANE_CTXS]: 6 = DEFAULT_EOB_PT_16_CDF[q.index()],
        eob_pt_32[EOB_PLANE_CTXS]: 7 = DEFAULT_EOB_PT_32_CDF[q.index()],
        eob_pt_64[EOB_PLANE_CTXS]: 8 = DEFAULT_EOB_PT_64_CDF[q.index()],
        eob_pt_128[EOB_PLANE_CTXS]: 9 = DEFAULT_EOB_PT_128_CDF[q.index()],
        eob_pt_256[EOB_PLANE_CTXS]: 9 = DEFAULT_EOB_PT_256_CDF[q.index()],
        eob_pt_512[EOB_PLANE_CTXS]: 9 = DEFAULT_EOB_PT_512_CDF[q.index()],
        eob_pt_1024[EOB_PLANE_CTXS]: 9 = DEFAULT_EOB_PT_1024_CDF[q.index()],
        dc_sign[PLANE_TYPES][DC_SIGN_GROUPS][DC_SIGN_CONTEXTS]: CDF_ROW_LEN = DEFAULT_DC_SIGN_CDF[q.index()],
        is_inter[IS_INTER_CONTEXTS]: CDF_ROW_LEN = DEFAULT_IS_INTER_CDF,
        skip_mode[SKIP_MODE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_SKIP_MODE_CDF,
        skip[SKIP_CONTEXTS]: CDF_ROW_LEN = DEFAULT_SKIP_CDF,
        single_mode[SINGLE_MODE_CONTEXTS]: 4 = DEFAULT_SINGLE_MODE_CDF,
        is_warp[WARP_MODE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_IS_WARP_CDF,
        warp_mv: CDF_ROW_LEN = DEFAULT_WARP_MV_CDF,
        warp_idx[WARP_IDX_CONTEXTS]: CDF_ROW_LEN = DEFAULT_WARP_IDX_CDF,
        warp_with_mvd: CDF_ROW_LEN = DEFAULT_WARP_WITH_MVD_CDF,
        warp_precision[BLOCK_SIZE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_WARP_PRECISION_CDF,
        warp_delta_param_low[WARP_DELTA_PARAM_CONTEXTS]: WARP_DELTA_PARAM_CDF_ROW_LEN = DEFAULT_WARP_DELTA_PARAM_LOW_CDF,
        warp_delta_param_high[WARP_DELTA_PARAM_CONTEXTS]: WARP_DELTA_PARAM_CDF_ROW_LEN = DEFAULT_WARP_DELTA_PARAM_HIGH_CDF,
        warp_delta_param_sign: CDF_ROW_LEN = DEFAULT_WARP_DELTA_PARAM_SIGN_CDF,
        warp_inter_intra[BLOCK_SIZE_GROUPS]: CDF_ROW_LEN = DEFAULT_WARP_INTER_INTRA_CDF,
        inter_intra[BLOCK_SIZE_GROUPS]: CDF_ROW_LEN = DEFAULT_INTER_INTRA_CDF,
        inter_intra_mode[BLOCK_SIZE_GROUPS]: INTERINTRA_MODE_ROW_LEN = DEFAULT_INTER_INTRA_MODE_CDF,
        wedge_inter_intra: CDF_ROW_LEN = DEFAULT_WEDGE_INTER_INTRA_CDF,
        wedge_quad: WEDGE_QUAD_ROW_LEN = DEFAULT_WEDGE_QUAD_CDF,
        wedge_angle[WEDGE_ANGLE_CONTEXTS]: WEDGE_ANGLE_ROW_LEN = DEFAULT_WEDGE_ANGLE_CDF,
        wedge_dist1: WEDGE_DIST1_ROW_LEN = DEFAULT_WEDGE_DIST1_CDF,
        wedge_dist2: WEDGE_DIST2_ROW_LEN = DEFAULT_WEDGE_DIST2_CDF,
        drl_mode[DRL_MODE_IDX_BANKS][DRL_MODE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_DRL_MODE_CDF,
        skip_drl_mode[DRL_MODE_IDX_BANKS]: CDF_ROW_LEN = DEFAULT_SKIP_DRL_MODE_CDF,
        tip_mode[TIP_CONTEXTS]: CDF_ROW_LEN = DEFAULT_TIP_MODE_CDF,
        tip_pred_mode: CDF_ROW_LEN = DEFAULT_TIP_PRED_MODE_CDF,
        tip_drl_mode[DRL_MODE_IDX_BANKS]: CDF_ROW_LEN = DEFAULT_TIP_DRL_MODE_CDF,
        single_ref[REF_CONTEXTS][REFS_PER_FRAME_MINUS_1]: CDF_ROW_LEN = DEFAULT_SINGLE_REF_CDF,
        comp_mode[COMP_MODE_CONTEXTS]: CDF_ROW_LEN = DEFAULT_COMP_MODE_CDF,
        is_joint[IS_JOINT_CONTEXTS]: CDF_ROW_LEN = DEFAULT_IS_JOINT_CDF,
        jmvd_scale_mode: JMVD_SCALE_MODE_CDF_ROW_LEN = DEFAULT_JMVD_SCALE_MODE_CDF,
        jmvd_adaptive_scale_mode: JMVD_ADAPTIVE_SCALE_MODE_CDF_ROW_LEN = DEFAULT_JMVD_ADAPTIVE_SCALE_MODE_CDF,
        compound_mode_non_joint[COMPOUND_MODE_CONTEXTS]: COMPOUND_MODE_NON_JOINT_CDF_ROW_LEN = DEFAULT_COMPOUND_MODE_NON_JOINT_CDF,
        compound_mode_same_refs[COMPOUND_MODE_CONTEXTS]: COMPOUND_MODE_SAME_REFS_CDF_ROW_LEN = DEFAULT_COMPOUND_MODE_SAME_REFS_CDF,
        compound_type: CDF_ROW_LEN = DEFAULT_COMPOUND_TYPE_CDF,
        comp_group_idx[COMP_GROUP_IDX_CONTEXTS]: CDF_ROW_LEN = DEFAULT_COMP_GROUP_IDX_CDF,
        cwp_idx[CWP_IDX_CONTEXTS]: CDF_ROW_LEN = DEFAULT_CWP_IDX_CDF,
        comp_ref0[REF_CONTEXTS][REFS_PER_FRAME_MINUS_1]: CDF_ROW_LEN = DEFAULT_COMP_REF0_CDF,
        comp_ref1[REF_CONTEXTS][COMP_REF1_BIT_TYPES][REFS_PER_FRAME_MINUS_1]: CDF_ROW_LEN = DEFAULT_COMP_REF1_CDF,
        interp_filter[INTERP_FILTER_CONTEXTS]: 4 = DEFAULT_INTERP_FILTER_CDF,
        use_amvd[AMVD_MODE_CONTEXTS][AMVD_CONTEXTS]: CDF_ROW_LEN = DEFAULT_USE_AMVD_CDF,
        use_optflow[USE_OPTFLOW_CONTEXTS]: CDF_ROW_LEN = DEFAULT_USE_OPTFLOW_CDF,
        use_refinemv[USE_REFINEMV_CONTEXTS]: CDF_ROW_LEN = DEFAULT_USE_REFINEMV_CDF,
        use_extend_warp[USE_EXTEND_WARP_CONTEXTS]: CDF_ROW_LEN = DEFAULT_USE_EXTEND_WARP_CDF,
        use_local_warp[USE_LOCAL_WARP_CONTEXTS]: CDF_ROW_LEN = DEFAULT_USE_LOCAL_WARP_CDF,
        use_most_probable_precision[MOST_PROBABLE_PRECISION_CONTEXTS]: CDF_ROW_LEN = DEFAULT_USE_MOST_PROBABLE_PRECISION_CDF,
        pb_mv_precision[PB_MV_PRECISION_CONTEXTS][PB_MV_PRECISION_FRAME_CONTEXTS]: PB_MV_PRECISION_CDF_ROW_LEN = DEFAULT_PB_MV_PRECISION_CDF,
        use_bawp: CDF_ROW_LEN = DEFAULT_USE_BAWP_CDF,
        use_bawp_chroma: CDF_ROW_LEN = DEFAULT_USE_BAWP_CHROMA_CDF,
        explicit_bawp[BAWP_SCALES_CONTEXTS]: CDF_ROW_LEN = DEFAULT_EXPLICIT_BAWP_CDF,
        explicit_bawp_scale: CDF_ROW_LEN = DEFAULT_EXPLICIT_BAWP_SCALE_CDF,
        use_wiener_ns: CDF_ROW_LEN = DEFAULT_USE_WIENER_NS_CDF,
        use_pc_wiener: CDF_ROW_LEN = DEFAULT_USE_PC_WIENER_CDF,
        flex_restoration_type[3][3]: CDF_ROW_LEN = DEFAULT_FLEX_RESTORATION_TYPE_CDF,
        wiener_ns_length[WIENER_NS_LENGTH_CONTEXTS]: CDF_ROW_LEN = DEFAULT_WIENER_NS_LENGTH_CDF,
        wiener_ns_uv_sym: CDF_ROW_LEN = DEFAULT_WIENER_NS_UV_SYM_CDF,
        wiener_ns_base: WIENER_NS_BASE_CDF_ROW_LEN = DEFAULT_WIENER_NS_BASE_CDF,
        is_long_side_dct[IS_LONG_SIDE_DCT_CONTEXTS]: CDF_ROW_LEN = DEFAULT_IS_LONG_SIDE_DCT_CDF,
        intra_tx_type_long[INTRA_TX_TYPE_LONG_SIZE_CONTEXTS]: INTRA_TX_TYPE_LONG_ROW_LEN = DEFAULT_INTRA_TX_TYPE_LONG_CDF,
        inter_tx_type_long[INTER_TX_TYPE_LONG_EOB_CONTEXTS][INTER_TX_TYPE_LONG_SIZE_CONTEXTS]: INTER_TX_TYPE_LONG_ROW_LEN = DEFAULT_INTER_TX_TYPE_LONG_CDF,
        inter_tx_type_set1[INTER_TX_TYPE_EOB_CONTEXTS][INTER_TX_TYPE_SET1_SIZE_CONTEXTS]: INTER_TX_TYPE_SET1_ROW_LEN = DEFAULT_INTER_TX_TYPE_SET1_CDF,
        inter_tx_type_set2[INTER_TX_TYPE_EOB_CONTEXTS]: INTER_TX_TYPE_SET2_ROW_LEN = DEFAULT_INTER_TX_TYPE_SET2_CDF,
        inter_tx_type_index_set1[INTER_TX_TYPE_EOB_CONTEXTS]: INTER_TX_TYPE_INDEX_ROW_LEN = DEFAULT_INTER_TX_TYPE_INDEX_SET1_CDF,
        inter_tx_type_index_set2[INTER_TX_TYPE_EOB_CONTEXTS]: INTER_TX_TYPE_INDEX_ROW_LEN = DEFAULT_INTER_TX_TYPE_INDEX_SET2_CDF,
        inter_tx_type_offset_set1[INTER_TX_TYPE_EOB_CONTEXTS]: INTER_TX_TYPE_OFFSET_SET1_ROW_LEN = DEFAULT_INTER_TX_TYPE_OFFSET_SET1_CDF,
        inter_tx_type_offset_set2[INTER_TX_TYPE_EOB_CONTEXTS]: INTER_TX_TYPE_OFFSET_SET2_ROW_LEN = DEFAULT_INTER_TX_TYPE_OFFSET_SET2_CDF,
        inter_tx_type_set3[INTER_TX_TYPE_EOB_CONTEXTS][INTER_TX_TYPE_SET34_SIZE_CONTEXTS]: INTER_TX_TYPE_SET3_ROW_LEN = DEFAULT_INTER_TX_TYPE_SET3_CDF,
        inter_tx_type_set4[INTER_TX_TYPE_EOB_CONTEXTS][INTER_TX_TYPE_SET34_SIZE_CONTEXTS]: INTER_TX_TYPE_SET4_ROW_LEN = DEFAULT_INTER_TX_TYPE_SET4_CDF,
        intra_tx_type_set1[INTRA_TX_TYPE_SIZE_CONTEXTS]: INTRA_TX_TYPE_SET1_ROW_LEN = DEFAULT_INTRA_TX_TYPE_SET1_CDF,
        intra_tx_type_set2[INTRA_TX_TYPE_SIZE_CONTEXTS]: INTRA_TX_TYPE_SET2_ROW_LEN = DEFAULT_INTRA_TX_TYPE_SET2_CDF,
        sec_tx_type[SEC_TX_TYPE_IS_INTER_CONTEXTS][SEC_TX_TYPE_TX_SIZE_CONTEXTS]: SEC_TX_TYPE_ROW_LEN = DEFAULT_SEC_TX_TYPE_CDF,
        most_probable_stx_set: MOST_PROBABLE_STX_SET_ROW_LEN = DEFAULT_MOST_PROBABLE_STX_SET_CDF,
        most_probable_stx_set_adst: MOST_PROBABLE_STX_SET_ADST_ROW_LEN = DEFAULT_MOST_PROBABLE_STX_SET_ADST_CDF,
        cctx_type: CCTX_TYPE_CDF_ROW_LEN = DEFAULT_CCTX_TYPE_CDF,
        palette_y_mode: CDF_ROW_LEN = DEFAULT_PALETTE_Y_MODE_CDF,
        palette_y_size: 8 = DEFAULT_PALETTE_Y_SIZE_CDF,
        identity_row_y[PALETTE_ROW_FLAG_CONTEXTS]: 4 = DEFAULT_IDENTITY_ROW_Y_CDF,
        palette_size_2_y_color[PALETTE_COLOR_CONTEXTS]: 3 = DEFAULT_PALETTE_SIZE_2_Y_COLOR_CDF,
        palette_size_3_y_color[PALETTE_COLOR_CONTEXTS]: 4 = DEFAULT_PALETTE_SIZE_3_Y_COLOR_CDF,
        palette_size_4_y_color[PALETTE_COLOR_CONTEXTS]: 5 = DEFAULT_PALETTE_SIZE_4_Y_COLOR_CDF,
        palette_size_5_y_color[PALETTE_COLOR_CONTEXTS]: 6 = DEFAULT_PALETTE_SIZE_5_Y_COLOR_CDF,
        palette_size_6_y_color[PALETTE_COLOR_CONTEXTS]: 7 = DEFAULT_PALETTE_SIZE_6_Y_COLOR_CDF,
        palette_size_7_y_color[PALETTE_COLOR_CONTEXTS]: 8 = DEFAULT_PALETTE_SIZE_7_Y_COLOR_CDF,
        palette_size_8_y_color[PALETTE_COLOR_CONTEXTS]: 9 = DEFAULT_PALETTE_SIZE_8_Y_COLOR_CDF,
        ..
        pub(crate) read_mv: MvCdfRows = MvCdfRows::from_defaults(),
        pub(crate) coeff: CoeffCdfRows = CoeffCdfRows::from_defaults(q),
    }
}

macro_rules! checked_block_row {
    ($rows:expr, $index:expr, $index_name:literal, $array:expr) => {{
        let max_exclusive = $rows.len();
        $rows
            .get_mut($index)
            .ok_or(TileCdfError::SelectorOutOfRange {
                array: $array,
                index_name: $index_name,
                actual: $index,
                max_exclusive,
            })
    }};
}

macro_rules! block_row_slice {
    ($rows:expr, $index:expr, $index_name:literal, $array:expr) => {{
        let row = checked_block_row!($rows, $index, $index_name, $array)?;
        Ok(row.as_mut_slice())
    }};
}

impl BlockCdfRows {
    #[expect(
        clippy::inline_always,
        reason = "constant selectors fold into a direct row read"
    )]
    #[inline(always)]
    pub(crate) fn row_mut(
        &mut self,
        selector: TileCdfSelector,
    ) -> Result<&mut [u16], TileCdfError> {
        match selector {
            TileCdfSelector::UseDpcmY => Ok(self.use_dpcm_y().as_mut_slice()),
            TileCdfSelector::DpcmModeY => Ok(self.dpcm_mode_y().as_mut_slice()),
            TileCdfSelector::UseDpcmUv => Ok(self.use_dpcm_uv().as_mut_slice()),
            TileCdfSelector::DpcmModeUv => Ok(self.dpcm_mode_uv().as_mut_slice()),
            TileCdfSelector::YModeSet => Ok(self.y_mode_set().as_mut_slice()),
            TileCdfSelector::YModeIndex { ctx } => {
                block_row_slice!(self.y_mode_index(), ctx, "ctx", TileCdfArray::YModeIndex)
            }
            TileCdfSelector::YModeOffset { ctx } => {
                block_row_slice!(self.y_mode_offset(), ctx, "ctx", TileCdfArray::YModeOffset)
            }
            TileCdfSelector::TxbSkip {
                plane_type,
                tx_size,
                ctx,
            } => {
                let plane_type = checked_plane_type(TileCdfArray::TxbSkip, plane_type)?;
                let tx_size = checked_tx_size(TileCdfArray::TxbSkip, tx_size)?;
                block_row_slice!(
                    self.txb_skip()[plane_type][tx_size],
                    ctx,
                    "ctx",
                    TileCdfArray::TxbSkip
                )
            }
            TileCdfSelector::UvModeCflNotAllowed { ctx } => block_row_slice!(
                self.uv_mode_cfl_not_allowed(),
                ctx,
                "ctx",
                TileCdfArray::UvModeCflNotAllowed
            ),
            TileCdfSelector::IsCfl { ctx } => {
                block_row_slice!(self.is_cfl(), ctx, "ctx", TileCdfArray::IsCfl)
            }
            TileCdfSelector::CflIndex => Ok(self.cfl_index().as_mut_slice()),
            TileCdfSelector::CflSign => Ok(self.cfl_sign().as_mut_slice()),
            TileCdfSelector::CflAlpha { ctx } => {
                block_row_slice!(self.cfl_alpha(), ctx, "ctx", TileCdfArray::CflAlpha)
            }
            TileCdfSelector::CflMhccp => Ok(self.cfl_mhccp().as_mut_slice()),
            TileCdfSelector::CflMhDir { size_group } => block_row_slice!(
                self.cfl_mh_dir(),
                size_group,
                "size_group",
                TileCdfArray::CflMhDir
            ),
            TileCdfSelector::UseDip { ctx } => {
                block_row_slice!(self.use_dip(), ctx, "ctx", TileCdfArray::UseDip)
            }
            TileCdfSelector::DipMode => Ok(self.dip_mode().as_mut_slice()),
            TileCdfSelector::VTxbSkip { ctx } => {
                block_row_slice!(self.v_txb_skip(), ctx, "ctx", TileCdfArray::VTxbSkip)
            }
            TileCdfSelector::EobExtra => Ok(self.eob_extra().as_mut_slice()),
            TileCdfSelector::EobPt { size, eob_ctx } => {
                let c = checked_eob_plane_ctx(eob_ctx)?;
                Ok(match size {
                    EobPtSize::Pt16 => self.eob_pt_16()[c].as_mut_slice(),
                    EobPtSize::Pt32 => self.eob_pt_32()[c].as_mut_slice(),
                    EobPtSize::Pt64 => self.eob_pt_64()[c].as_mut_slice(),
                    EobPtSize::Pt128 => self.eob_pt_128()[c].as_mut_slice(),
                    EobPtSize::Pt256 => self.eob_pt_256()[c].as_mut_slice(),
                    EobPtSize::Pt512 => self.eob_pt_512()[c].as_mut_slice(),
                    EobPtSize::Pt1024 => self.eob_pt_1024()[c].as_mut_slice(),
                })
            }
            TileCdfSelector::DcSign {
                plane_type,
                group,
                ctx,
            } => {
                let plane_type = checked_plane_type(TileCdfArray::DcSign, plane_type)?;
                let group = checked_dc_sign_group(group)?;
                block_row_slice!(
                    self.dc_sign()[plane_type][group],
                    ctx,
                    "ctx",
                    TileCdfArray::DcSign
                )
            }
            TileCdfSelector::IsInter { ctx } => {
                block_row_slice!(self.is_inter(), ctx, "ctx", TileCdfArray::IsInter)
            }
            TileCdfSelector::SkipMode { ctx } => {
                block_row_slice!(self.skip_mode(), ctx, "ctx", TileCdfArray::SkipMode)
            }
            TileCdfSelector::Skip { ctx } => {
                block_row_slice!(self.skip(), ctx, "ctx", TileCdfArray::Skip)
            }
            TileCdfSelector::SingleMode { ctx } => {
                block_row_slice!(self.single_mode(), ctx, "ctx", TileCdfArray::SingleMode)
            }
            TileCdfSelector::IsWarp { ctx } => {
                block_row_slice!(self.is_warp(), ctx, "ctx", TileCdfArray::IsWarp)
            }
            TileCdfSelector::WarpMv => Ok(self.warp_mv().as_mut_slice()),
            TileCdfSelector::WarpIdx { ctx } => {
                block_row_slice!(self.warp_idx(), ctx, "ctx", TileCdfArray::WarpIdx)
            }
            TileCdfSelector::WarpWithMvd => Ok(self.warp_with_mvd().as_mut_slice()),
            TileCdfSelector::WarpPrecision { block_size } => block_row_slice!(
                self.warp_precision(),
                block_size,
                "block_size",
                TileCdfArray::WarpPrecision
            ),
            TileCdfSelector::WarpDeltaParamLow { index_type } => block_row_slice!(
                self.warp_delta_param_low(),
                index_type,
                "index_type",
                TileCdfArray::WarpDeltaParamLow
            ),
            TileCdfSelector::WarpDeltaParamHigh { index_type } => block_row_slice!(
                self.warp_delta_param_high(),
                index_type,
                "index_type",
                TileCdfArray::WarpDeltaParamHigh
            ),
            TileCdfSelector::WarpDeltaParamSign => Ok(self.warp_delta_param_sign().as_mut_slice()),
            TileCdfSelector::WarpInterIntra { bsize_group } => block_row_slice!(
                self.warp_inter_intra(),
                bsize_group,
                "bsize_group",
                TileCdfArray::WarpInterIntra
            ),
            TileCdfSelector::InterIntra { bsize_group } => block_row_slice!(
                self.inter_intra(),
                bsize_group,
                "bsize_group",
                TileCdfArray::InterIntra
            ),
            TileCdfSelector::InterIntraMode { bsize_group } => block_row_slice!(
                self.inter_intra_mode(),
                bsize_group,
                "bsize_group",
                TileCdfArray::InterIntraMode
            ),
            TileCdfSelector::WedgeInterIntra => Ok(self.wedge_inter_intra().as_mut_slice()),
            TileCdfSelector::WedgeQuad => Ok(self.wedge_quad().as_mut_slice()),
            TileCdfSelector::WedgeAngle { quad } => {
                block_row_slice!(self.wedge_angle(), quad, "quad", TileCdfArray::WedgeAngle)
            }
            TileCdfSelector::WedgeDist1 => Ok(self.wedge_dist1().as_mut_slice()),
            TileCdfSelector::WedgeDist2 => Ok(self.wedge_dist2().as_mut_slice()),
            TileCdfSelector::DrlMode { idx, ctx } => {
                let bank = checked_block_row!(self.drl_mode(), idx, "idx", TileCdfArray::DrlMode)?;
                block_row_slice!(bank, ctx, "ctx", TileCdfArray::DrlMode)
            }
            TileCdfSelector::SkipDrlMode { idx } => {
                block_row_slice!(self.skip_drl_mode(), idx, "idx", TileCdfArray::SkipDrlMode)
            }
            TileCdfSelector::TipMode { ctx } => {
                block_row_slice!(self.tip_mode(), ctx, "ctx", TileCdfArray::TipMode)
            }
            TileCdfSelector::TipPredMode => Ok(self.tip_pred_mode().as_mut_slice()),
            TileCdfSelector::TipDrlMode { idx } => {
                block_row_slice!(self.tip_drl_mode(), idx, "idx", TileCdfArray::TipDrlMode)
            }
            TileCdfSelector::SingleRef { ctx, ref_idx } => {
                let bank =
                    checked_block_row!(self.single_ref(), ctx, "ctx", TileCdfArray::SingleRef)?;
                block_row_slice!(bank, ref_idx, "ref", TileCdfArray::SingleRef)
            }
            TileCdfSelector::CompMode { ctx } => {
                block_row_slice!(self.comp_mode(), ctx, "ctx", TileCdfArray::CompMode)
            }
            TileCdfSelector::IsJoint { ctx } => {
                block_row_slice!(self.is_joint(), ctx, "ctx", TileCdfArray::IsJoint)
            }
            TileCdfSelector::JmvdScaleMode => Ok(self.jmvd_scale_mode().as_mut_slice()),
            TileCdfSelector::JmvdAdaptiveScaleMode => {
                Ok(self.jmvd_adaptive_scale_mode().as_mut_slice())
            }
            TileCdfSelector::CompoundModeNonJoint { ctx } => block_row_slice!(
                self.compound_mode_non_joint(),
                ctx,
                "ctx",
                TileCdfArray::CompoundModeNonJoint
            ),
            TileCdfSelector::CompoundModeSameRefs { ctx } => block_row_slice!(
                self.compound_mode_same_refs(),
                ctx,
                "ctx",
                TileCdfArray::CompoundModeSameRefs
            ),
            TileCdfSelector::CompoundType => Ok(self.compound_type().as_mut_slice()),
            TileCdfSelector::CompGroupIdx { ctx } => {
                block_row_slice!(
                    self.comp_group_idx(),
                    ctx,
                    "ctx",
                    TileCdfArray::CompGroupIdx
                )
            }
            TileCdfSelector::CwpIdx { idx } => {
                block_row_slice!(self.cwp_idx(), idx, "idx", TileCdfArray::CwpIdx)
            }
            TileCdfSelector::CompRef0 { ctx, ref_idx } => {
                let bank =
                    checked_block_row!(self.comp_ref0(), ctx, "ctx", TileCdfArray::CompRef0)?;
                block_row_slice!(bank, ref_idx, "ref", TileCdfArray::CompRef0)
            }
            TileCdfSelector::CompRef1 {
                ctx,
                bit_type,
                ref_idx,
            } => {
                let bank =
                    checked_block_row!(self.comp_ref1(), ctx, "ctx", TileCdfArray::CompRef1)?;
                let bit_bank =
                    checked_block_row!(bank, bit_type, "bit_type", TileCdfArray::CompRef1)?;
                block_row_slice!(bit_bank, ref_idx, "ref", TileCdfArray::CompRef1)
            }
            TileCdfSelector::ReadMv(selector) => self.read_mv.row_mut(selector),
            TileCdfSelector::InterpFilter { ctx } => {
                block_row_slice!(self.interp_filter(), ctx, "ctx", TileCdfArray::InterpFilter)
            }
            TileCdfSelector::UseBawp => Ok(self.use_bawp().as_mut_slice()),
            TileCdfSelector::UseBawpChroma => Ok(self.use_bawp_chroma().as_mut_slice()),
            TileCdfSelector::UseAmvd { index, ctx } => {
                let bank =
                    checked_block_row!(self.use_amvd(), index, "index", TileCdfArray::UseAmvd)?;
                block_row_slice!(bank, ctx, "ctx", TileCdfArray::UseAmvd)
            }
            TileCdfSelector::UseOptflow { ctx } => {
                block_row_slice!(self.use_optflow(), ctx, "ctx", TileCdfArray::UseOptflow)
            }
            TileCdfSelector::UseRefinemv { ctx } => {
                block_row_slice!(self.use_refinemv(), ctx, "ctx", TileCdfArray::UseRefinemv)
            }
            TileCdfSelector::UseExtendWarp { ctx } => block_row_slice!(
                self.use_extend_warp(),
                ctx,
                "ctx",
                TileCdfArray::UseExtendWarp
            ),
            TileCdfSelector::UseLocalWarp { ctx } => {
                block_row_slice!(
                    self.use_local_warp(),
                    ctx,
                    "ctx",
                    TileCdfArray::UseLocalWarp
                )
            }
            TileCdfSelector::UseMostProbablePrecision { ctx } => block_row_slice!(
                self.use_most_probable_precision(),
                ctx,
                "ctx",
                TileCdfArray::UseMostProbablePrecision
            ),
            TileCdfSelector::PbMvPrecision { ctx, frame_ctx } => {
                let bank = checked_block_row!(
                    self.pb_mv_precision(),
                    ctx,
                    "ctx",
                    TileCdfArray::PbMvPrecision
                )?;
                block_row_slice!(bank, frame_ctx, "frame_ctx", TileCdfArray::PbMvPrecision)
            }
            TileCdfSelector::ExplicitBawp { ctx } => {
                block_row_slice!(self.explicit_bawp(), ctx, "ctx", TileCdfArray::ExplicitBawp)
            }
            TileCdfSelector::ExplicitBawpScale => Ok(self.explicit_bawp_scale().as_mut_slice()),
            TileCdfSelector::UseWienerNs => Ok(self.use_wiener_ns().as_mut_slice()),
            TileCdfSelector::UsePcWiener => Ok(self.use_pc_wiener().as_mut_slice()),
            TileCdfSelector::FlexRestorationType { tool, plane } => {
                let bank = checked_block_row!(
                    self.flex_restoration_type(),
                    tool,
                    "tool",
                    TileCdfArray::FlexRestorationType
                )?;
                block_row_slice!(bank, plane, "plane", TileCdfArray::FlexRestorationType)
            }
            TileCdfSelector::WienerNsLength { plane_ctx } => block_row_slice!(
                self.wiener_ns_length(),
                plane_ctx,
                "plane_ctx",
                TileCdfArray::WienerNsLength
            ),
            TileCdfSelector::WienerNsUvSym => Ok(self.wiener_ns_uv_sym().as_mut_slice()),
            TileCdfSelector::WienerNsBase => Ok(self.wiener_ns_base().as_mut_slice()),
            TileCdfSelector::IsLongSideDct { is_inter } => block_row_slice!(
                self.is_long_side_dct(),
                is_inter,
                "is_inter",
                TileCdfArray::IsLongSideDct
            ),
            TileCdfSelector::IntraTxTypeLong { tx_size_sqr } => block_row_slice!(
                self.intra_tx_type_long(),
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeLong
            ),
            TileCdfSelector::InterTxTypeLong { ctx, tx_size_sqr } => {
                let eob_row = checked_block_row!(
                    self.inter_tx_type_long(),
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeLong
                )?;
                block_row_slice!(
                    eob_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeLong
                )
            }
            TileCdfSelector::InterTxTypeSet1 { ctx, tx_size_sqr } => {
                let ctx_row = checked_block_row!(
                    self.inter_tx_type_set1(),
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeSet1
                )?;
                block_row_slice!(
                    ctx_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeSet1
                )
            }
            TileCdfSelector::InterTxTypeSet2 { ctx } => block_row_slice!(
                self.inter_tx_type_set2(),
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeSet2
            ),
            TileCdfSelector::InterTxTypeIndexSet1 { ctx } => block_row_slice!(
                self.inter_tx_type_index_set1(),
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeIndexSet1
            ),
            TileCdfSelector::InterTxTypeIndexSet2 { ctx } => block_row_slice!(
                self.inter_tx_type_index_set2(),
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeIndexSet2
            ),
            TileCdfSelector::InterTxTypeOffsetSet1 { ctx } => block_row_slice!(
                self.inter_tx_type_offset_set1(),
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeOffsetSet1
            ),
            TileCdfSelector::InterTxTypeOffsetSet2 { ctx } => block_row_slice!(
                self.inter_tx_type_offset_set2(),
                ctx,
                "ctx",
                TileCdfArray::InterTxTypeOffsetSet2
            ),
            TileCdfSelector::InterTxTypeSet3 { ctx, tx_size_sqr } => {
                let ctx_row = checked_block_row!(
                    self.inter_tx_type_set3(),
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeSet3
                )?;
                block_row_slice!(
                    ctx_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeSet3
                )
            }
            TileCdfSelector::InterTxTypeSet4 { ctx, tx_size_sqr } => {
                let ctx_row = checked_block_row!(
                    self.inter_tx_type_set4(),
                    ctx,
                    "ctx",
                    TileCdfArray::InterTxTypeSet4
                )?;
                block_row_slice!(
                    ctx_row,
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::InterTxTypeSet4
                )
            }
            TileCdfSelector::IntraTxTypeSet1 { tx_size_sqr } => block_row_slice!(
                self.intra_tx_type_set1(),
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeSet1
            ),
            TileCdfSelector::IntraTxTypeSet2 { tx_size_sqr } => block_row_slice!(
                self.intra_tx_type_set2(),
                tx_size_sqr,
                "tx_size_sqr",
                TileCdfArray::IntraTxTypeSet2
            ),
            TileCdfSelector::SecTxType {
                is_inter,
                tx_size_sqr,
            } => {
                let is_inter = checked_sec_tx_is_inter(is_inter)?;
                block_row_slice!(
                    self.sec_tx_type()[is_inter],
                    tx_size_sqr,
                    "tx_size_sqr",
                    TileCdfArray::SecTxType
                )
            }
            TileCdfSelector::MostProbableStxSet => Ok(self.most_probable_stx_set().as_mut_slice()),
            TileCdfSelector::MostProbableStxSetAdst => {
                Ok(self.most_probable_stx_set_adst().as_mut_slice())
            }
            TileCdfSelector::CctxType => Ok(self.cctx_type().as_mut_slice()),
            TileCdfSelector::PaletteYMode => Ok(self.palette_y_mode().as_mut_slice()),
            TileCdfSelector::PaletteYSize => Ok(self.palette_y_size().as_mut_slice()),
            TileCdfSelector::IdentityRowY { ctx } => {
                block_row_slice!(
                    self.identity_row_y(),
                    ctx,
                    "ctx",
                    TileCdfArray::IdentityRowY
                )
            }
            TileCdfSelector::PaletteYColorIndex { palette_size, ctx } => {
                let ctx = checked_context(
                    TileCdfArray::PaletteYColorIndex,
                    "ctx",
                    ctx,
                    PALETTE_COLOR_CONTEXTS,
                )?;
                match palette_size {
                    2 => Ok(self.palette_size_2_y_color()[ctx].as_mut_slice()),
                    3 => Ok(self.palette_size_3_y_color()[ctx].as_mut_slice()),
                    4 => Ok(self.palette_size_4_y_color()[ctx].as_mut_slice()),
                    5 => Ok(self.palette_size_5_y_color()[ctx].as_mut_slice()),
                    6 => Ok(self.palette_size_6_y_color()[ctx].as_mut_slice()),
                    7 => Ok(self.palette_size_7_y_color()[ctx].as_mut_slice()),
                    8 => Ok(self.palette_size_8_y_color()[ctx].as_mut_slice()),
                    _ => Err(TileCdfError::SelectorOutOfRange {
                        array: TileCdfArray::PaletteYColorIndex,
                        index_name: "palette_size",
                        actual: palette_size,
                        max_exclusive: 9,
                    }),
                }
            }
            #[cfg(test)]
            TileCdfSelector::Coeff(selector) => self.coeff.row_mut(selector),
            _ => Err(TileCdfError::UnexpectedSelector),
        }
    }

    pub(crate) fn avg_from_tile(&mut self, tile_num: u32, tile: &Self, num_log2: u8) {
        avg_cdf_store(
            &mut self.store,
            &tile.store,
            &Self::BOUNDS,
            tile_num,
            num_log2,
        );
        self.read_mv
            .average_from_tile(&tile.read_mv, tile_num, num_log2);
        self.coeff.avg_from_tile(tile_num, &tile.coeff, num_log2);
    }

    pub(crate) fn blend_from_saved(&mut self, saved: &Self) {
        blend_cdf_store(&mut self.store, &saved.store, &Self::BOUNDS);
        self.read_mv.blend_from_saved(&saved.read_mv);
        self.coeff.blend_from_saved(&saved.coeff);
    }

    #[cfg(any(debug_assertions, fuzzing, test))]
    pub(crate) fn rows_valid(&self) -> bool {
        cdf_store_valid(&self.store, &Self::BOUNDS)
            && self.read_mv.rows_valid()
            && self.coeff.rows_valid()
    }

    pub(crate) fn scale_counts_for_frame_end_update(&mut self) {
        scale_cdf_store(&mut self.store, &Self::BOUNDS);
        self.read_mv.scale_counts();
        self.coeff.scale_counts_for_frame_end_update();
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

fn checked_plane_type(array: TileCdfArray, plane_type: usize) -> Result<usize, TileCdfError> {
    checked_context(array, "plane_type", plane_type, PLANE_TYPES)
}

fn checked_tx_size(array: TileCdfArray, tx_size: usize) -> Result<usize, TileCdfError> {
    checked_context(array, "tx_size", tx_size, TX_SIZE_CONTEXTS)
}
