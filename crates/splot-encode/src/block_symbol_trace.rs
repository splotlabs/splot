// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private encoder intra-block mode-trace composition.
//!
//! This module advances `ENC-INTRA-BLOCK-MODE-TRACE`. It composes the ordered
//! AV2 § 5.20.5.3 mode-info prefix for the current minimal DC intra block —
//! `y_mode_set`, `y_mode_index`, then `uv_mode` — by reusing the merged luma and
//! chroma mode emitters, and proves the combined sequence roundtrips through one
//! in-tree AV2 § 8.2 symbol encoder/decoder with shared CDF state
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
//!
//! AV2 § 5.20.5.3 `intra_frame_mode_info()` calls `read_intra_y_mode()` then
//! `read_intra_uv_mode()` (before `residual()`), so the mode-info prefix is the
//! luma mode tokens followed by the chroma `uv_mode` token. The coefficient
//! symbols that follow in `residual()` are out of scope here and join the trace
//! in a later change.
//!
//! It does not emit coefficient or all-zero symbols, partition syntax, tile
//! payloads, tile CDF lifecycle, packets, a public encoder API, or modes beyond
//! the DC minimal tier. It is the home for the growing ordered block-symbol
//! trace.

#![allow(dead_code)]

use crate::error::{Error, Result};
use crate::intra_mode_emission::{
    IntraModeToken, emit_minimal_dc_chroma_uv_mode, emit_minimal_dc_luma_intra_mode,
};

/// Composes the ordered AV2 § 5.20.5.3 intra-block mode-info prefix
/// (`y_mode_set`, `y_mode_index`, `uv_mode`) for the current minimal DC subset.
pub(crate) fn compose_minimal_intra_dc_block_mode_trace() -> Result<Vec<IntraModeToken>> {
    let luma = emit_minimal_dc_luma_intra_mode()?;
    let uv = emit_minimal_dc_chroma_uv_mode()?;

    let total = luma.tokens().len().checked_add(uv.tokens().len()).ok_or(
        Error::IntraModeEmissionAllocationFailed {
            context: "intra block mode trace length",
        },
    )?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::IntraModeEmissionAllocationFailed {
            context: "intra block mode trace",
        })?;
    trace.extend_from_slice(luma.tokens());
    trace.extend_from_slice(uv.tokens());
    Ok(trace)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::intra_mode_emission::{
        IntraModeCdfRowSelector, IntraModeSyntax, roundtrip_intra_mode_tokens,
    };

    #[test]
    fn composes_ordered_mode_info_prefix() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();

        assert_eq!(trace.len(), 3);
        assert_eq!(trace[0].syntax(), IntraModeSyntax::YModeSet);
        assert_eq!(trace[1].syntax(), IntraModeSyntax::YModeIndex);
        assert_eq!(trace[2].syntax(), IntraModeSyntax::UvMode);
        assert!(matches!(
            trace[0].selector(),
            IntraModeCdfRowSelector::YModeSet
        ));
        assert!(matches!(
            trace[1].selector(),
            IntraModeCdfRowSelector::YModeIndex { ctx: 0 }
        ));
        assert!(matches!(
            trace[2].selector(),
            IntraModeCdfRowSelector::UvModeCflNotAllowed { ctx: 0 }
        ));
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
    }

    #[test]
    fn composed_trace_matches_concatenated_emitters() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
        let luma = emit_minimal_dc_luma_intra_mode().unwrap();
        let uv = emit_minimal_dc_chroma_uv_mode().unwrap();

        let mut expected = luma.tokens().to_vec();
        expected.extend_from_slice(uv.tokens());
        assert_eq!(trace, expected);
    }

    #[test]
    fn composed_trace_roundtrips_through_one_coder() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
        let proof = roundtrip_intra_mode_tokens(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0]);
        assert_eq!(proof.symbol_count(), 3);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn roundtrip_is_deterministic() {
        let trace = compose_minimal_intra_dc_block_mode_trace().unwrap();
        let first = roundtrip_intra_mode_tokens(&trace).unwrap();
        let second = roundtrip_intra_mode_tokens(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }
}
