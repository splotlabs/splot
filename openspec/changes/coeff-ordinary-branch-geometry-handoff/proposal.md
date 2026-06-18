## Why

The ordinary coefficient branch now derives `txClass` and `plane_type`, but the
same nonzero transform-block geometry is still passed twice: once in the
nonzero block start input and again in the state-context config. Deriving the
state-context geometry from the branch start removes another contradictory
caller fact before runtime `coeffs()` wiring.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-GEOMETRY-HANDOFF`.
- Add a crate-private ordinary branch wrapper that accepts nonzero branch
  inputs with state-context CDF facts only, derives `x4`, `y4`, `w4`, and `h4`
  from `NonZeroCoeffBlockStartInput.block`, and delegates to the existing
  `plane_type` handoff.
- Keep the all-zero branch unchanged and derive geometry only for the nonzero
  ordinary path.
- Add focused equivalence tests proving nonzero geometry handoff and all-zero
  preservation.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities
- `coeff-ordinary-branch-geometry-handoff`: Branch-level ordinary coefficient
  handoff that derives state-context geometry from the nonzero block start.

### Modified Capabilities
- `decoder-support`: Record the new branch-level geometry handoff row and proof.

## Impact

- Affected code:
  `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`,
  `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass.rs`, and
  focused coefficient-loop tests.
- Affected docs/tracking: implementation matrix, decoder support matrix,
  decoder conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, output behavior, AVM/dav2d
  invocation, raw `startX`/`startY`/`txSz` derivation, `compute_tx_type`, scan
  derivation, runtime `coeffs()` wiring, dequantization, reconstruction, or
  broad `decode_block()` / `decode_tile()` support is added.
