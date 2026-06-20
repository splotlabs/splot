// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder block-symbol trace composers for the **general** AV2 intra decode path.
//!
//! The minimal-tier composers in `block_symbol_trace` model the frozen single-block
//! acceptor; these model the symbol stream the AVM-validated general intra decode
//! path reads for one undivided 64x64 superblock, which leads with the § 5.20.3.2
//! `do_split` partition flag and codes the `txb_skip` symbols at the 64x64-leaf
//! transform contexts. The composed traces are driven through the shared § 8.2
//! coder by `block_symbol_trace::roundtrip_block_symbol_trace`.

// The encoder runtime does not yet consume these composers (it returns
// `NeedMoreData`); they are exercised by the block-symbol-trace roundtrip tests,
// matching the sibling emission modules' policy.
#![allow(dead_code)]

use crate::block_symbol_trace::{BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_64x64_luma_all_zero_token,
};
use crate::error::{Error, Result};
use crate::partition_emission::emit_root_do_split_none;

/// The coefficient CDF q-context for a skip frame whose `base_q_idx <= 90`:
/// `coeff_cdf_q_ctx_from_base_q_idx` bank `0` (the same bank the AVM-validated
/// `syn-flat-intra-64x64-q80` fixture's `base_q_idx == 80` selects).
const SKIP_FRAME_COEFF_CDF_Q_CTX: usize = 0;

/// The § 8.3.2 neutral V `txb_skip` context: `0`. For this skip block the chroma
/// block equals its transform size and the U plane is all-zero (`EobU == 0`), so
/// neither the chroma-larger-than-tx (`+3`) nor the `EobU != 0` (`+6`) term applies.
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;

/// Composes the complete ordered general intra DC skip-block trace read on the AV2
/// general intra decode path for one undivided 64x64 superblock: the § 5.20.3.2
/// `do_split == false` (`PARTITION_NONE`) flag, the § 5.20.5.3 mode-info prefix
/// (`y_mode_set`, `y_mode_index`, `uv_mode`, all `0` for DC), then the per-plane
/// § 5.20.7.27 `all_zero` (`txb_skip`) symbols (`1` each) in `residual()` order
/// Y, U, V.
///
/// Unlike `block_symbol_trace::compose_minimal_intra_dc_complete_all_zero_block_trace`
/// it leads with `do_split` and codes the luma/U `txb_skip` at the `TX_64X64` /
/// `TX_32X32` `txSzCtx` of a 64x64 4:2:0 leaf rather than the minimal `TX_4X4` ctx;
/// the V `txb_skip` keeps `ctx 0`. The coefficient CDF q-context is `0`.
pub(crate) fn compose_general_intra_dc_skip_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let total = modes
        .len()
        .checked_add(4)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general skip block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general skip block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_64x64_luma_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::block_symbol_trace::roundtrip_block_symbol_trace;

    #[test]
    fn composes_general_skip_block_trace_in_order() {
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();

        assert_eq!(trace.len(), 7);
        // do_split partition flag, mode prefix (Y set/index, UV), then per-plane
        // all_zero (Y, U, V) in residual() order.
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[3], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[4], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[5], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[6], BlockSymbolToken::Coeff(_)));
        // do_split=0, y_mode_set=0, y_mode_index=0, uv_mode=0, then luma/U/V all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 1, 1]
        );
    }

    #[test]
    fn general_skip_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 1, 1]);
        assert_eq!(proof.symbol_count(), 7);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn general_skip_block_roundtrip_is_deterministic() {
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();
        let first = roundtrip_block_symbol_trace(&trace).unwrap();
        let second = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }
}
