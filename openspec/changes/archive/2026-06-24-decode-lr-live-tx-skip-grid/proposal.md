## Why

The live local decoder mission LR path can now allocate explicit `CurrFrame`, `CdefFrame`, and
`LrTxSkip` storage shells, but the `LrTxSkip` shell has no way to accept the
already-proven dense grid without fabricating defaults. The next small brick is
to add that value-preserving population boundary before wiring real tile
transform records.

## What Changes

- Add Feature ID `DECODE-LR-LIVE-TX-SKIP-GRID` and matrix row
  `lr-live-tx-skip-grid`.
- Extend the private live LR storage shell so a complete
  `WienerNsLrTxSkipGrid` can populate the live `LrTxSkip` grid exactly once.
- Reject mismatched dimensions or attempted re-population as structured
  reconstruction errors instead of truncating, defaulting, or overwriting values.
- Keep the live local decoder mission runtime fail-closed before decoded frame samples,
  `FilterClass` retention, loop-restoration filtering/output, and reference
  refresh.

## Capabilities

### New Capabilities

- `lr-live-tx-skip-grid`: Covers live `LrTxSkip` grid population from a
  complete retained grid into the existing local decoder mission LR storage shell.

### Modified Capabilities

- `decoder-support`: Adds the new partial support row and preserves the
  unsupported scope for decoded sample population, filtering, output, reference
  refresh, and successful local decoder mission decode.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal/wienerns_lr/live_storage.rs`
  and focused LR live-storage tests.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, and generated decoder support/status docs.
- No public API, dependency graph, encoder, CLI, AVM/dav2d runtime, or broad AV2
  conformance behavior changes.
