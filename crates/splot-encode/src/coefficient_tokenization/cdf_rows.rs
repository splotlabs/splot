// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The scoped-default §8.2 CDF-row router (`CoefficientTokenCdfRows`) for the
//! generic `roundtrip_entropy_tokens` proof. Split out of
//! `coefficient_tokenization` to keep the parent file under the 1000-line budget.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoefficientTokenCdfRows {
    txb_skip: [[u16; 3]; COEFF_CDF_Q_CONTEXTS],
    txb_skip_16x16: [[u16; 3]; COEFF_CDF_Q_CONTEXTS],
    eob_pt_16: [[u16; 6]; COEFF_CDF_Q_CONTEXTS],
    eob_pt_256: [[u16; 9]; COEFF_CDF_Q_CONTEXTS],
    eob_extra: [[u16; 3]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob:
        [[[u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf:
        [[[u16; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_br_lf: [[[u16; COEFF_BR_LF_CDF_ROW_LEN]; COEFF_BR_LF_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_eob_hf:
        [[[u16; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_br_hf: [[[u16; COEFF_BR_CDF_ROW_LEN]; COEFF_BR_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_hf: [[[u16; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob_16x16_full:
        [[[u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_16x16:
        [[[u16; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_eob_hf_16x16:
        [[[u16; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_hf_16x16:
        [[[u16; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    dc_sign: [[u16; 3]; COEFF_CDF_Q_CONTEXTS],
    chroma_u_txb_skip: [[u16; 3]; COEFF_CDF_Q_CONTEXTS],
    chroma_eob_pt_16: [[u16; 6]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob_uv: [[u16; 6]; COEFF_CDF_Q_CONTEXTS],
    intra_tx_type_set1:
        [[u16; INTRA_TX_TYPE_SET1_CDF_ROW_LEN]; INTRA_TX_TYPE_SET1_TX_SIZE_SQR_COUNT],
    sec_tx_type_intra: [[u16; SEC_TX_TYPE_CDF_ROW_LEN]; SEC_TX_TYPE_TX_SIZE_SQR_COUNT],
}

macro_rules! coeff_eob_banks {
    ($table:expr, $tx_size:expr, $row_len:expr, $ctx_count:expr) => {{
        let mut bank = [[[0u16; $row_len]; $ctx_count]; COEFF_CDF_Q_CONTEXTS];
        let mut q = 0;
        while q < COEFF_CDF_Q_CONTEXTS {
            let mut ctx = 0;
            while ctx < $ctx_count {
                bank[q][ctx] = ($table)[q][$tx_size][ctx];
                ctx += 1;
            }
            q += 1;
        }
        bank
    }};
}

macro_rules! coeff_base_banks {
    ($table:expr, $tx_size:expr, $row_len:expr, $ctx_count:expr) => {{
        let mut bank = [[[0u16; $row_len]; $ctx_count]; COEFF_CDF_Q_CONTEXTS];
        let mut q = 0;
        while q < COEFF_CDF_Q_CONTEXTS {
            let mut ctx = 0;
            while ctx < $ctx_count {
                bank[q][ctx] = ($table)[q][$tx_size][ctx][COEFF_BASE_LF_TCQ_CTX_NEUTRAL];
                ctx += 1;
            }
            q += 1;
        }
        bank
    }};
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
            txb_skip_16x16: [
                DEFAULT_TXB_SKIP_CDF[0][LUMA_PLANE_TYPE][TX_SIZE_16X16_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[1][LUMA_PLANE_TYPE][TX_SIZE_16X16_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[2][LUMA_PLANE_TYPE][TX_SIZE_16X16_CTX][TXB_SKIP_CTX_NEUTRAL],
                DEFAULT_TXB_SKIP_CDF[3][LUMA_PLANE_TYPE][TX_SIZE_16X16_CTX][TXB_SKIP_CTX_NEUTRAL],
            ],
            eob_pt_16: [
                DEFAULT_EOB_PT_16_CDF[0][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[1][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[2][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_16_CDF[3][EOB_CTX_LUMA_INTRA],
            ],
            eob_pt_256: [
                DEFAULT_EOB_PT_256_CDF[0][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_256_CDF[1][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_256_CDF[2][EOB_CTX_LUMA_INTRA],
                DEFAULT_EOB_PT_256_CDF[3][EOB_CTX_LUMA_INTRA],
            ],
            eob_extra: DEFAULT_EOB_EXTRA_CDF,
            coeff_base_lf_eob: coeff_eob_banks!(
                DEFAULT_COEFF_BASE_LF_EOB_CDF,
                TX_SIZE_4X4_CTX,
                COEFF_BASE_LF_EOB_CDF_ROW_LEN,
                COEFF_BASE_LF_EOB_CTX_COUNT
            ),
            coeff_base_lf: coeff_base_banks!(
                DEFAULT_COEFF_BASE_LF_CDF,
                TX_SIZE_4X4_CTX,
                COEFF_BASE_LF_CDF_ROW_LEN,
                COEFF_BASE_LF_CTX_COUNT
            ),
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF,
            coeff_base_eob_hf: coeff_eob_banks!(
                DEFAULT_COEFF_BASE_EOB_CDF,
                TX_SIZE_4X4_CTX,
                COEFF_BASE_EOB_CDF_ROW_LEN,
                COEFF_BASE_EOB_CTX_COUNT
            ),
            coeff_br_hf: DEFAULT_COEFF_BR_CDF,
            coeff_base_hf: coeff_base_banks!(
                DEFAULT_COEFF_BASE_CDF,
                TX_SIZE_4X4_CTX,
                COEFF_BASE_CDF_ROW_LEN,
                COEFF_BASE_CTX_COUNT
            ),
            coeff_base_lf_eob_16x16_full: coeff_eob_banks!(
                DEFAULT_COEFF_BASE_LF_EOB_CDF,
                TX_SIZE_16X16_CTX,
                COEFF_BASE_LF_EOB_CDF_ROW_LEN,
                COEFF_BASE_LF_EOB_CTX_COUNT
            ),
            coeff_base_lf_16x16: coeff_base_banks!(
                DEFAULT_COEFF_BASE_LF_CDF,
                TX_SIZE_16X16_CTX,
                COEFF_BASE_LF_CDF_ROW_LEN,
                COEFF_BASE_LF_CTX_COUNT
            ),
            coeff_base_eob_hf_16x16: coeff_eob_banks!(
                DEFAULT_COEFF_BASE_EOB_CDF,
                TX_SIZE_16X16_CTX,
                COEFF_BASE_EOB_CDF_ROW_LEN,
                COEFF_BASE_EOB_CTX_COUNT
            ),
            coeff_base_hf_16x16: coeff_base_banks!(
                DEFAULT_COEFF_BASE_CDF,
                TX_SIZE_16X16_CTX,
                COEFF_BASE_CDF_ROW_LEN,
                COEFF_BASE_CTX_COUNT
            ),
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
            intra_tx_type_set1: DEFAULT_INTRA_TX_TYPE_SET1_CDF,
            sec_tx_type_intra: DEFAULT_SEC_TX_TYPE_CDF[SEC_TX_TYPE_INTRA_BANK],
        }
    }

    pub(crate) fn row_mut(&mut self, selector: CoefficientCdfRowSelector) -> Result<&mut [u16]> {
        match selector {
            CoefficientCdfRowSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: LUMA_PLANE_TYPE,
                tx_size: TX_SIZE_4X4_CTX,
                ctx: TXB_SKIP_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.txb_skip[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::TxbSkip {
                coeff_cdf_q_ctx,
                plane_type: LUMA_PLANE_TYPE,
                tx_size: TX_SIZE_16X16_CTX,
                ctx: TXB_SKIP_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.txb_skip_16x16[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::EobPt16 {
                coeff_cdf_q_ctx,
                eob_ctx: EOB_CTX_LUMA_INTRA,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.eob_pt_16[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::EobPt256 {
                coeff_cdf_q_ctx,
                eob_ctx: EOB_CTX_LUMA_INTRA,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS => {
                Ok(self.eob_pt_256[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::EobExtra { coeff_cdf_q_ctx }
                if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS =>
            {
                Ok(self.eob_extra[coeff_cdf_q_ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLfEob {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_LF_EOB_CTX_COUNT => {
                Ok(self.coeff_base_lf_eob[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLfEob {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_16X16_CTX,
                ctx,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_LF_EOB_CTX_COUNT => {
                Ok(self.coeff_base_lf_eob_16x16_full[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx,
                tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_LF_CTX_COUNT => {
                Ok(self.coeff_base_lf[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_16X16_CTX,
                ctx,
                tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_LF_CTX_COUNT => {
                Ok(self.coeff_base_lf_16x16[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBrLf {
                coeff_cdf_q_ctx,
                ctx,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BR_LF_CTX_COUNT => {
                Ok(self.coeff_br_lf[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseEob {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_EOB_CTX_COUNT => {
                Ok(self.coeff_base_eob_hf[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBaseEob {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_16X16_CTX,
                ctx,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_EOB_CTX_COUNT => {
                Ok(self.coeff_base_eob_hf_16x16[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBr {
                coeff_cdf_q_ctx,
                ctx,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BR_CTX_COUNT => {
                Ok(self.coeff_br_hf[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBase {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx,
                tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_CTX_COUNT => {
                Ok(self.coeff_base_hf[coeff_cdf_q_ctx][ctx].as_mut_slice())
            }
            CoefficientCdfRowSelector::CoeffBase {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_16X16_CTX,
                ctx,
                tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_CTX_COUNT => {
                Ok(self.coeff_base_hf_16x16[coeff_cdf_q_ctx][ctx].as_mut_slice())
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
            CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr }
                if tx_size_sqr < INTRA_TX_TYPE_SET1_TX_SIZE_SQR_COUNT =>
            {
                Ok(self.intra_tx_type_set1[tx_size_sqr].as_mut_slice())
            }
            CoefficientCdfRowSelector::SecTxTypeIntra { tx_size_sqr }
                if tx_size_sqr < SEC_TX_TYPE_TX_SIZE_SQR_COUNT =>
            {
                Ok(self.sec_tx_type_intra[tx_size_sqr].as_mut_slice())
            }
            selector => Err(Error::CoefficientTokenizationUnsupportedCdfSelector {
                syntax: selector.syntax_name(),
            }),
        }
    }
}
