## ADDED Requirements

### Requirement: Encoder minimal-intra IVF temporal-unit assembler

The encoder writer bridge SHALL provide a public `splot-core` function, tracked by
`ENC-MINIMAL-INTRA-IVF`, that assembles the frozen-tier minimal-intra temporal unit as one
IVF stream: the `OBU_TEMPORAL_DELIMITER`, `OBU_SEQUENCE_HEADER`, and `OBU_CLOSED_LOOP_KEY`
Annex B OBUs concatenated in the decoder-required order, inside one `AV02` 64x64 IVF frame.
The sequence header and frame header SHALL be consistent (both the frozen 64x64 single-picture
`Block64x64` tier). It SHALL NOT claim a decode-hash match to the conformance vector, a
complete coded tile, a packet, or `Context::receive_packet` output.

#### Scenario: The assembler emits a consistent IVF temporal unit

- **WHEN** `encode_minimal_intra_clk_ivf` is called with at least one coded tile byte
- **THEN** the result SHALL be a valid `AV02` 64x64 IVF with exactly one frame whose Annex B
  payload reparses as `[OBU_TEMPORAL_DELIMITER, OBU_SEQUENCE_HEADER, OBU_CLOSED_LOOP_KEY]`
- **AND** `CoreSeqView::from_sequence` of the sequence header SHALL be the frozen 64x64
  single-picture `Block64x64` tier the frame header was built against.

#### Scenario: An empty tile is rejected

- **WHEN** `encode_minimal_intra_clk_ivf` is called with empty `tile_data`
- **THEN** it SHALL return the typed `Frame` error, not panic.

#### Scenario: The bridge does not produce packets

- **WHEN** the assembler is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a decode-hash match, a complete coded
  tile, a packet, or Baseline Encoder Profile v1 output from it.
