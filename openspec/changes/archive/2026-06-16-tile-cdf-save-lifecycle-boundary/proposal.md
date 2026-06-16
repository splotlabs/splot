## Why

The current tile CDF boundary can copy default rows into tile-local state and
read/update selected rows, but successful tile completion does not yet expose a
transactional Tile-to-Saved-to-Frame CDF lifecycle boundary for the supported
subset. Phase 3 needs this before expanding beyond the one-tile minimal trace,
because AV2 ties `exit_symbol()` and frame-end CDF update to successful tile
syntax processing.

## What Changes

- Add Feature ID `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`.
- Add a crate-private lifecycle boundary for the currently supported
  partition/minimal-block CDF subset only.
- Apply final tile CDF rows to saved CDF rows only after successful
  `exit_symbol()` for the tile.
- Add subset `frame_end_update_cdf` behavior that copies saved rows into frame
  rows and scales row use counts per AV2 § 7.5.
- Preserve transactional failure behavior: symbol mismatch, symbol/CDF parse
  errors, and `exit_symbol()` failures must not mutate saved or frame CDF
  state.
- Keep minimal runtime hash/Y4M bytes unchanged.

## Capabilities

### New Capabilities
- `tile-cdf-save-lifecycle-boundary`: Crate-private AV2 tile CDF save/frame-end
  lifecycle boundary for the supported CDF subset.

### Modified Capabilities
- `decoder-support`: Track the new lifecycle boundary and narrow the existing
  CDF-selection notes without claiming full § 8.3 CDF coverage.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/cdf.rs`,
  `crates/splot-decode/src/tile_payload/runtime_frontier.rs`, and focused tests
  under `crates/splot-decode/src/tile_payload/cdf/`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs, and OpenSpec
  decoder-support requirements.
- Diagnostics: no new public diagnostic rule is expected; failures continue to
  surface through existing `decode/unsupported-feature`,
  `decode/resource-limit`, or `decode/malformed-source` paths.
- Dependencies: no new third-party dependencies and no AVM/dav2d integration.
