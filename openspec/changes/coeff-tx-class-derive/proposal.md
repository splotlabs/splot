## Why

The ordinary coefficient handoff still accepts `txClass` as a caller-resolved
fact, even though AV2 defines the `PlaneTxType -> txClass` mapping directly in
§ 8.3.2 and uses it immediately after `compute_tx_type()` in § 5.20.7.27.
Loading this small derivation in `splot-decode` removes one staged caller fact
without importing `splot-recon` into the entropy loop.

## What Changes

- Add Feature ID `DECODE-COEFF-TX-CLASS-DERIVE`.
- Add a crate-private, total decode-local helper that maps caller-resolved
  `PlaneTxType` values to the ordinary coefficient transform-class enum.
- Add a max-level handoff that accepts `PlaneTxType`, derives `txClass`, and
  delegates to the existing `maxLevel` derivation.
- Add focused tests for vertical, horizontal, 2D, identity, and out-of-range
  `PlaneTxType` values plus equivalence with the existing direct `txClass`
  path.
- Update decoder tracking, roadmap, and generated status/coverage docs.

## Capabilities

### New Capabilities
- `coeff-tx-class-derive`: Decode-local `PlaneTxType -> txClass` derivation for ordinary coefficient syntax.

### Modified Capabilities
- `decoder-support`: Record the new coefficient transform-class derivation row and proof.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/max_level.rs`
  and focused coefficient-loop tests.
- Affected docs/tracking: implementation matrix, decoder support matrix,
  decoder conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, output behavior, AVM/dav2d
  invocation, `compute_tx_type`, scan derivation, dequantization, reconstruction,
  or broad `decode_block()` / `decode_tile()` support is added.
