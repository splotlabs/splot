## Why

The ordinary coefficient branch now derives state-context geometry from
`NonZeroCoeffBlockStartInput.block`, but callers still have to build that block
geometry by hand. AV2 § 5.20.7.27 derives `x4`, `y4`, `w4`, and `h4` directly at
the start of `coeffs(plane, startX, startY, txSz)`, so the staged branch should
remove that contradictory caller fact next.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-COEFFS-GEOMETRY-HANDOFF`.
- Add a crate-private ordinary branch wrapper that accepts `plane`, `startX`,
  `startY`, `Tx_Width[txSz]`, and `Tx_Height[txSz]`-style caller facts, derives
  `AllZeroCoeffBlockInput { x4, y4, w4, h4 }`, and delegates to the existing
  geometry handoff.
- Keep all-zero behavior unchanged and derive block geometry for both all-zero
  and nonzero branch arms.
- Add focused equivalence tests proving nonzero handoff and all-zero
  preservation.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities
- `coeff-ordinary-branch-coeffs-geometry-handoff`: Branch-level ordinary
  coefficient handoff that derives block geometry from `coeffs()` geometry
  inputs before the existing state-context geometry handoff.

### Modified Capabilities
- `decoder-support`: Record the new branch-level `coeffs()` geometry handoff row
  and proof.

## Impact

- Affected code:
  `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`,
  `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_state_context_tests.rs`,
  and related module docs.
- Affected docs/tracking: implementation matrix, decoder support matrix, decoder
  conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, output behavior, AVM/dav2d
  invocation, `txSz` table lookup, `compute_tx_type`, scan derivation, runtime
  `coeffs()` wiring, dequantization, reconstruction, or broad `decode_block()` /
  `decode_tile()` support is added.
