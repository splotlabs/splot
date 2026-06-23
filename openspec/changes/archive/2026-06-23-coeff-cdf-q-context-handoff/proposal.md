## Why

The staged `useFsc` shared-facts wrapper still takes a caller-supplied
`coeff_cdf_q_ctx`, even though AV2 derives that coefficient-CDF q-context from
frame `base_q_idx` when coefficient CDFs are initialized. This change removes
that caller-resolved q-context surface before runtime `coeffs()` wiring.

## What Changes

- Add Feature ID `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF`.
- Add a crate-private loaded-but-unwired helper that derives the AV2 coefficient
  CDF q-context from `base_q_idx` using the § 6.17.2 `init_coeff_cdfs()`
  thresholds: `<= 90`, `<= 140`, `<= 190`, and `> 190`.
- Add a crate-private wrapper above the shared-facts `useFsc` handoff that
  accepts decoded all-zero inputs or one nonzero fact packet carrying
  `base_q_idx` instead of `coeff_cdf_q_ctx`.
- Preserve all-zero ordering by continuing to route decoded all-zero inputs
  through the existing ordinary all-zero path without requiring `base_q_idx`.
- Preserve selected-branch behavior by deriving `coeff_cdf_q_ctx` from
  `base_q_idx` only for nonzero inputs, then delegating to the existing
  shared-facts wrapper.
- Add focused tests for q-context threshold boundaries, all-zero bypass,
  ordinary/FSC selected-branch equivalence, selected-row effects for all four
  q-context buckets, and runtime no-output-change scope.
- Update implementation matrix, decoder support matrix, roadmap, generated
  status docs, decoder conformance coverage metadata, and the audit ledger.
- Non-goals: runtime `coeffs()` wiring, single-active-row CDF storage refactors,
  full CDF lifecycle/save/load semantics, full `compute_tx_type`, runtime
  derivation of `enable_fsc`, `PlaneTxType`, `fsc_mode`, `is_inter`, transform
  geometry, dequantization, inverse transform, residual add, reconstruction,
  output, reference refresh, encoder changes, dependency graph changes, and
  AVM/dav2d invocation.

## Capabilities

### New Capabilities

- `coeff-cdf-q-context-handoff`: crate-private loaded-but-unwired coefficient
  q-context derivation from frame `base_q_idx`, threaded into the staged
  `useFsc` shared-facts handoff.

### Modified Capabilities

- `decoder-support`: extend staged coefficient decode support with a partial
  row for deriving coefficient CDF q-context from `base_q_idx` before the
  shared-facts `useFsc` handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`, focused
  coefficient branch tests, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; helpers remain crate-private and loaded-but-unwired.
- Diagnostics impact: none; runtime validation diagnostics remain unchanged
  because the runtime `coeffs()` loop still does not call this wrapper.
- Dependencies and licensing: no new dependencies and no licensing changes.
