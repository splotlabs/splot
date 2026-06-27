// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The general intra multi-coefficient block composers (eob=2 two-coeff, visible-AC,
//! two-nonzero; eob=3) and IVF emitters.

use super::{CHROMA_SIGN_BIT_WIDTH, SKIP_FRAME_COEFF_CDF_Q_CTX, V_TXB_SKIP_CTX_NEUTRAL};
use crate::block_symbol_trace::{BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_64x64_luma_2d_base_tokens, general_intra_64x64_luma_eob3_base_tokens,
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
    super::emit_minimal_intra_ivf(trace)
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
    super::emit_minimal_intra_ivf(trace)
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
    super::emit_minimal_intra_ivf(trace)
}

/// Composes the general intra **eob=3** luma block trace: `do_split`, the mode prefix, the
/// coded luma base pass (a single nonzero level-4 AC at scan index 2 — raster 1, the horizontal
/// frequency-1 position — with scan indices 1 and 0 zero), then the single AC `sign_bit` § 8.2.5
/// bypass, then skipped U and V. This is the first eob>2 block: it exercises the `eob_extra` CDF
/// symbol (`eob_pt_1024 == 2`, `eob_extra == 0` -> eob 3). The level-4 AC at the horizontal
/// frequency reconstructs a horizontal low-frequency cosine (the transpose of the visible-AC
/// vertical cosine).
fn compose_general_intra_eob3_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_eob3_base_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(4)) // do_split + AC sign + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general eob=3 block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general eob=3 block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    // The single AC (scan index 2) `sign_bit` § 8.2.5 bypass — the only nonzero coefficient,
    // positive. The reverse-scan sign pass has nothing else to emit (scan 1 and the DC are zero).
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
/// frame whose luma block has **eob=3** — a single nonzero level-4 AC at scan index 2 (the
/// horizontal frequency-1 position), U and V skipped.
///
/// This is the encoder's first frame with `eob > 2`: it exercises the `eob_extra` CDF symbol
/// (the gateway to arbitrary-length blocks). The level-4 AC reconstructs a horizontal
/// low-frequency cosine. The cross-crate decode oracle lives in `splot-cli`.
pub fn emit_minimal_intra_eob3_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_eob3_block_trace()?;
    super::emit_minimal_intra_ivf(trace)
}

/// The scan-1 (vertical) AC sign for the 2-D block: NEGATIVE. With the scan-2 (horizontal) AC
/// positive, the two `sign_bit` bypasses carry different values, so only the spec-correct
/// reverse-scan order (scan 2 before scan 1) reconstructs consistently — the oracle proves it.
const COEFF_2D_SCAN1_NEGATIVE: bool = true;

/// Composes the general intra eob=3 **2-D** luma block trace: `do_split`, the mode prefix, the
/// coded luma base pass (two nonzero level-4 ACs — scan 1 vertical + scan 2 horizontal — with a
/// zero DC), then the two AC `sign_bit` § 8.2.5 bypasses in reverse-scan order (scan 2 positive,
/// then scan 1 negative), then skipped U and V. This is the first block whose reconstruction
/// varies in both dimensions: the horizontal and vertical low-frequency cosines superimposed.
fn compose_general_intra_2d_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_2d_base_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(5)) // do_split + 2 AC signs + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general 2-D block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general 2-D block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    // Sign pass, reverse scan `c = 2,1`: the scan-2 AC `sign_bit` (positive) precedes the scan-1
    // AC `sign_bit` (negative). Both are § 8.2.5 bypass literals (non-DC, non-axis under 2D).
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 0));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        COEFF_2D_SCAN1_NEGATIVE as u32,
    ));
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
/// frame whose luma block has **eob=3 with two nonzero level-4 ACs** — a positive horizontal AC
/// (scan 2) and a negative vertical AC (scan 1), U and V skipped.
///
/// This is the encoder's first frame whose reconstruction varies in **both** dimensions: the
/// horizontal and vertical low-frequency cosines superimposed (with opposite signs). The
/// cross-crate decode oracle lives in `splot-cli`.
pub fn emit_minimal_intra_2d_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_2d_block_trace()?;
    super::emit_minimal_intra_ivf(trace)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::block_symbol_trace::roundtrip_block_symbol_trace;

    #[test]
    fn composes_general_2d_block_trace_in_order() {
        let trace = compose_general_intra_2d_block_trace().unwrap();

        // do_split, 3 modes, 6 coded-luma base (txb_skip, eob_pt, eob_extra, scan2 base_eob,
        // scan1 base, DC base), 2 AC sign bypasses, U skip, V skip = 14 tokens.
        assert_eq!(trace.len(), 14);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        // Reverse-scan sign pass: scan-2 sign_bit (positive) then scan-1 sign_bit (negative).
        assert!(matches!(
            trace[10],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        assert!(matches!(
            trace[11],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        // do_split, modes 0/0/0; txb_skip=0, eob_pt_1024=2, eob_extra=0, scan2 coeff_base_eob=3
        // (level 4), scan1 coeff_base=4 (level 4), DC coeff_base=0; scan2 sign=0, scan1 sign=1;
        // U/V txb_skip=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 2, 0, 3, 4, 0, 0, 1, 1, 1]
        );
    }

    #[test]
    fn general_2d_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_2d_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(
            proof.decoded_symbols(),
            &[0, 0, 0, 0, 0, 2, 0, 3, 4, 0, 0, 1, 1, 1]
        );
        assert_eq!(proof.symbol_count(), 14);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_2d_ivf_differs_from_eob3_and_is_deterministic() {
        let two_d = emit_minimal_intra_2d_ivf().unwrap();
        assert!(!two_d.is_empty());
        // Distinct from the single-AC eob=3 frame: it adds the scan-1 nonzero AC + its sign.
        assert_ne!(two_d, emit_minimal_intra_eob3_ivf().unwrap());
        assert_eq!(two_d, emit_minimal_intra_2d_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&two_d);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn composes_general_eob3_block_trace_in_order() {
        let trace = compose_general_intra_eob3_block_trace().unwrap();

        // do_split, 3 modes, 6 coded-luma base (txb_skip, eob_pt, eob_extra, AC base_eob,
        // scan1 base, DC base), AC sign bypass, U skip, V skip = 13 tokens.
        assert_eq!(trace.len(), 13);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        // The single AC sign_bit bypass follows the base pass (scan 1 and DC are zero).
        assert!(matches!(
            trace[10],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        // do_split, modes 0/0/0; luma txb_skip=0, eob_pt_1024=2 (eob 3), eob_extra=0, AC
        // coeff_base_eob=3 (level 4), scan1 coeff_base=0, DC coeff_base=0, AC sign=0; U/V=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 2, 0, 3, 0, 0, 0, 1, 1]
        );
    }

    #[test]
    fn general_eob3_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_eob3_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(
            proof.decoded_symbols(),
            &[0, 0, 0, 0, 0, 2, 0, 3, 0, 0, 0, 1, 1]
        );
        assert_eq!(proof.symbol_count(), 13);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_eob3_ivf_differs_from_visible_ac_and_is_deterministic() {
        let eob3 = emit_minimal_intra_eob3_ivf().unwrap();
        assert!(!eob3.is_empty());
        // Distinct from the eob=2 visible-AC frame: it carries the eob_extra symbol + a
        // horizontal (not vertical) AC.
        assert_ne!(eob3, emit_minimal_intra_visible_ac_ivf().unwrap());
        assert_eq!(eob3, emit_minimal_intra_eob3_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&eob3);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }

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
