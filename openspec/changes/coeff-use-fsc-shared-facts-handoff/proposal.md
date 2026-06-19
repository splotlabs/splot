## Why

The derived `useFsc` condition wrapper still requires callers to pre-build both
ordinary and FSC nonzero branch inputs, which leaves a duplicated-fact surface
above the eventual AV2 section 5.20.7.27 `coeffs()` call-site. This change adds
the next narrow handoff: one shared nonzero fact packet that derives and builds
only the selected lower branch.

## What Changes

- Add Feature ID `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF`.
- Add a crate-private loaded-but-unwired wrapper in `splot-decode` that accepts
  decoded all-zero inputs or one shared nonzero coefficient fact packet.
- For nonzero inputs, derive the AV2 section 5.20.7.27
  `use_fsc = enable_fsc && plane_tx_type == IDTX && plane == 0 && (fsc_mode || is_inter)`
  condition from the shared facts.
- Lazily construct only the selected lower branch:
  ordinary inputs from the shared geometry, CDF q-context, ordinary base config,
  and lossless facts; FSC inputs from the same geometry plus generated
  `Tx_Width[txSz]` / `Tx_Height[txSz]`, `PlaneTxType`, `is_inter`, and CDF
  q-context facts.
- Preserve all-zero ordering by continuing to route decoded all-zero inputs
  through the existing ordinary all-zero branch without requiring nonzero shared
  facts.
- Add focused tests for all-zero bypass, ordinary selected-branch equivalence,
  FSC selected-branch equivalence, selected-branch-only validation, and runtime
  no-output-change scope.
- Update implementation matrix, decoder support matrix, roadmap, generated
  status docs, decoder conformance coverage metadata, and the audit ledger.
- Non-goals: runtime `coeffs()` wiring, full `compute_tx_type`, runtime
  derivation of `enable_fsc`, `PlaneTxType`, `fsc_mode`, `is_inter`,
  `coeff_cdf_q_ctx`, transform geometry, scan inputs, dequantization, inverse
  transform, residual add, reconstruction/output, reference refresh, encoder
  changes, dependency graph changes, and AVM/dav2d invocation.

## Capabilities

### New Capabilities

- `coeff-use-fsc-shared-facts-handoff`: crate-private loaded-but-unwired
  selected-branch construction for the AV2 `useFsc` branch from one shared
  nonzero coefficient fact packet.

### Modified Capabilities

- `decoder-support`: extend staged coefficient decode support with a partial
  row for the shared-facts `useFsc` handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/`, focused
  coefficient branch tests, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and decoder conformance coverage metadata.
- Public API impact: none; the helper remains crate-private and loaded-but-unwired.
- Diagnostics impact: none; runtime validation diagnostics remain unchanged
  because the runtime `coeffs()` loop still does not call this wrapper.
- Dependencies and licensing: no new dependencies and no licensing changes.
