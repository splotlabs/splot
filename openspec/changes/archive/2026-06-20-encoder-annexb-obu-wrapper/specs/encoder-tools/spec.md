## ADDED Requirements

### Requirement: Encoder writer-input minimal-intra Annex B OBU wrapper

The encoder writer bridge SHALL provide a public `splot-core` function, tracked by
`ENC-MINIMAL-INTRA-CLK-ANNEXB-OBU`, that wraps the minimal-intra `tile_group_obu()` payload
in AV2 Annex B framing (§ B.2): a `leb128` size prefix, the § 5.2.2 OBU header, then the
payload. It SHALL build the no-extension `OBU_CLOSED_LOOP_KEY` header (inferred layer ids
`0`) and drive `write_annexb_obu`. The result SHALL be a self-delimiting Annex B OBU. It
SHALL NOT claim a temporal delimiter, a sequence-header OBU, a decodable temporal unit, an
IVF stream, a complete coded tile, a packet, or `Context::receive_packet` output.

#### Scenario: The wrapper emits one round-trippable CLK OBU

- **WHEN** `encode_minimal_intra_clk_annexb_obu` is called with at least one coded tile byte
- **THEN** parsing the result as an Annex B bitstream SHALL yield exactly one OBU with no
  error, header type `OBU_CLOSED_LOOP_KEY`, and no header extension
- **AND** that OBU's payload SHALL equal the `tile_group_obu()` payload (ending in the coded
  tile bytes).

#### Scenario: An empty tile is rejected

- **WHEN** `encode_minimal_intra_clk_annexb_obu` is called with empty `tile_data`
- **THEN** it SHALL return the typed zero-size-tile `Write` error, not panic.

#### Scenario: The bridge does not produce packets

- **WHEN** the wrapper is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a temporal delimiter, a sequence
  header, an IVF stream, a complete coded tile, a packet, or Baseline Encoder Profile v1
  output from it.
