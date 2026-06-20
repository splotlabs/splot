## ADDED Requirements

### Requirement: Encoder temporal-delimiter OBU primitive

The encoder writer bridge SHALL provide a public `splot-core` function, tracked by
`ENC-TEMPORAL-DELIMITER-OBU`, that emits a standalone `OBU_TEMPORAL_DELIMITER` (§ 5.5) in
Annex B framing (§ B.2): a `leb128` size prefix, the no-extension § 5.2.2 header (inferred
`obu_mlayer_id == 0`, global `obu_xlayer_id == 31`), and an empty payload. It SHALL NOT claim
a sequence header, a temporal unit, an IVF stream, a packet, or `Context::receive_packet`
output.

#### Scenario: The primitive emits a round-trippable temporal delimiter

- **WHEN** `encode_temporal_delimiter_obu` is called
- **THEN** the result SHALL be the canonical two bytes `[0x01, 0x08]`
- **AND** parsing it as an Annex B bitstream SHALL yield exactly one OBU with no error, header
  type `OBU_TEMPORAL_DELIMITER`, no extension, and an empty payload.

#### Scenario: The bridge does not produce packets

- **WHEN** the primitive is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a sequence header, a temporal unit, an
  IVF stream, a packet, or Baseline Encoder Profile v1 output from it.
