## ADDED Requirements

### Requirement: First decodable minimal intra skip IVF

`splot-encode` SHALL emit a complete, decodable AV2 IVF stream for one 64x64 all-intra
`OBU_CLOSED_LOOP_KEY` DC skip frame, tracked by `ENC-MINIMAL-INTRA-SKIP-IVF`, via
`emit_minimal_intra_skip_ivf()`. The stream SHALL pair the general-intra DC skip `tile_data`
with the `base_q_idx`-80 minimal CLK container, and decoding it with `splot-decode` SHALL
reconstruct a flat frame (every sample the § 7.13.2 no-neighbour DC predictor). This is the
encoder's first decodable output; it is not a general encoder, a `Context::receive_packet`
packet, or Baseline Encoder Profile v1.

#### Scenario: The emitted skip stream decodes to a flat frame

- **WHEN** `emit_minimal_intra_skip_ivf()` produces an IVF and `splot decode --output-format
  raw` decodes it
- **THEN** decoding SHALL succeed
- **AND** the decoded frame SHALL be 6144 bytes (8-bit 4:2:0 64x64)
- **AND** every sample SHALL be `128` (the flat no-neighbour DC reconstruction of the skip
  block).

#### Scenario: The emitted stream is a single-frame AV02 IVF

- **WHEN** `emit_minimal_intra_skip_ivf()` produces an IVF
- **THEN** the bytes SHALL parse as exactly one frame in an `AV02` 64x64 IVF
- **AND** emission SHALL be deterministic.

#### Scenario: The bridge does not produce packets

- **WHEN** the skip IVF emitter is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a general encoder or Baseline Encoder
  Profile v1 output from it.
