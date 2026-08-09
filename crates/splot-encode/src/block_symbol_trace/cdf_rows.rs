// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mutable default-CDF rows used by the supported block trace.

use super::*;

pub(super) struct BlockSymbolTraceCdfRows {
    do_split_root: [u16; DO_SPLIT_CDF_ROW_LEN],
    y_mode_set: [u16; Y_MODE_SET_CDF_ROW_LEN],
    y_mode_index_tile_origin: [u16; INTRA_MODE_CDF_ROW_LEN],
    uv_mode_non_directional: [u16; INTRA_MODE_CDF_ROW_LEN],
    luma_txb_skip_64x64: [u16; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip_32x32: [u16; TXB_SKIP_CDF_ROW_LEN],
    v_txb_skip: [u16; V_TXB_SKIP_CDF_ROW_LEN],
    v_txb_skip_eobu: [u16; V_TXB_SKIP_CDF_ROW_LEN],
    eob_pt_1024: [u16; EOB_PT_1024_CDF_ROW_LEN],
    eob_pt_1024_chroma: [u16; EOB_PT_1024_CDF_ROW_LEN],
    eob_extra: [u16; EOB_EXTRA_CDF_ROW_LEN],
    coeff_base_lf_eob_dc: [u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    coeff_base_lf_eob_ac: [u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    coeff_base_lf_eob_uv: [u16; COEFF_BASE_LF_EOB_UV_CDF_ROW_LEN],
    coeff_base_lf_ctx1: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_ctx2: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_ctx9: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_ctx4: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_br_lf_dc: [u16; COEFF_BR_LF_CDF_ROW_LEN],
    dc_sign: [u16; DC_SIGN_CDF_ROW_LEN],
}

impl BlockSymbolTraceCdfRows {
    pub(super) fn from_defaults() -> Self {
        Self {
            do_split_root: DEFAULT_DO_SPLIT_CDF[ROOT_PARTITION_PLANE_START]
                [ROOT_64X64_DO_SPLIT_CTX],
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index_tile_origin: DEFAULT_Y_MODE_INDEX_CDF[0],
            uv_mode_non_directional: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF[0],
            luma_txb_skip_64x64: DEFAULT_TXB_SKIP_CDF[COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_64X64_CTX][TXB_SKIP_CTX_NEUTRAL],
            u_txb_skip_32x32: DEFAULT_TXB_SKIP_CDF[COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_32X32_CTX][CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
            v_txb_skip: DEFAULT_V_TXB_SKIP_CDF[COEFF_CDF_Q_CTX][V_TXB_SKIP_CTX_NEUTRAL],
            v_txb_skip_eobu: DEFAULT_V_TXB_SKIP_CDF[COEFF_CDF_Q_CTX][CHROMA_V_TXB_SKIP_CTX_EOBU],
            eob_pt_1024: DEFAULT_EOB_PT_1024_CDF[COEFF_CDF_Q_CTX][EOB_CTX_LUMA_INTRA],
            eob_pt_1024_chroma: DEFAULT_EOB_PT_1024_CDF[COEFF_CDF_Q_CTX][EOB_CTX_CHROMA],
            eob_extra: DEFAULT_EOB_EXTRA_CDF[COEFF_CDF_Q_CTX],
            coeff_base_lf_eob_dc: DEFAULT_COEFF_BASE_LF_EOB_CDF[COEFF_CDF_Q_CTX][TX_SIZE_64X64_CTX]
                [COEFF_BASE_LF_EOB_CTX_DC],
            coeff_base_lf_eob_ac: DEFAULT_COEFF_BASE_LF_EOB_CDF[COEFF_CDF_Q_CTX][TX_SIZE_64X64_CTX]
                [COEFF_BASE_LF_EOB_CTX_AC],
            coeff_base_lf_eob_uv: DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[COEFF_CDF_Q_CTX]
                [COEFF_BASE_LF_EOB_CTX_DC],
            coeff_base_lf_ctx1: DEFAULT_COEFF_BASE_LF_CDF[COEFF_CDF_Q_CTX][TX_SIZE_64X64_CTX]
                [COEFF_BASE_LF_CTX_EOB2_DC][COEFF_BASE_LF_TCQ_CTX],
            coeff_base_lf_ctx2: DEFAULT_COEFF_BASE_LF_CDF[COEFF_CDF_Q_CTX][TX_SIZE_64X64_CTX]
                [COEFF_BASE_LF_CTX_VISIBLE_AC_DC][COEFF_BASE_LF_TCQ_CTX],
            coeff_base_lf_ctx9: DEFAULT_COEFF_BASE_LF_CDF[COEFF_CDF_Q_CTX][TX_SIZE_64X64_CTX]
                [COEFF_BASE_LF_CTX_AC_BAND][COEFF_BASE_LF_TCQ_CTX],
            coeff_base_lf_ctx4: DEFAULT_COEFF_BASE_LF_CDF[COEFF_CDF_Q_CTX][TX_SIZE_64X64_CTX]
                [COEFF_BASE_LF_CTX_2D_DC][COEFF_BASE_LF_TCQ_CTX],
            coeff_br_lf_dc: DEFAULT_COEFF_BR_LF_CDF[COEFF_CDF_Q_CTX][COEFF_BR_LF_CTX_DC],
            dc_sign: DEFAULT_DC_SIGN_CDF[COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE]
                [DC_SIGN_CTX_NEUTRAL],
        }
    }

    pub(super) fn partition_row_mut(&mut self) -> &mut [u16] {
        self.do_split_root.as_mut_slice()
    }

    pub(super) fn mode_row_mut(&mut self, token: IntraModeToken) -> &mut [u16] {
        match token.selector() {
            IntraModeCdfRowSelector::YModeSet => self.y_mode_set.as_mut_slice(),
            IntraModeCdfRowSelector::YModeIndexTileOrigin => {
                self.y_mode_index_tile_origin.as_mut_slice()
            }
            IntraModeCdfRowSelector::UvModeNonDirectional => {
                self.uv_mode_non_directional.as_mut_slice()
            }
        }
    }

    pub(super) fn coefficient_row_mut(&mut self, token: CoefficientEntropyToken) -> &mut [u16] {
        match token.selector() {
            CoefficientCdfRowSelector::LumaTxbSkip64x64 => self.luma_txb_skip_64x64.as_mut_slice(),
            CoefficientCdfRowSelector::ChromaUTxbSkip32x32 => self.u_txb_skip_32x32.as_mut_slice(),
            CoefficientCdfRowSelector::ChromaVTxbSkipNeutral => self.v_txb_skip.as_mut_slice(),
            CoefficientCdfRowSelector::ChromaVTxbSkipAfterCodedU => {
                self.v_txb_skip_eobu.as_mut_slice()
            }
            CoefficientCdfRowSelector::LumaEobPt1024 => self.eob_pt_1024.as_mut_slice(),
            CoefficientCdfRowSelector::ChromaEobPt1024 => self.eob_pt_1024_chroma.as_mut_slice(),
            CoefficientCdfRowSelector::EobExtra => self.eob_extra.as_mut_slice(),
            CoefficientCdfRowSelector::LumaCoeffBaseLfEobDc => {
                self.coeff_base_lf_eob_dc.as_mut_slice()
            }
            CoefficientCdfRowSelector::LumaCoeffBaseLfEobAc => {
                self.coeff_base_lf_eob_ac.as_mut_slice()
            }
            CoefficientCdfRowSelector::ChromaCoeffBaseLfEob => {
                self.coeff_base_lf_eob_uv.as_mut_slice()
            }
            CoefficientCdfRowSelector::LumaCoeffBaseLfCtx1 => {
                self.coeff_base_lf_ctx1.as_mut_slice()
            }
            CoefficientCdfRowSelector::LumaCoeffBaseLfCtx2 => {
                self.coeff_base_lf_ctx2.as_mut_slice()
            }
            CoefficientCdfRowSelector::LumaCoeffBaseLfCtx9 => {
                self.coeff_base_lf_ctx9.as_mut_slice()
            }
            CoefficientCdfRowSelector::LumaCoeffBaseLfCtx4 => {
                self.coeff_base_lf_ctx4.as_mut_slice()
            }
            CoefficientCdfRowSelector::LumaCoeffBrLfDc => self.coeff_br_lf_dc.as_mut_slice(),
            CoefficientCdfRowSelector::LumaDcSign => self.dc_sign.as_mut_slice(),
        }
    }
}
