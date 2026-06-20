## ADDED Requirements

### Requirement: Encoder minimal-intra sequence-header OBU

The encoder writer bridge SHALL provide public `splot-core` functions, tracked by
`ENC-SEQUENCE-HEADER-OBU`, that build the frozen-tier minimal-intra `SequenceHeader` and emit
it as an `OBU_SEQUENCE_HEADER` (§ 5.4) in Annex B framing. The `SequenceHeader` SHALL be
parse-backed from the committed conformance vector's `sequence_header()` body. The OBU payload
SHALL be the body plus the § 5.2.1 / § 5.2.3 OBU tail (`obu_extension_flag = 0` then
`trailing_bits()`), under the no-extension § 5.2.2 `OBU_SEQUENCE_HEADER` header. The result
SHALL be byte-exact to the committed conformance vector's sequence-header OBU. It SHALL NOT
claim a temporal unit, an IVF stream, a frame OBU made consistent with it, a packet, or
`Context::receive_packet` output.

#### Scenario: The OBU matches the conformance vector

- **WHEN** `encode_minimal_intra_sequence_header_obu` is called
- **THEN** the result SHALL be byte-exact to the committed `syn-cos-intra-64x64-q180`
  vector's `OBU_SEQUENCE_HEADER` (`leb128(12)`, header `0x04`, then the 11-byte body)
- **AND** parsing it as an Annex B bitstream SHALL yield exactly one OBU, no error, header
  type `OBU_SEQUENCE_HEADER`, no extension.

#### Scenario: The bridge does not produce packets

- **WHEN** the sequence-header OBU function is available
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a temporal unit, an IVF stream, a
  packet, or Baseline Encoder Profile v1 output from it.
