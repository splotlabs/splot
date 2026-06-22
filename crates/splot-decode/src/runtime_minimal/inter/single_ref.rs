// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.20.7.12 `read_single_ref` entropy element.
//!
//! `read_single_ref` selects `RefFrame[0]` for a single-reference inter block by
//! reading a sequence of binary `single_ref` symbols (AV2
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-12`). The loop runs
//! `ref` from `0` to `NumTotalRefs - 2`; each iteration reads one `single_ref`
//! symbol over `TileSingleRefCdf[ctx][ref]`, returning `ref` on the first `1`
//! bit, and `NumTotalRefs - 1` when every symbol decoded to `0`. The per-decision
//! CDF row is `TileSingleRefCdf[ctx][ref]` where `ctx` is the § 8.3.2
//! neighbour-derived single_ref context (the same derivation as `comp_ref`,
//! `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2` line 1094) and `ref` is the
//! loop counter (cross-checked against AVM `read_single_ref` in
//! `av2/decoder/decodemv.c` and `av2_get_pred_cdf_single_ref` in
//! `av2/common/pred_common.h`, which index `single_ref_cdf[ctx][ref]` identically).
//!
//! This element is NOT yet runtime-reachable: `read_single_ref` is only read when
//! `NumTotalRefs >= 2`, which requires at least two valid reference slots — a
//! larger multi-reference runtime brick (§ 7.7 two-valid-slot feed plus a >= 3
//! frame reference-retention loop). This module proves the entropy element
//! bit-exact through a `SymbolEncoder` round-trip; the § 8.3.2 neighbour-context
//! derivation and the runtime wiring (relaxing the `NumTotalRefs == 1` gate) are
//! the explicit follow-on (the multi-reference brick).
//!
//! Feature tracking: `DECODE-INTER-SINGLE-REF-SYMBOL`.

use splot_core::symbol::SymbolDecoder;

use crate::tile_payload::{TileCdfSelector, TileCdfSubset};

/// AV2 § 5.20.7.12 `read_single_ref` error.
///
/// `allow(dead_code)` in non-test builds: this entropy element is intentionally
/// loaded-but-unwired (it is only read at runtime once `NumTotalRefs >= 2`, which
/// the deferred multi-reference brick enables). It is exercised by the
/// `SymbolEncoder` round-trip tests; the runtime wiring removes this allow.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, thiserror::Error)]
pub(crate) enum SingleRefReadError {
    /// `num_total_refs` was 0. AV2 § 6.19.7.11 requires `NumTotalRefs > 0` for any
    /// inter frame, and § 5.20.7.12 computes `NumTotalRefs - 1`, which would
    /// underflow at 0. (`NumTotalRefs == 1` is LEGAL: the § 5.20.7.12 loop is empty
    /// and returns 0 with no `single_ref` symbol read — that is not an error.)
    #[error("read_single_ref requires NumTotalRefs > 0 (§6.19.7.11), got {num_total_refs}")]
    InsufficientRefs {
        /// The caller-supplied `NumTotalRefs`.
        num_total_refs: usize,
    },
    /// Fewer per-decision contexts were supplied than the `num_total_refs - 1`
    /// decisions `read_single_ref` makes.
    #[error("read_single_ref needs {needed} contexts for NumTotalRefs {num_total_refs}, got {got}")]
    MissingContext {
        /// `num_total_refs - 1` — the number of `single_ref` decisions.
        needed: usize,
        /// The number of contexts supplied.
        got: usize,
        /// The caller-supplied `NumTotalRefs`.
        num_total_refs: usize,
    },
    /// A `single_ref` symbol could not be read from the tile arithmetic stream
    /// over the selected `TileSingleRefCdf[ctx][ref]` row.
    #[error("read_single_ref symbol read failed at ref {ref_idx}")]
    SymbolRead {
        /// The § 5.20.7.12 loop counter at which the read failed.
        ref_idx: usize,
    },
}

/// Reads AV2 § 5.20.7.12 `read_single_ref` and returns the selected `RefFrame[0]`.
///
/// Reads up to `num_total_refs - 1` binary `single_ref` symbols, one per
/// `ref ∈ 0..num_total_refs - 1`, each over `TileSingleRefCdf[contexts[ref]][ref]`
/// (the § 8.3.2 context is caller-supplied here; the neighbour-derivation is
/// deferred to the multi-reference runtime brick). Returns `ref` on the first
/// symbol that decodes to `1`, and `num_total_refs - 1` when all decode to `0`,
/// exactly mirroring the spec loop and AVM `read_single_ref`.
///
/// `contexts` supplies the per-decision § 8.3.2 context for each `ref` index; it
/// MUST hold at least `num_total_refs - 1` entries.
///
/// For `num_total_refs == 1` the § 5.20.7.12 loop is empty, so it returns 0 with
/// no `single_ref` symbol read (the legal one-reference case).
///
/// # Errors
/// Returns [`SingleRefReadError::InsufficientRefs`] when `num_total_refs == 0`
/// (§ 6.19.7.11 requires `NumTotalRefs > 0`; the spec computes `NumTotalRefs - 1`,
/// which underflows at 0),
/// [`SingleRefReadError::MissingContext`] when `contexts` is shorter than the
/// `num_total_refs - 1` decisions, or [`SingleRefReadError::SymbolRead`] when a
/// `single_ref` symbol cannot be read (an out-of-range CDF selector or a § 8.2
/// symbol-decode failure, e.g. a truncated tile payload).
///
/// `allow(dead_code)` in non-test builds: see [`SingleRefReadError`] — this is the
/// loaded-but-unwired § 5.20.7.12 entropy element, proven by the round-trip tests
/// until the multi-reference runtime brick wires it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn read_single_ref(
    cdfs: &mut TileCdfSubset,
    symbols: &mut SymbolDecoder<'_>,
    num_total_refs: usize,
    contexts: &[usize],
) -> Result<usize, SingleRefReadError> {
    if num_total_refs == 0 {
        return Err(SingleRefReadError::InsufficientRefs { num_total_refs });
    }
    // §5.20.7.12: for ( ref = 0; ref < NumTotalRefs - 1; ref++ ).
    let decisions = num_total_refs - 1;
    if contexts.len() < decisions {
        return Err(SingleRefReadError::MissingContext {
            needed: decisions,
            got: contexts.len(),
            num_total_refs,
        });
    }
    for (ref_idx, &ctx) in contexts.iter().enumerate().take(decisions) {
        // §8.3.2: TileSingleRefCdf[ctx][ref]. The context for this `ref` is
        // caller-supplied (the neighbour derivation is deferred to the
        // multi-reference runtime brick).
        let single_ref = cdfs
            .read_block_symbol_trace(TileCdfSelector::SingleRef { ctx, ref_idx }, symbols)
            .map_err(|_| SingleRefReadError::SymbolRead { ref_idx })?;
        // §5.20.7.12: if ( single_ref ) return ref.
        if single_ref.get() != 0 {
            return Ok(ref_idx);
        }
    }
    // §5.20.7.12: return NumTotalRefs - 1.
    Ok(decisions)
}

#[cfg(test)]
mod tests;
