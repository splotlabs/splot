## Why

The `useFsc` branch selector now exists, but it still accepts a caller-supplied
boolean for the AV2 section 5.20.7.27 condition. This change removes that next
contradictory-fact surface by deriving `useFsc` from the caller-resolved syntax
facts that the spec names, while keeping runtime `coeffs()` integration out of
scope.

## What Changes

- Add Feature ID `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF`.
- Add a crate-private loaded-but-unwired wrapper in `splot-decode` that derives
  `use_fsc = enable_fsc && plane_tx_type == IDTX && plane == 0 && (fsc_mode || is_inter)`
  for decoded nonzero coefficient blocks.
- Preserve AV2 ordering by continuing to route decoded all-zero inputs through
  the ordinary all-zero branch without requiring or evaluating `useFsc` condition
  facts.
- Delegate the derived nonzero condition into the existing
  `apply_coeff_use_fsc_branch` selector.
- Add focused tests for all-zero bypass, true/false derived condition cases, and
  non-selected contradictory fact preservation.
- Update implementation matrix, decoder support matrix, roadmap, generated
  status docs, decoder conformance coverage metadata, and the audit ledger.
- Non-goals: runtime `coeffs()` wiring, full `compute_tx_type`, derivation of
  `PlaneTxType`, `is_inter`, `fsc_mode`, `enable_fsc`, or `coeff_cdf_q_ctx`,
  dequantization, inverse transform, residual add, reconstruction/output,
  reference refresh, encoder changes, dependency graph changes, and AVM/dav2d
  invocation.

## Capabilities

### New Capabilities

- `coeff-use-fsc-condition-handoff`: crate-private loaded-but-unwired derivation
  of the AV2 `useFsc` condition before the existing coefficient branch selector.

### Modified Capabilities

- `decoder-support`: extend staged coefficient decode support with a partial
  row for the derived `useFsc` condition handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`, focused
  coefficient branch tests, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; the helper remains crate-private and loaded-but-unwired.
- Diagnostics impact: none; runtime validation diagnostics remain unchanged
  because the runtime `coeffs()` loop still does not call this wrapper.
- Dependencies and licensing: no new dependencies and no licensing changes.
