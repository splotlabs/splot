# bitstream spec delta

## ADDED Requirements

### Requirement: Sequence-header child parser coverage

`splot-core` SHALL provide bounded parsers for implemented `sequence_header_obu()` child structures mapped in `docs/IMPLEMENTATION-MATRIX.toml`.

#### Scenario: implemented child syntax is parsed

- **GIVEN** an Annex B bitstream containing an `OBU_SEQUENCE_HEADER`
- **AND** a child structure whose matrix row has `parse = done`
- **WHEN** the OBU is dispatched by `open_bitstream_unit(sz)`
- **THEN** the child syntax SHALL be parsed into typed Rust fields
- **AND** the parser SHALL not read past the declared OBU payload.

#### Scenario: child syntax is intentionally not implemented

- **GIVEN** an Annex B bitstream containing an `OBU_SEQUENCE_HEADER`
- **AND** a child structure whose matrix row is still `todo` or `partial`
- **WHEN** the parser reaches that feature boundary
- **THEN** the parser SHALL return a bounded unimplemented payload status or typed unimplemented error with the owning Feature ID
- **AND** it SHALL NOT silently skip unknown syntax bits.

### Requirement: HLS payload foundation

`splot-core` SHALL parse temporal delimiter, MSDO, and multi-frame-header payload syntax to the extent recorded in the matrix.

#### Scenario: MSDO local syntax is malformed

- **GIVEN** an MSDO OBU whose local syntax violates an implemented parser bound
- **WHEN** the bitstream is parsed
- **THEN** the parser SHALL return a structured error or invalid payload status
- **AND** the validator SHALL convert it to a diagnostic rather than panicking.
