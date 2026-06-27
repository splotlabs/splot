// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The general intra coded-luma-DC block composer and IVF emitter.

use super::{SKIP_FRAME_COEFF_CDF_Q_CTX, V_TXB_SKIP_CTX_NEUTRAL};
use crate::block_symbol_trace::{BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_64x64_luma_dc_coded_tokens,
};
use crate::error::{Error, Result};
use crate::partition_emission::emit_root_do_split_none;

/// The unsigned luma DC magnitude the coded frame emits: `6` (`coeff_base_eob` saturated at
/// `4`, level 5, plus `coeff_br == 1`). Negative, it reconstructs flat luma `127` at
/// `base_q_idx == 80`. It is the largest magnitude **below the § 5.20.7.28 golomb threshold** on
/// this frame: the minimal header's luma uses TCQ, so `read_quant` reads a golomb tail once
/// `quant >= maxLevel - allowTcq == 7`. Magnitude `7` (the q80 luma level, reconstructing `100`)
/// would need that tail — a follow-up brick.
pub(super) const CODED_LUMA_DC_MAGNITUDE: u32 = 6;

/// Composes the general intra DC coded-block trace: like
/// [`super::skip::compose_general_intra_dc_skip_block_trace`] but the luma block carries one coded
/// DC coefficient of unsigned `magnitude` and the given sign (`txb_skip == 0`, `eob_pt == 0`,
/// `coeff_base_eob`, optional `coeff_br`, `dc_sign`) at the general `TX_64X64` contexts,
/// while U and V stay skipped. The V `txb_skip` keeps the neutral `ctx 0` (`EobU == 0`,
/// since U is skipped).
fn compose_general_intra_dc_coded_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma =
        general_intra_64x64_luma_dc_coded_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX, magnitude, negative)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(3)) // do_split + U + V txb_skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general coded block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
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
/// frame whose luma block carries a single **coded** DC coefficient (U and V skipped).
///
/// Unlike [`super::skip::emit_minimal_intra_skip_ivf`] (all-zero residual → flat 128), this emits
/// real residual: decoding reconstructs a flat luma plane of `127` (`128` minus the dequantized
/// negative DC of magnitude `CODED_LUMA_DC_MAGNITUDE`) and flat `128` chroma. It is the
/// encoder's first decodable output carrying a coded coefficient. The cross-crate decode
/// oracle that proves the reconstruction lives in `splot-cli`.
///
/// # Errors
///
/// Returns an error if composing the block-symbol trace fails (token construction
/// or trace allocation), if entropy-coding the trace into tile data fails, or if
/// the AV2 IVF stream cannot be assembled by `splot-core`.
pub fn emit_minimal_intra_coded_dc_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_dc_coded_block_trace(CODED_LUMA_DC_MAGNITUDE, true)?;
    super::emit_minimal_intra_ivf(&trace)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::block_symbol_trace::roundtrip_block_symbol_trace;

    #[test]
    fn composes_general_coded_block_trace_in_order() {
        // Magnitude 6: coeff_base_eob saturates at 4 (level 5) plus coeff_br == 1.
        let trace =
            compose_general_intra_dc_coded_block_trace(CODED_LUMA_DC_MAGNITUDE, true).unwrap();

        // do_split, 3 mode symbols, 5 coded-luma symbols (txb_skip, eob_pt, coeff_base_eob,
        // coeff_br, dc_sign), then U and V txb_skip = 11 tokens.
        assert_eq!(trace.len(), 11);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        for token in &trace[1..4] {
            assert!(matches!(token, BlockSymbolToken::Mode(_)));
        }
        for token in &trace[4..11] {
            assert!(matches!(token, BlockSymbolToken::Coeff(_)));
        }
        // do_split=0, modes 0/0/0, luma txb_skip=0, eob_pt=0, coeff_base_eob=4, coeff_br=1,
        // dc_sign=1 (negative), then U/V all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 0, 4, 1, 1, 1, 1]
        );
    }

    #[test]
    fn general_coded_block_trace_roundtrips_through_one_coder() {
        let trace =
            compose_general_intra_dc_coded_block_trace(CODED_LUMA_DC_MAGNITUDE, true).unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 0, 4, 1, 1, 1, 1]);
        assert_eq!(proof.symbol_count(), 11);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_coded_dc_ivf_is_a_parseable_one_frame_av02_ivf() {
        let ivf = emit_minimal_intra_coded_dc_ivf().unwrap();
        assert!(!ivf.is_empty());

        // Structurally a single-frame AV02 64x64 IVF; the decode-to-flat-127 luma proof is
        // the cross-crate oracle in splot-cli.
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        let header = parsed.header.unwrap();
        assert_eq!(&header.fourcc, b"AV02");
        assert_eq!((header.width, header.height), (64, 64));
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn emit_minimal_intra_coded_dc_ivf_is_deterministic() {
        assert_eq!(
            emit_minimal_intra_coded_dc_ivf().unwrap(),
            emit_minimal_intra_coded_dc_ivf().unwrap()
        );
    }
}
