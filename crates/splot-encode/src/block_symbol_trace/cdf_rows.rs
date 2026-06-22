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
    do_split_root: [i32; DO_SPLIT_CDF_ROW_LEN],
    do_square_split_root: [i32; DO_SQUARE_SPLIT_CDF_ROW_LEN],
    y_mode_set: [i32; Y_MODE_SET_CDF_ROW_LEN],
    y_mode_index_tile_origin: [i32; INTRA_MODE_CDF_ROW_LEN],
    uv_mode_non_directional: [i32; INTRA_MODE_CDF_ROW_LEN],
    luma_txb_skip: [i32; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip: [i32; TXB_SKIP_CDF_ROW_LEN],
    v_txb_skip: [i32; V_TXB_SKIP_CDF_ROW_LEN],
    luma_txb_skip_64x64: [i32; TXB_SKIP_CDF_ROW_LEN],
    luma_txb_skip_16x16: [i32; TXB_SKIP_CDF_ROW_LEN],
    u_txb_skip_32x32: [i32; TXB_SKIP_CDF_ROW_LEN],
    eob_pt_16: [i32; EOB_PT_16_CDF_ROW_LEN],
    eob_pt_1024: [i32; EOB_PT_1024_CDF_ROW_LEN],
    eob_pt_256: [i32; EOB_PT_256_CDF_ROW_LEN],
    eob_pt_1024_chroma: [i32; EOB_PT_1024_CDF_ROW_LEN],
    eob_extra: [i32; EOB_EXTRA_CDF_ROW_LEN],
    coeff_base_lf_eob_tx64: [i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    // The `TX_16X16` low-frequency `coeff_base_eob` DC row (general 16x16 intra DC
    // tokenizer), the `(TX_16X16, DC ctx 0)` cell of
    // `DEFAULT_COEFF_BASE_LF_EOB_CDF[q][txSz][ctx]`.
    coeff_base_lf_eob_tx16: [i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    intra_tx_type_set1_4x4: [i32; INTRA_TX_TYPE_SET1_CDF_ROW_LEN],
    sec_tx_type_intra_4x4: [i32; SEC_TX_TYPE_INTRA_CDF_ROW_LEN],
    // The 4x4 low-frequency `coeff_base_eob` bank, indexed by the § 8.3.2
    // `coeff_base_eob` context (`COEFF_BASE_LF_EOB_CTX_COUNT` contexts). Sizing the
    // full generated context dimension makes the 4x4 LF EOB tier hole-free.
    coeff_base_lf_eob_4x4: [[i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT],
    coeff_base_lf_eob_ac_tx64: [i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN],
    // The 4x4 low-frequency non-EOB `coeff_base` bank at the neutral TCQ context,
    // indexed by the § 8.3.2 `coeff_base` low-frequency context
    // (`COEFF_BASE_LF_CTX_COUNT` contexts). The general LF walk derives this context
    // from the running `Level[]` (`coeff_base_lf_luma_context`); sizing the full
    // generated context dimension makes the 4x4 LF base tier hole-free.
    coeff_base_lf_4x4: [[i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT],
    coeff_base_lf_dc_tx64: [i32; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_dc_tx64_visible_ac: [i32; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_ac_tx64_ctx9: [i32; COEFF_BASE_LF_CDF_ROW_LEN],
    coeff_base_lf_dc_tx64_2d: [i32; COEFF_BASE_LF_CDF_ROW_LEN],
    // The `coeff_br` low-frequency bank, indexed by the § 8.3.2 `coeff_br` context
    // (`COEFF_BR_LF_CTX_COUNT` contexts; `coeff_br` has no transform-size dimension).
    // The general LF walk derives this context from the running `Level[]`
    // (`coeff_br_lf_luma_context`); sizing the full generated context dimension makes
    // the LF `coeff_br` tier hole-free.
    coeff_br_lf: [[i32; COEFF_BR_LF_CDF_ROW_LEN]; COEFF_BR_LF_CTX_COUNT],
    // The 4x4 HIGH-frequency `coeff_base_eob` bank, indexed by the § 8.3.2
    // `coeff_base_eob` context (`COEFF_BASE_EOB_CTX_COUNT` contexts, 4-symbol rows),
    // from the generated `DEFAULT_COEFF_BASE_EOB_CDF[q][4x4][ctx]` (distinct from the
    // 6-symbol LF `coeff_base_lf_eob_4x4` bank). The eob-11 HF EOB coefficient reaches
    // `coeff_base_eob_ctx(10) = 3`; sizing the full ctx dimension keeps the HF EOB
    // tier hole-free.
    coeff_base_eob_hf_4x4: [[i32; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT],
    // The HIGH-frequency `coeff_br` bank, indexed by the § 8.3.2 HF `coeff_br` context
    // (`COEFF_BR_CTX_COUNT`, 7; no transform-size dimension), from the generated
    // `DEFAULT_COEFF_BR_CDF[q][ctx]` (distinct from the 14-context LF `coeff_br_lf`
    // bank). The eob-11 HF EOB `coeff_br` reaches the constant ctx 0; sizing the full
    // ctx-7 dimension keeps the tier hole-free for the later non-EOB HF sub-brick.
    coeff_br_hf: [[i32; COEFF_BR_CDF_ROW_LEN]; COEFF_BR_CTX_COUNT],
    // The 4x4 HIGH-frequency non-EOB `coeff_base` bank at the neutral TCQ context,
    // indexed by the § 8.3.2 HF `coeff_base` context (`COEFF_BASE_CTX_COUNT`, 20;
    // 4-symbol rows), from the generated `DEFAULT_COEFF_BASE_CDF[q][4x4][ctx][tcq]`
    // (distinct from the 6-symbol LF `coeff_base_lf_4x4` bank). The general HF walk
    // derives this context from the running `Level[]` (`coeff_base_hf_luma_context`);
    // for 4x4 2D the reachable contexts are roughly 0..9, but sizing the full ctx-20
    // dimension keeps the HF non-EOB base tier hole-free.
    coeff_base_hf_4x4: [[i32; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT],
    // The TX_16X16 LF `coeff_base_eob` / non-EOB `coeff_base` and HF `coeff_base_eob` /
    // non-EOB `coeff_base` banks (`ENC-COEFF-TOKENIZE-16X16-BASE`), each indexed by the
    // same full § 8.3.2 context dimension as the 4x4 banks but sourced from the
    // `TX_SIZE_16X16_CTX` slice of the generated tables. The general 16x16 base pass
    // routes through these so every reached context is hole-free.
    coeff_base_lf_eob_16x16: [[i32; COEFF_BASE_LF_EOB_CDF_ROW_LEN]; COEFF_BASE_LF_EOB_CTX_COUNT],
    coeff_base_lf_16x16: [[i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT],
    coeff_base_eob_hf_16x16: [[i32; COEFF_BASE_EOB_CDF_ROW_LEN]; COEFF_BASE_EOB_CTX_COUNT],
    coeff_base_hf_16x16: [[i32; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT],
    dc_sign: [i32; DC_SIGN_CDF_ROW_LEN],
    v_txb_skip_eobu: [i32; V_TXB_SKIP_CDF_ROW_LEN],
    chroma_eob_pt_16: [i32; EOB_PT_16_CDF_ROW_LEN],
    coeff_base_lf_eob_uv: [i32; COEFF_BASE_LF_EOB_UV_CDF_ROW_LEN],
}

/// Builds the low-frequency non-EOB `coeff_base` bank at the neutral TCQ context for
/// the given `tx_size` by extracting the `tcq = COEFF_BASE_LF_TCQ_CTX_NEUTRAL` row at
/// every § 8.3.2 context from the generated `DEFAULT_COEFF_BASE_LF_CDF[q][txSz][ctx][tcq]`
/// table, at the minimal coefficient-CDF q-context. Total and panic-free: every index
/// is a const within the table dimensions.
const fn coeff_base_lf_bank(
    tx_size: usize,
) -> [[i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT] {
    let mut bank = [[0i32; COEFF_BASE_LF_CDF_ROW_LEN]; COEFF_BASE_LF_CTX_COUNT];
    let mut ctx = 0;
    while ctx < COEFF_BASE_LF_CTX_COUNT {
        bank[ctx] = DEFAULT_COEFF_BASE_LF_CDF[MINIMAL_COEFF_CDF_Q_CTX][tx_size][ctx]
            [COEFF_BASE_LF_TCQ_CTX_NEUTRAL];
        ctx += 1;
    }
    bank
}

/// Builds the HIGH-frequency non-EOB `coeff_base` bank at the neutral TCQ context for
/// the given `tx_size` by extracting the `tcq = COEFF_BASE_LF_TCQ_CTX_NEUTRAL` row at
/// every § 8.3.2 HF context from the generated `DEFAULT_COEFF_BASE_CDF[q][txSz][ctx][tcq]`
/// table (4-symbol rows), at the minimal coefficient-CDF q-context. Total and
/// panic-free: every index is a const within the table dimensions.
const fn coeff_base_hf_bank(
    tx_size: usize,
) -> [[i32; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT] {
    let mut bank = [[0i32; COEFF_BASE_CDF_ROW_LEN]; COEFF_BASE_CTX_COUNT];
    let mut ctx = 0;
    while ctx < COEFF_BASE_CTX_COUNT {
        bank[ctx] = DEFAULT_COEFF_BASE_CDF[MINIMAL_COEFF_CDF_Q_CTX][tx_size][ctx]
            [COEFF_BASE_LF_TCQ_CTX_NEUTRAL];
        ctx += 1;
    }
    bank
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
            coeff_base_lf_4x4: coeff_base_lf_bank(TX_SIZE_4X4_CTX),
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
            coeff_base_hf_4x4: coeff_base_hf_bank(TX_SIZE_4X4_CTX),
            coeff_base_lf_eob_16x16: DEFAULT_COEFF_BASE_LF_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_16X16_CTX],
            coeff_base_lf_16x16: coeff_base_lf_bank(TX_SIZE_16X16_CTX),
            coeff_base_eob_hf_16x16: DEFAULT_COEFF_BASE_EOB_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [TX_SIZE_16X16_CTX],
            coeff_base_hf_16x16: coeff_base_hf_bank(TX_SIZE_16X16_CTX),
            dc_sign: DEFAULT_DC_SIGN_CDF[MINIMAL_COEFF_CDF_Q_CTX][DC_SIGN_PLANE_TYPE_LUMA]
                [DC_SIGN_GROUP_VISIBLE][DC_SIGN_CTX_NEUTRAL],
            v_txb_skip_eobu: DEFAULT_V_TXB_SKIP_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [CHROMA_V_TXB_SKIP_CTX_EOBU],
            chroma_eob_pt_16: DEFAULT_EOB_PT_16_CDF[MINIMAL_COEFF_CDF_Q_CTX][EOB_CTX_CHROMA],
            coeff_base_lf_eob_uv: DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[MINIMAL_COEFF_CDF_Q_CTX]
                [COEFF_BASE_LF_EOB_CTX_DC],
        }
    }

    pub(super) fn row_mut(&mut self, token: BlockSymbolToken, index: usize) -> Result<&mut [i32]> {
        match token {
            // Bypass literals carry no CDF row; `roundtrip_block_symbol_trace`
            // dispatches them before ever calling `row_mut`, so this arm is
            // unreachable in practice.
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
