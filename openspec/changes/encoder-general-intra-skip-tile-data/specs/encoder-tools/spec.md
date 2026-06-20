## ADDED Requirements

### Requirement: Encoder general-intra DC skip tile_data bytes

The encoder SHALL finalize the general-intra DC skip-block symbol trace into its AV2 § 8.2.4
`tile_data` bytes, tracked by `ENC-GENERAL-INTRA-SKIP-TILE-DATA`. These bytes SHALL be the
entropy-coded payload a single-tile general intra frame carries directly — consumed by the
decoder from byte 0 via § 8.2.2 `init_symbol` with no structural prefix. The function SHALL
NOT claim a tile-group OBU, a frame, a packet, `Context::receive_packet` output, or a decode;
container assembly and the cross-crate decode oracle are later bricks.

#### Scenario: The skip tile_data is the proven trace's finalized bytes

- **WHEN** the general intra DC skip `tile_data` is emitted
- **THEN** the bytes SHALL be non-empty
- **AND** they SHALL equal the § 8.2.4-finalized bytes of the brick-2 skip-block trace, which
  round-trips through one § 8.2 coder to `[0, 0, 0, 0, 1, 1, 1]`
- **AND** emission SHALL be deterministic.

#### Scenario: The bridge does not produce packets

- **WHEN** the skip `tile_data` emitter is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a tile-group OBU, a frame, a packet, or
  Baseline Encoder Profile v1 output from it.
