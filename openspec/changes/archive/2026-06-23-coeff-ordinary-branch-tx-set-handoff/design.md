## Context

The ordinary non-FSC coefficient branch now derives geometry, transform-size
dimensions, adjusted base-context dimensions, `txSzCtx`, scan order, and the
non-lossless intra chroma non-directional `PlaneTxType` subset. That most recent
wrapper still takes `txSet` as a caller-resolved fact even though AV2 §5.20.8.3
defines `get_tx_set(txSz, plane)` in terms of generated transform-size
conversion tables and frame/block flags.

The full route to runtime `coeffs()` is still broader than this brick. Frame
state must eventually provide `reduced_tx_set` and `enable_chroma_dctonly`,
runtime syntax must route luma/inter/lossless branches, and `compute_tx_type`
must cover directional wide-angle mapping plus luma/inter transform state. This
change deliberately adds only the `txSet` derivation layer that can delegate to
the existing `Mode_To_Txfm` subset.

## Goals / Non-Goals

**Goals:**

- Add a loaded-but-unwired ordinary branch wrapper that derives AV2 §5.20.8.3
  `txSet` before delegating to `apply_coeff_ordinary_branch_from_mode_to_txfm`.
- Use generated §9.2 `Tx_Size_Sqr`, `Tx_Size_Sqr_Up`, `Tx_Width`, and
  `Tx_Height` conversion tables through the existing checked table helpers.
- Preserve all-zero behavior without requiring `txSet`, `UVMode`, or reduced-set
  facts.
- Reject invalid `reduced_tx_set` and invalid transform-size table domains before
  mutating coefficient context state, tile CDF rows, or symbol-decoder state.
- Keep the existing caller-resolved `txSet` path available for staged tests and
  future full `compute_tx_type` work.

**Non-Goals:**

- No full implementation of §5.20.7.29 `compute_tx_type`.
- No frame-header or block-syntax derivation of `reduced_tx_set`,
  `enable_chroma_dctonly`, `UVMode`, parity hiding, TCQ, or lossless facts.
- No luma `TxTypes` state, inter chroma luma lookup, lossless transform override,
  or directional `wide_angle_mapping`.
- No runtime `coeffs()` wiring, output changes, dequantization, reconstruction,
  reference refresh, public API changes, or external decoder invocation.

## Decisions

1. Add a new wrapper above `Mode_To_Txfm`.

   Existing staged tests need a caller-resolved `txSet` path for unsupported
   subsets and future full transform-type work. A new wrapper proves
   `get_tx_set` independently while preserving lower handoff APIs.

2. Reuse existing transform-size table validation helpers.

   The tx-size wrapper already validates generated conversion-table domains
   through `CoeffOrdinaryTxSizeTables`. Extending that internal table bundle to
   derive `txSet` avoids a second validation path and keeps fail-atomic ordering
   before delegation.

3. Validate `reduced_tx_set` as caller-resolved frame state.

   AV2 models `reduced_tx_set` as an f(2) value. The wrapper treats values above
   three as out of domain before evaluating the branch so staged callers cannot
   accidentally test impossible frame state.

4. Keep broad `compute_tx_type` unsupported.

   Deriving `txSet` does not by itself support luma, inter, lossless,
   directional wide-angle, or runtime syntax dispatch. The implementation,
   matrix, support rows, and docs must call this out explicitly.

## Risks / Trade-offs

- Wrong enum ordinal for `TX_16X16` / `TX_32X32` -> Mitigation: use local
  constants only for the spec comparisons and prove representative square,
  rectangular, and reduced-set branches in tests.
- Overclaiming runtime support -> Mitigation: keep the wrapper crate-private and
  loaded-but-unwired, with matrix/support notes listing runtime residuals.
- Additional wrapper layering -> Mitigation: follow the existing staged
  ordinary-branch pattern and compare behavior against the lower explicit
  `Mode_To_Txfm` path.
