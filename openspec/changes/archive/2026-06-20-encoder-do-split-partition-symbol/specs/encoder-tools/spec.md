## ADDED Requirements

### Requirement: Encoder root do_split partition symbol

The encoder block-symbol trace SHALL emit the AV2 § 5.20.4.1 `do_split == false`
(`PARTITION_NONE`) partition symbol for the root 64x64 superblock, tracked by
`ENC-DO-SPLIT-PARTITION-SYMBOL` — the first symbol the AVM-validated general intra decode
path reads. It SHALL be coded against `TileDoSplitCdf[plane_start 0][ctx 12]` and compose
through the existing block-symbol-trace § 8.2 coder. It SHALL NOT claim a full block trace, a
tile, a frame, a packet, or `Context::receive_packet` output.

#### Scenario: The root do_split symbol round-trips

- **WHEN** the root `do_split == false` token is composed into a one-token block-symbol trace
- **THEN** it SHALL be a `PARTITION_NONE` token (symbol 0) selecting
  `TileDoSplitCdf[plane_start 0][ctx 12]`
- **AND** round-tripping it through one § 8.2 coder SHALL decode to `[0]` with
  `symbol_count == 1`.

#### Scenario: The bridge does not produce packets

- **WHEN** the partition emitter is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a tile, a frame, a packet, or Baseline
  Encoder Profile v1 output from it.
