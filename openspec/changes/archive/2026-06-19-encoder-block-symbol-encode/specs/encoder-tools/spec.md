## ADDED Requirements

### Requirement: Encoder block-symbol entropy-coding entry point

The encoder SHALL provide a private production entry point, tracked by
`ENC-BLOCK-SYMBOL-ENCODE`, that encodes an ordered block-symbol trace into AV2 §8.2
entropy-coded bytes. It SHALL drive one in-tree `SymbolEncoder` over the trace —
writing each CDF token to its scoped default CDF row and each bypass token as a raw
literal — and `finish()` the §8.2 stream, returning the coded bytes that a §5.20.1
`tile_group_payload()` carries as a single tile's data. The block-symbol trace
roundtrip SHALL use this entry point for its encode half. It SHALL NOT assemble a
tile-group payload, a tile-group OBU, a frame, a packet, `Context::receive_packet`
output, public CLI success, or Baseline Encoder Profile v1 output.

#### Scenario: The entry point emits decodable all-zero bytes

- **WHEN** the complete all-zero intra block trace is encoded
- **THEN** the returned bytes SHALL be non-empty
- **AND** they SHALL equal the bytes the block-symbol roundtrip proves decodable, which
  decode to `[0, 0, 0, 1, 1, 1]`.

#### Scenario: The entry point is deterministic

- **WHEN** the same trace is encoded twice
- **THEN** the two byte vectors SHALL be identical.

#### Scenario: The entry point does not produce packets

- **WHEN** the block-symbol entropy-coding entry point is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a tile-group payload, OBU, frame,
  packet, or Baseline Encoder Profile v1 output from the entry point alone.
