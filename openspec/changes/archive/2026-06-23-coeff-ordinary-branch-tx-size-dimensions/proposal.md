## Why

The ordinary coefficient branch now derives block geometry from caller-provided
`coeffs()` dimensions, but staged callers still provide `Tx_Width[txSz]`,
`Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`, and `Tx_Height_Log2[txSz]` by hand.
Those facts already exist as generated AV2 § 9.2 conversion tables, so the next
handoff should derive them from `txSz` before runtime `coeffs()` integration.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS`.
- Add a crate-private ordinary branch wrapper that accepts `plane`, `startX`,
  `startY`, and `txSz`, derives `Tx_Width[txSz]`, `Tx_Height[txSz]`,
  `Tx_Width_Log2[txSz]`, and `Tx_Height_Log2[txSz]` from the generated
  `splot-core` conversion tables, and delegates to the existing `coeffs()`
  geometry handoff.
- Keep the remaining caller-resolved facts explicit, including `txSzCtx`,
  `PlaneTxType`, scan order, parity hiding, TCQ, lossless state, and
  coefficient-CDF q context.
- Reject invalid `txSz` indices before mutating coefficient context state,
  tile CDFs, or the symbol decoder.
- Add focused tests proving nonzero branch equivalence, all-zero preservation,
  and invalid-`txSz` fail-atomic behavior.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities
- `coeff-ordinary-branch-tx-size-dimensions`: Branch-level ordinary coefficient
  handoff that derives generated transform-size width/height dimensions from
  `txSz` before the existing `coeffs()` geometry handoff.

### Modified Capabilities
- `decoder-support`: Record the new branch-level transform-size dimension
  handoff row and proof.

## Impact

- Affected code:
  `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`,
  `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass.rs`,
  `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_branch_coeffs_geometry_tests.rs`,
  and related module docs.
- Affected docs/tracking: implementation matrix, decoder support matrix, decoder
  conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, output behavior, AVM/dav2d
  invocation, `Tx_Size_Sqr` or `txSzCtx` derivation, `Adjusted_Tx_Size`
  derivation, `compute_tx_type`, scan derivation, runtime `coeffs()` wiring,
  dequantization, reconstruction, or broad `decode_block()` / `decode_tile()`
  support is added.
