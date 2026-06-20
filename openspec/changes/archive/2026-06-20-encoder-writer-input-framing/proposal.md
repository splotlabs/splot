## Why

The encoder→packet path is blocked: `splot-core`'s tile-group/frame writer-input
models (`TileGroupFraming`, `TileGroupStructure`, `FrameHeaderCore`, `CoreSeqView`)
are `#[non_exhaustive]` with no public constructor — they are produced only by parsing
a stream, so `splot-encode` cannot hand-build them to drive `write_tile_group_obu`.
The maintainer approved building the `splot-core` writer bridge. This is the first,
smallest bridge brick: a public constructor for the § 5.20.1 single-tile
`TileGroupFraming` the encoder needs to assemble a tile-group payload.

## What Changes

- Add `ENC-WRITER-INPUT-FRAMING` as an encoder writer-input-bridge feature (code in
  `splot-core`, driven by the encoder mission).
- Add `TileGroupFraming::single_tile(tile_size) -> TileGroupFraming` in `splot-core`:
  the defect-free § 5.20.1 framing for a first single-tile tile group (`TileNum 0`, no
  `tile_size_minus_1` field, coded region from offset 0). It is the encoder-side
  inverse of `parse_tile_group_framing(payload, 0, 0, _, false)`.
- Prove the constructor reproduces exactly the parser's framing, and that a write via
  `write_tile_group_payload` then a reparse round-trips value-equal.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder writer-input single-tile
  framing constructor.

## Impact

- Affected code: `crates/splot-core/src/headers/tile_group.rs` (the constructor +
  tests). No new types, no signature changes; `#[non_exhaustive]` is preserved.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one new associated constructor on the existing public
  `splot-core` `TileGroupFraming`. No dependency-graph change (`splot-encode` already
  depends on `splot-core`).
- Validator/CLI impact: none.
