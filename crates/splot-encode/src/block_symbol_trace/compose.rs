// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The minimal-tier block-symbol trace composers: the AV2 § 5.20.5.3 mode-info
//! prefix, the per-plane § 5.20.7.27 all-zero blocks, the coded single-DC blocks
//! (luma, base-range, chroma), and the eob=2 multi-coefficient block (alone, or
//! with the § 5.20.8.2 `intra_tx_type` / `sec_tx_type` IST symbols). Split out of
//! `block_symbol_trace` to keep each file under the 1000-line source budget.

use super::*;

/// Composes the ordered AV2 § 5.20.5.3 intra-block mode-info prefix
/// (`y_mode_set`, `y_mode_index`, `uv_mode`) for the current minimal DC subset.
pub(crate) fn compose_minimal_intra_dc_block_mode_trace() -> Result<Vec<IntraModeToken>> {
    let luma = emit_minimal_dc_luma_intra_mode()?;
    let uv = emit_minimal_dc_chroma_uv_mode()?;

    let total = luma.tokens().len().checked_add(uv.tokens().len()).ok_or(
        Error::IntraModeEmissionAllocationFailed {
            context: "intra block mode trace length",
        },
    )?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::IntraModeEmissionAllocationFailed {
            context: "intra block mode trace",
        })?;
    trace.extend_from_slice(luma.tokens());
    trace.extend_from_slice(uv.tokens());
    Ok(trace)
}

/// Composes the ordered minimal intra DC all-zero block trace: the AV2 § 5.20.5.3
/// mode-info prefix (`y_mode_set`, `y_mode_index`, `uv_mode`) followed by the
/// first `residual()` symbol, the luma `txb_skip` (§ 5.20.7.27 `all_zero`).
pub(crate) fn compose_minimal_intra_dc_all_zero_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let total = modes
        .len()
        .checked_add(1)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "all-zero block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "all-zero block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(luma_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    Ok(trace)
}

/// Composes the complete ordered minimal intra DC all-zero block trace: the AV2
/// § 5.20.5.3 mode-info prefix, then the per-plane § 5.20.7.27 `all_zero`
/// (`txb_skip`) symbols for luma, U, and V (each `1` for an all-zero block),
/// read in `residual()` plane order Y, U, V.
pub(crate) fn compose_minimal_intra_dc_complete_all_zero_block_trace()
-> Result<Vec<BlockSymbolToken>> {
    let mut trace = compose_minimal_intra_dc_all_zero_block_trace()?;
    trace
        .try_reserve_exact(2)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "complete all-zero block trace",
        })?;
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Composes a coded intra DC block trace for a single nonzero luma DC coefficient
/// of unsigned `magnitude` and the given sign: the AV2 § 5.20.5.3 mode-info
/// prefix, then the luma `residual()` coded coefficient tokens (§ 5.20.7.27,
/// including `coeff_br` for `magnitude > LF_NUM_BASE_LEVELS`), then the all-zero U
/// and V `txb_skip` symbols, in `residual()` plane order Y, U, V.
fn compose_coded_dc_block_trace(magnitude: u32, negative: bool) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = luma_dc_coded_tokens(MINIMAL_COEFF_CDF_Q_CTX, magnitude, negative)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(2))
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "coded block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Composes the minimal ordered intra DC *coded* block trace: the mode-info
/// prefix, then the luma `residual()` for a single coded DC coefficient
/// (`txb_skip == 0`, `eob_pt_16`, `coeff_base_eob`, `dc_sign` per § 5.20.7.27),
/// then the all-zero U and V `txb_skip` symbols.
///
/// The luma block carries one nonzero DC coefficient of value `+1`; the chroma
/// planes are all-zero. This is the minimal *non-degenerate* (actually coded)
/// intra block symbol sequence.
pub(crate) fn compose_minimal_intra_dc_coded_block_trace() -> Result<Vec<BlockSymbolToken>> {
    compose_coded_dc_block_trace(MINIMAL_CODED_DC_MAGNITUDE, MINIMAL_CODED_DC_NEGATIVE)
}

/// Composes the minimal ordered intra DC coded *base-range* block trace: like
/// [`compose_minimal_intra_dc_coded_block_trace`] but with a luma DC magnitude in
/// the § 5.20.7.27 base-range tier, so the luma `residual()` additionally emits a
/// `coeff_br` symbol after `coeff_base_eob`.
///
/// The luma block carries one nonzero DC coefficient of value `+6` (level 5 base
/// plus `coeff_br = 1`); the chroma planes are all-zero.
pub(crate) fn compose_minimal_intra_dc_br_block_trace() -> Result<Vec<BlockSymbolToken>> {
    compose_coded_dc_block_trace(MINIMAL_BR_DC_MAGNITUDE, MINIMAL_BR_DC_NEGATIVE)
}

/// Composes the minimal ordered intra DC coded *chroma* block trace: the AV2
/// § 5.20.5.3 mode-info prefix, then the coded luma `residual()`, then the coded U
/// `residual()` — `txb_skip == 0`, chroma `eob_pt_16`, chroma `coeff_base_eob`
/// (CDF), then the U DC `sign_bit` as a § 8.2.5 `L(1)` bypass literal (a chroma
/// sign is not a `dc_sign` CDF symbol) — then the all-zero V `txb_skip` at the
/// § 8.3.2 V context 6 (`EobU != 0` once U is coded), in `residual()` plane order
/// Y, U, V.
///
/// The luma and U planes each carry one nonzero DC coefficient of value `+1`; the
/// V plane is all-zero. This is the minimal block whose chroma plane is coded.
pub(crate) fn compose_minimal_intra_dc_coded_chroma_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = luma_dc_coded_tokens(
        MINIMAL_COEFF_CDF_Q_CTX,
        MINIMAL_CODED_DC_MAGNITUDE,
        MINIMAL_CODED_DC_NEGATIVE,
    )?;
    let u_coeffs =
        chroma_u_dc_coded_coeff_tokens(MINIMAL_COEFF_CDF_Q_CTX, MINIMAL_CODED_CHROMA_DC_MAGNITUDE)?;
    // mode prefix + luma + U coefficients + the U `sign_bit` bypass + the V
    // all-zero `txb_skip`.
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(u_coeffs.len()))
        .and_then(|n| n.checked_add(2))
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "coded chroma block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "coded chroma block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.extend(u_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    // The U DC sign is a § 5.20.7.27 `sign_bit L(1)` bypass literal, not a CDF
    // `dc_sign` (that path is the luma DC / directional luma axis signs).
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        MINIMAL_CODED_CHROMA_DC_NEGATIVE as u32,
    ));
    // The U plane is coded (`EobU != 0`), so the V `txb_skip` uses § 8.3.2 context
    // 6 (the `+6` EobU term), not the all-zero-U neutral context 0.
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        CHROMA_V_TXB_SKIP_CTX_EOBU,
    )));
    Ok(trace)
}

/// Composes the minimal eob=2 multi-coefficient luma block trace: the AV2
/// § 5.20.5.3 mode-info prefix, then the coded luma `residual()` for a block with
/// two scan positions — one nonzero AC coefficient (level 1) at scan index 1 and a
/// zero DC at scan index 0 — then the all-zero U and V `txb_skip`.
///
/// Per § 5.20.7.27 the residual is `all_zero=0`, `eob_pt_16=1` (eob 2), then the
/// base pass over `c = eob-1..0`: the AC `coeff_base_eob` at context 1 (the
/// EOB-position coefficient, level 1, at scan index 1 = raster position 4 = row 1
/// col 0) and the DC `coeff_base` at the § 8.3.2 low-frequency context derived from
/// the AC's `Level[]` (the AC is the DC's significant neighbour, so the context is
/// 1; derived via `coeff_base_lf_luma_context`, not hard-coded). The sign pass then
/// reads the AC `sign_bit` (an § 8.2.5 bypass literal — pos (1,0) is neither the
/// luma DC nor a directional axis under TX_CLASS_2D); the DC is zero, so it carries
/// no sign. The ten-token trace is `[0,0,0, 0, 1, 0, 0, 0, 1, 1]`.
///
/// Transform-type scope: § 5.20.7.27 calls `transform_type()` between `eob_pt_16`
/// and the base pass, and for `eob > 1` the `transform_type()` `eob == 1` shortcut
/// no longer infers `DCT_DCT`. This trace therefore assumes a transform-set
/// configuration where `transform_type()` reads NO `intra_tx_type` symbol — the
/// DCT-only set (`get_tx_set` returns `TX_SET_DCTONLY`) or `reduced_tx_set == 2` for
/// intra (§ 5.20.7.27, the `!(reduced_tx_set == 2 && is_inter == 0)` guard) — AND
/// `enable_intra_ist == 0`, since § 5.20.7.29 (line 16603) otherwise reads a
/// `sec_tx_type` (intra secondary transform) symbol before the base pass for an
/// `eob > 1` DCT_DCT block. Both are consistent with the block's plain DCT_DCT
/// transform; the general `eob > 1` `intra_tx_type` / `sec_tx_type` signaling
/// (`set > 0` and `reduced_tx_set != 2`, or `enable_intra_ist`) is a later brick.
///
/// This is the first multi-coefficient block trace. The § 8.2 roundtrip proves the
/// symbols are self-consistent; conformance of the data-dependent `coeff_base`
/// context against a real decoder is established at the packet milestone (AVM
/// cross-check).
pub(crate) fn compose_minimal_intra_two_coeff_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    // Derive the AC's raster position from the AV2 2D scan order (scan index 1 maps
    // to raster position 4 in the 4x4 order, not 1), then derive the DC's § 8.3.2
    // coeff_base low-frequency context from the AC's Level[] (the AC of level 1 is
    // the DC's significant neighbour).
    let mut scan = [0u16; TX_4X4_WIDTH * TX_4X4_HEIGHT];
    coefficient_scan_order(TX_4X4_WIDTH, TX_4X4_HEIGHT, TransformClass::TwoD, &mut scan).map_err(
        |_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient scan order",
        },
    )?;
    let ac_raster_pos = scan[EOB2_AC_SCAN_INDEX] as usize;
    let mut level = [0u32; TX_4X4_WIDTH * TX_4X4_HEIGHT];
    level[ac_raster_pos] = EOB2_AC_LEVEL as u32;
    let dc_ctx = coeff_base_lf_luma_context(
        0,
        TX_4X4_BWL,
        TX_4X4_WIDTH,
        TX_4X4_HEIGHT,
        TX_CLASS_2D,
        0,
        &level,
    );
    debug_assert_eq!(dc_ctx, COEFF_BASE_LF_CTX_EOB2_DC);
    let total = modes
        .len()
        .checked_add(7) // all_zero + eob_pt + AC base_eob + DC base + AC sign + U + V
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient block trace",
        })?;
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(coded_luma_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(eob_pt_16_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        EOB_CTX_LUMA_INTRA,
        EOB_PT_16_SYMBOL_EOB2,
    )));
    // Base pass (c = eob-1..0): the AC `coeff_base_eob` then the DC `coeff_base`.
    trace.push(BlockSymbolToken::Coeff(coeff_base_lf_eob_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        COEFF_BASE_LF_EOB_CTX_EOB2_AC,
        EOB2_AC_LEVEL,
    )));
    trace.push(BlockSymbolToken::Coeff(coeff_base_lf_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        dc_ctx,
        COEFF_BASE_LF_TCQ_CTX_NEUTRAL,
        EOB2_DC_LEVEL,
    )));
    // Sign pass: the AC `sign_bit` (a §8.2.5 bypass literal); the zero DC has no sign.
    trace.push(BlockSymbolToken::bypass(1, EOB2_AC_NEGATIVE as u32));
    trace.push(BlockSymbolToken::Coeff(chroma_u_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
    )));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        MINIMAL_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Composes the minimal eob=2 multi-coefficient luma block trace WITH the
/// §5.20.8.2 `intra_tx_type` transform-type symbol, for the default-`reduced_tx_set`
/// `TX_SET_INTRA_1` configuration (removing the
/// [`compose_minimal_intra_two_coeff_block_trace`] `reduced_tx_set == 2` scope).
/// `transform_type()` is read right after `eob_pt_16` (§5.20.7.27 line 15474),
/// before the base pass; the 4x4 `DC_PRED` symbol is 0 (`DCT_DCT`). The eleven-token
/// trace is the eob=2 trace with that symbol inserted after `eob_pt_16`:
/// `[0,0,0, 0, 1, 0, 0, 0, 0, 1, 1]`. It still assumes `enable_intra_ist == 0` (no
/// `sec_tx_type`); that signaling is a later brick.
pub(crate) fn compose_minimal_intra_two_coeff_block_trace_with_tx_type()
-> Result<Vec<BlockSymbolToken>> {
    let base = compose_minimal_intra_two_coeff_block_trace()?;
    // Derive the insertion point from the `eob_pt_16` token kind so it tracks any
    // growth of the base trace, falling back to the known `EOB_PT_16_TRACE_INDEX`.
    let split = base
        .iter()
        .position(|token| {
            matches!(token, BlockSymbolToken::Coeff(coeff)
                if matches!(coeff.syntax(), CoefficientTokenSyntax::EobPt16))
        })
        .unwrap_or(EOB_PT_16_TRACE_INDEX)
        + 1;
    let total = base
        .len()
        .checked_add(1)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient tx-type block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient tx-type block trace",
        })?;
    trace.extend_from_slice(&base[..split]);
    trace.push(BlockSymbolToken::Coeff(intra_tx_type_set1_token(
        INTRA_TX_TYPE_SET1_TX_SIZE_SQR_4X4,
        INTRA_TX_TYPE_DCT_DCT_SYMBOL,
    )));
    trace.extend_from_slice(&base[split..]);
    Ok(trace)
}

/// Composes the eob=2 trace with BOTH §5.20.8.2 transform-type symbols —
/// `intra_tx_type` AND `sec_tx_type` (the IST secondary transform) — for the
/// `enable_intra_ist == 1` configuration. `sec_tx_type` (§5.20.8.2 line 16613) is read
/// right after `intra_tx_type` (line 16529), before the base pass; for this 4x4 DCT_DCT
/// `DC_PRED` eob=2 block the IST condition holds (`eob 2 != 1 && !Lossless && TxType ==
/// DCT_DCT && YMode != PAETH && eob 2 <= eobLim = IST_4X4_HEIGHT = 8`), and symbol 0 is
/// `sec_tx_type = 0` (IST off, no `most_probable_stx_set`). The twelve-token trace is
/// the tx-type trace with that symbol inserted after `intra_tx_type`:
/// `[0,0,0, 0, 1, 0, 0, 0, 0, 0, 1, 1]`.
pub(crate) fn compose_minimal_intra_two_coeff_block_trace_with_ist() -> Result<Vec<BlockSymbolToken>>
{
    let base = compose_minimal_intra_two_coeff_block_trace_with_tx_type()?;
    // `sec_tx_type` is read right after `intra_tx_type`; derive the insertion point
    // from the `intra_tx_type` token kind, falling back to just after `eob_pt_16`.
    let split = base
        .iter()
        .position(|token| {
            matches!(token, BlockSymbolToken::Coeff(coeff)
                if matches!(coeff.syntax(), CoefficientTokenSyntax::IntraTxType))
        })
        .unwrap_or(EOB_PT_16_TRACE_INDEX + 1)
        + 1;
    let total = base
        .len()
        .checked_add(1)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient IST block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "two-coefficient IST block trace",
        })?;
    trace.extend_from_slice(&base[..split]);
    trace.push(BlockSymbolToken::Coeff(sec_tx_type_intra_token(
        SEC_TX_TYPE_INTRA_TX_SIZE_SQR_4X4,
        SEC_TX_TYPE_IST_OFF_SYMBOL,
    )));
    trace.extend_from_slice(&base[split..]);
    Ok(trace)
}
