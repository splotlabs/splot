# bitstream spec delta

## ADDED Requirements

### Requirement: Padding OBU parsing

`splot-core` SHALL parse `padding_obu()` (AV2 v1.0.0 § 5.16) into typed syntax using the
§ 5.16 / § 6.15 rule that the last non-zero payload byte begins `trailing_bits()`,
surfacing it as `ParsedObu::Padding`. The padding parser SHALL consume the whole payload
(padding bytes plus its own trailing bits); dispatch SHALL NOT additionally run the
shared OBU trailing-bits logic for `OBU_PADDING`.

#### Scenario: empty padding payload

- **GIVEN** an `OBU_PADDING` with `obuPayloadSize == 0`
- **WHEN** `parse_padding_obu()` reads it
- **THEN** it SHALL return a padding length of 0 and a trailing length of 0.

#### Scenario: one-byte trailing-only payload

- **GIVEN** an `OBU_PADDING` with `obuPayloadSize == 1` whose single byte is valid
  `trailing_bits()`
- **WHEN** the parser reads it
- **THEN** it SHALL return a padding length of 0 and a trailing length of 1.

#### Scenario: arbitrary padding bytes

- **GIVEN** an `OBU_PADDING` whose payload is arbitrary non-zero `obu_padding_byte`
  values followed by valid `trailing_bits()`
- **WHEN** the parser reads it
- **THEN** it SHALL accept the padding bytes and parse the trailing bits from the last
  non-zero byte.

#### Scenario: all-zero payload rejected

- **GIVEN** a non-empty `OBU_PADDING` payload whose bytes are all zero
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured `padding/all-zero-payload` error
- **AND** SHALL NOT panic.

### Requirement: Metadata OBU parsing

`splot-core` SHALL parse `metadata_short_obu()` (AV2 v1.0.0 § 5.17.2) and
`metadata_group_obu()` (§ 5.17.3) into typed syntax, reading the 1-byte metadata unit
header fields, `metadata_type` (`leb128()`, retaining `Leb128Bytes`), and the shared
`metadata_unit()` (§ 5.17.1), surfacing them as `ParsedObu::MetadataShort` /
`ParsedObu::MetadataGroup` finished with `trailing_bits()` only (metadata OBUs are not
extensible). It SHALL never read past the OBU boundary and SHALL never panic on
arbitrary input.

#### Scenario: cancelled short metadata

- **GIVEN** a `metadata_short_obu()` with `muh_cancel_flag == 1`
- **WHEN** `parse_metadata_short()` reads it
- **THEN** it SHALL return after `metadata_type` with no metadata unit
- **AND** SHALL leave the reader positioned for the OBU `trailing_bits()`.

#### Scenario: short payload size underflow

- **GIVEN** a `metadata_short_obu()` whose `obuPayloadSize` is smaller than
  `2 + Leb128Bytes`
- **WHEN** the parser computes `metadataPayloadSize`
- **THEN** it SHALL return a `metadata/unit-payload-underflow` error rather than
  underflowing.

#### Scenario: group unit count too large

- **GIVEN** a `metadata_group_obu()` with `metadata_unit_cnt_minus_1 >= 16383`
- **WHEN** the parser reads it
- **THEN** it SHALL return a `metadata/group-unit-count-too-large` error.

#### Scenario: group header underflow

- **GIVEN** a non-cancelled group unit whose `muh_header_size` is too small to account
  for `Leb128Bytes`, the fixed header fields, and the layer maps
- **WHEN** the parser decrements `headerRemainingBytes`
- **THEN** it SHALL return a `metadata/group-header-underflow` error rather than
  underflowing.

### Requirement: Bounded metadata unit parsing

`splot-core` SHALL parse `metadata_unit(metadataPayloadSize)` (AV2 v1.0.0 § 5.17.1)
bounded to exactly `metadataPayloadSize` bytes via a sub-reader, parsing the typed
§ 5.17.4-§ 5.17.13 child payload selected by `metadata_type`, preserving reserved /
unknown / private types as raw (length only), and treating `metadata_unit_remaining_bit`
as ignorable padding (any value).

#### Scenario: child syntax bounded to the declared size

- **GIVEN** a `metadata_unit()` whose `metadataPayloadSize` is smaller than its child
  syntax needs
- **WHEN** the parser reads the child payload
- **THEN** it SHALL return a `metadata/unit-payload-underflow` error rather than reading
  into the OBU trailing bits or the next unit.

#### Scenario: unknown metadata type preserved as raw

- **GIVEN** a `metadata_unit()` with a reserved or private `metadata_type`
- **WHEN** the parser reads it
- **THEN** it SHALL preserve the raw payload length and SHALL NOT return
  `Unimplemented`.

#### Scenario: remaining bits any value

- **GIVEN** a `metadata_unit()` whose child payload is shorter than `metadataPayloadSize`
  and whose `metadata_unit_remaining_bit` bits are non-zero
- **WHEN** the parser reads it
- **THEN** it SHALL accept the unit (the remaining bits are ignorable).

### Requirement: Dispatch and inspect padding and metadata

`open_bitstream_unit` dispatch SHALL route `OBU_PADDING`, `OBU_METADATA_SHORT`, and
`OBU_METADATA_GROUP` to their parsers and remove them from the unimplemented branch, and
`splot inspect --json` SHALL surface the parsed payloads, summarizing raw payload lengths
rather than dumping bytes.

#### Scenario: inspector surfaces padding and metadata

- **GIVEN** a bitstream containing an `OBU_PADDING`, an `OBU_METADATA_SHORT`, and an
  `OBU_METADATA_GROUP`
- **WHEN** `splot inspect --json` reads it
- **THEN** the output SHALL include a `padding` view with padding/trailing lengths and
  `metadata_short` / `metadata_group` views with the header fields and per-unit metadata
  types and payload sizes
- **AND** SHALL NOT dump unbounded raw metadata payload bytes.
