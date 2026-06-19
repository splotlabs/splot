## Why

The block-symbol trace vocabulary is rich, but the only path that drives the §8.2
`SymbolEncoder` to bytes is the `roundtrip_block_symbol_trace` TEST helper. To pivot
toward an actual packet, the encoder needs a production entropy-coding entry point:
`trace -> coded bytes`. This is the first brick of the tile-body assembly phase
(toward Milestone A); the tile-group payload/OBU wrapper and `Context::receive_packet`
wiring build on it.

## What Changes

- Add `ENC-BLOCK-SYMBOL-ENCODE` as a private `splot-encode` encoder-tool feature.
- Add `encode_block_symbol_trace(trace) -> Result<Vec<u8>>` in `block_symbol_trace`:
  drive the §8.2 `SymbolEncoder` over the trace (one token per scoped default CDF row;
  a bypass literal writes its raw bits), `finish()` (§8.2.4 padding), and return the
  coded bytes — the bytes a §5.20.1 `tile_group_payload()` carries as a single tile's
  data.
- Refactor the `roundtrip_block_symbol_trace` test helper to call the new function for
  its encode half (DRY; the decode half is unchanged).
- Prove the production function emits non-empty, decodable §8.2 bytes for the complete
  all-zero intra block. No tile-group payload, OBU, frame, or packet yet.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the production block-symbol entropy-coding
  entry point.

## Impact

- Affected code: `crates/splot-encode/src/block_symbol_trace.rs`,
  `crates/splot-encode/src/block_symbol_trace_tests.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none (no new `splot-decode` edge; cross-tool decode checks live
  later at CLI/integration level, per the dependency graph).
- Validator/CLI impact: none.
