## Why

The tile CDF boundary can derive the AV2 § 8.3.2 partition-entry CDF rows, but production code still lacks a named crate-private boundary for the matching § 5.20.3.2 `S()` symbol reads. Adding that boundary is the next small step toward real tile syntax traversal while keeping `read_partition()` decisions and `decode_tile()` out of scope.

Feature ID: `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY`.

## What Changes

- Add a crate-private partition-entry symbol read helper that routes supported `TileCdfSelector` values through `SymbolDecoder::read_symbol(cdf)`.
- Preserve typed failure separation between CDF selector/bounds errors and symbol-decoder errors.
- Keep CDF update behavior controlled by the caller's existing `SymbolDecoderConfig`.
- Add focused positive and negative tests for enabled/disabled updates, selector failures, and symbol/CDF validation failures.
- Update decoder support docs, implementation matrix, OpenSpec requirements, and generated status docs.

Non-goals:

- No recursive `read_partition()` traversal or partition decision mapping.
- No `decode_tile()` implementation.
- No `exit_symbol()` validation after real syntax.
- No new CDF arrays, full Tile/Saved CDF banks, reconstruction, hashes, Y4M, reference refresh, external decoder invocation, dependencies, or public APIs.

## Capabilities

### New Capabilities

- `tile-partition-symbol-read-boundary`: crate-private support for individual partition-entry `S()` symbol reads over existing derived CDF selectors.

### Modified Capabilities

- `decoder-support`: record the new narrow symbol-read boundary and refine the existing tile CDF boundary wording so it no longer says all actual partition syntax reads are out of scope.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/cdf/`.
- Affected docs/status: `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder/feature status docs, and `docs/IMPLEMENTATION-MATRIX.toml`.
- Diagnostics: no new emitted `decode/*` diagnostic; failures stay crate-private typed errors.
- Dependencies: none.
