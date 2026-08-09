// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Mode-prefix composition for the supported general-intra block.

use crate::error::{Error, Result};
use crate::intra_mode_emission::{
    IntraModeToken, emit_minimal_dc_chroma_uv_mode, emit_minimal_dc_luma_intra_mode,
};

/// Composes `y_mode_set`, `y_mode_index`, and `uv_mode` for the current DC block.
pub(crate) fn compose_minimal_intra_dc_block_mode_trace() -> Result<Vec<IntraModeToken>> {
    let luma = emit_minimal_dc_luma_intra_mode()?;
    let uv = emit_minimal_dc_chroma_uv_mode()?;
    let total =
        luma.len()
            .checked_add(uv.len())
            .ok_or(Error::IntraModeEmissionAllocationFailed {
                context: "intra block mode trace length",
            })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::IntraModeEmissionAllocationFailed {
            context: "intra block mode trace",
        })?;
    trace.extend(luma);
    trace.extend(uv);
    Ok(trace)
}
