## Context

`DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF` routed the minimal flat-intra
runtime's traced luma and V all-zero coefficient blocks through the top
frame-facts coefficient wrapper. That runtime path still provides the wrapper
with locally hard-coded `txSz` ordinals for the traced transform dimensions.

This change removes those local ordinals from the runtime frontier. The traced
4x4-unit transform dimensions remain the source fact, and the runtime resolves
the AV2 transform-size enum by looking up matching width and height entries in
the generated AV2 section 9.2 `Tx_Width` and `Tx_Height` tables before entering
the all-zero coefficient frame-entry wrapper.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF` as a narrow runtime
  coefficient fact handoff.
- Resolve the minimal runtime luma 64x64 and V 16x16 all-zero wrapper inputs
  from traced transform geometry and generated AV2 conversion tables.
- Reject unsupported traced transform geometry with a typed local error before
  CDF or symbol state is consumed.
- Preserve all current minimal runtime output bytes, rollback behavior, public
  APIs, and dependency graph.

**Non-Goals:**

- Do not wire nonzero runtime `coeffs()` blocks.
- Do not derive runtime `PlaneTxType`, `fsc_mode`, `is_inter`, segment id,
  ordinary mode facts, or transform-block traversal beyond the traced minimal
  shapes.
- Do not implement dequantization, inverse transform, residual add,
  reconstruction output changes, reference refresh, or external decoder
  invocation.

## Decisions

- Keep the geometry-to-`txSz` resolver local to `block_symbol.rs` because this
  change only removes hard-coded runtime facts from the minimal block-symbol
  frontier. A shared helper would be premature until more runtime paths need the
  same conversion.
- Match generated `TX_WIDTH` and `TX_HEIGHT` entries by dimensions in pixels
  after checked `w4 * 4` and `h4 * 4` conversion. This keeps the mapping tied to
  the committed AV2 section 9.2 table data instead of duplicate ordinals.
- Return a typed `MinimalBlockSymbolTraceError` for overflow or unsupported
  geometry before entering the coefficient wrapper. That preserves the existing
  CDF transaction and avoids accidental symbol/CDF consumption on bad traced
  facts.
- Keep direct ordinary-branch equivalence tests in place, but build the runtime
  wrapper geometry through the new resolver so the test proves the new handoff
  while preserving previous behavior.

## Risks / Trade-offs

- Wrong geometry matching would mis-size all-zero coefficient context writes.
  Focused tests assert the generated-table dimensions selected for the traced
  luma and V inputs and preserve the existing direct-wrapper equivalence test.
- The local helper may later duplicate a broader transform-size derivation API.
  The duplication is acceptable while only the minimal traced runtime uses it;
  future nonzero `coeffs()` wiring can lift the helper once more callers exist.
- This is still not broad runtime coefficient decode. Matrix rows, roadmap
  notes, and conformance metadata must keep nonzero `coeffs()`, reconstruction,
  output expansion, and full decoder conformance partial or unsupported.
