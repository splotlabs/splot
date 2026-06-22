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
    // The §5.20.7.27 `eob_extra` binary CDF, indexed only by the coefficient
    // CDF q-context (no per-eobPt context). Routes the `eob_extra_token` through
    // this generic entropy proof, mirroring its block-symbol-trace routing.
    eob_extra: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
    // The 4x4 low-frequency `coeff_base_eob` bank, indexed by `[q][ctx]` over the
    // full §8.3.2 `coeff_base_eob` context dimension (`COEFF_BASE_LF_EOB_CTX_COUNT`).
    // Sizing the full dimension makes the entropy-proof 4x4-LF EOB tier hole-free
    // (eob 3/4 reaches ctx 2; eob<=2 only reached ctx 0/1).
    coeff_base_lf_eob:
        [[[i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    // The 4x4 low-frequency non-EOB `coeff_base` bank at the neutral TCQ context,
    // indexed by `[q][ctx]` over the full §8.3.2 `coeff_base` low-frequency context
    // dimension (`COEFF_BASE_LF_CTX_COUNT`). The general LF walk derives this context
    // from the running `Level[]`; sizing the full dimension makes the tier hole-free
    // (eob 3/4 reaches e.g. ctx 9).
    coeff_base_lf:
        [[[i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    // The `coeff_br` low-frequency bank, indexed by `[q][ctx]` over the full §8.3.2
    // `coeff_br` context dimension (`COEFF_BR_LF_CTX_COUNT`; no transform-size
    // dimension). `DEFAULT_COEFF_BR_LF_CDF` is already `[q][ctx][row]`.
    coeff_br_lf: [[[i32; COEFF_BR_LF_CDF_ROW_LEN]; COEFF_BR_LF_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    // The 4x4 HIGH-frequency `coeff_base_eob` bank, indexed by `[q][ctx]` over the
    // full §8.3.2 HF `coeff_base_eob` context dimension (`COEFF_BASE_EOB_CTX_COUNT`,
    // 4-symbol rows). Sourced from `DEFAULT_COEFF_BASE_EOB_CDF[q][TX_SIZE_4X4_CTX][ctx]`
    // (distinct from the 6-symbol LF `coeff_base_lf_eob` bank). The eob-11 HF EOB
    // coefficient reaches `coeff_base_eob_ctx(10) = 3`; sizing the full ctx-4 dimension
    // makes the HF EOB tier hole-free.
    coeff_base_eob_hf:
        [[[i32; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    // The HIGH-frequency `coeff_br` bank, indexed by `[q][ctx]` over the full §8.3.2 HF
    // `coeff_br` context dimension (`COEFF_BR_CTX_COUNT`, 7; no transform-size
    // dimension). `DEFAULT_COEFF_BR_CDF` is already `[q][ctx][row]` (distinct from the
    // LF `coeff_br_lf` bank, which has 14 contexts). The eob-11 HF EOB `coeff_br`
    // reaches the constant ctx 0; sizing the full ctx-7 dimension makes the tier
    // hole-free for the later non-EOB HF sub-brick.
    coeff_br_hf: [[[i32; COEFF_BR_CDF_ROW_LEN]; COEFF_BR_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS],
    dc_sign: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
    chroma_u_txb_skip: [[i32; 3]; COEFF_CDF_Q_CONTEXTS],
    chroma_eob_pt_16: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    coeff_base_lf_eob_uv: [[i32; 6]; COEFF_CDF_Q_CONTEXTS],
    // The `intra_tx_type` CDF is not coefficient-CDF-q-context indexed; it has one
    // `TX_SET_INTRA_1` row per `Tx_Size_Sqr` value.
    intra_tx_type_set1:
        [[i32; INTRA_TX_TYPE_SET1_CDF_ROW_LEN]; INTRA_TX_TYPE_SET1_TX_SIZE_SQR_COUNT],
    // The intra `sec_tx_type` CDF (`is_inter = 0` bank); one row per `Tx_Size_Sqr`.
    sec_tx_type_intra: [[i32; SEC_TX_TYPE_CDF_ROW_LEN]; SEC_TX_TYPE_TX_SIZE_SQR_COUNT],
}

/// Builds the 4x4 low-frequency `coeff_base_eob` bank `[q][ctx]` from the generated
/// `DEFAULT_COEFF_BASE_LF_EOB_CDF[q][4x4][ctx]` table. Total and panic-free: every
/// index is a const within the table dimensions.
fn coeff_base_lf_eob_bank()
-> [[[i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS] {
    let mut bank = [[[0i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT];
        COEFF_CDF_Q_CONTEXTS];
    let mut q = 0;
    while q < COEFF_CDF_Q_CONTEXTS {
        let mut ctx = 0;
        while ctx < COEFF_BASE_LF_EOB_CTX_COUNT {
            bank[q][ctx] = DEFAULT_COEFF_BASE_LF_EOB_CDF[q][TX_SIZE_4X4_CTX][ctx];
            ctx += 1;
        }
        q += 1;
    }
    bank
}

/// Builds the 4x4 HIGH-frequency `coeff_base_eob` bank `[q][ctx]` from the generated
/// `DEFAULT_COEFF_BASE_EOB_CDF[q][4x4][ctx]` table (4-symbol rows). Total and
/// panic-free: every index is a const within the table dimensions.
fn coeff_base_eob_hf_bank()
-> [[[i32; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS] {
    let mut bank =
        [[[0i32; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS];
    let mut q = 0;
    while q < COEFF_CDF_Q_CONTEXTS {
        let mut ctx = 0;
        while ctx < COEFF_BASE_EOB_CTX_COUNT {
            bank[q][ctx] = DEFAULT_COEFF_BASE_EOB_CDF[q][TX_SIZE_4X4_CTX][ctx];
            ctx += 1;
        }
        q += 1;
    }
    bank
}

/// Builds the 4x4 low-frequency non-EOB `coeff_base` bank `[q][ctx]` at the neutral
/// TCQ context from the generated `DEFAULT_COEFF_BASE_LF_CDF[q][4x4][ctx][tcq]` table.
/// Total and panic-free: every index is a const within the table dimensions.
fn coeff_base_lf_bank()
-> [[[i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS] {
    let mut bank =
        [[[0i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT]; COEFF_CDF_Q_CONTEXTS];
    let mut q = 0;
    while q < COEFF_CDF_Q_CONTEXTS {
        let mut ctx = 0;
        while ctx < COEFF_BASE_LF_CTX_COUNT {
            bank[q][ctx] =
                DEFAULT_COEFF_BASE_LF_CDF[q][TX_SIZE_4X4_CTX][ctx][COEFF_BASE_LF_TCQ_CTX_NEUTRAL];
            ctx += 1;
        }
        q += 1;
    }
    bank
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
            eob_extra: DEFAULT_EOB_EXTRA_CDF,
            coeff_base_lf_eob: coeff_base_lf_eob_bank(),
            coeff_base_lf: coeff_base_lf_bank(),
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF,
            coeff_base_eob_hf: coeff_base_eob_hf_bank(),
            coeff_br_hf: DEFAULT_COEFF_BR_CDF,
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
            CoefficientCdfRowSelector::CoeffBaseLf {
                coeff_cdf_q_ctx,
                tx_size: TX_SIZE_4X4_CTX,
                ctx,
                tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BASE_LF_CTX_COUNT => {
                Ok(self.coeff_base_lf[coeff_cdf_q_ctx][ctx].as_mut_slice())
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
            CoefficientCdfRowSelector::CoeffBr {
                coeff_cdf_q_ctx,
                ctx,
            } if coeff_cdf_q_ctx < COEFF_CDF_Q_CONTEXTS && ctx < COEFF_BR_CTX_COUNT => {
                Ok(self.coeff_br_hf[coeff_cdf_q_ctx][ctx].as_mut_slice())
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
