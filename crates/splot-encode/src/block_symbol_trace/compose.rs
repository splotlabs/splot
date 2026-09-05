// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mode-prefix composition for the supported general-intra block.

use crate::intra_mode_emission::{
    IntraModeToken, emit_minimal_dc_chroma_uv_mode, emit_minimal_dc_luma_intra_mode,
};

/// Composes `y_mode_set`, `y_mode_index`, and `uv_mode` for the current DC block.
pub(crate) fn compose_minimal_intra_dc_block_mode_trace() -> [IntraModeToken; 3] {
    let [y_mode_set, y_mode_index] = emit_minimal_dc_luma_intra_mode();
    let [uv_mode] = emit_minimal_dc_chroma_uv_mode();
    [y_mode_set, y_mode_index, uv_mode]
}
