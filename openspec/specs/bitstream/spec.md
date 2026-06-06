# bitstream Specification

## Purpose

The AV2 bitstream model and parsers in `splot-core`. Normative reference: AV2
v1.0.0. This capability never panics on malformed input — every failure is a typed
`Error`.

Tracked by Feature IDs: `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`,
`AV2-5.2.1-OBU-TYPE`, `AV2-B-ANNEXB-OBU-ENVELOPE`, plus the not-yet-parsed
header/syntax rows in `docs/IMPLEMENTATION-MATRIX.toml`.

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
