# bitstream spec delta

## ADDED Requirements

### Requirement: Operating point set OBU parsing

`splot-core` SHALL parse `operating_point_set_obu()` (AV2 v1.0.0 § 5.10) and its
`operating_point_payload()` children (§ 5.11, § 5.11.1-§ 5.11.5) into typed syntax,
dispatching on `obu_xlayer_id` to the global and local branches, reading the full
nested syntax (no skipped bits), never reading past the OBU boundary, and retaining
reserved-zero fields. It SHALL surface an `operating_point_set_obu()` as
`ParsedObu::OperatingPointSet` finished with the extensible OBU tail.

#### Scenario: reset-only OPS

- **GIVEN** an OPS OBU with `ops_cnt == 0`
- **WHEN** `parse_operating_point_set()` reads it
- **THEN** it SHALL return a record with no operating point payloads
- **AND** SHALL NOT read the optional header fields.

#### Scenario: payload size accounting

- **GIVEN** an `operating_point_payload()` that declares `ops_data_size`
- **WHEN** the parser reads it
- **THEN** it SHALL preserve the declared `ops_data_size` and the computed `opsBytes`
  measured from after `ops_data_size` through the closing `byte_alignment()`.

#### Scenario: reserved values retained

- **GIVEN** a local OPS with a non-zero `ops_reserved_2bits`, or a global OPS with
  `ops_mlayer_info_idc == 3`
- **WHEN** the parser reads it
- **THEN** it SHALL retain the value for the validator rather than returning an error.

#### Scenario: truncated input

- **GIVEN** an OPS OBU truncated mid-syntax
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured error
- **AND** SHALL NOT panic.

### Requirement: Buffer removal timing OBU parsing

`splot-core` SHALL parse `buffer_removal_timing_obu()` (AV2 v1.0.0 § 5.12) into typed
syntax in both forms selected by `br_ops_dependent_flag`, surfacing it as
`ParsedObu::BufferRemovalTiming` finished with `trailing_bits()` only (the OBU is not
extensible).

#### Scenario: extended-layer BRT

- **GIVEN** a BRT OBU with `br_ops_dependent_flag == 0`
- **WHEN** `parse_buffer_removal_timing()` reads it
- **THEN** it SHALL parse a single `br_time` and no per-operating-point records.

#### Scenario: OPS-dependent BRT

- **GIVEN** a BRT OBU with `br_ops_dependent_flag == 1`
- **WHEN** the parser reads it
- **THEN** it SHALL parse `br_ops_id`, `br_ops_cnt`, and each per-operating-point
  present flag with its optional `br_time_op`.

### Requirement: Dispatch and inspect OPS and BRT

`open_bitstream_unit` dispatch SHALL route `OBU_OPERATING_POINT_SET` and
`OBU_BUFFER_REMOVAL_TIMING` to their parsers, and `splot inspect --json` SHALL surface
the parsed payloads.

#### Scenario: inspector surfaces OPS and BRT

- **GIVEN** a bitstream containing an `operating_point_set_obu()` and a
  `buffer_removal_timing_obu()`
- **WHEN** `splot inspect --json` reads it
- **THEN** the output SHALL include an `operating_point_set` view and a
  `buffer_removal_timing` view with the key parsed fields.
