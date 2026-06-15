## ADDED Requirements

### Requirement: Tile-payload input derivation

`splot-decode` SHALL provide a crate-private derivation bridge that builds
tile-payload boundary input for the minimal closed-loop-key tile tier from
source-backed parser output. The bridge SHALL validate that the selected
`DecodePlannedObu` matches the borrowed `splot-core` OBU envelope and that the
envelope payload is the exact slice of the original input bytes before using any
payload bytes. It SHALL derive the § 5.19 tile-group structure itself from that
same envelope payload, then derive the § 5.20 `tile_group_payload()` byte region
only after a complete structure parse, using checked arithmetic for
`headerBytes`, payload size, payload base, and per-tile byte spans. It SHALL
derive tile-grid, frame, quantizer, and CDF update facts from `FrameHeaderCore`,
`TileInfo`, the locally parsed `TileGroupStructure`, and `TileGroupFraming`
rather than invented values. The bridge SHALL run the resulting boundary through
the context-owned `DecodeContext` worker pool. It SHALL remain crate-private and
plan-only.

#### Scenario: Single tile candidate reaches unsupported tile syntax boundary

- **WHEN** a selected closed-loop-key OBU has matching source metadata, a
  complete intra first tile-group header, a complete § 5.19 structure, a one-tile
  § 5.20 payload region, and parser-derived tile and quantizer facts
- **THEN** the bridge derives a deterministic `DecodeTilePayloadPlan`
- **AND** the plan preserves source kind, IVF context when present, OBU index,
  OBU byte offset, selected layer, tile byte span, MI range, `CurrentQIndex`, and
  the existing unsupported `decode_tile()` boundary metadata
- **AND** no public tile-payload API, reconstruction, decoded-frame hash, Y4M
  output, reference refresh, or external decoder invocation occurs

#### Scenario: Forged parser metadata is rejected before slicing

- **WHEN** the planned OBU metadata does not match the borrowed OBU envelope, the
  envelope payload is not the exact slice from the original input bytes, or the
  borrowed payload bytes do not fit the declared OBU size and source container
  bounds
- **THEN** the bridge rejects the input with a local crate-private derivation
  error before slicing tile payload bytes
- **AND** no tile work unit is retained

#### Scenario: Tile group payload region is bounded

- **WHEN** § 5.19 parsing is truncated, `headerBytes` or `payload_size` is absent,
  or `headerBytes + payload_size` does not fit inside the OBU payload
- **THEN** the bridge rejects the input without using saturating or truncating
  slicing

#### Scenario: Unsupported paths do not guess facts

- **WHEN** the frame header is not complete intra, the selected candidate is not
  the first-and-only tile group, the frame is bridge/inter/TIP/BRU-dependent,
  required `tile_info`, quantizer, or `disable_cdf_update` facts are absent, or
  the tile range is outside the minimal tier
- **THEN** the bridge stops with a local derivation error or the existing
  structured tile-boundary unsupported metadata
- **AND** it does not infer continuation state from the most recent header or
  hardcode unexposed parser facts

#### Scenario: Thread policy does not change derived boundary output

- **WHEN** the same accepted source-backed tile input is derived through
  `DecodeContext` configured with `auto`, `1`, and a fixed positive worker count
- **THEN** the returned plan metadata or local error is identical across those
  thread policies
- **AND** no direct Rayon, crossbeam, global pool, nested pool, ad-hoc thread, or
  queue usage is introduced outside `splot_parallel`

#### Scenario: Local reference tools remain outside the repo

- **WHEN** this plan-only derivation bridge is implemented and tested
- **THEN** no AVM, dav2d, ffmpeg, or other external decoder is located, invoked,
  downloaded, built, wrapped, required by tests, or added to CI
