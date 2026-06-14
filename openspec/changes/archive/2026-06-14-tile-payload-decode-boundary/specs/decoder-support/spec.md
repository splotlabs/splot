## ADDED Requirements

### Requirement: Tile payload decode boundary

The decoder support model SHALL provide a source-backed tile payload decode
boundary tracked by Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY` and decoder
support matrix row `tile-payload-decode`. The boundary SHALL consume bounded
tile-group payload framing metadata derived from AV2 § 5.20.1 and SHALL hand
each eligible non-bridge tile byte slice to the AV2 § 8.2 symbol-decoder
initialization boundary before stopping at the unsupported `decode_tile()` /
§ 8.3 syntax-element CDF-selection boundary. The boundary SHALL return structured
`decode/unsupported-feature` metadata for unsupported runtime tile syntax and
SHALL initially support only deterministic planning for the minimal single-tile
closed-loop-key boundary. It SHALL NOT claim multi-tile or multi-tile-group
runtime support, § 5.20.2-§ 5.20.10 block syntax, § 8.3 CDF bank ownership,
`exit_symbol()` validation after real block syntax, CDF copyback/averaging,
reconstruction, decoded-frame hashes, runtime Y4M output, reference refresh, or
AVM/dav2d execution support.

#### Scenario: Tile boundary enforces resource limits

- **WHEN** the tile payload boundary is asked to inspect a tile group with a
  tile count or tile payload byte count above the configured `DecodeLimits`
- **THEN** it fails before unbounded iteration, allocation, or symbol-decoder
  handoff
- **AND** the failure is represented as a typed decode resource-limit error
  that can render as `decode/resource-limit`

#### Scenario: Non-bridge tile reaches unsupported decode_tile boundary

- **WHEN** the boundary receives a non-bridge tile with a valid nonzero
  `tileSize` byte slice from § 5.20.1 framing
- **THEN** it bounds the slice to the framed tile bytes and verifies the
  AV2 § 8.2 `init_symbol(tileSize)` handoff point for that slice
- **AND** it stops before block syntax with `decode/unsupported-feature`
  metadata citing spec section `5.20.2.1`, matrix row
  `tile-payload-decode`, and Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY`
- **AND** it does not reconstruct pixels, compute decoded-frame hashes, write
  Y4M output, run `exit_symbol()`, update CDF banks or reference frames, or
  invoke external decoders

#### Scenario: Minimal tier yields deterministic tile work unit

- **WHEN** the boundary is invoked for a selected base-layer closed-loop-key
  frame candidate with a complete intra first tile group, one tile, one tile
  group, and a bounded nonzero payload
- **THEN** it returns one deterministic tile work unit containing source kind,
  OBU index/offset, optional IVF frame context, selected layer, tile number,
  payload byte offset, and payload byte length
- **AND** the same work-unit metadata is produced for thread policies `auto`,
  `1`, and a fixed positive worker count when reached through `DecodeContext`

#### Scenario: Unsupported bridge or inactive tile path is explicit

- **WHEN** the boundary is asked to process multiple tiles, multiple tile groups,
  bridge, BRU-inactive, inter-only, non-first tile group, missing complete frame
  facts, or otherwise non-minimal-tier tile behavior
- **THEN** it returns structured `decode/unsupported-feature` metadata instead
  of silently treating the tile as a normal intra non-bridge tile
- **AND** the diagnostic identifies the unsupported tile payload boundary rather
  than the generic CLI runtime stub

#### Scenario: Symbol exit and CDF copyback are deferred

- **WHEN** the boundary reaches the point where AV2 § 5.20.1 would run
  `decode_tile()`, `exit_symbol()`, `frame_end_update_cdf()`, or
  `decode_frame_wrapup()`
- **THEN** it records those operations as unsupported residuals rather than
  mutating CDF banks, output state, or reference state
- **AND** tests prove this deferral without requiring AVM or dav2d

#### Scenario: Runtime decode remains unsupported outside the boundary

- **WHEN** `splot decode` is run on any stream after this change
- **THEN** the CLI still follows the existing plan-only unsupported behavior
  unless a later OpenSpec change wires the tile payload boundary into a full
  runtime decode path
- **AND** no AVM, dav2d, ffmpeg, or external decoder is located or invoked by
  repo code, tests, `xtask`, or CI
