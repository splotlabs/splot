## Why

The minimal runtime hash/Y4M path still replays the first tile partition symbol
by hand, even though the decoder now has a spec-cited partition traversal
frontier. Wiring that runtime trace through the frontier reduces duplicated
§5.20.3.1 / §8.3 behavior while keeping the supported tier narrow and honest.

Primary Feature ID: `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.
Reused Feature ID: `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`.

## What Changes

- Add a narrow runtime bridge that uses the crate-private
  `tile-partition-traversal-boundary` to consume the minimal tier's root
  partition decision and reach the first `decode_block()` frontier.
- Resume the existing minimal flat-tile symbol trace after the frontier by
  carrying the live §8.2 symbol-decoder cursor returned by the traversal bridge,
  then continue checking only the already-supported traced intra/block symbols.
- Keep `decode_block()` syntax, `MiSizes` mutation, recursive tile traversal,
  reconstruction beyond the existing flat-frame fixture, CDF copyback/averaging,
  reference refresh, broad hash/Y4M output, and public tile APIs out of scope.
- Update decoder support and implementation matrix notes so this is recorded as
  a runtime consumer of the existing frontier, not broad `decode_tile()` support.

## Capabilities

### Modified Capabilities
- `decoder-support`: Record the runtime bridge and keep broad tile-payload,
  CDF lifecycle, block syntax, and reconstruction rows partial.
- `tile-partition-traversal-boundary`: State that the frontier can be consumed
  by the minimal runtime trace without promoting full `decode_tile()` support.

## Impact

- Affected code: `crates/splot-decode/src/runtime_minimal.rs` and, if needed,
  small crate-private adapters in
  `crates/splot-decode/src/tile_payload/partition_traversal.rs`.
- Affected tests: focused `splot-decode` runtime hash/Y4M tests plus
  partition-frontier tests proving checkpoint/resume behavior and unchanged
  out-of-tier failures.
- Affected docs: `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  decoder support/status output, and OpenSpec deltas.
- Validator/user impact: no validator behavior change and no new public decoder
  diagnostic category. Existing minimal hash/Y4M success remains the only public
  success tier.
- Dependencies: no new dependencies and no AVM/dav2d integration.
