## Why

The tile payload pipeline can currently reach the first `decode_block()` frontier,
but it still uses synthetic read-only MI-size context and does not model the
state mutation that AV2 `decode_block()` performs for `MiSizes`, `LeftMiSizes`,
and `AboveMiSizes`.

This is the next narrow state boundary needed before broad `read_partition()` /
`decode_tile()` traversal can advance safely beyond the first block frontier.

## What Changes

- Add Feature ID `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` for a crate-private
  `splot-decode` tile MI-size state boundary.
- Introduce bounded mutable state for luma/chroma `MiSizes`, `LeftMiSizes`, and
  `AboveMiSizes`, initialized with the same clear-context block-size sentinel
  used by the current minimal runtime frontier over superblock-padded context
  extents.
- Add a checked block-frontier update operation for the AV2 § 5.20.4.1
  luma/chroma MI-size writes, with typed errors for visible start bounds,
  padded footprint bounds, allocation, and arithmetic failures.
- Expose read-only state views back to the existing partition-context selector
  path so future traversal can consume mutated neighbor state without changing
  public APIs.
- Keep runtime decode behavior, output bytes, CDF lifecycle, reconstruction, and
  public APIs unchanged for this PR.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: add the `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` crate-private
  boundary requirement and keep broad tile decode/reconstruction claims partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/` only.
- Affected docs/status: decoder support matrix, implementation matrix, roadmap,
  generated decoder support/status coverage docs as needed.
- Diagnostics: no new emitted `decode/*` rule ID; boundary failures remain
  crate-private and map through existing unsupported/resource paths only when a
  later runtime caller exposes them.
- Dependencies: no new crates, no dependency-direction change, no AVM/dav2d
  integration, no encoder impact.
- Non-goals: full `decode_block()` syntax, recursive `read_partition()`, broad
  `decode_tile()`, transform/residual parsing, reconstruction expansion,
  reference refresh, multi-tile scheduling, public API changes, and external
  reference decoder tooling.
