## Why

The state-backed ordinary coefficient branch still requires callers to provide
`txClass` directly even after the max-level layer can derive it from
`PlaneTxType`. Lifting that derivation to the ordinary branch boundary removes
one more staged transform fact before the eventual runtime `coeffs()` wiring.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF`.
- Add a crate-private ordinary branch wrapper that accepts caller-resolved
  `PlaneTxType`, derives `txClass` with the existing AV2 section 8.3.2 helper,
  and delegates to the current state-backed ordinary branch handoff.
- Keep the all-zero branch unchanged and only derive `txClass` for the nonzero
  ordinary path.
- Add focused equivalence tests for vertical, horizontal, 2D, and out-of-range
  `PlaneTxType` values.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-tx-class-handoff`: Branch-level ordinary coefficient
  handoff that derives `txClass` from caller-resolved `PlaneTxType`.

### Modified Capabilities

- `decoder-support`: Record the new branch-level transform-class handoff row
  and proof.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass.rs`
  and focused coefficient-loop tests.
- Affected docs/tracking: implementation matrix, decoder support matrix,
  decoder conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, output behavior, AVM/dav2d
  invocation, `compute_tx_type`, scan derivation, runtime `coeffs()` wiring,
  dequantization, reconstruction, or broad `decode_block()` / `decode_tile()`
  support is added.
