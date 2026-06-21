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

mod chroma;
mod coded_dc;
mod multi_coeff;
mod skip;

// These crate-private composers are kept reachable at `crate::general_intra_trace::...`
// for follow-up bricks; no in-crate consumer reads them yet (matching the module's
// `#![allow(dead_code)]` policy).
#[allow(unused_imports)]
pub(crate) use skip::{
    compose_general_intra_dc_skip_block_trace, emit_minimal_intra_skip_temporal_unit,
    encode_general_intra_dc_skip_tile_data,
};

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

/// The coefficient CDF q-context for a skip frame whose `base_q_idx <= 90`:
/// `coeff_cdf_q_ctx_from_base_q_idx` bank `0` (the same bank the AVM-validated
/// `syn-flat-intra-64x64-q80` fixture's `base_q_idx == 80` selects).
pub(super) const SKIP_FRAME_COEFF_CDF_Q_CTX: usize = 0;

/// The § 8.3.2 neutral V `txb_skip` context: `0`. For these frames the chroma block equals its
/// transform and U is all-zero (`EobU == 0`), so neither the `+3` nor the `+6` term applies.
pub(super) const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;

/// The `base_q_idx` the minimal intra skip frame is muxed at: 80, the AVM- and
/// dav2d-validated `syn-flat-intra-64x64-q80` fixture's value. It is `<= 90`, so the decoder
/// derives coefficient CDF q-context `0` — the q-context [`skip::encode_general_intra_dc_skip_tile_data`]
/// codes its `txb_skip` symbols under.
pub(super) const SKIP_FRAME_BASE_Q_IDX: u8 = 80;

/// A chroma DC `sign_bit` is a § 8.2.5 `L(1)` bypass literal.
pub(super) const CHROMA_SIGN_BIT_WIDTH: u32 = 1;
