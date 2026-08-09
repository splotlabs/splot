// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Block-symbol traces for the supported undivided 64x64 general-intra packet paths.
//!
//! Each trace starts with the § 5.20.3.2 `do_split` flag, uses the fixed CDF rows
//! selected by the supported header, and is verified by decoder-backed CLI oracles.

mod chroma;
mod coded_dc;
mod multi_coeff;
mod skip;

pub(crate) use skip::emit_minimal_intra_skip_temporal_unit_with_base_q_idx;

pub use chroma::{
    emit_minimal_intra_all_planes_coded_ivf, emit_minimal_intra_coded_chroma_ivf,
    emit_minimal_intra_coded_chroma_v_ivf,
};
pub use coded_dc::emit_minimal_intra_coded_dc_ivf;
pub use multi_coeff::{
    emit_minimal_intra_2d_ivf, emit_minimal_intra_eob3_ivf, emit_minimal_intra_two_coeff_ivf,
    emit_minimal_intra_two_nonzero_ivf, emit_minimal_intra_visible_ac_ivf,
};
pub use skip::emit_minimal_intra_skip_ivf;

use crate::block_symbol_trace::{BlockSymbolToken, encode_block_symbol_trace};
use crate::error::{Error, Result};

/// The `base_q_idx` the minimal intra skip frame is muxed at: 80, the AVM- and
/// dav2d-validated `syn-flat-intra-64x64-q80` fixture's value. It is `<= 90`, so the decoder
/// derives coefficient CDF q-context `0`, matching the fixed rows used by the tile trace.
pub(super) const SKIP_FRAME_BASE_Q_IDX: u8 = 80;

/// A chroma DC `sign_bit` is a § 8.2.5 `L(1)` bypass literal.
pub(super) const CHROMA_SIGN_BIT_WIDTH: u32 = 1;

/// Assembles a complete, decodable single-frame AV2 IVF stream from an already-composed
/// general-intra block-symbol `trace`: finalizes the trace into `tile_data`
/// ([`crate::block_symbol_trace::encode_block_symbol_trace`]) and muxes it into the canonical
/// 64x64 all-intra `OBU_CLOSED_LOOP_KEY` container at [`SKIP_FRAME_BASE_Q_IDX`]. Every
/// `emit_minimal_intra_*_ivf` producer differs only in the `trace` it composes; this is their
/// shared tail. The cross-crate decode oracle that proves each reconstruction lives in `splot-cli`.
fn emit_minimal_intra_ivf(trace: &[BlockSymbolToken]) -> Result<Vec<u8>> {
    let tile_data = encode_block_symbol_trace(trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}
