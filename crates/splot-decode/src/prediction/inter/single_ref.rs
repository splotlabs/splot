// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::SymbolDecoder;

use crate::bitstream::tile_payload::{BlockSymbolTraceReadError, TileCdfSelector, TileCdfSubset};

#[derive(Debug, thiserror::Error)]
pub(crate) enum SingleRefReadError {
    #[error("read_single_ref requires NumTotalRefs > 0 (§6.19.7.11), got {num_total_refs}")]
    InsufficientRefs { num_total_refs: usize },
    #[error("read_single_ref needs {needed} contexts for NumTotalRefs {num_total_refs}, got {got}")]
    MissingContext {
        needed: usize,
        got: usize,
        num_total_refs: usize,
    },
    #[error("read_single_ref symbol read failed at ref {ref_idx}: {source}")]
    SymbolRead {
        ref_idx: usize,
        #[source]
        source: BlockSymbolTraceReadError,
    },
}

pub(crate) fn read_single_ref(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    num_total_refs: usize,
    contexts: &[usize],
) -> Result<usize, SingleRefReadError> {
    if num_total_refs == 0 {
        return Err(SingleRefReadError::InsufficientRefs { num_total_refs });
    }
    let decisions = num_total_refs - 1;
    let contexts = contexts
        .get(..decisions)
        .ok_or(SingleRefReadError::MissingContext {
            needed: decisions,
            got: contexts.len(),
            num_total_refs,
        })?;

    for (ref_idx, &ctx) in contexts.iter().enumerate() {
        let single_ref = cdfs
            .read_block_symbol_trace(TileCdfSelector::SingleRef { ctx, ref_idx }, symbols)
            .map_err(|source| SingleRefReadError::SymbolRead { ref_idx, source })?;
        if single_ref.get() != 0 {
            return Ok(ref_idx);
        }
    }
    Ok(decisions)
}

#[cfg(test)]
mod tests;
