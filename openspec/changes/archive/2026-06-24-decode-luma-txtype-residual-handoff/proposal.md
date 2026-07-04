## Why

The live local decoder mission probe now reaches a structured
`unsupported_dctonly_residual_luma_tx_type` diagnostic after the transform-record
residual handoff. The next decoder brick should consume the observed active
luma transform-type syntax for the syntax-only Wiener NS LR path instead of
stopping at the DCT-only residual frontier.

## What Changes

- Add Feature ID `DECODE-LUMA-TXTYPE-RESIDUAL-HANDOFF`.
- Resolve the parsed luma `PlaneTxType` from AV2 §5.20.8.2 / §5.20.8.3 and
  thread it into the existing staged ordinary coefficient branch for LR
  tx-skip record derivation.
- Derive scan order and coefficient contexts from the actual transform class
  while keeping reconstruction-safe callers fail-closed.
- Keep inverse transforms, residual addition, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, and successful local decoder mission
  decode as explicit non-goals.

## Capabilities

### New Capabilities

- `luma-txtype-residual-handoff`: Covers syntax-only LR handoff for
  active non-DCT luma transform types in the local decoder mission transform-record
  residual path.

### Modified Capabilities

- `decoder-support`: Records the new partial decoder-support row and the live
  local decoder mission probe evidence after the former DCT-only frontier advances.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/general_intra_residual.rs`
  and focused tests.
- Affected tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs, and OpenSpec.
- No new dependencies, crate-graph changes, unsafe code, public API changes, or
  encoder behavior changes.
