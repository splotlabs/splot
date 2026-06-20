## ADDED Requirements

### Requirement: Encoder writer-input single-tile structure constructor

The encoder writer bridge SHALL provide a public `splot-core` constructor for the
AV2 § 5.19 single-tile first-tile-group `TileGroupStructure`, tracked by
`ENC-WRITER-INPUT-STRUCTURE`, so an encoder can build the structure input that
`write_tile_group_obu` requires (the model is otherwise `#[non_exhaustive]` and
parse-only). The constructor SHALL return a structure for `NumTiles == 1` with
`tile_start_and_end_present_flag` inferred `0`, `tg_start = 0`, `tg_end = 0`, and
`outcome == Complete`, leaving the writer-ignored byte-accounting (`header_bytes` /
`payload_size`) `None`. It SHALL NOT provide a multi-tile / continuation structure, a
sequence-header / frame-header constructor, a tile-group OBU, a frame, a packet, or
`Context::receive_packet` output.

#### Scenario: The constructor has the canonical single-tile fields

- **WHEN** the single-tile first-group structure constructor is called
- **THEN** the result SHALL have `tile_start_and_end_present_flag == false`,
  `tg_start == 0`, `tg_end == 0`, `outcome == Complete`, and `header_bytes` /
  `payload_size` `None`.

#### Scenario: The structure round-trips through the writer

- **WHEN** the constructed structure is written via `write_tile_group_structure` for a
  `NumTiles == 1` layout and the emitted bits are reparsed
- **THEN** the reparsed structure SHALL have the same `tile_start_and_end_present_flag`,
  `tg_start`, and `tg_end`, and SHALL be `Complete`.

#### Scenario: The bridge does not produce packets

- **WHEN** the single-tile structure constructor is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a tile-group OBU, frame, packet,
  or Baseline Encoder Profile v1 output from the constructor alone.
