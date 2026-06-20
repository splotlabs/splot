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

use crate::block_symbol_trace::{
    BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace, encode_block_symbol_trace,
};
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

/// Encodes the general intra DC skip-block trace into its AV2 § 8.2.4-finalized
/// `tile_data` bytes — the entropy-coded payload of a single-tile general intra
/// frame, which the decoder consumes directly from byte 0 via § 8.2.2
/// `init_symbol` with no structural prefix (a single last tile carries no
/// `tile_size_minus_1` field).
///
/// These are the § 5.20 `tile_data` bytes for one 64x64 DC skip (all-zero)
/// superblock. For the decoder to read them back identically, the muxing frame
/// header (a later brick) must set `base_q_idx <= 90` (so the decoder derives
/// coefficient CDF q-context `0`, matching [`SKIP_FRAME_COEFF_CDF_Q_CTX`]) and
/// `disable_cdf_update == 0` (so the tile reader's adaptive CDFs match the
/// `CdfUpdateMode::Enabled` this trace is coded under). This function emits only
/// the tile bytes; container assembly and the cross-crate decode oracle are later
/// bricks.
pub(crate) fn encode_general_intra_dc_skip_tile_data() -> Result<Vec<u8>> {
    let trace = compose_general_intra_dc_skip_block_trace()?;
    encode_block_symbol_trace(&trace)
}

/// The `base_q_idx` the minimal intra skip frame is muxed at: 80, the AVM- and
/// dav2d-validated `syn-flat-intra-64x64-q80` fixture's value. It is `<= 90`, so the decoder
/// derives coefficient CDF q-context `0` — the q-context [`encode_general_intra_dc_skip_tile_data`]
/// codes its `txb_skip` symbols under.
const SKIP_FRAME_BASE_Q_IDX: u8 = 80;

/// Emits a complete, decodable AV2 IVF stream: one 64x64 all-intra `OBU_CLOSED_LOOP_KEY` frame
/// whose single tile is a DC skip (all-zero residual) block. This is `splot-encode`'s first
/// end-to-end decodable output.
///
/// It pairs the general-intra DC skip `tile_data` (`encode_general_intra_dc_skip_tile_data`)
/// with the `base_q_idx`-80 container
/// ([`splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx`]) whose
/// decoder-derived coefficient CDF q-context (`0`) matches the tile's coding. Decoding the
/// stream reconstructs a flat frame: every luma and chroma sample is the § 7.13.2 DC
/// prediction of a no-neighbour block — `128` for 8-bit — because the block is skipped.
///
/// The container is byte-identical to the `syn-flat-intra-64x64-q80` fixture apart from the
/// `tile_data`; only the tile bytes encode the skip block. The cross-crate decode that proves
/// the flat reconstruction lives in `splot-cli` (which depends on both `splot-encode` and
/// `splot-decode`).
pub fn emit_minimal_intra_skip_ivf() -> Result<Vec<u8>> {
    let tile_data = encode_general_intra_dc_skip_tile_data()?;
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

    #[test]
    fn skip_tile_data_is_nonempty_and_equals_the_proven_trace_bytes() {
        let tile_data = encode_general_intra_dc_skip_tile_data().unwrap();
        // A zero-size tile is a §8.2.2 defect; the §8.2.4 finalization always
        // emits at least the exit-window bytes.
        assert!(!tile_data.is_empty());

        // The emitted tile_data IS the finalized byte stream of the proven skip
        // trace — identical to the bytes the roundtrip decodes back, so the
        // standalone emission inherits that decodability proof.
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(tile_data, proof.bytes());
        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 1, 1]);
    }

    #[test]
    fn skip_tile_data_is_deterministic() {
        assert_eq!(
            encode_general_intra_dc_skip_tile_data().unwrap(),
            encode_general_intra_dc_skip_tile_data().unwrap()
        );
    }

    #[test]
    fn emit_minimal_intra_skip_ivf_is_a_parseable_one_frame_av02_ivf() {
        let ivf = emit_minimal_intra_skip_ivf().unwrap();
        assert!(!ivf.is_empty());

        // Structurally a single-frame AV02 64x64 IVF; the full decode-to-flat-128 proof is
        // the cross-crate oracle in splot-cli.
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        let header = parsed.header.unwrap();
        assert_eq!(&header.fourcc, b"AV02");
        assert_eq!((header.width, header.height), (64, 64));
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn emit_minimal_intra_skip_ivf_is_deterministic() {
        assert_eq!(
            emit_minimal_intra_skip_ivf().unwrap(),
            emit_minimal_intra_skip_ivf().unwrap()
        );
    }
}
