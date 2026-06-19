## Context

`DECODE-COEFF-FRAME-FACTS-HANDOFF` and
`DECODE-COEFF-PARITY-TCQ-HANDOFF` created a crate-private top coefficient
branch wrapper that can derive frame-scoped coefficient facts for nonzero blocks
and preserves an all-zero bypass. The minimal flat-intra runtime still reaches
`apply_coeff_ordinary_branch` directly for the traced luma and V all-zero
coefficient blocks.

This change is the narrow runtime entry step: use the existing all-zero variant
of the frame-facts wrapper for the traced runtime all-zero blocks. The all-zero
variant intentionally carries only coefficient geometry, because AV2
§ 5.20.7.27 handles `all_zero` before the nonzero `useFsc`, lossless, parity,
and TCQ derivations.

## Decisions

- Add a small local helper in `block_symbol.rs` that converts the traced
  all-zero block dimensions into `CoeffOrdinaryTxSizeGeometryConfig` and calls
  `apply_coeff_use_fsc_branch_from_frame_facts`.
- Use generated-table-compatible `tx_size` constants for the traced block
  shapes: `TX_64X64` for the luma block and `TX_16X16` for the V block.
- Replace the runtime error variant from ordinary-branch-specific to the top
  coefficient `useFsc` branch error so failures from the wrapper are surfaced
  without losing source information.
- Keep the existing CDF rollback transaction unchanged around the whole traced
  block-symbol frontier.

## Non-Goals

- Do not make the minimal runtime consume nonzero coefficient blocks.
- Do not derive runtime `PlaneTxType`, `fsc_mode`, `is_inter`, segment id,
  `TxTypes`, `UVMode`, or `AngleDeltaUV` facts.
- Do not change decoded hash/raw/Y4M output.
- Do not change public APIs or crate dependencies.

## Risks

- The wrapper all-zero config includes `tx_size`; using the wrong enum would
  mis-size the all-zero state update. Tests should continue to prove the
  minimal fixture hash/raw/Y4M bytes and block-symbol frontier pass unchanged.
- A future reviewer may misread this as broad runtime `coeffs()` support.
  Tracking notes and decoder support rows must say only the all-zero runtime
  entry is wired.
