// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The general intra eob=2 multi-coefficient block composers (two-coeff, visible-AC,
//! two-nonzero) and IVF emitters.

use super::{
    CHROMA_SIGN_BIT_WIDTH, SKIP_FRAME_BASE_Q_IDX, SKIP_FRAME_COEFF_CDF_Q_CTX,
    V_TXB_SKIP_CTX_NEUTRAL,
};
use crate::block_symbol_trace::{
    BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace, encode_block_symbol_trace,
};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_64x64_luma_two_coeff_tokens, general_intra_64x64_luma_two_nonzero_base_tokens,
    general_intra_64x64_luma_visible_ac_tokens, luma_dc_sign_token,
};
use crate::error::{Error, Result};
use crate::partition_emission::emit_root_do_split_none;

/// Composes the general intra eob=2 multi-coefficient luma block trace: `do_split`, the mode
/// prefix, a coded luma block carrying a single nonzero AC coefficient (level 1) at scan index
/// 1 with a zero DC (`txb_skip == 0`, `eob_pt_1024 == 1`, AC `coeff_base_eob`, DC `coeff_base`,
/// then the AC `sign_bit` § 8.2.5 bypass), then skipped U and V. The minimal level-1 AC residual
/// is sub-visible, so the reconstruction is flat 128 (see `emit_minimal_intra_two_coeff_ivf`).
fn compose_general_intra_two_coeff_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_two_coeff_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(4)) // do_split + AC sign + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general two-coefficient block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general two-coefficient block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    // The AC `sign_bit` is a § 8.2.5 bypass literal (positive); the zero DC has no sign.
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 0));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose luma block carries a single nonzero **AC** coefficient (eob=2, U and V skipped).
///
/// This is the encoder's first multi-coefficient (`eob > 1`) frame, exercising the scan walk
/// and the non-EOB `coeff_base` pass. The single AC coefficient is the minimal **level 1**,
/// whose dequantized residual is sub-visible (it rounds to ~0), so the decoded frame is flat at
/// 128 — distinct from a skip frame only in the entropy stream, not yet the reconstruction. A
/// visibly non-flat (cosine) AC needs a larger magnitude, whose `Level[]`-derived DC
/// `coeff_base` context differs and is computed per AC level; that is a follow-up. The
/// cross-crate decode oracle (it validates the eob=2 stream and reconstructs the frame) lives in
/// `splot-cli`.
pub fn emit_minimal_intra_two_coeff_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_two_coeff_block_trace()?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// Composes the general intra eob=2 **visible** multi-coefficient luma block trace: `do_split`,
/// the mode prefix, a coded luma block carrying a single nonzero AC coefficient of level 4 at
/// scan index 1 with a zero DC, then the AC `sign_bit` § 8.2.5 bypass, then skipped U and V.
/// Unlike the minimal level-1 AC (which rounds back to flat 128), the level-4 AC dequantizes to
/// a residual that reconstructs a visibly non-flat (low-frequency cosine) luma plane.
fn compose_general_intra_visible_ac_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_visible_ac_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(4)) // do_split + AC sign + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general visible-AC block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general visible-AC block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 0));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose luma block carries a single nonzero **level-4 AC** coefficient (eob=2, U and V
/// skipped), reconstructing a **visibly non-flat** low-frequency cosine luma plane.
///
/// This is the encoder's first frame where a coefficient visibly shapes the reconstruction
/// (every prior frame was flat). It builds on `emit_minimal_intra_two_coeff_ivf` (the level-1
/// AC, sub-visible) by raising the AC to level 4 — the largest `coeff_base_eob` base level with
/// no `coeff_br` tail — so the dequantized residual survives rounding. The cross-crate decode
/// oracle lives in `splot-cli`.
pub fn emit_minimal_intra_visible_ac_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_visible_ac_block_trace()?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// The DC sign for the two-nonzero-coefficient block: **negative** (`dc_sign == 1`). A negative
/// DC makes the oracle genuinely exercise the AV2 § 5.20.7.27 reverse-scan sign order: with both
/// signs positive the block reconstructs identically under either sign order, so an ordering bug
/// would pass undetected. With the DC negative and the AC positive, only the spec-correct order
/// (AC `sign_bit` before DC `dc_sign`) reconstructs consistently.
const TWO_NONZERO_DC_NEGATIVE: bool = true;

/// Composes the general intra eob=2 **two-nonzero-coefficient** luma block trace: `do_split`,
/// the mode prefix, the coded luma base pass (a level-4 AC at scan index 1 and a level-1 DC at
/// scan index 0), then the sign pass in AV2 § 5.20.7.27 order `c = eob-1 .. 0` (reverse scan):
/// the AC `sign_bit` § 8.2.5 bypass (c=1) FIRST, then the DC `dc_sign` CDF symbol (c=0). U and V
/// are skipped. This is the first block where two coefficients are nonzero; the reconstruction is
/// the visible-AC vertical cosine superimposed on the negative DC offset.
fn compose_general_intra_two_nonzero_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_two_nonzero_base_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(5)) // do_split + AC sign + DC sign + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general two-nonzero block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general two-nonzero block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    // Sign pass, reverse scan (`c = eob-1 .. 0`): the AC `sign_bit` § 8.2.5 bypass (c=1) is
    // emitted FIRST, then the DC `dc_sign` CDF symbol (c=0).
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 0));
    trace.push(BlockSymbolToken::Coeff(luma_dc_sign_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        TWO_NONZERO_DC_NEGATIVE,
    )));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose luma block carries **two nonzero coefficients** — a positive level-4 AC at scan
/// index 1 and a **negative** level-1 DC at scan index 0 (eob=2, U and V skipped). This is the
/// encoder's first block with more than one nonzero coefficient; it exercises the AV2 § 5.20.7.27
/// reverse-scan sign pass (AC `sign_bit` before DC `dc_sign`). The reconstruction is the
/// visible-AC vertical cosine superimposed on the negative DC offset. The cross-crate decode
/// oracle lives in `splot-cli`.
pub fn emit_minimal_intra_two_nonzero_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_two_nonzero_block_trace()?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::block_symbol_trace::roundtrip_block_symbol_trace;

    #[test]
    fn composes_general_two_coeff_block_trace_in_order() {
        let trace = compose_general_intra_two_coeff_block_trace().unwrap();

        // do_split, 3 modes, 4 coded-luma (txb_skip, eob_pt, AC base_eob, DC base),
        // AC sign bypass, U skip, V skip = 11 tokens.
        assert_eq!(trace.len(), 11);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        // do_split, modes 0/0/0; luma txb_skip=0, eob_pt_1024=1 (eob 2), AC coeff_base_eob=0
        // (level 1), DC coeff_base=0 (level 0), AC sign=0; U/V txb_skip=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1]
        );
    }

    #[test]
    fn general_two_coeff_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_two_coeff_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1]);
        assert_eq!(proof.symbol_count(), 11);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_two_coeff_ivf_differs_from_skip_and_is_deterministic() {
        let two = emit_minimal_intra_two_coeff_ivf().unwrap();
        assert!(!two.is_empty());
        // A distinct entropy stream from the skip frame (it carries the eob=2 AC symbols).
        assert_ne!(two, super::super::emit_minimal_intra_skip_ivf().unwrap());
        assert_eq!(two, emit_minimal_intra_two_coeff_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&two);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn composes_general_visible_ac_block_trace_in_order() {
        let trace = compose_general_intra_visible_ac_block_trace().unwrap();

        // do_split, 3 modes, 4 coded-luma (txb_skip, eob_pt, AC base_eob, DC base),
        // AC sign bypass, U skip, V skip = 11 tokens.
        assert_eq!(trace.len(), 11);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        // do_split, modes 0/0/0; luma txb_skip=0, eob_pt_1024=1 (eob 2), AC coeff_base_eob=3
        // (level 4, the largest no-coeff_br base level), DC coeff_base=0 (level 0), AC sign=0;
        // U/V txb_skip=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 1, 3, 0, 0, 1, 1]
        );
    }

    #[test]
    fn general_visible_ac_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_visible_ac_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 1, 3, 0, 0, 1, 1]);
        assert_eq!(proof.symbol_count(), 11);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_visible_ac_ivf_differs_from_two_coeff_and_is_deterministic() {
        let visible = emit_minimal_intra_visible_ac_ivf().unwrap();
        assert!(!visible.is_empty());
        // A distinct entropy stream from the level-1 (sub-visible) eob=2 frame.
        assert_ne!(visible, emit_minimal_intra_two_coeff_ivf().unwrap());
        assert_eq!(visible, emit_minimal_intra_visible_ac_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&visible);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn composes_general_two_nonzero_block_trace_in_order() {
        let trace = compose_general_intra_two_nonzero_block_trace().unwrap();

        // do_split, 3 modes, 4 coded-luma base (txb_skip, eob_pt, AC base_eob, DC base),
        // AC sign bypass, DC dc_sign, U skip, V skip = 12 tokens.
        assert_eq!(trace.len(), 12);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        // Reverse-scan sign pass (§5.20.7.27 c=eob-1..0): the AC sign_bit bypass (c=1) comes
        // FIRST, then the DC dc_sign CDF symbol (c=0).
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        assert!(matches!(trace[9], BlockSymbolToken::Coeff(_)));
        // do_split, modes 0/0/0; luma txb_skip=0, eob_pt_1024=1, AC coeff_base_eob=3 (level 4),
        // DC coeff_base=1 (level 1); AC sign=0 (positive); DC dc_sign=1 (negative); U/V txb_skip=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 1, 3, 1, 0, 1, 1, 1]
        );
    }

    #[test]
    fn general_two_nonzero_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_two_nonzero_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(
            proof.decoded_symbols(),
            &[0, 0, 0, 0, 0, 1, 3, 1, 0, 1, 1, 1]
        );
        assert_eq!(proof.symbol_count(), 12);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_two_nonzero_ivf_differs_from_visible_ac_and_is_deterministic() {
        let two = emit_minimal_intra_two_nonzero_ivf().unwrap();
        assert!(!two.is_empty());
        // Distinct from the single-nonzero (visible-AC) frame: it adds the nonzero DC + its sign.
        assert_ne!(two, emit_minimal_intra_visible_ac_ivf().unwrap());
        assert_eq!(two, emit_minimal_intra_two_nonzero_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&two);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }
}
