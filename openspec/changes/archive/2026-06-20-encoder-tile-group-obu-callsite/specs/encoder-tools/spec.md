## ADDED Requirements

### Requirement: Encoder writer-input minimal-intra tile-group OBU payload assembler

The encoder writer bridge SHALL provide a public `splot-core` function, tracked by
`ENC-MINIMAL-INTRA-TILE-GROUP-OBU`, that assembles a § 5.19 `tile_group_obu()` payload for
the frozen 64x64 single-picture `OBU_CLOSED_LOOP_KEY` intra tier from caller-supplied coded
tile bytes, without a parsed `SequenceHeader`. It SHALL build the matched
`(FrameHeaderCore, CoreSeqView)` via the parse-backed assembler, frame the tile bytes as the
single (last) tile of the first tile group, and drive `write_tile_group_obu`. The returned
bytes SHALL be the `tile_group_obu()` payload (embedded frame header + § 5.20.1 tile framing
+ tile data) and SHALL NOT include the § 5.2.2 OBU header / size wrapper. It SHALL NOT claim
a complete spec-conformant coded tile, a frame, a packet, or `Context::receive_packet`
output.

#### Scenario: The assembler emits a round-trippable first tile-group payload

- **WHEN** `encode_minimal_intra_clk_tile_group_obu` is called with at least one coded tile
  byte
- **THEN** the returned payload SHALL reparse as a first tile group
  (`is_first_tile_group`, `frame_header_present_flag`, and an embedded frame header all
  present)
- **AND** the coded tile bytes SHALL be the byte-aligned trailing region of the payload (the
  lone last tile reads no size field).

#### Scenario: An empty tile is rejected

- **WHEN** `encode_minimal_intra_clk_tile_group_obu` is called with empty `tile_data`
- **THEN** it SHALL return a typed `Write` error (the § 8.2.2 zero-size-tile defect), not
  panic.

#### Scenario: The bridge does not produce packets

- **WHEN** the assembler is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim an OBU header/size wrapper, a complete
  coded tile, a frame, a packet, or Baseline Encoder Profile v1 output from it.
