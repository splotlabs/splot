# bitstream delta: parse-annexb-and-obu-headers

## ADDED Requirements

### Requirement: Annex B + OBU header parsing

`splot-core` SHALL parse the AV2 Annex B envelope and OBU headers (AV2 v1.0.0
§ 4.11.6, § 5.2.1, § 5.2.2, Annex B) with strong types, panic-free, never copying
AV1 OBU header fields or the AV1 OBU type table.

#### Scenario: conformant Annex B stream

- **WHEN** a conformant Annex B stream is parsed
- **THEN** each OBU envelope, header, and payload is recovered with correct offsets

#### Scenario: malformed input

- **WHEN** truncated or out-of-range input is parsed
- **THEN** a typed `Error` is returned and any parseable prefix is retained
