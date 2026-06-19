// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The scoped-default §8.2 CDF-row router (`CoefficientTokenCdfRows`) for the
//! generic `roundtrip_entropy_tokens` proof. Split out of
//! `coefficient_tokenization` to keep the parent file under the 1000-line budget.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoefficientTokenCdfRows {
    txb_skip: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
    eob_pt_16: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob_ac: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf: [[i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_CDF_Q_CONTEXTS],
    coeff_br_lf: [[i32; 5]; COEFF_CDF_Q_CONTEXTS],
    dc_sign: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
    chroma_u_txb_skip: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
    chroma_eob_pt_16: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob_uv: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    // The `intra_tx_type` CDF is not coefficient-CDF-q-context indexed; this is the
    // 4x4 (`Tx_Size_Sqr = 0`) `TX_SET_INTRA_1` row.
    intra_tx_type_set1_4x4: [i32; INTRA_TX_TYPE_SET1_CDF_ROW_LEN],
}

impl CoefficientTokenCdfRows {
    pub(crate) fn from_defaults() -> Self {
        Self {
            txb_skip: [
                DEFAULT_TXB_SKIP_CDF[0][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[1][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[2][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[3][LUMA_PLANE_TYPE][TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
            ],
            eob_pt_16: [
                DEFAULT_EOB_PT_16_CDF[0][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[1][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[2][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[3][EOB_CTX_LUMA_INTRA],
            ],
            coeff_base_lf_eob: [
                DEFAULT_COEFF_BASE_LF_EOB_CDF[0][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[1][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[2][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[3][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_DC],
            ],
            coeff_base_lf_eob_ac: [
                DEFAULT_COEFF_BASE_LF_EOB_CDF[0][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_EOB2_AC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[1][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_EOB2_AC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[2][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_EOB2_AC],
                DEFAULT_COEFF_BASE_LF_EOB_CDF[3][TX_SIZE_4X4_CTX][COEFF_BASE_LF_EOB_CTX_EOB2_AC],
            ],
            coeff_base_lf: [
                DEFAULT_COEFF_BASE_LF_CDF[0][TX_SIZE_4X4_CTX][COEFF_BASE_LF_CTX_EOB2_DC]
                    [COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
                DEFAULT_COEFF_BASE_LF_CDF[1][TX_SIZE_4X4_CTX][COEFF_BASE_LF_CTX_EOB2_DC]
                    [COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
                DEFAULT_COEFF_BASE_LF_CDF[2][TX_SIZE_4X4_CTX][COEFF_BASE_LF_CTX_EOB2_DC]
                    [COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
                DEFAULT_COEFF_BASE_LF_CDF[3][TX_SIZE_4X4_CTX][COEFF_BASE_LF_CTX_EOB2_DC]
                    [COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
            ],
            coeff_br_lf: [
                DEFAULT_COEFF_BR_LF_CDF[0][COEFF_BR_LF_CTX_DC],
                DEFAULT_COEFF_BR_LF_CDF[1][COEFF_BR_LF_CTX_DC],
                DEFAULT_COEFF_BR_LF_CDF[2][COEFF_BR_LF_CTX_DC],
                DEFAULT_COEFF_BR_LF_CDF[3][COEFF_BR_LF_CTX_DC],
            ],
            dc_sign: [
                DEFAULT_DC_SIGN_CDF[0][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
                DEFAULT_DC_SIGN_CDF[1][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
                DEFAULT_DC_SIGN_CDF[2][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
                DEFAULT_DC_SIGN_CDF[3][LUMA_PLANE_TYPE][DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
            ],
            chroma_u_txb_skip: [
                DEFAULT_TXB_SKIP_CDF[0][INTRA_NON_FSC_TXB_SKIP_BANK][TX_SIZE_4X4_CTX]
                    [CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[1][INTRA_NON_FSC_TXB_SKIP_BANK][TX_SIZE_4X4_CTX]
                    [CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[2][INTRA_NON_FSC_TXB_SKIP_BANK][TX_SIZE_4X4_CTX]
                    [CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[3][INTRA_NON_FSC_TXB_SKIP_BANK][TX_SIZE_4X4_CTX]
                    [CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
            ],
            chroma_eob_pt_16: [
                DEFAULT_EOB_PT_16_CDF[0][EOB_CTX_CHROMA],
                DEFAULT_EOB_PT_16_CDF[1][EOB_CTX_CHROMA],
                DEFAULT_EOB_PT_16_CDF[2][EOB_CTX_CHROMA],
                DEFAULT_EOB_PT_16_CDF[3][EOB_CTX_CHROMA],
            ],
            coeff_base_lf_eob_uv: [
                DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[0][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[1][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[2][COEFF_BASE_LF_EOB_CTX_DC],
                DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[3][COEFF_BASE_LF_EOB_CTX_DC],
            ],
            intra_tx_type_set1_4x4: DEFAULT_INTRA_TX_TYPE_SET1_CDF
                [INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4],
        }
    }

    pub(crate) fn row_mut(&mut self, selector: CoefficientCdfRowSelector) -> Result<&mut [i32]> {
        match selector {
            CoefficientCdfRowSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: LUMA_PLANE_TYPE,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: TXB_SKIP_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.txb_skip[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::EobPt16 {
                coeff_cdf_q_ctx,
                eob_ctx: EOB_CTX_LUMA_INTRA,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.eob_pt_16[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLfEob {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: COEFF_BASE_LF_EOB_CTX_DC,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.coeff_base_lf_eob[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLfEob {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.coeff_base_lf_eob_ac[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: COEFF_BASE_LF_CTX_EOB2_DC,
                tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.coeff_base_lf[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBrLf {
                coeff_cdf_q_ctx,
                ctx: COEFF_BR_LF_CTX_DC,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.coeff_br_lf[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::DcSign {
                coeff_cdf_q_ctx,
                plane_type: LUMA_PLANE_TYPE,
                group: DC_SIGN_GROUP_VISIBLE,
                ctx: DC_SIGN_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.dc_sign[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: INTRA_NON_FSC_TXB_SKIP_BANK,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.chroma_u_txb_skip[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::EobPt16 {
                coeff_cdf_q_ctx,
                eob_ctx: EOB_CTX_CHROMA,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.chroma_eob_pt_16[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLfEobUv {
                coeff_cdf_q_ctx,
                ctx: COEFF_BASE_LF_EOB_CTX_DC,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.coeff_base_lf_eob_uv[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::IntraTxTypeSet1 {
                tx_size_sqr: INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4,
            } => Ok(self.intra_tx_type_set1_4x4.as_mut_slice()),
            selector => Err(Error::CoefficientTokenizationUnsupportedCdfSelector {
                syntax: selector.syntax_name(),
            }),
        }
    }
}
