## ADDED Requirements

### Requirement: Encoder writer-input single-tile framing constructor

The encoder writer bridge SHALL provide a public `splot-core` constructor for the
conformant AV2 § 5.20.1 single-tile `TileGroupFraming`, tracked by
`ENC-WRITER-INPUT-FRAMING`, so an encoder with a tile's § 8.2 coded bytes can build
the framing input that `write_tile_group_payload` / `write_tile_group_obu` require
(those models are otherwise `#[non_exhaustive]` and parse-only). The constructor SHALL
return the defect-free framing for a first single-tile tile group (`TileNum 0`, no
`tile_size_minus_1` field, coded region from offset 0), equal to what
`parse_tile_group_framing(payload, 0, 0, _, false)` reproduces. It SHALL NOT provide a
multi-tile framing constructor, a tile-group structure / frame-header / sequence-header
constructor, a tile-group OBU, a frame, a packet, or `Context::receive_packet` output.

#### Scenario: The constructor matches the parser

- **WHEN** the single-tile framing constructor is called with a tile size
- **THEN** the result SHALL be value-equal to the framing
  `parse_tile_group_framing` yields for a single-tile region of that size
  (`defect == None`, one tile, `TileNum 0`, no size-field offset, `tileSize` = the
  size).

#### Scenario: A write then reparse round-trips

- **WHEN** the constructed single-tile framing is written via
  `write_tile_group_payload` with the tile's coded bytes
- **THEN** the emitted region SHALL be byte-exact with the coded bytes (a single tile
  writes no size field)
- **AND** reparsing the region SHALL be value-equal to the constructed framing.

#### Scenario: The bridge does not produce packets

- **WHEN** the single-tile framing constructor is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a tile-group OBU, frame, packet,
  or Baseline Encoder Profile v1 output from the constructor alone.
