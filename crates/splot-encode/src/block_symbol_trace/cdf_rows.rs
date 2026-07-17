// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The unified scoped default-CDF rows for the minimal block-symbol trace
//! (`BlockSymbolTraceCdfRows` with its `from_defaults()` initializer and `row_mut()`
//! selector). Split out of `block_symbol_trace` to keep each file under the
//! 1000-line source budget.

use super::*;

/// Unified scoped default-CDF rows for the minimal block-symbol trace, built
/// directly from `splot-core` defaults so the trace module does not reach into
/// the emitter modules' private CDF-row internals.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BlockSymbolTraceCdfRows {
    do_split_root: [u16; DO_SPLIT_CDF_ROW_LEN],
    do_square_split_root: [u16; DO_SQUARE_SPLIT_CDF_ROW_LEN],
    y_mode_set: [u16; Y_MODE_SET_CDF_ROW_LEN],
    y_mode_index_tile_origin: [u16; INTRA_MODE_CDF_ROW_LEN],
    uv_mode_non_directional: [u16; INTRA_MODE_CDF_ROW_LEN],
    luma_txb_skip: [u16; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip: [u16; TXB_SKIP_CDF_ROW_LEN],
    v_txb_skip: [u16; V_TXB_SKIP_CDF_ROW_LEN],
    luma_txb_skip_64x64: [u16; TXB_SKIP_CDF_ROW_LEN],
    luma_txb_skip_16x16: [u16; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip_32x32: [u16; TXB_SKIP_CDF_ROW_LEN],
    eob_pt_16: [u16; EOB_PT_16_CDF_ROW_LEN],
    eob_pt_1024: [u16; EOB_PT_1024_CDF_ROW_LEN],
    eob_pt_256: [u16; EOB_PT_256_CDF_ROW_LEN],
    eob_pt_1024_chroma: [u16; EOB_PT_1024_CDF_ROW_LEN],
    eob_extra: [u16; EOB_EXTRA_CDF_ROW_LEN],
    coeff_base_lf_eob_tx64: [u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    coeff_base_lf_eob_tx16: [u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    intra_tx_type_set1_4x4: [u16; INTRA_TX_TYPE_SET1_CDF_ROW_LEN],
    sec_tx_type_intra_4x4: [u16; SEC_TX_TYPE_INTRA_CDF_ROW_LEN],
    coeff_base_lf_eob_4x4: [[u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT],
    coeff_base_lf_eob_ac_tx64: [u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    coeff_base_lf_4x4: [[u16; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT],
    coeff_base_lf_dc_tx64: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_dc_tx64_visible_ac: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_ac_tx64_ctx9: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_dc_tx64_2d: [u16; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_br_lf: [[u16; COEFF_BR_LF_CDF_ROW_LEN]; COEFF_BR_LF_CTX_COUNT],
    coeff_base_eob_hf_4x4: [[u16; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT],
    coeff_br_hf: [[u16; COEFF_BR_CDF_ROW_LEN]; COEFF_BR_CTX_COUNT],
    coeff_base_hf_4x4: [[u16; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT],
    coeff_base_lf_eob_16x16: [[u16; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT],
    coeff_base_lf_16x16: [[u16; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT],
    coeff_base_eob_hf_16x16: [[u16; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT],
    coeff_base_hf_16x16: [[u16; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT],
    dc_sign: [u16; DC_SIGN_CDF_ROW_LEN],
    v_txb_skip_eobu: [u16; V_TXB_SKIP_CDF_ROW_LEN],
    chroma_eob_pt_16: [u16; EOB_PT_16_CDF_ROW_LEN],
    coeff_base_lf_eob_uv: [u16; COEFF_BASE_LF_EOB_UV_CDF_ROW_LEN],
}

macro_rules! coeff_base_bank {
    ($table:expr, $tx_size:expr, $row_len:expr, $ctx_count:expr) => {{
        let mut bank = [[0u16; $row_len]; $ctx_count];
        let mut ctx = 0;
        while ctx < $ctx_count {
            bank[ctx] =
                ($table)[MINIMAL_COEFF_CDF_Q_CTX][$tx_size][ctx][COEFF_BASE_LF_TCQ_CTX_NEUTRAL];
            ctx += 1;
        }
        bank
    }};
}

impl BlockSymbolTraceCdfRows {
    pub(super) fn from_defaults() -> Self {
        Self {
            do_split_root: DEFAULT_DO_SPLIT_CDF[ROOT_PARTITION_PLANE_START]
                [ROOT_64X64_DO_SPLIT_CTX],
            do_square_split_root: DEFAULT_DO_SQUARE_SPLIT_CDF[ROOT_PARTITION_PLANE_START]
                [ROOT_64X64_DO_SQUARE_SPLIT_CTX],
            y_mode_set: DEFAULT_Y_MODE_SET_CDF,
            y_mode_index_tile_origin: DEFAULT_Y_MODE_INDEX_CDF[TILE_ORIGIN_Y_MODE_INDEX_CTX],
            uv_mode_non_directional: DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF
                [NON_DIRECTIONAL_UV_MODE_CTX],
            luma_txb_skip: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_4X4_CTX][TXB_SKIP_CTX_NEUTRAL],
            u_txb_skip: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_4X4_CTX][CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
            v_txb_skip: DEFAULT_V_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][V_TXB_SKIP_CTX_NEUTRAL],
            luma_txb_skip_64x64: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_64X64_CTX][TXB_SKIP_CTX_NEUTRAL],
            luma_txb_skip_16x16: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_16X16_CTX][TXB_SKIP_CTX_NEUTRAL],
            u_txb_skip_32x32: DEFAULT_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX][LUMA_PLANE_TYPE]
                [TX_SIZE_32X32_CTX][CHROMA_U_TXB_SKIP_CTX_NEUTRAL],
            eob_pt_16: DEFAULT_EOB_PT_16_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_LUMA_INTRA],
            eob_pt_1024: DEFAULT_EOB_PT_1024_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_LUMA_INTRA],
            eob_pt_256: DEFAULT_EOB_PT_256_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_LUMA_INTRA],
            eob_pt_1024_chroma: DEFAULT_EOB_PT_1024_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_CHROMA],
            eob_extra: DEFAULT_EOB_EXTRA_CDF[MINIMAL_COEFF_CDF_Q_CTX],
            coeff_base_lf_eob_tx64: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_64X64_CTX][COEFF_BASE_LF_EOB_CTX_DC],
            coeff_base_lf_eob_tx16: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_16X16_CTX][COEFF_BASE_LF_EOB_CTX_DC],
            intra_tx_type_set1_4x4: DEFAULT_INTRA_TX_TYPE_SET1_CDF
                [INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4],
            sec_tx_type_intra_4x4: DEFAULT_SEC_TX_TYPE_CDF[SEC_TX_TYPE_INTRA_BANK]
                [SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4],
            coeff_base_lf_eob_4x4: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_4X4_CTX],
            coeff_base_lf_eob_ac_tx64: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_64X64_CTX][COEFF_BASE_LF_EOB_CTX_EOB2_AC],
            coeff_base_lf_4x4: coeff_base_bank!(
                DEFAULT_COEFF_BASE_LF_CDF,
                TX_SIZE_4X4_CTX,
                COEFF_BASE_LF_CDF_ROW_LEN,
                COEFF_BASE_LF_CTX_COUNT
            ),
            coeff_base_lf_dc_tx64: DEFAULT_COEFF_BASE_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_64X64_CTX][COEFF_BASE_LF_CTX_EOB2_DC][COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
            coeff_base_lf_dc_tx64_visible_ac: DEFAULT_COEFF_BASE_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_64X64_CTX][COEFF_BASE_LF_CTX_VISIBLE_AC_DC][COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
            coeff_base_lf_ac_tx64_ctx9: DEFAULT_COEFF_BASE_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_64X64_CTX][COEFF_BASE_LF_CTX_AC_BAND_BASE][COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
            coeff_base_lf_dc_tx64_2d: DEFAULT_COEFF_BASE_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_64X64_CTX][COEFF_BASE_LF_CTX_2D_DC][COEFF_BASE_LF_TCQ_CTX_NEUTRAL],
            coeff_br_lf: DEFAULT_COEFF_BR_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX],
            coeff_base_eob_hf_4x4: DEFAULT_COEFF_BASE_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_4X4_CTX],
            coeff_br_hf: DEFAULT_COEFF_BR_CDF[MINIMAL_COEFF_CDF_Q_CTX],
            coeff_base_hf_4x4: coeff_base_bank!(
                DEFAULT_COEFF_BASE_CDF,
                TX_SIZE_4X4_CTX,
                COEFF_BASE_CDF_ROW_LEN,
                COEFF_BASE_CTX_COUNT
            ),
            coeff_base_lf_eob_16x16: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_16X16_CTX],
            coeff_base_lf_16x16: coeff_base_bank!(
                DEFAULT_COEFF_BASE_LF_CDF,
                TX_SIZE_16X16_CTX,
                COEFF_BASE_LF_CDF_ROW_LEN,
                COEFF_BASE_LF_CTX_COUNT
            ),
            coeff_base_eob_hf_16x16: DEFAULT_COEFF_BASE_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_16X16_CTX],
            coeff_base_hf_16x16: coeff_base_bank!(
                DEFAULT_COEFF_BASE_CDF,
                TX_SIZE_16X16_CTX,
                COEFF_BASE_CDF_ROW_LEN,
                COEFF_BASE_CTX_COUNT
            ),
            dc_sign: DEFAULT_DC_SIGN_CDF[MINIMAL_COEFF_CDF_Q_CTX][DC_SIGN_PLANE_TYPE_LUMA]
                [DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
            v_txb_skip_eobu: DEFAULT_V_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [CHROMA_V_TXB_SKIP_CTX_EOBU],
            chroma_eob_pt_16: DEFAULT_EOB_PT_16_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_CHROMA],
            coeff_base_lf_eob_uv: DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [COEFF_BASE_LF_EOB_CTX_DC],
        }
    }

    pub(super) fn row_mut(&mut self, token: BlockSymbolToken, index: usize) -> Result<&mut [u16]> {
        match token {
            BlockSymbolToken::Bypass { .. } => {
                Err(Error::BlockSymbolTraceUnsupportedSelector { index })
            }
            BlockSymbolToken::Partition(partition) => match partition.selector() {
                PartitionCdfRowSelector::DoSplit {
                    plane_start: ROOT_PARTITION_PLANE_START,
                    ctx: ROOT_64X64_DO_SPLIT_CTX,
                } => Ok(self.do_split_root.as_mut_slice()),
                PartitionCdfRowSelector::DoSquareSplit {
                    plane_start: ROOT_PARTITION_PLANE_START,
                    ctx: ROOT_64X64_DO_SQUARE_SPLIT_CTX,
                } => Ok(self.do_square_split_root.as_mut_slice()),
                _ => Err(Error::BlockSymbolTraceUnsupportedSelector { index }),
            },
            BlockSymbolToken::Mode(mode) => match mode.selector() {
                IntraModeCdfRowSelector::YModeSet => Ok(self.y_mode_set.as_mut_slice()),
                IntraModeCdfRowSelector::YModeIndex {
                    ctx: TILE_ORIGIN_Y_MODE_INDEX_CTX,
                } => Ok(self.y_mode_index_tile_origin.as_mut_slice()),
                IntraModeCdfRowSelector::UvModeCflNotAllowed {
                    ctx: NON_DIRECTIONAL_UV_MODE_CTX,
                } => Ok(self.uv_mode_non_directional.as_mut_slice()),
                _ => Err(Error::BlockSymbolTraceUnsupportedSelector { index }),
            },
            BlockSymbolToken::Coeff(coeff) => match coeff.selector() {
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.luma_txb_skip.as_mut_slice()),
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.u_txb_skip.as_mut_slice()),
                CoefficientCdfRowSelector::VTxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx: V_TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.v_txb_skip.as_mut_slice()),
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.luma_txb_skip_64x64.as_mut_slice()),
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_16X16_CTX,
                    ctx: TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.luma_txb_skip_16x16.as_mut_slice()),
                CoefficientCdfRowSelector::TxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: LUMA_PLANE_TYPE,
                    tx_size: TX_SIZE_32X32_CTX,
                    ctx: CHROMA_U_TXB_SKIP_CTX_NEUTRAL,
                } => Ok(self.u_txb_skip_32x32.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt16 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_LUMA_INTRA,
                } => Ok(self.eob_pt_16.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt1024 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_LUMA_INTRA,
                } => Ok(self.eob_pt_1024.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt256 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_LUMA_INTRA,
                } => Ok(self.eob_pt_256.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt1024 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_CHROMA,
                } => Ok(self.eob_pt_1024_chroma.as_mut_slice()),
                CoefficientCdfRowSelector::EobExtra {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                } => Ok(self.eob_extra.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                } => Ok(self.coeff_base_lf_eob_tx64.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_16X16_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                } => Ok(self.coeff_base_lf_eob_tx16.as_mut_slice()),
                CoefficientCdfRowSelector::IntraTxTypeSet1 {
                    tx_size_sqr: INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4,
                } => Ok(self.intra_tx_type_set1_4x4.as_mut_slice()),
                CoefficientCdfRowSelector::SecTxTypeIntra {
                    tx_size_sqr: SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4,
                } => Ok(self.sec_tx_type_intra_4x4.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx,
                } if ctx < COEFF_BASE_LF_EOB_CTX_COUNT => {
                    Ok(self.coeff_base_lf_eob_4x4[ctx].as_mut_slice())
                }
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_16X16_CTX,
                    ctx,
                } if ctx < COEFF_BASE_LF_EOB_CTX_COUNT => {
                    Ok(self.coeff_base_lf_eob_16x16[ctx].as_mut_slice())
                }
                CoefficientCdfRowSelector::CoeffBaseLfEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_EOB2_AC,
                } => Ok(self.coeff_base_lf_eob_ac_tx64.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } if ctx < COEFF_BASE_LF_CTX_COUNT => {
                    Ok(self.coeff_base_lf_4x4[ctx].as_mut_slice())
                }
                CoefficientCdfRowSelector::CoeffBaseLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_16X16_CTX,
                    ctx,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } if ctx < COEFF_BASE_LF_CTX_COUNT => {
                    Ok(self.coeff_base_lf_16x16[ctx].as_mut_slice())
                }
                CoefficientCdfRowSelector::CoeffBaseLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: COEFF_BASE_LF_CTX_EOB2_DC,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } => Ok(self.coeff_base_lf_dc_tx64.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: COEFF_BASE_LF_CTX_VISIBLE_AC_DC,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } => Ok(self.coeff_base_lf_dc_tx64_visible_ac.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: COEFF_BASE_LF_CTX_AC_BAND_BASE,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } => Ok(self.coeff_base_lf_ac_tx64_ctx9.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_64X64_CTX,
                    ctx: COEFF_BASE_LF_CTX_2D_DC,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } => Ok(self.coeff_base_lf_dc_tx64_2d.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBrLf {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx,
                } if ctx < COEFF_BR_LF_CTX_COUNT => Ok(self.coeff_br_lf[ctx].as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx,
                } if ctx < COEFF_BASE_EOB_CTX_COUNT => {
                    Ok(self.coeff_base_eob_hf_4x4[ctx].as_mut_slice())
                }
                CoefficientCdfRowSelector::CoeffBaseEob {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_16X16_CTX,
                    ctx,
                } if ctx < COEFF_BASE_EOB_CTX_COUNT => {
                    Ok(self.coeff_base_eob_hf_16x16[ctx].as_mut_slice())
                }
                CoefficientCdfRowSelector::CoeffBr {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx,
                } if ctx < COEFF_BR_CTX_COUNT => Ok(self.coeff_br_hf[ctx].as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBase {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_4X4_CTX,
                    ctx,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } if ctx < COEFF_BASE_CTX_COUNT => Ok(self.coeff_base_hf_4x4[ctx].as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBase {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    tx_size: TX_SIZE_16X16_CTX,
                    ctx,
                    tcq_ctx: COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
                } if ctx < COEFF_BASE_CTX_COUNT => Ok(self.coeff_base_hf_16x16[ctx].as_mut_slice()),
                CoefficientCdfRowSelector::DcSign {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    plane_type: DC_SIGN_PLANE_TYPE_LUMA,
                    group: DC_SIGN_GROUP_VISIBLE,
                    ctx: DC_SIGN_CTX_NEUTRAL,
                } => Ok(self.dc_sign.as_mut_slice()),
                CoefficientCdfRowSelector::VTxbSkip {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx: CHROMA_V_TXB_SKIP_CTX_EOBU,
                } => Ok(self.v_txb_skip_eobu.as_mut_slice()),
                CoefficientCdfRowSelector::EobPt16 {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    eob_ctx: EOB_CTX_CHROMA,
                } => Ok(self.chroma_eob_pt_16.as_mut_slice()),
                CoefficientCdfRowSelector::CoeffBaseLfEobUv {
                    coeff_cdf_q_ctx: MINIMAL_COEFF_CDF_Q_CTX,
                    ctx: COEFF_BASE_LF_EOB_CTX_DC,
                } => Ok(self.coeff_base_lf_eob_uv.as_mut_slice()),
                _ => Err(Error::BlockSymbolTraceUnsupportedSelector { index }),
            },
        }
    }
}
