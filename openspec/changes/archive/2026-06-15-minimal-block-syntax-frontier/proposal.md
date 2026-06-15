## Why

The minimal runtime tier now reaches the first `decode_block()` frontier through the tile partition traversal, but `runtime_minimal.rs` still owns the remaining traced flat block-symbol reads directly. Moving those reads behind a crate-private tile-payload block-symbol trace frontier reduces the `tile-payload-decode`, `tile-cdf-selection-boundary`, and `symbol-decoder` partial surface without claiming broad `decode_block()` or `decode_tile()` support.

## What Changes

- Add a crate-private minimal block-symbol trace frontier that consumes only the current supported flat intra 64x64 block trace after the partition frontier.
- Preserve the existing minimal hash/Y4M success behavior, output hashes, and unsupported/resource-limit diagnostics.
- Record the new Feature ID `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` in the implementation matrix and decoder support matrix.
- Add focused tests for the successful traced block-symbol sequence and mutation/error paths.
- Keep broad `mode_info()`, transform syntax, recursive `decode_tile()`, reconstruction, CDF lifecycle, and AVM/dav2d evidence out of scope.

## Capabilities

### New Capabilities
- `minimal-block-syntax-frontier`: crate-private minimal-tier block-symbol trace frontier for the flat intra symbols after the partition frontier.

### Modified Capabilities
None.

## Impact

- Affected code: `crates/splot-decode/src/runtime_minimal.rs`, `crates/splot-decode/src/tile_payload.rs`, and new or existing `crates/splot-decode/src/tile_payload/*` modules/tests.
- Affected docs: `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status/coverage docs, and decoder roadmap notes if needed.
- No public API, CLI, dependency graph, fixture, license, or external reference-decoder integration changes are planned.
