## Why

`splot-decode` can now run the crate-private tile-payload boundary through
`DecodeContext`, but only tests can construct the required boundary input facts.
The next decoder-mission slice should derive those facts from source-backed
parser output with checked byte containment, while still stopping before
`decode_tile()` and runtime output.

Feature ID: `DECODE-TILE-PAYLOAD-INPUT-DERIVATION`.

## What Changes

- Add a crate-private `splot-decode` adapter that turns a selected
  closed-loop-key frame candidate plus borrowed parser output and parsed
  frame/tile-group facts into `TilePayloadBoundaryInput`.
- Validate that planned OBU metadata matches the borrowed `ObuEnvelope` before
  slicing bytes, and slice only the § 5.20 `tile_group_payload()` region after a
  complete § 5.19 structure parse.
- Expose the already-read intra `disable_cdf_update` fact from
  `splot-core::headers::frame::FrameHeaderCore`, so `splot-decode` does not
  guess CDF update mode.
- Run the derived tile-payload boundary through `DecodeContext` and its
  context-owned `splot_parallel::WorkerPool`.
- Update decoder support docs, feature tracking, generated status docs, and
  OpenSpec to record the supported crate-private derivation bridge.
- Keep runtime `splot decode` output unsupported.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: add the source-backed, crate-private tile-payload input
  derivation bridge for the minimal closed-loop-key tile boundary.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/info.rs`,
  `crates/splot-decode/src/context.rs`, a new crate-private `splot-decode`
  derivation module, and focused `splot-decode` tests.
- Affected docs/status: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder/feature/spec status,
  and `docs/IMPLEMENTATION-MATRIX.toml`.
- No public tile-payload API, no CLI decode success path, no `splot-recon`
  dependency edge, no new third-party dependencies, and no AVM/dav2d repo or CI
  integration.
