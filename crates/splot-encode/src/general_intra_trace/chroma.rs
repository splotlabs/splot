// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The general intra coded-chroma (U, V) and all-planes-coded block composers and IVF emitters.

use super::coded_dc::CODED_LUMA_DC_MAGNITUDE;
use super::{CHROMA_SIGN_BIT_WIDTH, SKIP_FRAME_COEFF_CDF_Q_CTX, V_TXB_SKIP_CTX_NEUTRAL};
use crate::block_symbol_trace::{BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_32x32_chroma_u_dc_coded_tokens, general_intra_32x32_chroma_v_dc_coded_tokens,
    general_intra_64x64_luma_all_zero_token, general_intra_64x64_luma_dc_coded_tokens,
};
use crate::error::{Error, Result};
use crate::partition_emission::emit_root_do_split_none;

/// The unsigned chroma U DC magnitude the coded-chroma frame emits: `4` (`coeff_base_eob` 3,
/// level 4 — the base tier, no `coeff_br`/golomb). Negative, it dequantizes to a flat U
/// reconstruction below `128`.
const CODED_CHROMA_U_DC_MAGNITUDE: u32 = 4;

/// The § 8.3.2 V `txb_skip` context when the U plane is coded (`EobU != 0`): `6`.
const V_TXB_SKIP_CTX_EOBU: usize = 6;

/// Composes the general intra coded-*chroma* block trace: `do_split`, the mode prefix, a
/// **skipped** luma plane, then a single coded U DC coefficient (`txb_skip == 0`,
/// `eob_pt == 0`, `coeff_base_eob`, then the U DC `sign_bit` § 8.2.5 bypass literal) at the
/// general `TX_32X32` chroma contexts, then a skipped V plane whose `txb_skip` uses the
/// `EobU != 0` context `6`. The decoded frame isolates chroma residual: luma stays flat 128,
/// U carries the dequantized residual, V stays flat 128.
fn compose_general_intra_coded_chroma_u_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let u_coeffs =
        general_intra_32x32_chroma_u_dc_coded_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX, magnitude)?;
    let total = modes
        .len()
        .checked_add(u_coeffs.len())
        .and_then(|n| n.checked_add(4)) // do_split + luma skip + U sign + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_64x64_luma_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.extend(u_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_EOBU,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose **chroma U** block carries a single negative coded DC coefficient (luma and V
/// skipped).
///
/// This proves chroma residual reconstruction: decoding reconstructs a flat luma plane of
/// `128` (skipped), a flat U plane below `128` (the dequantized negative chroma DC), and a
/// flat V plane of `128` (skipped). The cross-crate decode oracle lives in `splot-cli`.
///
/// # Errors
///
/// Returns an error if composing the block-symbol trace fails (token construction
/// or trace allocation), if entropy-coding the trace into tile data fails, or if
/// the AV2 IVF stream cannot be assembled by `splot-core`.
pub fn emit_minimal_intra_coded_chroma_ivf() -> Result<Vec<u8>> {
    let trace =
        compose_general_intra_coded_chroma_u_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)?;
    super::emit_minimal_intra_ivf(&trace)
}

/// Composes the general intra coded-*chroma-V* block trace: `do_split`, the mode prefix, a
/// **skipped** luma plane, a **skipped** U plane, then a single coded V DC coefficient
/// (`VTxbSkip == 0`, `eob_pt == 0`, `coeff_base_eob`, then the V DC `sign_bit` § 8.2.5 bypass
/// literal) at the general `TX_32X32` chroma contexts. The V `txb_skip` uses the neutral
/// context `0` (`EobU == 0`, since U is skipped). The decoded frame isolates V residual: luma
/// and U stay flat 128, V carries the dequantized residual.
fn compose_general_intra_coded_chroma_v_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let v_coeffs = general_intra_32x32_chroma_v_dc_coded_tokens(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        magnitude,
        V_TXB_SKIP_CTX_NEUTRAL,
    )?;
    let total = modes
        .len()
        .checked_add(v_coeffs.len())
        .and_then(|n| n.checked_add(4)) // do_split + luma skip + U skip + V sign
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma V block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma V block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_64x64_luma_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.extend(v_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose **chroma V** block carries a single negative coded DC coefficient (luma and U
/// skipped).
///
/// The V-plane counterpart of [`emit_minimal_intra_coded_chroma_ivf`]: decoding reconstructs a
/// flat luma plane of `128` (skipped), a flat U plane of `128` (skipped), and a flat V plane
/// below `128` (the dequantized negative chroma DC). With the U and V coded frames this
/// completes the per-plane coded-residual set. The cross-crate decode oracle lives in
/// `splot-cli`.
///
/// # Errors
///
/// Returns an error if composing the block-symbol trace fails (token construction
/// or trace allocation), if entropy-coding the trace into tile data fails, or if
/// the AV2 IVF stream cannot be assembled by `splot-core`.
pub fn emit_minimal_intra_coded_chroma_v_ivf() -> Result<Vec<u8>> {
    let trace =
        compose_general_intra_coded_chroma_v_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)?;
    super::emit_minimal_intra_ivf(&trace)
}

/// Composes the general intra all-planes-coded block trace: `do_split`, the mode prefix, then
/// a single coded DC coefficient on **each** of luma (`TX_64X64`, CDF `dc_sign`), U, and V
/// (`TX_32X32`, `sign_bit` § 8.2.5 bypass), in `residual()` order Y, U, V. Because the U plane
/// is coded (`EobU != 0`), the V `txb_skip` uses the § 8.3.2 context `6`. This mirrors the q80
/// fixture's all-three-planes-coded structure with sub-golomb magnitudes.
fn compose_general_intra_all_planes_coded_block_trace(
    luma_magnitude: u32,
    chroma_magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_dc_coded_tokens(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        luma_magnitude,
        negative,
    )?;
    let u_coeffs =
        general_intra_32x32_chroma_u_dc_coded_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX, chroma_magnitude)?;
    let v_coeffs = general_intra_32x32_chroma_v_dc_coded_tokens(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        chroma_magnitude,
        V_TXB_SKIP_CTX_EOBU,
    )?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(u_coeffs.len()))
        .and_then(|n| n.checked_add(v_coeffs.len()))
        .and_then(|n| n.checked_add(3)) // do_split + U sign + V sign
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general all-planes coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general all-planes coded block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.extend(u_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    trace.extend(v_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose **luma, U, and V** blocks each carry a single negative coded DC coefficient.
///
/// This is the encoder's first frame with all three planes coded at once, mirroring the q80
/// fixture's structure: decoding reconstructs flat planes below `128` on every plane. The
/// cross-crate decode oracle lives in `splot-cli`.
///
/// # Errors
///
/// Returns an error if composing the block-symbol trace fails (token construction
/// or trace allocation), if entropy-coding the trace into tile data fails, or if
/// the AV2 IVF stream cannot be assembled by `splot-core`.
pub fn emit_minimal_intra_all_planes_coded_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_all_planes_coded_block_trace(
        CODED_LUMA_DC_MAGNITUDE,
        CODED_CHROMA_U_DC_MAGNITUDE,
        true,
    )?;
    super::emit_minimal_intra_ivf(&trace)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::block_symbol_trace::roundtrip_block_symbol_trace;

    #[test]
    fn composes_general_coded_chroma_block_trace_in_order() {
        // Magnitude 4: U coeff_base_eob = 3 (level 4), no coeff_br/golomb.
        let trace =
            compose_general_intra_coded_chroma_u_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();

        // do_split, 3 modes, luma txb_skip (skip), 3 coded-U symbols, U sign bypass,
        // V txb_skip (skip) = 10 tokens.
        assert_eq!(trace.len(), 10);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        for token in &trace[1..4] {
            assert!(matches!(token, BlockSymbolToken::Mode(_)));
        }
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        // do_split=0, modes 0/0/0, luma txb_skip=1 (skip), U txb_skip=0, U eob_pt=0,
        // U coeff_base_eob=3, U sign_bit=1 (negative), V txb_skip=1 (skip).
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 0, 0, 3, 1, 1]
        );
    }

    #[test]
    fn general_coded_chroma_block_trace_roundtrips_through_one_coder() {
        let trace =
            compose_general_intra_coded_chroma_u_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 0, 0, 3, 1, 1]);
        assert_eq!(proof.symbol_count(), 10);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_coded_chroma_ivf_is_parseable_and_deterministic() {
        let ivf = emit_minimal_intra_coded_chroma_ivf().unwrap();
        assert!(!ivf.is_empty());
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(ivf, emit_minimal_intra_coded_chroma_ivf().unwrap());
    }

    #[test]
    fn composes_general_coded_chroma_v_block_trace_in_order() {
        let trace =
            compose_general_intra_coded_chroma_v_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();

        // do_split, 3 modes, luma txb_skip (skip), U txb_skip (skip), 3 coded-V symbols,
        // V sign bypass = 10 tokens.
        assert_eq!(trace.len(), 10);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[9],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        // do_split=0, modes 0/0/0, luma txb_skip=1, U txb_skip=1, V txb_skip=0, V eob_pt=0,
        // V coeff_base_eob=3, V sign_bit=1 (negative).
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 1, 0, 0, 3, 1]
        );
    }

    #[test]
    fn general_coded_chroma_v_block_trace_roundtrips_through_one_coder() {
        let trace =
            compose_general_intra_coded_chroma_v_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 1, 0, 0, 3, 1]);
        assert_eq!(proof.symbol_count(), 10);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_coded_chroma_v_ivf_is_parseable_and_deterministic() {
        let ivf = emit_minimal_intra_coded_chroma_v_ivf().unwrap();
        assert!(!ivf.is_empty());
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(ivf, emit_minimal_intra_coded_chroma_v_ivf().unwrap());
    }

    #[test]
    fn composes_general_all_planes_coded_block_trace_in_order() {
        let trace = compose_general_intra_all_planes_coded_block_trace(
            CODED_LUMA_DC_MAGNITUDE,
            CODED_CHROMA_U_DC_MAGNITUDE,
            true,
        )
        .unwrap();

        // do_split, 3 modes, 5 coded-luma, 3 coded-U, U sign, 3 coded-V, V sign = 17 tokens.
        assert_eq!(trace.len(), 17);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[12],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        assert!(matches!(
            trace[16],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        // do_split, modes 0/0/0; luma txb_skip=0,eob_pt=0,base=4,br=1,dc_sign=1;
        // U txb_skip=0,eob_pt=0,base=3; U sign=1; V txb_skip=0,eob_pt=0,base=3; V sign=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 0, 4, 1, 1, 0, 0, 3, 1, 0, 0, 3, 1]
        );
    }

    #[test]
    fn general_all_planes_coded_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_all_planes_coded_block_trace(
            CODED_LUMA_DC_MAGNITUDE,
            CODED_CHROMA_U_DC_MAGNITUDE,
            true,
        )
        .unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(
            proof.decoded_symbols(),
            &[0, 0, 0, 0, 0, 0, 4, 1, 1, 0, 0, 3, 1, 0, 0, 3, 1]
        );
        assert_eq!(proof.symbol_count(), 17);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_all_planes_coded_ivf_is_parseable_and_deterministic() {
        let ivf = emit_minimal_intra_all_planes_coded_ivf().unwrap();
        assert!(!ivf.is_empty());
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(ivf, emit_minimal_intra_all_planes_coded_ivf().unwrap());
    }
}
