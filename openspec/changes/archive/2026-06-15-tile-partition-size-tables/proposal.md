## Why

Recursive AV2 tile partition traversal needs a bounded way to translate a
`PARTITION_*` decision and `BLOCK_*` size into the resulting sub-block size.
The spec tables for this are currently skipped by generated § 9 table support
because they contain symbolic `BLOCK_*` values, leaving later
`is_partition_allowed`, `decode_partition`, and chroma-offset work without a
typed local boundary.

## What Changes

- Add Feature ID `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY` for a
  crate-private decoder boundary over AV2 § 9.2 `Partition_Subsize` and
  `H_Partition_Midsize`.
- Add typed, bounds-checked helpers that return valid sub-block sizes or a
  typed invalid/out-of-range result without panicking.
- Add focused tests that pin representative valid entries, invalid entries,
  boundary sizes, and out-of-range table indices.
- Update the decoder support matrix, implementation matrix, roadmap/status
  docs, and OpenSpec archive artifacts to state the exact supported scope.
- Non-goals: no full or general generated symbolic table support in
  `cargo xtask gen-tables`, no full `partition_implied`, `init_allowed_partitions`,
  `is_partition_allowed`, recursive `read_partition()`/`decode_partition()`,
  `decode_tile()`, reconstruction, output, reference refresh, public API
  behavior, AVM/dav2d invocation, or dependency graph change.

## Capabilities

### New Capabilities
- `tile-partition-size-tables`: Crate-private AV2 § 9.2 partition subsize and
  horizontal midsize lookup boundary for future tile partition traversal.

### Modified Capabilities

## Impact

- Affected code: `xtask gen-tables` learns a narrow `BLOCK_*` symbol resolver
  for the two § 9.2 partition-size tables; `splot-core` generated conversion
  tables gain those arrays; `crates/splot-decode/src/tile_payload/` gains a
  focused wrapper module and tests.
- Affected docs: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-STATUS.md`, and
  `docs/IMPLEMENTATION-MATRIX.toml`.
- APIs/dependencies: no public API changes, no new dependencies, no external
  decoder integration, and no crate dependency direction change.
- Validator/diagnostics impact: none; the boundary is crate-private and returns
  typed internal errors for invalid table requests instead of emitting
  user-facing diagnostics.
