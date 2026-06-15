## Why

The tile-payload boundary now has the ingredients for partition syntax, but it
still stops before any recursive §5.20.3.1 traversal can consume a tile. The
next decoder-conformance step is to compose the existing size-table,
allowed-set, CDF-context, and one-decision boundaries into a bounded traversal
surface without claiming full `decode_tile()` or reconstruction.

Feature ID: `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`.

## What Changes

- Add a crate-private `splot-decode` partition traversal boundary for AV2
  §5.20.3.1 that advances from a tile root to the first `decode_block()`
  frontier in the already-supported minimal intra tile tier.
- Reuse the existing partition-size table, allowed-partition derivation,
  partition-decision, symbol-read, and CDF-selection boundaries instead of
  duplicating those rules.
- Record deterministic traversal steps, pending continuation children, and the
  first `decode_block()` frontier, including coordinates, block sizes, parent
  sizes, chroma propagation, selected partition, symbol-consumption trace, and
  the symbol-decoder checkpoint before block syntax.
- Keep `decode_tile()`, `decode_block()` syntax, reconstruction, residuals,
  `MiSizes` mutation, output, reference refresh, CDF copyback/averaging, and
  public API behavior out of scope.
- Update decoder support and implementation matrices plus generated status docs
  so the new supported slice is distinct from broader `tile-payload-decode`.

## Capabilities

### New Capabilities

- `tile-partition-traversal-boundary`: Crate-private AV2 §5.20.3.1 partition
  traversal frontier over existing tile partition decision components.

### Modified Capabilities

- `decoder-support`: Track `tile-partition-traversal-boundary` as its own
  decoder support row and keep broad tile payload decode partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload*`, especially a new
  traversal module plus existing partition/CDF helpers.
- Affected docs: `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/IMPLEMENTATION-MATRIX.toml`, generated decoder support/status docs, and
  the decoder roadmap if the planned boundary changes visible backlog wording.
- Validator/user impact: no new public success claim and no new public decoder
  diagnostic is expected; unsupported `decode_tile()` remains the runtime stop
  until broader tile syntax and block reconstruction land.
- Dependencies: no new third-party dependencies and no AVM/dav2d integration.
