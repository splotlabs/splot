// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The general intra DC **skip**-block composer, tile-data encoder, and IVF emitter.

use super::{SKIP_FRAME_BASE_Q_IDX, SKIP_FRAME_COEFF_CDF_Q_CTX, V_TXB_SKIP_CTX_NEUTRAL};
use crate::block_symbol_trace::{
    BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace, encode_block_symbol_trace,
};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_64x64_luma_all_zero_token,
};
use crate::error::{Error, Result};
use crate::partition_emission::emit_root_do_split_none;

/// Composes the complete ordered general intra DC skip-block trace read on the AV2 general intra
/// decode path for one undivided 64x64 superblock: the § 5.20.3.2 `do_split == false`
/// (`PARTITION_NONE`) flag, the § 5.20.5.3 mode-info prefix (`y_mode_set`, `y_mode_index`,
/// `uv_mode`, all `0` for DC), then the per-plane § 5.20.7.27 `all_zero` (`txb_skip`) symbols
/// (`1` each) in `residual()` order Y, U, V. Unlike the minimal-tier all-zero composer it leads
/// with `do_split` and codes the luma/U `txb_skip` at the `TX_64X64` / `TX_32X32` `txSzCtx` of a
/// 64x64 4:2:0 leaf (not `TX_4X4`); the V `txb_skip` keeps `ctx 0`, q-context `0`.
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

/// Encodes the general intra DC skip-block trace into its AV2 § 8.2.4-finalized `tile_data`
/// bytes — the entropy-coded payload of a single-tile general intra frame, which the decoder
/// consumes from byte 0 via § 8.2.2 `init_symbol` (a single last tile has no `tile_size_minus_1`
/// prefix). For an identical read-back the muxing header must set `base_q_idx <= 90` (q-context
/// `0`, matching [`SKIP_FRAME_COEFF_CDF_Q_CTX`]) and `disable_cdf_update == 0`. Emits only the
/// tile bytes; container assembly and the decode oracle are later bricks.
pub(crate) fn encode_general_intra_dc_skip_tile_data() -> Result<Vec<u8>> {
    let trace = compose_general_intra_dc_skip_block_trace()?;
    encode_block_symbol_trace(&trace)
}

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

/// Emits the minimal intra skip frame as a single coded **access unit** — the AV2 Annex B
/// temporal unit (`OBU_TEMPORAL_DELIMITER` + `OBU_SEQUENCE_HEADER` + the `OBU_CLOSED_LOOP_KEY`
/// frame OBU), without the IVF file wrapper. This is the access-unit form
/// `Context::receive_packet` returns in a `Packet`: it is self-delimiting (the decoder
/// auto-detects it as Annex B) and concatenating packets yields a valid stream, unlike emitting a
/// full IVF *file* per packet.
pub(crate) fn emit_minimal_intra_skip_temporal_unit() -> Result<Vec<u8>> {
    emit_minimal_intra_skip_temporal_unit_with_base_q_idx(SKIP_FRAME_BASE_Q_IDX)
}

/// Emits the minimal intra skip frame access unit (see [`emit_minimal_intra_skip_temporal_unit`])
/// muxed at a caller-chosen `base_q_idx` (AV2 § 5.18.6.1;
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`). The DC skip block has an all-zero
/// residual, so its flat reconstruction is independent of `base_q_idx`; only the frame-header
/// quantizer field changes. The tile's coefficient CDF q-context stays
/// [`SKIP_FRAME_COEFF_CDF_Q_CTX`] (`0`), which matches the decoder's current q-context for the
/// supported `base_q_idx` range; callers (`Context`) restrict `base_q_idx` to that range.
//
// TODO(spec: ENC-CONFIG-QP-FIELD): derive the coefficient CDF q-context from `base_q_idx`
// (the §8.3.2 `get_qctx` thresholds) so the full quantizer range is supported, co-evolving with
// the decoder's q-context (currently a documented placeholder pinned to 0).
pub(crate) fn emit_minimal_intra_skip_temporal_unit_with_base_q_idx(
    base_q_idx: u8,
) -> Result<Vec<u8>> {
    let tile_data = encode_general_intra_dc_skip_tile_data()?;
    splot_core::headers::frame::encode_minimal_intra_clk_temporal_unit_with_base_q_idx(
        base_q_idx, &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    use crate::block_symbol_trace::roundtrip_block_symbol_trace;

    #[test]
    fn skip_temporal_unit_is_the_skip_ivf_frame_payload() {
        // The access unit is exactly the bytes muxed into the IVF frame: an IVF made of the
        // canonical AV02 64x64 header plus this temporal unit equals the standalone skip IVF.
        let temporal_unit = emit_minimal_intra_skip_temporal_unit().unwrap();
        let mut ivf = Vec::new();
        splot_core::ivf::write_ivf_header(
            &mut ivf,
            &splot_core::ivf::IvfHeader::new(*b"AV02", 64, 64, 30, 1, 1),
        )
        .unwrap();
        splot_core::ivf::write_ivf_frame(&mut ivf, 0, &temporal_unit).unwrap();
        assert_eq!(ivf, emit_minimal_intra_skip_ivf().unwrap());
    }

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
