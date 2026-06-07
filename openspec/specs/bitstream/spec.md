# bitstream Specification

## Purpose

The AV2 bitstream model and parsers in `splot-core`. Normative reference: AV2
v1.0.0. This capability never panics on malformed input — every failure is a typed
`Error`.

Tracked by Feature IDs: `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`,
`AV2-5.2.1-OBU-TYPE`, `AV2-B-ANNEXB-OBU-ENVELOPE`, `AV2-5.8-LAYER-CONFIG-RECORD`,
`AV2-5.9-ATLAS-SEGMENT`, plus the not-yet-parsed header/syntax rows in
`docs/IMPLEMENTATION-MATRIX.toml`.
## Requirements
### Requirement: LEB128 decoding

The reader SHALL decode `leb128()` per AV2 v1.0.0 § 4.11.6: byte-aligned, at most
8 bytes, value bounded to `(1 << 32) - 1`, non-minimal encodings permitted.

#### Scenario: minimal and non-minimal encodings

- **WHEN** decoding `0x00` or `0x80 0x00`
- **THEN** both yield `0`, recording the number of bytes consumed

#### Scenario: overflow or overlong

- **WHEN** a value exceeds `u32` or uses more than 8 bytes
- **THEN** an `Error` is returned, never a panic

#### Scenario: bit-reader descriptor path

- **WHEN** the bit reader decodes the `leb128()` element `0x80 0x01`
- **THEN** it yields `128` and advances by two bytes
- **AND** a truncated or overlong code returns an `Error`, never a panic

### Requirement: AV2 OBU header

The reader SHALL parse the AV2 OBU header per AV2 v1.0.0 § 5.2.2 — the AV2 layout
(`obu_header_extension_flag`, `obu_type`, `obu_tlayer_id`, and the optional
`obu_mlayer_id`/`obu_xlayer_id`), NOT the AV1 OBU header. There is no
`obu_forbidden_bit`, `obu_has_size_field`, or AV1 OBU type table.

#### Scenario: inferred xlayer

- **WHEN** parsing an `OBU_MSDO` or `OBU_TEMPORAL_DELIMITER` without the extension
- **THEN** `obu_xlayer_id` is inferred to `GLOBAL_XLAYER_ID`

### Requirement: Annex B envelope

The reader SHALL parse the Annex B length-delimited envelope per AV2 v1.0.0
Annex B: each OBU is a `leb128()` length followed by `open_bitstream_unit(...)`.
Header parsing SHALL be bounded to the declared OBU size.

#### Scenario: conformant Annex B stream

- **WHEN** a conformant Annex B stream is parsed
- **THEN** each OBU envelope, header, and payload is recovered with correct offsets

#### Scenario: bounded header parse

- **WHEN** an OBU header signals an extension byte beyond its declared size
- **THEN** parsing fails within that OBU rather than reading into the next one

#### Scenario: malformed input

- **WHEN** truncated or out-of-range input is parsed
- **THEN** a typed `Error` is returned and any parseable prefix is retained

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

### Requirement: Layer configuration record OBU parsing

`splot-core` SHALL parse `layer_config_record_obu()` (AV2 v1.0.0 § 5.8) into typed
syntax, dispatching on `obu_xlayer_id` to `lcr_global_info()` or
`lcr_local_info(obu_xlayer_id)`, reading the full nested syntax (including the
length-bounded `lcr_global_payload()`), never skipping payload bits and never reading
past the OBU boundary, and retaining reserved-zero fields rather than rejecting them.

#### Scenario: minimal global record

- **GIVEN** a global LCR (`obu_xlayer_id == GLOBAL_XLAYER_ID`) with no optional sections
- **WHEN** the parser reads it
- **THEN** it SHALL return a global record exposing `lcr_global_config_record_id` and
  `lcr_xlayer_map`.

#### Scenario: global payload remaining bits and overflow

- **GIVEN** a global LCR with `lcr_global_payload_present_flag` set
- **WHEN** the parser reads the payload
- **THEN** it SHALL consume exactly `lcr_data_size * 8` bits including the trailing
  `lcr_remaining_payload_bit` bits
- **AND** parsed content exceeding `lcr_data_size * 8` SHALL return a structured error.

#### Scenario: truncated record

- **GIVEN** a layer configuration record OBU that ends mid-field
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured end-of-input error and SHALL NOT panic.

### Requirement: Atlas segment info OBU parsing

`splot-core` SHALL parse `atlas_segment_info_obu()` (AV2 v1.0.0 § 5.9) into typed
syntax for all five `ats_atlas_segment_mode_idc` modes plus `ats_label_segment_info()`,
never skipping payload bits and never reading past the OBU boundary, and SHALL
range-check the mode and the segment/region counts before any loop.

#### Scenario: single-mode atlas

- **GIVEN** an atlas OBU with `ats_atlas_segment_mode_idc == SINGLE_ATLAS`
- **WHEN** the parser reads it
- **THEN** it SHALL return a record with `num_segments == 1` and the nominal
  dimensions.

#### Scenario: out-of-range mode or count

- **GIVEN** an atlas OBU with `ats_atlas_segment_mode_idc` greater than 4, or a segment
  count reaching `MAX_NUM_ATLAS_SEGMENTS`
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured atlas-segment error before iterating, and SHALL
  NOT panic.

