## Why

The ordinary coefficient branch still requires callers to pass the duplicated
`plane_type`/`ptype` fact even though AV2 derives it directly from `plane` in
section 5.20.7.27. Lifting that derivation to the branch boundary removes one
more staged caller fact before runtime `coeffs()` wiring.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-PLANE-TYPE-HANDOFF`.
- Add a crate-private ordinary branch wrapper that accepts `plane`, derives
  `plane_type = usize::from(plane > 0)`, and delegates to the existing
  `PlaneTxType` branch handoff.
- Keep the all-zero branch unchanged and derive `plane_type` only for the
  nonzero ordinary path.
- Add focused equivalence tests for luma and chroma planes plus all-zero
  preservation.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities
- `coeff-ordinary-branch-plane-type-handoff`: Branch-level ordinary coefficient
  handoff that derives `plane_type`/`ptype` from caller-resolved `plane`.

### Modified Capabilities
- `decoder-support`: Record the new branch-level plane-type handoff row and
  proof.

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
