// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The general intra DC **skip**-block composer, tile-data encoder, and IVF emitter.

use crate::block_symbol_trace::{BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace};
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
/// (`1` each) in `residual()` order Y, U, V. The luma/U rows use the `TX_64X64` / `TX_32X32`
/// contexts of a 64x64 4:2:0 leaf; V uses the neutral `txb_skip` row.
pub(crate) fn compose_general_intra_dc_skip_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace();
    let total = modes.len() + 4;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general skip block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_64x64_luma_all_zero_token(),
    ));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token()));
    Ok(trace)
}

/// Encodes the general intra DC skip-block trace into its AV2 § 8.2.4-finalized `tile_data`
/// bytes — the entropy-coded payload of a single-tile general intra frame, which the decoder
/// consumes from byte 0 via § 8.2.2 `init_symbol` (a single last tile has no `tile_size_minus_1`
/// prefix). The supported muxing headers select coefficient CDF q-context `0` and leave CDF
/// updates enabled, matching these fixed rows.
#[cfg(test)]
fn encode_general_intra_dc_skip_tile_data() -> Result<Vec<u8>> {
    let trace = compose_general_intra_dc_skip_block_trace()?;
    crate::block_symbol_trace::encode_block_symbol_trace(&trace)
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
///
/// # Errors
///
/// Returns an error if encoding the general-intra DC skip tile data fails (trace
/// composition or entropy coding), or if the AV2 IVF stream cannot be assembled by
/// `splot-core`.
pub fn emit_minimal_intra_skip_ivf() -> Result<Vec<u8>> {
    super::emit_minimal_intra_ivf(&compose_general_intra_dc_skip_block_trace()?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn composes_general_skip_block_trace_in_order() {
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();

        assert_eq!(trace.len(), 7);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[3], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[4], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[5], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[6], BlockSymbolToken::Coeff(_)));
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 1, 1]
        );
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
