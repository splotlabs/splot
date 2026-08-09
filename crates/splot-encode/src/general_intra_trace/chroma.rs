// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The general intra coded-chroma (U, V) and all-planes-coded block composers and IVF emitters.

use super::CHROMA_SIGN_BIT_WIDTH;
use crate::block_symbol_trace::{BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_after_coded_u_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_32x32_chroma_u_dc_coded_tokens,
    general_intra_32x32_chroma_v_after_coded_u_dc_coded_tokens,
    general_intra_32x32_chroma_v_dc_coded_tokens, general_intra_64x64_luma_all_zero_token,
    general_intra_64x64_luma_dc_coded_tokens,
};
use crate::error::{Error, Result};
use crate::partition_emission::emit_root_do_split_none;

/// The chroma DC magnitude is `4` (`coeff_base_eob` 3,
/// level 4 — the base tier, no `coeff_br`/golomb). Negative, it dequantizes to a flat U
/// reconstruction below `128`.
///
/// Composes the general intra coded-*chroma* block trace: `do_split`, the mode prefix, a
/// **skipped** luma plane, then a single coded U DC coefficient (`txb_skip == 0`,
/// `eob_pt == 0`, `coeff_base_eob`, then the U DC `sign_bit` § 8.2.5 bypass literal) at the
/// general `TX_32X32` chroma contexts, then a skipped V plane whose `txb_skip` uses the
/// `EobU != 0` context `6`. The decoded frame isolates chroma residual: luma stays flat 128,
/// U carries the dequantized residual, V stays flat 128.
fn compose_general_intra_coded_chroma_u_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let u_coeffs = general_intra_32x32_chroma_u_dc_coded_tokens()?;
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
        general_intra_64x64_luma_all_zero_token(),
    ));
    trace.extend(u_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 1));
    trace.push(BlockSymbolToken::Coeff(
        chroma_v_all_zero_after_coded_u_token(),
    ));
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
    let trace = compose_general_intra_coded_chroma_u_block_trace()?;
    super::emit_minimal_intra_ivf(&trace)
}

/// Composes the general intra coded-*chroma-V* block trace: `do_split`, the mode prefix, a
/// **skipped** luma plane, a **skipped** U plane, then a single coded V DC coefficient
/// (`VTxbSkip == 0`, `eob_pt == 0`, `coeff_base_eob`, then the V DC `sign_bit` § 8.2.5 bypass
/// literal) at the general `TX_32X32` chroma contexts. The V `txb_skip` uses the neutral
/// context `0` (`EobU == 0`, since U is skipped). The decoded frame isolates V residual: luma
/// and U stay flat 128, V carries the dequantized residual.
fn compose_general_intra_coded_chroma_v_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let v_coeffs = general_intra_32x32_chroma_v_dc_coded_tokens()?;
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
        general_intra_64x64_luma_all_zero_token(),
    ));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(),
    ));
    trace.extend(v_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 1));
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
    let trace = compose_general_intra_coded_chroma_v_block_trace()?;
    super::emit_minimal_intra_ivf(&trace)
}

/// Composes the general intra all-planes-coded block trace: `do_split`, the mode prefix, then
/// a single coded DC coefficient on **each** of luma (`TX_64X64`, CDF `dc_sign`), U, and V
/// (`TX_32X32`, `sign_bit` § 8.2.5 bypass), in `residual()` order Y, U, V. Because the U plane
/// is coded (`EobU != 0`), the V `txb_skip` uses the § 8.3.2 context `6`. This mirrors the q80
/// fixture's all-three-planes-coded structure with sub-golomb magnitudes.
fn compose_general_intra_all_planes_coded_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_dc_coded_tokens()?;
    let u_coeffs = general_intra_32x32_chroma_u_dc_coded_tokens()?;
    let v_coeffs = general_intra_32x32_chroma_v_after_coded_u_dc_coded_tokens()?;
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
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 1));
    trace.extend(v_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 1));
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
    let trace = compose_general_intra_all_planes_coded_block_trace()?;
    super::emit_minimal_intra_ivf(&trace)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn composes_general_coded_chroma_block_trace_in_order() {
        let trace = compose_general_intra_coded_chroma_u_block_trace().unwrap();

        assert_eq!(trace.len(), 10);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        for token in &trace[1..4] {
            assert!(matches!(token, BlockSymbolToken::Mode(_)));
        }
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 0, 0, 3, 1, 1]
        );
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
    fn composes_general_coded_chroma_v_and_all_planes_traces_in_order() {
        let cases = [
            (
                compose_general_intra_coded_chroma_v_block_trace().unwrap(),
                &[(9, 1)][..],
                &[0, 0, 0, 0, 1, 1, 0, 0, 3, 1][..],
            ),
            (
                compose_general_intra_all_planes_coded_block_trace().unwrap(),
                &[(12, 1), (16, 1)][..],
                &[0, 0, 0, 0, 0, 0, 4, 1, 1, 0, 0, 3, 1, 0, 0, 3, 1][..],
            ),
        ];

        for (trace, bypasses, expected_symbols) in cases {
            assert_eq!(trace.len(), expected_symbols.len());
            assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
            for &(index, expected_value) in bypasses {
                assert!(matches!(
                    trace[index],
                    BlockSymbolToken::Bypass { width: 1, value }
                        if value == expected_value
                ));
            }
            assert_eq!(
                trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
                expected_symbols
            );
        }
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
    fn emit_minimal_intra_all_planes_coded_ivf_is_parseable_and_deterministic() {
        let ivf = emit_minimal_intra_all_planes_coded_ivf().unwrap();
        assert!(!ivf.is_empty());
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(ivf, emit_minimal_intra_all_planes_coded_ivf().unwrap());
    }
}
