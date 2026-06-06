# bitstream delta: parse-sequence-header

## ADDED Requirements

### Requirement: sequence header parsing

`splot-core` SHALL model and parse `sequence_header_obu()` (AV2 v1.0.0 § 5.4) from a
bounded OBU payload, modeling only spec-cited fields (no fabricated syntax).

#### Scenario: conformant sequence header

- **WHEN** a conformant sequence header OBU payload is parsed
- **THEN** the modeled § 5.4 fields are recovered

#### Scenario: truncated sequence header

- **WHEN** the payload ends before a field is complete
- **THEN** a typed `Error` is returned, never a panic
